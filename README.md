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
cargo build --release -p pmux-ui
```

`cargo build --release` (sin args) = client lib + `pwctl` (sin ventana).

```
src/        lib `pmux` (IPC, attach, paint) — sin PTY
runtime/    `pmux-runtime` + pwctl
ui/egui/    crate `pmux-ui`, bin `pmux`
```

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
cargo install --path runtime --bin pwctl
hash -r
```

Adentro de un pane, `pwctl` ya está en PATH (al lado del bin + `~/.cargo/bin`).

### macOS

Xcode CLT + Rust. La UI usa Menlo del sistema (`/System/Library/Fonts/Menlo.ttc`). Opcional: [JetBrainsMono Nerd Font](https://www.nerdfonts.com/font-downloads) en `~/Library/Fonts/` (iconos en la terminal).

### Linux / WSL2

Solo runtime (recomendado para attach remoto): **no hace falta fuente ni GTK.**

```bash
sudo apt install build-essential
cargo build --release --bin pwctl
./target/release/pwctl start
```

UI: GTK + WebKit **y una mono**. Sin fuente, egui cae al default y se ve mal.

```bash
sudo apt install build-essential pkg-config libgtk-3-dev libwebkit2gtk-4.1-dev \
  fonts-jetbrains-mono fonts-dejavu-core
cargo build --release -p pmux-ui
```

Orden que busca `pmux` (primera que exista):

| Path |
|------|
| `~/Library/Fonts/JetBrainsMonoNerdFont-Regular.ttf` |
| `~/.local/share/fonts/JetBrainsMonoNerdFont-Regular.ttf` (o `.otf`) |
| `/usr/share/fonts/truetype/jetbrains-mono/JetBrainsMono-Regular.ttf` |
| `/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf` |
| `/usr/share/fonts/TTF/JetBrainsMono-Regular.ttf` |
| `/System/Library/Fonts/Menlo.ttc` |

Nerd Font a mano:

```bash
mkdir -p ~/.local/share/fonts
# bajá JetBrainsMonoNerdFont-Regular.ttf de https://www.nerdfonts.com/font-downloads
cp JetBrainsMonoNerdFont-Regular.ttf ~/.local/share/fonts/
fc-cache -fv
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

### Theme

Default: **caffeine** (café muteado). Ya **no** se copia el tema de Kitty — el Monokai de `kitty.conf` era el neon.

`~/.config/pmux/pmux.toml`:

```toml
# caffeine | kitty | theme.conf | ~/path/to.conf
theme = "caffeine"
```

O `$PMUX_THEME`. Para tunear: copiá `themes/caffeine.conf` → `~/.config/pmux/theme.conf` (sintaxis Kitty: `color0`…`color15`, `foreground`, `background`, `cursor`, `active_border_color`).

```bash
cp themes/caffeine.conf ~/.config/pmux/theme.conf
# en pmux.toml:
theme = "theme.conf"
```

Para volver a Kitty: `theme = "kitty"`. Reiniciá la UI (el palette se carga al arrancar).

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

## LAN (TCP + token)

Sin SSH. Tráfico **sin TLS** — solo en red de confianza. Quien tenga el token tipea en tus shells.

En el runtime:

```bash
pwctl start            # genera token si no hay, lo imprime
# pwctl start --rotate # token nuevo (daemon tiene que estar down)
pwctl token            # mostrar
```

Default: `0.0.0.0:7878`. Off: `listen = "off"` en `pmux.toml` o `PMUX_LISTEN=off`.

En la Mac (misma LAN):

```bash
export PMUX_HOST=192.168.x.x:7878
export PMUX_TOKEN=<el token de pwctl start>
cargo pmux
# o
pwctl ping
```

Unix sock local **no** pide token.

## Remoto (SSH)

El daemon es un servidor local. Tunelás el sock.

**Importante:** reconstruí `pwctl` en el server. El UI ahora hace **un** RPC por tick (`poll_ui`); el daemon viejo no lo entiende.

```bash
# server (WSL / lab)
cargo build --release --bin pwctl
./target/release/pwctl stop
./target/release/pwctl start

# Mac — unix-forward, sin compresión (suma latencia)
rm -f /tmp/pmux-wsl.sock
ssh -N -o Compression=no -o IPQoS=throughput \
  -L /tmp/pmux-wsl.sock:/tmp/pmux.sock USER@HOST
```

Otra terminal:

```bash
export PMUX_SOCK=/tmp/pmux-wsl.sock
pwctl ping
cargo pmux
```

Sin `PMUX_SOCK` la Mac arranca daemon local. WSL2 + Windows extra hop se siente; si podés, SSH directo al Linux (IP de WSL), no al Windows.

El teclado y el PTY van en el mismo round-trip. Igual no es local: RTT de SSH ≈ delay al tipear.

## Layout del repo

```
src/            runtime (lib) + pwctl
ui/egui/        frontend egui (bin: pmux)
spec.md         modelo / principios
```

La UI no es dueña de los PTYs. Mañana otro toolkit = otro `ui/<algo>`, mismo socket.
