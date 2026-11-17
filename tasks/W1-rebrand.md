# W1 Rebrand and de-cloud

**Owns:** `crates/zed` (brand and paths only), `assets/settings/fyx.json`, `crates/client` (URL scheme), `crates/paths`, `crates/cli` (binary name and lookup), `crates/release_channel`, `crates/windows_resources`, `crates/zed/resources/windows/zed.iss`, the startup-network hook in `crates/project/src/agent_registry_store.rs`.

**Deliverable**
- Name Fyx, bundle ID `dev.fyx.Fyx`, data/config dirs `~/.config/fyx` etc., URL scheme `fyx://`, so it coexists with an installed Zed.
- Telemetry off by default and upload stubbed; `server_url` pointing nowhere (or own server); zed.dev sign-in and collab panels hidden; auto-update feed disabled. Keep BYO-key providers and ACP agents.
- Stub `client` rather than deleting it: `project` and `workspace` reference it for collab.
- Every edit marked `fyx:`; prefer settings, env vars and feature flags over code edits.

**Look first:** hardcoded `https://zed.dev`, `cloud.zed.dev`, `zed.dev/releases`; crates `telemetry`, `crashes`, `auto_update*`, `collab_ui`, `onboarding`.

**Done when:** a dev build launches as Fyx alongside Zed, and no request leaves the machine at startup (check with Little Snitch or `nettop`).
