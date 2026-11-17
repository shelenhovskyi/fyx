# W4: Fyx app icon and logo

All four steps are done. The user picked **concept A, "tree F"** (2026-11-17). Step 4 generated the
full set with `script/fyx-icons`; W1 does the remaining wiring (see "For W1" below).

## 1. Icon assets Zed ships (inventory, `integration` at `1ec364bfc0`)

### App icon, per release channel

The suffix is `""` for Stable and `-preview`, `-nightly` or `-dev` for the other channels.
`crates/zed/build.rs:222-229` and `crates/windows_resources/src/windows_resources.rs:47-51` map
`RELEASE_CHANNEL` to the suffix; an unknown channel gets `-dev`.

| File | Format and sizes | Used by |
|---|---|---|
| `crates/zed/resources/app-icon{,-preview,-nightly,-dev}.png` | PNG, 512×512, RGBA | see below |
| `crates/zed/resources/app-icon{,-preview,-nightly,-dev}@2x.png` | PNG, 1024×1024, RGBA | see below |
| `crates/zed/resources/windows/app-icon{,-preview,-nightly,-dev}.ico` | ICO with frames 256, 128, 64, 32, 16 | see below |

The PNGs are used by:
- **macOS bundle:** `crates/zed/Cargo.toml:289/298/307/316`, `[package.metadata.bundle-<channel>] icon = [@2x, 1x]`. cargo-bundle builds the app `.icns` from these two PNGs, so **the repo has no app `.icns` or `.iconset`**.
- **Linux tarball:** `script/bundle-linux:215-217` copies the 1x and @2x files to `share/icons/hicolor/512x512/apps/zed.png` and `1024x1024/apps/zed.png`. `zed.desktop.in:10` has `Icon=$APP_ICON`, which `bundle-linux:222` sets to `zed`.
- **Flatpak:** `script/flatpak/bundle-flatpak:19-37` sets `ICON_FILE`, and `flatpak/manifest-template.json:40` installs the 512 px file under `hicolor/512x512`.
- **X11 window icon:** `crates/zed/build.rs:241-255` resizes the 1x PNG to 256 px at build time.
- **FreeBSD:** `script/bundle-freebsd:120-122` has the same copies, commented out.

The `.ico` files are used by:
- **The exe icon** of `zed`, `cli` and `auto_update_helper`: `crates/windows_resources/src/windows_resources.rs:41-84`, through `ICON_DIR` = `crates/zed/resources/windows`. W1 already renamed the product names there to Fyx; the icons are still Zed's.
- **The Inno Setup installer icon:** `zed.iss:22` `SetupIconFile`, with `script/bundle-windows.ps1:272-314` choosing the name per channel.
- `crates/zed/build.rs:233`, `icon_path()` on Windows.

### Other icon files

| File | Format and sizes | Used by | Notes |
|---|---|---|---|
| `crates/zed/resources/Document.icns` | ICNS with one `ic10` entry: a 1024 px PNG | macOS document type icon: `script/bundle-mac:133-136`, `info/DocumentTypes.plist` (`CFBundleTypeIconFile` "Document") | one file for every channel. ImageMagick here has no ICNS coder; I read the entries with a small parser. |
| `crates/auto_update_helper/app-icon.ico` | ICO 16, 24, 32, 48, 64, 96, 128, 256, 512 | nothing found: `auto_update_helper` gets its icon from `windows_resources` | apparently orphaned; replace it or delete it with the set |
| `crates/explorer_command_injector/AppxManifest{,-Preview,-Nightly}.xml:24-53` | references `resources\logo_150x150.png` and `resources\logo_70x70.png` | the Windows 11 Explorer context-menu package | **the PNGs are not in the repo**, and nothing in `script/` generates them. Left for W1 (see "For W1"). |
| Inno wizard bitmaps `inno-*.bmp`, `inno-small-*.bmp` | — | `zed.iss:20-21` | commented out, not shipped |
| `docs/theme/favicon.png` | PNG 64×64 | the mdBook docs site | Zed mark |
| `crates/gpui/examples/image/app-icon.png` | PNG 512 | a GPUI example | not product branding; leave it |

### In-app logos and branded icons (W1's call)

| File | What it is | Used by |
|---|---|---|
| `assets/images/zed_logo.svg` (96×96) | Zed wordmark mark | `VectorName::ZedLogo` (`crates/ui/src/components/image.rs`), shown on the welcome page (`crates/workspace/src/welcome.rs`) and in onboarding (`crates/onboarding/src/onboarding.rs`) |
| `assets/images/zed_x_copilot.svg` | Zed × Copilot lockup | Copilot sign-in (`crates/copilot_ui/src/sign_in.rs`) |
| `assets/icons/ai_zed.svg`, `zed_agent.svg`, `zed_agent_two.svg`, `zed_assistant.svg`, `zed_predict*.svg`, `zed_src_custom.svg`, `zed_src_extension.svg` | feature icons drawn from the Zed mark | the agent panel, edit prediction, AI settings and others (`IconName::{AiZed, ZedAgent, ZedAssistant, ZedPredict*, ZedSrc*}`). They are in-product glyphs rather than app icons; whether Fyx redraws them is a W1 decision. |

## 2. Concepts (`assets/fyx/icon-concepts/`)

