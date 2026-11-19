# Coordinator handoff (moving the Fyx swarm into a Claude Project)

Written 2026-11-20 by the local coordinator session for the Claude Project's coordinator. The owner
decided on 2026-11-20 to move the whole swarm into the Claude Project. **All work stays on the
owner's Windows PC.** Threads use **Work locally** (Remote Control), never cloud sandboxes: the
gate, the Windows builds and the owner's manual checks need this PC.

## How to start a local thread (important)

- **Folder:** start every worker thread in its **existing** folder below, **without** the Worktree
  option. `.claude/hooks/session-env.sh` maps `CARGO_TARGET_DIR` by worktree folder name
  (`.claude/fyx-targets.json`). A new worktree gets no target dir and starts a cold ~30 GB Zed
  build in `./target`.
- **One thread per folder:** two threads in one folder overwrite each other.
- **First message:** "You are W<n> in the Fyx swarm. Read only the Handoff at the top of
  `tasks/progress/W<n>.md` and the Efficiency section of `.claude/CLAUDE.md`, then continue."

| Role | Owns | Folder (under D:\projects\fyx) | Branch |
| --- | --- | --- | --- |
| W0 gatekeeper | merges, `script/gate.sh`, pushes `integration`, `fyx/main` | `.` (main checkout, `integration`) | integration |
| W1 | terminal_host, terminal_ext, fyx_relay, SSH, main.rs hooks, settings_content fyx.rs | `.claude/worktrees/fyx-w1-rebrand-904c67` | claude/fyx-w1-rebrand-904c67 |
| W2c | ui_ext model, 0.2–0.4 imports, fyx_guard(_core), fyx_registry, WIT proposals | `.claude/worktrees/fyx-w8-vscode-adapter-6395b4` | claude/fyx-w2c-host-0-2-cbde2c |
| W3 | ui_ext_host (renderer, consent, cards), ui_ext_wasm runtime | `.claude/worktrees/fyx-w3-renderer-e20d2b` | claude/fyx-w3-renderer-e20d2b |
| W4 | ui_ext_sdk, demos, fyx_ext_harness, extensions/fyx_* | `.claude/worktrees/fyx-w4-sdk-demos-5d1497` | claude/fyx-w4-sdk-demos-5d1497 |
| W5 | agent_review | `.claude/worktrees/fyx-w5-agent-review-45eb11` | claude/fyx-w5-agent-review-45eb11 |
| W7 judge (Sonnet) | reviews in tasks/reviews | `.claude/worktrees/fyx-w7-judge-cd66a5` | claude/fyx-w7-judge-cd66a5 |
| W10 docs (Sonnet) | docs/fyx, user-check sheets, fyx-extensions repo | `.claude/worktrees/fyx-w10-docs-a5772b` | claude/fyx-w10-docs-a5772b |

## Protocol

- Each worker iterates with `cargo check` + filtered nextest, then runs whole-crate tests +
  clippy + fmt (+ `script/fyx-api-docs` when public API changes). It pushes its branch and reports
  `ready <sha> <one line>`.
- **W0 merges** `ready` branches into `integration` (workers never merge each other's branches;
  harness matches stay explicit, no `_ =>`). Then the gate:
  - **FAST** for every batch: about 8–11 min.
  - **FULL** only before a `fyx/main` push (at most 2 a day), or for a Cargo.lock version bump of
    an existing crate, or upstream hooks in editor/workspace/project/gpui: about 20 min.
  - W0 pushes `integration` when green and `fyx/main` after a green FULL.
- At ~200k context a worker writes a Handoff (≤3 KB, top of its progress file; history to
  `tasks/progress/archive/`), pushes, and is restarted fresh.
- Crate ownership: report breaks to the owner, don't patch others' crates. `Cargo.toml`,
  `Cargo.lock` and `crates/ui_ext/wit/` are W0-only.

## Rules only the owner can change

- **Pushing and PRs:** never push `main` or tags, never force-push (merge, don't rebase pushed
  branches), and open no PRs to zed-industries/zed.
- **Settings and config:** system and security settings belong to the owner, and so do edits to
  `.claude/CLAUDE.md` and `.claude/settings.json`.
- **Accounts and credentials:** never touch the owner's real `~/.claude` (use a throwaway
  `CLAUDE_CONFIG_DIR`). The owner signs in themselves, and agents never see passwords or tokens.
  No real SSH hosts of the owner in tests.
- **Dev builds:** a dev `fyx.exe` runs only with `--user-data-dir` (never the real profile). Never
  open this repo in a dev build.
- **Talking to the owner:** in Ukrainian, short, with English UI labels. Technical decisions are
  the coordinator's. The owner decides money, system settings, main/tags, force-push, upstream
  PRs, and product direction.

## State at handoff (2026-11-20 morning)

- `fyx/main` = 184b0bdeae (FULL green). `origin/integration` = 2052bd96a8 (relay, both sides).
- **W0:** midday FULL batch with W1 41454ec015 (5.17 SSH tunnel; project-crate hooks), W3
  2d185d05e6 (relay registry side) and 455103b73c (string fixes), W2c f14e0d6296 (gap 8:
  mcp-initialized WIT, staged) and 768aa26cdd (5.20 account-dir protection for extensions).
  W0 still has to delete `E:\sccache` once no cargo is running.
- **W1:** in order:
  1. relay main.rs glue;
  2. focus info for W3's `terminal_focused` (§B.11);
  3. the AccountDirs impl + `set_account_dirs` (recipe in `tasks/progress/notes/W2c-account-dirs-glue.md`);
  4. tab strings: the "<ext> has access" text, `profiles.rs:664`, the "In this project, use <account>" wording;
  5. optional: a throwaway-sshd check.
- **W2c:** the gap-8 probe test once integration has the WIT.
- **W3:** EditHost refuses account dirs; agent-panel "+" entries "Claude Code — Work" with a
  per-thread account (agent_servers/src/fyx.rs; a new account starts a new thread).
- **W4:** 5.15 non-relay part. **W5, W7, W10:** idle.
- **Remaining P5:** 5.19 W7 security review, then freeze WIT 0.4.

## Owner's day-3 manual check (today, after the midday FULL)

1. W0 builds a snapshot from the green sha: fyx.exe, fyx-relay.exe, demos (`script/fyx-build-demos`),
   check-probe.
2. W0 reports the sha, paths and demos list.
3. W10 fills the 6 `ЗАПОВНИТИ` placeholders in `tasks/progress/user-check-day-3.md`
   (profile `D:\fyx-user-check\profile-day3`).
4. W0 merges W10's commit.
5. The owner gets the sheet and the exe path.

Step 7.3 (X on the last window) is to be checked with a real mouse. The earlier "doesn't quit"
came from a synthetic click and is not a bug.

## Migration order

1. W10, then W7, W5 and W4 (idle or near idle).
2. W2c, W3 and W1, each at its next handoff point.
3. W0 last, after the day-3 snapshot is out.

When a worker's thread is running, its old local session stops and gets archived. The old
coordinator stops routing once the Project's coordinator confirms it has taken over.
