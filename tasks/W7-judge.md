# W7 Judge / reviewer / docs (does not compile)

**Owns:** `docs/` (except `docs/research/`, frozen, and `docs/vscode-coverage.md`, W8's), `tasks/` (except `tasks/progress/<stream>.md` and `tasks/wit-0.2.md`), `PLAN.md` status and backlog, reviews in `tasks/reviews/`.

**Deliverable**
- Each cycle: check every merged change against the WIT and ARCHITECTURE.md rules (no WASM in `render()`, theme tokens only, `fyx:` markers on upstream edits).
- Find duplication between streams; flag agents that drift so they get a fresh restart.
- Keep `PLAN.md` status current; write manual test scripts for phase 4.
- Measure the upstream diff: `git diff $(cat FYX_UPSTREAM) --stat -- . ':!crates/ui_ext*' ':!crates/agent_review' ':!docs' ':!tasks'`.
