// Minimal ZIP reader for .vsix packages (stored and deflate entries, no ZIP64).
// tools/vsix-convert/src/zip.ts and sidecars/vscode-host/src/zip.ts are byte-identical copies:
// the two packages are deliberately independent, and a test in each fails when they differ.
import { crc32, deflateRawSync, inflateRawSync } from "node:zlib";

/**
 * Caps on the uncompressed sizes the central directory declares. A `.vsix` is untrusted input
 * and deflate reaches ratios above 1000:1, so the compressed file size says nothing about what
 * inflating it costs.
 */
export interface ZipLimits {
  maxEntryBytes: number;
  maxTotalBytes: number;
}

const MiB = 1024 * 1024;
export const DEFAULT_ZIP_LIMITS: Readonly<ZipLimits> = { maxEntryBytes: 64 * MiB, maxTotalBytes: 256 * MiB };

export interface ZipEntry {
  readonly name: string;
  /** Uncompressed size from the central directory; `read()` fails unless the data matches it. */
  readonly size: number;
  /** Inflates the entry now; nothing is inflated before the first call. */
  read(): Buffer;
}

export type ZipEntries = Map<string, ZipEntry>;

export function readZip(data: Buffer, limits: Partial<ZipLimits> = {}): ZipEntries {
  const { maxEntryBytes, maxTotalBytes } = { ...DEFAULT_ZIP_LIMITS, ...limits };
  const eocd = findEndOfCentralDirectory(data);
  const count = data.readUInt16LE(eocd + 10);
  let offset = data.readUInt32LE(eocd + 16);
  const entries: ZipEntries = new Map();
  let total = 0;
  for (let k = 0; k < count; k++) {
    need(data, offset, 46);
    if (data.readUInt32LE(offset) !== 0x02014b50) throw new Error("zip: bad central directory");
    const flags = data.readUInt16LE(offset + 8);
    const method = data.readUInt16LE(offset + 10);
    const crc = data.readUInt32LE(offset + 16);
    const compressedSize = data.readUInt32LE(offset + 20);
    const size = data.readUInt32LE(offset + 24);
    const nameLength = data.readUInt16LE(offset + 28);
    const extraLength = data.readUInt16LE(offset + 30);
    const commentLength = data.readUInt16LE(offset + 32);
    const localOffset = data.readUInt32LE(offset + 42);
    need(data, offset + 46, nameLength);
    const name = data.toString("utf8", offset + 46, offset + 46 + nameLength);
    offset += 46 + nameLength + extraLength + commentLength;
    if (name.endsWith("/")) continue;
    const unsafe = unsafePathReason(name);
    if (unsafe) throw new Error(`zip: unsafe entry name ${JSON.stringify(name)}: ${unsafe}`);
    if (entries.has(name)) throw new Error(`zip: duplicate entry ${name}`);
    if (flags & 0x1) throw new Error(`zip: ${name} is encrypted`);
    if (size === 0xffffffff || compressedSize === 0xffffffff || localOffset === 0xffffffff) {
      throw new Error(`zip: ${name} needs ZIP64, which is not supported`);
    }
    if (method !== 0 && method !== 8) throw new Error(`zip: unsupported compression method ${method} for ${name}`);
    if (method === 0 && compressedSize !== size) throw new Error(`zip: stored entry ${name} has mismatched sizes`);
    if (size > maxEntryBytes) throw new Error(`zip: ${name} is ${size} bytes uncompressed, above the ${maxEntryBytes}-byte entry limit`);
    total += size;
    if (total > maxTotalBytes) throw new Error(`zip: archive is above the ${maxTotalBytes}-byte uncompressed limit`);
    need(data, localOffset, 30);
    if (data.readUInt32LE(localOffset) !== 0x04034b50) throw new Error(`zip: bad local header for ${name}`);
    const localNameLength = data.readUInt16LE(localOffset + 26);
    const localExtraLength = data.readUInt16LE(localOffset + 28);
    const start = localOffset + 30 + localNameLength + localExtraLength;
    need(data, start, compressedSize);
    const raw = data.subarray(start, start + compressedSize);
    entries.set(name, { name, size, read: () => inflateEntry(name, method, raw, size, crc) });
  }
  return entries;
}

function inflateEntry(name: string, method: number, raw: Buffer, size: number, crc: number): Buffer {
  let content: Buffer;
  if (method === 0) {
    content = Buffer.from(raw);
  } else {
    try {
      // `maxOutputLength` must be at least 1; an empty entry is still checked by the length test.
      content = inflateRawSync(raw, { maxOutputLength: Math.max(1, size) });
    } catch (err) {
      throw new Error(`zip: ${name} does not inflate to its declared ${size} bytes: ${(err as Error).message}`);
    }
  }
  if (content.length !== size) throw new Error(`zip: ${name} inflates to ${content.length} bytes, not the declared ${size}`);
  if (crc32(content) !== crc) throw new Error(`zip: ${name} fails its CRC check`);
  return content;
}

function need(data: Buffer, offset: number, length: number): void {
  if (offset + length > data.length) throw new Error("zip: truncated archive");
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

/** Builds a zip, stored unless `deflate` is set; used by tests to make fixture .vsix files. */
export function writeZip(files: Record<string, string | Buffer>, options: { deflate?: boolean } = {}): Buffer {
  const method = options.deflate ? 8 : 0;
  const locals: Buffer[] = [];
  const centrals: Buffer[] = [];
  let offset = 0;
  for (const [name, content] of Object.entries(files)) {
    const plain = typeof content === "string" ? Buffer.from(content) : content;
    const body = method === 8 ? deflateRawSync(plain) : plain;
    const nameBuf = Buffer.from(name);
    const crc = crc32(plain);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(method, 8);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(body.length, 18);
    local.writeUInt32LE(plain.length, 22);
    local.writeUInt16LE(nameBuf.length, 26);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(method, 10);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(body.length, 20);
    central.writeUInt32LE(plain.length, 24);
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
