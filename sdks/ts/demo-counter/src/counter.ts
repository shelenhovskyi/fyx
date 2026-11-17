// Fyx demo 1 in TypeScript: the counter panel of `extensions/fyx_demo_counter`, with the same
// behaviour contract (see that crate's doc and its test `behaves_as_documented`). Keep the two in
// step: `sdks/ts/test/counter-contract.ts` and `sdks/ts/host-check` replay that contract against
// this component.

import {
    ButtonProps,
    FlexProps,
    ListProps,
    Surface,
    SurfaceTree,
    TextProps,
    TreeError,
    listRows as buildListRows,
    log,
    registerExtension,
} from 'fyx-ui-sdk';
import type { Extension, ListRow, Node, NodeId, UiEvent, Update } from 'fyx-ui-sdk';

const SURFACE_ID = 'counter';
const ROW_HEIGHT = 24;

interface CounterNodes {
    label: NodeId;
    decrement: NodeId;
    increment: NodeId;
    reset: NodeId;
    list: NodeId;
}

function checkSurface(surfaceId: string): void {
    if (surfaceId !== SURFACE_ID) {
        throw new Error(`unknown surface ${JSON.stringify(surfaceId)}`);
    }
}

export class Counter implements Extension {
    private count = 0;
    private readonly tree = new SurfaceTree();
    private nodes: CounterNodes | undefined;

    private label(): TextProps {
        return new TextProps(`Count: ${this.count}`).bold();
    }

    private decrement(): ButtonProps {
        return new ButtonProps('Decrement').icon('dash').disabled(this.count === 0);
    }

    private increment(): ButtonProps {
        return new ButtonProps('Increment').icon('plus').style('filled');
    }

    private reset(): ButtonProps {
        return new ButtonProps('Reset').style('subtle').disabled(this.count === 0);
    }

    private list(): ListProps {
        return new ListProps(this.count, ROW_HEIGHT);
    }

    private sync(nodes: CounterNodes): Update {
        this.tree.setProps(nodes.label, this.label());
        this.tree.setProps(nodes.decrement, this.decrement());
        this.tree.setProps(nodes.increment, this.increment());
        this.tree.setProps(nodes.reset, this.reset());
        this.tree.setProps(nodes.list, this.list());
        return this.tree.takeUpdate();
    }

    surfaces(): Surface[] {
        return [Surface.panel(SURFACE_ID, 'Counter', 'right', 'hash', 320)];
    }

    mount(surfaceId: string): Node[] {
        checkSurface(surfaceId);
        let label = 0;
        let decrement = 0;
        let increment = 0;
        let reset = 0;
        let list = 0;
        const mounted = this.tree.mount((tree) => {
            tree.flex(FlexProps.column().gap(8).padding(8).grow(), (tree) => {
                label = tree.text(this.label());
                tree.flex(FlexProps.row().gap(4), (tree) => {
                    decrement = tree.button(this.decrement());
                    increment = tree.button(this.increment());
                    reset = tree.button(this.reset());
                });
                list = tree.list(this.list());
            });
        });
        this.nodes = { label, decrement, increment, reset, list };
        return mounted;
    }

    handleEvent(surfaceId: string, event: UiEvent): Update {
        checkSurface(surfaceId);
        const nodes = this.nodes;
        if (nodes === undefined) {
            throw new TreeError({ kind: 'not-mounted' });
        }
        if (event.tag !== 'click') {
            return { tag: 'unchanged' };
        }
        const id = event.val;
        if (id === nodes.increment) {
            this.count = Math.min(this.count + 1, 0xffff_ffff);
        } else if (id === nodes.decrement) {
            this.count = Math.max(this.count - 1, 0);
        } else if (id === nodes.reset) {
            this.count = 0;
        } else {
            return { tag: 'unchanged' };
        }
        return this.sync(nodes);
    }

    listRows(surfaceId: string, listNode: NodeId, start: number, count: number): ListRow[] {
        if (surfaceId !== SURFACE_ID || this.nodes?.list !== listNode) {
            log('warn', `list-rows for unknown list ${listNode} on ${JSON.stringify(surfaceId)}`);
            return [];
        }
        const total = this.count;
        return buildListRows(start, count, total, (index, tree) => {
            tree.flex(FlexProps.row().padding(4).align('center'), (tree) => {
                tree.text(`Row ${index + 1} of ${total}`);
            });
        });
    }
}

export const { surfaces, mount, handleEvent, listRows } = registerExtension(() => new Counter());
