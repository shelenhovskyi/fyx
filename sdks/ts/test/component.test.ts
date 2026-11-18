// The counter contract against the built components: each is transpiled with `jco transpile` and
// instantiated in Node with a mocked `fyx:ui/host` import (WASI comes from jco's preview2-shim).
// Build them first with `npm run build:all`; a component that is not built is skipped.

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { describe, test } from 'node:test';

import { readCustomSections } from '../scripts/api-version-section.mjs';
import { replayCounterContract } from './counter-contract.ts';
import type { CounterWorld } from './counter-contract.ts';
import { installHostMock } from './host-mock.ts';

const sdkRoot = fileURLToPath(new URL('..', import.meta.url));
const jcoBin = join(sdkRoot, 'node_modules/.bin/jco');
// Inside the package, so the transpiled modules resolve `@bytecodealliance/preview2-shim` (WASI in
// Node) from its node_modules.
const transpiledRoot = join(sdkRoot, 'test/.transpiled');
let instanceCount = 0;

/**
 * Transpiles `component` to an ES module whose `fyx:ui/host` import is `global-host.mjs`, which
 * forwards to the mock `installHostMock` installs.
 */
function transpile(component: string, name: string): string {
    const dir = join(transpiledRoot, name);
    rmSync(dir, { recursive: true, force: true });
    const hostModule = '../../global-host.mjs';
    const result = spawnSync(
        jcoBin,
        [
            'transpile',
            component,
            '--name',
            'counter',
            '--quiet',
            '--map',
            `fyx:ui/host=${hostModule}`,
            '--map',
            `fyx:ui/host@0.1.0=${hostModule}`,
            '-o',
            dir,
        ],
        { encoding: 'utf8' },
    );
    assert.equal(result.status, 0, `jco transpile failed:\n${result.stdout}${result.stderr}`);
    return join(dir, 'counter.js');
}

/** A fresh instance: every distinct URL is a separate module, so a separate component instance. */
async function instantiate(transpiled: string): Promise<CounterWorld> {
    instanceCount += 1;
    return import(`${pathToFileURL(transpiled).href}?instance=${instanceCount}`);
}

function componentWit(component: string): string {
    const result = spawnSync(jcoBin, ['wit', component], { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout;
}

const fyxImports = ['import fyx:ui/host@0.1.0;', 'import fyx:ui/types@0.1.0;'];
const backends = [
    {
        name: 'starlingmonkey',
        path: join(sdkRoot, 'demo-counter/dist/fyx_ui.wasm'),
        imports: [
            ...fyxImports,
            'import wasi:cli/stderr@0.2.12;',
            'import wasi:clocks/monotonic-clock@0.2.12;',
            'import wasi:clocks/wall-clock@0.2.12;',
            'import wasi:io/error@0.2.12;',
            'import wasi:io/poll@0.2.12;',
            'import wasi:io/streams@0.2.12;',
            'import wasi:random/random@0.2.12;',
        ],
        todo: undefined,
    },
    {
        name: 'starlingmonkey-no-wasi',
        path: join(sdkRoot, 'demo-counter/dist/no-wasi/fyx_ui.wasm'),
        imports: fyxImports,
        todo: undefined,
    },
    {
        name: 'quickjs',
        path: join(sdkRoot, 'demo-counter/dist/quickjs/fyx_ui.wasm'),
        imports: fyxImports,
        // componentize-qjs 0.5 lowers a u32 above 2^31 - 1 as 2^31 - 1, so list rows (ids from
        // 0x8000_0000) come out with duplicate ids. See sdks/ts/README.md.
        todo: 'componentize-qjs saturates u32 values above 2^31 - 1',
    },
];

for (const backend of backends) {
    describe(`${backend.name} component`, { skip: existsSync(backend.path) ? false : `${backend.path} not built` }, () => {
        test('carries one fyx:ui-api-version section of 0.1.0', () => {
            const sections = readCustomSections(readFileSync(backend.path), 'fyx:ui-api-version');
            assert.deepEqual(
                sections.map((section) => [...section]),
                [[0, 0, 0, 1, 0, 0]],
            );
        });

        test('exports the ui-extension world and imports only fyx:ui', () => {
            const wit = componentWit(backend.path);
            for (const signature of [
                'export surfaces: func() -> list<surface>;',
                'export mount: func(surface-id: string) -> result<list<node>, string>;',
                'export handle-event: func(surface-id: string, event: ui-event) -> result<update, string>;',
                'export list-rows: func(surface-id: string, list-node: node-id, start: u32, count: u32) -> list<list-row>;',
            ]) {
                assert.ok(wit.includes(signature), `missing ${signature}\n${wit}`);
            }
            const imports = wit
                .split('\n')
                .map((line) => line.trim())
                .filter((line) => line.startsWith('import '));
            assert.deepEqual(imports.sort(), [...backend.imports].sort());
        });

        test('behaves as documented', { todo: backend.todo }, async (context) => {
            const transpiled = transpile(backend.path, backend.name);
            installHostMock();
            const started = performance.now();
            const timed = await instantiate(transpiled);
            const instantiated = performance.now();
            timed.surfaces();
            context.diagnostic(
                `node: compile + instantiate ${(instantiated - started).toFixed(1)} ms, ` +
                    `first surfaces() ${(performance.now() - instantiated).toFixed(2)} ms`,
            );

            const host = installHostMock();
            replayCounterContract(await instantiate(transpiled), host);
        });
    });
}
