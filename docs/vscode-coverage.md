# VS Code extension coverage (W8 prototype)

What happens when unmodified VS Code extensions from Open VSX run in the Fyx sidecar
(`sidecars/vscode-host`) and through the converter (`tools/vsix-convert`). Measured on 2026-11-17 on
Windows 11 with Node 24.18, against a mocked host (`src/mock-host.ts`) that records every JSON-RPC
message and checks every `fyx:ui` node and patch against the schema.

## Summary

- **21 extensions** (all MIT, verified Open VSX namespaces, pinned in
  `sidecars/vscode-host/extensions.json`). 18 ship code, 3 are purely declarative; 2 of the code
  ones (One Dark Pro, IntelliJ keybindings) also contribute themes or keybindings.
- **14 of 18 code extensions activate** in the sandboxed sidecar. The 4 that do not: 2 stop at an
  API Fyx does not implement (`SemanticTokensLegend`, `registerDocumentLinkProvider`; REST Client
  also stops at `registerCodeLensProvider` after registering its commands), 1 needs to spawn its
  language server, which the sandbox forbids.
- **70 of 135 contributed commands** run without error (10 more skipped because they send network
  requests). Most failures are one cause each: todo-tree needs VS Code's bundled ripgrep (31
  commands), rainbow-csv and comment-anchors never finish activating (29), webviews (4), a file
  dialog (1).
- **75 distinct `vscode.*` APIs** were touched; **0 schema violations** in 1,606 recorded messages.
- With missing APIs stubbed (probe mode), **17 of 18 activate**, and the complete list of what is
  missing is short: webviews (5 extensions), language features beyond completion/hover/formatting
  (document links, symbols, code lens, definitions, references, semantic tokens, inlay hints), and
  `showOpenDialog`.
- **Converter**: 16 themes in 3 families, 220 snippets in 5 Zed scopes, 76 keybindings from 4 extensions.

The shim covers 92 of the 220 functions, values and events in the VS Code namespaces
(`commands` 4/4, `env` 19/21, `extensions` 3/3, `l10n` 3/3, `window` 27/57, `workspace` 24/47,
`languages` 12/40; none of `debug`, `tasks`, `scm`, `notebooks`, `tests`, `authentication`, `lm`,
`chat`, `comments`) and 62 of 188 classes and enums. Everything else throws
`<api> is not implemented in Fyx` (JSON-RPC error `-32001`, `data.api` names the API).

## Results

| Extension | What it does | Result in Fyx |
|---|---|---|
| wmaurer.change-case | text case commands | **works**: 17/17 commands edit the buffer through `vscode/applyEdit` |
| Tyriar.sort-lines | sort/unique/shuffle selected lines | **works**: 14/14 commands |
| oderwat.indent-rainbow | colours indentation | **activates, decorates**, but its colours are raw `rgba()` values, which `fyx:ui` drops (theme tokens only), so the host gets ranges with no colour |
| wayou.vscode-todo-highlight | highlights TODO/FIXME | **works**: decorations, status item, QuickPick listing; raw colours dropped as above |
| shardulm94.trailing-spaces | highlights and deletes trailing whitespace | **works**: decoration, delete commands, status bar messages |
| mkxml.vscode-filesize | file size in the status bar | **works**: status item shows "69 bytes", details go to its output channel |
| EditorConfig.EditorConfig | applies .editorconfig | **works**: sets `tabSize: 2, insertSpaces: true` on the editor from the workspace's `.editorconfig` |
| christian-kohler.path-intellisense | path completion | **works**: 14 completions for `'./` in a JS file |
| formulahendry.auto-rename-tag | renames paired HTML tags | **blocked by the sandbox**: spawns its language server (`child_process`) |
| usernamehw.errorlens | inline diagnostics | **works**: decorates the host-provided diagnostic; 17/17 commands |
| mechatroner.rainbow-csv | CSV columns, RBQL | **missing API**: `SemanticTokensLegend` at activation; also needs semantic tokens, document symbols, inlay hints, a webview |
| ExodiusStudios.comment-anchors | anchor tree views | **missing API**: `registerDocumentLinkProvider`; its 3 tree views mount and show placeholder rows |
| Gruntfuggly.todo-tree | TODO tree view | **environment**: looks for VS Code's bundled ripgrep under `env.appRoot`, shows an error and registers no commands; its tree view mounts |
| adpyke.codesnap | code screenshots | **webview only**: `createWebviewPanel` |
| humao.rest-client | HTTP requests from `.http` files | **partial**: registers commands, completion (52 items in an `.http` file) and hover, then stops at `registerCodeLensProvider`; network commands not run |
| jock.svg | SVG preview, minify | **partial**: 3/6 commands; preview needs a webview, its language server cannot start in the sandbox |
| zhuangtongfa.material-theme (One Dark Pro) | theme + settings commands | **works** apart from its changelog webview; writing back into its own theme files is refused (extension folder is read-only) |
| k--kato.intellij-idea-keybindings | keymap + import command | keymap converts; the import command needs `showOpenDialog` |
| xabikos.JavaScriptSnippets | snippets | **converted**: 220 snippets into `javascript`, `typescript`, `tsx`, `html`, `vue.js` |
| dracula-theme.theme-dracula | 2 themes | **converted** |
| GitHub.github-vscode-theme | 9 themes (light and dark) | **converted**, appearances correct |

Per-extension details (every API used, effects the host saw, each command's outcome, RPC traffic)
are in the generated section at the end.

## Findings

### Sandboxing with Node's permission model

Each extension runs in its own sidecar process with `--permission`, read access to the extension
and its sandbox folder, and write access to the sandbox folder only. What that looks like in
practice:

1. **It blocks what it should.** Language-server extensions (auto-rename-tag, svg) cannot spawn
   their server; writes to the extension's own folder (One Dark Pro) and outside the workspace fail
   with `NoPermissions`. Granting `--allow-child-process` would let the child run unrestricted, so
   it is not an option; language servers should go through Zed's own LSP support instead.
2. **Node 24 has no network permission.** An extension can open sockets freely. The coverage runner
   therefore skips REST Client's request commands. A real deployment needs an OS-level sandbox or a
   proxy for network policy.
3. **Compatibility shims were needed** (all in `src/main.ts` / `src/mock-host.ts`, none of them
   widen access):
   - `fs.existsSync` throws `ERR_ACCESS_DENIED` for ungranted paths although it is documented never
     to throw. REST Client probes `~/.rest-client` at load time. The sidecar restores the contract
     (inaccessible means "does not exist").
   - `process.binding` is denied; bundled applicationinsights calls `process.binding("natives")`
     at load time to list builtin modules. The sidecar answers that single call from
     `module.builtinModules`.
   - Each extension gets a private `HOME`/`USERPROFILE` inside its sandbox, so caches under `~`
     land in the grant.
   - Under `--permission`, `existsSync("")` resolves against the cwd, so the sidecar's cwd must be
     a granted folder.
   - **Windows:** grants compare paths case-sensitively, drive letter included, while VS Code's
     `Uri.fsPath` lowercases the drive (`c:\…`). Every grant is passed in both spellings.

### Schema gaps (`fyx:ui@0.1.0`)

These were worked around without touching the WIT; each is a candidate for the integrator:

| Gap | Workaround in the prototype | Suggested change |
|---|---|---|
| No modal surface kind (QuickPick, InputBox) | `panel` at `bottom`, announced at runtime | `surface-kind::modal` |
| Surfaces are declared once (`surfaces()`), but VS Code creates QuickPicks and output channels at runtime | extra notification `fyx/surfacesChanged {added, removed}`; status items and output are folded into static per-extension surfaces | a host import such as `add-surface` / `remove-surface` |
| No way to say "list rows changed, row count did not" | the list node is re-keyed (remove + insert under a new id) so the host drops cached rows; compatible with W7's integration-1 rule that the host re-requests rows after any update touching a list | a patch `invalidate-rows(node-id)` |
| No cancel event (Escape on a QuickPick) | extra request `fyx/surfaceClosed` | `ui-event::dismissed` |
| No indent for tree rows | flex `padding` = depth × 12 (also pads vertically) | an `indent` field on flex or list rows |
| No tooltips | status item tooltips are dropped | `tooltip: option<string>` on button |
| Colours are tokens only | `ThemeColor` ids map to the nearest token; raw CSS colours (indent-rainbow, todo-highlight) are dropped and reported | keep as is for `fyx:ui`; decorations (layer 4, W6) may need a palette index or a raw-colour escape hatch |

The JSON mirror itself (encoding rules below) needed no change, and it matches the serde derive W2
put on the `ui_ext` types. The mock host also enforces W2's host rules (`ui_ext::Tree`) and refuses
what the host refuses, atomically: only `flex` nodes have children, `before` names a sibling, at
most 2,048 patches per update and 2,048 nodes per list row, and at most 16,384 nodes per tree, which
also caps a `mount` and a `replace`. An invalid `mount` or `list-rows` reply fails the whole call,
as it does in the host.

### API gaps, by how many extensions they block

| API | Extensions | Note |
|---|---|---|
| `window.createWebviewPanel` | 5 (codesnap, svg, One Dark Pro, rainbow-csv, comment-anchors) | by design out of scope (research doc: webviews are the expensive escape hatch); MCP-Apps-style `ui:web` later |
| `languages.registerDocumentLinkProvider` | 2 | cheap to add next to completion/hover |
| `languages.registerDocumentSymbolProvider` | 2 | maps to Zed's outline |
| `languages.registerCodeLensProvider`, `registerDefinitionProvider`, `registerReferenceProvider` | 1 each | |
| semantic tokens (`SemanticTokensLegend`, `registerDocumentRangeSemanticTokensProvider`), `registerInlayHintsProvider` | 1 | |
| `window.showOpenDialog` | 1 | host file picker |
| VS Code's bundled ripgrep under `env.appRoot` | 1 (todo-tree) | environment, not an API |

### Converter

- Themes follow `crates/theme_importer`'s mapping; differences (appearance detection, `include`,
  `tokenColors` file references, colour normalization) are listed in
  `tools/vsix-convert/README.md`. Spot check: Dracula's background `#282a36`, keyword `#ff79c6`,
  string `#f1fa8c`, comment `#6272a4` come through as expected.
