# Agent cockpit: many agents in one IDE (research, 2026-11-20)

Research only, no code. Question from the owner: a Fyx feature like Claude Projects, where one
coordinator agent starts other agents and those start subagents. Direction already agreed with the
coordinator: Fyx does **not** build its own orchestration brain (Claude Code already has subagents
and agent teams). Fyx builds the IDE **cockpit** around agents that already exist: isolation,
visibility, review with a serialized merge queue, and limits.

This builds on [worktrees and parallel agents](../../research/worktrees-and-parallel-agents.md)
(§8, the "narrow layer" verdict) and on plan rows 5.4, 7.8 and 7.9 in
[plan.md](../platform/plan.md). It does not replace them.

## TL;DR

- **Demand is real but niche.** Every major tool shipped parallel agents in 2026, and a small group
  of power users runs 4 to 20 agents a day. Most developers peak at one or two agents at a time.
  The cockpit is worth building only as a thin layer on 7.9, aimed at that power-user niche, which
  includes the owner.
- **Isolation and thread lists are solved.** Review throughput, merge sequencing, shared resources
  (disk, build caches, ports), "who waits on whom" across a tree of agents, and budgets per tree
  are not solved by anyone. Windows and Rust are underserved: Conductor is macOS-only, and the
  Codex app launched macOS-only.
- **The UX to copy is Claude Projects:** one chat with a coordinator, threads as cards,
  notifications only when you are needed, decisions as buttons, results batched.
- **MVP:** an agent tree board, a "needs you" inbox, a cockpit MCP server the coordinator calls,
  a serialized merge queue over `script/gate.sh`, and host-owned locks plus budgets. All of it
  lives in Fyx crates. The upstream footprint stays at the hooks plan 5.3, 5.4 and 7.9 already
  need.

## 1. Would people want this?

### 1.1 What already exists (2026)

