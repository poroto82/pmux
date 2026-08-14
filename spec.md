Specification — Development Workspace Runtime
1. Objetivo

Construir una aplicación de escritorio nativa, multiplataforma, que proporcione un entorno de trabajo basado en workspaces, tiling y componentes.

El sistema debe combinar:

terminales;
procesos;
agentes de IA;
logs;
monitores;
puertos;
archivos;
previews;
widgets;
eventualmente WebViews.

El usuario debe poder organizar estos componentes mediante un sistema de tiling/floating inspirado en un window manager, pero contenido dentro de una aplicación.

La aplicación no debe depender de un terminal como unidad fundamental.

La unidad fundamental es:

Workspace
    └── Layout Tree
          ├── Split
          ├── Split
          └── Component
2. Principios arquitectónicos
P-01 — El terminal no es el núcleo

El terminal es un componente:

Component
├── Terminal
├── Agent
├── Logs
├── Ports
├── Processes
├── Files
└── Browser

El layout engine no debe conocer ninguno de estos tipos.

P-02 — El layout es un árbol

El layout debe representarse mediante un árbol binario.

Split
├── Component
└── Split
    ├── Component
    └── Component
P-03 — El runtime y la UI están separados

La UI no debe ser propietaria de:

procesos;
PTYs;
sesiones;
agentes.

Estos deben pertenecer al runtime.

UI
 │
 │ IPC
 ▼
Runtime
 ├── Sessions
 ├── Processes
 ├── PTYs
 └── Events
P-04 — Persistencia

Cerrar la UI no debería implicar necesariamente perder las sesiones.

Debe ser posible:

detach
   ↓
close UI
   ↓
reopen
   ↓
attach
P-05 — Native-first

La aplicación principal no debe ejecutarse sobre Chromium/Electron.

Debe utilizar:

ventana nativa;
rendering GPU;
UI nativa/custom.

WebView/Chromium puede existir posteriormente como un componente opcional.

3. Modelo conceptual

El modelo principal es:

Application
│
├── Workspaces
│
├── Runtime
│
├── Component Registry
│
└── Action Registry

Un workspace:

Workspace
│
├── Layout
├── Components
├── Sessions
└── State
4. Layout Engine
4.1 LayoutNode

Conceptualmente:

enum LayoutNode {
    Split {
        direction: Direction,
        ratio: f32,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },

    Leaf {
        component_id: ComponentId,
    },
}

Direcciones:

enum Direction {
    Horizontal,
    Vertical,
}
4.2 Operaciones

El layout engine debe proporcionar:

split_horizontal()
split_vertical()

close()

focus_left()
focus_right()
focus_up()
focus_down()

resize()

swap()

move()

fullscreen()

float()
tile()

Estas operaciones deben trabajar sobre el árbol, no sobre componentes específicos.

5. Tiling

Ejemplo:

┌──────────────────────────────┐
│ Terminal                     │
└──────────────────────────────┘

Split vertical:

┌──────────────┬───────────────┐
│ Terminal     │ Agent         │
└──────────────┴───────────────┘

Split horizontal del Agent:

┌──────────────┬───────────────┐
│              │ Agent         │
│ Terminal     ├───────────────┤
│              │ Ports         │
└──────────────┴───────────────┘

El árbol:

Horizontal
├── Terminal
└── Vertical
    ├── Agent
    └── Ports
6. Floating

El sistema debe soportar componentes floating.

Un componente puede existir fuera del árbol de tiling:

Workspace
├── Tiled Layout
│   ├── Terminal
│   └── Agent
│
└── Floating
    └── Ports

Un floating component debe tener:

x
y
width
height
z-index

Debe poder convertirse entre:

tiled ↔ floating
7. Workspaces

Debe existir soporte para múltiples workspaces.

Ejemplo:

Workspace: Backend
Workspace: Frontend
Workspace: Agents
Workspace: Monitoring

El usuario debe poder cambiar rápidamente entre ellos.

Los workspaces deben persistir.

8. Workspace Rules

Los workspaces deben permitir reglas declarativas.

Ejemplo:

