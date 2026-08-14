# pmux

Desktop workspace runtime (tiling panes + PTY sessions + IPC). Spec: `spec.md`.

## Agent / Claude inside a pane

Each terminal session exports:

| Var | Meaning |
|-----|---------|
| `PW_WORKSPACE_ID` | Stable workspace id (`ws_…`) |
| `PW_WORKSPACE_NAME` | Human workspace name |
| `PW_PANE_ID` | Stable pane id (`pane_…`) |
| `PW_PANE_NAME` | Human pane name (auto: `caffeinated_turing` style) |
| `PMUX_SOCK` | Unix socket for `pwctl` |

Use skill `pmux-agent` (`.claude/skills/` / `.cursor/skills/`) or:

```bash
pwctl panes "$PW_WORKSPACE_NAME"
pwctl send "$PW_WORKSPACE_NAME" <pane> "<cmd>"
pwctl read "$PW_WORKSPACE_NAME" <pane>
pwctl view README.md                  # WebView preview (md/html/url/image)
```

Args accept name **or** id.

## Dev binaries

Runtime crate (`runtime/`, bin `pwctl`). Desktop UI lives in `ui/egui` (crate `pmux-ui`, bin `pmux`). Shared lib: `src/` (`pmux`).

```bash
cargo build --release          # pwctl + pmux (UI)
cargo build --release --bin pwctl   # solo runtime (WSL / sin GUI)
cargo pmux                     # run UI (alias)
# inside a pane, pwctl is on PATH (next to the bin + ~/.cargo/bin)

# host shell (optional):
cargo install --path runtime --bin pwctl
pwctl panes "$PW_WORKSPACE_NAME"
```

Layout:

```text
src/           # lib `pmux` (IPC, attach, VTE paint) — no PTY
runtime/       # daemon + PTY + pwctl
ui/egui/       # crate `pmux-ui`, bin `pmux`
```

## Runtime / attach

Un solo runtime (daemon) en `/tmp/pmux.sock`. UI y `pwctl` son clientes.

```bash
pwctl start              # solo daemon (sin ventana; no hace falta binario UI)
# LAN: pwctl start imprime token; Mac: PMUX_HOST=ip:7878 PMUX_TOKEN=… cargo pmux
cargo pmux               # UI: attach; si no hay daemon, spawnea pwctl --daemon
pwctl ping
pwctl stop               # mata daemon + PTYs
```

| Acción | Efecto |
|--------|--------|
| **✕** / Cmd+Q / **⌘⇧D** | Detach. Runtime sigue. Reabrir UI / `pwctl` = mismas sesiones. |
| `pwctl stop` / palette `kill_runtime` | Mata daemon + PTYs. |

Al reattach, el daemon reenvía ~256KB de output PTY al emulador (no pantalla vacía).

Si ping OK, abrir UI **no** borra el socket.
