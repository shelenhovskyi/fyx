// Retained surfaces, as `fyx:ui` expects: the sidecar renders a model to a flat node list, the
// host keeps the last tree, and every later change travels as a diff — in the `handle-event`
// result when an event caused it, otherwise as a coalesced `push-update` request.
//
// A diff only applies to the tree the host really has. `push-update` is a request so the sidecar
// learns when the host refused one; until a full `replace` lands after that, the host's tree is
// unknown and no patch is computed against it.

import { diff, type ListRow, type Node, type NodeId, type Surface, type UiEvent, type Update } from "../schema.ts";

export interface SurfaceModel {
  readonly surface: Surface;
  /** Surfaces that come and go at runtime (QuickPick, InputBox) instead of being declared up front. */
  readonly transient?: boolean;
  /** Called before the first mount; may await extension activation or data providers. */
  prepare?(): Promise<void>;
  render(): Node[];
  handleEvent(event: UiEvent): Promise<void> | void;
  listRows?(listNode: NodeId, start: number, count: number): ListRow[] | Promise<ListRow[]>;
  /** The host closed the surface (QuickPick dismissed). */
  closed?(): void;
}

export interface SurfaceSink {
  /** Resolves once the host applied the update; rejects when it refused it. */
  pushUpdate(surfaceId: string, update: Update): Promise<unknown>;
  surfacesChanged(added: Surface[], removed: string[]): void;
}

export class SurfaceRegistry {
  private models = new Map<string, SurfaceModel>();
  private mounted = new Map<string, Node[]>();
  private dirty = new Set<string>();
  private inEvent = new Set<string>();
  /** Surfaces with a `push-update` the host has not answered yet; at most one each. */
  private inFlight = new Set<string>();
  /** Surfaces whose host tree is unknown after a refused update: the next update is a `replace`. */
  private resync = new Set<string>();
  private flushScheduled = false;
  private sink: SurfaceSink;

  constructor(sink: SurfaceSink) {
    this.sink = sink;
  }

  /** Surfaces declared at startup (`surfaces()` in the WIT world). */
  declared(): Surface[] {
    return [...this.models.values()].filter((m) => !m.transient).map((m) => m.surface);
  }

  get(id: string): SurfaceModel | undefined {
    return this.models.get(id);
  }

  add(model: SurfaceModel): void {
    this.models.set(model.surface.id, model);
    if (model.transient) this.sink.surfacesChanged([model.surface], []);
  }

  remove(id: string): void {
    const model = this.models.get(id);
    if (!model) return;
    this.models.delete(id);
    this.mounted.delete(id);
    this.dirty.delete(id);
    this.resync.delete(id);
    if (model.transient) this.sink.surfacesChanged([], [id]);
  }

  isMounted(id: string): boolean {
    return this.mounted.has(id);
  }

  async mount(id: string): Promise<Node[]> {
    const model = this.require(id);
    await model.prepare?.();
    const nodes = model.render();
    this.mounted.set(id, nodes);
    this.dirty.delete(id);
    this.resync.delete(id);
    return nodes;
  }

  async handleEvent(id: string, event: UiEvent): Promise<Update> {
    const model = this.require(id);
    this.inEvent.add(id);
    try {
      await model.handleEvent(event);
    } finally {
      this.inEvent.delete(id);
    }
    // The handler may have closed a transient surface.
    if (!this.models.has(id)) return "unchanged";
    return this.takeUpdate(id);
  }

  async listRows(id: string, listNode: NodeId, start: number, count: number): Promise<ListRow[]> {
    const model = this.require(id);
    if (!model.listRows) return [];
    return model.listRows(listNode, start, count);
  }

  closed(id: string): void {
    this.models.get(id)?.closed?.();
  }

  /** The model changed; the host hears about it on the next tick (or in the event result). */
  invalidate(id: string): void {
    if (!this.mounted.has(id)) return;
    this.dirty.add(id);
    if (this.inEvent.has(id)) return;
    this.scheduleFlush();
  }

  /** Sends pending updates now (tests and shutdown). */
  flush(): void {
    this.flushScheduled = false;
    for (const id of [...this.dirty]) {
      if (this.inEvent.has(id) || this.inFlight.has(id)) continue;
      const update = this.takeUpdate(id);
      if (update !== "unchanged") this.send(id, update);
    }
  }

  /** Whether the next update for `id` will be a full `replace` (the host refused the last one). */
  needsResync(id: string): boolean {
    return this.resync.has(id);
  }

  private scheduleFlush(): void {
    if (this.flushScheduled) return;
    this.flushScheduled = true;
    setImmediate(() => this.flush());
  }

  private send(id: string, update: Update): void {
    this.inFlight.add(id);
    const isReplace = update !== "unchanged" && "replace" in update;
    this.sink.pushUpdate(id, update).then(
      () => this.settle(id, false, isReplace),
      () => this.settle(id, true, isReplace),
    );
  }

  private settle(id: string, refused: boolean, wasReplace: boolean): void {
    this.inFlight.delete(id);
    if (!this.mounted.has(id)) return;
    if (refused) {
      this.resync.add(id);
      // Resend the whole tree now. A refused `replace` is not retried in a loop: the next change
      // to the model sends another one.
      if (!wasReplace) this.dirty.add(id);
    }
    if (this.dirty.has(id)) this.scheduleFlush();
  }

  private takeUpdate(id: string): Update {
    this.dirty.delete(id);
    const model = this.models.get(id);
    const prev = this.mounted.get(id);
    if (!model || !prev) return "unchanged";
    const next = model.render();
    this.mounted.set(id, next);
    if (this.resync.delete(id)) return { replace: [...next] };
    return diff(prev, next);
  }

  private require(id: string): SurfaceModel {
    const model = this.models.get(id);
    if (!model) throw Object.assign(new Error(`unknown surface ${id}`), { code: -32000 });
    return model;
  }
}

/** `$(icon-name) text` (VS Code codicon syntax) → leading icon and remaining label. */
export function splitCodicons(text: string): { icon: string | null; label: string } {
  let icon: string | null = null;
  const label = text
    .replace(/\$\(([a-z0-9-]+)(?:~spin)?\)/gi, (_m, name: string) => {
      icon ??= name;
      return "";
    })
    .replace(/\s+/g, " ")
    .trim();
  return { icon, label };
}
