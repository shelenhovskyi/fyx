# Web views as extensions (pluggable providers, CEF first)

Status: research and plan, 2026-11-20. No product code until the owner approves. Targets
`fyx:ui@0.5.0` (0.4 is frozen). Plan item: [platform plan 7.17](../platform/plan.md#717-web-views-as-extensions).
Builds on [servo-webview.md](servo-webview.md), which this document generalises.

## Recommendation

1. **The base app ships a web view *slot*, not a web engine.** A new Fyx crate `webview_host`
   owns a backend-neutral surface element, a versioned helper protocol, a helper supervisor
   (download, verify, launch, sandbox, restart) and the agent tools. It links no browser code and
   adds no heavy dependency. With no provider installed, a web view renders a card "This needs a
   web view provider" with an **Install** button that opens the Extensions page filtered to
   providers.
2. **Engines arrive as provider extensions.** `fyx_webview_cef` is an ordinary registry entry
   whose manifest declares a `[fyx_webview_provider]` table and one signed native helper per
   platform. The helper (Chromium Embedded Framework plus a small Rust host built on the `cef`
   crate) is **downloaded on first use**, never at Fyx install and never inside the `.fyxext`,
   after one dialog that states the size (about 120 MB download, about 300 MB on disk).
3. **The helper runs out of process and draws into a GPUI texture.** CEF off-screen rendering
   with shared textures (`OnAcceleratedPaint`) gives a GPU surface per frame; the helper copies it
   into its own pool of shareable surfaces and tells Fyx which one is ready. macOS uses IOSurface
   and GPUI's existing `paint_surface`, so it needs no GPUI change. Windows needs one `fyx:` hook
   (D3D11 shared handle in `SurfaceSource` and a real `draw_surfaces`). Linux and every fallback
   use CPU frames in shared memory. No native child windows: no z-order, clipping or popover bugs.
4. **Native code is a separate trust class.** WASM extensions cannot carry native code. Only
   provider extensions may, they need a new capability `native:helper`, a registry rule that only
   the registry owner may publish one (for 1.0), a separate consent dialog, and an OS sandbox
   around the helper.
5. **Extensions and agents see one API.** Extensions get a WIT 0.5 `webview` node behind
   `ui:web` with a manifest origin list. Agents get Fyx tools (`web_open`, `web_snapshot`,
   `web_click`, `web_type`, `web_screenshot`, gated `web_eval`) that the CEF provider maps to
   in-process DevTools Protocol calls, with no open debugging port. Servo and wry can be later
   providers behind the same protocol.

Ship order: macOS end to end with a locally built helper (no registry), then the registry and
download path, then WIT 0.5, then agent tools, then Windows, Linux on the CPU path last.

## Why not the other shapes

| Shape | Problem |
| --- | --- |
| CEF inside the WASM sandbox | Impossible: CEF is a native multi-process program with a GPU process |
| CEF bundled in Fyx | Owner decision: optional only. Also +300 MB for every user and Chromium's release cadence on Fyx's release train |
| Extension downloads and spawns CEF through `process:spawn` | `process:spawn` resolves programs on `PATH`, outside the worktree, by design (spec decision 24). A WASM extension fetching and running arbitrary native code would make every T2 extension a native-code extension |
| wry / system web view | Native child window above GPUI: z-order, clipping, scroll lag, no Wayland (see servo-webview.md). Kept as an optional overlay provider |
| Servo only | Partial web compatibility, no CDP for agents, 0.x API churn. Kept as a later provider |

## Layers

```
consumer extension (WASM)        agent (ACP / MCP)
  webview node, ui:web             web_* tools
          \                          /
           webview_host (in Fyx, always present)
           element · supervisor · downloader · policy · protocol v1
                         | named pipe / Unix socket + shared surfaces
           provider helper (native, downloaded, OS-sandboxed)
           fyx-webview-cef  |  later: fyx-webview-servo, wry overlay
```

- **Consumer extensions** never pick an engine. They declare `ui:web` and origins.
- **The user picks the provider** (`fyx.webview.provider`, default: the only installed one).
- **Provider extensions** carry no WASM component, or a tiny one for a settings page. The
  `[fyx_ui]` `component` key becomes optional for an extension whose only content is a provider.

## Manifests

### Consumer (`extension.toml`)

```toml
[fyx_ui]
api_version = "0.5.0"
component = "fyx_ui.wasm"
capabilities = ["ui:panel", "ui:web"]

[fyx_ui.web]
# Top-level navigation and subresource allow-list. `asset:` is the extension's own assets/ dir.
origins = ["asset:", "https://docs.rs", "https://*.docs.rs"]
# Optional: a persistent cookie/storage partition. Default: ephemeral, per extension.
persistent_storage = false
```

`origins` follows the `network_domains` rules of the registry (no IP literals, no `localhost`, no
wildcard over a public suffix) and is shown at install. `ui:web` is **T1**: rendering a page
from a fixed list is a read of the network, like `network:fetch` but without the extension
seeing bytes; it is approved once at install. Origins added by an update hold the update (as
domains do today). `postMessage` from the page reaches only the owning extension.

### Provider (`extension.toml`)

```toml
id = "fyx-webview-cef"
name = "Chromium web view (CEF)"
version = "1.142.0"            # provider major . Chromium major . patch

[fyx_ui]
api_version = "0.5.0"
capabilities = ["native:helper"]

[fyx_webview_provider]
engine = "cef"
engine_version = "142.0.10+chromium-142.0.7444.59"
protocol = 1                    # helper protocol major this provider speaks
features = ["shared-texture", "cdp", "ime", "popups"]
```

The helper artifacts are **not** listed in the manifest the extension author controls: they come
from the signed registry release (below), so the bytes Fyx runs are always the bytes the
registry signed.

## Registry: native artifacts

`index.json` releases gain one optional field, covered by the release signature like every other
field (canonical bytes, `fyx-registry/release/v1` domain):

```json
"natives": [
  {"platform": "macos-aarch64", "url": "https://github.com/<org>/fyx-webview-cef/releases/download/v1.142.0/fyx-webview-cef-macos-aarch64.tar.zst",
   "sha256": "<64 hex>", "size": 118000000, "unpacked_size": 310000000,
   "entry": "Fyx Web View.app/Contents/MacOS/Fyx Web View"},
  {"platform": "windows-x86_64", "...": "..."}
]
```

Rules:

| Rule | Where |
| --- | --- |
| `natives` requires the `native:helper` capability and vice versa | `fyx-registry verify`, client |
| Size up to 512 MiB per artifact (the 50 MiB cap stays for `.fyxext`) | format |
| `platform` from a closed list (`macos-aarch64`, `macos-x86_64`, `windows-x86_64`, `linux-x86_64`) | format |
| Any release with `natives` needs the registry owner's approval, every version, like T2 | `policy` check |
| For 1.0, `natives` only for ids on an owner-signed allow-list in `policy/native-providers.toml` | `policy` check + snapshot |
| Artifacts are built by the provider repo's own CI from a pinned CEF binary distribution (sha256 pinned in the repo); `publish.yml` checks a GitHub artifact attestation for the commit before signing | registry CI |
| `urgent` and the block list apply as for packages; Chromium security releases ship as `urgent` | existing |

The client adds `native:helper` as its own consent class: the dialog says "runs a native program
(Chromium) in an OS sandbox", the engine, the download and disk size, and the platform. It is
never silent, and a provider update that changes `engine_version` installs without a prompt
only if nothing else in the consent set changed.

## Download, verification, install

1. **When:** the first time a `webview` node mounts (or the user presses **Download** on the
   provider page). Not at extension install: a user may install the provider and never open a
   page.
2. **Where:** `<fyx data>/webview/<provider id>/<version>/<platform>/`. Download into
   `…/.partial/` with resume (HTTP range), check `size` then `sha256` streaming, unpack the
   `.tar.zst` (`async-tar` + `zstd`, both in the workspace) into a sibling temp dir with the
   install store's path rules (no `..`, no absolute paths, no symlinks leaving the root, no
   device files), then atomically rename. Keep the previous version for rollback, delete older
   ones and unused providers on a disk-guard pass.
