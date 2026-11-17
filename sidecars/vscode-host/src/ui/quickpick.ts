// `window.createQuickPick` / `showQuickPick` / `createInputBox` / `showInputBox` → transient
// surfaces announced with `fyx/surfacesChanged`. `fyx:ui@0.1.0` has no modal surface kind yet
// ("tab, modal later"), so they use `panel` at the bottom; the host should present them as a
// picker. The tree: an `input` (filter / value) above a virtualized `list` of items.
//   input-changed  → filter (QuickPick) or validate (InputBox)
//   input-submitted→ accept the active item / the value
//   row-click      → accept that item (or toggle it with canSelectMany)
// Cancelling is `fyx/surfaceClosed`, outside the WIT world.

import type { HostContext } from "../context.ts";
import { NotImplementedInFyx, notSupported } from "../coverage.ts";
import { node, props, ROW_NODE_ID_BASE, type ListRow, type Node, type NodeId, type UiEvent } from "../schema.ts";
import { EventEmitter, QuickPickItemKind } from "../shim/types.ts";
import { splitCodicons, type SurfaceModel } from "./surfaces.ts";

const ROOT = 1;
const TITLE = 2;
const INPUT = 3;
const MESSAGE = 4;
const BUTTONS = 5;
const BUTTON_BASE = 100;
const MAX_BUTTONS = 800;
const LIST_BASE = 1000;
const ROW_HEIGHT = 24;

let serial = 0;

export interface QuickPickItem {
  label: string;
  description?: string;
  detail?: string;
  kind?: number;
  picked?: boolean;
  alwaysShow?: boolean;
}

abstract class QuickInputBase implements SurfaceModel {
  readonly surface;
  readonly transient = true;
  protected ctx: HostContext;
  protected shown = false;
  protected hideEmitter = new EventEmitter<void>();
  protected acceptEmitter = new EventEmitter<void>();
  protected valueEmitter = new EventEmitter<string>();
  readonly onDidHide = this.hideEmitter.event;
  readonly onDidAccept = this.acceptEmitter.event;
  readonly onDidChangeValue = this.valueEmitter.event;
  protected buttonEmitter = new EventEmitter<QuickInputButton>();
  readonly onDidTriggerButton = this.buttonEmitter.event;
  /** A panel does not close when it loses focus, so `ignoreFocusOut` holds whatever it is set to. */
  ignoreFocusOut = false;
  protected state = { value: "", placeholder: "" };
  private chromeState: { title?: string; step?: number; totalSteps?: number; enabled: boolean; busy: boolean; buttons: readonly QuickInputButton[] } = {
    enabled: true,
    busy: false,
    buttons: [],
  };

  constructor(ctx: HostContext, kind: string) {
    this.ctx = ctx;
    this.surface = {
      id: `${ctx.ext.id}/${kind}/${++serial}`,
      title: "",
      kind: { panel: { position: "bottom", icon: "search", "default-size": 300 } } as const,
    };
  }

  get value(): string {
    return this.state.value;
  }
  set value(v: string) {
    this.state.value = v;
    this.update();
  }
  get title(): string | undefined {
    return this.chromeState.title;
  }
  set title(v: string | undefined) {
    this.chromeState.title = v;
    this.update();
  }
  get step(): number | undefined {
    return this.chromeState.step;
  }
  set step(v: number | undefined) {
    this.chromeState.step = v;
    this.update();
  }
  get totalSteps(): number | undefined {
    return this.chromeState.totalSteps;
  }
  set totalSteps(v: number | undefined) {
    this.chromeState.totalSteps = v;
    this.update();
  }
  /** While false, accepting (Enter, a row click) does nothing. */
  get enabled(): boolean {
    return this.chromeState.enabled;
  }
  set enabled(v: boolean) {
    this.chromeState.enabled = v;
  }
  get busy(): boolean {
    return this.chromeState.busy;
  }
  set busy(v: boolean) {
    this.chromeState.busy = v;
    this.update();
  }
  get buttons(): readonly QuickInputButton[] {
    return this.chromeState.buttons;
  }
  set buttons(v: readonly QuickInputButton[]) {
    if (v.length > MAX_BUTTONS) throw new Error(`QuickInput: at most ${MAX_BUTTONS} buttons`);
    this.chromeState.buttons = [...v];
    this.update();
  }
  get placeholder(): string {
    return this.state.placeholder;
  }
  set placeholder(v: string | undefined) {
    this.state.placeholder = v ?? "";
    this.update();
  }

  show(): void {
    if (this.shown) return;
    this.shown = true;
    (this.surface as { title: string }).title = this.title ?? this.state.placeholder ?? "";
    this.ctx.surfaces.add(this);
  }

