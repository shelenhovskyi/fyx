# Writing a Fyx UI extension

This guide is for authors of Fyx UI extensions: WASM components that add native, themed panels and
status bar items to Fyx through the `fyx:ui@0.1.0` API. It covers the project layout, the manifest,
building and loading, the mental model, and a complete counter walkthrough.

Related pages:

- [SDK reference](sdk-reference.md): every public item of the Rust SDK `ui_ext_sdk`.
- [Limits and gotchas](limits-and-gotchas.md): time budget, caps, rows, inputs, icons, versions,
  and what is planned for 0.2.

> **Status: sprint prototype.** `fyx:ui@0.1.0` is frozen for the sprint, but it is not a stable
> API. The only way to load a UI extension is an environment variable read at startup. There is no
> registry, no install UI and no hot reload. The SDK is not published to crates.io, and `diff-view`
> is a placeholder. Expect breaking changes in 0.2. These docs describe what the code on the
> `integration` branch does, limits included.

Rule sources are cited in HTML comments as `file:line`, so you can check every claim against the
code.

## What a UI extension is

A UI extension is a WebAssembly component (target `wasm32-wasip2`) that implements the
`fyx:ui/ui-extension` world in `crates/ui_ext/wit/fyx-ui.wit`. It contributes **surfaces**:

- **Panels** in the left, right or bottom dock, each with a dock icon and a default size.
- **Status items** on the left or right of the status bar.

<!-- crates/ui_ext/wit/fyx-ui.wit:134-159 -->

The extension never draws. It describes each surface as a tree of **nodes** (flex containers,
text, buttons, inputs, virtualized lists and tables, icons, markdown, and a diff view). Fyx keeps
that tree and renders it with its own GPUI components, coloured by the active theme. When the user
clicks or types, Fyx calls the extension once, and the extension answers with **patches** to the
tree.

<!-- crates/ui_ext/wit/fyx-ui.wit:81-118, 175-190 -->

A UI extension is separate from a Zed-style extension (`zed:extension`). It runs in its own
wasmtime `Store` with its own call queue, on its own engine. A slow call in the same extension's
`zed:extension` component therefore never delays click handling. The `[fyx_ui]` manifest table is
ignored by upstream Zed's manifest parser, so one directory can hold both kinds.

<!-- crates/ui_ext_wasm/src/ui_ext_wasm.rs:3-9, 137-138 -->

