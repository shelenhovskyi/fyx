// APIs that used to be silent no-ops: each now works through the protocol or throws
// NotImplementedInFyx. Driven through JSON-RPC against a small extension written per run.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { MockHost, tempDir } from "../src/mock-host.ts";
import { Uri } from "../src/shim/types.ts";
import type { Node } from "../src/schema.ts";

const EXTENSION = String.raw`
const vscode = require("vscode");
const events = [];
const fruit = { name: "fruit", children: ["apple", "pear"] };
const veg = { name: "veg", children: ["kale"] };
const leaves = new Map([...fruit.children.map((c) => [c, fruit]), ...veg.children.map((c) => [c, veg])]);
const provider = {
  getChildren: (e) => (e === undefined ? [fruit, veg] : typeof e === "object" ? e.children : []),
  getTreeItem: (e) => (typeof e === "object" ? { label: e.name, collapsibleState: 1 } : { label: e }),
  getParent: (e) => (typeof e === "object" ? undefined : leaves.get(e)),
};
function activate(context) {
  const view = vscode.window.createTreeView("gapsTree", { treeDataProvider: provider });
  context.subscriptions.push(
    vscode.languages.registerHoverProvider("plaintext", {
      provideHover(doc, pos) {
        const range = doc.getWordRangeAtPosition(pos);
        return range ? new vscode.Hover(doc.getText(range)) : undefined;
      },
    }),
  );
  const reg = (id, fn) => context.subscriptions.push(vscode.commands.registerCommand(id, fn));
  reg("gaps.watch", () => {
    const all = vscode.workspace.createFileSystemWatcher("**/*.json");
    const rel = vscode.workspace.createFileSystemWatcher(new vscode.RelativePattern(vscode.workspace.workspaceFolders[0], "src/*.ts"), false, true, false);
    all.onDidCreate((u) => events.push("all created " + vscode.workspace.asRelativePath(u)));
    all.onDidChange((u) => events.push("all changed " + vscode.workspace.asRelativePath(u)));
    all.onDidDelete((u) => events.push("all deleted " + vscode.workspace.asRelativePath(u)));
    rel.onDidCreate((u) => events.push("rel created " + vscode.workspace.asRelativePath(u)));
    rel.onDidChange((u) => events.push("rel changed " + vscode.workspace.asRelativePath(u)));
    context.subscriptions.push(all, rel);
  });
  reg("gaps.events", () => events.splice(0));
  reg("gaps.progress", () =>
    vscode.window.withProgress({ location: vscode.ProgressLocation.Notification, title: "Long", cancellable: true }, (_progress, token) =>
      new Promise((resolve) => token.onCancellationRequested(() => resolve("cancelled"))),
    ),
  );
  reg("gaps.lang", () => {
    const d = vscode.languages.setLanguageConfiguration("plaintext", { wordPattern: /[a-z]+(?:-[a-z]+)*/, comments: { lineComment: "#" } });
    context.subscriptions.push(d);
    return null;
  });
  reg("gaps.unlang", () => context.subscriptions.at(-1).dispose());
  reg("gaps.treeChrome", () => {
    view.title = "Produce";
    view.description = "2 kinds";
    view.badge = { value: 3, tooltip: "new" };
  });
  reg("gaps.reveal", () => view.reveal("pear", { select: true }).then(() => view.selection));
  reg("gaps.checkbox", () => view.onDidChangeCheckboxState(() => {}));
  reg("gaps.disposeTree", () => view.dispose());
  reg("gaps.pick", () => {
    const qp = vscode.window.createQuickPick();
    qp.items = [{ label: "one" }, { label: "two" }];
    qp.buttons = [{ iconPath: new vscode.ThemeIcon("refresh"), tooltip: "Refresh" }];
    qp.onDidTriggerButton((b) => events.push("button " + b.tooltip));
    qp.onDidAccept(() => events.push("accepted " + qp.selectedItems.map((i) => i.label).join(",")));
    qp.show();
    qp.title = "Pick";
    qp.busy = true;
    globalThis.gapsPick = qp;
  });
  reg("gaps.pickDone", () => {
    const qp = globalThis.gapsPick;
    qp.busy = false;
    qp.enabled = true;
  });
  reg("gaps.pickDisable", () => void (globalThis.gapsPick.enabled = false));
  reg("gaps.itemButton", () => globalThis.gapsPick.onDidTriggerItemButton(() => {}));
}
module.exports = { activate };
`;

let host: MockHost;
let dir: string;
let ws: string;

before(async () => {
  dir = tempDir("fyx-gaps-");
  const ext = join(dir, "ext");
  ws = join(dir, "ws");
  mkdirSync(ext);
  mkdirSync(ws);
  writeFileSync(
    join(ext, "package.json"),
    JSON.stringify({
      name: "gaps",
      publisher: "fyx-test",
      version: "1.0.0",
      main: "./extension.js",
      activationEvents: ["*"],
      contributes: { views: { explorer: [{ id: "gapsTree", name: "Gaps" }] } },
    }),
  );
  writeFileSync(join(ext, "extension.js"), EXTENSION);
  ({ host } = MockHost.inProcess({ autoPick: false }));
  await host.initialize({ extensionPath: ext, workspaceFolders: [Uri.file(ws).toString()], storageDir: join(dir, "storage") });
  await host.rpc.request("initialized");
  await host.settle();
});