3. **Trust on every launch:** record the tree's hash manifest at unpack; before each launch check
   the entry file's hash (cheap) and, once a day, the whole tree. A mismatch deletes the
   version and asks to download again.
4. **macOS without an Apple Developer account:** CI ad-hoc signs the helper bundle and its four
   CEF helper apps (`… Helper`, `(GPU)`, `(Renderer)`, `(Plugin)`). Fyx downloads with its own
   HTTP client, so no quarantine attribute is set and Gatekeeper does not assess it; Apple
   Silicon still requires a signature, which ad-hoc satisfies. When there is a Developer ID,
   CI signs and notarizes instead (7.1 "later").
5. **Progress UI:** a progress row in the web view's placeholder and on the provider page;
   cancel keeps the partial file.

## Process and sandbox model

- **One helper process per provider per Fyx instance**, started lazily, many browsers inside it
  (CEF allows one `CefInitialize` per process). CEF then spawns its own renderer, GPU and utility
  subprocesses from the helper bundle. Idle shutdown after 5 minutes with no web views. Crash:
  every web view shows **Reload**; restart with backoff; three crashes in a minute disable the
  provider until the user re-enables it.
- **Launch:** by `webview_host`, never by an extension. Clean environment (the scrubbing rules
  of security-modes.md), cwd = the helper's install dir, arguments fixed by the host
  (`--fyx-protocol=1 --fyx-pipe=<name> --user-data-dir=<profile root> --use-adapter-luid=<Fyx's GPU>`).
