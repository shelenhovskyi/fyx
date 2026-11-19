# Cloud builds

Which Fyx builds and tests can run in a Claude Code cloud session (Linux container) instead of on
the Windows dev PC, the commands, and what has to stay on the PC.

Measured on 2026-11-19 on `integration` at `1a73059`, in a fresh cloud container: 4 cores, 15 GB
RAM, about 60 GB of writable disk, Ubuntu 24.04, x86_64. Every number below is a cold build (empty
`target/`), because a cloud container starts from a fresh clone every session.

## Results

| Step | Command | Result | Wall time |
| --- | --- | --- | --- |
| Toolchain | `rustup` reads `rust-toolchain.toml` (1.98.1 + `wasm32-wasip2`) on the first `cargo` call | ok | ~1 min |
| System packages | see [Setup](#setup) | ok | 9 s |
| Clippy, 14 Fyx crates | `cargo clippy $FYX_CRATES --all-targets -- --deny warnings` | green | 7 m 33 s |
| Test build | `cargo nextest run $FYX_CRATES --no-run` | ok | 5 m 55 s |
| Tests | `cargo nextest run $FYX_CRATES --no-tests=pass --no-fail-fast` | 920 run, 916 passed, 4 failed (fixed, see below) | 3 m 54 s |
| Demo extensions, fyx:ui 0.1 | `script/fyx-build-demos` | ok, 9 components | 37 s |
| Demo extensions, fyx:ui 0.4 | `script/fyx-build-demos --fyx-ui-0-4` | ok, 14 components | 48 s |
| wasmtime backend policy | `script/fyx-check-wasmtime-policy` | ok | 13 s |
| API reference up to date | `script/fyx-api-docs --check` | ok | 15 s |
| Licenses | `script/check-licenses` | ok | 3 s |
| Registry template policy tests | `cd tools/fyx-extensions-template/scripts && python3 -I -m unittest` | 44 ok | <1 s |
| fyx-extensions policy tests | `cd scripts && python3 -I -m unittest` (fyx-extensions repo) | 24 ok | <1 s |

A cold pass over all of it takes about 20 minutes and leaves `target/` at 15 GB. After that, a
change in one crate rebuilds in well under a minute (`cargo nextest run -p fyx_registry` after a
test edit: 29 s).

`FYX_CRATES` is the Fyx crate list:

```sh
FYX_CRATES="-p ui_ext -p ui_ext_host -p ui_ext_sdk -p ui_ext_sdk_v0_1 -p ui_ext_sdk_v0_2 \
  -p ui_ext_wasm -p agent_review -p fyx_registry -p fyx_ext -p fyx_ext_harness -p fyx_guard \
  -p fyx_guard_core -p fyx_relay -p fyx_api_docs"
```

The in-tree extensions (`extensions/fyx_*`) are workspace members, so `-p fyx_demo_todo` and so
on work too; `script/fyx-build-demos` builds them for `wasm32-wasip2` with the right features.

## Linux-only findings

- **`fyx_registry`: 4 tests failed on Linux only.** The CLI refuses a signing key file that group
  or others can read (`check_key_file_permissions`, Unix only), and the test fixture wrote its key
  files with the default umask (0644). Windows never runs that check, so the gate on the PC was
  green. Fixed in the test fixture (`Fixture::write_key` sets 0600); the CLI check is unchanged.
- **Code under `cfg(windows)` is not compiled or tested in the cloud**: parts of
  `ui_ext_wasm/src/process.rs`, `fyx_ext/src/package.rs`, `fyx_guard`, `fyx_guard_core/src/paths.rs`,
  `fyx_relay`, `agent_review/src/review_pane.rs` and `ui_ext_wasm/src/tests_v0_3.rs`. A change
  there still needs a check on the PC or in `fyx_gate.yml` (Windows runner). The reverse also
  holds: a cloud run covers `cfg(unix)` code the PC never builds.
- **`cargo-deny` has no prebuilt download here**: the network policy refuses GitHub release
  downloads (HTTP 403), so `cargo install cargo-deny --locked` builds it from crates.io. Without it
  `gate.sh` only warns; CI runs the advisory check.

## Setup

A fresh container has rustup and cargo but not the Linux GUI libraries GPUI links, nor nextest.
These lines make one ready (add them to the cloud environment's setup script so each session
starts with them):

```sh
apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq libasound2-dev libfontconfig-dev \
  libgit2-dev libglib2.0-dev libva-dev libvulkan1 libwayland-dev libx11-xcb-dev \
  libxkbcommon-x11-dev libzstd-dev llvm jq gettext-base elfutils musl-tools musl-dev
curl -sSfL https://get.nexte.st/latest/linux | tar zxf - -C ~/.cargo/bin
```

This is the apt part of `script/linux` without the packages the image already has. Disk: keep one
`target/` per session; 15 GB for the full Fyx set fits, two parallel target dirs do too, a `zed`
build on top of that is not measured.

## What runs where

**Cloud session** (one per stream, in parallel):

- `cargo check`, clippy and filtered nextest of any Fyx crate, including the ones that pull in GPUI
  (`ui_ext_host`, `agent_review`).
- Building the demo and agent extensions for `wasm32-wasip2`, for every world
  (`script/fyx-build-demos [--fyx-ui-0-2 | --fyx-ui-0-3 | --fyx-ui-0-4]`).
- The workspace-wide checks at the top of `gate.sh` (wasmtime policy, API docs, licenses), and the
  registry's Python tests in both repos.
- A pre-gate before handing a branch to W0. `script/gate.sh` guards its Windows-only steps
  (`powershell`, `cygpath`, `tasklist`) with `command -v`, but a whole `FAST=1` run on Linux was not
  measured here; the per-crate commands above are the tested path.

**Windows PC** (DESKTOP-8KUUFF7):

- The merge gate that decides a merge into `integration` (`script/gate.sh`, W0), because the target
  platform is Windows and `cfg(windows)` code is only built there.
- `FULL=1` gates and `fyx.exe` builds (`cargo build -p zed`) before a `fyx/main` push.
- Running a dev build of Fyx with extensions loaded, manual tests (`docs/fyx/manual-tests.md`) and
  anything that needs a window, a GPU or screenshots.
- Anything that needs the user's local credentials or signed-in tools.

## Rules for cloud sessions

- The `.claude/hooks/session-env.sh` paths (`CARGO_TARGET_DIR` from `.claude/fyx-targets.json`,
  WinGet, CMake) are for the PC; in the cloud the default `target/` is fine.
- `script/fyx-build-demos` writes `fyx_ui.wasm` next to each demo's `extension.toml`; the file is
  ignored by git, so it never shows up in a commit.
- A cloud run is not the merge gate: report its result in the progress file and let W0 gate on the
  PC.
