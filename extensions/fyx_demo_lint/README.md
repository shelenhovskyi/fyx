# TODO lint demo (`fyx-demo-lint`)

A Fyx UI extension for the staged `fyx:ui` 0.4 that publishes its own diagnostics
([design, section E](../../docs/fyx/wit-0.4/design.md), plan 6.6). Fyx cannot hand an
extension the files of a worktree, so the demo lints what you select: select text in a file
(Select All for the whole file) and every `TODO` in it becomes a warning. The editor underlines
it, the diagnostics panel lists it with the extension's name as its source, and the status bar
counts it. Selecting again in the same file replaces that file's warnings; a selection without a
`TODO` clears them.

Its panel, **TODO lint** (right dock), lists the published warnings; a click opens the file at
the line, and **Clear** clears every file. Stopping the extension clears them too.

It needs `ui:panel`, `context:editor` (T1, the selection), `diagnostics:publish` (T1) and
`workspace:open` (T1). A private file's selection has no text, so it is never linted.

## Build and run

```sh
script/fyx-build-demos --fyx-ui-0-4
```

builds it with the other 0.4 demos into `<target>/fyx-demos-v0_4/`. `fyx_ext_harness` runs it
headless against fake worktrees and checks what it publishes (`tests/demos_v0_4.rs`). Fyx shows
the warnings once its host publishes extension diagnostics (plan 6.6, W2c and W3).
