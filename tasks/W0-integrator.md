# W0 Integrator / CI (lead + owner)

**Owns:** `Cargo.toml`, `Cargo.lock`, `crates/ui_ext/wit/`, `.cargo/config.toml`, `script/gate.sh`, `FYX_UPSTREAM`, the `integration` branch.

**Deliverable**
- Merge worker branches into `integration` only through `script/gate.sh` (check + clippy + nextest on changed crates + license check). Red is never merged.
- `FULL=1 script/gate.sh` (full `zed` build) on a schedule, not per merge.
- Apply dependency and WIT change requests from other streams; announce WIT changes to W2, W3, W4, W8.
- Weekly job: rebase the Fyx stack onto the newest stable tag, open an issue on conflict.

**Done when:** `integration` builds `zed`, the app launches, and the demo panel works by hand.
