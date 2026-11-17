// `window.createOutputChannel` → one bottom panel per extension listing its channels.
// The panel is declared up front because `fyx:ui` surfaces are static; it is empty until the
// extension creates a channel.

import type { HostContext } from "../context.ts";
import { node, props, type LogLevel as UiLogLevel, type Node } from "../schema.ts";
import { EventEmitter, LogLevel } from "../shim/types.ts";
import type { SurfaceModel } from "./surfaces.ts";

const ROOT = 1;
const EMPTY = 2;
/** Shown per channel; older text is dropped so one patch stays small. */
const MAX_CHARS = 16 * 1024;

export class OutputChannelImpl {
  readonly name: string;
  readonly serial: number;
  readonly languageId?: string;
  content = "";
  visible = false;
  private panel: OutputPanel;
  // LogOutputChannel
  logLevel: number = LogLevel.Info;
  private levelEmitter = new EventEmitter<number>();
  readonly onDidChangeLogLevel = this.levelEmitter.event;

  constructor(panel: OutputPanel, serial: number, name: string, languageId?: string) {
    this.panel = panel;
    this.serial = serial;
    this.name = name;
    this.languageId = languageId;
  }

  append(value: string): void {
    this.content = (this.content + value).slice(-MAX_CHARS);
    this.panel.changed();
  }
  appendLine(value: string): void {
    this.append(`${value}\n`);
  }
  replace(value: string): void {
    this.content = value.slice(-MAX_CHARS);
    this.panel.changed();
  }
  clear(): void {
    this.replace("");
  }
  show(): void {
    this.visible = true;
    this.panel.changed();
  }
  hide(): void {
    this.visible = false;
    this.panel.changed();
  }
  dispose(): void {
    this.panel.remove(this);
  }

  trace(message: string, ...args: unknown[]): void {
    this.log(LogLevel.Trace, "trace", message, args);
  }
  debug(message: string, ...args: unknown[]): void {
    this.log(LogLevel.Debug, "debug", message, args);
  }
  info(message: string, ...args: unknown[]): void {
    this.log(LogLevel.Info, "info", message, args);
  }
  warn(message: string, ...args: unknown[]): void {
    this.log(LogLevel.Warning, "warning", message, args);
  }
  error(error: string | Error, ...args: unknown[]): void {
    this.log(LogLevel.Error, "error", error instanceof Error ? (error.stack ?? error.message) : error, args);
  }

  private log(level: number, label: string, message: string, args: unknown[]): void {
    if (level < this.logLevel) return;
    const extra = args.length ? ` ${args.map((a) => (typeof a === "string" ? a : JSON.stringify(a))).join(" ")}` : "";
    this.appendLine(`${new Date().toISOString()} [${label}] ${message}${extra}`);
    const uiLevel: UiLogLevel = level <= LogLevel.Trace ? "trace" : level === LogLevel.Debug ? "debug" : level === LogLevel.Info ? "info" : level === LogLevel.Warning ? "warn" : "error";
    this.panel.ctx.log(uiLevel, `[${this.name}] ${message}${extra}`);
  }
}

export class OutputPanel implements SurfaceModel {
  readonly surface;
  readonly ctx: HostContext;
  private channels: OutputChannelImpl[] = [];
  private serial = 0;

  constructor(ctx: HostContext) {
    this.ctx = ctx;
    this.surface = {
      id: `${ctx.ext.id}/output`,
      title: `Output: ${ctx.ext.displayName}`,
      kind: { panel: { position: "bottom", icon: "output", "default-size": 240 } } as const,
    };
    ctx.surfaces.add(this);
  }

  create(name: string, languageIdOrOptions?: string | { log: true }): OutputChannelImpl {
    const channel = new OutputChannelImpl(this, ++this.serial, name, typeof languageIdOrOptions === "string" ? languageIdOrOptions : undefined);
    this.channels.push(channel);
    this.changed();
    return channel;
  }

  get all(): readonly OutputChannelImpl[] {
    return this.channels;
  }

  changed(): void {
    this.ctx.surfaces.invalidate(this.surface.id);
  }

  remove(channel: OutputChannelImpl): void {
    this.channels = this.channels.filter((c) => c !== channel);
    this.changed();
  }

  render(): Node[] {
    const nodes: Node[] = [node(ROOT, null, props.flex({ direction: "column", gap: 8, padding: 8, grow: true }))];
    if (this.channels.length === 0) nodes.push(node(EMPTY, ROOT, props.text("No output yet.", { color: "text-muted" })));
    for (const c of this.channels) {
      const base = 10 + c.serial * 3;
      nodes.push(node(base, ROOT, props.flex({ direction: "column", gap: 2 })));
      nodes.push(node(base + 1, base, props.text(c.name, { bold: true })));
      nodes.push(node(base + 2, base, props.text(c.content, { monospace: true })));
    }
    return nodes;
  }

  handleEvent(): void {}
}
