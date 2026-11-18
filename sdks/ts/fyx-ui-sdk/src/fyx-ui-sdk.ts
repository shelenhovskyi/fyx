// Guest SDK for Fyx UI extensions written in TypeScript and compiled to a WASM component with
// `jco componentize` against the `fyx:ui@0.1.0` world. It mirrors `crates/ui_ext_sdk` (the Rust SDK):
// the same tree builder, props builders, `PatchSet`, `SurfaceTree` and list-row helpers, with the
// same checks, so both SDKs answer the host identically.
//
// Only erasable TypeScript is used (no enums, namespaces or parameter properties), so Node can run
// the sources directly with type stripping, and jco's bundler can compile them as they are.

import type {
    Align,
    ButtonStyle,
    ColorToken,
    DockPosition,
    ListRow,
    Node,
    NodeId,
    Patch,
    Props,
    StatusBarSide,
    Surface as SurfaceRecord,
    UiEvent,
    Update,
} from 'fyx:ui/types@0.1.0';
import { log as hostLog, pushUpdate as hostPushUpdate } from 'fyx:ui/host@0.1.0';
import type { LogLevel } from 'fyx:ui/host@0.1.0';

export type * as wit from 'fyx:ui/types@0.1.0';
export type {
    Align,
    ButtonStyle,
    ColorToken,
    DockPosition,
    FlexDirection,
    InsertOp,
    ListRow,
    Node,
    NodeId,
    Patch,
    Props,
    StatusBarSide,
    SurfaceKind,
    UiEvent,
    Update,
} from 'fyx:ui/types@0.1.0';
export type { LogLevel } from 'fyx:ui/host@0.1.0';
export type Surface = SurfaceRecord;

/** The `fyx:ui` WIT version this SDK is written against, as [major, minor, patch]. */
export const API_VERSION: readonly [number, number, number] = Object.freeze([0, 1, 0] as const);

/**
 * Name of the custom section the host reads the API version from before instantiating a
 * component. `jco componentize` cannot emit it, so `scripts/build-component.mjs` appends it.
 */
export const API_VERSION_SECTION = 'fyx:ui-api-version';

/** {@link API_VERSION} as stored in the section: three big-endian u16s, as in `zed:api-version`. */
export function apiVersionSectionBytes(): Uint8Array {
    const bytes = new Uint8Array(6);
    const view = new DataView(bytes.buffer);
    API_VERSION.forEach((part, index) => view.setUint16(index * 2, part, false));
    return bytes;
}

/**
 * First id handed out inside a list row. Rows are addressed by (list node, index) and rebuilt on
 * every scroll, so their ids are only unique within one row; starting here keeps them apart from
 * the surface's main tree, whose ids count up from 1.
 */
export const ROW_NODE_ID_BASE: NodeId = 0x8000_0000;

const U16_MAX = 0xffff;
const U32_MAX = 0xffff_ffff;
const S32_MIN = -0x8000_0000;
const S32_MAX = 0x7fff_ffff;

// The component model traps when it cannot lower a number into the WIT integer type, and a trap
// disables the extension. Checking where the number enters the SDK turns that into an exception
// the caller can see and `mount`/`handle-event` report as an error.
function checkInteger(name: string, value: number, min: number, max: number): number {
    if (!Number.isInteger(value) || value < min || value > max) {
        throw new RangeError(`${name} must be an integer in ${min}..=${max}, got ${value}`);
    }
    return value;
}

const checkU16 = (name: string, value: number): number => checkInteger(name, value, 0, U16_MAX);
const checkU32 = (name: string, value: number): number => checkInteger(name, value, 0, U32_MAX);

/** Writes a message to the host's log. */
export function log(level: LogLevel, message: string): void {
    hostLog(level, message);
}

/**
 * Sends an update the extension initiated itself for `surfaceId`. In 0.1 this only works during a
 * call into the extension (see docs/fyx/limits-and-gotchas.md). Produce it with
 * {@link SurfaceTree.takeUpdate} so the extension's mirror matches what the host draws.
 */
export function pushUpdate(surfaceId: string, update: Update): void {
    hostPushUpdate(surfaceId, update);
}

/** The text of a thrown value, for the `string` error of `mount` and `handle-event`. */
export function errorMessage(error: unknown): string {
    if (error instanceof Error) {
        return error.message;
    }
    return String(error);
}

// ---------------------------------------------------------------------------------------------
// Props builders

/** Something that converts into a `props` variant: every builder below, or a `Props` value. */
export interface IntoProps {
    intoProps(): Props;
}

