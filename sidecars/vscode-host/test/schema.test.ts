import { test } from "node:test";
import assert from "node:assert/strict";
import { applyUpdate, diff, MAX_PATCHES_PER_UPDATE, node, props, validateNodes, type Node, type Update } from "../src/schema.ts";
import { SurfaceRegistry, type SurfaceModel } from "../src/ui/surfaces.ts";

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

test("diff falls back to replace above the per-update patch cap", () => {
  const rows = (n: number, label: string): Node[] => [node(1, null, props.flex()), ...Array.from({ length: n }, (_, k) => node(k + 2, 1, props.text(`${label}${k}`)))];
  const atCap = diff(rows(MAX_PATCHES_PER_UPDATE, "a"), rows(MAX_PATCHES_PER_UPDATE, "b"));
  assert.ok(typeof atCap === "object" && "patches" in atCap && atCap.patches.length === MAX_PATCHES_PER_UPDATE);
  const next = rows(MAX_PATCHES_PER_UPDATE + 1, "b");
  assert.deepEqual(diff(rows(MAX_PATCHES_PER_UPDATE + 1, "a"), next), { replace: next });
});

test("surface registry: one push in flight, replace after a refusal, no retry loop", async () => {
  let label = "a";
  const model: SurfaceModel = {
    surface: { id: "s", title: "S", kind: "panel", "default-size": null } as never,
    render: () => [node(1, null, props.flex()), node(2, 1, props.text(label))],
    handleEvent: () => {},
  };
  const sent: Update[] = [];
  const answers: ((refuse: boolean) => void)[] = [];
  const registry = new SurfaceRegistry({
    pushUpdate: (_id, update) => {
      sent.push(update);
      return new Promise((resolve, reject) => answers.push((refuse) => (refuse ? reject(new Error("no")) : resolve(null))));
    },
    surfacesChanged: () => {},
  });
  registry.add(model);
  await registry.mount("s");
  const tick = () => new Promise((r) => setImmediate(r));

  label = "b";
  registry.invalidate("s");
  await tick();
  assert.equal(sent.length, 1);
  label = "c";
  registry.invalidate("s");
  await tick();
  assert.equal(sent.length, 1, "no second push while the first is unanswered");

  answers[0](true);
  await tick();
  await tick();
  assert.equal(sent.length, 2);
  assert.deepEqual(sent[1], { replace: model.render() }, "a refused diff is followed by the whole tree");

  answers[1](true);
  await tick();
  await tick();
  assert.equal(sent.length, 2, "a refused replace is not retried until the model changes");
  assert.ok(registry.needsResync("s"));

  label = "d";
  registry.invalidate("s");
  await tick();
  assert.deepEqual(sent[2], { replace: model.render() });
  answers[2](false);
  await tick();
  label = "e";
  registry.invalidate("s");
  await tick();
  assert.ok(typeof sent[3] === "object" && "patches" in sent[3], "accepted replace: patches again");
});
