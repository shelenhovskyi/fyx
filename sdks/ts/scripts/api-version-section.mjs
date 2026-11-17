// Reads and appends the `fyx:ui-api-version` custom section of a WASM component.
//
// The host (crates/extension_host/src/wasm_host/fyx_ui.rs, `parse_api_version`) reads the version
// from this section before instantiating: three big-endian u16s. The Rust SDK emits it with
// `#[link_section]`; `jco componentize` has no way to add a custom section, so the build appends
// one to the finished component. Two copies are rejected by the host, so this never adds a second.

import { readFileSync } from 'node:fs';

export const API_VERSION_SECTION = 'fyx:ui-api-version';

const WASM_MAGIC = [0x00, 0x61, 0x73, 0x6d];
// Version 0x0d and layer 1 mark a component, as opposed to a core module (version 1, layer 0).
const COMPONENT_VERSION_AND_LAYER = [0x0d, 0x00, 0x01, 0x00];
const CUSTOM_SECTION_ID = 0;

/** The `fyx:ui` version a WIT package declares, from its `package fyx:ui@X.Y.Z;` line. */
export function witPackageVersion(witPath) {
    const source = readFileSync(witPath, 'utf8');
    const match = /^\s*package\s+fyx:ui@(\d+)\.(\d+)\.(\d+)\s*;/m.exec(source);
    if (match === null) {
        throw new Error(`${witPath} declares no fyx:ui package version`);
    }
    return match.slice(1, 4).map(Number);
}

export function versionSectionBytes([major, minor, patch]) {
    const bytes = new Uint8Array(6);
    const view = new DataView(bytes.buffer);
    [major, minor, patch].forEach((part, index) => {
        if (!Number.isInteger(part) || part < 0 || part > 0xffff) {
            throw new RangeError(`version part ${part} does not fit a u16`);
        }
        view.setUint16(index * 2, part, false);
    });
    return bytes;
}

function encodeUnsignedLeb128(value) {
    const bytes = [];
    let remaining = value;
    do {
        let byte = remaining & 0x7f;
        remaining = Math.floor(remaining / 128);
        if (remaining !== 0) {
            byte |= 0x80;
        }
        bytes.push(byte);
    } while (remaining !== 0);
    return bytes;
}

function decodeUnsignedLeb128(bytes, offset) {
    let value = 0;
    let scale = 1;
    for (let position = offset; position < bytes.length && position < offset + 5; position += 1) {
        const byte = bytes[position];
        value += (byte & 0x7f) * scale;
        scale *= 128;
        if ((byte & 0x80) === 0) {
            return { value, next: position + 1 };
        }
    }
    throw new Error(`malformed LEB128 at byte ${offset}`);
}

function checkComponentHeader(bytes) {
    const header = [...WASM_MAGIC, ...COMPONENT_VERSION_AND_LAYER];
    if (bytes.length < header.length || header.some((byte, index) => bytes[index] !== byte)) {
        throw new Error('not a WASM component (bad magic, version or layer)');
    }
}

/** The data of every top-level custom section named `name`, in order. */
export function readCustomSections(bytes, name) {
    checkComponentHeader(bytes);
    const decoder = new TextDecoder('utf-8', { fatal: true });
    const found = [];
    let offset = 8;
    while (offset < bytes.length) {
        const id = bytes[offset];
        const size = decodeUnsignedLeb128(bytes, offset + 1);
        const end = size.next + size.value;
        if (end > bytes.length) {
            throw new Error(`section at byte ${offset} runs past the end of the component`);
        }
        if (id === CUSTOM_SECTION_ID) {
            const nameLength = decodeUnsignedLeb128(bytes, size.next);
            const nameEnd = nameLength.next + nameLength.value;
            if (nameEnd > end) {
                throw new Error(`custom section name at byte ${offset} runs past its section`);
            }
            if (decoder.decode(bytes.subarray(nameLength.next, nameEnd)) === name) {
                found.push(bytes.slice(nameEnd, end));
            }
        }
        offset = end;
    }
    return found;
}

/** `bytes` with a custom section `name` holding `data` appended after the last section. */
export function appendCustomSection(bytes, name, data) {
    checkComponentHeader(bytes);
    const encodedName = new TextEncoder().encode(name);
    const payload = [...encodeUnsignedLeb128(encodedName.length), ...encodedName, ...data];
    const section = Uint8Array.from([
        CUSTOM_SECTION_ID,
        ...encodeUnsignedLeb128(payload.length),
        ...payload,
    ]);
    const result = new Uint8Array(bytes.length + section.length);
    result.set(bytes, 0);
    result.set(section, bytes.length);
    return result;
}

/**
 * `bytes` with the API version section set to `version`. Unchanged if the section already holds
 * that version; an error if it holds another one or appears more than once.
 */
export function withApiVersionSection(bytes, version) {
    const expected = versionSectionBytes(version);
    const existing = readCustomSections(bytes, API_VERSION_SECTION);
    if (existing.length > 1) {
        throw new Error(`component already has ${existing.length} ${API_VERSION_SECTION} sections`);
    }
    if (existing.length === 1) {
        const [data] = existing;
        if (data.length === expected.length && data.every((byte, index) => byte === expected[index])) {
            return bytes;
        }
        throw new Error(`component already has ${API_VERSION_SECTION} = [${data.join(', ')}]`);
    }
    return appendCustomSection(bytes, API_VERSION_SECTION, expected);
}
