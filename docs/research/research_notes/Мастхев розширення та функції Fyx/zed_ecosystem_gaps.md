# Zed ecosystem gaps: what users want that a UI extension API would unlock (as of 2026-11-17)

Method note: reaction, upvote and comment counts were pulled live on 2026-11-17 through the GitHub REST search API (issues, `sort=reactions`) and the GraphQL discussion search (`sort:top`) for `zed-industries/zed`. Registry numbers come from `https://api.zed.dev/extensions?max_schema_version=1`, also fetched 2026-11-17. "r" is total reactions, "+1" is thumbs-up, "up" is discussion upvotes and "c" is comments. Since about 2025, Zed has steered feature requests into GitHub Discussions (category "Feature Requests"), so newer demand shows up as discussion upvotes, not issue reactions. That last point is my inference from the data: many older issues were closed in bulk on 2025-01-30 and 2025-03-12, and newer requests are almost all discussions.

## 1. Extension UI API, panel, webview and plugin requests in zed-industries/zed

### Takeaway
UI-for-extensions demand is concentrated in one big open issue, "Webview via Extensions" #21208 (506 reactions). Zed lists it under "Triage" on its roadmap. Next comes a community RFC, "Visual Extension API" (discussion #53403, 80 upvotes). A Zed team member said in May 2026 that it is "not something we're likely to get to in the near future". Even so, Zed staff published an experimental WASM-GPUI plugin spike (`zed-industries/embedded_gpui`) in July 2026. Fyx competes with that future upstream direction.