  hide(): void {
    if (!this.shown) return;
    this.shown = false;
    this.ctx.surfaces.remove(this.surface.id);
    this.hideEmitter.fire();
  }

  dispose(): void {
    this.hide();
  }

  closed(): void {
    this.hide();
  }

  protected update(): void {
    if (this.shown) this.ctx.surfaces.invalidate(this.surface.id);
  }

  protected chrome(nodes: Node[], message?: { text: string; error: boolean }): void {
    nodes.push(node(ROOT, null, props.flex({ direction: "column", gap: 4, padding: 8, background: "elevated-surface" })));
    const title = [this.title, this.step && this.totalSteps ? `(${this.step}/${this.totalSteps})` : undefined].filter(Boolean).join(" ");
    if (title) nodes.push(node(TITLE, ROOT, props.text(title, { bold: true })));
    nodes.push(node(INPUT, ROOT, props.input(this.state.value, this.state.placeholder)));
    if (message?.text) nodes.push(node(MESSAGE, ROOT, props.text(message.text, { color: message.error ? "error" : "text-muted" })));
    if (this.chromeState.buttons.length > 0) {
      nodes.push(node(BUTTONS, ROOT, props.flex({ direction: "row", gap: 4 })));
      for (const [ix, button] of this.chromeState.buttons.entries()) {
        nodes.push(node(BUTTON_BASE + ix, BUTTONS, props.button(button.tooltip ?? "", { icon: buttonIcon(button), style: "subtle" })));
      }
    }
  }

  /** A click on one of `buttons`; returns whether the event was one. */
  protected handleButtonClick(event: UiEvent): boolean {
    if (!("click" in event)) return false;
    const button = this.chromeState.buttons[event.click - BUTTON_BASE];
    if (button) this.buttonEmitter.fire(button);
    return true;
  }

  abstract render(): Node[];
  abstract handleEvent(event: UiEvent): Promise<void> | void;
}

export class QuickPickImpl<T extends QuickPickItem> extends QuickInputBase {
  private itemsState: readonly T[] = [];
  private filtered: T[] = [];
  private generation = 0;
  private activeEmitter = new EventEmitter<readonly T[]>();
  private selectionEmitter = new EventEmitter<readonly T[]>();
  readonly onDidChangeActive = this.activeEmitter.event;
  readonly onDidChangeSelection = this.selectionEmitter.event;
  canSelectMany = false;
  matchOnDescription = false;
  matchOnDetail = false;
  private keepScroll = false;
  private sortLabels = true;
  activeItems: readonly T[] = [];
  selectedItems: readonly T[] = [];

  constructor(ctx: HostContext) {
    super(ctx, "quick-pick");
  }

  // Item buttons would sit inside list rows, whose node ids are row-local, so a click could not
  // name its row; they are not rendered and this event could never fire.
  get onDidTriggerItemButton(): never {
    this.ctx.coverage.missing("vscode.QuickPick.onDidTriggerItemButton");
    throw new NotImplementedInFyx("vscode.QuickPick.onDidTriggerItemButton", "item buttons are not rendered");
  }

  // Presentation hints the panel cannot honour: recorded in the coverage report, not silent.
  get keepScrollPosition(): boolean {
    return this.keepScroll;
  }
  set keepScrollPosition(v: boolean) {
    if (v) this.ctx.coverage.note("vscode.QuickPick.keepScrollPosition", "accepted, no effect: fyx:ui lists have no scroll position to keep");
    this.keepScroll = v;
  }
  get sortByLabel(): boolean {
    return this.sortLabels;
  }
  set sortByLabel(v: boolean) {
    if (v !== this.sortLabels) this.ctx.coverage.note("vscode.QuickPick.sortByLabel", "accepted, no effect: filtered items always keep their order");
    this.sortLabels = v;
  }

  get items(): readonly T[] {
    return this.itemsState;
  }
  set items(items: readonly T[]) {
    if (items.some((i) => ((i as { buttons?: unknown }).buttons as unknown[] | undefined)?.length)) {
      this.ctx.coverage.note("vscode.QuickPickItem.buttons", "not rendered: buttons inside list rows cannot be addressed (row-local node ids)");
    }
    this.itemsState = items;
    if (this.canSelectMany) this.selectedItems = items.filter((i) => i.picked);
    this.refilter();
  }

  override set value(v: string) {
    this.state.value = v;
    this.refilter();
  }
  override get value(): string {
    return this.state.value;
  }