- **IPC:** one channel per helper: a Unix socket (macOS, Linux) or a named pipe with an ACL for
  the current user and a random name (Windows), handed over at spawn. Length-prefixed
  `postcard` messages; a `hello` exchange negotiates `protocol` and features. Frames never go
  through the pipe, only surface ids and dirty rects.
- **Chromium's own sandbox stays on** for renderers and GPU: macOS needs the helper-app layout
  (above); Windows needs `cef_sandbox` linked into the helper exe (built with the same MSVC as
  CEF, checked in the spike); Linux needs user namespaces. `--no-sandbox` is never used: without
  namespaces the provider reports "unsupported".
- **Outer sandbox around the helper's browser process:** macOS: a Seatbelt profile from 7.5 that
  allows reading the install dir, read-write on the profile root, Mach lookups CEF needs, and
  outbound network; no home, no worktrees, no Keychain. Windows: a job object (kill on close,
  memory cap) first; a restricted token or AppContainer after the spike shows CEF's GPU process
  still starts. Linux: landlock for the file system.
- **Network policy is enforced in the helper** (`CefRequestHandler` /
  `CefResourceRequestHandler`): each browser carries the origin list its owner was granted;
  navigation or subresource outside it is cancelled and reported as `navigation-blocked`.
  `file://`, `chrome://` (except `chrome://credits`), `devtools://` and downloads are refused;
  an extension's `asset:` maps to a custom scheme served from its assets dir by Fyx through the
  pipe. Popups (`window.open`) are denied or opened in Fyx's agent browser tab, per the owner.
- **Storage:** a `CefRequestContext` per partition. Default partition: ephemeral per extension.
  Agent browsing uses its own partition, never one the user signed into.
- **File pickers, dialogs, permissions:** `<input type=file>` goes to Fyx, which shows its own
  dialog and passes only the chosen paths; `alert`/`confirm`/`prompt` become GPUI modals;
  camera, mic, geolocation, notifications, clipboard read: denied in 1.0.

## Helper protocol v1 (sketch)

Fyx → helper: `hello`, `create {id, partition, origins, size, scale, source}`, `resize`,
`navigate`, `reload`, `back`, `forward`, `input {mouse|wheel|key|ime-*}`, `focus`, `blur`,
`post-message {id, json}`, `devtools {id, method, params}` (agent tools only), `screenshot`,
`close`, `frame-ack {id, surface}`.

Helper → Fyx: `hello`, `surfaces {id, platform handles}` (once per pool change),
`frame {id, surface, dirty}`, `cursor`, `title`, `load {state, url}`, `navigation-blocked`,
`message {id, json}`, `ime {show, caret_rect}`, `dialog`, `file-dialog`, `devtools-event`,
`crashed {id, reason}`.

Frame flow: a pool of 3 surfaces per browser. On `OnAcceleratedPaint` the helper blits the CEF
texture (valid only during the callback) into the next free pool surface, sends `frame`, and
reuses a surface only after `frame-ack`. Popups (`<select>`, `PET_POPUP`) are composited into the
same pool surface by the helper.

## GPUI surface hooks per platform