workspace: backend

rules:
  - component: terminal
    position: left

  - component: agent
    position: right

  - component: ports
    position: bottom

Esto permitirá crear workspaces reproducibles.

9. Component System

Todo contenido visual debe implementar una abstracción común.

Conceptualmente:

trait Component {
    fn id(&self) -> ComponentId;

    fn update(&mut self, event: Event);

    fn render(&mut self, ctx: &mut RenderContext);

    fn input(&mut self, input: InputEvent);

    fn actions(&self) -> Vec<Action>;

    fn state(&self) -> ComponentState;
}

El sistema debe diferenciar:

Component
ComponentState
ComponentActions
ComponentEvents
10. Componentes MVP

El MVP debe incluir solamente:

Terminal
Terminal
 └── PTY
      └── shell
Logs
Logs
 ├── stream
 ├── search
 └── follow
Processes
Processes
 ├── PID
 ├── CPU
 ├── Memory
 └── actions
Ports
Ports
 ├── port
 ├── process
 └── actions
11. Terminal

No implementar terminal emulation desde cero inicialmente.

El componente Terminal debe utilizar una implementación existente para:

ANSI;
cursor;
scrollback;
selección;
colores;
Unicode;
input;
resize.

Debe conectarse al runtime mediante un PTY.

Arquitectura:

Terminal Component
       │
       ▼
   Session ID
       │
       ▼
Runtime
       │
       ▼
      PTY
       │
       ▼
    Shell
12. Sessions

Una sesión representa un proceso interactivo persistente.

Session
├── id
├── cwd
├── environment
├── process
├── PTY
└── state

Una sesión no pertenece a un pane.

Un pane referencia una sesión.

Esto es importante.

Pane A ────┐
            ├── Session X
Pane B ────┘

Esto permite que la misma sesión pueda eventualmente ser visualizada desde diferentes lugares.

13. Runtime

El runtime administra:

Sessions
Processes
PTYs
Agents
Events

Debe existir independientemente de la UI.

Inicialmente puede vivir dentro del mismo proceso.

Posteriormente debe poder convertirse en:

workspace-ui
      │
      │ IPC
      ▼
workspace-runtime

sin cambiar el modelo interno.

14. Event System

Crear un event bus central.

Eventos:

SessionCreated
SessionExited

ProcessStarted
ProcessExited

PortOpened
PortClosed

PaneCreated
PaneClosed
PaneFocused

WorkspaceCreated
WorkspaceClosed

ComponentCreated
ComponentDestroyed

AgentStarted
AgentFinished
AgentNeedsInput

Los componentes pueden suscribirse a eventos.

15. Actions

Todas las acciones deben utilizar un sistema común.

Ejemplos:

split_horizontal
split_vertical

focus_left
focus_right

close_pane

restart_process

kill_process

open_port

new_terminal

new_agent

Una acción puede invocarse desde:

keyboard shortcut
command palette
context menu
button
automation
plugin
16. Command Palette

Debe existir un command palette global.

Ejemplo:

Ctrl+Shift+P

> Split pane horizontally
  Split pane vertically
  New terminal
  New agent
  Focus left
  Focus right
  Close pane
  Toggle floating
  Restart process

La command palette debe consumir directamente el Action Registry.

No debe tener acciones hardcodeadas.

17. Keybindings

Los shortcuts deben ser configurables.

Ejemplo:

keybindings:
  split_horizontal: Ctrl+Shift+H
  split_vertical: Ctrl+Shift+V

  focus_left: Ctrl+Alt+Left
  focus_right: Ctrl+Alt+Right

  close_pane: Ctrl+Shift+W

  command_palette: Ctrl+Shift+P
18. Persistence

Persistir:

workspace
layout tree
component configuration
floating components
sessions
cwd
environment metadata
focus
active workspace

Ejemplo:

~/.config/app/
    workspaces/
        backend.json
        frontend.json
        agents.json

La persistencia debe permitir:

save
restore
rename
duplicate
delete
19. Agents

Los agentes deben ser componentes normales.

Ejemplo:

AgentComponent
      │
      ▼
   Session
      │
      ▼
