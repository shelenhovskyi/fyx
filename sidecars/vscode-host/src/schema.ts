// JSON mirror of the frozen `fyx:ui@0.1.0` WIT `types` interface (crates/ui_ext/wit/fyx-ui.wit).
//
// Encoding (the serde default for `#[serde(rename_all = "kebab-case")]`, so a Rust model in
// `crates/ui_ext` can derive it unchanged):
//   * record      → JSON object, field names exactly as in WIT (kebab-case): `row-count`, `old-text`.
//   * enum        → kebab-case string: `"text-muted"`, `"elevated-surface"`.
//   * option<T>   → `null` or the value; always present.
//   * variant     → externally tagged `{ "<case>": payload }`; a case without payload is the bare
//                   string (`"unchanged"`). `%list` is `"list"`.
//   * tuple<A, B> → JSON array `[a, b]`.
//   * result<T,E> → the JSON-RPC `result`, or a JSON-RPC `error` whose `message` is `E`.
// Nothing here may be extended without a WIT change by the integrator (W0).

export type NodeId = number;

export type ColorToken =
  | "text"
  | "text-muted"
  | "text-accent"
  | "error"
  | "warning"
  | "success"
  | "border"
  | "surface"
  | "elevated-surface";

export const COLOR_TOKENS: readonly ColorToken[] = [
  "text",
  "text-muted",
  "text-accent",
  "error",
  "warning",
  "success",
  "border",
  "surface",
  "elevated-surface",
];

export type FlexDirection = "row" | "column";
export type Align = "start" | "center" | "end" | "stretch";
export type ButtonStyle = "default" | "filled" | "subtle" | "danger";

export interface FlexProps {
  direction: FlexDirection;
  gap: number;
  padding: number;
  grow: boolean;
  align: Align;
  background: ColorToken | null;
}

export interface TextProps {
  content: string;
  color: ColorToken | null;
  bold: boolean;
  monospace: boolean;
}

export interface ButtonProps {
  label: string;
  icon: string | null;
  style: ButtonStyle;
  disabled: boolean;
}

export interface InputProps {
  value: string;
  placeholder: string;
}

export interface ListProps {
  "row-count": number;
  "row-height": number;
}

export interface TableProps {
  columns: string[];
  "row-count": number;
}

export interface IconProps {
  name: string;
  color: ColorToken | null;
}

export interface DiffViewProps {
  "old-text": string;
  "new-text": string;
  language: string | null;
}

export type Props =
  | { flex: FlexProps }
  | { text: TextProps }
  | { button: ButtonProps }
  | { input: InputProps }
  | { list: ListProps }
  | { table: TableProps }
  | { icon: IconProps }
  | { markdown: string }
  | { "diff-view": DiffViewProps };

export interface Node {
  id: NodeId;
  parent: NodeId | null;
  props: Props;
}

export interface InsertOp {
  node: Node;
  before: NodeId | null;
}

export type Patch = { "set-props": [NodeId, Props] } | { insert: InsertOp } | { remove: NodeId };

export type Update = "unchanged" | { replace: Node[] } | { patches: Patch[] };

export interface ListRow {
  index: number;
  nodes: Node[];
}

export type UiEvent =
  | { click: NodeId }
  | { "input-changed": [NodeId, string] }
  | { "input-submitted": [NodeId, string] }
  | { "row-click": [NodeId, number] };

export type DockPosition = "left" | "right" | "bottom";

export interface PanelDescriptor {
  position: DockPosition;
  icon: string;
  "default-size": number;
}

export type StatusBarSide = "left" | "right";

export interface StatusItemDescriptor {
  side: StatusBarSide;
  priority: number;
}

export type SurfaceKind = { panel: PanelDescriptor } | { "status-item": StatusItemDescriptor };

export interface Surface {
  id: string;
  title: string;
  kind: SurfaceKind;
}

export type LogLevel = "trace" | "debug" | "info" | "warn" | "error";

// ---------------------------------------------------------------------------------------------
// Builders with the WIT defaults spelled out, so callers only name what differs.

