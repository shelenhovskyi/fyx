# Platform 1: a home for third-party UI extensions

Status: **draft for approval**. Evidence: [research report](../../research/reports/Мастхев%20розширення%20та%20функції%20Fyx.md).
Owner decisions (2026-11-17): first-party extensions are postponed; authors write Rust
(TypeScript deferred until after 1.0, decided 2026-11-18, decision 3); distribution is our own
GitHub-reviewed registry; network and processes only with consent; old `fyx:ui` versions keep
working.
Revised for the `fyx:ui` 0.2 decisions, [decided 2026-11-17](../wit-0.2/design.md#decisions-2026-11-17): editor decorations ship in 0.2 as section L, so
notifications and progress become section M; the frozen 101-icon enum (§I) and the modal gesture
rule (§C: a modal opens only within 5 s after a click or Enter) are settled.

## 1. Goal

A third-party author can go from `fyx-ext new` to an extension that users find, install, update
and trust, in one evening, in Rust. A user can install it without reading a
permission essay, and a bad update can be pulled off every machine within hours. The API covers
what the most-loved extensions in other editors are made of: panels, pickers, notifications,
line decorations, code lenses, read-only documents, network and processes behind consent.

## 2. Decisions

### Scope and order

1. No new first-party extensions. `fyx_demo_*` stay as SDK examples and test fixtures only.
2. Four tracks, shipped in this order; each phase is usable on its own:
   - **P1 local platform**: install folder, dev install, hot reload, `fyx-ext` CLI, Rust template,
     consent tiers T0–T1, Extension Inspector, generated reference docs.
   - **P2 `fyx:ui@0.2.0`**: `docs/fyx/wit-0.2/design.md` sections A–L as [decided 2026-11-17](../wit-0.2/design.md#decisions-2026-11-17)
     (L: editor decorations, blocks above or below a line, `ui:editor-decorations`), plus
     section M (notifications and progress, decision 22).
   - **P3 registry**: package format, registry repo and CI, signatures, updates, block list,
     rollback.
   - **P4 `fyx:ui@0.3.0`**: decoration extras beyond 0.2 §L, picker protocol, code lens and hover, virtual documents,
     commands, network and process capabilities (tier T2).
   - **1.0 freeze** after P4 (decision 30).

### Authoring (convenience)

3. **Rust only until 1.0** (owner, 2026-11-18): one SDK, Rust `fyx-sdk` (today's
   `ui_ext_sdk`). A TypeScript SDK (`@fyx/sdk` with `jco componentize`, prototyped in `sdks/ts`)
   is deferred until after 1.0, and whether to do it at all is re-evaluated then. Why: two SDKs
   double the cost of every API version (each world change needs both and a parity test), and
   AI-assisted Rust lowers the barrier TypeScript was meant to remove. The WIT stays
   language-neutral, so another guest language later needs no host change.
4. One CLI, `fyx-ext`, a separate binary in a Fyx crate (no upstream `cli` edit):
   `new <name>`, `build`, `dev`, `check`, `test`, `package`.
5. `fyx-ext check` validates the manifest, and compares capabilities **declared** in
   `extension.toml` with host imports the component **actually uses**: an undeclared import is an
   error, an unused declared capability is a warning.
6. Dev install: "Install Dev Extension" picks a folder and persists it in settings
   (`fyx.extensions.dev_paths`). `FYX_UI_DEV_EXTENSIONS` stays as an override for scripts.
7. Hot reload for dev extensions: a change to the component file, debounced **300 ms**, drops the
   instance and re-mounts every surface in place. Panels stay docked and open; modals close with
   `dismissed(reload)`; ticks are cancelled. A failed reload shows the error in the panel footer
   with Retry; the editor never restarts.
8. A failure no longer means "disabled until restart": the extension page gets **Restart
   extension**, and a dev extension restarts automatically on the next file change.
9. Extension Inspector, a host panel shown in dev builds or with
   `fyx.extensions.developer_mode = true`: live tree, patch log, event log, per-call duration
   against the budget, push count, capability calls and denials, the extension's `log` output.
10. `fyx_sdk::testing`: run the component in wasmtime without GPUI, mount surfaces, send
    events, assert the tree. `fyx-ext test` runs them.
11. The API reference is generated from WIT doc comments on every merge; a doc test fails when
    `docs/fyx/sdk-reference.md` names an item that the WIT does not have.

### Distribution

12. Package: `<id>-<version>.fyxext`, a zip of `extension.toml`, the component, optional
    `assets/`, and `signature.json`. Max **50 MiB** packed, component max **32 MiB**.
13. Installed layout: `<data_dir>/extensions/fyx/<id>/<version>/`, the active version in
    `<data_dir>/extensions/fyx/state.json`. The **previous version is kept** for rollback; older
    ones are deleted after a successful update.
14. Registry = a GitHub repo `fyx-extensions` under the project's org. An entry is
    `extensions/<id>.toml`: source repo URL, commit SHA, path, owner GitHub logins.
15. Registry CI builds every entry **from source** in a pinned toolchain image, runs
    `fyx-ext check` and `fyx-ext test`, signs the package (Ed25519) and publishes package plus
    `index.json` to GitHub Releases. Authors never upload binaries.
16. Review by tier (owner, 2026-11-18). An entry whose release is **T0 or T1** merges by itself on
    green CI. A new T0–T1 extension needs no review. A new version of one needs one of the entry's
    own owners, as the PR's author or by an approving review. Any release that is **T2** needs the
    owner of the registry, for every version. So does anything that adds a T2 capability, a network
    domain or a process name. The owner also approves every `urgent` flag. An owner change needs
    an existing owner of the entry. A **reserved id** (`fyx-*`, …) can be registered only by an
    entry owned by the registry owner alone, with the owner's approval. A PR opened by the owner
    (of the registry, or of the entry) counts as that owner's approval, since GitHub does not let
    an author approve their own PR (accepted 2026-11-18). Every entry PR also gets an **agent review** (owner,
    2026-11-18), a CI job that can only escalate.
    - **What it reviews:** the diff and the built package, for capabilities against code,
      obfuscation, network and process use, and dubious assets.
    - **Isolation:** no signing keys or write token, and the PR content is untrusted data. Text in
      the PR that addresses the reviewer is itself a concern.
    - **Verdict:** a structured verdict with reasons, posted as a comment. `concern` adds the
      `needs-owner` label and blocks auto-merge until the owner approves. `pass` counts toward T0–T1
      auto-merge and never approves anything.
    - **Triggers and merge** (accepted 2026-11-18): the decision runs on `workflow_run`, because a
      review on a fork PR gets a read-only token. Auto-merge uses a GitHub App token
      (`AUTO_MERGE_TOKEN`, Environment `auto-merge`), because a merge with `GITHUB_TOKEN` would not
      start the publish workflow. The owner creates that App with the repository; until then green
      PRs are merged by hand.

    Design: [registry-repo.md](registry-repo.md#review-rules).
17. Client: the Extensions page gets a Fyx tab with search, install, update, uninstall,
    enable/disable, **pin version**, **roll back**, capability list in plain language, publisher,
    source link. Auto-update is on, but a new version installs only when it is **at least 24 h
    old** in the index (security fixes flagged `urgent` skip the wait).
18. Signatures: releases, the snapshot and the block list are signed by **separate keys**, one role
    each (`release`, `snapshot`, `blocklist`). Separate release and block-list keys were decided by
    the owner on 2026-11-18; the snapshot role came with the format hardening of triage-3.
    - **Three keysets.** The client compiles in three keysets, one per role, each with a current
      key and one rotation key. It verifies each document only against the keys of its role, so a
      stolen release key cannot forge a block list.
    - **Where the keys live.** The release and snapshot keys are in the release-signing
      environment, and the block-list key is in its own environment.
    - **The snapshot.** `snapshot.json` pins `index.json` by size and SHA-256, has a `sequence`
      that never goes down and an `expires_at` at most 7 days out, and can revoke keys. CI
      re-signs it on every publish and every 12 h, for 48 h, so it never lapses. An expired or
      rolled-back snapshot stops installs and updates from the index, and installed extensions
      keep running.
    - **Bad signatures.** A package from the registry with a bad signature is refused.
    - **Files from disk.** A `.fyxext` file installed from disk is allowed with an "Unsigned, not
      from the registry" badge and never auto-updates.
    - Formats are in [registry-format.md](registry-format.md), keys and CI in
      [registry-repo.md](registry-repo.md#keys).
19. Block list: `blocklist.json` (id, version range or package hash, reason, `urgent`), signed with
    the block-list key (decision 18) for 48 h at a time (`expires_at`, at most 7 days) and
    re-signed every 12 h. It is fetched at startup
    and every **6 h**. A matching installed version is **disabled at once**, the user gets a
    notification, and if the kept previous version is not blocked the host rolls back to it. The
    last good block list is cached; a fetch failure never disables anything. A list whose signed
    `generated_at` is 7 days old, or whose `expires_at` has passed, is shown as stale and still
    applies.
20. Zed's extension registry API is not called by Fyx code added in this milestone.

### Permissions and isolation (security)

21. Capability tiers, by risk, not by count of prompts:

    | Tier | Prompt | Capabilities |
    |---|---|---|
    | T0 | none, shown on the extension page | `ui:panel`, `ui:status-bar`, `ui:modal`, `ui:notifications`, `ui:editor-decorations`, `ui:picker`, `ui:code-lens`, `ui:hover`, `ui:virtual-documents`, `ui:commands`, `ui:images`, `ui:canvas` |
    | T1 | one dialog at install | `workspace:read-buffers`, `workspace:read-files`, `workspace:diagnostics` |
    | T2 | at first use, per extension, revocable | `network:fetch`, `process:spawn`, `workspace:edit`, `agent:observe` |

    An update that adds a T1 or T2 capability, a network domain or a process name is **held**: the
    old version keeps running and the page shows "Update needs approval". T0 additions install
    silently.

    How this meets the interim consent guard of integration-4 #3 (agreed with W3 for plan 1.5):
    - **T0-only extensions load without a prompt** when a Fyx install dialog showed their
      capabilities: install from file, or install dev UI extension.
    - **A folder that appears without a Fyx install action keeps the one-time Enable.** That
      covers a folder copied into the installed directory, one linked by upstream's Install Dev
      Extension, and one added to `dev_paths` by hand.
    - **After that, only T1 matters.** An update that adds T0 or changes the component loads
      silently. One that adds T1 is held until **Approve**.
22. Section M of 0.2, notifications and progress (design drafted 2026-11-18 in
    [wit-0.2/design.md §M](../wit-0.2/design.md#m-notifications-and-progress-fyxuinotifications)).
    `ui:notifications`, tier T0.
    - **`notify`.** Level, title, body and at most 2 actions, with host-timed auto-dismiss. At most
      **5 notifications per minute** per extension and 3 open at once; extra ones go to the log.
    - **Progress.** `progress-start` / `progress-update` / `progress-end`, shown in the status
      bar or on the owner panel, at most 4 running.
    - **Events.** Actions, closes and progress cancels come back as `ui-event`s to an owner
      surface. An action click grants the modal gesture.
    - **Provenance.** The extension's name and id are always drawn by the host, and the extension
      cannot style them.
23. `network:fetch` is a host import, not WASI sockets. The manifest lists domains
    (`example.com` or `*.example.com`); others fail with `denied`. HTTPS only, except
    `localhost`/`127.0.0.1`. Timeout **30 s**, response cap **16 MiB**, at most **8** requests
    in flight. No editor cookies, tokens or proxy credentials are attached.
24. `process:spawn` runs only programs named in the manifest, resolved on the user's `PATH`
    **outside** the worktree; a worktree-relative program needs a trusted worktree. The
    environment is filtered to `PATH`, `HOME`/`USERPROFILE`, `LANG` and the variables the
    manifest names. At most **4** processes per extension; on Windows each runs in a job object,
    and the whole tree is killed when the extension is disabled, reloaded or uninstalled.
25. Per-instance limits: own wasmtime `Store`, no WASI preopens and no inherited environment,
    linear memory cap **256 MiB**, existing time budgets (200 ms per call, 10 ms epoch). The first
    `mount` after instantiation gets **2 s**, so large components can start.
26. In an untrusted worktree, `process:spawn` and `workspace:edit` are denied without a prompt.
    Revoking a capability takes effect at once: in-flight requests are cancelled and later calls
    return `denied`.
27. Wasmtime: Cranelift only (Winch never enabled); a wasmtime security release is adopted within
    **14 days**; the gate runs an advisory check (`cargo deny advisories`) that fails on open
    wasmtime advisories.

### API functionality and stability

28. **Scope decided 2026-11-19 (owner): the smaller 0.3.** 0.3 ships images, canvas, the T2 imports
    and notifications. The decoration extras, the picker protocol, code lens and hover, virtual
    documents and commands below move to 0.4 ([0.3 open questions](../wit-0.3/open-questions.md)).
    As first drafted, `fyx:ui@0.3.0` added, in this priority order:
    - **Decoration extras** on top of 0.2 §L ([decided 2026-11-17](../wit-0.2/design.md#decisions-2026-11-17)), which already ships block
      decorations (a `fyx:ui` tree above or below a line, anchored by the host, at most 512 per
      extension): the kinds `inline-style`, `eol-text`, `inline-text`, `gutter-sign` and
      `line-background`; extension-owned namespaces with `clear-namespace`; the host asking for
      decorations of the visible range only, debounced **50 ms**, instead of eager placement;
      §L's deferred `decoration-orphaned` event and anchoring to a buffer handle. Colours stay
      theme tokens, except a `color-swatch` value the host renders as a small swatch.
    - **Picker protocol** on the 0.2 modal: host fuzzy matching, multi-select, `row-highlighted`
      event for live preview, picker sources and actions registered by extensions.
    - **Code lens and hover** contributions, rendered by the editor's existing code lens and hover
      UI.
    - **Virtual documents**: `fyx-ext://<id>/<path>` opens a read-only editor with text and a
      language name from the extension.
    - **Commands**: `[[fyx_ui.commands]]` in the manifest appear in the command palette and can be
      bound in the keymap; running one calls `handle-command(id)`.
    - **Images** (owner, 2026-11-18): an `image` node, design in
      [wit-0.3/design.md §12](../wit-0.3/design.md#12-images-plan-410412). `ui:images`, T0.
      - PNG and SVG from the package's `assets/`, or generated by the guest (at most 1 MiB).
      - At most 2 MiB per file and 2048 × 2048 pixels; 64 MiB of decoded pixels per extension.
      - Decoded off the main thread.
      - SVG through usvg/resvg with no external references, no scripts and no network.
      - Clipped to its box, in panels only (not status items, modals, pickers or decorations).
      - Alt text required.
    - **Canvas** (owner, 2026-11-18): a static vector draw list, design in
      [§13](../wit-0.3/design.md#13-canvas-plan-413415). `ui:canvas`, T0.
      - Ops: line, rect, rounded-rect, circle, path, text.
      - Colours from theme tokens and a fixed eight-colour data palette only.
      - At most 16384 ops per canvas; the whole list is re-sent on every update.
      - `canvas-click` with local coordinates. Pointer-move and drag come in 0.4, at most 30 per
        second.
      - Animation only through ticks (100 ms floor).
      - Clipped to its box; a description is required.

    Priority (owner's rule): P4 is the first candidate to cut, and images and canvas rank below the
    rest of P4. If P4 slips, images stay in 0.3 and the canvas moves to 0.4.
29. Version support: every world version keeps working until **two** later minor versions have
    shipped (0.1 until 0.3 inclusive, removed in 0.4). The host runs each component against the
    world it declares. **Decided 2026-11-19 (owner):** no support for pre-1.0
    versions. The frozen 0.3 becomes `fyx:ui@1.0.0`, and loading 0.1 and 0.2 is removed at the
    1.0 cut. The two-version rule applies from 1.0 on.
30. `fyx:ui@1.0.0` = 0.3 once it has run **4 weeks** with no breaking change. After 1.0 only
    additive changes until 2.0, and 1.x stays supported for **12 months** after 2.0 ships.

## 3. Units

| Unit | Where | Does | Depends on |
|---|---|---|---|
| `fyx_ext_cli` | new crate, binary `fyx-ext` | new/build/dev/check/test/package | `ui_ext`, `wasm-tools` crates |
| Install store | `ui_ext_host::install` | layout, `state.json`, dev paths, rollback, version pin | `fs`, settings |
| Registry client | `ui_ext_host::registry_client` | `index.json`, 24 h age gate, signatures, block list | `http_client` |
| Permission broker | `ui_ext_host::permissions` | tiers, consent dialogs, held updates, revocation, persisted grants in `state.json` | install store |
| Host imports | `extension_host::wasm_host::fyx_ui` (moves to a Fyx crate with Phase 5 #9) | `fetch`, `spawn`, notifications, decorations, picker, commands | permission broker |
| Hot reload | `ui_ext_host::registry` | watch, debounce, re-mount in place | install store |
| Extension Inspector | `ui_ext_host::inspector` | dev panel (decision 9) | host call log |
| SDK | `crates/ui_ext_sdk` | bindings, builders, testing harness | WIT |
| Registry | repo `fyx-extensions` + its CI | build from source, review rules, signing, block list | `fyx-ext` |
| Extensions page tab | `ui_ext_host` + one `fyx:` hook in `extensions_ui` | search, install, pin, rollback, capabilities | registry client |

Existing work reused: `script/fyx-new-extension` (branch `claude/fyx-ext-template`) becomes
`fyx-ext new`; the TS prototype (branch `claude/fyx-ts-sdk`) stays in `sdks/ts` as an
unsupported experiment (decision 3); capability chips
(`60507a5016`) become the tier display.

## 4. Failure modes

1. **An update with a new T2 capability is never approved, then the old version is blocked.**
   The extension is disabled, not upgraded to the held version; the page offers "Approve update"
   or "Uninstall".
2. **The only installed version is blocked and there is nothing to roll back to.** The extension
   is disabled and stays installed; the notification names the reason from the block list; no
   other version installs without the 24 h age gate unless `urgent`.
3. **Hot reload while a modal is open, ticks are scheduled and a fetch is in flight.** The modal
   closes with `dismissed(reload)`, ticks and the request are cancelled, child processes are
   killed, and the new instance mounts; no timer or process from the old instance survives.
4. **A repository plants `git.exe` in the worktree and an extension declares `git`.** The host
   resolves `git` from `PATH` outside the worktree; the planted binary is never run.
5. **Registry and block list unreachable for a week** (offline laptop). Installed extensions keep
   running with the cached block list; after **7 days** the extensions page shows "Block list is
   7 days old"; nothing is disabled for being offline.
6. **A component of 20 MiB starts slowly.** Instantiation runs off the main thread, the
   panel shows a loading state, and the first `mount` has the 2 s budget, not 200 ms.

## 5. Out of scope

- New first-party extensions; the VS Code adapter (Phase 5 #12 keeps its place, it is not part of
  this milestone); webviews (`ui:web`); key capture and overlay labels; inlay hints; graph
  columns; ratings, reviews and download counts in the registry; paid extensions; settings sync of
  extensions; a registry for Zed-format extensions (languages, themes); agent review policy
  (W5's Phase 5 #1 stays separate; this milestone only adds `agent:observe` for extensions);
  TypeScript and other guest languages (decision 3: deferred until after 1.0, then re-evaluated).
