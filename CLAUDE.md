# pworkspaces

Desktop workspace runtime (tiling panes + PTY sessions + IPC). Spec: `spec.md`.

## Agent / Claude inside a pane

Each terminal session exports:

| Var | Meaning |
|-----|---------|
| `PW_WORKSPACE_ID` | Stable workspace id (`ws_…`) |
| `PW_WORKSPACE_NAME` | Human workspace name |
| `PW_PANE_ID` | Stable pane id (`pane_…`) |
| `PW_PANE_NAME` | Human pane name (auto: `caffeinated_turing` style) |
| `PWORKSPACES_SOCK` | Unix socket for `pwctl` |

Use skill `pworkspaces-agent` (`.claude/skills/` / `.cursor/skills/`) or:

```bash
pwctl panes "$PW_WORKSPACE_NAME"
pwctl send "$PW_WORKSPACE_NAME" <pane> "<cmd>"
pwctl read "$PW_WORKSPACE_NAME" <pane>
pwctl view README.md                  # WebView preview (md/html/url/image)
```

Args accept name **or** id.

## Dev binaries

Runtime crate (lib + `pwctl`). Desktop UI lives in `ui/egui` (swap later → `ui/<toolkit>`).

```bash
cargo build --bin pworkspaces --bin pwctl
cargo run --bin pworkspaces
# inside a pane, pwctl is on PATH (next to pworkspaces + ~/.cargo/bin)

# host shell (optional):
cargo install --path . --bin pwctl
pwctl panes "$PW_WORKSPACE_NAME"
```

Layout:

```text
src/           # runtime lib + pwctl
ui/egui/       # egui/eframe frontend (bin: pworkspaces)
```

## Runtime / attach

Un solo runtime (daemon) en `/tmp/pworkspaces.sock`. UI y `pwctl` son clientes.

```bash
pwctl start              # solo daemon (sin ventana)
cargo run --bin pworkspaces   # UI: attach; si no hay daemon, lo arranca
pwctl ping
pwctl stop               # mata daemon + PTYs
```

| Acción | Efecto |
|--------|--------|
| **✕** / Cmd+Q / **⌘⇧D** | Detach. Runtime sigue. Reabrir UI / `pwctl` = mismas sesiones. |
| `pwctl stop` / palette `kill_runtime` | Mata daemon + PTYs. |

Al reattach, el daemon reenvía ~256KB de output PTY al emulador (no pantalla vacía).

Si ping OK, abrir UI **no** borra el socket.
