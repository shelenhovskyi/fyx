# Judge review: `fyx:ui@0.2.0` design draft

**Scope:** `origin/claude/fyx-wit-0.2-design` at `0074863613`:
- `docs/fyx/wit-0.2/design.md`
- `docs/fyx/wit-0.2/fyx-ui-0.2.wit`
- `docs/fyx/wit-0.2/migration.md`

**Checked against:**
- `tasks/wit-0.2.md` rows 1–9 on `fyx/main` (`f3718cae58`).
- W6's decorations proposal, which is row 10 here; it is numbered "9" on
  `origin/claude/fyx-w6-decorations-b1fa30` (`21a69e7f05`, `tasks/wit-0.2.md` §9).
- The architecture rules.

**Method:** documents and the WIT only; nothing else was built.

## Verdict

**Accept the draft as the basis for 0.2, after the two fixes below.** It is careful work:

- **One rule for every import that has an effect.** Each such call is checked when it is made and
  applied after the export returns, in call order (§A). This generalizes the push/reply ordering
  from integration-1 #3 and integration-2 #7.
- **0.1 components get a working compatibility path.** It is Zed's own pattern: a frozen WIT
  directory per version, one bindings module per version, a dispatch enum, and the version read
  from a custom section before instantiation.
- **Every backlog row except row 10 has a decision.** Each decision comes with its semantics, a
  file-level change list and tests.

**Fix before accepting:**
- Row 10, decorations, is missing (finding 1).
- The `replace` cap contradicts the host's (finding 2).

## Checks

### The WIT is valid

`wasm-tools component wit docs/fyx/wit-0.2/fyx-ui-0.2.wit` (1.263.0) on the file at
`0074863613` exits 0 and prints the world (464 lines).

The draft also claims a dummy component validates
(`component embed --dummy` + `new` + `validate`). I did not re-run that.

### 0.1 guests still load (migration.md §1–3, §6)

Yes, by design:
- The 0.1 WIT moves byte-identical to `since_v0.1.0/`, in a commit of its own that must leave
  `gate.sh` green.
- `UiBindings::{V0_2_0, V0_1_0}` dispatch on the version section, with the import fallback kept.
- There is one linker per version.
- Conversions map 0.1 into the superset `ui_ext` model: `IconRef::Upstream`, `indent 0`, no
  tooltip, no secret, no selection.
- 0.1 keeps its quirks: the trap on the 65th push, and a full refetch on any list `set-props`.

The test plan builds the probe for both versions and runs every existing test against the 0.1
build. That also closes integration-1 F2.

One behaviour change is intended:
- **Before:** Fyx's version gating ignores the release channel (W2's decision, `fyx_ui.rs:73-79`).
- **After:** Stable and Preview are capped at 0.1 while 0.2 is staged; see decision 4 below. The
  draft says so explicitly and updates the comment.

### The host keeps the tree, with no WASM calls in `render()`

This holds for every new feature:

| Feature | How it avoids WASM in render |
|---|---|
| Ticks | Executor timers feed the surface's event queue as `QueuedItem::Tick` (§B). |
| Modal dismissal | `on_before_dismiss` always returns `Dismiss(true)` without asking the guest (§C). `dismissed` is queued afterwards. |
| Keyboard selection | Handled in the host: it updates `selected` in its own tree first, then reports `row-selected` (§C). This mirrors the input echo. |
| `spinner` | Animated by the host (§I). |
| A modal opening | Shows an empty frame until `mount` replies (§C). |
| `show-surface`, `add-surface` and the other surface effects | Applied by the registry after the call (§A, §E). |

The flat tree, theme tokens only, and one call per input event are unchanged.

## Backlog coverage