The sandbox: the component gets WASI p2, all of it linked (Rust's std imports parts of it), with
stderr inherited and nothing else granted. It has no preopened directories, no environment
variables and no arguments, and no network: TCP, UDP and name lookup are denied, so `connect`
fails with "access denied". It cannot read the worktree or buffers yet. Its store is capped (32 MiB
of memory; see "Limits" in `limits-and-gotchas.md`). Besides WASI, the host functions are
`push-update` and `log`.

<!-- crates/ui_ext_wasm/src/ui_ext_wasm.rs:452-453 (WasiCtxBuilder::new + inherit_stderr), crates/ui_ext/wit/fyx-ui.wit:163-173 -->

## Project layout

A UI extension is one directory holding the manifest and the built component, plus the Rust crate
that produces the component:

```
my_counter/
├── Cargo.toml          # cdylib crate depending on ui_ext_sdk
├── extension.toml      # id, name, version + the [fyx_ui] table
├── fyx_ui.wasm         # the built component, copied here (not committed)
└── src/
    └── lib.rs          # implements ui_ext_sdk::Extension, ends with register_extension!
```

The demo extensions in this repository (`extensions/fyx_demo_counter`,
`extensions/fyx_demo_todo`) have this layout. Each one also has a `.gitignore` that ignores
`/fyx_ui.wasm`.

`Cargo.toml` of a standalone extension:

```toml
[package]
name = "my_counter"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
# The SDK is not on crates.io yet: point at a Fyx checkout.
ui_ext_sdk = { path = "../fyx/crates/ui_ext_sdk" }
```

- `crate-type = ["cdylib"]` is required. On `wasm32-wasip2`, cargo then emits a component.
- `ui_ext_sdk` is a path dependency because the crate is not published yet (it is planned as
  `fyx-sdk`). It inherits its edition and lints from the Fyx workspace, so a full Fyx checkout
  must exist at that path. Its external dependencies are `wit-bindgen` 0.41 and, through
  `ui_ext`, `serde` and `thiserror`.
- If your crate sits *inside* the Fyx checkout but is not a workspace member, add an empty
  `[workspace]` table to its `Cargo.toml`. Without it, cargo refuses to build a package that is
  inside a workspace but not a member of it. The host's test fixture `probe` does the same.

<!-- crates/ui_ext_sdk/Cargo.toml (wit-bindgen = "0.41", edition.workspace), ARCHITECTURE.md crate table ("Published later as fyx-sdk"), crates/ui_ext_wasm/fixtures/probe/Cargo.toml -->

The SDK is built and tested with the toolchain pinned in `rust-toolchain.toml` (1.98.1, which has
the `wasm32-wasip2` target). Older toolchains are untested.

## `extension.toml`

```toml
id = "my-counter"
name = "My counter"
description = "A counter panel."
version = "0.1.0"
schema_version = 1
authors = ["You"]

[fyx_ui]
api_version = "0.1.0"
component = "fyx_ui.wasm"
capabilities = ["ui:panel"]
```

Fyx reads `id` and the `[fyx_ui]` table. It ignores the other top-level keys, but keep them: they
are what upstream Zed's manifest parser expects.

<!-- crates/ui_ext_wasm/src/ui_ext_wasm.rs:151-155 (ExtensionToml { id, fyx_ui }) -->

`[fyx_ui]` accepts exactly these three keys. Unknown keys are an error (`deny_unknown_fields`).

| Key | Rule |
|---|---|
| `api_version` | A semver string. It must be a version this build supports (exactly `0.1.0` today), and it must equal the version the component itself declares (see [Versions](limits-and-gotchas.md#versions)). |
| `component` | A path inside the extension directory. It must be non-empty and relative, with only normal path segments: `fyx_ui.wasm` and `bin/fyx_ui.wasm` pass, but `./fyx_ui.wasm`, `../x.wasm` and absolute paths are rejected. |
| `capabilities` | A list of strings, empty by default. Every panel surface needs `"ui:panel"` and every status item needs `"ui:status-bar"`, or loading fails. |

<!-- crates/ui_ext_wasm/src/ui_ext_wasm.rs:139-149 (UiManifest, deny_unknown_fields, capabilities default), 171-183 (version supported, component path: all components Normal), 188-203 (check_surfaces), 551-557 (manifest version == component version, check_surfaces) -->

The capability check runs only when Fyx loads the extension from its directory. A bare `.wasm`
loaded for quick iteration (below) skips it.

<!-- crates/ui_ext_host/src/registry.rs:330-350 -->

## Building

```bash
cargo build --target wasm32-wasip2 --release
```

```bash
cp target/wasm32-wasip2/release/my_counter.wasm fyx_ui.wasm
```

The output is already a component, with no separate `wasm-tools component new` step.
`register_extension!` embeds the custom section `fyx:ui-api-version` that tells Fyx which API
version the component targets. Release builds with `strip = true` keep that section (checked on
the walkthrough crate below).

<!-- crates/ui_ext_sdk/src/ui_ext_sdk.rs:210-215 -->

For the demos in this repository, `script/fyx-build-demos` runs the build, copies each
`<crate>.wasm` to `extensions/<crate>/fyx_ui.wasm`, and prints a ready `FYX_UI_DEV_EXTENSIONS=...`
line (`;`-separated on Windows, `:` elsewhere):

```bash
script/fyx-build-demos --release
```

<!-- script/fyx-build-demos -->

Inside the Fyx checkout, `.cargo/config.toml` sets `rustc-wrapper = "sccache"`. On a machine
without sccache, override it with an empty `RUSTC_WRAPPER`:

```bash
RUSTC_WRAPPER= cargo build --target wasm32-wasip2 --release
```

## Installing as a dev extension

For the sprint, UI extensions load **only** from the environment variable
`FYX_UI_DEV_EXTENSIONS`, which Fyx reads once at startup. Upstream's "Install Dev Extension"
command does not load `[fyx_ui]` components, and when the variable is unset nothing loads.

<!-- crates/ui_ext_host/src/ui_ext_host.rs:11, crates/ui_ext_host/src/registry.rs:29-33, 172, 305-312 -->

The value is a `PATH`-style list (`;` on Windows, `:` elsewhere). Each entry is one of:

- **An extension directory**: `extension.toml` with a `[fyx_ui]` table, plus the component it
  names. This is the normal case, and it gets the manifest and capability checks above.
- **A bare `.wasm` component**, for quick iteration. The extension id is the file stem with `_`
  replaced by `-`, and there are no manifest or capability checks.

<!-- crates/ui_ext_host/src/registry.rs:329-350 -->

```bash
FYX_UI_DEV_EXTENSIONS=/path/to/my_counter ./target/debug/fyx --foreground
```

On Windows (`cmd`):

```
set FYX_UI_DEV_EXTENSIONS=C:\src\my_counter;C:\src\fyx\extensions\fyx_demo_todo
```

The log reports each step: `fyx:ui: loading dev extension <path>`, then
`fyx:ui: loaded <id> with N surface(s)`, or `fyx:ui: failed to load dev extension <path>: <reason>`.
Messages the extension writes with `log` appear under the log target `fyx_ui`, prefixed with the
extension id.

<!-- crates/ui_ext_host/src/registry.rs:316-325, 352-356; crates/ui_ext_wasm/src/ui_ext_wasm.rs:297 -->

To pick up a new build, restart Fyx. Nothing reloads a running extension.

Each panel gets a button in its dock. The action `fyx_ui::ToggleExtensionPanel { key }` focuses or
unfocuses a panel by its key `"<extension id>/<surface id>"`, so you can bind it to a key. If two
loaded surfaces share a key, the second one is skipped with an error in the log.

<!-- crates/ui_ext_host/src/ui_ext_host.rs:32-38, crates/ui_ext_host/src/panel.rs:57, crates/ui_ext_host/src/registry.rs:76-83 -->

## The mental model

These rules shape everything else. The SDK handles most of the bookkeeping, but you need them to
understand what you get back.

**1. The host retains the tree; the extension never renders.** `mount` returns a surface's whole
tree once. Fyx keeps it and redraws from its copy every frame, and it never calls into WASM from
`render()`. In the current host, `mount` runs once when the extension loads, not when the panel
first opens. It runs again only to resync after Fyx rejects an update (rule 5). Reopening a panel
does not remount it.

<!-- crates/ui_ext/wit/fyx-ui.wit:183-184; crates/ui_ext_host/src/registry.rs:86-88 (SurfaceState::new on register) → crates/ui_ext_host/src/surface.rs:92 (mount in new), 327-341 (remount on invalid update) -->

**2. Flat tree, parents first.** WIT types cannot be recursive, so a tree travels as a flat
`list<node>` in which every node names its parent's id. A parent must come before its children,
and siblings keep list order. Several nodes may have no parent: a surface is a forest of roots.

<!-- crates/ui_ext/wit/fyx-ui.wit:93-99; crates/ui_ext/src/tree.rs:72, 260-268 (UnknownParent) -->

**3. Only `flex` nodes have children.** Lists and tables get their rows from `list-rows`, not
from child nodes. A child under any other kind of node is rejected.

<!-- crates/ui_ext/src/types.rs:136-139; crates/ui_ext/src/tree.rs:241-246, 269-274 -->

**4. Node ids are yours, and they are forever.** You choose every `node-id` (a `u32`). An id must
be unique within the surface and must never be reused for a different node while the surface
lives. Fyx routes events and patches by id. The SDK's `IdAllocator` only counts up, and
`SurfaceTree` keeps counting across remounts.

<!-- crates/ui_ext/wit/fyx-ui.wit:5-7; crates/ui_ext_sdk/src/ui_ext_sdk.rs:241-267, 601-604 -->

**5. Changes are patches, applied atomically.** `handle-event` answers `unchanged`, a full
`replace`, or a list of `set-props`, `insert` and `remove` patches. Fyx applies a batch all or
nothing. If one patch breaks a rule (unknown id, child under a non-flex node, a `before` that is
not a sibling, a duplicate id, a cap), Fyx keeps the old tree, shows the error in the panel
footer, and calls `mount` again to resync. The extension is not told why, so the SDK's
`SurfaceTree` checks the same rules *before* queueing an edit.

<!-- crates/ui_ext/wit/fyx-ui.wit:101-118; crates/ui_ext/src/tree.rs:188-207; crates/ui_ext_host/src/surface.rs:287-343; crates/ui_ext_sdk/src/ui_ext_sdk.rs:568-573 -->

**6. One call per input event, one call at a time.** A click is one `handle-event` call. Typing is
debounced (150 ms), and a newer queued `input-changed` for the same input replaces the older one.
Enter sends `input-submitted` with the text. Events reach the extension one at a time, in order,
and each reply is applied before the next event goes out. An event whose node has left the tree
is dropped. There are no hover, scroll or focus events in 0.1. Scrolling a list makes `list-rows`
calls, not events.

<!-- crates/ui_ext/wit/fyx-ui.wit:126-132, 186-187; crates/ui_ext_host/src/surface.rs:170-219; crates/ui_ext_host/src/view.rs:30, 216-252 -->

**7. No calls from render, and none outside a host call.** Your code runs only while Fyx is
calling it: `surfaces`, `mount`, `handle-event` or `list-rows`. `push-update` therefore only works
*during* one of those calls. It cannot drive a timer (see
[Limits](limits-and-gotchas.md#push-update-only-during-a-call)).

**8. Theme tokens only.** There are no raw colours and no font choices. Colours are the nine
`color-token` values (`text`, `text-muted`, `text-accent`, `error`, `warning`, `success`, `border`,
`surface`, `elevated-surface`), mapped to the active theme. Your UI follows theme changes with no
work on your side.

<!-- crates/ui_ext/wit/fyx-ui.wit:9-21; crates/ui_ext_host/src/view.rs:763-794 -->

**9. Slow or crashing extensions are disabled.** Each call has a 200 ms budget (2 s for
instantiation and `surfaces`). A call that runs past it, traps or panics disables the extension
until Fyx restarts. Returning `Err(String)` is different: it is shown to the user and the extension
keeps running.

<!-- crates/ui_ext_wasm/src/ui_ext_wasm.rs:62-71, 693-734, 663-675 -->

## Walkthrough: a counter in 60 lines

The extension below is a right-dock panel with a bold label, an **Add** button and a **Reset**
button that is disabled at zero. The file is 68 lines, 59 of them code, and builds as-is for
`wasm32-wasip2`. It was verified with `cargo build --target wasm32-wasip2 --release` and
`cargo test` against the SDK on `integration`.

### Step 1: the crate

Create `my_counter/` with the `Cargo.toml` and `extension.toml` from
[Project layout](#project-layout) and [`extension.toml`](#extensiontoml).

### Step 2: state and props

```rust
use ui_ext_sdk::*;

const SURFACE_ID: &str = "my-counter";

#[derive(Default)]
struct Counter {
    count: u32,
    tree: SurfaceTree,
    label: NodeId,
    add: NodeId,
    reset: NodeId,
}

impl Counter {
    fn label(&self) -> TextProps {
        TextProps::new(format!("Count: {}", self.count)).bold()
    }

    fn reset(&self) -> ButtonProps {
        ButtonProps::new("Reset").disabled(self.count == 0)
    }
}
```

`use ui_ext_sdk::*` brings in the WIT types (`Node`, `NodeId`, `UiEvent`, `Update`, the props
records), their builders, `SurfaceTree` and the `Extension` trait.

`SurfaceTree` is your copy of what Fyx shows. It hands out node ids and turns your edits into
patches.

The node id fields start at 0. The SDK never hands out 0 (ids start at 1), so before `mount` no
real event can match them.

Writing each node's props as a function of the state is the pattern the demos use. You call the
same function in `mount` and after every change, and `SurfaceTree` only sends what actually
changed.

<!-- crates/ui_ext_sdk/src/ui_ext_sdk.rs:69-70 (re-exports), 249-251 (ids start at 1), 562-567 -->

### Step 3: declare the surface

```rust
impl Extension for Counter {
    fn new() -> Self {
        Self::default()
    }

    fn surfaces(&self) -> Vec<Surface> {
        let panel = Surface::panel(SURFACE_ID, "My counter", DockPosition::Right, "hash", 280);
        vec![panel]
    }
```

Fyx calls `surfaces` once, right after instantiating the component. That is also when `new` runs,
since the SDK creates your instance on the first call.

The panel arguments are: an id unique within your extension, a title (the dock button's tooltip),
the dock, a dock icon and a default size in pixels. The size is clamped to 120..=2000. The icon is
a snake_case name of Zed's `IconName` (`hash` is `#`), and an unknown name shows a fallback icon.

<!-- crates/ui_ext/wit/fyx-ui.wit:180-181; crates/ui_ext_sdk/src/ui_ext_sdk.rs:225-238, 920-936; crates/ui_ext_host/src/panel.rs:39-55; crates/ui_ext_host/src/view.rs:749-752 -->

### Step 4: build the tree in `mount`

```rust
    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        if surface_id != SURFACE_ID {
            return Err(format!("unknown surface {surface_id:?}"));
        }
        let (label, reset) = (self.label(), self.reset());
        let mut ids = (0, 0, 0);
        let nodes = self.tree.mount(|tree| {
            tree.flex(FlexProps::column().gap(8).padding(8), |tree| {
                ids.0 = tree.text(label);
                tree.row(|tree| {
                    ids.1 = tree.button(ButtonProps::new("Add").icon("plus"));
                    ids.2 = tree.button(reset);
                });
            });
        })?;
        (self.label, self.add, self.reset) = ids;
        Ok(nodes)
    }
```

- `SurfaceTree::mount` runs your closure with a `TreeBuilder`. A flex node's closure adds its
  children, and every builder call returns the new node's id.
- The props are computed *before* `self.tree.mount` because the closure cannot borrow `self`
  while `self.tree` is mutably borrowed.
- The ids are stored on every mount, not just the first. `mount` can be called again (rule 1),
  and the new tree gets new ids. The count lives in `self.count`, so it survives a remount.
- `?` works because `TreeError` converts into `String`.

On the first mount, the flat list this returns is five nodes, parents first: column (1), text (2), row (3), Add (4),
Reset (5).

<!-- crates/ui_ext_sdk/src/ui_ext_sdk.rs:313-329, 549-553, 601-618 -->

### Step 5: answer events with patches

```rust
    fn handle_event(&mut self, surface_id: &str, event: UiEvent) -> Result<Update, String> {
        if surface_id != SURFACE_ID {
            return Err(format!("unknown surface {surface_id:?}"));
        }
        match event {
            UiEvent::Click(id) if id == self.add => self.count = self.count.saturating_add(1),
            UiEvent::Click(id) if id == self.reset => self.count = 0,
            _ => return Ok(Update::Unchanged),
        }
        self.tree.set_props(self.label, self.label())?;
        self.tree.set_props(self.reset, self.reset())?;
        Ok(self.tree.take_update())
    }
}
```

- `set_props` records the new props and queues a `set-props` patch, but only if they differ from
  what Fyx already shows. The first Add from 0 changes the label and enables Reset (two patches),
  while the next Add only changes the label (one patch).
- `take_update` hands the queued patches to Fyx: `patches` if there are any, `unchanged`
  otherwise.
- Use `saturating_add` rather than `+= 1`: a panic aborts the call, and a trapped call disables
  the extension.

<!-- crates/ui_ext_sdk/src/ui_ext_sdk.rs:677-690, 769-772, 479-486; crates/ui_ext_wasm/src/ui_ext_wasm.rs:716-721 -->

### Step 6: export it

```rust
register_extension!(Counter);
```

The macro implements the generated WIT exports for your type and embeds the
`fyx:ui-api-version` section. Call it exactly once, at the crate root.

<!-- crates/ui_ext_sdk/src/ui_ext_sdk.rs:163-219 -->

### Step 7: build and load

```bash
cargo build --target wasm32-wasip2 --release
```

```bash
cp target/wasm32-wasip2/release/my_counter.wasm fyx_ui.wasm
```

```bash
FYX_UI_DEV_EXTENSIONS=$PWD /path/to/fyx/target/debug/fyx --foreground
```

Open the right dock: a `#` button opens **My counter**.

### Step 8 (optional): unit-test it on your machine

The SDK works off WASM: `log` prints to stderr and `push_update` is recorded for
`take_pushed_updates()`. You can therefore test the extension with plain `cargo test`.

<!-- crates/ui_ext_sdk/src/ui_ext_sdk.rs:100-131 -->

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_then_reset() {
        let mut counter = Counter::new();
        let nodes = counter.mount("my-counter").expect("mount");
        assert_eq!(nodes.len(), 5);

        let update = counter.handle_event("my-counter", UiEvent::Click(counter.add));
        let Ok(Update::Patches(patches)) = update else {
            panic!("expected patches, got {update:?}");
        };
        assert_eq!(patches.len(), 2); // the label and the now enabled Reset button

        let update = counter.handle_event("my-counter", UiEvent::Click(counter.add));
        let Ok(Update::Patches(patches)) = update else {
            panic!("expected patches, got {update:?}");
        };
        assert_eq!(patches.len(), 1); // Reset was already enabled: only the label changed
    }
}
```

## Next steps

The snippets below were compiled and tested the same way as the counter.

### A virtualized list

A list node only declares its row count and a fixed row height. Fyx asks for rows as they scroll
into view, and you build each row as a small subtree with `ui_ext_sdk::list_rows`:

```rust
// in mount, inside a flex node:
//     self.list = tree.list(ListProps::new(self.count, 24));

fn list_rows(&mut self, _surface_id: &str, list_node: NodeId, start: u32, count: u32) -> Vec<ListRow> {
    if list_node != self.list {
        return Vec::new();
    }
    let total = self.count;
    list_rows(start, count, total, |index, tree| {
        tree.row(|tree| {
            tree.text(format!("Row {} of {total}", index + 1));
        });
    })
}
```

- `list_rows` clamps the requested range to `total`. It builds each row with ids starting at
  `ROW_NODE_ID_BASE`, and it leaves out (and logs) any row Fyx would reject.
- Each row needs exactly one root node. Ids inside a row are row-local, and a click anywhere in a
  row arrives as `row-click(list, index)`.
- Fyx requests the visible rows again when the list node's props change. Read
  [Rows](limits-and-gotchas.md#rows-row-local-ids-row-click-and-refresh) before relying on that.

`extensions/fyx_demo_counter` is the complete version of this example.

<!-- crates/ui_ext_sdk/src/ui_ext_sdk.rs:373-422 -->

### An input

```rust
fn filter_input(text: &str) -> InputProps {
    InputProps::new(text).placeholder("Filter")
}

// in mount:
//     self.filter = tree.input(filter_input(&self.query));

// in handle_event:
match event {
    UiEvent::InputChanged((id, text)) if id == self.filter => {
        // Record what the user typed without echoing it back.
        self.tree.set_props_from_host(id, filter_input(&text))?;
        self.query = text;
        // ...set the props of the nodes that depend on the query, then:
        Ok(self.tree.take_update())
    }
    UiEvent::InputSubmitted((id, text)) if id == self.filter => {
        // Record the submitted text first, or clearing would look like "no change".
        self.tree.set_props_from_host(id, filter_input(&text))?;
        self.tree.set_props(id, filter_input(""))?;
        self.query.clear();
        Ok(self.tree.take_update())
    }
    _ => Ok(Update::Unchanged),
}
```

The user owns an input's text while typing, which is why the first arm uses
`set_props_from_host`. See [Input text ownership](limits-and-gotchas.md#input-text-ownership) for
the details.

Note the double parentheses in `UiEvent::InputChanged((id, text))`: a WIT tuple payload becomes one
Rust tuple field.

`extensions/fyx_demo_todo` shows inputs, filter buttons, row clicks and list rebuilding together.
