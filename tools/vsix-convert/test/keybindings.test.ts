import { test } from "node:test";
import assert from "node:assert/strict";
import { convertKey, convertKeybindings, convertWhen, SIDECAR_COMMAND_ACTION } from "../src/keybindings.ts";

test("key syntax", () => {
  assert.equal(convertKey("ctrl+shift+p", "linux"), "ctrl-shift-p");
  assert.equal(convertKey("Shift+Ctrl+K Ctrl+C", "linux"), "shift-ctrl-k ctrl-c", "modifier order is kept; Zed accepts any order");
  assert.equal(convertKey("cmd+k cmd+s", "macos"), "cmd-k cmd-s");
  assert.equal(convertKey("cmd+k", "windows"), "win-k");
  assert.equal(convertKey("alt+f12", "linux"), "alt-f12");
  assert.equal(convertKey("ctrl+-", "linux"), "ctrl--");
  assert.equal(convertKey("ctrl++", "linux"), "ctrl-+");
  assert.equal(convertKey("escape", "linux"), "escape");
  assert.deepEqual(convertKey("ctrl+numpad_add", "linux"), { error: 'key "numpad_add" has no Zed name' });
});

test("when clauses", () => {
  assert.equal(convertWhen("editorTextFocus && !editorReadonly").context, "Editor");
  assert.equal(convertWhen("terminalFocus || editorFocus").context, "Terminal || Editor");
  assert.equal(convertWhen("editorTextFocus && editorLangId == 'markdown'").context, "Editor && extension == md");
  assert.equal(convertWhen("editorTextFocus && editorLangId != python").context, "Editor && !extension == py");
  assert.equal(convertWhen("resourceExtname == .csv").context, "extension == csv");
  assert.equal(convertWhen("config.foo.enabled").context, undefined);
  assert.equal(convertWhen("editorFocus && (config.a || suggestWidgetVisible)").context, "Editor");
  assert.equal(
    convertWhen("editorFocus && (inSnippetMode || suggestWidgetVisible)").context,
    "Editor && (in_snippet || showing_completions)",
  );
  assert.match(convertWhen("config.foo.enabled").warnings[0], /broader/);
  assert.equal(convertWhen("view == todo-tree-view").context, undefined);
});

test("bindings: built-ins map to Zed actions, extension commands to the sidecar", () => {
  const { keymap, warnings } = convertKeybindings(
    [
      { key: "ctrl+shift+i", mac: "cmd+shift+i", command: "editor.action.formatDocument", when: "editorTextFocus" },
      { key: "ctrl+d", command: "editor.action.addSelectionToNextFindMatch" },
      { key: "f9", command: "sortLines.sortLines", when: "editorTextFocus" },
      { key: "ctrl+k ctrl+x", command: "ext.withArgs", args: { a: 1 } },
      { key: "ctrl+shift+k", command: "-editor.action.deleteLines" },
      { key: "ctrl+alt+x", command: "workbench.action.somethingUnknown" },
    ],
    { platform: "macos", extensionCommands: new Set(["sortLines.sortLines", "ext.withArgs"]) },
  );
  const editor = keymap.find((s) => s.context === "Editor")!;
  assert.equal(editor.bindings["cmd-shift-i"], "editor::Format");
  assert.deepEqual(editor.bindings["ctrl-d"], ["editor::SelectNext", { replace_newest: false }]);
  assert.deepEqual(editor.bindings["f9"], [SIDECAR_COMMAND_ACTION, { command: "sortLines.sortLines" }]);
  assert.equal(editor.bindings["ctrl-shift-k"], null, "removals unbind");
  const global = keymap.find((s) => s.context === undefined)!;
  assert.deepEqual(global.bindings["ctrl-k ctrl-x"], [SIDECAR_COMMAND_ACTION, { command: "ext.withArgs", args: { a: 1 } }]);
  assert.ok(warnings.some((w) => w.includes("workbench.action.somethingUnknown") && w.includes("skipped")));
});
