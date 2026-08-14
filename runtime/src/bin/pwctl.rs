//! CLI client for pmux runtime.
//!
//! Connects to the running runtime via Unix socket and sends commands.
//!
//! Usage:
//!   pwctl status
//!   pwctl list
//!   pwctl create <workspace>
//!   pwctl destroy <workspace>
//!   pwctl switch <workspace>
//!   pwctl panes <workspace>
//!   pwctl add <workspace> [--name <name>] [--no-session]
//!   pwctl split <workspace> [--direction h|v] [--name <name>] [--no-session]
//!   pwctl close <workspace> <pane>
//!   pwctl send <workspace> <pane> <command...>
//!   pwctl read <workspace> <pane>
//!   pwctl focus <workspace> <pane>
//!   pwctl ping

use pmux::attach;
use pmux::ipc::{self, PaneInfo, Request, Response, StatusInfo, WorkspaceInfo};
use pmux::ipc_client::IpcClient;
use pmux::token;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--daemon") {
        pmux_runtime::daemon::run();
    }

    if args.is_empty() {
        print_usage();
        std::process::exit(1);
    }

    match args[0].as_str() {
        "start" => {
            let rotate = args.iter().any(|a| a == "--rotate");
            match start_daemon(rotate) {
                Ok(msg) => println!("{msg}"),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        "token" => {
            let rotate = args.iter().any(|a| a == "--rotate");
            if rotate && IpcClient::ping() {
                eprintln!("error: runtime is up — pwctl stop then start --rotate");
                std::process::exit(1);
            }
            match token::ensure(rotate) {
                Ok(t) => {
                    println!("{}", t);
                    if rotate {
                        eprintln!("wrote {}", token::token_path().display());
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        "list" | "ls" => {
            if let Err(e) = print_runtime_list() {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
            return;
        }
        "stop" => {
            // fall through as Shutdown below via rewritten args
        }
        _ => {}
    }

    let args = if args[0] == "stop" {
        vec!["shutdown".into()]
    } else {
        args
    };

    let request = match parse_args(&args) {
        Ok(req) => req,
        Err(msg) => {
            eprintln!("error: {}", msg);
            std::process::exit(1);
        }
    };

    let client = match IpcClient::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cannot connect to runtime: {e}");
            eprintln!("is the daemon running? (`pwctl start` or `pwctl ping`)");
            std::process::exit(1);
        }
    };

    let response = match client.request(request) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    match response {
        Response::Ok { data } => {
            if let Some(data) = data {
                println!("{}", serde_json::to_string_pretty(&data).unwrap());
            } else {
                println!("ok");
            }
        }
        Response::Error { message } => {
            eprintln!("error: {}", message);
            std::process::exit(1);
        }
    }
}

fn parse_args(args: &[String]) -> Result<Request, String> {
    let cmd = args[0].as_str();

    match cmd {
        "ping" => Ok(Request::Ping),
        "status" => Ok(Request::Status),
        "list" | "ls" => Ok(Request::ListWorkspaces),

        "create" | "new" => {
            let name = args.get(1).ok_or("usage: pwctl create <workspace>")?;
            Ok(Request::CreateWorkspace { name: name.clone() })
        }

        "destroy" | "rm" => {
            let ws = args.get(1).ok_or("usage: pwctl destroy <workspace>")?;
            Ok(Request::DestroyWorkspace {
                workspace: ws.clone(),
            })
        }

        "switch" | "sw" => {
            let ws = args.get(1).ok_or("usage: pwctl switch <workspace>")?;
            Ok(Request::SwitchWorkspace {
                workspace: ws.clone(),
            })
        }

        "panes" | "ps" => {
            let ws = args.get(1).ok_or("usage: pwctl panes <workspace>")?;
            Ok(Request::ListPanes {
                workspace: ws.clone(),
            })
        }

        "add" => {
            let ws = args.get(1).ok_or("usage: pwctl add <workspace> [--name <n>] [--no-session]")?;
            let mut name = None;
            let mut spawn_session = true;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--name" | "-n" => {
                        i += 1;
                        name = Some(args.get(i).ok_or("--name requires value")?.clone());
                    }
                    "--no-session" => spawn_session = false,
                    _ => return Err(format!("unknown flag: {}", args[i])),
                }
                i += 1;
            }
            Ok(Request::AddPane {
                workspace: ws.clone(),
                name,
                spawn_session,
            })
        }

        "split" => {
            let ws = args
                .get(1)
                .ok_or("usage: pwctl split <workspace> [--direction h|v] [--name <n>]")?;
            let mut direction = "horizontal".to_string();
            let mut name = None;
            let mut spawn_session = true;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--direction" | "-d" => {
                        i += 1;
                        direction = args.get(i).ok_or("--direction requires value")?.clone();
                    }
                    "--name" | "-n" => {
                        i += 1;
                        name = Some(args.get(i).ok_or("--name requires value")?.clone());
                    }
                    "--no-session" => spawn_session = false,
                    _ => return Err(format!("unknown flag: {}", args[i])),
                }
                i += 1;
            }
            Ok(Request::SplitPane {
                workspace: ws.clone(),
                direction,
                name,
                spawn_session,
            })
        }

        "close" => {
            let ws = args.get(1).ok_or("usage: pwctl close <workspace> <pane>")?;
            let pane = args.get(2).ok_or("usage: pwctl close <workspace> <pane>")?;
            Ok(Request::ClosePane {
                workspace: ws.clone(),
                pane: pane.clone(),
            })
        }

        "send" | "exec" => {
            if args.len() < 4 {
                return Err("usage: pwctl send <workspace> <pane> <command...>".into());
            }
            let ws = &args[1];
            let pane = &args[2];
            let command = args[3..].join(" ");
            Ok(Request::SendCommand {
                workspace: ws.clone(),
                pane: pane.clone(),
                command,
            })
        }

        "read" | "output" => {
            let ws = args.get(1).ok_or("usage: pwctl read <workspace> <pane>")?;
            let pane = args.get(2).ok_or("usage: pwctl read <workspace> <pane>")?;
            Ok(Request::ReadOutput {
                workspace: ws.clone(),
                pane: pane.clone(),
            })
        }

        "focus" => {
            let ws = args.get(1).ok_or("usage: pwctl focus <workspace> <pane>")?;
            let pane = args.get(2).ok_or("usage: pwctl focus <workspace> <pane>")?;
            Ok(Request::FocusPane {
                workspace: ws.clone(),
                pane: pane.clone(),
            })
        }

        "view" | "preview" | "open" => parse_view(&args[1..]),

        "shutdown" | "kill-server" | "stop" => Ok(Request::Shutdown),

        _ => Err(format!("unknown command: {}", cmd)),
    }
}

