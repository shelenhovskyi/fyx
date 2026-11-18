# License audit 1: GPL fork with Apache-2.0 SDK crates

**Scope.** `integration` at `f3718ca` against upstream `v1.28.1` (`FYX_UPSTREAM`, `e8bc16acee`).
The diff covers 185 files, +34660/−46.

**Method.**
- Ran `script/check-licenses` and `script/fyx-check-licenses-index`.
- Ran `cargo tree`/`cargo metadata` for every Apache crate, the guest ones with
  `--target wasm32-wasip2`.
- Diffed `Cargo.lock` and both fixture lockfiles against `v1.28.1`.
- Ran `script/generate-licenses` with cargo-about 0.8.2 (result under "cargo-about" below).
- Read both Node lockfiles.
- Scanned 4-line windows of the Apache Rust/WIT/TS sources against every upstream source file.
- Scanned 3-line windows of the sidecar shim against VS Code's MIT sources (`extHostTypes.ts`,
  `extHostTypes/{position,range,selection}.ts`, `base/common/event.ts`).
- Grepped for "Zed" in metadata, packaging and UI strings.

Not legal advice. The items marked "owner decision" need the repository owner, and ideally a lawyer
before any commercial use.

## Verdict

The Cargo side is clean:
- Every Fyx crate declares its license and has the correct symlink.
- The Apache crates (`ui_ext`, `ui_ext_sdk`, the three demos, two fixtures) depend only on
  permissive crates.
- No third-party crate was added to `Cargo.lock` since `v1.28.1`.
- No Zed code was copied into the Apache Rust crates.

Five things block a public release:
1. **F1:** `tools/vsix-convert` is declared Apache-2.0, but its own comments say it is a port of
   GPL Zed code.
2. **F2:** the bundles still ship under Zed's name, bundle IDs and logo.
3. **F3:** the macOS DMG makes users accept Zed Industries' Terms of Service.
4. **F4:** the repository README presents the project as Zed.
5. **F5:** the Apache license text that Fyx's own crates point to names only Zed Industries as
   copyright holder.

## Findings

| # | Severity | Area | Owner |
|---|---|---|---|
| F1 | High | GPL-derived code under Apache-2.0 in `tools/vsix-convert` | W8 + owner decision |
| F2 | High | Zed trademark in bundle metadata, icons, installer, welcome screen | W1 / W0 |
| F3 | High | DMG license agreement is Zed's Terms of Service | W0 |
| F4 | Medium | README is Zed's; no GPL §5(a) "modified version" notice | W0 / owner |
| F5 | Medium | `LICENSE-APACHE` copyright line names only Zed Industries | W0 / owner decision |
| F6 | Low | Apache demo test `include_str!`s a GPL source file | W4 |
| F7 | Low | "Zed" in the Apache contract docs and tool metadata | W2 / W4 / W8 |
| F8 | Low | `crates/zed` package metadata still Zed's | W1 |
| F9 | Info | Node packages had no license file in their directory | fixed here (sidecar); W8 (converter) |
| F10 | Info | `ui_ext_sdk` reads WIT from outside its crate | W4 / W0 before publishing `fyx-sdk` |

### F1 (High): `tools/vsix-convert` contains GPL-derived code but is declared Apache-2.0

The following files say in their own words that they derive from GPL-3.0-or-later crates:
- `tools/vsix-convert/src/theme.ts:1-3`: "Port of `crates/theme_importer/src/vscode/converter.rs`
  … The color tables below mirror `convert_theme_colors` and `convert_status_colors` one to one."
- `tools/vsix-convert/src/syntax.ts:1`: "Port of `crates/theme_importer/src/vscode/syntax.rs`".
- `tools/vsix-convert/src/snippets.ts:272`: "A port of Zed's `Snippet::parse` acceptance rules"
  (`crates/snippet`).
- `tools/vsix-convert/README.md:23,57` say the same.

`theme_importer` and `snippet` are both `GPL-3.0-or-later`. A port or close translation is a
derivative work, so the package can't be licensed Apache-2.0: `package.json` and the root entry of
`package-lock.json` say `"license": "Apache-2.0"`. The colour-key tables are arguably facts. The
structure, fallback order and selection logic, ported "one to one", are not.