export const props = {
  flex: (p: Partial<FlexProps> = {}): Props => ({
    flex: { direction: "column", gap: 0, padding: 0, grow: false, align: "stretch", background: null, ...p },
  }),
  text: (content: string, p: Partial<Omit<TextProps, "content">> = {}): Props => ({
    text: { content, color: null, bold: false, monospace: false, ...p },
  }),
  button: (label: string, p: Partial<Omit<ButtonProps, "label">> = {}): Props => ({
    button: { label, icon: null, style: "default", disabled: false, ...p },
  }),
  input: (value: string, placeholder = ""): Props => ({ input: { value, placeholder } }),
  list: (rowCount: number, rowHeight: number): Props => ({ list: { "row-count": rowCount, "row-height": rowHeight } }),
  icon: (name: string, color: ColorToken | null = null): Props => ({ icon: { name, color } }),
  markdown: (source: string): Props => ({ markdown: source }),
};

export function node(id: NodeId, parent: NodeId | null, p: Props): Node {
  return { id, parent, props: p };
}

export function propsKind(p: Props): string {
  return Object.keys(p)[0];
}

// ---------------------------------------------------------------------------------------------
// Validation: the invariants the WIT comments state. Used by tests and by the mock host.

const U32 = 0xffffffff;
const U16 = 0xffff;

export function validateNodes(nodes: readonly Node[], options: { singleRoot?: boolean } = {}): string[] {
  const errors: string[] = [];
  const seen = new Set<NodeId>();
  let roots = 0;
  for (const n of nodes) {
    if (!Number.isInteger(n.id) || n.id < 0 || n.id > U32) errors.push(`node id ${n.id} is not a u32`);
    if (seen.has(n.id)) errors.push(`duplicate node id ${n.id}`);
    if (n.parent === null) roots++;
    else if (!seen.has(n.parent)) errors.push(`node ${n.id}: parent ${n.parent} does not precede it`);
    seen.add(n.id);
    errors.push(...validateProps(n.props).map((e) => `node ${n.id}: ${e}`));
  }
  if (options.singleRoot && roots !== 1) errors.push(`expected exactly one root, found ${roots}`);
  return errors;
}

export function validateProps(p: Props): string[] {
  const keys = Object.keys(p);
  if (keys.length !== 1) return [`props must have exactly one case, got ${JSON.stringify(keys)}`];
  const errors: string[] = [];
  const v = (p as Record<string, unknown>)[keys[0]] as Record<string, unknown>;
  const token = (name: string, value: unknown) => {
    if (value !== null && !COLOR_TOKENS.includes(value as ColorToken)) errors.push(`${name}: unknown color token ${JSON.stringify(value)}`);
  };
  const u = (name: string, value: unknown, max: number) => {
    if (!Number.isInteger(value) || (value as number) < 0 || (value as number) > max) errors.push(`${name}: ${JSON.stringify(value)} out of range`);
  };
  const str = (name: string, value: unknown) => {
    if (typeof value !== "string") errors.push(`${name}: expected string`);
  };
  const bool = (name: string, value: unknown) => {
    if (typeof value !== "boolean") errors.push(`${name}: expected bool`);
  };
  switch (keys[0]) {
    case "flex":
      if (!["row", "column"].includes(v.direction as string)) errors.push("flex.direction");
      if (!["start", "center", "end", "stretch"].includes(v.align as string)) errors.push("flex.align");
      u("flex.gap", v.gap, U16);
      u("flex.padding", v.padding, U16);
      bool("flex.grow", v.grow);
      token("flex.background", v.background);
      break;
    case "text":
      str("text.content", v.content);
      token("text.color", v.color);
      bool("text.bold", v.bold);
      bool("text.monospace", v.monospace);
      break;
    case "button":
      str("button.label", v.label);
      if (v.icon !== null) str("button.icon", v.icon);
      if (!["default", "filled", "subtle", "danger"].includes(v.style as string)) errors.push("button.style");
      bool("button.disabled", v.disabled);
      break;
    case "input":
      str("input.value", v.value);
      str("input.placeholder", v.placeholder);
      break;
    case "list":
      u("list.row-count", v["row-count"], U32);
      u("list.row-height", v["row-height"], U16);
      break;
    case "table":
      if (!Array.isArray(v.columns)) errors.push("table.columns");
      u("table.row-count", v["row-count"], U32);
      break;
    case "icon":
      str("icon.name", v.name);
      token("icon.color", v.color);
      break;
    case "markdown":
      if (typeof v !== "string") errors.push("markdown: expected string");
      break;
    case "diff-view":
      str("diff-view.old-text", v["old-text"]);
      str("diff-view.new-text", v["new-text"]);
      break;
    default:
      errors.push(`unknown props case ${keys[0]}`);
  }
  return errors;
}

