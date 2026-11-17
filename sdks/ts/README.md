# Fyx UI extensions in TypeScript (prototype)

This prototype tests whether Fyx UI extensions can be written in TypeScript and compiled with
[`jco componentize`](https://github.com/bytecodealliance/jco) against the `fyx:ui@0.1.0` world
(`crates/ui_ext/wit/fyx-ui.wit`), and whether the resulting components meet the host's contract.

**Result: yes, with the StarlingMonkey backend.** The TypeScript counter demo is a valid
`fyx:ui@0.1.0` component. It loads under the same wasmtime 48 `Config` and WASI p2 linker that
`crates/extension_host/src/wasm_host/fyx_ui.rs` uses, and it passes the Rust counter's behaviour
contract call for call. What it costs: a component **12× the size** of the Rust demo, about
**4.7 s** of cold Cranelift compile on every Fyx start, and about **36 MiB** of compiled code,
which is more than Fyx's whole 32 MiB compilation cache. Per-call latency is well under a
millisecond. The QuickJS backend gives much smaller components, but **it corrupts u32 values above
2³¹ − 1** and fails the contract, so it is not usable yet.

Nothing outside `sdks/ts/` is used except the WIT file, which is read in place.

## Layout

| Path | What it is |
|---|---|
| `fyx-ui-sdk/src/fyx-ui-sdk.ts` | The SDK: props builders, `TreeBuilder`, `PatchSet`, `SurfaceTree`, list-row helpers, `registerExtension`. It mirrors `crates/ui_ext_sdk`. |
| `fyx-ui-sdk/src/wit/` | Types generated from the WIT (`npm run gen-types`, which runs `jco guest-types`). |
| `demo-counter/src/counter.ts` | The counter demo, with the same contract as `extensions/fyx_demo_counter`. |
| `demo-counter/extension.toml` | Manifest, so a Fyx dev build can load `demo-counter/` through `FYX_UI_DEV_EXTENSIONS`. |
| `demo-counter/component.wit` | `wasm-tools component wit` of the built component, rewritten by `npm run build`. |
| `scripts/build-component.mjs` | componentize, then add the `fyx:ui-api-version` section, then write the WIT. |
| `scripts/api-version-section.mjs` | Reads and appends the version custom section. |
| `test/` | Node tests: the SDK, the demo sources, and the built components through `jco transpile`. |
| `host-check/` | A standalone Rust tool that loads a component the way Fyx does, times it and replays the contract. It is not a member of the Fyx workspace. |

## Use

You need Node 22.18 or later, which runs `.ts` files directly with type stripping.
`wasm-tools` is optional; without it, the build prints the WIT with `jco wit` instead.

```bash
cd sdks/ts && npm ci
```

```bash
npm run check
```

`check` runs `tsc`, builds all three profiles, then runs `npm test`. To build only the
recommended profile, run `npm run build`. It writes `demo-counter/dist/fyx_ui.wasm`. To try the
demo in a Fyx dev build:

```bash
FYX_UI_DEV_EXTENSIONS=$PWD/demo-counter ./target/debug/fyx --foreground
```

This was not run in this prototype, because it has no Fyx app build. `host-check` stands in for
it.

To load a component with Fyx's runtime settings, outside Fyx:

```bash
cargo run --release --manifest-path host-check/Cargo.toml -- demo-counter/dist/fyx_ui.wasm --runs 15
```

The first build of `host-check` compiles wasmtime and takes a few minutes. Fyx's
`.cargo/config.toml` wraps rustc in `sccache`, so if `sccache` is not installed, prefix the command
with `RUSTC_WRAPPER=`.

## Writing an extension

```ts
import { FlexProps, Surface, SurfaceTree, TextProps, registerExtension } from 'fyx-ui-sdk';
import type { Extension, Node, UiEvent, Update } from 'fyx-ui-sdk';

class Hello implements Extension {
    private tree = new SurfaceTree();
    private greeting = 0;
    private button = 0;

    surfaces(): Surface[] {
        return [Surface.panel('hello', 'Hello', 'right', 'sparkle', 240)];
    }

    mount(surfaceId: string): Node[] {
        return this.tree.mount((tree) => {
            tree.column((tree) => {
                this.greeting = tree.text('Hello');
                this.button = tree.button('Wave');
            });
        });
    }

    handleEvent(surfaceId: string, event: UiEvent): Update {
        if (event.tag === 'click' && event.val === this.button) {
            this.tree.setProps(this.greeting, new TextProps('Hello, world').bold());
        }
        return this.tree.takeUpdate();
    }
}

export const { surfaces, mount, handleEvent, listRows } = registerExtension(() => new Hello());
```

The API follows the Rust SDK closely:

| Rust (`ui_ext_sdk`) | TypeScript (`fyx-ui-sdk`) |
|---|---|
| `impl Extension` + `register_extension!(T)` | `implements Extension` + `export const { surfaces, mount, handleEvent, listRows } = registerExtension(() => new T())` |
| `Result<_, String>` from `mount` / `handle_event` | throw. The `message` of the thrown value becomes the WIT `err`. |
| `TextProps::new("x").bold()`, `FlexProps::column().gap(8)` | `new TextProps('x').bold()`, `FlexProps.column().gap(8)` |
| `Props`, `Update`, `UiEvent` enums | the WIT variants as jco maps them: `{ tag: 'click', val: id }`. Enums are string unions such as `'filled'`. |
| `SurfaceTree`, `TreeBuilder`, `PatchSet`, `IdAllocator`, `list_row`, `list_rows`, `row_range` | the same names in camelCase. `len()` becomes `size` / `length`. |
| `TreeError` variants | `TreeError` with a `detail.kind` discriminant. Messages match the Rust `Display` output. |
| `Surface::panel(..)`, `Surface::status_item(..)` | `Surface.panel(..)`, `Surface.statusItem(..)` |
| `log`, `push_update` | `log`, `pushUpdate` |
| `ROW_NODE_ID_BASE`, `API_VERSION`, `TreeLimits::default()` | `ROW_NODE_ID_BASE`, `API_VERSION`, `DEFAULT_TREE_LIMITS` |

Where TypeScript cannot give the Rust guarantees, the SDK checks at runtime instead:

- **Integers.** The builders throw a `RangeError` for numbers that do not fit their WIT type (`u16`
  gap, `u32` row count, and so on). Otherwise the component model traps while lowering the value,
  and a trap disables the extension.
- **Aliasing.** Builders are mutable for chaining, but `intoProps` copies their fields. The
  `SurfaceTree` mirror stores copies too, so changing a builder or props object later never
  changes a node that was already built.
- **Equality.** `setProps` compares props structurally. A missing key and an `undefined` option
  count as equal.
- **Row checks.** The host's checks on list rows (`ui_ext::Tree::from_row`) are ported to
  `rowRejection`, with the host's messages. The Rust SDK calls `ui_ext` itself.
- **Exceptions in `list-rows` and `surfaces`.** These exports have no error channel. In Rust a
  panic there traps. Uncaught exceptions are far more common in JS, so `registerExtension` logs a
  throwing `listRows` and answers no rows. `surfaces` logs the error and rethrows it (a trap),
  as in Rust.

The sources use only erasable TypeScript: no enums, namespaces or parameter properties. That way
Node runs them as they are in tests, and jco's bundler (rolldown) compiles them without a
separate `tsc` step.

## Findings

All numbers are from one run in a 4-vCPU Linux cloud container. Read them as orders of magnitude.
The wasmtime numbers come from `host-check` (wasmtime 48.0.7, Cranelift, parallel compilation,
epoch interruption, `wasmtime_wasi::p2::add_to_linker_async`), which matches Fyx's runtime. The
Rust demo is `cargo build --release --target wasm32-wasip2 -p fyx_demo_counter`.

| | Rust demo | TS, StarlingMonkey (recommended) | TS, StarlingMonkey, no WASI | TS, QuickJS |
|---|---|---|---|---|
| Component size | 0.96 MiB (0.14 MiB stripped of debug info) | 11.92 MiB | 11.91 MiB | 1.70 MiB |
| gzip -9 | 0.27 MiB (0.05 MiB stripped) | 3.84 MiB | 3.83 MiB | 0.64 MiB |
| Build (componentize / cargo) | ~5 min cold, workspace deps | 4.3 s | 4.3 s | 1.2 s |
| Cold compile, every Fyx start | 68 ms | **4.7 s** | 4.3 s | 0.71 s |
| Incremental-cache entries (cap: 32 MiB for all extensions) | 0.42 MiB | **35.7 MiB** | 35.7 MiB | 5.1 MiB |
| Recompile with a warm cache | 42 ms | 1.5 s | 1.7 s | 0.26 s |
| Instantiate (budget 2 s) | 0.05 ms | 0.08 ms | 0.06 ms | 0.13 ms |
| First `surfaces()` (budget 2 s) | 0.02 ms | 0.69 ms | 0.68 ms | 0.18 ms |
| First `mount` / first click (budget 200 ms) | 0.01 / 0.00 ms | 0.48 / 0.57 ms | 0.46 / 0.57 ms | 0.22 / 0.21 ms |
| Resident memory per mounted instance | 0.03 MiB | 2.6 MiB | 2.5 MiB | 0.65 MiB |
| WASI imports | 15 (`wasi:cli/*`, `io`, `clocks`, `random/insecure-seed`, all `@0.2.9`) | 7 (`io/poll`, `io/error`, `io/streams`, `cli/stderr`, `clocks/monotonic-clock`, `clocks/wall-clock`, `random/random`, all `@0.2.12`) | none | none (stubbed) |
| Counter contract (`host-check` and Node) | pass | **pass** | pass | **fail** (see below) |

What these numbers mean for the host:

- **Exports.** The component exports exactly the world's four functions with the WIT's
  signatures. The full output is in `demo-counter/component.wit`:

  ```wit
  export surfaces: func() -> list<surface>;
  export mount: func(surface-id: string) -> result<list<node>, string>;
  export handle-event: func(surface-id: string, event: ui-event) -> result<update, string>;
  export list-rows: func(surface-id: string, list-node: node-id, start: u32, count: u32) -> list<list-row>;
  ```

- **Version section.** jco has no way to emit a custom section. `scripts/api-version-section.mjs`
  appends a top-level custom section `fyx:ui-api-version` holding `00 00 00 01 00 00` (three
  big-endian u16s: 0.1.0). It refuses to add a second copy, because the host rejects two.
  `host-check` reads it with the host's own logic (`wasmparser` `parse_all`), and so does
  `wasm-tools objdump`. Without the section, the host would fall back to the `fyx:ui/host@0.1.0`
  import, which every component built with this SDK has, but the section is what Fyx's docs
  require.

- **WASI.** jco's defaults import `wasi:http/types` and `wasi:http/outgoing-handler`. Fyx links
  `wasmtime_wasi::p2` but not `wasi:http`, so such a component **fails to instantiate** (wasmtime:
  "matching implementation was not found in the linker"). `--disable http` alone still imports
  `wasi:http/types`. The recommended profile is `--disable http fetch-event stdio`, which leaves
  clocks, random, stderr and io. jco 1.42 targets WASI 0.2.12 while wasmtime 48 implements an
  earlier 0.2.x; wasmtime's semver matching links them, and this was verified by instantiation.
  The host's WASI context inherits only stderr and preopens nothing, so the filesystem and
  environment imports that the other profiles drop would be empty anyway.

- **Startup is dominated by compilation, not by instantiation.** StarlingMonkey is snapshotted
  with Wizer at build time, so instantiation and the first call are fast and far under the 2 s and
  200 ms budgets. However, Cranelift needs about 4.7 s for the 12 MiB engine. Fyx compiles on a
  background task outside the budgets, so nothing times out, but the panel appears seconds after
  startup. Fyx's incremental cache is in-memory only, so this happens on every launch. One
  StarlingMonkey extension also fills more than the cache's whole 32 MiB cap, which would evict the
  other extensions' entries and keep its own reloads near cold speed. Each TS extension carries its
  own copy of the engine, so N extensions mean N × 12 MiB to load and about N × 4.7 s of compile
  CPU.

- **Node.** Under `jco transpile`, Node (V8) compiles and instantiates the StarlingMonkey
  component in about 60–90 ms, and its first call takes about 20–30 ms. That is fine for tests, but
  it says nothing about Fyx, which runs Cranelift.

## Limits and gotchas

- **QuickJS backend is broken for this world** (componentize-qjs 0.5, through jco 1.42). Two
  issues:
  1. By default it uses the component-model async ABI (`waitable.join`). Fyx's wasmtime `Config`
     does not enable component-model-async, so wasmtime refuses to compile the component. The
     `quickjs` profile passes `--backend-qjs-disable-async` to avoid this.
  2. **It lowers u32 values above 2³¹ − 1 as 2³¹ − 1.** List rows use ids from `0x8000_0000`
     (`ROW_NODE_ID_BASE`, fixed by the Rust contract), so every row comes out as
     `[(2147483647, none), (2147483647, some(2147483647))]`, which the host rejects as a duplicate
     id. The Node component test marks this as `todo`, and `host-check` fails on it. StarlingMonkey
     lowers these values correctly. Report it upstream before considering QuickJS. It would bring
     the size down to 1.7 MiB and the compile to 0.7 s.
- **Time and randomness depend on the WASI profile.** With `starlingmonkey-no-wasi`, `Date.now()`
  returns the build time, and `Math.random()` replays the same sequence in every instance: both
  are frozen into the Wizer snapshot. This was verified by instantiating a probe twice, a second
  apart. With the recommended profile, both behave normally. On QuickJS with stubbed WASI,
  `Date.now()` traps (the extension is disabled) and `Math.random()` is deterministic.
- **`console.*` writes nowhere** in every profile here: stdio is disabled, and QuickJS has no
  `console` at all. Use `log()`, which goes to Fyx's log.
- **No timers in 0.1**, as for Rust: a component runs only while the host calls it, so a
  `setTimeout` callback has no moment to fire. Keep extension code synchronous: the exports must
  return their values, not promises. The SDK is synchronous throughout.
- **Errors.** A thrown value becomes the `err` string of `mount` and `handle-event`. An uncaught
  exception anywhere else traps, and a trap disables the extension until Fyx restarts. The SDK
  catches exceptions in `listRows` for this reason (see above).
- **Bundling.** `jco componentize` bundles TypeScript with rolldown automatically. It treats the
  `fyx:ui/*` imports as external, which is correct, and prints a notice about them. Any npm
  dependency is bundled into the component, so keep dependencies small. Node built-ins were not
  tried.
- **The WIT is frozen and read in place.** `fyx-ui-sdk/src/wit/` is generated from it. If the
  world changes, run `npm run gen-types` and update `API_VERSION`. A test checks `API_VERSION`
  against the WIT's `package` line.
- **Not verified here:** loading in a real Fyx build (no app build in this prototype; `host-check`
  is the stand-in), Windows and macOS, and jco's `--aot` (Weval) mode.

## Verdict

The contract works end to end with StarlingMonkey, and the SDK can mirror the Rust API almost one
to one. The cost is in loading: a 12 MiB engine per extension, seconds of compile on every launch,
and more compiled code than Fyx's whole cache. Before TypeScript becomes a supported path, one of
these would help:

1. Persist the incremental compilation cache to disk, or give UI components their own cache, with
   a larger cap.
2. Share one engine between all JS extensions. jco's `--script-only` mode builds one engine
   component plus small script components, which would need host support.
3. Wait for the QuickJS u32 fix: 1.7 MiB and 0.7 s per extension.