export type PropsLike = Props | IntoProps;

export function intoProps(props: PropsLike): Props {
    return 'tag' in props ? cloneData(props) : props.intoProps();
}

// Builders mutate and return `this` for chaining; `intoProps` snapshots their fields, so changing a
// builder after handing it to the SDK never changes a node that was already built.

export class FlexProps implements IntoProps {
    private direction: 'row' | 'column';
    private gapValue = 0;
    private paddingValue = 0;
    private growValue = false;
    private alignValue: Align = 'start';
    private backgroundValue: ColorToken | undefined = undefined;

    private constructor(direction: 'row' | 'column') {
        this.direction = direction;
    }

    /** A column with no gap, no padding and children aligned to the start. */
    static column(): FlexProps {
        return new FlexProps('column');
    }

    /** A row with no gap, no padding and children aligned to the start. */
    static row(): FlexProps {
        return new FlexProps('row');
    }

    gap(gap: number): this {
        this.gapValue = checkU16('gap', gap);
        return this;
    }

    padding(padding: number): this {
        this.paddingValue = checkU16('padding', padding);
        return this;
    }

    grow(): this {
        this.growValue = true;
        return this;
    }

    align(align: Align): this {
        this.alignValue = align;
        return this;
    }

    background(background: ColorToken): this {
        this.backgroundValue = background;
        return this;
    }

    intoProps(): Props {
        return {
            tag: 'flex',
            val: {
                direction: this.direction,
                gap: this.gapValue,
                padding: this.paddingValue,
                grow: this.growValue,
                align: this.alignValue,
                background: this.backgroundValue,
            },
        };
    }
}

export class TextProps implements IntoProps {
    private content: string;
    private colorValue: ColorToken | undefined = undefined;
    private boldValue = false;
    private monospaceValue = false;

    constructor(content: string) {
        this.content = content;
    }

    color(color: ColorToken): this {
        this.colorValue = color;
        return this;
    }

    bold(): this {
        this.boldValue = true;
        return this;
    }

    monospace(): this {
        this.monospaceValue = true;
        return this;
    }

    intoProps(): Props {
        return {
            tag: 'text',
            val: {
                content: this.content,
                color: this.colorValue,
                bold: this.boldValue,
                monospace: this.monospaceValue,
            },
        };
    }
}

export class ButtonProps implements IntoProps {
    private label: string;
    private iconValue: string | undefined = undefined;
    private styleValue: ButtonStyle = 'default';
    private disabledValue = false;

    constructor(label: string) {
        this.label = label;
    }

    icon(icon: string): this {
        this.iconValue = icon;
        return this;
    }

    style(style: ButtonStyle): this {
        this.styleValue = style;
        return this;
    }

    disabled(disabled: boolean): this {
        this.disabledValue = disabled;
        return this;
    }

    intoProps(): Props {
        return {
            tag: 'button',
            val: {
                label: this.label,
                icon: this.iconValue,
                style: this.styleValue,
                disabled: this.disabledValue,
            },
        };
    }
}

export class InputProps implements IntoProps {
    private value: string;
    private placeholderValue = '';

    constructor(value: string) {
        this.value = value;
    }

    placeholder(placeholder: string): this {
        this.placeholderValue = placeholder;
        return this;
    }

    intoProps(): Props {
        return { tag: 'input', val: { value: this.value, placeholder: this.placeholderValue } };
    }
}

export class ListProps implements IntoProps {
    private rowCount: number;
    private rowHeight: number;

    constructor(rowCount: number, rowHeight: number) {
        this.rowCount = checkU32('row-count', rowCount);
        this.rowHeight = checkU16('row-height', rowHeight);
    }

    intoProps(): Props {
        return { tag: 'list', val: { rowCount: this.rowCount, rowHeight: this.rowHeight } };
    }
}

export class TableProps implements IntoProps {
    private columns: string[];
    private rowCount: number;

    constructor(columns: Iterable<string>, rowCount: number) {
        this.columns = Array.from(columns);
        this.rowCount = checkU32('row-count', rowCount);
    }

    intoProps(): Props {
        return { tag: 'table', val: { columns: [...this.columns], rowCount: this.rowCount } };
    }
}

export class IconProps implements IntoProps {
    private name: string;
    private colorValue: ColorToken | undefined = undefined;

    constructor(name: string) {
        this.name = name;
    }

    color(color: ColorToken): this {
        this.colorValue = color;
        return this;
    }

    intoProps(): Props {
        return { tag: 'icon', val: { name: this.name, color: this.colorValue } };
    }
}

