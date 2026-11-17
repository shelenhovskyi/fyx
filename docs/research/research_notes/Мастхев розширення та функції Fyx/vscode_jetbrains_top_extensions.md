# Top VS Code / Open VSX / JetBrains extensions and the UI primitives they need (as of 2026-11-17)

Method note (applies to every number below). Install and download counts were pulled live on **2026-11-17** from the
public Marketplace backends, not from screenshots or third-party aggregators:

- VS Code Marketplace: `POST https://marketplace.visualstudio.com/_apis/public/gallery/extensionquery` (the API the
  Marketplace website itself uses), sorted by install count, top 200, plus per-id lookups. Each row cites the item page
  `https://marketplace.visualstudio.com/items?itemName=<id>`, which shows the same counter.
- Open VSX: `https://open-vsx.org/api/-/search?sortBy=downloadCount` and `https://open-vsx.org/api/<ns>/<name>`
  (counter is *downloads*, not installs).
- JetBrains Marketplace: `https://plugins.jetbrains.com/api/searchPlugins?orderBy=downloads` and
  `https://plugins.jetbrains.com/api/plugins/<id>` (counter is *downloads*).
- UI surfaces: (a) the `contributes` section of each extension's **published** `package.json` (fetched from the
  Marketplace "Microsoft.VisualStudio.Code.Manifest" asset, or GitHub `HEAD`), and (b) a grep of 37 shallow-cloned
  extension source repos for the VS Code API calls `createWebviewPanel`, `registerWebviewViewProvider`,
  `registerCustom(Text)EditorProvider`, `createTextEditorDecorationType`, `gutterIconPath`, `contentText` (after/before
  text), `registerCodeLensProvider`, `registerInlayHintsProvider`, `registerHoverProvider`, `createStatusBarItem`,
  `showQuickPick|createQuickPick`, `createTreeView|registerTreeDataProvider`, `createTestController`,
  `createCommentController`, notebook APIs, `createDiagnosticCollection`, `registerCompletionItemProvider`,
  `registerTextDocumentContentProvider`, `registerFileDecorationProvider`. Counts are "number of source files that
  contain the call" (test dirs excluded), so they show presence, not intensity.

---

## Q1. What are the top VS Code extensions by installs (Marketplace and Open VSX), excluding language packs/themes?

### Takeaway
The Marketplace top-200 is dominated by Microsoft language stacks (Python 245.8M, Pylance 212.4M, C/C++ 107.2M, Jupyter
111.5M) and AI assistants; once language support, packs and themes are removed, the top tool extensions are Live Server
(84.5M), Copilot Chat (79.3M), Prettier (73.0M), GitLens (54.0M), ESLint (53.9M), Docker (53.3M), Code Runner (42.8M),
the Remote trio (Dev Containers 42.8M, WSL 41.3M, Remote-SSH 38.7M) and GitHub Pull Requests (37.5M). Open VSX (used by
Cursor, Windsurf, VSCodium, Gitpod etc.) has a different, AI/LSP-heavy top list and lacks Microsoft-licensed remote and
Live Share extensions.

### Cited Findings

**Ranked table (tool extensions; Marketplace installs 2026-11-17).** Columns: *UI surfaces* = from manifest + source grep
(TV = tree view, WVv = webview view in a side panel, WVp = webview editor panel, CE = custom editor, DEC = text-editor
decorations, GUT = gutter icon, CL = CodeLens, INL = inlay hints, HOV = hover provider, SB = status bar item, QP = quick
pick / input box, LANG = diagnostics/completion/code-action/formatting provider, TEST = Testing API, CMT = Comments API,
NB = Notebook API, FDEC = file (explorer) decorations). *Fyx class*: Z = Zed already ships an equivalent; N = hostable
with current/planned fyx:ui (dock panel, status bar, tree/list/table, quick-pick/modal, block decorations, host markdown/
diff/editor views); D = needs inline decoration / gutter / hover / CodeLens / inlay primitives; L = needs a language-
provider bridge (diagnostics/completion/code actions — or ship as an LSP server); W = webview is essential to the
product; R = needs core remote/FS/terminal integration rather than UI.

