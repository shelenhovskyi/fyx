# Fyx user guide

This guide is for people who install and use Fyx. If you want to write a UI extension, read
[Writing a Fyx UI extension](extension-authoring.md) instead.

It describes `origin/integration` as of 2026-11-18. Features that are built on a stream's branch
but not merged into `integration` yet are marked **(coming)**.

- [What Fyx is](#what-fyx-is)
- [Privacy defaults](#privacy-defaults)
- [Reviewing agent changes](#reviewing-agent-changes)
- [UI extensions](#ui-extensions)
- [Security modes for agents (coming)](#security-modes-for-agents-coming)
- [Where Fyx keeps its files](#where-fyx-keeps-its-files)
- [Troubleshooting](#troubleshooting)

<!-- Base: origin/integration 571c6826a5. (coming) sources: claude/fyx-w1-rebrand-904c67,
claude/fyx-w3-renderer-e20d2b, the W5 commits 29999cd5f3 and a98221e345 (carried on the W1 and W4
branches), claude/fyx-w9-guard, claude/fyx-w9-fixes. -->

## What Fyx is

Fyx is a code editor forked from [Zed](https://zed.dev). Editing, languages, Git, terminals,
themes, keymaps and the agent panel work as they do in Zed. Fyx adds the following:

- **Review of agent changes.** When review mode is on, edits an AI agent proposes wait until you
  accept or reject them, hunk by hunk.
- **UI extensions.** Sandboxed WASM extensions can add dock panels and status bar items. They
  never draw anything themselves: Fyx renders them with its own components and your theme.
- **No Zed cloud.** Fyx does not sign in to zed.dev, collaborate, send telemetry or check for
  updates. See [Privacy defaults](#privacy-defaults).
- **(coming) Security modes.** Optional modes keep private files, secret environment variables
  and agent config files out of agents' reach.

Things that differ from Zed:

- **Own name and folders.** The app is called Fyx and the binary is `fyx.exe`. Fyx uses its own
  config and data folders, so it never reads or changes a Zed installation, and the two can run
  side by side. The URL scheme is `fyx://`.
- **No zed.dev account.** There is no sign-in button, onboarding page, collaboration panel or
  hosted Zed models.
  - The agent's default model is Anthropic's `claude-sonnet-5-5` with your own API key. Pick any
    other provider you have configured.
  - Edit predictions are off (`"provider": "none"`), because Zed's provider needs an account.
    Bring your own provider.
- **No Zed extension registry.** Fyx does not install extensions from Zed's registry or suggest
  them. Zed extensions you install as dev extensions still work.
- **Dev builds only.** Fyx is a prototype, built and tested on Windows 11 only. There are no
  releases, installers or signed binaries yet. To build it, see the [README](../../README.md).

<!-- assets/settings/fyx.json; crates/paths/src/paths.rs (APP_NAME = "Fyx"); tasks/progress/W1.md -->

## Privacy defaults

Fyx's defaults are in `assets/settings/fyx.json`. They are layered over Zed's defaults, and you can
override any of them in your settings.

| What | Fyx default | Effect |
|---|---|---|
| `server_url` | `"http://127.0.0.1:0"` | Every zed.dev address (sign-in, collaboration, extension registry, telemetry upload, update checks) is built from this value. It points nowhere, so these requests fail before anything is sent. |
| `telemetry.diagnostics`, `telemetry.metrics` | `false` | No telemetry and no crash reports. |
| `auto_update` | `false` | No update checks. Fyx has no update feed. |
| `edit_predictions.provider` | `"none"` | No edit prediction requests. |
| `auto_install_extensions`, `suggest_extensions` | off | Nothing is installed or suggested from a registry. |

<!-- assets/settings/fyx.json -->

To use Zed's services anyway, set `"server_url": "https://zed.dev"` in your settings. Zed's terms
then apply. The `ZED_SERVER_URL` environment variable also overrides the setting.

### Network at startup

On a fresh profile, with no folder open and no AI keys set, Fyx makes no connection outside your
machine at startup. This was measured on Windows 11. Fyx also does not fetch the ACP agent
registry at startup. It fetches the registry when you open the registry page or configure a
registry agent.

<!-- tasks/progress/W1.md "Verification"; crates/project/src/agent_registry_store.rs (fyx:) -->

Fyx still connects in these cases. None of them goes to zed.dev.

- **A folder is open and an AI key exists** in the environment or the keychain. The agent then
  authenticates each provider at startup, and some providers download their model list with
  your key (Anthropic, Google, OpenRouter, Vercel, OpenCode).
  - **(coming)** Providers will start only when you first open the model picker or send a prompt.
    A first prompt waits up to 20 s for a model.
- **`GH_COPILOT_TOKEN` or `GITHUB_COPILOT_TOKEN` is set.** Copilot Chat downloads its models from
  GitHub at startup.
  - **(coming)** It will download them when you first use the provider.
- **Anthropic usage events** (`/v1/log/zed`) are sent with Anthropic requests, even with
  telemetry off.
  - **(coming)** They will follow `telemetry.metrics`, so Fyx will not send them.
- **LM Studio and llama.cpp** are probed on `localhost:1234` and `localhost:8080`. These
  connections stay on your machine.
  - **(coming)** The probes will be removed.
- **When you open a file:**
  - Language servers check GitHub or npm for a newer version, once per server per launch.
  - The default Prettier is installed from npm.
  - Git blame hovers load author avatars.

<!-- tasks/reviews/network-audit-1.md "Verdict" and "Still phones home"; (coming): 4b83f766d2 on claude/fyx-w1-rebrand-904c67 -->

To turn off every AI feature, including these requests, set `"disable_ai": true`.

## Reviewing agent changes

When review mode is on, Fyx holds an agent's file edits until you have decided on each hunk. Fyx
decides what lands: the agent only proposes, and no extension can decide for you. The full
reference is [Agent review](agent-review.md), and the rules are in [Review rules](review-rules.md).

### Turning it on

Review mode is off by default. To turn it on, use any of these:

- In the agent panel's options menu, click **Review Agent Edits**.
- Run the command `agent_review: toggle review mode`.
- Add this to your user settings:

  ```json
  "agent_review": { "enabled": true }
  ```

The `FYX_AGENT_REVIEW` environment variable overrides the setting: `0` turns review off, and any
other value turns it on.

Only your user settings can turn review off. A project's settings cannot do it.

Turning review off does not release edits that are already waiting. They still need a decision.

<!-- crates/agent_review/src/agent_review.rs: REVIEW_MODE_VAR, "Review Agent Edits" menu entry; review_pane.rs: ToggleReviewMode -->

### What is reviewed

| Where the edit comes from | Reviewed? |
|---|---|
| An ACP agent that asks permission and sends a diff, such as Claude Agent | Yes. Your decisions answer the permission request. |
| An ACP v2 agent whose permission request carries a patch for one file | Yes, against the file as it is now. |
| An agent that writes through Fyx (`fs/write_text_file`) | Yes. The write waits until you submit, and only accepted hunks land. |
| Zed's built-in agent | No. Its edits land as in Zed. |
| An ACP v2 edit that Fyx cannot show as a diff: several files in one request, no patch text, or a patch that does not apply | No. A notification says so. Decide in the agent panel. |
| Shell and terminal commands (`sed -i`, `cat >`, scripts) | No. Nothing is blocked, but the turn's review pane lists the changed files under "changed without review". |

A short version of this table is shown under every review pane, and the status bar item's tooltip
shows the long version.

**(coming)** Review of the built-in agent:

- Its edit and write tools will be reviewed. Fyx will hold the edit before saving it, keep only
  accepted hunks and tell the agent about the rest.
- Its delete, move and copy tools will still ask permission, as in Zed.

<!-- crates/agent_review/src/agent_review.rs: COVERAGE, NOT_REVIEWED; (coming) 29999cd5f3 -->

### The review pane

Fyx opens one review pane per agent turn. The pane lists the files of the turn and shows one diff
at a time. Each hunk has its own **Accept** and **Reject** buttons, and a comment field. The header
counts what you have decided and what is still waiting.

These keys work while the pane has focus:

| Keys | Action |
|---|---|
| `alt-y` / `alt-n` | Accept / reject the hunk |
| `shift-alt-y` / `shift-alt-n` | Accept / reject the whole file |
| `ctrl-shift-y` / `ctrl-shift-n` | Accept / reject everything in the turn, including edits the turn makes later |
| `alt-j` / `alt-k` | Next / previous hunk |
| `alt-c` | Comment on the hunk |
| `escape` | Leave the comment field |
| `ctrl-enter` | Submit (every hunk must be decided) |

<!-- crates/agent_review/src/review_pane.rs: review_keys -->

### Decisions for each turn

- **Accept all or reject all for the turn.** `ctrl-shift-y` or `ctrl-shift-n` also decides the
  turn's later edits. The pane header then shows "accepting the rest of the turn" or "rejecting
  the rest of the turn". Files that a rule marks `always_ask` are still held for review.
- **What the agent hears back.**
  - If the agent wrote through Fyx, your comments and the hunks you rejected go back to it: the
    write fails with a message that lists them.
  - An ACP permission answer carries no message, so an agent that asked permission only learns
    "allowed" or "rejected".
- **Your own unsaved edits.** If the file has unsaved edits of yours, Fyx applies the accepted
  hunks but does not save the file.
- **The text moved.** If Fyx cannot apply the accepted hunks, for example because the text moved
  in the meantime, the review stays open with the reason. Submit again, or reject the file.
- **Cancelled edits.** Waiting edits are cancelled when the agent's turn stops, its connection
  drops, or you close the last pane of the turn. The agent is told the file is unchanged.
- **(coming) Permission cards.** While Fyx reviews an edit that came with a permission request,
  the request's card in the agent panel will say so. It will offer **Open review** instead of
  Allow and Reject.

<!-- (coming) 29999cd5f3 -->

### The status bar

The status bar item is shown while review mode is on or a review is waiting:

- **Review on**: review mode is on and nothing is waiting.
- **Review: N waiting**: N reviews need a decision. Click it to bring them forward. The command
  `agent_review: open reviews` does the same.

A review can wait with review mode off, for example one that an `always_ask` rule opened.

<!-- crates/agent_review/src/review_pane.rs: status item labels, OpenReviews -->

### "Changed without review"

An agent can change files without going through Fyx, for example with a shell command. Files that
change on disk during the agent's turn this way are listed in the turn's review pane, under
**N files changed without review**. Click a file to open it: its Git gutter shows the change.
Nothing is blocked or undone.

<!-- crates/agent_review/src/review_pane.rs: "changed without review", "Open the file; its git gutter shows the change" -->

### Review rules

Rules live in `.fyx/review.toml` at the root of a worktree. All three keys are optional:

```toml
# .fyx/review.toml
auto_accept = ["docs/**", "*.md"]          # lands without review
always_ask  = ["Cargo.lock", ".github/**"] # always reviewed
deny        = ["*.env", "secrets/**"]      # never written by an agent
```

What each kind does:

- **`deny`**: the file is never written by an agent. The write fails with a message naming the
  rule, and a permission request is answered "reject".
- **`always_ask`**: the file is always reviewed, even after "accept all in the turn".
- **`auto_accept`**: the edit lands without review.

How the rules apply:

- `deny` and `always_ask` apply even with review mode off. `auto_accept` applies only while review
  mode is on.
- `auto_accept` applies only in worktrees you trust: the ones Zed's restricted mode lets you
  trust, or all of them when `session.trust_all_worktrees` is on. The file comes with the
  repository, so a cloned project could otherwise wave agent edits through.
- When several rules match a file, the first kind in this order wins: `deny`, `always_ask`,
  `auto_accept`. With the file above, `docs/security.md` lands without review. Add it to
  `always_ask`, and it is always reviewed.
- A pattern without `/` matches the file name in any folder: `*.env` matches `config/prod.env`.
- A pattern with `/` matches the path from the worktree root: `docs/**` matches
  `docs/guide/intro.md`.
- Each decision a rule made is listed in the review pane with its rule, for example
  "by rule `deny = "*.env"` in …/.fyx/review.toml".

How Fyx loads the file:

- Fyx reads the file when an agent thread starts in the project.
- **Changes while agents run.** If the file changes while agent sessions are running, the new
  rules do not apply by themselves, because an agent may have written them. A notification
  offers **Apply new review rules**. Until you click it, the previous rules stay in force.
- **Bad file.** If the file does not parse (bad TOML, an unknown key, a bad pattern), Fyx ignores
  the whole file and shows a notification with the error.

<!-- crates/agent_review/src/review_rules.rs (trusted, auto_accept); agent_review.rs: "Apply new review rules" -->

**(coming)** The project file will only make review stricter:

- **`auto_accept` moves to your user settings**, `agent_review.auto_accept`. In
  `.fyx/review.toml`, it will be ignored with a notification.

  ```json
  "agent_review": {
    "enabled": true,
    "auto_accept": ["docs/**", "*.md"]
  }
  ```

- **Stricter rules** will apply at once. A change that removes a restriction (a pattern dropped,
  or the file deleted) will wait for **Apply new review rules**. This also holds when a new
  thread starts. At start, the file will apply as it is.
- **Bad file.** A file that does not parse will keep the previous rules in force instead of
  dropping them.
- **Matching.** Rules will match the resolved path of the file, ignoring case on Windows and
  macOS.

<!-- (coming) a98221e345 -->

### Not covered yet

- Shell and terminal commands. They are listed as "changed without review", but not held.
- Zed's built-in agent. **(coming)** Its edit and write tools will be reviewed.
- ACP v2 edits to several files at once, or edits without a usable patch.
- Remote projects: `.fyx/review.toml` is read through the local file system only.
- A rejected permission request cannot carry your comments to the agent. This is a limit of ACP.

## UI extensions

A UI extension is a WASM component that adds dock panels or status bar items to Fyx. It describes
what to show (text, buttons, inputs, lists, tables, icons, markdown), and Fyx draws it in its own
style, with your theme. Each extension runs in its own sandbox:

- no files, environment variables or arguments;
- no network: every connection attempt fails with "access denied";
- at most 32 MiB of memory (**(coming)** 256 MiB);
- a time limit on every call. An extension that runs too long or crashes is disabled until you
  restart Fyx. **(coming)** You will be able to restart it in place (see
  [Failures and restarts](#failures-and-restarts)).

**(coming)** Links in an extension's markdown will open only if they are `http(s)://` addresses,
and only after you confirm the address.

UI extensions are separate from Zed extensions (languages, themes, slash commands). Those work as
in Zed.

<!-- crates/ui_ext_wasm/src/ui_ext_wasm.rs: deny_network, UI_MEMORY_LIMIT; docs/fyx/limits-and-gotchas.md; (coming) 52a9103c61, b89ab63da9 -->

### Installing an extension from a folder

1. Run the command `fyx_ui: install dev UI extension`.
2. Pick the extension's folder. Its `extension.toml` must have a `[fyx_ui]` table, and its
   component must be built.
3. Fyx asks **Install the UI extension `<id>`?**. The prompt lists what the extension asks for,
   its component and its folder.
4. Click **Install**.

Fyx links the folder into its installed-extensions directory and loads the extension. The
extension loads again on every start. When you rebuild its component, Fyx reloads it.

Every UI extension in the installed-extensions directory loads at start, including folders copied
there by hand or linked by Zed's **Install Dev Extension**.

To remove the extension, uninstall it on the Extensions page.

<!-- crates/ui_ext_host/src/install.rs: InstallDevUiExtension, prompt_and_install; registry.rs: scan_installed, hot reload of linked dev extensions -->

### The consent prompt (coming)

Fyx will load an extension from the installed-extensions directory only after you have enabled it
as it is now. The install command will record your consent when you click **Install**. Anything
else that appears there will wait for you, and Fyx will show a notification:

> The UI extension `<id>` is installed but not enabled. It asks for: …

- **Enable** loads the extension and remembers your answer.
- **Not now** leaves it unloaded until the next start.

Fyx will ask again when the extension asks for more capabilities. It will also ask again when its
component changes, unless the extension is installed from a folder: then any build in that folder
counts as enabled. Answers will be kept in `fyx-ui-consent.json` in the data folder.

The same change will refuse folders whose path contains `&`, `|`, `^`, `<`, `>`, `%`, `!` or `"`.

<!-- (coming) 6ecd47980f on claude/fyx-w3-renderer-e20d2b: consent.rs, registry.rs offer_consent -->

### Capability chips

On the Extensions page, the card of a loaded UI extension shows what it uses:

- **`ui:panel`**: it adds dock panels.
- **`ui:status-bar`**: it adds status bar items.

<!-- crates/ui_ext_host/src/ui_ext_host.rs: capability_chips; crates/extensions_ui/src/components/extension_card.rs (fyx:) -->

### Panels and status items

Each panel gets a button in its dock. To open a panel from the keyboard, bind the action
`fyx_ui::ToggleExtensionPanel` with the panel's key, `"<extension id>/<surface id>"`:

```json
{ "bindings": { "ctrl-alt-1": ["fyx_ui::ToggleExtensionPanel", { "key": "fyx-demo-counter/counter" }] } }
```

Inside a panel:

- `tab` and `shift-tab` move between controls.
- `enter` or `space` presses a button.
- The arrow keys, `home`, `end`, `pageup` and `pagedown` move through lists.
- `escape` returns to the editor.

<!-- crates/ui_ext_host/src/ui_ext_host.rs: ToggleExtensionPanel; keyboard.rs -->

### Loading extensions with an environment variable

For quick tests, set `FYX_UI_DEV_EXTENSIONS` to a list of extension folders before you start Fyx.
Separate the folders with `;` on Windows and `:` elsewhere. Fyx reads the variable once, at
startup, and reloads these extensions when you rebuild them. They load without any prompt:
setting the variable is your consent.

```bat
set FYX_UI_DEV_EXTENSIONS=C:\src\fyx\extensions\fyx_demo_counter;C:\src\fyx\extensions\fyx_demo_todo
fyx.exe --foreground
```

<!-- crates/ui_ext_host/src/registry.rs: DEV_EXTENSIONS_ENV, watch_component -->

### Failures and restarts

Today, an extension that crashes or runs past its time limit is disabled until you restart Fyx.
Some errors offer **Retry**.

**(coming)** Restarting in place:

- **Restart extension.** A stopped panel or status item will show the reason and
  **Restart extension**, which loads the extension again on a fresh instance. The command
  `fyx_ui: restart UI extension` does the same for any loaded extension.
- **A rebuild that does not load.** If a rebuilt component does not load, the old one will keep
  running. The surface's footer will show the error with **Retry**. The next good build will load
  by itself and clear the error.
- **Panels stay put.** A reload will remount the panels in place: they stay docked and open.

<!-- crates/ui_ext_host/src/view.rs: "Retry"; (coming) 0004efba01 on claude/fyx-w3-renderer-e20d2b -->

### Dev folders in settings (coming)

`fyx_ui: install dev UI extension` will add the folder to a setting instead of linking it. The
folder stays where it is:

```json
"fyx": { "extensions": { "dev_paths": ["C:/src/my_counter"] } }
```

- `fyx_ui: remove dev UI extension` takes a folder off the list. You can also edit the setting.
- A folder you add to the setting by hand needs one **Enable**.

<!-- (coming) db6619b9f8 on claude/fyx-w3-renderer-e20d2b -->

### Packages, updates, rollback and pinning (coming)

A `.fyxext` file is a packaged UI extension. It is a zip of `extension.toml`, the component and
optional assets.

| Command | What it does |
|---|---|
| `fyx_ui: install UI extension from file` | Shows the package's id, version and capabilities, marked "Unsigned, not from the registry". Then installs it. Installing another version of the same id updates it. |
| `fyx_ui: roll back UI extension` | Goes back to the version that the last update replaced. Run it again to undo the rollback. |
| `fyx_ui: toggle pin UI extension` | Keeps an extension on its version. Other versions are refused until you unpin it. |
| `fyx_ui: uninstall UI extension` | Removes every version. |

- Fyx keeps the current version and the one it replaced. Older versions are deleted.
- On the first load, Fyx compiles the component and saves the result, so later starts are faster.
- A package is refused if it is over 50 MiB, its component is over 32 MiB, or it unpacks to more
  than 128 MiB or 1024 files. It is also refused if any file inside tries to escape the package
  folder.
- Signed packages from a Fyx registry come later.

<!-- (coming) d74c8bb2ac on claude/fyx-w3-renderer-e20d2b: install_store.rs, store_actions.rs -->

## Security modes for agents (coming)

None of this is on `integration` yet. It is built on `claude/fyx-w9-guard`, and the fixes from its
review are on `claude/fyx-w9-fixes`.

Agents run with your user's rights. A prompt injection in a README, an issue or a web page could
make an agent read your secrets, send them out, or write to an agent config file so that code runs
later. Security modes will reduce that risk. The design and the full rules are on the W9 branch,
in `docs/fyx/security-modes.md`.

Every mode will be **off by default**. Only your user settings will be able to change the mode: a
project's settings have no such key.

```json
"fyx_security": {
  "mode": "audit",                // "off" (default), "audit" or "shield"
  "allow_env": ["GITHUB_TOKEN"]   // secret variables agent terminals may keep
}
```

| Mode | What it does |
|---|---|
| `off` | Nothing. Fyx behaves like Zed. |
| `audit` | Blocks nothing. Logs each action that `shield` would block, as `fyx_guard audit: would block …`. |
| `shield` | Blocks the actions below and logs each block as `fyx_guard: blocked …`. |

<!-- claude/fyx-w9-guard: crates/fyx_guard/src/fyx_guard.rs (SecurityMode, log prefixes) -->

Under `shield`:

- **Private files.** ACP agents cannot read files that match Zed's `private_files` setting through
  Fyx (`.env*`, `*.pem`, `*.key` and the others in that setting). Zed already applies this
  setting to the built-in agent.
- **Agent configs.** Agents cannot write `.claude/`, `.codex/`, `.zed/`, `.fyx/`, `.mcp.json` or
  `.git/hooks/`, at any depth. This covers ACP agents that write through Fyx, and the built-in
  agent's edit and write tools.
- **Secret variables.** In agent terminals, secret variables are set to empty, for example
  `AWS_*`, `GITHUB_TOKEN`, `GH_TOKEN`, `*_API_KEY`, `*_TOKEN`, `*_SECRET*` and `SSH_AUTH_SOCK`.
  Without `SSH_AUTH_SOCK`, `git push` over SSH stops working in agent terminals. To keep a
  variable, add it to `allow_env`.
- **Project additions.** A project can add patterns in `.fyx/security.toml`, but never relax
  anything:

  ```toml
  private_files = ["config/credentials/**", "*.p12"]
  read_only     = ["infra/**", "Makefile"]
  ```

- **Fail closed.** If this file cannot be read or parsed, agents cannot read or write in that
  worktree. An unknown `mode` value counts as `shield`.

A blocked action fails with a message that tells the agent which rule blocked it and not to
retry. A write that shield blocks never opens a review pane.

We recommend running `audit` for a while first. Read the log, add the variables you need to
`allow_env`, then switch to `shield`.

**Shield is not a sandbox.** The first version (`claude/fyx-w9-guard`) also has gaps that its
judge review asks to fix. The fixes are on `claude/fyx-w9-fixes`. Until they are merged:

- The built-in agent's move, copy, delete and create-folder tools are not checked.
- `.git/config` is not protected, and neither is Fyx's own config folder.
- An agent's terminal can edit your settings file and lower the mode, and the change applies
  without asking you. With the fixes, Fyx asks you to confirm before it lowers protection.

Limits that stay until a later step:

- External agents' own tools (Claude Agent's `Read` and `Bash`, for example) read the disk directly,
  without asking Fyx.
- A terminal command can still `cat .env` or read `~/.aws/credentials`.
- Paths are checked by name, so a symbolic link to `.claude/` with another name is not caught.

Network control, placeholder secrets and an OS sandbox for agents are planned for later steps.

<!-- claude/fyx-w9-guard: docs/fyx/security-modes.md "Known limits of step 1"; tasks/reviews/w9-step1.md F2-F4; claude/fyx-w9-fixes 3851a95912 -->

## Where Fyx keeps its files

Fyx's folders are separate from Zed's. Fyx is tested on Windows only; the macOS and Linux paths
come from the code.

| What | Windows | macOS | Linux |
|---|---|---|---|
| Settings (`settings.json`), keymap (`keymap.json`), themes | `%APPDATA%\Fyx` | `~/.config/fyx` | `$XDG_CONFIG_HOME/fyx` (usually `~/.config/fyx`) |
| Data: extensions, database, agent threads, external agents | `%LOCALAPPDATA%\Fyx` | `~/Library/Application Support/Fyx` | `$XDG_DATA_HOME/fyx` (usually `~/.local/share/fyx`) |
| Log (`Fyx.log`, previous run in `Fyx.log.old`) | `%LOCALAPPDATA%\Fyx\logs` | `~/Library/Logs/Fyx` | `~/.local/share/fyx/logs` |

Inside the data folder:

| Path | What |
|---|---|
| `extensions/installed/` | Installed extensions, including linked UI extension folders |
| `fyx-ui-consent.json` | **(coming)** Your **Enable** answers for UI extensions |
| `extensions/fyx/` | **(coming)** The UI extension install store: one folder per version, plus `state.json` with the active, previous and pinned version of each extension |

In each project, at the root of a worktree:

| Path | What |
|---|---|
| `.fyx/review.toml` | Project review rules |
| `.fyx/security.toml` | **(coming)** Extra private and read-only paths for shield mode |

`fyx --user-data-dir <DIR>` keeps all data and config in `<DIR>`. Use it for a throwaway profile.

<!-- crates/paths/src/paths.rs: config_dir, data_dir, logs_dir, log_file, extensions_dir; crates/extension_host/src/extension_host.rs: installed_dir -->

## Troubleshooting

**Where is the log?** Run `zed: open log`, or open `Fyx.log` from the table above. Fyx's messages
start with these prefixes:

- `fyx:ui:` for UI extensions. Messages an extension writes itself appear under `fyx_ui`, after
  its id.
- `fyx: agent review:` for review.
- **(coming)** `fyx_guard` for security modes. At start, Fyx will log the active mode, for example
  `fyx_guard: security mode Shield`.

**A UI extension does not appear.**

- Look in the log for `fyx:ui: failed to load …`. Common causes:
  - The `extension.toml` has no `[fyx_ui]` table.
  - The component is not built.
  - The extension targets an API version Fyx does not support.
  - A panel is declared without the `ui:panel` capability.
- Two extensions cannot use the same id. The second one is refused, with an error in the log.
- `FYX_UI_DEV_EXTENSIONS` is read only at startup. Set it in the same shell that starts Fyx.
- **(coming)** Look for a notification asking you to **Enable** it. If you clicked **Not now**,
  restart Fyx to get the notification again.

**A panel shows an error.** An extension that crashes or runs past its time limit is disabled.
Restart Fyx to load it again. Some errors offer **Retry**. **(coming)** Use **Restart extension**
or `fyx_ui: restart UI extension` instead of restarting Fyx.

**Agent edits land without a review.**

- Check the status bar: **Review on** means review mode is on.
- Check whether `FYX_AGENT_REVIEW=0` is set in your environment.
- The edit may come from a path that review does not cover, such as the built-in agent (see
  [What is reviewed](#what-is-reviewed)). Terminal changes show up under
  "changed without review".
- Check `auto_accept` in `.fyx/review.toml`, if the worktree is trusted.

**A rule in `.fyx/review.toml` has no effect.**

- Look for a notification with a parse error. Today, a bad file is ignored as a whole.
- If the file changed while an agent ran, look for **Apply new review rules**.
- `auto_accept` applies only in trusted worktrees and only with review mode on.

**(coming) Shield blocks something you need.** Switch to `audit` and read the
`fyx_guard audit:` lines:

- For a terminal variable, add it to `allow_env`.
- For a file, check your `private_files` setting.

**(coming) `git push` fails in an agent terminal under shield.** `SSH_AUTH_SOCK` is blanked. Add it
to `allow_env` if you want agents to push.

**The agent asks you to sign in or has no model.** Fyx has no zed.dev models. Set an API key for
Anthropic or another provider in the agent panel's settings, or use an external ACP agent.

**Start from a clean profile.** Run `fyx --user-data-dir <empty folder>`. Your normal profile is
left alone.