export class DiffViewProps implements IntoProps {
    private oldText: string;
    private newText: string;
    private languageValue: string | undefined = undefined;

    constructor(oldText: string, newText: string) {
        this.oldText = oldText;
        this.newText = newText;
    }

    language(language: string): this {
        this.languageValue = language;
        return this;
    }

    intoProps(): Props {
        return {
            tag: 'diff-view',
            val: { oldText: this.oldText, newText: this.newText, language: this.languageValue },
        };
    }
}

export function markdownProps(source: string): Props {
    return { tag: 'markdown', val: source };
}

/** Builders for the `surface` records an extension returns from `surfaces`. */
export const Surface = {
    panel(
        id: string,
        title: string,
        position: DockPosition,
        icon: string,
        defaultSize: number,
    ): Surface {
        return {
            id,
            title,
            kind: {
                tag: 'panel',
                val: { position, icon, defaultSize: checkU32('default-size', defaultSize) },
            },
        };
    },

    statusItem(id: string, title: string, side: StatusBarSide, priority: number): Surface {
        return {
            id,
            title,
            kind: {
                tag: 'status-item',
                val: { side, priority: checkInteger('priority', priority, S32_MIN, S32_MAX) },
            },
        };
    },
};

// ---------------------------------------------------------------------------------------------
// Plain-data helpers. WIT values in JS are trees of objects, arrays, strings, numbers and booleans,
// where an absent option is `undefined` whether or not the key exists.

function cloneData<T>(value: T): T {
    if (Array.isArray(value)) {
        return value.map(cloneData) as T;
    }
    if (value !== null && typeof value === 'object') {
        const copy: Record<string, unknown> = {};
        for (const [key, field] of Object.entries(value)) {
            copy[key] = cloneData(field);
        }
        return copy as T;
    }
    return value;
}

/** Structural equality of WIT values, treating a missing key like `undefined`. */
export function dataEqual(left: unknown, right: unknown): boolean {
    if (left === right) {
        return true;
    }
    if (Array.isArray(left) || Array.isArray(right)) {
        return (
            Array.isArray(left) &&
            Array.isArray(right) &&
            left.length === right.length &&
            left.every((item, index) => dataEqual(item, right[index]))
        );
    }
    if (
        left === null ||
        right === null ||
        typeof left !== 'object' ||
        typeof right !== 'object'
    ) {
        return false;
    }
    const leftRecord = left as Record<string, unknown>;
    const rightRecord = right as Record<string, unknown>;
    const keys = new Set([...Object.keys(leftRecord), ...Object.keys(rightRecord)]);
    for (const key of keys) {
        if (!dataEqual(leftRecord[key], rightRecord[key])) {
            return false;
        }
    }
    return true;
}

export function isContainer(props: Props): boolean {
    return props.tag === 'flex';
}

// ---------------------------------------------------------------------------------------------
// Errors

/** The host's caps on a surface (`ui_ext::TreeLimits`), which {@link SurfaceTree} enforces. */
export interface TreeLimits {
    /** Patches in a `patches` update, or nodes in one list row. */
    readonly maxNodesPerUpdate: number;
    /** Nodes in the whole tree, so also in a `mount` or `replace`. */
    readonly maxNodes: number;
}

export const DEFAULT_TREE_LIMITS: TreeLimits = Object.freeze({
    maxNodesPerUpdate: 2048,
    maxNodes: 16384,
});

export type TreeErrorDetail =
    | { kind: 'not-mounted' }
    | { kind: 'unknown-node'; id: NodeId }
    | { kind: 'not-a-child'; parent: NodeId; child: NodeId }
    | { kind: 'is-root'; id: NodeId }
    | { kind: 'not-a-container'; node: NodeId; propsKind: Props['tag'] }
    | { kind: 'update-too-large'; count: number; max: number }
    | { kind: 'tree-too-large'; count: number; max: number }
    | { kind: 'invalid-row'; reason: string };

function describeTreeError(detail: TreeErrorDetail): string {
    switch (detail.kind) {
        case 'not-mounted':
            return 'surface is not mounted';
        case 'unknown-node':
            return `node ${detail.id} is not in the surface's tree`;
        case 'not-a-child':
            return `node ${detail.child} is not a child of node ${detail.parent}`;
        case 'is-root':
            return `node ${detail.id} is a root of the surface's tree`;
        case 'not-a-container':
            return `node ${detail.node} would be a ${detail.propsKind} with children; only flex nodes have children`;
        case 'update-too-large':
            return `update would carry ${detail.count} nodes or patches, the cap is ${detail.max}`;
        case 'tree-too-large':
            return `tree would have ${detail.count} nodes, the cap is ${detail.max}`;
        case 'invalid-row':
            return `invalid list row: ${detail.reason}`;
    }
}

