// Unit tests of the SDK, run on its TypeScript sources. Most mirror a test of
// `crates/ui_ext_sdk/src/ui_ext_sdk.rs` with the same name.

import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { beforeEach, test } from 'node:test';

import {
    API_VERSION,
    ButtonProps,
    DiffViewProps,
    FlexProps,
    IconProps,
    IdAllocator,
    InputProps,
    ListProps,
    PatchSet,
    ROW_NODE_ID_BASE,
    Surface,
    SurfaceTree,
    TableProps,
    TextProps,
    TreeBuilder,
    TreeError,
    apiVersionSectionBytes,
    intoProps,
    listRow,
    listRows,
    log,
    pushUpdate,
    registerExtension,
    rowRange,
    rowRejection,
} from 'fyx-ui-sdk';
import type { Node, Props, Update } from 'fyx-ui-sdk';
import {
    appendCustomSection,
    readCustomSections,
    versionSectionBytes,
    witPackageVersion,
    withApiVersionSection,
} from '../scripts/api-version-section.mjs';
import { installHostMock } from './host-mock.ts';
import type { HostMock } from './host-mock.ts';
import { plain } from './counter-contract.ts';

const witFile = fileURLToPath(new URL('../../../crates/ui_ext/wit/fyx-ui.wit', import.meta.url));

let host: HostMock;
beforeEach(() => {
    host = installHostMock();
});

const text = (content: string): Props => intoProps(new TextProps(content));
const shape = (nodes: readonly Node[]) => nodes.map((node) => [node.id, node.parent ?? null]);

function treeErrorKind(call: () => unknown): string {
    try {
        call();
    } catch (error) {
        assert.ok(error instanceof TreeError, `expected a TreeError, got ${String(error)}`);
        return error.detail.kind;
    }
    assert.fail('expected a TreeError');
}

/** A minimal stand-in for the host's retained tree, applying updates with the host's rules. */
class HostTree {
    private nodes: Node[] = [];

    constructor(nodes: Node[]) {
        this.apply({ tag: 'replace', val: nodes });
    }

    apply(update: Update): void {
        if (update.tag === 'unchanged') {
            return;
        }
        if (update.tag === 'replace') {
            assert.equal(rowRejectionForest(update.val), undefined);
            this.nodes = plain(update.val);
            return;
        }
        let next = plain(this.nodes);
        for (const patch of update.val) {
            if (patch.tag === 'set-props') {
                const node = next.find((candidate) => candidate.id === patch.val[0]);
                assert.ok(node, `set-props on unknown node ${patch.val[0]}`);
                node.props = plain(patch.val[1]);
            } else if (patch.tag === 'remove') {
                const removed = new Set([patch.val]);
                for (const node of next) {
                    if (node.parent !== undefined && removed.has(node.parent)) {
                        removed.add(node.id);
                    }
                }
                assert.ok(next.some((node) => node.id === patch.val), `remove of unknown ${patch.val}`);
                next = next.filter((node) => !removed.has(node.id));
            } else {
                const { node, before } = patch.val;
                assert.ok(!next.some((existing) => existing.id === node.id), `duplicate ${node.id}`);
                const parent = next.find((candidate) => candidate.id === node.parent);
                if (node.parent !== undefined) {
                    assert.equal(parent?.props.tag, 'flex', `insert under non-flex ${node.parent}`);
                }
                next = HostTree.insertAt(next, plain(node), before);
            }
        }
        this.nodes = next;
    }

    // Keeps `nodes` in pre-order: a new node goes right before `before`'s subtree, or after the
    // last node of its parent's subtree.
    private static insertAt(nodes: Node[], node: Node, before: number | undefined): Node[] {
        const index =
            before !== undefined
                ? nodes.findIndex((candidate) => candidate.id === before)
                : HostTree.subtreeEnd(nodes, node.parent);
        assert.ok(index >= 0, `unknown sibling ${before}`);
        return [...nodes.slice(0, index), node, ...nodes.slice(index)];
    }

    private static subtreeEnd(nodes: Node[], parent: number | undefined): number {
        if (parent === undefined) {
            return nodes.length;
        }
        const inside = new Set([parent]);
        let end = nodes.findIndex((node) => node.id === parent) + 1;
        while (end < nodes.length) {
            const candidate = nodes[end];
            if (candidate?.parent === undefined || !inside.has(candidate.parent)) {
                break;
            }
            inside.add(candidate.id);
            end += 1;
        }
        return end;
    }