All three use a rounded tile on Apple's 1024 grid: an 824 px tile with a 184 px corner radius and
a 100 px transparent margin. Each is one SVG with flat shapes and at most two gradients, so it
renders the same in librsvg, ImageMagick and browsers. Zed's mark is a "Z" made of nested
outline squares on black, blue, starfield or grey tiles. None of the three uses a letter in
outline style, nested squares, or Zed's tile colours.

| Concept | Idea | Colours | At 16 px |
|---|---|---|---|
| **A, tree F** (`a.svg`) | An "F" drawn as a flat UI tree: a stem with two branches that end in separate nodes. It stands for the extension tree: flat nodes, each naming its parent. | white on an orange to coral gradient | clearly an "F"; the nodes merge into the bars, which is fine |
| **B, fix mark** (`b.svg`) | A check whose long stroke crosses a white stroke, reading as both the "x" of Fyx and an approved change: agent review. | mint to cyan check and a white stroke on deep navy | busiest of the three; the cross is visible at 32 px, at 16 px it is a check with a speck. The navy tile almost disappears on dark backgrounds, so it would need a lighter tile edge. |
| **C, slot** (`c.svg`) | Three fixed white tiles and one yellow tile plugged in at an angle: panels that come from extensions. | white and yellow on a violet gradient | reads as a 2×2 grid with one odd tile; the angle is gone at 16 px, but the yellow keeps it distinct |

The preview is `assets/fyx/icon-concepts/preview.png`. It shows each concept at 512, 64, 32 and
16 px on light (`#F4F4F5`) and dark (`#18181B`) backgrounds, plus 16 and 32 px enlarged 4× with
nearest neighbour. Sizes of 64 px and below are drawn from the tile without the macOS margin,
which is what the Windows `.ico` frames and small Linux icons should use. To rebuild it:
`python assets/fyx/icon-concepts/build_preview.py assets/fyx/icon-concepts <scratch dir>`.

My ranking for legibility and distinctness: A, then C, then B. B carries the review idea best,
but it is the weakest at 16 px.

## 4. The full set (done)

The sources are in `assets/fyx/icon/`:
- `fyx-icon.svg`: the app icon in Stable colours;
- `fyx-document.svg`: a page with a folded corner carrying the tile;
- `fyx-logo.svg`: the one-colour mark.

`script/fyx-icons` (Python plus ImageMagick 7 with librsvg) writes everything below. Its output is
deterministic: a second run gives byte-identical files, checked with `sha1sum`.

| Output | Sizes | Notes |
|---|---|---|
| `crates/zed/resources/app-icon{,-preview,-nightly,-dev}.png` | 512, 8-bit RGBA | Apple's grid with a 100/1024 margin, like Zed's |
| `crates/zed/resources/app-icon{,-preview,-nightly,-dev}@2x.png` | 1024 | the same |
| `crates/zed/resources/windows/app-icon{,-preview,-nightly,-dev}.ico` | 256, 128, 64, 48, 32, 24, 16 | the tile without the margin, as in Zed's `.ico` files; 48 and 24 are new |
| `crates/zed/resources/Document.icns` | one `ic10` entry, a 1024 PNG | written directly by the script, since ImageMagick here has no ICNS writer |
| `docs/theme/favicon.png` | 64 | the tile |
| `assets/images/fyx_logo.svg` | 96×96 view box | a copy of `fyx-logo.svg`. One colour, because GPUI draws `Vector`s as a mask tinted with the element's colour (`crates/ui/src/components/image.rs:85-96`). |

The channels differ by tile hue only, keeping the white mark, so each stays readable at 16 px:

| Channel | Tile gradient |
|---|---|
| Stable | orange `#FF8A1F` to coral `#F0452E` |
| Preview | blue `#3D8BFF` to `#2A4FE0` |
| Nightly | violet `#8B5CF6` to `#4C1D95` |
| Dev | grey `#9AA3AF` to `#5B6472` |

A badge was rejected because it is unreadable at 16 px. The 16 and 32 px frames pulled from the
`.ico` files still read as the "F" on light and dark backgrounds.

Linux needs no separate files: `script/bundle-linux` and Flatpak copy the PNGs above.
`crates/auto_update_helper/app-icon.ico` was deleted: nothing referenced it, since the helper's
icon comes from `windows_resources`.

## For W1 (wiring)

- **In-app logo.** Add `VectorName::FyxLogo` → `images/fyx_logo.svg` in
  `crates/ui/src/components/image.rs`. Switch the welcome page (`crates/workspace/src/welcome.rs`)
  and onboarding (`crates/onboarding/src/onboarding.rs`) to it. Then decide whether
  `assets/images/zed_logo.svg` stays, since the Copilot lockup still uses Zed's mark.
- **Explorer package tiles.** `crates/explorer_command_injector/AppxManifest{,-Preview,-Nightly}.xml`
  reference `resources\logo_150x150.png` and `resources\logo_70x70.png`, which are in neither the
  repo nor any script. `script/fyx-icons` can produce them, by resizing the tile to 150 and 70,
  once W1 says where the packaging step expects them.
- **Linux icon name.** `script/bundle-linux:215-222` still installs the icon as
  `share/icons/hicolor/*/apps/zed.png` with `APP_ICON="zed"`; that is a name, not artwork, so it
  is W1's.
- `docs/src/development/windows.md:201` mentions `resources/windows/app-icon.ico`. The path is
  unchanged, so nothing to do.
