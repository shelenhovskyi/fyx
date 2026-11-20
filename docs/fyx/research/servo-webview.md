# Servo as a webview in Fyx

Status: research, 2026-11-20. No product code. Targets WIT 0.5 (0.4 is frozen).

## Recommendation

1. **Run Servo out of process**, in a helper binary (`fyx-webview`) built from its **own Cargo
   workspace**, not as a dependency of the editor. One helper per Fyx instance hosts one `Servo` and
   many `WebView`s. This buys crash isolation (SpiderMonkey, script panics), an OS sandbox around the
   web content, and, just as important, keeps Servo's huge dependency tree and its non-semver
   0.x churn out of Fyx's `Cargo.lock`. The helper can be downloaded on first use, so Fyx's own
   binary does not grow.
2. **Hand frames to GPUI as shared GPU textures**, not pixels:
   - macOS: surfman renders into an IOSurface; the helper sends its mach port; Fyx wraps it in a
     `CVPixelBuffer` and draws it with GPUI's existing `Window::paint_surface` / `surface()` element.
     **This path exists in GPUI today** (it is what screen sharing uses), so macOS is zero-copy with
     no GPUI change.
   - Windows: Servo's GL runs on ANGLE over D3D11; share the frame as a DXGI NT shared handle and
     open it on GPUI's D3D11 device. GPUI's `draw_surfaces` in `gpui_windows/src/directx_renderer.rs`
     is a **no-op stub** and `SurfaceSource` is macOS-only, so this needs a small `fyx:` hook in
     `gpui`/`gpui_windows`.
   - Linux: `gpui_wgpu` also ignores `PrimitiveBatch::Surfaces`. dmabuf import into wgpu needs
     wgpu-hal external memory; defer it and use the CPU path.
   - Fallback everywhere: `SoftwareRenderingContext` + `read_to_image()` (RGBA8) uploaded as a GPUI
     image. Good enough for docs, previews and MCP Apps at moderate sizes; costs about 8 MB of copy per
     1080p frame, so it is not the end state for scrolling-heavy content.
3. **Keep one backend-neutral API.** A Fyx crate (`webview_host`) owns a `WebViewBackend` trait with
   two implementations, Servo-helper and wry. Extensions see only the WIT 0.5 `webview` element
   behind a `ui:web` permission; agents see a small automation tool set (below), never WebDriver
   directly.
4. **Do not use a native child window for Servo.** That is what wry/`gpui-wry` must do: it floats
   above GPUI (no popovers, menus or drag previews over it, clipping and z-order bugs, scroll sync
   lag) and does not work on Wayland. Servo's whole advantage is rendering into our texture.

Ship order: macOS Servo with zero-copy, then Windows (CPU path first, shared handle second), Linux on
the CPU path. Keep wry as the "real web compatibility" backend for content Servo cannot render yet.

## Facts this rests on

| Topic | What we know | Confidence |
| --- | --- | --- |
| Crate | `servo` 0.1.0 on crates.io 2026-04-13; 0.4.0 end of July 2026; 0.5 in August 2026 | high (release posts, Phoronix) |
| Stability | `servo` and its `servo-*` dependencies do not follow semver; any release may break embedders | high (Servo blog, April 2026) |
| Embedding API | `ServoBuilder`, `WebViewBuilder`, delegates (`ServoDelegate`, `WebViewDelegate`), `RenderingContext` with window, offscreen and software variants; the embedder calls `WebView::paint` then `RenderingContext::present`; `read_to_image` reads the back buffer | high (docs.rs, Servo slides) |
| GPU interop precedent | Slint: surfman GL frame blitted into a shared texture, IOSurface→Metal on macOS, shared Vulkan memory on Linux, ANGLE→D3D12 on Windows. godot-servo: one offscreen surfman surface, stable texture handle; Servo and host must be on the same GPU | medium (third-party write-ups) |
| CPU precedent | `iced_servo` uses `SoftwareRenderingContext` + `read_to_image`, drawn as a wgpu texture; pumps only the active tab | high |
| Multiprocess | Opt-in (`--multiprocess` / builder option), single-process by default; full support only on Linux and macOS; sandboxing is not audited and Servo does not aim for Chromium-level sandboxing | medium (2025 posts, NLnet page) |
| WebDriver | Moved out of libservo into servoshell (2025); embedders host the WebDriver server themselves; conformance tests run in CI. No CDP. Servo has a devtools server speaking Firefox's remote protocol | medium |
| Size | servoshell Linux tarball about 46 MB compressed; expect roughly 50–100 MB unpacked | low, measure in P0 |
| Memory | No published numbers | unknown, measure in P0 |
| License | MPL-2.0 (Servo, SpiderMonkey) | high |