**Fix (pick one, owner decision):**
- **(a) Recommended.** Relicense the package as `GPL-3.0-or-later`: `package.json`, the
  lockfile's root entry, the README, and a `LICENSE-GPL -> ../../LICENSE-GPL` symlink. It is a
  standalone CLI that nobody links against, and Fyx itself is GPL, so nothing is lost. The themes,
  snippets and keymaps it converts are the user's data. GPL doesn't reach them.
- **(b)** Keep Apache-2.0 and rewrite `theme.ts`, `syntax.ts` and the snippet checker clean-room:
  from the VS Code colour reference and Zed's published theme JSON schema, by someone who hasn't
  read the Rust.

This audit didn't change the license, because which option to take is the owner's call.
`sidecars/vscode-host` doesn't have this problem (see "Checked and clean").

### F2 (High): Zed's name, IDs and logo still ship in every bundle except the Windows exe metadata

W1 rebranded the runtime: `APP_NAME`, `release_channel`, the menus, User-Agent, URL scheme and
the exe version info. W1's progress notes say macOS and Linux packaging were left out of scope.
The following are still Zed's:
- **Bundle metadata used by `cargo bundle`:** `crates/zed/Cargo.toml:288-318`,
  `[package.metadata.bundle-*]`. It has `name = "Zed"`, `"Zed Preview"`, `"Zed Nightly"` and
  `"Zed Dev"`, with `identifier = "dev.zed.Zed*"`. A macOS build would claim Zed's bundle ID, which
  breaks the "coexists with Zed" goal as well as the trademark rule.
- **App icons:** these are Zed's logo, unchanged since upstream:
  - `crates/zed/resources/app-icon*.png`
  - `crates/zed/resources/Document.icns`
  - `crates/zed/resources/windows/app-icon*.ico`

  Zed's licenses grant no rights to its logos (`docs/research/…#name-trademark-and-distribution`).
- **Linux:**
  - `script/bundle-linux:225-234` exports `APP_NAME="Zed"`, which feeds `zed.desktop.in` and the
    flatpak metainfo.
  - `crates/zed/resources/flatpak/zed.metainfo.xml.in:9,37-41` has `developer id="dev.zed"` and
    zed.dev and zed-industries URLs.
- **Windows installer:**
  - `script/bundle-windows.ps1:272-314` sets `$appName = "Zed"` and its siblings.
  - `crates/zed/resources/windows/zed.iss:5-6` has `AppPublisher=Zed Industries` and
    `AppPublisherURL=https://www.zed.dev/`.

  W1 already lists these as follow-ups.
- **In-app UI:**
  - The welcome page shows "Welcome to Zed" / "Welcome back to Zed" and the Zed logo
    (`crates/workspace/src/welcome.rs:454-456,483`).
  - The onboarding page shows the Zed logo (`crates/onboarding/src/onboarding.rs:350`).
  - About 380 non-test string literals in `crates/*/src` still contain "Zed". Most are settings
    docs and messages. Triage by visibility: titles, dialogs and notifications first.

**Fix:**
- Give the bundles `Fyx*` names and `dev.fyx.Fyx*` IDs. Use the same values `release_channel`
  already returns, so there is one source.
- Replace the icons with Fyx's own.
- Change the publisher to "Fyx contributors" and the URLs to the Fyx repository.
- Add `fyx:` hooks for the welcome and onboarding title and logo.

None of the files are owned by this audit: W1 owns `crates/zed`, and the rest is W0's to assign.
This blocks any distributed binary on any OS, including the Windows installer.

### F3 (High): the macOS DMG makes users accept Zed Industries' Terms of Service

`script/bundle-mac:291-293` runs `dmg-license script/terms/terms.json`. That attaches
`script/terms/terms.rtf`, generated from `legal/terms.md` by `script/generate-terms-rtf`, as the
DMG's click-through agreement. It is Zed Industries' Terms of Service. A Fyx user would be asked
to accept a contract with a company that isn't distributing the software. That is misleading,
and it suggests an affiliation that doesn't exist. Similarly, `legal/privacy-policy.md` and
`legal/subprocessors.md` describe Zed's services, which Fyx turns off by default.

