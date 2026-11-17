// The behaviour contract of the counter demo, as written in the crate doc of
// `extensions/fyx_demo_counter/src/fyx_demo_counter.rs` and pinned by its test
// `behaves_as_documented`. Replayed against the TypeScript sources and against the built component.

import assert from 'node:assert/strict';

import type { ListRow, Node, Props, Surface, UiEvent, Update } from 'fyx-ui-sdk';
import type { HostMock } from './host-mock.ts';

/** The world's exports, whether called directly or through a transpiled component. */
export interface CounterWorld {
    surfaces(): Surface[];
    mount(surfaceId: string): Node[];
    handleEvent(surfaceId: string, event: UiEvent): Update;
    listRows(surfaceId: string, listNode: number, start: number, count: number): ListRow[];
}

const ROW_NODE_ID_BASE = 0x8000_0000;

/** Drops keys whose value is `undefined`, so absent options compare equal however they are lifted. */
export function plain<T>(value: T): T {
    return JSON.parse(JSON.stringify(value)) as T;
}

/**
 * The `err` of a `result` export. Source-level exports throw the string; jco throws an `Error`
 * whose `payload` is the string.
 */
export function resultError(call: () => unknown): string {
    try {
        call();
    } catch (error) {
        if (typeof error === 'string') {
            return error;
        }
        if (error !== null && typeof error === 'object' && 'payload' in error) {
            return String(error.payload);
        }
        throw error;
    }
    assert.fail('expected the call to return an error');
}

function text(content: string, bold = false): Props {
    return { tag: 'text', val: { content, bold, monospace: false } };
}

function button(
    label: string,
    style: 'default' | 'filled' | 'subtle',
    disabled: boolean,
    icon?: string,
): Props {
    return plain({ tag: 'button', val: { label, icon, style, disabled } });
}

function flex(direction: 'row' | 'column', gap: number, padding: number, grow: boolean): Props {
    return { tag: 'flex', val: { direction, gap, padding, grow, align: 'start' } };
}

function list(rowCount: number): Props {
    return { tag: 'list', val: { rowCount, rowHeight: 24 } };
}

function setPropsIds(update: Update): number[] {
    assert.equal(update.tag, 'patches', `expected patches, got ${JSON.stringify(update)}`);
    if (update.tag !== 'patches') {
        return [];
    }
    return update.val.map((patch) => {
        assert.equal(patch.tag, 'set-props', `expected only set-props, got ${JSON.stringify(patch)}`);
        return patch.tag === 'set-props' ? patch.val[0] : -1;
    });
}

export function replayCounterContract(world: CounterWorld, host: HostMock): void {
    const click = (id: number): Update => world.handleEvent('counter', { tag: 'click', val: id });
    const unchanged: Update = { tag: 'unchanged' };

    assert.deepEqual(plain(world.surfaces()), [
        {
            id: 'counter',
            title: 'Counter',
            kind: { tag: 'panel', val: { position: 'right', icon: 'hash', defaultSize: 320 } },
        },
    ]);

    const nodes = plain(world.mount('counter'));
    assert.deepEqual(nodes, [
        { id: 1, props: flex('column', 8, 8, true) },
        { id: 2, parent: 1, props: text('Count: 0', true) },
        { id: 3, parent: 1, props: flex('row', 4, 0, false) },
        { id: 4, parent: 3, props: button('Decrement', 'default', true, 'dash') },
        { id: 5, parent: 3, props: button('Increment', 'filled', false, 'plus') },
        { id: 6, parent: 3, props: button('Reset', 'subtle', true) },
        { id: 7, parent: 1, props: list(0) },
    ]);

    assert.deepEqual(click(4), unchanged);
    const firstIncrement = click(5);
    assert.deepEqual(setPropsIds(firstIncrement), [2, 4, 6, 7]);
    assert.deepEqual(plain(firstIncrement), {
        tag: 'patches',
        val: [
            { tag: 'set-props', val: [2, text('Count: 1', true)] },
            { tag: 'set-props', val: [4, button('Decrement', 'default', false, 'dash')] },
            { tag: 'set-props', val: [6, button('Reset', 'subtle', false)] },
            { tag: 'set-props', val: [7, list(1)] },
        ],
    });
    assert.deepEqual(setPropsIds(click(5)), [2, 7]);
    assert.deepEqual(world.handleEvent('counter', { tag: 'input-changed', val: [5, 'x'] }), unchanged);
    assert.deepEqual(click(999), unchanged);

    const rows = plain(world.listRows('counter', 7, 1, 10));
    assert.deepEqual(rows, [
        {
            index: 1,
            nodes: [
                {
                    id: ROW_NODE_ID_BASE,
                    props: { tag: 'flex', val: { direction: 'row', gap: 0, padding: 4, grow: false, align: 'center' } },
                },
                { id: ROW_NODE_ID_BASE + 1, parent: ROW_NODE_ID_BASE, props: text('Row 2 of 2') },
            ],
        },
    ]);
    assert.deepEqual(
        world.listRows('counter', 7, 0, 10).map((row) => row.index),
        [0, 1],
    );
    assert.deepEqual(world.listRows('counter', 7, 5, 10), []);

    assert.deepEqual(setPropsIds(click(6)), [2, 4, 6, 7]);
    assert.deepEqual(click(6), unchanged);
    assert.deepEqual(world.listRows('counter', 7, 0, 10), []);
    assert.equal(
        resultError(() => world.handleEvent('other', { tag: 'click', val: 5 })),
        'unknown surface "other"',
    );
    assert.equal(resultError(() => world.mount('other')), 'unknown surface "other"');

    click(5);
    const remounted = plain(world.mount('counter'));
    assert.deepEqual(
        remounted.map((node) => node.id),
        [8, 9, 10, 11, 12, 13, 14],
    );
    assert.deepEqual(remounted[1]?.props, text('Count: 1', true));
    assert.deepEqual(click(5), unchanged);
    assert.deepEqual(setPropsIds(click(12)), [9, 14]);
    assert.equal(world.listRows('counter', 14, 0, 10).length, 2);

    const logsBefore = host.logs.length;
    assert.deepEqual(world.listRows('counter', 7, 0, 10), []);
    assert.deepEqual(host.logs.slice(logsBefore), [
        { level: 'warn', message: 'list-rows for unknown list 7 on "counter"' },
    ]);
    assert.deepEqual(host.pushes, []);
}
