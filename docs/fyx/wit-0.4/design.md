# `fyx:ui@0.4.0`: terminals and CLI integrations

Status (2026-11-19): **design, for implementation** (platform plan P5). The direction and the rule
changes in [§C.7](#c7-rules-that-change-decided-by-the-owner-2026-11-19) were decided by the owner
on 2026-11-19. The open technical questions were answered by the coordinator the same day
([Decisions](#decisions-2026-11-19)). Nothing is implemented yet. The WIT below is a diff for W0,
who stages it in `crates/ui_ext/wit/since_v0.4.0/fyx-ui.wit` (integrator-only).

**Source.** The owner's research, in Ukrainian: [`terminal-api/research.md`](../terminal-api/research.md)
(branch `fyx/terminal-api-research`, f21c198c92). That file has the survey (what the terminal code
does today, how Claude Code, Gemini CLI, Codex, Copilot, Amp, OpenCode and Aider integrate, and
how VS Code, iTerm2, WezTerm, kitty, Zellij and JetBrains do it). This file turns its layers B and
C, its §7 (multiple accounts) and its §6 (order of work) into a design. Layer A, the terminal
foundation, is summarised in [§A](#a-the-terminal-foundation-layer-a-host-only) as the host work
that B and C build on.

**The owner's requirements.**
1. An extension can be a full terminal of its own.
2. Anyone can build a full integration for any CLI tool, not only Claude Code.
3. The user accepts powerful capabilities by hand, at install.

**Principles.**
- **Nothing tool-specific in Fyx's core.** The host offers generic primitives. Claude Code,
  Gemini CLI and the rest are extensions. The first two, `fyx_claude_code` and `fyx_gemini_cli`,
  are ours, and the second one proves the primitives are general. Tool names appear in the core
  only as data: setting presets and the forbidden-name lists of [§C.6](#c6-multiple-accounts).
- **Fyx always draws the terminal.** An extension decides what a terminal is: the program, a
  byte-stream backend, or a panel around it. The grid, input, IME, selection, search and links are
  Fyx's native terminal view. There is no terminal emulator in WASM (research §2: canvas has no
  keyboard, no ANSI colours, no cell metrics, and a 200 ms call budget).
- **No global byte stream.** No extension sees all output of all terminals. There is no raw VT
  where structured data does, no write into the user's shell without a gesture, and no direct
  write by an extension into `~`.

**This world and the other 0.4 work.** 0.4 also carries plan 4.2–4.4 (decoration extras, the
picker, code lens, hover, virtual documents, commands), designed in
[wit-0.3/design.md §1–§6](../wit-0.3/design.md#1-decoration-extras-plan-42). They join the same
staged world and are not repeated here. If the 1.0 cut (owner Q9: the frozen 0.3 becomes
`fyx:ui@1.0.0`) comes before this world freezes, it is renamed `1.1.0` when it is frozen. Its
content does not change.

## Summary

| § | What | Plan | Capabilities, tier |
|---|---|---|---|
| A | Terminal foundation: ids, the OSC tap with OSC 7501, tab status, agent sessions list, pseudo-terminal plumbing, the `terminal` node, output sanitising | 5.1–5.4 | none (host) |
| B.1 | Terminal profiles in the "+" menu | 5.8 | `terminal:profiles` T1 |
| B.2 | Pseudo-terminals and the `terminal` node | 5.8 | `terminal:pseudo` T1 |
| B.3 | Created terminals: declared programs in a pty | 5.8 | `terminal:create` T2 |
| B.4 | Environment for terminals | 5.8 | `terminal:env` T2 (all new terminals: a second, stronger checkbox) |
| B.5 | Status on terminals, observing terminals | 5.3, 5.8 | own: none; others' status `terminal:status` T1; `terminal:observe` T2 |
| B.6 | Package files | 5.8 | `package:files` T0 |
| B.7 | Loopback listeners with a token, and the MCP wrapper | 5.10 | `network:listen` T2 |
| B.8 | Discovery files | 5.11 | `fs:discovery-file` T2 |
| B.9 | `fyx-relay` for command hooks | 5.11 | `relay:hooks` T2 |
| B.10 | Proposed edits through agent review | 5.12 | `review:propose` T1 |
| B.11 | Editor context, opening files, pasting into a tool | 5.13 | `context:editor` T1, `workspace:open` T1, `workspace:diagnostics` T1 |
| B.12 | Approval cards for a tool's own permission prompts | 5.13 | `ui:approvals` T0 |
| C.1–C.5 | Consent: T2 accepted at install with checkboxes, T3 at the moment of action, journal, update diff | 5.5–5.7 | `terminal:read` T3, `terminal:write` T3 |
| C.6 | Multiple accounts: forbidden auth and config-dir names, account profiles, Fyx profiles (variant A) | 5.5, 5.20 | none |
| D | Extension settings: typed `[[fyx_ui.settings]]`, a host-drawn form, values read and watched | 6.1 | none |

The research's names map to these: `net:listen` is `network:listen` (next to `network:fetch`),
"relay" is `relay:hooks`, "context" is `context:editor`, "package files" is `package:files`.

## Decisions (2026-11-19)

The owner decided the direction and the rule changes of §C.7. The coordinator answered the
research's open questions:

| Research question | Decision |
|---|---|
| 2. Undocumented IDE protocols (Claude, Copilot, Amp, Codex), or only documented ones? | **Documented protocols and OSC 7501 first** (Gemini CLI's IDE companion spec, Qwen's, OSC 7501, hooks). Claude Code's IDE protocol comes in phase 4 (plan 5.15) with a **compat test** pinned to a Claude Code version that fails loudly when the protocol changes. |
| 3. Environment in every terminal, or in profile terminals only? | **Profile terminals by default**, with an "Also all new terminals" checkbox at install (§B.4). |
| 4, 5. SSH terminals; PowerShell shell integration | **Phase 6** (plan 5.17). |
| 5. Windows: named pipes or loopback? | **Loopback TCP on every OS.** No named pipes and no Unix sockets for listeners. |
| 1. Do hooks from `CLAUDE_CODE_PLUGIN_DIRS` wait for Claude's trust dialog? | Open; checked in plan 5.15 before the Claude extension relies on them. |
| 6. Copilot and Amp lock schemas, Codex `/ide`, OSC 7501 adoption | Open; checked from source when an extension for that tool starts. None blocks the primitives. |

## A. The terminal foundation (layer A, host only)

No extension API. Owner W1, plan 5.1–5.4. B and C depend on it.

1. **`TerminalId` and a registry.** Every terminal gets a `TerminalId`. A Fyx crate (new
   `terminal_ext`) observes `Terminal` and `TerminalView` creation with `cx.observe_new` and
   release with `on_release`, so `Project` needs no new event. Every local terminal gets
   `FYX_TERMINAL_ID` (not secret) and `FYX_TERMINAL_TOKEN` (128 random bits, the terminal's
   secret, used by `fyx-relay`, §B.9) through one `fyx:` hook in `insert_zed_terminal_env`
   (`crates/terminal/src/terminal.rs:656`). The same hook adds the env of §B.4.
2. **The OSC tap.** A `TeePty` in `terminal_ext` implements alacritty's `EventedPty`. It delegates
   `register` and `writer` to the real pty, and its reader passes bytes through a small OSC
   scanner (it looks only for `ESC ]` … `BEL`/`ST`). The bytes reach alacritty unchanged. One line
   in `spawn_event_loop` (`crates/terminal/src/alacritty.rs:214`) swaps it in; the alacritty fork
   is not touched. W1 prototypes this first. If it fails, a patch to the fork needs the owner
   (the minimal-hooks rule).
   - **Parsed:** OSC 7 (cwd), 9, 777 and 99 (notifications), `9;4` (progress), 133 and 633
     (command blocks, phase 6), 1337 `SetUserVar`, and **7501** (program status).
   - **OSC 7501** (Program Status, rev 0.4): `ESC ] 7501 ; key=value:… ST`. The keys are `state`
     (`idle`, `working`, `blocked`, `done`, `error`), `progress`, `kind` (`permission`,
     `question`, `auth`), `app`, `id`, `title` and `msg` (base64). The tap answers the support
     query `OSC 7501 ; ?` through the existing `PtySender`. **The reply's exact form is taken from
     the spec before 5.2 starts.** A reply is never built from output data.
   - **Limits, on the IO thread:** an OSC longer than 8 KiB is dropped; at most 50 parsed OSCs
     per second per terminal, and the rest are counted and dropped.
3. **Tab status.** `TerminalStatus { state, label, detail, progress, attention, source }`, kept by
   a global `TerminalDecorations` provider in `terminal_ext`. `tab_content`
   (`crates/terminal_view/src/terminal_view.rs:1612`) only asks whether the terminal has a status
   and draws the icon and badge (about 15 lines). **The source is always drawn by the host**
   ("from Claude Code via OSC 7501", "from extension X"), so an extension cannot pass itself off as
   the tool.
4. **Agent sessions list.** A Fyx panel lists every terminal with a status, "needs attention"
   first; a click focuses the terminal. Fyx notifies on `blocked` and `done` while the window is
   not focused. Owner W3 (5.4).
5. **Pseudo-terminal plumbing.** Built on the existing display-only terminals
   (`terminal.rs:984-1093`, `write_raw_output`). Input and resize come back through the
   display-only input path.
6. **The `terminal` node.** The existing `TerminalView` (`terminal_view.rs:243`) embedded in an
   extension panel by the `ui_ext_host` panel container.
7. **Sanitising** of pseudo-terminal output (§B.2).

After phase 1 alone, Claude Code 2.1.295+ (and Codex once it ships OSC 7501) shows its state on
the tab with no extension and no setting.

**Upstream diff for all of P5:** about 25 lines in three files (`alacritty.rs:214`,
`terminal.rs:656`, `terminal_view.rs:1612`), plus one hook in `crates/agent_servers/src/acp.rs`
for account profiles (§C.6). Each is marked `fyx:` and listed in
`tasks/reviews/upstream-diff-audit-1.md`. The parts useful to every Zed user (OSC 7, OSC 9/777,
OSC 133, tab status) may go upstream as PRs, which only the owner opens.

## B. The integration kit (layer B)

Every primitive is generic. A tool's specifics live in the extension's manifest and code.

### B.1 Terminal profiles (`terminal:profiles`, T1)

A profile is a recipe in the manifest. It shows in the terminal panel's "+" menu as
"<profile name> — <extension name>", and the user starts it.

```toml
[[fyx_ui.terminal_profiles]]
id = "claude"                         # unique in the extension, [a-z0-9-]{1,32}
name = "Claude Code"
icon = "ai-claude"                    # a frozen icon name; extension SVGs are not host chrome
program = "claude"                    # resolved like [[fyx_ui.processes]] (0.3 design §9)
args = ["--ide"]                      # literal; may contain ${package}
cwd = "worktree"                      # "worktree" (the active one) or "home"
account_tool = "claude"               # optional: the user picks an account profile (§C.6)
```

- **Resolution** is 0.3 design §9's: `PATH` outside the worktrees, or a worktree `path` only in a
  trusted worktree. No shell is started unless the profile names one.
- **The menu entry shows the full command line** on hover. A program in `RUNS_ANY_CODE` (shells,
  interpreters) is marked "runs any command you type".
- **A profile terminal belongs to the user.** The extension gets its env (§B.4, if granted), may
  set its status (§B.5) and may paste into it (§B.11). Reading or writing it is T3 (§C.3).
- **Limits:** 16 profiles per extension; `args` at most 64 items of 1 KiB.

### B.2 Pseudo-terminals and the `terminal` node (`terminal:pseudo`, T1)

A pseudo-terminal has no process. The extension is the backend: SSH, a serial port, tmux control
mode, a container, or a TUI written in WASM. It writes output bytes and gets input bytes and
resizes.

- **`terminals.open-pseudo(request)`** opens one as a tab in the terminal panel, or inside the
  extension's panel when a `terminal` node names it (below). It returns a `terminal-handle`.
- **`terminals.write-output(handle, bytes)`**: at most 64 KiB per call and 4 MiB/s per terminal
  (the host queues it and feeds alacritty in chunks, so a flood never runs inside one call).
  Above the rate, the call fails with `rate-limited`.
- **Input:** `io-event::terminal-input(handle, bytes)`, coalesced to at most 30 events per second
  and 64 KiB each. **Resize:** `io-event::terminal-resized(handle, cols, rows)`, the last one only.
- **Sanitising.** The host removes these from pseudo-terminal output before alacritty sees it:
  OSC 52 (clipboard), OSC 8 (hyperlinks), OSC 1337, OSC 7501, and every sequence the terminal
  answers (DA, DSR/CPR `CSI 6n`, DECRQSS, XTGETTCAP, OSC colour queries). The terminal never
  writes a reply to the extension's input built from its output (the CVE class of research §4).
- **The `terminal` node** (`node-kind::terminal(terminal-props { handle })`, in `types`, marked
  "0.4") draws the native terminal view inside an extension panel, so an extension can build
  tabs, blocks or side panels around a real terminal (requirement 1). One node per handle; a
  handle shown in a node is not also a tab. A node may also show a created terminal (§B.3) of the
  same extension, never someone else's.
- **Tab marker:** "<extension name>" in the tab's source line.

### B.3 Created terminals (`terminal:create`, T2)

`terminals.create(request)` runs a **declared** program in a real pty, as the extension's own
terminal. The extension reads its output and writes its input.

```toml
[[fyx_ui.terminal_programs]]
name = "codex"
args = ["-c", "notify=*", "**"]       # argument patterns, as 0.3 design §9 (Q2)
```

- **The program list, resolution, argument patterns and `RUNS_ANY_CODE`** are those of
  `[[fyx_ui.processes]]` (0.3 design §9). Unavailable in an untrusted worktree
  (`untrusted-worktree`).
- **Environment:** the user's normal terminal env, plus `FYX_TERMINAL_ID` and
  `FYX_TERMINAL_TOKEN`, plus the extension's `terminal:env` names if granted.
- **Output:** `io-event::terminal-output(handle, bytes)`, raw (it is the extension's own program),
  in chunks of at most 64 KiB, at most 30 per second; the rest is buffered up to 1 MiB, then the
  oldest output is dropped and the next event says how much.
- **Input:** `terminals.send-input(handle, bytes)`, at most 64 KiB per call.
- **Exit:** `io-event::terminal-exited(handle, option<s32>)`.
- **Tab marker:** "started by <extension name>", always visible.
- **Lifetime:** killed with its process tree (0.3 design §9's groups and job objects) on reload,
  disable, uninstall and revocation. At most 8 per extension.

### B.4 Environment for terminals (`terminal:env`, T2)

The extension declares names and values to add to terminal environments.

```toml
[fyx_ui.terminal_env]
vars = { CLAUDE_CODE_SSE_PORT = "${listen.ide.port}", ENABLE_IDE_INTEGRATION = "true",
         CLAUDE_CODE_PLUGIN_DIRS = "${package}/claude-plugin" }
scope = "profiles"                    # "profiles" (default) or "all-new"
```

- **Scope.** By default the vars go into the extension's own terminals: profiles (§B.1), created
  terminals (§B.3). `scope = "all-new"` asks for every new terminal too. That is a second, stronger
  checkbox at install ("Also add these to every new terminal you open"), unticked by default
  ([Decisions](#decisions-2026-11-19)). If the user leaves it unticked, the vars go into profile
  terminals only. Terminals already open are not changed; once the scope widens, their tab offers
  "Restart to apply".
- **Values** are literals, or templates with `${package}` (§B.6), `${listen.<name>.port}` (§B.7),
  `${relay}` (§B.9) and `${account.<tool>}` (resolved by the host to the terminal's account
  profile dir, §C.6; the extension never sees the path). `${listen.<name>.token}` is allowed only
  for a listener that declares `token_in_env = true`. The extension never sees the token itself.
  `${pid}` is the process id of Fyx (the window's process). Gemini CLI looks for its discovery
  file by the IDE's pid, and in Fyx the shell is Fyx's direct child, so the CLI's own heuristic
  (the shell's grandparent, or the third process from the root on Windows) does not find Fyx;
  `GEMINI_CLI_IDE_PID = "${pid}"` does. (`${pid}` was a discovery-file placeholder only, §B.8.)
  Values have at most 4 KiB, no NUL and no newline. At most 32 names.
- **Names** match `[A-Za-z_][A-Za-z0-9_]*`. **Refused at load**, in any letter case:
  - every name `ui_ext::process::is_forbidden_env` refuses (code loaders and redirections:
    `LD_*`, `DYLD_*`, `NODE_OPTIONS`, `PYTHONPATH`, `BASH_ENV`, `ENV`, `PROMPT_COMMAND`, `GIT_*`
    editors and SSH, `RUSTC_WRAPPER`, …);
  - `PATH`, `PATHEXT`, `COMSPEC`, `HOME`, `USERPROFILE`, `XDG_*`, `ZDOTDIR`, `SHELL`, `TERM*`,
    `FYX_*` and `ZED_*`: names that move where a shell finds programs and its configuration, and
    the host's own names (integration-6 N1);
  - the auth and config-dir names of §C.6. A `*_TOKEN` name is allowed only when its value is
    exactly `${listen.<name>.token}` of one `token_in_env` listener (§C.6, rule 5).

  This is stricter than the research, which would allow `PATH` and the loaders with a warning:
  injecting them into the user's interactive shell is the strongest form of N1's attack.
  `[fyx_ui.process].env` stays the 0.3 allowlist. Tools' own names (`CLAUDE_CODE_*`,
  `GEMINI_CLI_IDE_*`, `OPENCODE_*` and so on) have no end, so here the list refuses names
  instead of allowing them. The registry's agent review flags a name it does not recognise.
- **The install checkbox lists every name and value.** A value that points into the package is
  shown as "files from this extension". A name the host knows makes the tool load code (a preset
  list of data, such as `CLAUDE_CODE_PLUGIN_DIRS`) is marked "makes <tool> load this extension's
  plugins".

### B.5 Status on terminals and observing them

- **`terminals.set-status(handle, option<terminal-status>)`.** On the extension's own terminals
  (pseudo, created, its profiles), no capability is needed. On any other terminal it needs
  `terminal:status` (T1), and the handle comes from `terminal:observe` or from a relay request
  (§B.9) that names the terminal. The host draws the source next to it, always. At most 4
  updates per second per terminal. A status from OSC 7501 and one from an extension can both
  exist; the tab shows the one that needs attention, then the newest.
- **`terminal:observe` (T2):** `terminals.list()` and the events `terminal-created`,
  `terminal-closed` and `terminal-status-changed`, with title, cwd, origin, status and the account
  profile's **name** (never its path). **Command lines are redacted** (`•••`) without a T3 read
  (§C.3), as Zellij does. Handles are opaque and different for each extension.

### B.6 Package files (`package:files`, T0)

The extension's installed folder, read-only, as a path the host substitutes for `${package}` in
profile `args` (§B.1) and in env values (§B.4). The extension cannot write there. The path is the
version's own folder, so an update changes it; open terminals keep the old value and show
"Restart to apply". The `${package}` template is the whole capability: the extension itself never
reads the path, and on its own `package:files` grants nothing beyond what §B.1 and §B.4 show.
**What a `.fyxext` contains:** besides `extension.toml`, the component and `assets/`, `fyx-ext
package` packs the top-level folders that a `${package}/<path>` in the manifest names (a plugin
folder such as `claude-plugin/`), and nothing else, so a folder referenced from profile args or
env values reaches the user's machine and a stray one does not.

### B.7 Loopback listeners and the MCP wrapper (`network:listen`, T2)

The host listens; the extension handles messages.

```toml
[[fyx_ui.listeners]]
name = "ide"
protocol = "mcp-websocket"            # "mcp-websocket", "mcp-streamable-http", "websocket", "http"
auth = { header = "x-claude-code-ide-authorization" }   # or { bearer = true } or { query = "auth" }
token_in_env = false
```

- **Bind:** `127.0.0.1:0` on every OS ([Decisions](#decisions-2026-11-19)). No IPv6 wildcard, no
  LAN, no named pipe. At most 4 listeners per extension.
- **Token:** at least 128 random bits, generated by the host per listener and per run, compared
  in constant time. It travels only in a discovery file (§B.8) or, with `token_in_env`, in
  terminal env. The extension never sees it.
- **Refused before the extension sees anything:** a wrong or missing token; any `Origin` header
  (a browser page cannot reach the listener); a `Host` that is not `127.0.0.1:<port>` or
  `localhost:<port>` (DNS rebinding). Also refused: more than 16 open connections, messages over
  4 MiB, and more than 100 messages per second per connection.
- **`mcp-*`:** the host speaks JSON-RPC 2.0 and MCP: `initialize`, `tools/list` and `tools/call`.
  The extension registers tools with `mcp.register-tools(listener, list<tool-def>)` (name,
  description, JSON schema as text) and answers `io-event::mcp-call(call)` with
  `mcp.respond(call-id, result<json, mcp-error>)`. It sends notifications with
  `mcp.notify(listener, method, params)`, to every connection or to one. A call may stay open
  until the extension answers (needed for a blocking `openDiff`). It is cancelled when its
  connection closes (`io-event::mcp-cancelled`).
- **Handshake.** The host answers `initialize` itself, so nothing else marks a session ready. After
  the client's `initialize` and `notifications/initialized` it delivers
  `io-event::mcp-initialized(mcp-session)` once per connection: the listener, the connection id and
  the client's `clientInfo` name and version (each cut to 256 bytes, empty when the client sent
  none). The extension pushes its first context then, with
  `listeners.notify(listener, some(connection), …)`.
- **`mcp-streamable-http`** serves the path `/mcp` (Gemini CLI posts there).
- **Known limitation:** a tool inside a Docker sandbox reaches the host as
  `host.docker.internal`, which the `Host` check refuses. That stays refused: the check is the
  protection against DNS rebinding, and there is no safe way to relax it for one name.
- **`websocket` and `http`:** raw frames or requests as `io-event::listener-message` with a
  connection id, answered with `listener.send` / `listener.respond`. Text only (UTF-8 JSON in
  practice), at most 4 MiB.
- **Lifetime:** listeners open when the extension loads with the capability granted, and close on
  reload, disable, uninstall and revocation. The port is new every run.

### B.8 Discovery files (`fs:discovery-file`, T2)

CLI tools find an IDE through a file at a known path. The host writes it; the extension only
declares it.

```toml
[[fyx_ui.discovery_files]]
listener = "ide"
dir = "${account.claude}/ide"         # or "${tmp}/<name>"; resolved by the host
default_dir = "~/.claude/ide"         # used when no account profile applies
name = "${port}.lock"
content = { pid = "${pid}", workspaceFolders = "${worktrees}", ideName = "Fyx",
            transport = "ws", authToken = "${token}" }
```

- **Placeholders:** `${port}`, `${token}`, `${pid}`, `${worktrees}` (the window's worktree roots),
  `${worktrees.joined}` (the same roots in one string, joined with the platform's path separator,
  `:` or `;`), `${platform.windows}`, `${uid}` (Unix) and `${tmp}`.
- **The content is a JSON document with typed substitution** (W4's `fyx_claude_code` and
  `fyx_gemini_cli`). The manifest's `content` table is the document; the host fills it and writes
  JSON, and a value that is exactly one placeholder takes that placeholder's JSON type:
  `${port}` and `${pid}` are numbers, `${worktrees}` is an array of strings, `${platform.windows}`
  is a boolean, and `${token}` is a string. A placeholder inside a longer string is substituted
  as text, escaped for JSON. A nested table is a literal object, so
  `ideInfo = { name = "fyx", displayName = "Fyx" }` is written as it stands. The extension
  escapes nothing. Gemini CLI's file is `{ port = "${port}", workspacePath = "${worktrees.joined}",
  authToken = "${token}", ideInfo = { name = "fyx", displayName = "Fyx" } }`: a number, a joined
  string and an object, none of which a text-only template can write (without `ideInfo` the CLI
  refuses IDE mode in anything it does not know).
- **Where it may write:**
  - `dir` resolves under `${account.<tool>}` (an account profile dir, §C.6), under `default_dir`
    when that is in the user's home, or under `${tmp}`;
  - the file name must contain `${port}` or `${pid}`, and end in `.lock` or `.json`, so a
    discovery file can never overwrite a tool's settings (`~/.claude/settings.json`);
  - no `..`, no symlink anywhere in the path (checked with `O_NOFOLLOW` or the Windows
    equivalent, on the opened handles);
  - **a folder that exists under `${tmp}` and belongs to another user is refused**, not only a
    symlink: `/tmp` is shared, and another user could pre-create `${tmp}/gemini/ide` and read the
    token. The host creates its own folders `0700` and checks the owner of each existing one;
  - a placeholder inside a folder name is refused. Gemini CLI's spec dir is
    `${tmp}/gemini-ide-${uid}` on POSIX, so its extension uses `${tmp}/gemini/ide`, which the CLI
    reads on every OS (the spec's own dir on Windows; on POSIX the CLI's legacy fallback, which it
    reads without its permission checks). If Gemini drops the fallback, `${uid}` in folder names,
    or a dir per OS, is added before 0.4 freezes.
- **How:** directories `0700` (created if missing), the file `0600`, written to a temporary name
  in the same directory and renamed. On Windows the ACL grants only the user.
- **Cleanup:** removed when the listener closes, on exit, revoke and uninstall; at start, the
  host removes its own stale files (pid no longer running, recorded in
  `<data_dir>/extensions/fyx/discovery.json`). It never removes a file it did not write.
- **Account profiles:** with several account profiles for the tool, one file is written in each
  account dir of the current Fyx profile, so the CLI in any of them finds the right window.
- **Protected paths:** the extension still cannot write `.claude/` and the other protected paths
  ([§C.7](#c7-rules-that-change-decided-by-the-owner-2026-11-19)). `fyx_guard` allows these
  host-written files explicitly, by the declared template.
- **The checkbox says what it means:** "Lets Claude Code (and anything that reads this folder)
  connect to this extension. Claude Code will run what this extension tells it to." A discovery
  file counts as running code in that tool.

### B.9 `fyx-relay` for command hooks (`relay:hooks`, T2)

Most tools' hooks can only run a command (Gemini, Codex, Cursor, Kiro, Aider). Fyx ships a small
binary, `fyx-relay`, next to the `fyx` CLI.

```toml
[[fyx_ui.relay_channels]]
name = "pre-tool"
timeout_ms = 30000                    # at most 600000
on_failure = { exit = 0 }             # when Fyx or the extension does not answer
```

- **Use:** a hook runs `"$FYX_RELAY" --channel <extension-id>/<name> [args…]`. `FYX_RELAY`, the
  binary's absolute path, is in every Fyx terminal; `${relay}` gives it to profile args.
- **What it does:** reads at most 1 MiB of stdin and its argv, and sends them to the running Fyx
  over a host loopback endpoint (the relay endpoint, port in `FYX_RELAY_PORT`), authenticated by
  the terminal's `FYX_TERMINAL_TOKEN`. The host maps the token to the terminal, checks that the
  channel exists and that the extension has `relay:hooks`, and delivers
  `io-event::relay-request(id, channel, terminal: option<terminal-handle>, argv, stdin)`. The
  extension answers `relay.respond(id, exit-code, stdout, stderr)`, and the relay prints them and
  exits with that code.
- **Fails open by default:** no token (a terminal Fyx did not start), Fyx not running, an unknown
  channel, a revoked capability or a timeout all end with the channel's `on_failure` (default:
  exit 0, no output), so a relay hook never blocks the user's tool because Fyx is closed. A
  channel that must fail closed declares `on_failure = { exit = 2 }`, which the install checkbox
  shows.
- **Limits:** 16 channels per extension, 8 requests in flight per extension, answers of at most
  1 MiB.
- **Owner:** W1 builds the binary and the endpoint (`crates/fyx_relay`, a new Fyx crate).

### B.10 Proposed edits through agent review (`review:propose`, T1)

An extension passes a tool's proposed edit to Fyx's agent review, hunk by hunk.

- **`review.propose(request)`** with `path` (a worktree file), `new-content` or `unified-diff`, a
  `title`, `session` (the extension's key for a tool session) and `apply: bool`. It returns an id.
- **`review.withdraw(id) -> result<string, host-error>`** closes an open proposal and returns its
  current text, with the user's edits so far, without a `review-decided`. Gemini's `closeDiff`
  needs exactly that (close the diff, return the text), and Claude's `close_tab` and
  `closeAllDiffTabs` need the closing.
- **A decided proposal is never withdrawn.** After the user decides, a tool may still ask to close
  the diff (Claude Code sends `closeAllDiffTabs` at every prompt and `close_tab` twice after each
  answered diff). The extension answers these from its own state; it calls `review.withdraw` only
  for a proposal that is still open.
- **The outcome** comes later: `io-event::review-decided(id, result<review-outcome,
  review-error>)`. `review-outcome` has `decision` (`accepted`, `partial`, `rejected`) and
  `final-content`, the text after the user's choices and edits. A blocking protocol (Claude's
  `openDiff`) keeps its MCP call open until then. A notifying one (Gemini's
  `ide/diffAccepted` / `ide/diffRejected`) sends a notification.
- **`apply = false` (default):** nothing is written; the tool writes the file itself with
  `final-content`, as Claude Code does. **`apply = true`:** the host applies the accepted hunks to
  the buffer as one undoable transaction and never saves. Reviewing is the consent to each edit,
  so this needs no `workspace:edit`. Applying without a review is `workspace:edit` (T2).
- **Turns:** `review.begin-turn(session)` and `review.end-turn(session, changed-paths)` map to
  `agent_review`'s `begin_turn` / `end_turn` / `note_changed_paths`
  (`crates/agent_review/src/agent_review.rs:560-634`). Files the tool changed during a turn without
  a proposal show as "changed without review".
- **Two paths for a tool that has hooks** (Claude Code, 2.1.326 live recording):
  - **Post-hoc (primary).** `PreToolUse` and `PostToolUse` hooks reach the extension through
    `fyx-relay` (§B.9). The extension takes a snapshot of the file before the tool writes it and
    proposes the diff afterwards, so the change reaches agent review hunk by hunk whatever the
    tool's permission mode. It is after the fact: the tool has already written the file.
    - **Reject restores the before-snapshot.** If the file changed after the agent's write (the
      user typed in it, another tool wrote it), Fyx shows a conflict view and never overwrites
      silently.
    - **Fyx tells the agent what was reverted.** The default channel is the hook's
      `additionalContext`: on the next `UserPromptSubmit`, or on `PostToolUse` when "hold edits"
      is on. Pasting the note into the tool's terminal (§B.11) happens only if the user chooses it.
  - **`openDiff` (bonus).** The tool's own IDE protocol proposes the edit before it writes
    (`apply = false`, a blocking call). It happens only when the tool asks, which for Claude Code
    is manual (`default`) permission mode only; `auto` is Claude's default mode now, so this path
    is the exception.
  - **Hold edits for review (optional, a 5.15 follow-up, not in the first cut).** A setting turns
    on a blocking `PreToolUse` hook: the relay request waits, Fyx shows the diff, and the answer
    allows or denies the tool call, under the channel's timeout and `on_failure` (§B.9). It is the
    only way to review before the write in `auto` mode, and it fails open by default like any
    relay channel.
- **Provenance:** the review panel names the extension and the tool session. Accepting or
  rejecting stays the user's; the extension cannot answer a review (spec §5, agent review policy
  stays with W5).
- **Limits:** 64 open proposals per extension, 8 MiB per file. Protected and private paths are
  refused (`protected.rs`, `is_private_path`).
- **Review comments (plan 7.2).** The user comments on hunks in the review pane and sends the
  open comments to the agent as one prompt, with file and line references ("Send comments to
  agent", `agent_review::SendComments`). `agent_review` (W5, `review_comments.rs`) keeps them:
  - A comment on a **rejected** hunk explains the rejection, so it stays in the decision's
    feedback (the tool result, or `review-outcome` for an extension), as before.
  - A comment on an **accepted** hunk asks for follow-up work, so it is no longer in the
    feedback. It stays open until the user sends it, or goes in front of the user's next prompt
    in the session, so nothing is lost when nobody presses Send.
  - Comments on a review that is **still open** can be sent too; they are marked sent ("Sent to
    the agent" in the pane) and left out of that review's feedback later.
  - **Native agent panel (done):** the comments are the thread's next prompt
    (`AcpThread::send`): at once when the thread is idle, queued until the turn stops when it is
    generating, and open again if the turn ends in an error.
  - **Extensions (terminal agents):** comments on an extension's proposals go to that extension.
    `AgentReview::take_extension_comments(name) -> Vec<CommentBatch>` hands them over; the bridge
    in `ui_ext_host` (W3) takes them on `AgentReviewEvent::CommentsChanged` and delivers one
    `io-event::review-comments` per batch (WIT below, for W0). No new capability:
    `review:propose` covers comments on the extension's own proposals. The extension forwards the
    `prompt` to its tool on the tool's next hook through `fyx-relay` (§B.9): Claude Code's
    `UserPromptSubmit` (or `PostToolUse`) answers `{"hookSpecificOutput": {"additionalContext":
    prompt}}`, Gemini's `BeforeAgent` the same; a `Stop` hook that fires while comments wait
    answers `{"decision": "block", "reason": prompt}`, so the agent works on them at once. Pasting
    the prompt into the tool's terminal (§B.11) happens only if the user chooses it.

  ```wit
  // review gains (0.4):
  enum comment-state { accepted, rejected, pending }
  record review-comment {
      path: file-ref,
      /// Lines of the file after the change, 1-based and inclusive (`end` < `start`: lines were
      /// removed after `start`); none when the review showed a fragment of the file.
      lines: option<tuple<u32, u32>>,
      state: comment-state,
      /// The first changed line, at most 80 characters.
      excerpt: string,
      text: string,
  }
  record review-comments {
      /// The proposals' `session`.
      session: string,
      comments: list<review-comment>,
      /// The comments as one prompt for the agent, with paths relative to the project.
      prompt: string,
  }
  // io-event gains (0.4), after review-decided:
  //   review-comments(review-comments)
  ```
- **Crate owner:** `agent_review` is W5's. The bridge lives in `ui_ext_host` and uses only
  `agent_review`'s public API; a missing hook is asked of W5.

### B.11 Editor context, opening files, pasting into a tool

- **`context:editor` (T1).** `context.watch(options)` subscribes to `io-event::context-changed`:
  the active file, open files **in most-recent order** (Gemini's `ide/contextUpdate` orders files
  by last focus), cursor and selection (text up to 64 KiB), and `trusted: option<bool>`, whether
  the worktree is trusted (the same flag as `isTrusted` in Gemini's update; `none` when Fyx has no
  answer), debounced 300 ms. Only
  worktree files; private paths (`is_private_path`) appear as "a private file" with no text.
  The install dialog warns when it is combined with `network:*`, `terminal:*` writes or
  `relay:hooks`: "It can pass what you select to <tool>".
- **`workspace:open` (T1).** `workspace.open(path, range, focus: bool)` opens a worktree file at a
  range. `focus = true` needs the user's gesture, or a listener or relay request less than 2 s
  old from a terminal that has focus. Otherwise the file opens in a background tab and never
  takes focus from typing.
- **File references from a tool** (under `context:editor`). A tool's IDE protocol speaks absolute
  paths (`openFile`, `at_mentioned`), and the WIT speaks `file-ref`. `context.to-file-ref(abs-path)`
  turns an absolute path into a `file-ref` of one of the window's worktrees, and
  `context.absolute-path(file-ref)` goes the other way. The host does the conversion, so the
  extension still never learns a path outside the worktrees: a path that is in no worktree, or is
  private (`is_private_path`), is an error, not a `file-ref`.
- **Diagnostics read (`workspace:diagnostics`, T1).** Claude Code calls `getDiagnostics` about
  four times a turn (around each edit, for one file and for all files), and diffs the answers
  ("Found 1 new diagnostic issue"). The 0.3 WIT only passes diagnostics inside provider requests,
  so a tool extension had nothing to answer with. 0.4 adds a pull read and a change event to the
  `context` interface:
  - `context.diagnostics(path: option<file-ref>)` returns a snapshot. With a `file-ref` it is that
    one file; with `none` it is every worktree file of the window that has diagnostics (what the
    editor's language servers report, open or not).
  - Each entry has `range` (a `text-range`, **0-based**, columns in UTF-8 bytes as everywhere in
    the WIT), `severity` (`error`, `warning`, `information`, `hint`), `message`, `source` (the
    server, such as `rust-analyzer`) and `code`, both optional. A message is cut at 2 KiB.
  - **Worktree files only.** A private file (`is_private_path`) is `not-permitted` when asked for
    by name and is left out of the all-files answer without a trace (no entry, no count). A
    `file-ref` that is in no worktree is `invalid-argument`; the extension never gets a path
    outside the worktrees. A file with no diagnostics returns an empty list.
  - **Capped at 500 entries** in one answer, errors first, then warnings, information and hints,
    then by path and line. `truncated` is true when entries were left out.
  - **`diagnostics-changed` (optional).** `context.watch-diagnostics(on)` subscribes to
    `io-event::diagnostics-changed(list<file-ref>)`: the files whose diagnostics changed,
    debounced 300 ms, at most 200 files per event (a larger change sends an empty list: "everything
    may have changed"). It carries no entries, so a caller caches the last snapshot and re-reads
    only when told. Without a subscription `diagnostics` still works; it is a fresh read.
  - The install dialog's combination warning (above) applies: diagnostics can carry source text
    in messages.
- **Paste into a tool:** `terminals.paste(handle, text)` into the extension's profile terminal
  (§B.1) or own terminal. Only after a gesture in the extension's UI or a command the user ran
  ("Send selection to Claude"). Only while the terminal's foreground program has bracketed-paste
  mode on (alacritty `TermMode::BRACKETED_PASTE`); otherwise `unavailable`, since a newline
  would run a command. The host strips ESC and every control character except tab and newline,
  wraps the text in the bracketed-paste markers, and caps it at 16 KiB. This is how
  `@file#L5-10` mentions reach Claude Code.

### B.12 Approval cards (`ui:approvals`, T0)

A tool asks for permission to run a step (Claude/Codex/Copilot `PermissionRequest`, Gemini
`BeforeTool`, OpenCode `permission.asked`). The extension can show the question as a Fyx card:

- `approvals.ask({title, detail, options})` with "Allow once", "Always allow" and "Deny" (the
  extension chooses which, at least one), answered by `io-event::approval-answered(id, option)`.
- **Host-drawn:** the extension's name and the tool's name (as the extension states it, marked
  "says") are drawn by the host. The card has a 1 s click guard, no keyboard answer, and does not
  grant anything in Fyx. The answer goes back to the tool through the extension.
- **Limits:** 4 open per extension; a closed card answers nothing.

## C. Consent in 0.4 (layer C)

### C.1 Tiers

| Tier | When the user agrees | What it covers |
|---|---|---|
| **T0** | never asked; shown on the extension page | UI, `package:files`, `ui:approvals`, status on own terminals |
| **T1** | listed in the install dialog; installing is the consent; revocable | reads and the user-started kinds: `workspace:read-*`, `workspace:diagnostics`, `context:editor`, `workspace:open`, `terminal:profiles`, `terminal:pseudo`, `terminal:status`, `review:propose` |
| **T2** | **one checkbox per item in the install dialog**, unticked by default; untick = `denied`; revocable; can be ticked later on the extension page | `network:fetch`, `network:listen`, `process:spawn`, `terminal:create`, `terminal:env`, `terminal:observe`, `fs:discovery-file`, `relay:hooks`, `workspace:edit`, `agent:observe` |
| **T3** (new) | **at the moment of action**, on one terminal, after a gesture; never in advance, except the deliberate "Always allow" of §C.3 | `terminal:read`, `terminal:write` on the user's terminals |

**Combinations are spelled out** in the dialog: `context:editor` or a T1 read with any network,
relay or terminal write says "It can send what it reads to …"; a `RUNS_ANY_CODE` program says "It
can run any program on your computer" (0.3 Q2).

### C.2 T2 at install, with checkboxes

The install dialog (file, dev UI, registry) has four groups, named by how consent works:

1. **Shows in Fyx** (T0): one line.
2. **Allowed without asking** (T1): one line per capability, no checkbox. T1 includes
   `terminal:pseudo`, `terminal:status`, `workspace:open` and `review:propose` (and the other T1
   capabilities of §C.1), so this group is not only reading. `terminal:observe` is not here: it
   is a read-only T2 item, in the next group, with a checkbox.
3. **Off unless you tick it** (T2): a checkbox per item, unticked:
   - one per program for `process:spawn` and `terminal:create`, with its argument patterns;
   - one per listener for `network:listen`, and one per discovery file, with its path;
   - one for `network:fetch` with its domain list, one for `terminal:env` with every name and
     value, plus the second, stronger checkbox for `scope = "all-new"`;
   - one each for `terminal:observe`, `relay:hooks` (with its channels and `on_failure`),
     `workspace:edit` and `agent:observe`.

   The author's one-line reason for each item (`reason = "…"` in the manifest, at most 200
   characters, required for T2 items, plain text) is shown under it as "The author says: …".
4. **Asks you each time** (T3): which T3 capabilities it may ask for. No checkbox.

- **Buttons:** "Install" installs with what is ticked. A T2 item left unticked is `denied`; the
  extension sees `denied` and should say what it cannot do. Every checkbox ignores clicks for 1 s
  after the dialog appears.
- **Updates:** the update dialog (§C.5) shows only the groups that have changes, with no empty
  groups.
- **Later:** the extension page lists every item with its state and a switch; turning a T2 item
  on asks once more, with the same text and the 1 s guard.
- **Dev installs without a dialog** (`FYX_UI_DEV_EXTENSIONS`, dev builds only) keep 0.3's
  first-use prompt (0.3 design §7).
- **0.3 extensions** go through the same dialog when installed under a host that ships this
  model. This is host behaviour, not ABI: `permission-state::ask` stays for the dev path, and
  `denied` still changes only on the extension page.
- **Revocation** stops everything at once: created terminals and their process trees are killed,
  listeners close, discovery files go, relay channels answer `on_failure`, the env leaves new
  terminals (open ones show "Restart to apply"), and the extension gets
  `io-event::permission-changed`.

### C.3 T3: at the moment of action

- **`terminal:read`.** On a user terminal, the tab's menu "Share with <extension>…" (or a card
  from `terminals.request-read(handle, scope)`, which needs a gesture in the extension's UI).
  The scope is the last command block (needs shell integration, phase 6) or the last N lines
  (N ≤ 500), as plain text without ANSI. The result is `io-event::terminal-read-done(id,
  result<string, terminal-error>)`. The card offers "Once" and "This terminal until it closes".
- **`terminal:write`.** Only on a terminal the user shared for writing. **Every command is
  confirmed** on a card that shows the exact text (`terminals.request-write(handle, text,
  submit: bool)`). Printable text and tab only; no control keys, no ESC. `submit` adds Enter
  after the confirmation. The tab shows "typed by <extension>" for 5 s.
- **Cards** have the 1 s click guard, no keyboard answer, and are host-drawn with provenance.
- **Never** in an untrusted worktree, and never for a terminal of another Fyx profile.

**"Always allow" for T3** (the owner's rule):
- It is turned on only from a T3 card ("Always allow <extension>") or on the extension page,
  never in the install dialog, and separately for reading and writing.
- First, a warning in plain words: "<extension> will see everything that appears in your
  terminals, including passwords and tokens, and run any command as you." The confirm button
  works after 1 s.
- While it holds, every terminal tab shows "<extension> has access", and the status bar shows an
  indicator with one-click revoke.
- Every read and write goes to the journal (§C.4); a write also highlights the tab.
- It resets when the extension's component changes (a new sha256) and does not apply in an
  untrusted worktree. **Only T3 "Always allow" is bound to the component hash.** T1 and T2
  consent is not (integration-6 R9, `consent.rs`), but updates are held by §C.5.

### C.4 The journal

The extension page shows a usage journal, kept in
`<data_dir>/extensions/fyx/journal/<id>.jsonl` (the last 2000 entries or 30 days, whichever is
fewer; cleared on uninstall):
- programs started (name, argv, cwd, exit);
- terminal reads (which terminal, scope, size) — **never the content**;
- terminal writes (which terminal, the exact text the user confirmed);
- listeners opened (port) and connections accepted (count per hour);
- discovery files written and removed (path);
- relay requests (channel, count per hour), proposed edits (path, outcome);
- T2 or T3 consent changes.

Writes are batched off the main thread; a full disk drops entries and shows "Journal incomplete".

### C.5 Updates show a diff

An update that **adds or widens** a T1 or T2 item (a new capability, program, argument pattern,
domain, env name or value, listener, discovery path, relay channel, or `scope = "all-new"`) is
held. The old version keeps running, and the page shows "Update needs approval" with a diff:

> **New in 1.3:** runs `codex` with `-c notify=*`; adds `CODEX_TUI_NOTIFY` to your profile
> terminals. **Removed:** `network:fetch` to api.example.com.

New T2 items are checkboxes, unticked. Approving installs the update with what is ticked.
Removals and T0 additions install silently (spec decision 21). The registry sees the same diff in
the PR comment (`registry.py comment`).

### C.6 Multiple accounts

Research §7: a user has a personal Claude account in `~/.claude` and a work one in
`~/.claude-work`, and the same for other tools.

**Account profiles** (user settings, `fyx.accounts`): a name ("Personal", "Work"), a tool, the
env name that selects its config dir (`CLAUDE_CONFIG_DIR`, `GEMINI_CLI_HOME`, `CODEX_HOME`,
`COPILOT_HOME`, `OPENCODE_CONFIG_DIR`; presets are settings data), the dir, and a colour.
- Fyx only puts that one env name into a terminal (a profile terminal with `account_tool`, or a
  plain terminal the user picks it for) or into an ACP agent's environment (one `fyx:` hook in
  `crates/agent_servers/src/acp.rs`, next to the existing `CLAUDE_CODE_OAUTH_TOKEN` handling at
  `:1453`). **The user signs in with the tool's own command, inside that dir.**
- **Per project:** "In this project, use Work" is stored in user settings by project path, never
  in the repository, so a repository cannot pick an account for you. The "+" menu ("Claude Code —
  Work") and the agent panel offer the choice too.
- **Always visible:** the account's name and colour on the terminal tab, in the sessions list and
  in the agent panel.
- **Discovery files** go into the account's dir (§B.8), so the CLI connects to the right window.

**Rules built into the product** (not only registry rules):
1. **Fyx never touches credentials.** It does not read, copy, store or move tokens between
   profiles. An account dir is an opaque path to Fyx. Only the official tool signs in.
2. **No automatic switching.** Fyx never changes account because of a rate limit, a
   `rate_limit` error or an exhausted quota. A limit message shows as it is ("limit until
   15:00"), with no "continue with another account" button.
3. **Changing account starts a new session.** Fyx never carries a session, its history or its
   context over to another account to continue the same work on another account's limit.
4. **No load spreading:** no "run on every account", no queue that hands work to accounts, no
   account pool.
5. **Extensions cannot do any of this.** These names are refused for `terminal:env` and
   `process:spawn` (the latter is already an allowlist), in any case:
   - config dirs: `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GEMINI_CLI_HOME`, `COPILOT_HOME`,
     `OPENCODE_CONFIG_DIR`, `OPENCODE_CONFIG`, `OPENCODE_CONFIG_CONTENT`, `QWEN_CODE_HOME`,
     `AMP_HOME`, and every name an account-profile preset uses;
   - credentials: `CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`,
     `OPENAI_API_KEY`, `GEMINI_API_KEY`, `GOOGLE_API_KEY`, `GOOGLE_APPLICATION_CREDENTIALS`,
     `GH_TOKEN`, `GITHUB_TOKEN`, `COPILOT_GITHUB_TOKEN`, and every name ending in `_API_KEY`,
     `_TOKEN`, `_SECRET`, `_PASSWORD` or `_CREDENTIALS`.
   - **One exception** (W2c ba7186021a): a `terminal:env` name ending in `_TOKEN` is allowed when
     its value is exactly one `token_in_env` listener's host token, `${listen.<name>.token}`, and
     nothing else. That token is Fyx's own, not a credential. The credential names listed by
     name above, the account-dir names and `FYX_*` stay refused even then.

   The list lives in `ui_ext::process` next to `is_forbidden_env` (W2c, plan 5.5).
   Account dirs join the protected paths, like `.claude/`. An extension sees only an account
   profile's name, never its path (`${account.<tool>}` is resolved by the host).
6. **Registry rule:** extensions that rotate accounts, get around limits, or share or resell
   access are not accepted in `fyx-extensions` and are blocked if found (registry-repo.md, review
   rules).

Honest limit: a person at a terminal can do anything by hand, and Fyx does not stop them. The aim
is that Fyx builds and opens no mechanism for it.

**The agent panel** talks to Claude through upstream Zed's ACP adapter on the Agent SDK. Anthropic's
terms say Agent SDK products should use API keys. This is upstream behaviour (`crates/agent_servers`
is unchanged since the fork), and the owner decided on 2026-11-19 to leave it and follow upstream.

**Fyx profiles, variant A** (the owner's choice): a Fyx profile is a separate data dir through
Zed's `--user-data-dir` (`crates/paths/src/paths.rs:103-135`, `crates/cli/src/main.rs:526`):
settings, keymap, themes, extensions, history and accounts are separate, and one profile is one
process. Variant B (profiles inside one process, like VS Code) is rejected: Zed's settings store
and extension host are global to the process.
- **Profile manager:** create an empty profile, a copy, or one from a template; name, colour,
  icon.
- **Choosing:** at start when there is more than one profile (or "always open the last"); a chip
  with the profile's name and colour in the title bar; "Open in profile…"; the Dock menu;
  `fyx --profile <name>`.
- **A project remembers its profile:** opening a work repository from the personal profile
  offers to open it in "Work".
- **Accounts belong to the profile:** in "Work", account profiles default to the work dirs.
- **Later:** "take the theme and keymap from Personal".
- **The same rules:** switching profile is the user's action only, never automatic; extensions do
  not see other profiles and cannot start them.
- **To check (W1, 5.20):** how macOS shows several running instances of one `.app` with different
  `--user-data-dir`, and whether `fyx` CLI opens a file in the running instance of the right
  profile.

### C.7 Rules that change (decided by the owner 2026-11-19)

| Rule | Where | New rule |
|---|---|---|
| T2 at first use | [spec decision 21](../platform/spec.md), [0.3 design §7](../wit-0.3/design.md#7-t2-consent-at-first-use-plan-45) | T2 is accepted at install, one checkbox per item (§C.2); first use stays only for dev installs without a dialog. New tier T3 at the moment of action (§C.3). Journal (§C.4), update diff (§C.5). |
| "A terminal is the user's" | [0.3 design, "Not in 0.3"](../wit-0.3/design.md#not-in-03) | **The user's terminals are the user's by default; an extension's own terminals are its own.** Pseudo and created terminals belong to the extension; everything else needs T3. |
| 0.3 is frozen | spec decision 30 | Terminals go into the new staged world `since_v0.4.0`. 0.3 does not change. |
| Upstream edits are minimal hooks | `.claude/CLAUDE.md` | Unchanged: about 25 lines in three Zed files plus the ACP hook (§A). It changes only if the `TeePty` prototype fails and the alacritty fork needs a patch, which then goes to the owner. |
| Protected paths for extensions | 0.3 design §10, `protected.rs` | Unchanged for extensions. The host writes discovery files by the declared template (§B.8), and `fyx_guard` allows that explicitly. Account dirs join the protected paths. |

## D. Extension settings (plan 6.1)

Status (2026-11-20): **design, for implementation**, written by W4 at the coordinator's request.
It has to land in this world before it freezes. The SDK side (`ui_ext_sdk::settings`) and the
harness fake (`fyx_ext_harness::settings`) are built; the host side (W2c model and broker, W3
UI) and the WIT (W0) are below.

**The problem.** An extension has no way to let the user choose anything. `fyx_claude_code`
hard-codes that `openDiff` blocks; `fyx_gemini_cli` cannot turn its hooks off; a linter demo
cannot take a severity. Each would build its own panel form and store nothing, since
extensions have no storage.

**The answer.** The manifest declares typed settings. The host stores the values in the user's
settings file, draws the form, checks every value against the declaration, and gives the
extension only checked values. The extension reads them and is told when they change. It
never draws the form and never writes a value in 0.4.

### D.1 Manifest: `[[fyx_ui.settings]]`

```toml
[[fyx_ui.settings]]
key = "block-on-diff"
title = "Wait for review"
description = "Claude waits until you decide on a diff in agent review."
type = "boolean"
default = true

[[fyx_ui.settings]]
key = "max-open-diffs"
title = "Open diffs at most"
type = "integer"
default = 8
min = 1
max = 64

[[fyx_ui.settings]]
key = "severity"
title = "Report as"
type = "choice"
default = "warning"
choices = [
    { value = "error", title = "Error" },
    { value = "warning", title = "Warning" },
]

[[fyx_ui.settings]]
key = "extra-args"
title = "Extra arguments"
type = "text-list"
default = []
max_items = 16
max_length = 256
```

| Field | Rule |
|---|---|
| `key` | 1 to 64 bytes of `a-z`, `0-9` and `-`, starting with a letter; unique in the manifest. It is also the key in the settings file. |
| `title` | 1 to 80 bytes, one line. Drawn as the control's label. |
| `description` | Optional, at most 500 bytes, plain text (no Markdown, no links). |
| `type` | `boolean`, `integer`, `number`, `text`, `choice` or `text-list`. |
| `default` | Required, and valid for its own declaration (the same rules as a user's value). |
| `min`, `max` | `integer` (whole numbers, i64) and `number` (finite f64) only; optional; `min <= max`. |
| `max_length` | `text` and `text-list` items: at most this many bytes, 1 to 4096; default 256. One line: no control characters. |
| `max_items` | `text-list` only: 1 to 64, default 16. |
| `choices` | `choice` only: 2 to 32 entries `{ value, title }`, `value` as a key (unique), `title` 1 to 80 bytes. |

- At most **32 settings** per extension.
- **No capability.** Reading its own settings gives an extension nothing it did not declare.
  The install dialog lists nothing for them (they are not consent), and the extension page
  shows the form.
- **No secrets.** There is no secret or password type, and a `key` that contains `token`,
  `secret`, `password`, `api-key` or `apikey` is refused: values sit in the user's plain
  `settings.json`, which users share. Credentials stay with the tool (`account_tool`, §C.6).
- **No paths and no commands as types.** A `text` value is a string; what the extension does
  with it is its own business, under its capabilities. Nothing a setting says widens a
  capability: a `text` setting cannot add a program to `[[fyx_ui.terminal_programs]]`.
- The table is refused for a world below 0.4, like the other 0.4 tables (`world_capabilities`).

### D.2 Storage and the effective value

The values live in the **user** settings file, under the extension's id:

```json
"fyx_extensions": {
  "fyx-claude-code": { "block-on-diff": false, "max-open-diffs": 4 }
}
```

- The effective value of a key is the user's value when it is valid, else the default.
- An invalid value (wrong type, out of range, unknown choice, too long, too many items) is
  **not** passed on: the extension gets the default, and the form shows the user's value with
  the error next to it. An unknown key under the extension's id is ignored and shown greyed out
  ("not used by this version") so an update that drops a setting loses nothing.
- **User level only in 0.4.** Project settings (`.zed/settings.json` in a worktree) cannot set
  `fyx_extensions`, the same rule as M6.1 for review: a repository must not change how an
  extension behaves without the user. Per-project values are in [Not in 0.4](#not-in-04).
- Values are never shared between extensions. Uninstalling keeps them (like Zed keeps an
  extension's settings) until the user deletes them; the extension page offers "Reset".

### D.3 The form (host-drawn)

W3 draws the form from the declarations, on the extension's page in **Fyx Extensions** under
a **Settings** tab, and links it from the extension's panel menu.

| Type | Control |
|---|---|
| `boolean` | switch |
| `integer`, `number` | number field with `min`/`max` |
| `text` | one-line text field |
| `choice` | dropdown of the titles |
| `text-list` | list of one-line fields with add and remove |

Each control shows the title, the description, a "Reset to default" button when the value is
not the default, and the error of an invalid value. A change writes the user settings file
through Zed's settings store (the same write the settings UI makes), so editing
`settings.json` by hand works too and goes through the same check.

### D.4 Reading and watching

```wit
/// 0.4. The extension's own settings (design.md §D): declared in `[[fyx_ui.settings]]`, set by
/// the user, checked by the host. No capability.
interface settings {
    use host.{host-error};

    variant setting-value {
        boolean(bool),
        integer(s64),
        number(f64),
        text(string),
        /// The choice's `value`, never its title.
        choice(string),
        text-list(list<string>),
    }

    record setting {
        key: string,
        value: setting-value,
        /// False when the user set a valid value (which may equal the default).
        is-default: bool,
    }

    /// The effective value of one declared key; `invalid-argument` for a key the manifest does
    /// not declare.
    get: func(key: string) -> result<setting-value, host-error>;
    /// Every declared key, in manifest order.
    all: func() -> list<setting>;
    /// Shows the extension's Settings tab; `key` scrolls to that setting. Only while the user
    /// acts on one of the extension's surfaces (an event call), else `not-permitted`.
    open: func(key: option<string>) -> result<_, host-error>;
}
```

- `io-event::settings-changed(list<string>)`: the keys whose **effective** value changed, in
  manifest order, delivered through `handle-io` once per settings-file change. A change that
  leaves every effective value the same (an invalid value replaced by another invalid one, a
  key the extension does not declare) sends nothing. No watch call: every 0.4 extension with
  `[[fyx_ui.settings]]` gets it.
- The values are fixed for the length of one export call: a change made while the extension
  runs arrives as the next `handle-io`.
- `get` and `all` are cheap host reads (no I/O); an extension may call them on every event, but
  the SDK's pattern is to read at `new()` and on `settings-changed`.
- Reload, disable and uninstall need nothing: values are read, not held by the host for the
  extension.

### D.5 SDK and harness (W4, built behind `fyx-ui-0-4`)

- `ui_ext_sdk::settings`: `SettingValue` (with `as_bool`, `as_integer`, `as_number`,
  `as_text`, `as_choice`, `as_text_list`), `Setting`, `get`, `all` and `open`. Until W0
  stages the interface, the calls return `unavailable` on WASM; off WASM they answer from
  `set_settings` (for an extension's unit tests) and `open` records `HostEffect::Settings`.
  `IoEvent::SettingsChanged` and the WASM glue come with the staged WIT.
- `fyx_ext_harness::settings`: the reference checks of §D.1 and §D.2 on the manifest's TOML
  (`SettingSpec::parse_all`), and a fake store: `Harness::settings()`,
  `Harness::set_user_settings(json)` (returns the changed keys, as the host would send them),
  `Harness::setting(key)`, `Harness::setting_error(key)`, `Harness::take_settings_changed()`.
  `load_package` reads `[[fyx_ui.settings]]`. Once the WIT is staged,
  the fake links `settings` and queues `settings-changed` for `deliver_io`.
- `fyx-ext check` runs the same checks on `[[fyx_ui.settings]]`.

The checks are written once, in the harness, as the reference. W2c ports them into
`ui_ext::settings` for the host (or makes the harness use them, once they are there);
`harness_conformance` then pins the two together, as for the other tables.

### D.6 Host side (W2c, W3)

| Owner | Work |
|---|---|
| W2c | `ui_ext::settings`: `SettingSpec`, `SettingValue`, the checks of §D.1 (manifest) and §D.2 (values); `FyxUiTable.settings`; the `settings` import in `ui_ext_wasm`; reading `fyx_extensions.<id>` from the user settings store (never project settings), computing effective values, and sending `settings-changed` on a store change |
| W3 | The Settings tab and its controls (§D.3), "Reset" for one key and for all, the greyed-out unknown keys, `settings.open` |
| W0 | The WIT below, and `settings` in the V0_4 world lists (`ui_ext_wasm`, `fyx_ext`, harness) |

### D.7 Tests

- A manifest with a duplicate key, a `default` outside `min`/`max`, a `choice` default not in
  `choices`, a key with `token` in it, or 33 settings is refused; so is the table on a 0.3
  world.
- A user value of the wrong type or out of range gives the default to the extension and an
  error in the form; fixing it sends `settings-changed` with that key only.
- Project settings that set `fyx_extensions` change nothing.
- `open` outside an event call is `not-permitted`.
- The harness and the host agree on every check (`harness_conformance`).

## E. Diagnostics from extensions (plan 6.6)

Plan 6.6 lets an extension report problems in files (a linter, a spell checker, a policy check) the
way a language server does: squiggles in the editor, entries in the diagnostics panel and the
status-bar counts, with the extension as the source. It is planned for "0.4 or the first post-1.0
WIT"; the diff below fits either, and reuses the 0.4 `context` types of §B.11.

### E.1 What the extension sees (`diagnostics:publish`, T1)

- New capability `diagnostics:publish`, T1 ("Shows problems in your files"). It is independent of
  `workspace:diagnostics` (reading what language servers report): a linter that only publishes
  needs no read.
- `context.publish-diagnostics(path, list<diagnostic>)` replaces this extension's diagnostics of
  one file; an empty list clears them. `context.clear-diagnostics(none)` clears every file.
- The `diagnostic` record of §B.11 is reused: `range` 0-based with UTF-8 byte columns, `severity`,
  `message` (cut at 2 KiB), `code`. **`source` is ignored**: the host sets it to the extension's
  display name, so an extension cannot pose as `rust-analyzer`.
- Worktree files only: a `file-ref` in no worktree is `invalid-argument`, a private file
  (`is_private_path`) is `not-permitted`.
- Limits: 500 entries per file and 5,000 per extension (more is `invalid-argument` and changes
  nothing); 20 publishes per second per extension (more is `unavailable`, retry later). A range
  past the end of the file is clamped to it.
- An extension's diagnostics last while it runs: stopping, crashing, uninstalling, disabling or
  revoking the capability clears them all. A hot reload (1.3) clears them too; the new instance
  publishes again.

### E.2 Host side (W2c, W3)

The project already shows diagnostics from any source through `LspStore::merge_diagnostic_entries`
(public). Each running extension instance gets its own `LanguageServerId` from
`project.languages().next_language_server_id()` per project; it is never registered as a server,
so nothing sends LSP requests to it.

- `publish-diagnostics` resolves the `file-ref` to an absolute path, converts each range to
  `Unclipped<PointUtf16>` against the file's text (the open buffer's snapshot, or the file loaded
  through the project's `Fs` when it is not open; columns are UTF-8 bytes in the WIT), and calls
  `merge_diagnostic_entries` with one `DocumentDiagnosticsUpdate { server_id, diagnostics:
  DocumentDiagnostics { document_abs_path, version: None, diagnostics }, result_id: None,
  registration_id: None, disk_based_sources: &[] }` and a merge that keeps nothing old of that
  server (`|_, _, _| false`).
- Each entry is a `DiagnosticEntry` with `Diagnostic { source: Some(<display name>), severity,
  message, code, source_kind: DiagnosticSourceKind::Other, is_primary: true, group_id: <unique per
  entry>, .. }`.
- Clearing is the same call with an empty list for each file the extension published; the host
  keeps that set per instance.
- The diagnostics panel, the editor and the status bar need no change: they follow
  `Event::DiagnosticsUpdated` for any server id. Remote projects: local projects only in the first
  version (a remote host has its own `LspStore`; the update would have to be forwarded).

| Owner | Work |
|---|---|
| W0 | The WIT below; `diagnostics:publish` in the capability lists and the install-dialog text |
| W2c | The import in `ui_ext_wasm`, the checks of §E.1, the limits |
| W3 | The per-instance collection in `ui_ext_host`: server ids, conversion, publish, clear on stop, crash, reload, uninstall and revoke |
| W5 (W4's part) | `fyx_sdk` wrapper and harness double; a linter demo (`fyx-lint-demo`: marks `TODO` lines) |

### E.3 WIT diff

In `interface context` (0.4 file, or the next world's copy), after `watch-diagnostics`:

```wit
    /// Plan 6.6, `diagnostics:publish`: replaces this extension's diagnostics of `path` (an empty
    /// list clears them). `source` is ignored: the host names the extension. In no worktree is
    /// `invalid-argument`, a private file `not-permitted`; over 500 entries per file or 5,000 per
    /// extension is `invalid-argument`; over 20 calls a second is `unavailable`.
    publish-diagnostics: func(path: file-ref, diagnostics: list<diagnostic>) -> result<_, host-error>;
    /// `diagnostics:publish`: clears this extension's diagnostics of one file, or of every file
    /// with `none`.
    clear-diagnostics: func(path: option<file-ref>) -> result<_, host-error>;
```

### E.4 Tests

- The linter demo publishes two entries in a worktree file: the editor shows them, the panel
  lists them with the extension's name as source, and the status bar counts them.
- `source: some("rust-analyzer")` is shown as the extension's name.
- Publishing an empty list clears the file; stopping the extension clears every file.
- A private file and a file outside the worktrees are refused and publish nothing.
- 501 entries for one file are refused and leave the previous ones.
- Columns: a line with a multi-byte character before the range lands on the same characters.

## F. Tree views and file decorations (plan 6.7)

Plan 6.7 adds two things a bookmarks, outline or test-explorer extension needs: rows that expand
and collapse, and badges or colours on project-panel entries. The `list` node already draws
`list-row.indent`; what is missing is the chevron, its event and the keys. Like §E, the diff fits
0.4 or the first post-1.0 world.

### F.1 Tree rows (no capability)

- `list-row` gets `expansion: expansion`, an enum `leaf`, `collapsed`, `expanded`. The host draws
  a chevron before the row's nodes (right for `collapsed`, down for `expanded`, an empty slot of
  the same width for `leaf`, so leaves line up with their siblings), after the indentation.
- A click on the chevron, or Right on a `collapsed` row and Left on an `expanded` one while the
  list has keyboard focus, sends the new event `row-toggled(tuple<node-id, u32, bool>)`: the list,
  the row index, and whether the row asks to be expanded. It is not a `row-click`, and it grants no
  activation (§C.3 gestures): expanding is browsing.
- The host changes nothing itself. The extension answers with the new `row-count` and an
  `invalidate-rows` patch from the toggled row on (or a smaller range), and sets the row's
  `expansion` in the rows it returns. Until then the old rows stay. This keeps the extension the
  owner of the tree's shape, as with every other list state, and keeps virtualization working
  for trees of any size.
- Left on a `leaf` or `collapsed` row moves the selection to the nearest earlier row with a
  smaller `indent` (its parent) and reports `row-selected`, as arrow keys do in modals today. That
  needs no extension code.
- Existing 0.4 extensions are unchanged: a row without the field (an older world) is a `leaf`,
  and no chevron slot is drawn when no row of the list has ever been anything but `leaf`.

### F.2 File decorations (`workspace:file-decorations`, T1)

- New capability `workspace:file-decorations`, T1 ("Marks files in the project panel").
- `context.decorate-files(list<file-decoration>)` sets or replaces this extension's decoration of
  each listed file or directory; `context.clear-file-decorations(option<list<file-ref>>)` removes
  some, or all with `none`.
- `record file-decoration { path: file-ref, badge: option<string>, color: option<color-token>,
  tooltip: option<string> }`. `badge` is at most 2 characters (graphemes), drawn after the name
  like the git status letter; `color` tints the file name (`text-accent`, `warning`, `error`,
  `success`, `text-muted`; the other tokens are `invalid-argument`); `tooltip` (at most 256 bytes)
  is added to the entry's tooltip under the extension's name.
- Worktree entries only; private paths are `not-permitted`. At most 10,000 decorated paths per
  extension and 500 per call; more is `invalid-argument` and changes nothing.
- Several extensions on one entry: badges are shown side by side, at most 2, in extension id
  order; the colour of the first one with a colour wins. Git status and diagnostics colours are
  drawn as today and win over an extension's colour.
- A decoration is not inherited by a folder's children, and does not propagate to parents: the
  extension decorates what it means.
- Decorations last while the extension runs, like §E: stop, crash, reload, uninstall, disable or
  revoke clear them.

### F.3 Host side

- **Tree rows (W3, `ui_ext_host`):** the chevron slot, its click handler and the Left/Right keys in
  the list renderer; `row-toggled` routed like `row-click`.
- **Decorations:** a new Fyx file in an upstream crate both sides already depend on,
  `crates/project/src/fyx_file_decorations.rs` (W2c, with a `fyx:` `mod` line): a `Global` entity
  `FileDecorations` holding, per extension, a map from `ProjectPath` to the decoration, and an
  event when it changes. `ui_ext_host` (W3) writes it; `project_panel` reads it. No new crate
  dependency, so no Cargo change.
- **Project panel hook (W3, one `fyx:` block in `project_panel.rs`):** `EntryDetails` gets the
  entry's decorations from the global when the panel builds its visible entries, and
  `render_entry` draws the badges next to `git_indicator` and applies the colour where
  `filename_text_color` is chosen when no git or diagnostic colour applies. The panel subscribes
  to the global's event and calls `cx.notify()`.

| Owner | Work |
|---|---|
| W0 | The WIT below; `workspace:file-decorations` in the capability lists and the install-dialog text |
| W2c | `fyx_file_decorations.rs` in `project`; the imports and checks in `ui_ext_wasm` |
| W3 | Chevrons, `row-toggled` and keys in the list renderer; the project-panel hook; clearing on stop |
| W5 (W4's part) | `fyx_sdk` wrappers, harness doubles, a `fyx-bookmarks-demo` (a tree of bookmarks grouped by folder, bookmarked files badged `★`) |

### F.4 WIT diff

In `interface types`, before `record list-row`, and one field in it:

```wit
    /// Plan 6.7. A tree row's chevron. `leaf` draws an empty chevron slot.
    enum expansion { leaf, collapsed, expanded }

    record list-row {
        index: u32,
        indent: u8,
        nodes: list<node>,
        /// Plan 6.7. The chevron; Right, Left or a click on it sends `row-toggled`.
        expansion: expansion,
    }
```

In `variant ui-event`, after `row-selected`:

```wit
        /// Plan 6.7. The list node, the row index, and `true` when the row asks to be expanded.
        /// The host changes no rows: the extension answers with new rows. Grants no activation.
        row-toggled(tuple<node-id, u32, bool>),
```

In `interface context`, after the §E functions:

```wit
    /// Plan 6.7, `workspace:file-decorations`.
    record file-decoration {
        path: file-ref,
        /// At most 2 characters, after the name.
        badge: option<string>,
        /// `text-accent`, `warning`, `error`, `success` or `text-muted`.
        color: option<color-token>,
        /// At most 256 bytes, under the extension's name in the entry's tooltip.
        tooltip: option<string>,
    }

    /// Sets or replaces this extension's decoration of each path. At most 500 per call and
    /// 10,000 per extension (`invalid-argument`, nothing changes); a private file is
    /// `not-permitted`, one in no worktree `invalid-argument`.
    decorate-files: func(decorations: list<file-decoration>) -> result<_, host-error>;
    /// Removes this extension's decorations of these paths, or all of them with `none`.
    clear-file-decorations: func(paths: option<list<file-ref>>) -> result<_, host-error>;
```

`list-row` is a record, so adding a field changes the type: in 0.4 that is fine while the world is
staged; after it ships, the next world carries a new `list-row` and the host maps an older
extension's rows to `leaf`.

### F.5 Tests

- The bookmarks demo shows a two-level tree; a click on a folder's chevron sends `row-toggled`
  with `true`, the demo returns the children, and the panel shows them indented under it.
- Right and Left on the focused list expand and collapse; Left on a leaf selects its parent.
- `row-toggled` does not count as a gesture: `workspace.open` with `focus: true` in its handler is
  `not-permitted`.
- Bookmarked files show `★` in the project panel; removing a bookmark clears it; stopping the
  extension clears every badge.
- A badge of 3 characters and the colour `surface` are refused; a private file is refused.
- Two extensions on one file show both badges in id order.

## WIT diff (for W0)

Copy-forward rule: `types`, `host` and every 0.3 interface are copied byte for byte except where
a comment says "0.4". New parts below. The sketch
uses WIT's escape for keywords (`%list`), and no parameter is named after a keyword (`outcome`,
not `result`). W2c's staged draft, `tasks/wit/fyx-ui-0.4.wit` on
`claude/fyx-w2c-host-0-2-cbde2c` (145f748bbf), parses and wins where the two differ.

```wit
// types: one new node kind, marked "0.4".
record terminal-props { handle: u64 }
// node-kind gains: terminal(terminal-props)

/// 0.4. Terminals (design.md §B.1–§B.5, §B.11, §C.3).
interface terminals {
    use host.{host-error};

    type terminal-handle = u64;
    enum terminal-origin { pseudo, created, profile, user }
    enum status-state { idle, working, blocked, done, error }
    record terminal-status {
        state: status-state,
        label: string,                // at most 64 bytes
        detail: option<string>,       // at most 512 bytes
        progress: option<u8>,         // 0..=100
        attention: bool,
    }
    record terminal-info {
        handle: terminal-handle,
        origin: terminal-origin,
        title: string,
        cwd: option<string>,
        status: option<terminal-status>,
        account: option<string>,      // the account profile's name, never its path
    }
    record pseudo-request { title: string, cols: option<u16>, rows: option<u16>, in-panel: bool }
    record create-request { program: string, args: list<string>, cwd: option<string>, title: option<string> }
    enum read-scope { last-command, last-lines }

    open-pseudo: func(request: pseudo-request) -> result<terminal-handle, host-error>;   // terminal:pseudo
    write-output: func(handle: terminal-handle, bytes: list<u8>) -> result<_, host-error>;
    create: func(request: create-request) -> result<terminal-handle, host-error>;        // terminal:create
    send-input: func(handle: terminal-handle, bytes: list<u8>) -> result<_, host-error>;
    close: func(handle: terminal-handle);
    set-status: func(handle: terminal-handle, status: option<terminal-status>) -> result<_, host-error>;
    %list: func() -> result<list<terminal-info>, host-error>;                            // terminal:observe
    paste: func(handle: terminal-handle, text: string) -> result<_, host-error>;         // gesture
    request-read: func(handle: terminal-handle, scope: read-scope, lines: u16) -> result<u64, host-error>;  // T3, gesture
    request-write: func(handle: terminal-handle, text: string, submit: bool) -> result<u64, host-error>;    // T3, gesture
}

/// 0.4. Loopback listeners and MCP (§B.7).
interface listeners {
    use host.{host-error};
    record tool-def { name: string, description: string, input-schema: string }
    record mcp-call { call-id: u64, listener: string, connection: u64, tool: string, arguments: string }
    record mcp-error { code: s32, message: string }
    /// Delivered once per connection after `initialize` and `notifications/initialized`.
    record mcp-session { listener: string, connection: u64, client-name: string, client-version: string }
    register-tools: func(listener: string, tools: list<tool-def>) -> result<_, host-error>;
    respond: func(call-id: u64, outcome: result<string, mcp-error>) -> result<_, host-error>;
    notify: func(listener: string, connection: option<u64>, method: string, params: string) -> result<_, host-error>;
    send: func(listener: string, connection: u64, text: string) -> result<_, host-error>;
    reply: func(request-id: u64, status: u16, body: string) -> result<_, host-error>;
}

/// 0.4. Command-hook relay (§B.9).
interface relay {
    use host.{host-error};
    respond: func(id: u64, exit-code: u8, stdout: string, stderr: string) -> result<_, host-error>;
}

/// 0.4. Proposed edits through agent review (§B.10).
interface review {
    use host.{host-error};
    use types.{file-ref};
    variant proposed { new-content(string), unified-diff(string) }
    record proposal { path: file-ref, change: proposed, title: string, session: string, apply: bool }
    enum decision { accepted, partial, rejected }
    record review-outcome { decision: decision, final-content: string }
    propose: func(request: proposal) -> result<u64, host-error>;
    withdraw: func(id: u64) -> result<string, host-error>;     // the current text; no review-decided
    begin-turn: func(session: string) -> result<_, host-error>;
    end-turn: func(session: string, changed: list<file-ref>) -> result<_, host-error>;
}

/// 0.4. Editor context and opening files (§B.11).
interface context {
    use host.{host-error};
    use types.{file-ref, text-range};
    record editor-context {
        active: option<file-ref>,
        open: list<file-ref>,                                   // most recently focused first
        trusted: option<bool>,                                  // the worktree's trust
        selection: option<tuple<text-range, option<string>>>,   // text absent for private files
    }
    watch: func(on: bool) -> result<_, host-error>;                                      // context:editor

    /// 0.4. Diagnostics (§B.11, `workspace:diagnostics`, T1).
    enum severity { error, warning, information, hint }
    record diagnostic {
        range: text-range,                                      // zero-based, UTF-8 byte columns
        severity: severity,
        message: string,                                        // cut at 2 KiB
        source: option<string>,                                 // the language server, e.g. "rust-analyzer"
        code: option<string>,                                   // numeric codes as text
    }
    record file-diagnostics { path: file-ref, diagnostics: list<diagnostic> }
    record diagnostics-snapshot {
        files: list<file-diagnostics>,                          // only files with at least one entry
        truncated: bool,                                        // more than 500 entries; errors kept first
    }
    /// `some` = that file (private: not-permitted; in no worktree: invalid-argument);
    /// `none` = every worktree file with diagnostics (private files left out).
    diagnostics: func(path: option<file-ref>) -> result<diagnostics-snapshot, host-error>;
    watch-diagnostics: func(on: bool) -> result<_, host-error>;                          // workspace:diagnostics
    open: func(path: file-ref, range: option<text-range>, focus: bool) -> result<_, host-error>;  // workspace:open
    to-file-ref: func(abs-path: string) -> result<file-ref, host-error>;                 // context:editor
    absolute-path: func(path: file-ref) -> result<string, host-error>;                   // context:editor
}

/// 0.4. Approval cards (§B.12).
interface approvals {
    use host.{host-error};
    enum approval-option { allow-once, always-allow, deny }
    ask: func(title: string, detail: string, options: list<approval-option>) -> result<u64, host-error>;
}

// io-event gains (0.4):
//   terminal-input(tuple<u64, list<u8>>), terminal-resized(tuple<u64, u16, u16>),
//   terminal-output(tuple<u64, list<u8>, u64 /* dropped bytes */>), terminal-exited(tuple<u64, option<s32>>),
//   terminal-created(terminal-info), terminal-closed(u64), terminal-status-changed(terminal-info),
//   terminal-read-done(tuple<u64, result<string, string>>), terminal-write-done(tuple<u64, result<_, string>>),
//   mcp-call(mcp-call), mcp-cancelled(u64), mcp-initialized(mcp-session), listener-message(tuple<string, u64, string>),
//   listener-request(tuple<u64, string, string, string> /* id, listener, path, body */),
//   relay-request(tuple<u64, string, option<u64>, list<string>, string>),
//   review-decided(tuple<u64, result<review-outcome, string>>),
//   review-comments(review-comments) /* plan 7.2, §B.10; record in review */, context-changed(editor-context),
//   diagnostics-changed(list<file-ref> /* empty = anything may have changed */),
//   approval-answered(tuple<u64, option<approval-option>>)
//
// permissions: t2-capability gains network-listen, terminal-create, terminal-env, terminal-observe,
// fs-discovery-file, relay-hooks; a new enum t3-capability { terminal-read, terminal-write } is
// answered per request, not by query().
```

Extension settings (§D), for the same staged world:

```wit
/// 0.4. The extension's own settings (design.md §D). No capability.
interface settings {
    use host.{host-error};

    variant setting-value {
        boolean(bool),
        integer(s64),
        number(f64),
        text(string),
        choice(string),
        text-list(list<string>),
    }
    record setting { key: string, value: setting-value, is-default: bool }

    get: func(key: string) -> result<setting-value, host-error>;
    all: func() -> list<setting>;
    open: func(key: option<string>) -> result<_, host-error>;
}

// io: `use settings.{}` is not needed (keys are strings); io-event gains, after
// approval-answered:
//   /// 0.4. The keys whose effective value changed, in manifest order.
//   settings-changed(list<string>)
// world ui-extension: `import settings;` after `import approvals;`.
```

The manifest gains `[[fyx_ui.terminal_profiles]]`, `[[fyx_ui.terminal_programs]]`,
`[fyx_ui.terminal_env]`, `[[fyx_ui.listeners]]`, `[[fyx_ui.discovery_files]]`,
`[[fyx_ui.relay_channels]]` and `[[fyx_ui.settings]]` (§D.1). Each T2 item takes a `reason`. All of them are refused for a world
below 0.4 (`world_capabilities`, integration-5 A4), and `fyx-ext` and the harness mirror the
lists.

## The first extensions

**`fyx_claude_code`** (plan 5.15, W4):
1. Profile "Claude Code" in "+", with `account_tool = "claude"`; optionally the env in every new
   terminal (the stronger checkbox).
2. `CLAUDE_CODE_PLUGIN_DIRS` → `${package}/claude-plugin`. **All of that plugin's hooks are command
   hooks** through `fyx-relay`, written `"${FYX_RELAY:-true}" --channel fyx-claude-code/<name>`
   (W4's skeleton, 655298641f): Claude Code 2.1.326 accepts `http` hooks only with a literal URL,
   and the listener's port is new every run, so a plugin file cannot name it. Outside Fyx,
   `FYX_RELAY` is unset and each hook runs `true` and exits 0. The extension's `http` listener is
   not used for hooks. The tab state
   already comes from OSC 7501; the hooks add details (the tool, a permission request's text, the
   last message) and the agent turn.
3. `network:listen` (`mcp-websocket`) + `fs:discovery-file` `${account.claude}/ide/${port}.lock` +
   env `CLAUDE_CODE_SSE_PORT`, `ENABLE_IDE_INTEGRATION=true` → the IDE protocol:
   `selection_changed`, `at_mentioned`, `getDiagnostics`, `openFile`, and `openDiff` →
   `review:propose` (blocking). **Print and SDK runs never connect to the IDE:** `claude -p` and
   Agent SDK runs open no WebSocket even with `--ide`; they get hooks only, so turns, status and
   review for them come from hooks (item 4 and §B.10). The IDE protocol is for interactive runs.
   Edge behaviour of 2.1.326: `closeAllDiffTabs` and a duplicate `close_tab` after a decision
   answer without withdrawing (§B.10); `getDiagnostics` gets a file URI in two Windows forms,
   `file:///C:/work/app.py` and `file://C:\work\app.py`, and the extension accepts both (through
   `context.to-file-ref`, §B.11); paths in `openDiff` and `at_mentioned` carry backslashes. `getDiagnostics` answers from
   `context.diagnostics` (§B.11); the extension keeps the last snapshot and refreshes it on
   `diagnostics-changed`, so the four calls a turn are cheap and stable.
4. Turns: `UserPromptSubmit` → `begin-turn`, `Stop` → `end-turn`. **After the user rejects a
   diff (`DIFF_REJECTED`) Claude ends the turn with no `Stop` hook,** so the extension also ends
   the turn when it answers a rejection. **Review is post-hoc first:** `PreToolUse` and
   `PostToolUse` through `fyx-relay` snapshot the file before and propose the diff after
   (§B.10), because `auto` is now Claude's default permission mode and `openDiff` comes only in
   manual mode. `openDiff` is the bonus path. "Hold edits for review", a blocking `PreToolUse`
   hook where Fyx allows or denies under a timeout, is an optional setting and a 5.15 follow-up.
5. **Compat test:** a recorded protocol transcript from a pinned Claude Code version, replayed
   against the extension in the harness; a scheduled manual check against the newest CLI before
   each release. Known gaps: in `acceptEdits` and `auto` modes (and in print and SDK runs) Claude
   writes without `openDiff`, which the post-hoc path covers after the fact; the protocol is
   reverse-engineered [RE].

**`fyx_gemini_cli`** (plan 5.16, W4; skeleton 81be13f34b): the documented IDE companion spec over
`mcp-streamable-http` (path `/mcp`) with a Bearer token, the discovery file in
`${tmp}/gemini/ide/gemini-ide-server-${pid}-${port}.json` (typed content, §B.8),
`GEMINI_CLI_IDE_SERVER_PORT` and `GEMINI_CLI_IDE_PID = "${pid}"`, `ide/contextUpdate` from
`context:editor`, and non-blocking `openDiff` → `ide/diffAccepted` / `ide/diffRejected` from
`review:propose`, with `closeDiff` through `review.withdraw`. **Pinned to Gemini CLI 0.69.0** by a
compat test (`src/compat/README.md` in the extension); where the CLI's source and the companion
spec differ, the source wins. Writing it found the gaps that became §B.4's `${pid}`, §B.7's
`/mcp`, §B.8's typed content and folder-owner check, §B.10's `withdraw` and §B.11's `trusted` and
order: this is the proof that the kit is general, and each primitive the Claude extension did not
need was added before 0.4 freezes. **Its hooks are deferred past the skeleton.** Gemini has no
plugin-dir env like `CLAUDE_CODE_PLUGIN_DIRS`, only `GEMINI_CLI_SYSTEM_DEFAULTS_PATH`, which names
a settings file. The extension will ship that file in its package (`${package}/…`) and set the
variable in `terminal:env` with `scope = "profiles"`, so only its profile terminals read it, and
the dialog marks it "makes Gemini CLI load this extension's settings". Until then Gemini's edits
outside `openDiff` show as "changed without review" and there are no agent turns. Known
limitation: Gemini's Docker sandbox is refused by the `Host` check (§B.7).

## Changes by component

| Area | Owner | Files |
|---|---|---|
| Terminal foundation, terminal host for extensions, relay | W1 | new `crates/terminal_ext` (TeePty, OSC parsers, `TerminalDecorations`, registry, env hook, pseudo-terminal bridge, sanitiser), new `crates/fyx_relay` (binary and endpoint); hooks in `alacritty.rs:214`, `terminal.rs:656`, `terminal_view.rs:1612`; shell-integration scripts in `assets/fyx/shell/` |
| Terminal host trait | W2c (trait), W1 (implementation) | `ui_ext::terminal::TerminalHost` (W2c) implemented by `terminal_host` (W1). Besides the per-terminal calls it has **`set_integration(ext, profiles, env, env_all_new, package_dir)`**: the extension's accepted terminal profiles (§B.1), its `terminal:env` vars (§B.4), whether the stronger "all new terminals" box was ticked, and the folder `${package}` resolves to (§B.6). `ui_ext_host` calls it on load and on every consent change, and `close_all(ext)` clears it on reload, disable, uninstall and revocation |
| WIT model, manifest, permissions broker, listeners, review bridge, context | W2c | `crates/ui_ext` (0.4 model, manifest tables, forbidden names of §B.4/§C.6), `crates/ui_ext_wasm` (0.4 bindings, `listeners.rs`, `relay.rs` host side), `crates/ui_ext_host/src/permissions.rs` and `consent.rs` (T2-at-install states, T3, journal store) |
| Consent UI, terminal node, sessions panel, cards | W3 | `ui_ext_host`: install dialog with checkboxes, extension page switches and journal, update diff, T3 cards and "Always allow", `terminal` node in the panel container, agent sessions panel, approval cards |
| SDK, harness, demos, first extensions | W4 | `ui_ext_sdk` 0.4 module, `fyx_ext_harness` fakes for terminals, listeners, relay and review; `extensions/fyx_demo_pseudo_terminal`, `extensions/fyx_claude_code`, `extensions/fyx_gemini_cli` |
| Staged WIT | W0 | `crates/ui_ext/wit/since_v0.4.0/fyx-ui.wit` |
| Registry template | W10 | `tools/fyx-extensions-template` `policy/capability-tiers.json` (T3 counts as T2 for the release tier, so `fyx_registry`'s signed `tier` gains no value), the diff in `registry.py comment` |

## Tests

- **Foundation:** OSC 7501 from a recorded Claude Code session sets the tab state; the support
  query gets a reply; a 9 KiB OSC is dropped; 1000 OSCs per second are capped; the tap passes the
  bytes to alacritty unchanged (byte-for-byte replay of a session with and without the tap).
- **Pseudo-terminal:** OSC 52, DA and DSR in extension output never reach the grid and never
  produce a reply; a 100 MiB write is refused per call and rate-limited; resize events coalesce.
- **Created terminal:** an undeclared program fails; reload kills the tree; the tab shows
  "started by"; an untrusted worktree refuses it.
- **Env:** a forbidden name (`PATH`, `HOME`, `LD_PRELOAD`, `CLAUDE_CONFIG_DIR`, `FOO_API_KEY`)
  refuses the manifest; `scope = "all-new"` with the box unticked reaches profile terminals only;
  the extension never sees `${listen.*.token}` or `${account.*}` values.
- **Listener:** a wrong token, an `Origin` header and a foreign `Host` are refused before any
  `io-event`; a blocking MCP call is cancelled when its connection closes.
- **Discovery file:** a template that names `settings.json`, contains `..` or crosses a symlink
  is refused; the file is `0600` (ACL on Windows) and goes away on revoke, exit and stale pid.
- **Relay:** without a token it exits with `on_failure`; with Fyx closed it exits 0 at once; a
  timeout gives `on_failure`; the answer's exit code passes through.
- **Review:** a blocking proposal returns the final content after a partial accept; `apply =
  true` is one undo; a protected path is refused.
- **Consent:** an unticked T2 item is `denied`; ticking it later asks once with the 1 s guard; an
  update with a new program is held and its diff lists it; revoke kills, closes and cleans up;
  T3 cards ignore clicks for 1 s and keys always; "Always allow" resets on a new component hash.
- **Accounts:** an extension cannot read an account dir's path; a limit message never offers
  another account; the per-project account is not read from the repository.

## Threats

| Threat | Answer |
|---|---|
| An extension reads everything in the user's terminals | No global stream; T3 per terminal; command lines redacted for `terminal:observe`; "Always allow" is loud, journaled and hash-bound |
| An extension types into the user's shell | T3 write with the exact text confirmed per command; no control keys; paste only in bracketed mode |
| A terminal reply carries output back into input | Sanitiser strips every answered sequence; replies are never built from output (research §4) |
| A web page or LAN host reaches a listener | `127.0.0.1` only, token, `Origin` refused, `Host` checked |
| A discovery file overwrites a tool's config | Name must contain `${port}`/`${pid}`, `.lock`/`.json`, no symlinks, host-written only |
| Env changes what the user's shell runs | Loaders, `PATH`, `HOME`, `XDG_*`, `ZDOTDIR` refused; profile scope by default; the exact list is ticked by the user |
| An extension impersonates Claude Code on a tab or a card | The source and provenance are host-drawn; the tool's name on a card is marked "says" |
| Account abuse (rotation, limit evasion) | §C.6 rules 1–6 in the product; auth and config-dir names refused for extensions |
| Prompt fatigue at install | Grouped dialog; T2 unticked; T3 never in advance |
| A relay hook blocks the user's tool when Fyx is down | Fails open by default; a closed channel is shown at install |

## Not in 0.4

| Item | Why |
|---|---|
| A terminal emulator in WASM (canvas grid) | Research §2; a byte-stream backend (§B.2) covers it |
| Raw access to every terminal's bytes | VS Code's lesson (research §4); T3 reads give text, once |
| Clipboard writes from a pseudo-terminal (OSC 52) | A separate T2 later |
| Declarative ACP agents in the agent panel | Research layer B; no consumer in P5; after the Gemini extension |
| SSH and remote terminals for listeners and relay | Phase 6 (5.17): loopback from a remote machine needs a tunnel |
| Named pipes and Unix sockets for listeners | Loopback TCP on every OS ([Decisions](#decisions-2026-11-19)) |
| Variant B Fyx profiles (in one process) | §C.6 |
| Per-project extension settings | §D.2: a repository must not change an extension's behaviour; later, as a user-level "for this project" value, like the per-project account |
| An extension writing its own settings | §D: the user owns the values; an extension that needs state gets storage, a separate design |
| Secret settings (tokens, passwords) | §D.1: `settings.json` is plain and shared; credentials stay with the tool |
