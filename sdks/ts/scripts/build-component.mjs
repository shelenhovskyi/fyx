#!/usr/bin/env node
// Builds a TypeScript Fyx UI extension into a `fyx:ui@0.1.0` component:
//
//   1. `jco componentize` bundles the entry module (and the SDK) and embeds it in a JS engine,
//      with the WASI features of the chosen profile.
//   2. Appends the `fyx:ui-api-version` custom section, which jco cannot emit.
//   3. Writes the component's WIT (`wasm-tools component wit`, or `jco wit` without wasm-tools)
//      next to it, so the exports and imports can be reviewed.
//
// Usage: node scripts/build-component.mjs <entry.ts> <out.wasm> [--profile <profile>]
// Profiles: starlingmonkey (default), starlingmonkey-no-wasi, quickjs. See sdks/ts/README.md.

import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
    API_VERSION_SECTION,
    readCustomSections,
    witPackageVersion,
    withApiVersionSection,
} from './api-version-section.mjs';

const sdkRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const witDir = resolve(sdkRoot, '../../crates/ui_ext/wit');
const witFile = join(witDir, 'fyx-ui.wit');
const jcoBin = join(sdkRoot, 'node_modules/.bin/jco');

const PROFILES = {
    // StarlingMonkey (SpiderMonkey). jco's default also imports `wasi:http`, which the host does not
    // link, so the component would not instantiate. Without http and stdio it imports only clocks,
    // random, stderr and io from WASI p2, all of which the host links; `Date.now` and
    // `Math.random` then work. `console.*` writes nowhere.
    starlingmonkey: ['--disable', 'http', 'fetch-event', 'stdio'],
    // StarlingMonkey with no WASI imports at all. `Date.now` returns the build time and
    // `Math.random` replays the same sequence in every instance, both frozen in the snapshot.
    'starlingmonkey-no-wasi': ['--disable', 'all'],
    // QuickJS, with WASI stubbed (only `fyx:ui` is imported) and the synchronous ABI: the default
    // async ABI needs wasmtime's component-model-async feature, which the host does not enable.
    // `Date.now` traps and `console` is undefined. Fails the counter contract: componentize-qjs
    // saturates u32 values above 2^31 - 1, which list-row ids are.
    quickjs: ['--backend', 'quickjs', '--backend-qjs-stub-wasi', '--backend-qjs-disable-async'],
};

const USAGE = `usage: build-component.mjs <entry.ts> <out.wasm> [--profile ${Object.keys(PROFILES).join('|')}]`;

function parseArguments(argv) {
    const positional = [];
    let profile = 'starlingmonkey';
    for (let index = 0; index < argv.length; index += 1) {
        if (argv[index] === '--profile') {
            profile = argv[index + 1];
            index += 1;
        } else {
            positional.push(argv[index]);
        }
    }
    const [entry, out] = positional;
    if (entry === undefined || out === undefined || !(profile in PROFILES)) {
        throw new Error(USAGE);
    }
    return { entry: resolve(entry), out: resolve(out), profile };
}

function run(command, args, options = {}) {
    const result = spawnSync(command, args, { encoding: 'utf8', ...options });
    if (result.error !== undefined) {
        throw result.error;
    }
    if (result.status !== 0) {
        throw new Error(
            `${command} ${args.join(' ')} exited with ${result.status}\n${result.stdout}${result.stderr}`,
        );
    }
    return result.stdout;
}

function hasWasmTools() {
    return spawnSync('wasm-tools', ['--version'], { encoding: 'utf8' }).status === 0;
}

function main() {
    const { entry, out, profile } = parseArguments(process.argv.slice(2));
    mkdirSync(dirname(out), { recursive: true });

    const started = performance.now();
    run(jcoBin, [
        'componentize',
        entry,
        '--wit',
        witDir,
        '--world-name',
        'ui-extension',
        ...PROFILES[profile],
        '--out',
        out,
    ]);
    const componentizeMs = performance.now() - started;

    const version = witPackageVersion(witFile);
    const component = withApiVersionSection(readFileSync(out), version);
    writeFileSync(out, component);
    const sections = readCustomSections(component, API_VERSION_SECTION);
    if (sections.length !== 1) {
        throw new Error(`expected one ${API_VERSION_SECTION} section, found ${sections.length}`);
    }

    const witOutput = hasWasmTools()
        ? run('wasm-tools', ['component', 'wit', out])
        : run(jcoBin, ['wit', out]);
    const witPath = out.replace(/\.wasm$/, '') + '.wit';
    writeFileSync(witPath, witOutput);

    const imports = witOutput
        .split('\n')
        .map((line) => line.trim())
        .filter((line) => line.startsWith('import '));
    console.log(`${out}`);
    console.log(`  profile: ${profile}, componentize: ${(componentizeMs / 1000).toFixed(1)} s`);
    console.log(`  size: ${component.length} bytes (${(component.length / 2 ** 20).toFixed(2)} MiB)`);
    console.log(`  ${API_VERSION_SECTION}: ${version.join('.')}`);
    console.log(`  wit: ${witPath}`);
    console.log(`  imports: ${imports.length === 0 ? 'none' : ''}`);
    for (const line of imports) {
        console.log(`    ${line}`);
    }
}

main();