Claude / Codex / other agent

El agente puede tener:

status
messages
tool calls
terminal
actions

Pero el layout engine no debe saber que existe Claude.

Para él es:

ComponentId = X
20. WebView

No formar parte del runtime principal.

Debe existir eventualmente como:

BrowserComponent

Arquitectura:

Workspace
├── Terminal
├── Agent
├── Ports
└── BrowserComponent
          │
          ▼
        WebView

Esto permite mostrar:

localhost:3000
localhost:5173
Grafana
documentation
custom web UI

sin convertir toda la aplicación en Electron.

21. Plugin Architecture

Fuera del MVP.

La arquitectura debe, sin embargo, permitirla posteriormente.

Tipos potenciales:

Native Component
Process Component
Web Component

Los plugins deben poder registrar:

Component
Action
Command
Event Handler

Pero no desarrollar todavía el sistema de instalación/distribución.

22. Tecnología

Primera opción:

Rust

Arquitectura:

Rust
│
├── UI / GPU renderer
├── Layout Engine
├── Workspace
├── Runtime
├── PTY
├── Process Manager
├── Event Bus
├── Persistence
└── IPC

Evaluar antes de comprometerse con un toolkit:

GPUI
Slint
Iced
egui

El benchmark mínimo debe probar:

native window
GPU rendering
text
scroll
keyboard
mouse
split panes
resize
multiple panes
large text buffers
23. Restricciones técnicas
No
Electron como aplicación principal.
Chromium como renderer principal.
Layout acoplado al terminal.
Terminal implementado desde cero en MVP.
Plugins inicialmente.
Agentes inicialmente.
Docker inicialmente.
Sistema distribuido inicialmente.
Sí
Rust.
GPU rendering.
native window.
layout tree.
PTY existente.
session model.
event bus.
persistence.
component abstraction.
action registry.
24. Fases de desarrollo
Fase 0 — UI Spike

Objetivo: elegir toolkit.

Construir:

Window
 └── Pane

Probar:

texto;
input;
scroll;
split;
resize;
focus;
4+ panes.

No implementar terminal todavía.

Fase 1 — Layout Engine

Implementar completamente:

Split
Leaf
Focus
Resize
Close
Swap
Move
Fullscreen
Floating

Crear tests unitarios para las operaciones sobre el árbol.

Fase 2 — Terminal

Integrar:

PTY
Terminal emulator
Shell

Resultado:

┌──────────────────┬──────────────────┐
│ Terminal         │ Terminal         │
│                  │                  │
└──────────────────┴──────────────────┘
Fase 3 — Persistence

Cerrar y recuperar:

workspace
layout
terminal sessions
cwd
Fase 4 — Components

Implementar:

Terminal
Logs
Ports
Processes

Demostrar:

┌──────────────────┬──────────────────┐
│ Terminal         │ Ports            │
├──────────────────┼──────────────────┤
│ Logs             │ Processes        │
└──────────────────┴──────────────────┘
Fase 5 — Actions

Implementar:

Action Registry
Command Palette
Keybindings
Context menus
Fase 6 — Runtime

Separar:

UI

de:

Runtime

mediante una interfaz IPC estable.

Fase 7 — Agents

Agregar:

AgentComponent

y conectar Claude/Codex/etc.

Fase 8 — WebView

Agregar:

BrowserComponent

como componente opcional.

Fase 9 — Plugins

Recién entonces diseñar:

Plugin API
Component API
Action API
Event API
25. Criterio de éxito del MVP

El MVP se considera exitoso cuando el usuario puede hacer esto:

1. Abrir aplicación.

2. Crear workspace "Project".

3. Crear terminal.

4. Dividirla horizontalmente.

5. Dividir el segundo pane verticalmente.

6. Convertir uno de los panes en Ports.

7. Crear otro Terminal.

8. Mover/focalizar panes.

9. Abrir un proceso en un terminal.

10. Ver el proceso/puerto desde los widgets.

11. Cerrar la aplicación.

12. Volver a abrirla.

13. Restaurar workspace y sesiones.

