# pmux

Workspaces con tiling, PTYs y un runtime que no se muere cuando cerrás la ventana.

Como tmux: el daemon es dueño de las sesiones. La UI es un cliente. `pwctl` también.

```
  pmux (UI)  ─┐
  pwctl      ─┼─►  /tmp/pmux.sock  ─►  daemon  ─►  PTYs / shells
  agente     ─┘
```

✕ o ⌘⇧D = detach. Las shells siguen. `pwctl stop` mata el runtime.

## Build

Hace falta [Rust](https://rustup.rs) (stable). **Runtime y UI se buildean aparte.**

```bash
# runtime (daemon + pwctl) — WSL / server, sin GUI
cargo build --release --bin pwctl

# UI (ventana) — máquina con display
cargo build --release -p pmux
```

`cargo build --release` (sin args) = solo runtime.

Bins:

```
target/release/pwctl    # CLI + daemon (`pwctl start` / `pwctl --daemon`)
target/release/pmux     # ventana; si no hay daemon, spawnea pwctl
```

Debug:

```bash
cargo run --bin pwctl -- start
cargo pmux                 # alias → run UI
```

`pwctl` en PATH (host):

```bash
cargo install --path . --bin pwctl
hash -r
```

Adentro de un pane, `pwctl` ya está en PATH (al lado del bin + `~/.cargo/bin`).

### macOS

Xcode CLT + Rust. Nada más. UI y runtime.

### Linux / WSL2

Solo runtime (recomendado para attach remoto):

```bash
sudo apt install build-essential
cargo build --release --bin pwctl
./target/release/pwctl start
```

UI en Linux (opcional): GTK + WebKit.

```bash
sudo apt install build-essential pkg-config libgtk-3-dev libwebkit2gtk-4.1-dev
cargo build --release -p pmux
```

### Windows

Todavía no. El IPC es Unix socket.

## Uso

Un comando:

```bash
./target/release/pmux
# o
cargo pmux
```

Si no hay daemon, `pmux` spawnea `pwctl --daemon` y se attacha.

| Gesto | Efecto |
|--------|--------|
| ✕ / Cmd+Q / **⌘⇧D** | Detach. Runtime vivo. |
| `pwctl stop` / palette `kill_runtime` | Mata daemon + PTYs. |
| Reabrir `pmux` | Mismas sesiones + ~256KB de replay. |

Solo runtime, sin ventana:

```bash
pwctl start
pwctl ping
pwctl list
pwctl stop
```

Socket: `/tmp/pmux.sock` (override `$PMUX_SOCK`). Layout: `~/.config/pmux/`.

### Teclas

En Mac, Ctrl de esta tabla es **⌘**.

| Atajo | Qué |
|--------|-----|
| ⌘⇧P | Palette |
| ⌘P | Quick open (preview) |
| ⌘⇧N | Terminal nueva |
| ⌘⇧H / ⌘⇧V | Split |
| ⌘⇧W | Cerrar pane |
| ⌘⇧F | Fullscreen pane |
| ⌘⇧G | Float / tile |
| ⌘⇧M | Chrome |
| ⌘⇧R | Refresh (rehidratar PTY desde replay) |
| ⌘⇧D | Detach |
| Ctrl+Tab | Workspace siguiente |
| Ctrl+[ / ] | Foco entre panes |

Click en un tab de workspace para cambiar. Doble click para renombrar. Arrastrar texto en la terminal copia al soltar.

### `pwctl`

Args: nombre o id.

```bash
pwctl list
pwctl panes "$PW_WORKSPACE_NAME"
pwctl send "$PW_WORKSPACE_NAME" <pane> "cargo test"
pwctl read "$PW_WORKSPACE_NAME" <pane>
pwctl view README.md
pwctl view spec.md
```

En un pane el runtime exporta `PW_WORKSPACE_ID`, `PW_WORKSPACE_NAME`, `PW_PANE_ID`, `PW_PANE_NAME`, `PMUX_SOCK`.

## Remoto (SSH)

El daemon ya es un servidor local. Tunelás el sock:

```bash
# en el server (WSL / lab): solo runtime
pwctl start
ssh -N -L /tmp/pmux-lab.sock:/tmp/pmux.sock user@lab
PMUX_SOCK=/tmp/pmux-lab.sock pmux
```

Sin `PMUX_SOCK` la laptop arranca un daemon **local**. Dos UIs al mismo runtime se pisan el output en vivo (un buffer); attach de a una, o ⌘⇧R para resync.

## Layout del repo

```
src/            runtime (lib) + pwctl
ui/egui/        frontend egui (bin: pmux)
spec.md         modelo / principios
```

La UI no es dueña de los PTYs. Mañana otro toolkit = otro `ui/<algo>`, mismo socket.
