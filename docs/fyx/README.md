# Fyx documentation

Every Fyx document under `docs/fyx/` and every review under `tasks/reviews/`, one line each. The
repository overview is [ARCHITECTURE.md](../../ARCHITECTURE.md), and the sprint plan is
[PLAN.md](../../PLAN.md). The lists cover `origin/integration`; documents still on a stream's
branch are listed under [Coming](#coming).

## Using Fyx

- [user-guide.md](user-guide.md): for people who install and use Fyx. Covers privacy defaults, agent change review, UI extensions, security modes, where Fyx keeps its files and troubleshooting.
- [agent-review.md](agent-review.md): reference for agent change review: turning it on, what is reviewed, the review pane and its keys.
- [review-rules.md](review-rules.md): reference for `.fyx/review.toml` (`always_ask`, `deny`) and the user setting `agent_review.auto_accept`.
- [security-modes.md](security-modes.md): design and rules of the `fyx_security` modes (`off`, `audit`, `shield`) and the later steps.

## Writing UI extensions

- [extension-authoring.md](extension-authoring.md): how to write, build and install a `fyx:ui` extension in Rust.
- [sdk-reference.md](sdk-reference.md): the `ui_ext_sdk` guest SDK, item by item.
- [limits-and-gotchas.md](limits-and-gotchas.md): what the host enforces (time budget, caps, sandbox) and the surprising behaviour.

## Platform and API design

- [platform/spec.md](platform/spec.md): Platform 1 spec, a home for third-party UI extensions (install, permissions, registry), with numbered decisions.
- [platform/plan.md](platform/plan.md): Platform 1 work plan: the tasks for each phase, with owners and their "done when" checks.
- [platform/registry-format.md](platform/registry-format.md): the formats of the registry index, block list, keysets, cache and CLI, as `crates/fyx_registry` implements them.
- [platform/registry-repo.md](platform/registry-repo.md): design of the future `shelenhovskyi/fyx-extensions` repository: layout, entry format, review rules, CI, keys and the block-list process (plan 3.1, 3.2).
- [wit-0.2/design.md](wit-0.2/design.md): `fyx:ui@0.2.0` design and the decisions of 2026-11-17.
- [wit-0.2/migration.md](wit-0.2/migration.md): 0.1 → 0.2 coexistence, version gating, SDK and test plan. The 0.2 WIT itself is in `crates/ui_ext/wit/since_v0.2.0/`.
- [wit-0.3/design.md](wit-0.3/design.md): `fyx:ui@0.3.0` design, decided 2026-11-19 (the smaller 0.3), staged, not frozen.
- The 0.3 WIT world is staged in
  [`crates/ui_ext/wit/since_v0.3.0/fyx-ui.wit`](../../crates/ui_ext/wit/since_v0.3.0/fyx-ui.wit)
  (the docs copy was removed: one source).
- [wit-0.3/open-questions.md](wit-0.3/open-questions.md): Q1–Q14 with options, recommended answers
  and kind, for the full and the smaller 0.3; all answered (2026-11-19).
- [wit-0.3/migration.md](wit-0.3/migration.md): 0.2 → 0.3 coexistence, version gating, SDK and test plan.
- [wit-0.4/design.md](wit-0.4/design.md): `fyx:ui@0.4.0` terminals and CLI integrations (plan P5):
  the integration kit, the 0.4 consent model with T3, multiple accounts and Fyx profiles.
- [terminal-api/research.md](terminal-api/research.md): research (in Ukrainian) on terminal access for extensions and tool-agnostic CLI integrations (Claude Code, Gemini CLI, Codex and others): the code today, terminal-as-extension shapes, prior art, a consent model with install-time grants, a three-layer proposal and phases.

## Development

- [ci.md](ci.md): Fyx's own GitHub workflows, the merge gate and the advisory triage.
- [mac-build.md](mac-build.md): local unsigned Mac build, `script/fyx-mac-check`, first launch past Gatekeeper.
- [manual-tests.md](manual-tests.md): phase 4 manual test scripts for the running editor.

## Reviews and audits (`tasks/reviews/`)

- [integration-1.md](../../tasks/reviews/integration-1.md): judge review of the first integration (W2 host bindings and W4 SDK).
- [integration-2.md](../../tasks/reviews/integration-2.md): judge review of the second integration.
- [integration-3.md](../../tasks/reviews/integration-3.md): judge review of the third integration (W5 agent review on `fyx/main`).
- [integration-4.md](../../tasks/reviews/integration-4.md): judge review of everything merged after integration-3.
- [w3-deep-1.md](../../tasks/reviews/w3-deep-1.md): deep review of W3's renderer, containers and hooks.
- [w5-deep-1.md](../../tasks/reviews/w5-deep-1.md): deep review of W5's agent change review.
- [w9-step1.md](../../tasks/reviews/w9-step1.md): judge review of security step 1 (`fyx_guard`), with the fixes it asks for.
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
| `docs/fyx/api/fyx-ui-0.1.0.md` | `claude/fyx-w4-sdk-demos-5d1497` | Generated reference of the `fyx:ui@0.1.0` WIT world. |
| `docs/fyx/api/ui_ext_sdk.md` | `claude/fyx-w4-sdk-demos-5d1497` | Generated list of the `ui_ext_sdk` public items. |
| `docs/fyx/api/fyx_ext_harness.md` | `claude/fyx-w4-sdk-demos-5d1497` | Generated list of the `fyx_ext_harness` public items, for `fyx-ext test`. |
| `tasks/reviews/triage-3.md` | `claude/fyx-w7-judge-cd66a5` | Five specialist reports sorted into action lists for each owner. |
| `tasks/reviews/async-1.md` | `claude/fyx-review-async` | Concurrency, async and GPUI lifecycle review. |
| `tasks/reviews/bugs-1.md` | `claude/fyx-review-bugs` | Logic-bug review. |
| `tasks/reviews/perf-1.md` | `claude/fyx-review-perf` | Performance review of Fyx's own code. |
| `tasks/reviews/security-2.md` | `claude/fyx-review-security` | Security review of consent, install, packages, registry, the review gate and shield. |
| `tasks/reviews/ux-1.md` | `claude/fyx-review-ux` | UX review of agent review, UI extensions and `fyx-ext`. |