/** An edit the host would reject, refused before it reaches the host. Messages match the Rust SDK. */
export class TreeError extends Error {
    readonly detail: TreeErrorDetail;

    constructor(detail: TreeErrorDetail) {
        super(describeTreeError(detail));
        this.name = 'TreeError';
        this.detail = detail;
    }
}

/**
 * The checks `ui_ext::Tree::from_row` applies to one list row, with the host's own messages: at
 * most `maxNodesPerUpdate` nodes, unique ids, parents before children, children only under flex
 * nodes, and exactly one root. Returns the reason the host would reject the row, if any.
 */
export function rowRejection(nodes: readonly Node[], limits: TreeLimits): string | undefined {
    if (nodes.length > limits.maxNodesPerUpdate) {
        return `update carries ${nodes.length} nodes or patches, the cap is ${limits.maxNodesPerUpdate}`;
    }
    if (nodes.length > limits.maxNodes) {
        return `tree would exceed ${limits.maxNodes} nodes`;
    }
    const kinds = new Map<NodeId, Props['tag']>();
    let roots = 0;
    for (const node of nodes) {
        if (kinds.has(node.id)) {
            return `duplicate node id ${node.id}`;
        }
        if (node.parent === undefined) {
            roots += 1;
        } else {
            const parentKind = kinds.get(node.parent);
            if (parentKind === undefined) {
                return `node ${node.id} names parent ${node.parent}, which is not an earlier node of the tree`;
            }
            if (parentKind !== 'flex') {
                return `node ${node.parent} is a ${parentKind} node and cannot have children`;
            }
        }
        kinds.set(node.id, node.props.tag);
    }
    if (roots !== 1) {
        return `a list row must have exactly one root node, found ${roots}`;
    }
    return undefined;
}

// ---------------------------------------------------------------------------------------------
// Tree building

/**
 * Hands out the node ids of one surface. Ids only ever increase, so an id is never reused for a
 * different node while the surface lives, as the `fyx:ui` contract requires.
 */
export class IdAllocator {
    private next: NodeId;

    constructor(first: NodeId = 1) {
        this.next = checkU32('first node id', first);
    }

    static startingAt(first: NodeId): IdAllocator {
        return new IdAllocator(first);
    }

    allocate(): NodeId {
        const id = this.next;
        if (id > U32_MAX) {
            throw new RangeError('node ids are exhausted (u32)');
        }
        this.next += 1;
        return id;
    }

    /** The id the next call to {@link IdAllocator.allocate} returns. */
    peek(): NodeId {
        return this.next;
    }
}

/**
 * Builds a flat node list from nested code. Flex nodes take a callback that builds their
 * children; every node gets a fresh id and its parent's id, and parents precede children.
 */
export class TreeBuilder {
    private readonly ids: IdAllocator;
    private readonly built: Node[] = [];
    private parent: NodeId | undefined;

    /** A builder whose top-level nodes have no parent, or are children of `parent`. */
    constructor(ids: IdAllocator, parent?: NodeId) {
        this.ids = ids;
        this.parent = parent;
    }

    /** A builder whose top-level nodes are children of `parent`, for inserting into a live tree. */
    static under(ids: IdAllocator, parent: NodeId): TreeBuilder {
        return new TreeBuilder(ids, parent);
    }

    /** Adds a node without children and returns its id. */
    node(props: PropsLike): NodeId {
        const id = this.ids.allocate();
        this.built.push({ id, parent: this.parent, props: intoProps(props) });
        return id;
    }

    /**
     * Adds a flex node and the children `buildChildren` adds, and returns the node's id. Only flex
     * nodes have children; the host rejects children under any other kind.
     */
    flex(props: FlexProps, buildChildren: (tree: TreeBuilder) => void): NodeId {
        const id = this.node(props);
        const outerParent = this.parent;
        this.parent = id;
        try {
            buildChildren(this);
        } finally {
            this.parent = outerParent;
        }
        return id;
    }

    column(buildChildren: (tree: TreeBuilder) => void): NodeId {
        return this.flex(FlexProps.column(), buildChildren);
    }

    row(buildChildren: (tree: TreeBuilder) => void): NodeId {
        return this.flex(FlexProps.row(), buildChildren);
    }

