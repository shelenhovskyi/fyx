# Research: must-have features beyond the plan, and porting VS Code extensions

Written 2026-11-20 against `origin/integration` at `212119f189`. Research and planning only: no code
changed, no cargo builds. Numbers marked **measured** were measured for this note or come from
`tasks/reviews/perf-1.md` and the 4.13/4.14 bench notes; numbers marked **estimate** are reasoned
from those plus public sources and need a real bench before anyone quotes them.

> **Owner decision, 2026-11-20:** Fyx will not support VS Code extensions at all: no Node
> sidecar, no `.vsix` conversion as a feature. PLAN.md backlog item 12 is dropped. The W8 code
> (`sidecars/vscode-host`, `tools/vsix-convert`) stays unsupported until a separate removal step.
> Sections 2 and 3 below are the research behind that decision.

Three questions:

1. Which must-have editor features are missing from the current plan (P1–P5, the 1.0 cut around
   2026-12-16, the public registry, deferred TS SDK and proxied `network:fetch`)?
2. Can Fyx run VS Code extensions, how much of the API is realistic, and under what licences?
3. How much slower would a ported VS Code extension (Node sidecar over JSON-RPC) be than a native
   Rust/WASM extension?

## Summary

- **Must-haves not in the plan, in order:** a Fyx self-updater with signed releases; local or
  bring-your-own edit predictions (Zed's are cloud, and Fyx turns cloud off); local history with an
  agent-turn timeline; an extension settings schema with a generated settings UI; local crash
  reports. These five are P0 for 1.0 or just after. Commands in the palette and keymap are already
  planned for 0.4 (4.4) and should not slip again.
- **VS Code port verdict:** do not aim for "runs VS Code extensions". Keep W8 as a small
  **compatibility tier**: the converter for themes, snippets and keymaps (done, needs the F1 licence
  fix), plus the Node sidecar for simple command, status-bar, decoration and tree-view extensions.
  The extensions people actually miss are either Microsoft-licensed (Pylance, C/C++, C# Dev Kit,
  Remote, Live Share, Copilot: not usable outside Microsoft's products), language servers (already
  covered by Zed's LSP), or webview-heavy (needs an embedded browser, which breaks Fyx's sandbox
  story). Theia shows full API parity is a multi-year, multi-company effort.
- **Speed:** a ported extension is not slower where users look. UI updates go through the same
  `ui_ext` tree and renderer, and the renderer dominates (14.5 ms per frame for 1024 texts), while
  the transport adds 0.04–2 ms. The real costs are **startup** (about 45 ms for an empty Node process
  plus 50–300 ms to load and activate a typical extension, against 29–43 ms for a release WASM
  component today and a few ms once `.cwasm` caching lands) and **memory** (about 32 MB per idle Node
  process, so 5–15× a small WASM guest, and one process per extension for isolation).

## 1. Must-have features beyond the current plan

### What Fyx already inherits from Zed

Worth stating so nobody re-plans it: LSP, tree-sitter syntax, DAP debugger, git panel, remote
development over SSH (WSL on Windows), tasks with VS Code `tasks.json`/`launch.json` import
(`crates/task/src/vscode_format.rs`, `vscode_debug_format.rs`), VS Code settings and keymap import
(`crates/settings/src/vscode_import.rs`, `assets/keymaps/*/vscode.json`), theme import
(`crates/theme_importer`), vim and helix modes, multi-buffers, project search, the Jupyter REPL, the
agent panel and ACP. Zed's own comparison page names Git, AI and LSP as built in where VS Code
needs extensions ([zed.dev/compare/vscode](https://zed.dev/compare/vscode)).

What Fyx loses by being cloud-off: Zed's hosted edit predictions, collaboration (channels, calls,
shared projects), the hosted agent models, Zed's auto-updater endpoint and crash uploads. Each of
these is a gap users notice on day one.

### Gaps, prioritised

Size: S ≤ 3 days, M ≈ 1–2 weeks, L ≈ 3–6 weeks, XL > 6 weeks, for one stream. "API" means it fits
the `fyx:ui` extension API (a WIT change plus host work); "core" means it is editor work in Fyx
crates or a `fyx:` hook.

| # | Feature | Why users expect it | Fits | Size | Priority |
|---|---|---|---|---|---|
| G1 | **Self-updater with signed releases** (channel, Ed25519 like the registry, delta optional, rollback) | Every editor updates itself; without it 1.0 users stay on 1.0 forever and security fixes do not reach them | core (W1; reuses the registry's signing) | M | **P0, before 1.0** |
| G2 | **Local / BYO edit predictions** (llama.cpp or Ollama endpoint, Copilot, Codestral-style providers) | Cursor Tab and Copilot made inline next-edit prediction a baseline; cloud-off Fyx has none by default. Check which non-Zed providers upstream still ships before building | core (provider config) + optional API later | M | **P0** |
| G3 | **Local history + agent timeline**: every save and every agent turn as a restorable snapshot, per file and per turn | VS Code Timeline / JetBrains Local History; for Fyx it is the natural pair to agent change review (undo an agent turn after accepting it) | core, next to `agent_review` (W5 owns) | M | **P0** |
| G4 | **Extension settings schema + generated settings UI**: `[settings]` in the manifest (types, defaults, descriptions), read with a `settings` import, edited in the extensions page | VS Code `contributes.configuration`; without it every extension invents its own config panel | API (0.4) + W3 UI | M | P0/P1 |
| G5 | **Local crash reports** with an opt-in "send" that opens a prefilled issue, never an upload | Zed's crash upload is off; users and maintainers still need minidumps and panic logs | core (W1) | S | P0 |
| G6 | **Commands in palette and keymap, code lens, hover** (plan 4.4, moved to 0.4) | The first thing any extension author asks for; also what `fyx_vscode::RunCommand` needs | API | M | already planned; keep it in 0.4 |
| G7 | **Diagnostics from extensions** (non-LSP linters, review rules) into the Problems/diagnostics UI | VS Code `languages.createDiagnosticCollection`; errorlens-style extensions | API | S–M | P1 |
| G8 | **Tree views and file decorations** (badges/colours on project-panel entries) | VS Code TreeView + FileDecorationProvider; todo-tree, bookmarks, git tools | API (tree exists as `list`; badges are new) | M | P1 |
| G9 | **Test explorer** (discover, run, show results inline) | VS Code Testing API, JetBrains; Zed has runnables/tasks only | API + core panel | L | P1 |
| G10 | **3-way merge editor** for conflicts | VS Code merge editor; Zed only highlights conflict regions | core | L | P1 |
| G11 | **Spell checking** | Expected by prose and comment writers; Zed has none built in | registry extension over an LSP (e.g. Harper, codebook) | S | P1 (registry, not core) |
| G12 | **Settings and extension sync** without a cloud: export/import or a git-backed sync folder | VS Code Settings Sync; Fyx profiles (5.20) cover the local half | core | M | P2 |
| G13 | **UI localisation** (at least strings in Fyx's own crates; Ukrainian first) | VS Code language packs; Zed is English-only | core, large string sweep | L | P2 |
| G14 | **Accessibility** (screen reader for the editor and `fyx:ui` panels) | Required in many workplaces; GPUI's support is thin | core, upstream-heavy | XL | P2 (track upstream) |
| G15 | **Notebooks beyond the REPL** (`.ipynb` editor) | Data-science users | core | XL | P2 |
| G16 | **Sandboxed web view surface** | Biggest blocker for VS Code ports (section 2) and for rich previews | core (embedded WebView2/WKWebView/WebKitGTK) | XL | **decide, default no** |
| G17 | **Registry trust signals**: download counts, verified publisher, "reviewed by" badge, report button | Users judge extensions by these on Open VSX and the Marketplace | registry + W3 page | S–M | P1, with the public registry |

Not listed because the plan already covers them: terminal focus and tab account strings (P5),
relay hooks (5.12), the P5 security review, WIT 0.4 freeze, `network:fetch` through a proxy (D.2),
the TS SDK (D.1), multiple accounts and profiles (5.20), the Extension Inspector (1.7).

**Recommendation.** Add G1, G2, G3 and G5 to the plan as a "P6 product basics" block owned by W1
(G1, G5), W5 or its successor (G3) and whoever owns language/AI settings (G2), and put G4 and G7
into the 0.4 WIT next to 4.4 so the freeze includes them. G16 needs an owner decision; the default
recommended here is no.

## 2. Porting VS Code extensions

### What Fyx has today (W8)

From `tasks/progress/W8.md` and `docs/vscode-coverage.md` on `origin/claude/fyx-w8c-adapter`
(`149ce0073a`, not on `integration` yet):

- `tools/vsix-convert`: themes, snippets and keybindings into Fyx formats; 16 themes, 220 snippets,
  76 keybindings converted from the test set. **Blocked by licence finding F1**
  (`tasks/reviews/license-audit-1.md`): it ports GPL-3.0-or-later code from `theme_importer` and
  `snippet` but is declared Apache-2.0.
- `sidecars/vscode-host`: a Node sidecar that loads unmodified `.vsix` under `node --permission`
  (refuses to start without it), with a shim for 92 of 220 namespace functions and 62 of 188
  classes; everything else throws `<api> is not implemented in Fyx`. Speaks the `fyx:ui` JSON mirror
  over NDJSON JSON-RPC to a **mock** host.
- Result on 21 MIT Open VSX extensions: 14 of 18 code extensions activate, 70 of 135 commands run,
  0 schema violations. Failures: webviews (5 extensions), language features beyond
  completion/hover/formatting, `showOpenDialog`, spawned language servers (blocked by the sandbox).
- Not done: the Rust side (plan backlog item 12): launching the sidecar, bridging to the `ui_ext`
  tree, answering `push-update`, `fyx_vscode::RunCommand`, and a network sandbox.

### Prior art

| Project | How it runs VS Code extensions | Lesson for Fyx |
|---|---|---|
| **VSCodium, code-server, openvscode-server, Eclipse Che (current), Positron, Cursor, Windsurf, Kiro, Antigravity** | They *are* VS Code: forks of Code-OSS with the real workbench and extension host. Compatibility is free | Not applicable: Fyx's UI is GPUI, not a DOM. All of them use Open VSX, not Microsoft's Marketplace |
| **Eclipse Theia** | An independent reimplementation of the `vscode` API in its own plugin host (Node), with a daily API comparator report ([vscode-theia-comparator](https://github.com/eclipse-theia/vscode-theia-comparator)); claimed "full compatibility" in 2023 ([Eclipse blog](https://eclipse-foundation.blog/2023/12/)). The report checks interfaces only, not behaviour, and some APIs are stubs ([Theia docs](https://theia-ide.org/docs/user_install_vscode_extensions/)) | The only non-fork that reached parity, and it took years with several companies (TypeFox, EclipseSource, Ericsson, Red Hat). Theia also renders with a DOM, so webviews are iframes for it |
| **Zed** | Does not run VS Code extensions; imports themes, keymaps, settings and tasks instead ([zed.dev/compare/vscode](https://zed.dev/compare/vscode)) | Same approach as Fyx's converter. Zed's own extensions are WASM, like Fyx's |
| **coc.nvim (Neovim)** | Not a shim: popular VS Code extensions are rewritten as `coc-*` packages against a VS Code-like API | Rewriting against a similar API is how most editors grew their ecosystems |
| **Lapce, Helix, Sublime, JetBrains** | No VS Code compatibility | — |
| **Open VSX** | The registry everyone outside Microsoft uses; publishers sign the Eclipse publisher agreement and choose a licence, MIT if none is given ([Open VSX FAQ](https://eclipse.org/legal/open-vsx-registry-faq)) | The only legal source. Eclipse now sells a managed registry to heavy commercial consumers (AWS, Google, Cursor) ([GCN](https://gcn.com/eclipse-foundation-open-vsx-managed-registry/20563/)), so a Fyx client should cache and mirror rather than hit it per user |

### Licensing

- **Microsoft's Marketplace is off-limits.** Its terms limit Marketplace offerings to Visual Studio
  products ([VSCodium docs](https://github.com/VSCodium/vscodium/blob/master/docs/index.md),
  [Eclipse 2020](https://www.eclipse.org/community/eclipse_newsletter/2020/march/1.php)). Microsoft
  enforces it in its own extensions: the C/C++ extension stopped working in VSCodium and Cursor in
  April 2025 ([The Register](https://www.theregister.com/2025/04/24/microsoft_vs_code_subtracts_cc_extension/)).
  Pylance, C# Dev Kit, Remote-*, Live Share and the Copilot extensions carry similar licences.
- **Open VSX is allowed, per extension licence.** Open VSX does not require an OSI licence, so each
  extension's own licence decides. The W8 test set was MIT only; keep that rule (an allow-list of
  OSI licences, checked at install and in the registry CI).
- **Code-OSS and `@types/vscode` are MIT**, so the shim's API catalog is fine.
- **F1 must be fixed before anything ships:** relicense `tools/vsix-convert` as
  GPL-3.0-or-later (simplest; it is a tool, not linked into extensions), or rewrite the colour
  mapping clean-room from VS Code's MIT theme-colour reference.
- **Trademark:** "Visual Studio Code" is Microsoft's mark. Say "runs many extensions from Open VSX",
  not "VS Code compatible".

### Realistic API coverage, by extension kind

| Kind | Share of what users install (rough) | Path in Fyx | Realistic? |
|---|---|---|---|
| Themes, icon themes, snippets, keymaps | high count, low effort | converter → native Fyx/Zed assets, no runtime | **yes, done** (after F1) |
| TextMate grammars | many niche languages | Zed uses tree-sitter; TextMate grammars do not port | **no**: needs a tree-sitter grammar per language |
| Language extensions that wrap an LSP server | the most valuable ones (Rust, Go, Python via Pyright, YAML, …) | use Zed's LSP: extract the server command and settings into a Zed language extension, or use the existing Zed extension | **yes, via LSP, not via the sidecar** (the sandbox forbids `child_process`, correctly) |
| Debuggers (DAP) | medium | Zed's debugger runs DAP adapters; the `.vsix` JS that configures them does not port | partial, per adapter |
| Text commands, status bar, decorations, QuickPick, InputBox, output, simple tree views | long tail of small utilities | the W8 shim | **yes**: 14/18 activate today; needs the Rust bridge and 4.4 commands |
| Language features beyond LSP in JS (code lens, links, symbols, semantic tokens, inlay hints) | medium | needs 0.4 4.4 (code lens, hover) and more WIT | partial, after 0.4 |
| Webviews (Markdown previews, GitLens graph, REST/HTTP UIs, Jupyter, AI chat panels) | many popular ones | an embedded browser engine (G16) | **not recommended**: XL, a second rendering stack, and a large hole in the sandbox |
| SCM providers, notebooks, testing, authentication, `lm`/`chat` | medium | none planned | no |
| Microsoft-licensed extensions | the most requested | — | **no, licence** |

### Security reality of the sidecar

Node's permission model is described by Node itself as a seat belt for trusted code, not a security
boundary against malicious code, and Node 24 has no network permission at all (W8 notes this).
So a sidecar extension can open sockets freely. To match Fyx's tier model the sidecar needs an
OS sandbox: AppContainer or a restricted token plus job object on Windows, `sandbox-exec`/seatbelt
on macOS, landlock or bubblewrap on Linux. That is an L task per OS. Until then, every sidecar
extension must be shown as **T2 with full network access** at install, and the registry should not
list them in the default view.

### Verdict

**Decided by the owner on 2026-11-20: no VS Code extension support of any kind**, including the
converter. The recommendation below, which kept a small compatibility tier, is superseded and kept
only as a record.

Superseded recommendation: ship the converter (after F1) as a feature: "import themes, snippets and keymaps from Open VSX".
Keep the sidecar as an **experimental compatibility tier behind a setting**, finish the Rust bridge
(backlog 12) only after 0.4 lands commands (4.4), and do not build webviews for it. Put the effort
saved into G4/G7/G8 so that native extensions can do what the small VS Code utilities do, and into a
porting guide ("your VS Code extension in Rust/WASM") with the shim's API catalog as a map.

Rough sizes if the owner still wants the tier: Rust bridge and launcher M; OS sandbox L per OS;
code lens/links/symbols after 0.4 M; `showOpenDialog` S; a per-extension licence and tier check in
install S.

## 3. Performance: Node sidecar vs native WASM

### Native Fyx extensions (measured)

From `tasks/reviews/perf-1.md` (release-fast, a 4-core machine with other builds running, ±30%) and
the 4.13/4.14 notes:

| What | Time |
|---|---|
| Load a release 80 KB component (compile + instantiate + `surfaces`) | 43 ms; 29 ms again in the same process |
| Load a debug 4.2 MB demo | 109 ms (69 ms again) |
| `handle-event` round trip, foreground → tokio → guest → back | 32 µs |
| 1 `set-props` / 2048 `set-props` | 34 µs / 0.58 ms |
| `replace` / `mount` of 16384 nodes | 4.8 ms / 11 ms |
| `list-rows` of 40 rows × 3 nodes | 0.11 ms |
| Canvas lift, 10 000 ops (4.13) | 1.1–1.3 ms |
| Renderer, one frame with 1024 texts / 1024 buttons | 14.5 ms / 54 ms |

Compilation is not cached across restarts yet; plan task 1.1 adds a precompiled `.cwasm` per
installed version. With that, load becomes deserialize + instantiate: **estimate 1–5 ms** for small
components. Wasmtime's own guidance puts pooled instantiation in microseconds
([Tuning Wasmtime for fast instantiation](https://docs.wasmtime.dev/examples-fast-instantiation.html)).

Memory per WASM extension was not measured. **Estimate:** a small Rust guest's linear memory is
1–4 MiB plus its compiled code; the hard cap is 256 MiB (plan 1.6). The Extension Inspector (1.7)
is the place to show it.

### Node sidecar (measured for this note)

Ryzen 5 7500F, Windows 11, Node 24.18, scripts in the scratchpad (not committed): a parent Node
process spawns a child under `--permission` and does NDJSON JSON-RPC echo round trips, 20 000 calls,
first 100 dropped.

| What | Result |
|---|---|
| `node -e 0` wall time (median of 10), with or without `--permission` | 47 ms (the permission model adds nothing measurable) |
| Idle RSS of one Node process | 32 MB |
| Spawn → first JSON-RPC reply | 40–45 ms |
| Round trip, tiny payload | p50 38 µs, p99 145 µs |
| Round trip, 2048 text nodes (108 KB JSON) | p50 1.6 ms, p99 2.1 ms |

This is Node↔Node; the real host side parses with `serde_json` in Rust, so treat these as an
upper bound for the transport. On top of the empty process come the shim and the extension itself:
parsing and running a typical 0.5–5 MB extension bundle and its `activate()`. VS Code reports
per-extension activation in "Developer: Show Running Extensions"; typical values are tens to a few
hundred ms, and VS Code's own startup budget for extension host spawn plus registration is in the
hundreds of ms on a machine with 40 extensions
([John Papa](https://www.johnpapa.net/is-your-vs-code-extension-slow-heres-how-to-speed-it-up)).
The W8 coverage run has not recorded activation times; adding them is a one-line change in the
coverage runner and should be done before quoting these.

### Side by side

| Dimension | Native WASM | Ported (Node sidecar) | Ratio |
|---|---|---|---|
| Cold start, small extension | 29–43 ms today; **estimate** 1–5 ms with `.cwasm` (1.1) | 45 ms process + **estimate** 50–300 ms load and `activate()` | **2–8× today, 20–100× after 1.1** |
| Memory, idle small extension | **estimate** 2–8 MB | 32 MB measured empty, **estimate** 40–80 MB with a real extension | **5–15×** |
| Memory, 20 extensions | **estimate** 40–160 MB in one process | one process each for isolation: **estimate** 0.8–1.6 GB; a shared host (as VS Code does) saves most of it but loses per-extension sandboxing | 10× |
| Per call, small (click → handler → small patch) | 32–34 µs | 38 µs p50, 145 µs p99 + JS handler | **~1–4×**, both far below a frame |
| Per call, large (2048-node update) | 0.58 ms | 1.6 ms p50 | **~3×** |
| UI update latency the user sees (event → repaint) | transport + renderer; renderer is 14.5 ms for 1024 texts | same renderer, +0.04–2 ms transport | **1.0–1.15×**: the renderer dominates |
| CPU-heavy work in the extension | Cranelift-compiled WASM | V8 JIT | similar; within about 1.5× either way, depends on the code (**estimate**) |
| Crash isolation | trap stops one instance | process dies, restart needed | — |

**Conclusion.** For interactive UI a port feels the same as a native extension, because both are
bounded by the `fyx:ui` renderer, whose full-tree re-render (perf-1 finding 1) matters more than
either transport. The port loses on startup and memory, and those losses grow with the number of
installed extensions, which is the opposite of what the "fast editor" pitch promises. That supports
keeping the compatibility tier small and opt-in.

### Benches to add before quoting numbers externally

1. Activation time and RSS per extension in `sidecars/vscode-host/scripts/run-coverage.ts`.
2. The Rust↔Node round trip once the bridge exists (same probe as `handle-event`).
3. `.cwasm` load time and per-instance memory for the demos after plan 1.1, shown in the Inspector.

## Sources

- Repository: `PLAN.md`, `docs/fyx/platform/plan.md`, `tasks/reviews/perf-1.md`,
  `tasks/reviews/license-audit-1.md`, `tasks/progress/archive/W2c.md` (4.13), `tasks/progress/archive/W3.md`
  (4.14), and on `origin/claude/fyx-w8c-adapter`: `tasks/progress/W8.md`, `docs/vscode-coverage.md`.
- [VSCodium docs: Marketplace terms and Open VSX](https://github.com/VSCodium/vscodium/blob/master/docs/index.md)
- [Eclipse newsletter 2020: Open VSX and Marketplace terms](https://www.eclipse.org/community/eclipse_newsletter/2020/march/1.php)
- [The Register 2025: C/C++ extension blocked in forks](https://www.theregister.com/2025/04/24/microsoft_vs_code_subtracts_cc_extension/)
- [Open VSX Registry FAQ](https://eclipse.org/legal/open-vsx-registry-faq)
- [GCN: Open VSX managed registry customers](https://gcn.com/eclipse-foundation-open-vsx-managed-registry/20563/)
- [Zed vs VS Code comparison](https://zed.dev/compare/vscode)
- [vscode-theia-comparator](https://github.com/eclipse-theia/vscode-theia-comparator),
  [Theia: installing VS Code extensions](https://theia-ide.org/docs/user_install_vscode_extensions/),
  [Eclipse Foundation blog, Dec 2023](https://eclipse-foundation.blog/2023/12/)
- [Wasmtime: tuning for fast instantiation](https://docs.wasmtime.dev/examples-fast-instantiation.html)
- [John Papa: is your VS Code extension slow?](https://www.johnpapa.net/is-your-vs-code-extension-slow-heres-how-to-speed-it-up)
- [kvakil: Node.js startup measurements](https://www.kvakil.me/posts/2023-05-09-nodejs-startup-series-intro-and-measuring-startup-time.html)
