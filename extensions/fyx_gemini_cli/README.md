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

It uses `ui:panel`, `terminal:profiles`, `terminal:env`, `network:listen`,
`fs:discovery-file`, `review:propose` and `context:editor`.

## Status

This is the host-free skeleton:
- the manifest, which passes `fyx-ext check`;
- the companion core (`src/companion.rs`), tested natively;
- the compat test ([`src/compat/README.md`](src/compat/README.md)), pinned to Gemini CLI 0.69.0.

The same README lists the primitives Gemini needs that the kit lacks. One of them, typed
discovery content, blocks IDE mode until the host can write it. The wiring to the host's events
comes with plan 5.14.

That README also gives the plan the coordinator decided for hooks and turns, which come after
the skeleton. The hooks go in a settings file in the package, loaded through
`GEMINI_CLI_SYSTEM_DEFAULTS_PATH` in profile terminals only.

```sh
cargo test -p fyx_gemini_cli
```

```sh
script/fyx-build-demos --fyx-ui-0-4
```
