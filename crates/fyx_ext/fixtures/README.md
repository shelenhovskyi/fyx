# Test fixtures

Components with no logic that import and export everything in a `fyx:ui` world, so the tests can
check what `fyx-ext check` reads from imports, exports and custom sections. Regenerate them from
the repository root with `wasm-tools` 1.263 or newer:

```bash
wasm-tools component embed --dummy crates/ui_ext/wit --world ui-extension -o core.wasm
wasm-tools component new core.wasm -o crates/fyx_ext/fixtures/dummy-v0.1.wasm
wasm-tools component embed --dummy docs/fyx/wit-0.2/fyx-ui-0.2.wit --world ui-extension -o core.wasm
wasm-tools component new core.wasm -o crates/fyx_ext/fixtures/dummy-v0.2.wasm
```

They are committed rather than built in the tests because `wit-component`'s `dummy-module` feature
would add `wat` and `wast` to `Cargo.lock`.