El resultado debería verse aproximadamente así:

┌──────────────────────────────────────────────────────────────┐
│ Project       Backend       Frontend       Agents       +   │
├───────────────────────────────┬──────────────────────────────┤
│                               │                              │
│ Terminal                      │ Agent                        │
│                               │                              │
│ $ npm run dev                 │ ● working                    │
│                               │                              │
├───────────────────────────────┼──────────────────────────────┤
│ Logs                          │ Ports                        │
│                               │                              │
│ > server started              │ :3000  node       ●          │
│ > connected database          │ :5432  postgres   ●          │
│                               │ :6379  redis      ○          │
└───────────────────────────────┴──────────────────────────────┘

Ese sería el producto mínimo que yo construiría.

Y la regla de oro durante el desarrollo sería:

Hyprland te da la filosofía del layout. tmux te da la persistencia de sesiones. VS Code/Claude te dan el concepto de componentes. Pero el Workspace Runtime es la pieza que los une.

Eso mantiene el proyecto enfocado y evita que termine siendo simplemente "otro terminal con 40 features".

ok te sumo algo mas, podremos enviar comando a cad pane? quizas limitar ocmunicacion dentro de un workspace ya que epor ejemplo el agente podria enviar comandos a los otros peanes, por lo que deberian tener id de cada pane y tambien de workspace para restauracion

Sí. Y eso que acabás de agregar cambia una parte importante de la arquitectura: el pane deja de ser solamente una unidad visual y pasa a ser una entidad direccionable dentro del runtime.

Eso me parece muy bueno, especialmente para agentes.

Por ejemplo:

Workspace: backend
├── Pane: terminal-api
├── Pane: terminal-tests
├── Pane: claude
└── Pane: ports

Claude podría tener permisos para hacer:

send_command(
    workspace="backend",
    pane="terminal-tests",
    command="npm test"
)

Pero no debería poder arbitrariamente tocar cualquier workspace.

Ahí introduciría un concepto nuevo: Workspace Scope.

Actualización de la especificación
26. Identidad de recursos

Todos los recursos persistentes deben tener IDs estables.

Application
│
├── WorkspaceId
│
├── PaneId
│
├── ComponentId
│
└── SessionId

Ejemplo:

Workspace
id: ws_01JABC...

Pane
id: pane_01JDEF...

Component
id: comp_01JGHI...

Session
id: sess_01JKLM...

No usar índices como identidad:

pane 0
pane 1
pane 2

porque el layout puede cambiar.

27. Workspace como namespace

Cada workspace debe actuar como un namespace lógico.

workspace backend
│
├── pane terminal-api
├── pane terminal-tests
├── pane claude
└── pane ports

Un comando dirigido debe tener siempre un contexto:

workspace_id
pane_id

Por ejemplo:

workspace = ws_backend
pane = pane_tests
command = "npm test"

Esto permite distinguir:

backend / tests

de:

frontend / tests

aunque tengan el mismo nombre.

28. Pane Address

Definir un identificador direccionable:

WorkspaceId / PaneId

Ejemplo:

ws_backend/pane_tests

El runtime debe poder resolver:

WorkspaceId
        │
        ▼
Workspace
        │
        ▼
PaneId
        │
        ▼
Pane
        │
        ▼
Component / Session
29. Pane Registry

El runtime debe mantener un registro:

struct PaneRegistry {
    panes: HashMap<PaneId, Pane>,
}

Pero el pane debe pertenecer a un workspace:

struct Pane {
    id: PaneId,
    workspace_id: WorkspaceId,
    component_id: ComponentId,
    session_id: Option<SessionId>,
}
30. Comunicación entre panes

Los panes deben poder recibir comandos mediante el runtime.

Ejemplo:

Agent
  │
  │ send_command
  ▼
Runtime
  │
  ▼
Workspace
  │
  ▼
Pane
  │
  ▼
Session
  │
  ▼
PTY

API conceptual:

send_command(
    workspace_id,
    pane_id,
    command,
)
31. Ejemplo

Tenemos:

Workspace: backend

pane_api
pane_tests
pane_claude