| Platform | Frame path | GPUI change | Owner |
| --- | --- | --- | --- |
| macOS | IOSurface pool; mach send rights passed once over a mach port rendezvous set up at spawn (Unix sockets cannot carry mach ports); Fyx wraps each IOSurface in a `CVPixelBuffer` once and calls `Window::paint_surface` | none | `webview_host` |
| Windows | D3D11 textures created with `SHARED_NTHANDLE` + keyed mutex on the adapter Fyx uses; handles duplicated into Fyx once; Fyx opens them with `OpenSharedResource1` | `fyx:` hook: `SurfaceSource::D3D11` variant in `crates/gpui/src/elements/surface.rs`, real `draw_surfaces` in `crates/gpui_windows/src/directx_renderer.rs` (today a no-op) drawing a textured quad | W1 or W3 + W0 review |
| Linux | CPU frames first; dmabuf import later (needs wgpu external memory) | `fyx:` hook in `gpui_wgpu` for `PrimitiveBatch::Surfaces` (today ignored) only at the dmabuf step | later |
| All (fallback) | CEF `OnPaint` BGRA into a shared-memory ring (`shm_open` / `CreateFileMapping`) with dirty rects | a streaming texture updated in place instead of a new atlas sprite per frame: one small `fyx:` hook in GPUI's image path, or a dedicated element in `webview_host` if GPUI's public API allows | spike decides |

The element (`WebViewElement` in `webview_host`) implements GPUI's input handler for IME, maps
`FocusHandle` to `focus`/`blur`, converts mouse coordinates by element bounds and scale, sends
`resize` on layout change (debounced), and repaints on `frame` without re-rendering the extension
tree (depends on 7.7).

Keys: while a web view has focus, Fyx keybindings are suspended except a short escape list
(focus out, command palette, close tab), as in servo-webview.md.

## WIT 0.5 additions (sketch for W0)

```wit
// ui-types
variant webview-source { url(string), html(string), asset(string) }
record webview-props {
    source: webview-source,
    min-height: u32,
    // Shown while no provider is installed or while the engine downloads.
    placeholder: option<string>,
}
// new node kind
webview(webview-props)

// events delivered to the owning extension
webview-loaded(node-id, string)              // url
webview-message(node-id, string)             // JSON from window.fyx.postMessage
webview-navigation-blocked(node-id, string)
webview-unavailable(node-id, webview-unavailable-reason)  // no-provider, downloading, crashed, unsupported-platform

// host import, needs ui:web
interface webview {
    post-message: func(node: node-id, json: string) -> result<_, webview-error>;
    navigate: func(node: node-id, url: string) -> result<_, webview-error>;
    reload: func(node: node-id) -> result<_, webview-error>;
}
```

There is no `eval` for extensions: a page talks to its extension only through `postMessage`
(`window.fyx.postMessage` injected into the main frame, origin-checked). Extensions never learn
which provider runs the page.

## Agent tools

| Tool | CEF mapping (in-process `SendDevToolsMessage`, no port) | Consent |
| --- | --- | --- |
| `web_open(url)` | new browser in the agent tab, agent partition | project web policy (allowed origins); outside it, a T3-style card |
| `web_snapshot()` | `Accessibility.getFullAXTree` plus visible text, trimmed | none |
| `web_click(target)`, `web_type(text)`, `web_key(key)` | `DOM`/`Input.dispatch*Event` | none in the agent tab |
| `web_screenshot()` | `Page.captureScreenshot` | none |
| `web_console()`, `web_network()` | `Runtime`/`Log`/`Network` events, bounded buffers | none |
| `web_eval(js)` | `Runtime.evaluate` | per call, like T3, "Always allow" per project |

Agents act only in the **agent browser tab** (a Fyx item, visible to the user, showing what the
agent sees), never in an extension's web view or a page the user signed into. Tools come from
`webview_host` and are offered to ACP agents and through `fyx_relay`'s MCP mode (7.16), so they
work the same with any provider that advertises `cdp` (Servo would offer a smaller set).

Optional, off by default: an external CDP endpoint for Playwright through the 0.4 loopback
listener pattern (`127.0.0.1:0`, host token, `Origin`/`Host` checks; B.7), only for the agent
partition.

## Licensing and maintenance

- CEF is BSD-3; Chromium is BSD with many third-party licenses. The helper ships
  `chrome://credits` and Fyx's provider page links it. Standard CEF builds have no proprietary
  codecs (H.264, AAC) and no Widevine; adding them needs our own CEF build and codec licensing:
  out of scope.
- Chromium ships security fixes every 1–4 weeks. The provider repo tracks one CEF branch, CI
  rebuilds on each CEF release, and Fyx warns on the provider page when the helper is more than
  8 weeks older than the newest provider release. This recurring cost is the main price of CEF.
- `cef` (tauri-apps/cef-rs) versions as `<crate>+<CEF>`; the helper lives in its **own
  repository** (`fyx-webview-cef`, owner creates it) with its own `Cargo.lock`, so neither CEF
  nor `cef` enter Fyx's workspace.

## Phased plan (macOS first)

