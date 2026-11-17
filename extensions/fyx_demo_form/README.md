# Form demo (`fyx-demo-form`)

A Fyx UI extension with one right-dock panel, **Sign-up form** (dock icon `user_check`). It has
three inputs (name, email, age), **Submit** and **Reset** buttons, and a status line.

- A field shows its error under the input, in the `error` colour token, once the user has edited
  it. The rules: a name of 1 to 40 characters, an address like `name@example.com`, and an age
  from 13 to 120.
- **Submit** is disabled until every field is valid. Enter in a field submits a valid form.
- A submit shows `Submitted: <name> <<email>>, age <age>` in `success` and empties the form.
  **Reset** empties it without submitting.

How to write your own: [docs/fyx/extension-authoring.md](../../docs/fyx/extension-authoring.md).

## Build and run

```bash
script/fyx-build-demos
```

```bash
FYX_UI_DEV_EXTENSIONS=$PWD/extensions/fyx_demo_form ./target/debug/fyx --foreground
```

Unit tests run on the host target:

```bash
cargo test -p fyx_demo_form
```

This demo has been built and unit-tested, including a replay of every update through the host's
tree model (`ui_ext::Tree`). It has not yet been run in the editor.

## What it demonstrates

- **Error text inserted and removed.** An error is a text node inserted as the last child of the
  field's column when it appears, patched with `set-props` when its message changes, and removed
  when the value becomes valid. Shown errors survive a remount.
- **A button gated on state.** Submit's `disabled` follows `is_valid()`, and `SurfaceTree` sends
  the `set-props` only when it flips.
- **Several inputs, no echo.** Typed values are recorded with `set_props_from_host`; a submit or
  reset sets the inputs to empty, which is a deliberate change the host applies.
- The crate docs in `src/fyx_demo_form.rs` pin the first mount's node ids and the patches each
  event produces.