  private refilter(): void {
    const q = this.state.value.toLowerCase();
    this.filtered = this.itemsState.filter((item) => {
      if (!q || item.alwaysShow) return true;
      if (item.kind === QuickPickItemKind.Separator) return false;
      const hay = [item.label, this.matchOnDescription ? item.description : "", this.matchOnDetail ? item.detail : ""].join(" ").toLowerCase();
      return fuzzyContains(hay, q);
    });
    this.activeItems = this.filtered.filter((i) => i.kind !== QuickPickItemKind.Separator).slice(0, 1);
    this.activeEmitter.fire(this.activeItems);
    this.generation++;
    this.update();
  }

  private get listId(): NodeId {
    return LIST_BASE + this.generation;
  }

  render(): Node[] {
    const nodes: Node[] = [];
    this.chrome(nodes, this.busy ? { text: "Loading…", error: false } : undefined);
    nodes.push(node(this.listId, ROOT, props.list(this.filtered.length, ROW_HEIGHT)));
    return nodes;
  }

  listRows(listNode: NodeId, start: number, count: number): ListRow[] {
    if (listNode !== this.listId) return [];
    return this.filtered.slice(start, start + count).map((item, k) => {
      const index = start + k;
      const base = ROW_NODE_ID_BASE;
      const nodes: Node[] = [node(base, null, props.flex({ direction: "row", gap: 6, align: "center" }))];
      if (item.kind === QuickPickItemKind.Separator) {
        nodes.push(node(base + 1, base, props.text(item.label, { color: "text-muted", bold: true })));
        return { index, nodes };
      }
      const { icon, label } = splitCodicons(item.label);
      if (this.canSelectMany) nodes.push(node(base + 1, base, props.icon(this.selectedItems.includes(item) ? "check" : "blank")));
      if (icon) nodes.push(node(base + 2, base, props.icon(icon)));
      nodes.push(node(base + 3, base, props.text(label)));
      if (item.description) nodes.push(node(base + 4, base, props.text(splitCodicons(item.description).label, { color: "text-muted" })));
      if (item.detail) nodes.push(node(base + 5, base, props.text(splitCodicons(item.detail).label, { color: "text-muted" })));
      return { index, nodes };
    });
  }

  handleEvent(event: UiEvent): void {
    if (this.handleButtonClick(event)) return;
    if ("input-changed" in event && event["input-changed"][0] === INPUT) {
      this.value = event["input-changed"][1];
      this.valueEmitter.fire(this.state.value);
    } else if (!this.enabled) {
      return;
    } else if ("input-submitted" in event && event["input-submitted"][0] === INPUT) {
      if (!this.canSelectMany) {
        this.selectedItems = this.activeItems;
        this.selectionEmitter.fire(this.selectedItems);
      }
      this.acceptEmitter.fire();
    } else if ("row-click" in event && event["row-click"][0] === this.listId) {
      const item = this.filtered[event["row-click"][1]];
      if (!item || item.kind === QuickPickItemKind.Separator) return;
      this.activeItems = [item];
      this.activeEmitter.fire(this.activeItems);
      if (this.canSelectMany) {
        this.selectedItems = this.selectedItems.includes(item) ? this.selectedItems.filter((i) => i !== item) : [...this.selectedItems, item];
        this.selectionEmitter.fire(this.selectedItems);
        this.generation++;
        this.update();
      } else {
        this.selectedItems = [item];
        this.selectionEmitter.fire(this.selectedItems);
        this.acceptEmitter.fire();
      }
    }
  }
}

type ValidationMessage = string | { message: string; severity: number } | undefined;

export class InputBoxImpl extends QuickInputBase {
  private selectionState: readonly [number, number] | undefined;
  private promptState: string | undefined;
  private validationState: ValidationMessage;

  constructor(ctx: HostContext) {
    super(ctx, "input-box");
  }

  get valueSelection(): readonly [number, number] | undefined {
    return this.selectionState;
  }
  set valueSelection(v: readonly [number, number] | undefined) {
    if (v) this.ctx.coverage.note("vscode.InputBox.valueSelection", "accepted, no effect: fyx:ui inputs carry no selection");
    this.selectionState = v;
  }

  get prompt(): string | undefined {
    return this.promptState;
  }
  set prompt(v: string | undefined) {
    this.promptState = v;
    this.update();
  }

  get validationMessage(): ValidationMessage {
    return this.validationState;
  }
  set validationMessage(v: ValidationMessage) {
    this.validationState = v;
    this.update();
  }

  get password(): boolean {
    return false;
  }
  // fyx:ui 0.1 has no masked input, so a secret typed here would show in plain text. Refuse it
  // until the `input` props gain a mask (0.2 candidate).
  set password(v: boolean) {
    if (!v) return;
    this.ctx.coverage.missing("vscode.InputBox.password");
    throw notSupported("vscode.InputBox.password", "masked input (password: true)");
  }