    text(props: TextProps | string): NodeId {
        return this.node(typeof props === 'string' ? new TextProps(props) : props);
    }

    button(props: ButtonProps | string): NodeId {
        return this.node(typeof props === 'string' ? new ButtonProps(props) : props);
    }

    input(props: InputProps): NodeId {
        return this.node(props);
    }

    /** A virtualized list; the host pulls its rows with `list-rows`. */
    list(props: ListProps): NodeId {
        return this.node(props);
    }

    table(props: TableProps): NodeId {
        return this.node(props);
    }

    icon(props: IconProps | string): NodeId {
        return this.node(typeof props === 'string' ? new IconProps(props) : props);
    }

    markdown(source: string): NodeId {
        return this.node(markdownProps(source));
    }

    diffView(props: DiffViewProps): NodeId {
        return this.node(props);
    }

    nodes(): readonly Node[] {
        return this.built;
    }

    finish(): Node[] {
        return this.built;
    }
}

/**
 * Builds one row of a virtualized list; its ids start at {@link ROW_NODE_ID_BASE}. `build` must add
 * exactly one top-level node, the row's root, and the row must pass the host's own checks, or the
 * host would reject the whole `list-rows` answer. Throws a {@link TreeError} otherwise.
 */
export function listRow(
    index: number,
    build: (tree: TreeBuilder) => void,
    limits: TreeLimits = DEFAULT_TREE_LIMITS,
): ListRow {
    const tree = new TreeBuilder(IdAllocator.startingAt(ROW_NODE_ID_BASE));
    build(tree);
    const nodes = tree.finish();
    const reason = rowRejection(nodes, limits);
    if (reason !== undefined) {
        throw new TreeError({ kind: 'invalid-row', reason });
    }
    return { index: checkU32('row index', index), nodes };
}

/** The row indices of a `list-rows(start, count)` request that exist in a list of `rowCount` rows. */
export function rowRange(
    start: number,
    count: number,
    rowCount: number,
): { start: number; end: number } {
    const end = Math.min(start + count, rowCount);
    return { start: Math.min(start, end), end };
}

/**
 * Answers a `list-rows(start, count)` request for a list of `rowCount` rows by calling `buildRow`
 * for each row that exists. A row the host would reject is logged and left out, so the other rows
 * still show.
 */
export function listRows(
    start: number,
    count: number,
    rowCount: number,
    buildRow: (index: number, tree: TreeBuilder) => void,
): ListRow[] {
    const range = rowRange(start, count, rowCount);
    const rows: ListRow[] = [];
    for (let index = range.start; index < range.end; index += 1) {
        try {
            rows.push(listRow(index, (tree) => buildRow(index, tree)));
        } catch (error) {
            log('error', `list row ${index} left out: ${errorMessage(error)}`);
        }
    }
    return rows;
}

// ---------------------------------------------------------------------------------------------
// Patches

/** An ordered batch of patches, turned into an `update` with {@link PatchSet.intoUpdate}. */
export class PatchSet {
    private readonly queued: Patch[] = [];

    setProps(id: NodeId, props: PropsLike): this {
        this.queued.push({ tag: 'set-props', val: [id, intoProps(props)] });
        return this;
    }

    /** Inserts one node under `node.parent`, before the sibling `before` or at the end. */
    insert(node: Node, before?: NodeId): this {
        this.queued.push({ tag: 'insert', val: { node, before } });
        return this;
    }

    /**
     * Inserts a subtree in parents-first order, such as one built with {@link TreeBuilder.under}.
     * Its top-level nodes (whose parent is outside `nodes`) go before `before`, keeping their order;
     * deeper nodes are appended to their new parents.
     */
    insertSubtree(nodes: readonly Node[], before?: NodeId): this {
        const subtreeIds = new Set(nodes.map((node) => node.id));
        for (const node of nodes) {
            const isTopLevel = node.parent === undefined || !subtreeIds.has(node.parent);
            this.insert(node, isTopLevel ? before : undefined);
        }
        return this;
    }

    /** Removes the node and its whole subtree. */
    remove(id: NodeId): this {
        this.queued.push({ tag: 'remove', val: id });
        return this;
    }

    isEmpty(): boolean {
        return this.queued.length === 0;
    }

    get length(): number {
        return this.queued.length;
    }

    patches(): readonly Patch[] {
        return this.queued;
    }

    /** `unchanged` when there is nothing to send, `patches` otherwise. */
    intoUpdate(): Update {
        if (this.queued.length === 0) {
            return { tag: 'unchanged' };
        }
        return { tag: 'patches', val: [...this.queued] };
    }
}