    toNodes(): Node[] {
        return this.nodes;
    }
}

function rowRejectionForest(nodes: Node[]): string | undefined {
    const ids = new Set<number>();
    for (const node of nodes) {
        if (ids.has(node.id)) {
            return `duplicate node id ${node.id}`;
        }
        if (node.parent !== undefined && !ids.has(node.parent)) {
            return `unknown parent ${node.parent}`;
        }
        ids.add(node.id);
    }
    return undefined;
}

function applyAndCompare(hostTree: HostTree, surface: SurfaceTree): void {
    hostTree.apply(surface.takeUpdate());
    assert.deepEqual(plain(hostTree.toNodes()), plain(surface.toNodes()));
}

test('builder flattens nested code parents first', () => {
    const ids = new IdAllocator();
    const tree = new TreeBuilder(ids);
    const leaves: number[] = [];
    const root = tree.column((tree) => {
        leaves.push(tree.text('title'));
        tree.row((tree) => {
            leaves.push(tree.button('a'));
            leaves.push(tree.button('b'));
        });
        leaves.push(tree.list(new ListProps(3, 20)));
    });
    const nodes = tree.finish();
    assert.deepEqual(shape(nodes), [
        [1, null],
        [2, 1],
        [3, 1],
        [4, 3],
        [5, 3],
        [6, 1],
    ]);
    assert.equal(root, 1);
    assert.deepEqual(leaves, [2, 4, 5, 6]);
    assert.deepEqual(nodes[1]?.props, text('title'));
    assert.equal(ids.peek(), 7);
});

test('builder under parent and sibling roots', () => {
    const tree = TreeBuilder.under(IdAllocator.startingAt(10), 3);
    tree.text('first');
    tree.column((tree) => {
        tree.text('nested');
    });
    assert.deepEqual(shape(tree.finish()), [
        [10, 3],
        [11, 3],
        [12, 11],
    ]);
});

test('props builders', () => {
    assert.deepEqual(plain(intoProps(FlexProps.row().gap(4).padding(8).grow().background('surface'))), {
        tag: 'flex',
        val: { direction: 'row', gap: 4, padding: 8, grow: true, align: 'start', background: 'surface' },
    });
    assert.deepEqual(plain(intoProps(new TextProps('a').color('error').bold().monospace())), {
        tag: 'text',
        val: { content: 'a', color: 'error', bold: true, monospace: true },
    });
    assert.deepEqual(plain(intoProps(new ButtonProps('b').icon('x').style('danger').disabled(true))), {
        tag: 'button',
        val: { label: 'b', icon: 'x', style: 'danger', disabled: true },
    });
    assert.deepEqual(intoProps(new InputProps('v').placeholder('p')), {
        tag: 'input',
        val: { value: 'v', placeholder: 'p' },
    });
    assert.deepEqual(intoProps(new TableProps(['a', 'b'], 2)), {
        tag: 'table',
        val: { columns: ['a', 'b'], rowCount: 2 },
    });
    assert.deepEqual(plain(intoProps(new IconProps('check').color('success'))), {
        tag: 'icon',
        val: { name: 'check', color: 'success' },
    });
    assert.deepEqual(plain(intoProps(new DiffViewProps('a', 'b').language('Rust'))), {
        tag: 'diff-view',
        val: { oldText: 'a', newText: 'b', language: 'Rust' },
    });
    assert.deepEqual(Surface.statusItem('s', 'S', 'left', -3).kind, {
        tag: 'status-item',
        val: { side: 'left', priority: -3 },
    });
});

test('builders snapshot their fields and reject numbers WIT cannot carry', () => {
    const builder = new TextProps('before');
    const tree = new TreeBuilder(new IdAllocator());
    tree.text(builder);
    builder.bold();
    assert.deepEqual(tree.nodes()[0]?.props, text('before'));

    assert.throws(() => FlexProps.row().gap(65536), RangeError);
    assert.throws(() => FlexProps.row().padding(-1), RangeError);
    assert.throws(() => new ListProps(1.5, 24), RangeError);
    assert.throws(() => new ListProps(2 ** 32, 24), RangeError);
    assert.throws(() => Surface.statusItem('s', 'S', 'left', 2 ** 31), RangeError);
});

