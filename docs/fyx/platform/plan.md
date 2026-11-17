# Platform 1: work plan

Implements [spec.md](spec.md) (decision numbers in brackets). Owners are the existing streams;
"new" means a stream the coordinator starts. Every task ends with the behaviour seen in the
running editor, not only green tests. Integrator-only files (`Cargo.toml`, `Cargo.lock`,
`crates/ui_ext/wit/`) go through W0 as usual. Revised for the 0.2 decisions, [decided 2026-11-17](../wit-0.2/design.md#decisions-2026-11-17):
decorations moved from P4 to P2 (§L); notifications and progress are §M.

## Before P1: merge what already exists

| # | Owner | Task | Done when |
|---|---|---|---|
| 0.1 | W0 | Merge `claude/fyx-ext-template` (`559014f693`) and `claude/fyx-ts-sdk` (`af96c0f873`) after review; they seed `fyx-ext new` and `sdks/ts` | both on `integration`, gate green |
| 0.2 | W0 | Merge the pending audit and review branches (`fyx-audit-network`, `-diff`, `-licenses`, `fyx-review-w5`, `fyx-rebase-report`) and route their findings; network-audit findings feed P1.6 and P4.6 | findings listed in `tasks/reviews/` with owners |

## P1 local platform

| # | Owner | Task | Depends on | Done when |
|---|---|---|---|---|
| 1.1 | W3 | Install store: layout, `state.json`, keep previous version, rollback, pin [13]; keep a precompiled `.cwasm` per installed version, keyed by the wasmtime engine, so large components start without recompiling | — | install, update, roll back a local `.fyxext` in the dev build; a second start skips compilation; only `.cwasm` files Fyx wrote into the install store are ever deserialized, never one from a package |
| 1.2 | W3 | "Install Dev Extension" + `fyx.extensions.dev_paths`; env var stays as override [6] | 1.1 | dev extension survives restart without the env var |
| 1.3 | W3 | Hot reload in place + "Restart extension" after a failure [7, 8] | 1.2 | edit → rebuild → panel updates within 1 s; failure 3 of spec §4 passes |
| 1.4 | new `platform-cli` | `fyx-ext` crate and binary: `new` (from the template), `build`, `dev`, `check` with declared-vs-used capabilities, `package` [4, 5, 12] | 0.1 | `fyx-ext new demo && fyx-ext dev` opens the panel in Fyx |
| 1.5 | W3 | Permission broker, tiers T0–T1, install dialog, revocation, capability list on the page (reuse capability chips) [21, 26] | 1.1 | T1 extension asks once; revoke stops reads at once |
| 1.6 | W3 | Instance limits: no preopens, no env, 256 MiB memory, 2 s first mount [25] | — | a guest that allocates 300 MiB traps without hurting others |
| 1.7 | W3 | Extension Inspector panel [9] | 1.3 | tree, patches, events, timings visible for the counter demo |
| 1.8 | W4 | `fyx_sdk::testing` harness + `fyx-ext test` [10] | 1.4 | demo tests run headless in CI |
| 1.9 | W7 | Generated API reference + doc drift test [11]; rewrite `extension-authoring.md` around `fyx-ext` | 1.4 | reference regenerates on merge; drift test catches a renamed item |
| 1.10 | W0 | wasmtime policy: Cranelift only, `cargo deny advisories` in `script/gate.sh` [27] | — | gate fails on a seeded advisory |

## P2 `fyx:ui@0.2.0`

| # | Owner | Task | Depends on | Done when |
|---|---|---|---|---|
| 2.1 | W0 | Open questions answered and §L decorations added, [decided 2026-11-17](../wit-0.2/design.md#decisions-2026-11-17). Left: add section M (notifications, progress) [22]; land the 0.2 WIT next to 0.1 [29] | — | both worlds in `crates/ui_ext/wit/`, 0.1 demos unchanged and working |
| 2.2 | W2c | Host side of A–M (including the `decorations` interface of §L), version dispatch | 2.1 | probe fixture covers every new import |
| 2.3 | W3 | Renderer side of A–K and M: ticks, modal with keyboard selection and the click-or-Enter gesture rule, dismissed, runtime surfaces, frozen icons, notifications and progress | 2.2 | clock status item, picker modal, progress demo in the dev build |
| 2.4 | W4 | Rust SDK 0.2 (with `place_decoration` / `remove_decoration` for §L) + demos moved to 0.2 as examples | 2.2 | demos build for both 0.1 and 0.2 |
| 2.6 | W6 (restart) | Decorations §L on W6's anchor mechanism: an extension backend for `Decorations`, worktree-relative path matching, the registry effects, the 512 cap | 2.2 | a demo places blocks under TODO lines that follow edits and route clicks to `handle-event` |

## P3 registry

| # | Owner | Task | Depends on | Done when |
|---|---|---|---|---|
| 3.1 | W0 | Create repo `shelenhovskyi/fyx-extensions` (owner's decision, 2026-11-17); entry format, review rules as CODEOWNERS + a CI check for decision 16 [14, 16] | 1.4 | a PR adding a T2 capability cannot merge without a maintainer |
| 3.2 | W0 | Registry CI: build Rust extensions from source in a pinned image, `check`, `test`, Ed25519 sign, publish `index.json` + packages to Releases [15] | 3.1, 1.8 | a sample extension appears in `index.json` with a valid signature |
| 3.3 | W3 | Registry client: index, 24 h age gate, `urgent`, signature check, unsigned-file badge [17, 18] | 3.2 | install from the index; tampered package refused |
| 3.4 | W3 | Block list: fetch every 6 h, disable, auto-rollback, cached copy, 7-day warning [19] | 3.3 | spec §4 failures 1, 2, 5 pass |
| 3.5 | W3 + one `fyx:` hook | Extensions page Fyx tab: search, install, update, pin, roll back, held updates [17, 21] | 3.3, 1.5 | user flow done end to end in the dev build |

## P4 `fyx:ui@0.3.0` and T2

| # | Owner | Task | Depends on | Done when |
|---|---|---|---|---|
| 4.1 | W0 | 0.3 design draft for decision 28 (cloud task, like the 0.2 draft), then the WIT | P2 | design reviewed by W7 |
| 4.2 | W6 | Decoration extras beyond 0.2 §L (decision 28): inline kinds, gutter signs, line backgrounds, namespaces, visible-range pull, `color-swatch`, `decoration-orphaned` | 4.1, 2.6 | TODO highlighter and gutter-sign demo work while editing |
| 4.3 | W3 | Picker protocol on the 0.2 modal | 4.1 | two extensions contribute sources to one picker |
| 4.4 | W3 | Code lens, hover, virtual documents, commands in palette and keymap | 4.1 | each has a demo in the gallery |
| 4.5 | W3 | T2 consent at first use, held updates for new T2 / domains / programs [21] | 1.5 | spec §4 failure 1 passes |
| 4.6 | W2c | `network:fetch` host import with domain list and caps [23] | 4.5 | undeclared domain → `denied`; 17 MiB response cut |
| 4.7 | W2c | `process:spawn`: PATH outside worktree, filtered env, job objects [24] | 4.5 | spec §4 failure 4 passes; reload kills the process tree |
| 4.8 | W2c | `workspace:edit` (undoable transactions), `agent:observe` read-only sessions | 4.5 | an edit undoes with one ctrl-z; observe cannot accept or reject |
| 4.9 | W7 | Judge pass over P1–P4, then the 1.0 freeze clock starts [30] | 4.1–4.8 | review file in `tasks/reviews/` |

## Deferred after 1.0

TypeScript extensions are deferred until after 1.0, and whether to do them at all is re-evaluated
then (owner, 2026-11-18; spec decision 3). P3 builds and signs Rust extensions only. `sdks/ts`
stays in the repository as an unsupported prototype.

| # | Was | Task | Revisit |
|---|---|---|---|
| D.1 | 2.5, new `ts-sdk` | `@fyx/sdk` on the current world with `jco componentize`, `fyx-ext new --lang ts`, SDK parity test | after the 1.0 freeze |

## Parallelism

At most 2 compiling streams plus the gatekeeper (owner limit). Non-compiling work runs alongside:
1.9, 2.1, 3.1, 3.2 (registry repo) and 4.1 (design). Suggested pairs:
P1 = W3 (1.1→1.3→1.5→1.7) + `platform-cli` (1.4, then 1.8 with W4); P2 = W2c + W3, then W6
for 2.6;
P3 = W3 alone (registry CI is non-compiling); P4 = W6 + W3, then W2c.