// ---------------------------------------------------------------------------------------------
// SurfaceTree

interface LiveNode {
    parent: NodeId | undefined;
    props: Props;
    children: NodeId[];
}

/**
 * The extension-side copy of one surface's tree, mirroring what the host has applied.
 *
 * Edits are recorded and queued as patches; {@link SurfaceTree.takeUpdate} hands the queue to the
 * host. A `setProps` that changes nothing is dropped, so re-setting every node after a state change
 * sends only what actually changed.
 *
 * The host applies an update atomically, rejects the whole batch if one patch breaks its rules,
 * and does not tell the extension. So every edit is checked against those rules first; an edit
 * that breaks one throws a {@link TreeError} and leaves the mirror and the queue as they were (ids
 * it allocated stay unused), so the queue is always a batch the host accepts.
 */
export class SurfaceTree {
    private readonly ids = new IdAllocator();
    private readonly treeLimits: TreeLimits;
    private mounted = false;
    private readonly live = new Map<NodeId, LiveNode>();
    private readonly rootIds: NodeId[] = [];
    private queue = new PatchSet();

    /** A tree checked against `limits` instead of the host's defaults. */
    constructor(limits: TreeLimits = DEFAULT_TREE_LIMITS) {
        this.treeLimits = limits;
    }

    limits(): TreeLimits {
        return this.treeLimits;
    }

    /**
     * Builds the surface's tree from scratch and returns it for `mount`. Pending patches are
     * dropped. Ids keep counting up from earlier trees, so an id from a previous mount never names
     * a node of the new tree. Throws, keeping the previous tree, if the new one has more nodes than
     * one update may carry.
     */
    mount(build: (tree: TreeBuilder) => void): Node[] {
        const tree = new TreeBuilder(this.ids);
        build(tree);
        const nodes = tree.finish();
        this.checkUpdateSize(nodes.length);
        this.checkTreeSize(nodes.length);

        this.live.clear();
        this.rootIds.length = 0;
        this.queue = new PatchSet();
        this.mounted = true;
        this.record(nodes, undefined);
        return nodes;
    }

    /** Like {@link SurfaceTree.mount}, but as a `replace` update. */
    replace(build: (tree: TreeBuilder) => void): Update {
        return { tag: 'replace', val: this.mount(build) };
    }

    isMounted(): boolean {
        return this.mounted;
    }

    contains(id: NodeId): boolean {
        return this.live.has(id);
    }

    props(id: NodeId): Props | undefined {
        const node = this.live.get(id);
        return node === undefined ? undefined : cloneData(node.props);
    }

    parent(id: NodeId): NodeId | undefined {
        return this.live.get(id)?.parent;
    }

    children(id: NodeId): readonly NodeId[] {
        return this.live.get(id)?.children ?? [];
    }

    roots(): readonly NodeId[] {
        return this.rootIds;
    }

    get size(): number {
        return this.live.size;
    }

    isEmpty(): boolean {
        return this.live.size === 0;
    }

    /** The whole live tree as a flat, parents-first node list, as the host holds it. */
    toNodes(): Node[] {
        const nodes: Node[] = [];
        const stack = [...this.rootIds].reverse();
        for (let id = stack.pop(); id !== undefined; id = stack.pop()) {
            const node = this.live.get(id);
            if (node !== undefined) {
                nodes.push({ id, parent: node.parent, props: cloneData(node.props) });
                stack.push(...[...node.children].reverse());
            }
        }
        return nodes;
    }

    /** Replaces a node's props. Returns whether they changed; unchanged props queue no patch. */
    setProps(id: NodeId, props: PropsLike): boolean {
        const next = intoProps(props);
        const node = this.node(id);
        if (dataEqual(node.props, next)) {
            return false;
        }
        SurfaceTree.checkContainer(id, node, next);
        this.checkUpdateSize(this.queue.length + 1);

        node.props = next;
        this.queue.setProps(id, next);
        return true;
    }

    /**
     * Records props the host already shows without queuing a patch, such as an input's value after
     * an `input-changed` event: echoing the user's text back could fight their typing.
     */
    setPropsFromHost(id: NodeId, props: PropsLike): void {
        const next = intoProps(props);
        const node = this.node(id);
        SurfaceTree.checkContainer(id, node, next);
        node.props = next;
    }

