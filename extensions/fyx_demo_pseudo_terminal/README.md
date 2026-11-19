# WASM shell demo (`fyx-demo-pseudo-terminal`)

A Fyx UI extension for the staged `fyx:ui` 0.4 that is the backend of a pseudo-terminal
([design, section B.2](../../docs/fyx/wit-0.4/design.md)). One panel, **WASM shell** (bottom
dock), shows the terminal with Fyx's native terminal view (a `terminal` node) and the shell
written in Rust behind it:

- a prompt and line editing: backspace, Ctrl-C and Ctrl-L; arrow keys and other escape
  sequences are ignored;
- the commands `help`, `echo <text>`, `count`, `size`, `colors` and `clear`;
- above the terminal, its size and the buttons **Clear**, **Busy** and **Idle** (the last two set
  and clear the status Fyx draws on the terminal).

It needs only `ui:panel` and `terminal:pseudo` (T1): it runs no program.

## Build and run

The demo builds only with the SDK's `fyx-ui-0-4` feature; other builds show a panel that says
so. Build it on its own, so the feature does not reach the other demos:

```sh
cargo build --target wasm32-wasip2 -p fyx_demo_pseudo_terminal --features fyx_demo_pseudo_terminal/fyx-ui-0-4
```

`fyx_ext_harness` runs it headless (`tests/demos_v0_4.rs`). Fyx itself runs it once its host
has pseudo-terminals (plan 5.8) and the `terminal` node (plan 5.9).
