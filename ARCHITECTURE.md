# Fyx architecture

Fyx is a fork of Zed (pinned tag in `FYX_UPSTREAM`) that adds a native, themed, sandboxed UI API for
WASM extensions, with agent change review as a first-class host object. Research and rationale:
[docs/research/extensible-editor-based-on-zed.md](docs/research/extensible-editor-based-on-zed.md).

## Fork discipline (Brave/VSCodium pattern)

The fork is measured by the size of its diff outside its own crates. Keep it at a few dozen lines.

- All new code lives in Fyx-owned crates (below). Upstream crates get only thin hooks.
- Every upstream edit carries a `fyx:` marker comment, so `git grep -n 'fyx:'` lists the whole
  upstream surface.
- Branding, URLs and disabled services go through default settings, env vars and feature flags,
  not scattered edits.
- Rebase onto every stable release while the diff is small. A weekly CI job tries the stack on the
  newest tag and opens an issue on conflict.

## Crates

| Crate | License | Role |
|---|---|---|
| `crates/ui_ext` | Apache-2.0 | The `fyx:ui@0.1.0` WIT world (`wit/fyx-ui.wit`), the staged `fyx:ui@0.2.0` world (`wit/since_v0.2.0/fyx-ui.wit`, model in `ui_ext::v0_2`) and the transport-independent tree/patch model. No wasmtime, no GPUI. |
| `crates/ui_ext_host` | GPL-3.0-or-later | `ExtensionPanel: Panel` / status items; maps tree nodes to GPUI elements and theme tokens; routes events by node id. |
| `crates/ui_ext_sdk` | Apache-2.0 | Guest SDK (`wit-bindgen`) for Rust extensions targeting `wasm32-wasip2`. Published later as `fyx-sdk`. |
| `crates/agent_review` | GPL-3.0-or-later | Host model of agent-proposed edits; hunk-level accept/reject; fed by ACP `session/request_permission`. |

Host-side WIT bindings live in a new module inside `extension_host` (the one place that already owns
the wasmtime engine).

## The UI contract

- **Retained, never immediate.** GPUI renders synchronously every frame; extension calls are async,
  cross-thread and serialized. The host keeps the last tree and draws it; updates arrive as patches.
  **Zero WASM calls inside `render()`.**
- **Flat tree.** WIT cannot express recursive types, so a tree is `list<node>` with parent ids,
  followed by `set-props` / `insert` / `remove` patches.
- **Theme tokens only.** No raw colours cross the boundary.
- **Virtualized lists** pull rows with `list-rows(start, count)`, like GPUI's `uniform_list`.
- **One extension call per input event**; scroll, hover and typing are debounced; patches have a node
  cap; slow extensions are disabled by time budget.
- **Separate world.** Only UI extensions implement `fyx:ui`; existing Zed extensions are untouched and
  a future official Zed API cannot collide.
- **UI calls get their own queue or `Store`**, so a slow `download-file` in the same extension cannot
  block click handling, and a shorter epoch than Zed's 100 ms.
- **Heavy models cross as handles, not copies.** Buffers and syntax trees stay in the host; complex
  surfaces (diff view, markdown, review) are host components the extension only parameterizes.
- **Policy and consent stay in the host.** Extensions observe agent sessions; they never decide
  whether an agent edit lands.

## Layers

| # | Surface | Rendered by | Capability | Sprint |
|---|---|---|---|---|
| 1 | Containers: dock panel, status item (tab, modal later) | `ui_ext_host` | `ui:panel`, `ui:status-bar` | yes |
| 2 | Declarative tree: flex, text, button, input, list, table, icon | `ui_ext_host` → GPUI | (layer 1) | yes |
| 3 | Host components: `diff-view`, `markdown`, editor view | Zed's editor/diff/markdown | `workspace:read-buffers` | yes |
| 4 | Buffer decorations anchored to CRDT anchors | editor block/inlay system | `ui:editor-decorations` | stretch |
| 5 | Proposed-edit model, hunk apply/reject/comment | `agent_review` | `agent:review` | basic |
| 6 | Out-of-process sidecar over JSON-RPC (same schema); opt-in web surface for MCP Apps | same renderer / isolated webview | `process:sidecar`, `ui:web` | no |

## Upstream hook points (expected)

| Upstream file | Hook | Owner |
|---|---|---|
| `Cargo.toml` | workspace members + deps for Fyx crates | W0 |
| `.cargo/config.toml` | sccache, lighter dev debug info | W0 |
| `crates/workspace` | dynamic panel registration from the extension host | W3 |
| `crates/extension_host` | `fyx:ui` bindings module, version gating without the nightly limit | W2 |
| `crates/zed` | `ui_ext_host::init`, `agent_review::init`; brand/paths | W1, W3 |
| `crates/acp_thread` / `agent_ui` | route permission requests into `agent_review` | W5 |
| `crates/editor` | decoration hook | W6 |

## Branding

Name **Fyx**, CLI `fyx`, bundle ID `dev.fyx.Fyx`, config `~/.config/fyx`, URL scheme `fyx://`, WIT
namespace `fyx:ui`. Never use "Zed" in the product name. License headers stay in every file;
license notices are generated with `cargo-about` (`script/generate-licenses`).