  render(): Node[] {
    const nodes: Node[] = [];
    const validation = typeof this.validationMessage === "string" ? this.validationMessage : this.validationMessage?.message;
    this.chrome(nodes, validation ? { text: validation, error: true } : this.busy ? { text: "Loading…", error: false } : { text: this.prompt ?? "", error: false });
    return nodes;
  }

  handleEvent(event: UiEvent): void {
    if (this.handleButtonClick(event)) return;
    if ("input-changed" in event && event["input-changed"][0] === INPUT) {
      this.state.value = event["input-changed"][1];
      this.valueEmitter.fire(this.state.value);
      this.update();
    } else if ("input-submitted" in event && event["input-submitted"][0] === INPUT && this.enabled) {
      this.state.value = event["input-submitted"][1];
      this.acceptEmitter.fire();
    }
  }
}

export interface QuickInputButton {
  iconPath: unknown;
  tooltip?: string;
}

function buttonIcon(button: QuickInputButton): string | null {
  const icon = button.iconPath;
  if (icon && typeof icon === "object" && typeof (icon as { id?: unknown }).id === "string") return (icon as { id: string }).id;
  // Image paths and URIs cannot be shown: fyx:ui icons are names only.
  return null;
}

function fuzzyContains(haystack: string, needle: string): boolean {
  if (haystack.includes(needle)) return true;
  let i = 0;
  for (const ch of haystack) if (ch === needle[i] && ++i === needle.length) return true;
  return false;
}

type Thenable<T> = T | PromiseLike<T>;

/** `window.showQuickPick`. */
export async function showQuickPick(
  ctx: HostContext,
  itemsOrPromise: Thenable<readonly (string | QuickPickItem)[]>,
  options: { placeHolder?: string; title?: string; canPickMany?: boolean; matchOnDescription?: boolean; matchOnDetail?: boolean; onDidSelectItem?: (item: unknown) => unknown } = {},
  token?: { isCancellationRequested: boolean; onCancellationRequested(l: () => void): unknown },
): Promise<unknown> {
  const qp = new QuickPickImpl<QuickPickItem & { original: unknown }>(ctx);
  qp.title = options.title;
  qp.placeholder = options.placeHolder;
  qp.canSelectMany = options.canPickMany ?? false;
  qp.matchOnDescription = options.matchOnDescription ?? false;
  qp.matchOnDetail = options.matchOnDetail ?? false;
  qp.busy = true;
  qp.show();
  const raw = await itemsOrPromise;
  qp.items = raw.map((i) => (typeof i === "string" ? { label: i, original: i } : { ...i, original: i }));
  qp.busy = false;
  return new Promise((resolve) => {
    let done = false;
    const finish = (value: unknown) => {
      if (done) return;
      done = true;
      resolve(value);
      qp.dispose();
    };
    qp.onDidChangeActive((items) => items[0] && options.onDidSelectItem?.(items[0].original));
    qp.onDidAccept(() => finish(qp.canSelectMany ? qp.selectedItems.map((i) => i.original) : qp.selectedItems[0]?.original));
    qp.onDidHide(() => finish(undefined));
    token?.onCancellationRequested(() => finish(undefined));
  });
}

/** `window.showInputBox`. */
export async function showInputBox(
  ctx: HostContext,
  options: { value?: string; prompt?: string; placeHolder?: string; title?: string; password?: boolean; validateInput?: (value: string) => unknown } = {},
  token?: { onCancellationRequested(l: () => void): unknown },
): Promise<string | undefined> {
  const box = new InputBoxImpl(ctx);
  box.password = options.password ?? false;
  box.title = options.title;
  box.prompt = options.prompt;
  box.placeholder = options.placeHolder;
  box.value = options.value ?? "";
  box.show();
  const validate = async (value: string) => {
    const result = (await options.validateInput?.(value)) as string | { message: string; severity: number } | undefined | null;
    box.validationMessage = result ?? undefined;
    return !result || (typeof result === "object" && result.severity !== 3);
  };
  return new Promise((resolve) => {
    let done = false;
    const finish = (value: string | undefined) => {
      if (done) return;
      done = true;
      resolve(value);
      box.dispose();
    };
    box.onDidChangeValue((v) => void validate(v));
    box.onDidAccept(async () => {
      if (await validate(box.value)) finish(box.value);
    });
    box.onDidHide(() => finish(undefined));
    token?.onCancellationRequested(() => finish(undefined));
  });
}
