# Local Mac build

Plan item 7.1. Fyx has no Apple Developer account yet, so a Mac build is local: built on your Mac,
signed ad-hoc, not notarized. It is for your own machine, not for handing out.

## Build and check

From the repository root on the Mac (Xcode 26 or newer with its macOS SDK, rustup, Node for
`dmg-license`):

```sh
script/fyx-mac-check
```

It runs the Fyx crates' tests, builds the `.app` and DMG with `script/bundle-mac`, checks the
bundle and starts it once with a temporary `--user-data-dir`. Options: `--skip-tests`,
`--skip-build` (check the last build), `--debug` (`bundle-mac -d`: faster, no DMG). The temporary
profile stays in `$TMPDIR/fyx-mac-check.*` for a look.

What it checks:
- bundle id and name of the channel in `crates/zed/RELEASE_CHANNEL` (`dev` gives "Fyx Dev",
  `dev.fyx.Fyx-Dev`), the Fyx icon and `Document.icns`, no Zed name in `Info.plist`;
- no `embedded.provisionprofile` (Zed's profiles in `crates/zed/contents/` are not copied);
- `codesign --verify --deep --strict` passes, the signature is ad-hoc, the entitlements are
  `crates/zed/resources/fyx.entitlements` (JIT for wasmtime, Apple Events; no camera,
  microphone, contacts, calendars, location or photos);
- the DMG `target/<triple>/release/Fyx-<arch>.dmg` exists and `hdiutil verify` passes; its
  click-through text is Fyx's GPL license, not Zed's Terms of Service;
- the app keeps running for 20 s, writes into the temporary data dir, and logs no panic.

Logs always go to `~/Library/Logs/Fyx/Fyx.log`, whatever the data dir; the check reads only the
lines its own launch appended. A non-dev channel allows one running instance per channel: quit
your Fyx of that channel before the check.

The bundle alone: `script/bundle-mac` (release + DMG), `script/bundle-mac -d` (debug `.app`),
`-i` installs into `/Applications`, `-o` opens the result.

`script/bundle-mac` ends with `script/fyx-prune-mac-target`: it deletes the units in
`target/<triple>/<profile>/deps` that the build did not use (old hashes from earlier merges, about
10 GB a day otherwise), and deletes `target/debug` whole when `/` is under 40 GB free
(`FYX_DISK_GUARD_MIN_GB`). `--dry-run` lists what it would delete.

## First launch (Gatekeeper)

A build you made on the same Mac is not quarantined and opens normally. A DMG or `.app` that came
from elsewhere (AirDrop, a download, a USB stick) carries the quarantine flag, and macOS refuses
an unnotarized app with "cannot be opened" or "is damaged". Either:
- in Finder, Control-click the app → Open → Open (once; macOS 15 and later: System Settings →
  Privacy & Security → "Open Anyway" after the first refusal), or
- remove the flag: `xattr -dr com.apple.quarantine "/Applications/Fyx.app"`.

Only do this for a build you made yourself.

## Signing later

`script/bundle-mac` still has the identity path: set `FYX_MACOS_SIGNING_IDENTITY`,
`FYX_APPLE_NOTARIZATION_TEAM` and the `MACOS_CERTIFICATE*` / `APPLE_NOTARIZATION_*` variables,
plus `FYX_MACOS_PROVISIONING_PROFILE` if Fyx ever needs a profile. That signs with the hardened
runtime and notarizes, and is where `fyx.entitlements` starts to matter. `MACOS_SIGNING_KEY`
picks a local self-signed identity instead of ad-hoc. Debug symbols go to Sentry only with
`FYX_SENTRY_ORG` and `FYX_SENTRY_PROJECT` set, never to Zed's project.

## Untested

Written on Windows without a Mac (2026-11-20), checked with `bash -n` and shellcheck only:
- every step of `script/fyx-mac-check` and the changed parts of `script/bundle-mac`
  (ad-hoc branch, DMG name, `-d` remote-server path, Sentry guard);
- the debug `.app` path `target/<triple>/debug/bundle/osx/<name>.app`;
- that the app runs without the dropped entitlements once it is signed with the hardened
  runtime (ad-hoc builds do not use it, so they are unaffected).
