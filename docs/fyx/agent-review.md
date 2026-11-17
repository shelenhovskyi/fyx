# Agent review

With review mode on, Fyx holds an agent's file edits until you have accepted or rejected them hunk by
hunk. Fyx decides what lands. The agent only proposes, and no extension can decide for you.

## Turning it on

- Setting: `"agent_review": { "enabled": true }` in your user settings. You can also use the agent
  panel's options menu (**Review Agent Edits**) or the command `agent_review: toggle review mode`.
- The `FYX_AGENT_REVIEW` environment variable overrides the setting: `0` turns review off, any other
  value turns it on.
- Only your user settings can turn review off. A project can only make it stricter, with
  [review rules](review-rules.md).
- Turning review off does not release edits that are already waiting. They still need a decision.

While review mode is on, the status bar shows **Review on**. When reviews are waiting, it shows
their count; click it to bring the waiting reviews forward.

## What is reviewed

| Path | Reviewed? |
|---|---|
| ACP agents that ask permission with a diff (v1 `session/request_permission`, e.g. Claude Agent) | Yes: the diff opens for review, and your decisions answer the request. |
| ACP v2 agents whose permission request carries a single-file patch | Yes, against the file as it is now. |
| Writes through Fyx (`fs/write_text_file`) | Yes: the write waits until you submit, and only accepted hunks land. |
| ACP v2 edits Fyx cannot show as a diff: several files in one request, no patch text, or a patch that does not apply | No. A notification says so; decide in the agent panel. |
| Shell and terminal commands (`sed -i`, `cat >`, scripts) | No. Files that change on disk during the agent's turn without going through review are listed in the turn's review pane as "changed without review". Nothing is blocked. |
| Zed's built-in agent | No. Its edits land as in Zed. |

The same summary is in the setting's description and under every review pane.

## Reviewing

- One review pane per agent turn. It lists the turn's files and shows one diff at a time.
- Each hunk has its own Accept and Reject buttons.
- Keys while the pane has focus:

  | Keys | Action |
  |---|---|
  | `alt-y` / `alt-n` | Accept / reject the hunk |
  | `shift-alt-y` / `shift-alt-n` | Accept / reject the whole file |
  | `ctrl-shift-y` / `ctrl-shift-n` | Accept / reject everything in the turn, including later edits of the turn |
  | `alt-j` / `alt-k` | Next / previous hunk |
  | `alt-c` | Comment on the hunk |
  | `ctrl-enter` | Submit |

- Comments, and the hunks you rejected, go back to the agent when it wrote through Fyx: the write
  fails with a message that lists them. An ACP permission answer carries no message.
- If Fyx cannot write the hunks you accepted (for example, the text moved in the meantime), the
  review stays open with the reason. Submit again, or reject the file.
- If the file has unsaved edits of yours, Fyx applies the accepted hunks but does not save the file.
- When the agent's turn stops, or its connection drops, its waiting edits are cancelled: the agent
  is told the file is unchanged. The same happens when you close the last pane of a turn.

## See also

- [Review rules](review-rules.md): `.fyx/review.toml`.