| Row | Item | In the draft | Verdict |
|---|---|---|---|
| 1 | `invalidate-rows` | §G: optional range, `TreeError::NotAList`, `Touched` result, precise 0.2 refetch rule | ok |
| 2 | `request-tick` / `tick` | §B: per surface, one-shot, 100 ms floor, visibility throttling, token bucket, CPU-share clamp, error stop | ok; tick floor is open question 1 |
| 3 | Row-local ids | §H: doc comments on `node-id`, `list-row`, `row-click` | ok |
| 4 | Frozen icons | §I: `enum icon`, 101 cases, an exhaustive `match`, `IconRef` for 0.1 | ok technically; **decision 2** |
| 5 | Modal | §C: `surface-kind::modal`, `show-surface`/`hide-surface`, a gesture rule, keyboard selection | ok; **decision 3** |
| 6 | Runtime surfaces | §E: `add-surface`/`remove-surface`, id rule, cap of 32, `StatusBar::remove_item_by_id` hook | ok |
| 7 | `dismissed` | §D: modals only, `dismiss-reason` | ok |
| 8 | `indent`, `tooltip` | §J: `list-row.indent: u8` (clamped at 32), `button-props.tooltip` (max 1024 bytes) | ok; also fixes W8-8 |
| 9 | `push-update -> result` | §F: `result<_, host-error>`; on `err` nothing is queued; the 65th push returns `limit-exceeded` instead of trapping | ok, but see finding 2. The draft's source list (`design.md:9`) cites only rows 1–8. |
| 10 | Decorations (W6) | **absent.** Only mentioned as "may need their own escape hatch" (`design.md:625`) | **finding 1** |

## Findings, most severe first

### 1. High: decorations (row 10) have no decision (design author, W0 decides scope)