**Fix:**
- Drop the `dmg-license` step from Fyx bundles: GPL software needs no click-through.
- Alternatively, replace `terms.rtf` with the GPL-3.0 notice.
- Keep `legal/` only as upstream reference, or remove it from Fyx's tree, and say which in the
  README.

Owner: W0, since `script/bundle-mac` isn't owned by any stream.

### F4 (Medium): the README is Zed's, and the GPL "modified version" notice is missing

`README.md` is upstream's unchanged:
- It opens with "# Zed" and "Welcome to Zed…".
- It has Zed's CI badges and zed.dev download links.
- It ends with "Zed is developed by Zed Industries, Inc." plus a sponsorship link.

For a public fork that suggests official affiliation, which Zed's brand guidelines forbid.
GPL-3.0 §5(a) also requires the modified work to "carry prominent notices stating that you
modified it, and giving a relevant date". The `fyx:` markers and git history help, but nothing
prominent says so.

**Fix:** put a Fyx README at the root. It should:
- State that Fyx is a modified version of Zed (upstream tag `v1.28.1`, © Zed Industries, Inc.),
  modified since 2026-11.
- State that Fyx isn't affiliated with or endorsed by Zed Industries.
- Give the licensing split: GPL-3.0-or-later app; Apache-2.0 for `crates/ui_ext`,
  `crates/ui_ext_sdk`, `extensions/fyx_*` and `sidecars/vscode-host`.

Keep upstream's licensing section text. Owner: W0, or the repository owner.

### F5 (Medium): the Apache license text names only Zed Industries as copyright holder

Every Apache crate's `LICENSE-APACHE` is a symlink to the root file, as `script/check-licenses`
requires. The root file opens with `Copyright 2022 - 2025 Zed Industries, Inc.`. That is correct
for upstream's Apache crates (gpui, extension_api…). For `ui_ext`, `ui_ext_sdk`, the demos and the
sidecar it says Zed Industries owns code Zed didn't write. The license file that crates.io and npm
package with `fyx-sdk` would carry the wrong notice.

