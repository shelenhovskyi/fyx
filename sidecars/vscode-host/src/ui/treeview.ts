// `contributes.views` + `window.registerTreeDataProvider` / `createTreeView` → one panel surface
// per view. The visible (expanded) part of the tree is flattened into a virtualized `list`; rows
// are pulled with `list-rows`. `fyx:ui` has no way to say "rows changed, row count did not", so a
// data change re-keys the list node (remove + insert with a new id), which makes the host drop its
// cached rows. Depth is shown with flex padding because the schema has no indent prop.

import type { HostContext } from "../context.ts";
import { node, props, type ListRow, type Node, type NodeId, type UiEvent } from "../schema.ts";
import { EventEmitter, ThemeIcon, TreeItemCollapsibleState, Uri, type Event } from "../shim/types.ts";
import type { SurfaceModel } from "./surfaces.ts";

const ROOT = 1;
const MESSAGE = 2;
const EMPTY = 3;
const LIST_BASE = 1000;
const ROW_HEIGHT = 22;
const ROW_ID_BASE = 1_000_000;
const ROW_ID_STRIDE = 8;
const INDENT = 12;

interface TreeDataProvider<T> {
  getChildren(element?: T): T[] | undefined | null | PromiseLike<T[] | undefined | null>;
  getTreeItem(element: T): TreeItemLike | PromiseLike<TreeItemLike>;
  getParent?(element: T): T | undefined | null | PromiseLike<T | undefined | null>;
  resolveTreeItem?(item: TreeItemLike, element: T, token: unknown): TreeItemLike | PromiseLike<TreeItemLike>;
  onDidChangeTreeData?: Event<T | T[] | undefined | null | void>;
}

interface TreeItemLike {
  label?: string | { label: string };
  id?: string;
  description?: string | boolean;
  resourceUri?: Uri;
  iconPath?: unknown;
  collapsibleState?: number;
  command?: { command: string; arguments?: unknown[] };
  contextValue?: string;
}

interface Row {
  element: unknown;
  item: TreeItemLike;
  depth: number;
  key: string;
  expandable: boolean;
  expanded: boolean;
}

export class TreeViewSurface implements SurfaceModel {
  readonly surface;
  readonly viewId: string;
  private ctx: HostContext;
  private provider?: TreeDataProvider<unknown>;
  private providerSubscription?: { dispose(): unknown };
  private rows: Row[] = [];
  private expanded = new Map<string, boolean>();
  private listGeneration = 0;
  private prepared = false;
  private refreshing?: Promise<void>;
  message?: string;
  title: string;
  description?: string;
  readonly view: TreeViewApi;

  constructor(ctx: HostContext, viewId: string, title: string, position: "left" | "right" | "bottom") {
    this.ctx = ctx;
    this.viewId = viewId;
    this.title = title;
    this.surface = {
      id: `${ctx.ext.id}/view/${viewId}`,
      title,
      kind: { panel: { position, icon: "list-tree", "default-size": 280 } } as const,
    };
    this.view = new TreeViewApi(this);
  }

  setProvider(provider: TreeDataProvider<unknown>): void {
    this.providerSubscription?.dispose();
    this.provider = provider;
    this.providerSubscription = provider.onDidChangeTreeData?.(() => void this.refresh());
    if (this.prepared) void this.refresh();
  }

  async prepare(): Promise<void> {
    await this.ctx.activate(`onView:${this.viewId}`);
    this.prepared = true;
    await this.refresh();
    this.view.setVisible(true);
  }

  /** Re-reads the visible part of the tree from the provider. */
  refresh(): Promise<void> {
    const run = async () => {
      const rows: Row[] = [];
      if (this.provider) await this.collect(undefined, 0, "", rows);
      this.rows = rows;
      this.listGeneration++;
      this.ctx.surfaces.invalidate(this.surface.id);
    };
    // Serialize refreshes; a refresh requested while one runs starts after it.
    const next = (this.refreshing ?? Promise.resolve()).then(run, run);
    this.refreshing = next.finally(() => {
      if (this.refreshing === next) this.refreshing = undefined;
    });
    return next.catch((err) => this.ctx.coverage.error(`tree view ${this.viewId}: ${(err as Error).message}`));
  }

  private async collect(parent: unknown, depth: number, parentKey: string, out: Row[]): Promise<void> {
    const children = (await this.provider!.getChildren(parent)) ?? [];
    for (const [ix, element] of children.entries()) {
      const item = await this.provider!.getTreeItem(element);
      const label = labelOf(item);
      const key = item.id ?? `${parentKey}/${label}#${ix}`;
      const state = item.collapsibleState ?? TreeItemCollapsibleState.None;
      const expandable = state !== TreeItemCollapsibleState.None;
      const expanded = expandable && (this.expanded.get(key) ?? state === TreeItemCollapsibleState.Expanded);
      out.push({ element, item, depth, key, expandable, expanded });
      if (expanded) await this.collect(element, depth + 1, key, out);
      if (out.length > 10_000) return;
    }
  }

