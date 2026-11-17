# Fyx CI

Fyx has two GitHub Actions workflows of its own. All files are named `.github/workflows/fyx_*.yml`
so they never collide with upstream's files on a rebase. The 46 workflows inherited from Zed are
still in `.github/workflows/`, unchanged and disabled (see below).

The authoritative merge gate is still `script/gate.sh`, run by the gatekeeper on the owner's
Windows PC before anything is merged into `integration`. CI runs the same script after the merge,
on a clean machine, so a gate that only passes on a warm local tree shows up red.

## `fyx_gate.yml`: the gate on a clean Windows runner

**Runs on:** every push to `integration` or `fyx/main`, and by hand (`workflow_dispatch`).
**Runner:** `windows-latest` (GitHub-hosted). **Concurrency:** one run per branch; a newer push
cancels the older run on the same branch.

Steps:

1. Enables `core.longpaths` for git (some git dependencies have very long paths).
2. Checks out the full history, blobless, because `gate.sh` diffs against the tag in
   `FYX_UPSTREAM` to find the changed crates.
3. Frees disk by deleting SDKs the build never uses (Android, Haskell, CodeQL, Go, Ruby, PyPy, Java,
   SQL Server, MongoDB, ...), then puts `CARGO_TARGET_DIR` on the drive with the most free space.
   The step logs free space before and after.
4. Installs the MSVC Spectre-mitigated libraries with the Visual Studio installer if the image lacks
   them for its default MSVC version. The `msvc_spectre_libs` build script fails without them.
5. Makes sure CMake is on `PATH` (installs it with Chocolatey if not).
6. Installs the toolchain pinned in `rust-toolchain.toml` (`rustup toolchain install` with no
   arguments reads the file), `cargo-nextest` and `cargo-deny` (prebuilt binaries through
   `taiki-e/install-action`), and sccache.
7. Runs `bash script/gate.sh` with `CARGO_INCREMENTAL=0`. sccache is the `rustc` wrapper (set in
   `.cargo/config.toml`) and stores its cache in the GitHub Actions cache
   (`SCCACHE_GHA_ENABLED=true`), so later runs reuse compiled crates.
8. Always prints free disk, the size of the target directory and sccache statistics, so you can
   see how close a run came to filling the disk.

