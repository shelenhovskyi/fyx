import { test } from "node:test";
import assert from "node:assert/strict";
import { applyUpdate, diff, node, props, validateNodes, type Node } from "../src/schema.ts";

const tree: Node[] = [
  node(1, null, props.flex({ direction: "row" })),
  node(2, 1, props.text("a")),
  node(3, 1, props.flex({ direction: "row", gap: 4 })),
  node(4, 3, props.icon("x")),
];

test("JSON mirror follows the WIT field and case names", () => {
  assert.deepEqual(props.flex(), { flex: { direction: "column", gap: 0, padding: 0, grow: false, align: "stretch", background: null } });
  assert.deepEqual(props.list(10, 22), { list: { "row-count": 10, "row-height": 22 } });
  assert.deepEqual(props.text("x", { color: "text-muted" }), { text: { content: "x", color: "text-muted", bold: false, monospace: false } });
  assert.deepEqual(JSON.parse(JSON.stringify(node(5, null, props.markdown("**m**")))), { id: 5, parent: null, props: { markdown: "**m**" } });
});

test("validateNodes enforces ids, parent order and prop types", () => {
  assert.deepEqual(validateNodes(tree), []);
  assert.match(validateNodes([node(1, 2, props.text("x")), node(2, null, props.text("y"))]).join(), /parent 2 does not precede/);
  assert.match(validateNodes([node(1, null, props.text("x")), node(1, null, props.text("y"))]).join(), /duplicate node id 1/);
  assert.match(validateNodes([node(1, null, { text: { content: "x", color: "red" as never, bold: false, monospace: false } })]).join(), /unknown color token/);
  assert.match(validateNodes([node(1, null, props.list(-1, 22))]).join(), /row-count/);
  assert.match(validateNodes([node(1, null, props.text("a")), node(2, null, props.text("b"))], { singleRoot: true }).join(), /exactly one root/);
  assert.match(validateNodes([node(1, null, props.button("b")), node(2, 1, props.text("x"))]).join(), /only flex nodes have children/);
});

test("diff produces patches that applyUpdate replays", () => {
  const next: Node[] = [
    node(1, null, props.flex({ direction: "row" })),
    node(5, 1, props.text("new first")),
    node(2, 1, props.text("a changed")),
    node(6, 1, props.text("appended")),
  ];
  const update = diff(tree, next);
  assert.ok(update !== "unchanged" && "patches" in update);
  assert.deepEqual(update.patches, [
    { remove: 3 },
    { insert: { node: next[1], before: 2 } },
    { "set-props": [2, props.text("a changed")] },
    { insert: { node: next[3], before: null } },
  ]);
  assert.deepEqual(applyUpdate(tree, update), next);
  assert.equal(diff(next, next), "unchanged");
});

test("diff falls back to replace when nodes move", () => {
  const swapped: Node[] = [tree[0], tree[2], tree[3], tree[1]];
  assert.deepEqual(diff(tree, swapped), { replace: swapped });
  const reparented: Node[] = [tree[0], tree[1], node(3, 2, tree[2].props)];
  assert.ok(typeof diff(tree, reparented) === "object" && "replace" in (diff(tree, reparented) as object));
});

test("applyUpdate rejects invalid patches", () => {
  assert.throws(() => applyUpdate(tree, { patches: [{ remove: 99 }] }), /no node 99/);
  assert.throws(() => applyUpdate(tree, { patches: [{ insert: { node: node(2, 1, props.text("dup")), before: null } }] }), /already exists/);
  assert.throws(() => applyUpdate(tree, { patches: [{ insert: { node: node(9, 1, props.text("x")), before: 4 } }] }), /not a sibling/);
  assert.deepEqual(applyUpdate(tree, { patches: [{ remove: 3 }] }).map((n) => n.id), [1, 2], "remove takes the subtree");
  assert.throws(() => applyUpdate(tree, { patches: Array.from({ length: 2049 }, () => ({ remove: 2 })) }), /host cap/);
});
