// `contributes.views` + `window.registerTreeDataProvider` / `createTreeView` → one panel surface
// per view. The visible (expanded) part of the tree is flattened into a virtualized `list`; rows
// are pulled with `list-rows`. `fyx:ui` has no way to say "rows changed, row count did not", so a
// data change re-keys the list node (remove + insert with a new id), which makes the host drop its
// cached rows. Depth is shown with flex padding because the schema has no indent prop.

import type { HostContext } from "../context.ts";
import { NotImplementedInFyx } from "../coverage.ts";
import { node, props, type ListRow, type Node, type NodeId, type UiEvent } from "../schema.ts";
import { EventEmitter, ThemeIcon, TreeItemCollapsibleState, Uri, type Event } from "../shim/types.ts";
import type { SurfaceModel } from "./surfaces.ts";

const ROOT = 1;
const MESSAGE = 2;
const EMPTY = 3;
const HEADER = 4;
const HEADER_TITLE = 5;
const HEADER_DESCRIPTION = 6;
const HEADER_BADGE = 7;
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
  /** The title the surface was declared with; `fyx:ui` cannot rename a surface. */
  readonly declaredTitle: string;
  title: string;
  description?: string;
  badge?: { value: number; tooltip: string };
  readonly view: TreeViewApi;

  constructor(ctx: HostContext, viewId: string, title: string, position: "left" | "right" | "bottom") {
    this.ctx = ctx;
    this.viewId = viewId;
    this.declaredTitle = title;
    this.title = title;
    this.surface = {
      id: `${ctx.ext.id}/view/${viewId}`,
      title,
      kind: { panel: { position, icon: "list-tree", "default-size": 280 } } as const,
    };
    this.view = new TreeViewApi(this, ctx);
  }

  setProvider(provider: TreeDataProvider<unknown>): void {
    this.providerSubscription?.dispose();
    this.provider = provider;
    this.providerSubscription = provider.onDidChangeTreeData?.(() => void this.refresh());
    if (this.prepared) void this.refresh();
  }

  /** Drops `provider` if it is the current one (its registration or view was disposed). */
  clearProvider(provider: TreeDataProvider<unknown>): void {
    if (this.provider !== provider) return;
    this.providerSubscription?.dispose();
    this.providerSubscription = undefined;
    this.provider = undefined;
    this.expanded.clear();
    if (this.prepared) void this.refresh();
  }

  currentProvider(): TreeDataProvider<unknown> | undefined {
    return this.provider;
  }

  /** Expands every ancestor of `element` (and `element` itself `expandLevels` deep), then refreshes. */
  async reveal(element: unknown, expandLevels: number): Promise<void> {
    const provider = this.provider;
    if (!provider) throw new Error(`TreeView ${this.viewId}: reveal needs a registered TreeDataProvider`);
    if (!provider.getParent) throw new Error(`TreeView ${this.viewId}: reveal needs the TreeDataProvider to implement getParent`);
    const path = [element];
    for (let parent = await provider.getParent(element); parent !== undefined && parent !== null; parent = await provider.getParent(parent)) {
      if (path.length > 1000) throw new Error(`TreeView ${this.viewId}: getParent does not reach a root`);
      path.unshift(parent);
    }
    let parentKey = "";
    let parent: unknown = undefined;
    const toExpand: { element: unknown; key: string }[] = [];
    for (const [depth, current] of path.entries()) {
      const children = (await provider.getChildren(parent)) ?? [];
      const ix = children.indexOf(current);
      if (ix < 0) throw new Error(`TreeView ${this.viewId}: reveal: element is not a child of its getParent result`);
      const item = await provider.getTreeItem(current);
      const key = item.id ?? `${parentKey}/${labelOf(item)}#${ix}`;
      if (depth < path.length - 1) this.expanded.set(key, true);
      else if (expandLevels > 0 && (item.collapsibleState ?? TreeItemCollapsibleState.None) !== TreeItemCollapsibleState.None) {
        this.expanded.set(key, true);
        toExpand.push({ element: current, key });
      }
      parentKey = key;
      parent = current;
    }
    for (let level = 1; level < expandLevels && toExpand.length > 0; level++) {
      for (const { element: expanding, key: expandingKey } of toExpand.splice(0)) {
        for (const [ix, child] of ((await provider.getChildren(expanding)) ?? []).entries()) {
          const item = await provider.getTreeItem(child);
          if ((item.collapsibleState ?? TreeItemCollapsibleState.None) === TreeItemCollapsibleState.None) continue;
          const key = item.id ?? `${expandingKey}/${labelOf(item)}#${ix}`;
          this.expanded.set(key, true);
          toExpand.push({ element: child, key });
        }
      }
    }
    await this.refresh();
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
    const retitled = this.title !== this.declaredTitle;
    if (retitled || this.description || this.badge) {
      nodes.push(node(HEADER, ROOT, props.flex({ direction: "row", gap: 6, align: "center" })));
      if (retitled) nodes.push(node(HEADER_TITLE, HEADER, props.text(this.title, { bold: true })));
      if (this.description) nodes.push(node(HEADER_DESCRIPTION, HEADER, props.text(this.description, { color: "text-muted" })));
      if (this.badge) {
        const label = this.badge.tooltip ? `${this.badge.value} (${this.badge.tooltip})` : `${this.badge.value}`;
        nodes.push(node(HEADER_BADGE, HEADER, props.text(label, { color: "text-accent", bold: true })));
      }
    }
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
  private ctx: HostContext;
  private visibilityEmitter = new EventEmitter<{ visible: boolean }>();
  private selectionEmitter = new EventEmitter<{ selection: unknown[] }>();
  readonly expandEmitter = new EventEmitter<{ element: unknown }>();
  readonly collapseEmitter = new EventEmitter<{ element: unknown }>();
  readonly onDidChangeVisibility = this.visibilityEmitter.event;
  readonly onDidChangeSelection = this.selectionEmitter.event;
  readonly onDidExpandElement = this.expandEmitter.event;
  readonly onDidCollapseElement = this.collapseEmitter.event;
  visible = false;
  selection: unknown[] = [];

  constructor(surface: TreeViewSurface, ctx: HostContext) {
    this.surface = surface;
    this.ctx = ctx;
  }

  get message(): string | undefined {
    return this.surface.message;
  }
  set message(v: string | undefined) {
    this.surface.message = v;
    this.surface.changedChrome();
  }
  // Tree item checkboxes are not rendered, so this event could never fire.
  get onDidChangeCheckboxState(): never {
    this.ctx.coverage.missing("vscode.TreeView.onDidChangeCheckboxState");
    throw new NotImplementedInFyx("vscode.TreeView.onDidChangeCheckboxState", "tree item checkboxes are not rendered");
  }
  get title(): string {
    return this.surface.title;
  }
  set title(v: string | undefined) {
    this.surface.title = v ?? this.surface.declaredTitle;
    this.surface.changedChrome();
  }
  get description(): string | undefined {
    return this.surface.description;
  }
  set description(v: string | undefined) {
    this.surface.description = v;
    this.surface.changedChrome();
  }
  get badge(): { value: number; tooltip: string } | undefined {
    return this.surface.badge;
  }
  set badge(v: { value: number; tooltip: string } | undefined) {
    this.surface.badge = v ? { value: v.value, tooltip: v.tooltip ?? "" } : undefined;
    this.surface.changedChrome();
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

  /**
   * Expands the element's ancestors and selects it. `focus` is the host's to give, and `fyx:ui`
   * has no scroll-to-row yet, so the row is not scrolled into view.
   */
  async reveal(element: unknown, options?: { select?: boolean; focus?: boolean; expand?: boolean | number }): Promise<void> {
    if (options?.focus) this.ctx.coverage.note("vscode.TreeView.reveal", "focus is the host's to give; the row is not scrolled into view (no scroll-to-row in fyx:ui)");
    const expand = options?.expand === true ? 1 : typeof options?.expand === "number" ? Math.min(options.expand, 3) : 0;
    await this.surface.reveal(element, expand);
    if (options?.select !== false) this.select(element);
  }

  /** As in VS Code, disposing the view also disposes its data provider registration. */
  dispose(): void {
    const provider = this.surface.currentProvider();
    if (provider) this.surface.clearProvider(provider);
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
