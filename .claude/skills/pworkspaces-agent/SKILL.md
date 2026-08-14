---
name: pworkspaces-agent
description: >-
  Operate pmux panes via pwctl using PW_* environment variables.
  Use when inside a pmux terminal, when PW_WORKSPACE_ID or PW_PANE_ID
  is set, or when the user asks to send commands to other panes, list panes,
  preview files/URLs, or control the workspace runtime.
---

# pmux agent

You are running inside a pmux pane. The runtime speaks JSON over a Unix socket; `pwctl` is the CLI.

## Discover context

```bash
echo "ws=$PW_WORKSPACE_NAME id=$PW_WORKSPACE_ID"
echo "pane=$PW_PANE_NAME id=$PW_PANE_ID"
echo "sock=${PMUX_SOCK:-${PWORKSPACES_SOCK:-/tmp/pmux.sock}}"
```

Prefer names when present; ids always work.

## Commands

```bash
pwctl panes "$PW_WORKSPACE_NAME"
pwctl send "$PW_WORKSPACE_NAME" terminal-tests "cargo test"
pwctl read "$PW_WORKSPACE_NAME" terminal-api
pwctl send "$PW_WORKSPACE_ID" "$PW_PANE_ID" "echo hi"   # id form also OK
pwctl list
pwctl status
pwctl ping                              # daemon health
pwctl view README.md                    # WebView (uses $PW_WORKSPACE_NAME)
pwctl view "$PW_WORKSPACE_NAME" spec.md
pwctl view "$PW_WORKSPACE_NAME" shot.png
pwctl view "$PW_WORKSPACE_NAME" http://localhost:5173
pwctl view "$PW_WORKSPACE_NAME" ./plugin-ui/   # directory → index.html
```

Workspace/pane args accept **name or id**.

`pwctl view` reuses the first view pane in the workspace (creates one if none). Native WebView: md, html, pdf, images, http(s).

## Rules

1. Stay inside `$PW_WORKSPACE_*` unless the user explicitly asks otherwise.
2. Prefer `send`/`read` over guessing UI state.
3. After `send`, `read` the target pane to verify output.
4. Do not `destroy` workspaces or close panes unless asked.
5. Do not `pwctl shutdown` unless asked — that kills the daemon and all PTYs.
6. Name panes you create (`--name`) so they stay addressable.
7. Preview docs/UI with `pwctl view`, not `cat` / dump markdown in a terminal.

Closing the UI detaches; daemon keeps sessions. Reopen `pmux` to reattach.

Frontend lives in `ui/egui` (runtime crate is the repo root).

## Typical layout

```text
workspace backend
├── terminal-api
├── terminal-tests
├── preview         ← WebView (`pwctl view`)
└── claude          ← you ($PW_PANE_NAME)
```

Example:

```bash
pwctl send "$PW_WORKSPACE_NAME" terminal-tests "cargo test"
sleep 1
pwctl read "$PW_WORKSPACE_NAME" terminal-tests
pwctl view "$PW_WORKSPACE_NAME" README.md
```
