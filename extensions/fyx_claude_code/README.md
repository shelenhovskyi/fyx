# Claude Code (`fyx-claude-code`)

Claude Code in Fyx's terminal, connected to the editor (plan 5.15,
[design, "The first extensions"](../../docs/fyx/wit-0.4/design.md)). It targets the staged
`fyx:ui` 0.4.

- **Profile.** **Claude Code — Claude Code** in the terminal panel's **+** menu runs
  `claude --ide` in the active worktree. You pick the account (Claude config dir) for each
  terminal (`account_tool = "claude"`). The terminal is yours: the extension cannot read it or
  type into it.
- **Env** (profile terminals only):
  - `CLAUDE_CODE_SSE_PORT`: the port of Fyx's IDE server;
  - `ENABLE_IDE_INTEGRATION=true`;
  - `CLAUDE_CODE_PLUGIN_DIRS=${package}/claude-plugin`, the Claude plugin shipped in this
    package.
- **IDE server.** Fyx listens on `127.0.0.1` (`mcp-websocket`, header
  `x-claude-code-ide-authorization`) and writes `<account>/ide/<port>.lock` for Claude to find it.
  The extension offers these tools:
  - `openFile`;
  - `openDiff`: a blocking proposal in agent review, hunk by hunk. Claude writes the final text;
  - `close_tab` and `closeAllDiffTabs`;
  - `getDiagnostics`.

  It sends `selection_changed` and `at_mentioned` to Claude.
- **Hooks.** The plugin (`claude-plugin/hooks/hooks.json`) reports these to the extension
  through `fyx-relay`, and the extension sets the tab's status:
  - `SessionStart` and `SessionEnd`;
  - `UserPromptSubmit`, which begins an agent turn;
  - `PreToolUse` and `PostToolUse` (the tool, and the files it wrote);
  - `Notification` (Claude waits for you);
  - `Stop`, which ends the turn with the files it changed.

  Every hook exits 0 at once, so Claude never waits on Fyx. Outside Fyx `FYX_RELAY` is unset and
  each hook runs `true`. An interactive Claude runs the plugin's hooks only after you trust the
  folder in Claude.

Capabilities: these T2 capabilities are listed at install:
- `terminal:env`;
- `network:listen`;
- `fs:discovery-file`;
- `relay:hooks`.

It also uses `ui:panel`, `terminal:profiles`, `package:files`, `review:propose`,
`context:editor` and `workspace:open`.

## Status

This is the skeleton with no host dependency:
- the manifest, which passes `fyx-ext check`;
- the plugin;
- the protocol core in plain Rust (`src/ide.rs`, `src/hooks.rs`), tested natively;
- the compat test ([`src/compat/README.md`](src/compat/README.md)), pinned to Claude Code
  2.1.326.

The core is wired to the host's `mcp-call`, `relay-request`, `review-decided` and
`context-changed` events once the SDK has them (plan 5.14).

## Build and test

```sh
cargo test -p fyx_claude_code
```

```sh
script/fyx-build-demos --fyx-ui-0-4
```

The first runs the protocol core and the compat transcripts. The second builds the extension
with the 0.4 demos into `<target>/fyx-demos-v0_4/fyx_claude_code/`, together with
`claude-plugin/`. `fyx_ext_harness` loads that package and starts its profile with an account
(`tests/demos_v0_4.rs`).