Before anything else, `gate.sh` runs the two workspace-wide checks described in
[Advisories and the wasmtime backend policy](#advisories-and-the-wasmtime-backend-policy). Then it
checks, lints and tests every crate changed since the upstream tag, which today includes
upstream crates with `fyx:` hooks (`zed`, `workspace`, `project`, `agent_ui`, `acp_thread`,
`extension_host`, ...), not only the Fyx crates. `FULL=1` (the full `zed` binary build) is not run
in CI.

<!-- TRIAL-RESULTS -->

### Reading a red run

The gate pipes cargo's output through `tail`, so the step log shows the last lines of the failing
command. For the whole output, run `bash script/gate.sh` locally or re-run the failing `cargo`
command with the same `-p` list (printed as the first line, `gate: <crates>`).

## Advisories and the wasmtime backend policy

Platform spec decision 27 (plan task 1.10): extension code is compiled by **native Cranelift
only**, a wasmtime security release is adopted within **14 days**, and the gate fails on open
advisories. Two fast steps at the top of `gate.sh` enforce it. Both run on every gate, also when no
crate changed, because a new advisory or a feature flipped by a dependency bump needs no Fyx diff.

### Why Cranelift only

Every extension, `fyx:ui` component and agent tool runs inside wasmtime, so a wasmtime compiler bug
is a sandbox escape. wasmtime has three ways to run code: Cranelift (the optimizing native
compiler, the most fuzzed and the default), Winch (a fast baseline compiler) and Pulley (a portable
interpreter for targets without a native backend). In 2026 the critical sandbox escapes were in
Winch and in non-x86 backends (April: CVE-2026-34987 in Winch, CVE-2026-34971 in aarch64
Cranelift). Every extra backend is more code that an attacker-supplied module can reach and that
Fyx has to patch within 14 days. Fyx needs only Cranelift on the host architecture, so it does not
ship the others.

### What the workspace enables today

`cargo tree -e features -i wasmtime --target all` shows one wasmtime (48.0.7) with these features:
`anyhow`, `async`, `component-model`, `cranelift`, `demangle`, `incremental-cache`,
`parallel-compilation`, `runtime` and their implied `std`, `once_cell`,
`wasmtime-jit-icache-coherence`, `wasmtime-unwinder`, all from the workspace dependency in the
root `Cargo.toml` (identical to upstream v1.28.1), which `extension_host` uses. tree-sitter's
`wasmtime-c-api-impl` adds `gc` and `gc-null`. No `winch`, `pulley` or `all-arch`, and no Winch
crate in the graph.

`extension_host` is the only Fyx-relevant wasmtime user: upstream's extension runtime and Fyx's
`fyx:ui` runtime (`crates/extension_host/src/wasm_host/fyx_ui.rs`) both build their `Engine` with
the default strategy, which is Cranelift when Winch is not compiled in. `ui_ext_host` reaches
wasmtime only through `extension_host`. The other wasmtime users, `extension_cli` and
`edit_prediction_cli`, are upstream tools with no extra features. There is no `ui_ext_wasm` crate.

One exception is allowed: `wasmtime-internal-cranelift` and `cranelift-codegen` have their `pulley`
feature on, because wasmtime 48 turns it on whenever `cranelift` is on. That compiles Cranelift's
Pulley lowering in but leaves it unreachable: without wasmtime's own `pulley` feature,
`Engine::new` rejects a Pulley target ("does not match the host").

### How it is enforced

1. **`script/fyx-check-wasmtime-policy`** always runs, locally and in CI, in about 6 s. It reads the
   resolved features with `cargo tree --locked --target all -i <crate> --depth 0` (all targets,
   dev-dependencies included, the same feature unification a workspace build gets) and fails if:
   - `wasmtime` has `winch`, `pulley`, `all-arch` or `profile-pulley`, or lacks `cranelift`;
   - `wasmtime-internal-cranelift` or `cranelift-codegen` has `all-arch`;
   - `wasmtime-internal-winch` or `winch-codegen` is built at all. (A crate that is in
     `Cargo.lock` only through a weak `dep?/feature` is not built and does not count.)

   For each hit it prints `wasmtime-policy: RED: <reason>` and the inverted feature tree that shows
   who enables it, then `gate.sh` prints `gate: RED` and stops.
2. **`cargo deny --locked --config script/fyx-deny.toml check advisories bans`** checks the
   RustSec database for every crate in the graph on every target, and repeats the wasmtime bans
   above as `[bans]` rules (`deny` for the Winch crates, `[[bans.features]]` for the features).
   It only runs `advisories` and `bans`: licenses are `script/check-licenses`' job, and duplicate
   versions and wildcards are left to upstream (`multiple-versions` and `wildcards` are `allow`).
   Yanked crates only warn. `collab` is excluded from the graph, as in the rest of the gate: it is
   server-only and Fyx never builds it. Upstream Zed has no `deny.toml`, so the config is
   Fyx-owned and lives under `script/` to keep the repository root upstream's.

   cargo-deny is optional locally: without it `gate.sh` prints
   `gate: WARNING: cargo-deny is not installed, so the advisory check is SKIPPED here (CI runs it)`
   and goes on. In CI (`CI=true`) a missing cargo-deny is `gate: RED`. To run it locally, install
   it once with `cargo install cargo-deny --locked`. It clones the advisory database into
   `~/.cargo/advisory-dbs` on the first run and fetches updates on later runs, so it needs network.

The two layers overlap on purpose: the script needs nothing but cargo, so the policy holds on a
machine without cargo-deny, and cargo-deny repeats it in CI. Keep their lists in sync.

### When the advisory check is red

- **A wasmtime advisory** (`wasmtime`, `wasmtime-wasi`, `wasmtime-*`, `cranelift-*`): never
  ignored. The integrator bumps wasmtime within 14 days of the release, usually a patch release
  inside the pinned line: `cargo update -p wasmtime -p wasmtime-wasi`.
- **A fix exists inside the semver range upstream allows**: the integrator runs
  `cargo update -p <crate>` (or `--precise <version>`) and commits `Cargo.lock`. No ignore.
- **Upstream-only and not fixable by Fyx**: no patched release, or an upstream crate's version
  requirement excludes the fix, and the crate is reachable without any Fyx crate. Add
  `{ id = "RUSTSEC-…", reason = "<crate> via <path>; <why>" }` to `ignore` in
  `script/fyx-deny.toml` with an `# expires YYYY-MM-DD` comment (at most three months out), and a
  row in the table below. Re-triage every ignore on each upstream rebase and by its expiry date.
- **Pulled in by a Fyx crate**: fix it in the Fyx crate. Not ignorable.

### Advisory triage (2026-11-17, `integration` at `1ec364b`, upstream v1.28.1)

"Who pulls it in" was checked twice: every hit is reachable from the workspace with all Fyx crates
(`ui_ext*`, `agent_review`, `fyx_demo_*`) and `collab` removed, and the same version is in
upstream v1.28.1's `Cargo.lock`. No hit comes from a Fyx crate or a dependency Fyx added. The path
is one shortest path from `zed` or another workspace member.

**Fixable with a lockfile update (no ignore; red until the integrator commits `Cargo.lock`):**

| Advisory | Crate | Kind | Path | Who | Fix |
| --- | --- | --- | --- | --- | --- |
| RUSTSEC-2026-0422 | wasmtime 48.0.7 | vulnerability: typed functions with `i31`/`nullref` arguments may panic | `ui_ext_host` → `extension_host` → `wasmtime` | upstream, Fyx runtime depends on it | 48.0.9 |
| RUSTSEC-2026-0423 | wasmtime 48.0.7 | vulnerability: cross-engine type confusion with host-created struct types | same | same | 48.0.9 |
| RUSTSEC-2026-0424 | wasmtime 48.0.7 | vulnerability: unchecked memory use when moving borrowed resources between components | same | same | 48.0.9 |
| RUSTSEC-2026-0420 | wasmtime-wasi 48.0.7 | vulnerability: wasip3 UDP `send` to a receive-only address | `extension_host` → `wasmtime-wasi` | same | 48.0.9 |
| RUSTSEC-2026-0533 | ammonia 4.1.8 | vulnerability: mutation XSS through embedded SVG/MathML | `docs_preprocessor` → `mdbook` → `ammonia` | upstream (docs tooling) | 4.1.9 |
| RUSTSEC-2026-0414 | pulldown-cmark 0.13.4 | vulnerability: exponential events with `ENABLE_WIKILINKS` | `markdown` → `pulldown-cmark` | upstream | 0.13.6 |
| RUSTSEC-2026-0433 | pulldown-cmark 0.13.4 | vulnerability: quadratic nested image wikilinks | same | upstream | 0.13.6 |
| RUSTSEC-2026-0486 | rustix 1.1.4 | unsound: xattr with a zero-length buffer | `tempfile` → `rustix` | upstream | 1.1.6 |
| RUSTSEC-2026-0386 | tokio 1.52.1 | unsound: `AsyncFd::new` violates I/O safety | `gpui_tokio` → `tokio` | upstream | 1.54.1 |
| RUSTSEC-2026-0391 | zstd 0.13.3 | vulnerability: abandoned stream data leaks into the next stream | `util` → `rust-embed` → `include-flate` → `zstd` | upstream | 0.13.5 |
| RUSTSEC-2026-0398 | zstd-safe 7.3.0 | vulnerability: `Seekable::decompress` sets a `Vec` length past capacity | `zstd` → `zstd-safe` | upstream | 7.4.1 |
| RUSTSEC-2026-0403 | zstd-safe 7.3.0 | vulnerability: uninitialized memory after a failed legacy-frame decompression | same | upstream | 7.4.1 |
| RUSTSEC-2026-0410 | zstd-sys 2.0.16 | vulnerability: wasm allocator shim corrupts memory on allocation failure | `zstd-safe` → `zstd-sys` | upstream | 2.1.3 |

The integrator's update for all of them, a lockfile-only change (96 lines each way, no added or removed crates):

```bash
cargo update -p wasmtime -p wasmtime-wasi -p pulldown-cmark@0.13.4 -p rustix@1.1.4 -p tokio -p zstd -p zstd-safe -p zstd-sys
```

```bash
cargo update -p ammonia --precise 4.1.9
```

`--precise 4.1.9` keeps ammonia on 4.1: a plain `cargo update -p ammonia` goes to 4.2.5, which
adds a second html5ever, markup5ever and phf.

**Upstream-only, not fixable by Fyx (ignored in `script/fyx-deny.toml`, expire 2027-02-15):**

| Advisory | Crate | Kind | Path | Who | Why Fyx cannot fix it |
| --- | --- | --- | --- | --- | --- |
| RUSTSEC-2023-0071 | rsa 0.9.10 | vulnerability: Marvin timing side channel | `client` → `rpc` → `rsa` | upstream | fixed only in 0.10; `rpc` requires `^0.9` |
| RUSTSEC-2026-0459 | libsqlite3-sys 0.30.1 | vulnerability: bundled SQLite CVE-2026-119375 | `db` → `sqlez` → `libsqlite3-sys` | upstream | fixed only in 0.39; `sqlez` and `sqlx-sqlite` require `^0.30.1` |
| RUSTSEC-2026-0253 | lru 0.16.4 | unsound: use-after-free if `LruCache::pop` panics | `language` → `language_core` → `lru` | upstream | fixed only in 0.18.2; `language_core`, `agent_ui`, `aws-sdk-s3` require `^0.16` |
| RUSTSEC-2026-0194 | quick-xml 0.30.0, 0.37.5, 0.39.3 | vulnerability: quadratic duplicate-attribute check | `gpui` → `zed-scap` → `xcb` → `quick-xml`; `gpui_linux` → `wayland-client` → `wayland-scanner`; `accesskit_unix` → … → `zbus_xml` | upstream | fixed only in 0.41; `xcb` (build script), `wayland-scanner` and `zbus_xml` pin older lines |
| RUSTSEC-2026-0195 | quick-xml 0.30.0, 0.37.5, 0.39.3 | vulnerability: unbounded namespace allocation in `NsReader` | same | upstream | same |
| RUSTSEC-2024-0370 | proc-macro-error 1.0.4 | unmaintained | `xtask` → `gh-workflow` → `merge_derive` | upstream (dev tool) | no maintained release |
| RUSTSEC-2024-0384 | instant 0.1.13 | unmaintained | `repl` → `async-dispatcher` → `futures-lite` 1 → `fastrand` 1 | upstream | no maintained release |
| RUSTSEC-2024-0449 | unsafe-libyaml 0.2.11 | unmaintained | `agent_skills` → `serde_yaml_ng` | upstream | no maintained release |
| RUSTSEC-2025-0052 | async-std 1.13.2 | unmaintained (discontinued) | `fs` → `async-tar` → `async-std` | upstream | no maintained release |
| RUSTSEC-2025-0134 | rustls-pemfile 2.2.0 | unmaintained | `reqwest_client` → `zed-reqwest` | upstream | no maintained release |
| RUSTSEC-2025-0141 | bincode 1.3.3 | unmaintained | `cli` → `ipc-channel` | upstream | no maintained release |
| RUSTSEC-2026-0192 | ttf-parser 0.25.1 | unmaintained | `gpui_wgpu` → `cosmic-text` → `fontdb` | upstream | no maintained release |
| RUSTSEC-2026-0206 | rustybuzz 0.20.1 | unmaintained | `gpui` → `usvg` | upstream | no maintained release |
| RUSTSEC-2026-0370 | zvariant 5.11.0 | unmaintained | `gpui_linux` → `accesskit_unix` → `accesskit_atspi_common` | upstream | no maintained release |
| RUSTSEC-2026-0371 | zbus_names 4.3.2 | unmaintained | same → `atspi-common` | upstream | no maintained release |
| RUSTSEC-2026-0372 | zvariant_derive 5.11.0 | unmaintained | same → `zvariant` | upstream | no maintained release |
| RUSTSEC-2026-0441 | pin-utils 0.1.0 | unmaintained | `fs` → `async-tar` → `async-std` | upstream | no maintained release |
| RUSTSEC-2026-0542 | humantime 2.3.0 | unmaintained | `languages` → `pet` → `env_logger` | upstream | no maintained release |
| RUSTSEC-2026-0543 | chrono 0.4.42 | unmaintained | direct dependency of `gpui` and many upstream crates | upstream | no maintained release |

**Only through `collab` (excluded from the graph, so neither red nor ignored):**

| Advisory | Crate | Kind | Path |
| --- | --- | --- | --- |
| RUSTSEC-2026-0258 | h2 0.3.27 | vulnerability: unbounded empty DATA frames | `collab` → `reqwest` 0.11 → `h2` |
| RUSTSEC-2026-0387 | crypto-bigint 0.4.9, 0.5.5 | vulnerability: timing variability | `collab` → `aws-sdk-s3` (enables `sigv4a`) → `aws-sigv4` → `crypto-bigint` |
| RUSTSEC-2025-0134 | rustls-pemfile 1.0.4 | unmaintained | `collab` → `reqwest` 0.11 |

**Yanked (warnings only):** chacha20, scc, spin 0.9.8 and spin 0.10.0, all upstream.

## `fyx_rebase_check.yml`: weekly upstream rebase check

**Runs on:** Mondays at 06:17 UTC, and by hand. A manual run takes an optional `tag` input to try a
specific upstream tag instead of the newest one. **Runner:** `ubuntu-latest`.
**Permissions:** `contents: read`, `issues: write`. The checkout keeps no credentials, so the job
cannot push.

Steps:

1. Checks out `integration` with full history.
2. Lists the tags of `zed-industries/zed` and picks the newest stable one (`vX.Y.Z`, no `-pre`
   suffix). It fetches only that tag, into `refs/zed-upstream/<tag>`, so the fork's own copies of
   upstream tags are never overwritten.
3. In a throwaway local branch, runs `git rebase --onto <tag> $(cat FYX_UPSTREAM)`. That replays
   the Fyx stack (`FYX_UPSTREAM..integration`) onto the new tag, the same way the integrator would.
4. If the rebase conflicts, it records where the rebase stopped and its conflicted files, aborts,
   and runs `git merge-tree --merge-base=<FYX_UPSTREAM>` to list every file where the whole stack
   conflicts with the new tag (the rebase alone stops at the first conflict).
5. Issues:
   - On conflict, it opens an issue titled **"Rebase onto `<tag>` conflicts"** with both file lists
     and a link to the run. If such an issue is already open, it updates its title and body instead,
     and comments when the tag changed. There is at most one such issue.
   - When the rebase is clean, or `integration` is already on the newest tag, it closes the open
     issue with a comment, if there is one.

The rebased branch exists only on the runner and is never pushed. Doing the actual rebase,
updating `FYX_UPSTREAM` and re-running the gate stays a manual integrator task.

The schedule only fires for the copy of the workflow on the default branch (`fyx/main`), and
manual runs also need the file to be on `fyx/main`.

## Known local issues

Things the gate works around on one machine only. CI never takes these paths.

- **`example.com` resolves to a loopback address** (found 2026-11-18 on the Windows dev machine:
  its DNS answers `example.com` A with `127.0.0.1`; the AAAA records are real). Upstream's
  `agent` tests `test_fetch_tool_follows_same_host_redirect`,
  `test_fetch_tool_refuses_redirect_to_loopback` and `test_fetch_tool_reauthorizes_redirect_to_new_host`
  resolve `example.com` for real, so the fetch tool's loopback check refuses it and they fail.
  They fail the same way on `integration` without any Fyx change to `crates/agent`. When `agent`
  is in the gated set and `example.com` resolves to `127.*`, `script/gate.sh` skips exactly
  these three with a `gate: WARNING` line. Fix on the machine (find what filters its DNS) and the
  skip turns itself off.

## Secrets

None. Both workflows use only the automatic `GITHUB_TOKEN`, with the permissions listed above. The
sccache step uses the Actions cache credentials that every job gets. Nothing needs to be added in
**Settings → Secrets and variables → Actions**.

## Zed's inherited workflows

The 46 upstream workflows stay in `.github/workflows/` and are disabled on this repository through
the GitHub API. Do not delete or edit them: they belong to upstream, and changing them would
conflict on every rebase. Most of them would not work here anyway: they use Zed's self-hosted and
Namespace runners, Zed's secrets, and Zed's infrastructure.

To re-enable one, for example `run_tests.yml`:

- In the browser: **Actions** tab → pick the workflow in the left sidebar → **Enable workflow**.
- With the GitHub CLI:

  ```bash
  gh workflow enable run_tests.yml --repo shelenhovskyi/fyx
  ```

- With the REST API: `PUT /repos/shelenhovskyi/fyx/actions/workflows/run_tests.yml/enable`.

To disable it again, use **Disable workflow** in the same menu, `gh workflow disable`, or
`PUT .../disable`. Check the workflow's `runs-on:` and `secrets.*` first: a workflow that needs a
runner label the repository does not have waits in the queue until it times out.

If you need a modified copy of an upstream workflow, add it as a new `fyx_*.yml` file instead of
editing the original.