fn parse_view(args: &[String]) -> Result<Request, String> {
    let (workspace, path) = match args {
        [path] => {
            let ws = std::env::var("PW_WORKSPACE_NAME")
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| std::env::var("PW_WORKSPACE_ID").ok().filter(|s| !s.is_empty()))
                .ok_or(
                    "usage: pwctl view [<workspace>] <path>\n(inside a pane, PW_WORKSPACE_NAME is enough)",
                )?;
            (ws, path.clone())
        }
        [ws, path] => (ws.clone(), path.clone()),
        _ => return Err("usage: pwctl view [<workspace>] <path>".into()),
    };
    Ok(Request::OpenView {
        workspace,
        path: resolve_path(&path),
    })
}

fn resolve_path(path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    let p = std::path::PathBuf::from(path);
    if p.is_absolute() {
        return p.display().to_string();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(&p))
        .unwrap_or(p)
        .display()
        .to_string()
}

/// Start headless runtime (`pwctl --daemon`) if ping fails.
fn start_daemon(rotate: bool) -> Result<String, String> {
    let already = ping_ok();
    if already && rotate {
        return Err("runtime already up — pwctl stop then start --rotate".into());
    }
    let token = token::ensure(rotate).map_err(|e| e.to_string())?;
    attach::ensure_running().map_err(|e| e.to_string())?;
    let sock = ipc::socket_path();
    let mut out = format!(
        "{} ({})",
        if already { "already running" } else { "started" },
        sock.display()
    );
    if let Some(addr) = ipc::tcp_listen_addr() {
        out.push_str(&format!("\nlisten: {addr}  (LAN TCP, token required)"));
    }
    out.push_str(&format!("\ntoken:  {token}"));
    out.push_str(&format!("\nfile:   {}", token::token_path().display()));
    Ok(out)
}

