# 5.6 broker API, agreed with W3 (2026-11-19)

This is my sketch, which W3 OK'd, with W3's changes applied. Names are mine to choose.

## ui_ext_wasm::permissions
- `Tier::T3`, and `tier_of("terminal:read" | "terminal:write") = T3`.
- `enum ConsentItem { Capability(String), Program { capability, name }, Listener(String),
  DiscoveryFile(String), EnvAllNew }`, with `.capability()` and a stable `.key()` such as
  `terminal:create#codex`.
- `ItemLine { item, tier, title, details: Vec<String>, reason: Option<String>, warnings:
  Vec<String> }`, **Serialize/Deserialize**.
  - Warnings: the §C.1 combinations ("It can send what it reads to …", RUNS_ANY_CODE "It can
    run any program on your computer") and the env notes ("files from this extension", "makes
    <tool> load this extension's plugins").
  - Alternatively a `consent_warnings(&UiManifest) -> Vec<String>`. W3 draws the text as given,
    and `registry.py comment` can reuse it.
- `consent_items(&UiManifest) -> Vec<ItemLine>`: dialog groups 1–4. It must work for **0.3
  manifests too**.
- `consent_diff(granted: &[ItemLine], new: &UiManifest) -> ConsentDiff { added, widened,
  removed }`.
  - It compares against **what was granted** (stored in the record), not against an old manifest.
  - Widening an item that is **Denied** installs silently and stays Denied. The page switch shows
    the new details.
  - Added and widened items are both unticked checkboxes. An unticked widened item becomes Denied.
- `CapabilityCheck` gets default methods: `item_state(ext, &ConsentItem)` and `t3_request(ext,
  T3Prompt) -> BoxFuture<T3Answer>` (default Deny).
- `T3Prompt { capability: T3Capability, terminal: <host TerminalId, W1's type, not the extension
  handle>, terminal_title, scope: Option<(ReadScope,u16)>, text: Option<String>, submit }`.
  `T3Answer { Once, UntilClosed, Deny }`.
- The runtime checks before `t3_request`: a gesture for request-read, an untrusted worktree,
  another profile's terminal, and that writes happen only on a terminal shared for writing.
  - Caps: 1 pending prompt per (ext, terminal) and 4 per ext. Past that, the answer is Deny with
    no card.
  - If the terminal closes or the extension unloads, the oneshot receiver is dropped. W3 watches
    `Sender::cancellation()`.
- The journal is `journal::JournalStore::at(<data_dir>/extensions/fyx/journal)`, passed in
  `LoadOptions::journal`.
  - `read(ext) -> JournalPage { entries, incomplete }` and `clear(ext)` may block; W3 calls them
    on the background executor and needs no live feed.
  - Entry kinds: ProgramStarted, TerminalRead (never content), TerminalWrite{text}, ListenerOpened,
    Connections/h, DiscoveryFile, Relay/h, ProposedEdit, ConsentChanged{item_key,state}. Keep 2000
    entries or 30 days.

## ui_ext_host (my permissions.rs + consent.rs)
- `ConsentRecord` gets `items: BTreeMap<key,bool>` and `granted: Vec<ItemLine>`, both serde
  default.
- `install(ext, record, ticked)`: unticked T2 items, and unticked widened items, become Denied.
- `set_item(ext, &item, allowed)` takes **T1 items too**, since the page revokes T1. W3 shows the
  confirm and the 1 s guard only when a T2 item is turned on. `set_item` notifies `changes()` and
  writes the journal.
- `item_state(ext, &item)` and `items(ext) -> Vec<(ConsentItem, PermissionState)>`.
- "Always allow":
  - `set_always_allow(ext, T3Capability, on: bool)` takes the sha from the record's
    `Grant::Component` itself and refuses a DevFolder grant.
  - `can_always_allow(ext) -> bool`.
  - The setting resets when the sha changes, and never applies in an untrusted worktree.
  - Changing it notifies `changes()` and writes ConsentChanged with key `terminal:read#always`.
  - `always_allowed() -> Vec<(ext, T3Capability)>` across all extensions, for the status bar and
    the tab marker, with a change signal (`changes()` is enough).
- `set_t3_prompter(UnboundedSender<(ext, T3Prompt, oneshot::Sender<T3Answer>)>)`. A dropped
  sender counts as Deny.
- `NeedsApproval { diff }`. W3 moves its UI from `new_capabilities` to `diff`.
- The tab menu "Share with <ext>…" starts from the user. It goes through
  `share_terminal(ext, TerminalId, T3Capability, Once|UntilClosed)` and `unshare`, and the runtime
  learns of it: it sends terminal-read-done, or opens writes. **W1 adds the menu item** and calls
  this API; W3 adds nothing there.
- The 0.3 prompter (`set_prompter`, `answer_prompt`) stays for FYX_UI_DEV_EXTENSIONS.
- When the host side lands, send W3 the commit.