  private get listId(): NodeId {
    return LIST_BASE + this.listGeneration;
  }

  render(): Node[] {
    const nodes: Node[] = [node(ROOT, null, props.flex({ direction: "column", grow: true }))];
    if (this.message) nodes.push(node(MESSAGE, ROOT, props.text(this.message, { color: "text-muted" })));
    if (!this.provider) {
      nodes.push(node(EMPTY, ROOT, props.text(`No data provider registered for view ${this.viewId}.`, { color: "text-muted" })));
      return nodes;
    }
    nodes.push(node(this.listId, ROOT, props.list(this.rows.length, ROW_HEIGHT)));
    return nodes;
  }

  listRows(listNode: NodeId, start: number, count: number): ListRow[] {
    if (listNode !== this.listId) return [];
    return this.rows.slice(start, start + count).map((row, k) => {
      const index = start + k;
      const base = ROW_ID_BASE + index * ROW_ID_STRIDE;
      const nodes: Node[] = [node(base, null, props.flex({ direction: "row", gap: 4, padding: row.depth * INDENT, align: "center" }))];
      nodes.push(node(base + 1, base, props.icon(row.expandable ? (row.expanded ? "chevron-down" : "chevron-right") : "blank")));
      const icon = iconName(row.item.iconPath);
      if (icon) nodes.push(node(base + 2, base, props.icon(icon)));
      nodes.push(node(base + 3, base, props.text(labelOf(row.item))));
      const description = row.item.description;
      if (typeof description === "string" && description) nodes.push(node(base + 4, base, props.text(description, { color: "text-muted" })));
      return { index, nodes };
    });
  }

  async handleEvent(event: UiEvent): Promise<void> {
    if (!("row-click" in event)) return;
    const [list, index] = event["row-click"];
    const row = list === this.listId ? this.rows[index] : undefined;
    if (!row) return;
    this.view.select(row.element);
    if (row.expandable) {
      this.expanded.set(row.key, !row.expanded);
      (row.expanded ? this.view.collapseEmitter : this.view.expandEmitter).fire({ element: row.element });
      await this.refresh();
    } else if (row.item.command) {
      await this.ctx.executeCommand(row.item.command.command, ...(row.item.command.arguments ?? []));
    }
  }

  closed(): void {
    this.view.setVisible(false);
  }

  changedChrome(): void {
    this.ctx.surfaces.invalidate(this.surface.id);
  }
}

/** The `TreeView<T>` object `window.createTreeView` returns. */
export class TreeViewApi {
  private surface: TreeViewSurface;
  private visibilityEmitter = new EventEmitter<{ visible: boolean }>();
  private selectionEmitter = new EventEmitter<{ selection: unknown[] }>();
  readonly expandEmitter = new EventEmitter<{ element: unknown }>();
  readonly collapseEmitter = new EventEmitter<{ element: unknown }>();
  readonly onDidChangeVisibility = this.visibilityEmitter.event;
  readonly onDidChangeSelection = this.selectionEmitter.event;
  readonly onDidExpandElement = this.expandEmitter.event;
  readonly onDidCollapseElement = this.collapseEmitter.event;
  readonly onDidChangeCheckboxState = new EventEmitter<unknown>().event;
  visible = false;
  selection: unknown[] = [];
  badge: unknown;

  constructor(surface: TreeViewSurface) {
    this.surface = surface;
  }

  get message(): string | undefined {
    return this.surface.message;
  }
  set message(v: string | undefined) {
    this.surface.message = v;
    this.surface.changedChrome();
  }
  get title(): string {
    return this.surface.title;
  }
  set title(v: string) {
    this.surface.title = v;
  }
  get description(): string | undefined {
    return this.surface.description;
  }
  set description(v: string | undefined) {
    this.surface.description = v;
  }

  setVisible(visible: boolean): void {
    if (this.visible === visible) return;
    this.visible = visible;
    this.visibilityEmitter.fire({ visible });
  }

  select(element: unknown): void {
    this.selection = [element];
    this.selectionEmitter.fire({ selection: this.selection });
  }

  async reveal(element: unknown, options?: { select?: boolean }): Promise<void> {
    if (options?.select !== false) this.select(element);
  }

  dispose(): void {
    this.visibilityEmitter.dispose();
    this.selectionEmitter.dispose();
  }
}

function labelOf(item: TreeItemLike): string {
  if (typeof item.label === "string") return item.label;
  if (item.label && typeof item.label === "object") return item.label.label;
  if (item.resourceUri) return item.resourceUri.path.split("/").pop() ?? "";
  return "";
}

function iconName(iconPath: unknown): string | null {
  if (iconPath instanceof ThemeIcon) return iconPath.id;
  if (iconPath && typeof iconPath === "object" && typeof (iconPath as { id?: unknown }).id === "string") return (iconPath as { id: string }).id;
  // File paths and URIs are images; fyx:ui icons are names only.
  return null;
}
