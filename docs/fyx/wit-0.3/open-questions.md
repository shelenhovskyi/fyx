# `fyx:ui` 0.3: open questions for the owner

Status: **for the owner's answer**, 2026-11-18. Each question here is one of the "Open questions
for the owner" in [design.md](design.md#open-questions-for-the-owner) (Q1–Q14).

**Why now.** The judge review integration-5 (`tasks/reviews/integration-5.md`, branch
`claude/fyx-w7-judge-cd66a5`, 2c827f977e) says the 0.3 freeze clock (spec decision 30: 4 weeks with
no breaking change, then `fyx:ui@1.0.0`) cannot start while Q1–Q12 are open. Its recommendations:
- answer them in one sitting;
- take scope out rather than leave a question open.

**Two versions.** The answers depend on what 0.3 contains, which is the review's decision 2:

- **(a) Full 0.3:** everything in design.md stays in 0.3, and plan 4.2–4.4 are built before the
  clock starts.
- **(b) Smaller 0.3** (the review's recommendation): images, canvas, the T2 imports
  (`network:fetch`, `process:spawn`, `workspace:edit`, `agent:observe`) and notifications. The
  picker, code lens, hover, virtual documents, commands and decoration extras (design §1–§5) move
  to 0.4. So does document context (§6), whose only users are those providers. Under (b), **Q3, Q6,
  Q11 and Q12 vanish**: they move to 0.4 with their features.

**Kind** of each question:
- **Product:** a product-direction or user-facing policy call, so the owner decides.
- **Technical:** the coordinator may decide it for the owner under the decision mandate, and only
  the recommendation needs a nod.

## Already decided

| Q | Decision | Source |
|---|---|---|
| Q9 | **Version lifetime** (integration-5 decision 4; decided 2026-11-18, matching the owner's earlier rule in spec decision 29). See the note below. | spec decision 29, integration-5 decision 4 |
| Q13 | **Image budget and error semantics.** 64 MiB of decoded pixels per extension, 32 live generated images, 2 concurrent decodes per extension and 4 in total. The error rules are as in design §12, "Errors". | design.md Q13 (decided 2026-11-18) |

**Q9 in detail.** This replaces design.md's earlier Q9 recommendation, "0.1 removed in 1.0, 0.2 in
1.1".
- **What was decided:**
  - 0.1 and 0.2 keep loading through 1.x;
  - their removal is stated as "two minor versions after 1.0", which is spec decision 29's
    two-version rule counted from 1.0 instead of from 0.4. 0.4 never ships if 1.0 follows 0.3.
  - 0.3 is the same world as 1.0 under another package version, so it loads as long as 1.0 does.
- **To confirm:** the two clauses differ only if 1.x reaches 1.3 or later. "Through all of 1.x"
  keeps 0.1 and 0.2 until 2.0; "two minor versions after 1.0" allows removing them from 1.2.
  Nothing changes before 1.2 either way, and the code keeps `MIN_VERSION = 0.1.0`
  (`ui_ext_wasm.rs:130`).
- **Consequence:** the TypeScript SDK, which targets 0.1 only, keeps a supported target through 1.x.

## (a) Full 0.3

| Q | Question | Options | Recommended, and why | Kind |
|---|---|---|---|---|
| Q1 | Close the `zed:extension` side door before T2 ships? A Fyx package's `extension.wasm` gets upstream `fetch` with no check, and `process:exec` for `*`. | (1) The registry refuses packages with an `extension.wasm`, and a local install warns. (2) Route upstream `fetch` and `exec` through the Fyx broker now. (3) Leave it and document it. | **(1).** Without it, the domain and program lists of §8 and §9 protect nothing. (2) is the real fix, but it changes upstream's extension host and can come after 1.0 without touching the `fyx:ui` WIT. The install dialog for a local or unsigned package that has one says "Also runs a Zed extension with unrestricted network and process access". Nothing checks this on integration yet. | Product (what the registry accepts) |
| Q2 | May `[[fyx_ui.processes]]` name interpreters and shells (`node`, `python`, `sh`, `bash`, `pwsh`, `cmd`, `osascript`, `git -c`)? | (1) No. (2) Yes, with no extra rules. (3) Yes, with optional `args` patterns and a host list of "runs any code" programs. | **(3).** (1) breaks linters and formatters that are scripts. (2) hides that such a grant means "any program". With (3), the prompt says "can run any program on your computer", and registry review (decision 16) requires argument patterns or a written reason. | Product (security policy and prompt wording) |
| Q3 | Put document context (§6: text, selections and diagnostics in provider requests) into 0.3? | (1) Yes, as drafted, with no read imports. (2) No; providers get positions only. | **(1).** Providers and pull decorations are of little use without text. Record fields cannot be added after 1.0 without duplicating the requests. `workspace:read-files` can come later, additively. | Product (scope; hard to change after 1.0) |
| Q4 | `localhost` and private addresses for `network:fetch`? | (1) Never. (2) `localhost`/`127.0.0.1` only with an explicit port in the manifest, and other private addresses only from a user setting. (3) Whatever the manifest lists. | **(2).** Local dev servers are useful, but loopback and the LAN are where SSRF hurts. The prompt says "services on your computer, port 8080". A public-looking domain that resolves to a private address works only when the user lists it in `fyx.extensions.network.private_hosts`, never by the manifest alone. W2c's `network:fetch` already does the loopback part: `http://` only for a declared loopback host with a port, and pinned public addresses. The setting is not built yet. | Technical (security default; the setting is additive) |
| Q5 | `workspace:edit` without a gesture, and how visible? | (1) Require a click or Enter. (2) No gesture, but visible edits. | **(2).** Formatters and fixers edit without a click. It means one undo step per buffer. An edit that touches more than one file, or a file not open in an editor, raises the non-focusing notification with Show and Undo. The host never saves. The protected paths are fixed in the host (`ui_ext::edit::PROTECTED_DIRECTORIES`: `.git`, `.fyx`, `.zed`, `.vscode`, `.idea`), and an extension cannot change them. | Product (user-facing behaviour) |
| Q6 | Accept four upstream hooks in hot editor files? They are `HighlightKey::FyxExtension`, `InlayId::Extension`, gutter-sign painting and the code-lens merge, plus the palette hook for commands. | (1) Accept all. (2) Skip the code-lens hook and draw lenses as §L blocks. (3) Drop the features that need hooks. | **(1).** Each hook is small and marked `fyx:`. Without the code-lens hook, a line with a language-server lens shows two lens lines. The cost is merge conflicts on upstream syncs, which the rebase report tracks. | Technical (upstream-diff budget) |
| Q7 | Memory cap for 0.3 components? | (1) 32 MiB. (2) 256 MiB for 0.3 and 32 MiB for 0.1/0.2. (3) 256 MiB for all. | **(3), as the code already does** (`UI_MEMORY_LIMIT = 256 MiB` for every version, `ui_ext_wasm.rs:174`). A 16 MiB response or an 8 MiB virtual document needs a copy in guest memory. The figure is a cap, not a reservation, and spec decision 25 says 256 MiB. A separate 32 MiB cap for old worlds would add a second limit to test and buy little. | Technical |
| Q8 | How much of a review does `agent:observe` see? | (1) Full hunk text and comments for files inside the worktrees, with opaque ids. (2) Metadata only: paths and counts. | **(1).** It makes review-aware linters possible, not only dashboards. Files outside the worktrees show nothing (§11). The capability is T2 and asked at first use, so the prompt is where the user agrees to it. | Product (privacy of agent work) |
| Q9 | Version lifetime after 1.0 | — | **Decided**; see above. | — |
| Q10 | Consent prompt form for T2 at first use? | (1) A non-modal notification with a 1 s click guard and no keyboard answer. (2) A modal dialog. | **(1).** A modal interrupts typing and is answered by reflex. The guard stops a click meant for something else from granting. | Product (UX) |
| Q11 | Picker contributions across extensions? | (1) Only into pickers that opt in (`open-to-sources`). (2) Into any picker. (3) None in 0.3. | **(1).** Contributors see the query of dynamic pickers, and the owner never sees their items. A "Search everywhere" picker made only of contributions is not in 0.3. | Technical (API shape) |
| Q12 | Default key bindings in the manifest? | (1) Not in 0.3. (2) Suggested keys with conflict rules. | **(1).** Users bind commands themselves. Suggested keys would need conflict rules against the user's and upstream's keymaps. Additive later. | Product (small) |
| Q13 | Image budget | — | **Decided**; see above. | — |
| Q14 | Canvas paint strategy? | (1) Paint the draw list each frame, and cache a texture only if the bench misses. (2) A texture per update by default. | **(1).** It switches to a texture cached per update only if the 16384-op paint bench misses 4 ms. A texture costs width × height × 4 × scale² per canvas, up to 256 MiB at 4096 × 4096 on a 2× screen. It does not change the WIT. | Technical (renderer only, no ABI) |

Under (a), the owner answers **7 product questions**: Q1, Q2, Q3, Q5, Q8, Q10 and Q12. Q4, Q6, Q7,
Q11 and Q14 are technical.

## (b) Smaller 0.3: images, canvas, T2 imports, notifications

The recommended answers and reasons are those of (a) unless the row says otherwise.

| Q | Under (b) | Recommended | Kind |
|---|---|---|---|
| Q1 | Stays: the T2 imports ship. | (1), as in (a) | Product |
| Q2 | Stays: `process:spawn` ships. | (3), as in (a) | Product |
| Q3 | **Vanishes.** Document context goes to 0.4 with the providers, its only users. In 0.4 it is added with them, so no request record needs duplicating. | — | — |
| Q4 | Stays: `network:fetch` ships. | (2), as in (a) | Technical |
| Q5 | Stays: `workspace:edit` ships. | (2), as in (a) | Product |
| Q6 | **Vanishes.** Every hook serves a moved feature: extension highlights, inlays and gutter signs (§1), code lens (§3), the palette (§5). Images, canvas and the T2 imports need no new upstream hook. | — | — |
| Q7 | Stays, with a weaker reason: only the 16 MiB fetch response needs the room, since virtual documents move. | (3), as in (a), already in the code | Technical |
| Q8 | Stays: `agent:observe` ships. | (1), as in (a) | Product |
| Q9 | **Decided.** | — | — |
| Q10 | Stays: T2 consent ships. | (1), as in (a) | Product |
| Q11 | **Vanishes.** The picker goes to 0.4. | — | — |
| Q12 | **Vanishes.** Commands go to 0.4. | — | — |
| Q13 | **Decided.** | — | — |
| Q14 | Stays: canvas ships. | (1), as in (a) | Technical |

Under (b), the owner answers **5 product questions**: Q1, Q2, Q5, Q8 and Q10. Q4, Q7 and Q14 are
technical.

**What else (b) changes** (from integration-5, A3):
- **Remove the moved interfaces from the staged 0.3 WIT:** `decorations` (0.3 extras),
  `documents`, `language`, `virtual-documents`, `picker`, and their exports. Today they are
  declared but not linked, and they trap when called.
- **Keep 0.2's `place-decoration`.** A 0.3 component should not lose it.
- **What becomes 0.4:** plan 4.2–4.4 and design §1–§6, together with Q3, Q6, Q11 and Q12.
- **Update spec decision 28** to list what 0.3 ships.
