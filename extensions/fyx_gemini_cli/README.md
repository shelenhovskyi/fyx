# Gemini CLI (`fyx-gemini-cli`)

Gemini CLI in Fyx's terminal, with Fyx as its IDE companion (plan 5.16,
[design, "The first extensions"](../../docs/fyx/wit-0.4/design.md)). It follows Gemini CLI's
documented companion spec, targets the staged `fyx:ui` 0.4, and is the second tool on the 0.4
integration kit.

- **Profile.** **Gemini CLI — Gemini CLI** in the terminal panel's **+** menu runs `gemini` in the
  active worktree, with the account (`GEMINI_CLI_HOME`) you pick. Run `/ide enable` in it once.
- **Companion server.** Fyx listens on `127.0.0.1` (MCP over streamable HTTP, Bearer token),
  writes `gemini-ide-server-<pid>-<port>.json` into the temp folder, and sets
  `GEMINI_CLI_IDE_SERVER_PORT` in the profile's terminal.
- **Diffs.** `openDiff` opens a proposal in agent review and answers at once. Your decision,
  with your edits, goes back as `ide/diffAccepted` or `ide/diffRejected`. If you answer in the
  terminal instead, the CLI calls `closeDiff`.
- **Context.** `ide/contextUpdate` sends Gemini your open files (newest first), the active
  file's cursor and your selection.
- **Turns.** The package's `gemini-settings/system-defaults.json`, loaded through
  `GEMINI_CLI_SYSTEM_DEFAULTS_PATH` in the profile's terminal, runs Gemini's hooks through
  `fyx-relay`. Each prompt is an agent turn with the files Gemini wrote, and the terminal's tab
  shows the tool Gemini runs and when it waits for you. Gemini runs these hooks in trusted
  folders only.

It uses `ui:panel`, `terminal:profiles`, `terminal:env`, `network:listen`,
`fs:discovery-file`, `review:propose`, `context:editor`, `package:files`, `relay:hooks` and
`terminal:status`.

## Status

Wired on the 0.4 SDK (`src/wired.rs`): `mcp-call`, `review-decided` and `context-changed` drive
the companion (`src/companion.rs`), `relay-request` drives the hooks (`src/hooks.rs`), and the
discovery template writes the shape the CLI reads.
The compat test ([`src/compat/README.md`](src/compat/README.md)), pinned to Gemini CLI 0.69.0,
replays natively and through the built component in `fyx_ext_harness`. The same README lists
what the kit still lacks for Gemini.

That README also says how the hooks work and what limits them (trusted folders, env
redaction).

```sh
cargo test -p fyx_gemini_cli
```

```sh
script/fyx-build-demos --fyx-ui-0-4
```
