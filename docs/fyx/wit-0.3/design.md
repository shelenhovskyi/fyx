# `fyx:ui@0.3.0`: design draft

Status: **draft for review** (platform plan 4.1). Nothing is decided and no code has changed. The
world is in [`fyx-ui-0.3.wit`](fyx-ui-0.3.wit), next to this file. It parses with
`wasm-tools component wit` (1.263.0), and a dummy component embedded from it validates with
`wasm-tools component embed --dummy` + `component new` + `validate`. How 0.1, 0.2 and 0.3
components coexist is in [`migration.md`](migration.md).

Sources:
- [Platform 1 spec](../platform/spec.md) decisions 21–30 and [plan](../platform/plan.md) tasks
  4.1–4.8 (4.10–4.15 for §12 and §13, added 2026-11-18), as revised for the 0.2 decisions (branch `claude/fyx-w4-sdk-demos-5d1497`, `5f0ef15`).
- [`wit-0.2/design.md`](../wit-0.2/design.md) with its "Decisions (2026-11-17)" (same branch,
  `de4b2c0`; `integration` still has the undecided draft).
- [`docs/vscode-coverage.md`](../../vscode-coverage.md), "Schema gaps" and "API gaps".
- The network audit `tasks/reviews/network-audit-1.md` (branch `claude/fyx-audit-network`).
- Code: `integration` at `1ec364b` for `crates/ui_ext`, `crates/ui_ext_host`,
  `crates/agent_review`, `crates/editor` and `crates/command_palette`; `crates/ui_ext_wasm` from
  branch `claude/fyx-w3-renderer-e20d2b` at `be53175` (the move out of
  `extension_host/src/wasm_host/fyx_ui.rs`, `b535d9e`, is not on `integration` yet).
- Not found on any branch: `docs/fyx/security-modes.md` and `docs/fyx/agent-review.md`. The
  agent-review model is taken from `crates/agent_review/src/*.rs` and
  [`review-rules.md`](../review-rules.md). Where this draft depends on a security mode, it says so.

## Summary

| § | Change | Plan | Capability, tier | Decision asked |
|---|---|---|---|---|
| 0 | Composition with 0.2: async work through `handle-io`, gestures, ids, consent, manifest | 4.1 | — | Q5, Q9 |
| 1 | Decoration extras: five styled kinds and `color-swatch` in namespaces, visible-range pull, document targets, `orphaned` | 4.2 | `ui:editor-decorations`, T0 | Q6 |
| 2 | Picker protocol: `surface-kind::picker` on the 0.2 modal lifecycle, host fuzzy matching, multi-select, live preview, sources from other extensions | 4.3 | `ui:picker`, T0 | Q11 |
| 3 | Hover and code lens providers, drawn by the editor's own UI | 4.4 | `ui:hover`, `ui:code-lens`, T0 | Q6 |
| 4 | Virtual documents at `fyx-ext://<id>/<path>` | 4.4 | `ui:virtual-documents`, T0 | — |
| 5 | Commands: manifest `[[fyx_ui.commands]]`, palette, keymap, `handle-command` | 4.4 | `ui:commands`, T0 | Q12 |
| 6 | Document context for requests (text, selections, diagnostics) | not planned | `workspace:read-buffers`, `workspace:diagnostics`, T1 | **Q3** |
| 7 | T2 consent at first use, `permissions` interface, held updates | 4.5 | — | Q10 |
| 8 | `network:fetch` | 4.6 | `network:fetch`, T2 | **Q1, Q4** |
| 9 | `process:spawn` | 4.7 | `process:spawn`, T2 | **Q1, Q2** |
| 10 | `workspace:edit` as one undoable step | 4.8 | `workspace:edit`, T2 | **Q5** |
| 11 | `agent:observe`, read-only | 4.8 | `agent:observe`, T2 | Q8 |
| 12 | `image` node: PNG and SVG from the package or generated, decoded off the main thread, panels only | 4.10–4.12 | `ui:images`, T0 | Q13 |
| 13 | `canvas` node: a static vector draw list with theme and data colours, `canvas-click`. **Last in P4**: moves to 0.4 if P4 slips | 4.13–4.15 | `ui:canvas`, T0 | Q14 |

Not changed from 0.2: the flat retained tree, theme tokens (exceptions: §1 `color-swatch` and the
pixels of §12 images; §13 adds a fixed data palette),
the effect rule (A), ticks (B), modals (C, D), runtime surfaces (E), push results (F), row
invalidation (G), the frozen icons (I), the caps of 2048 patches per update, 16384 nodes per tree
and 2048 per row, 32 surfaces and 512 block decorations per extension. No WASM call ever runs
inside `render()`: every 0.3 feature draws from host-retained state.

## 0. Composition with 0.2

### 0.1 One world, a superset

0.3 copies 0.2's `types`, `host` and `decorations` forward and adds interfaces. A 0.3 component
can do everything a 0.2 one can, with the same semantics. Three 0.2 types change shape, because a
0.3 world is a new package anyway and SDKs map them:

- `surface-kind` gains `picker` (§2);
- `decoration-location.path: string` becomes `target: decoration-target` (§1), with
  `path(...)` meaning exactly what 0.2's field meant;
- `ui-event` gains `orphaned` (§1) and `host-error-code` gains `denied` (§7).

0.2 section M (notifications, progress) was drafted on 2026-11-18. Its `notifications` interface and
its three `ui-event` cases are copied into 0.3 unchanged. Nothing else in this draft depends on
it.

### 0.2 Version dispatch

As in 0.2's migration: one frozen WIT directory per version (`since_v0.3.0/`), one bindings module
(`crates/ui_ext_wasm/src/since_v0_3_0.rs`), a third arm of `UiBindings`, `MAX_VERSION = 0.3.0`, and
the release-channel split (Dev and Nightly accept 0.3; Stable and Preview stay at the newest
frozen version) until 0.3 is declared frozen. The `ui_ext` model stays the superset of every
world. 0.1 keeps working until the version after 0.3 ships (spec decision 29; Q9 asks what that
means once 1.0 follows 0.3). Details in [`migration.md`](migration.md).

### 0.3 Work outside a call: ids now, outcomes through `handle-io`

A UI component runs only inside a host call with a 200 ms budget (`UI_CALL_BUDGET`,
`ui_ext_wasm.rs:110`), on its own queue on tokio (`ui_ext_wasm.rs:595-602`). Its imports are
synchronous. A fetch takes seconds, a process runs for minutes, and an edit needs the main thread
and maybe a buffer load. None of that can happen inside an import.

So every T2 operation follows one rule, an extension of 0.2 §A:

1. The import (`fetch`, `spawn`, `apply-edit`, `agent-observe.start`) checks what it can at once
   (manifest, syntax, caps, in-flight counts) and returns an id or a `host-error`.
2. The operation is recorded in the call's effect list and starts **after** the export returns,
   in call order with pushes and surface changes. A trap drops it with the rest of the list.
3. The permission broker is consulted when the effect is applied, on the main thread. If the
   state is `ask`, the operation waits for the prompt (§7); the wait counts against no budget.
4. The outcome is one call of the new export `handle-io(io-event)`, queued on the extension's
   call queue behind calls already queued. Its effects apply like any call's. It is not tied to a
   surface; it pushes to surfaces with `push-update`.
5. Ordering: the outcomes of one operation arrive in order (all `process-output` of a process
   before its `process-exited`). Outcomes of different operations are in completion order.
6. Backpressure: at most **256** `handle-io` calls queued per extension. Past that the host stops
   reading process pipes and holds finished fetches; nothing is dropped. A guest that keeps the
   queue full only slows itself.
7. Disable, reload, uninstall, and revoking the capability cancel the operations: fetches are
   dropped, processes are killed (§9), edits not yet applied are dropped. A reloaded instance gets
   no outcomes of the old instance's ids.

Rejected: WASI 0.3 async imports (`future<result<…>>`), not available to this host (0.2 §F says the
same); a per-operation export (`fetch-done`, `process-output`, …), which multiplies exports for no
gain over one variant.

### 0.4 Gestures in 0.3

0.2 §C grants a *transient activation* on `click`, `row-click` and `input-submitted`, lasting 5 s
or one `show-surface`. 0.3 adds three sources, all a click or a key the user pressed on something
of this extension:

- `handle-command` (palette, keymap, a code lens click, a gutter sign click);
- a `picker-source-event::accepted` (Enter, alt-Enter or secondary-Enter on an item of this
  source).

Calls that need the activation, and consume it: `show-surface` on a modal or picker,
`open-virtual-document`, `permissions.request`. Nothing else needs a gesture; in particular
`workspace:edit` does not (Q5).

### 0.5 Ids and keys

| Id | Rule | Shared with |
|---|---|---|
| surface, block decoration | 0.2: `[a-z0-9._-]{1,64}`, one namespace | — |
| picker | a surface id; globally `"<extension>/<surface>"` | surfaces |
| decoration namespace | `[a-z0-9._-]{1,64}`, per extension | nothing (separate from surface ids) |
| command | `[a-z0-9._-]{1,64}` in the manifest; globally `"<extension>/<command>"` | — |
| picker source | `[a-z0-9._-]{1,64}` in the manifest | — |
| fetch, process, edit, provider, hover request, picker session | `u64` / `u32` chosen by the host | — |
| document, worktree, agent review/session/turn | opaque `u64`, per extension, stable for the session | — |

Host-chosen ids never repeat within an instance. Opaque ids differ per extension, so two
extensions cannot correlate what they see without the user's data passing through them.

### 0.6 Manifest

```toml
[fyx_ui]
api_version = "0.3.0"
component = "ui.wasm"
capabilities = ["ui:panel", "ui:picker", "ui:commands", "network:fetch", "process:spawn"]

[fyx_ui.network]
domains = ["api.example.com", "*.cdn.example.com", "localhost:8080"]

[[fyx_ui.processes]]
name = "git"                       # resolved on PATH outside the worktrees
args = ["status", "--porcelain=v2", "**"]   # optional argument patterns (Q2)

[[fyx_ui.processes]]
name = "eslint"
path = "node_modules/.bin/eslint"  # worktree-relative: needs a trusted worktree

[fyx_ui.process]
env = ["NO_COLOR", "GIT_DIR"]

[[fyx_ui.commands]]
id = "todo.next"
title = "Next TODO"

[[fyx_ui.picker_sources]]
id = "todos"
picker = "acme.find/everything"    # "<extension>/<picker surface>"
```

`UiManifest` (`ui_ext_wasm.rs:197-207`) has `deny_unknown_fields`, so a 0.2 host would fail a 0.3
manifest with "unknown field `network`" before it reports the version. migration.md §2 reads
`api_version` first.

Every capability, domain, program, environment name and command is listed on the extensions page.
An update that adds a T1 or T2 capability, a domain, a program, an argument pattern or an
environment name is **held** (spec decision 21); one that adds a command, a source or a T0
capability installs silently.

## 1. Decoration extras (plan 4.2)