| Product | Shape | Isolation | Review / merge | Gaps that matter to us |
| --- | --- | --- | --- | --- |
| Zed Parallel Agents (upstream, in our fork) | Threads Sidebar, any agent per thread (Zed agent, ACP, Terminal Threads) | Per-thread git worktree, archive snapshots it | "Review the diff and merge through your normal Git workflow" | No hierarchy, no merge queue, no cost, external CLI sessions are invisible ([docs](https://zed.dev/docs/ai/parallel-agents)) |
| Cursor 3.x Agents Window | Agents first; `/multitask` starts parallel async subagents (3.2) | Worktrees, `.cursor/worktrees.json` setup | Per-agent diff | Forum bugs: agents write to the root repo instead of the worktree ([changelog 3.2](https://www.cursor.com/changelog/04-24-26), [forum](https://forum.cursor.com/tag/worktrees)) |
| Claude Code subagents + agent teams | A lead session spawns full sessions as teammates, with a shared task list and mailbox | Up to the user; `claude --worktree` | None built in; "no built-in rollback", task list tracks completion, not correctness | Experimental, off by default, needs tmux or iTerm2 for panes ([docs](https://code.claude.com/docs/en/agent-teams)) |
| OpenAI Codex app | "Command center" for agents, threads grouped by project | Built-in worktrees under `$CODEX_HOME/worktrees`, not auto-deleted | Diff review and comments in the thread | Launched macOS-only ([OpenAI](https://openai.com/index/introducing-the-codex-app/), [worktrees](https://developers.openai.com/codex/app/worktrees)) |
| Conductor (YC) | Mac app around Claude Code / Codex: chats, terminals, diffs, checks, PRs | Worktree per workspace | Diff + PR + archive | Apple Silicon only, Windows waitlist ([site](https://www.conductor.build/)) |
| Sculptor (Imbue), Crystal, Superset, Paseo | Desktop runners for parallel Claude Code | Containers or worktrees | Diffs | Imbue reports 4 to 6 parallel agents as typical ([Sculptor](https://imbue.com/sculptor/)) |
| GitHub Agent HQ / Mission Control | Every agent session across vendors, assign, watch, review | Branch-level | PR review, audit logs, admin policies | Cloud and PR-centric, not local builds ([blog](https://github.blog/news-insights/product-news/welcome-home-agents/)) |

Conclusion: **launching, listing and isolating agents is a commodity.** Building it again earns
nothing, and our fork already has upstream's version.

### 1.2 Who the users are

- **Mass market: one or two agents.** Jellyfish telemetry: 84% of Claude Code users peak at 1 to 2
  concurrent agents, and only 6.1% of active minutes have more than one agent running. 69% of
  Stack Overflow respondents work with one agent. (Figures from §8.1 of the worktrees research.)
- **Power users: 4 to 20 agents.** Solo developers and small teams on Claude Code / Codex CLIs,
  often with a subscription that makes tokens feel flat-rate. They write the blog posts, buy
  Conductor, and are the audience here. Our owner (about 8 agents on one Windows PC, a coordinator
  and threads) sits squarely in this group.
- **Teams and enterprises:** they want governance and audit (Agent HQ's pitch), not a local
  cockpit. Out of scope.

### 1.3 What they complain about (consistent across sources)

1. **Review is the bottleneck, not generation.** "The real bottleneck moved from writing code to
   reviewing it" (HN). Faros: review time up sharply when agents write the code (median +441%,
   second-hand). Superset: "most of our agents spend more time waiting for us to review their work
   than they spend doing it" ([Superset](https://superset.sh/blog/roadmap-to-100-agents),
   [Osmani](https://addyosmani.com/blog/agentic-code-review/)).
2. **Merges still collide; they need sequencing.** Worktrees only move conflicts to merge time;
   "if several finish together, the merges need sequencing"; advice is rebase right before each
   merge so conflicts arrive one at a time
   ([dev.to](https://dev.to/battyterm/5-lessons-from-running-ai-coding-agents-in-parallel-53on),
   [MindStudio](https://www.mindstudio.ai/blog/parallel-agentic-development-playbook)).
3. **Coordination bookkeeping grows with agent count.** Beyond 3 to 5 agents, tracking which
   branch is active, done or blocked becomes the job; agents cannot see each other's state.
4. **Runtime isolation is missing.** Worktrees isolate files, not ports, databases, `.env`,
   build caches or disk. Duplicated `node_modules` and build dirs, stale worktrees eating disk
   ([Upsun](https://developer.upsun.com/posts/ai/git-worktrees-for-parallel-ai-coding-agents),
   [dev.to](https://dev.to/koenvg/when-worktrees-share-ram-gba)). For Rust it is worse: every
   `target/` here is 1 to 15 GB and sccache barely hits across worktrees.
5. **Cost.** N agents burn N times the tokens in the same window; an HN thread questions $1000 a
   month for 9 agents. Nobody shows cost per agent tree, let alone caps it.
6. **Attention and trust.** Oversight, not compute, is the limit: each extra agent adds a variable
   supervision cost; several sources recommend fewer threads to keep quality
   ([AgentConn](https://agentconn.com/blog/oversight-budget-agent-fleet-2026)).

Our own pain on this PC matches items 2 to 5 exactly: disk blowups from parallel builds, stale
shared artifacts (now handled by `script/fyx-cargo` and `script/fyx-disk-guard`), a merge queue run
by hand through the integrator thread, unclear who waits on whom, and cost.

### 1.4 Solved vs unsolved

| Need | State in 2026 |
| --- | --- |
| Start N agents, each in a worktree | Solved everywhere (Zed, Cursor, Codex, Conductor, Claude Code) |
| A list of threads with a status dot | Solved (Zed Threads Sidebar, Codex, Cursor) |
| Per-thread diff review | Solved, shallow (no comments back to the agent in most tools) |
| Agents started **outside** the IDE are visible | Unsolved (Zed sees only its own threads and terminals) |
| A **tree** (coordinator, workers, subagents), who waits on whom | Unsolved outside chat products like Claude Projects |
| Notify only when a human is needed, batch the rest | Partly (Zed 5.4-style notifications on done/blocked); no digest, no decision buttons in IDEs |
| Serialized merge queue with a local gate | Unsolved locally (GitHub merge queue is cloud/PR only) |
| Shared resources: build slots, disk, ports | Unsolved; users write scripts (we did: `fyx-cargo`, `fyx-disk-guard`) |
| Budget and depth limits per agent tree, one "stop all" | Unsolved |
| Windows-first, Rust-aware | Underserved (Conductor and the Codex app launched Mac-only) |

### 1.5 Honest verdict

Demand is **niche but intense**: a few percent of agent users, but they are the loudest and the
ones who pay for extra tools. A general "agent manager" would be crowded and mostly duplicate
upstream Zed and Claude desktop. A **cockpit that does the IDE-side jobs no one does** (review,
merge queue, shared build resources, budgets, cross-tool visibility on Windows) is defensible and
directly useful to the owner. Build it as the next step of 7.9, not as a new product line, and
stop at the MVP until dogfooding shows it saves time.

Main risk: Anthropic and OpenAI keep moving their desktop apps towards exactly this shape (Claude
desktop already has projects, threads and decision cards). Fyx must not compete on the chat; it
must be the place where the code, the builds and the merges are, and plug the chat in.

## 2. Making it as friendly as Claude Projects

### 2.1 What Claude Projects gets right

A person writes in one chat. A coordinator splits the work into threads. Each thread is a card
with a title, a checklist and a state. The person hears about a thread only when it is done, blocked
on them, or needs a decision, and a decision is a card with buttons and a recommended option. When
they return, they read a short summary, not a log. Reversible steps happen without asking;
irreversible ones wait for a click.

### 2.2 Top UX principles

1. **One conversation in, cards out.** The person talks only to the coordinator. Workers appear as
   cards, never as chats you must read.
2. **Interrupt only when needed.** Notify on four events: needs a decision, ready for review,
   failed, or hit a limit. Everything else updates cards silently.
3. **Decisions are buttons with a recommended default.** The agent keeps going on the recommended
   option while it waits, unless the step is irreversible.
4. **Batch the results.** When the person comes back: "Since 14:10: 3 merged, 1 needs you, 1
   failed the gate", with links. No scrollback.
5. **Safe by default, limits visible.** Reversible work runs freely; merges to the default branch,
   pushes and deletes need a click. Budget, depth and concurrency limits are on screen, with one
   "Stop all".
6. *(Fyx-specific)* **The host owns shared resources.** Build slots, disk, ports and the merge
   queue are Fyx's job, not something each agent must remember from a CLAUDE.md.

### 2.3 Inside Fyx: panels

All new UI lives in a Fyx crate (working name `fyx_cockpit`, growing out of the 7.9 board), with
data from `agent_review`, the relay (5.12) and ACP. No new upstream panels.

- **Cockpit (left dock, replaces nothing):** the 7.9 board shown as a tree:
  project → coordinator → workers → subagents. Each row: title, state chip
  (working / needs you / review ready / queued / merged / failed / stopped), worktree, diff size,
  cost today, `target/` size. "Needs you" rows sort first. A footer shows the tree's budget bar,
  concurrency (`4/6 agents, 1/2 build slots`) and **Stop all**.
- **Thread card (opens in the center):** the checklist the agent reports, the last message, the
  diff (`agent_review` hunks with comments that go back to the agent), and buttons: Open
  worktree, Open terminal, Pause, Stop, Send to merge queue.
- **Inbox (a badge on the cockpit icon and the title bar):** a queue of decisions and approvals.
  Each item is a card: question, options as buttons, the recommended one first, and "Reply
  instead". Answering sends the choice to the agent that asked.
- **Merge queue (a tab in the cockpit):** an ordered list. For each item, Fyx rebases the branch
  onto the target, runs the project's gate, and fast-forwards on green. Exactly one runs at a time.
  A red gate sends the branch back to its agent with the log, without asking the person.
- **Coordinator chat:** just a normal agent thread (Claude Code over ACP, or a Terminal Thread)
  pinned at the top of the tree. Fyx adds tools to it; it does not replace it.

### 2.4 How the coordinator plugs in (no orchestration brain)

Fyx ships a small **cockpit MCP server** (a mode of the `fyx_relay` binary, or a sibling binary)
that any agent can be given. Its tools map one-to-one to the UX:

| Tool | Effect in Fyx |
| --- | --- |
| `start_worker(title, brief, base_branch)` | Creates a worktree (`fyx/<name>` branch, `.worktreeinclude`, own `CARGO_TARGET_DIR` slot), starts Claude Code there, adds a child row under the caller |
| `report_progress(checklist)` | Updates the card's checklist; silent |
| `request_decision(question, options, recommended)` | Posts an Inbox card and notifies; returns the answer to the caller |
| `ready_for_review()` | Moves the card to "review ready" and notifies |
| `enqueue_merge(target)` | Adds the branch to the merge queue |
| `acquire(resource)` / `release(resource)` | Host locks: `cargo-build`, `disk`, named ports; the same lock `script/fyx-cargo` takes |

Claude Code decides what to delegate and how (subagents, agent teams, or `start_worker`). Fyx
only sees and serves. External sessions that do not use the server still show up read-only from
hooks (relay, OSC 7501) and from `~/.claude/projects/*.jsonl`, as in 7.9.

### 2.5 Flows

- **First run.** Open the cockpit. Fyx scans `git worktree list`, the relay and Claude session
  files and shows what already runs: "6 agents in 4 worktrees, 2 need you". A single prompt offers
  "Run a coordinator here", which picks the repo, detects the gate (`script/gate.sh`, else
  `cargo test` / `npm test`), the merge target (`integration` if it exists, else the default
  branch) and shows the defaults below in one editable card. Nothing else to configure.
- **Daily.** Type a goal in the coordinator chat. Cards appear as workers start. You get a
  notification only for Inbox items and review-ready cards. Review in the card, press "Send to
  merge queue". Walk away; come back to a digest.
- **Something goes wrong.** Budget hit: the tree pauses and one Inbox card asks "Raise to $X /
  Stop". Disk low: `fyx-disk-guard` runs, the cockpit shows what it freed, and new builds wait.
  Gate red: the branch goes back to its worker automatically.

### 2.6 Defaults

| Setting | Default | Why |
| --- | --- | --- |
| Concurrent workers per tree | 4 (max 8) | Practitioner ceiling is 2 to 6 before oversight fails |
| Build slots (shared `target`) | 1 per shared target dir | What `fyx-cargo` does today; avoids disk blowups |
| Depth | coordinator → worker → subagent (2) | Deeper trees are invisible and costly |
| Budget | Soft daily cap per tree; pause with a card, never kill silently | 7.8's cap, scoped to the tree |
| Notifications | Inbox, review ready, failed, limit hit; only while the window is unfocused | 5.4's rule |
| Digest | On window focus after 30+ min away | Batches results |
| Irreversible steps | Merge into the default branch, push, branch delete: always a click | Matches the project's rules |
| Worktree cleanup | After merge, archive with snapshot (upstream behavior), sweep its target slot | Stale worktrees and artifacts were our top disk cost |

### 2.7 Upstream footprint

Everything above is new Fyx code: `fyx_cockpit` (panel, inbox, merge queue), the MCP mode of
`fyx_relay`, `agent_review` for diffs. Upstream hooks stay at what plan rows already require: the
panel registration in `zed.rs`, 5.3's terminal status, 5.4's notification, and the 2 to 3 cold-file
hooks from worktrees research §8.3 (`fyx/<name>` branch, `.worktreeinclude`). Nothing touches
`agent_panel.rs`, `sidebar.rs` or `title_bar.rs` (the hottest upstream files); the Inbox badge goes
on the cockpit's own dock icon. A later `fyx:ui` extension point could let extensions add cards
(for example a CI card), but the MVP does not need it.

### 2.8 UI/UX research and prototype v2 (2026-11-20)

A clickable prototype lives next to this file
([agent-cockpit-prototype.html](agent-cockpit-prototype.html)). The owner's verdict on v1: it looks
good, but parts are cluttered or unclear. This section is the focused UX research and the audit
that produced v2.

**What calm multi-agent tools do**

- **Group by status, with one "Needs attention" bucket.** Cursor's Agents Window, grouped by
  status, gets a "Needs Attention" section holding every agent blocked on you (questions, approvals,
  plans) *plus finished work you have not reviewed*. A parent's status rolls up from its subtasks,
  so a parent is never shown as stuck behind its children
  ([Cursor forum](https://forum.cursor.com/tag/agents-window/378),
  [cloud agents notes](https://cursor.com/docs/release-notes/cloud-agents.md)).
- **Attention, not activity.** A supervision surface shows what needs judgment; an activity feed
  shows everything that happened. Mixing them is the main source of noise
  ([Supervision surfaces](https://blakecrosley.com/blog/agents-need-supervision-surfaces)).
- **Recede what orients, keep the task in focus.** Linear's 2024 and 2026 refreshes cut tabs,
  headers and panels, lowered border contrast, and let navigation recede so the current view and
  its actions read first ([Linear](https://linear.app/now/how-we-redesigned-the-linear-ui),
  [2026 refresh](https://linear.app/changelog/2026-03-12-ui-refresh)).
- **Progressive disclosure.** Summary first, steps, tools and costs behind "details"; risk-tiered
  approval, so only consequential steps ask
  ([Smashing](https://smashingmagazine.com/2026/02/designing-agentic-ai-practical-ux-patterns/),
  [UI/UX Atlas](https://uiux-atlas.pages.dev/lessons/emerging/multi-agent-orchestration-ux)).
- **Simple lists win.** GitHub's agents panel and VS Code's Agent Sessions are a flat list with
  status, a click to open, stop and archive
  ([GitHub](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/manage-agents),
  [VS Code](https://code.visualstudio.com/docs/agents/run/sessions/manage-sessions)). Kanban
  boards break down when agents are many or interdependent.
- **Complaints to avoid:** Cursor users report unclear completion alerts, wrong statuses ("waiting
  for plan approval" shown as complete) and review split across two windows
  ([forum](https://forum.cursor.com/t/cursor-agents-ui-unreliable-task-switching-ambiguous-message-submission-and-unclear-completion-alerts/170506)).

**Audit of v1 and what v2 changed**

| v1 problem | v2 change |
| --- | --- |
| 12 status chips (working, needs you, ready for review, in merge queue, merging, merged, gate red, stopped, paused, idle, done, outside Fyx) | Four buckets the person cares about: **Needs you, Working, Merging, Done**, shown as a dot (color plus shape). The specific reason is one line of text under the name ("Question: …", "Review +88 −12", "Conflict in render.rs") |
| Decisions lived in three places: chat cards, an Inbox popover and toasts; the same agent also appeared twice in the tree | One place to act: the agent's card. The tree's "Needs you" section, the title-bar pill, the chat line and the toast all just **open that card**. The Inbox popover is gone |
| Title bar crowded: budget bar, agent count, build slot, Inbox, Stop all | One pill: **"3 need you →"** (opens the next one) or a quiet "All quiet · 4 working". Budget moved to the tree footer and becomes a "Needs you" item only at 80% |
| Every row showed chip, cost, build size, branch and diff | Row = dot, name, reason only when it needs you, and one right-hand word: Answer / Review / Resolve, or minutes running. Cost, branch, folder, build size sit in the card under **Details and controls** |
| Card had six equal buttons | **One primary action per state**: answer options; *Approve and merge*; *Let the agent fix it*; *Resume*. Pause, Stop, Open folder, Open terminal moved under Details |
| Subagents always expanded; merged and outside agents always visible | Subagents collapse to "2 helper agents · 1 working"; **Done** and **Outside Fyx** are collapsed groups |
| Chat logged every merge and failed gate as system messages | Chat has only the coordinator's words and one line per "needs you" (marked *handled* once done). Merges and auto-handled failures go to the away summary and the terminal |
| Toasts stacked, also for silent events | At most one toast, "+N more", only for "needs you"; a toast disappears when its item is handled |
| Jargon: target, gate red, FAST=1 script/gate.sh, build slot, depth 2, in merge queue | Plain words: build files, "a check failed", "Merging into integration", "Approved, waiting to merge". The gate command is under "How merging works" |
| Merge queue had a tab, a Tree/Queue toggle and a header of five facts | One "Merging" view opened from the footer or a merging card, with a 3-segment progress bar per branch |
| Status text could disagree with reality (v1 kept stale toasts) | Status line is derived from state in one function, the rule Cursor users asked for |

**Rules v2 encodes (for the real implementation)**

1. Only `NeedsYou(reason)` interrupts; `Working`, `Merging`, `Done` never do.
2. Every "needs you" item has exactly one place to act, and every pointer to it opens that place.
3. One primary button per card state; the rest is behind Details.
4. Show the reason, not the state name.
5. Collapse what is finished, nested or outside Fyx; show counts instead.
6. Quiet chrome: low-contrast borders, no colored chips except the yellow "needs you".

## 3. MVP vs later

### MVP (in this order; each step usable alone)

1. **Agent tree board.** 7.9 with parent/child rows, "needs you" first, cost and `target/` size,
   Stop and Stop all. Includes external Claude Code sessions read-only.
2. **Inbox and notifications.** Decision cards with buttons, review-ready and failure events, the
   digest on return.
3. **Cockpit MCP server.** `start_worker`, `report_progress`, `request_decision`,
   `ready_for_review`, `enqueue_merge`; the coordinator is plain Claude Code with this server.
4. **Merge queue.** One at a time: rebase, run the project gate (here `script/gate.sh` with
   `FAST=1`), fast-forward on green, return to the worker on red.
5. **Host resources and limits.** The `fyx-cargo` lock and `fyx-disk-guard` as cockpit services
   (`acquire`/`release`), a per-tree budget and concurrency cap.

Success check (two weeks of dogfooding with the owner's ~8 agents): no disk-full incidents, a
blocked agent is noticed in under 5 minutes, merges no longer go through a human-run integrator
thread, and the owner reads digests instead of threads.

### Later

- Cloud and remote workers (Mac ↔ Windows hand-off, review bundles D13).
- Best-of-n: the same brief to 2 to 3 workers, compare diffs in one card.
- Per-worktree ports and databases (`ZED_WORKTREE_ROOT`-based task hooks).
- Agent-side conflict resolution when the queue's rebase fails.
- Cost forecasting before `start_worker` and a weekly report.
- Extension cards through `fyx:ui` (CI status, issue tracker, PR).
- Multi-person projects (out of scope while Fyx is a solo tool).

### Not doing

- Our own planner, task decomposition or agent-to-agent messaging: Claude Code owns that.
- A full worktree manager or a replacement for the Threads Sidebar (worktrees research §8.3).
- Hooks in `agent_panel.rs`, `sidebar.rs` or `title_bar.rs`.

## Sources

Zed: [Parallel Agents](https://zed.dev/docs/ai/parallel-agents),
[Terminal Threads](https://zed.dev/blog/terminal-threads). Cursor:
[3.2 changelog](https://www.cursor.com/changelog/04-24-26),
[worktree forum threads](https://forum.cursor.com/tag/worktrees). Claude Code:
[agent teams](https://code.claude.com/docs/en/agent-teams),
[Osmani on agent teams](https://addyosmani.com/blog/claude-code-agent-teams/). OpenAI:
[Codex app](https://openai.com/index/introducing-the-codex-app/),
[Codex worktrees](https://developers.openai.com/codex/app/worktrees). Runners:
[Conductor](https://www.conductor.build/), [Sculptor](https://imbue.com/sculptor/),
[Superset](https://superset.sh/blog/roadmap-to-100-agents). GitHub:
[Agent HQ](https://github.blog/news-insights/product-news/welcome-home-agents/),
[Mission Control](https://github.blog/ai-and-ml/github-copilot/how-to-orchestrate-agents-using-mission-control/).
Pain points: [Osmani, agentic code review](https://addyosmani.com/blog/agentic-code-review/),
[AgentConn, oversight budget](https://agentconn.com/blog/oversight-budget-agent-fleet-2026),
[MindStudio playbook](https://www.mindstudio.ai/blog/parallel-agentic-development-playbook),
[Upsun on worktrees](https://developer.upsun.com/posts/ai/git-worktrees-for-parallel-ai-coding-agents),
[5 lessons](https://dev.to/battyterm/5-lessons-from-running-ai-coding-agents-in-parallel-53on).
Usage data (Jellyfish, Stack Overflow, METR) as cited in
[worktrees research §8.1](../../research/worktrees-and-parallel-agents.md). Most sources are vendor
or personal blogs; figures marked second-hand were not checked against the original reports.
