# W8 VS Code adapter (parallel, non-blocking)

**Owns:** `tools/vsix-convert/`, `sidecars/vscode-host/` (Node/TypeScript).

**Deliverable**
- Converter for VS Code themes, snippets and keybindings into Fyx formats (Zed already has a theme importer).
- Node sidecar running unmodified `.vsix` with a shim for the 15–20 most used `vscode.*` APIs (commands, configuration, workspace fs, documents, diagnostics, formatting, status bar, QuickPick, Output, decorations, TreeView). Unimplemented APIs throw an explicit error.
- Speaks the same tree schema as `fyx:ui` over JSON-RPC; mock the host side.
- Run 10–20 simple Open VSX extensions; write a coverage report in `docs/vscode-coverage.md`.

**Never** touches the Rust build.
