# Fyx user guide

This guide is for people who install and use Fyx. If you want to write a UI extension, read
[Writing a Fyx UI extension](extension-authoring.md) instead.

It describes `origin/integration` (64d6c67f63) as of 2026-11-18. Features that are built on a stream's branch
but not merged into `integration` yet are marked **(coming)**.
Features that are designed and approved but not built anywhere yet are marked **(planned)**.

- [What Fyx is](#what-fyx-is)
- [Privacy defaults](#privacy-defaults)
- [Reviewing agent changes](#reviewing-agent-changes)
- [UI extensions](#ui-extensions)
- [Terminals and command-line tools (planned)](#terminals-and-command-line-tools-planned-fyxui-04)
- [Security modes for agents](#security-modes-for-agents)
- [Where Fyx keeps its files](#where-fyx-keeps-its-files)
- [Troubleshooting](#troubleshooting)

<!-- Base: origin/integration 64d6c67f63. (coming) sources: claude/fyx-w1-rebrand-904c67 (4b83f766d2),
claude/fyx-w3-renderer-e20d2b (d74c8bb2ac, db6619b9f8, 0004efba01, cade516b32),
claude/fyx-w9-fixes (3851a95912). -->

## What Fyx is

Fyx is a code editor forked from [Zed](https://zed.dev). Editing, languages, Git, terminals,
themes, keymaps and the agent panel work as they do in Zed. Fyx adds the following:

- **Review of agent changes.** When review mode is on, edits an AI agent proposes wait until you
  accept or reject them, hunk by hunk.
- **UI extensions.** Sandboxed WASM extensions can add dock panels and status bar items. They
  never draw anything themselves: Fyx renders them with its own components and your theme.
- **Security modes.** Optional modes keep private files, secret environment variables and agent
  config files out of agents' reach.
- **No Zed cloud.** Fyx does not sign in to zed.dev, collaborate, send telemetry or check for
  updates. See [Privacy defaults](#privacy-defaults).

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
| The built-in agent's edit and write tools | Yes. Fyx holds the edit before saving it, keeps only accepted hunks and tells the agent about the rest. |
| The built-in agent's delete, move and copy tools | No. They ask permission, as in Zed. |
| An ACP v2 edit that Fyx cannot show as a diff: several files in one request, no patch text, or a patch that does not apply | No. A notification says so. Decide in the agent panel. |
| Shell and terminal commands (`sed -i`, `cat >`, scripts) | No. Nothing is blocked, but the turn's review pane lists the changed files under "changed without review". |

A short version of this table is shown under every review pane, and the status bar item's tooltip
shows the long version.

A write that the [security guard](#security-modes-for-agents) refuses is refused before review, so
it never opens a review pane.

<!-- crates/agent_review/src/agent_review.rs: COVERAGE, NOT_REVIEWED -->

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
- **Permission cards.** While Fyx reviews an edit that came with a permission request, the
  request's card in the agent panel says so. It offers **Open review** instead of Allow and
  Reject.

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

There are two places for rules:

- **`.fyx/review.toml`**, at the root of a worktree, can only make review stricter. The file comes
  with the repository, and an agent can write to it.
  - `always_ask`: always reviewed, even after "accept all in the turn".
  - `deny`: never written by an agent. The write fails with a message naming the rule, and a
    permission request is answered "reject".
- **`agent_review.auto_accept`**, in your user settings, lists files whose edits land without
  review. Only you can loosen review.

```toml
# .fyx/review.toml
always_ask = ["Cargo.lock", ".github/**"]
deny       = ["*.env", "secrets/**"]
```

```json
// settings.json
"agent_review": {
  "enabled": true,
  "auto_accept": ["docs/**", "*.md"]
}
```

How the rules apply:

- `deny` and `always_ask` apply even with review mode off. `auto_accept` applies only while review
  mode is on.
- When several rules match a file, the first kind in this order wins: `deny`, `always_ask`,
  `auto_accept`. With the files above, `docs/security.md` lands without review. Add
  `always_ask = ["docs/security.md"]` to the project file, and it is always reviewed.
- A pattern without `/` matches the file name in any folder: `*.env` matches `config/prod.env`.
- A pattern with `/` matches the path from the worktree root: `docs/**` matches
  `docs/guide/intro.md`.
- Rules match the file the edit really goes to, with `.` and `..` resolved. On Windows and macOS,
  matching ignores case.
- Each decision a rule made is listed in the review pane with its rule, for example
  "by rule `deny = "*.env"` in …/.fyx/review.toml".

How Fyx handles changes to the file:

- Fyx reads the file when an agent thread starts in the project, and whenever it changes on disk.
- **Stricter rules** apply at once.
- **Looser rules.** A change that removes a restriction (a pattern dropped, or the file deleted)
  waits for you. A notification lists what would be removed and offers **Apply new review rules**.
  Until you click it, the old rules stay in force. This also holds when a new thread starts. Fyx
  does not remember this across restarts: at start, the file applies as it is.
- **Bad file.** If the file has bad TOML, an unknown key or a bad pattern, the old rules stay in
  force and a notification shows the error.
- **`auto_accept` in the project file** is ignored, and a notification says so.

<!-- crates/agent_review/src/review_rules.rs; agent_review.rs: "Apply new review rules", ignored_auto_accept notification -->

### Not covered yet

- Shell and terminal commands. They are listed as "changed without review", but not held.
- The built-in agent's delete, move and copy tools.
- ACP v2 edits to several files at once, or edits without a usable patch.
- Remote projects: `.fyx/review.toml` is read through the local file system only.
- A rejected permission request cannot carry your comments to the agent. This is a limit of ACP.

## UI extensions

A UI extension is a WASM component that adds dock panels or status bar items to Fyx. It describes
what to show (text, buttons, inputs, lists, tables, icons, markdown), and Fyx draws it in its own
style, with your theme. Each extension runs in its own sandbox:

- no files, environment variables or arguments;
- no network: every connection attempt fails with "access denied";
- at most 256 MiB of memory;
- a time limit on every call. An extension that runs too long or crashes is disabled until you
  restart Fyx. **(coming)** You will be able to restart it in place (see
  [Failures and restarts](#failures-and-restarts)).

Links in an extension's markdown open only if they are `http(s)://` addresses, and only after you
confirm the address.

UI extensions are separate from Zed extensions (languages, themes, slash commands). Those work as
in Zed.

<!-- crates/ui_ext_wasm/src/ui_ext_wasm.rs: deny_network, UI_MEMORY_LIMIT; docs/fyx/limits-and-gotchas.md; crates/ui_ext_host/src/view.rs: open_extension_link -->

### Installing an extension from a folder

1. Run the command `fyx_ui: install dev UI extension`.
2. Pick the extension's folder. Its `extension.toml` must have a `[fyx_ui]` table, and its
   component must be built.
3. Fyx asks **Install the UI extension `<id>`?**. The prompt lists what the extension asks for,
   its component and its folder.
4. Click **Install**.

Fyx links the folder into its installed-extensions directory and loads the extension. The
extension loads again on every start. When you rebuild its component, Fyx reloads it.

The folder's path cannot contain `&`, `|`, `^`, `<`, `>`, `%`, `!` or `"`.

To remove the extension, uninstall it on the Extensions page.

<!-- crates/ui_ext_host/src/install.rs: InstallDevUiExtension, prompt_and_install, cmd_special_char; registry.rs: scan_installed, hot reload of linked dev extensions -->

### The consent prompt

Fyx loads an extension from the installed-extensions directory only after you have enabled it as
it is now. The install command records your consent when you click **Install**. Anything else that
appears there waits for you, for example a folder copied there by hand or linked by Zed's
**Install Dev Extension**. Fyx then shows a notification:

> The UI extension `<id>` is installed but not enabled. It asks for: …

- **Enable** loads the extension and remembers your answer.
- **Not now** leaves it unloaded until the next start.

Fyx asks again when the extension asks for more capabilities. It also asks again when its
component changes, unless the extension is installed from a folder: then any build in that folder
counts as enabled. Answers are kept in `fyx-ui-consent.json` in the data folder.

**(coming)** Consent by tier:

- An extension that asks only for T0 capabilities (panels and status items) will load without a
  prompt when a Fyx install dialog showed it.
- A folder that appears without a Fyx install action keeps the one-time **Enable**.
- After that, only an update that adds a T1 capability (reading your files or buffers) waits for
  **Approve**. `fyx_ui: revoke UI extension permission` takes a T1 grant back.

**(planned, 0.4)** Fyx 0.4 changes this for the powerful items: they are accepted at install, one
checkbox each, instead of at first use. See
[Terminals and command-line tools](#terminals-and-command-line-tools-planned-fyxui-04).

<!-- crates/ui_ext_host/src/consent.rs; registry.rs: offer_consent; (coming) cade516b32 on claude/fyx-w3-renderer-e20d2b -->

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
startup, and reloads these extensions when you rebuild them. They load without the consent
prompt: setting the variable is your consent.

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

### Network access and proxies (coming, `fyx:ui` 0.3)

`network:fetch`, the permission for an extension to reach the internet, comes with `fyx:ui` 0.3.
An extension may reach only the domains it declares, and Fyx asks you the first time it does.

**If you use a proxy:** with a proxy set in Fyx's settings, an extension's requests to its
domains are **refused**. The extension gets the message "Fyx is set to use a proxy, and
network:fetch does not go through proxies yet". Fyx checks the address a domain resolves to
before it connects, and the current HTTP client cannot keep that check through a proxy. Requests
to `localhost` still work. Support for proxies is planned after 1.0.
<!-- source: W2c a1c2557b27, crates/ui_ext_wasm/src/network.rs; docs/fyx/wit-0.3/design.md §8 -->

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

## Terminals and command-line tools (planned, `fyx:ui` 0.4)

**Everything in this section is planned, not built.** It describes what Fyx 0.4 will do, from
[the design](wit-0.4/design.md) that the owner approved on 2026-11-19. Nothing here is on
`integration` yet, and the details may change before it ships.

Fyx will make your terminal a good home for tools like Claude Code, Gemini CLI and Codex. Fyx has
no code of its own for any one tool: each tool gets an extension, and the extension asks you for
what it needs.

### What you will see without installing anything

- **State on the terminal tab.** A tool that reports its state (Claude Code 2.1.295 and later, and
  Codex when it ships it) will show **working**, **needs you** or **done** on its tab, with no
  extension and no setting. Fyx always writes who reported it ("from Claude Code via OSC 7501").
- **An agent sessions panel.** It lists every terminal that has a state, with the ones that need
  you first. A click takes you to the terminal, and Fyx notifies you when a tool is blocked or
  done while the window is in the background.

### Installing an extension: one checkbox for each power

When you install an extension (from a file or, later, from the registry), the dialog has four
groups, named by how consent works:

1. **Shows in Fyx.** Panels and status items. Nothing to decide.
2. **Allowed without asking.** What it can see (your files, buffers, diagnostics, your editor's
   selection) and what it can do inside Fyx without leaving it (add terminal profiles, open
   terminals of its own, show a status on a tab, open files, propose edits for your review).
   Listed, with no checkbox: installing is the consent, and you can take each back later.
   Watching your other terminals is not here: it is read-only, but it is in the next group.
3. **Off unless you tick it.** One **checkbox per item, all unticked**. Each item says
   what it is, and shows the author's own reason as "The author says: …". For example:
   - **Run a program**: `codex` with these arguments. A shell or interpreter is marked "runs any
     command you type".
   - **Start terminals** for those programs.
   - **Add these environment variables** to the terminals it starts, with every name and value
     listed. A second, stronger checkbox, **Also add these to every new terminal you open**, is
     separate and also unticked.
   - **Listen on this computer** so a tool can connect (127.0.0.1 only, with a secret token that
     the extension never sees).
   - **Write a file** so a tool finds this window (the path is shown). "Lets Claude Code, and
     anything that reads this folder, connect to this extension. Claude Code will run what this
     extension tells it to."
   - **Receive a tool's command hooks**, with the channel names, and what happens when Fyx is
     closed (the tool normally carries on).
   - **Watch your terminals**, **change files**, **watch agent reviews**, **reach these web
     domains**.
4. **Asks you each time.** The powers it may ask for at the moment it needs them (below). No
   checkbox: nothing is granted in advance.

Click **Install** to install with what you ticked. An item you leave unticked is simply denied;
the extension loads and should tell you what it cannot do. Every checkbox ignores clicks for one
second after the dialog opens, so a double click cannot tick one by accident.

Later, the extension's page lists every item with a switch. Turning one on asks once more, with the
same text.

When an extension updates, see [Updates show what changed](#updates-show-what-changed)
below. **When you turn an item off or
remove the extension, everything stops at once:** terminals it started are killed with their
processes, its listeners close, its files are removed, its hooks answer as if Fyx were closed, and
its environment variables leave new terminals (terminals already open show **Restart to apply**).

Extensions you install without a dialog (the `FYX_UI_DEV_EXTENSIONS` variable, dev builds only)
keep today's behaviour: the first use of a power asks.

### Reading and typing in your terminals: asked each time

Your terminals are yours. By default no extension can read what is in them or type into them.

- **Share for reading.** From a terminal tab's menu, **Share with <extension>…**, or from a card
  the extension shows when you click something in its own panel. You choose what is shared: the
  last command and its output, or the last lines (at most 500), as plain text. Then **Once**, or
  **This terminal until it closes**.
- **Typing.** An extension can type into a terminal only after you shared it for writing, and
  **every command is confirmed** on a card that shows the exact text. Only printable text and
  tabs: no control keys. The tab then shows "typed by <extension>" for 5 seconds.
- The cards are drawn by Fyx, show the extension's name, ignore clicks for the first second and
  cannot be answered from the keyboard.
- Never in a folder you have not trusted, and never for a terminal of another Fyx profile.

**"Always allow".** You can turn it on from one of these cards or from the extension's page, for
reading and for writing separately, never at install. First Fyx says in plain words: "*<extension>*
will see everything that appears in your terminals, including passwords and tokens, and run any
command as you." The confirm button works after one second. While it is on, every terminal tab
shows "<extension> has access", the status bar shows an indicator with one click to switch it off,
every read and write is in the journal, and a write highlights the tab. It resets when the
extension's code changes.

### The journal

The extension's page shows what it did: programs it started (with arguments and the folder),
terminals it read (which, how much, never the content) and wrote to (the exact text you
confirmed), listeners it opened, files it wrote and removed, hook requests, proposed edits and your
consent changes. It keeps the last 2000 entries or 30 days. A full disk can drop entries, and the
page then says "Journal incomplete".

### Updates show what changed

An update that **adds or widens** what an extension can do is held. The old version keeps running,
and the page says **Update needs approval**, with a diff:

> **New in 1.3:** runs `codex` with `-c notify=*`; adds `CODEX_TUI_NOTIFY` to your profile
> terminals. **Removed:** `network:fetch` to api.example.com.

The update dialog shows only the groups that have changes, with no empty groups. New items that
need a checkbox are unticked, and approving installs the update with what you ticked. Things the update removes, and anything that is only a panel or a status item, install
without asking.

### Several accounts

If you have a personal and a work account for the same tool (two Claude logins, for example), Fyx
keeps them apart without ever touching them:

- **Account profiles** (settings: `fyx.accounts`) are a name, a tool, the folder that tool uses
  for that account (the one the tool's own setting points at, such as `CLAUDE_CONFIG_DIR`) and a
  colour. Fyx only sets that one variable for a terminal or agent you start with that account.
- **You sign in with the tool's own command**, inside that folder. Fyx never reads, copies or
  moves a token.
- **Per project:** "In this project, use Work". The choice is stored in your settings by project
  path, never in the repository, so a repository cannot pick an account for you. The "+" menu
  (for example "Claude Code — Work") and the agent panel offer the choice too.
- **Always visible:** the account's name and colour on the terminal tab, in the sessions list and
  in the agent panel.
- **No switching behind your back.** When a tool says "limit until 15:00", Fyx shows it as it is.
  It never changes account because of a limit, never carries a session to another account, and
  has no "continue with another account" button or queue that spreads work over accounts. If you
  switch accounts yourself, a new session starts.
- Extensions cannot choose, read or change an account, and the registry does not accept extensions
  that rotate accounts, get around limits or share access.

**Fyx profiles.** A Fyx profile is a separate copy of Fyx's data: settings, keymap, themes,
extensions, history and accounts, run as its own process (the same as `fyx --user-data-dir`, with
a manager). You create an empty profile, a copy, or one from a template, with a name, a colour and
an icon. Fyx asks which to open at start when you have more than one, shows the profile's name and
colour in the title bar, and can open a project in the profile it belongs to (`fyx --profile
<name>`). In a "Work" profile, account profiles default to the work folders. Switching profile is
always your action: nothing switches for you, and extensions cannot see other profiles or start
them.

The agent panel talks to Claude through Zed's own adapter, the same as upstream Zed. Anthropic's
terms say products built on the Agent SDK should use API keys; Fyx follows upstream here.

<!-- docs/fyx/wit-0.4/design.md §A (foundation), §B.4-B.9, §C.1-§C.6; plan P5 tasks 5.1-5.20; none of it is built yet -->

## Security modes for agents

Agents run with your user's rights. A prompt injection in a README, an issue or a web page could
make an agent read your secrets, send them out, or write to an agent config file so that code runs
later. Security modes reduce that risk. The design and the full rules are in
[Security modes](security-modes.md).

Every mode is **off by default**. Only your user settings can change the mode: a project's
settings have no such key.

```json
"fyx_security": {
  "mode": "audit",                // "off" (default), "audit" or "shield"
  "allow_env": ["GITHUB_TOKEN"]   // secret variables agent terminals may keep
}
```

| Mode | What it does today |
|---|---|
| `off` | Nothing. Fyx behaves like Zed. |
| `audit` | Blocks nothing. Logs each action that `shield` would block, as `fyx_guard audit: would block …`. |
| `shield` | Blocks the actions below and logs each block as `fyx_guard: blocked …`. |

<!-- crates/fyx_guard/src/fyx_guard.rs: SecurityMode, log prefixes; assets/settings/fyx.json -->

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

**Shield is not a sandbox, and fixes for it are pending.** Its judge review asked for fixes, which
are built but not merged yet. Until they are:

- The built-in agent's move, copy, delete and create-folder tools are not checked.
- `.git/config` is not protected, and neither is Fyx's own config folder.
- An agent's terminal can edit your settings file and lower the mode, and the change
  applies without asking you. With the fix, Fyx will ask you to confirm before it lowers
  protection.

Limits that stay until a later step:

- External agents' own tools (Claude Agent's `Read` and `Bash`, for example) read the disk directly,
  without asking Fyx.
- A terminal command can still `cat .env` or read `~/.aws/credentials`.
- Paths are checked by name, so a symbolic link to `.claude/` with another name is not caught.

Network control, placeholder secrets and an OS sandbox for agents are planned for later steps.

<!-- docs/fyx/security-modes.md "Known limits of step 1"; tasks/reviews/w9-step1.md F2-F4; (coming) claude/fyx-w9-fixes 3851a95912 -->

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
| `fyx-ui-consent.json` | Your **Enable** answers for UI extensions |
| `extensions/fyx/` | **(coming)** The UI extension install store: one folder per version, plus `state.json` with the active, previous and pinned version of each extension |
| `extensions/fyx/journal/<id>.jsonl` | **(planned, 0.4)** What an extension did: the last 2000 entries or 30 days; removed with the extension |

In each project, at the root of a worktree:

| Path | What |
|---|---|
| `.fyx/review.toml` | Project review rules |
| `.fyx/security.toml` | Extra private and read-only paths for shield mode |

`fyx --user-data-dir <DIR>` keeps all data and config in `<DIR>`. Use it for a throwaway profile.

<!-- crates/paths/src/paths.rs: config_dir, data_dir, logs_dir, log_file, extensions_dir; crates/extension_host/src/extension_host.rs: installed_dir; crates/ui_ext_host/src/consent.rs -->

## Troubleshooting

**Where is the log?** Run `zed: open log`, or open `Fyx.log` from the table above. Fyx's messages
start with these prefixes:

- `fyx:ui:` for UI extensions. Messages an extension writes itself appear under `fyx_ui`, after
  its id.
- `fyx: agent review:` for review.
- `fyx_guard` for security modes. At start, Fyx logs the active mode, for example
  `fyx_guard: security mode Shield`.

**A UI extension does not appear.**

- Look for a notification asking you to **Enable** it. If you clicked **Not now**, restart Fyx to
  get the notification again.
- Look in the log for `fyx:ui: failed to load …`. Common causes:
  - The `extension.toml` has no `[fyx_ui]` table.
  - The component is not built.
  - The extension targets an API version Fyx does not support.
  - A panel is declared without the `ui:panel` capability.
- Two extensions cannot use the same id. The second one is refused, with an error in the log.
- `FYX_UI_DEV_EXTENSIONS` is read only at startup. Set it in the same shell that starts Fyx.

**A panel shows an error.** An extension that crashes or runs past its time limit is disabled.
Restart Fyx to load it again. Some errors offer **Retry**. **(coming)** Use **Restart extension**
or `fyx_ui: restart UI extension` instead of restarting Fyx.

**Agent edits land without a review.**

- Check the status bar: **Review on** means review mode is on.
- Check whether `FYX_AGENT_REVIEW=0` is set in your environment.
- The edit may come from a path that review does not cover (see
  [What is reviewed](#what-is-reviewed)). Terminal changes show up under
  "changed without review".
- Check `agent_review.auto_accept` in your settings.

**A rule in `.fyx/review.toml` has no effect.**

- Look for a notification with a parse error. A bad file keeps the previous rules.
- If you removed a restriction, look for **Apply new review rules**.
- `auto_accept` belongs in your user settings, not in this file.

**Shield blocks something you need.** Switch to `audit` and read the
`fyx_guard audit:` lines:

- For a terminal variable, add it to `allow_env`.
- For a file, check your `private_files` setting.

**`git push` fails in an agent terminal under shield.** `SSH_AUTH_SOCK` is blanked. Add it
to `allow_env` if you want agents to push.

**The agent asks you to sign in or has no model.** Fyx has no zed.dev models. Set an API key for
Anthropic or another provider in the agent panel's settings, or use an external ACP agent.

**Fyx crashed.** Fyx writes each crash to the `crashes` folder of its data dir: a summary
(`<session>.json`) and, for most crashes, a minidump (`<session>.dmp`). Nothing is uploaded. At the
next start a notification offers **Report on GitHub**, which opens a prefilled issue in your
browser for you to read, complete and submit; `fyx: open crash reports` lists the last 20 crashes
(Enter reports one, the secondary confirm key shows its files). Attach the minidump only if you
want to: it can hold parts of memory, such as open files. Set `FYX_CRASH_REPORTS=0` to turn the
crash handler off.

**Start from a clean profile.** Run `fyx --user-data-dir <empty folder>`. Your normal profile is
left alone.