after(async () => {
  await host.close();
  rmSync(dir, { recursive: true, force: true });
});

const texts = (nodes: Node[]) => nodes.flatMap((n) => ("text" in n.props ? [n.props.text.content] : "button" in n.props ? [n.props.button.label] : []));
const file = (rel: string) => Uri.file(join(ws, ...rel.split("/"))).toString();

test("file system watchers fire for host file events, filtered by glob and ignore flags", async () => {
  await host.executeCommand("gaps.watch");
  await host.rpc.request("vscode/didChangeWatchedFiles", {
    changes: [
      { uri: file("a.json"), type: 1 },
      { uri: file("src/x.ts"), type: 1 },
      { uri: file("src/x.ts"), type: 2 },
      { uri: file("src/deep/y.ts"), type: 1 },
      { uri: file("cfg/b.json"), type: 2 },
      { uri: file("a.json"), type: 3 },
    ],
  });
  assert.deepEqual(await host.executeCommand("gaps.events"), [
    "all created a.json",
    "rel created src/x.ts",
    "all changed cfg/b.json",
    "all deleted a.json",
  ]);
  await assert.rejects(host.rpc.request("vscode/didChangeWatchedFiles", { changes: [{ uri: file("a.json"), type: 9 }] }), /unknown change type 9/);
});

test("withProgress cancellation reaches the task's token", async () => {
  const result = host.executeCommand("gaps.progress");
  await host.settle(5);
  const start = host.traffic
    .map((t) => t.message as { method?: string; params?: { id: number; phase: string; cancellable?: boolean } })
    .find((m) => m.method === "vscode/progress" && m.params?.phase === "start");
  assert.equal(start?.params?.cancellable, true);
  await host.rpc.request("vscode/cancelProgress", { id: start!.params!.id });
  assert.equal(await result, "cancelled");
});

test("setLanguageConfiguration reaches the host and drives the word pattern", async () => {
  const uri = file("words.txt");
  await host.openDocument(uri, "a well-known word", "plaintext");
  const word = async () => ((await host.rpc.request("vscode/provideHover", { uri, position: { line: 0, character: 4 } })) as { contents: string[] }).contents[0];
  assert.equal(await word(), "well");
  await host.executeCommand("gaps.lang");
  assert.deepEqual(host.languageConfigurations.get("plaintext"), { wordPattern: { regexp: "[a-z]+(?:-[a-z]+)*", flags: "" }, comments: { lineComment: "#" } });
  assert.equal(await word(), "well-known", "getWordRangeAtPosition uses the language's wordPattern");
  await host.executeCommand("gaps.unlang");
  assert.equal(host.languageConfigurations.get("plaintext"), null, "disposing tells the host");
  assert.equal(await word(), "well");
});

test("tree view: title, description and badge render; reveal expands; dispose clears the provider", async () => {
  const id = "fyx-test.gaps/view/gapsTree";
  await host.mount(id);
  await host.executeCommand("gaps.treeChrome");
  await host.settle();
  assert.deepEqual(texts(host.trees.get(id)!), ["Produce", "2 kinds", "3 (new)"]);

  assert.deepEqual(await host.executeCommand("gaps.reveal"), ["pear"]);
  await host.settle();
  const list = host.trees.get(id)!.find((n) => "list" in n.props)!;
  const rows = await host.listRows(id, list.id, 0, 10);
  assert.deepEqual(rows.map((r) => texts(r.nodes)[0]), ["fruit", "apple", "pear", "veg"], "the parent of the revealed element is expanded");

  await assert.rejects(host.executeCommand("gaps.checkbox"), /vscode\.TreeView\.onDidChangeCheckboxState is not implemented in Fyx/);

  await host.executeCommand("gaps.disposeTree");
  await host.settle();
  assert.ok(texts(host.trees.get(id)!).some((t) => t.startsWith("No data provider registered")));
  assert.deepEqual(host.schemaErrors, []);
});

test("QuickPick: buttons fire, title and busy re-render after show, enabled gates accept", async () => {
  await host.executeCommand("gaps.pick");
  await host.settle();
  const id = [...host.surfaces.keys()].find((k) => k.includes("quick-pick"))!;
  await host.mount(id);
  assert.deepEqual(texts(host.trees.get(id)!), ["Pick", "Loading…", "Refresh"]);
  await host.executeCommand("gaps.pickDone");
  await host.settle();
  assert.deepEqual(texts(host.trees.get(id)!), ["Pick", "Refresh"]);

  const button = host.trees.get(id)!.find((n) => "button" in n.props)!;
  await host.event(id, { click: button.id });
  const input = host.trees.get(id)!.find((n) => "input" in n.props)!;
  await host.executeCommand("gaps.pickDisable");
  await host.event(id, { "input-submitted": [input.id, ""] });
  assert.deepEqual(await host.executeCommand("gaps.events"), ["button Refresh"], "no accept while disabled");
  await host.executeCommand("gaps.pickDone");
  await host.event(id, { "input-submitted": [input.id, ""] });
  assert.deepEqual(await host.executeCommand("gaps.events"), ["accepted one"]);

  await assert.rejects(host.executeCommand("gaps.itemButton"), /vscode\.QuickPick\.onDidTriggerItemButton is not implemented in Fyx/);
  assert.deepEqual(host.schemaErrors, []);
});
