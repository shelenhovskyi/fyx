# UX review: agent review, UI extensions, fyx-ext (ux-1)

Base: `origin/fyx/main` at `571c682` (same tree as `integration`).

**Method.**
- I read the UI code, strings, actions, interceptors and docs of `crates/agent_review`,
  `crates/ui_ext_host`, `crates/ui_ext_wasm`, the `fyx:` hooks in `agent_ui`, `extensions_ui`,
  `settings_ui`, and `docs/fyx/*.md`.
- I compared every review key with `assets/keymaps/default-{linux,macos,windows}.json`.
- I built `fyx-ext` and ran its whole first-run path in the cloud box: `new`, `check`, `build`,
  `dev` (with a stub `fyx`), `package`, broken manifests and a compile error. The log is in
  [Appendix A](#appendix-a-fyx-ext-run-log).
- I did not run the GUI. I also did not run GPUI tests: building `agent_review` means building
  most of Zed, which the 4-core box could not do in this session. Where a finding depends on
  runtime behaviour, it says which code path it follows.
- No product code was changed.

**Missing from `fyx/main`.** Two things in the brief are not on `fyx/main` or any pushed branch
(I checked `claude/fyx-w9-*`, `claude/fyx-registry-core` and `integration`):
- The **consent notification (Enable / Not now)**. UI extensions load without any consent. The
  only prompt is the confirmation of the `fyx_ui: install dev UI extension` action
  (`crates/ui_ext_host/src/install.rs:126-138`). Extensions found in the installed directory, or
  in `FYX_UI_DEV_EXTENSIONS`, load with no prompt.
- The **permission card text "decide it in the review pane"**. `agent_ui` has no `fyx:` hook in
  the permission card (finding 4).

I review what exists. Where something is missing, I list the copy and flow it needs.

## Top 5

1. **One stray chord accepts or rejects a whole agent turn, with no undo.** Inside a review,
   `ctrl-shift-y` (Windows: Zed's "toggle all docks") accepts the turn. `ctrl-shift-n` (Linux and
   Windows: "new window"; macOS: "select down") rejects it. Both also cover edits that arrive
   later in the turn (F1).
2. **Review panes steal keyboard focus.** A pane opens and focuses itself while you are typing in
   the same window, including in the agent panel's message box (F2).
3. **Closing a review tab silently throws away every waiting edit of the turn.** This includes
   `ctrl-w`, "Close Others" and "Close All". Nothing asks first (F3).
4. **The permission card and the review pane decide the same edit.** Clicking the card's Allow
   lands the whole edit and silently closes the review, even for `always_ask` files (F4).
5. **A UI extension that fails to load gives the user and the author nothing.** The failure
   goes to the log only, and the install toast still says "Installed". `fyx-ext check` says
   "passes" for a manifest the host then refuses (F5).

## Findings, ranked by user impact

Severity: **H** = loses work or decisions, or a common flow fails silently; **M** = confusing
or slow, with a workaround; **L** = polish.

### F1 (H). Turn-wide decisions sit on chords that Zed already uses, with no confirmation or undo

**Where.**
- `crates/agent_review/src/review_pane.rs:68-85` holds the key table, and `:104-121` the
  keystroke interceptor.
- `crates/agent_review/src/agent_review.rs:497-517` (`decide_turn`) applies the decision at once
  and stores it for the rest of the turn.

**What the user experiences.**
- The interceptor runs before the keymap whenever focus is inside a review pane.
- It takes these chords from Zed:
  - `ctrl-shift-y`: `workspace::ToggleAllDocks` on Windows (`default-windows.json:677`).
  - `ctrl-shift-n`: `workspace::NewWindow` on Linux and Windows (`default-linux.json:667`,
    `default-windows.json:660`), and `editor::SelectDown` on macOS (`default-macos.json:116`).
- A Windows user who hides the docks to see more of the diff accepts every edit of the turn,
  including edits the agent has not proposed yet. On Linux, opening a new window rejects
  everything.
- Nothing confirms or undoes it. The header shows "· accepting the rest of the turn", but no
  control turns that off.
- The chords also clash with what users have learned from Zed's own agent diff, which is the same
  task:

  | Task | Zed agent diff (Linux/Win · macOS) | Fyx review pane (all platforms) |
  |---|---|---|
  | Accept a hunk | `alt-y` / `ctrl-alt-y` · `cmd-y` | `alt-y` |
  | Reject a hunk | `ctrl-alt-z` · `cmd-alt-z` | `alt-n` (`ctrl-alt-z` does nothing) |
  | Everything in view | `shift-alt-y` / `shift-alt-z` = Keep All / Reject All | `shift-alt-y` = accept **one file**; `shift-alt-z` does nothing |

  Muscle memory for `shift-alt-y` ("keep all") accepts one file here. The second hunk-level
  key, `ctrl-alt-z` reject, is ignored.
- The keys are not in any keymap file, so the keymap editor shows no conflict, and users cannot
  rebind them (see F9).

**Fix.**
1. Move the bindings into `assets/keymaps/default-*.json` under a `"context": "AgentReview"`
   block. Using Zed's agent-diff keys makes the review pane behave like Zed's agent diff:

   | Action | Linux / Windows | macOS |
   |---|---|---|
   | `agent_review::AcceptHunk` | `alt-y`, `ctrl-alt-y` | `cmd-y`, `cmd-alt-y` |
   | `agent_review::RejectHunk` | `alt-n`, `ctrl-alt-z` | `cmd-alt-z`, `alt-n` |
   | `agent_review::AcceptAllHunks` (file) | `shift-alt-y` | `shift-alt-y` |
   | `agent_review::RejectAllHunks` (file) | `shift-alt-z` | `shift-alt-z` |
   | `agent_review::AcceptTurn` | `ctrl-alt-shift-y` | `cmd-alt-shift-y` |
   | `agent_review::RejectTurn` | `ctrl-alt-shift-z` | `cmd-alt-shift-z` |
   | `agent_review::SubmitReview` | `ctrl-enter` | `cmd-enter` |
   | `agent_review::NextHunk` / `PreviousHunk` | `alt-j` / `alt-k` | `alt-j` / `alt-k` |
   | `agent_review::CommentHunk` | `alt-c` | `alt-c` |

   None of `ctrl-alt-shift-y`, `ctrl-alt-shift-z`, `cmd-alt-shift-y` or `cmd-alt-shift-z` is
   bound in any default keymap. I checked every modifier order.
2. Ask before a turn-wide decision while more than one file is waiting. Use a
   `window.prompt(PromptLevel::Warning, …)` with:
   - Title: "Accept all 4 waiting edits in this turn?"
   - Detail: "Edits the agent makes later in this turn will be accepted too. Files matched by an
     `always_ask` rule still wait for you."
   - Buttons: "Accept All" / "Cancel". For reject: "Reject All" / "Cancel", with the detail
     "The agent is told the files are unchanged."
3. Make the sticky part undoable. Next to "· accepting the rest of the turn", add a button
   "Stop accepting" (action `agent_review::ClearTurnDecision`). Clicking it removes the
   `turn_decisions` entry, so later edits are held again.

### F2 (H). A new review takes keyboard focus away from where the user is typing

**Where.**
- `review_pane.rs:187-200`: `open_reviews` passes `focus = window.is_window_active()` to
  `activate_item` and `add_item_to_active_pane`.
- `review_pane.rs:453-459`: `show_review` focuses the diff editor whenever the window is active.
- `review_pane.rs:144`: every `AgentReviewEvent::Opened` runs `open_reviews`.

**What the user experiences.**
- The agent proposes an edit while the user is typing in the agent panel's message box, in
  another editor, or in the terminal. The review tab opens and the keyboard moves into it,
  mid-word.
- The keys typed next go to a read-only diff. If one of them is a review chord (`alt-y`,
  `alt-n`, or F1's `ctrl-shift-y`/`n`), it decides hunks the user has not read.
- The same thing happens again for every following file of the turn, because `show_next` calls
  `show_review` after each submit.
- The code protects *background windows* only.

**Fix.**
1. Move focus only if focus is already inside a `ReviewPane`, or if nothing in the window has
   focus. Otherwise, open or activate the tab without focus (`focus_item = false`), and announce
   it in the status bar (F12).
2. Show a toast once per turn, not once per file:
   - Text: "The agent is waiting for your review of `main.rs`."
   - Button: "Review" (`agent_review::OpenReviews`).
3. Give `OpenReviews` a default workspace binding so the keyboard path is one chord: `ctrl-k y`
   (Linux/Windows) / `cmd-k y` (macOS). Neither is bound in any default keymap today. The
   obvious `ctrl-k r` / `cmd-k r` is taken by `editor::RevealInFileManager`, and
   `ctrl-alt-shift-r` is taken by `search::ToggleRegex` (Linux) and `task::Spawn` (macOS).
   Mention the binding in the toast.

### F3 (H). Closing the review tab rejects the whole turn without asking

**Where.**
- `review_pane.rs:272-277` (`on_release` → `pane_closed`).
- `agent_review.rs:467-486`: the last pane of a turn closes every open review of it.
- `ReviewPane` implements neither `is_dirty` nor `can_save`, so Zed's close flow never prompts.

**What the user experiences.**
- `ctrl-w` / `cmd-w`, a middle click, "Close Others", "Close All" and "Close Inactive Tabs" all
  cancel every waiting edit in the turn. The agent is told the files are unchanged.
- No prompt and no toast appear. The status count simply drops. Users who tidy tabs with
  "Close All" lose the agent's work.
- `docs/fyx/agent-review.md:52-53` states this behaviour in one sentence, but nothing in the UI
  does.

**Fix.**
1. Make `ReviewPane::is_dirty` return true while `open_reviews_in_turn(turn)` is non-empty, and
   make `Item::can_save` false. Zed then asks before closing. If the generic "save?" prompt
   cannot be avoided, override the close path with a custom prompt:
   - Title: "Close the review? 3 edits are still waiting."
   - Detail: "Closing rejects them, and the agent is told the files are unchanged."
   - Buttons: "Reject and Close" / "Keep Reviewing" (default).
2. Show the tab's dirty dot while edits wait, so a pane with pending work stands out.

### F4 (H). The agent panel's permission card and the review pane both decide the same edit

**Where.**
- `agent_review.rs:986-998` opens a review for a v1 permission request.
- `agent_review.rs:839-851` (`permission_answered`) drops the review when the request is
  answered elsewhere.
- `crates/agent_ui` has no `fyx:` hook in the permission card. The only hooks are the menu entry
  at `agent_panel.rs:5773` and init at `agent_ui.rs:687`.

**What the user experiences.**
- The agent panel still shows Zed's own permission buttons ("Allow", "Reject", and for some
  agents "Always Allow"), while a review pane shows the same edit hunk by hunk.
- Clicking Allow in the panel lands the whole edit, even when the file matches an `always_ask`
  rule. The review tab then empties with no message.
- Users cannot tell which control is authoritative. The brief says the card should say
  "decide it in the review pane", but nothing on `fyx/main` does.

**Fix.**
1. While a review holds a permission request, the card should hide its Allow/Reject options
   and show:
   - Label: "Waiting for your review. Decide each change in the review pane."
   - Button: "Open Review" (`agent_review::OpenReviews`, with the binding from F2 in its
     tooltip).
2. If Fyx keeps the card's buttons, answering there must go through the review: Allow = accept
   all hunks and submit, Reject = reject all. Then `always_ask` and the turn history stay
   truthful.

### F5 (H). UI extension load failures are invisible to users and authors

**Where.**
- `crates/ui_ext_host/src/registry.rs:338-341`: an installed extension that fails to load, and
  `:319` an id conflict, both go to `log::error!` only.
- `:411-417`: a failed hot reload goes to the log only.
- `:779-783`: a `FYX_UI_DEV_EXTENSIONS` entry that fails to load goes to the log only.
- `install.rs:139-156`: the install toast says "Installed the UI extension {id}" before the
  asynchronous load result exists.
- `ui_ext_host.rs:67-81`: the extensions page shows capability chips only for *loaded*
  extensions, so a broken one looks like an ordinary Zed extension with no features.
- `crates/fyx_ext/src/inspect.rs:204-208`: `fyx-ext check` cannot see which surfaces a component
  declares. It passes a manifest with `capabilities = []` that the host then refuses (see
  Appendix A, "capability removed").

**What the user experiences.** An author follows `fyx-ext new` → `fyx-ext dev`, removes or
mistypes nothing visible, and Fyx starts with an empty dock: no panel button, no error, no toast.
The only trace is in `Fyx.log`. The same happens after an update that breaks the API version,
after an id clash with another extension, and after a rebuild with a broken surface list
(the old instance keeps running, so the author tests stale code without knowing).

**Fix.**
1. Report load failures in every workspace with a toast. Use one `NotificationId` per
   extension id, so a reload loop shows one toast, not many:
   - Text: "The UI extension `{id}` could not load: {reason}"
   - Button: "Open Log" (`zed::OpenLog`)
2. Show a "Failed to load" chip on the extension's card on the extensions page, with the
   reason as its tooltip.
3. Show the install toast only after the scan result arrives. Use "Installed `{id}`: it adds
   {n} panel(s)." or the failure text above.
4. On a failed hot reload, keep the old instance and show:
   - Text: "`{id}` was rebuilt, but the new build did not load: {reason}. The panel still runs
     the previous build."
5. In `fyx-ext check`, instantiate the component in the same sandbox the host uses (the CLI
   already depends on `wasmparser`; `ui_ext_wasm` can be reused), call `surfaces()`, and run
   the same capability check. Until then, change the note so it does not read as an all-clear
   (strings table, S20).

### F6 (H). A crashed extension shows a raw wasm error with no next step, and recovery is mouse-only

**Where.**
- `crates/ui_ext_host/src/surface.rs:248-255` builds "`{id} stopped: {error:#}`".
- `crates/ui_ext_wasm/src/ui_ext_wasm.rs:844` builds "disabled after an earlier failure:
  {reason}".
- `view.rs:457-474`: the failed state and its Retry button. Retry is not in the surface's Tab
  order.
- `view.rs:513-534`: the error footer is truncated with no tooltip, and only a mouse click
  dismisses it.

**What the user experiences.**
- After a trap or a time-out, the panel (and, in compact mode, the status bar) shows red text
  like "counter stopped: fyx:ui extension counter: error while executing at wasm backtrace: 0:
  0x1f2a - …: wasm trap: unreachable". In the status bar this text is not truncated
  (`view.rs:446-462` builds the compact `h_flex` without `truncate`), so it can push other items
  away.
- Nothing says whose fault it is, whether data was lost, or what to do. The only real recovery
  is to restart Fyx (`limits-and-gotchas.md:37`), and the panel does not say so.
- Keyboard users cannot reach Retry, and cannot read or dismiss a truncated footer error.

**Fix.**
1. Show a short, blame-free message in the panel, and put the technical text behind it:
   - Message: "{Title} stopped working and was turned off. Restart Fyx to turn it back on."
   - Secondary label, muted, selectable: the raw error.
   - Button: "Copy Error Details". For dev extensions, add "Reload" (it reloads the component,
     which the hot-reload path already supports).
2. In compact (status bar) mode, show only "{Title} stopped", truncated, with the full message as
   its tooltip.
3. Add the Retry button and the footer's dismiss control to `focus_order`. Give the footer a
   tooltip with the full text, and an "×" `IconButton` with `aria_label("Dismiss error")`.

### F7 (M). Rule decisions are silent, and the docs say they are listed

**Where.**
- `agent_review.rs:868-880`: a `deny` on a write records the decision and returns.
- `:808-826`: a `deny` or `auto_accept` on a permission request does the same.
- Neither emits `AgentReviewEvent::Opened`, so no pane opens. Only `insert` (`:772`) emits.
- `docs/fyx/review-rules.md:29-30`: "Each decision is listed in the turn's review pane."

**What the user experiences.** An agent's write to `.env` is refused by `deny`. The user sees
nothing in Fyx. The only trace is the agent saying it was refused, possibly much later.
`auto_accept` edits are invisible too. The decision appears in a turn pane only if one is
already open for that turn.

**Fix.**
1. Show `deny` decisions with a toast, one per turn and per rule, so a burst of writes does not
   spam:
   - Text: "Fyx blocked the agent's edit to `.env` (rule `deny = "*.env"`)."
   - Button: "Show Review Rules" (opens `.fyx/review.toml`).
2. Open the turn pane in the background for rule-decided edits, so the history the docs describe
   exists.
3. Until then, correct `review-rules.md:29-30` to: "Decisions made by a rule are listed in the
   turn's review pane while it is open."

### F8 (M). Rule notifications blame the agent, show raw parser output, and cannot be acted on later

**Where.** `agent_review.rs:1380-1393` (rules file invalid), `:1407-1426` (rules file changed),
and `review_rules.rs:148-162`: `auto_accept` is dropped in untrusted worktrees, and nothing says
so.

**What the user experiences.**
- The user edits `.fyx/review.toml` themselves while an agent thread is running and gets:
  "Agent review: /home/u/p/.fyx/review.toml changed. Review rules may have been changed by an
  agent, so the new rules apply only when you apply them." This suggests the agent tampered with
  it.
- If the toast is dismissed, no command applies the pending rules. The old rules then stay
  until the next thread starts, which is not obvious.
- A parse error shows Rust/TOML output: "Agent review: … is ignored: invalid review rules: TOML
  parse error at line 3, column 1 | 3 | auto_acept = … unknown field `auto_acept`, expected one
  of `auto_accept`, `always_ask`, `deny`". This is usable, but long, and starts with an
  absolute path.
- In a worktree the user has not trusted, `auto_accept` silently does nothing.

**Fix.**
1. Use the new strings S9–S12 (strings table).
2. Add the action `agent_review: apply review rules` for the pending rules, and name it in the
   notification so it can be found after dismissal.
3. When a rules file with `auto_accept` loads in an untrusted worktree, say so once per worktree
   (S12).

### F9 (M). Review keys are invisible to Zed's keymap tooling, and tooltips show raw key strings

**Where.**
- `review_pane.rs:87-100` (`key_label`) and `:884-896` (`Tooltip::text(key)`).
- The interceptor itself is at `:104-121`.

**What the user experiences.**
- Hovering "Accept" shows a tooltip containing only `alt-y`: no action name, and on macOS no ⌥
  glyph.
- The keys do not appear in the keymap editor, the command palette's binding column, or "Open
  Default Key Bindings", and they cannot be rebound or disabled.
- `escape` is taken too, so the diff editor never sees Escape (for example, to collapse a
  multi-cursor selection).

**Fix.**
1. Use keymap bindings (F1).
2. Use `Tooltip::for_action_title("Accept hunk", &AcceptHunk)` (and the like), so the tooltip
   shows the action name and the platform's glyphs.
3. Set `aria_keyshortcuts` on each button from the same binding.
4. If an interceptor must stay (the code comment says bindings registered with `bind_keys`
   are lost on keymap reload), keep it only as a fallback for actions that have no binding in the
   loaded keymap.

### F10 (M). Failed actions in the review pane do nothing visible

**Where.**
- `review_pane.rs:666-673`: Submit with undecided hunks fails, logged at `info`.
- `:624-632`: a turn-wide decision that fails is logged at `info`.
- `:557-576` and `:604-614`: hunk updates that fail are logged at `debug`.

**What the user experiences.** `ctrl-enter` with one hunk still undecided does nothing. The
Submit button is disabled with no tooltip saying why. If the header says "Move the cursor into a
hunk", the hunk keys do nothing either.

**Fix.**
1. Show the reason in the file header with `Color::Warning`, for example S5: "2 hunks are still
   undecided. Decide them with alt-y / alt-n, or accept or reject the whole file."
2. Give the disabled Submit button a tooltip with the same text.
3. When no hunk is selected, let the hunk keys move to the first undecided hunk and then act,
   instead of doing nothing.

### F11 (M). Parts of the review pane cannot be reached by keyboard

**Where.**
- `review_pane.rs:713-728`: the turn's file list is reachable by mouse only (`show_review`).
- `:748-768`: the "changed without review" files open by mouse only.
- The comment field is reachable with `alt-c` and left with `escape`; there is no Tab
  navigation.

**What the user experiences.**
- In a turn with several files, keyboard users can only take them in order. They cannot jump
  to a specific file or come back to one.
- They cannot open the files listed as changed without review.

**Fix.**
1. Add `agent_review::NextFile` / `PreviousFile`. They need no default binding beyond the
   command palette. `alt-]` is taken by edit predictions on Linux and Windows, so use
   `ctrl-alt-j` / `ctrl-alt-k`. I did not find those in the default keymaps, but check them
   with the keymap editor before shipping.
2. Add `agent_review::OpenUnreviewedFile`: a picker of the "changed without review" paths.
3. Make the file buttons focusable with `tab_index`, so Tab walks header → files → diff.

### F12 (M). The status bar item counts across windows and does nothing on click when nothing waits

**Where.**
- `review_pane.rs:1018-1035`.
- `agent_review.rs:489-494`: `open_review_count` counts across all projects.
- `:158-182`: `open_reviews` filters by project.

**What the user experiences.**
- Window B shows "Review: 2 waiting" for reviews that belong to window A. Clicking it does
  nothing.
- Clicking "Review on" with nothing waiting does nothing.
- The tooltip is the 60-word `COVERAGE` paragraph.
- The item offers no way to turn review off, so users go looking for the agent panel's
  options menu.

**Fix.**
1. Count only reviews whose files belong to this workspace's project.
2. Use the strings S13–S15.
3. On click with nothing waiting, open a small menu:
   - "Turn Off Review" (`agent_review::ToggleReviewMode`)
   - "Review Rules…" (opens or creates `.fyx/review.toml`)
   - "What Is Reviewed" (opens `docs/fyx/agent-review.md`, or shows `COVERAGE` in a popover)

### F13 (M). `FYX_AGENT_REVIEW` silently overrides the toggle

**Where.**
- `agent_review.rs:95-113`: the menu entry is disabled with no reason.
- The toggle writes a setting that has no effect, and only `log::warn!` says so.

**What the user experiences.** "Review Agent Edits" is greyed out in the agent panel menu, and
`agent_review: toggle review mode` from the palette does nothing. Nothing explains either.

**Fix.**
1. Give the disabled entry a documentation aside or tooltip (S16).
2. When the action runs under the override, show the same text as a toast instead of writing the
   setting.

### F14 (M). The Settings UI has no switch for Fyx review, and shows Zed's similarly named one

**Where.**
- `crates/settings_ui/src/page_data.rs:3055-3057`: the "Agent Review" item is Zed's
  `toolbar.agent_review`, described as "Show agent review buttons in the editor toolbar."
- Fyx's `agent_review.enabled` (`settings_content/src/fyx.rs:10-20`) has no Settings UI entry.

**What the user experiences.** Searching Settings for "review" finds a toggle named
"Agent Review", which controls toolbar buttons in Zed's own agent diff, not Fyx review.

**Fix.**
1. Add a Settings UI item in the AI / Agent section: title "Review Agent Edits", JSON path
   `agent_review.enabled`, description S17.
2. Optionally retitle Zed's item to "Agent Diff Toolbar Buttons". That is an upstream edit, so
   mark it with `fyx:`.

### F15 (M). Extension panels cannot be opened from the command palette

**Where.**
- `crates/ui_ext_host/src/ui_ext_host.rs:52-59`: `ToggleExtensionPanel { key }` has a required
  field, so `build_action_type` fails and the palette omits it (`gpui/src/window.rs:7036`).
- `docs/fyx/extension-authoring.md:266-268` tells authors to write a binding by hand.

**What the user experiences.** A keyboard-only user has no way to open an extension panel. The
dock buttons are mouse targets, and the palette has no entry.

**Fix.**
1. Add the action `fyx_ui: open extension panel`. It takes no arguments and opens a picker of
   loaded panels by title: "Counter (fyx-demo-counter)".
2. Keep `ToggleExtensionPanel { key }` for bindings.
3. Docs: "Open any extension panel from the command palette with **fyx_ui: open extension
   panel**, or bind `fyx_ui::ToggleExtensionPanel` with its key."

### F16 (M). Extension controls have no accessible names, and icon-only buttons have no tooltip

**Where.**
- `crates/ui_ext/src/types.rs:76-81`: `ButtonProps` has no tooltip or label field, and
  `InputProps` (`:85-88`) only has a placeholder.
- `view.rs:705-715`: icon-only buttons render as `IconButton` with no `aria_label` and no
  tooltip.
- The review pane's per-hunk buttons (`review_pane.rs:1084-1122`) are all named "Accept" /
  "Reject", with no hunk context.

**What the user experiences.**
- Screen readers announce extension icon buttons as unnamed buttons. Sighted users get no
  tooltip either.
- In a long diff, a screen reader hears "Accept, Accept, Accept…".

**Fix.**
1. Host, now: give icon-only buttons a fallback name from the icon, `aria_label` +
   `Tooltip::text`. For example, `IconName::Sparkle` → "Sparkle". This is poor, but better than
   nothing.
2. WIT 0.2: add `tooltip: option<string>` to `button-props` and `label: option<string>` to
   `input-props`. `fyx-ext check` should then warn on icon-only buttons without a tooltip, by
   inspecting the tree returned by `mount` in the sandbox (F5 fix 5).
3. Review pane: use `aria_label(format!("Accept hunk {} of {}", index + 1, count))` and the
   matching Reject label. Set `toggle_state` together with `aria_pressed`, if `ButtonLike` exposes
   it; otherwise use `aria_value("accepted")`.

### F17 (M). The install confirmation and the capability chips speak in manifest ids

**Where.**
- `install.rs:28-42` and `:122-136`.
- `registry.rs:182-201` together with `ui_ext_host.rs:67-81`: the chips are the raw strings
  `ui:panel` and `ui:status-bar`. Zed's own chips say "Themes", "Language Servers"
  (`extension_card.rs:429-433`).

**What the user experiences.**
- The dialog reads: "Install the UI extension fyx-demo-counter?" / "It asks for:\nui:panel: add
  dock panels\n\nComponent: fyx_ui.wasm\nFolder: …".
- An extension with no capabilities shows "It asks for:\nnone".
- On the extensions page, cards show `ui:panel` next to Zed's "Themes".

**Fix.** Use the strings S18–S19. The chips become "Panels" and "Status Bar Items".

### F18 (M). `fyx-ext dev` stops right away when a Fyx is already running

**Where.**
- `crates/fyx_ext/src/build.rs:157-190`.
- `crates/zed/src/main.rs:356-379`: a release build with an instance already running prints
  `zed is already running` and exits. Note that the string still says "zed".

**What the user experiences.** An author already running Fyx types the README's
`fyx-ext dev --fyx /path/to/fyx`. The spawned Fyx hands off to the running instance and exits,
so `FYX_UI_DEV_EXTENSIONS` never reaches it. `fyx-ext` prints "Fyx exited (exit status: 0)" and
stops watching. The extension never appears. Reproduced with a stub `fyx` that exits at once:
`fyx-ext` stops watching and returns 0.

**Fix.**
1. If the child exits within 5 seconds with status 0, print S24 and keep watching (`--no-launch`
   behaviour) instead of returning.
2. Rebrand `main.rs:378` to "Fyx is already running" (`// fyx: product name`).
3. Long term: launch a separate instance by default. Pass `--user-data-dir
   <crate>/target/fyx-dev` where the single-instance check honours it (Linux and Windows use the
   data dir; macOS needs checking).

### F19 (M). The authoring guide describes an older host and never mentions `fyx-ext`

See [Docs accuracy](#docs-accuracy). The first-run path a new author reads
(`extension-authoring.md:13-17, 56-109, 230-264`) uses `script/fyx-new-extension`, manual
`cargo build` and `cp`, and an environment variable. It says there is no install UI and no hot
reload, but both exist. `fyx-ext` (`new`/`build`/`dev`/`check`/`package`) is not mentioned in
any page under `docs/fyx/` except the platform spec.

**Fix.**
1. Replace the "Quick start" with the `fyx-ext` path: `fyx-ext new my-panel`, then `cd
   my_panel`, then `fyx-ext dev --fyx <fyx>`.
2. Rewrite the status box and "Installing as a dev extension" to match the code, with the text
   given in the docs table.

### F20 (L). Smaller `fyx-ext` friction

| # | Where | What happens | Fix |
|---|---|---|---|
| a | `.cargo/config.toml:31`; `docs/fyx/extension-authoring.md:223-228` | `cargo run -p fyx_ext -- new demo` in a fresh checkout fails at once with `could not execute process 'sccache …'`. The doc mentions this only under "Building", not for the CLI. The first run also clones every git dependency of the Zed workspace (4.5 minutes here) before it compiles a 2,000-line CLI. | Put `RUSTC_WRAPPER= cargo install --path crates/fyx_ext` at the top of the authoring guide. Later, ship `fyx-ext` as a release asset. |
| b | `main.rs:146-147` | After `new`, "Next: fyx-ext dev --fyx <path to fyx>". `fyx-ext` is not on `PATH` when the author ran it through `cargo run`. | Print S21 (it includes the install hint). |
| c | `main.rs:30-34, 109-113` | `--lang` advertises `ts`, then refuses it with "…(platform plan 2.5)…", which is internal jargon. | Hide `ts` from the help (`#[value(hide = true)]`) until it works. Error text S22. |
| d | `main.rs:40-41`; `template.rs` | The default `--sdk-branch` is `integration`. The repository's main branch is `fyx/main`. | Default to `fyx/main`, or better, `--sdk-rev` of the commit `fyx-ext` was built from. |
| e | `main.rs:56-58, 62-64, 79-81, 87-89` | `--path` has no help text in `dev`, `check` or `package`, and `--release` has none in `build`. | "The extension directory (with extension.toml)" / "Build with the release profile". |
| f | `build.rs:157-160, 228-230` | `fyx-ext dev` without `--fyx` builds first (25 s the first time), *then* fails with "no Fyx binary". | Resolve the Fyx binary before building. |
| g | `inspect.rs:204-208` | Every `build`, `check`, `dev` rebuild and `package` prints `note: "ui:panel" is needed by the surfaces the extension declares; the host checks it when it loads the extension`. This is noise that reads like a problem. | Remove it, or say what it means (S20). |
| h | `manifest.rs:189` | `error: unknown capability "ui:panels"` gives no hint. | S23. |
| i | `template.rs:223, 245, 258, 324` | The README says "Open the right dock and click the sparkle button". The sparkle is the dock icon; the button inside is "Click me" with a plus icon. After one click, the panel says "Clicked 1 times". | README: "Open the right dock (sparkle icon) and click **Click me**." Label: `Clicked {n} time` / `Clicked {n} times` by count. |
| j | `package.rs` | `package` writes `demo-0.1.0.fyxext`. No Fyx command installs a `.fyxext`, and the CLI says nothing about what to do with it. | Print "fyx-ext: packaged …. Fyx cannot install .fyxext files yet; install the folder with **fyx_ui: install dev UI extension**." |
| k | `main.rs:105` (`new` name rule) | `My-Panel` is refused without a suggestion. | Append "Try `my-panel`." (lower-cased, separators normalised) when one exists. |

### F21 (L). Tab wraps inside an extension surface, and Escape is the only way out

**Where.** `crates/ui_ext_host/src/keyboard.rs:46-72` and `:120-135` (wrap-around).

**What the user experiences.**
- Tab cycles through a panel's controls forever. Escape returns to the editor
  (`FocusCenterPane`), as Zed's own panels do. This is acceptable, but it is documented only in a
  code comment.
- The Retry button and the error footer are outside the cycle (F6).

**Fix.**
1. Document it in `extension-authoring.md`, in "Keyboard": "Tab and Shift-Tab move between the
   panel's controls and wrap around. Escape returns focus to the editor."
2. Include the host's own controls in `focus_order`.

### F22 (L). Users cannot hide extension status items

**Where.** `panel.rs:340-343` and `:374-377`: `hide_setting` returns `None` with the comment
"Present only while its extension is loaded, which the user controls."

**What the user experiences.** An unwanted status item can only be removed by uninstalling the
extension. Zed's own items can be hidden.

**Fix.** Return a `HideStatusItem` that writes `fyx_ui.hidden_status_items: ["<ext>/<surface>"]`
and honours it in `attach_to_workspace`.

### F23 (L). Copy and consistency

- `review_pane.rs:298` hard-codes "(alt-c)" in the placeholder. After F1/F9 this should come
  from the keymap, or be dropped: "Comment for the agent on this hunk".
- "Agent turn review" (`:780`): Zed titles are noun phrases in Title Case ("Agent Diff"). Use
  "Review: Agent Turn" to match the tab text "Review: {file}".
- Action namespaces are `agent_review::` and `fyx_ui::`, while Zed's analogues are `agent::` and
  `zed::`. Keep them: they avoid upstream clashes and read well in the palette
  (`agent_review: open reviews`, `fyx_ui: install dev UI extension`).
- Zed calls the same decision "Keep"/"Reject" in its agent diff, and Fyx uses "Accept"/"Reject".
  Keep "Accept". It is clearer because nothing has landed yet. Mention the difference once in
  `agent-review.md` (docs table).
- `view.rs:824` "Lists inside list rows are not supported" is author-facing text shown to end
  users. Prefix it with the extension's title: "{Title}: this part of the panel cannot be shown."
  Log the detail.

## Copy-ready strings

| # | Where | Current | Proposed |
|---|---|---|---|
| S1 | `review_pane.rs:832` | `Could not apply: {error}. Change the decisions and submit again, or reject the file.` | `Fyx could not apply your accepted changes: {reason}. Reject the file and ask the agent to make the edit again.` (Changing the decisions does not help when the file moved, which is the common cause, `agent_review.rs:1519-1524`.) |
| S2 | `agent_review.rs:1520` | `the file no longer contains the text the agent proposed to change` | `the file changed after the agent proposed this edit` |
| S3 | `proposed_edit.rs:360, 371` | `hunks overlap or are out of order` / `hunk past the end of the text` | `the proposed edit is inconsistent (this is a Fyx bug; please report it)` |
| S4 | `review_pane.rs:821` | `Move the cursor into a hunk (alt-j / alt-k)` | `Select a change with alt-j / alt-k` (key names from the keymap, F9) |
| S5 | new, `review_pane.rs:666-673` | (nothing; logged) | `{n} changes are still undecided. Accept or reject them first.` |
| S6 | `review_pane.rs:949` | `No edits are waiting. New edits of this turn appear here.` | `Nothing to review right now. New edits from this agent turn will appear here.` |
| S7 | `review_pane.rs:908-913` | `undecided hunks` / `partly accepted` / `no changes` | `waiting for you` / `partly accepted` / `no changes` |
| S8 | `agent_review.rs:1274-1276` | `Agent review: Fyx cannot review this edit hunk by hunk ({reason}). Decide it in the agent panel; it is not reviewed.` | `This agent edit can't be shown as a diff ({reason}), so it is not reviewed. Allow or reject it in the agent panel.` Button: `Open Agent Panel` |
| S9 | `agent_review.rs:1384-1385` | `Agent review: {path} is ignored: {error:#}` | `Review rules in {worktree}/.fyx/review.toml were not applied: {first line of error}. Fix the file and save it.` Button: `Open review.toml` |
| S10 | `agent_review.rs:1411-1413` | `Agent review: {path} changed. Review rules may have been changed by an agent, so the new rules apply only when you apply them.` | `.fyx/review.toml in {worktree} changed. The previous rules stay in force until you apply the new ones.` |
| S11 | `agent_review.rs:1424` (button) | `Apply new review rules` | `Apply New Rules` (and add the command `agent_review: apply review rules`) |
| S12 | new (untrusted `auto_accept`) | (nothing) | `auto_accept rules in {worktree}/.fyx/review.toml are off because this folder is not trusted. deny and always_ask still apply.` Button: `Trust Folder…` |
| S13 | `review_pane.rs:1024` | `Review on` | `Review On` (Title Case like other status buttons) |
| S14 | `review_pane.rs:1025-1026` | `Review: 1 waiting` / `Review: {n} waiting` | `1 edit to review` / `{n} edits to review` |
| S15 | `review_pane.rs:1031` (tooltip) | the 60-word `COVERAGE` text | When waiting: `Open the waiting reviews`. When idle: `Agent edits wait for your review. Click for options.` |
| S16 | `agent_review.rs:106-113` (disabled entry) | (no reason) | `Set by FYX_AGENT_REVIEW={value} when Fyx started. Unset it and restart Fyx to use this setting.` |
| S17 | Settings UI item | (none) | Title `Review Agent Edits`. Description: `Hold edits from external agents until you accept or reject each change. Shell commands and Fyx's built-in agent are not reviewed.` |
| S18 | `install.rs:132, 124` | `Install the UI extension {id}?` / `It asks for:\n{caps}\n\nComponent: …\nFolder: …` | `Install "{name}"?` / `This extension can:\n• add panels to the dock\n• add items to the status bar\n\nIt cannot read your files or use the network.\n\nFrom: {folder}` (no-capability case: `This extension adds no panels or status bar items.`) |
| S19 | `install.rs:37-39`; chips `registry.rs:182-201` | `ui:panel: add dock panels` / chip `ui:panel`, `ui:status-bar` | `add panels to the dock` / chips `Panels`, `Status Bar Items` |
| S20 | `fyx_ext/src/inspect.rs:207` | `note: "ui:panel" is needed by the surfaces the extension declares; the host checks it when it loads the extension` | (drop it); or `note: fyx-ext cannot see which surfaces the component declares; Fyx refuses it if a panel lacks "ui:panel" or a status item lacks "ui:status-bar"` |
| S21 | `fyx_ext/src/main.rs:144-147` | `Next:\n  cd {dir}\n  fyx-ext dev --fyx <path to fyx>` | `Next:\n  cd {dir}\n  fyx-ext dev --fyx <path to the fyx binary>\n(If fyx-ext is not on PATH: RUSTC_WRAPPER= cargo install --path <fyx checkout>/crates/fyx_ext)` |
| S22 | `fyx_ext/src/main.rs:110-112` | `TypeScript extensions arrive with the TypeScript SDK (platform plan 2.5); use --lang rust` | `TypeScript extensions are not supported yet. Use --lang rust (the default).` |
| S23 | `fyx_ext/src/manifest.rs:189` | `unknown capability "ui:panels"` | `unknown capability "ui:panels"; known capabilities: "ui:panel", "ui:status-bar"` |
| S24 | new, `fyx_ext/src/build.rs:186-189` | `fyx-ext: Fyx exited (exit status: 0)` (and stops) | `fyx-ext: Fyx exited right away, probably because another Fyx is running; that one did not load the extension. Quit it and run fyx-ext dev again, or start Fyx yourself with FYX_UI_DEV_EXTENSIONS={dir}. Still watching for changes.` |
| S25 | `ui_ext_host/src/surface.rs:249-250` | `{id} stopped: {error:#}` | `{Title} stopped working and was turned off. Restart Fyx to turn it back on.` (raw error as secondary text) |
| S26 | `ui_ext_wasm.rs:844` | `disabled after an earlier failure: {reason}` | `turned off after an earlier error` (raw reason in the log) |
| S27 | `install.rs:155` | `Installed the UI extension {id}` | `Installed "{name}". It adds {n} panel(s) and {m} status bar item(s).` (sent after the load result) |
| S28 | `crates/zed/src/main.rs:378` | `zed is already running` | `Fyx is already running` |
| S29 | new (F2 toast) | (nothing) | `The agent is waiting for your review of {file}.` Button: `Review` |
| S30 | new (F1 prompt) | (nothing) | Title `Accept all {n} waiting edits in this turn?`, detail `Edits the agent makes later in this turn will be accepted too. Files matched by an always_ask rule still wait for you.`, buttons `Accept All` / `Cancel` |
| S31 | new (F3 prompt) | (nothing) | Title `Close the review? {n} edits are still waiting.`, detail `Closing rejects them, and the agent is told the files are unchanged.`, buttons `Reject and Close` / `Keep Reviewing` |
| S32 | new (F4 card) | Zed's Allow / Reject | `Waiting for your review. Decide each change in the review pane.` Button: `Open Review` |

## Docs accuracy

| Doc | Says | Code does | New text |
|---|---|---|---|
| `extension-authoring.md:13-17` | "The only way to load a UI extension is an environment variable read at startup. There is no registry, no install UI and no hot reload." | `fyx_ui: install dev UI extension` (`install.rs`); the installed-extensions directory is scanned (`registry.rs:1-17`); dev extensions hot-reload (`registry.rs:15-17`). | "UI extensions load from Fyx's installed-extensions folder (use **fyx_ui: install dev UI extension** to link a folder there) or from `FYX_UI_DEV_EXTENSIONS`. Dev extensions reload when their component file changes. There is no registry yet." |
| `extension-authoring.md:17, 345`; `fyx_ext/src/main.rs:41` | Behaviour is described "on the `integration` branch"; `--sdk-branch` defaults to `integration`. | The main branch is `fyx/main`. | "…what the code on `fyx/main` does…" |
| `extension-authoring.md:56-109` | Quick start uses `script/fyx-new-extension`, manual `cargo build` and `cp`. | `fyx-ext new`/`build`/`dev` do all of it, plus checks. | A `fyx-ext` quick start (F19), with the script kept as an alternative. |
| `extension-authoring.md:230-236` | "UI extensions load **only** from the environment variable… Upstream's 'Install Dev Extension' command does not load `[fyx_ui]` components." | Upstream's dev install links into the installed directory, which the registry scans (`registry.rs:7-11`). | "Install with **fyx_ui: install dev UI extension**, Zed's **Install Dev Extension**, or `FYX_UI_DEV_EXTENSIONS`." (Check upstream's link behaviour once more before publishing.) |
| `extension-authoring.md:264` | "To pick up a new build, restart Fyx. Nothing reloads a running extension." | Hot reload after a 300 ms pause in writes (`registry.rs:56-57`). | "Rebuild (`fyx-ext build`, or let `fyx-ext dev` do it); Fyx reloads the component and remounts its surfaces." |
| `extension-authoring.md:266-268` | Bind `ToggleExtensionPanel { key }` to open a panel by keyboard. | True, and the only keyboard path (F15). | Add the palette path once F15 lands. |
| `extension-authoring.md:333-336` | Returning `Err(String)` "is shown to the user". | Shown as a truncated red footer, mouse-dismiss only (F6). | Fine once F6 lands; add "Keep the message short: it is shown on one line." |
| `review-rules.md:29-30` | "Each decision is listed in the turn's review pane." | Only if a pane is already open for that turn (F7). | See F7 fix 3. |
| `review-rules.md:58-60` | Notification offers **Apply new review rules**; "until you click it, the previous rules stay in force." | True; there is no way to apply after dismissal (F8). | Add: "If you dismiss it, run **agent_review: apply review rules**." (after F8 lands) |
| `review-rules.md:50-53` | `auto_accept` applies only in trusted worktrees. | True; nothing in the UI says so (F8, S12). | Add: "Fyx tells you once when it ignores `auto_accept` for this reason." (after S12 lands) |
| `agent-review.md:36-45` | Key table: `alt-n`, `shift-alt-y`, `ctrl-shift-y/n`, `ctrl-enter` on every platform. | Matches the code; conflicts with Zed on all three OSes (F1). | Replace with F1's table; add one line: "Zed's agent diff says Keep; Fyx says Accept, because nothing has landed until you submit." |
| `agent-review.md:52-53` | Closing the last pane cancels the turn's waiting edits. | True, and silent (F3). | Keep; after F3: "Fyx asks before closing a pane with waiting edits." |
| `agent-review.md:16-17` | "When reviews are waiting, it shows their count; click it to bring the waiting reviews forward." | Count is global, click is per project (F12). | "…the number of edits waiting in this window's project…" (after F12). |
| Brief / `ARCHITECTURE.md` | Interim consent notification (Enable / Not now); permission card "decide it in the review pane". | Neither exists on `fyx/main`. | Either land them or drop them from the brief and docs. For the consent notification, copy: title `"{name}" wants to add panels to your workspace`, body `It cannot read your files or use the network.`, buttons `Enable` / `Not Now`, and **Don't Ask Again for This Extension** in the notification's menu. Show it once per extension per install, not per window or workspace: store the answer in the extension's state, not in the workspace. |

## Notification spam check

| Notification | Scope | Repeats? | Verdict |
|---|---|---|---|
| Rules file invalid / changed (`agent_review.rs:1440-1460`) | Every window whose active workspace has the project. `NotificationId::unique::<T>()` replaces the previous one. | Once per save. Saving a broken file five times shows one toast, refreshed five times. | OK. Add the command from F8 so dismissing loses nothing. |
| Unreviewable edit (`:1272-1279`) | Same. | One per permission request. An agent that edits many multi-file calls replaces the toast each time; the user sees one, refreshed. | OK, but the text does not say which file or call. Include the tool call title: "{tool call title}: …". |
| Host write failure (`:1137-1142`) | Same. | One per failure. | OK. |
| Install result (`install.rs:150-157`) | One workspace. | Per install. | Wrong timing (F5). |
| Load failures (proposed, F5) | Per extension id. | Must be one id per extension, so a hot-reload loop shows one toast. | — |
| Review opened (proposed, F2) | Per turn. | Must be once per turn, not per file. | — |
| Consent (missing) | — | — | When it lands, per extension and not per window (docs table, last row). |

No current notification has a "don't ask again", and none needs one, except the future consent
prompt.

## Appendix A: fyx-ext run log

Built with `RUSTC_WRAPPER= cargo --config 'build.rustc-wrapper=""' build -p fyx_ext`. The first
build took 4 min 37 s, almost all of it cloning the workspace's git dependencies. Run from a
scratch directory:

```text
$ cargo run -q -p fyx_ext -- --help            # in the Fyx checkout, no sccache installed
error: could not execute process `sccache …/rustc -vV` (never executed)

$ fyx-ext new demo
fyx-ext: created ./demo
Next:
  cd ./demo
  fyx-ext dev --fyx <path to fyx>

$ fyx-ext new demo                              # again
error: ./demo already exists and is not empty
$ fyx-ext new My-Panel
error: name must be lower-case letters and digits separated by single '-' or '_', starting with a letter: My-Panel
$ fyx-ext new tsdemo --lang ts
error: TypeScript extensions arrive with the TypeScript SDK (platform plan 2.5); use --lang rust

$ cd demo && fyx-ext check                      # before any build
error: ./fyx_ui.wasm does not exist yet; run `fyx-ext build`
$ fyx-ext check --path /nonexistent
error: reading "/nonexistent/extension.toml": No such file or directory (os error 2)

$ fyx-ext build                                 # 26 s, first build
fyx-ext: built …/target/wasm32-wasip2/debug/demo.wasm -> ./fyx_ui.wasm
note: "ui:panel" is needed by the surfaces the extension declares; the host checks it when it loads the extension
$ cargo test                                    # 1 passed

$ fyx-ext dev                                   # no --fyx, no FYX_BIN: builds first, then
error: no Fyx binary: pass --fyx <path>, set FYX_BIN, or use --no-launch      (exit 1)
$ fyx-ext dev --no-launch --no-watch
fyx-ext: start Fyx with FYX_UI_DEV_EXTENSIONS=/…/demo
$ fyx-ext dev --fyx ./fakefyx -- --foreground   # stub that sleeps 12 s; edited src/demo.rs at 5 s
fyx-ext: starting ../fakefyx with FYX_UI_DEV_EXTENSIONS=/…/demo
fyx-ext: watching /…/demo for changes (Ctrl-C to stop)
fake fyx got FYX_UI_DEV_EXTENSIONS=/…/demo args: --foreground
fyx-ext: built …                                # rebuild on change: works
fyx-ext: Fyx exited (exit status: 0)            # and fyx-ext stops (F18)

$ fyx-ext package
fyx-ext: packaged ./demo-0.1.0.fyxext (46066 bytes)     # nothing in Fyx installs it (F20 j)

# Broken manifests
capabilities = []            → "fyx-ext: demo 0.1.0 passes"   (exit 0; Fyx refuses it at load, log only: F5)
capabilites = [...]          → TOML error with caret, "expected one of api_version, component, capabilities"   (good)
capabilities = ["ui:panels"] → error: unknown capability "ui:panels"   (no hint: F20 h)
api_version = "0.2.0"        → two clear errors (supported version; mismatch with component)   (good)
trailing `oops = `           → TOML error with caret   (good)
syntax error in src          → cargo's diagnostics, then "error: cargo build failed"   (good)
```

What works well: `new` refuses to overwrite, and its name rule is stated in the error. `check`
points at `fyx-ext build` when the component is missing. Manifest typos get caret diagnostics
with the accepted keys. `dev` passes arguments through to Fyx and rebuilds on change. The
generated crate builds and tests as-is.

## Suggested `.rules` additions

None. The one recurring trap (keys taken by `cx.intercept_keystrokes` are invisible to the keymap
editor and to conflict checks) is Fyx-specific and should go in `crates/agent_review/.rules` and
`crates/ui_ext_host/.rules` if the team agrees:

> Do not add review or surface keys with `cx.intercept_keystrokes` alone: put them in
> `assets/keymaps/default-*.json` under the pane's key context, check each chord against all
> three default keymaps, and use `Tooltip::for_action*` so the binding shows.
