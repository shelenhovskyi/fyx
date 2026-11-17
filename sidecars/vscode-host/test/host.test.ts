// End to end against the in-process sidecar: the kitchen-sink fixture extension is loaded
// unmodified and driven only through JSON-RPC, as the Fyx host would.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { MockHost, tempDir } from "../src/mock-host.ts";
import { ErrorCodes, RpcError } from "../src/rpc.ts";
import { Uri } from "../src/shim/types.ts";
import type { Node } from "../src/schema.ts";

const fixture = join(import.meta.dirname, "fixtures", "kitchen-sink");
const ID = "fyx-test.kitchen-sink";
let host: MockHost;
let workspace: string;
let docUri: string;

before(async () => {
  workspace = tempDir("fyx-ws-");
  mkdirSync(join(workspace, "docs"));
  writeFileSync(join(workspace, "docs", "a.txt"), "x");
  ({ host } = MockHost.inProcess());
  const result = await host.initialize({
    extensionPath: fixture,
    workspaceFolders: [Uri.file(workspace).toString()],
    settings: { "sink.greeting": "Hey" },
    storageDir: join(workspace, ".storage"),
  });
  assert.equal(result.extension.id, ID);
  assert.equal(result.extension.displayName, "Kitchen Sink");
  docUri = Uri.file(join(workspace, "notes.txt")).toString();
});

after(async () => {
  await host.close();
  rmSync(workspace, { recursive: true, force: true });
});

const text = (nodes: Node[]) => nodes.flatMap((n) => ("text" in n.props ? [n.props.text.content] : "button" in n.props ? [n.props.button.label] : []));

test("declares status bar, output and contributed view surfaces", () => {
  assert.deepEqual([...host.surfaces.keys()].sort(), [`${ID}/output`, `${ID}/status-left`, `${ID}/status-right`, `${ID}/view/sinkTree`]);
  assert.deepEqual(host.surfaces.get(`${ID}/status-right`)!.kind, { "status-item": { side: "right", priority: 0 } });
  assert.deepEqual(host.surfaces.get(`${ID}/view/sinkTree`)!.kind, { panel: { position: "left", icon: "list-tree", "default-size": 280 } });
});

test("activates on onLanguage and publishes diagnostics", async () => {
  let cov = await host.coverage();
  assert.equal(cov.activated, false, "not activated before a plaintext document opens");
  await host.openDocument(docUri, "hello TODO   \nworld  ", "plaintext");
  await host.settle();
  cov = await host.coverage();
  assert.equal(cov.activated, true, cov.activationError);
  const diags = host.diagnostics.get(docUri) as { severity: string; message: string; range: unknown }[];
  assert.deepEqual(diags, [{ range: { start: { line: 0, character: 6 }, end: { line: 0, character: 10 } }, message: "TODO found", severity: "warning" }]);
  assert.ok(host.logs.some((l) => l.message.includes("activating")));
  assert.equal(host.contextKeys.get("sink.active"), true);
});

test("status bar item renders as a button and its click runs the command", async () => {
  const id = `${ID}/status-right`;
  const nodes = await host.mount(id);
  const button = nodes.find((n) => "button" in n.props)!;
  assert.deepEqual(button.props, { button: { label: "Sink", icon: "check", style: "subtle", disabled: false } });
  const update = await host.event(id, { click: button.id });
  assert.deepEqual(update, { patches: [{ "set-props": [button.id, { button: { label: "world", icon: "check", style: "subtle", disabled: false } }] }] });
  assert.deepEqual(host.schemaErrors, []);
});

test("output channel content arrives as push-update patches", async () => {
  const id = `${ID}/output`;
  await host.mount(id);
  assert.ok(text(host.trees.get(id)!).some((t) => t.includes("activated")));
  assert.equal(await host.executeCommand("sink.hello", "Ada"), "Hey, Ada", "configuration from initialize.settings");
  await host.settle();
  assert.ok(text(host.trees.get(id)!).some((t) => t.includes("Hey, Ada")), "retained tree was patched");
  assert.ok(host.traffic.some((t) => "method" in t.message && t.message.method === "push-update"));
  assert.deepEqual(host.schemaErrors, []);
});

test("after the host refuses a push-update, the sidecar resends the whole tree", async () => {
  const id = `${ID}/output`;
  await host.mount(id);
  const pushes = () =>
    host.traffic
      .map((t) => t.message as { method?: string; params?: { "surface-id": string; update: unknown } })
      .filter((m) => m.method === "push-update" && m.params?.["surface-id"] === id)
      .map((m) => m.params?.update);
  const before = pushes().length;
  host.refuseNextPush = id;
  await host.executeCommand("sink.hello", "Refused");
  await host.settle();
  const sent = pushes().slice(before);
  assert.equal(sent.length, 2, JSON.stringify(sent));
  assert.ok(typeof sent[0] === "object" && sent[0] !== null && "patches" in sent[0], "first the diff, which the host refuses");
  assert.ok(typeof sent[1] === "object" && sent[1] !== null && "replace" in sent[1], "then a full replace");
  assert.ok(text(host.trees.get(id)!).some((t) => t.includes("Hey, Refused")), "the host tree caught up");
  assert.ok(host.logs.some((l) => l.level === "warn" && l.message.includes("refused push-update")));

  await host.executeCommand("sink.hello", "Again");
  await host.settle();
  const next = pushes().slice(before + 2);
  assert.ok(next.length > 0 && next.every((u) => typeof u === "object" && u !== null && "patches" in u), "patches resume once the host has the tree");
  assert.deepEqual(host.schemaErrors, []);
});