**Motivation.** 0.2 §L ships block decorations only. The coverage run's decoration users
(`indent-rainbow`, `todo-highlight`, `trailing-spaces`, `errorlens`; `vscode-coverage.md:338-470`)
style text, paint lines and write after the end of a line; none draws a tree. §L also deferred
`decoration-orphaned` and anchoring to a buffer handle.

**Shape** (WIT `decorations`, the 0.3 half):

| Kind | Draws | Editor mechanism |
|---|---|---|
| `inline-style` | foreground, background, underline (straight, wavy), strikethrough, bold, italic on a range | `Editor::highlight_text_key` with a new `HighlightKey::FyxExtension(usize)` (hook, below) |
| `eol-text` | muted text after the end of a line, ≤ 256 bytes | an inlay at the line end, new `InlayId::Extension(usize)` (hook) |
| `inline-text` | an inlay before or after a position, ≤ 64 bytes | the same inlay id |
| `gutter-sign` | one frozen `icon` per line per extension, optional tooltip and click command | painted from an editor `Addon` (hook in `element.rs`) |
| `line-background` | whole lines | `Editor::highlight_rows::<FyxLineBackground>` (public, no hook) |
| `color-swatch` | a small bordered square of `0xRRGGBBAA` | `Inlay::color`-style inlay with the extension inlay id |

**Colours.** `decoration-color` is a theme token or `accent(n)`, the theme's accent colours
(the ones bracket colouring uses), modulo their count. That closes the coverage gap of
indent-rainbow without raw colours: it needs distinct colours per depth, not specific ones.
`color-swatch` is the only raw colour in the world. It is a fixed-size square with a border, so it
cannot paint text or recolour the editor (spec decision 28).

**Namespaces.** `define-namespace(name, options)` declares one of up to 16 per extension.
`set-decorations(namespace, document, lines?, list)` replaces the namespace's decorations in one
document, in `lines` or everywhere. `clear-namespace` removes them from every document. Two
extensions' namespaces never touch.

**Visible-range pull.** With `options.pull = some(selector)`, the host calls
`provide-decorations(request)` for every matching open document whose visible range changes:

- the request carries the document context (§6), the visible lines plus one screen above and
  below, and a reason (`opened`, `scrolled`, `edited`, `refresh`);
- debounced **50 ms** per editor (spec decision 28); at most one request in flight per namespace
  and document; a newer request replaces a queued one;
- the reply replaces the namespace's decorations in exactly those lines, so scrolling away and
  back costs one call, not a full-document pass;
- decorations outside the requested lines stay where the anchors put them until a later request
  covers them.

Eager `set-decorations` still works for results computed elsewhere (after a process or fetch). A
namespace can be both.

**Document targets and `orphaned`.** `decoration-location.target` is `path(…)` (0.2) or
`document(id)`, an open document from a request; it goes away with the document. When the line a
block decoration is anchored to is deleted (the anchor's row range collapses into a deleted
range), the host sends `handle-event(id, orphaned)` once. The block stays where the anchor fell
until the extension moves (`place-decoration` again) or removes it. Path-targeted blocks report
it once per buffer.

**Limits.** 10000 decorations per call and per namespace and document, 50000 per extension; one
gutter sign per line per extension, at most 2 signs drawn per line (the rest show as a "+" with all
tooltips); text lengths as above; 512 decoration effects per call (as §L). Over a cap:
`limit-exceeded`, nothing applied.

**Capability and tier.** `ui:editor-decorations`, T0. Text in requests needs
`workspace:read-buffers` (§6).

**Consent UX hooks.** None (T0). The extensions page lists "Draws in the editor".

**Host and renderer changes.**
- `crates/ui_ext/src/types.rs`: `DecorationTarget`, `DecorationColor`, `StyledDecoration` and its
  kinds, `UiEvent::Orphaned`.
- `crates/ui_ext_wasm/src/ui_ext_wasm.rs`: `HostEffect::{DefineNamespace, SetDecorations,
  ClearNamespace}`; `UiState.namespaces`; the caps; `UiExtension::provide_decorations`.
- `crates/ui_ext_host/src/decorations.rs`:
  - `DecorationTarget::Document(BufferId)` beside the path form;
  - orphan detection in `sync_editor` (`decorations.rs:281-422`): an anchor whose row became part
    of a deleted range;
  - a per-editor `StyledLayer` that keeps anchored ranges per `(extension, namespace)` and writes
    them through the four mechanisms above, rebuilt only for changed namespaces;
  - the pull scheduler: visible range from the editor's scroll position, 50 ms debounce per editor;
  - skip editors marked as agent-review editors (§11, threat 3).
- Upstream hooks (each a few lines marked `fyx:`): `HighlightKey::FyxExtension(usize)` in
  `crates/editor/src/display_map.rs:164`; `InlayId::Extension(usize)` and its match arms in
  `crates/project/src/project.rs:514-531`; extra gutter icons from an `Addon` method in
  `crates/editor/src/element.rs` and the `Addon` trait (`editor.rs:789`). Q6 asks to accept them.
- SDK: `define_namespace`, `set_decorations`, `clear_namespace`, `Extension::provide_decorations`,
  builders for every kind.

**Tests.**
- host: each cap; a namespace id clash is fine across extensions; `orphaned` reaches
  `handle-event`; a 10001-item reply is rejected whole.
- renderer (`decoration_tests.rs`): every kind lands in the right editor API; edits move styled
  ranges; scrolling issues one request after 50 ms; a reply replaces only its lines; a second
  editor on the same buffer gets the same decorations without a second request; deleting the
  anchored line sends `orphaned` once; `accent(9)` wraps.
- manual: the TODO highlighter (pull namespace, `inline-style` + `gutter-sign`) and a gutter-sign
  demo while editing (plan 4.2's done-when).

**Threats.**
1. *Hiding code.* `inline-style` could make text invisible (foreground = background). Mitigation:
   the host never applies a foreground equal to the resolved background at that point; it drops
   the foreground and logs once. `inline-text` cannot replace text, only insert beside it.
2. *Fake diagnostics or fake review UI.* An `eol-text` "✓ reviewed" on agent code. Mitigation:
   extension inlays and signs use their own muted style and never the diagnostic colours of the
   editor's own inline diagnostics; they are never drawn in agent-review editors (§11).
3. *Load.* A guest answering every scroll with 10000 decorations. Mitigation: the debounce, one
   request in flight, the caps, and the 200 ms budget; decorations are applied off the
   render path.

## 2. Picker protocol (plan 4.3)

**Motivation.** In 0.2 a picker is an extension modal with an input and a list (§C): every
keystroke is an `input-changed` call and the extension filters. That works for small lists. Zed's
own pickers filter in the host, keep up at 100000 items, and look the same everywhere. VS Code's
QuickPick, the most used UI API in the coverage run, maps directly onto a host picker.

**Shape.** A new `surface-kind::picker(picker-descriptor)`:
- **Lifecycle is the 0.2 modal's** (§C): declared static or with `add-surface`, opened with
  `show-surface` after a gesture, one modal per window, `dismissed(reason)` through `handle-event`
  for the owner when the user closes it, `hide-surface` closes it without an event. The same
  `ExtensionModal` wraps it, so the same-type trap and the focus rules hold. `mount` is never
  called: the body is Zed's `Picker`, drawn by the host.
- **Items** come from *sources*. The owner's picker surface is its own source (source id = surface
  id). Other extensions contribute sources with `[[fyx_ui.picker_sources]]`, if the descriptor has
  `open-to-sources`.
- **A session** is one showing. When it opens, every source gets
  `picker-source-event(source, session, opened(query))` and answers with
  `picker.set-items(source, session, items, append)`, now or later (after a fetch).
- **Matching** is the host's: `fuzzy::match_strings` on a background thread over `filter-text` or
  `label`, the matched characters highlighted, ties broken by source order (owner first, then
  sources by manifest `priority`, then extension id). With `dynamic: true` the host does not
  filter: every query change (debounced 100 ms) goes to the sources as `query-changed` and they
  send new items (workspace symbol search, a web search).
- **Selection** is the host's. `highlighted(key)` goes to the source of the highlighted item,
  coalesced and at most every 50 ms, for a live preview (the extension may show a decoration or
  update a panel; the picker itself shows no extension content).