| Step | What | Size | Depends on |
| --- | --- | --- | --- |
| 17a spike | Standalone repo: `cef` crate, OSR with shared textures, one page, IOSurface handed to a tiny GPUI test app over a mach rendezvous; then the same on Windows with a D3D11 handle. Measure download/disk size, RSS for 1 and 5 pages, cold start, ad-hoc signing on Apple Silicon, `cef_sandbox` with our MSVC | S (4–5 days) | — |
| 17b core | `webview_host` crate: protocol v1, supervisor (spawn, restart, idle stop, job object), `WebViewElement` with input, focus, IME, cursor, dialogs; CPU path; a dev setting `fyx.webview.dev_helper_path` to run a locally built helper; macOS | M (2 weeks) | 17a; W0: new member in `Cargo.toml` |
| 17c macOS zero-copy | IOSurface pool to `paint_surface` | S | 17b |
| 17d provider + registry | `[fyx_webview_provider]`, optional `component`, `native:helper` consent, downloader and verifier, natives in `ui_ext_registry`, `fyx-registry` + `policy` rules, provider repo CI with attestation; template change in fyx and fyx-extensions together | M | 17b, 1.1, 5.6/5.7, P3 |
| 17e sandbox | macOS Seatbelt profile for the helper; network policy in the helper | S | 17d, 7.5 |
| 17f WIT 0.5 | `webview` node, events, `webview` import, `ui:web` T1, `[fyx_ui.web]`; SDK, harness fake, demo (a docs viewer) | M | 17c; 0.5 opened by W0 |
| 17g agent tools | agent browser tab, `web_*` tools, web policy, consent | M | 17d, 7.16 relay MCP mode for the MCP route |
| 17h Windows | `fyx:` hook for D3D11 surfaces in GPUI, shared-handle pool, `cef_sandbox`, job object | M | 17b; Windows build of the provider |
| 17i later | Linux on the CPU path; Servo provider on protocol v1; wry overlay provider; external CDP endpoint | — | 17f |

Total to a usable macOS feature (17a–17g): about 8–10 engineer-weeks; Windows adds about 2.
At most two of these compile at once (owner limit); 17a and the registry half of 17d do not
compile Fyx and run alongside.

## Integrator notes

- New workspace member `crates/webview_host` (GPUI, `postcard`, `async-tar`, `zstd`,
  `sha2`; no CEF). Integrator `Cargo.toml` change.
- `fyx:` hooks: Windows `SurfaceSource` variant + `draw_surfaces`; maybe a streaming-texture hook
  for CPU frames (17a decides); none on macOS.
- WIT 0.5: the `webview` node, events and import; `ui:web` (reserved in ARCHITECTURE.md layer 6).
  `spec.md` §5 lists web views as out of scope for platform 1; this item moves them to 0.5.
- Registry format: the `natives` field, its rules, and `policy/native-providers.toml`; change
  `tools/fyx-extensions-template/` and `shelenhovskyi/fyx-extensions` together.

## Open questions for the owner

1. Native providers first-party only for 1.0 (recommended), or open to other publishers with
   owner review?
2. New repository `shelenhovskyi/fyx-webview-cef` for the helper (recommended), or
   `tools/fyx-webview-cef/` excluded from the workspace?
3. `ui:web` as T1 with a fixed origin list (recommended), or T2 at first use?
4. Keep Servo and wry as planned later providers, or drop them?

## Sources

- servo-webview.md (this repo) for GPUI surface facts and the Servo comparison.
- `cef` crate (tauri-apps/cef-rs): https://github.com/tauri-apps/cef-rs, https://docs.rs/crate/cef/latest
- CEF shared-texture OSR rework (Windows and Mac): https://bitbucket.org/chromiumembedded/cef/pull-requests/285
- `OnAcceleratedPaint` sample (Windows D3D11): https://github.com/twobrainsgmbh/cef-mixer
- Shared-texture OSR in Electron (Windows-tested): https://github.com/electron/electron/pull/42001
- CEF on Linux, DMA-BUF HiDPI caveat: https://pkg.go.dev/github.com/bnema/purego-cef2gtk@v0.5.1
- GPUI in this repo: `crates/gpui/src/elements/surface.rs` (`SurfaceSource`, macOS only),
  `crates/gpui/src/window.rs` (`paint_surface`), `crates/gpui_windows/src/directx_renderer.rs`
  (`draw_surfaces` no-op).

Facts marked by the spike (17a): CEF sizes, RSS, macOS shared-texture behaviour on the CEF
version we pin, and `cef_sandbox` with our toolchain. The CEF size figures above are estimates.
