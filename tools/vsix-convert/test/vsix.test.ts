import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, writeFileSync, rmSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { writeZip } from "../src/zip.ts";
import { convertExtension, listFiles, openExtension, outputFiles, writeOutput, type ConversionResult } from "../src/vsix.ts";
import { zedSnippetParseError, type ZedSnippet } from "../src/snippets.ts";

function fixtureVsix(): Buffer {
  const pkg = {
    name: "fixture-pack",
    publisher: "fyx-test",
    displayName: "%displayName%",
    version: "1.2.3",
    description: "Fixture",
    contributes: {
      themes: [
        { label: "Fixture Light", uiTheme: "vs", path: "./themes/light.json" },
        { label: "Fixture Dark+", uiTheme: "vs-dark", path: "./themes/dark-plus.json" },
      ],
      snippets: [{ language: "javascript", path: "./snippets/js.code-snippets" }],
      commands: [{ command: "fixture.hello", title: "Hello" }],
      keybindings: [{ key: "ctrl+alt+h", command: "fixture.hello" }],
    },
  };
  return writeZip({
    "extension.vsixmanifest": "<xml/>",
    "extension/package.json": JSON.stringify(pkg),
    "extension/package.nls.json": JSON.stringify({ displayName: "Fixture Pack" }),
    "extension/themes/light.json": `{
      // comments and trailing commas, as VS Code allows
      "colors": { "editor.background": "#ffffff", },
      "tokenColors": [{ "scope": "comment", "settings": { "foreground": "#008000" } },],
    }`,
    "extension/themes/base.json": JSON.stringify({
      colors: { "editor.background": "#000000", "panel.border": "#333333" },
      tokenColors: [{ scope: "string", settings: { foreground: "#ff0000" } }],
    }),
    "extension/themes/dark-plus.json": JSON.stringify({
      include: "./base.json",
      colors: { "editor.background": "#1e1e1e" },
      tokenColors: "./tokens.json",
    }),
    "extension/themes/tokens.json": JSON.stringify({
      tokenColors: [{ scope: "comment", settings: { foreground: "#6a9955" } }],
    }),
    "extension/snippets/js.code-snippets": JSON.stringify({
      header: { prefix: "hdr", body: ["// ${TM_FILENAME}", "$0"] },
    }),
  });
}