    /**
     * Builds a subtree under the flex node `parent`, before its child `before` or after its last
     * child, and returns the ids of the subtree's top-level nodes.
     */
    insert(
        parent: NodeId,
        before: NodeId | undefined,
        build: (tree: TreeBuilder) => void,
    ): NodeId[] {
        const position = this.insertPosition(parent, before);
        const tree = TreeBuilder.under(this.ids, parent);
        build(tree);
        const nodes = tree.finish();
        this.checkUpdateSize(this.queue.length + nodes.length);
        this.checkTreeSize(this.live.size + nodes.length);
        return this.commitInsert(parent, position, before, nodes);
    }

    /** Removes a node and its whole subtree. */
    remove(id: NodeId): void {
        this.node(id);
        this.checkUpdateSize(this.queue.length + 1);
        this.commitRemove(id);
    }

    /**
     * Replaces node `id` and its subtree with the nodes `build` adds, at the same position, and
     * returns their ids. The new nodes get new ids, so the host treats them as new: a list rebuilt
     * this way has all its rows requested again. Checked as a whole: on error nothing is removed.
     */
    rebuild(id: NodeId, build: (tree: TreeBuilder) => void): NodeId[] {
        const parent = this.node(id).parent;
        if (parent === undefined) {
            throw new TreeError({ kind: 'is-root', id });
        }
        const siblings = this.children(parent);
        const position = siblings.indexOf(id);
        if (position < 0) {
            throw new TreeError({ kind: 'not-a-child', parent, child: id });
        }
        const nextSibling = siblings[position + 1];
        const removed = this.subtreeSize(id);

        const tree = TreeBuilder.under(this.ids, parent);
        build(tree);
        const nodes = tree.finish();
        this.checkUpdateSize(this.queue.length + 1 + nodes.length);
        this.checkTreeSize(this.live.size - removed + nodes.length);

        this.commitRemove(id);
        return this.commitInsert(parent, position, nextSibling, nodes);
    }

    /** Removes every child of `parent`. */
    clearChildren(parent: NodeId): void {
        const children = [...this.node(parent).children];
        this.checkUpdateSize(this.queue.length + children.length);
        for (const child of children) {
            this.commitRemove(child);
        }
    }

    /** Patches queued since the last call, as an update for `handle-event` or {@link pushUpdate}. */
    takeUpdate(): Update {
        const queue = this.queue;
        this.queue = new PatchSet();
        return queue.intoUpdate();
    }

    pending(): PatchSet {
        return this.queue;
    }

    private node(id: NodeId): LiveNode {
        if (!this.mounted) {
            throw new TreeError({ kind: 'not-mounted' });
        }
        const node = this.live.get(id);
        if (node === undefined) {
            throw new TreeError({ kind: 'unknown-node', id });
        }
        return node;
    }

    private static checkContainer(id: NodeId, node: LiveNode, props: Props): void {
        if (node.children.length > 0 && !isContainer(props)) {
            throw new TreeError({ kind: 'not-a-container', node: id, propsKind: props.tag });
        }
    }

    private checkUpdateSize(count: number): void {
        const max = this.treeLimits.maxNodesPerUpdate;
        if (count > max) {
            throw new TreeError({ kind: 'update-too-large', count, max });
        }
    }

    private checkTreeSize(count: number): void {
        const max = this.treeLimits.maxNodes;
        if (count > max) {
            throw new TreeError({ kind: 'tree-too-large', count, max });
        }
    }

    private insertPosition(parent: NodeId, before: NodeId | undefined): number {
        const parentNode = this.node(parent);
        if (!isContainer(parentNode.props)) {
            throw new TreeError({
                kind: 'not-a-container',
                node: parent,
                propsKind: parentNode.props.tag,
            });
        }
        if (before === undefined) {
            return parentNode.children.length;
        }
        const position = parentNode.children.indexOf(before);
        if (position < 0) {
            throw new TreeError({ kind: 'not-a-child', parent, child: before });
        }
        return position;
    }

    private subtreeSize(id: NodeId): number {
        let count = 0;
        const stack = [id];
        for (let next = stack.pop(); next !== undefined; next = stack.pop()) {
            count += 1;
            stack.push(...this.children(next));
        }
        return count;
    }

    private commitInsert(
        parent: NodeId,
        position: number,
        before: NodeId | undefined,
        nodes: Node[],
    ): NodeId[] {
        const topLevel = nodes.filter((node) => node.parent === parent).map((node) => node.id);
        this.record(nodes, { parent, position });
        this.queue.insertSubtree(nodes, before);
        return topLevel;
    }

