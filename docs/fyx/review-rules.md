# Review rules

Review rules say how Fyx treats agent edits to some files before a review opens. Fyx decides the
policy: neither the agent nor an extension can change it.

There are two places for rules, and they can do different things:

| Where | Can set | Why |
|---|---|---|
| `.fyx/review.toml` at the root of a worktree | `always_ask`, `deny`: review only gets stricter | The file comes with the repository, and an agent can write it (through its terminal, or a write you accepted without noticing what it changes). |
| Your user settings, `agent_review.auto_accept` | `auto_accept`: edits that land without review | Only you can loosen review. |

```toml
# .fyx/review.toml
always_ask = ["Cargo.lock", ".github/**"] # always reviewed
deny       = ["*.env", "secrets/**"]      # never written by an agent
```

```json
// settings.json
"agent_review": {
  "enabled": true,
  "auto_accept": ["docs/**", "*.md"]
}
```

`auto_accept` in a project file is ignored, and a notification says so. Any other unknown key in
the file is an error.

<!-- crates/agent_review/src/review_rules.rs -->

## What each kind does

| Kind | Agent writes a matching file | Agent asks permission to edit a matching file | Applies with review mode off |
|---|---|---|---|
| `deny` | Nothing is written. The agent's write fails with a message naming the rule and telling it not to retry. | Answered "reject" without a review. | yes |
| `always_ask` | Held for review, even when you chose "Accept all in turn". | A review opens. | yes |
| `auto_accept` (user settings) | Written without a review. | Answered "allow" without a review. | no |

The `deny` and `always_ask` rules apply even with review mode off, because they only make review stricter.

Each decision is listed in the turn's review pane with the rule that made it, for example
"by rule `deny = "*.env"` in …/.fyx/review.toml". A review opened by `always_ask` says so above
the diff.

<!-- crates/agent_review/src/agent_review.rs: review_write, open_permission_preview -->

## Matching

- Rules are checked per file. The first kind that matches wins, in the order `deny`,
  `always_ask`, `auto_accept`.
  - Example: with `auto_accept = ["docs/**"]` in your settings and
    `always_ask = ["docs/security.md"]` in the project, `docs/security.md` is always reviewed.
- A pattern without `/` matches the file name at any depth: `*.env` matches `.env` and
  `config/prod.env`, `Cargo.lock` matches every `Cargo.lock`.
- A pattern with `/` matches the path relative to the worktree root, with `/` separators on every
  platform.
  - `*` stays within one directory and `**` crosses directories: `docs/**` matches
    `docs/guide/intro.md`, but `.github/*.yml` does not match `.github/workflows/ci.yml`.
  - `auto_accept` patterns from your settings are relative to the root of the worktree that holds
    the file.
- Patterns use [globset](https://docs.rs/globset) syntax (`?`, `[abc]`, `{a,b}`).
- Fyx matches the file the edit really goes to: the opened file's path, with `.` and `..`
  resolved. On Windows and macOS the comparison ignores case, as those file systems do, so
  `Config/PROD.ENV` matches `*.env`.
- In a project with several worktrees, each worktree's own `.fyx/review.toml` covers its files.
  For nested worktrees, the innermost one wins.

## Loading and changes

- Fyx reads the file when an agent thread starts in the project, and whenever the file changes on
  disk.
- A change that only adds restrictions applies at once.
- A change that removes any restriction waits until you confirm it. Removing a `deny` pattern,
  dropping an `always_ask` pattern, or deleting the file all count.
  - A notification lists what would be removed and offers **Apply new review rules**. Until you
    click it, the previous rules stay in force.
  - This also holds when a new thread starts, including a subagent the agent opens itself: an
    agent cannot get looser rules applied by starting a thread.
- A file that cannot be read or parsed (bad TOML, an unknown key, an invalid pattern) keeps the
  previous rules in force. A notification shows the error. Fyx never applies half a file and never
  drops rules silently.
- Fyx remembers, per worktree, the rules you last agreed to (in `agent_review/confirmed_rules.json`
  in its data directory). Until you agree to a change, every remembered restriction stays in force
  together with anything stricter in the file, across restarts too: an agent that deletes a `deny`
  and waits for the next start gets nothing.
- The first time Fyx sees a worktree, its file applies as it is, since a project file can only add
  restrictions.
- Remote projects are not supported yet: the file is read through the local file system.

## Limits

- A rejected permission request carries no message in ACP. An agent whose edit is refused through
  a permission request is only told "rejected". When the agent writes through Fyx
  (`fs/write_text_file`, or Zed's built-in agent), the write fails with the rule in the error
  message.
- Rules apply to agent edits that reach Fyx: permission requests, `fs/write_text_file`, and the
  edit, write, delete, move and copy tools of Zed's built-in agent. For a move or copy, the
  strictest rule of its two paths decides: copying a denied file elsewhere is denied too.
  - An agent that changes files another way, for example with a shell command, bypasses them.
  - The turn's review pane lists such files as "changed without review", including a rewritten
    `.fyx/review.toml`. Nothing is blocked.