- Snippets: Zed reads the VS Code format, but drops any snippet using variables; the converter
  rewrote `${TM_FILENAME_BASE}` in JavaScriptSnippets' `exportDefaultFunction` into a placeholder.
  `html` and `vue` snippets need the matching Zed language extensions.
- Keybindings: IntelliJ's 220 entries give 63 Zed bindings. The 136 skipped entries mostly name
  VS Code commands without a Zed action (`editor.action.codeAction` ×9, debug, tasks, terminal
  focus), and 50 `when` clauses lost a condition Zed contexts cannot express
  (`editorHasCodeActionsProvider`, `editorReadonly`, `config.*`), which makes those bindings
  broader. Extension commands map to `fyx_vscode::RunCommand`, an action the host does not have
  yet.

### Shim bugs the run found (fixed, with tests)

Configuration properties without `default` must default by type (`array` → `[]`; todo-highlight
crashed). `setDecorations` must accept `hoverMessage: null` (indent-rainbow). Selections from the
host must be clamped to the document. Assigning `editor.options` and `editor.selections` must
reach the host (EditorConfig, change-case). An extension's own command that never got registered
must fail with "command not found", not fall through to the host. `languages.getDiagnostics` must
include the host's diagnostics (errorlens).

### Partial APIs (after the W8b review fixes)

Nothing in the shim is a silent no-op any more: an API either works as listed here, throws
`NotImplementedInFyx` (JSON-RPC `-32001` with `data.api`), or is a presentation hint that is
accepted and recorded in the coverage report's `partial` notes.

**Work now** (were no-ops):

| API | What happens |
|---|---|
| `workspace.createFileSystemWatcher` | fires for `vscode/didChangeWatchedFiles` from the host, filtered by glob (string or `RelativePattern`) and the three ignore flags; `dispose` stops it |
| `languages.setLanguageConfiguration` | `wordPattern` drives `TextDocument.getWordRangeAtPosition`; the whole configuration goes to the host as `vscode/setLanguageConfiguration` (`null` on dispose) |
| `window.registerTreeDataProvider().dispose()`, `TreeView.dispose()` | removes the provider; the view shows "No data provider" |
| `TreeView.title`, `description`, `badge` | re-render as a header row (`fyx:ui` cannot rename a surface, so a changed title is a bold header, not the panel title) |
| TreeView rows | at most 10,000 visible rows, counted across all levels; past that a muted line says the view is cut off, and the coverage report notes it. Rows deeper than 5,461 levels keep the deepest indent, because flex `padding` is a `u16` and one out-of-range row would fail the host's whole `list-rows` reply |
| `TreeView.reveal` | expands the element's ancestors (needs `getParent`, as in VS Code) and `expand` levels up to 3, then selects it. It cannot scroll the row into view or take focus: `fyx:ui` has no scroll-to-row |
| `QuickPick`/`InputBox` `buttons` | rendered as buttons; a click fires `onDidTriggerButton` |
| `QuickPick`/`InputBox` `title`, `step`, `totalSteps`, `busy` after `show()` | re-render |
| `QuickPick`/`InputBox` `enabled` | `false` ignores accept (Enter, row clicks) |
| `InputBox.validationMessage`, `prompt` | re-render |
| `window.withProgress` cancellation | `vscode/cancelProgress {id}` from the host cancels the task's token |

**Throw `NotImplementedInFyx`:**

| API | Why |
|---|---|
| `InputBox.password = true`, `showInputBox({password: true})` | "masked input (password: true) is not supported in Fyx 0.1": a plain `input` node would show the secret. Masking is a 0.2 candidate for the `input` props |
| `TreeView.onDidChangeCheckboxState` | tree item checkboxes are not rendered |
| `QuickPick.onDidTriggerItemButton` | item buttons would sit inside list rows, whose node ids are row-local, so a click could not name its row |
| every API in the VS Code catalog the shim lacks | as before |

Probe mode (missing APIs become inert stubs) now needs `main.ts --fyx-coverage-probe`, which only
the coverage runner passes (`MockHost.spawn({ probe: true })`). `FYX_VSCODE_PROBE` in the
environment is ignored with a warning, so a launcher cannot turn it on by accident.

**Accepted hints with no visible effect** (each recorded in the coverage report):
`StatusBarItem.tooltip` and `color`, raw colours in decoration types, `QuickPick.keepScrollPosition`,
`QuickPick.sortByLabel = false`, `QuickPickItem.buttons`, `InputBox.valueSelection`, and
`TreeView.reveal`'s `focus`. `QuickPick.ignoreFocusOut` needs nothing: the panel never closes on
focus loss.

**Events for values that are fixed in Fyx 0.1** never fire, because the value never changes:
`window.onDidChangeWindowState`, `onDidChangeActiveColorTheme`, `workspace.onDidGrantWorkspaceTrust`
(the workspace is always trusted), `env.onDidChangeTelemetryEnabled`, `onDidChangeShell`,
`onDidChangeLogLevel`, `extensions.onDidChange`.

### Known limits of this run

- The host is mocked: no rendering, no real buffers, no LSP. "Works" means the extension did what
  it does in VS Code as far as the recorded traffic shows (edits, decorations, status items,
  completions), not that a user saw it.