    private commitRemove(id: NodeId): void {
        const parentId = this.live.get(id)?.parent;
        const parent = parentId === undefined ? undefined : this.live.get(parentId);
        const siblings = parent === undefined ? this.rootIds : parent.children;
        const index = siblings.indexOf(id);
        if (index >= 0) {
            siblings.splice(index, 1);
        }

        const stack = [id];
        for (let next = stack.pop(); next !== undefined; next = stack.pop()) {
            const node = this.live.get(next);
            if (node !== undefined) {
                this.live.delete(next);
                stack.push(...node.children);
            }
        }
        this.queue.remove(id);
    }

    /**
     * Records parents-first `nodes`; top-level ones are placed at `insertAt.position` among the
     * children of `insertAt.parent`, or appended to the roots.
     */
    private record(
        nodes: readonly Node[],
        insertAt: { parent: NodeId; position: number } | undefined,
    ): void {
        let position = insertAt?.position;
        for (const node of nodes) {
            this.live.set(node.id, {
                parent: node.parent,
                props: cloneData(node.props),
                children: [],
            });
            const isTopLevel = insertAt !== undefined && insertAt.parent === node.parent;
            const parent = node.parent === undefined ? undefined : this.live.get(node.parent);
            const siblings = parent === undefined ? this.rootIds : parent.children;
            if (position !== undefined && isTopLevel) {
                const index = Math.min(position, siblings.length);
                siblings.splice(index, 0, node.id);
                position = index + 1;
            } else {
                siblings.push(node.id);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Exports

/**
 * A UI extension. One instance lives for the lifetime of the WASM instance; the host calls it from
 * a single thread, one call at a time.
 *
 * `mount` and `handleEvent` fail by throwing: the thrown value's message becomes the `string`
 * error of the WIT `result`.
 */
export interface Extension {
    /** Every surface the extension contributes; called once after instantiation. */
    surfaces(): Surface[];
    /** The initial tree of `surfaceId`, parents first. {@link SurfaceTree.mount} builds one. */
    mount(surfaceId: string): Node[];
    handleEvent?(surfaceId: string, event: UiEvent): Update;
    /** Rows `start..start + count` of the virtualized list `listNode`; see {@link listRows}. */
    listRows?(surfaceId: string, listNode: NodeId, start: number, count: number): ListRow[];
}

/** The functions of the `ui-extension` world, as `jco componentize` expects them to be exported. */
export interface UiExtensionExports {
    surfaces(): Surface[];
    mount(surfaceId: string): Node[];
    handleEvent(surfaceId: string, event: UiEvent): Update;
    listRows(surfaceId: string, listNode: NodeId, start: number, count: number): ListRow[];
}

/**
 * Wires an extension to the world's exports. Export the result's functions by name from the
 * component's entry module:
 *
 * ```ts
 * export const { surfaces, mount, handleEvent, listRows } = registerExtension(() => new Counter());
 * ```
 *
 * The extension is created on the first call, inside `surfaces` (2 s budget), as in the Rust SDK.
 */
export function registerExtension(create: () => Extension): UiExtensionExports {
    let extension: Extension | undefined;
    const instance = (): Extension => {
        extension ??= create();
        return extension;
    };
    // jco turns an exception thrown from an export returning `result` into its `err`, and any other
    // exception into a trap, which disables the extension until Fyx restarts.
    const asResult = <T>(call: () => T): T => {
        try {
            return call();
        } catch (error) {
            throw errorMessage(error);
        }
    };
    return {
        surfaces(): Surface[] {
            try {
                return instance().surfaces();
            } catch (error) {
                log('error', `surfaces() failed: ${errorMessage(error)}`);
                throw error;
            }
        },
        mount(surfaceId: string): Node[] {
            return asResult(() => instance().mount(surfaceId));
        },
        handleEvent(surfaceId: string, event: UiEvent): Update {
            return asResult(() => {
                const target = instance();
                return target.handleEvent === undefined
                    ? { tag: 'unchanged' }
                    : target.handleEvent(surfaceId, event);
            });
        },
        // `list-rows` has no error channel. A throwing implementation is logged and answers no rows
        // instead of trapping, since a trap would disable the whole extension.
        listRows(surfaceId: string, listNode: NodeId, start: number, count: number): ListRow[] {
            try {
                const target = instance();
                return target.listRows === undefined
                    ? []
                    : target.listRows(surfaceId, listNode, start, count);
            } catch (error) {
                log('error', `list-rows(${listNode}) failed: ${errorMessage(error)}`);
                return [];
            }
        },
    };
}