W6 proposes `host.place-decoration(id, location, nodes)` and `remove-decoration(id)`, where a
decoration id acts as a surface id for `push-update`, `handle-event` and `list-rows`, under the
`ui:editor-decorations` capability (`tasks/wit-0.2.md` §9 on W6's branch). The draft does not
evaluate it. Three of its own rules collide with it:

- **§E's surface cap.** At most 32 live surfaces per extension, and every id must be live. A
  linter or blame extension places one decoration per diagnostic or per line, often hundreds. If
  decoration ids are surface ids, the cap breaks them. If they are not, then `push-update` and
  `handle-event` need a second id namespace.
- **§A's effect rule.** `place-decoration` has an effect, so it belongs in the effect list, in
  order, with `host-error`. W6 proposes `result<_, string>`, which does not fit §A.
- **Location.** W6 anchors by path and line at call time. The draft never discusses anchoring or
  path checks, for example a path outside the worktree.

**Fix.** Add a §L that does one of two things:

- **(a) Bring decorations into 0.2:**
  - as an interface `fyx:ui/decorations`, linked only for `ui:editor-decorations`;
  - with `place-decoration` returning `result<_, host-error>` and recorded as a `HostEffect`;
  - with decoration ids in the surface namespace but under their own cap, for example 512, not
    counted against the 32 surfaces;
  - with `remove-decoration` sending no event, like `remove-surface`.
- **(b) Defer decorations to 0.3** explicitly, and keep W6's dev-file data source until then.

This is decision 1 below.

### 2. Medium: the `replace` cap in §F contradicts the host, which causes remount loops (W2, design)

§F's import-time check accepts "a `replace` with at most 16384 nodes"
(`design.md` §F; test plan "a `replace` with 16385 nodes returns `limit-exceeded`"). Today the
host's `Tree::replace` and `mount` reject anything over `max_nodes_per_update` = 2048
(`crates/ui_ext/src/tree.rs:168` with `:210`). The SDK caps `mount` at 2048 as well. That is
integration-1 #11, still open.

Result: a 3000-node push passes the check, is rejected when applied, and remounts the surface.
The remount returns the same tree and is rejected again. After 3 tries the surface fails
(`MAX_REMOUNTS_WITHOUT_INPUT`).

**Fix:** pick one cap for `replace` and `mount` in 0.2, and use it in the WIT comment, the §F
check, `tree.rs` and the SDK.

- Recommended: 16384, the tree cap. A whole surface can then always be resent, which is exactly
  what §F's "resend whole" recovery needs.
- Alternatively, check 2048 in §F and say so in the WIT comment (`fyx-ui-0.2.wit:265-267`).

### 3. Medium: mutually exclusive SDK features break when an extension depends on two crates (W4)

migration.md §4 picks the world with Cargo features: `fyx-ui-0-1` or `fyx-ui-0-2`, enforced by
`compile_error!` when both or neither are set. Cargo unifies features. So an extension that
depends on a helper crate using the SDK's defaults, while itself selecting `fyx-ui-0-1`, fails to
compile with no way out. The draft acknowledges the trap for the workspace only (a `gate.sh`
check).

**Fix:** generate both bindings modules (`bindings::v0_1`, `bindings::v0_2`) in every build and
pick the world in the macro, for example `register_extension!(T)` for 0.2 and
`register_extension!(T, v0_1)`. Verify this before adopting it. Each `generate!` embeds its own
component-type section, so check with `wasm-tools component wit` that a 0.1 build imports only
`@0.1.0` interfaces. If features stay instead, document the trap in `sdk-reference.md`.

### 4. Low: the gesture window lets typing open a focus-stealing modal (design, §C)

Transient activation is granted for `input-changed` and lasts 5 s (§C "User gesture"). Because
typing is debounced, every keystroke pause in an extension's own input opens a fresh window. A
`show-surface` from a tick within that window steals focus mid-typing.

**Fix:** grant activation only for `click`, `row-click`, `input-submitted` and `row-selected`,
not for `input-changed`. This is part of open question 2 / decision 3.

### 5. Low: smaller points (design author)

- **Status-bar hook.** `StatusBar::remove_item_by_id` (§E) adds about 8 upstream lines. Accept
  it: the "render empty" alternative leaks a view per removal. Add it to the
  integration-3 upstream table when it lands.
- **Surface id rule.** 0.2 rejects static surfaces whose ids break `[a-z0-9._-]{1,64}`. Say in
  migration.md §4 that porting a 0.1 extension to 0.2 can require renaming its surface ids. The
  ids are part of panel keys and persisted state.
- **Hidden-panel ticks.** These are deferred until the panel is visible, then delivered at once.
  A panel that is opened after hours gets one tick, not a backlog. Good. State it in the WIT doc
  comment of `request-tick`, not only in design.md.
- **No `HostEffect` for `log`.** It has no effect, so it is correctly excluded. Mention that in
  §A, so nobody routes it through the effect list later.

## Decisions for the user (owner)

Most important first:

1. **Scope of 0.2: are decorations in or out?**
   - In: as a separate capability-gated interface following §A, with its own cap.
   - Out: deferred to 0.3, and 0.2 ships with ticks, modals, runtime surfaces and the rest.

   This decides whether W6's layer-4 work becomes an extension API now or stays host-only (finding 1).
2. **Frozen icon enum.**
   - Accept the 101-icon enum: extensions never break on upstream renames, but every new icon
     needs a new world version.
   - Or keep strings with a documented list: icons can grow without a world version, but upstream
     renames can still break extensions.
3. **Modal focus policy.**
   - May an extension open a focus-stealing modal up to 5 s after a user gesture, or only inside
     the event call? Together with finding 4: should typing count as a gesture?
   - Ship keyboard selection with 0.2 modals, or leave it for 0.3? Without it, pickers work by
     mouse and Enter only.

Integrator-level choices, recommended answers:
- Tick floor: keep 100 ms.
- Status-bar hook: accept.
- `replace` cap: 16384 (finding 2).
- Staging: cap Stable/Preview at 0.1 until 0.2 is frozen. This reverses W2's no-channel-cap rule,
  but only temporarily, and it follows Zed.
