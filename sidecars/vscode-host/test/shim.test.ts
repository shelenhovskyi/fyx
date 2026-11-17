import { test } from "node:test";
import assert from "node:assert/strict";
import { ConfigStore, flatten } from "../src/shim/config.ts";
import { DocumentStore } from "../src/shim/documents.ts";
import { globToRegExp, matchSelector } from "../src/shim/languages.ts";
import { DiagnosticSeverity, Position, Range, Selection, TextEdit, Uri } from "../src/shim/types.ts";
import { splitCodicons } from "../src/ui/surfaces.ts";
import { tokenForColor } from "../src/ui/theme.ts";

test("Uri parses, prints and joins like VS Code", () => {
  const u = Uri.parse("https://example.com/a%20b/c?q=1#frag");
  assert.deepEqual([u.scheme, u.authority, u.path, u.query, u.fragment], ["https", "example.com", "/a b/c", "q=1", "frag"]);
  assert.equal(u.toString(), "https://example.com/a%20b/c?q%3D1#frag");
  const f = Uri.file("/home/me/x.txt");
  assert.equal(f.toString(), "file:///home/me/x.txt");
  assert.equal(Uri.joinPath(f, "..", "y", "z.txt").path, "/home/me/y/z.txt");
  const win = Uri.file("C:\\Users\\me\\x.txt");
  assert.equal(win.path, "/c:/Users/me/x.txt");
  assert.equal(win.toString(), "file:///c%3A/Users/me/x.txt");
  assert.equal(Uri.parse(win.toString()).path, win.path, "round-trips through toString");
  assert.equal(win.with({ path: "/c:/other" }).path, "/c:/other");
});

test("Range and Position", () => {
  const r = new Range(2, 5, 1, 0);
  assert.deepEqual([r.start.line, r.end.line], [1, 2], "start/end are ordered");
  assert.ok(r.contains(new Position(1, 9)));
  assert.ok(!r.contains(new Position(3, 0)));
  assert.equal(new Range(0, 0, 0, 4).intersection(new Range(0, 2, 0, 8))?.end.character, 4);
  assert.ok(new Selection(1, 4, 1, 0).isReversed);
  assert.equal(DiagnosticSeverity[DiagnosticSeverity.Warning], "Warning", "numeric enums have reverse mapping");
});

test("TextDocument line and offset math, edits and change events", () => {
  const store = new DocumentStore();
  const changes: unknown[] = [];
  store.onDidChange.event((e) => changes.push(e.contentChanges));
  const doc = store.open(Uri.file("/w/a.txt"), "plaintext", 1, "one\r\ntwo\nthree");
  assert.equal(doc.lineCount, 3);
  assert.equal(doc.lineAt(0).text, "one");
  assert.equal(doc.lineAt(1).text, "two");
  assert.equal(doc.offsetAt(new Position(2, 2)), 11);
  assert.deepEqual(doc.positionAt(6).toJSON(), { line: 1, character: 1 });
  assert.equal(doc.getText(new Range(1, 0, 2, 3)), "two\nthr");
  assert.deepEqual(doc.getWordRangeAtPosition(new Position(2, 2))?.toJSON(), { start: { line: 2, character: 0 }, end: { line: 2, character: 5 } });
  store.applyConfirmed(doc, 1, [TextEdit.insert(new Position(0, 0), ">"), TextEdit.replace(new Range(2, 0, 2, 5), "3")]);
  assert.equal(doc.getText(), ">one\r\ntwo\n3");
  assert.equal(doc.version, 2);
  store.applyConfirmed(doc, 1, [TextEdit.insert(new Position(0, 0), "again")]);
  assert.equal(doc.getText(), ">one\r\ntwo\n3", "a host sync that arrived first already holds the confirmed edit");
  store.replace(doc.uri, 1, "stale");
  assert.equal(doc.getText(), ">one\r\ntwo\n3", "older host versions are ignored");
  store.replace(doc.uri, 5, "fresh");
  assert.equal(doc.getText(), "fresh");
  assert.equal(changes.length, 2);
});

test("configuration layers, sections and change events", () => {
  const config = new ConfigStore();
  config.addContributedDefaults({ properties: { "ext.enabled": { default: true }, "ext.colors.error": { default: "red" } } });
  config.setUserSettings({ ext: { colors: { error: "blue" } }, "files.exclude": { "**/out": true } }, false);
  const ext = config.get("ext");
  assert.equal(ext.get("enabled"), true);
  assert.equal(ext.get("colors.error"), "blue");
  assert.deepEqual(ext.get("colors"), { error: "blue" });
  assert.equal(ext.enabled, true, "values are also properties");
  assert.equal(config.get().get("editor.tabSize"), 4, "core defaults");
  assert.equal(config.get("files").get<Record<string, boolean>>("exclude")!["**/out"], true);
  assert.equal(ext.get("missing", 7), 7);
  let affected = false;
  config.onDidChange.event((e) => (affected = e.affectsConfiguration("ext.colors")));
  config.setUserSettings({ "ext.colors.error": "green" });
  assert.ok(affected);
  assert.deepEqual(flatten({ "[python]": { "editor.tabSize": 2 } }), { "[python]": { "editor.tabSize": 2 } });
});

test("globs and document selectors", () => {
  assert.ok(globToRegExp("**/*.{ts,js}").test("src/a/b.ts"));
  assert.ok(globToRegExp("**/*.{ts,js}").test("b.js"));
  assert.ok(!globToRegExp("**/*.{ts,js}").test("b.json"));
  assert.ok(globToRegExp("**/node_modules/**").test("a/node_modules/x/y.js"));
  assert.ok(globToRegExp("*.md").test("docs/readme.md"), "relative globs match at any depth, as in files.exclude");
  assert.ok(globToRegExp("src/[ab].txt").test("src/a.txt"));
  const store = new DocumentStore();
  const doc = store.open(Uri.file("/w/x.csv"), "csv", 1, "");
  assert.equal(matchSelector("csv", doc), 10);
  assert.equal(matchSelector("*", doc), 5);
  assert.equal(matchSelector({ scheme: "untitled" }, doc), 0);
  assert.equal(matchSelector([{ language: "plaintext" }, { pattern: "**/*.csv" }], doc), 10);
});

test("codicons and colours", () => {
  assert.deepEqual(splitCodicons("$(sync~spin) Syncing $(x)"), { icon: "sync", label: "Syncing" });
  assert.deepEqual(tokenForColor({ id: "statusBarItem.errorBackground" }), { token: "error" });
  assert.deepEqual(tokenForColor({ id: "editorWarning.foreground" }), { token: "warning" });
  assert.deepEqual(tokenForColor("rgba(255,0,0,0.3)"), { token: null, dropped: "rgba(255,0,0,0.3)" });
});
