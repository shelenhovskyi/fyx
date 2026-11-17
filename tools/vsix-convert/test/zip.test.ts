import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { readZip, unsafePathReason, writeZip } from "../src/zip.ts";

const here = join(import.meta.dirname, "..", "src", "zip.ts");
const twin = join(import.meta.dirname, "..", "..", "..", "sidecars", "vscode-host", "src", "zip.ts");

test("zip.ts is byte-identical to the sidecar's copy", () => {
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

/** Rewrites the uncompressed size the central directory declares for every entry. */
function declareSize(zip: Buffer, size: number): Buffer {
  const out = Buffer.from(zip);
  for (let i = 0; i + 4 <= out.length; i++) if (out.readUInt32LE(i) === 0x02014b50) out.writeUInt32LE(size, i + 24);
  return out;
}

test("stored and deflated entries round-trip", () => {
  const text = "hello ".repeat(1000);
  for (const deflate of [false, true]) {
    const entries = readZip(writeZip({ "a.txt": text, "empty.txt": "" }, { deflate }));
    assert.equal(entries.get("a.txt")?.size, text.length);
    assert.equal(entries.get("a.txt")?.read().toString(), text);
    assert.equal(entries.get("empty.txt")?.read().length, 0);
  }
});

test("declared sizes above the entry or total cap are refused before inflating", () => {
  const two = writeZip({ "a.bin": Buffer.alloc(600), "b.bin": Buffer.alloc(600) }, { deflate: true });
  assert.throws(() => readZip(two, { maxEntryBytes: 500 }), /a\.bin is 600 bytes uncompressed, above the 500-byte entry limit/);
  assert.throws(() => readZip(two, { maxTotalBytes: 1000 }), /above the 1000-byte uncompressed limit/);
  assert.equal(readZip(two, { maxEntryBytes: 600, maxTotalBytes: 1200 }).size, 2);
  // A bomb that declares its real size: refused by the default caps without inflating anything.
  assert.throws(() => readZip(declareSize(writeZip({ "bomb.bin": "x" }, { deflate: true }), 300 * 1024 * 1024)), /above the \d+-byte entry limit/);
});

test("an entry that inflates beyond its declared size stops at the declared size", () => {
  // 64 MiB of zeros deflates to ~64 KiB; the archive claims 100 bytes.
  const bomb = declareSize(writeZip({ "bomb.bin": Buffer.alloc(64 * 1024 * 1024) }, { deflate: true }), 100);
  const entries = readZip(bomb);
  assert.equal(entries.get("bomb.bin")?.size, 100, "listing the archive inflates nothing");
  assert.throws(() => entries.get("bomb.bin")?.read(), /does not inflate to its declared 100 bytes/);
  const short = declareSize(writeZip({ "a.txt": "abc" }, { deflate: true }), 10);
  assert.throws(() => readZip(short).get("a.txt")?.read(), /inflates to 3 bytes, not the declared 10/);
});

test("corrupt archives are rejected", () => {
  const zip = writeZip({ "a.txt": "abc" });
  const badCrc = Buffer.from(zip);
  badCrc[30 + "a.txt".length] ^= 0xff;
  assert.throws(() => readZip(badCrc).get("a.txt")?.read(), /CRC/);
  const encrypted = Buffer.from(zip);
  for (let i = 0; i + 4 <= encrypted.length; i++) if (encrypted.readUInt32LE(i) === 0x02014b50) encrypted.writeUInt16LE(1, i + 8);
  assert.throws(() => readZip(encrypted), /encrypted/);
  const truncated = Buffer.concat([zip.subarray(0, 10), zip.subarray(zip.length - 22)]);
  assert.throws(() => readZip(truncated), /truncated|bad central directory|bad local header/);
  const twice = writeZip({ "a.txt": "1", "b.txt": "2" });
  const nameAt = twice.lastIndexOf("b.txt");
  twice.write("a", nameAt);
  assert.throws(() => readZip(twice), /duplicate entry a\.txt/);
});
