# Fyx

Fyx is a code editor forked from [Zed](https://github.com/zed-industries/zed). It adds two things
on top of the upstream editor:

- **A native UI API for WASM extensions.** Extensions written for `wasm32-wasip2` add dock panels
  and status bar items through the `fyx:ui` WIT world. They never draw: they describe a retained
  tree of nodes (flex, text, buttons, inputs, virtualized lists and tables, icons, markdown), and
  Fyx renders it with its own components, coloured by the active theme. Each extension runs
  sandboxed in its own wasmtime store, with no preopened directories, environment variables or
  arguments.
- **Hunk-level review of agent changes.** Edits proposed by an ACP agent become a host-side review
  model, where each hunk is accepted or rejected on its own before it lands.

How it fits together: [ARCHITECTURE.md](ARCHITECTURE.md). The sprint plan and its progress:
[PLAN.md](PLAN.md).

## Documentation

- [User guide](docs/fyx/user-guide.md): privacy defaults, agent change review, UI extensions,
  security modes, where Fyx keeps its files, troubleshooting.
- [Writing a UI extension](docs/fyx/extension-authoring.md), with the
  [SDK reference](docs/fyx/sdk-reference.md) and [limits and gotchas](docs/fyx/limits-and-gotchas.md).
- [Platform plan](docs/fyx/platform/plan.md): the road to third-party UI extensions, after the
  [platform spec](docs/fyx/platform/spec.md).
- [All Fyx docs and reviews](docs/fyx/README.md).

## Status

Fyx is a **sprint prototype**. It is built and tested as a **Windows dev build** only. There are no
releases, installers or signed binaries, and nothing here is a stable API: `fyx:ui@0.1.0` will
break in 0.2. UI extensions are installed from a folder or loaded from an environment variable;
there is no extension registry yet.

Fyx follows a pinned upstream stable tag (see [`FYX_UPSTREAM`](FYX_UPSTREAM)) and is rebased onto
newer tags. Fyx's CI is described in [docs/fyx/ci.md](docs/fyx/ci.md).

## Building on Windows

You need:

- [rustup](https://www.rust-lang.org/tools/install). The toolchain version, components and targets
  are pinned in [`rust-toolchain.toml`](rust-toolchain.toml); run `rustup toolchain install` in
  the repository root to install them.
- Visual Studio 2022 or newer, or the Build Tools, with the MSVC x64/x86 build tools, a Windows 10
  or 11 SDK, and the **MSVC Spectre-mitigated libraries** for x64/x86. The build fails without the
  Spectre libraries. The full component list is in
  [docs/src/development/windows.md](docs/src/development/windows.md).
- [CMake](https://cmake.org/download) on `PATH` (a wasmtime dependency needs it).
- [sccache](https://github.com/mozilla/sccache) on `PATH`. `.cargo/config.toml` sets it as the
  `rustc` wrapper, so the build fails if it is missing.
- Git Bash, to run the scripts in `script/`.
- Long paths enabled for Git (`git config --system core.longpaths true`) and Windows.

Then, from the repository root:

```bash
export CARGO_INCREMENTAL=0
cargo build -p zed
```

The binary is `target\debug\fyx.exe`. Run it with `cargo run -p zed`.

`CARGO_INCREMENTAL=0` matters: sccache cannot cache incremental builds, and incremental artifacts
grow the target directory by gigabytes. Expect the target directory to take 25–60 GB.

Before proposing a change, run the merge gate (check, clippy, tests and license check on every
crate changed since the upstream tag):

```bash
bash script/gate.sh
```

## Trying the demo extensions

Build the three demo extensions (counter, todo list, component gallery) for `wasm32-wasip2`:

```bash
bash script/fyx-build-demos
```

The script copies each component next to its `extension.toml` and prints a
`FYX_UI_DEV_EXTENSIONS=...` line. Set that variable and start Fyx; each demo adds a panel with a
button in its dock:

```bat
set FYX_UI_DEV_EXTENSIONS=C:\src\fyx\extensions\fyx_demo_counter;C:\src\fyx\extensions\fyx_demo_todo;C:\src\fyx\extensions\fyx_demo_gallery
target\debug\fyx.exe --foreground
```

To write your own UI extension, start with
[docs/fyx/extension-authoring.md](docs/fyx/extension-authoring.md), then the
[SDK reference](docs/fyx/sdk-reference.md) and [limits and gotchas](docs/fyx/limits-and-gotchas.md).

## Licenses

- The Fyx application is licensed under the **GPL-3.0-or-later** ([LICENSE-GPL](LICENSE-GPL)),
  like the upstream code it is built from. This includes `ui_ext_host` and `agent_review`.
- The extension API crates `ui_ext` (the WIT world and tree model) and `ui_ext_sdk` (the guest
  SDK), and the demo extensions, are licensed under the **Apache-2.0**
  ([LICENSE-APACHE](LICENSE-APACHE)), so extensions can use them under any license.
- Each crate states its license in its `Cargo.toml`. Third-party notices are generated with
  `script/generate-licenses`.

## Not affiliated with Zed Industries

Fyx is an independent fork. It is not affiliated with, endorsed by, or supported by Zed Industries,
Inc. "Zed" is used here only to name the upstream project Fyx is forked from. Please report Fyx
problems to this repository, not to the Zed project.