| # | Extension (id) | Category | Installs (Marketplace) | Open VSX downloads | Rating (n) | What it does | UI surfaces used | Zed built-in equivalent | Fyx class |
|---|---|---|---|---|---|---|---|---|---|
| 1 | Live Server (`ritwickdey.LiveServer`) — [item](https://marketplace.visualstudio.com/items?itemName=ritwickdey.LiveServer), [Open VSX](https://open-vsx.org/extension/ritwickdey/LiveServer) | Web dev | 84,548,118 | 3,179,359 | 4.40 (520) | Local dev server with live reload, opens browser | SB ("Go Live"), QP, commands/menus ([package.json](https://github.com/ritwickdey/vscode-live-server/blob/HEAD/package.json)) | None | N (+ process/network capability) |
| 2 | GitHub Copilot Chat (`GitHub.copilot-chat`) — [item](https://marketplace.visualstudio.com/items?itemName=GitHub.copilot-chat) | AI | 79,331,460 | n/a | 2.74 (423) | Chat/agent, inline edits | Native chat contributions (`chatParticipants`, `chatSessions`, `languageModelTools`…), 2 TV, SB, QP, `terminalQuickFixes` (from published manifest) | Agent Panel ([docs](https://zed.dev/docs/ai/agent-panel.md)), Edit Prediction ([docs](https://zed.dev/docs/ai/edit-prediction.md)) | Z |
| 3 | GitHub Copilot (`GitHub.copilot`) — [item](https://marketplace.visualstudio.com/items?itemName=GitHub.copilot) | AI | 74,921,696 | n/a | 4.08 (1059) | Inline completions | Inline completion provider, SB | Edit Prediction ([docs](https://zed.dev/docs/ai/edit-prediction.md)) | Z |
| 4 | Prettier (`esbenp.prettier-vscode`) — [item](https://marketplace.visualstudio.com/items?itemName=esbenp.prettier-vscode) | Formatter | 73,025,384 | 9,971,196 | 3.54 (492) | Code formatter | LANG (formatting), SB; no views ([package.json](https://github.com/prettier/prettier-vscode/blob/HEAD/package.json)) | "Zed's built-in Prettier integration" ([docs](https://zed.dev/docs/configuring-zed)) | Z |
| 5 | IntelliCode (`VisualStudioExptTeam.vscodeintellicode`) — [item](https://marketplace.visualstudio.com/items?itemName=VisualStudioExptTeam.vscodeintellicode) | AI completion | 62,725,115 | n/a | 3.90 (104) | Ranked completions | LANG (completion) | Edit Prediction ([docs](https://zed.dev/docs/ai/edit-prediction.md)) | Z |
| 6 | GitLens (`eamodio.gitlens`) — [item](https://marketplace.visualstudio.com/items?itemName=eamodio.gitlens), [Open VSX](https://open-vsx.org/extension/eamodio/gitlens) | Git | 53,980,324 | 19,499,809 | 3.37 (946) | Blame, history, commit graph, worktrees | **16 TV + 5 WVv**, 2 view containers, CE (`gitlens.rebase`), WVp, DEC (after-line blame), GUT, CL, HOV (6 files), SB (5 files), QP (66 files), FDEC ([package.json](https://github.com/gitkraken/vscode-gitlens/blob/HEAD/package.json)) | Git Panel, inline blame, gutter diff hunks, File History, stash, conflicts; **no repository graph** ([docs](https://zed.dev/docs/git)) | D + partial W (graph/home are webviews) |
| 7 | ESLint (`dbaeumer.vscode-eslint`) — [item](https://marketplace.visualstudio.com/items?itemName=dbaeumer.vscode-eslint) | Lint | 53,889,487 | 6,723,105 | 4.29 (251) | JS/TS lint via LSP | LANG (LSP client), SB, QP; no views ([package.json](https://github.com/microsoft/vscode-eslint/blob/HEAD/package.json)) | Diagnostics & quick fixes via LSP ([docs](https://zed.dev/docs/diagnostics.md)); ESLint server bundling not verified this session | Z (via LSP) |
| 8 | Docker (`ms-azuretools.vscode-docker`) — [item](https://marketplace.visualstudio.com/items?itemName=ms-azuretools.vscode-docker) → v2.0.0 is a shim with `extensionDependencies: ["ms-azuretools.vscode-containers"]`; Container Tools — [item](https://marketplace.visualstudio.com/items?itemName=ms-azuretools.vscode-containers) | Containers | 53,329,500 (+ Container Tools 19,766,647) | 7,358,122 / 15,145,568 | 4.48 (107) | Container/image/registry explorer, compose, Dockerfile LSP | **7 TV** in a view container, QP (21 files), SB, `withProgress` (25 files), LANG, tasks/debuggers; **no webviews** (published manifest; [repo](https://github.com/microsoft/vscode-containers)) | Dev Containers only ([docs](https://zed.dev/docs/dev-containers)); no Docker explorer | N (+ process capability) |
| 9 | Code Runner (`formulahendry.code-runner`) — [item](https://marketplace.visualstudio.com/items?itemName=formulahendry.code-runner) | Productivity | 42,844,555 | n/a | 4.40 (286) | Run current file/snippet | Commands, editor-title menu, QP, terminal/output ([package.json](https://github.com/formulahendry/vscode-code-runner/blob/HEAD/package.json)) | Tasks ([docs](https://zed.dev/docs/tasks.md)) | Z |
| 10 | Dev Containers (`ms-vscode-remote.remote-containers`) — [item](https://marketplace.visualstudio.com/items?itemName=ms-vscode-remote.remote-containers) | Remote | 42,781,491 | not on Open VSX | 4.05 (60) | Open folder inside container | 3 TV, `remoteHelp`, `resourceLabelFormatters` (published manifest) | Dev Containers ("still marked as in development") ([docs](https://zed.dev/docs/dev-containers)) | Z / R |
| 11 | WSL (`ms-vscode-remote.remote-wsl`) — [item](https://marketplace.visualstudio.com/items?itemName=ms-vscode-remote.remote-wsl) | Remote | 41,341,192 | not on Open VSX | 4.79 (86) | Remote into WSL | Remote resolver | Not verified this session | R |
| 12 | Remote - SSH (`ms-vscode-remote.remote-ssh`) — [item](https://marketplace.visualstudio.com/items?itemName=ms-vscode-remote.remote-ssh); OSS clone on Open VSX `jeanp413.open-remote-ssh` 1,236,194 dl ([Open VSX](https://open-vsx.org/extension/jeanp413/open-remote-ssh)) | Remote | 38,711,048 | n/a (MS-licensed) | 3.59 (223) | Remote dev over SSH | Remote resolver, QP; tree comes from Remote Explorer (31,097,544 installs, 1 TV) ([item](https://marketplace.visualstudio.com/items?itemName=ms-vscode.remote-explorer)) | SSH remote development ([docs](https://zed.dev/docs/remote-development)) | Z |
| 13 | GitHub Pull Requests (`GitHub.vscode-pull-request-github`) — [item](https://marketplace.visualstudio.com/items?itemName=GitHub.vscode-pull-request-github) | Git/PR | 37,520,464 | 6,757,430 | 3.94 (149) | Review/merge PRs and issues | **9 TV + 2 WVv**, WVp (PR overview), **CMT** (review threads), DEC, HOV, SB, QP, FDEC, completion ([package.json](https://github.com/microsoft/vscode-pull-request-github/blob/HEAD/package.json)) | No PR workflow; only clickable PR/issue refs ([docs](https://zed.dev/docs/git)) | D + CMT + W (PR description page) |
| 14 | Claude Code (`anthropic.claude-code`) — [item](https://marketplace.visualstudio.com/items?itemName=anthropic.claude-code), [Open VSX](https://open-vsx.org/extension/Anthropic/claude-code) | AI agent | 30,457,773 | 67,330,356 | 3.70 (857) | Agent UI | **3 WVv** (published manifest) | Agent Panel ([docs](https://zed.dev/docs/ai/agent-panel.md)) | Z (or W) |
| 15 | Auto Rename Tag (`formulahendry.auto-rename-tag`) — [item](https://marketplace.visualstudio.com/items?itemName=formulahendry.auto-rename-tag) | HTML | 26,278,270 | n/a | 3.69 (215) | Rename paired tag on edit | Configuration only (published manifest) | Not verified | N (buffer edit events, no UI) |
| 16 | Rainbow CSV (`mechatroner.rainbow-csv`) — [item](https://marketplace.visualstudio.com/items?itemName=mechatroner.rainbow-csv) | Visual aid | 25,214,435 | 3,490,323 | 4.79 (163) | Column colouring, RBQL queries | grammars + `semanticTokenScopes`, DEC, **INL**, HOV, SB, 1 WVp (RBQL console) ([package.json](https://github.com/mechatroner/vscode_rainbow_csv/blob/HEAD/package.json)) | None found | D (+ INL) |
| 17 | Live Share (`MS-vsliveshare.vsliveshare`) — [item](https://marketplace.visualstudio.com/items?itemName=MS-vsliveshare.vsliveshare) | Collaboration | 24,472,769 | not on Open VSX | 4.24 (160) | Real-time co-editing | 4 TV, `statusBarItems`, debuggers, shared terminals (published manifest) | Channels/collaboration ([docs](https://zed.dev/docs/collaboration/channels.md)) | Z |
| 18 | WakaTime (`WakaTime.vscode-wakatime`) — [item](https://marketplace.visualstudio.com/items?itemName=WakaTime.vscode-wakatime) | Metrics | 23,110,999 | n/a | 4.07 (43) | Time tracking | SB, QP, input box only ([package.json](https://github.com/wakatime/vscode-wakatime/blob/HEAD/package.json)) | None (Zed has a WakaTime extension upstream — not verified) | N |
| 19 | Path Intellisense (`christian-kohler.path-intellisense`) — [item](https://marketplace.visualstudio.com/items?itemName=christian-kohler.path-intellisense) | Productivity | 19,691,906 | 593,580 | 4.85 (124) | File-path completion | LANG (completion only) ([package.json](https://github.com/ChristianKohler/PathIntellisense/blob/HEAD/package.json)) | Partial via LSPs | L |
| 20 | Code Spell Checker (`streetsidesoftware.code-spell-checker`) — [item](https://marketplace.visualstudio.com/items?itemName=streetsidesoftware.code-spell-checker) | Lint | 18,649,046 | 2,265,882 | 4.47 (314) | Spelling diagnostics + fixes | LANG (diagnostics, code actions), **3 TV + 1 WVv**, 2 view containers ([package.json](https://github.com/streetsidesoftware/vscode-spell-checker/blob/HEAD/package.json)) | None built in | L (+ N for views) |
| 21 | autoDocstring (`njpwerner.autodocstring`) — [item](https://marketplace.visualstudio.com/items?itemName=njpwerner.autodocstring) | Python | 17,922,125 | n/a | 4.91 (68) | Docstring templates | LANG (completion/snippet) | Snippets | L |
| 22 | Git History (`donjayamanne.githistory`) — [item](https://marketplace.visualstudio.com/items?itemName=donjayamanne.githistory) | Git | 17,558,371 | 686,504 | 4.44 (179) | Log/graph, file history | **WVp** (log view), 2 TV, QP (8 files) ([package.json](https://github.com/DonJayamanne/gitHistoryVSCode/blob/HEAD/package.json)) | File History only ([docs](https://zed.dev/docs/git)) | W (or N with virtualized table) |
| 23 | Codex (`openai.chatgpt`) — [item](https://marketplace.visualstudio.com/items?itemName=openai.chatgpt) | AI agent | 16,878,095 | 13,698,291 | 2.64 (941) | Agent UI | **2 WVv + CE** (`chatgpt.conversationEditor`) (published manifest) | Agent Panel ([docs](https://zed.dev/docs/ai/agent-panel.md)) | Z (or W) |
| 24 | Git Graph (`mhutchie.git-graph`) — [item](https://marketplace.visualstudio.com/items?itemName=mhutchie.git-graph) | Git | 15,739,579 | 1,233,140 | **4.93 (681)** | Commit graph, branch ops | **WVp only** (3 files), SB, QP; no views in manifest ([package.json](https://github.com/mhutchie/vscode-git-graph/blob/HEAD/package.json)) | No graph ([docs](https://zed.dev/docs/git)) | W (or new graph-lane primitive) |
| 25 | EditorConfig (`EditorConfig.EditorConfig`) — [item](https://marketplace.visualstudio.com/items?itemName=EditorConfig.EditorConfig) | Config | 15,418,709 | 2,400,166 | 4.23 (87) | Apply .editorconfig | None | `.editorconfig` honoured (e.g. `end_of_line` overrides `line_ending`) ([docs](https://zed.dev/docs/configuring-zed)) | Z |
| 26 | Markdown All in One (`yzhang.markdown-all-in-one`) — [item](https://marketplace.visualstudio.com/items?itemName=yzhang.markdown-all-in-one) | Markdown | 14,967,141 | 745,702 | 4.69 (172) | Shortcuts, TOC, lists, preview plugins | keybindings, completion, DEC, CL, `markdown.markdownItPlugins` / `previewStyles` (published manifest; [repo](https://github.com/yzhang-gh/vscode-markdown)) | Markdown preview exists in Zed (no dedicated docs page found) | D + L |
| 27 | vscode-pdf (`tomoki1207.pdf`) — [item](https://marketplace.visualstudio.com/items?itemName=tomoki1207.pdf) | Viewer | 14,587,973 | n/a | 4.47 (129) | PDF viewer | **CE** (`pdf.preview`, webview) (published manifest) | None | W (or host PDF component) |
| 28 | Live Preview (`ms-vscode.live-server`) — [item](https://marketplace.visualstudio.com/items?itemName=ms-vscode.live-server) | Web dev | 14,092,602 | 2,319,949 | 4.44 (79) | Embedded browser preview | WVp (embedded page) | None | W |
| 29 | indent-rainbow (`oderwat.indent-rainbow`) — [item](https://marketplace.visualstudio.com/items?itemName=oderwat.indent-rainbow) | Visual aid | 13,557,970 | 608,223 | 4.82 (143) | Colour indentation | **DEC only** ([package.json](https://github.com/oderwat/vscode-indent-rainbow/blob/HEAD/package.json)) | `indent_guides.coloring: "indent_aware"` (rainbow indentation), `background_coloring` ([docs](https://zed.dev/docs/configuring-zed)) | Z |
| 30 | markdownlint (`DavidAnson.vscode-markdownlint`) — [item](https://marketplace.visualstudio.com/items?itemName=DavidAnson.vscode-markdownlint) | Lint | 12,596,077 | n/a | 4.52 (86) | Markdown lint | LANG (diagnostics, fixes), tasks ([package.json](https://github.com/DavidAnson/vscode-markdownlint/blob/HEAD/package.json)) | Via LSP only | L |
| 31 | Better Comments (`aaron-bond.better-comments`) — [item](https://marketplace.visualstudio.com/items?itemName=aaron-bond.better-comments) | Visual aid | 10,808,691 | 807,270 | 4.83 (215) | Colour comment tags | **DEC only** ([package.json](https://github.com/aaron-bond/better-comments/blob/HEAD/package.json)) | None | D |
| 32 | Markdown Preview Enhanced (`shd101wyy.markdown-preview-enhanced`) — [item](https://marketplace.visualstudio.com/items?itemName=shd101wyy.markdown-preview-enhanced) | Markdown | 10,671,689 | 2,558,330 | 4.33 (145) | Rich preview (mermaid, math, export) | **WVp + CE** ([package.json](https://github.com/shd101wyy/vscode-markdown-preview-enhanced/blob/HEAD/package.json)) | Markdown preview (basic) | W (host markdown covers the basic case) |
| 33 | Error Lens (`usernamehw.errorlens`) — [item](https://marketplace.visualstudio.com/items?itemName=usernamehw.errorlens), [Open VSX](https://open-vsx.org/extension/usernamehw/errorlens) | Diagnostics display | 10,335,994 | 1,475,935 | **4.87 (190)** | Inline diagnostic messages | **DEC** (after-line text + line background), **GUT**, SB, optional CL ([package.json](https://github.com/usernamehw/vscode-error-lens/blob/HEAD/package.json)) | "show diagnostics information inline" (off by default) ([docs](https://zed.dev/docs/configuring-zed)) | Z (D if re-implemented) |
| 34 | SQL Server / mssql (`ms-mssql.mssql`) — [item](https://marketplace.visualstudio.com/items?itemName=ms-mssql.mssql) | Database | 10,016,515 | n/a | 3.17 (218) | SQL Server client | 5 TV + 1 WVv, **CE** (`mssql.queryStudio`, `executionPlanView`), notebook renderer, LANG (published manifest) | None | W (grid could be N table) |
| 35 | Color Highlight (`naumovs.color-highlight`) — [item](https://marketplace.visualstudio.com/items?itemName=naumovs.color-highlight) | Visual aid | 9,757,587 | 233,720 | 4.62 (108) | Colour swatches for CSS colours | **DEC only** (incl. before/after text) ([repo](https://github.com/enyancc/vscode-ext-color-highlight)) | LSP document colors (not verified) | D |
| 36 | Vim (`vscodevim.vim`) — [item](https://marketplace.visualstudio.com/items?itemName=vscodevim.vim) | Keymap | 9,200,449 | 2,017,247 | 4.09 (395) | Vim emulation | `type` command interception, DEC (cursor/visual), GUT, SB, QP ([package.json](https://github.com/VSCodeVim/Vim/blob/HEAD/package.json)) | Vim Mode ([docs](https://zed.dev/docs/vim.md)) | Z |
| 37 | Todo Tree (`Gruntfuggly.todo-tree`) — [item](https://marketplace.visualstudio.com/items?itemName=Gruntfuggly.todo-tree), [Open VSX](https://open-vsx.org/extension/Gruntfuggly/todo-tree) | Notes/TODO | 7,931,024 | 511,777 | 4.71 (217) | Tree of TODO/FIXME tags | **1 TV** (own view container), DEC, GUT, SB, QP, virtual doc ([package.json](https://github.com/Gruntfuggly/todo-tree/blob/HEAD/package.json)) | None (project search only) | N + D (tree native; highlights need DEC) |
| 38 | REST Client (`humao.rest-client`) — [item](https://marketplace.visualstudio.com/items?itemName=humao.rest-client) | HTTP | 7,760,292 | 411,946 | **4.91 (388)** | `.http` files, send requests | **CL** ("Send Request"), **WVp** (response), HOV, SB, QP, LANG, document links ([package.json](https://github.com/Huachao/vscode-restclient/blob/HEAD/package.json)) | None | D (CL) + N (response as host editor/markdown view instead of webview) |
| 39 | Thunder Client (`rangav.vscode-thunder-client`) — [item](https://marketplace.visualstudio.com/items?itemName=rangav.vscode-thunder-client) | HTTP | 7,714,931 | 3,176,178 | 2.35 (777) | Postman-like GUI | **1 WVv** + WVp (published manifest) | None | W |
| 40 | Project Manager (`alefragnani.project-manager`) — [item](https://marketplace.visualstudio.com/items?itemName=alefragnani.project-manager) | Navigation | 7,689,942 | 409,435 | 4.77 (145) | Saved/auto-detected projects | **7 TV**, QP, SB, FDEC ([package.json](https://github.com/alefragnani/vscode-project-manager/blob/HEAD/package.json)) | Recent-projects picker (not verified) | N |
| 41 | Kubernetes (`ms-kubernetes-tools.vscode-kubernetes-tools`) — [item](https://marketplace.visualstudio.com/items?itemName=ms-kubernetes-tools.vscode-kubernetes-tools) | Containers | 7,677,269 | n/a | 4.46 (41) | Cluster explorer | 3 TV, QP, LANG (published manifest) | None | N |
| 42 | Hex Editor (`ms-vscode.hexeditor`) — [item](https://marketplace.visualstudio.com/items?itemName=ms-vscode.hexeditor) | Viewer | 7,550,291 | n/a | 3.51 (61) | Binary editing | **CE** (`hexEditor.hexedit`) + 1 WVv (published manifest) | None | W (or N with virtualized table) |
| 43 | SQLTools (`mtxr.sqltools`) — [item](https://marketplace.visualstudio.com/items?itemName=mtxr.sqltools) | Database | 7,212,818 | 1,152,940 | 3.45 (146) | DB connections, queries | 4 TV, **WVp** (results grid), CL, DEC, SB, QP (published manifest; [repo](https://github.com/mtxr/vscode-sqltools)) | None | N (table) + D (CL) |
| 44 | Test Explorer UI (`hbenl.vscode-test-explorer`) — [item](https://marketplace.visualstudio.com/items?itemName=hbenl.vscode-test-explorer) | Testing | 5,675,648 | 222,179 | 4.41 (29) | Legacy test tree (now bridged to native Testing via Test Adapter Converter, 5,379,815 installs — [item](https://marketplace.visualstudio.com/items?itemName=ms-vscode.test-adapter-converter)) | 1 TV, DEC, GUT, CL, HOV ([package.json](https://github.com/hbenl/vscode-test-explorer/blob/HEAD/package.json)) | Tasks/runnables (no test explorer) | N + D |
| 45 | Import Cost (`wix.vscode-import-cost`) — [item](https://marketplace.visualstudio.com/items?itemName=wix.vscode-import-cost) | Visual aid | 5,601,447 | 113,558 | 3.03 (68) | Package size after import line | **DEC** (after-line text) ([repo](https://github.com/wix/import-cost)) | None | D (inline after-text) |
| 46 | Bookmarks (`alefragnani.Bookmarks`) — [item](https://marketplace.visualstudio.com/items?itemName=alefragnani.Bookmarks) | Navigation | 5,223,287 | 419,171 | 4.58 (201) | Line bookmarks | **GUT**, DEC, 2 TV, QP ([package.json](https://github.com/alefragnani/vscode-bookmarks/blob/HEAD/package.json)) | None | D (gutter) + N |
| 47 | Peacock (`johnpapa.vscode-peacock`) — [item](https://marketplace.visualstudio.com/items?itemName=johnpapa.vscode-peacock) | Visual aid | 4,521,029 | 249,677 | 4.59 (180) | Per-workspace window colour | Writes `workbench.colorCustomizations` (18 files), SB, QP ([package.json](https://github.com/johnpapa/vscode-peacock/blob/HEAD/package.json)) | Theme overrides (manual) | N (needs workspace theme-override capability) |
| 48 | Draw.io Integration (`hediet.vscode-drawio`) — [item](https://marketplace.visualstudio.com/items?itemName=hediet.vscode-drawio) | Diagrams | 4,299,486 | 315,080 | **4.91 (161)** | Edit .drawio files | **4 CE** (webview) ([package.json](https://github.com/hediet/vscode-drawio/blob/HEAD/package.json)) | None | W |
| 49 | Image preview (`kisstkondoros.vscode-gutter-preview`) — [item](https://marketplace.visualstudio.com/items?itemName=kisstkondoros.vscode-gutter-preview) | Visual aid | 4,322,475 | 182,501 | 4.83 (60) | Image thumbnails in gutter/hover | **GUT + HOV** ([repo](https://github.com/kisstkondoros/gutter-preview)) | None | D |
| 50 | Jest (`Orta.vscode-jest`) — [item](https://marketplace.visualstudio.com/items?itemName=Orta.vscode-jest) | Testing | 3,919,377 | n/a | 3.29 (108) | Jest runner | **TEST** (4 files), DEC, GUT, CL, HOV, SB, QP, WVp ([package.json](https://github.com/jest-community/vscode-jest/blob/HEAD/package.json)) | Tasks/runnables | D + test-tree |
| 51 | SQLite Viewer (`qwtel.sqlite-viewer`) — [item](https://marketplace.visualstudio.com/items?itemName=qwtel.sqlite-viewer) | Database | 3,778,785 | n/a | 3.87 (86) | Browse .sqlite | **3 CE** (webview) (published manifest) | None | W (or N table as custom editor) |
| 52 | Playwright Test (`ms-playwright.playwright`) — [item](https://marketplace.visualstudio.com/items?itemName=ms-playwright.playwright) | Testing | 3,238,986 | n/a | 4.02 (43) | Playwright runner | **TEST**, **2 WVv** in panel container, DEC, HOV ([package.json](https://github.com/microsoft/playwright-vscode/blob/HEAD/package.json)) | None | D + W (trace viewer) |
| 53 | vscode-pets (`tonybaloney.vscode-pets`) — [item](https://marketplace.visualstudio.com/items?itemName=tonybaloney.vscode-pets) | Fun | 2,777,908 | 220,751 | **4.92 (125)** | Animated pets | **1 WVv** ([package.json](https://github.com/tonybaloney/vscode-pets/blob/HEAD/package.json)) | None | W (or canvas primitive + `request-tick`) |
| 54 | Jupyter (`ms-toolsai.jupyter`) — [item](https://marketplace.visualstudio.com/items?itemName=ms-toolsai.jupyter) | Notebooks | 111,471,531 (+ Renderers 90,237,801) | 11,587,333 | 2.63 (376) | Notebooks | **NB**, `notebookRenderer` (webview), 1 WVv (variables) (published manifest) | REPL ([docs](https://zed.dev/docs/repl.md)) | Z (partial) / W |
| 55 | Cline (`saoudrizwan.claude-dev`) — [item](https://marketplace.visualstudio.com/items?itemName=saoudrizwan.claude-dev) | AI agent | 5,964,844 | 7,458,370 | 3.97 (328) | Agent UI | **1 WVv** (published manifest) | Agent Panel | W |
| 56 | SonarQube for IDE (`SonarSource.sonarlint-vscode`) — [item](https://marketplace.visualstudio.com/items?itemName=SonarSource.sonarlint-vscode) | Lint | 4,792,784 | 2,281,502 | 4.14 (134) | Static analysis | LANG, 6 TV + 3 WVv (published manifest) | Via LSP | L + N |
| 57 | Database Client / MySQL (`cweijan.vscode-mysql-client2`) — [item](https://marketplace.visualstudio.com/items?itemName=cweijan.vscode-mysql-client2) | Database | 3,060,703 | 566,091 (`cweijan.vscode-database-client2`) | 4.21 (270) | Multi-DB client | 4 TV + 1 WVv, 2 CE, 2 notebook types (published manifest) | None | W (grid) / N (tree) |

Other notable counts (Marketplace, 2026-11-17, same API): Postman 2,754,922 (2 WVv) — [item](https://marketplace.visualstudio.com/items?itemName=Postman.postman-for-vscode); MongoDB 3,248,688 (3 TV) — [item](https://marketplace.visualstudio.com/items?itemName=mongodb.mongodb-vscode); Edit CSV 2,556,781 — [item](https://marketplace.visualstudio.com/items?itemName=janisdd.vscode-edit-csv); Spreadsheet Viewer 6,855,896 (2 CE) — [item](https://marketplace.visualstudio.com/items?itemName=GrapeCity.gc-excelviewer); GitLab 3,539,024 (5 TV + 4 WVv) — [item](https://marketplace.visualstudio.com/items?itemName=GitLab.gitlab-workflow); TODO Highlight 6,198,472 (DEC only) — [item](https://marketplace.visualstudio.com/items?itemName=wayou.vscode-todo-highlight); Bracket Pair Colorizer 7,843,024 and BPC2 6,768,573 (deprecated; built into VS Code) — [item](https://marketplace.visualstudio.com/items?itemName=CoenraadS.bracket-pair-colorizer-2); Numbered Bookmarks 291,010 — [item](https://marketplace.visualstudio.com/items?itemName=alefragnani.numbered-bookmarks); Continue 4,575,500 (2 WVv) — [item](https://marketplace.visualstudio.com/items?itemName=Continue.continue); Windsurf/Codeium 3,977,513 — [item](https://marketplace.visualstudio.com/items?itemName=Codeium.codeium); Gemini Code Assist 5,706,454 — [item](https://marketplace.visualstudio.com/items?itemName=Google.geminicodeassist); Vitest 1,254,065 — [item](https://marketplace.visualstudio.com/items?itemName=vitest.explorer); Foam 288,685 — [item](https://marketplace.visualstudio.com/items?itemName=foam.foam-vscode).

**Excluded categories (noted, Marketplace installs 2026-11-17)** — language support: Python 245,826,174, Pylance 212,439,370, Python Debugger 151,213,394, C/C++ 107,157,831, Red Hat Java 58,718,366, C# 44,426,280, Go 20,158,563, rust-analyzer 6,876,217; language packs: Chinese (Simplified) 57,267,431, Japanese 15,956,014, Spanish 12,343,883, Russian 9,383,770; icon/colour themes: Material Icon Theme 36,260,427 (4.94★, 401), vscode-icons 24,838,504, GitHub Theme 20,656,497, One Dark Pro 12,933,102, Dracula 11,156,484 — all from the same Marketplace query ([Python item](https://marketplace.visualstudio.com/items?itemName=ms-python.python), [Material Icon Theme item](https://marketplace.visualstudio.com/items?itemName=PKief.material-icon-theme)).

**Open VSX top by downloads (2026-11-17)** — [search API](https://open-vsx.org/api/-/search?sortBy=downloadCount&sortOrder=desc&size=80): Pyrefly 95,819,327; Claude Code 67,330,356; Python Debugger 64,034,052; Python 62,835,634; Red Hat Java 48,401,128; Python Environments 47,180,723; Ruby LSP 44,893,091; DEVSENSE PHP/Composer/Profiler ~44M each; Go 43,628,179; clangd 31,969,333; GitLens 19,499,809; Container Tools 15,145,568; Codex 13,698,291; Jupyter 11,587,333; Prettier 9,971,196; Cline 7,458,370; Docker 7,358,122; GitHub PRs 6,757,430; ESLint 6,723,105; Kilo Code 4,818,974; Rainbow CSV 3,490,323; Live Server 3,179,359; Thunder Client 3,176,178; Code Spell Checker 2,265,882; Live Preview 2,319,949.
- Microsoft's Remote - SSH and Live Share were not found on Open VSX (API returned no record for `ms-vscode-remote/remote-ssh` and `MS-vsliveshare/vsliveshare`); the OSS `jeanp413.open-remote-ssh` has 1,236,194 downloads — [Open VSX](https://open-vsx.org/extension/jeanp413/open-remote-ssh).
- Several Open VSX copies are stale: e.g. Todo Tree last published 2022-02-08, Git Graph 2021-04-17, REST Client 2022-06-21, Draw.io 2023-01-19 (API `timestamp` field) — [Todo Tree on Open VSX](https://open-vsx.org/extension/Gruntfuggly/todo-tree), [Git Graph on Open VSX](https://open-vsx.org/extension/mhutchie/git-graph).

**"Most loved" (high rating with ≥100 reviews), Marketplace 2026-11-17:** Material Icon Theme 4.94 (401), Git Graph 4.93 (681), REST Client 4.91 (388), Draw.io 4.91 (161), autoDocstring 4.91 (68), Error Lens 4.87 (190), PlatformIO 4.86 (4151), Path Intellisense 4.85 (124), Better Comments 4.83 (215), indent-rainbow 4.82 (143), Rainbow CSV 4.79 (163), vscode-icons 4.77 (465), Project Manager 4.77 (145), Todo Tree 4.71 (217). Lowest-rated high-install tools: Thunder Client 2.35 (777), Codex 2.64 (941), Copilot Chat 2.74 (423), Gemini Code Assist 1.86 (921) — same API query; item pages e.g. [Git Graph](https://marketplace.visualstudio.com/items?itemName=mhutchie.git-graph), [Thunder Client](https://marketplace.visualstudio.com/items?itemName=rangav.vscode-thunder-client).

### Inferences
- Marketplace "installs" are cumulative and inflated by extension packs and dependencies (e.g. Jupyter Keymap 91M, Jupyter Cell Tags 71.5M and Jupyter Slide Show 71.4M ride on Jupyter; Docker 2.0 now auto-installs Container Tools). Rank by installs ≠ rank by active use.
- The "most loved" list is disproportionately small, focused tools that use plain decorations/gutter/tree (Error Lens, Better Comments, indent-rainbow, Todo Tree, Bookmarks, Project Manager) plus a few webview "apps" (Git Graph, Draw.io). The decoration-only ones are the cheapest big win for Fyx.
- Open VSX matters for Fyx's VS Code adapter: it is the registry a non-Microsoft fork can legally use, and Microsoft's remote/Live Share/Pylance/C# Dev Kit are absent or replaced there.

### Gaps
- Marketplace exposes no active-user or uninstall numbers; installs ≠ users.
- Whether Zed bundles the ESLint server, supports WSL remotes, has a recent-projects picker, or has a WakaTime extension was not verified this session (marked "not verified" in the table).
- Live Share, Thunder Client, Remote-SSH, Copilot, Claude Code sources are closed or partial; their UI was inferred from the published manifest only, not source.

---

## Q2. Which VS Code contribution points / UI APIs do these extensions use, and what share requires a webview?

### Takeaway
Across the 57-row sample, the commonest surfaces are status bar (~36% of extensions), decorations (~32%), tree views
(~32%) and quick pick (~30%); webviews appear in ~39% of extensions but are **essential** to only ~30% of them
(~25% of sample installs). Inline decorations, gutter, CodeLens and hover cluster in the small, highly rated
"visual aid" and testing extensions; inlay hints are almost unused by top non-language extensions (1 of 57).

### Cited Findings
- VS Code UI containers exposed to extensions: Activity Bar, Primary/Secondary Sidebar, Editor (custom editors/webviews),
  Panel, Status Bar; items: Tree Views, Welcome Views, Webview Views, view/editor/panel toolbars, Status Bar Items;
  common elements: Command Palette, Quick Picks, Notifications, Webviews, Context Menus, Walkthroughs, Settings —
  [VS Code UX Guidelines overview](https://code.visualstudio.com/api/ux-guidelines/overview).
- Manifest view-type counts (from `contributes.views[*].type`, default "tree"): GitLens 16 tree + 5 webview + custom
  editor `gitlens.rebase` ([package.json](https://github.com/gitkraken/vscode-gitlens/blob/HEAD/package.json)); GitHub
  PRs 9 tree + 2 webview ([package.json](https://github.com/microsoft/vscode-pull-request-github/blob/HEAD/package.json));
  Code Spell Checker 3 tree + 1 webview ([package.json](https://github.com/streetsidesoftware/vscode-spell-checker/blob/HEAD/package.json));
  Project Manager 7 tree ([package.json](https://github.com/alefragnani/vscode-project-manager/blob/HEAD/package.json));
  Bookmarks 2 tree ([package.json](https://github.com/alefragnani/vscode-bookmarks/blob/HEAD/package.json)); Todo Tree 1
  tree ([package.json](https://github.com/Gruntfuggly/todo-tree/blob/HEAD/package.json)); Container Tools 7 tree, 0
  webview ([repo](https://github.com/microsoft/vscode-containers)); SQLTools 4 tree ([repo](https://github.com/mtxr/vscode-sqltools));
  Playwright 2 webview views in a panel container ([package.json](https://github.com/microsoft/playwright-vscode/blob/HEAD/package.json));
  vscode-pets 1 webview view ([package.json](https://github.com/tonybaloney/vscode-pets/blob/HEAD/package.json));
  Claude Code 3 webview views, Cline 1, Continue 2, Thunder Client 1, Postman 2, Codex 2 + custom editor (published
  manifests via the Marketplace API; items: [Claude Code](https://marketplace.visualstudio.com/items?itemName=anthropic.claude-code),
  [Cline](https://marketplace.visualstudio.com/items?itemName=saoudrizwan.claude-dev),
  [Thunder Client](https://marketplace.visualstudio.com/items?itemName=rangav.vscode-thunder-client)).
- Custom editors (all webview-based): Draw.io ×4, Markdown Preview Enhanced, vscode-pdf, Hex Editor, SQLite Viewer ×3,
  Spreadsheet Viewer ×2, mssql ×2 (`queryStudio`, `executionPlanView`), Database Client ×2, LaTeX Workshop PDF, GitLens
  rebase, Codex conversation — [Draw.io package.json](https://github.com/hediet/vscode-drawio/blob/HEAD/package.json),
  [MPE package.json](https://github.com/shd101wyy/vscode-markdown-preview-enhanced/blob/HEAD/package.json),
  [LaTeX Workshop package.json](https://github.com/James-Yu/LaTeX-Workshop/blob/HEAD/package.json), others from
  published manifests ([SQLite Viewer](https://marketplace.visualstudio.com/items?itemName=qwtel.sqlite-viewer),
  [Hex Editor](https://marketplace.visualstudio.com/items?itemName=ms-vscode.hexeditor)).
- Webview *panels* (not declared in the manifest, found by `createWebviewPanel` grep): Git Graph (3 files), Git History,
  REST Client (response view, 2 files), GitLens, GitHub PRs, MongoDB (3), SQLTools (results), Jest, Rainbow CSV (RBQL),
  Peacock (1), MPE (3) — repos: [Git Graph](https://github.com/mhutchie/vscode-git-graph),
  [REST Client](https://github.com/Huachao/vscode-restclient), [MongoDB](https://github.com/mongodb-js/vscode),
  [SQLTools](https://github.com/mtxr/vscode-sqltools), [Jest](https://github.com/jest-community/vscode-jest).
- Decorations-only extensions (no views at all; `contributes` is just `configuration`): indent-rainbow
  ([package.json](https://github.com/oderwat/vscode-indent-rainbow/blob/HEAD/package.json)), Better Comments
  ([package.json](https://github.com/aaron-bond/better-comments/blob/HEAD/package.json)); Color Highlight and TODO
  Highlight have only commands + configuration ([Color Highlight repo](https://github.com/enyancc/vscode-ext-color-highlight),
  [TODO Highlight repo](https://github.com/wayou/vscode-todo-highlight)); Import Cost uses after-line `contentText`
  decorations ([repo](https://github.com/wix/import-cost)).
- Gutter icons (`gutterIconPath`) found in: Error Lens, Bookmarks, Todo Tree, GitLens (2 files), Test Explorer, Jest
  (3), Image preview, Vim — repos e.g. [Error Lens](https://github.com/usernamehw/vscode-error-lens),
  [Image preview](https://github.com/kisstkondoros/gutter-preview).
- CodeLens providers found in: GitLens, REST Client, SQLTools, MongoDB (playgrounds), Jest (3 files), Test Explorer,
  Import Cost, Markdown All in One, Draw.io, Error Lens, Cline — e.g. [REST Client](https://github.com/Huachao/vscode-restclient).
- Inlay hint providers found only in Rainbow CSV, Jest and Cline among the 37 repos —
  [Rainbow CSV](https://github.com/mechatroner/vscode_rainbow_csv).
- Testing API (`createTestController`) in Jest (4 files), Playwright (2), GitLens, Cline; Comments API
  (`createCommentController`) in GitHub PRs (2), GitLens (3), Jest, Copilot Chat —
  [Playwright repo](https://github.com/microsoft/playwright-vscode), [GitHub PRs repo](https://github.com/microsoft/vscode-pull-request-github).
- Quick pick is pervasive: GitLens 66 source files, Copilot Chat 22, Container Tools 21, Jest 13, GitHub PRs 12 —
  [GitLens repo](https://github.com/gitkraken/vscode-gitlens), [vscode-copilot-chat repo](https://github.com/microsoft/vscode-copilot-chat).
- Progress notifications (`withProgress`): GitLens 35 files, Container Tools 25, GitHub PRs 20, Copilot Chat 19 —
  same repos.
- Peacock's core mechanism is writing `workbench.colorCustomizations` (18 source files) rather than any view —
  [Peacock repo](https://github.com/johnpapa/vscode-peacock).
- Test Explorer UI (5.68M) depends on `ms-vscode.test-adapter-converter` (5.38M), i.e. the legacy tree is bridged to the
  native Testing API — published manifest of [Test Explorer UI](https://marketplace.visualstudio.com/items?itemName=hbenl.vscode-test-explorer).

**Computed shares over the 57-row table (56 entries with installs; Docker+Container Tools counted once; 1.268B summed
installs). Classification is mine, from the manifest + grep evidence above:**

| Surface | # extensions | % of extensions | % of summed installs |
|---|---|---|---|
| Status bar item | 20 | 35.7% | 47.4% |
| Quick pick / input | 17 | 30.4% | 35.1% |
| Tree view | 18 | 32.1% | 33.6% |
| Text decorations (inline style / after-text / line bg) | 18 | 32.1% | 25.9% |
| Webview (any: view, panel, custom editor, renderer) | 22 | 39.3% | 30.8% |
| — webview **essential** to the product | 17 | 30.4% | 24.8% |
| Custom editor | 9 | 16.1% | 9.8% |
| Language providers (diagnostics/completion/code actions/format) | 13 | 23.2% | 28.2% |
| Hover provider | 7 | 12.5% | 10.9% |
| Gutter icons | 7 | 12.5% | 7.2% |
| CodeLens | 6 | 10.7% | 7.4% |
| Notebook API | 3 | 5.4% | 9.8% |
| File (explorer) decorations | 3 | 5.4% | 7.8% |
| Testing API | 2 | 3.6% | 0.6% |
| Comments API | 1 | 1.8% | 3.0% |
| Inlay hints | 1 | 1.8% | 2.0% |
| Zed already has an equivalent built in | 15 | 26.8% | 49.3% |

### Inferences
- "Webview-essential" extensions fall into four buckets: (1) **AI agent UIs** (Claude Code, Codex, Cline, Continue —
  React apps in a webview view); (2) **viewers/editors for non-text formats** (PDF, hex, SQLite, Draw.io, spreadsheet,
  notebook renderers); (3) **graph/visual git** (Git Graph, Git History log); (4) **GUI clients** (Thunder Client,
  Postman, DB result grids, embedded browser preview). Bucket 1 is already covered by Zed's agent panel/ACP; buckets
  3–4 could mostly move to a native virtualized table + a "graph lane" cell renderer; bucket 2 genuinely needs either a
  host component (PDF/image/hex) or a webview.
- Many big extensions use a webview for a *secondary* screen (GitLens Home/Graph, PR description, REST Client response,
  SQLTools results, Spell Checker info view). For an adapter, degrading those screens to host markdown/table/editor
  views keeps the core value.
- Inlays are a weak priority from this evidence; CodeLens and after-line text decorations matter more (Error Lens,
  Import Cost, GitLens blame, REST Client "Send Request", test run/debug lenses).

### Gaps
- Grep found 0 hits in Code Spell Checker's repo (its client code is in a monorepo layout the glob missed), and ESLint/
  Prettier show only LSP-client code; their per-API usage is under-counted.
- Webview usage of closed-source extensions (Live Share, Thunder Client panels, Copilot) is known only from manifests.
- The "essential vs optional webview" split and the per-extension need sets are judgement calls (documented in the
  table), not measured.

---

## Q3. Most popular JetBrains plugins and their UI needs

### Takeaway
JetBrains' top downloads are dominated by bundled/first-party plugins (AI Assistant 259M, Kotlin, Docker, GitHub,
GitLab, HTTP Client, Jupyter). The top third-party "loved" plugins are small editor-augmentation tools — Rainbow Brackets
(25.3M), .ignore (20.0M), .env files (33.2M), GitToolBox (10.8M), Translation (10.9M), Key Promoter X (8.3M, 4.94★),
String Manipulation (5.1M, 4.93★), CodeGlance Pro (4.7M), Grep Console (3.4M) — whose needs map to highlighting/
decorations, gutter icons, popups/balloons, status bar and tool windows, not to embedded web UIs.

### Cited Findings
- Top JetBrains Marketplace plugins by downloads (2026-11-17, [API](https://plugins.jetbrains.com/api/searchPlugins?orderBy=downloads&max=20)):
  JetBrains AI Assistant 259,377,234 (2.12★) — [page](https://plugins.jetbrains.com/plugin/22282); GitHub Copilot
  57,508,660 — [page](https://plugins.jetbrains.com/plugin/17718); Subversion 53,134,805; Qoder CN (Lingma) 42,597,298;
  Chinese language pack 40,305,875; Kotlin 38,136,671; Junie 38,130,623; Docker 34,124,610 — [page](https://plugins.jetbrains.com/plugin/7724);
  **.env files 33,182,898 (4.80★)** — [page](https://plugins.jetbrains.com/plugin/9525); GitHub 29,027,354; GitLab
  28,285,545; Lombok 27,540,604; **Rainbow Brackets 25,268,487 (4.58★)** — [page](https://plugins.jetbrains.com/plugin/10080);
  IdeaVim 22,596,322 — [page](https://plugins.jetbrains.com/plugin/164); MCP Server 21,729,919; **.ignore 19,972,090** —
  [page](https://plugins.jetbrains.com/plugin/7495); Kubernetes 19,910,235; Dev Containers 19,461,153; Material Theme UI
  19,041,103; Jupyter 18,532,044; HTTP Client 16,845,269 — [page](https://plugins.jetbrains.com/plugin/13121); SonarQube
  for IDE 16,347,725; CSV Editor 13,945,286 (4.54★) — [page](https://plugins.jetbrains.com/plugin/10037); Ideolog
  13,076,055; **Translation 10,942,661 (4.89★)** — [page](https://plugins.jetbrains.com/plugin/8579); **GitToolBox
  10,803,401 (4.64★)** — [page](https://plugins.jetbrains.com/plugin/7499); Windsurf 10,427,387; Atom Material Icons
  10,221,244; Prettier 8,742,003; **Key Promoter X 8,280,838 (4.94★)** — [page](https://plugins.jetbrains.com/plugin/9792);
  CheckStyle-IDEA 7,227,963; JPA Buddy 6,900,210 (4.91★).
- Further third-party plugins (same API, search by name): MavenHelper 6,311,107 (4.90★) — [page](https://plugins.jetbrains.com/plugin/7179);
  MyBatisX 5,617,098 — [page](https://plugins.jetbrains.com/plugin/10119); StringManipulation 5,085,201 (4.93★) —
  [page](https://plugins.jetbrains.com/plugin/2162); plantuml4idea 5,102,425 — [page](https://plugins.jetbrains.com/plugin/7017);
  Database Navigator 5,062,381 — [page](https://plugins.jetbrains.com/plugin/1800); CodeGlance Pro 4,725,865 (4.86★) —
  [page](https://plugins.jetbrains.com/plugin/18824); Grep Console 3,381,390 (4.92★) — [page](https://plugins.jetbrains.com/plugin/7125);
  Statistic 3,340,813 — [page](https://plugins.jetbrains.com/plugin/4509); Rainbow CSV 1,837,742 — [page](https://plugins.jetbrains.com/plugin/12896);
  Nyan Progress Bar 1,559,707 (4.92★) — [page](https://plugins.jetbrains.com/plugin/8575); AceJump 1,288,503 —
  [page](https://plugins.jetbrains.com/plugin/7086); Indent Rainbow 1,108,055 — [page](https://plugins.jetbrains.com/plugin/13308);
  Extra ToolWindow Colorful Icons 1,885,484 (4.93★).
- UI needs from plugin descriptions:
  - Rainbow Brackets: colours brackets, variables, indent guides, XML/HTML tag names, YAML/JSON property names, "Scope
    highlighting", "Current Block Highlighting" → range text colouring/background decorations —
    [page](https://plugins.jetbrains.com/plugin/10080). Zed shipped native rainbow brackets (`colorize_brackets`, stable
    Dec 2025) — [Zed blog](https://zed.dev/blog/rainbow-brackets).
  - Key Promoter X: shows the shortcut you should have used when you click with the mouse, plus a tool window of most-used
    mouse actions, and can prompt to create a shortcut → notifications/balloons + a tool-window table + an action-
    invocation hook — [page](https://plugins.jetbrains.com/plugin/9792).
  - StringManipulation: case switching, sorting, filtering, aligning, escaping/encoding → editor actions on selections +
    popup menu (quick pick) — [page](https://plugins.jetbrains.com/plugin/2162).
  - GitToolBox: ahead/behind and uncommitted counts "in project view and status bar", "current line blame in editor and
    status bar", commit-message completion, auto-fetch → file-tree decorations, status bar, after-line text, completion,
    timers — [page](https://plugins.jetbrains.com/plugin/7499).
  - .ignore: language support for dozens of ignore-file formats → syntax/completion and (per plugin) ignored-file colouring
    in the project view — [page](https://plugins.jetbrains.com/plugin/7495).
  - Grep Console: colours/filters/folds console output, can also "highlight the editor" → terminal/console output
    decorations — [page](https://plugins.jetbrains.com/plugin/7125).
  - CodeGlance Pro: minimap in the editor pane with markup, error-stripe, VCS and caret highlights → custom editor-side
    component — [page](https://plugins.jetbrains.com/plugin/18824). (Zed has a built-in minimap setting —
    [docs](https://zed.dev/docs/configuring-zed).)
  - Translation: multiple engines, TTS, word book → popups/balloons and a tool window — [page](https://plugins.jetbrains.com/plugin/8579).
  - MyBatisX: "mapper and xml can jump back and forth" → gutter navigation icons + completion — [page](https://plugins.jetbrains.com/plugin/10119).
  - MavenHelper: dependency-conflict analyzer and "Run Maven Goal" popup → editor tab with tree/table + quick pick —
    [page](https://plugins.jetbrains.com/plugin/7179).
  - Statistic: files sorted by extension with size/LOC → tool-window table — [page](https://plugins.jetbrains.com/plugin/4509).
  - HTTP Client (JetBrains-bundled): compose/execute requests from the editor, completion, response handling, gRPC/
    WebSocket/GraphQL → gutter run icons + response pane — [page](https://plugins.jetbrains.com/plugin/13121).
  - AceJump: jump to any visible position by typing a character then a label → overlay labels on text (inline
    decorations) + keyboard capture — [page](https://plugins.jetbrains.com/plugin/7086).
  - Nyan Progress Bar: replaces progress bar painting — purely cosmetic custom painting — [page](https://plugins.jetbrains.com/plugin/8575).

### Inferences
- JetBrains' loved plugins confirm the VS Code pattern: the highest satisfaction comes from small, precise editor
  augmentations (colouring, gutter marks, inline blame, popups) rather than web UIs. None of the third-party top
  JetBrains plugins listed requires an embedded browser; PlantUML/Markdown previews are the exception (image/HTML preview).
- Several JetBrains favourites map directly to fyx:ui primitives already planned: tool window ≈ dock panel; table ≈
  virtualized table; popup ≈ quick pick/modal; balloons ≈ notifications (not in the Fyx primitive list — gap); status bar
  widget ≈ status-bar item.

### Gaps
- JetBrains Marketplace has no public "rating count" in this API response, so "loved" is approximated by rating + downloads.
- UI surfaces for JetBrains plugins are inferred from descriptions, not source code.
- The `searchPlugins` API rejects `max` > 20, so the top list was paged 20 at a time (top 80 fetched).

---

## Q4. What did VS Code's API make easy vs. hard, and why does it restrict DOM access?

### Takeaway
VS Code deliberately gives extensions no DOM/CSS access and runs them in a separate Extension Host process for stability
and performance; anything beyond native trees/decorations/quick picks must go through webviews, which VS Code itself
calls resource-heavy and a last resort. This is the same trade-off Fyx makes (declarative tree, theme tokens only, no
webview), and the evidence shows VS Code's native set already covers the majority of the most-loved extensions.

### Cited Findings
- "Extensions have no access to the DOM of VS Code UI"; "You cannot write an extension that applies custom CSS to VS Code
  or adds an HTML element to VS Code UI"; reason: "To ensure that extensions cannot interfere with the stability and
  performance of VS Code" they run in an Extension Host process; "A custom style sheet … would work against the DOM
  structure and class names" — [Extension Capabilities overview](https://code.visualstudio.com/api/extension-capabilities/overview).
- For a "fully customized user interface, use the Webview API" — [same page](https://code.visualstudio.com/api/extension-capabilities/overview).
- "Webviews are pretty amazing, but they should also be used sparingly and only when VS Code's native API is
  inadequate"; "Webviews are resource heavy and run in a separate context from normal extensions"; "A poorly designed
  webview can also easily feel out of place within VS Code"; "Just because you can do something with webviews, doesn't
  mean you should" — [Webview API guide](https://code.visualstudio.com/api/extension-guides/webview).
- `retainContextWhenHidden` "has high memory overhead"; `getState`/`setState` "have much lower performance overhead" —
  [Webview API guide](https://code.visualstudio.com/api/extension-guides/webview).
- UX guideline Do: "Only use webviews when absolutely necessary", "Ensure all elements in the view are themeable",
  "Ensure your views follow accessibility guidance"; Don't: use for promotions, wizards, open on every window, open on
  updates, "Repeat existing functionality" — [UX Guidelines: Webviews](https://code.visualstudio.com/api/ux-guidelines/webviews).
- VS Code exposes a fixed set of containers/items (tree views, webview views, status bar items, quick picks,
  notifications, context menus, walkthroughs) — [UX Guidelines overview](https://code.visualstudio.com/api/ux-guidelines/overview).
- Evidence of native-API gaps pushing extensions to webviews: GitLens ships 5 webview views and a webview custom editor
  for interactive rebase alongside 16 tree views ([package.json](https://github.com/gitkraken/vscode-gitlens/blob/HEAD/package.json));
  Git Graph is a single webview panel ([repo](https://github.com/mhutchie/vscode-git-graph)).
- Evidence of native-API absorption: bracket pair colorization became built in and the Bracket Pair Colorizer 2 listing
  is titled "[Deprecated]" (6,768,573 installs) — [item](https://marketplace.visualstudio.com/items?itemName=CoenraadS.bracket-pair-colorizer-2);
  Test Explorer UI is bridged to the native Testing API via Test Adapter Converter —
  [item](https://marketplace.visualstudio.com/items?itemName=ms-vscode.test-adapter-converter).
- Zed parallel: Zed added native rainbow brackets (`colorize_brackets`, preview 0.215.0, stable Dec 2025) —
  [Zed blog](https://zed.dev/blog/rainbow-brackets); LSP CodeLens display (`code_lens`: off/on/menu) —
  [Zed settings docs](https://zed.dev/docs/configuring-zed), shipped disabled by default per
  [Zed 1.1.5 release notes](https://zed.dev/releases/stable/1.1.5); rainbow indent guides
  (`indent_guides.coloring: "indent_aware"`) and inline diagnostics — [Zed settings docs](https://zed.dev/docs/configuring-zed).
- Zed's documented extension types are languages, themes, icon themes, snippets, debuggers and MCP servers — no UI/panel
  extension type — [Zed extensions docs](https://zed.dev/docs/extensions).

### Inferences
- Easy in VS Code (and therefore what most popular extensions use): status bar items, quick picks, tree views, text
  decorations with gutter icons and before/after text, CodeLens, hover, language providers. Hard (forcing webviews):
  graphs/canvases, data grids with editing, forms with many fields, non-text file viewers, chat UIs with rich streaming
  content.
- VS Code's "decoration type" model (style object + ranges) is the single most leveraged primitive among loved
  extensions; Fyx's block-decoration-only plan misses the inline/after-line/range-style half of it.
- The trend in both VS Code and Zed is to absorb popular decoration extensions into the core (bracket colouring, inline
  diagnostics, rainbow indent, code lens, minimap) — so extension demand shifts toward workflow tools (git graph, DB,
  HTTP, TODO tree, bookmarks, test explorers).

### Gaps
- I found no VS Code-team post-mortem quantifying webview usage across the Marketplace; the "~30% webview-essential"
  figure is from this sample only.
- No primary source checked for VS Code's reasoning on CodeLens/inlay design.

---

## Q5. Which UI primitives unlock the largest share of these extensions for Fyx (native vs. new primitives vs. webview)?

### Takeaway
Excluding the 15 extensions Zed already covers (≈49% of sample installs), Fyx's current/planned set (dock panel + tree/
list/table + status bar + quick pick) natively hosts only ~4 of 41 remaining extensions (~22% of their installs). The
biggest unlocks, in greedy order, are: a **language-provider bridge** (diagnostics/completion/code actions; +7 exts,
→42.5%), **webview** (+11 exts, →58.8%), remote/core hooks, then **inline decorations** (range styling + after-line text),
**CodeLens**, **gutter marks**, **hover**, **file-tree decorations** (unlocks GitLens core + Project Manager, →87%),
Comments API, inlays, notebooks, testing.

### Cited Findings
- Greedy coverage computed over the 41 non-Zed-builtin rows of the Q1 table (needs as classified there; process/network,
  keybinding and settings needs treated as non-UI capabilities; webview counted only when essential):

| Step | Primitive added | Extensions hostable | % of installs (of 41-row pool) |
|---|---|---|---|
| 0 | Dock panel + tree/list/table + status bar + quick pick | 4/41 (Live Server, Auto Rename Tag, WakaTime, Peacock) | 21.5% |
| 1 | + language-provider bridge (diagnostics, completion, code actions) | 11/41 (+Docker/Containers, Path Intellisense, Spell Checker, autoDocstring, markdownlint, Kubernetes, …) | 42.5% |
| 2 | + webview (views, panels, custom editors) | 22/41 (+Git Graph, Git History, Thunder Client, PDF, Hex, Draw.io, SQLite Viewer, MPE, Live Preview, pets, Cline…) | 58.8% |
| 3 | + remote/core integration (WSL) | 23/41 | 65.2% |
| 4 | + inline decorations (range style, after-line text, line background) | 26/41 (+Better Comments, Color Highlight, Import Cost) | 69.3% |
| 5 | + CodeLens | 28/41 (+Test Explorer, Markdown All in One) | 72.7% |
| 6 | + gutter marks | 30/41 (+Todo Tree, Bookmarks) | 74.7% |
| 7 | + hover | 33/41 (+Image preview, REST Client, SQLTools) | 77.5% |
| 8 | + file (explorer) decorations | 35/41 (+GitLens core, Project Manager) | 87.1% |
| 9 | + Comments API | 36/41 (+GitHub PRs minus PR webview) | 92.9% |
| 10 | + inlay hints | 37/41 (+Rainbow CSV) | 96.9% |
| 11 | + notebooks | 39/41 (mssql, Database Client) | 98.9% |
| 12 | + Testing API | 41/41 (Jest, Playwright) | 100% |

  (Computed from the install counts and surface evidence cited in Q1/Q2; sources: [Marketplace items](https://marketplace.visualstudio.com/items?itemName=eamodio.gitlens)
  and the per-extension `package.json` links above.)
- If, after step 1, the decoration family (DEC + GUT + CL + HOV + FDEC) is added *instead of* webview, 12 more
  extensions become hostable without any web surface (11 → 23 of 41): Better Comments, Color Highlight, Import Cost,
  Todo Tree, Bookmarks, Image preview, Test Explorer, Markdown All in One, REST Client (response as host editor view),
  SQLTools (results as table), GitLens (minus Graph/Home webviews), Project Manager — same computation.
- Zed already covers: Copilot/IntelliCode/Copilot Chat (Edit Prediction, Agent Panel — [docs](https://zed.dev/docs/ai/edit-prediction.md),
  [docs](https://zed.dev/docs/ai/agent-panel.md)), Prettier and inline diagnostics and rainbow indent guides and minimap
  and code lens ([settings docs](https://zed.dev/docs/configuring-zed)), Remote-SSH ([docs](https://zed.dev/docs/remote-development)),
  Dev Containers ([docs](https://zed.dev/docs/dev-containers)), Live Share ([channels](https://zed.dev/docs/collaboration/channels.md)),
  Vim ([docs](https://zed.dev/docs/vim.md)), Code Runner (Tasks — [docs](https://zed.dev/docs/tasks.md)), Jupyter
  partially (REPL — [docs](https://zed.dev/docs/repl.md)), core git (panel, inline blame, file history, stash, conflicts —
  [docs](https://zed.dev/docs/git)), rainbow brackets ([blog](https://zed.dev/blog/rainbow-brackets)).
- Zed has no repository-wide git graph/log view and no dedicated PR workflow — [Zed git docs](https://zed.dev/docs/git).

### Inferences
- **Recommended primitive order for Fyx (by installs unlocked without a webview):**
  1. **Inline decoration types** (range foreground/background/border, whole-line background, before/after-line text
     with theme tokens) — the single most used primitive among loved extensions (Error Lens, Better Comments, indent-
     rainbow, Color Highlight, Import Cost, TODO Highlight, Todo Tree, GitLens blame). Fyx's block-above/below decorations
     do not cover these; "inline inlays" should be decided in favour of after-line virtual text at minimum.
  2. **Gutter marks** (icon/colour per line, click → command) — Bookmarks, Todo Tree, Error Lens, test status, Image
     preview, MyBatisX-style navigation.
  3. **CodeLens-style clickable line annotations** — REST Client "Send Request", SQLTools "Run", test run/debug, GitLens
     authorship. Could reuse Zed's own LSP code-lens rendering if the host exposes it to WASM.
  4. **Hover contributions** (markdown host component in a hover) — REST Client, Image preview, GitLens, test results.
  5. **File-tree (project panel) decorations** (badge/colour per path) — GitLens, Project Manager, GitToolBox, .ignore.
  6. **Language-provider bridge** for diagnostics/completion/code actions. Many such extensions are really language
     servers; Fyx may get them "for free" through Zed's LSP extension path rather than fyx:ui.
  7. **Notifications/progress** (`withProgress` is in 20–35 files of GitLens, Container Tools, GitHub PRs) — not in the
     current Fyx list; cheap and widely used.
  8. **Virtualized table + graph-lane cell** would move Git Graph/Git History/DB result grids/hex view out of the
     webview bucket; a **canvas/image** primitive with `request-tick` would cover pets/minimap-like toys.
- **Webview remains required** for ~17/57 sample extensions (~25% of installs): AI agent UIs (already a Zed built-in
  domain), non-text viewers (PDF, Draw.io, notebooks renderers), Postman/Thunder-style GUIs, embedded browser preview.
  Of these, only Draw.io, PDF/notebook renderers and embedded browser preview lack a plausible native replacement.
- For the **VS Code adapter**, extensions whose manifest has only `configuration`/`commands` + decorations/status bar
  (indent-rainbow, Better Comments, Error Lens, Import Cost, WakaTime, Peacock, Live Server) are the lowest-risk first
  targets; tree-view extensions (Todo Tree, Bookmarks, Project Manager, Container Tools, Kubernetes, SQLTools trees) map
  1:1 onto the retained tree; anything with `type: "webview"` views or `customEditors` should be flagged unsupported.

### Gaps
- The need sets are hand-classified; a mechanical pass over the full top-500 manifests (counting `views[].type`,
  `customEditors`, and API calls in bundled JS) would give a firmer webview percentage.
- Whether Zed's internal LSP CodeLens/inlay rendering can be re-used for WASM-provided items was not investigated (Fyx
  codebase question).
- Active-usage weighting (vs. cumulative installs) is unavailable from any public source found.
