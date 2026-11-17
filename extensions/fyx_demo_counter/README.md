# Counter demo (`fyx-demo-counter`)

A Fyx UI extension with one right-dock panel, **Counter** (dock icon `#`). It has a bold
`Count: N` label, **Decrement** / **Increment** / **Reset** buttons, and a virtualized list with
one row per count (`Row i of N`). It is the smallest complete example of the `fyx:ui@0.1.0` API:
a `SurfaceTree` mount, `set-props` patches in reply to clicks, and `list-rows`.

How to write your own: [docs/fyx/extension-authoring.md](../../docs/fyx/extension-authoring.md).

## Build and run

```bash
script/fyx-build-demos
```

This builds both demos for `wasm32-wasip2`, copies this crate's component to `fyx_ui.wasm` next
to `extension.toml`, and prints the `FYX_UI_DEV_EXTENSIONS=...` value to start Fyx with. To load
only this demo, point the variable at this directory and start the dev build:

```bash
FYX_UI_DEV_EXTENSIONS=$PWD/extensions/fyx_demo_counter ./target/debug/fyx --foreground
```

Open the right dock and click `#`.

Unit tests run on the host target:

```bash
cargo test -p fyx_demo_counter
```

## What it demonstrates

- **One state, one props function per node.** `label()`, `decrement()`, `increment()`, `reset()`
  and `list()` compute each node's props from `count`. `mount` builds the tree from them, and
  `sync` re-sets all five after a click. `SurfaceTree` sends only the props that actually changed.
- **Patches, never `replace`.** The first Increment from 0 changes four nodes (label, Decrement,
  Reset, list), while the next one changes only two (label, list). A click that changes nothing
  (Decrement at 0) answers `unchanged`.
- **Row refresh through the list's props.** `row-count` changes on every click, which makes Fyx
  request the visible rows again, so every row's `of N` stays current.
- **Remounts.** The node ids are stored on every `mount`, and the count lives outside the tree,
  so a remount (after Fyx rejects an update) keeps the count.

## Behaviour contract

The host integration test `test_demo_counter_round_trip` (in `extension_host`) relies on this
behaviour. The full contract is the crate doc of `src/fyx_demo_counter.rs`, pinned by its test
`behaves_as_documented`. In short:

- Surface `counter`: a right dock panel titled "Counter", icon `hash`, default size 320.
- The first mount returns ids 1–7: column (1), label (2), button row (3), Decrement (4),
  Increment (5), Reset (6) and list (7). Later mounts continue at 8–14, 15–21, and so on, because
  the id allocator survives remounts.
- `list-rows("counter", <list id>, start, count)` returns rows `start..min(start + count, count)`.
  Each row is a row flex (`0x8000_0000`) holding one text (`0x8000_0001`).
- An unknown surface id is an error.

Change this behaviour only together with that test.

## Manifest

`extension.toml` declares `[fyx_ui]` with `api_version = "0.1.0"`, `component = "fyx_ui.wasm"`
and `capabilities = ["ui:panel"]`. The built `fyx_ui.wasm` is ignored by git.