Claude ejecuta:

send_command(
    workspace="backend",
    pane="pane_tests",
    command="cargo test"
)

El usuario ve:

┌─────────────────────────────┬─────────────────────┐
│ API                         │ Claude              │
│                             │                     │
│ $ cargo run                 │ Running tests...    │
│                             │                     │
├─────────────────────────────┼─────────────────────┤
│ Tests                       │ Ports              │
│                             │                     │
│ $ cargo test                │ :3000 ●             │
│                             │ :5432 ●             │
└─────────────────────────────┴─────────────────────┘
32. Pero hay una distinción MUY importante

No haría:

Agent → PTY directamente

Haría:

Agent
  ↓
Intent / Action
  ↓
Authorization
  ↓
Runtime
  ↓
Pane
  ↓
Session
  ↓
PTY

Esto te permite controlar qué puede hacer un agente.

33. Permission Model

Cada componente/agente debe tener un conjunto de permisos.

Por ejemplo:

agent:
  permissions:
    panes:
      scope: workspace

    actions:
      - read
      - send_command

    processes:
      - read

    filesystem:
      scope: workspace

Entonces Claude puede:

✓ leer terminales
✓ enviar comandos
✓ leer logs
✓ consultar puertos

pero:

✗ acceder a otro workspace
✗ destruir otros workspaces
✗ ejecutar acciones administrativas

salvo que el usuario lo autorice.

34. Workspace Scope

Definir scopes:

none
pane
workspace
application

Ejemplo:

Agent A
scope = workspace

Puede acceder:

