// Minimal ZIP reader for .vsix packages (stored and deflate entries, no ZIP64).
// tools/vsix-convert/src/zip.ts and sidecars/vscode-host/src/zip.ts are byte-identical copies:
// the two packages are deliberately independent, and a test in each fails when they differ.
import { crc32, inflateRawSync } from "node:zlib";

export type ZipEntries = Map<string, Buffer>;

export function readZip(data: Buffer): ZipEntries {
  const eocd = findEndOfCentralDirectory(data);
  const count = data.readUInt16LE(eocd + 10);
  let offset = data.readUInt32LE(eocd + 16);
  const entries: ZipEntries = new Map();
  for (let k = 0; k < count; k++) {
    if (data.readUInt32LE(offset) !== 0x02014b50) throw new Error("zip: bad central directory");
    const method = data.readUInt16LE(offset + 10);
    const compressedSize = data.readUInt32LE(offset + 20);
    const nameLength = data.readUInt16LE(offset + 28);
    const extraLength = data.readUInt16LE(offset + 30);
    const commentLength = data.readUInt16LE(offset + 32);
    const localOffset = data.readUInt32LE(offset + 42);
    const name = data.toString("utf8", offset + 46, offset + 46 + nameLength);
    offset += 46 + nameLength + extraLength + commentLength;
    if (name.endsWith("/")) continue;
    const unsafe = unsafePathReason(name);
    if (unsafe) throw new Error(`zip: unsafe entry name ${JSON.stringify(name)}: ${unsafe}`);
    const localNameLength = data.readUInt16LE(localOffset + 26);
    const localExtraLength = data.readUInt16LE(localOffset + 28);
    const start = localOffset + 30 + localNameLength + localExtraLength;
    const raw = data.subarray(start, start + compressedSize);
    if (method === 0) entries.set(name, Buffer.from(raw));
    else if (method === 8) entries.set(name, inflateRawSync(raw));
    else throw new Error(`zip: unsupported compression method ${method} for ${name}`);
  }
  return entries;
}

/**
 * Why `path` is not a safe relative POSIX path, or `undefined` when it is. Rejected: absolute
 * paths, `.`/`..`/empty segments, backslashes (a separator on Windows), `:` (drive letters and
 * NTFS alternate data streams) and control characters.
 */
export function unsafePathReason(path: string): string | undefined {
  if (path.length === 0) return "empty path";
  if (path.startsWith("/")) return "absolute path";
  if (path.includes("\\")) return "backslash";
  if (path.includes(":")) return "colon (drive letter or stream name)";
  if (/[\x00-\x1f]/.test(path)) return "control character";
  for (const segment of path.split("/")) {
    if (segment === "" || segment === "." || segment === "..") return `segment ${JSON.stringify(segment)}`;
  }
  return undefined;
}

function findEndOfCentralDirectory(data: Buffer): number {
  for (let i = data.length - 22; i >= Math.max(0, data.length - 22 - 0xffff); i--) {
    if (data.readUInt32LE(i) === 0x06054b50) return i;
  }
  throw new Error("zip: end of central directory not found");
}

/** Builds a stored (uncompressed) zip; used by tests to make fixture .vsix files. */
export function writeZip(files: Record<string, string | Buffer>): Buffer {
  const locals: Buffer[] = [];
  const centrals: Buffer[] = [];
  let offset = 0;
  for (const [name, content] of Object.entries(files)) {
    const body = typeof content === "string" ? Buffer.from(content) : content;
    const nameBuf = Buffer.from(name);
    const crc = crc32(body);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(body.length, 18);
    local.writeUInt32LE(body.length, 22);
    local.writeUInt16LE(nameBuf.length, 26);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(body.length, 20);
    central.writeUInt32LE(body.length, 24);
    central.writeUInt16LE(nameBuf.length, 28);
    central.writeUInt32LE(offset, 42);
    locals.push(local, nameBuf, body);
    centrals.push(central, nameBuf);
    offset += 30 + nameBuf.length + body.length;
  }
  const centralSize = centrals.reduce((sum, b) => sum + b.length, 0);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  const count = Object.keys(files).length;
  eocd.writeUInt16LE(count, 8);
  eocd.writeUInt16LE(count, 10);
  eocd.writeUInt32LE(centralSize, 12);
  eocd.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, ...centrals, eocd]);
}
