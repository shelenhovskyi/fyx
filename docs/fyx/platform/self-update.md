# Self-update (plan 6.2)

Status: design and core only. `crates/fyx_update` holds the pure logic and its tests. Nothing is
wired into the app yet, and no release key exists yet.

## Goals

- Fyx checks for new versions only when the user turns this on (`fyx.updates.check`, default
  off). Zed's own update endpoint stays off.
- Fyx never installs anything without consent. The user sees "Fyx 1.0.1 is available" with
  "What's new" and "Restart to update". Nothing changes before that button is pressed.
- Trust comes from one Ed25519 release key owned by Fyx. There is no notarization or Authenticode
  yet, because Fyx has no Apple Developer account and no Windows code-signing certificate. Those
  checks are added beside this one later, never instead of it.
- A failed update can always be rolled back to the previous version.

## Where updates come from

Releases of `shelenhovskyi/fyx` on GitHub. Each release carries, per channel:

- `fyx-<channel>.json`: the manifest.
- `fyx-<channel>.json.sig`: one line, `<key id> <base64 Ed25519 signature>`, over the manifest
  file's exact bytes. There is no canonical JSON to get wrong.
- The artifacts the manifest names.

The client reads `https://github.com/shelenhovskyi/fyx/releases/latest/download/fyx-<channel>.json`
and its `.sig`. These are two small GETs with no cookies and no identifiers beyond what any HTTP
request carries.

## Manifest (`fyx-release/1`)

```json
{
  "schema": "fyx-release/1",
  "channel": "stable",
  "version": "1.0.1",
  "published_at": 1800000000,
  "notes_url": "https://github.com/shelenhovskyi/fyx/releases/tag/v1.0.1",
  "artifacts": [
    { "target": "aarch64-apple-darwin", "kind": "mac-app-tar-gz",
      "url": "https://github.com/shelenhovskyi/fyx/releases/download/v1.0.1/Fyx-aarch64.app.tar.gz",
      "size": 123456789, "sha256": "…" }
  ]
}
```

Unknown fields are refused, so a new field means a new schema.

## Checks (`fyx_update`)

1. `verify_manifest` runs these checks in order:
   1. The key id must be compiled in (`RELEASE_KEYS`).
   2. The signature must be valid over the exact bytes.
   3. The JSON must parse, and the schema must match.
   4. The channel must be this build's channel.
   5. `published_at` must be no more than 10 minutes in the future and no older than 30 days.
      That window limits freeze attacks: an attacker can replay an old manifest only until it
      expires.
2. `offer` compares the manifest's version with the current version and with the highest version
   any verified manifest has shown before, which the host stores. A replayed older manifest
   therefore never offers a downgrade. The result is one of:
   - `Update` with the artifact for this target;
   - `UpToDate`;
   - `NoBuildForTarget`.
3. `check_artifact` checks the download's size and then its SHA-256 against the manifest before
   anything is unpacked.
4. `install_plan` and `rollback_plan` return rename steps for the host to run on restart.
   - `Layout::beside(installed)` puts the staged copy at `.<name>.fyx-update` and the previous
     copy at `.<name>.fyx-previous`, both next to the installed app on the same volume, so each
     rename is atomic.
   - Install: remove the old previous copy, move installed to previous, move staged to installed.
   - Rollback: remove the staged copy, move installed to staged, move previous to installed.

## What the host does (not wired yet)

- The update setting is off by default. When it is on, the host checks once a day and on
  "fyx: check for updates".
- The host downloads to `<data dir>/updates`, runs `check_artifact`, and unpacks into the staged
  path.
- macOS: the unpacked `.app` is ad-hoc signed, like local builds (see `docs/fyx/mac-build.md`).
  Gatekeeper's quarantine attribute is not set by Fyx's own download, so the app opens without the
  first-run prompt. This is a trade-off the owner accepted in 7.1.
- Windows: a running exe cannot be renamed while Fyx is open. A small helper waits for Fyx to exit,
  runs the plan, and starts the new version.
- If the new version does not reach "started" within 60 seconds, two times in a row, the host
  rolls back and tells the user. "fyx: roll back update" does the same on request.

## Open items

- The owner generates the release key with `ring` or `openssl genpkey -algorithm ed25519`. The
  public half goes into `RELEASE_KEYS`; the private half goes only into the release workflow's
  secret. No key is committed.
- A release workflow that builds, writes the manifest, signs it, and uploads all the files.
- The host wiring above, with its setting, notification, and Windows helper.
- Workspace lines for W0, the integrator. These were not added to the root `Cargo.toml`:
  - in `members`: `"crates/fyx_update",`
  - in `[workspace.dependencies]`: `fyx_update = { path = "crates/fyx_update" }`

  All of the crate's dependencies are existing workspace dependencies: `base64`, `hex`, `ring`,
  `semver`, `serde`, `serde_json`, `sha2`, and `thiserror`.
