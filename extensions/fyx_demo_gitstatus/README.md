# Git status demo (`fyx-demo-gitstatus`)

A Fyx UI extension with one left-dock panel, **Git status** (dock icon `git_branch`). It has:

- a branch line (`main`, `2 ahead, 1 behind origin/main`) and a summary (`7 changed files, 3 staged`);
- a virtualized list with one row per changed file: a status icon and letter coloured by theme
  token (`M` `warning`, `A` `success`, `D` and `U` `error`, `R` `text-accent`, `?` `text-muted`),
  the path, and `staged` for staged changes;
- a details box: clicking a row shows the file's path, status and `+added` / `-removed` counts.

**The repository is mocked.** `fyx:ui@0.1.0` gives extensions no access to the worktree, so the
panel shows the fixed `MOCK_STATUS`.

How to write your own: [docs/fyx/extension-authoring.md](../../docs/fyx/extension-authoring.md).

## Build and run

```bash
script/fyx-build-demos
```

```bash
FYX_UI_DEV_EXTENSIONS=$PWD/extensions/fyx_demo_gitstatus ./target/debug/fyx --foreground
```

Unit tests run on the host target:

```bash
cargo test -p fyx_demo_gitstatus
```

This demo has been built and unit-tested, including a replay of every update through the host's
tree model (`ui_ext::Tree`). It has not yet been run in the editor.

## What it demonstrates

- **Rows with icons and colours.** Each row is one row flex built with `list_rows`; nothing inside
  a row sends its own event.
- **`row-click` to details with `remove` + `insert`.** The details box's children are removed
  with `SurfaceTree::clear_children` and the new ones added with `SurfaceTree::insert`, in one
  batch. Clicking the selected row again answers `unchanged`.
- The crate docs in `src/fyx_demo_gitstatus.rs` pin the first mount's node ids and the patches
  each event produces.
