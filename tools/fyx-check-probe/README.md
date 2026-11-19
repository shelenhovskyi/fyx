# Check Probe

A test UI extension for dev-build checks of the T3 terminal cards (`terminal:read`,
`terminal:write`), approval cards and `terminal:observe` (the user checklist's block 10). No
extension that ships with Fyx asks for T3, so this one does. It is not a demo and not a workspace
member: it has a `[workspace]` of its own, so it builds without touching the root `Cargo.toml`.

Build it for the current `fyx:ui` 0.4 world and copy the component next to the manifest:

```sh
tools/fyx-check-probe/build.sh [<out dir>]
```

`<out dir>` (default `tools/fyx-check-probe/out`) then holds `extension.toml` and `fyx_ui.wasm`;
install it with **Install Dev Extension…**. Rebuild it whenever the 0.4 WIT changes: Fyx refuses
a component built against an older copy of the world.

The panel has one command input; type a command and press Enter:

| Command | What it does |
| --- | --- |
| `help` | Lists the commands. |
| `list` | Lists the terminals as `<handle>:<origin>:<title>` (`terminal:observe`). |
| `read <n> [lines]` | Asks to read the last lines (default 40) of terminal `n` (T3 card). |
| `read-command <n>` | Asks to read the last command's output of terminal `n`. |
| `write <n> <text>` | Asks to type `text` into terminal `n`, without Enter. |
| `write-enter <n> <text>` | The same, then Enter. |
| `open` | Opens a pseudo-terminal of its own (`terminal:pseudo`). |
| `approve` | Shows an approval card for a made-up tool (`ui:approvals`). |
| `clear` | Clears the log. |