test('patch set insert subtree only positions top-level nodes', () => {
    const tree = TreeBuilder.under(IdAllocator.startingAt(10), 1);
    tree.column((tree) => {
        tree.text('child');
    });
    tree.text('sibling');
    const patches = new PatchSet().insertSubtree(tree.finish(), 5).patches();
    assert.deepEqual(
        patches.map((patch) => (patch.tag === 'insert' ? [patch.val.node.id, patch.val.before ?? null] : null)),
        [
            [10, 5],
            [11, null],
            [12, 5],
        ],
    );
});

test('patch set into update', () => {
    assert.deepEqual(new PatchSet().intoUpdate(), { tag: 'unchanged' });
    const patches = new PatchSet();
    patches.remove(3).setProps(4, new TextProps('x'));
    assert.equal(patches.length, 2);
    assert.deepEqual(plain(patches.intoUpdate()), {
        tag: 'patches',
        val: [
            { tag: 'remove', val: 3 },
            { tag: 'set-props', val: [4, plain(text('x'))] },
        ],
    });
});

test('surface tree skips unchanged props', () => {
    const surface = new SurfaceTree();
    let label = 0;
    surface.mount((tree) => {
        tree.column((tree) => {
            label = tree.text('a');
        });
    });
    assert.equal(surface.setProps(label, new TextProps('a')), false);
    assert.deepEqual(surface.takeUpdate(), { tag: 'unchanged' });
    assert.equal(surface.setProps(label, new TextProps('b')), true);
    assert.equal(surface.setProps(label, new TextProps('b')), false);
    assert.deepEqual(plain(surface.takeUpdate()), {
        tag: 'patches',
        val: [{ tag: 'set-props', val: [label, plain(text('b'))] }],
    });
    // A `Props` value written with explicit `undefined` options equals the builder's output.
    assert.equal(
        surface.setProps(label, { tag: 'text', val: { content: 'b', color: undefined, bold: false, monospace: false } }),
        false,
    );
});

test('surface tree rejects edits before mount and on unknown nodes', () => {
    const surface = new SurfaceTree();
    assert.equal(treeErrorKind(() => surface.setProps(1, new TextProps('x'))), 'not-mounted');
    assert.equal(treeErrorKind(() => surface.remove(1)), 'not-mounted');
    surface.mount((tree) => {
        tree.text('root');
    });
    assert.equal(treeErrorKind(() => surface.setProps(99, new TextProps('x'))), 'unknown-node');
    assert.equal(treeErrorKind(() => surface.insert(99, undefined, () => {})), 'unknown-node');
    assert.equal(treeErrorKind(() => surface.rebuild(1, () => {})), 'is-root');
    assert.equal(
        new TreeError({ kind: 'not-a-container', node: 1, propsKind: 'text' }).message,
        'node 1 would be a text with children; only flex nodes have children',
    );
});

test('surface tree patches reproduce its tree', () => {
    const surface = new SurfaceTree();
    let list = 0;
    let header = 0;
    let body = 0;
    const mounted = surface.mount((tree) => {
        tree.column((tree) => {
            header = tree.row((tree) => {
                tree.text('title');
            });
            body = tree.column((tree) => {
                tree.text('one');
                tree.text('two');
            });
            list = tree.list(new ListProps(1, 20));
        });
    });
    const hostTree = new HostTree(mounted);

    const [inserted] = surface.insert(body, surface.children(body)[1], (tree) => {
        tree.row((tree) => {
            tree.button('between');
        });
    });
    surface.setProps(list, new ListProps(2, 20));
    applyAndCompare(hostTree, surface);

    assert.ok(inserted !== undefined);
    surface.remove(inserted);
    surface.insert(header, undefined, (tree) => {
        tree.icon('check');
    });
    applyAndCompare(hostTree, surface);

    const rebuilt = surface.rebuild(list, (tree) => {
        tree.list(new ListProps(2, 20));
    });
    assert.notDeepEqual(rebuilt, [list]);
    surface.clearChildren(body);
    applyAndCompare(hostTree, surface);
    assert.deepEqual(surface.children(body), []);
});

test('surface tree never reuses ids across mounts', () => {
    const surface = new SurfaceTree();
    const first = surface.mount((tree) => {
        tree.column((tree) => {
            tree.text('a');
        });
    });
    surface.setProps(2, new TextProps('pending'));
    const second = surface.mount((tree) => {
        tree.column((tree) => {
            tree.text('a');
        });
    });
    assert.deepEqual(first.map((node) => node.id), [1, 2]);
    assert.deepEqual(second.map((node) => node.id), [3, 4]);
    assert.deepEqual(surface.takeUpdate(), { tag: 'unchanged' });
    assert.equal(surface.contains(1), false);
});