## Rendering options compared

| Option | Copies per frame | GPUI work | Platforms | Verdict |
| --- | --- | --- | --- | --- |
| Shared GPU texture (IOSurface / DXGI handle / dmabuf) | 0 (one GPU blit inside Servo) | macOS none; Windows implement `draw_surfaces` + `SurfaceSource` variant; Linux wgpu external memory | macOS now, Windows next, Linux later | **target** |
| CPU readback (`read_to_image`) | GPU→CPU→GPU, about 8 MB at 1080p | none (image element); better a dedicated streaming texture than the sprite atlas | all | **fallback and first milestone** |
| Native child window | 0 | overlay positioning only | macOS, Windows, X11 | reject for Servo |

Cross-process sharing is standard on each OS (IOSurface mach port, `DXGI_SHARED_RESOURCE` NT handle
duplicated into Fyx, dmabuf fd over a Unix socket), so out-of-process does not cost extra copies; it
costs about one frame of latency for the frame-ready message.

## Input, focus, IME, scroll

The Fyx element translates GPUI events into Servo `InputEvent`s with coordinates relative to the
element's bounds and the window scale factor:

- Mouse move/down/up, wheel (pixel and line deltas): Servo scrolls itself; Fyx never scrolls the
  texture.
- Keyboard: GPUI `Keystroke` to `keyboard-types` key/code/modifiers. Editor keybindings must not
  steal keys while the webview is focused, except a short escape list (focus-out, command palette).
- IME: the element implements GPUI's input handler and forwards marked text as composition events;
  Servo's delegate reports when to show the IME and the caret rect, which goes back as the
  handler's `bounds_for_range`.
