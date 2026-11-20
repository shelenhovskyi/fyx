# Coordinator handoff (final, 2026-11-20): the Fyx swarm moved into the Claude Project "fyx"

The local coordinator and all worker sessions are archived. Coordination now lives in the Claude
Project "fyx". **All work stays on the owner's Windows PC**: threads use **Work locally**, never
cloud sandboxes. Each worker's Handoff is at the top of `tasks/progress/W<n>.md` on its branch.

## Starting a local thread

- **Folder:** start each worker thread in its **existing** folder, **without** the Worktree
  option. `.claude/hooks/session-env.sh` sets `CARGO_TARGET_DIR` to the one shared target
  `D:/fyx-targets/shared` for every folder (2026-11-20: one build at a time on this PC).
- **Paths (2026-11-20):** everything Fyx lives under `D:\fyx\`: `fyx` (repo + `.claude\worktrees`),
  `fyx-extensions`, `builds`, `user-check`, `scratch`. `D:\projects\fyx` and the other old paths
  are junctions to them. Target dirs stay at `D:\fyx-targets` (junction to `E:\fyx-targets`).
- **One thread per folder.**
- **First message:** "You are W<n> in the Fyx swarm. Read only the Handoff at the top of
  `tasks/progress/W<n>.md` and the Efficiency section of `.claude/CLAUDE.md`, then continue."
- **Do not delete** `.claude/worktrees/*` until the owner says so.

| Role | Owns | Folder (under D:\fyx\fyx) | Branch |
| --- | --- | --- | --- |
| W0 gatekeeper | merges, `script/gate.sh`, pushes `integration` and `fyx/main` | `.` | integration |
| W1 | terminal_host, terminal_ext, fyx_relay, SSH, main.rs hooks, settings_content fyx.rs | `.claude/worktrees/fyx-w1-rebrand-904c67` | claude/fyx-w1-rebrand-904c67 |
| W2c | ui_ext model, imports, fyx_guard(_core), fyx_registry, WIT proposals | `.claude/worktrees/fyx-w8-vscode-adapter-6395b4` | claude/fyx-w2c-host-0-2-cbde2c |
| W3 | ui_ext_host, ui_ext_wasm runtime | `.claude/worktrees/fyx-w3-renderer-e20d2b` | claude/fyx-w3-renderer-e20d2b |
| W4 | ui_ext_sdk, demos, fyx_ext_harness, extensions/fyx_* | `.claude/worktrees/fyx-w4-sdk-demos-5d1497` | claude/fyx-w4-sdk-demos-5d1497 |
| W10 docs | docs/fyx, user-check sheets, fyx-extensions repo | `.claude/worktrees/fyx-w10-docs-a5772b` | claude/fyx-w10-docs-a5772b |

W5 (agent_review) and W7 (judge) were archived by the owner. Their folders are still there.

## Protocol (unchanged)

- **Worker → ready:** a worker pushes its branch and reports `ready <sha> <line>`. Before that it
  runs whole-crate tests + clippy + fmt (+ `script/fyx-api-docs` on public API changes).
- **Merging:** W0 merges; workers never merge each other's branches. Harness matches stay
  explicit, no `_ =>`. Crate owners fix their own crates.
- **Gate:** FAST for every batch, about 8–15 min. FULL (about 27 min) only:
  - before a `fyx/main` push, at most 2 a day;
  - for a Cargo.lock version bump;
  - for upstream hooks in editor, workspace, project or gpui.
- **Context limit:** at ~200k context a worker writes a Handoff (≤3 KB), pushes, and restarts fresh.

## Merged in this session

- **fyx/main** = 5b6d4e7b77 (midday FULL green, 6317/6317). It contains:
  - the relay, both sides;
  - gap 8 (WIT 0.4 `mcp-initialized`);
  - 5.17 SSH tunnel;
  - 5.20 account-dir protection;
  - W3's string fixes.
- **integration**, after the last FAST (W0 confirms the sha):
  - W2c 9d0e1e6365 (gap-8 probe) + its handoff;
  - W1 206711884b (relay glue, `terminal_focus`, AccountDirs);
  - W4 ece4eeaa3a (`mcp-initialized` in the SDK, fyx_claude_code and fyx_gemini_cli; 5.15 non-relay);
  - W10 2e4b2fe419 (§B.7);
  - the owner's hooks bc646d7fcf.
- **W0 rights:** W0 runs with `FYX_INTEGRATOR=1` in its `settings.local.json`, so the owner's
  guard hook lets it edit integrator files and push `fyx/main`. Nobody pushes `main`.

## Open, in order

1. **W3 c876a77c49** (final ready, pushed, NOT merged): agent-panel "+" account entries and the
   account-dir edit test. Needs a FAST. Not yet checked in a dev build.
2. **Day-3 manual check:**
   - W0 builds a snapshot from the green sha: fyx.exe, fyx-relay.exe, demos via
     `script/fyx-build-demos`, check-probe;
   - W10 fills the 6 `ЗАПОВНИТИ` in `tasks/progress/user-check-day-3.md` (profile
     `D:\fyx-user-check\profile-day3`);
   - the owner gets the sheet.

   Step 7.3 (X on the last window) is checked with a real mouse; the earlier "doesn't quit" was a
   synthetic click.
3. **W3 + W1:** `ui_ext_host::set_terminal_focus` (W3), then W1's main.rs line (`terminal_focused`, §B.11).
4. **W1:**
   - tab strings: the "<ext> has access" text, `profiles.rs:664`, the "In this project, use
     <account>" wording;
   - optional: a sshd check, only against a throwaway local WSL/Docker sshd.
5. **W4:** the relay hooks for fyx_claude_code and fyx_gemini_cli, and `diff_rejected`
   end-turn/status (now unblocked); Gemini gaps 2 and 3.
6. **5.19:** security review of P5, then freeze WIT 0.4. The 1.0 cut follows later.
7. **W0:** delete `E:\sccache` (sccache is off) when no cargo is running.

## Waiting for the owner

- **fyx-extensions:** 92e1e02 is already in its `main` (2b97f3d). The repo is private, its Actions
  workflows are disabled, and turning them on is the owner's call.
- **Optional:** a live Gemini CLI compat recording (needs the owner's Google sign-in).
- **Before going public:** move `.claude/settings.json` and `assets/fyx/icon-concepts` out of the repo.

## Owner-only rules

- **Pushes and PRs:** never push `main` or tags, never force-push, no PRs to zed-industries/zed.
- **Settings and config:** permission and config changes (`.claude/settings*.json`,
  `.claude/CLAUDE.md`) need the owner's yes typed in that session, not relayed by a peer. System
  and security settings are the owner's.
- **Accounts and credentials:** never the real `~/.claude` (use a throwaway `CLAUDE_CONFIG_DIR`).
  The owner signs in themselves, and agents never see credentials. No real SSH hosts in tests.
- **Dev builds:** dev `fyx.exe` only with `--user-data-dir`. Never open this repo in a dev build.
- **Talking to the owner:** Ukrainian, short, English UI labels.
- **Decisions:** technical ones are the coordinator's. Money, main/tags, force-push, upstream PRs
  and product direction are the owner's.
