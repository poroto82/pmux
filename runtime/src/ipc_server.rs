//! IPC server — Unix socket + optional LAN TCP (token required).

use std::io::{BufRead, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use pmux::action::{ActionContext, ActionResult};
use pmux::ids::ComponentId;
use pmux::ipc::{
    self, ActionOutcome, PaneInfo, Request, Response, StatusInfo, WorkspaceInfo,
};
use pmux::layout::Direction;
use pmux::token;

use crate::runtime::Runtime;

/// Start the IPC server on a background thread.
/// Returns the listener's path for cleanup.
pub fn start(runtime: Arc<Mutex<Runtime>>) -> std::io::Result<std::path::PathBuf> {
    start_with_shutdown(runtime, None)
}

pub fn start_with_shutdown(
    runtime: Arc<Mutex<Runtime>>,
    shutdown: Option<Arc<AtomicBool>>,
) -> std::io::Result<std::path::PathBuf> {
    let path = ipc::socket_path();

    let _ = std::fs::remove_file(&path);

    let listener = UnixListener::bind(&path)?;
    let path_clone = path.clone();

    let rt_u = runtime.clone();
    let sd_u = shutdown.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let rt = rt_u.clone();
                    let sd = sd_u.clone();
                    thread::spawn(move || {
                        let reader = stream.try_clone().unwrap();
                        handle_rpc_loop(rt, sd, std::io::BufReader::new(reader), stream);
                    });
                }
                Err(e) => {
                    eprintln!("ipc accept error: {}", e);
                }
            }
        }
    });

    if let Some(addr) = ipc::tcp_listen_addr() {
        match TcpListener::bind(&addr) {
            Ok(tcp) => {
                let token = Arc::new(token::ensure(false)?);
                eprintln!("pmux tcp listen={addr} (token required)");
                let rt_t = runtime;
                let sd_t = shutdown;
                thread::spawn(move || {
                    for stream in tcp.incoming() {
                        match stream {
                            Ok(stream) => {
                                let _ = stream.set_nodelay(true);
                                let rt = rt_t.clone();
                                let sd = sd_t.clone();
                                let tok = token.clone();
                                thread::spawn(move || handle_tcp(rt, sd, stream, tok));
                            }
                            Err(e) => eprintln!("tcp accept error: {e}"),
                        }
                    }
                });
            }
            Err(e) => {
                eprintln!("tcp bind {addr} failed: {e} (unix socket still up)");
            }
        }
    }

    Ok(path_clone)
}

fn handle_tcp(
    runtime: Arc<Mutex<Runtime>>,
    shutdown: Option<Arc<AtomicBool>>,
    stream: std::net::TcpStream,
    expected: Arc<String>,
) {
    let reader = match stream.try_clone() {
        Ok(r) => r,
        Err(_) => return,
    };
    let mut buf_reader = std::io::BufReader::new(reader);
    let mut writer = stream;

    let line = match ipc::read_message(&mut buf_reader) {
        Ok(line) if !line.is_empty() => line,
        _ => return,
    };
    let request: Request = match serde_json::from_str(&line) {
        Ok(r) => r,
        Err(_) => {
            let resp = Response::error("unauthorized");
            let _ = ipc::write_message(&mut writer, &serde_json::to_string(&resp).unwrap());
            return;
        }
    };
    match request {
        Request::Auth { token } if token::eq(&token, expected.as_str()) => {
            let resp = Response::ok();
            if ipc::write_message(&mut writer, &serde_json::to_string(&resp).unwrap()).is_err() {
                return;
            }
        }
        _ => {
            let resp = Response::error("unauthorized");
            let _ = ipc::write_message(&mut writer, &serde_json::to_string(&resp).unwrap());
            return;
        }
    }

    handle_rpc_loop(runtime, shutdown, buf_reader, writer);
}

fn handle_rpc_loop<R, W>(
    runtime: Arc<Mutex<Runtime>>,
    shutdown: Option<Arc<AtomicBool>>,
    mut buf_reader: R,
    mut writer: W,
) where
    R: BufRead,
    W: Write,
{
    loop {
        let line = match ipc::read_message(&mut buf_reader) {
            Ok(line) if line.is_empty() => break,
            Ok(line) => line,
            Err(_) => break,
        };

        let request: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response::error(format!("invalid request: {}", e));
                let _ = ipc::write_message(&mut writer, &serde_json::to_string(&resp).unwrap());
                continue;
            }
        };

        let response = {
            let mut rt = runtime.lock().unwrap();
            dispatch(&mut rt, shutdown.as_deref(), request)
        };

        let json = serde_json::to_string(&response).unwrap();
        if ipc::write_message(&mut writer, &json).is_err() {
            break;
        }
    }
}