- Commands run once each with a fixed editor state (a JS file, lines 1–4 selected), so commands
  that need a specific file type or a response (REST Client's save/fold) succeed trivially.
- change-case reports an uncaught `Position` error: it splits a multi-line selection by `os.EOL`
  (`\r\n` on Windows) on an LF document, computes a negative column, and VS Code's `Position`
  throws the same way. Not a shim bug.
- `workspace.createFileSystemWatcher` and `setLanguageConfiguration` depended on host traffic the
  mock did not send during this run (see "Partial APIs" below for what they do now).

## Protocol

JSON-RPC 2.0, one JSON message per line on stdin/stdout. Requests can go both ways.

### `fyx:ui` (the WIT world, as JSON)

| Direction | Method | Params → result |
|---|---|---|
| host → sidecar | `initialize` | `{extensionPath, workspaceFolders?, settings?, storageDir?}` → `{extension, contributes, surfaces}` |
| host → sidecar | `initialized` | runs `*`, `onStartupFinished`, `workspaceContains:` activation |
| host → sidecar | `surfaces` | → `surface[]` (WIT `surfaces`) |
| host → sidecar | `mount` | `{"surface-id"}` → `node[]` (WIT `mount`) |
| host → sidecar | `handle-event` | `{"surface-id", event: ui-event}` → `update` (WIT `handle-event`) |
| host → sidecar | `list-rows` | `{"surface-id", "list-node", start, count}` → `list-row[]` |
| sidecar → host | `push-update` | `{"surface-id", update}` → `null`, or an error when the host refuses the update (WIT `host.push-update`, coalesced per tick). A request, not a notification: at most one per surface is in flight, and after a refusal the sidecar's next update for that surface is a full `replace`. Diffs above the 2048-patch cap are sent as `replace` too. |
| sidecar → host | `log` | `{level, message}` (WIT `host.log`; extension `console.*` goes here) |
| sidecar → host | `fyx/surfacesChanged` | `{added: surface[], removed: string[]}` (not in the WIT, see gaps) |
| host → sidecar | `fyx/surfaceClosed` | `{"surface-id"}` (not in the WIT) |
| host → sidecar | `shutdown`, `exit`; `fyx/coverage` (test harness) | |

Encoding of WIT values (the serde default for `#[serde(rename_all = "kebab-case")]`): records are
objects with the WIT field names (`row-count`); enums are kebab-case strings (`text-muted`);
`option<T>` is `null` or the value; variants are externally tagged (`{"flex": {...}}`, a case
without payload is the bare string `"unchanged"`); tuples are arrays; `result<T, string>` is the
JSON-RPC result or an error with code `-32000` and the string as `message`.

Surfaces per extension: `<id>/status-left` and `<id>/status-right` (`status-item`, all status bar
items of one side as buttons in a row), `<id>/output` (bottom panel, one text block per output
channel), `<id>/view/<viewId>` per contributed tree view (panel; expandable rows in a virtualized
`list`), and transient `<id>/quick-pick/<n>` / `<id>/input-box/<n>`.

### VS Code services (`vscode/*`)

| Direction | Method | Purpose |
|---|---|---|
| host → sidecar | `vscode/didOpenTextDocument`, `didChangeTextDocument` (full text + version), `didCloseTextDocument`, `didSaveTextDocument`, `willSaveTextDocument` (→ edits) | document sync |
| host → sidecar | `vscode/didChangeActiveEditor`, `didChangeSelection` | editor state |
| host → sidecar | `vscode/didChangeConfiguration`, `didChangeDiagnostics` | settings, host diagnostics |
| host → sidecar | `vscode/executeCommand` | run an extension command |
| host → sidecar | `vscode/provideFormatting` (document or range), `provideOnTypeFormatting`, `provideCompletion`, `provideHover` | language features (LSP-shaped JSON) |
| sidecar → host | `vscode/registerCommand`, `unregisterCommand`, `registerProvider`, `unregisterProvider` | what the extension offers |
| sidecar → host | `vscode/executeCommand` | built-in commands the sidecar does not handle |
| sidecar → host | `vscode/applyEdit` (→ bool), `showTextDocument`, `saveTextDocument`, `saveAll`, `insertSnippet`, `setSelections`, `setEditorOptions`, `revealRange` | editor actions |
| sidecar → host | `vscode/publishDiagnostics`, `createDecorationType`, `setDecorations`, `disposeDecorationType` | diagnostics, decorations (layer 4) |
| sidecar → host | `vscode/showMessage` (→ chosen item), `progress`, `setContext`, `updateConfiguration`, `clipboard/readText`, `clipboard/writeText`, `openExternal` | window services |
| host → sidecar | `vscode/cancelProgress` `{id}` | the user cancelled a `cancellable` progress (`id` from `vscode/progress` start) |
| host → sidecar | `vscode/didChangeWatchedFiles` `{changes: [{uri, type}]}` (`type`: 1 created, 2 changed, 3 deleted, as LSP) | file events for `createFileSystemWatcher`; the host watches the workspace, the sidecar filters per watcher |
| sidecar → host | `vscode/setLanguageConfiguration` `{language, configuration}` (RegExps as `{regexp, flags}`; `configuration: null` when disposed) | comments, brackets, indentation and on-enter rules for the host to apply |

`vscode/applyEdit` carries `{edits: [{uri, version, edits}], fileOperations?}`. The host applies it
all or nothing and answers `true`, or `false` when it applied nothing, including when a `version`
is not the buffer's current one. The sidecar changes its copy of a document only after `true`, at
`version + 1`; if a `didChangeTextDocument` arrived before the answer, that text already holds the
edit. Versions come from the host: a full-text sync is dropped only when its version is older than
the sidecar's.

Error codes: `-32000` extension error (WIT `err`), `-32001` not implemented in Fyx, plus the
JSON-RPC standard codes.

## Reproducing

```bash
cd sidecars/vscode-host
npm install
npm run fetch      # downloads into vsix/ (gitignored), checks SHA-256 and license
npm run coverage   # ~2 minutes: two passes per extension (strict, then probe)
```

The runner writes `.cache/coverage/<id>.json`, `<id>.traffic.jsonl` (every RPC message) and
regenerates the section below.

## Per-extension details

<!-- BEGIN GENERATED: npm run coverage -->

| Extension | Converter | Activated | APIs used | Not implemented | Commands ok/run | Probe: all missing APIs |
|---|---|---|---|---|---|---|
| wmaurer.change-case@1.0.0 | nothing to convert, 0 warning(s) | yes | 5 | none | 17/17 | none |
| Tyriar.sort-lines@1.12.0 | 1 keybinding(s), 0 warning(s) | yes | 5 | none | 14/14 | none |
| oderwat.indent-rainbow@8.3.1 | nothing to convert, 0 warning(s) | yes | 8 | none | 0/0 | none |
| wayou.vscode-todo-highlight@1.0.5 | nothing to convert, 0 warning(s) | yes | 18 | none | 2/2 | none |
| shardulm94.trailing-spaces@0.4.4 | nothing to convert, 0 warning(s) | yes | 12 | none | 3/3 | none |
| mkxml.vscode-filesize@3.2.2 | 1 keybinding(s), 0 warning(s) | yes | 9 | none | 1/1 | none |
| EditorConfig.EditorConfig@0.18.2 | nothing to convert, 0 warning(s) | yes | 16 | none | 1/1 | none |
| christian-kohler.path-intellisense@2.8.0 | nothing to convert, 0 warning(s) | yes | 12 | none | 0/0 | none |
| formulahendry.auto-rename-tag@0.1.10 | nothing to convert, 0 warning(s) | **no**: Error: Access to this API has been restricted. Use --allow-child-process to manage permissions. (at ChildProcess.spawn ( | 21 | none | 0/0 | none |
| usernamehw.errorlens@3.30.0 | nothing to convert, 0 warning(s) | yes | 21 | none | 17/17 | none |
| mechatroner.rainbow-csv@3.24.1 | nothing to convert, 0 warning(s) | **no**: vscode.SemanticTokensLegend is not implemented in Fyx | 0 | `vscode.SemanticTokensLegend` | 0/24 | `languages.registerDocumentRangeSemanticTokensProvider`, `languages.registerDocumentSymbolProvider`, `languages.registerInlayHintsProvider`, `SemanticTokensLegend`, `window.createWebviewPanel` |
| ExodiusStudios.comment-anchors@1.9.6 | nothing to convert, 0 warning(s) | **no**: vscode.languages.registerDocumentLinkProvider is not implemented in Fyx | 14 | `vscode.languages.registerDocumentLinkProvider` | 0/5 | `languages.registerDocumentLinkProvider`, `window.createWebviewPanel` |
| Gruntfuggly.todo-tree@0.0.215 | nothing to convert, 0 warning(s) | yes | 13 | none | 0/31 | none |
| adpyke.codesnap@1.3.4 | nothing to convert, 0 warning(s) | yes | 4 | `vscode.window.createWebviewPanel` | 0/1 | `window.createWebviewPanel` |
| humao.rest-client@0.25.0 | 10 snippet(s) in 1 scope(s), 11 keybinding(s), 10 warning(s) | **no**: vscode.languages.registerCodeLensProvider is not implemented in Fyx | 21 | `vscode.languages.registerCodeLensProvider` | 5/5 | `languages.registerCodeLensProvider`, `languages.registerDefinitionProvider`, `languages.registerDocumentLinkProvider`, `languages.registerDocumentSymbolProvider`, `languages.registerReferenceProvider` |
| jock.svg@1.5.3 | nothing to convert, 0 warning(s) | yes | 38 | `vscode.window.createWebviewPanel` | 3/6 | `window.createWebviewPanel` |
| xabikos.JavaScriptSnippets@1.8.0 | 220 snippet(s) in 5 scope(s), 3 warning(s) | n/a (no code) |  |  |  |  |
| dracula-theme.theme-dracula@2.25.1 | 2 theme(s), 0 warning(s) | n/a (no code) |  |  |  |  |
| zhuangtongfa.material-theme@3.20.6 | 5 theme(s), 0 warning(s) | yes | 6 | `vscode.window.createWebviewPanel` | 7/7 | `window.createWebviewPanel` |
| GitHub.github-vscode-theme@6.3.5 | 9 theme(s), 0 warning(s) | n/a (no code) |  |  |  |  |
| k--kato.intellij-idea-keybindings@1.7.8 | 63 keybinding(s), 190 warning(s) | yes | 3 | `vscode.window.showOpenDialog` | 0/1 | `window.showOpenDialog` |

### wmaurer.change-case

APIs used: `commands.registerCommand`, `Range`, `Selection`, `window.activeTextEditor`, `window.showQuickPick`

Effects seen by the host: 0 diagnostics, 0 decoration types / 0 ranges, providers [], edited [javascript], host commands []


Commands: 17 of 17 ran without error.

Errors:
- uncaught: Error: Illegal argument: line and character must be non-negative (at new Position (…/types.ts:337:42) \| at new Range (…/types.ts:377:41))

RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×55, host→sidecar initialized ×1, host→sidecar vscode/didOpenTextDocument ×7, host→sidecar vscode/didChangeDiagnostics ×1, host→sidecar vscode/didChangeActiveEditor ×18, host→sidecar mount ×4, host→sidecar vscode/executeCommand ×17, sidecar→host log ×1, sidecar→host vscode/registerCommand ×17, sidecar→host fyx/surfacesChanged ×2, host→sidecar list-rows ×1, host→sidecar handle-event ×1, sidecar→host vscode/applyEdit ×3, host→sidecar <response> ×3, sidecar→host vscode/setSelections ×5, host→sidecar vscode/didChangeTextDocument ×3, host→sidecar fyx/coverage ×1

### Tyriar.sort-lines

APIs used: `commands.registerCommand`, `ExtensionContext.subscriptions`, `Range`, `window.activeTextEditor`, `workspace.getConfiguration`

Effects seen by the host: 0 diagnostics, 0 decoration types / 0 ranges, providers [], edited [javascript], host commands []


Commands: 14 of 14 ran without error.

RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×54, host→sidecar initialized ×1, host→sidecar vscode/didOpenTextDocument ×7, host→sidecar vscode/didChangeDiagnostics ×1, host→sidecar vscode/didChangeActiveEditor ×15, host→sidecar mount ×3, host→sidecar vscode/executeCommand ×14, sidecar→host log ×1, sidecar→host vscode/registerCommand ×14, sidecar→host vscode/applyEdit ×14, host→sidecar <response> ×14, host→sidecar vscode/didChangeTextDocument ×11, host→sidecar fyx/coverage ×1

### oderwat.indent-rainbow

APIs used: `ExtensionContext.subscriptions`, `Range`, `window.activeTextEditor`, `window.createTextEditorDecorationType`, `window.onDidChangeActiveTextEditor`, `workspace.getConfiguration`, `workspace.onDidChangeConfiguration`, `workspace.onDidChangeTextDocument`

Partial: `vscode.window.createTextEditorDecorationType`: raw colours/icons dropped (fyx:ui carries theme tokens only): backgroundColor=rgba(128,32,32,0.6)

Effects seen by the host: 0 diagnostics, 6 decoration types / 2 ranges, providers [], edited [], host commands []


RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×15, host→sidecar initialized ×1, sidecar→host log ×1, sidecar→host vscode/createDecorationType ×6, host→sidecar vscode/didOpenTextDocument ×7, host→sidecar vscode/didChangeDiagnostics ×1, host→sidecar vscode/didChangeActiveEditor ×1, sidecar→host vscode/setDecorations ×6, host→sidecar mount ×3, host→sidecar fyx/coverage ×1

### wayou.vscode-todo-highlight

APIs used: `commands.registerCommand`, `ExtensionContext.subscriptions`, `ExtensionContext.workspaceState`, `Range`, `window.activeTextEditor`, `window.createOutputChannel`, `window.createStatusBarItem`, `window.createTextEditorDecorationType`, `window.onDidChangeActiveTextEditor`, `window.outputChannel`, `window.showQuickPick`, `window.statusBarItem`, `workspace.findFiles`, `workspace.getConfiguration`, `workspace.onDidChangeConfiguration`, `workspace.onDidChangeTextDocument`, `workspace.openTextDocument`, `workspace.rootPath`

Partial: `vscode.StatusBarItem.tooltip`: accepted but not shown: fyx:ui buttons have no tooltip; `vscode.window.createTextEditorDecorationType`: raw colours/icons dropped (fyx:ui carries theme tokens only): color=#fff, backgroundColor=#ffbd2a, overviewRulerColor=rgba(255,189,42,0.8)

Names VS Code lacks too (proposed/feature detection): `vscode.window.manullyCancel`, `vscode.window.outputChannel`, `vscode.window.statusBarItem`

Effects seen by the host: 0 diagnostics, 2 decoration types / 1 ranges, providers [], edited [], host commands []

- surface `status-left`: 1 flex, 1 button: [$(checklist) 1]
- surface `output`: 2 flex, 2 text: TodoHighlight · #1 …/app.js#1 TODO: tidy

Commands: 2 of 2 ran without error.

RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×23, host→sidecar initialized ×1, sidecar→host log ×1, sidecar→host vscode/createDecorationType ×2, sidecar→host vscode/registerCommand ×3, host→sidecar vscode/didOpenTextDocument ×7, host→sidecar vscode/didChangeDiagnostics ×1, host→sidecar vscode/didChangeActiveEditor ×3, sidecar→host vscode/setDecorations ×4, host→sidecar mount ×4, host→sidecar vscode/executeCommand ×2, sidecar→host vscode/updateConfiguration ×1, sidecar→host fyx/surfacesChanged ×2, host→sidecar list-rows ×1, host→sidecar handle-event ×2, sidecar→host push-update ×3, host→sidecar fyx/coverage ×1

### shardulm94.trailing-spaces

APIs used: `commands.registerTextEditorCommand`, `ExtensionContext.extensionMode`, `ExtensionContext.subscriptions`, `Range`, `window.createTextEditorDecorationType`, `window.onDidChangeActiveTextEditor`, `window.setStatusBarMessage`, `window.visibleTextEditors`, `workspace.getConfiguration`, `workspace.onDidChangeConfiguration`, `workspace.onDidChangeTextDocument`, `workspace.onDidOpenTextDocument`

Partial: `vscode.window.createTextEditorDecorationType`: raw colours/icons dropped (fyx:ui carries theme tokens only): backgroundColor=rgba(255,0,0,0.3), borderColor=rgba(255,100,100,0.15)

Effects seen by the host: 0 diagnostics, 1 decoration types / 1 ranges, providers [], edited [javascript], host commands []

- surface `status-left`: 1 flex, 2 button: [Deleting 1 trailing space region] · [No trailing spaces to delete!]

Commands: 3 of 3 ran without error.

RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×24, host→sidecar initialized ×1, sidecar→host vscode/createDecorationType ×1, sidecar→host log ×1, sidecar→host vscode/registerCommand ×3, host→sidecar vscode/didOpenTextDocument ×7, host→sidecar vscode/didChangeDiagnostics ×1, host→sidecar vscode/didChangeActiveEditor ×4, sidecar→host vscode/setDecorations ×4, host→sidecar mount ×3, host→sidecar vscode/executeCommand ×3, sidecar→host vscode/applyEdit ×1, host→sidecar <response> ×1, sidecar→host push-update ×2, host→sidecar vscode/didChangeTextDocument ×1, host→sidecar handle-event ×2, host→sidecar fyx/coverage ×1

### mkxml.vscode-filesize

APIs used: `commands.registerCommand`, `ExtensionContext.subscriptions`, `window.activeTextEditor`, `window.createOutputChannel`, `window.createStatusBarItem`, `window.onDidChangeActiveTextEditor`, `workspace.getConfiguration`, `workspace.onDidChangeConfiguration`, `workspace.onDidSaveTextDocument`

Partial: `vscode.StatusBarItem.tooltip`: accepted but not shown: fyx:ui buttons have no tooltip

Effects seen by the host: 0 diagnostics, 0 decoration types / 0 ranges, providers [], edited [], host commands []

- surface `status-left`: 1 flex, 1 button: [104 bytes]
- surface `output`: 2 flex, 2 text: filesize · …/app.js +-----------------------------------------------…

Commands: 1 of 1 ran without error.

RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×18, host→sidecar initialized ×1, sidecar→host log ×2, sidecar→host vscode/registerCommand ×1, host→sidecar vscode/didOpenTextDocument ×7, host→sidecar vscode/didChangeDiagnostics ×1, host→sidecar vscode/didChangeActiveEditor ×2, host→sidecar mount ×3, host→sidecar vscode/executeCommand ×1, sidecar→host push-update ×1, host→sidecar handle-event ×1, host→sidecar fyx/coverage ×1

### EditorConfig.EditorConfig

APIs used: `commands.registerCommand`, `Disposable`, `ExtensionContext.subscriptions`, `languages.registerCompletionItemProvider`, `Uri`, `window.activeTextEditor`, `window.createOutputChannel`, `window.onDidChangeActiveTextEditor`, `window.onDidChangeWindowState`, `window.showErrorMessage`, `workspace.asRelativePath`, `workspace.fs`, `workspace.getConfiguration`, `workspace.onDidSaveTextDocument`, `workspace.onWillSaveTextDocument`, `workspace.workspaceFolders`

Effects seen by the host: 0 diagnostics, editor options {"tabSize":2,"insertSpaces":true,"cursorStyle":1,"lineNumbers":1,"indentSize":2}, 0 decoration types / 0 ranges, providers [completion], edited [], host commands [], messages: error: An .editorconfig file already exists in this workspace.

- surface `output`: 2 flex, 2 text: EditorConfig · Initializing document watcher... Document watcher initial…

Commands: 1 of 1 ran without error.

Language features:
- completion plaintext: 0 item(s)
- completion javascript: 0 item(s)
- completion html: 0 item(s)
- completion csv: 0 item(s)
- completion http: 0 item(s)
- completion svg: 0 item(s)
- completion markdown: 0 item(s)

RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×24, host→sidecar initialized ×1, sidecar→host log ×1, sidecar→host vscode/registerProvider ×1, sidecar→host vscode/registerCommand ×2, host→sidecar vscode/didOpenTextDocument ×7, host→sidecar vscode/didChangeDiagnostics ×1, host→sidecar vscode/didChangeActiveEditor ×2, sidecar→host vscode/setEditorOptions ×1, host→sidecar mount ×3, host→sidecar vscode/executeCommand ×1, sidecar→host vscode/showMessage ×1, host→sidecar <response> ×1, host→sidecar vscode/provideCompletion ×7, host→sidecar fyx/coverage ×1

### christian-kohler.path-intellisense

APIs used: `ExtensionContext.subscriptions`, `languages.registerCompletionItemProvider`, `Position`, `Range`, `RelativePattern`, `Uri`, `workspace.createFileSystemWatcher`, `workspace.findFiles`, `workspace.fs`, `workspace.getConfiguration`, `workspace.getWorkspaceFolder`, `workspace.workspaceFolders`

Partial: `vscode.workspace.createFileSystemWatcher`: returns a watcher whose events never fire (no host file events yet)

Effects seen by the host: 0 diagnostics, 0 decoration types / 0 ranges, providers [completion], edited [], host commands []


Language features:
- completion plaintext: 0 item(s)
- completion javascript: 14 item(s)
- completion html: 0 item(s)
- completion csv: 0 item(s)
- completion http: 0 item(s)
- completion svg: 0 item(s)
- completion markdown: 0 item(s)

RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×22, host→sidecar initialized ×1, sidecar→host log ×1, sidecar→host vscode/registerProvider ×2, host→sidecar vscode/didOpenTextDocument ×7, host→sidecar vscode/didChangeDiagnostics ×1, host→sidecar vscode/didChangeActiveEditor ×1, host→sidecar mount ×3, host→sidecar vscode/provideCompletion ×7, host→sidecar fyx/coverage ×1

### formulahendry.auto-rename-tag

Activation error: Error: Access to this API has been restricted. Use --allow-child-process to manage permissions. (at ChildProcess.spawn (node:internal/child_process:401:28) \| at spawn (node:child_process:796:9))

APIs used: `CodeAction`, `CodeActionKind`, `CodeLens`, `CompletionItem`, `Diagnostic`, `Disposable`, `DocumentLink`, `ExtensionContext.asAbsolutePath`, `ExtensionContext.subscriptions`, `languages.createDiagnosticCollection`, `version`, `window.createOutputChannel`, `window.showErrorMessage`, `window.showInformationMessage`, `workspace.getConfiguration`, `workspace.onDidChangeConfiguration`, `workspace.onDidCloseTextDocument`, `workspace.onDidOpenTextDocument`, `workspace.onDidSaveTextDocument`, `workspace.onWillSaveTextDocument`, `workspace.workspaceFolders`

Effects seen by the host: 0 diagnostics, 0 decoration types / 0 ranges, providers [], edited [], host commands [], messages: info: A request has failed. See the output for more information. / error: Couldn't start client Auto Rename Tag

- surface `output`: 2 flex, 2 text: Auto Rename Tag · [Error - 7:03:48 PM] Starting client failed Error: Access…

Errors:
- activate: Error: Access to this API has been restricted. Use --allow-child-process to manage permissions. (at ChildProcess.spawn (node:internal/child_process:401:28) \| at spawn (node:child_process:796:9))

RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×15, host→sidecar initialized ×1, sidecar→host log ×2, sidecar→host vscode/showMessage ×2, host→sidecar <response> ×2, host→sidecar vscode/didOpenTextDocument ×7, host→sidecar vscode/didChangeDiagnostics ×1, host→sidecar vscode/didChangeActiveEditor ×1, host→sidecar mount ×3, host→sidecar fyx/coverage ×1

### usernamehw.errorlens

APIs used: `commands.registerCommand`, `commands.registerTextEditorCommand`, `ExtensionContext.subscriptions`, `languages.getDiagnostics`, `languages.onDidChangeDiagnostics`, `Range`, `Selection`, `ThemeColor`, `window.activeTextEditor`, `window.createStatusBarItem`, `window.createTextEditorDecorationType`, `window.onDidChangeActiveTextEditor`, `window.onDidChangeTextEditorVisibleRanges`, `window.onDidChangeVisibleTextEditors`, `window.showInformationMessage`, `window.showWarningMessage`, `window.visibleTextEditors`, `workspace.getConfiguration`, `workspace.getWorkspaceFolder`, `workspace.onDidChangeConfiguration`, `workspace.onDidCloseTextDocument`

Partial: `vscode.StatusBarItem.color`: dropped: fyx:ui buttons take a style, not a colour

Effects seen by the host: 0 diagnostics, 0 decoration types / 1 ranges, providers [], edited [], host commands [], messages: info: "…/workspace" - Added to "errorLens.excludeWorkspaces" (now ignored) / warning: Diagnostic has no "source". / info: There's no problem at the active line. / info: There's no problem at the active line.


Commands: 17 of 17 ran without error.

RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×49, host→sidecar initialized ×1, sidecar→host log ×1, sidecar→host vscode/createDecorationType ×9, sidecar→host vscode/registerCommand ×20, host→sidecar vscode/didOpenTextDocument ×7, host→sidecar vscode/didChangeDiagnostics ×1, host→sidecar vscode/didChangeActiveEditor ×18, sidecar→host vscode/setDecorations ×8, host→sidecar mount ×3, host→sidecar vscode/executeCommand ×17, sidecar→host vscode/updateConfiguration ×7, sidecar→host vscode/disposeDecorationType ×63, sidecar→host vscode/setSelections ×3, sidecar→host vscode/revealRange ×3, sidecar→host vscode/showMessage ×4, host→sidecar <response> ×4, host→sidecar fyx/coverage ×1

### mechatroner.rainbow-csv

Activation error: vscode.SemanticTokensLegend is not implemented in Fyx

APIs used: none

Threw "not implemented in Fyx": `vscode.SemanticTokensLegend` (1×)

Effects seen by the host: 0 diagnostics, 0 decoration types / 0 ranges, providers [], edited [], host commands []


Commands: 0 of 24 ran without error.
- `rainbow-csv.CSVLint`: error: Error: command 'rainbow-csv.CSVLint' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task_q
- `rainbow-csv.SetVirtualHeader`: error: Error: command 'rainbow-csv.SetVirtualHeader' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/proce
- `rainbow-csv.SetHeaderLine`: error: Error: command 'rainbow-csv.SetHeaderLine' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/
- `rainbow-csv.SetCommentPrefix`: error: Error: command 'rainbow-csv.SetCommentPrefix' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/proce
- `rainbow-csv.RBQL`: error: Error: command 'rainbow-csv.RBQL' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task_queu
- `rainbow-csv.SetJoinTableName`: error: Error: command 'rainbow-csv.SetJoinTableName' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/proce
- `rainbow-csv.ColumnEditBefore`: error: Error: command 'rainbow-csv.ColumnEditBefore' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/proce
- `rainbow-csv.ColumnEditAfter`: error: Error: command 'rainbow-csv.ColumnEditAfter' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/proces
- `rainbow-csv.ColumnEditSelect`: error: Error: command 'rainbow-csv.ColumnEditSelect' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/proce
- `rainbow-csv.RainbowSeparator`: error: Error: command 'rainbow-csv.RainbowSeparator' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/proce
- `rainbow-csv.ToggleColumnTracking`: error: Error: command 'rainbow-csv.ToggleColumnTracking' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/p
- `rainbow-csv.ExcelCopy`: error: Error: command 'rainbow-csv.ExcelCopy' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task
- `rainbow-csv.MarkdownCopy`: error: Error: command 'rainbow-csv.MarkdownCopy' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/t
- `rainbow-csv.RainbowSeparatorOff`: error: Error: command 'rainbow-csv.RainbowSeparatorOff' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/pr
- `rainbow-csv.RainbowSeparatorOn`: error: Error: command 'rainbow-csv.RainbowSeparatorOn' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/pro
- `rainbow-csv.ToggleRowBackground`: error: Error: command 'rainbow-csv.ToggleRowBackground' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/pr
- `rainbow-csv.Align`: error: Error: command 'rainbow-csv.Align' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task_que
- `rainbow-csv.Shrink`: error: Error: command 'rainbow-csv.Shrink' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task_qu
- `rainbow-csv.VirtualAlign`: error: Error: command 'rainbow-csv.VirtualAlign' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/t
- `rainbow-csv.VirtualShrink`: error: Error: command 'rainbow-csv.VirtualShrink' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/
- `rainbow-csv.CopyBack`: error: Error: command 'rainbow-csv.CopyBack' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task_
- `rainbow-csv.SampleHead`: error: Error: command 'rainbow-csv.SampleHead' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/tas
- `rainbow-csv.SampleTail`: error: Error: command 'rainbow-csv.SampleTail' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/tas
- `rainbow-csv.GoToColumn`: error: Error: command 'rainbow-csv.GoToColumn' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/tas

Errors:
- activate: vscode.SemanticTokensLegend is not implemented in Fyx

RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×63, host→sidecar initialized ×1, host→sidecar vscode/didOpenTextDocument ×7, sidecar→host log ×1, host→sidecar vscode/didChangeDiagnostics ×1, host→sidecar vscode/didChangeActiveEditor ×25, host→sidecar mount ×3, host→sidecar vscode/executeCommand ×24, host→sidecar fyx/coverage ×1

### ExodiusStudios.comment-anchors

Activation error: vscode.languages.registerDocumentLinkProvider is not implemented in Fyx

APIs used: `EventEmitter`, `ExtensionContext.globalStoragePath`, `ExtensionContext.subscriptions`, `TreeItem`, `window.activeTextEditor`, `window.createOutputChannel`, `window.createTreeView`, `window.onDidChangeActiveTextEditor`, `workspace.getConfiguration`, `workspace.onDidChangeConfiguration`, `workspace.onDidChangeTextDocument`, `workspace.onDidChangeWorkspaceFolders`, `workspace.onDidCloseTextDocument`, `workspace.workspaceFolders`

Threw "not implemented in Fyx": `vscode.languages.registerDocumentLinkProvider` (1×)

Effects seen by the host: 0 diagnostics, 0 decoration types / 0 ranges, providers [], edited [], host commands []

- surface `output`: 2 flex, 2 text: Comment Anchors · [Comment Anchors] Failed to build resources: ENOENT: no s…
- surface `view/fileAnchors`: 1 flex, 1 list; rows: Searching for anchors...
- surface `view/workspaceAnchors`: 1 flex, 1 list; rows: Click to start scanning
- surface `view/epicAnchors`: 1 flex, 1 list; rows: Click to start scanning

Commands: 0 of 5 ran without error.
- `commentAnchors.parse`: error: Error: command 'commentAnchors.parse' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task_
- `commentAnchors.toggle`: error: Error: command 'commentAnchors.toggle' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task
- `commentAnchors.listTags`: error: Error: command 'commentAnchors.listTags' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/ta
- `commentAnchors.openFileAndRevealLine`: error: Error: command 'commentAnchors.openFileAndRevealLine' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:intern
- `commentAnchors.launchWorkspaceScan`: error: Error: command 'commentAnchors.launchWorkspaceScan' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal

Errors:
- activate: vscode.languages.registerDocumentLinkProvider is not implemented in Fyx

RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×31, host→sidecar initialized ×1, sidecar→host log ×2, host→sidecar vscode/didOpenTextDocument ×7, host→sidecar vscode/didChangeDiagnostics ×1, host→sidecar vscode/didChangeActiveEditor ×6, host→sidecar mount ×6, host→sidecar list-rows ×3, host→sidecar vscode/executeCommand ×5, host→sidecar fyx/coverage ×1

### Gruntfuggly.todo-tree

APIs used: `commands.executeCommand`, `env.appRoot`, `EventEmitter`, `ExtensionContext.globalState`, `ExtensionContext.subscriptions`, `ExtensionContext.workspaceState`, `ThemeIcon`, `TreeItem`, `window.createStatusBarItem`, `window.createTreeView`, `window.showErrorMessage`, `workspace.getConfiguration`, `workspace.registerTextDocumentContentProvider`

Effects seen by the host: 0 diagnostics, 0 decoration types / 0 ranges, providers [], edited [], host commands [], messages: error: Todo-Tree: Failed to find vscode-ripgrep - please install ripgrep manually and set 'todo-tree

- surface `view/todo-tree-view`: 1 flex, 1 list; rows:  Scan mode: workspace and open files \|  Nothing found

Commands: 0 of 31 ran without error.
- `todo-tree.showFlatView`: error: Error: command 'todo-tree.showFlatView' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/tas
- `todo-tree.showTagsOnlyView`: error: Error: command 'todo-tree.showTagsOnlyView' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process
- `todo-tree.showTreeView`: error: Error: command 'todo-tree.showTreeView' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/tas
- `todo-tree.refresh`: error: Error: command 'todo-tree.refresh' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task_que
- `todo-tree.expand`: error: Error: command 'todo-tree.expand' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task_queu
- `todo-tree.collapse`: error: Error: command 'todo-tree.collapse' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task_qu
- `todo-tree.filter`: error: Error: command 'todo-tree.filter' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task_queu
- `todo-tree.filterClear`: error: Error: command 'todo-tree.filterClear' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task
- `todo-tree.groupByTag`: error: Error: command 'todo-tree.groupByTag' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task_
- `todo-tree.ungroupByTag`: error: Error: command 'todo-tree.ungroupByTag' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/tas
- `todo-tree.groupBySubTag`: error: Error: command 'todo-tree.groupBySubTag' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/ta
- `todo-tree.ungroupBySubTag`: error: Error: command 'todo-tree.ungroupBySubTag' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/
- `todo-tree.scanOpenFilesOnly`: error: Error: command 'todo-tree.scanOpenFilesOnly' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/proces
- `todo-tree.scanCurrentFileOnly`: error: Error: command 'todo-tree.scanCurrentFileOnly' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/proc
- `todo-tree.scanWorkspaceAndOpenFiles`: error: Error: command 'todo-tree.scanWorkspaceAndOpenFiles' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:interna
- `todo-tree.scanWorkspaceOnly`: error: Error: command 'todo-tree.scanWorkspaceOnly' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/proces
- `todo-tree.addTag`: error: Error: command 'todo-tree.addTag' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task_queu
- `todo-tree.removeTag`: error: Error: command 'todo-tree.removeTag' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task_q
- `todo-tree.exportTree`: error: Error: command 'todo-tree.exportTree' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task_
- `todo-tree.showOnlyThisFolder`: error: Error: command 'todo-tree.showOnlyThisFolder' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/proce
- `todo-tree.showOnlyThisFolderAndSubfolders`: error: Error: command 'todo-tree.showOnlyThisFolderAndSubfolders' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:i
- `todo-tree.switchScope`: error: Error: command 'todo-tree.switchScope' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task
- `todo-tree.excludeThisFolder`: error: Error: command 'todo-tree.excludeThisFolder' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/proces
- `todo-tree.excludeThisFile`: error: Error: command 'todo-tree.excludeThisFile' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/
- `todo-tree.removeFilter`: error: Error: command 'todo-tree.removeFilter' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/tas
- `todo-tree.resetAllFilters`: error: Error: command 'todo-tree.resetAllFilters' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/
- `todo-tree.reveal`: error: Error: command 'todo-tree.reveal' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task_queu
- `todo-tree.resetCache`: error: Error: command 'todo-tree.resetCache' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/task_
- `todo-tree.toggleItemCounts`: error: Error: command 'todo-tree.toggleItemCounts' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process
- `todo-tree.toggleBadges`: error: Error: command 'todo-tree.toggleBadges' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/process/tas
- `todo-tree.toggleCompactFolders`: error: Error: command 'todo-tree.toggleCompactFolders' not found (at CommandRegistry.execute (…/api.ts:96:28) \| at process.processTicksAndRejections (node:internal/pro

RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×79, host→sidecar initialized ×1, sidecar→host log ×1, sidecar→host vscode/showMessage ×1, host→sidecar <response> ×1, host→sidecar vscode/didOpenTextDocument ×7, host→sidecar vscode/didChangeDiagnostics ×1, host→sidecar vscode/didChangeActiveEditor ×32, host→sidecar mount ×4, sidecar→host vscode/setContext ×43, host→sidecar list-rows ×1, host→sidecar vscode/executeCommand ×31, host→sidecar fyx/coverage ×1

### adpyke.codesnap

APIs used: `commands.registerCommand`, `ExtensionContext.extensionPath`, `ExtensionContext.subscriptions`, `Uri`

Threw "not implemented in Fyx": `vscode.window.createWebviewPanel` (1×)

Effects seen by the host: 0 diagnostics, 0 decoration types / 0 ranges, providers [], edited [], host commands []


Commands: 0 of 1 ran without error.
- `codesnap.start`: error: vscode.window.createWebviewPanel is not implemented in Fyx

RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×17, host→sidecar initialized ×1, host→sidecar vscode/didOpenTextDocument ×7, host→sidecar vscode/didChangeDiagnostics ×1, host→sidecar vscode/didChangeActiveEditor ×2, host→sidecar mount ×3, host→sidecar vscode/executeCommand ×1, sidecar→host log ×1, sidecar→host vscode/registerCommand ×1, host→sidecar fyx/coverage ×1

### humao.rest-client

Activation error: vscode.languages.registerCodeLensProvider is not implemented in Fyx

APIs used: `commands.registerCommand`, `CompletionItem`, `env.clipboard`, `EventEmitter`, `ExtensionContext.asAbsolutePath`, `ExtensionContext.subscriptions`, `extensions.getExtension`, `languages.match`, `languages.registerCompletionItemProvider`, `languages.registerHoverProvider`, `languages.setLanguageConfiguration`, `SnippetString`, `Uri`, `window.activeTextEditor`, `window.createOutputChannel`, `window.createStatusBarItem`, `window.onDidChangeActiveTextEditor`, `window.showQuickPick`, `workspace.getConfiguration`, `workspace.onDidChangeConfiguration`, `workspace.onDidCloseTextDocument`

Threw "not implemented in Fyx": `vscode.languages.registerCodeLensProvider` (1×)

Partial: `vscode.languages.setLanguageConfiguration`: stored but not applied: Zed language config comes from its own config.toml; `vscode.StatusBarItem.tooltip`: accepted but not shown: fyx:ui buttons have no tooltip

Effects seen by the host: 0 diagnostics, 0 decoration types / 0 ranges, providers [completion, hover], edited [], host commands []

- surface `status-right`: 1 flex, 1 button: [No Environment]
- surface `output`: 2 flex, 2 text: REST

Commands: 5 of 15 ran without error.
- `rest-client.request`: skipped (network)
- `rest-client.rerun-last-request`: skipped (network)
- `rest-client.cancel-request`: skipped (network)
- `rest-client.switch-environment`: skipped (network)
- `rest-client.history`: skipped (network)
- `rest-client.clear-history`: skipped (network)
- `rest-client.copy-response-body`: skipped (network)
- `rest-client.generate-codesnippet`: skipped (network)
- `rest-client.copy-codesnippet`: skipped (network)
- `rest-client.copy-request-as-curl`: skipped (network)

Language features:
- completion plaintext: 0 item(s)
- hover plaintext: no hover
- completion javascript: 0 item(s)
- hover javascript: no hover
- completion html: 0 item(s)
- hover html: no hover
- completion csv: 0 item(s)
- hover csv: no hover
- completion http: 52 item(s)
- hover http: no hover
- completion svg: 0 item(s)
- hover svg: no hover
- completion markdown: 0 item(s)
- hover markdown: no hover

Errors:
- activate: vscode.languages.registerCodeLensProvider is not implemented in Fyx

RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×43, host→sidecar initialized ×1, host→sidecar vscode/didOpenTextDocument ×7, sidecar→host log ×7, host→sidecar vscode/didChangeDiagnostics ×1, sidecar→host vscode/registerCommand ×16, host→sidecar vscode/didChangeActiveEditor ×6, sidecar→host vscode/registerProvider ×4, host→sidecar mount ×4, host→sidecar vscode/executeCommand ×5, host→sidecar handle-event ×2, sidecar→host fyx/surfacesChanged ×2, host→sidecar list-rows ×1, host→sidecar vscode/provideCompletion ×7, host→sidecar vscode/provideHover ×7, host→sidecar fyx/coverage ×1

### jock.svg

APIs used: `CodeAction`, `CodeActionKind`, `CodeLens`, `commands.registerCommand`, `commands.registerTextEditorCommand`, `CompletionItem`, `Diagnostic`, `Disposable`, `DocumentLink`, `env.clipboard`, `env.clipboard.writeText`, `env.language`, `ExtensionContext.asAbsolutePath`, `ExtensionContext.extensionMode`, `ExtensionContext.extensionUri`, `ExtensionContext.subscriptions`, `ExtensionContext.workspaceState`, `languages.createDiagnosticCollection`, `Uri`, `version`, `window.activeTextEditor`, `window.createOutputChannel`, `window.createTextEditorDecorationType`, `window.onDidChangeActiveTextEditor`, `window.onDidChangeVisibleTextEditors`, `window.showErrorMessage`, `window.showInformationMessage`, `window.showWarningMessage`, `window.visibleTextEditors`, `workspace.createFileSystemWatcher`, `workspace.getConfiguration`, `workspace.onDidChangeConfiguration`, `workspace.onDidChangeTextDocument`, `workspace.onDidCloseTextDocument`, `workspace.onDidOpenTextDocument`, `workspace.onDidSaveTextDocument`, `workspace.onWillSaveTextDocument`, `workspace.workspaceFolders`

Threw "not implemented in Fyx": `vscode.window.createWebviewPanel` (3×)

Partial: `vscode.window.createTextEditorDecorationType`: raw colours/icons dropped (fyx:ui carries theme tokens only): color=#99f; `vscode.workspace.createFileSystemWatcher`: returns a watcher whose events never fire (no host file events yet)

Effects seen by the host: 0 diagnostics, 2 decoration types / 0 ranges, providers [], edited [], host commands [], messages: info: A request has failed. See the output for more information. / error: Couldn't start client Language Server SVG / warning: [SVG] Irreversibly broken warning, Backup your svg!

- surface `output`: 2 flex, 2 text: Language Server SVG · [Error - 7:04:46 PM] Starting client failed Error: Access…

Commands: 3 of 6 ran without error.
- `svg.showSvg`: error: vscode.window.createWebviewPanel is not implemented in Fyx
- `_svg.showSvgByUri`: error: vscode.window.createWebviewPanel is not implemented in Fyx
- `svg.showAllSvg`: error: vscode.window.createWebviewPanel is not implemented in Fyx

Errors:
- uncaught: Error: Access to this API has been restricted. Use --allow-child-process to manage permissions. (at ChildProcess.spawn (node:internal/child_process:401:28) \| at spawn (node:child_process:796:9))

RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×27, host→sidecar initialized ×1, host→sidecar vscode/didOpenTextDocument ×7, sidecar→host log ×5, sidecar→host vscode/registerCommand ×6, sidecar→host vscode/createDecorationType ×2, sidecar→host vscode/showMessage ×3, host→sidecar <response> ×4, host→sidecar vscode/didChangeDiagnostics ×1, host→sidecar vscode/didChangeActiveEditor ×7, sidecar→host vscode/setDecorations ×2, host→sidecar mount ×3, host→sidecar vscode/executeCommand ×6, sidecar→host vscode/clipboard/writeText ×1, host→sidecar fyx/coverage ×1

### zhuangtongfa.material-theme

APIs used: `commands.registerCommand`, `Disposable`, `Uri`, `workspace.fs`, `workspace.getConfiguration`, `workspace.onDidChangeConfiguration`

Threw "not implemented in Fyx": `vscode.window.createWebviewPanel` (1×)

Effects seen by the host: 0 diagnostics, 0 decoration types / 0 ranges, providers [], edited [], host commands []


Commands: 7 of 7 ran without error.

Errors:
- uncaught: vscode.window.createWebviewPanel is not implemented in Fyx
- uncaught: NoPermissions (FileSystemError): …/OneDark-Pro.json (at FileSystemError.NoPermissions (…/types.ts:812:12) \| at fsError (…/workspace.ts:30:101))
- uncaught: NoPermissions (FileSystemError): …/markdown-inside.css (at FileSystemError.NoPermissions (…/types.ts:812:12) \| at fsError (…/workspace.ts:30:101))
- uncaught: NoPermissions (FileSystemError): …/base-inside.css (at FileSystemError.NoPermissions (…/types.ts:812:12) \| at fsError (…/workspace.ts:30:101))
- uncaught: NoPermissions (FileSystemError): …/atom-one-dark-inside.css (at FileSystemError.NoPermissions (…/types.ts:812:12) \| at fsError (…/workspace.ts:30:101))

RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×29, host→sidecar initialized ×1, sidecar→host log ×3, sidecar→host vscode/registerCommand ×7, host→sidecar vscode/didOpenTextDocument ×7, host→sidecar vscode/didChangeDiagnostics ×1, host→sidecar vscode/didChangeActiveEditor ×8, host→sidecar mount ×3, host→sidecar vscode/executeCommand ×7, sidecar→host vscode/updateConfiguration ×6, host→sidecar fyx/coverage ×1

### k--kato.intellij-idea-keybindings

APIs used: `commands.registerCommand`, `ExtensionContext.subscriptions`, `window.showQuickPick`

Threw "not implemented in Fyx": `vscode.window.showOpenDialog` (1×)

Effects seen by the host: 0 diagnostics, 0 decoration types / 0 ranges, providers [], edited [], host commands []


Commands: 0 of 1 ran without error.
- `intellij.importKeyMapsSchema`: error: vscode.window.showOpenDialog is not implemented in Fyx

RPC traffic: host→sidecar initialize ×1, sidecar→host <response> ×23, host→sidecar initialized ×1, host→sidecar vscode/didOpenTextDocument ×7, host→sidecar vscode/didChangeDiagnostics ×1, host→sidecar vscode/didChangeActiveEditor ×2, host→sidecar mount ×5, host→sidecar vscode/executeCommand ×1, sidecar→host log ×1, sidecar→host vscode/registerCommand ×2, sidecar→host fyx/surfacesChanged ×4, host→sidecar list-rows ×2, host→sidecar handle-event ×2, host→sidecar fyx/coverage ×1

<!-- END GENERATED -->
