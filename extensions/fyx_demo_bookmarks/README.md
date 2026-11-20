# Bookmarks demo (`fyx-demo-bookmarks`)

A Fyx UI extension for the staged `fyx:ui` 0.4 with a tree view and project-panel badges
([design, section F](../../docs/fyx/wit-0.4/design.md), plan 6.7).

Its panel, **Bookmarks** (left dock), has a button that bookmarks the active file, or removes
its bookmark, and a two-level tree of the bookmarks grouped by folder. A folder row has a
chevron: click it, or press Right and Left while the list has focus, to expand and collapse it.
A click on a file opens it. Every bookmarked file gets a `★` in the project panel, with
"Bookmarked" in its tooltip; removing the bookmark removes the badge, and stopping the
extension removes them all. Bookmarks are not saved: they last while the extension runs.

It needs `ui:panel`, `context:editor` (T1, which file is active), `workspace:open` (T1) and
`workspace:file-decorations` (T1).

## Build and run

```sh
script/fyx-build-demos --fyx-ui-0-4
```

builds it with the other 0.4 demos into `<target>/fyx-demos-v0_4/`. `fyx_ext_harness` runs it
headless: it bookmarks files, folds the tree with `row-toggled` and checks the badges
(`tests/demos_v0_4.rs`). Fyx draws the chevrons and badges once its host has them (plan 6.7,
W2c and W3).