### Cited Findings
**Main issues and discussions, ranked by demand**
- #21208 "Webview via Extensions": OPEN, opened 2024-11-26, 506 reactions (477 +1), 39 comments. Use cases include preview panes (image/PDF/SVG) and interactive web apps that edit a file. — [Issue #21208](https://github.com/zed-industries/zed/issues/21208)
  - Contributor huacnlee (2024-11-26) built a Wry-based WebView prototype. The webview "will always render on top of the GPUI elements", and on Linux/Wayland Wry cannot work as a child window. — [#21208 comments](https://github.com/zed-industries/zed/issues/21208)
  - Contributor notpeter (2024-11-26) said a platform-agnostic API needs significant work and that the feature is "likely still a ways off". Contributor Zollerboy1 (2025-05-14) proposed exposing a subset of GPUI to the WASM runtime as an alternative. — [#21208 comments](https://github.com/zed-industries/zed/issues/21208)
  - The issue sits in the "Triage" column of the official roadmap. It is the only extension-related roadmap item. — [zed.dev/roadmap](https://zed.dev/roadmap)
- #5269 "Add a plugin interface to extend the editor": CLOSED 2024-04-18, 714 reactions (600 +1), 34 comments. It was closed when the WASM extension system shipped, which has no UI. — [Issue #5269](https://github.com/zed-industries/zed/issues/5269)
- Discussion #53403 "RFC: Visual Extension API - Allow Extensions to Render GUI": OPEN, opened 2026-04-08, 80 upvotes, 47 reactions, 15 comments. It proposes GPUI-rendered sidebar panels, status-bar items and custom views, with no webviews in the MVP. Use cases listed: i18n manager, database browser, API client, git GUI/commit graph, task runner, color picker, image preview, plot/chart. — [Discussion #53403](https://github.com/zed-industries/zed/discussions/53403)
  - The RFC's phased plan: Phase 1 status-bar items (text/icon), Phase 2 read-only panels (text, buttons, lists), Phase 3 interactive panels. — [Discussion #53403](https://github.com/zed-industries/zed/discussions/53403)
  - Zed contributor macraig, 2026-05-21: the work "needs to be driven by our team", is "not something we're likely to get to in the near future", and is "very much on our radar as part of the bigger extensions story." — [Discussion #53403](https://github.com/zed-industries/zed/discussions/53403)
  - Commenters name the use cases they want: Linear/Jira tickets for the current branch, an AWS resource explorer, a live DB table viewer, CI/CD status (clementGilardy, 2026-05-19), a JSONL preview and a SQLite viewer (AirboZH), a DB viewer to replace VS Code (nakajimayoshi). One commenter said the lack of UI extensions "caused me to move away from zed". — [Discussion #53403](https://github.com/zed-industries/zed/discussions/53403)
- Discussion #48947 "Allow extensions to add buttons to a specific area of the UI": 18 upvotes, opened 2026-02-11. It asks for status-bar and side-panel buttons, with examples like a QR code generator, a color selector, and Makefile targets as clickable buttons. — [Discussion #48947](https://github.com/zed-industries/zed/discussions/48947)
- #21400 "Allow Extensions to Create and Manage Customizable Side Panels" was closed 2024-12-16 as "unactionable". notpeter: "at some point in the future we intend to support visual elements". — [Issue #21400](https://github.com/zed-industries/zed/issues/21400)
- #17325 "Custom Views in Extension API" (closed 2024-09-04, 9 reactions) and #18877 / discussion #18880 "Generic Panel Extension Support" (closed) are earlier duplicates. — [Issue #17325](https://github.com/zed-industries/zed/issues/17325), [Discussion #18880](https://github.com/zed-industries/zed/discussions/18880)

**Smaller, more specific extension-API requests (2025–2026 discussions)**
- "Extension API: support custom read-only file previews": #59598, 14 up, 2026-06-19. — [Discussion #59598](https://github.com/zed-industries/zed/discussions/59598)
- "Allow language extensions to opt into native previews": #62317, 29 up, 2026-08-07. — [Discussion #62317](https://github.com/zed-industries/zed/discussions/62317)
- "Extension API: Expose Inlay/Decoration APIs to Extensions": #49500, 7 up, 2026-02-18. This maps to Fyx's planned decorations. — [Discussion #49500](https://github.com/zed-industries/zed/discussions/49500)
- "Extension API: register a status-bar action": #61727, 6 up, 2026-07-26. — [Discussion #61727](https://github.com/zed-industries/zed/discussions/61727)
- "Extension API: Intercept and transform editor text transactions": #53131, 15 up. "API for in-place text transformation": #33844, 16 up. — [Discussion #53131](https://github.com/zed-industries/zed/discussions/53131), [Discussion #33844](https://github.com/zed-industries/zed/discussions/33844)
- "Add `on_agent_event` hook to Extension API with capability-based user consent": #56326, 4 up. — [Discussion #56326](https://github.com/zed-industries/zed/discussions/56326)
- "Public VCS provider API for status and diffs": #63930, 16 up, 2026-09-08. — [Discussion #63930](https://github.com/zed-industries/zed/discussions/63930)
- "Obsidian-style Markdown live preview: opt-in extension API vs. minimal core feature": #67063, 2026-11-16. "Interactive markdown note graph view": #66829, 2026-11-11. — [Discussion #67063](https://github.com/zed-industries/zed/discussions/67063), [Discussion #66829](https://github.com/zed-industries/zed/discussions/66829)
- Requests for headless (non-UI) APIs: "let extensions arbitrarily send and receive LSP messages" #20042 (OPEN, 69 reactions), "Command palette/projects/text buffer extension API" #18043 (OPEN, 40 reactions, 14 comments), "let extensions spawn local HTTP servers" #20040 (OPEN, 15). — [Issue #20042](https://github.com/zed-industries/zed/issues/20042), [Issue #18043](https://github.com/zed-industries/zed/issues/18043), [Issue #20040](https://github.com/zed-industries/zed/issues/20040)
- Localization through language-pack extensions, which would need UI string hooks: "Localization/i18n RFC" #43592 (39 up), issue #7409 "Localization and support for different locales" (OPEN, 164 reactions). — [Discussion #43592](https://github.com/zed-industries/zed/discussions/43592), [Issue #7409](https://github.com/zed-industries/zed/issues/7409)

**Upstream experimentation (an important duplication risk)**
- `zed-industries/embedded_gpui`, created 2026-07-08, last push 2026-09-07, 60 stars. Its description is "An experimental UI plugin API base for Zed". The README says it runs "GPUI running *inside* a Wasm component" compiled to `wasm32-wasip2`. The host replays the guest's display lists as native GPUI primitives. It has an object-capability model (`Revocable`, `Attenuated`, `Audited`), a per-turn epoch budget with a memory cap, and an optional QuickJS runtime that renders "UI as a data tree" from JS plugins. The README says: "nothing here is a supported API yet". — [embedded_gpui repo](https://github.com/zed-industries/embedded_gpui)
- Commenter tiagoacarvalho (2026-11-04/06) pointed to embedded_gpui and started a public fork that implements RFC #53403 step by step, beginning with the status-bar item. — [Discussion #53403](https://github.com/zed-industries/zed/discussions/53403), [tiagoacarvalho/extension-api-with-ui](https://github.com/tiagoacarvalho/extension-api-with-ui)

### Inferences
- Fyx's `fyx:ui` design, a retained declarative tree built from host components and theme tokens only, is close to RFC #53403's "restricted API, GPUI-based, theming consistency" principles. It is a different model from embedded_gpui, where the guest runs real GPUI and the host replays its display lists. If upstream ever ships embedded_gpui, Fyx's API would probably diverge from it, not duplicate it. Fyx should plan a compatibility or porting story instead of assuming upstream stays empty.
- The 506 reactions on #21208 largely reflect demand for *any* extension UI. Most of the use cases cited (previews, DB viewers, API clients) fit a native declarative tree plus host components (markdown, diff, editor view, table) without a webview. True webview needs (HTML/JS-rendered previews such as Mermaid live editors or PDF) are not covered by Fyx's API.
- The status-bar item is the most-cited minimum viable surface (#48947, #61727, RFC phase 1), so Fyx's status-bar items cover the cheapest and broadest demand.

### Gaps
- GitHub search caps results and cannot sort discussions by upvotes directly (`sort:top` was used). Some lower-ranked UI-API discussions may be missing.
- I did not find any public Zed staff statement after 2026-05-21 on the timeline for UI extensions or embedded_gpui.

## 2. Top-voted feature requests overall, and which need UI that extensions cannot draw today

### Takeaway
Many of the largest historical requests have already shipped in core: debugger, git panel, git graph, devcontainers, Windows, bookmarks, minimap, rainbow brackets, Helix. The biggest open demand that a UI extension could serve is:
- the test runner/explorer (#5242, 934 reactions)
- GitHub PR review inside Zed (discussion #34759, 637 upvotes)
- settings sync (#6569/#5010)
- local file history (#24004, 286 up)
- a database viewer (#24000 at 264 up, plus #50628 at 145 up)
- test coverage in the gutter (#24002, 86 up)
- "Find All References"-style result views (#5117, 224 reactions)
- a call hierarchy panel (#14203, 135 reactions)
- a markdown preview with math/WYSIWYG/custom styling
- an integrated web browser (#44609, 89 up)

### Cited Findings
**Top issues by reactions (all-time, live 2026-11-17)**, with status and my UI-blocked classification

| # | Title | Reactions (+1) / comments | Status | Needs extension-drawn UI? |
|---|---|---|---|---|
| [5065](https://github.com/zed-industries/zed/issues/5065) | Build and Debug support | 2732 (2137) / 86 | closed 2025-06-18 (shipped) | n/a, shipped in core |
| [5259](https://github.com/zed-industries/zed/issues/5259) | Rainbow brackets | 1270 / 58 | closed 2025-12-03 (shipped) | decorations; shipped |
| [8279](https://github.com/zed-industries/zed/issues/8279) | Telescope-style search box | 1061 / 59 | closed 2026-06-19 | quick-pick/modal |
| [4355](https://github.com/zed-industries/zed/issues/4355) | Smooth scrolling | 1052 / 31 | OPEN | core rendering, not extension |
| [4523](https://github.com/zed-industries/zed/issues/4523) | Diff between two arbitrary files | 942 / 42 | closed 2025-06-18 | host diff view |
| [5242](https://github.com/zed-industries/zed/issues/5242) | Test runner integration | 934 (898) / 53 | **OPEN**, roadmap "Triage" | **Yes**: test explorer tree, results, gutter status |
| [11473](https://github.com/zed-industries/zed/issues/11473) | Dev Containers | 929 / 163 | closed 2026-04-01 (shipped) | n/a |
| [26560](https://github.com/zed-industries/zed/issues/26560) | Staged and Unstaged diffs | 897 / 119 | closed 2026-07-15 | n/a |
| [10910](https://github.com/zed-industries/zed/issues/10910) | Which-Key like menu | 876 / 31 | closed 2025-12-17 | n/a |
| [5344](https://github.com/zed-industries/zed/issues/5344) | Sticky scroll | 826 / 23 | closed 2025-11-10 | n/a |
| [21538](https://github.com/zed-industries/zed/issues/21538) | Add support for jj SCM | 806 (580) / 59 | **OPEN**, roadmap "Planned" | partly: a VCS provider API plus panel UI |
| [8843](https://github.com/zed-industries/zed/issues/8843) | Icon extensions | 726 / 78 | closed 2025-01-31 (icon themes shipped) | n/a |
| [5273](https://github.com/zed-industries/zed/issues/5273) | Jupyter Notebook | 704 / 100 | closed 2025-03-03 → discussion #25936 | **Yes**: notebook cell UI with rich outputs |
| [5347](https://github.com/zed-industries/zed/issues/5347) | Remote development | 650 / 108 | closed 2024-10-30 (SSH shipped) | n/a |
| [4930](https://github.com/zed-industries/zed/issues/4930) | easymotion vim | 530 / 39 | OPEN, roadmap "Triage" | decorations/labels overlay (possible with editor decorations) |
| [9662](https://github.com/zed-industries/zed/issues/9662) | Secondary editor windows (multi-monitor) | 508 / 54 | OPEN, roadmap "Planned" | core |
| [21208](https://github.com/zed-industries/zed/issues/21208) | Webview via Extensions | 506 / 39 | OPEN | is the UI API itself |
| [14801](https://github.com/zed-industries/zed/issues/14801) | Flash.nvim style search | 488 / 77 | OPEN | decorations overlay |
| [9459](https://github.com/zed-industries/zed/issues/9459) | Open `.code-workspace` files | 446 / 52 | OPEN | no |
| [7808](https://github.com/zed-industries/zed/issues/7808) | Make the Python experience wonderful | 430 / 31 | OPEN | partly (venv picker, test explorer) |
| [5308](https://github.com/zed-industries/zed/issues/5308) | Minimap | 408 / 60 | closed 2025-01-30 (shipped) | n/a |
| [4901](https://github.com/zed-industries/zed/issues/4901) | Inline errors (Error Lens) | 395 / 68 | closed 2025-02-20 | decorations; shipped |
| [12589](https://github.com/zed-industries/zed/issues/12589) | No NodeJS download without consent | 371 / 97 | OPEN, roadmap "In Progress" | no |
| [10696](https://github.com/zed-industries/zed/issues/10696) | Mermaid in Markdown preview | 364 / 21 | closed 2026-02-13 (shipped) | n/a |
| [9721](https://github.com/zed-industries/zed/issues/9721) | Difftastic for diffs | 348 / 16 | OPEN | **Yes/partly**: custom diff view fed by external tool |
| [6722](https://github.com/zed-industries/zed/issues/6722) | Drag tab to new window | 334 / 33 | OPEN | core |
| [11895](https://github.com/zed-industries/zed/issues/11895) | Highlight TODO/INFO comments | 335 / 33 | closed 2025-11-21 | highlighting shipped; a **TODO tree panel** is not covered |
| [16827](https://github.com/zed-industries/zed/issues/16827) | File history viewing (local + Git) | 261 / 19 | closed 2025-12-01 | git file history shipped; local history still open (below) |
| [10647](https://github.com/zed-industries/zed/issues/10647) | Configurable mouse bindings | 239 / 19 | OPEN | no |
| [5117](https://github.com/zed-industries/zed/issues/5117) | Traditional "Find All References" views | 224 / 27 | **OPEN** | **Yes**: results tree panel |
| [5305](https://github.com/zed-industries/zed/issues/5305) | Spell Checking | 206 / 74 | closed 2025-03-01 | served by LSP extension `codebook` (167k downloads, see §3) |
| [31102](https://github.com/zed-industries/zed/issues/31102) | RTL text | 206 / 45 | OPEN | core |
| [5010](https://github.com/zed-industries/zed/issues/5010) | Settings Sync | 186 / 9 | OPEN, roadmap "In Progress" | upstream doing it |
| [24878](https://github.com/zed-industries/zed/issues/24878) | Offline support (meta) | 177 / 14 | OPEN | no |
| [34813](https://github.com/zed-industries/zed/issues/34813) | Git 3-way merge | 177 / 13 | OPEN, roadmap "Planned" | upstream doing it |
| [13300](https://github.com/zed-industries/zed/issues/13300) | Per-project themes | 176 / 38 | OPEN, roadmap "Planned" | upstream doing it |
| [57982](https://github.com/zed-industries/zed/issues/57982) | Branch Merging | 168 / 14 | OPEN, roadmap "Planned" | upstream |
| [7121](https://github.com/zed-industries/zed/issues/7121) | Allow disabling unwanted panels | 166 / 29 | OPEN | no |
| [14203](https://github.com/zed-industries/zed/issues/14203) | 'Show Call Hierarchy' panel | 135 / 19 | **OPEN** | **Yes**: tree panel (see Gaps: a call-hierarchy feature may already partly exist) |
| [10966](https://github.com/zed-industries/zed/issues/10966) / [21717](https://github.com/zed-industries/zed/issues/21717) / [40813](https://github.com/zed-industries/zed/issues/40813) | Markdown preview default / WYSIWYG / LaTeX | 121 / 114 / 76 | OPEN | **Yes/partly**: host markdown component, custom preview |
| [58974](https://github.com/zed-industries/zed/issues/58974) | Side-by-side merge conflict UI | 105 / 4 | OPEN, roadmap "Planned" | upstream |
| [17547](https://github.com/zed-industries/zed/issues/17547) | Type to search in project panel | 90 / 11 | OPEN | core |

Sources: the per-row issue links, plus the live search [GitHub issues sorted by reactions](https://github.com/zed-industries/zed/issues?q=is%3Aissue+sort%3Areactions-%2B1-desc) and [zed.dev/roadmap](https://zed.dev/roadmap) for the roadmap columns.

**Top Feature-Request discussions by upvotes (live 2026-11-17)**

| # | Title | Upvotes / comments | State | Needs extension-drawn UI? |
|---|---|---|---|---|
| [26770](https://github.com/zed-industries/zed/discussions/26770) | Side-by-side Git View | 927 / 83 | closed (split diffs shipped Feb 2026) | n/a |
| [34759](https://github.com/zed-industries/zed/discussions/34759) | GitHub integration for reviewing PRs inside Zed | 637 / 42 | **OPEN** | **Yes**: PR list, review threads, comments panel (GitHub PR extension in VS Code) |
| [6569](https://github.com/zed-industries/zed/discussions/6569) | Settings Sync | 379 / 60 | OPEN | upstream in progress |
| [35996](https://github.com/zed-industries/zed/discussions/35996) | Integrate devenv support | 335 / 7 | OPEN | no |
| [25936](https://github.com/zed-industries/zed/discussions/25936) | Jupyter Notebook | 317 (1193 reactions) / 165 | OPEN | **Yes**, but "Notebooks" is on the upstream "Coming Up Next" roadmap |
| [26511](https://github.com/zed-industries/zed/discussions/26511) | Commit Log | 316 / 17 | closed (shipped) | n/a |
| [26866](https://github.com/zed-industries/zed/discussions/26866) | Add Git Graph | 312 / 13 | closed (shipped Apr 2026) | n/a |
| [24004](https://github.com/zed-industries/zed/discussions/24004) | Local History for Files | 286 / 34 | **OPEN** | **Yes**: timeline panel + host diff view |
| [24000](https://github.com/zed-industries/zed/discussions/24000) | SQL Database view and query console | 264 / 12 | **OPEN** | **Yes**: tree + table + editor view |
| [26515](https://github.com/zed-industries/zed/discussions/26515) | Branch Merging | 200 / 9 | OPEN | upstream planned |
| [11086](https://github.com/zed-industries/zed/discussions/11086) | Background Image Support | 155 / 18 | OPEN | no (theme/core) |
| [26084](https://github.com/zed-industries/zed/discussions/26084) | Git Worktrees support | 146 / 20 | OPEN | partly |
| [50628](https://github.com/zed-industries/zed/discussions/50628) | Built-in database viewer panel (SQLite, PostgreSQL, MySQL) | 145 / 23 | **OPEN** | **Yes** |
| [34436](https://github.com/zed-industries/zed/discussions/34436) | Planning mode / Specs | 141 / 11 | OPEN | upstream "Plan Mode" planned |
| [49472](https://github.com/zed-industries/zed/discussions/49472) | Context window usage indicator for ACP agents | 131 / 11 | OPEN | status-bar item (agent-adjacent) |
| [43384](https://github.com/zed-industries/zed/discussions/43384) | Custom Markdown Preview theming/styling | 127 / 20 | OPEN | partly |
| [25156](https://github.com/zed-industries/zed/discussions/25156) | File nesting | 124 / 7 | OPEN | core project panel |
| [37583](https://github.com/zed-industries/zed/discussions/37583) | Jujutsu workspace support | 105 / 7 | OPEN | partly |
| [44609](https://github.com/zed-industries/zed/discussions/44609) | Integrated Web-Browser | 89 / 13 | OPEN | **Yes**, needs a webview (not in Fyx API) |
| [49245](https://github.com/zed-industries/zed/discussions/49245) | Side-by-side merge conflict UI | 88 / 8 | OPEN | upstream planned |
| [49313](https://github.com/zed-industries/zed/discussions/49313) | Mobile app to control agent runs | 86 / 9 | OPEN | no |
| [24002](https://github.com/zed-industries/zed/discussions/24002) | Show test coverage in the gutter | 86 / 11 | **OPEN** | **Yes**: gutter decorations + panel |
| [58333](https://github.com/zed-industries/zed/discussions/58333) | Collapse ACP/tool transcript blocks by default | 85 / 14 | OPEN | agent panel (relevant to Fyx agent review) |
| [32293](https://github.com/zed-industries/zed/discussions/32293) | Different themes for different windows | 83 / 21 | OPEN | upstream planned |
| [24224](https://github.com/zed-industries/zed/discussions/24224) | Markdown math (LaTeX) | 81 / 21 | OPEN | partly |
| [29996](https://github.com/zed-industries/zed/discussions/29996) | Auto port forward with remote development | 78 / 13 | OPEN | ports panel (partly) |
| [56252](https://github.com/zed-industries/zed/discussions/56252) | Remote devcontainers | 71 / 7 | OPEN, roadmap "Triage" | no |
| [51633](https://github.com/zed-industries/zed/discussions/51633) | RFC: Live typst preview | 66 / 8 | OPEN | **Yes**: custom preview pane |
| [48185](https://github.com/zed-industries/zed/discussions/48185) | Branch Compare Panel | 65 / 6 | closed | n/a |
| [47184](https://github.com/zed-industries/zed/discussions/47184) | GitHub Codespaces | 64 / 3 | OPEN | no |
| [30275](https://github.com/zed-industries/zed/discussions/30275) | Enhance Markdown editing and preview | 57 / 4 | OPEN | partly |
| [58338](https://github.com/zed-industries/zed/discussions/58338) | Native Claude Code IDE integration (terminal CLI ↔ editor) | 50 / 7 | OPEN | partly (diff review) |
| [47695](https://github.com/zed-industries/zed/discussions/47695) | Show file diff preview before approving agent edit tool calls | 31 / 7 | OPEN | **directly Fyx agent-review territory** |
| [49452](https://github.com/zed-industries/zed/discussions/49452) | Surface Claude Code subagent/team activity in agent panel | 29 / 4 | OPEN | agent panel |
| [26082](https://github.com/zed-industries/zed/discussions/26082) | AsciiDoc rendered preview | 27 / 2 | OPEN | **Yes**: custom preview |
| [45244](https://github.com/zed-industries/zed/discussions/45244) | Diagram-as-code canvas preview (Mermaid/PlantUML/D2) | 17 / 2 | OPEN | **Yes** (canvas/webview) |
| [51251](https://github.com/zed-industries/zed/discussions/51251) | Simple Browser | 16 / 6 | closed | webview |
| [60531](https://github.com/zed-industries/zed/discussions/60531) | Native Bruno HTTP client integration | 6 / 0 | OPEN | **Yes**: REST client |
| [66443](https://github.com/zed-industries/zed/discussions/66443) | Plugin for API requests from .http/.rest files | 1 / 0 | OPEN | **Yes**: REST client response viewer |
| [65619](https://github.com/zed-industries/zed/discussions/65619) | Add hex editor | 4 / 1 | OPEN | **Yes**: custom view |
| [65749](https://github.com/zed-industries/zed/discussions/65749), [65947](https://github.com/zed-industries/zed/discussions/65947) | Node.js tooling panel; Rust package manager panel | 1 each | OPEN | **Yes**: panels |

Source for the ranking: GraphQL search `repo:zed-industries/zed sort:top` (see the per-row links). Discussion search also showed the 1,076 discussions it returned for that query.

**Other categories the brief asked about**
- REST client: the only issue, #21995 "Integrated REST client", was closed after one day (2024-12-13/14, 0 reactions). Demand appears only in small 2026 discussions (#60531, #66443) and in RFC #53403's "API Client" use case. — [Issue #21995](https://github.com/zed-industries/zed/issues/21995), [Discussion #53403](https://github.com/zed-industries/zed/discussions/53403)
- Database client: issue #9128 "Built-in database tool similar to Pycharm" (38 reactions) was closed 2025-01-30. Demand moved to discussions #24000 (264 up) and #50628 (145 up). — [Issue #9128](https://github.com/zed-industries/zed/issues/9128)
- Bookmarks: #4526 (61 reactions, 34 comments) closed 2026-04-17. Bookmarks are in core, with a tracking issue #55871 still open and a "Bookmarks" tab. — [Issue #4526](https://github.com/zed-industries/zed/issues/4526), [Issue #55871](https://github.com/zed-industries/zed/issues/55871), [Issue #56268](https://github.com/zed-industries/zed/issues/56268)
- Test coverage gutter: issue #19384 was closed 2025-01-31 and moved to discussion #24002 (86 up). — [Issue #19384](https://github.com/zed-industries/zed/issues/19384)
- Git blame: inline blame (#4793, 125 reactions) shipped 2024. "Display git blame for line in the status bar" #12133 (55 reactions) closed 2026-06-29. — [Issue #4793](https://github.com/zed-industries/zed/issues/4793), [Issue #12133](https://github.com/zed-industries/zed/issues/12133)
- Docker/Kubernetes: no high-reaction feature issues for a Docker/K8s explorer panel. Docker activity is devcontainer bugs (2026-04). — [Issue #53478](https://github.com/zed-industries/zed/issues/53478), [Issue #19287](https://github.com/zed-industries/zed/issues/19287)
- Image/PDF/SVG previews: a core image viewer exists (pan & zoom #9584 closed 2026-01-21). "PDF preview" #23524 (71 reactions) was closed the same day it was filed (2025-01-23). — [Issue #9584](https://github.com/zed-industries/zed/issues/9584), [Issue #23524](https://github.com/zed-industries/zed/issues/23524)
- Markdown preview extras: "Select text in markdown preview" #16727 (103) closed 2026-03-27. "Edit Markdown with Live Preview in a Single View" #15066 (36) closed. Mermaid export/zoom is in #65530. — [Issue #16727](https://github.com/zed-industries/zed/issues/16727), [Discussion #65530](https://github.com/zed-industries/zed/discussions/65530)
- Other press coverage: the Register (2026-04-30) reports Zed has "only 1,000 extensions" versus VS Code's 100,000+. A 2026 review calls the extension ecosystem Zed's "most significant limitation". — [The Register](https://www.theregister.com/2026/04/30/zed_team_releases_version_10/), [dev-productivity-guide review](https://dev-productivity-guide.pages.dev/blog/zed-editor-review-2026/)

### Inferences
- Ranked by evidence, the highest-demand **UI-blocked** extension candidates that are not on the upstream roadmap are:
  1. GitHub/GitLab PR review (637 up). Zed's direction here is "Delta", which replaces PRs, so a classic GitHub PR panel may stay a gap.
  2. Test explorer (934 reactions; upstream marks it "Triage", not planned).
  3. Local file history (286 up).
  4. Database viewer/query console (264 + 145 up, two separate threads).
  5. Find-references and call-hierarchy result panels (224 and 135).
  6. Test coverage gutter (86 up).
  7. Custom previews: typst 66, AsciiDoc 27, LaTeX/WYSIWYG markdown 114/76.
  8. A REST/HTTP client (low explicit votes, but a staple of other editors' extension marketplaces).
- Several requests need *editor decorations* rather than panels: coverage gutters, easymotion/flash labels (530 and 488 reactions, both open), and Difftastic-style diffs. Fyx's planned decorations are therefore demand-backed.
- Notebooks/Jupyter has huge demand (704 + 1193 reactions), but it is on upstream's "Coming Up Next" list, so Fyx should not build it.

### Gaps
- Issues closed on 2025-01-30, 2025-02-20 and 2025-03-01/03 (spell check, database, minimap, Jupyter) may have been closed because they were moved to discussions, not because they shipped. I could not verify each closure reason.
- "Call hierarchy" is still OPEN as #14203. But bug reports #64379 (closed 2026-11-03) and #64480 (2026-09) mention "Call hierarchy", which suggests a core implementation shipped around September 2026. I did not confirm this in release notes; the keyword search was case-sensitive and missed it. — [Issue #64379](https://github.com/zed-industries/zed/issues/64379), [Issue #64480](https://github.com/zed-industries/zed/issues/64480)
- Code lens also seems to exist in core (bug reports since 2026-04, e.g. #55046, #58864), but I found no announcement. — [Issue #58864](https://github.com/zed-industries/zed/issues/58864)
- I found no reliable Reddit (r/ZedEditor) or Hacker News aggregate ranking of missing features. Search only turned up anecdotal threads.

## 3. Most-downloaded extensions in the Zed registry, and the dominant categories

### Takeaway
The registry is entirely "headless": languages/grammars/LSPs and themes account for almost all downloads. The top result is `html` with 7.87M downloads. The highest-downloaded non-language, non-theme extension is the Context7 MCP server (180k). No extension in the registry can add a panel or view, which matches the API's limits.

### Cited Findings
- The registry API returned 1,000 extensions on 2026-11-17. That may be a page cap rather than the true total; the Register independently cites "only 1,000 extensions" in April 2026. — [api.zed.dev/extensions](https://api.zed.dev/extensions?max_schema_version=1), [The Register](https://www.theregister.com/2026/04/30/zed_team_releases_version_10/)
- Top 30 by `download_count` (2026-11-17):

| Rank | ID | Downloads | Provides |
|---|---|---|---|
| 1 | html | 7,872,680 | languages, grammars, LSP |
| 2 | git-firefly | 1,845,806 | languages, grammars |
| 3 | toml | 1,570,606 | languages, LSP |
| 4 | java | 1,460,400 | languages, LSP, DAP |
| 5 | catppuccin | 1,231,688 | themes |
| 6 | dockerfile | 1,199,528 | languages, LSP, DAP |
| 7 | sql | 891,624 | languages, grammars |
| 8 | php | 873,284 | languages, LSP, DAP |
| 9 | vue | 740,871 | languages, LSP |
| 10 | ruby | 647,613 | languages, LSP, DAP |
| 11 | catppuccin-icons | 640,885 | icon themes |
| 12 | xml | 551,947 | languages, LSP |
| 13 | material-icon-theme | 538,808 | icon themes |
| 14 | macos-classic | 517,327 | themes |
| 15 | emmet | 506,029 | LSP |
| 16 | csharp | 491,974 | languages, LSP |
| 17 | tokyo-night | 490,591 | themes |
| 18 | scss | 484,723 | languages, LSP |
| 19 | make | 456,556 | languages |
| 20 | lua | 449,885 | languages, LSP |
| 21 | terraform | 372,327 | languages, LSP |
| 22 | astro | 336,188 | languages, LSP |
| 23 | one-dark-pro | 329,179 | themes |
| 24 | kotlin | 329,107 | languages, LSP |
| 25 | log | 326,913 | languages |
| 26 | elixir | 318,762 | languages, LSP, DAP |
| 27 | dart | 317,381 | languages, LSP, DAP |
| 28 | github-theme | 304,244 | themes |
| 29 | nvim-nightfox | 301,077 | themes |
| 30 | swift | 300,363 | languages, LSP, DAP |

  — [api.zed.dev/extensions](https://api.zed.dev/extensions?max_schema_version=1)
- Notable non-language entries: `vscode-icons` 219k, `jetbrains-themes` 209k, `biome` 206k, `csv` 205k, `mcp-server-context7` 180k (the top context server), `codebook` (spell-check LSP) 167k, `docker-compose` 166k, `rainbow-csv` 152k, `typst` 145k. — [api.zed.dev/extensions](https://api.zed.dev/extensions?max_schema_version=1)
- Number of extensions per capability (an extension can count in several): themes 352, languages 277, grammars 277, language-servers 265, context-servers 58, snippets 44, icon-themes 25, debug-adapters 14, agent-servers 1. — [api.zed.dev/extensions](https://api.zed.dev/extensions?max_schema_version=1)
- Summed downloads per capability: languages/grammars about 38.3M, language-servers about 34.9M, themes about 23.1M, debug-adapters about 5.6M, context-servers about 3.0M, icon-themes about 3.0M, snippets about 2.4M, agent-servers about 39k. — [api.zed.dev/extensions](https://api.zed.dev/extensions?max_schema_version=1)

### Inferences
- Demand for visual customization is strong even within the allowed surface: themes plus icon themes come to about 26M downloads. Theme-token-based extension UI will look native, which matters to this audience.
- `csv`/`rainbow-csv` (about 357k combined) and `log` (327k) show that users install data-file viewers even when all they get is highlighting. A table or virtualized-list based CSV/log viewer is a strong first-party Fyx demo, since it is UI-only.
- The top context server (Context7, 180k) shows MCP is the current fallback for integrations, as discussion #53403 notes.

### Gaps
- The API response may be capped at 1,000 entries and has no paging parameters I could confirm, so the long tail and the exact total are uncertain.
- `download_count` is cumulative and counts updates/reinstalls, so it measures installs over time, not active users.
- Slash-command extensions don't appear as a separate `provides` value in this API response, so I could not count them.

## 4. What Zed shipped or announced in 2025–2026 (do not duplicate)

### Takeaway
Upstream Zed has shipped most of the big "IDE parity" items, and its roadmap now centres on agents, collaboration (Delta/DeltaDB), notebooks, settings sync and git merging. Extension UI is in "Triage" only. Fyx should avoid notebooks, settings sync, git graph/log/merge/conflict UIs, per-project themes, bookmarks and devcontainers. It should treat agent-edit review as a partially contested area, given Delta's "review agent-generated code in context".

### Cited Findings
**Shipped (with dates)**
- Git integration (git panel) on 2025-03-12. Icon themes 2025-02-05. Debugger 2025-06-18. AI can be fully disabled since 2025-07-23. — [Native Git support](https://zed.dev/blog), [The Debugger is Here](https://zed.dev/blog), [Issue #5065](https://github.com/zed-industries/zed/issues/5065), [Issue #4367](https://github.com/zed-industries/zed/issues/4367)
- ACP / external agents: Gemini CLI (2025-08-27), Claude Code beta (2025-09-03), JetBrains joins ACP (2025-10-06), Codex (2025-10-16), Agent Extensions (2025-11-06), ACP Registry (2026-01-28), Parallel Agents (2026-04-22), Terminal Threads (2026-05-20), Sandboxing (2026-08-05). — [zed.dev/blog](https://zed.dev/blog)
- Windows on 2025-10-15. Worktree Trust 2025-12-17. Settings rebuilt 2025-12-12. Rainbow brackets 2025-12-03. — [zed.dev/blog](https://zed.dev/blog), [Issue #5394](https://github.com/zed-industries/zed/issues/5394)
- Dev Containers on 2026-01-07 (issue #11473 closed 2026-04-01). "66 Git Improvements" from the community 2026-02-05. Theme Builder 2026-02-12. Split Diffs 2026-02-18. — [zed.dev/blog](https://zed.dev/blog), [Issue #11473](https://github.com/zed-industries/zed/issues/11473)
- Git Graph: first mentioned in stable release notes v0.231.1 (2026-04-08). Commit log #47402 closed 2026-04-29. Git file history #16827 closed 2025-12-01. Staged/unstaged diffs #26560 closed 2026-07-15. — [Zed releases](https://github.com/zed-industries/zed/releases), [Issue #47402](https://github.com/zed-industries/zed/issues/47402), [Issue #16827](https://github.com/zed-industries/zed/issues/16827), [Issue #26560](https://github.com/zed-industries/zed/issues/26560)
- Bookmarks: first mentioned in release notes at v1.0.0. #4526 closed 2026-04-17. — [Zed releases](https://github.com/zed-industries/zed/releases), [Issue #4526](https://github.com/zed-industries/zed/issues/4526)
- Zed 1.0 on 2026-04-29: Mac/Windows/Linux, Git, SSH remoting, debugger, rainbow brackets, parallel agents, ACP agents (Claude Agent, Codex, OpenCode, Cursor), Zed for Business. The post says nothing about extension UI. Next up is DeltaDB, a CRDT sync engine described as giving "humans and agents a shared view" and letting teammates review agent code in context. — [Zed is 1.0](https://zed.dev/blog/zed-1-0)
- Delta ("Introducing Delta", 2026-08-12) and "Replace PRs with Delta – Now in Public Beta" (2026-09-16). — [zed.dev/blog](https://zed.dev/blog)
- Other closed core issues: Helix keymap (#4642, 2025-12-19), Which-Key (#10910, 2025-12-17), sticky scroll (#5344, 2025-11-10), semantic tokens (#7450, 2026-02-18), Mermaid preview (#10696, 2026-02-13), TODO highlighting (#11895, 2025-11-21), agent skills (#49057, 2026-05-27), Telescope-style search (#8279, 2026-06-19), Jupyter tracking (#9778, 2026-10-15), debugger hover values (#32932, 2026-10-19). — the linked issues on [github.com/zed-industries/zed](https://github.com/zed-industries/zed/issues/9778)
- Latest stable release on 2026-11-17 was v1.28.1 (2026-11-11). Zed ships roughly weekly. — [Zed releases](https://github.com/zed-industries/zed/releases)

**Roadmap (zed.dev/roadmap, fetched 2026-11-17; undated page, © 2026)**
- Working on: Instant Sharing, Code History as Context, Async Collaboration. Shipped there: Edit Tool Improvements, Skills, UI for Terminal Agents. — [zed.dev/roadmap](https://zed.dev/roadmap)
- Coming up next: ACP improvements, Zed on the Web, Plan Mode, **Notebooks**, Hands-Free Coding. — [zed.dev/roadmap](https://zed.dev/roadmap)
- Planned: per-project themes (#13300), dictation, Branch Merging (#57982), line-by-line staging (#45295), Git 3-way merge (#34813), jj (#21538), sudo remote dev, breadcrumbs navigation (#9996), secondary windows (#9662), thread-history search (#58676), side-by-side merge conflict UI (#58974). — [zed.dev/roadmap](https://zed.dev/roadmap)
- In progress: clickable breadcrumbs (#5272), Settings Sync (#5010/#59978), no-consent Node downloads (#12589). — [zed.dev/roadmap](https://zed.dev/roadmap)
- Triage (not scheduled): easymotion (#4930), **Webview via Extensions (#21208)**, smooth scrolling (#4355), **Test runner integration (#5242)**, **PR-style review comments in Git Diff UI feeding the agent thread (#59157)**, multi-repo git panel (#59221), resource monitor (#59451), remote devcontainers (#59500), voice input (#59726), remote REPL (#59907). — [zed.dev/roadmap](https://zed.dev/roadmap)

### Inferences
- **Overlap risk with Fyx's agent review**: Delta (agent-code review in context) and roadmap triage item #59157 (PR-style review comments in Git Diff UI feeding the agent thread) both touch Fyx's hunk-level accept/reject for ACP edits. Discussion #47695 (31 up, "Show file diff preview before approving edit tool calls") is still open, so pre-approval diff review looks like an unfilled gap upstream. — [Discussion #47695](https://github.com/zed-industries/zed/discussions/47695)
- Test runner/explorer (#5242) and extension UI (#21208) are both "Triage", meaning acknowledged but not scheduled. Fyx shipping them as UI extensions would fill a gap and not collide in the short term. The risk is that upstream adopts embedded_gpui later.
- Do not duplicate: notebooks, settings sync, git graph/log/branch merge/3-way merge/conflict UI, bookmarks, per-project themes, devcontainers, plan mode, multi-agent/terminal agents.

### Gaps
- The blog index I fetched lists nothing after 2026-09-16. October–November 2026 posts may exist that I did not see.
- The roadmap page has no dates on individual items.

## 5. Community forks and alternatives of Zed, and what they add

### Takeaway
There is an active 2026 fork scene. Its features cluster into four groups: (a) privacy/no-AI (Zedless), (b) agent-centric workbenches (Glass, Flint, Paddleboard, Hive, Batcave), (c) **panels Zed lacks**, which are exactly the UI-blocked features (Lathe: git explorer and PR review; mentenaz Forge: database/workflow/dashboard panels; dbt-zed: query results panel; ZedRail: activity bar), and (d) localization/cosmetics. At least one fork (tiagoacarvalho) is building an extension UI API directly. That makes it Fyx's nearest conceptual competitor.

### Cited Findings
- **Glass** (Glass-HQ/Glass-v1): 910 stars, created 2026-01-18. "the original Zed fork", now archived, with "a complete v2 rebuild". Glass also maintains a GPUI fork, "for building fast, cross-platform apps across macOS, iOS, Linux, Windows, and the web". — [Glass-HQ/Glass-v1](https://github.com/Glass-HQ/Glass-v1), [Glass-HQ/gpui](https://github.com/Glass-HQ/gpui), [glassapp.dev](https://glassapp.dev/)
- **Zedless**: 1,284 stars for the old variant (created 2025-02-13), 64 for the new repo. "Zed fork focused on privacy and being local-first". — [old-school-zedless](https://github.com/zedless-editor/old-school-zedless), [zedless](https://github.com/zedless-editor/zedless)
- **superzed** (maxktz): 316 stars, archived 2026-09. — [maxktz/superzed](https://github.com/maxktz/superzed)
- **Lathe** (paterschris): 73 stars, created 2026-03-19. Adds "a git explorer and a graph you can check out, rebase, squash, and reset from (with undo), a merge conflict tab, in-editor PR review, peek-style Go to Definition, a live theme customizer, an Android/iOS device panel, multiple accounts per AI agent, terminals that move between windows". — [paterschris/lathe](https://github.com/paterschris/lathe)
- **dbt-zed** (arezki1990): 45 stars. "a Zed fork with native dbt support: Jinja-SQL highlighting, dbt LSP, query results panel". — [arezki1990/dbt-zed](https://github.com/arezki1990/dbt-zed)
- **mentenaz Forge / zed_Source**: a GPL-3.0 fork built because "there's no extension route". It has Database (live SQLite/Postgres/MySQL/MSSQL plus a schema graph), a visual workflow editor and a dashboard panel, posted to RFC #53403 on 2026-10-05. The repo now describes "built-in Node, .NET, Rust, Python and database tooling". — [Discussion #53403](https://github.com/zed-industries/zed/discussions/53403), [mentenaz repo](https://github.com/mentenaz/zed_Source)
- **tiagoacarvalho/extension-api-with-ui**: created around 2026-11-06, 2 stars. "Extending Zed's Extension API to support addition of new UI elements". Its stated principles include "Extensions UI must not crash zed", "Limit extension's resources and permissions" and "Extensions do not invent new UI elements. They build declaratively ontop of already existing ones." — [tiagoacarvalho/extension-api-with-ui](https://github.com/tiagoacarvalho/extension-api-with-ui)
- **Agent-centric forks**: Flint ("terminal-first fork of Zed built for developers who use tools such as Codex, Claude Code, and Pi", 21 stars); Paddleboard ("AI-native fork… built-in agent workbench", 5 stars); shotsan/zed-custom ("persistent memory, LSP symbol search, headless web browsing", 9 stars); Hive; Batcave; V-Agent (local-first AI, no telemetry). — [shenghsi/flint](https://github.com/shenghsi/flint), [paddleboarddev/paddleboard](https://github.com/paddleboarddev/paddleboard), [shotsan/zed-custom](https://github.com/shotsan/zed-custom), [tomikng/hive](https://github.com/tomikng/hive), [andrewlau624/batcave](https://github.com/andrewlau624/batcave), [otzpt/V-Agent](https://github.com/otzpt/V-Agent)
- **UI and cosmetic forks**: ZedRail (a VS Code-style activity bar, 5 stars); Unkno394/zed (editor wallpapers including animated GIFs, window transparency, a visual theme browser, 17 stars); ReachForStar/zed-ZH_CN (a full Simplified Chinese UI, 78 stars); Zenith (a PhpStorm/WebStorm parity plan, created 2026-11-17); zee295/zed (Zed GPUI in the browser with a Rust backend, 18 stars). — [Qu3ntinS/zed-rail](https://github.com/Qu3ntinS/zed-rail), [Unkno394/zed](https://github.com/Unkno394/zed), [ReachForStar/zed-ZH_CN](https://github.com/ReachForStar/zed-ZH_CN), [wp-sigmadevs/zenith](https://github.com/wp-sigmadevs/zenith), [zee295/zed](https://github.com/zee295/zed)
- **GPUI ecosystem**: longbridge/gpui-kit (formerly gpui-component), 20,919 stars, with Rust GUI components on GPUI. Its author huacnlee is a Zed contributor who prototyped the webview. bite-gpui republishes GPUI as crates "so forks can depend on them". — [longbridge/gpui-kit](https://github.com/longbridge/gpui-kit), [bite-gpui](https://github.com/bite-gpui/bite-gpui), [Issue #21208](https://github.com/zed-industries/zed/issues/21208)

### Inferences
- Forks independently converge on the same UI-blocked features that rank high in demand: git explorer/graph with actions, PR review, DB panels, query results, an activity bar, theme customizers. This is extra evidence that these features need UI, and that some people are willing to fork just to get them.
- None of the forks I found offers a *sandboxed extension UI API plus agent-edit review*. Fyx's combination looks unoccupied. tiagoacarvalho's fork is at the planning stage and embedded_gpui is an upstream spike.
- Fork star counts are small apart from Glass and Zedless, so traction for "Zed plus panels" forks is unproven.

### Gaps
- I could not verify Glass v2's feature set from its site. Only repo descriptions were checked.
- Fork discovery used GitHub repository search and the upstream fork list sorted by stars. Forks hosted elsewhere, or not described as Zed forks, were missed.
- I did not check whether Lathe's or mentenaz's panels are actually functional beyond their descriptions and screenshots.
