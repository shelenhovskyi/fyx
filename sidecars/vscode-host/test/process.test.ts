// The real transport: the sidecar as a child process speaking NDJSON on stdio, under Node's
// permission model, loading the fixture packed as a .vsix.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { MockHost, tempDir } from "../src/mock-host.ts";
import { Uri } from "../src/shim/types.ts";
import { writeZip } from "../src/zip.ts";

const fixture = join(import.meta.dirname, "fixtures", "kitchen-sink");

function packFixture(target: string): void {
  const files: Record<string, Buffer> = { "extension.vsixmanifest": Buffer.from("<PackageManifest/>") };
  const walk = (dir: string) => {
    for (const name of readdirSync(dir)) {
      const full = join(dir, name);
      if (statSync(full).isDirectory()) walk(full);
      else files[`extension/${relative(fixture, full).split(sep).join("/")}`] = readFileSync(full);
    }
  };
  walk(fixture);
  writeFileSync(target, writeZip(files));
}

test("sidecar process loads a .vsix under --permission and keeps stdout clean", async () => {
  const dir = tempDir("fyx-proc-");
  const host = MockHost.spawn({ permission: { read: [dir], write: [dir] } });
  try {
    const vsix = join(dir, "kitchen-sink.vsix");
    packFixture(vsix);
    const ws = join(dir, "ws");
    const result = await host.initialize({ extensionPath: vsix, workspaceFolders: [Uri.file(ws).toString()], storageDir: join(dir, "storage") });
    assert.equal(result.extension.id, "fyx-test.kitchen-sink");
    const uri = Uri.file(join(ws, "a.txt")).toString();
    await host.openDocument(uri, "TODO", "plaintext");
    await host.settle();
    assert.equal((await host.coverage()).activated, true, host.sidecarStderr);
    assert.equal(host.diagnostics.get(uri)?.length, 1);
    // The extension's console.log became a `log` notification instead of corrupting stdout.
    assert.ok(host.logs.some((l) => l.message === "kitchen-sink activated"));
    assert.equal(await host.executeCommand("sink.hello", "proc"), "Hello, proc");
    assert.deepEqual(host.schemaErrors, []);
  } finally {
    await host.close();
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the permission model blocks workspace.fs outside the granted folders", async () => {
  const dir = tempDir("fyx-perm-");
  const outside = tempDir("fyx-outside-");
  const host = MockHost.spawn({ permission: { read: [dir, fixture], write: [dir] } });
  try {
    await host.initialize({ extensionPath: fixture, workspaceFolders: [Uri.file(outside).toString()], storageDir: join(dir, "storage") });
    await host.openDocument(Uri.file(join(outside, "a.txt")).toString(), "x", "plaintext");
    await host.settle();
    // sink.fs writes into the (ungranted) workspace folder.
    await assert.rejects(host.executeCommand("sink.fs"), /NoPermissions|ERR_ACCESS_DENIED/);
  } finally {
    await host.close();
    rmSync(dir, { recursive: true, force: true });
    rmSync(outside, { recursive: true, force: true });
  }
});

test("probe mode records missing APIs without throwing", async () => {
  const dir = tempDir("fyx-probe-");
  const host = MockHost.spawn({ permission: { read: [dir, fixture], write: [dir] }, env: { FYX_VSCODE_PROBE: "1" } });
  try {
    await host.initialize({ extensionPath: fixture, workspaceFolders: [Uri.file(dir).toString()], storageDir: join(dir, "storage") });
    await host.openDocument(Uri.file(join(dir, "a.txt")).toString(), "x", "plaintext");
    await host.settle();
    assert.equal(await host.executeCommand("sink.webview"), null, "the stub returns instead of throwing");
    assert.equal((await host.coverage()).notImplemented["vscode.window.createWebviewPanel"], 1);
  } finally {
    await host.close();
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a crafted publisher or name cannot steer the storage folder", async () => {
  const dir = tempDir("fyx-crafted-");
  try {
    for (const [k, fields] of [
      { publisher: "../../escaped", name: "x" },
      { publisher: "p", name: "..\\..\\escaped" },
      { publisher: "C:", name: "x" },
    ].entries()) {
      const vsix = join(dir, `crafted-${k}.vsix`);
      writeFileSync(vsix, writeZip({ "extension/package.json": JSON.stringify({ ...fields, version: "1.0.0" }) }));
      const { host } = MockHost.inProcess();
      try {
        await assert.rejects(host.initialize({ extensionPath: vsix, storageDir: join(dir, "storage") }), /is not a valid extension name/);
      } finally {
        await host.close();
      }
    }
    assert.deepEqual(readdirSync(dir).filter((f) => !f.endsWith(".vsix")).sort(), ["storage"]);
    assert.deepEqual(readdirSync(join(dir, "storage")), ["unpacked"], "no storage folder was created for a crafted id");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