**Fix (owner decision on the holder's name):** add a line to the root `LICENSE-APACHE`, such as
`Copyright 2026 Fyx contributors (Fyx-owned crates)`. The Windows exe already uses the name "Fyx
contributors". This keeps the symlink rule. Upstream's line stays as it is. Owner: W0, since it
changes an upstream file with a `fyx:`-style note in the commit.

### F6 (Low): an Apache demo test embeds a GPL source file

`extensions/fyx_demo_gallery/src/fyx_demo_gallery.rs:1616-1619`, the
`icons_are_zed_icon_names_except_the_unknown_one` test, runs
`include_str!("../../../crates/icons/src/icons.rs")`. `icons` is GPL-3.0-or-later. Only the test
binary embeds it, and that binary isn't distributed, so this isn't a release blocker. But:
- the Apache crate can't build its tests outside this checkout (as a template, or once published);
- it ties an Apache crate's source to a GPL file.

The 12 icon names in `KNOWN_ICONS` are facts and fine.

**Fix:** move the cross-check to `ui_ext_host`'s tests, which are GPL and depend on `ui` anyway.
Owner: W4.

### F7 (Low): "Zed" in Apache contract docs and tool metadata

These are nominative and factual uses, which are acceptable. The WIT, though, is the published
contract that third parties read:
- `crates/ui_ext/wit/fyx-ui.wit:74`: "rendered by Zed's own diff/editor machinery".
- `crates/ui_ext_sdk/src/ui_ext_sdk.rs:81`: "as in Zed's `zed:api-version`".
- `extensions/fyx_demo_gallery/src/fyx_demo_gallery.rs:60`.
- The `tools/vsix-convert/package.json` description: "into Zed/Fyx formats".

**Fix:** say "the host" or "Fyx" in the WIT and SDK docs, and keep "Zed" only where it names
upstream code or formats. Owners: W2 (WIT, integrator-only file), W4, W8.

### F8 (Low): `crates/zed` package metadata

`crates/zed/Cargo.toml:2,8` has `description = "The fast, collaborative code editor."` and
`authors = ["Zed Team <hi@zed.dev>"]`. The package is `publish = false`, and its name is
deliberately kept as `zed` (W1). `cargo-bundle` and some Linux packagers read `authors` and
`description`, so these strings can reach installers. **Fix:** with F2, set
`authors = ["Fyx contributors"]`, or keep Zed's and add Fyx's (attribution is fine; only the
"maintainer" field matters). Owner: W1.

### F9 (Info, partly fixed): Node packages without a license file

Both packages are `private: true` and Apache-2.0 in `package.json`, but their directories had no
license file. That differs from the Cargo convention, and `npm pack` would ship none. **Fixed here
for `sidecars/vscode-host`:** added a `LICENSE-APACHE -> ../../LICENSE-APACHE` symlink. **Not
done for `tools/vsix-convert`:** under F1 its license is about to change, so W8 adds the matching
symlink with that fix.

### F10 (Info): `ui_ext_sdk` reads WIT from outside its crate

`crates/ui_ext_sdk/src/ui_ext_sdk.rs:61` has `path: "../ui_ext/wit"`. That is not a license
problem, since both crates are Apache. But `cargo package` doesn't include files outside the
crate, so publishing `fyx-sdk` would fail, or would need the WIT vendored in. Vendoring would then
need the license and copyright handled. Track it for the publication step. Owner: W4/W0.

## Checked and clean

- **Fyx crate licenses.** All nine Fyx Cargo packages have a `license` field, and a symlink of the
  same name that resolves:
  - `ui_ext`, `ui_ext_sdk`: Apache.
  - `ui_ext_host`, `agent_review`: GPL.
  - `fyx_demo_counter`, `fyx_demo_todo`, `fyx_demo_gallery`: Apache.
  - The fixtures `fyx_ui_test_probe` and `fyx_ui_test_status_item`: Apache.

  `script/check-licenses` and `script/fyx-check-licenses-index` both pass. No AGPL anywhere.
- **New third-party crates.** None since `v1.28.1`: the `Cargo.lock` diff adds only the seven
  first-party workspace packages. `ui_ext_sdk`'s `wit-bindgen = "0.41"` was already in the lock,
  via `zed_extension_api`. The two fixture lockfiles add only `thiserror`/`thiserror-impl`
  2.0.20 and 2.0.21, which are MIT OR Apache-2.0.
- **`ui_ext` dependency tree.** `cargo tree -p ui_ext` has 11 packages, all MIT OR Apache-2.0,
  except `unicode-ident` ((MIT OR Apache-2.0) AND Unicode-3.0, proc-macro only). The dev-only
  dependencies `proptest` and `serde_json` are MIT OR Apache-2.0.
- **Guest trees on `wasm32-wasip2`.** For `ui_ext_sdk` and the three demos, `-e normal,build`
  gives 89–90 packages:
  - MIT OR Apache-2.0 / Apache-2.0 OR MIT, including the `MIT/Apache-2.0` spelling: 50.
  - Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT (the wasm-tools family): 10.
  - Apache-2.0: 2–3 (the Fyx crates).
  - MIT only (`slab`, `synstructure`, `simd-adler32`, `zmij`): 4.
  - Unicode-3.0 (ICU stack, `yoke`, `zerovec`…): 18.
  - Zlib (`foldhash`): 1.
  - Unlicense OR MIT (`memchr`), 0BSD OR MIT OR Apache-2.0 (`adler2`), MIT OR Zlib OR Apache-2.0
    (`miniz_oxide`), (MIT OR Apache-2.0) AND Unicode-3.0 (`unicode-ident`): 1 each.

  No GPL, AGPL, LGPL, MPL or unknown licenses. The Unicode-3.0, Zlib and MIT-only crates come in
  through `wit-bindgen-rust-macro`, via `wasm-metadata` → `url` → `idna`. They are proc-macro
  host dependencies and aren't linked into the guest. The guest links `ui_ext`, `serde`,
  `thiserror`, `wit-bindgen`, `wit-bindgen-rt`, `bitflags`, `futures-*`, `once_cell`, `memchr`,
  `pin-project-lite` and `slab`, all permissive.
- **cargo-about policy.** `script/licenses/zed-licenses.toml` accepts every license expression
  above: MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, Unicode-3.0 and Zlib. Unlicense isn't
  in the list, but `memchr` is offered under MIT. `private = { ignore = true }` skips all Fyx
  crates, since the workspace sets `publish = false`. With no new third-party crates, the policy
  needs no change. For the full run, see "cargo-about" below.
- **No Zed code copied into the Apache Rust/WIT crates.** A scan of 4-line windows (≥120
  characters) found zero matches between `ui_ext`, `ui_ext_sdk`, the demos and fixtures, and any
  upstream `.rs`/`.wit`/`.ts`/`.js` file. The `zed:api-version`-style section in `ui_ext_sdk` was
  written independently (`zed_extension_api` is Apache-2.0 anyway). `bindings` are generated from
  Fyx's own `wit/fyx-ui.wit` (`package fyx:ui@0.1.0`), which imports nothing from `zed:extension`.
- **Node lockfiles.** Both packages have only devDependencies, none of them shipped:
  - `@types/node` 24.19.2: MIT.
  - `undici-types` 7.24.6: MIT.
  - `typescript` 5.9.3: Apache-2.0.
  - `@types/vscode` 1.146.0 (sidecar only): MIT.

  All resolve from registry.npmjs.org. Neither package has runtime dependencies.
- **No VS Code extension code committed.**
  - `git ls-files` has no `.vsix`, `vsix/`, `.cache/` or `node_modules` entries.
  - Both `.gitignore` files exclude them.
  - `extensions.json` only lists IDs and versions, which `npm run fetch` downloads with
    SHA-256 and license checks.
  - `test/fixtures/kitchen-sink` is an original Apache-2.0 fixture ("Test fixture: a plain
    CommonJS VS Code extension…").
  - `docs/vscode-coverage.md` lists API names, not extension source.
- **Sidecar shim vs VS Code (MIT).** I compared it against the files listed under "Method". There
  is one 3-line match: the trivial `Color` constructor field assignments, which are dictated by
  the API. `Position.translate`, `with` and the rest are written differently from upstream's.
  `src/api-catalog.json` holds only runtime names extracted from `@types/vscode` (MIT): facts, no
  doc text.
- **Upstream GPL crates in Fyx GPL crates.** `ui_ext_host` and `agent_review` are GPL and use GPL
  crates, as intended. The upstream hooks (`agent_ui`, `extension_host`, `zed`) only add Fyx
  workspace dependencies.

## cargo-about

`script/generate-licenses` (cargo-about 0.8.2, `--fail`, with stderr treated as failure) passed
on Linux at `f3718ca`. It wrote a 30361-line `licenses.md` with 320 license sections:
- MIT: 179
- Apache-2.0: 99
- BSD-3-Clause: 13
- ISC: 9
- Zlib: 5
- MPL-2.0: 5
- Unicode-3.0: 2
- CC0-1.0: 2
- BSD-2-Clause: 2
- bzip2-1.0.6, MIT-0, CDLA-Permissive-2.0, BSL-1.0: 1 each

There were no errors, no unaccepted licenses and no GPL family licenses. As expected, no Fyx crate
appears in the output (`private = { ignore = true }`). The output was not committed: it is a
bundle-time artifact.

The config sets no `targets`, so the run covers crates for every platform: the Windows-only
`windows_*` crates are listed, for example.

## Fixes made in this audit

- `sidecars/vscode-host/LICENSE-APACHE`: a new symlink to `../../LICENSE-APACHE` (F9).
- This report.

No license fields were missing in Fyx-owned files, so none were added. F1 and F5 change licenses
or notices and need an owner decision. F2–F4 touch upstream or unowned files. All of them are
left to the owners above.