backend/*

pero no:

frontend/*
agents/*

Otro agente podría tener:

scope = pane

y solamente acceder a:

backend/pane_claude
35. Cross-workspace communication

Debe estar deshabilitada por defecto.

Para enviar algo desde:

backend/claude

a:

frontend/tests

debe existir una autorización explícita.

Conceptualmente:

PermissionDenied:
agent ws_backend/pane_claude
cannot access ws_frontend/pane_tests

Esto además es muy útil para evitar que un agente haga cosas inesperadas fuera de su proyecto.

36. Pane Actions

Un pane debería exponer acciones.

Pane
├── focus
├── close
├── split
├── resize
├── send_command
├── read_output
└── inspect

Pero no todas tienen que estar disponibles para todos.

Por ejemplo:

User
  → todas

Agent
  → read_output
  → send_command
  → inspect

Widget
  → inspect

External plugin
  → depende de permisos
37. Read vs Write

Separaría explícitamente:

read pane

de:

write pane

Y:

read_output
send_input
send_command

no deberían ser exactamente lo mismo.

Por ejemplo:

send_input

Envía bytes al PTY:

Ctrl+C
Enter
arrow keys
send_command

Es una operación de mayor nivel:

"npm test"

El runtime puede convertirla apropiadamente en input:

npm test\n

Esto es importante porque permite después implementar políticas.

38. Pane Messaging

Además de comandos, permitiría mensajes estructurados.

Ejemplo:

{
  "type": "request",
  "workspace_id": "ws_backend",
  "source_pane": "pane_claude",
  "target_pane": "pane_tests",
  "action": "run_command",
  "payload": {
    "command": "cargo test"
  }
}

El runtime valida:

source
target
scope
permissions
action

antes de ejecutar.

39. Eventos hacia agentes

El flujo también puede ir en sentido contrario.

PortOpened
      ↓
Workspace Runtime
      ↓
Agent

Claude podría recibir:

port.opened
{
    port: 3000,
    process: "node"
}

o:

process.exited
{
    pane: "pane_api",
    exit_code: 1
}

Eso permite agentes realmente reactivos.

40. Workspace Event Scope

Los eventos también deben respetar el namespace.

Un agente en:

ws_backend

por defecto recibe:

ws_backend/*

pero no:

ws_frontend/*

Esto evita que el event bus se convierta accidentalmente en un canal global.

41. Persistencia

Los IDs deben sobrevivir a la restauración.

Ejemplo:

{
  "workspace": {
    "id": "ws_backend",
    "name": "backend"
  },

  "panes": [
    {
      "id": "pane_api",
      "component": "terminal",
      "session": "sess_api"
    },
    {
      "id": "pane_tests",
      "component": "terminal",
      "session": "sess_tests"
    },
    {
      "id": "pane_claude",
      "component": "agent",
      "session": "sess_claude"
    }
  ]
}

Después de reiniciar:

ws_backend
   │
   ├── pane_api
   ├── pane_tests
   └── pane_claude

mantiene sus IDs.

42. Importante: separar Pane ID de Session ID

Esto es fundamental.

No hacer:

pane_id == session_id

Porque un pane es una entidad visual y una sesión es una entidad de ejecución.

Debe ser:

Pane
 │
 └── session_id

y potencialmente:

Pane A ──┐
         ├── Session X
Pane B ──┘

Además:

Session X

podría existir sin estar actualmente visible.

43. Esto habilita algo muy potente

Podrías tener un agente:

Claude

viendo:

workspace/backend

y haciendo:

read_output(pane_api)

send_command(pane_tests, "cargo test")

read_output(pane_tests)

send_command(pane_api, "cargo run")

Mientras la UI simplemente visualiza todo.

El agente no necesita "hacer click" en la interfaz.

Opera contra el runtime.

Esto es mucho más robusto que automatizar la UI.

44. API conceptual del Runtime

Yo apuntaría eventualmente a algo de este estilo:

Workspace
├── list()
├── create()
├── destroy()
└── restore()

Pane
├── create()
├── split()
├── close()
├── move()
├── resize()
├── focus()
├── read()
└── send()

Session
├── create()
├── attach()
├── detach()
├── restart()
└── terminate()

Process
├── list()
├── inspect()
├── kill()
└── restart()

Event
├── subscribe()
└── unsubscribe()

Y por encima:

Authorization
45. Nueva arquitectura completa

Con esta modificación, yo dejaría la arquitectura así:

                         APPLICATION
                              │
                    ┌─────────▼─────────┐
                    │    Workspace UI    │
                    │                    │
                    │ Layout             │
                    │ Components         │
                    │ Input              │
                    └─────────┬──────────┘
                              │
                             IPC
                              │
                    ┌─────────▼──────────┐
                    │  WORKSPACE RUNTIME │
                    │                    │
                    │ Workspace Registry │
                    │ Pane Registry      │
                    │ Session Registry   │
                    │ Process Manager    │
                    │ PTY Manager        │
                    │ Event Bus          │
                    │ Action Registry    │
                    │ Authorization      │
                    └─────────┬──────────┘
                              │
              ┌───────────────┼────────────────┐
              │               │                │
              ▼               ▼                ▼
             PTY           Processes         Agents
              │                                │
              ▼                                │
           Shell                               │
                                               │
                         ┌─────────────────────┘
                         │
                         ▼
                    Actions / Events

Y hay una idea que yo agregaría explícitamente a la spec:

El Workspace Runtime es una API local direccionable. La UI es solamente uno de sus clientes.

Eso significa que en el futuro podrías tener:

Desktop UI ───────┐
                  │
CLI ──────────────┤
                  ├──> Workspace Runtime
Agent ────────────┤
                  │
Remote UI ────────┘

Y ahí la cosa empieza a ponerse realmente interesante: el mismo runtime que mantiene tus sesiones puede ser utilizado por un agente para operar sobre un workspace sin necesidad de controlar la interfaz gráfica.

Eso, para mí, es una de las partes más fuertes de la idea.

ok entocnes, russt para la ventana mas ls lib graficas y el workspace runtime es lo que desarrollariamos en si?

Exactamente. Y haría una pequeña corrección conceptual:

Rust no es "el proyecto" y el toolkit gráfico tampoco. Son la infraestructura sobre la que construís el producto. El verdadero producto es el Workspace Runtime + modelo de composición + sistema de componentes.

Yo lo separaría así:

┌──────────────────────────────────────────────────────┐
│                    Desktop App                       │
│                                                      │
│   Rust                                               │
│   ├── Window / GPU / UI toolkit                      │
│   ├── Layout Renderer                                │
│   └── Input                                           │
│                                                      │
├──────────────────────────────────────────────────────┤
│              WORKSPACE RUNTIME                       │
│              ← producto principal                    │
│                                                      │
│   ├── Workspaces                                     │
│   ├── Pane Registry                                  │
│   ├── Layout Engine                                  │
│   ├── Sessions                                       │
│   ├── PTYs                                           │
│   ├── Processes                                      │
│   ├── Components                                     │
│   ├── Actions                                        │
│   ├── Events                                         │
│   ├── Permissions                                    │
│   └── Persistence                                    │
│                                                      │
├──────────────────────────────────────────────────────┤
│                    OS                                │
│                                                      │
│   processes / PTY / filesystem / networking          │
└──────────────────────────────────────────────────────┘
La UI sería un cliente del Runtime

Esto es particularmente importante.

No quiero que termines con:

Button → Terminal → Process → Workspace → State

todo mezclado.

Quiero:

                   ┌───────────────┐
                   │   Desktop UI  │
                   └───────┬───────┘
                           │
                          API
                           │
                   ┌───────▼───────┐
                   │    Runtime    │
                   └───────┬───────┘
                           │
              ┌────────────┼────────────┐
              ▼            ▼            ▼
           Session      Process       Event
              │
              ▼
             PTY

Entonces incluso podrías tener mañana:

                  Workspace Runtime
                  /       |       \
                 /        |        \
            Desktop       CLI       Agent
               UI

Los tres pueden operar sobre el mismo workspace.

Y ahí está la parte interesante

Podrías eventualmente tener un CLI:

workspace list
workspace switch backend

workspace panes
workspace pane send pane_tests "cargo test"

workspace pane read pane_api

Y el agente podría utilizar exactamente la misma API.

No necesitás enseñarle a Claude:

"hacé click acá, escribí esto en este terminal..."

Sino:

send_command(
    workspace_id,
    pane_id,
    "cargo test"
)

El runtime se encarga.

El stack quedaría aproximadamente

Yo lo pensaría en 4 capas:

1. OS Layer

No la desarrollamos.

Windows
macOS
Linux

Proporciona:

ventanas;
procesos;
PTY;
filesystem;
networking.
2. UI Infrastructure

Principalmente aprovechamos librerías.

Rust
+
GUI toolkit
+
GPU renderer
+
terminal renderer

Acá no queremos reinventar la rueda.

3. Workspace Runtime

Esto sí es nuestro.

Workspace
Pane
Layout
Session
Process
PTY
Component
Action
Event
Permission
Persistence

Esta es la parte que tiene identidad propia.

4. Components

También nuestro, pero construidos sobre el runtime:

Terminal
Agent
Logs
Ports
Processes
Files
Docker
Browser
Git
...
Y yo haría otra separación

No llamaría al proyecto simplemente "terminal".

Porque mentalmente te va a arrastrar hacia:

terminal emulator + panes + widgets

En cambio:

Workspace Runtime

te obliga a pensar correctamente.

El terminal sería simplemente:

TerminalComponent

igual que:

AgentComponent
PortsComponent
LogsComponent
El primer prototipo ni siquiera necesita verse lindo

De hecho, yo haría el primer objetivo:

Rust
│
├── Workspace Runtime
│
├── Layout Engine
│
├── Pane Registry
│
└── Fake UI

Y probaría:

create workspace
       ↓
create pane
       ↓
split pane
       ↓
assign component
       ↓
create session
       ↓
send command
       ↓
receive output
       ↓
save
       ↓
restore

Sin Claude, sin Chromium, sin Docker, sin plugins.

Si eso funciona bien, tenés el corazón del sistema.

Después la UI simplemente se convierte en:

Runtime state
      ↓
Layout
      ↓
Render

Y eso también te permite probar el runtime automáticamente sin depender de una GUI.

En una frase

Sí:

Rust + toolkit gráfico = infraestructura.

Workspace Runtime + Layout/Pane model + Sessions + Components + Actions + Events + Permissions = el producto que realmente estamos construyendo.
