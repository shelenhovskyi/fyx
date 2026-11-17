# Markdown demo (`fyx-demo-markdown`)

A Fyx UI extension with one right-dock panel, **Markdown preview** (dock icon `file_markdown`). It
has an input, **Undo line** and **Clear** buttons, and a `markdown` node that renders the document.

`fyx:ui@0.1.0` inputs are single-line, so the document is built line by line: the input holds the
line being typed and is previewed live, Enter appends it to the document, and an empty line is a
paragraph break. The panel starts with a three-line sample.

How to write your own: [docs/fyx/extension-authoring.md](../../docs/fyx/extension-authoring.md).

## Build and run

```bash
script/fyx-build-demos
```

```bash
FYX_UI_DEV_EXTENSIONS=$PWD/extensions/fyx_demo_markdown ./target/debug/fyx --foreground
```

Unit tests run on the host target:

```bash
cargo test -p fyx_demo_markdown
```

This demo has been built and unit-tested, including a replay of every update through the host's
tree model (`ui_ext::Tree`). It has not yet been run in the editor.

## What it demonstrates

- **A `markdown` node patched with `set-props`.** Every keystroke the host sends (debounced) sets
  the preview, the line and character count, and the buttons, and only the ones that changed.
- **Typing without echo, clearing after Enter.** `input-changed` is recorded with
  `SurfaceTree::set_props_from_host`. On `input-submitted` the submitted text is recorded first,
  so the following `set-props` to an empty value is a real change and clears the field.
- The crate docs in `src/fyx_demo_markdown.rs` pin the first mount's node ids and the patches
  each event produces.
