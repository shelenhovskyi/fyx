# W0 Integrator / CI (lead + owner)

**Owns:** `Cargo.toml`, `Cargo.lock` (and fixture lockfiles), `crates/ui_ext/wit/`, `.cargo/config.toml`, `script/gate.sh`, `script/fyx-prune-target`, `script/fyx-check-licenses-index`, `script/fyx-check-wasmtime-policy`, `script/fyx-deny.toml`, `FYX_UPSTREAM`, `tasks/wit-0.2.md`, the `fyx_*` CI workflows in `.github/workflows/`, upstream test expectations changed by the rebrand, the `integration` branch.

**Deliverable**
- Merge worker branches into `integration` only through `script/gate.sh` (wasmtime backend policy + `cargo deny` advisories, then check + clippy + nextest on changed crates + license check). Red is never merged.
- `FULL=1 script/gate.sh` (full `zed` build) on a schedule, not per merge.
- Gate modes (coordinator with the user, 2026-11-19, second revision):
  - **FAST** (`FAST=1 script/gate.sh`) accepts every batch into `integration`. It tests the crates
    changed since the last green `fyx/main` plus their workspace reverse dependencies, limited to
    the crates Fyx changed since `FYX_UPSTREAM` (`script/fyx-gate-crates`; `zed` is in it whenever
    the `ui_ext` family changes). The workspace-wide steps (wasmtime policy, `cargo deny`, API docs,
    template tests, licenses, lockfiles) still run. WIT changes and new dependencies of Fyx crates
    need only FAST.
  - **FULL** (`FULL=1 script/gate.sh`): every crate Fyx changed since `FYX_UPSTREAM`, plus the
    `zed` build. It runs only before a push to `fyx/main`, at most twice a day: one batch around
    midday, one at the end of the day, or when the user needs a build. A batch also needs FULL
    when `Cargo.lock` changes the version of an existing crate, or when an upstream `fyx:` hook
    touches a crate with many dependents (`editor`, `workspace`, `project`, `gpui`).
  - Push `integration` after a green FAST gate. Fast-forward and push `fyx/main` only after a
    green FULL gate.
  - Measurement: every gate prints its wall time and keeps cargo's timing report of the test
    build (and of the `zed` build in FULL) in `D:\fyx-builds\gate-timings\<sha>-*.html`. No
    sccache (2% hits, coordinator 2026-11-19) and no separate `cargo check` (clippy covers it).
- Disk (coordinator, 2026-11-19; the user wants SSD-frugal builds): `script/fyx-disk-guard` runs
  after every FULL gate and every push (`cargo sweep --time 2` and `fyx-prune-target` on each idle
  target). Its WARN line (D: under 150 GB, E: under 40 GB) goes to the coordinator. When a worker
  is archived, its target folder is deleted the same day.
- Apply dependency and WIT change requests from other streams; announce WIT changes to W2, W3, W4, W8.
- Weekly job: rebase the Fyx stack onto the newest stable tag, open an issue on conflict.

**Done when:** `integration` builds `zed`, the app launches, and the demo panel works by hand.

**Shared cargo target (Windows dev box, 2026-11-20).** Every folder builds into
`D:/fyx-targets/shared` (on E:) through `script/fyx-cargo` (`script/gate.sh` and `script/clippy`
call it themselves; `guard-commands.sh` denies a bare `cargo build/check/test/nextest/clippy/run`).
It holds a lock in the target for the whole command, touches the files that differ from the
worktree that built last (cargo names workspace artifacts by their path relative to the workspace,
so worktrees share them and an older file here looks up to date), and prunes superseded hashes
(`script/fyx-prune-target`) when the target holds over 120 GB or E: has under 30 GB free.
