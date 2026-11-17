// `window.createStatusBarItem` → two `status-item` surfaces per extension (left and right).
// `fyx:ui` declares surfaces once and has no per-item surface, so every item of one side is a
// button in one flex row, ordered by VS Code priority (higher sits further left).

import type { HostContext } from "../context.ts";
import { node, props, type Node, type StatusBarSide, type UiEvent } from "../schema.ts";
import { Disposable, StatusBarAlignment, ThemeColor } from "../shim/types.ts";
import { splitCodicons, type SurfaceModel } from "./surfaces.ts";

const ROOT = 1;
const ITEM_BASE = 100;

type Command = string | { command: string; title?: string; arguments?: unknown[] };

export class StatusBarItemImpl {
  readonly id: string;
  readonly alignment: number;
  readonly priority: number | undefined;
  readonly serial: number;
  private bar: StatusBar;
  private state: { name?: string; text: string; tooltip?: unknown; color?: unknown; backgroundColor?: ThemeColor; command?: Command; accessibilityInformation?: unknown } = { text: "" };
  visible = false;

  constructor(bar: StatusBar, serial: number, id: string, alignment: number, priority: number | undefined) {
    this.bar = bar;
    this.serial = serial;
    this.id = id;
    this.alignment = alignment;
    this.priority = priority;
  }

  get name() { return this.state.name; }
  set name(v) { this.set("name", v); }
  get text() { return this.state.text; }
  set text(v: string) { this.set("text", v ?? ""); }
  get tooltip() { return this.state.tooltip; }
  set tooltip(v) {
    if (v) this.bar.ctx.coverage.note("vscode.StatusBarItem.tooltip", "accepted but not shown: fyx:ui buttons have no tooltip");
    this.set("tooltip", v);
  }
  get color() { return this.state.color; }
  set color(v) {
    if (v) this.bar.ctx.coverage.note("vscode.StatusBarItem.color", "dropped: fyx:ui buttons take a style, not a colour");
    this.set("color", v);
  }
  get backgroundColor() { return this.state.backgroundColor; }
  set backgroundColor(v) { this.set("backgroundColor", v); }
  get command() { return this.state.command; }
  set command(v) { this.set("command", v); }
  get accessibilityInformation() { return this.state.accessibilityInformation; }
  set accessibilityInformation(v) { this.set("accessibilityInformation", v); }

  show(): void {
    this.visible = true;
    this.bar.changed(this);
  }
  hide(): void {
    this.visible = false;
    this.bar.changed(this);
  }
  dispose(): void {
    this.visible = false;
    this.bar.remove(this);
  }

  private set<K extends keyof StatusBarItemImpl["state"]>(key: K, value: StatusBarItemImpl["state"][K]): void {
    this.state[key] = value;
    if (this.visible) this.bar.changed(this);
  }
}

export class StatusBar {
  readonly ctx: HostContext;
  private items = new Set<StatusBarItemImpl>();
  private serial = 0;
  readonly left: StatusBarSurface;
  readonly right: StatusBarSurface;

  constructor(ctx: HostContext) {
    this.ctx = ctx;
    this.left = new StatusBarSurface(this, "left");
    this.right = new StatusBarSurface(this, "right");
    ctx.surfaces.add(this.left);
    ctx.surfaces.add(this.right);
  }

  create(idOrAlignment?: string | number, alignmentOrPriority?: number, priority?: number): StatusBarItemImpl {
    let id: string;
    let alignment: number;
    let prio: number | undefined;
    if (typeof idOrAlignment === "string") {
      id = idOrAlignment;
      alignment = alignmentOrPriority ?? StatusBarAlignment.Left;
      prio = priority;
    } else {
      id = `${this.ctx.ext.id}.${this.serial + 1}`;
      alignment = idOrAlignment ?? StatusBarAlignment.Left;
      prio = alignmentOrPriority;
    }
    const item = new StatusBarItemImpl(this, ++this.serial, id, alignment, prio);
    this.items.add(item);
    return item;
  }

  /** `window.setStatusBarMessage`: a temporary leftmost item. */
  message(text: string, hideAfter?: number | PromiseLike<unknown>): Disposable {
    const item = this.create(StatusBarAlignment.Left, Number.MAX_SAFE_INTEGER);
    item.text = text;
    item.show();
    const d = new Disposable(() => item.dispose());
    if (typeof hideAfter === "number") setTimeout(() => d.dispose(), hideAfter).unref?.();
    else if (hideAfter) void Promise.resolve(hideAfter).finally(() => d.dispose());
    return d;
  }

  visibleItems(side: StatusBarSide): StatusBarItemImpl[] {
    const alignment = side === "left" ? StatusBarAlignment.Left : StatusBarAlignment.Right;
    return [...this.items]
      .filter((i) => i.visible && i.alignment === alignment)
      .sort((a, b) => (b.priority ?? 0) - (a.priority ?? 0) || a.serial - b.serial);
  }

  find(serial: number): StatusBarItemImpl | undefined {
    for (const i of this.items) if (i.serial === serial) return i;
    return undefined;
  }

  changed(item: StatusBarItemImpl): void {
    const surface = item.alignment === StatusBarAlignment.Right ? this.right : this.left;
    this.ctx.surfaces.invalidate(surface.surface.id);
  }

  remove(item: StatusBarItemImpl): void {
    this.items.delete(item);
    this.changed(item);
  }
}

class StatusBarSurface implements SurfaceModel {
  readonly surface;
  private bar: StatusBar;
  private side: StatusBarSide;

  constructor(bar: StatusBar, side: StatusBarSide) {
    this.bar = bar;
    this.side = side;
    this.surface = {
      id: `${bar.ctx.ext.id}/status-${side}`,
      title: `${bar.ctx.ext.displayName} (status bar, ${side})`,
      kind: { "status-item": { side, priority: 0 } } as const,
    };
  }

  render(): Node[] {
    const nodes: Node[] = [node(ROOT, null, props.flex({ direction: "row", gap: 8, align: "center" }))];
    for (const item of this.bar.visibleItems(this.side)) {
      const { icon, label } = splitCodicons(item.text);
      if (!icon && !label) continue;
      const bg = item.backgroundColor instanceof ThemeColor ? item.backgroundColor.id : "";
      const style = /error/i.test(bg) ? "danger" : /(warning|prominent)/i.test(bg) ? "filled" : "subtle";
      nodes.push(node(ITEM_BASE + item.serial, ROOT, props.button(label, { icon, style })));
    }
    return nodes;
  }

  async handleEvent(event: UiEvent): Promise<void> {
    if (!("click" in event)) return;
    const item = this.bar.find(event.click - ITEM_BASE);
    const command = item?.command;
    if (!command) return;
    if (typeof command === "string") await this.bar.ctx.executeCommand(command);
    else await this.bar.ctx.executeCommand(command.command, ...(command.arguments ?? []));
  }
}
