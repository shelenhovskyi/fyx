# Review rules

A project can tell Fyx how to treat agent edits to some of its files. The rules live in
`.fyx/review.toml` at the root of a worktree, and Fyx applies them before a review opens. The
policy is decided by Fyx: neither the agent nor an extension can change it.

```toml
# .fyx/review.toml
auto_accept = ["docs/**", "*.md"]          # lands without review
always_ask  = ["Cargo.lock", ".github/**"] # always reviewed
deny        = ["*.env", "secrets/**"]      # never written by an agent
```

All three keys are optional lists of patterns. Any other key is an error.

<!-- crates/agent_review/src/review_rules.rs -->

## What each kind does

| Kind | Agent writes a matching file | Agent asks permission to edit a matching file | Applies with review mode off |
|---|---|---|---|
| `deny` | Nothing is written. The agent's write fails with a message naming the rule and telling it not to retry. | Answered "reject" without a review. | yes |
| `always_ask` | Held for review, even when you chose "Accept all in turn". | A review opens. | yes |
| `auto_accept` | Written without a review. | Answered "allow" without a review. | no |

The `deny` and `always_ask` rules apply even with review mode off, because they only make review stricter.
`auto_accept` only relaxes review, so it applies only when review mode is on.

Each decision is listed in the turn's review pane, with the rule that made it ("by rule
`deny = "*.env"` in …/.fyx/review.toml"). A review opened by `always_ask` says so above the diff.

<!-- crates/agent_review/src/agent_review.rs: review_write, open_permission_preview -->

## Matching

- Rules are checked per file. The first kind that matches wins, in the order `deny`,
  `always_ask`, `auto_accept`. For example, with `auto_accept = ["docs/**"]` and
  `always_ask = ["docs/security.md"]`, `docs/security.md` is always reviewed.
- A pattern without `/` matches the file name at any depth: `*.env` matches `.env` and
  `config/prod.env`, `Cargo.lock` matches every `Cargo.lock`.
- A pattern with `/` matches the path relative to the worktree root, with `/` separators on every
  platform. `*` stays within one directory, `**` crosses directories: `docs/**` matches
  `docs/guide/intro.md`, `.github/*.yml` does not match `.github/workflows/ci.yml`.
- Patterns use [globset](https://docs.rs/globset) syntax (`?`, `[abc]`, `{a,b}`).
- In a project with several worktrees, each worktree's own `.fyx/review.toml` covers its files.
  For nested worktrees, the innermost one wins.

## Trust

`.fyx/review.toml` comes with the repository, so a cloned project could try to wave agent edits
through. To prevent that, `auto_accept` applies only in worktrees you trust: the ones Zed's
restricted mode lets you trust, or every worktree when `session.trust_all_worktrees` is on.
`deny` and `always_ask` always apply.

## Loading and errors

- Fyx reads the file when an agent thread starts in the project.
- It reads the file again whenever the file changes on disk.
- If the file does not parse (bad TOML, an unknown key, an invalid pattern), Fyx ignores the whole
  file and shows a notification with the error. It never applies half a file.
- Remote projects are not supported yet: the file is read through the local file system.

## Limits

- A rejected permission request carries no message in ACP. An agent whose edit is refused through
  a permission request is only told "rejected". When the agent writes through Fyx
  (`fs/write_text_file`), the write fails with the rule in the error message.
- Rules apply to agent edits that reach Fyx: permission requests and `fs/write_text_file`. An
  agent that changes files another way, for example with a shell command, bypasses them. The
  turn's review pane lists such files as "changed without review", but nothing is blocked.