fn ping_ok() -> bool {
    IpcClient::ping()
}

fn rpc(req: Request) -> Result<Response, String> {
    let client = IpcClient::connect().map_err(|e| e.to_string())?;
    client.request(req).map_err(|e| e.to_string())
}

fn rpc_data<T: serde::de::DeserializeOwned>(req: Request) -> Result<T, String> {
    match rpc(req)? {
        Response::Ok { data: Some(v) } => {
            serde_json::from_value(v).map_err(|e| e.to_string())
        }
        Response::Ok { data: None } => Err("missing data".into()),
        Response::Error { message } => Err(message),
    }
}

fn print_runtime_list() -> Result<(), String> {
    let sock = ipc::socket_path();
    if !ping_ok() {
        println!("runtime: down");
        println!("socket:  {}  (one sock, all workspaces)", sock.display());
        println!("hint:    pwctl start");
        return Ok(());
    }

    let pid = std::fs::read_to_string(attach::pid_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "?".into());
    let status: StatusInfo = rpc_data(Request::Status)?;
    let workspaces: Vec<WorkspaceInfo> = rpc_data(Request::ListWorkspaces)?;

    println!("runtime: up");
    println!("pid:     {pid}");
    println!("socket:  {}  (one sock, all workspaces)", sock.display());
    if let Some(addr) = ipc::tcp_listen_addr() {
        println!("listen:  {addr}  (TCP, token)");
    }
    println!(
        "counts:  {} workspace(s)  {} session(s)",
        status.workspaces, status.sessions
    );
    println!();
    if workspaces.is_empty() {
        println!("(no workspaces)");
        return Ok(());
    }
    for ws in &workspaces {
        let mark = if ws.active { "*" } else { " " };
        println!("{mark} {}  {}  {} pane(s)", ws.name, ws.id, ws.pane_count);
        let panes: Vec<PaneInfo> = rpc_data(Request::ListPanes {
            workspace: ws.id.clone(),
        })?;
        for p in panes {
            let name = p.name.as_deref().unwrap_or("(unnamed)");
            let live = if p.has_session { "pty" } else { "-" };
            let focus = if p.focused { "  focus" } else { "" };
            println!("    {name:<18} {live:<4} {}{focus}", p.id);
        }
    }
    Ok(())
}

fn print_usage() {
    eprintln!("pwctl — pmux CLI

Commands:
  ping                          Health check (daemon up?)
  start [--rotate]              Start daemon; print LAN token (rotate = new token)
  token [--rotate]              Show (or regenerate) LAN token
  stop | shutdown               Stop daemon + kill all sessions
  list                          Runtime + workspaces + panes (what's alive)
  status                        Runtime counts (JSON)
  create <workspace>            Create workspace
  destroy <workspace>           Destroy workspace
  switch <workspace>            Switch active workspace
  panes <workspace>             List panes
  add <workspace> [opts]        Add pane (--name, --no-session)
  split <workspace> [opts]      Split pane (--direction h|v, --name, --no-session)
  close <workspace> <pane>      Close pane
  send <workspace> <pane> <cmd> Send command (name or id)
  read <workspace> <pane>       Read pane output (name or id)
  focus <workspace> <pane>      Focus pane (name or id)
  view [<workspace>] <path>     Preview md/image in view pane
                                (workspace optional if PW_WORKSPACE_NAME set)

UI: `pmux` attaches (starts `pwctl --daemon` if needed).
  ✕ / ⌘⇧D detach (keep runtime) · `pwctl stop` kills runtime.");
}
