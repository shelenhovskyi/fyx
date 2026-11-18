// The real transport: the sidecar as a child process speaking NDJSON on stdio, under Node's
// permission model, loading the fixture packed as a .vsix.
import { test } from "node:test";
import assert from "node:assert/strict";
import { chmodSync, chownSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { join, relative, sep } from "node:path";
import { defaultStorageDir, ensurePrivateDir, sandboxProblem, unpackVsix } from "../src/host.ts";
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

test("the sidecar refuses to start without the permission model or with a grant that voids it", () => {
  const main = join(import.meta.dirname, "..", "src", "main.ts");
  const here = join(import.meta.dirname, "..");
  for (const [flags, expected] of [
    [[], /refusing to start: the sidecar must run under Node's permission model/],
    [["--permission", `--allow-fs-read=${here}`, "--allow-child-process"], /refusing to start: the sidecar's sandbox is too wide: drop --allow-child-process/],
    [["--permission", "--allow-fs-read=*"], /sandbox is too wide: drop --allow-fs-read=\*/],
  ] as const) {
    const run = spawnSync(process.execPath, [...flags, main], { cwd: here, input: "", encoding: "utf8", timeout: 20_000 });
    assert.equal(run.status, 78, `${flags.join(" ")}: ${run.stderr}`);
    assert.match(run.stderr, expected);
    assert.equal(run.stdout, "", "nothing reaches the protocol stream");
  }
});

test("sandboxProblem accepts only path-scoped grants", () => {
  const grants = (granted: string[]) => ({ has: (scope: string) => granted.includes(scope) });
  assert.equal(sandboxProblem(grants([])), undefined);
  assert.match(sandboxProblem(undefined)!, /--permission/);
  assert.equal(sandboxProblem(grants(["fs.write", "worker", "addon"])), "the sidecar's sandbox is too wide: drop --allow-fs-write=*, --allow-worker, --allow-addons");
  assert.equal(sandboxProblem({ has: () => { throw new Error("unknown scope"); } }), undefined, "a scope this Node lacks cannot be granted");
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

test("probe mode records missing APIs without throwing, and only with --fyx-coverage-probe", async () => {
  for (const mode of ["flag", "env only"] as const) {
    const dir = tempDir("fyx-probe-");
    const host = MockHost.spawn({
      permission: { read: [dir, fixture], write: [dir] },
      ...(mode === "flag" ? { probe: true } : { env: { FYX_VSCODE_PROBE: "1" } }),
    });
    try {
      await host.initialize({ extensionPath: fixture, workspaceFolders: [Uri.file(dir).toString()], storageDir: join(dir, "storage") });
      await host.openDocument(Uri.file(join(dir, "a.txt")).toString(), "x", "plaintext");
      await host.settle();
      if (mode === "flag") {
        assert.equal(await host.executeCommand("sink.webview"), null, "the stub returns instead of throwing");
      } else {
        await assert.rejects(host.executeCommand("sink.webview"), /createWebviewPanel is not implemented in Fyx/, "the environment variable alone enables nothing");
        assert.match(host.sidecarStderr, /FYX_VSCODE_PROBE is ignored/);
      }
      assert.equal((await host.coverage()).notImplemented["vscode.window.createWebviewPanel"], 1);
    } finally {
      await host.close();
      rmSync(dir, { recursive: true, force: true });
    }
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

const posix = process.platform !== "win32";

test("unpacked .vsix files go to a private cache and are reused only when complete", () => {
  const dir = tempDir("fyx-unpack-");
  try {
    const vsix = join(dir, "kitchen-sink.vsix");
    packFixture(vsix);
    const cache = join(dir, "cache");
    const root = unpackVsix(vsix, cache);
    if (posix) assert.equal(statSync(cache).mode & 0o777, 0o700);
    assert.deepEqual(JSON.parse(readFileSync(join(root, "package.json"), "utf8")).name, "kitchen-sink");
    assert.deepEqual(readdirSync(cache), [root.slice(cache.length + 1)], "no staging folder is left behind");

    writeFileSync(join(root, "reuse-probe"), "");
    assert.equal(unpackVsix(vsix, cache), root);
    assert.ok(existsSync(join(root, "reuse-probe")), "a complete unpack is reused");

    // A crashed unpack: package.json present, marker missing. Its files must not survive.
    rmSync(join(root, ".fyx-unpack-complete"));
    writeFileSync(join(root, "package.json"), JSON.stringify({ name: "planted", main: "./evil.js" }));
    assert.equal(unpackVsix(vsix, cache), root);
    assert.equal(JSON.parse(readFileSync(join(root, "package.json"), "utf8")).name, "kitchen-sink");
    assert.ok(!existsSync(join(root, "reuse-probe")), "the incomplete folder was replaced, not merged");

    // A marker that does not hold this archive's digest (for example, one packed in the .vsix).
    writeFileSync(join(root, ".fyx-unpack-complete"), "0".repeat(64));
    writeFileSync(join(root, "package.json"), JSON.stringify({ name: "planted" }));
    unpackVsix(vsix, cache);
    assert.equal(JSON.parse(readFileSync(join(root, "package.json"), "utf8")).name, "kitchen-sink");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a failed unpack leaves nothing that can be reused", () => {
  const dir = tempDir("fyx-unpack-fail-");
  try {
    const vsix = join(dir, "broken.vsix");
    const zip = writeZip({ "extension/package.json": "{}", "extension/out/a.js": "x" });
    zip[zip.lastIndexOf("extension/out/a.js") - 46 + 16] ^= 0xff; // corrupt the second entry's CRC
    writeFileSync(vsix, zip);
    const cache = join(dir, "cache");
    assert.throws(() => unpackVsix(vsix, cache), /CRC/);
    assert.deepEqual(readdirSync(cache), []);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the cache folder must be private to this user", { skip: !posix }, () => {
  const dir = tempDir("fyx-private-");
  try {
    assert.match(defaultStorageDir(), new RegExp(`fyx-vscode-host-${process.getuid?.()}$`));
    const open = join(dir, "open");
    mkdirSync(open, { mode: 0o777 });
    chmodSync(open, 0o777);
    ensurePrivateDir(open);
    assert.equal(statSync(open).mode & 0o777, 0o700, "an own folder is closed to group and others");

    mkdirSync(join(dir, "elsewhere"));
    symlinkSync(join(dir, "elsewhere"), join(dir, "link"));
    assert.throws(() => ensurePrivateDir(join(dir, "link")), /not a folder/);

    if (process.getuid?.() === 0) {
      const foreign = join(dir, "foreign");
      mkdirSync(foreign, { mode: 0o700 });
      chownSync(foreign, 4242, 4242);
      assert.throws(() => ensurePrivateDir(foreign), /owned by uid 4242/);
      const vsix = join(dir, "k.vsix");
      packFixture(vsix);
      assert.throws(() => unpackVsix(vsix, foreign), /owned by uid 4242/);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