test("converts a .vsix end to end", () => {
  const dir = mkdtempSync(join(tmpdir(), "vsix-convert-"));
  try {
    const vsix = join(dir, "fixture.vsix");
    writeFileSync(vsix, fixtureVsix());
    const result = convertExtension(openExtension(vsix), { platform: "linux" });
    assert.equal(result.id, "fyx-test.fixture-pack");
    const family = result.themeFamily!;
    assert.equal(family.name, "Fixture Pack", "package.nls.json placeholders are resolved");
    assert.deepEqual(
      family.themes.map((t) => [t.name, t.appearance]),
      [
        ["Fixture Light", "light"],
        ["Fixture Dark+", "dark"],
      ],
    );
    const dark = family.themes[1].style;
    assert.equal(dark["editor.background"], "#1e1e1eff", "child overrides included base");
    assert.equal(dark["border"], "#333333ff", "base colours are inherited");
    const syntax = dark.syntax as Record<string, { color: string }>;
    assert.equal(syntax.string.color, "#ff0000ff", "base token colours are inherited");
    assert.equal(syntax.comment.color, "#6a9955ff", "string tokenColors path is loaded");

    const snippet = result.snippets.get("javascript")!.header;
    assert.deepEqual(snippet.body, ["// ${1:TM_FILENAME}", "$0"]);
    assert.equal(zedSnippetParseError((snippet.body as string[]).join("\n")), undefined);
    assert.deepEqual(result.keymap, [
      { bindings: { "ctrl-alt-h": ["fyx_vscode::RunCommand", { command: "fixture.hello" }] } },
    ]);

    const files = outputFiles(result);
    assert.deepEqual([...files.keys()].sort(), [
      "CONVERSION-WARNINGS.txt",
      "extension.toml",
      "keymap.json",
      "snippets/javascript.json",
      "themes/fyx-test-fixture-pack.json",
    ]);
    assert.match(files.get("extension.toml")!, /snippets = \["snippets\/javascript.json"\]/);
    assert.match(files.get("extension.toml")!, /schema_version = 1/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("cli writes the converted extension", () => {
  const dir = mkdtempSync(join(tmpdir(), "vsix-convert-cli-"));
  try {
    const vsix = join(dir, "fixture.vsix");
    writeFileSync(vsix, fixtureVsix());
    const cli = join(import.meta.dirname, "..", "src", "cli.ts");
    const run = spawnSync(process.execPath, [cli, vsix, "-o", join(dir, "out")], { encoding: "utf8" });
    assert.equal(run.status, 0, run.stderr);
    assert.match(run.stdout, /fyx-test.fixture-pack: 2 theme\(s\), 1 snippet\(s\) in 1 scope\(s\), 1 keybinding\(s\)/);
    const theme = JSON.parse(
      readFileSync(join(dir, "out", "fyx-test.fixture-pack", "themes", "fyx-test-fixture-pack.json"), "utf8"),
    );
    assert.equal(theme.$schema, "https://zed.dev/schema/themes/v0.2.0.json");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

function vsixWith(pkg: Record<string, unknown>, extra: Record<string, string> = {}): Buffer {
  return writeZip({ "extension/package.json": JSON.stringify(pkg), ...extra });
}

test("a crafted publisher or name cannot steer the output folder", () => {
  const dir = mkdtempSync(join(tmpdir(), "vsix-convert-escape-"));
  try {
    const keybindings = [{ key: "ctrl+alt+h", command: "workbench.action.files.save" }];
    const cases: Record<string, unknown>[] = [
      { publisher: "../../../escaped", name: "x" },
      { publisher: "p", name: "../escaped" },
      { publisher: "..", name: "x" },
      { publisher: "p", name: ".." },
      { publisher: "C:", name: "x" },
      { publisher: "p\\..\\..", name: "x" },
      { publisher: "/abs", name: "x" },
      { publisher: "p", name: 7 },
    ];
    for (const [k, fields] of cases.entries()) {
      const vsix = join(dir, `crafted-${k}.vsix`);
      writeFileSync(vsix, vsixWith({ ...fields, contributes: { keybindings } }));
      assert.throws(() => convertExtension(openExtension(vsix)), /is not a valid extension name/, JSON.stringify(fields));
      const cli = join(import.meta.dirname, "..", "src", "cli.ts");
      const out = join(dir, "nested", "out");
      const run = spawnSync(process.execPath, [cli, vsix, "-o", out], { encoding: "utf8" });
      assert.equal(run.status, 1, run.stdout);
      assert.match(run.stderr, /is not a valid extension name/);
    }
    assert.deepEqual(listFiles(dir).filter((f) => !f.endsWith(".vsix")), [], "nothing was written");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("writeOutput rejects ids and paths that leave the output folder", () => {
  const dir = mkdtempSync(join(tmpdir(), "vsix-convert-write-"));
  try {
    const out = join(dir, "out");
    const base: ConversionResult = { id: "p.x", manifest: { name: "x" }, snippets: new Map(), warnings: [] };
    for (const id of ["..", "../x", "a/b", "a\\b", "C:x", "C:\\x", "/abs", "", "."]) {
      assert.throws(() => writeOutput(out, { ...base, id }, new Map([["keymap.json", "[]"]])), /not a safe folder name/, id);
    }
    for (const path of ["../keymap.json", "a/../../x", "/etc/x", "C:/x", "D:x", "themes\\..\\..\\x", "themes/a:b"]) {
      assert.throws(
        () => writeOutput(out, base, new Map([["keymap.json", "[]"], [path, "evil"]])),
        /outside the output folder/,
        path,
      );
    }
    assert.equal(existsSync(out), false, "a rejected file aborts before anything is written");
    assert.equal(writeOutput(out, base, new Map([["themes/t.json", "{}"]])), join(out, "p.x"));
    assert.deepEqual(listFiles(out), ["p.x/themes/t.json"]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("an unsafe snippet scope is skipped with a warning", () => {
  const files = outputFiles({
    id: "p.x",
    manifest: { name: "x" },
    snippets: new Map<string, Record<string, ZedSnippet>>([
      ["../../evil", { a: { body: "a" } }],
      ["c#", { b: { body: "b" } }],
    ]),
    warnings: [],
  });
  assert.deepEqual([...files.keys()].sort(), ["CONVERSION-WARNINGS.txt", "extension.toml", "snippets/c#.json"]);
  assert.match(files.get("CONVERSION-WARNINGS.txt")!, /snippets scope "..\/..\/evil" skipped/);
});

test("an extension folder cannot read files outside itself", () => {
  const dir = mkdtempSync(join(tmpdir(), "vsix-convert-folder-"));
  try {
    writeFileSync(join(dir, "secret.json"), JSON.stringify({ colors: { "editor.background": "#123456" } }));
    mkdirSync(join(dir, "ext"));
    writeFileSync(
      join(dir, "ext", "package.json"),
      JSON.stringify({ publisher: "p", name: "x", contributes: { themes: [{ label: "T", path: "../secret.json" }] } }),
    );
    const files = openExtension(join(dir, "ext"));
    assert.equal(files.read("../secret.json"), undefined);
    const result = convertExtension(files);
    assert.equal(result.themeFamily, undefined);
    assert.match(result.warnings.join("\n"), /file not found: \.\.\/secret\.json/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
