# W5 Agent change review

**Owns:** `crates/agent_review/`, a thin hook in `crates/acp_thread` / `crates/agent_ui`.

**Deliverable**
- Host model of proposed edits: hunks per file, accept/reject per hunk, comment.
- Diff view built on Zed's existing editor/diff machinery.
- Entry point from ACP `session/request_permission`; policy and consent decided in the host.
- Host owns key routing while a review mode is active.

**Done when:** an ACP agent (e.g. Claude Agent) proposes an edit and you accept one hunk and reject another in the running editor.
