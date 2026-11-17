# TODO demo (`fyx-demo-todo`)

A Fyx UI extension with one left-dock panel, **TODOs** (dock icon `list_todo`). It collects
`TODO`, `FIXME` and `HACK` comments and shows them in a virtualized list. The panel has:

- a summary line (`N of M items in F files`);
- a filter input, matched against the comment text and the path;
- kind buttons **All** / **TODO** / **FIXME** / **HACK**, with the active one filled;
- a **Rescan** button;
- the list (kind, message and `path:line` per row);
- a detail line with the source line of the clicked row.

**The files are mocked.** `fyx:ui@0.1.0` gives extensions no access to the worktree, so the panel
reads four fixed files from `MockWorktree`, behind a `TodoSource` trait that a host-backed source
can replace later.

How to write your own: [docs/fyx/extension-authoring.md](../../docs/fyx/extension-authoring.md).

## Build and run

```bash
script/fyx-build-demos
```

```bash
FYX_UI_DEV_EXTENSIONS=$PWD/extensions/fyx_demo_todo ./target/debug/fyx --foreground
```

Open the left dock and click the TODO icon. Unit tests run on the host target:

```bash
cargo test -p fyx_demo_todo
```

As of `tasks/progress/W3.md` and `W4.md`, this demo has been built and unit-tested but not yet
run in the editor. The counter demo is the one verified there.

## What it demonstrates

- **Typing without echo.** On `input-changed`, the filter's text is recorded with
  `SurfaceTree::set_props_from_host`, never sent back as `set-props`, so it cannot fight the
  user's typing. A keystroke that leaves the query unchanged answers `unchanged`.
- **Rebuilding a list whose rows changed.** In 0.1, Fyx re-requests rows only when the list node is
  touched. When the matching items change (filter, kind button, rescan), `SurfaceTree::rebuild`
  replaces the list node with a new id, so Fyx asks for every visible row again. Because a rebuild
  resets the scroll position, it happens only when the visible rows actually differ, compared by
  content. `list-rows` for an old list id gets an empty answer.
- **`row-click`.** Clicking a row sends `row-click(list, index)`. The panel maps the index to the
  item and updates the detail line. The selection survives a refilter while its item stays
  visible.
- **Theme tokens.** The kind markers are coloured `text-accent` (TODO), `error` (FIXME) and
  `warning` (HACK). The summary is `text-muted`, or `error` if reading the files failed.

## Scanner rules

A marker is the upper-case word `TODO`, `FIXME` or `HACK` on its own, so `TODO_LIST` and `TODOS`
do not count. It may be followed by `(owner)` and `:`. A trailing `*/` or `-->` is stripped from
the message. The earliest marker on a line wins.

## Manifest

`extension.toml` declares `[fyx_ui]` with `api_version = "0.1.0"`, `component = "fyx_ui.wasm"`
and `capabilities = ["ui:panel"]`. The built `fyx_ui.wasm` is ignored by git.
