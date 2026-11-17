# W2b progress: property tests for the `ui_ext` tree model

Branch `claude/fyx-w2b-proptest`, from `origin/integration`.

## State: done (2026-11-17). No bugs found in `ui_ext`.

- New file: `crates/ui_ext/tests/properties.rs`, with 16 proptest properties.
- `crates/ui_ext/Cargo.toml`: `proptest.workspace = true` under `[dev-dependencies]`. `Cargo.lock`: one line
  (`ui_ext` now depends on `proptest`).
- `src/` is unchanged. There are no `#[ignore]`d regressions, because no property failed.
- `README.md`: the two `> [!IMPORTANT]` review-reminder lines were prepended, as the root `CLAUDE.md` "HARD
  RULE" requires for any source change. Removing them is the human reviewer's step.
- `cargo test -p ui_ext`: 22 unit and 16 property tests pass, about 9 s in a debug build.
  `cargo clippy -p ui_ext --release --all-targets --all-features -- --deny warnings` (same flags as
  `script/clippy`) and `cargo fmt --check` are both clean.

## Properties and case counts

| Property | Cases | Checks |
|---|---|---|
| `tree_matches_reference_model` | 1024 (1–7 updates each, ~4,100 updates per run) | Applies random `unchanged`, `replace` and `patches` updates, with random limits, to `Tree` and to an independent reference model (one pre-order `Vec<Node>`). Checks that both give the same outcome (ok/err, failing patch index), the same touched set and the same `to_nodes()`. On error the tree equals the earlier clone. Empty touched ⇒ tree unchanged. Every id that appeared, disappeared or changed parent or props is touched. Structural invariants hold. `from_nodes(to_nodes(t)) == t` after every step. `len <= max_nodes`. |
| `flat_list_round_trips` | 512 | `from_nodes` accepts every valid parents-first list (shuffled ids, up to 48 nodes). `to_nodes` is parents-first and pre-order, keeping list order among siblings (equal to the model). `from_nodes(to_nodes(t)) == t`. |
| `invalid_node_lists_are_rejected` | 512 | Corrupts a valid list: duplicate id, unknown parent, self-parent, leaf parent, child moved before its parent. Both `from_nodes` and `replace` return the exact error, and `replace` leaves the old tree intact. |
| `failed_batch_leaves_tree_unchanged` | 512 | Puts one always-failing patch at a random index of a valid batch (unknown set-props or remove target, unknown parent, unknown `before`). The result is `Patch { index, exact error }` and the tree is unchanged. |
| `remove_drops_whole_subtree_and_frees_its_ids` | 512 | Removing a random node returns exactly its subtree as touched and drops all of it. No orphan survives. The other nodes keep their props, parent and child order. Removed ids can be inserted again, also later in the same batch. Live ids give `DuplicateId`. Removing the same node twice fails at index 1, and the batch rolls back. |
| `only_flex_nodes_have_children` | 512 | An insert under a non-flex node gives `NotAContainer` with the WIT kind name. Under a flex node it appends. `set-props` that turns a flex node with children into a leaf is rejected. The same change on a childless node is accepted. `set-props` to flex is always accepted. |
| `before_must_be_a_sibling_under_the_same_parent` | 512 | `before` that is a sibling under the same parent (or a root, for `parent: None`) inserts right before it. Anything else, including an unknown id, gives `UnknownSibling`, and the tree is unchanged. |
| `batches_above_the_patch_cap_are_rejected` | 512 | Any batch with `max_nodes_per_update + k` patches (cap 0–7) gives `UpdateTooLarge { count, max }` whatever it contains, and the tree is unchanged. A batch of exactly the cap is accepted. |
| `node_cap_holds_across_batches` | 512 (1–29 batches each) | With small caps, insert/remove batches never take the tree past `max_nodes`. The first insert at the cap fails with `Patch { index, TreeTooLarge }` and the batch rolls back. Inserts work again after removals. |
| `mount_replace_and_row_limits` | 512 | `from_nodes` and `replace` are capped by `max_nodes` only, even above the per-update cap (finding 11). `replace` returns the old ∪ new ids. `from_row` returns `UpdateTooLarge`, then `TreeTooLarge`, then `RowRootCount` (when the root count isn't 1). |
| JSON round-trips (6 properties) | 512 each | `serde_json` string and `Value` round-trips of every `types.rs` type: all props structs and enums, `Props`, `Node`, `InsertOp`, `Patch`, `Update`, `ListRow`, `UiEvent`, `PanelDescriptor`, `StatusItemDescriptor`, `SurfaceKind`, `Surface`, `LogLevel`. Values cover the full integer ranges and arbitrary Unicode strings, including control characters. |

The reference-model run covers a broad mix. Per run there are about 2,100 successful patch batches, 600
updates that touch nothing, 280 replaces and 500 `UpdateTooLarge` errors. Every `TreeError` variant
reachable from a patch appears, plus `TreeTooLarge` from patches and from replace. About half the batches
are valid by construction and the rest include mutated patches.

**Mutation check.** To make sure the suite is not vacuous, I injected each bug below into `tree.rs` by
hand, one at a time, and reverted it afterwards. Each one failed 2–4 properties: remove skips
grandchildren; `before` off by one; non-atomic batch; removed descendants missing from the touched set;
no container check in `set-props`; no leaf-parent check on insert; no duplicate check; children
reversed in `to_nodes`; `replace` capped by the per-update limit; node cap off by one; patch cap off by
one; removed node left in its parent's child list.

## Observations for W2 / W3 (not bugs; no change made)

- **"Empty touched iff unchanged" holds in one direction only.** `Ok(empty)` ⇒ unchanged is tested and
  holds. The converse does not, as designed. A `set-props` with equal props, or an insert and a remove of
  the same id in one batch, reports touched ids although the final tree is unchanged. This is harmless
  for row refresh (it only causes a re-request).
- **The touched set does not include the parent of an inserted or removed node,** although that parent's
  child list changed. That matches the doc comment. W3 must not rely on the touched set alone to decide
  which parents to re-render.
- **The host does not enforce the WIT's "an id must not be reused for a different node while the surface
  lives."** `Tree` frees an id as soon as its node is removed, even within the same batch (the test pins
  this behaviour). Only the SDK's id allocator prevents reuse. A non-SDK guest, or a JSON sidecar, can
  reuse ids and the host will accept it. To enforce the rule, the host would have to remember every id it
  has seen for each surface.
- **When an insert breaks several rules, the error the host reports depends on the order of its checks:**
  duplicate id, then node cap, then parent, container, sibling. The model test compares only the failing
  index. The targeted properties pin the exact error only where a single rule is broken.
- **JSON round-trips cannot detect renamed fields or variants,** because serde is symmetric. The existing
  unit test `json_form_matches_wit_names` pins the wire names.