test("tree view: rows, expansion and leaf commands", async () => {
  const id = `${ID}/view/sinkTree`;
  const nodes = await host.mount(id);
  const list = nodes.find((n) => "list" in n.props)!;
  assert.ok("list" in list.props && list.props.list["row-count"] === 3, "fruits, veg, nuts (sink.hello added nuts)");
  let rows = await host.listRows(id, list.id, 0, 10);
  assert.deepEqual(rows.map((r) => text(r.nodes)), [["fruits"], ["veg"], ["nuts", "leaf"]]);
  assert.deepEqual(rows[0].nodes[1].props, { icon: { name: "chevron-right", color: null } });

  await host.event(id, { "row-click": [list.id, 0] });
  const relisted = host.trees.get(id)!.find((n) => "list" in n.props)!;
  assert.notEqual(relisted.id, list.id, "the list node is re-keyed so the host drops cached rows");
  rows = await host.listRows(id, relisted.id, 0, 10);
  assert.deepEqual(rows.map((r) => text(r.nodes)[0]), ["fruits", "apple", "pear", "veg", "nuts"]);
  assert.equal((rows[1].nodes[0].props as { flex: { padding: number } }).flex.padding, 12, "depth shown as padding");

  await host.event(id, { "row-click": [relisted.id, 2] });
  await host.settle();
  assert.ok(text(host.trees.get(`${ID}/output`)!).some((t) => t.includes("Hey, pear")));
  assert.deepEqual(host.schemaErrors, []);
});

test("QuickPick and InputBox are transient surfaces driven by ui-events", async () => {
  assert.equal(await host.executeCommand("sink.pick"), "alpha");
  host.options.autoPick = 2;
  assert.equal(await host.executeCommand("sink.pick"), "gamma");
  host.options.messageChoice = 0;
  assert.deepEqual(await host.executeCommand("sink.ask"), { answer: "fyx", choice: "OK" });
  assert.deepEqual(host.messages.at(-1), { severity: "info", message: "Hi fyx", modal: false, items: ["OK", "Cancel"] });
  const added = host.traffic.filter((t) => "method" in t.message && t.message.method === "fyx/surfacesChanged");
  assert.ok(added.length >= 4, "added and removed");
  assert.deepEqual([...host.surfaces.keys()].filter((k) => k.includes("quick-pick") || k.includes("input-box")), [], "closed after accept");
  assert.deepEqual(host.schemaErrors, []);
});

test("editor commands, formatting and hover go through the host buffer", async () => {
  await host.setActiveEditor(docUri);
  await host.executeCommand("sink.upper");
  assert.equal(host.buffers.get(docUri)!.text, "HELLO TODO   \nworld  ");
  assert.deepEqual(host.selections.get(docUri), [{ anchor: { line: 0, character: 0 }, active: { line: 0, character: 5 } }], "selection assignment reaches the host");
  const edits = await host.format(docUri);
  assert.deepEqual(edits, [
    { range: { start: { line: 0, character: 10 }, end: { line: 0, character: 13 } }, newText: "" },
    { range: { start: { line: 1, character: 5 }, end: { line: 1, character: 7 } }, newText: "" },
  ]);
  const hover = await host.rpc.request("vscode/provideHover", { uri: docUri, position: { line: 1, character: 2 } });
  assert.deepEqual(hover, { contents: ["**world**"], range: { start: { line: 1, character: 0 }, end: { line: 1, character: 5 } } });
  const decorated = [...host.decorations.values()][0];
  assert.deepEqual(decorated, [{ range: { start: { line: 0, character: 6 }, end: { line: 0, character: 10 } } }], "null hoverMessage is dropped");
  const style = [...host.decorationTypes.values()][0] as Record<string, unknown>;
  assert.deepEqual(style, { backgroundColor: "warning" }, "ThemeColor kept as token, raw #ff0000 dropped");
});

test("workspace.fs and findFiles use the real file system", async () => {
  const r = (await host.executeCommand("sink.fs")) as { back: string; found: string[]; isFile: boolean };
  assert.equal(r.back, "written by sink");
  assert.deepEqual(r.found, ["docs/a.txt", "sub/note.txt"]);
  assert.equal(r.isFile, true);
});

test("unimplemented APIs fail with an explicit error naming the API", async () => {
  await assert.rejects(host.executeCommand("sink.webview"), (e: RpcError) => {
    assert.equal(e.code, ErrorCodes.NotImplemented);
    assert.equal(e.message, "vscode.window.createWebviewPanel is not implemented in Fyx");
    assert.deepEqual(e.data, { api: "vscode.window.createWebviewPanel" });
    return true;
  });
  const cov = await host.coverage();
  assert.equal(cov.notImplemented["vscode.window.createWebviewPanel"], 1);
  assert.ok(cov.used["vscode.window.createStatusBarItem"] >= 1);
  assert.ok(cov.used["vscode.languages.createDiagnosticCollection"] >= 1);
  assert.match(cov.partial["vscode.window.createTextEditorDecorationType"], /color=#ff0000/);
});
