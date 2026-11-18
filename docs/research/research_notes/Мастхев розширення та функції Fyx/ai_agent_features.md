# Must-have AI-agent features in code editors (state as of November 2026): reviewing, controlling and trusting agent-made changes

Scope note: research done 2026-11-17. Primary sources (vendor docs, changelogs, protocol sites) are preferred; third-party blogs are flagged as such. Where a fact has an older date, the date is given inline.

## 1. How do products present agent diffs (inline vs. review pane vs. changeset), granularity, accept/reject, comments back to the agent, "follow the agent"?

### Takeaway
The 2026 baseline is a multi-file changeset view (pill or panel with file count and +/- stats) that opens a multi-diff, with keep/undo at hunk, file and "all" level, plus inline per-file overlays. Leaders have moved on: line-level comments sent back to the agent as a batch (JetBrains Air, Warp, Antigravity), "mark as reviewed" progress, "since last seen" scopes, and automatic AI review of the agent's diff before the human looks. One counter-trend: several products (JetBrains Air, VS Code with Claude/Codex/cloud harnesses) drop per-hunk accept/reject and just commit each turn to a branch, so review becomes a branch diff.

### Cited Findings
**Zed (upstream of Fyx)**
- Agent panel shows how many files/lines the agent edited; "Review Changes" (`ctrl-shift-r`) opens a multi-buffer tab with all changes; each hunk can be accepted or rejected, or the whole set — [Zed docs: Agent Panel](https://zed.dev/docs/ai/agent-panel)
- `agent.single_file_review: true` shows the diffs inline in each file with the same keep/reject controls, temporarily overriding the buffer's git diff — [Zed docs: Agent Panel](https://zed.dev/docs/ai/agent-panel)
- Follow mode: crosshair icon makes Zed jump to each file the agent touches; holding cmd/ctrl on submit follows automatically — [Zed docs: Agent Panel](https://zed.dev/docs/ai/agent-panel)
- Open bug (Feb 2026): review UI appears only in the "Review Changes" multibuffer, not in singleton editors — [zed#49856](https://github.com/zed-industries/zed/issues/49856)
- Parallel-agents page: threads sidebar shows per-thread branch, elapsed time and diff stats (+23 −2); "as threads finish, review their edits, compare approaches, and merge back" — [zed.dev/parallel-agents](https://zed.dev/parallel-agents)

**VS Code / GitHub Copilot**
- Agent edits are saved to disk immediately and tracked as pending review ("working set"); you can run tests on them before deciding — [VS Code docs: Review code edits](https://code.visualstudio.com/docs/agents/run/review-code-edits)
- Keep/Undo controls exist per change, per file, and for all files; Keep doesn't stage/commit; Undo reverts without rewinding the conversation; staging a change in Source Control also keeps the pending edit — [VS Code docs](https://code.visualstudio.com/docs/agents/run/review-code-edits)
- A pill above chat input shows affected files and diff stats and opens a multi-diff editor; the Agents window has a Changes view with multi-file diff, "Mark as Reviewed" (cleared if file changes again), inline/side-by-side toggle, and an Accessible Diff Viewer — [VS Code docs](https://code.visualstudio.com/docs/agents/run/review-code-edits)
- Changes made through terminal commands, scripts or untracked tools do NOT appear in the working set — [VS Code docs](https://code.visualstudio.com/docs/agents/run/review-code-edits)
- For Claude or Codex harness sessions and cloud sessions, you "compare diffs rather than keeping or undoing individual edits" — [VS Code docs](https://code.visualstudio.com/docs/agents/run/review-code-edits)
- `chat.editing.autoAcceptDelay` auto-keeps pending edits after a delay (Local harness only; Local harness deprecated, removal February 2027) — [VS Code docs](https://code.visualstudio.com/docs/agents/run/review-code-edits)
- `chat.tools.edits.autoApprove` takes globs deciding which edits need a pre-save diff approval (example: approve `**/*` but require approval for `.vscode/*.json` and `.env`) — [VS Code docs](https://code.visualstudio.com/docs/agents/run/review-code-edits)

**Cursor**
- CLI (Feb 2026): unified `/changes` (Ctrl+R) with a Session tab showing only this session's edits, open diffs in editor; Apr 2026 git-style unified diffs — [Cursor CLI changelog](https://cursor.com/docs/cli/changelog) (summarised via search; verify exact dates)
- 3.26/3.27 (Oct 2026): PR tab remembers last view (Changes/Description/Commits/Checks/Reviews); generated files start collapsed with a badge; "Add to Chat" in Review Changes; Changes tab keeps scope/branch/picked commits across restarts; Agent Review tabs stay open; very large diffs list every file with line counts — [Cursor release notes](https://cursor.com/docs/release-notes)
- Agent Review: dedicated AI review of local changes vs main branch; Quick vs Deep depth; triggers automatic, `/agent-review`, or Source Control tab; reads `BUGBOT.md` rules — [Cursor docs: Agent Review](https://cursor.com/docs/agent/agent-review)
- Bugbot = PR reviewer; Autofix spawns a Cloud Agent to push fix commits; repo config can lower but not raise autofix mode — [Cursor Bugbot autofix blog](https://cursor.com/blog/bugbot-autofix), [Cursor Bugbot docs](https://cursor.com/docs/bugbot); forum report of Autofix acting despite being disabled — [Cursor forum](https://forum.cursor.com/t/bugbot-auto-fixes-when-it-should-not/155416)
- Cursor for iOS (3.9, June 2026) lets users review diffs and merge PRs from a phone — [learncursor.dev (third-party)](https://www.learncursor.dev/research/whats-new-in-cursor-2026)

**Claude Code (IDE integrations)**
- VS Code extension in Manual mode shows a side-by-side diff before editing; accept, reject or redirect; per-change buttons need v2.1.275+; diffs with >100 changes open without per-change buttons — [Claude Code docs: VS Code](https://code.claude.com/docs/en/vs-code) (via search snippet)
- Earlier in 2026 users reported they could only accept/reject whole files (hunk-level request) — [DataCamp (Apr 2026)](https://www.datacamp.com/tutorial/claude-code-in-vs-code); [feature request mirror](https://claudeissues.com/issue/42448-feature-request-hunk-level-diff-review-for-edits-in-vscode-extension)

**Kiro (AWS)**
- Autopilot applies changes immediately; Supervised pauses after each turn for diff review — [Kiro docs: Privacy and Security](https://kiro.dev/docs/privacy-and-security/)
- 0.10 (Feb 18, 2026): Supervised mode presents changes as hunks; each hunk can be accepted, rejected or "discussed with inline chat"; accept per file or all — [Kiro changelog 0.10](https://kiro.dev/changelog/ide/0-10/)
- 0.8 added "Per-File Code Review" — [Kiro changelog 0.8](https://kiro.dev/changelog/ide/0-8/)
- Kiro docs say supervised mode is a code review workflow, "not a security control"; protected-file patterns force a confirmation in either mode — [Kiro docs](https://kiro.dev/docs/privacy-and-security/)

**JetBrains Air / Junie**
- Air: single "All Changes" tab with per-file +/- counts, unified/side-by-side, F7 navigation. No accept/reject: Air commits the agent's work to the task branch after every turn; feedback = line comments collected in a Comments tool and sent together as a follow-up; "Add to Task" for code fragments; `/review` agentic review — [JetBrains Air help: Review changes](https://www.jetbrains.com/help/air/review-changes.html)
- Air agentic review runs as a separate review subtask/agent session with its own context and leaves comments to triage; JetBrains says it doesn't replace human review — [JetBrains Air help: Agentic review](https://www.jetbrains.com/help/air/agentic-review.html) (via search snippet)
- Junie: after a task, changed/added files are listed in a "Done" panel; revert file by file, then accept or reject the whole result — [Junie IDE plugin docs](https://junie.jetbrains.com/docs/junie-ide-plugin.html)

**Warp**
- Built-in diff editor, changes grouped into hunks (↑/↓ between hunks, ←/→ between files); nothing written until "Accept Changes"; "Refine" (R) regenerates via natural language; "Edit" (E) lets you hand-edit before applying; "Apply Code Diffs" permission: Always allow / Agent decides / Always ask — [Warp docs: Code diffs](https://docs.warp.dev/agents/local-agents/code-diffs.md)
- Interactive code review: inline comments batched and applied by the agent — [Warp docs: Code review](https://docs.warp.dev/code/code-review)

**Google Antigravity**
- Agent pauses when it produces an implementation plan or code diff; Google-Docs-style inline comments on artifacts; "Proceed" or "Review" to send comments as feedback; walkthrough artifact after implementation (with screenshots/recordings for browser tasks) — [Antigravity docs: Artifact review](https://www.antigravity.google/docs/artifact-review), [Implementation plan](https://www.antigravity.google/docs/implementation-plan), [Walkthrough](https://www.antigravity.google/docs/walkthrough)
- CLI supports line-level feedback comments on artifacts — [Antigravity docs: CLI artifacts](https://www.antigravity.google/docs/cli/artifacts)

**Windsurf → Devin Desktop (Cognition)**
- Windsurf renamed Devin Desktop on June 2, 2026 (v3.0.12); Cascade removed Sept 8, 2026 (v3.9.19), Devin Local is the only agent — [Devin Desktop changelog](https://docs.devin.ai/desktop/changelog)
- v3.11.212 (Oct 23, 2026): Changes tab with per-file diffs for local sessions and a "Since last seen" scope; view up to 8 sessions side by side — [Devin Desktop changelog](https://docs.devin.ai/desktop/changelog)

**Cline**
- Every file edit and command requires approval in Act mode unless auto-approve is on — [AgentsCamp comparison (third-party)](https://agentscamp.com/guides/comparisons/cline-vs-kilo-code-vs-roo-code)
- Checkpoint "Compare" opens editor diff viewer across all files at that checkpoint — [Cline docs: Checkpoints](https://docs.cline.bot/core-workflows/checkpoints)

### Inferences
- Fyx's hunk-level accept/reject + per-turn review pane is at parity with Zed/VS Code/Kiro, not ahead. Differentiators visible in the market that Fyx lacks: (a) line comments batched back to the agent as one follow-up (Air, Warp, Antigravity, Kiro "discuss hunk"); (b) "Mark as reviewed" + "since last seen" review-progress tracking (VS Code, Devin Desktop); (c) "edit before apply" (Warp); (d) automatic AI pre-review of the turn's diff (Cursor Agent Review, Air agentic review) — the last could be an extension use case, since it observes but doesn't decide.
- VS Code's "edits on disk, pending review" model lets tests run on unaccepted changes; Fyx should decide explicitly whether proposed-but-unaccepted edits are on disk (affects running tests, LSP diagnostics).
- The trend of "no accept/reject, commit every turn to a task branch" (Air) suggests offering a branch/worktree-diff review mode alongside the gate.

### Gaps
- Could not confirm exact current Cursor IDE in-editor diff UX (keep/undo per hunk) from primary docs; release notes are huge and only partly read.
- Codex IDE extension change-review UI isn't described on the approvals page; not researched further.
- Did not verify whether Zed fixed issue #49856.

## 2. Checkpoints / undo / restore to a previous turn; shadow git; worktree-per-agent and parallel/background agents; how are their results reviewed?

### Takeaway
Checkpoints that restore files and/or conversation per turn are table stakes (Claude Code, VS Code, Cline, Kilo, Cursor), and every vendor documents the same blind spot: shell commands, subagents, and external side effects aren't covered. Worktree-per-agent isolation plus a fleet view (sidebar/board, up to N side-by-side sessions) is the 2026 norm; review of parallel agents is mostly "branch diff → merge/PR", with best-of-n comparison in Cursor.

### Cited Findings
- Claude Code: checkpoint before file edits; `/rewind` (or Esc Esc) restores code, conversation, or both; changes via Bash/external programs not tracked; most subagent edits not in parent's history; external actions can't be undone — [likeone.ai guide (third-party)](https://likeone.ai/blog/claude-code-checkpoints-rewind-guide-2026/); sandbox + rewind on macOS/Linux/WSL reports some files as skipped — [Claude Code settings reference](https://code.claude.com/docs/en/settings-reference) (via search summary)
- VS Code: checkpoints on by default (`chat.checkpoints.enabled`), snapshot affected files before each request; Restore Checkpoint removes later turns; doesn't reverse terminal commands, network requests, deployments; `chat.editRequests` reverts and resends an edited request — [VS Code docs](https://code.visualstudio.com/docs/agents/run/review-code-edits)
- Cline: shadow Git repository separate from project history; snapshot after each tool use incl. untracked files; Restore Files / Restore Task Only / Restore Files & Task; Compare button; large repos cost storage/speed — [Cline docs: Checkpoints](https://docs.cline.bot/core-workflows/checkpoints)
- Kilo Code: checkpoints on by default, stored in a dedicated Git repo outside the project, "Revert to here"; Agent Manager runs agents in parallel in git worktrees with inline review and side-by-side multi-model comparison; v7 (GA Apr 2, 2026) built on the OpenCode server — [AgentsCamp (third-party)](https://agentscamp.com/guides/comparisons/cline-vs-kilo-code-vs-roo-code), [Kilo docs: Checkpoints](https://kilo.ai/docs/code-with-ai/features/checkpoints)
- Roo Code shut down May 15, 2026, repo archived at v3.54.0; README points to ZooCode and Cline — [Roo Code repo](https://github.com/RooCodeInc/Roo-Code) (as reported by [AgentsCamp](https://agentscamp.com/guides/comparisons/cline-vs-kilo-code-vs-roo-code))
- Void editor deprecated; repo archived June 2, 2026 (sources disagree on pause date: Aug 2025 vs Jan 12, 2026) — [bodegaone.ai (third-party)](https://www.bodegaone.ai/blog/void-editor-discontinued-alternatives), [toolchew (third-party)](https://toolchew.com/en/void-editor-review/)
- Cursor 3 (Apr 2026): Agents Window runs many agents across local, worktree, cloud and SSH; `/worktree` and `/best-of-n` (same task across models, each in its own worktree, compare, then `/apply-worktree`) — [Cursor changelog 3.0](https://cursor.com/changelog/3-0), [Cursor docs: worktrees](https://cursor.com/docs/configuration/worktrees); forum reports Agents Window lacked these commands at launch — [Cursor forum](https://forum.cursor.com/t/cursor-3-worktrees-best-of-n/156507)
- Cursor Oct 2026: Projects with coordinator + workers shown in List/Board views; coordinators link worker PRs to tasks; "Input Needed" status; cloud agents open PRs ready for review by default; Cmd+Z restores an archived agent — [Cursor release notes](https://cursor.com/docs/release-notes)
- Zed: per-thread choice of working copy vs own Git worktree; Threads Sidebar; import historical ACP threads — [zed.dev/parallel-agents](https://zed.dev/parallel-agents), [Zed docs: Agent Panel](https://zed.dev/docs/ai/agent-panel). Zed 1.0 shipped Apr 29, 2026 with parallel agents as headline — [third-party](https://aitechconnect-976.pages.dev/news/zed-1-0-parallel-agents-agent-client-protocol)
- Zed Terminal Threads (May 20, 2026): run `claude`, `amp`, `codex` CLIs as managed threads in the Agent Panel; post describes no review/diff tracking for them; ACP agents still have "tighter integration with Zed's editing and review workflows" — [Zed blog: Terminal Threads](https://zed.dev/blog/terminal-threads)
- Devin Desktop: Worktree option in location picker (Sept 8), Merge button for worktree sessions (July 29), `post_setup_worktree` hooks (Aug 21), Agent Command Center tabs + local sessions in a background process (Nov 17, 2026) — [Devin Desktop changelog](https://docs.devin.ai/desktop/changelog)
- JetBrains Air: each task in its own Git worktree or Docker container; bring changes back to local workspace before accept/commit — [JetBrains Air help](https://www.jetbrains.com/help/air/review-and-integrate.html) (via search snippet)
- VS Code: worktree sessions in the Agents window always use "Allow all" permissions — [VS Code docs: approvals](https://code.visualstudio.com/docs/agents/run/approvals)
- GitHub Copilot cloud agent works on a single `copilot/` branch, opens a draft PR, can't mark ready/approve/merge; commits signed; PR CI requires human approval to run — [GitHub docs: risks and mitigations](https://docs.github.com/en/copilot/concepts/agents/cloud-agent/risks-and-mitigations), [GitHub blog](https://github.blog/news-insights/product-news/github-copilot-meet-the-new-coding-agent/)

### Inferences
- Fyx's planned "detect writes that bypass the gate" directly addresses the universal checkpoint/review blind spot (terminal-made changes). A shadow-git snapshot per turn (Cline/Kilo style) would let Fyx both detect and revert out-of-band writes, and would extend review to Zed-style Terminal Threads, which currently have no review at all.
- Worktree sessions often force "Allow all" (VS Code); Fyx could differentiate by keeping per-turn review even in worktree sessions (review the turn diff in the worktree, then merge).

### Gaps
- No primary source found on how Cursor IDE checkpoints are stored.
- Didn't verify Claude Code checkpoint retention period (one third-party guide says 30 days).

## 3. Permission and policy models: auto-approve, allow/deny lists, sandboxing, YOLO, hooks, enterprise policy; security research recommendations

### Takeaway
The industry converged in 2026 on two orthogonal layers — (1) approval policy (manual → rules → AI "auto-review" classifier → allow-all/YOLO) and (2) OS-level sandbox for terminal commands with network allowlists — plus managed/enterprise settings that can pin or forbid modes. Anthropic's own data shows human approval prompts are largely rubber-stamped (97% approved), which is the main argument vendors give for classifier-based auto modes. Security researchers keep finding the same class of bug: agents writing to config files (settings.json, mcp.json) that then grant themselves more power, and exfiltration via the "lethal trifecta".

### Cited Findings
**Claude Code**
- Modes: Manual (`default`), `acceptEdits`, `plan`, `auto`, `dontAsk`, `bypassPermissions`; allow/ask/deny rules layer over all modes, deny blocks in every mode — [Claude Code docs: Permission modes](https://code.claude.com/docs/en/permission-modes)
- Auto mode: separate classifier model reviews actions; broad allow rules (`Bash(*)`, interpreters) are dropped on entering auto; blocked by default: `curl | bash`, sending sensitive data externally, prod deploys, force push, `git reset --hard`, terraform destroy, merging unapproved PRs; falls back to prompting after 3 consecutive / 20 total blocks; default classifier model Sonnet 5 — [Claude Code docs](https://code.claude.com/docs/en/permission-modes)
- Protected paths (`.git`, `.vscode`, `.idea`, `.husky`, `.claude`, `.gitconfig`, shell rc files, `.npmrc`, `.mcp.json`) aren't pre-approved by allow rules; `permissions.blockWritesOutsideWorkingDirectories` (v2.1.314+) — [Claude Code docs](https://code.claude.com/docs/en/permission-modes)
- Repos can't self-enable auto/bypass: `auto` and `bypassPermissions` ignored in project/local settings; managed settings `disableAutoMode`, `disableBypassPermissionsMode` — [Claude Code docs](https://code.claude.com/docs/en/permission-modes)
- Auto mode default for Pro/Max/Ultra/Team from Aug 14, 2026; data: users approve 97% of permission prompts but reject 39% of plans; 62% have used bypass or "don't ask again" on Bash; 25% of interactive sessions start in bypass; in a 1,053-tester study humans caught 13.6% of dangerous commands vs 89% by auto mode; human block rate fell from ~17% to ~5% after 50+ prompts — [Anthropic: Auto mode default](https://claude.com/resources/articles/auto-mode-default-in-claude-code)
- Hooks: 30+ events incl. `PreToolUse` (allow/deny/ask/defer + `updatedInput`), `PermissionRequest`, `PostToolUse`, `PostToolBatch`, `Stop`, `FileChanged`, `ConfigChange`, `WorktreeCreate`, `PreCompact`, `Elicitation`; hook types command, http, mcp_tool, prompt, agent (experimental) — [Claude Code docs: Hooks](https://code.claude.com/docs/en/hooks)

**OpenAI Codex**
- Sandbox modes `read-only`, `workspace-write` (default with Auto preset, network off), `danger-full-access`; OS enforcement Seatbelt (macOS), bwrap+seccomp (Linux), MXC (Windows) — [Codex docs: approvals & security](https://learn.chatgpt.com/docs/agent-approvals-security)
- Approval: `on-request`, `never`, granular categories (`sandbox_approval`, `rules`, `mcp_elicitations`, `request_permissions`, `skill_approval`); `untrusted` retired; `approvals_reviewer = "auto_review"` sends eligible requests to a reviewer agent (critical risk denied, high risk needs user authorization; failures fail closed) — [Codex docs](https://learn.chatgpt.com/docs/agent-approvals-security)
- Protected paths read-only even in writable roots: `.git`, `.agents`, `.codex`, `.aws` — [Codex docs](https://learn.chatgpt.com/docs/agent-approvals-security)
- Network proxy with domain allow/deny (deny wins), local binding blocked by default; admins: `allowed_approval_policies`, `allowed_approvals_reviewers`, `requirements.toml` — [Codex docs](https://learn.chatgpt.com/docs/agent-approvals-security)
- `--full-auto` removed (versions disputed: 0.147.0 Aug 7, 2026 vs 0.154.0); `--yolo` bypasses sandbox and approvals — [codeagentswarm (third-party)](https://www.codeagentswarm.com/en/guides/codex-yolo-mode), [dev.to (third-party)](https://dev.to/its-tahir/i-went-looking-for-codex-auto-mode-its-called-approve-for-me-now-376p)

**VS Code / Copilot**
- `chat.permissions.default`: Manual, Assisted (LLM judge), Allow all, Autopilot (auto-approves and auto-answers questions), Advanced Autopilot (experimental); `/yolo` per session — [VS Code docs: approvals](https://code.visualstudio.com/docs/agents/run/approvals)
- Scoped approvals: command prefix/exact, host, file/folder (separate read/edit), Allow Once / Session / Always; URL request vs response approval (response approval guards prompt injection) — [VS Code docs](https://code.visualstudio.com/docs/agents/run/approvals)
- Terminal auto-approve per subcommand, `false` overrides `true`; docs say it's "best-effort convenience, not a security boundary"; `chat.tools.terminal.blockDetectedFileWrites` (default `outsideWorkspace`) — [VS Code docs](https://code.visualstudio.com/docs/agents/run/approvals)
- Sandbox (`chat.agent.sandbox.enabled`) still applies under Allow all/Autopilot; preview on macOS/Linux/WSL2, experimental on Windows; network domain allow/deny; managed settings can force sandbox; policy can disable Claude/Codex harnesses — [VS Code docs](https://code.visualstudio.com/docs/agents/run/approvals)

**Others**
- Cursor: "Always Run" adds to `terminalAllowlist` in `permissions.json`; Auto-review run mode (middle ground between Allowlist and Run Everything) reached CLI Sept 2026; Cursor hooks honour Claude Code `PreToolUse` deny responses — [Cursor release notes](https://cursor.com/docs/release-notes), [Cursor CLI changelog](https://cursor.com/docs/cli/changelog)
- Devin Desktop: teams enforce terminal allow/deny via CLI permission scopes (June 16), `sandbox.excluded` allow/ask/deny (July 4), "explicit deny always wins across permission layers" (Aug 10), agents and hooks disabled in Restricted Mode — [Devin Desktop changelog](https://docs.devin.ai/desktop/changelog)
- Antigravity: terminal policy "Request Review" with allow list vs "Always Proceed" (older Off/Auto/Turbo); strict mode forces review; terminal sandbox preview on macOS/Linux — [agentpedia (third-party)](https://agentpedia.codes/blog/antigravity-auto-accept-autopilot-guide), [Google Cloud Medium](https://medium.com/google-cloud/agent-permissions-in-antigravity-da9d326e63f4)
- Junie: asks by default; Action Allowlist; Brave Mode skips approvals and JetBrains advises against it — [Junie docs](https://junie.jetbrains.com/docs/junie-ide-plugin.html)
- Cline: command auto-approval off by default since v4.0.0 (June 26, 2026) — [AgentsCamp (third-party)](https://agentscamp.com/guides/comparisons/cline-vs-kilo-code-vs-roo-code)
- GitHub cloud agent: firewall on outbound by default, org-level firewall settings (Apr 3, 2026) — [GitHub changelog](https://github.blog/changelog/2026-04-03-organization-firewall-settings-for-copilot-cloud-agent/), [GitHub docs](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/cloud-agent/customize-the-agent-firewall)

**Security research**
- Lethal trifecta (private data + untrusted content + external communication), named by Simon Willison June 16, 2025 — [Simon Willison](https://simonwillison.net/2025/Jun/16/the-lethal-trifecta/), [Arcjet summary](https://arcjet.com/learn/lethal-trifecta)
- Kiro (2025): agent could write `.vscode/settings.json` to allowlist commands without consent → arbitrary code execution — [Embrace The Red](https://embracethered.com/blog/posts/2025/aws-kiro-aribtrary-command-execution-with-indirect-prompt-injection)
- Cross-agent privilege escalation: one agent rewriting another agent's config — [Embrace The Red](https://embracethered.com/blog/posts/2025/cross-agent-privilege-escalation-agents-that-free-each-other)
- Copilot RCE via prompt injection editing settings (CVE-2025-53773) — [Embrace The Red](https://embracethered.com/blog/posts/2025/github-copilot-remote-code-execution-via-prompt-injection); Roo Code CVE-2025-53536 write to `.vscode/settings.json` when Write auto-approved — [radar.offseq](https://radar.offseq.com/threat/cve-2025-53536-cwe-552-files-or-directories-access-1bfed6cf); Cline .env exfiltration via image rendering — [Embrace The Red](https://embracethered.com/blog/posts/2025/cline-vulnerable-to-data-exfiltration)
- Aug 2026: report of Kiro rewriting `~/.kiro/settings/mcp.json` after reading hidden webpage instructions (not verified against primary) — [hyrax.dev](https://hyrax.dev/blog/august-2026-ai-coding-agent-rce-pattern)
- Pillar Security (2026): sandboxes of Cursor, Codex CLI, Gemini CLI, Antigravity defeated by manipulating what trusted host tools do with agent-written output ("trust handoff") — [CSO Online](https://www.csoonline.com/article/4199408/ai-agents-can-escape-sandboxes-without-ever-breaking-them.html), [CSA research note (Jul 22, 2026)](https://labs.cloudsecurityalliance.org/research/csa-research-note-ai-coding-agent-sandbox-escapes-20260722-c/)
- Mindgard: Antigravity persistent code execution even in most restrictive mode (vendor claim) — [Mindgard](https://mindgard.ai/blog/google-antigravity-persistent-code-execution-vulnerability)
- Trail of Bits publishes a hardened devcontainer for Claude Code on untrusted codebases — [trailofbits/claude-code-devcontainer](https://github.com/trailofbits/claude-code-devcontainer) (found via mirror; not verified)

### Inferences
- `.fyx/review.toml` "always review" should ship with a default protected list mirroring Claude Code/Codex (`.git`, `.vscode`, `.idea`, `.fyx` itself, `.claude`, `.codex`, `.mcp.json`, shell rc, `.env*`, CI workflows) — the documented 2025–2026 exploit class is exactly agent writes to these files. The `.fyx/review.toml` file must itself be always-review and not loosenable by a repo (Claude Code ignores `auto`/`bypass` in project settings for this reason).
- "Pillar trust handoff" research supports Fyx's model: even when the sandbox holds, the danger is the host acting on agent-written files; a review gate on writes plus bypass detection is the right layer.
- 97% approval rate means hunk-by-hunk review of everything will be rubber-stamped; Fyx should tier review (auto-accept low-risk paths, force review on protected paths, highlight risky hunks).

### Gaps
- No primary Trail of Bits 2026 guidance document found.
- Cursor sandbox details for local agents not documented in sources read.

## 4. Spec-driven / plan-first flows, rules files, memory, context-window UI

### Takeaway
Plan-first gates are table stakes (Claude Code plan mode, Cursor `/plan`, Antigravity 2.0 `/plan` and plan review policy, Cline Plan/Act, Kilo Plan agent, Kiro specs). AGENTS.md is the de-facto cross-tool rules file (Cursor reads nested AGENTS.md and dedupes CLAUDE.md mirrors); Agent Skills (`SKILL.md`) became a cross-vendor format. Context meters + compaction are standard; ACP stabilised session usage (context window + cost) in June 2026.

### Cited Findings
- Claude Code plan mode: edits blocked until plan approval; options "Yes, and use auto mode", "Yes, manually approve edits", "No, keep planning"; Ctrl+G opens plan in editor — [Claude Code docs](https://code.claude.com/docs/en/permission-modes)
- Users reject 39% of plans vs 3% of individual permission requests — [Anthropic](https://claude.com/resources/articles/auto-mode-default-in-claude-code)
- Kiro specs in `.kiro/specs/` (requirements, design, tasks); 0.10 adds design-first specs, bugfix specs (`bugfix.md`), pre/post task hooks — [Kiro changelog 0.10](https://kiro.dev/changelog/ide/0-10/); steering files for persistent conventions — [AWS Builder guide](https://builder.aws.com/content/39juiKF2uwxhek0RuYHhjf24JjL)
- Antigravity 2.0 `/plan` mode: research, then plan for review, waits for approval — [Antigravity on X](https://x.com/antigravity/status/2103611698800140697); plan review policy (review every plan / when worthwhile / skip) per a Sept 2026 guide — [agentpedia (third-party)](https://agentpedia.codes/blog/antigravity-auto-accept-autopilot-guide)
- Cursor: nested `AGENTS.md` and nested always-apply rules apply when editing files in that dir; no duplicate instructions when CLAUDE.md mirrors AGENTS.md; skills with invalid frontmatter flagged — [Cursor release notes](https://cursor.com/docs/release-notes)
- Devin Desktop: workflows deprecated in favour of skills; migration wizard for hooks/workflows/memories — [Devin Desktop changelog](https://docs.devin.ai/desktop/changelog)
- Agent Skills spec: folder with `SKILL.md`, name ≤64 chars, description ≤1024; ~100 tokens metadata loaded at start — [agentskills.io](https://agentskills.io/specification) (via [classicdba summary](https://classicdba.com/generative-ai/agent-client-protocol-in-october-2026-what-acp-stabilized-what-v2-draft-adds-and-how-it-differs-from-mcp)); ACP has an Agent Skills RFD — [ACP RFD](https://agentclientprotocol.com/rfds/agent-skills.md)
- Zed: token usage near profile selector; `agent.auto_compact`, "Context Compacted" entry inspectable; `/compact`; banner near limit for <80k windows; external agents' token display depends on integration — [Zed docs](https://zed.dev/docs/ai/agent-panel)
- Cursor: `/summarize`, background summaries in long sessions — [Cursor release notes](https://cursor.com/docs/release-notes)

### Inferences
- For Fyx review, plans are a review object too (39% rejection rate shows plan review is where humans actually exercise judgement). Showing the ACP plan next to the turn's diff ("plan vs. what changed") is a cheap differentiator.

### Gaps
- Did not research memory UIs in depth (Cursor memories, Windsurf memories) beyond Devin migration notes.

## 5. Agent observability: session timeline, tool-call log, cost/usage, per-turn diff, review of agent-produced PRs

### Takeaway
Observability is converging on: per-session logs with subagent visibility, commit-to-session traceability, fleet dashboards with status ("Input Needed"), diff stats per thread, and cumulative cost. GitHub leads on PR-provenance (Agent-Logs-Url commit trailer); ACP now carries usage/cost.

### Cited Findings
- GitHub: agent commits link back to session logs via an `Agent-Logs-Url` trailer (Mar 20, 2026) — [GitHub changelog](https://github.blog/changelog/2026-03-20-trace-any-copilot-coding-agent-commit-to-its-session-logs); more session-log visibility incl. subagents (Mar 19) — [GitHub changelog](https://github.blog/changelog/2026-03-19-more-visibility-into-copilot-coding-agent-sessions/); session side panel in issues to view progress/logs/steer (Apr 23) — [GitHub changelog](https://github.blog/changelog/2026-04-23-view-and-manage-agent-sessions-from-issues-and-projects/); steering a running session — [GitHub docs](https://docs.github.com/en/copilot/how-tos/copilot-on-github/use-copilot-agents/manage-and-track-agents); JetBrains Agent Debug panel logs summary (June 22) — [GitHub changelog](https://github.blog/changelog/2026-06-22-new-features-and-claude-as-agent-provider-preview-in-jetbrains-ides/)
- Agent HQ / mission control: third-party agents (Claude, Codex) in public preview Feb 4, 2026 on github.com, Mobile, VS Code 1.109+ — [baeseokjae guide (third-party)](https://baeseokjae.github.io/posts/github-agent-hq-guide-2026/)
- Cursor: Projects Board view, Input Needed status, hook scripts get `parent_agent_id` for subagent events — [Cursor release notes](https://cursor.com/docs/release-notes)
- ACP session usage updates (context-window use and optional cumulative cost) stabilised Jun 5, 2026 — [ACP updates](https://agentclientprotocol.com/updates)
- Zed threads sidebar shows elapsed time and +/- per thread — [zed.dev/parallel-agents](https://zed.dev/parallel-agents)
- Copilot cloud agent runs Copilot code review on its own code by default and security checks — [GitHub docs](https://docs.github.com/en/copilot/concepts/agents/cloud-agent/risks-and-mitigations)

### Inferences
- Fyx's per-turn review pane is already a "diff of what changed per turn"; adding an exportable per-turn audit record (prompt → plan → tool calls → hunks accepted/rejected → by whom) would be a natural trust feature and an extension data source (observation-only API).
- A commit trailer linking accepted hunks to the Fyx session (like GitHub's Agent-Logs-Url) is low-cost provenance.

### Gaps
- No good source on per-tool-call cost breakdown UIs in editors.

## 6. ACP status 2026: spec version, agents/clients, registry, gaps relevant to review

### Takeaway
ACP v1 is the stable baseline (Rust/TS SDKs 1.0 on Jun 25, 2026); v2 is a Draft since Jul 20, 2026. The registry (Zed + JetBrains, Jan 2026) lists dozens of agents incl. Claude, Codex, Gemini, Copilot, Cursor, Junie, Kiro CLI, Cline, Devin Local. Critical for Fyx: the v2 RFD "Client Filesystem and Terminal Execution Surface" removes `fs/read_text_file`, `fs/write_text_file` and client terminals, leaving no client-mediated write path — so a gate that relies on `fs/write_text_file` will not work on v2 connections; review must rely on tool-call diff content, permission requests and disk-change detection.

### Cited Findings
- ACP updates timeline: session config options (Oct 24, 2025); JetBrains' Sergey Ignatov joins as Lead Maintainer (Feb 4, 2026); session list (Feb 18); registry released + Transports WG + session info updates (Mar 9); session resume & close (Apr 22); logout (Apr 23); additional workspace roots (May 21); session delete (Jun 1); message IDs + session usage (Jun 5); model config category (Jun 24); SDKs 1.0 (Jun 25); request cancellation (Jun 29); boolean config options (Jul 6); v2 Draft (Jul 20); elicitation (Jul 22); tool call names (Sep 17); client-provided system prompts (Nov 6, 2026) — [ACP updates](https://agentclientprotocol.com/updates)
- Remote transport still in progress (Streamable HTTP & WebSocket RFD) — [ACP intro](https://agentclientprotocol.com/overview/introduction), [RFD](https://agentclientprotocol.com/rfds/streamable-http-websocket-transport.md)
- Registry: launched by Zed and JetBrains (beta Jan 28, 2026 per heise/JetBrains; ACP updates page lists registry release Mar 9, 2026); 9 agents at launch (Auggie, Claude Code, Codex CLI, Factory Droid, Gemini CLI, GitHub Copilot, Mistral Vibe, OpenCode, Qwen Code); JetBrains needs IDE 2025.3.2+ — [heise](https://www.heise.de/en/news/Integrate-AI-Agents-into-the-Editor-JetBrains-and-Zed-Release-ACP-Registry-11160709.html), [classicdba (third-party)](https://classicdba.com/generative-ai/agent-client-protocol-in-october-2026-what-acp-stabilized-what-v2-draft-adds-and-how-it-differs-from-mcp), [Zed blog](https://zed.dev/blog/acp-registry). Cursor joined Mar 2026 — [JetBrains blog](https://blog.jetbrains.com/ai/2026/03/cursor-joined-the-acp-registry-and-is-now-live-in-your-jetbrains-ide/). No published security review process for registry entries found — [classicdba](https://classicdba.com/generative-ai/agent-client-protocol-in-october-2026-what-acp-stabilized-what-v2-draft-adds-and-how-it-differs-from-mcp)
- Agents page lists 51 agents incl. Claude Agent (via Zed's SDK adapter), Codex CLI (via adapter), Cline, Cursor, Gemini CLI, GitHub Copilot (public preview), Goose, Junie, Kiro CLI, OpenCode, OpenHands, Qwen Code, Mistral Vibe — [ACP agents](https://agentclientprotocol.com/get-started/agents.md)
- Clients: Zed, JetBrains, Neovim (CodeCompanion, agentic.nvim, avante.nvim), Emacs (agent-shell.el), Sublime Text, Qt Creator, Visual Studio/VS Code extensions, Obsidian, Unity, many desktop/mobile/messaging clients — [ACP clients](https://agentclientprotocol.com/get-started/clients.md)
- Devin Desktop: ACP always on (Sept 10, 2026), MCP servers passed by ACP clients (Sept 24) — [Devin Desktop changelog](https://docs.devin.ai/desktop/changelog)
- **v2 removes client filesystem and terminal**: removes `clientCapabilities.fs`, `clientCapabilities.terminal`, `fs/read_text_file`, `fs/write_text_file`, `terminal/*`; terminal content becomes agent-owned display-only; agents to move to "their own sandboxing"; no mention of edit review/approval; draft 2026-06-02, updated 2026-07-14 — [ACP RFD v2 client FS/terminal](https://agentclientprotocol.com/rfds/v2/client-filesystem-terminal-capabilities.md)
- **v2 diff content**: required `changes` array (add/delete/modify/move/copy, optional `fileType` text/binary/directory/symlink, `mimeType`) + optional `patch` in `git_patch` format with absolute paths (display-only, not `git apply`-able); latest revision 2026-10-12, still proposal — [ACP RFD v2 Diff File States](https://agentclientprotocol.com/rfds/v2/diff-file-states.md)
- **v2 permission requests**: `title`, `description`, `options`, optional `subject` union (`tool_call` by id, `command` with command+absolute cwd); no persistence/scope semantics for "always" decisions — [ACP RFD v2 Permission Requests](https://agentclientprotocol.com/rfds/v2/permission-requests.md)
- Other v2 RFDs relevant to review: Tool Call Updates, Plan Variants, Background Work, Terminal Output, Subagents; other RFDs: session fork, proxy chains ("Agent Extensions via ACP Proxies"), MCP-over-ACP, plan operations, session compaction — [ACP llms.txt index](https://agentclientprotocol.com/llms.txt)
- Known external-agent limits in Zed: no editing previously sent messages; Zed profiles/skills don't apply to external agents (third-party report) — [codex.danielvaughan.com](https://codex.danielvaughan.com/2026/05/05/codex-cli-in-zed-parallel-agents-acp-integration-ide-workflows/)
- Economics: Anthropic planned (May 13) to move Agent SDK/`claude -p`/ACP usage to a separate credit pool on June 15, 2026, then paused; Zed (updated June 16) says ACP usage continues to work with subscriptions — [Zed blog](https://zed.dev/blog/anthropic-subscription-changes), [The New Stack](https://thenewstack.io/anthropic-pauses-claude-agent-sdk-subscription-change/). Zed cited this as one rationale for Terminal Threads — [Zed blog](https://zed.dev/blog/terminal-threads)

### Inferences
- Fyx's gate design (fs/write_text_file + request_permission) is v1-specific. For v2, Fyx needs: (1) review built from tool-call `diff` content (`changes` + `patch`) — good news, the v2 shape gives file-level ops and renderable patch for the review pane; (2) disk-change detection as the primary enforcement (since there's no client write path, the agent writes directly); (3) revert-on-reject via snapshot. The planned "bypass detection" feature becomes the core mechanism, not an edge case.
- No ACP primitive exists for "edit proposal awaiting approval" or for the client to tell the agent which hunks were rejected; Fyx could propose an RFD (or use `_meta`/proxy chains) for hunk-level accept/reject feedback, which would be genuinely differentiating and fits Zed/JetBrains' maintainership.
- Permission v2 has no "always allow" persistence semantics — space for Fyx's `.fyx/review.toml` to be the client-side policy layer.

### Gaps
- Did not confirm whether Claude Agent ACP adapter routes edits through `fs/write_text_file` today (relevant to how much Fyx's gate already sees).
- No authoritative current registry agent count (third-party claims 30+ / 42 conflict with 51 on agents page; registry ≠ agents list).
- No v2 stabilisation timeline published.

## 7. MCP ecosystem in editors: server management UI, MCP Apps, popular servers

### Takeaway
MCP 2026-07-28 (stateless core, OAuth alignment) is live; MCP Apps (`ui://` HTML resources in sandboxed iframes, SEP-1865) is Final and supported by Claude, ChatGPT, VS Code (experimental), Goose, Postman, Cursor. Editor MCP UIs now include registries/galleries, per-server on/off, OAuth re-auth, and auto-review of MCP tool calls. Top coding servers: GitHub, Context7, Playwright.

### Cited Findings
- SEP-1865 MCP Apps: status Final, Extensions Track; `ui://` resources, `text/html;profile=mcp-app`; mandatory iframe sandboxing, predeclared templates reviewable before render, auditable JSON-RPC, hosts can require approval for UI-initiated tool calls — [MCP SEP-1865](https://modelcontextprotocol.io/seps/1865-mcp-apps-interactive-user-interfaces-for-mcp); stable version 2026-01-26 — [ext-apps spec](https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/2026-01-26/apps.mdx)
- Host support claimed: Claude, ChatGPT, VS Code (`chat.mcp.apps.enabled` experimental), Goose, Postman, Cursor — [ayautomate (third-party)](https://www.ayautomate.com/blog/mcp-apps); 869 registry servers with MCP Apps, common UI breakage — [dev.to mcpscore](https://dev.to/mcpscore/mcp-apps-in-the-registry-869-servers-and-the-five-ways-their-uis-break-1kb8)
- MCP 2026-07-28: RC May 21, final July 28, 2026; removes initialize handshake and `Mcp-Session-Id`; per-request protocol version/capabilities in `_meta`; OAuth/OIDC alignment — [MCP blog RC](https://blog.modelcontextprotocol.io/posts/2026-07-28-release-candidate/), [Anthropic](https://claude.com/fr/blog/bringing-mcp-2026-07-28-to-claude)
- Editor MCP management: Devin Desktop supports 2026-07-28 (Oct 12), installs from every configured registry (Oct 23), on/off + re-auth in Agent Command Center (Nov 8) — [Devin Desktop changelog](https://docs.devin.ai/desktop/changelog); Cursor: "Connecting…" state, 15-min OAuth window, auto-review of MCP tool calls, one bad tool no longer hides others — [Cursor release notes](https://cursor.com/docs/release-notes); Kiro 0.10: MCP prompts, resource templates, elicitation — [Kiro changelog](https://kiro.dev/changelog/ide/0-10/); VS Code: tool approvals per MCP server, response approval — [VS Code docs](https://code.visualstudio.com/docs/agents/run/approvals)
- Popular servers: GitHub, Context7, Playwright converge at top of rankings (star counts vary; editorial sources) — [mcp.directory](https://mcp.directory/blog/top-10-most-popular-mcp-servers), [dupple](https://dupple.com/learn/best-mcp-servers); ~40-tool ceiling in Cursor noted — [same search, third-party](https://capitalandcompute.net/blog/best-mcp-servers/)

### Inferences
- MCP Apps' security model (sandboxed iframe, auditable JSON-RPC, consent for UI-initiated tool calls) is close to Fyx's planned sandboxed UI API for extensions; aligning Fyx's extension UI API with MCP Apps would let MCP-server UIs render inside Fyx agent sessions and reuse an existing ecosystem.
- MCP config files (`.mcp.json`, `~/.kiro/settings/mcp.json`) are a known injection target → belong on Fyx's always-review list.

### Gaps
- No neutral usage dataset for MCP server popularity.
- Zed's own MCP Apps support status not checked.

## 8. User complaints and pain points about reviewing AI changes; surveys

### Takeaway
Trust is conditional on verifiability: in Stack Overflow 2026, ~48% trust AI only when they can easily verify its output and only 6.6% trust it for important decisions. Review is now the bottleneck (HN), approval prompts are rubber-stamped (Anthropic data), and AI reviewers add noise. Pain points: giant unreviewable diffs, changes outside the tracked set (terminal), whole-file-only accept, and cost of large-repo snapshots.

### Cited Findings
- Stack Overflow 2026 survey (released Oct 6, 2026, 30k+ responses): ~48% trust AI when output can be easily verified; 6.6% for important decisions; 78.7% say source attribution important; 66% use coding assistants/agents; 26.2% automated agent workflows; only 24% have an official AI policy — [Stack Overflow press release](https://stackoverflow.co/company/press/archive/stack-overflow-annual-developer-survey-ai-trust-conditional), [SO blog](https://stackoverflow.blog/2026/10/06/the-results-of-the-2026-developer-survey-are-here/)
- 2025 survey trust figures are inconsistent across secondary sources (46% distrust vs 29–33% trust) — [windowsforum (third-party)](https://windowsforum.com/news/stack-overflow-survey-ai-coding-use-soars-as-developer-trust-falls.446666/)
- HN: "AI coding agents generate more code, but not more software" — review already a bottleneck and worse with agents; tests pass while hiding subtle bugs — [HN 50059523](https://news.ycombinator.com/item?id=50059523); AI code review noise leads developers to ignore feedback; small diffs get better feedback — [HN 46766961](https://news.ycombinator.com/item?id=46766961)
- Meta RADAR study reportedly: +105.9% significant lines per human-landed diff; reviewed diffs ~1/3 the revert rate of unreviewed (secondary) — [remio.ai](https://www.remio.ai/post/hacker-news-put-code-review-on-trial-ai-made-the-bottleneck-impossible-to-ignore)
- SonarSource: 96% don't fully trust AI code, only 48% verify before committing; Faros AI: +441.5% review duration (secondary, unverified) — [AgentConn](https://agentconn.com/blog/verify-dont-review-coding-agents-2026/)
- Anthropic: manual prompts lead to reflexive click-through; 97% approval — [Anthropic](https://claude.com/resources/articles/auto-mode-default-in-claude-code)
- Kiro practitioners: big-bang autopilot refactor = "one giant diff that nobody can honestly review" — [Cloudride (third-party)](https://www.cloudride.co.il/blog/kiro-ide-2026-review/)
- Requests for hunk-level review in Claude Code VS Code (2026) — [claudeissues mirror](https://claudeissues.com/issue/33932-feature-vs-code-extension-diff-review-ui-similar-to-github-copilot-edits-review)

### Inferences
- "Trust when verifiable" argues for review features that make verification cheap: risk-ranked hunks, tests/diagnostics attached to the turn, plan-vs-diff, attribution of each hunk to the tool call and prompt that produced it (78.7% value attribution).

### Gaps
- Did not find Reddit threads with quantified complaints; HN coverage is anecdotal.

## 9. Table stakes vs. differentiating features (synthesis for Fyx)

### Takeaway
Table stakes in Nov 2026: multi-file changeset review with hunk/file/all keep-undo; per-turn checkpoints with restore files/conversation; plan mode; allow/ask/deny rules with deny-wins; terminal sandbox with network allowlist; AI auto-review/classifier mode; worktree-isolated parallel agents with a fleet sidebar; AGENTS.md/skills; context meter + compaction; MCP management with OAuth; hooks. Differentiators: closed feedback loop (line comments → agent), review-progress tracking, AI pre-review of each turn, out-of-band write detection, protected-path policy that repos can't loosen, provenance/audit trail, and review that survives ACP v2's removal of client fs.

### Cited Findings
- Hunk/file/all review: Zed, VS Code, Kiro — [Zed docs](https://zed.dev/docs/ai/agent-panel), [VS Code docs](https://code.visualstudio.com/docs/agents/run/review-code-edits), [Kiro 0.10](https://kiro.dev/changelog/ide/0-10/)
- Checkpoints: Claude Code, VS Code, Cline, Kilo — see section 2 sources
- Two-layer approvals + sandbox: VS Code, Codex — [VS Code](https://code.visualstudio.com/docs/agents/run/approvals), [Codex](https://learn.chatgpt.com/docs/agent-approvals-security)
- AI reviewer of approvals: Claude Code auto mode, Codex auto_review, VS Code Assisted, Cursor Auto-review — [Claude Code](https://code.claude.com/docs/en/permission-modes), [Codex](https://learn.chatgpt.com/docs/agent-approvals-security), [VS Code](https://code.visualstudio.com/docs/agents/run/approvals), [Cursor CLI changelog](https://cursor.com/docs/cli/changelog)
- AI review of diffs: Cursor Agent Review/Bugbot, Air agentic review, Copilot cloud agent self-review — [Cursor](https://cursor.com/docs/agent/agent-review), [Air](https://www.jetbrains.com/help/air/agentic-review.html), [GitHub](https://docs.github.com/en/copilot/concepts/agents/cloud-agent/risks-and-mitigations)
- Comments-to-agent: Air, Warp, Antigravity, Kiro — section 1 sources
- Review progress: VS Code Mark as Reviewed, Devin "Since last seen" — [VS Code](https://code.visualstudio.com/docs/agents/run/review-code-edits), [Devin](https://docs.devin.ai/desktop/changelog)
- Provenance: GitHub Agent-Logs-Url — [GitHub changelog](https://github.blog/changelog/2026-03-20-trace-any-copilot-coding-agent-commit-to-its-session-logs)

### Inferences
- Agent-adjacent extension ideas consistent with "observe, never decide" (derived from features above, not from a user survey): AI pre-reviewer that annotates hunks (Cursor Agent Review / Air style); secret/PII scanner flagging hunks with credentials; test-impact runner attaching results to a turn; cost/usage dashboard from ACP usage updates; session audit exporter / commit-trailer provenance; plan-vs-diff checker; MCP-Apps renderer; policy linter for `.fyx/review.toml`; notification bridges (Slack/Telegram — many ACP messaging clients exist per [ACP clients](https://agentclientprotocol.com/get-started/clients.md)).
- Biggest strategic risk for Fyx found in this research: ACP v2 dropping `fs/write_text_file` (draft). Biggest opportunity: no editor currently reviews Terminal-Thread/CLI agents' edits (Zed Terminal Threads, VS Code working set excludes terminal writes) — a snapshot-based gate that covers any agent would be unique.

### Gaps
- No direct user survey on "which agent-adjacent extensions users want"; the extension list above is inference from product features, not demand data.