/** Applies an update to a retained tree, as the host does. Throws on an invalid patch. */
export function applyUpdate(tree: readonly Node[], update: Update): Node[] {
  if (update === "unchanged") return [...tree];
  if ("replace" in update) return [...update.replace];
  let nodes = [...tree];
  for (const patch of update.patches) {
    if ("set-props" in patch) {
      const [id, p] = patch["set-props"];
      const ix = nodes.findIndex((n) => n.id === id);
      if (ix < 0) throw new Error(`set-props: no node ${id}`);
      nodes[ix] = { ...nodes[ix], props: p };
    } else if ("remove" in patch) {
      const doomed = new Set([patch.remove]);
      if (!nodes.some((n) => n.id === patch.remove)) throw new Error(`remove: no node ${patch.remove}`);
      for (const n of nodes) if (n.parent !== null && doomed.has(n.parent)) doomed.add(n.id);
      nodes = nodes.filter((n) => !doomed.has(n.id));
    } else {
      const { node: n, before } = patch.insert;
      if (nodes.some((m) => m.id === n.id)) throw new Error(`insert: id ${n.id} already exists`);
      if (n.parent !== null && !nodes.some((m) => m.id === n.parent)) throw new Error(`insert: no parent ${n.parent}`);
      if (before === null) {
        // Siblings are ordered by list position, so appending keeps every invariant.
        nodes.push(n);
      } else {
        const at = nodes.findIndex((m) => m.id === before);
        if (at < 0 || nodes[at].parent !== n.parent) throw new Error(`insert: ${before} is not a sibling`);
        nodes.splice(at, 0, n);
      }
    }
  }
  return nodes;
}

/**
 * Turns `prev` into `next` with the fewest simple patches: `set-props` for changed nodes, `remove`
 * for vanished subtrees, `insert` for new nodes. Falls back to `replace` when a surviving node moved
 * (new parent or new sibling order), since the patch set has no move.
 */
export function diff(prev: readonly Node[], next: readonly Node[]): Update {
  const prevById = new Map(prev.map((n) => [n.id, n]));
  const nextById = new Map(next.map((n) => [n.id, n]));
  for (const n of next) {
    const old = prevById.get(n.id);
    if (old && old.parent !== n.parent) return { replace: [...next] };
  }
  const survivors = (id: NodeId) => prevById.has(id) && nextById.has(id);
  if (siblingOrder(prev, survivors) !== siblingOrder(next, survivors)) return { replace: [...next] };

  const patches: Patch[] = [];
  const removed = new Set<NodeId>();
  for (const n of prev) {
    if (nextById.has(n.id)) continue;
    if (n.parent !== null && removed.has(n.parent)) {
      removed.add(n.id);
      continue;
    }
    removed.add(n.id);
    patches.push({ remove: n.id });
  }
  for (const n of next) {
    const old = prevById.get(n.id);
    if (old) {
      if (JSON.stringify(old.props) !== JSON.stringify(n.props)) patches.push({ "set-props": [n.id, n.props] });
      continue;
    }
    // Insert before the next sibling that already exists (or was inserted earlier), else append.
    const siblings = next.filter((m) => m.parent === n.parent);
    const after = siblings.slice(siblings.indexOf(n) + 1).find((m) => prevById.has(m.id));
    patches.push({ insert: { node: n, before: after ? after.id : null } });
  }
  return patches.length === 0 ? "unchanged" : { patches };
}

function siblingOrder(nodes: readonly Node[], keep: (id: NodeId) => boolean): string {
  const groups = new Map<NodeId | null, NodeId[]>();
  for (const n of nodes) {
    if (!keep(n.id)) continue;
    const g = groups.get(n.parent) ?? [];
    g.push(n.id);
    groups.set(n.parent, g);
  }
  return [...groups].map(([p, ids]) => `${p}:${ids.join(",")}`).sort().join(";");
}
