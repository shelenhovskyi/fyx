# Fyx documentation

Every Fyx document under `docs/fyx/` and every review under `tasks/reviews/`, one line each. The
repository overview is [ARCHITECTURE.md](../../ARCHITECTURE.md), and the sprint plan is
[PLAN.md](../../PLAN.md). The lists cover `origin/integration`; documents still on a stream's
branch are listed under [Coming](#coming).

## Using Fyx

- [user-guide.md](user-guide.md): for people who install and use Fyx. Covers privacy defaults, agent change review, UI extensions, security modes, where Fyx keeps its files and troubleshooting.
- [agent-review.md](agent-review.md): reference for agent change review: turning it on, what is reviewed, the review pane and its keys.
- [review-rules.md](review-rules.md): reference for `.fyx/review.toml` (`auto_accept`, `always_ask`, `deny`), matching, trust and loading.

## Writing UI extensions

- [extension-authoring.md](extension-authoring.md): how to write, build and install a `fyx:ui` extension in Rust.
- [sdk-reference.md](sdk-reference.md): the `ui_ext_sdk` guest SDK, item by item.
- [limits-and-gotchas.md](limits-and-gotchas.md): what the host enforces (time budget, caps, sandbox) and the surprising behaviour.

## Platform and API design

- [platform/spec.md](platform/spec.md): Platform 1 spec, a home for third-party UI extensions (install, permissions, registry), with numbered decisions.
- [platform/plan.md](platform/plan.md): Platform 1 work plan: the tasks for each phase, with owners and their "done when" checks.
- [wit-0.2/design.md](wit-0.2/design.md): `fyx:ui@0.2.0` design and the decisions of 2026-11-17.
- [wit-0.2/fyx-ui-0.2.wit](wit-0.2/fyx-ui-0.2.wit): the decided 0.2 WIT world, before it moves to `crates/ui_ext/wit/since_v0.2.0/`.
- [wit-0.2/migration.md](wit-0.2/migration.md): 0.1 → 0.2 coexistence, version gating, SDK and test plan.

## Development

- [ci.md](ci.md): Fyx's own GitHub workflows and the merge gate.
- [manual-tests.md](manual-tests.md): phase 4 manual test scripts for the running editor.

## Reviews and audits (`tasks/reviews/`)

- [integration-1.md](../../tasks/reviews/integration-1.md): judge review of the first integration (W2 host bindings and W4 SDK).
- [integration-2.md](../../tasks/reviews/integration-2.md): judge review of the second integration.
- [integration-3.md](../../tasks/reviews/integration-3.md): judge review of the third integration (W5 agent review on `fyx/main`).
- [w3-deep-1.md](../../tasks/reviews/w3-deep-1.md): deep review of W3's renderer, containers and hooks.
- [w5-deep-1.md](../../tasks/reviews/w5-deep-1.md): deep review of W5's agent change review.
- [wit-0.2-design.md](../../tasks/reviews/wit-0.2-design.md): judge review of the `fyx:ui@0.2.0` design draft.
- [network-audit-1.md](../../tasks/reviews/network-audit-1.md): every outbound request Fyx can still make, and the fixes.
- [license-audit-1.md](../../tasks/reviews/license-audit-1.md): license audit of a GPL fork with Apache-2.0 SDK crates.
- [upstream-diff-audit-1.md](../../tasks/reviews/upstream-diff-audit-1.md): Fyx's diff outside its own crates, with a ranked plan to shrink it.
- [rebase-rehearsal-v1.29.1-pre.md](../../tasks/reviews/rebase-rehearsal-v1.29.1-pre.md): rebase rehearsal onto upstream v1.29.1-pre and `main`.
- [triage-2.md](../../tasks/reviews/triage-2.md): five audit reports turned into action lists for each owner.

## Coming

These documents are on a stream's branch and not on `integration` yet. Each line names the branch;
links get added when the document lands.

| Document | Branch | What it is |
|---|---|---|
| `docs/fyx/security-modes.md` | `claude/fyx-w9-guard`, `claude/fyx-w9-fixes` | Design and rules of the `fyx_security` modes (`off`, `audit`, `shield`) and the later steps. |
| `docs/fyx/platform/registry-format.md` | `claude/fyx-registry-core` | The registry index, block list, cache and CLI formats, as `crates/fyx_registry` implements them. |
| `docs/fyx/wit-0.3/design.md` | `claude/fyx-wit-0.3-design` | `fyx:ui@0.3.0` design draft, for review. |
| `docs/fyx/wit-0.3/fyx-ui-0.3.wit` | `claude/fyx-wit-0.3-design` | The draft 0.3 WIT world. |
| `docs/fyx/wit-0.3/migration.md` | `claude/fyx-wit-0.3-design` | 0.2 → 0.3 coexistence, version gating, SDK and test plan. |
| `docs/fyx/api/fyx-ui-0.1.0.md` | `claude/fyx-w4-sdk-demos-5d1497` | Generated reference of the `fyx:ui@0.1.0` WIT world. |
| `docs/fyx/api/ui_ext_sdk.md` | `claude/fyx-w4-sdk-demos-5d1497` | Generated list of the `ui_ext_sdk` public items. |
| `docs/fyx/api/fyx_ext_harness.md` | `claude/fyx-w4-sdk-demos-5d1497` | Generated list of the `fyx_ext_harness` public items, for `fyx-ext test`. |
| `tasks/reviews/integration-4.md` | `claude/fyx-w7-judge-cd66a5` | Judge review of everything merged after integration-3. |
| `tasks/reviews/w9-step1.md` | `claude/fyx-w7-judge-cd66a5` | Judge review of security step 1 (`fyx_guard`), with the fixes it asks for. |
| `tasks/reviews/triage-3.md` | `claude/fyx-w7-judge-cd66a5` | Five specialist reports sorted into action lists for each owner. |
| `tasks/reviews/async-1.md` | `claude/fyx-review-async` | Concurrency, async and GPUI lifecycle review. |
| `tasks/reviews/bugs-1.md` | `claude/fyx-review-bugs` | Logic-bug review. |
| `tasks/reviews/perf-1.md` | `claude/fyx-review-perf` | Performance review of Fyx's own code. |
| `tasks/reviews/security-2.md` | `claude/fyx-review-security` | Security review of consent, install, packages, registry, the review gate and shield. |
| `tasks/reviews/ux-1.md` | `claude/fyx-review-ux` | UX review of agent review, UI extensions and `fyx-ext`. |
