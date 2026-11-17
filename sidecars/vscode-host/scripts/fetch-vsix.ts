// Downloads the pinned extensions in extensions.json from open-vsx.org into vsix/ (gitignored),
// checking each file against the SHA-256 Open VSX publishes and the license it reports.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const MAX_BYTES = 25 * 1024 * 1024;
const ALLOWED_LICENSES = new Set(["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC"]);
const root = join(import.meta.dirname, "..");
const outDir = join(root, "vsix");
const list = JSON.parse(readFileSync(join(root, "extensions.json"), "utf8")).extensions as { id: string; version: string }[];
mkdirSync(outDir, { recursive: true });

let failed = 0;
for (const { id, version } of list) {
  const [namespace, name] = id.split(".");
  const target = join(outDir, `${id}-${version}.vsix`);
  try {
    const meta = (await (await fetch(`https://open-vsx.org/api/${namespace}/${name}/${version}`)).json()) as {
      license?: string;
      files?: { download?: string; sha256?: string };
      error?: string;
    };
    if (meta.error || !meta.files?.download) throw new Error(meta.error ?? "no download URL");
    if (!meta.license || !ALLOWED_LICENSES.has(meta.license)) throw new Error(`license ${meta.license} is not on the permissive list`);
    const download = new URL(meta.files.download);
    if (download.hostname !== "open-vsx.org") throw new Error(`refusing download from ${download.hostname}`);
    const shaUrl = meta.files.sha256 ? new URL(meta.files.sha256) : undefined;
    if (shaUrl && shaUrl.hostname !== "open-vsx.org") throw new Error(`refusing checksum from ${shaUrl.hostname}`);
    const expected = shaUrl ? (await (await fetch(shaUrl)).text()).trim().split(/\s+/)[0].toLowerCase() : undefined;
    let data: Buffer;
    if (existsSync(target)) data = readFileSync(target);
    else {
      const res = await fetch(download);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const length = Number(res.headers.get("content-length") ?? 0);
      if (length > MAX_BYTES) throw new Error(`${length} bytes exceeds the ${MAX_BYTES} byte cap`);
      data = Buffer.from(await res.arrayBuffer());
      if (data.length > MAX_BYTES) throw new Error(`${data.length} bytes exceeds the cap`);
    }
    const actual = createHash("sha256").update(data).digest("hex");
    if (expected && actual !== expected) throw new Error(`sha256 mismatch: expected ${expected}, got ${actual}`);
    writeFileSync(target, data);
    console.log(`${id}@${version}: ${(data.length / 1024).toFixed(0)} KiB, ${meta.license}, sha256 ${expected ? "verified" : "not published"}`);
  } catch (err) {
    failed++;
    console.error(`ERROR ${id}@${version}: ${(err as Error).message}`);
  }
}
process.exit(failed ? 1 : 0);
