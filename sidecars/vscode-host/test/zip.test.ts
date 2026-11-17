import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { readZip, unsafePathReason, writeZip } from "../src/zip.ts";

const here = join(import.meta.dirname, "..", "src", "zip.ts");
const twin = join(import.meta.dirname, "..", "..", "..", "tools", "vsix-convert", "src", "zip.ts");

test("zip.ts is byte-identical to the converter's copy", () => {
  assert.equal(readFileSync(here, "utf8"), readFileSync(twin, "utf8"), `${here} and ${twin} differ: fix both`);
});

test("unsafe entry names are rejected", () => {
  for (const name of [
    "extension/../../evil.js",
    "../evil.js",
    "/etc/passwd",
    "extension\\..\\evil.js",
    "..\\evil.js",
    "C:/Windows/evil.js",
    "C:evil.js",
    "extension/package.json:stream",
    "extension//x",
    "extension/./x",
    "extension/a\u0000b",
  ]) {
    assert.ok(unsafePathReason(name), `${JSON.stringify(name)} should be unsafe`);
    assert.throws(() => readZip(writeZip({ [name]: "x" })), /unsafe entry name/, name);
  }
  for (const name of ["extension/package.json", "extension/out/a..b.js", "extension/.vscodeignore"]) {
    assert.equal(unsafePathReason(name), undefined, name);
  }
});
