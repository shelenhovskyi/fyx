import { test } from "node:test";
import assert from "node:assert/strict";
import { convertSnippetFile, toZedBody, zedScopeFor, zedSnippetParseError } from "../src/snippets.ts";

test("contributed snippets land in the language's Zed scope", () => {
  const out = convertSnippetFile(
    { log: { prefix: "log", body: ["console.log($1);", "$0"], description: "Log" } },
    "typescriptreact",
  );
  assert.deepEqual([...out.keys()], ["tsx"]);
  assert.deepEqual(out.get("tsx")!.log, { prefix: "log", body: ["console.log($1);", "$0"], description: "Log" });
});

test(".code-snippets scopes are split per language; no scope is global", () => {
  const out = convertSnippetFile(
    {
      a: { prefix: "a", body: "a", scope: "javascript,typescript" },
      b: { prefix: "b", body: "b" },
      c: { prefix: "c", body: "c", scope: "shellscript", isFileTemplate: true },
    },
    undefined,
  );
  assert.deepEqual([...out.keys()].sort(), ["javascript", "shell script", "snippets", "typescript"]);
  assert.equal(out.get("javascript")!.a.body, "a");
  assert.equal("scope" in out.get("javascript")!.a, false, "VS Code-only fields are dropped");
  assert.ok(out.get("snippets")!.b);
});

test("language ids map to Zed's snippet_scope_id", () => {
  const warnings: string[] = [];
  assert.equal(zedScopeFor("cpp"), "c++");
  assert.equal(zedScopeFor("shellscript"), "shell script");
  assert.equal(zedScopeFor("javascriptreact"), "javascript");
  assert.equal(zedScopeFor("Haskell", warnings), "haskell");
  assert.match(warnings[0], /not built into Zed/);
});

test("variables and transforms become bodies Zed can parse", () => {
  const cases: [string, string][] = [
    ["const ${1:name} = require('$1');$0", "const ${1:name} = require('$1');$0"],
    ["// ${TM_FILENAME}\n$1", "// ${2:TM_FILENAME}\n$1"],
    ["$CURRENT_YEAR-$CURRENT_YEAR $1", "${2:CURRENT_YEAR}-${2:CURRENT_YEAR} $1"],
    ["${TM_SELECTED_TEXT:default} $0", "${1:default} $0"],
    ["${1|one,two|} x", "${1|one,two|} x"],
    ["${1/(.*)/${1:/upcase}/} done", "$1 done"],
    ["${TM_FILENAME/(.*)\\..+$/$1/}", "${2:TM_FILENAME}"],
    ["price: $ 5 and \\$x", "price: \\$ 5 and \\$x"],
    ["${1:outer ${2:inner}}", "${1:outer ${2:inner}}"],
  ];
  for (const [input, expected] of cases) {
    const { body } = toZedBody(input);
    assert.equal(body, expected, input);
    assert.equal(zedSnippetParseError(body), undefined, `Zed must accept ${body}`);
  }
});

test("zedSnippetParseError mirrors Zed's parser", () => {
  assert.equal(zedSnippetParseError("${1:x} $0"), undefined);
  assert.match(zedSnippetParseError("$TM_FILENAME") ?? "", /integer/);
  assert.match(zedSnippetParseError("${1:x") ?? "", /closing brace/);
  assert.match(zedSnippetParseError("${1|a,b}") ?? "", /pipe/);
});