- Focus: GPUI `FocusHandle` maps to `WebView::focus`/`blur`.
- Delegate callbacks back to Fyx: cursor shape, title, load state, new-window requests (deny or open
  in Fyx's browser), dialogs (alert/confirm/prompt shown as GPUI modals), clipboard, permission
  prompts (deny by default).

API names above are from the 0.x docs and will move; pin one Servo version per helper release.

## Threading and process model

- `Servo` is single-threaded from the embedder's side: create it, spin its event loop
  (`spin_event_loop` when the `EventLoopWaker` fires) and paint on one thread that owns the GL
  context. Script, layout and the constellation run on Servo's own threads.
- In-process would mean running that on GPUI's main thread from a foreground task woken by the
  waker. It works, but a Servo panic or hang takes the editor down, and Servo would land in Fyx's
  dependency graph.
- Out-of-process helper: its own main loop, no winit window. IPC over a Unix socket / named pipe
  with a small versioned protocol: `create(id, size, scale)`, `resize`, `navigate`, `input`,
  `focus`, `eval_js`, `screenshot`, `close` from Fyx; `frame_ready(id, texture_handle)`,
  `delegate_event`, `crashed` from the helper. On crash Fyx shows a "reload" placeholder and
  restarts the helper.
- Sandbox the helper with the OS: Seatbelt on macOS, AppContainer/restricted token on Windows,
  landlock + seccomp on Linux. Network allowed only to origins the webview's policy permits
  (enforced in the helper's fetch layer); no `file://` except a Fyx-provided extension asset root.

## Web compatibility

Servo renders static and documentation-style pages, markdown/HTML previews and simple apps well and
improves monthly (0.5 fixed DuckDuckGo rendering and sped up text). Expect gaps in heavy web apps,
some media and codecs (GStreamer on Linux/macOS), WebRTC, service workers and accessibility. That
fits Fyx's use cases (extension-owned HTML, MCP Apps, docs, previews, agent-opened pages) but not
"any website". Check the WPT pass-rate page on servo.org before each upgrade.

## How agents drive it

Agents get Fyx tools, not a protocol: `web_open(url)`, `web_snapshot()` (DOM/text outline via
injected JS), `web_click(selector|point)`, `web_type(text)`, `web_eval(js)` (gated),
`web_screenshot()` (`read_to_image`). These map onto the helper protocol, so they work identically
with the wry backend (where they map to `evaluate_script` and native input). Hosting Servo's
WebDriver server inside the helper is possible later for external tooling; there is no CDP, so
Playwright/Puppeteer will not connect.

## Licensing

MPL-2.0 is file-level copyleft: modified Servo files must stay MPL and their source available; our
own code is unaffected. MPL-2.0 is explicitly compatible with GPL-3.0/AGPL-3.0 (secondary licenses)
and Apache-2.0 combinations, so it is fine with Zed's licensing. A separate helper binary makes the
boundary even clearer. Ship Servo's license notices with the helper.

## Comparison with wry

| | Servo helper | wry |
| --- | --- | --- |
| Rendering | into a GPUI texture, composited like any element | native child window on top of GPUI |
| Web compat | partial | full (WebKit, WebView2, WebKitGTK) |
| Size | helper about 50–100 MB, downloaded on demand | small; WebView2 runtime is preinstalled on Windows 11 |
| Linux | X11 and Wayland via texture | WebKitGTK; `gpui-wry` is macOS/Windows only; no Wayland child windows |
| Crash isolation | ours (separate process) | OS engines are multi-process already |
| Control and automation | full (input, JS, readback) | JS and native input; no pixel readback on every OS |
| API stability | 0.x, breaking releases | stable |

## Prototype plan

| Step | What | Effort |
| --- | --- | --- |
| P0 spike | Standalone repo: `servo` 0.5 + offscreen/software context, load a page, `read_to_image` to PNG. Measure build time, binary size, RSS for 1 and 5 webviews, cold start, on Windows and macOS. Check mozjs builds with our MSVC toolchain | 3–5 days |
| P1 CPU path | Helper protocol; Fyx crate `webview_host` with `WebViewBackend`; GPUI element that streams frames into a texture; input, scroll, focus, IME, cursor, dialogs | 1.5–2 weeks |
| P2a macOS zero-copy | IOSurface from surfman to `paint_surface` | 3–5 days |
| P2b Windows zero-copy | DXGI shared handle; `SurfaceSource` Windows variant and `draw_surfaces` in `gpui_windows` (`fyx:` hook) | 1–1.5 weeks |
| P3 WIT 0.5 | `webview` element (`src`/`html`, size, `post-message` both ways, load events) behind `ui:web`; origin allow-list in the manifest | 1 week, integrator |
| P4 agent tools | The tool set above, plus review of what agents may open | 1 week |
| wry backend | Same trait via `gpui-wry`-style overlay, macOS/Windows | about 1 week, parallel |

Total about 6–9 engineer-weeks to a usable macOS/Windows feature.

## Integrator notes

- The helper lives in its own workspace (separate repo or `tools/fyx-webview/` with its own
  `Cargo.toml`/`Cargo.lock`, excluded from the root workspace), so Servo never enters Fyx's
  `Cargo.lock`. Excluding it from the root workspace is an integrator change.
- `webview_host` (new workspace member, depends only on GPUI and IPC crates) is an integrator
  `Cargo.toml` change.
- GPUI hooks: Windows `SurfaceSource` + `draw_surfaces`; later wgpu external-texture surfaces.
- WIT 0.5: `webview` element and `ui:web` permission (already reserved in ARCHITECTURE.md layer 6).

## Sources

- servo 0.4.0 on docs.rs: https://docs.rs/crate/servo/latest
- `OffscreenRenderingContext`: https://docs.rs/servo/latest/servo/struct.OffscreenRenderingContext.html
- Servo blog, April 2026 (crate, semver warning): https://servo.org/blog/2026/05/31/april-in-servo/
- Servo blog, June 2026: https://servo.org/blog/2026/07/31/june-in-servo/
- Servo 0.4 / 0.5 (Phoronix): https://www.phoronix.com/news/Servo-0.4-Released, https://phoronix.com/news/Servo-0.5-Released
- Simon Willison, servo crate exploration: https://simonwillison.net/2026/apr/13/servo-crate-exploration
- Embedding Servo (slides): https://servo.org/files/2025-09-13-the-joy-and-value-of-embedded-servo-systems.pdf
- Slint + Servo: https://slint.dev/blog/using-servo-with-slint, https://slint.dev/blog/servo-with-slint-update
- godot-servo: https://github.com/shiena/godot-servo
- iced_servo: https://docs.rs/crate/iced_servo/0.1.0/source/README.md
- surfman: https://github.com/servo/surfman
- KDAB, Servo in Qt: https://www.kdab.com/embedding-servo-in-qt/
- Multiprocess: https://servo.org/blog/2025/10/24/this-month-in-servo/, https://nlnet.nl/project/Servo-Multiprocess/
- WebDriver via servoshell: https://servo.org/blog/2025/08/22/this-month-in-servo/
- GPUI in this repo: `crates/gpui/src/window.rs` (`paint_surface`), `crates/gpui/src/elements/surface.rs`,
  `crates/gpui_windows/src/directx_renderer.rs` (`draw_surfaces` stub),
  `crates/gpui_wgpu/src/wgpu_renderer.rs` (`PrimitiveBatch::Surfaces` ignored)