fn dispatch(rt: &mut Runtime, shutdown: Option<&AtomicBool>, req: Request) -> Response {
    match req {
        Request::Ping => Response::ok_data("pong"),

        Request::Auth { .. } => Response::ok(),

        Request::Status => Response::ok_data(StatusInfo {
            workspaces: rt.workspaces.count(),
            sessions: rt.sessions.count(),
            actions: rt.actions.count(),
            components: rt.components.count(),
        }),

        Request::ListWorkspaces => {
            let active_id = rt.workspaces.active_id().cloned();
            let workspaces: Vec<WorkspaceInfo> = rt
                .workspaces
                .list()
                .iter()
                .map(|ws| WorkspaceInfo {
                    id: ws.id.to_string(),
                    name: ws.name.clone(),
                    pane_count: ws.pane_count(),
                    active: Some(&ws.id) == active_id.as_ref(),
                })
                .collect();
            Response::ok_data(workspaces)
        }

        Request::CreateWorkspace { name } => {
            let id = rt.create_workspace(&name);
            Response::ok_data(serde_json::json!({
                "id": id.to_string(),
                "name": name,
            }))
        }

        Request::DestroyWorkspace { workspace } => {
            let ws = match rt.workspaces.resolve_workspace(&workspace) {
                Some(ws) => ws.id.clone(),
                None => return Response::error(format!("workspace not found: {}", workspace)),
            };
            rt.destroy_workspace(&ws);
            Response::ok()
        }

        Request::SwitchWorkspace { workspace } => {
            let ws = match rt.workspaces.resolve_workspace(&workspace) {
                Some(ws) => ws.id.clone(),
                None => return Response::error(format!("workspace not found: {}", workspace)),
            };
            if rt.switch_workspace(&ws) {
                Response::ok()
            } else {
                Response::error("switch failed")
            }
        }

        Request::ListPanes { workspace } => {
            let ws = match rt.workspaces.resolve_workspace(&workspace) {
                Some(ws) => ws,
                None => return Response::error(format!("workspace not found: {}", workspace)),
            };
            let ws_id = ws.id.clone();
            let focused = ws.layout().focused().cloned();
            let pane_snapshots: Vec<(String, Option<String>, pmux::ids::PaneId)> = ws
                .panes()
                .map(|p| (p.id.to_string(), p.name.clone(), p.id.clone()))
                .collect();
            let panes: Vec<PaneInfo> = pane_snapshots
                .into_iter()
                .map(|(id, name, pane_id)| PaneInfo {
                    id,
                    name,
                    has_session: rt.pane_has_live_session(&ws_id, &pane_id),
                    focused: Some(&pane_id) == focused.as_ref(),
                })
                .collect();
            Response::ok_data(panes)
        }

        Request::AddPane {
            workspace,
            name,
            spawn_session,
        } => {
            let ws_id = match rt.workspaces.resolve_workspace(&workspace) {
                Some(ws) => ws.id.clone(),
                None => return Response::error(format!("workspace not found: {}", workspace)),
            };
            match rt.add_pane(&ws_id, ComponentId::new(), name.as_deref(), spawn_session) {
                Some(pane_id) => Response::ok_data(serde_json::json!({
                    "pane_id": pane_id.to_string(),
                })),
                None => Response::error("failed to add pane"),
            }
        }

        Request::SplitPane {
            workspace,
            direction,
            name,
            spawn_session,
        } => {
            let ws_id = match rt.workspaces.resolve_workspace(&workspace) {
                Some(ws) => ws.id.clone(),
                None => return Response::error(format!("workspace not found: {}", workspace)),
            };
            let dir = match direction.as_str() {
                "horizontal" | "h" => Direction::Horizontal,
                "vertical" | "v" => Direction::Vertical,
                _ => return Response::error(format!("invalid direction: {}", direction)),
            };
            match rt.split(&ws_id, None, dir, ComponentId::new(), name.as_deref(), spawn_session) {
                Some(pane_id) => Response::ok_data(serde_json::json!({
                    "pane_id": pane_id.to_string(),
                })),
                None => Response::error("split failed"),
            }
        }

        Request::ClosePane { workspace, pane } => {
            let (ws_id, pane_id) = match resolve_pane(rt, &workspace, &pane) {
                Ok(ids) => ids,
                Err(resp) => return resp,
            };
            if rt.close_pane(&ws_id, &pane_id) {
                Response::ok()
            } else {
                Response::error("close failed")
            }
        }

        Request::SendCommand {
            workspace,
            pane,
            command,
        } => {
            let (ws_id, pane_id) = match resolve_pane(rt, &workspace, &pane) {
                Ok(ids) => ids,
                Err(resp) => return resp,
            };
            match rt.send_command(&ws_id, &pane_id, &command) {
                Ok(()) => Response::ok(),
                Err(e) => Response::error(e.to_string()),
            }
        }

        Request::ReadOutput { workspace, pane } => {
            let (ws_id, pane_id) = match resolve_pane(rt, &workspace, &pane) {
                Ok(ids) => ids,
                Err(resp) => return resp,
            };
            match rt.read_output(&ws_id, &pane_id) {
                Ok(data) => {
                    let text = String::from_utf8_lossy(&data).to_string();
                    Response::ok_data(serde_json::json!({
                        "output": text,
                        "bytes": data,
                    }))
                }
                Err(e) => Response::error(e.to_string()),
            }
        }

        Request::ReadReplay { workspace, pane } => {
            let (ws_id, pane_id) = match resolve_pane(rt, &workspace, &pane) {
                Ok(ids) => ids,
                Err(resp) => return resp,
            };
            match rt.replay_output(&ws_id, &pane_id) {
                Ok(data) => Response::ok_data(serde_json::json!({
                    "output": String::from_utf8_lossy(&data),
                    "bytes": data,
                })),
                Err(e) => Response::error(e.to_string()),
            }
        }

        Request::FocusPane { workspace, pane } => {
            let (ws_id, pane_id) = match resolve_pane(rt, &workspace, &pane) {
                Ok(ids) => ids,
                Err(resp) => return resp,
            };
            if rt.focus_pane(&ws_id, &pane_id) {
                Response::ok()
            } else {
                Response::error("focus failed")
            }
        }

        Request::OpenView { workspace, path } => {
            let ws_id = match rt.workspaces.resolve_workspace(&workspace) {
                Some(ws) => ws.id.clone(),
                None => return Response::error(format!("workspace not found: {}", workspace)),
            };
            match rt.open_view(&ws_id, &path) {
                Ok(pane_id) => Response::ok_data(serde_json::json!({
                    "pane_id": pane_id.to_string(),
                    "path": path,
                })),
                Err(e) => Response::error(e.to_string()),
            }
        }

        Request::Snapshot { workspace } => {
            Response::ok_data(rt.ui_snapshot(workspace.as_deref()))
        }

        Request::PollUi {
            workspace,
            inputs,
            resizes,
        } => {
            let ws_key = workspace.or_else(|| {
                rt.workspaces.active_id().map(|id| id.to_string())
            });
            let Some(ws_key) = ws_key else {
                return Response::ok_data(ipc::PollUiData {
                    snapshot: rt.ui_snapshot(None),
                    outputs: Vec::new(),
                });
            };
            for r in resizes {
                if let Ok((ws, pane)) = resolve_pane(rt, &ws_key, &r.pane) {
                    let _ = rt.resize_pty(&ws, &pane, r.cols, r.rows);
                }
            }
            for inp in inputs {
                if let Ok((ws, pane)) = resolve_pane(rt, &ws_key, &inp.pane) {
                    let _ = rt.send_input(&ws, &pane, &inp.bytes);
                }
            }
            let snapshot = rt.ui_snapshot(Some(&ws_key));
            let mut outputs = Vec::new();
            for pane in &snapshot.panes {
                if pane.component_type != "terminal" {
                    continue;
                }
                let Ok((ws, pid)) = resolve_pane(rt, &ws_key, &pane.id) else {
                    continue;
                };
                if let Ok(data) = rt.read_output(&ws, &pid) {
                    if !data.is_empty() {
                        outputs.push(ipc::PaneBytes {
                            pane: pane.id.clone(),
                            bytes: data,
                        });
                    }
                }
            }
            Response::ok_data(ipc::PollUiData { snapshot, outputs })
        }

        Request::SendInput {
            workspace,
            pane,
            bytes,
        } => {
            let (ws_id, pane_id) = match resolve_pane(rt, &workspace, &pane) {
                Ok(ids) => ids,
                Err(resp) => return resp,
            };
            match rt.send_input(&ws_id, &pane_id, &bytes) {
                Ok(()) => Response::ok(),
                Err(e) => Response::error(e.to_string()),
            }
        }

        Request::ResizePty {
            workspace,
            pane,
            cols,
            rows,
        } => {
            let (ws_id, pane_id) = match resolve_pane(rt, &workspace, &pane) {
                Ok(ids) => ids,
                Err(resp) => return resp,
            };
            match rt.resize_pty(&ws_id, &pane_id, cols, rows) {
                Ok(()) => Response::ok(),
                Err(e) => Response::error(e.to_string()),
            }
        }

        Request::ExecuteAction {
            name,
            workspace,
            pane,
        } => {
            let mut ctx = ActionContext::new();
            if let Some(key) = workspace.as_deref() {
                if let Some(ws) = rt.workspaces.resolve_workspace(key) {
                    ctx = ctx.with_workspace(ws.id.clone());
                }
            }
            if let Some(pkey) = pane.as_deref() {
                let ws_id = ctx
                    .workspace_id
                    .clone()
                    .or_else(|| rt.workspaces.active_id().cloned());
                if let Some(ws_id) = ws_id {
                    if let Some(p) = rt.workspaces.get(&ws_id).and_then(|w| w.resolve_pane(pkey)) {
                        ctx = ctx.with_pane(p.id.clone());
                    }
                }
            }
            let result = rt.execute_action(&name, &ctx);
            let active_id = rt.workspaces.active_id().map(|id| id.to_string());
            match result {
                ActionResult::Ok => Response::ok_data(ActionOutcome {
                    ok: true,
                    message: None,
                    active_id,
                }),
                ActionResult::Message(message) => Response::ok_data(ActionOutcome {
                    ok: true,
                    message: Some(message),
                    active_id,
                }),
                ActionResult::Error(e) => Response::error(e),
            }
        }

        Request::PaletteItems { query } => {
            Response::ok_data(rt.actions.palette_items(&query))
        }

        Request::RenameWorkspace { workspace, name } => {
            let ws_id = match resolve_ws(rt, &workspace) {
                Ok(id) => id,
                Err(resp) => return resp,
            };
            match rt.rename_workspace(&ws_id, &name) {
                Ok(n) => Response::ok_data(serde_json::json!({ "name": n })),
                Err(e) => Response::error(e.to_string()),
            }
        }

        Request::SetWorkspaceCwd { workspace, cwd } => {
            let ws_id = match resolve_ws(rt, &workspace) {
                Ok(id) => id,
                Err(resp) => return resp,
            };
            match rt.set_workspace_cwd(&ws_id, cwd) {
                Ok(c) => Response::ok_data(serde_json::json!({ "cwd": c })),
                Err(e) => Response::error(e.to_string()),
            }
        }

        Request::QuickOpenRoot { workspace, pane } => {
            let ws_id = match resolve_ws(rt, &workspace) {
                Ok(id) => id,
                Err(resp) => return resp,
            };
            let pane_id = pane.as_deref().map(pmux::ids::PaneId::from_raw);
            let root = rt.quick_open_root(&ws_id, pane_id.as_ref());
            Response::ok_data(serde_json::json!({ "root": root.display().to_string() }))
        }

        Request::SwapPanes { workspace, a, b } => {
            let ws_id = match resolve_ws(rt, &workspace) {
                Ok(id) => id,
                Err(resp) => return resp,
            };
            let a = pmux::ids::PaneId::from_raw(a);
            let b = pmux::ids::PaneId::from_raw(b);
            if rt.swap_panes(&ws_id, &a, &b) {
                Response::ok()
            } else {
                Response::error("swap failed")
            }
        }

        Request::ResizeSplit {
            workspace,
            index,
            ratio,
        } => {
            let ws_id = match resolve_ws(rt, &workspace) {
                Ok(id) => id,
                Err(resp) => return resp,
            };
            if rt.resize_split_at(&ws_id, index, ratio) {
                Response::ok()
            } else {
                Response::error("resize failed")
            }
        }

        Request::SetFloatGeom {
            workspace,
            pane,
            x,
            y,
            width,
            height,
        } => {
            let (ws_id, pane_id) = match resolve_pane(rt, &workspace, &pane) {
                Ok(ids) => ids,
                Err(resp) => return resp,
            };
            if rt.set_float_geom(&ws_id, &pane_id, x, y, width, height) {
                Response::ok()
            } else {
                Response::error("float geom failed")
            }
        }

        Request::KillListenPid { pid } => match rt.kill_listen_pid(pid) {
            Ok(()) => Response::ok(),
            Err(e) => Response::error(e),
        },

        Request::Save => match rt.save() {
            Ok(()) => Response::ok(),
            Err(e) => Response::error(e.to_string()),
        },

        Request::Shutdown => {
            let _ = rt.save();
            if let Some(flag) = shutdown {
                flag.store(true, Ordering::SeqCst);
            }
            Response::ok()
        }
    }
}

fn resolve_ws(
    rt: &Runtime,
    key: &str,
) -> Result<pmux::ids::WorkspaceId, Response> {
    rt.workspaces
        .resolve_workspace(key)
        .map(|w| w.id.clone())
        .ok_or_else(|| Response::error(format!("workspace not found: {key}")))
}

/// Resolve workspace + pane by human name or stable id string.
fn resolve_pane(
    rt: &Runtime,
    workspace_key: &str,
    pane_key: &str,
) -> Result<(pmux::ids::WorkspaceId, pmux::ids::PaneId), Response> {
    let (ws, pane) = rt
        .workspaces
        .resolve(workspace_key, pane_key)
        .ok_or_else(|| {
            Response::error(format!(
                "address not found: {}/{}",
                workspace_key, pane_key
            ))
        })?;
    Ok((ws.id.clone(), pane.id.clone()))
}