test('list rows clamp and use row ids', () => {
    assert.deepEqual(rowRange(2, 10, 5), { start: 2, end: 5 });
    assert.deepEqual(rowRange(7, 3, 5), { start: 5, end: 5 });
    const rows = listRows(1, 10, 3, (index, tree) => {
        tree.row((tree) => {
            tree.text(`row ${index}`);
        });
    });
    assert.deepEqual(
        rows.map((row) => row.index),
        [1, 2],
    );
    assert.deepEqual(shape(rows[0]?.nodes ?? []), [
        [ROW_NODE_ID_BASE, null],
        [ROW_NODE_ID_BASE + 1, ROW_NODE_ID_BASE],
    ]);
});

test('rows the host would reject are left out', () => {
    const rows = listRows(0, 3, 3, (index, tree) => {
        tree.text(`root ${index}`);
        if (index === 1) {
            tree.text('second root');
        }
    });
    assert.deepEqual(
        rows.map((row) => row.index),
        [0, 2],
    );
    assert.deepEqual(host.logs, [
        {
            level: 'error',
            message:
                'list row 1 left out: invalid list row: a list row must have exactly one root node, found 2',
        },
    ]);
    assert.equal(
        treeErrorKind(() =>
            listRow(0, (tree) => {
                tree.flex(FlexProps.row(), () => {});
                tree.text('x');
            }),
        ),
        'invalid-row',
    );
    assert.equal(
        rowRejection(
            [
                { id: 1, props: text('a') },
                { id: 2, parent: 1, props: text('b') },
            ],
            { maxNodesPerUpdate: 10, maxNodes: 10 },
        ),
        'node 1 is a text node and cannot have children',
    );
});

test('host functions reach the host import', () => {
    log('info', 'hello');
    pushUpdate('surface', { tag: 'unchanged' });
    assert.deepEqual(host.logs, [{ level: 'info', message: 'hello' }]);
    assert.deepEqual(host.pushes, [{ surfaceId: 'surface', update: { tag: 'unchanged' } }]);
});

test('api version matches the wit package', () => {
    assert.deepEqual([...API_VERSION], witPackageVersion(witFile));
    assert.deepEqual([...apiVersionSectionBytes()], [0, 0, 0, 1, 0, 0]);
    assert.deepEqual([...versionSectionBytes([...API_VERSION])], [...apiVersionSectionBytes()]);
});

test('api version section is appended once', () => {
    const header = Uint8Array.from([0x00, 0x61, 0x73, 0x6d, 0x0d, 0x00, 0x01, 0x00]);
    const withSection = withApiVersionSection(header, [0, 1, 0]);
    assert.deepEqual(
        [...withSection.slice(header.length)],
        [0, 25, 18, ...new TextEncoder().encode('fyx:ui-api-version'), 0, 0, 0, 1, 0, 0],
    );
    assert.equal(withApiVersionSection(withSection, [0, 1, 0]), withSection);
    assert.throws(() => withApiVersionSection(withSection, [0, 2, 0]), /already has/);
    const duplicated = appendCustomSection(withSection, 'fyx:ui-api-version', versionSectionBytes([0, 1, 0]));
    assert.equal(readCustomSections(duplicated, 'fyx:ui-api-version').length, 2);
    assert.throws(() => withApiVersionSection(duplicated, [0, 1, 0]), /2 fyx:ui-api-version sections/);
    const coreModule = Uint8Array.from([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);
    assert.throws(() => withApiVersionSection(coreModule, [0, 1, 0]), /not a WASM component/);
});

test('register extension keeps one instance and maps errors', () => {
    let created = 0;
    const world = registerExtension(() => {
        created += 1;
        return {
            surfaces: () => [Surface.panel('echo', 'Echo', 'left', 'hash', 200)],
            mount(surfaceId) {
                throw new Error(`no ${surfaceId}`);
            },
            listRows() {
                throw new Error('broken rows');
            },
        };
    });
    assert.equal(world.surfaces()[0]?.id, 'echo');
    assert.equal(world.surfaces().length, 1);
    assert.equal(created, 1);
    assert.throws(() => world.mount('x'), (error) => error === 'no x');
    assert.deepEqual(world.handleEvent('echo', { tag: 'click', val: 1 }), { tag: 'unchanged' });
    assert.deepEqual(world.listRows('echo', 1, 0, 1), []);
    assert.deepEqual(host.logs, [{ level: 'error', message: 'list-rows(1) failed: broken rows' }]);
});