- **Multi-select** (`multi-select: true`): Tab marks or unmarks the highlighted item (fzf's key);
  Enter accepts the marked items, or the highlighted one when none is marked.
- **Actions**: Enter, plus at most two `picker-action`s on alt-Enter and secondary-Enter (ctrl, or
  cmd on macOS), listed in the footer. `accepted(keys, key)` goes to each source whose items were
  accepted, with its own keys only, and grants the gesture (§0.4). Then every source gets
  `closed`.

**Limits.** 10000 items per source and session, 2000 per `set-items` call, 50000 per session;
labels and details ≤ 512 bytes; keys ≤ 128 bytes; 16 sources per picker (more are ignored, by
priority, with a log); `set-items` for a closed session is dropped without error.

**Capability and tier.** `ui:picker`, T0, for owning a picker and for contributing a source.

**Consent UX hooks.** None. The picker footer names the extension of the highlighted item, so a
contributed item cannot pass as the owner's.

**Host and renderer changes.**
- `types.rs`: `SurfaceKind::Picker(PickerDescriptor)`, `PickerAction`, `PickerItem`.
- `ui_ext_wasm.rs`: `HostEffect::SetPickerItems`; `UiExtension::picker_source_event`; the cap
  checks; `check_surfaces` accepts `ui:picker`.
- New `crates/ui_ext_host/src/picker.rs`: `ExtensionPickerDelegate: PickerDelegate`, the source
  registry (manifest sources by picker key), sessions, the background matcher, coalescing of
  `highlighted`.
- `crates/ui_ext_host/src/modal.rs` (0.2): the modal body is a `SurfaceView` or a
  `Picker<ExtensionPickerDelegate>`.
- `registry.rs`: picker sources from manifests, routing of events to the contributing extension.
- SDK: `Surface::picker`, `set_picker_items`, `Extension::picker_source_event`, a `PickerSession`
  helper that tracks the session and resends items.

**Tests.**
- renderer: typing filters without any extension call; 50000 items stay responsive (the match
  runs off the main thread); `highlighted` is coalesced; Tab marks, Enter accepts the marked keys;
  alt-Enter carries `alt-enter`; Escape sends `dismissed(escape)` to the owner and `closed` to every
  source; a dynamic picker sends `query-changed` once per 100 ms of typing; stale-session items are
  dropped;
- two extensions contribute to one picker, and each `accepted` carries only that source's keys
  (plan 4.3's done-when);
- host: `set-items` caps; `ui:picker` missing → `not-permitted`.

**Threats.**
1. *Spoofed items.* A contributor adds "Accept all agent edits". Mitigation: accepting runs only
   that contributor's own code; the footer shows its name; host actions are not reachable from any
   extension (Not in 0.3).
2. *Leaking the query.* Dynamic pickers send what the user types to every source. Mitigation:
   only to sources the owner opened up (`open-to-sources`); a source with `network:fetch` is shown
   on the extensions page as "receives what you type in <picker>".
3. *Item floods.* Caps above; matching is off the main thread.

## 3. Hover and code lens (plan 4.4)

**Motivation.** Spec decision 28: contributions rendered by the editor's existing hover and code
lens UI. The coverage run's `registerCodeLensProvider` and hover users.

**Shape.** `language.register-provider(kind, selector) -> provider-id`, up to 16 per extension.
The host calls:
- `provide-hover(request)` when the editor asks its semantics provider for a hover at a position
  in a matching document. The reply is `none`, `ready(markdown)` or `pending`; a pending hover is
  answered with `resolve-hover(request, markdown?)` within **3 s** (after a fetch, say). The
  markdown (≤ 64 KiB) joins the language server's hover blocks in the same popover, under a
  separator with the extension's name.
- `provide-code-lenses(request)` for the visible lines of a matching document, with the same
  50 ms debounce and line-range replacement as the decoration pull. A lens is a title (≤ 256
  bytes), a tooltip and an optional `command-invocation`; a click runs `handle-command` with the
  gesture.

**Limits.** 256 lenses per reply, 8 per line; one hover request in flight per provider; a pending
hover is dropped when the popover closes.

**Capability and tier.** `ui:hover`, `ui:code-lens`, T0. Document text needs
`workspace:read-buffers` (§6); without it a provider still gets the path, language and position.

**Consent UX hooks.** None.

**Host and renderer changes.**
- New `crates/ui_ext_host/src/language.rs`:
  - hover: wraps each editor's `SemanticsProvider` (`editor.rs:3314-3320`, trait at
    `editor.rs:11984`) in a `FyxSemanticsProvider` that delegates everything and appends extension
    hovers to `hover()`'s result. It is installed from the `observe_new::<Editor>` that
    `decorations.rs:231` already has. **No upstream edit.** If upstream later resets the provider,
    the wrapper is reinstalled on `EditorEvent`; a test pins this.
  - code lens: an `ExtensionLensSource` that the code lens hook asks per buffer and visible range.
- Upstream hook: in `crates/editor/src/code_lens.rs:214` (`refresh_code_lenses`), merge lines from
  a `CodeLensSource` global into `CodeLensState` before grouping by row, so an extension lens and a
  language server lens on the same line share one line (`CODE_LENS_SEPARATOR`). About 30 lines
  marked `fyx:`; the lens item carries a new action kind instead of a `project::CodeAction`, whose
  `server_id` an extension does not have (Q6 has the fallback without the hook).
- `ui_ext_wasm.rs`: `UiExtension::{provide_hover, provide_code_lenses}`, `HostEffect::
  {RegisterProvider, UnregisterProvider, ResolveHover}`.
- SDK: `register_provider`, `resolve_hover`, the two trait methods.

**Tests.** A hover from a fake backend appears under the language server's in one popover; a
`pending` hover resolved after 1 s appears, one resolved after 4 s does not; a lens click runs
`handle-command` with `source: code-lens` and the gesture; lenses follow edits; a provider whose
selector does not match is never called; with code lens disabled in settings no lens request is
made. Gallery demos for both (plan 4.4's done-when).

**Threats.**
1. *Phishing links in hovers.* Only `https:` links are clickable; `file:`, `command:`,
   `zed:`, `fyx-ext:` and others render as text. The hover block is labelled with the extension.
2. *Slow providers.* The 200 ms budget; a provider that times out three times in a row is skipped
   for 60 s with one `warn`.
3. *Reading code without consent.* No text without `workspace:read-buffers` (T1).

## 4. Virtual documents (plan 4.4)

**Shape.** `open-virtual-document(path)` (needs the gesture) opens a read-only editor tab for
`fyx-ext://<extension id>/<path>`, or focuses it. The host pulls the text with
`provide-virtual-document(path) -> {text, language, title}` when the tab opens, after
`invalidate-virtual-document(path)`, and after the extension reloads. `close-virtual-document`
closes its tabs. Owner-targeted decorations, hovers and lenses can target it: requests for it
carry `document-origin::virtual(path)`.

**Semantics.**
- The buffer is a local buffer with `Capability::ReadOnly` and no `File`, so no language server
  attaches and Save writes nothing; Save As is the user's own act and stays available.
- The tab title is "<extension name>: <title or last path component>", with the extension icon, so
  it cannot pass as a project file.
- While an answer is pending (after a reload), the tab keeps the last text and shows "Updating…".
  An error reply shows the message in the editor's place and a Retry button.
- Not restored across restarts in 0.3: the item is not a `SerializableItem`.

**Limits.** 8 MiB per document, 64 open documents and 64 MiB per extension, path ≤ 256 bytes with
the worktree-path rule.

**Capability and tier.** `ui:virtual-documents`, T0.

**Host and renderer changes.** New `crates/ui_ext_host/src/virtual_documents.rs` (an `Item`
wrapping a read-only `Editor`, the path registry, pull and retry); `ui_ext_wasm.rs` effects and
`UiExtension::provide_virtual_document`; `registry.rs` closes the tabs on uninstall; SDK wrappers.

**Tests.** Open, invalidate, reload (text kept, then refreshed), error and Retry, 8 MiB + 1 byte
rejected, no gesture → `not-permitted`, the editor refuses typing, no language server starts.

**Threats.** Impersonating a file (title prefix and icon); memory (caps); tab spam (gesture rule,
one tab per path).

## 5. Commands (plan 4.4)

**Shape.** `[[fyx_ui.commands]]` with `id` and `title` (≤ 128 bytes), at most 64 per extension.
They appear in the command palette as "<extension name>: <title>" and can be bound in the keymap:

```json
{ "context": "Workspace", "bindings": { "ctrl-alt-t": ["fyx_ui::RunExtensionCommand", { "command": "acme.todo/todo.next" }] } }
```

Running one calls `handle-command(id, argument, context)` with the gesture (§0.4). `context` has
the source (`palette`, `keymap`, `code-lens`, `gutter-sign`) and the focused editor's document
context, if any (§6). A command of a disabled or failed extension shows a toast instead.

**Semantics.**
- Commands are static, so the palette lists them before the extension has ever run, and the
  extensions page lists them for review. No `when` clauses in 0.3: a command that does not apply
  says so in a notification.
- `RunExtensionCommand { command: "<extension>/<id>" }` is one parameterised action, hidden from
  the palette itself (`CommandPaletteFilter::hide_action_types`), so there is one action type for
  every extension command.
- Extensions cannot bind keys. A manifest key suggestion is Q12.

**Host and renderer changes.**
- New `crates/ui_ext_host/src/commands.rs`: the action, the palette entries, dispatch to
  `handle-command`, a toast for a missing extension.
- Upstream hook: `command_palette_hooks` gets a global `ExtraCommands` provider (`Rc<dyn Fn(&App)
  -> Vec<(SharedString, Box<dyn Action>)>>`), and `CommandPalette::new`
  (`crates/command_palette/src/command_palette.rs:113-137`) chains it after `available_actions`.
  About 15 lines marked `fyx:`. The existing `GlobalCommandPaletteInterceptor` is not usable: it
  holds one interceptor and vim replaces it (`crates/vim/src/state.rs:792`).
- `UiManifest.commands`; `UiExtension::handle_command`.

**Tests.** The palette lists the command and runs it; fuzzy search finds it by title; a keymap
binding runs it; the call has the gesture (`show-surface` on a modal inside it succeeds); a
command id not in the manifest is rejected at load; vim mode on does not hide the entries.

**Threats.** *Impersonation* ("Agent review: accept all"): the "<extension name>:" prefix is the
host's, and titles starting with a host namespace (`agent`, `editor`, `workspace`, `fyx`, `zed`)
are rejected by `fyx-ext check` and at load. *Keymap hijack*: not possible, extensions bind nothing.

## 6. Document context and T1 reads ("N", not in the plan)

**Motivation.** Spec decision 21 defines T1 capabilities (`workspace:read-buffers`,
`workspace:read-files`, `workspace:diagnostics`) and P1 builds their install-time dialog, but no
WIT version adds any way to read. §1, §3 and §5 are of little use without the text they are about:
a TODO highlighter needs the lines, errorlens needs diagnostics. And imports cannot read buffers:
they run on the extension's tokio queue, while buffers live on the main thread. **Q3** asks the
owner to put this into 0.3.

**Shape (proposed).** No read imports. The host snapshots what a request is about on the main
thread and passes it along, in `documents.document-context`:
- `info`: id, origin (worktree file, own virtual document, or `other` with no name), language,
  version, line count. Always present.
- `text`: the requested lines plus up to 200 around them, ≤ 1 MiB, and `selections`: only with
  `workspace:read-buffers`.
- `diagnostics` in those lines, ≤ 1000: only with `workspace:diagnostics`.

`workspace:read-files` (files that are not open) is not in this draft: it needs an async read
like §0.3 and has no consumer in plan 4.x. It is additive later.

**Why now.** After 1.0 only additive changes are allowed (spec decision 30). New functions and new
variant cases are additive for the SDKs; new record fields are not. If `document-context` ships
without `text`, every provider request record has to be duplicated in 1.x.

**Threats.** Files outside every worktree and untitled buffers are `other`: no path, no text. The
`private_files` setting (`.env`, keys) is honoured: such documents never carry text. With
`network:fetch` also granted, the T2 prompt says the extension can send what it reads (§7).

## 7. T2 consent at first use (plan 4.5)

**Model** (spec decisions 21, 26). The P1 permission broker (`ui_ext_host::permissions`) keeps one
state per extension and T2 capability: `ask`, `granted`, `denied`, persisted in
`<data_dir>/extensions/fyx/state.json` together with what was approved (the domain list, the
programs with their argument patterns and the environment names). Revocation on the extensions
page takes effect at once: in-flight work is cancelled, later calls return `denied`, and the
extension gets `permission-changed`.

**When the prompt appears.** When the first T2 operation is *applied* (§0.3 step 3), or on
`permissions.request` (with a gesture). The operation waits; its timeouts start after the answer.
One prompt per extension at a time. Dismissing the prompt without answering fails the waiting
operations with `denied`, keeps the state `ask`, and suppresses new prompts for that capability for
10 minutes.

**What it looks like.** A workspace notification in the active window, not a modal, so it never
takes focus from typing:

> **TODO Sync** (by octo, from the Fyx registry) wants to **connect to** api.todo.example and
> \*.cdn.todo.example.
> It can also read your open files, so it could send their content there.
> [Allow] [Don't allow] · Manage on the extensions page

- The buttons ignore input for the first **1 s** after the notification appears, and neither
  Enter nor Escape answers it, so a keystroke or click aimed elsewhere cannot grant anything.
- The "also read" line appears when a T1 read is granted or declared together with
  `network:fetch` or `process:spawn`; with `process:spawn` + any program the host lists as running
  arbitrary code (Q2), the line says "It can run any program on your computer".
- Unsigned and dev extensions carry their badge in the prompt.
- In an untrusted worktree, `process:spawn` and `workspace:edit` are `unavailable` and fail with
  `denied` without a prompt (decision 26); the error message says the worktree is restricted.

**Held updates.** An update that adds a T2 capability, a domain, a program, an argument pattern or
an environment name is held, the old version keeps running, and the page shows "Update needs
approval" (decision 21, failure 1 of spec §4).

**WIT.** `permissions.query(capability)` (sync, from a mirror the host refreshes in `UiState`
before every call), `permissions.request(capability)` (gesture), `io-event::permission-changed`.

**Host and renderer changes.** `crates/ui_ext_host/src/permissions.rs` (P1.5) gains T2 states,
the prompt, the 1 s guard and the 10-minute suppression; `ui_ext_wasm.rs` gets the state mirror
and `HostEffect::RequestPermission`; `install.rs` `describe_capabilities` (`install.rs:29-43`)
gets plain-language lines for every 0.3 capability.

**Tests.** The first fetch shows one prompt for three parallel fetches; Allow starts all three;
Don't allow fails all three with `denied` and persists; a dismiss suppresses prompts for 10 min
(advance the clock); a click on Allow within 1 s does nothing; revoke during a running process
kills it and sends `permission-changed`; an untrusted worktree never prompts; a held update keeps
the old version running.

**Threats.** Prompt fatigue (one prompt per capability, suppression), click-jacking (the 1 s
guard, no keyboard answer), confused consent (the exact domains and programs are in the prompt,
combinations are spelled out), stale consent (grants are bound to the approved lists).

## 8. `network:fetch` (plan 4.6)

**Shape.** `network.fetch(request) -> id`, `cancel-fetch(id)`, outcome `io-event::fetch-done(id,
result<http-response, fetch-error>)`. A host import, not WASI sockets (spec decision 23).

**Semantics.**
- **URL.** `https://` only. `http://` only to a declared `localhost:<port>` or
  `127.0.0.1:<port>` (Q4). No user info (`https://user:pass@…` is rejected). ≤ 8 KiB.
- **Domain list.** `example.com` matches that host only; `*.example.com` matches any subdomain at
  any depth, not `example.com` itself. Hosts are compared after IDNA (punycode) conversion,
  lowercasing and dropping a trailing dot. IP literals never match a domain pattern. The port must
  be 443 unless the pattern names one (`example.com:8443`). `fyx-ext check` and the loader reject
  patterns whose base is a public suffix (`*.com`, `*.co.uk`, `*.github.io`), using the public
  suffix list compiled into the host.
- **Redirects.** Followed by the host, at most 5 hops. Every hop is checked like the first URL
  (scheme, list, address); an off-list hop ends the request with `redirect-not-allowed(host)`. The
  3xx itself is not returned. `http-response.url` is the URL that answered.
- **Addresses (DNS rebinding, SSRF).** The host resolves names itself. For a public pattern every
  resolved address must be a global unicast address: not loopback, private (RFC 1918, ULA
  `fc00::/7`), link-local (`169.254.0.0/16` with the cloud metadata address, `fe80::/10`), CGNAT
  (`100.64.0.0/10`), multicast, unspecified, documentation ranges, or an IPv4-mapped form of
  these. It connects to the address it checked, never resolving twice, so a name cannot pass the
  check with one address and connect to another. `localhost` never goes to DNS. Corporate hosts
  that resolve to private addresses need a user setting (Q4).
- **Proxy.** With a proxy configured in Fyx, a fetch to a declared domain is **refused in 1.0**
  (`other`: "Fyx is set to use a proxy, and network:fetch does not go through proxies yet"; W2c,
  a1c2557b27).
  - **Why:** the host pins the address it checked and keeps the domain for TLS SNI. reqwest's
    CONNECT through a proxy cannot do both, so the proxy would resolve the name again and the
    address check would mean nothing.
  - **What still works:** `localhost` goes direct, as before. Proxy URLs with credentials are
    never used. Fyx reads only its own proxy setting, never environment proxies.
  - **After 1.0:** proxied fetches through a hand-made CONNECT tunnel that pins the checked address
    and keeps SNI. This does not change the WIT.
- **TLS.** The system roots plus webpki roots; no extension-supplied CA, no client certificates,
  no TLS bypass.
- **Headers.** The guest may set any header except the hop-by-hop and framing set (WIT comment),
  `proxy-*` and `accept-encoding`. `accept-encoding` is host-owned because the host decodes the
  body and applies the cap to the decoded size: it sends only the encodings it can decode,
  `accept-encoding: gzip, deflate, br, zstd` (W2c, 1cec81f03f).
- **User-agent.** The host sets `user-agent: Fyx/<major.minor> (extension <id>/<version>)` unless
  the guest sets one (W2c, 9fad627507). Fyx's version comes from the app; the extension's version
  from `extension.toml`, and only when it is a plain version string, otherwise `/<version>` is left
  out.
- **No credentials.** The host adds no cookie, no authorization and no editor token. There is no
  cookie jar; `set-cookie` is returned to the guest as a header.
- **Decompression.** The host decodes `gzip`, `deflate` (zlib, as RFC 9110 defines it), `br` and
  `zstd` (W2c, 1cec81f03f). The 16 MiB cap applies to the decoded body, counted while streaming,
  so a compression bomb ends with `too-large`. zstd frames may use a window of at most 8 MiB
  (`window_log_max(23)`), so a frame cannot make the decoder allocate 128 MiB; a wider window
  fails. Any other `content-encoding` gives `other`. A `content-length` above 16 MiB fails before
  the body is read.
- **The WIT comment** on `headers` names `accept-encoding` among the host-set headers (W0,
  e1da7d5e3f, in `crates/ui_ext/wit/since_v0.3.0/fyx-ui.wit`; also in this folder's
  `fyx-ui-0.3.wit`).
- **Timeout.** 30 s for the whole request, redirects included; the guest may ask for less.

**Limits.** 8 in flight (decision 23), 120 started per minute (token bucket), request body 4 MiB,
response 16 MiB, headers 64 KiB, 64 request headers. Over the in-flight or rate cap: the import
returns `limit-exceeded`.

**Capability and tier.** `network:fetch`, T2 at first use; the domain list is part of the grant.

**Host and renderer changes.**
- New `crates/ui_ext_host/src/network.rs`: a dedicated client, not the editor's shared
  `HttpClient`, built on the workspace's `reqwest` (`crates/reqwest_client`) with
  `redirect::Policy::none()` (the host follows hops itself after each check), a
  `reqwest::dns::Resolve` that filters addresses, no cookie store, the proxy from settings without
  credentials, streaming body with the cap; `FetchBroker` per extension with the counters and the
  cancel handles.
- `ui_ext_wasm.rs`: `HostEffect::{Fetch, CancelFetch}`, the in-flight count in `UiState`, and the
  WASI context built with `allow_tcp(false)`, `allow_udp(false)`, `allow_ip_name_lookup(false)`,
  stated explicitly instead of relying on the default (the network audit marks the default
  "unverified"). `inherit_stderr` (`ui_ext_wasm.rs:536`) stays.
- `UiManifest.network.domains` with the pattern rules; `install.rs` lists them.
- Extension Inspector (P1.7): one line per request (method, host, path, status, bytes, time),
  never bodies or header values.

**Tests** (a local TLS test server with a test CA injected only in tests).
- An undeclared domain → `host-not-allowed` (plan 4.6's done-when); a 17 MiB body → `too-large`,
  and a 1 MiB gzip that inflates to 17 MiB → `too-large`;
- a redirect to an off-list host, to `http://`, and a sixth hop → `redirect-not-allowed`;
- a declared domain whose resolver returns `127.0.0.1`, `10.0.0.1`, `169.254.169.254`,
  `::ffff:10.0.0.1` → `host-not-allowed`; a resolver that answers public then private on the second
  lookup (rebinding) still connects to the checked address;
- `*.example.com` does not match `example.com`, `evilexample.com` or `example.com.evil.net`;
  `*.co.uk` is rejected at load; `EXAMPLE.com.` matches `example.com`; a punycode homograph does
  not match its ASCII look-alike;
- user info in the URL, a forbidden header, `http://` to a public host → errors at the import;
- the ninth request in flight → `limit-exceeded`; cancel → `cancelled`; revoke → `denied` for the
  in-flight ones; reload drops them and the new instance gets no `fetch-done` for old ids;
- the guest's WASI sockets: a probe that opens a TCP socket fails (closes the audit item).

**Threats.**
1. *The `zed:extension` side door.* A package that also ships an `extension.wasm` gets upstream's
   `fetch`/`fetch_stream`, which have **no capability check** (network audit item 8,
   `since_v0_8_0.rs:633-667`), and `process:exec` granted for `*` by default
   (`assets/settings/default.json:2376-2380`). The domain list means nothing then. **Q1.**
2. *Exfiltration to a declared domain.* An extension with T1 reads and `network:fetch` can send
   code to its own server. That is what the grant allows; the prompt says so, the registry reviews
   new domains (decision 16), the Inspector shows every request.
3. *DNS rebinding and SSRF to local services* (routers, `169.254.169.254`, Docker's API on
   `localhost:2375`): the address check with connection pinning, `localhost` only by explicit
   port, HTTPS everywhere else (a rebound name also fails the certificate check).
4. *Redirect laundering* (an on-list open redirector to an off-list host): every hop is checked.
5. *Lookalike domains* (IDN homographs, `example.com.evil.net`): comparison after IDNA on whole
   labels.
6. *Credential leaks*: no cookies, tokens or proxy credentials from the editor; the guest sends
   only what it holds itself.
7. *Resource abuse* (the editor as a DDoS client): 8 in flight, 120 per minute, 30 s, and the
   registry's block list (decision 19).
8. *Response bombs*: decoded-size cap while streaming, header cap, memory cap of the store (Q7).

## 9. `process:spawn` (plan 4.7)

**Shape.** `process.spawn(request) -> id`, `write-stdin`, `close-stdin`, `kill`; outcomes
`process-output` (≤ 64 KiB chunks), then one `process-exited`, or one `spawn-failed` instead.

**Semantics.**
- **Programs.** `request.program` is a `name` from `[[fyx_ui.processes]]`, never a path. The host
  resolves it to an absolute path itself and spawns that path, so the OS never searches:
  - on `PATH`, skipping every `PATH` entry that is empty, relative (`.`), or inside any worktree
    of any open workspace after canonicalisation (spec failure 4: a planted `git.exe` in the
    worktree is never run);
  - on Windows with `PATHEXT` limited to `.exe` and `.com`. `.bat` and `.cmd` are refused
    (`not-found`) because `cmd.exe` re-parses their arguments (the BatBadBut class,
    CVE-2024-24576); Windows' own search of the application and current directory never happens,
    since the path is absolute;
  - with `path = "…"` in the manifest: a worktree-relative program (`node_modules/.bin/eslint`),
    only in a trusted worktree (decision 24), resolved inside the worktree of `cwd`, and refused if
    it leaves the worktree through a symlink.
  - The resolved path is resolved again on every spawn (a `PATH` change applies) and shown in the
    Inspector.
- **Arguments.** Passed as an argument vector, never through a shell. Every argument must match
  the manifest's `args` pattern at its position (`**` matches the rest), as Zed's `process:exec`
  capability does (Q2).
  - **No `args` means no arguments:** the program may run only with none. Write
    `args = ["**"]` to allow any arguments (W2c, a1c2557b27).
- **Working directory.** A directory inside a worktree (`file-ref`), default the first worktree
  root; refused if it leaves the worktree through a symlink. No worktree: `unavailable`.
- **Environment.** Cleared, then `PATH`, `HOME` (`USERPROFILE`, `SystemRoot`, `TEMP` and `TMP` on
  Windows, where programs fail without them), `LANG` and `LC_*` from Fyx's environment, the
  names in `[fyx_ui.process].env` taken from Fyx's environment, then the guest's `env` pairs,
  which may only use those declared names. Never allowed, even when declared: `LD_*`, `DYLD_*`,
  `NODE_OPTIONS`, `PYTHONSTARTUP`, `PYTHONPATH`, `PERL5OPT`, `RUBYOPT`, `BASH_ENV`, `ENV`,
  `GIT_CONFIG_*`, `GIT_SSH`, `GIT_SSH_COMMAND`, `GIT_EXEC_PATH`, `GIT_ASKPASS`, `SSH_ASKPASS`.
  The loader rejects a manifest that declares them.
- **Process tree.** Every process starts in its own group and is killed with everything it
  started:
  - Unix: `process_group(0)`; kill sends `SIGTERM` to the group, then `SIGKILL` after 2 s. On Linux
    `PR_SET_PDEATHSIG(SIGKILL)` in `pre_exec`, so the child dies if Fyx crashes.
  - Windows: a job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, the process assigned
    **before it runs** (created suspended, then assigned, then resumed, or with
    `PROC_THREAD_ATTRIBUTE_JOB_LIST`), so a grandchild started in its first instant is in the job
    too. Closing the job handle kills the tree, which also covers a Fyx crash.
  - `CREATE_NO_WINDOW` on Windows (Zed's `util::command` already does), no console flashes.
- **When processes die.** `kill`; disable, hot reload (spec failure 3), uninstall, revoking
  `process:spawn`, the extension trapping, and Fyx quitting. The last workspace window closing does
  not kill them, since the extension lives across windows.
- **I/O.** Output is read as it comes, in chunks of up to 64 KiB per event. No pty: programs see
  pipes (`isatty` false). stdin is closed at start unless `stdin: true`.

**Limits.** 4 processes per extension (decision 24), 256 arguments and 128 KiB of arguments, 32
environment pairs, `write-stdin` ≤ 1 MiB per call and 4 MiB queued per process, output backpressure
(§0.3: the pipe is not read while 256 `handle-io` calls are queued, so a chatty process blocks on
its own write). No wall-clock limit: watchers and servers are legitimate.

**Capability and tier.** `process:spawn`, T2 at first use, denied without a prompt in an
untrusted worktree (decision 26). The program list, argument patterns and environment names are
part of the grant.

**Host and renderer changes.**
- New `crates/ui_ext_host/src/processes.rs`: resolution, environment, spawning with groups and job
  objects (`windows` crate on Windows, `libc` on Unix), output pumps with backpressure, kill on
  every path above, a per-extension `ProcessSet` owned by the registry entry so dropping the entry
  kills them.
- `ui_ext_wasm.rs`: `HostEffect::{Spawn, WriteStdin, CloseStdin, Kill}`, the count in `UiState`.
- `registry.rs`: reload and unload kill the set before the new instance mounts (P1.3).
- `UiManifest.processes`, `UiManifest.process.env`, the forbidden-name check; `install.rs` lists
  them, marking programs on the interpreter list (Q2).

**Tests.**
- A worktree containing `git` / `git.exe` and a `PATH` with `.` first: the system git runs (spec
  failure 4, plan 4.7's done-when); a worktree directory on `PATH` is skipped;
- reload while a process tree runs (a shell that starts a sleeping grandchild): no process of the
  tree survives (plan 4.7); the same for disable, revoke and uninstall; on Windows a grandchild
  started immediately is killed with the job;
- an undeclared program → `not-found` at the import; `foo.cmd` on Windows → `not-found`; a
  `path` program in an untrusted worktree → `denied`;
- environment: an undeclared name in `env` → `invalid-argument`; `LD_PRELOAD` declared → load
  error; the child sees only the filtered set (it prints its environment);
- the fifth process → `limit-exceeded`; output of 10 MiB arrives in order in ≤ 64 KiB chunks;
  a guest that never returns quickly from `handle-io` stalls the child instead of growing memory;
- `kill` on an exited id → `invalid-argument`; `process-exited.killed` is set for host kills.

**Threats.**
1. *The interpreter loophole.* Declaring `node`, `python`, `sh`, `bash`, `pwsh`, `cmd`,
   `osascript` or `git` (with `-c core.sshCommand=…`) is arbitrary code execution outside the
   sandbox: the program list then bounds nothing. **Q2.**
2. *Planted binaries* in the worktree (spec failure 4): `PATH` filtering, absolute spawn, trusted
   worktree for `path` programs.
3. *Argument injection through `cmd.exe`*: no batch files, no shell.
4. *Environment injection* (`LD_PRELOAD`, `NODE_OPTIONS`, `GIT_SSH_COMMAND`): the cleared
   environment and the forbidden names.
5. *Escaping the kill*: a child that calls `setsid` or double-forks leaves the Unix process group.
   Residual risk on macOS and Linux; on Linux the registry entry can also make Fyx a child
   subreaper (`PR_SET_CHILD_SUBREAPER`) so orphans are reparented to Fyx and killed with the set.
   On macOS a Fyx crash can leave the direct child running. Windows job objects have neither gap.
6. *`.git` hooks and repository config* (`core.fsmonitor`, hooks run by `git status`): running
   `git` in an untrusted repository runs its configured programs. That is why `process:spawn` is
   denied in untrusted worktrees (decision 26).
7. *Resource abuse*: 4 processes, no pty, backpressure; CPU use of a declared program is the
   user's to see in the Inspector.
8. *The `zed:extension` side door* for processes: §8 threat 1, **Q1**.

## 10. `workspace:edit` (plan 4.8)

**Shape.** `workspace-edit.apply-edit(edit) -> id`; outcome `io-event::edit-done(id, result)`.
An edit is a label and up to 64 file edits; each names a document or a `file-ref`, optionally the
version it was computed against, and non-overlapping text edits.

**Semantics.**
- **All or nothing.** The host checks every target first (paths, versions, overlaps, the
  protected list), loading buffers of files that are not open. If one check fails, nothing is
  applied and `edit-done` carries the error.
- **One ctrl-z.** Per buffer, all its text edits go into one `buffer.edit(…)` call inside one
  transaction. Zed groups transactions that come within `group_interval` of each other
  (300 ms), so an extension edit right after typing would merge with the typing and one ctrl-z would
  undo both. The host calls `finalize_last_transaction` before and after the extension's
  transaction, so it is exactly one undo step of its own.
- **Several files.** Applied together; the host then shows a notification "<extension>: <label>,
  3 files" with **Show** (opens a multibuffer of the edited ranges, built like
  `Editor::open_project_transaction`, in which one ctrl-z undoes the whole edit) and **Undo**
  (undoes each buffer's transaction). In each file's own editor one ctrl-z undoes that file's
  part. The notification never takes focus (Q5).
- **Nothing is saved.** Edited buffers stay dirty. Autosave, if the user enabled it, saves them as
  it would the user's typing.
- **Versions.** `expected-version` is the `document-info.version` the extension saw; a mismatch is
  `conflict`. Without it, the edit applies to the current text, and ranges past the end clamp.
- **Create.** `create: true` creates an empty file at a new path in an existing directory, then
  applies the edits. No delete, rename or move (Not in 0.3).
- **Where it may write.** Only files inside a worktree of a local or SSH project the user opened:
  - the path is worktree-relative with the path rule; `document` targets must have origin `file`;
  - the target is refused if it, or a parent, is a symlink leaving the worktree (the worktree
    entry is `is_external`), or if it is ignored-and-excluded (`file_scan_exclusions`);
  - **protected paths** are refused with `not-allowed`, **in every security mode**, shield or not
    (W2c, e740fd00d5):
    - the directories `.git`, `.fyx` (agent review rules, so an extension cannot set
      `auto_accept`), `.zed` (settings, tasks: code execution), `.vscode` and `.idea`, at any depth;
    - every agent config, from the one list `ui_ext::protected` that the agent shield
      (`fyx_guard`) uses too: `.claude/`, `.codex/`, `.gemini/`, `.agents/`, `.cursor/`,
      `.mcp.json`, `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `.rules`, `.cursorrules`,
      `.vscode/tasks.json`, `.vscode/launch.json`;
    - the worktree's `read_only` patterns, links out of the project, and files matching the
      `private_files` setting (`fyx_guard::extension_write_denial`);
  - collaboration guests (a project shared by someone else) are refused: that is another person's
    project;
  - untrusted worktree: `denied` without a prompt (decision 26).

**Limits.** 64 files, 4096 text edits and 4 MiB of new text per edit, 4 edits in flight, files
over 16 MiB refused, label ≤ 128 bytes.

**Capability and tier.** `workspace:edit`, T2 at first use.

**Host and renderer changes.**
- New `crates/ui_ext_host/src/workspace_edit.rs`: target resolution through `Project`
  (`find_project_path`, worktree entries for symlink and exclusion checks), the protected list,
  buffer loading, the transaction rule above, the notification with Show and Undo, the
  multibuffer.
- `ui_ext_wasm.rs`: `HostEffect::ApplyEdit`, the in-flight count.
- `types.rs` / `crates/ui_ext/src/io.rs` (new, the model of §0.3's I/O types): `WorkspaceEdit`,
  `TextEdit`, `EditError`.

**Tests.**
- A single-file edit in the open editor undoes with one ctrl-z, also when the user typed 100 ms
  before it (plan 4.8's done-when); the typing is still there after the undo;
- a three-file edit: one ctrl-z in the Show multibuffer restores all three; **Undo** in the
  notification does the same; each file's own editor undoes its part;
- `../outside.rs`, an absolute path, a symlink to `/etc`, `.git/hooks/pre-commit`,
  `.fyx/review.toml`, `.zed/tasks.json`, `.env` (a `private_files` match) → `not-allowed`, nothing
  applied, also when the other 63 files are fine;
- a stale `expected-version` → `conflict`; overlapping edits → `invalid`;
- an untrusted worktree → `denied`, no prompt; a collaboration guest project → `not-allowed`;
- the edit is not saved; with autosave on it is saved like typing.

**Threats.**
1. *Code execution through config* (`.zed/tasks.json`, `.vscode/tasks.json`, `.git/hooks/*`,
   `.git/config`): protected paths.
2. *Weakening agent review* (`.fyx/review.toml` with `auto_accept = "**"`): protected path.
3. *Writing outside the project* (`..`, absolute paths, symlinks): the path rule, `is_external`.
4. *Silent sabotage* (a one-character change in a file the user is not looking at): multi-file
   and not-open edits always raise the notification with Show and Undo; the Inspector logs every
   edit with its label.
5. *Undo confusion*: the finalize rule keeps the extension's step separate from the user's.
6. *Re-landing rejected agent code*: an extension with `agent:observe` and `workspace:edit` could
   write a hunk the user rejected. The edit is labelled and undoable like any other; the prompt
   for the second of the two capabilities says "it can see agent edits and change files" (§11).

## 11. `agent:observe` (plan 4.8)

**What the model has.** `agent_review::AgentReview` (`crates/agent_review/src/agent_review.rs`)
holds open `Review`s (id, session, turn, the `.fyx/review.toml` rule that applied, source
`Permission` or `Write`, the `ProposedEdit` with its hunks, whether it was submitted), per-turn
history (`TurnEntry`: path, resolution, rule), unreviewed changes per turn, and review mode on or
off. It emits `AgentReviewEvent::{Opened, Updated, Resolved, Closed, TurnEnded,
UnreviewedChanges}` (`agent_review.rs:185-197`). Only the host resolves a review (module doc,
`agent_review.rs:1-6`).

**What 0.3 exposes.** The `agent-observe` interface, after `start()`:
- `snapshot(list<review-info>)` once, then `opened`, `updated`, `resolved` with the full
  `review-info`, `closed(review)`, `turn-ended(turn, files)`, `unreviewed-changes(turn, files)` and
  `review-mode(bool)`, each mapped one to one from the events above (mode from observing
  `AgentReview`).
- `review-info`: opaque review, session and turn ids (per extension, §0.5); the file as a
  worktree-relative `file-ref`, or `none` for a file outside every worktree, in which case no hunk
  is sent either; source; rule kind (not the pattern or rules file path); resolution; submitted;
  hunks with rows, old and new text, decision and the user's comment, cut at 256 KiB with
  `truncated`.
- Not exposed: the ACP session id, the agent's name, the thread's messages, permission request
  ids, absolute paths, the review pane's state.

**What it cannot do.** There is no function in the world that writes to `AgentReview`: no accept,
reject, comment, submit, decide-turn or set-mode. The guarantee is structural:
- `agent-observe` has `start` and `stop` only;
- no interface dispatches host actions, so `agent_review::AcceptAllHunks` and the other actions
  (`review_pane.rs` exports) are unreachable; extension commands are only
  `RunExtensionCommand`, which calls the extension itself (§5);
- extension UI is kept out of the review UI: decorations, inlays, signs, hovers and lenses are not
  drawn in editors the review pane owns (an `Addon` marker `AgentReviewEditor` set by
  `review_pane.rs`, checked by `decorations.rs` and `language.rs`), and extension modals cannot
  open without the user's gesture on that extension (§0.4);
- `workspace:edit` refuses `.fyx/` (§10), so review rules cannot be changed either.

**Limits.** 256 KiB of hunk text per review, 1000 reviews in a snapshot (oldest dropped first,
with `truncated` on the snapshot's last item), event delivery through the §0.3 queue
(backpressure: the host keeps only the latest `updated` per review while the queue is full).

**Capability and tier.** `agent:observe`, T2 at first use. Review mode off means no reviews and
no events, only `review-mode(false)`.

**Host and renderer changes.**
- New `crates/ui_ext_host/src/agent_observe.rs`: subscribes to `AgentReview::global`, maps
  reviews, assigns opaque ids, truncates, forwards as `io-event::agent`.
- `crates/agent_review/src/review_pane.rs`: register the `AgentReviewEditor` addon on its editors
  (a Fyx crate, no upstream edit).
- Integrator: `ui_ext_host` gains a dependency on `agent_review` (`Cargo.toml` is integrator-only;
  check that `agent_review` does not depend on `ui_ext_host`, which would be a cycle; today it
  does not).

**Tests.** The snapshot after `start` matches the open reviews; each event kind arrives once in
order; a review of `/etc/hosts` arrives with `file: none` and no hunks; a 1 MiB hunk is cut at
256 KiB; the world exports and imports contain no way to decide a review (a test over the parsed
WIT that no `agent-observe` function takes a review id); a decoration placed on a path that the
review pane shows is not drawn there; revoking stops events at once (plan 4.8's done-when:
"observe cannot accept or reject").

**Threats.**
1. *Exfiltrating unreviewed agent code* with `network:fetch`: the combined prompt line (§7), the
   Inspector, registry review.
2. *Influencing the decision* by UI spoofing inside the review pane: no extension drawing in
   review editors; no gesture-free modals.
3. *Changing the rules* through `.fyx/review.toml`: protected path.
4. *Correlating sessions across extensions*: opaque per-extension ids.

## 12. Images (plan 4.10–4.12)

Added 2026-11-18 at the owner's request ("Давай заплануємо те що ти пропонуєш"). It ranks below
the rest of P4: if P4 slips, images stay in 0.3 and canvas (§13) moves to 0.4 (spec decision 28).

**What.** An `image` node shows a PNG or SVG from the extension's package. The most common uses
are a logo, a preview of an image file, and a diagram the extension generated.

```wit
record image-props {
    source: image-source,      // asset("assets/logo.svg") or generated(handle)
    alt: string,               // required, 1–256 bytes
    width: option<u32>,        // logical px, 1–2048
    height: option<u32>,
    fit: image-fit,            // contain | cover
    tint: option<color-token>, // SVG drawn as a one-colour mask, like host icons
}
```

**Sources.**

- **Package assets.** `assets/` files only: the path is relative, uses `/`, contains no `..`, and
  resolves inside the installed version folder (canonicalized, like `component`). Assets are
  covered by the package hash and the registry signature, and they change only with an update.
- **Generated.** `images.add-image(format, bytes)` takes at most **1 MiB** and returns a `u32`
  handle. `remove-image` frees it, and a reload or disable frees every handle. At most **32**
  live generated images per extension.
- **No other source.** No URLs, no `file:` paths, no `data:` URIs in `asset`. There is no network
  at all: the component has none (`deny_network`), and neither do the decoders (below).

**Limits.**

| Limit | Value | Why |
|---|---|---|
| Encoded file | 2 MiB (asset), 1 MiB (generated) | Package cap is 50 MiB; a panel image larger than this is a mistake |
| Decoded size | at most 2048 × 2048 pixels, after the device scale factor | 16 MiB RGBA, the most one image can cost |
| Decoded pixels per extension | **64 MiB** (decided, Q13) | Four full-size images or about a thousand 64 × 64 thumbnails. Past it, new images show their alt text with "image limit reached" in the log |
| Formats | PNG (first frame of an APNG), SVG | `png` 0.18 and `resvg`/`usvg` 0.46 are already in `Cargo.lock` (GPUI uses them); JPEG, GIF and WebP later, additively |
| Concurrent decodes | 2 per extension, 4 in total | Bounds transient memory (below) |
| Where | panel trees and panel list rows | Not status items, modals, pickers or decorations: those sit next to host UI and would be easy to spoof (threats). An `image` there rejects the update (`invalid-argument` for a push) |

**Errors (decided 2026-11-18).** Each check runs at the earliest point where it can be made:

| Case | When it is checked | Result |
|---|---|---|
| `asset` path syntax (relative, `/` separators, no `..`, root, drive, backslash or `:`) | when the update is applied | the whole update is rejected (`invalid-argument` for a push; a resync for a reply), like any invalid props |
| asset exists, its size (2 MiB), its dimensions (2048 × 2048), its format | at decode | the alt placeholder with an error icon, and one log line naming the file and the reason; the update stands |
| `generated` handle returned earlier in the same call | — | usable: effects apply in call order, so `add-image` is applied before the update that uses it |
| `generated` handle never returned, or returned later in the same call | when the update is applied | a reference error: the update is rejected, like a patch that names a missing node |
| `generated` handle that was removed (`remove-image`, or freed by a reload) | at draw | the alt placeholder and one log line; no trap, no rejected update |
| `tint` on a PNG | at decode | ignored, one log line; `fyx-ext check` warns when a mounted tree has it |

**Decoding.** Decoding never happens on the main thread and never inside `render()`.

1. The first time a node needs an image at a given size and scale, the host queues a decode on
   the background executor. Until it finishes, the node draws a muted box of its size with the
   alt text.
2. Decoded images are cached per extension, by (source, pixel size). An LRU evicts images that
   are not on screen once the 64 MiB budget is reached.
3. A decode that fails, or takes longer than **250 ms**, leaves the alt text and an error icon.
   One log line names the file and the reason.

**PNG.** Read the IHDR first and refuse anything over 2048 × 2048 before allocating. Decode with
`png::Limits { bytes: 16 MiB }`. APNG: first frame only. Interlaced and 16-bit images are
converted to 8-bit RGBA.

**SVG.** Parse with `usvg` using these `Options`:

- an `image_href_resolver` that refuses every `<image>` href (external files, URLs and `data:`);
- no resource directory;
- fonts only from Fyx's bundled fonts, so a system font list cannot be probed.

Then check the parsed tree before rendering: at most 100 000 nodes, at most 16 nesting levels of
`<use>`, and blur filters with a standard deviation of at most 64. Render with `resvg` into a
pixmap of the target size. usvg runs no scripts and does no network, and the resolver above
closes the file path. An SVG with an external reference, or a `<script>`, `<foreignObject>` or
CSS `@import`, is still drawn without that part, and the log says so.

**Memory math.**

- Steady state: decoded RGBA is width × height × 4 bytes. One image is at most 16 MiB, and one
  extension at most 64 MiB.
- Transient, per decode: about 2 MiB of encoded input, plus the 16 MiB output buffer, plus one row
  of zlib and filter state for PNG, or the usvg tree for SVG (bounded by the 100 000-node cap,
  roughly 20 MiB). That is at most about 40 MiB.
- With 4 concurrent decodes, the worst case is about 160 MiB for under a second. It is not
  charged to the component's 256 MiB, because decoding runs in the host.

**Theme.** An image has its own colours, the second exception to the theme-token rule after
§1 `color-swatch`. `tint` lets icon-like SVGs follow the theme. The design accepts the exception:
a picture of a file cannot be made of tokens.

**Threats.**

1. **Decoder bombs.** The dimension check runs before allocation. The output byte limit, the
   node, depth and filter caps, the 250 ms time cap and the 2 of 4 concurrency limit bound CPU and
   memory. A crash in a decoder is a host bug, so the decoders run inside `catch_unwind`, and a
   panic counts as a failed decode.
2. **Spoofing host UI.** An image could draw a fake "Allow" button, a fake review pane or a fake
   Fyx dialog. Three things limit this:
   - Images exist only inside the extension's own panel, under the panel header that names the
     extension, and they are clipped to their box.
   - They are not in status items, modals or pickers.
   - They are never interactive.

   Host prompts (consent, review, security) are host modals, which an extension cannot draw.
   Registry review (decision 16) does not look at assets, so the block list is the remedy for a
   phishing extension.
3. **Tracking and exfiltration.** No network, and no remote references in SVG, so an image cannot
   phone home or carry data out in a URL.
4. **Path tricks.** `asset` paths are checked like `component` (no `..`, no root, no drive, no
   backslash). They are resolved and canonicalized, and must stay inside the version folder,
   which also stops a link out of it.
5. **Resource exhaustion through churn.** Calling `add-image` and `remove-image` in a loop is
   bounded by the 32 live handles, the 64 pushes per call, and the decode queue. A full queue
   makes `add-image` return `limit-exceeded`.

**Capability.** `ui:images`, tier **T0**: it shows pixels in the extension's own panel and reads
nothing. The extensions page lists it as "shows images".

**Tests.**

- `ui_ext` validation: the alt text is required; sizes and paths are checked; an `image` in a
  status item, modal, picker or decoration tree is rejected; 0.1 and 0.2 components never see
  these props (version gating).
- Decoder fixtures in `crates/ui_ext_host/tests/images/`, each checked to be refused or capped
  without a large allocation:
  - a 1 KiB PNG whose IHDR says 65535 × 65535;
  - a zlib bomb inside a 2048 × 2048 PNG;
  - a truncated PNG and a 100-frame APNG;
  - an SVG with nested entity expansion ("billion laughs") and one with 17 levels of `<use>`;
  - an SVG with `<image href>` set to `http://`, `file:///C:/Windows/win.ini` and `data:` (none is
    loaded);
  - an SVG with `<script>`, and one with a 10 000 px blur (the time cap).

  A test also asserts that the decoder module has no HTTP client in reach: it takes no
  `HttpClient`, and its resolver is fixed.
- Budget: 5 images of 2048 × 2048 leave the fifth as alt text once the LRU cannot evict.
- Renderer: the placeholder shows before decoding, the image after; `contain` and `cover`
  rectangles are right; nothing is drawn outside the box; `tint` recolours.
- Harness (W4): `fyx_ext_harness` records image nodes and `add-image` calls. Its asset checks
  match the host's.

## 13. Canvas (plan 4.13–4.15)

Ranked last in P4: if P4 slips, the canvas moves to 0.4 and §12 stays (spec decision 28).

**What.** A `canvas` node holds a static vector draw list, for charts, sparklines, gauges and
small diagrams. It is retained like every node: the host keeps the list and paints it, with no
WASM call inside `render()`.

```wit
record canvas-props {
    width: u32, height: u32,   // logical px, 1–4096
    description: string,       // required, 1–1024 bytes
    ops: list<draw-op>,        // line, rect, rounded-rect, circle, path, text
    clickable: bool,
    fit: canvas-fit,           // clip | scale-down
}
```

**Colours.** Every paint is `token(color-token)` or `data(data-color)`, with an `opacity` of 0 to
100 %:

- `data-color` is a fixed palette of eight series colours. The host picks a light and a dark
  variant of each, so they stay distinct on the active theme and under common colour-vision
  deficiencies.
- No raw colours. A theme change repaints the canvas with no call to the extension.

**Ops and caps.**

| Op | Fields |
|---|---|
| `line` | from, to, stroke |
| `rect`, `rounded-rect` | bounds (+ radius), optional fill, optional stroke |
| `circle` | center, radius, optional fill, optional stroke |
| `path` | segments (`move-to`, `line-to`, `quad-to`, `cubic-to`, `close`), fill (non-zero) and/or stroke |
| `text` | baseline point, one line of at most 256 bytes, paint, size (`small`, `medium`, `large`: the host's UI font sizes), anchor, monospace |

- Strokes are 0.5 to 16 logical px.
- Every coordinate must be finite and within ±32768. A NaN or an infinity rejects the update.
- Per canvas: **16384 ops**, 65 536 path segments, and 256 KiB of text.
- Per surface: 16 canvases.
- Canvases appear in panel trees only: not in list rows, status items, modals, pickers or
  decorations.

**Updates.** The whole draw list is sent again in every `set-props`, so there are no op patches
in 0.3. That is simple, and cheap enough:

- **Transfer.** An op lifts as about 40 to 60 bytes of canonical ABI. 10 000 ops are about
  500 KiB, and lifting them into host types and validating them costs **about 1 ms** in a release
  build (target; the bench below measures it). A full 16384-op canvas is about 1.6 ms, well
  inside the 200 ms call budget.
- **Retained memory.** About 48 bytes per op in the host, so about 0.8 MiB for a full canvas plus
  its text. The worst case for one surface is 16 × (0.8 + 0.25) MiB, about 17 MiB.
- **Paint.** One GPUI `canvas` element per node:
  - quads (`paint_quad`) for rectangles and rounded rectangles;
  - `PathBuilder` paths for lines, circles and paths;
  - shaped lines for text (cached by text, size and font).
  - Target: 16384 ops in under 4 ms per frame on the dev machine. If the bench misses it, the host
    rasterizes the list into a texture on update and paints the texture (Q14).

**Animation.** Only through ticks: `request-tick` already has a 100 ms floor (10 frames per
second) and 1 s in the background. A full canvas re-sent every tick costs about 16 ms per second
of host time. There are no host-driven animations, transitions or timers.

**Input.** On a `clickable` canvas, a click sends `canvas-click { canvas, x, y }` in canvas
coordinates. These are mapped back through `scale-down` and the panel's scroll, so they match the
draw list. Hit-testing is the extension's job. A click grants the click-or-Enter activation, like
`click` (§0.4).

**Not in 0.3, planned for 0.4:**

- pointer-move and hover (`canvas-pointer-moved`), coalesced to at most **30 per second**;
- drag (`canvas-drag` start, move, end) at the same rate;
- wheel and zoom.

Each needs one more `ui-event` case, which is additive.

**Clipping and size.** The canvas paints only inside its `width` × `height` box, and that box
only inside the panel. With `clip`, a narrower parent clips the right edge. With `scale-down`,
the drawing shrinks to the parent's width, keeping its aspect ratio, and never grows.

**Accessibility.** `description` is required. It is the tooltip and the accessible label, and the
host shows it instead of the drawing when the canvas is rejected or the extension is stopped.

**Threats.**

1. **Paint and transfer cost.** This is bounded by the caps above, the 100 ms tick floor, one call
   per input event, and the paint bench.
2. **Spoofing.** The same limits as images: panels only, clipped, under the panel header, not in
   status items or modals. Clicks go only to the extension's own handler, never to host actions.
3. **Text shaping cost.** At most 256 bytes per text op and 256 KiB per canvas, with a shaping
   cache.
4. **Float tricks.** NaN, infinities and huge values are rejected at validation, so no NaN reaches
   GPUI's layout or path tessellation.

**Capability.** `ui:canvas`, tier **T0**, listed as "draws charts and diagrams".

**Tests.**

- `ui_ext` validation:
  - each cap at its limit and one past it (16385 ops, 65537 segments, 256 KiB + 1 of text, a
    17th canvas);
  - NaN, infinities and ±32769 coordinates;
  - a missing `description`;
  - a canvas in a list row, status item or modal;
  - paths with only `move-to`, and `close` without a `move-to`.
- Bench (`cargo bench -p ui_ext_wasm`): lifting and validating 10 000 and 16384 ops from a guest,
  target ≤ 1 ms per 10 000 ops. The renderer paints 16384 mixed ops, target ≤ 4 ms. Results go
  into `limits-and-gotchas.md`.
- Renderer: snapshots of each op kind with tokens and data colours in a light and a dark theme;
  nothing drawn outside the box; `scale-down`.
- Hit-test mapping: a click at a known point gives the expected canvas coordinates with
  `scale-down` and with the panel scrolled; a non-clickable canvas sends nothing.
- Ticks: an animated canvas at 100 ms keeps one pending tick and pauses while its panel is hidden
  (0.2 §B).
- Harness and SDK (W4): `fyx_ext_harness` can send `canvas-click` and assert draw lists. The SDK
  has typed builders (`Canvas::new(w, h, "…").rect(…).path(…)`) and a `data-color` helper. The
  demo is a bar-chart panel with click-to-select and a tick-driven sparkline.

## Changes by component (file level)

Paths assume W3's `crates/ui_ext_wasm` (`be53175`); if that move does not land, read
`crates/extension_host/src/wasm_host/fyx_ui.rs` for `ui_ext_wasm.rs`.

| Component | Files | Changes |
|---|---|---|
| WIT (W0) | new `crates/ui_ext/wit/since_v0.3.0/fyx-ui.wit` | this draft's WIT, minus its header comment |
| Model | `crates/ui_ext/src/types.rs` | `SurfaceKind::Picker`, `PickerDescriptor`, `PickerItem`, `DecorationTarget`, `DecorationColor`, `StyledDecoration`, `UiEvent::Orphaned`, `DocumentContext` and friends (§1–§6) |
| Model | new `crates/ui_ext/src/io.rs` | `IoEvent`, `HttpRequest`/`HttpResponse`, `SpawnRequest`, `WorkspaceEdit`, `AgentEvent`, `PermissionState` (§0.3, §7–§11) |
| Host | `crates/ui_ext_wasm/src/ui_ext_wasm.rs` | `UiBindings::V0_3_0`, `MAX_VERSION = 0.3.0`, new `HostEffect`s, `UiState` counters and permission mirror, exports `handle_io`, `handle_command`, `provide_*`, `picker_source_event`; WASI sockets off explicitly; `UiManifest` new tables and lenient version-first parse; memory cap (Q7) |
| Host | new `crates/ui_ext_wasm/src/since_v0_3_0.rs` | `bindgen!` for 0.3, conversions, import impls |
| Host tests | `crates/ui_ext_wasm/src/tests.rs`, `fixtures/probe` (built for 0.1, 0.2, 0.3) | migration.md §6 |
| Renderer | `crates/ui_ext_host/src/backend.rs` | `UiBackend::{handle_io, handle_command, provide_decorations, provide_hover, provide_code_lenses, provide_virtual_document, picker_source_event}` |
| Renderer | `crates/ui_ext_host/src/registry.rs` | apply new effects; route I/O outcomes onto the extension queue; own `ProcessSet`, `FetchBroker`, providers, namespaces, commands, sources per extension; kill and cancel on reload and unload |
| Renderer | `crates/ui_ext_host/src/decorations.rs` | document targets, `orphaned`, styled layer, pull scheduler, skip review editors (§1) |
| Renderer | `crates/ui_ext_host/src/modal.rs` (0.2) | picker body (§2) |
| Renderer | new `picker.rs`, `language.rs`, `virtual_documents.rs`, `commands.rs` in `crates/ui_ext_host/src/` | §2–§5 |
| Renderer | new `network.rs`, `processes.rs`, `workspace_edit.rs`, `agent_observe.rs` in `crates/ui_ext_host/src/` | §8–§11 |
| Renderer | `crates/ui_ext_host/src/permissions.rs` (P1.5) | T2 states, prompt, guards, mirror refresh (§7) |
| Renderer | `crates/ui_ext_host/src/install.rs` | capability descriptions, domain/program/env listing, held-update reasons |
| Agent review | `crates/agent_review/src/review_pane.rs` | `AgentReviewEditor` addon (§11) |
| Upstream hooks | `crates/command_palette_hooks/src/command_palette_hooks.rs`, `crates/command_palette/src/command_palette.rs` | extra palette commands, ~15 lines (§5) |
| Upstream hooks | `crates/editor/src/display_map.rs`, `crates/project/src/project.rs`, `crates/editor/src/element.rs` + `Addon` in `editor.rs`, `crates/editor/src/code_lens.rs` | `HighlightKey::FyxExtension`, `InlayId::Extension`, gutter signs, code lens merge (§1, §3; Q6) |
| SDK | `crates/ui_ext_sdk/src/ui_ext_sdk.rs`, `host_model.rs` | a `v0_3` bindings module and default world, wrappers for every import, default no-op exports, `PickerSession`, `take_host_effects` for the new effects (migration.md §4) |
| TS SDK | `sdks/ts` (P2.5) | the same surface; parity test covers the new imports and exports (spec decision 3) |
| Demos | `extensions/fyx_demo_*` | TODO highlighter (§1), shared picker with two sources (§2), hover and lens (§3), virtual document (§4), commands (§5), a fetch demo against a local server, a `git status` panel (§9) |
| Model | `crates/ui_ext/src/types.rs`, `tree.rs` | `Props::Image`, `Props::Canvas`, `DrawOp` and friends, `UiEvent::CanvasClick`; validation of §12–§13 caps and of where the nodes may appear (§12, §13) |
| Host | `crates/ui_ext_wasm/src/since_v0_3_0.rs` | `images` import (`add-image`, `remove-image`), generated-image store and caps, conversions of the canvas types (§12, §13) |
| Renderer | new `crates/ui_ext_host/src/images.rs` | asset resolution, decode queue on the background executor, PNG and SVG decoders with the limits of §12, per-extension LRU cache (§12) |
| Renderer | new `crates/ui_ext_host/src/canvas.rs`, `view.rs` | the `image` and `canvas` elements, placeholder, clipping, `scale-down`, hit-test mapping for `canvas-click` (§12, §13) |
| SDK, harness | `crates/ui_ext_sdk`, `crates/fyx_ext_harness` | builders for `image` and `canvas`, `add_image`, harness support and asset checks (§12, §13) |
| Demos | `extensions/fyx_demo_*` | an image gallery panel (asset PNG and SVG, a generated SVG), a bar-chart and sparkline canvas (§12, §13) |
| Docs | `docs/fyx/sdk-reference.md`, `extension-authoring.md`, `limits-and-gotchas.md`, `docs/vscode-coverage.md` | new APIs, the limits of §1–§13, consent, coverage updates |

## Open questions for the owner

Ranked by how much of the design depends on the answer and how hard it is to change after 1.0.
[open-questions.md](open-questions.md) has each question with its options, the recommended answer
and its kind (product or technical), for the full 0.3 and for the smaller 0.3 of integration-5.

1. **Close the `zed:extension` side door before T2 ships?** A Fyx package may also contain an
   `extension.wasm` (`zed:extension`), whose `fetch`/`fetch_stream` have no capability check and
   whose `process:exec` is granted for `*` by default (§8 threat 1, §9 threat 8). With it, §8's
   domain list and §9's program list protect nothing. *Recommended:* the Fyx registry refuses
   packages that contain an `extension.wasm` until its network and process imports go through the
   Fyx permission broker with the same lists; a local or unsigned install that has one shows "Also
   runs a Zed extension with unrestricted network and process access" in its install dialog.
2. **May `[[fyx_ui.processes]]` name interpreters and shells?** `node`, `python`, `sh`, `bash`,
   `pwsh`, `cmd`, `osascript`, and `git` with `-c`, run arbitrary code (§9 threat 1).
   *Recommended:* yes, with optional `args` patterns per program; the host keeps a list of
   "runs any code" programs, and for those the prompt says "can run any program on your computer"
   and registry review (decision 16) requires argument patterns or a written reason.
3. **Put document context (§6, T1 reads) into 0.3?** Providers and pull decorations are of little
   use without text, and record fields cannot be added after 1.0 without duplicating the requests.
   *Recommended:* yes, as drafted (text, selections, diagnostics in requests; no read imports);
   `workspace:read-files` later, additively.
4. **`localhost` and private addresses for `network:fetch`.** Local dev servers are useful, but
   `localhost` and LAN hosts are where SSRF hurts (§8 threat 3), and corporate hosts often resolve
   to private addresses. *Recommended:* `localhost`/`127.0.0.1` only with an explicit port in the
   manifest, shown in the prompt as "services on your computer, port 8080"; a public-looking domain
   that resolves to a private address only when the user lists it in a setting
   (`fyx.extensions.network.private_hosts`), never by the manifest alone.
5. **`workspace:edit` without a gesture, and how visible?** Formatters and fixers edit without a
   click. *Recommended:* no gesture needed; one undo step per buffer; any edit that touches more
   than one file or a file not open in an editor raises the non-focusing notification with Show and
   Undo; nothing is ever saved by the host; the protected-path list of §10 is fixed in the host and
   not configurable by extensions.
6. **Four upstream hooks in the editor** (§1, §3): `HighlightKey::FyxExtension`,
   `InlayId::Extension`, gutter-sign painting, the code lens merge, plus the palette hook (§5).
   Each is small and marked `fyx:`, but they sit in hot upstream files. *Recommended:* accept all;
   the fallback without the code lens hook is drawing extension lenses as §L blocks, which gives two
   lens lines when a language server also has a lens on that line.
7. **Memory cap for 0.3 components.** W3 set 32 MiB (`UI_MEMORY_LIMIT`, `ui_ext_wasm.rs:126`), the
   spec says 256 MiB (decision 25), and a 16 MiB response or an 8 MiB virtual document needs a copy
   in guest memory. *Recommended:* 256 MiB for 0.3 components (it is a cap, not a reservation),
   32 MiB kept for 0.1 and 0.2.
8. **How much of a review does `agent:observe` see?** *Recommended:* full hunk text and comments
   for files inside the worktrees, nothing for files outside, opaque ids (§11); the alternative,
   metadata only (paths and counts), makes review dashboards possible but not review-aware linters.
9. **Version lifetime once 1.0 follows 0.3.** Decision 29 removes 0.1 "in 0.4", which never ships.
   **Open, the owner's decision.** The owner's rule, applied as written, removes 0.1 in 1.0 and 0.2
   in 1.1. Integration-5 (decision 4) recommends keeping both through all of 1.x. 0.3 stays as long
   as 1.0, since it is the same world under another package version. The options are in
   [open-questions.md](open-questions.md#q9-version-lifetime-open-the-owners-decision).
10. **Consent prompt form.** *Recommended:* the non-modal notification of §7 with the 1 s guard and
    no keyboard answer; a modal dialog would interrupt typing and be answered by reflex.
11. **Picker contributions across extensions.** *Recommended:* only into pickers that opt in
    (`open-to-sources`); contributors see the query of dynamic pickers, the owner never sees their
    items; a "Search everywhere" host picker made only of contributions is not in 0.3.
12. **Default key bindings in the manifest.** *Recommended:* not in 0.3; users bind commands
    themselves. Suggested keys would need conflict rules against the user's and upstream's keymaps.
13. **Image budget per extension (§12).** **Decided 2026-11-18** (coordinator, for the owner): 64 MiB
    of decoded pixels, 32 live generated images, 2 concurrent decodes per extension (4 in total).
    Also decided: images in panel list rows (the LRU covers virtualized rows), only a `clickable`
    canvas is interactive, and `canvas-click` grants the click-or-Enter activation. Error
    semantics, also decided: asset path syntax fails the update when it is applied; existence,
    size and dimensions are checked at decode and fall back to the alt text with a log line; a
    removed generated handle gives the alt text with a log line and no trap; a handle from
    earlier in the same call is usable and one from later is a reference error; `tint` is SVG
    only, ignored with a log line on PNG, and `fyx-ext check` warns (§12, "Errors").
14. **Canvas paint strategy (§13).** *Recommended:* paint the draw list directly with GPUI each
    frame and switch to a texture cached per update only if the 16384-op bench misses 4 ms. A
    texture costs width × height × 4 × scale² of memory per canvas (up to 256 MiB at 4096 × 4096
    on a 2× screen), which is why it is not the default.

## Not in 0.3

| Item | Why |
|---|---|
| Webviews (`ui:web`) | Layer 6, out of scope (spec §5) |
| Streaming responses, WebSockets, SSE, `wasi:http`, raw sockets | `fetch` covers request/response; streaming needs flow control through `handle-io`; additive later |
| A pty or terminal for processes | Pipes only; a terminal is the user's |
| `workspace:read-files`, file watching, workspace search | §6; additive later |
| Saving, deleting, renaming or moving files | §10: edits leave buffers dirty; file operations need their own review |
| Persistent storage and secret storage for extensions | No consumer in plan 4.x; tokens typed into a `secret` input live in memory; additive later |
| Settings contributions | Needs schema merging into upstream settings |
| Running host actions or built-in commands | Would reach agent review actions (§11); the extension's own commands only |
| Default key bindings, `when` clauses for commands | Q12; §5 |
| Completion, definition, references, symbols, document links, semantic tokens, inlay-hint providers | Coverage API gaps with one or two users each; `inline-text` covers drawing inlays |
| `row-node-click` (0.2 §H, "revisit in 0.3") | Still needs hit-testing inside `uniform_list` rows; the host picker (§2) covers QuickPick items, whose buttons were the main consumer |
| `update-surface`, panel `visibility-changed` | 0.2 deferrals; no new consumer |
| Extension SVG icons in host chrome (buttons, tabs, the dock), raw colours beyond `color-swatch` | 0.2 §I and the theme-token rule; §12 images are content inside a panel, not icons for host chrome |
| Canvas pointer-move, hover, drag and wheel | §13: 0.4, coalesced to at most 30 per second |
| Images in status items, modals, pickers and decorations; clickable images | §12, spoofing; wrap the image in a flex next to a button instead |
| JPEG, GIF, WebP; animated images | §12; additive later. Animation goes through ticks |
| Accepting, rejecting or commenting on agent edits; agent thread content | §11; agent review policy stays with W5 (spec §5) |
| Edits in collaboration guest projects | §10 |
| A "Search everywhere" host picker | Q11 |
| Network and process gating for `zed:extension` components | Upstream API; Q1 handles it at the registry |

## Rejected

| Proposal | Why |
|---|---|
| Synchronous read imports (`read-buffer`) | Imports run on the extension's tokio queue; buffers live on the main thread (§6) |
| Following redirects with the HTTP client's own policy | It cannot check each hop against the list and the address rules (§8) |
| Extension-chosen environment variable names | Loader and interpreter injection (§9) |
| A modal consent dialog | Q10 |
| Code lens and hover drawn as extension trees | The editor's own UI is what users expect, and keeps lenses on one line with the language server's (§3) |
| One decoration API for blocks and styles | Blocks are retained trees with events; styles are ranges in namespaces with a pull. Different lifetimes (0.2 §L's reason for imports over surfaces) |
| `GlobalCommandPaletteInterceptor` for extension commands | One global, replaced by vim (§5) |
