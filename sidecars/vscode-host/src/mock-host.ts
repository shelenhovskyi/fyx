// A stand-in for the Fyx editor on the other end of the sidecar's JSON-RPC connection. It records
// every message in both directions, keeps the retained `fyx:ui` trees the way the real host would
// (applying `push-update` patches and checking every node and patch against the schema), mirrors
// buffers so `vscode/applyEdit` can be verified, and answers the sidecar's requests with scripted
// defaults. Used by the tests and by scripts/run-coverage.ts.

import { spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ExtensionHost, type InitializeParams, type InitializeResult } from "./host.ts";
import { ErrorCodes, RpcError, RpcPeer, pairedPeers, type RpcMessage } from "./rpc.ts";
import { applyUpdate, validateNodes, validateProps, type ListRow, type Node, type Surface, type UiEvent, type Update } from "./schema.ts";

export interface TrafficEntry {
  direction: "host→sidecar" | "sidecar→host";
  message: RpcMessage;
}

export interface MockHostOptions {
  /** Pick this item index when a QuickPick appears (false: leave it open). */
  autoPick?: number | false;
  /** Submit this value when an InputBox appears (false: leave it open). */
  autoInput?: string | false;
  /** Answer `vscode/showMessage` with this item index (default: dismiss). */
  messageChoice?: number;
}

interface WireEdit {
  range: { start: { line: number; character: number }; end: { line: number; character: number } };
  newText: string;
}

export class MockHost {
  readonly rpc: RpcPeer;
  readonly traffic: TrafficEntry[] = [];
  readonly trees = new Map<string, Node[]>();
  readonly surfaces = new Map<string, Surface>();
  readonly schemaErrors: string[] = [];
  /** Test hook: refuse the next `push-update` for this surface, as a host whose tree diverged would. */
  refuseNextPush: string | undefined;
  /** Test hook: answer every `vscode/applyEdit` with `false`. */
  refuseEdits = false;
  readonly logs: { level: string; message: string }[] = [];
  readonly messages: { severity: string; message: string; items: string[] }[] = [];
  readonly diagnostics = new Map<string, unknown[]>();
  readonly decorationTypes = new Map<string, unknown>();
  readonly decorations = new Map<string, unknown[]>();
  readonly hostCommands: { command: string; args: unknown[] }[] = [];
  readonly providers = new Map<number, unknown>();
  readonly buffers = new Map<string, { text: string; version: number }>();
  readonly contextKeys = new Map<string, unknown>();
  readonly editorOptions = new Map<string, unknown>();
  readonly selections = new Map<string, unknown[]>();
  options: MockHostOptions;
  private child?: ChildProcess;
  private stderr = "";
  private quickInputs: Promise<unknown>[] = [];
  /** Transient surfaces the sidecar closed; a late update for one of them is ignored, not an error. */
  private closedSurfaces = new Set<string>();

  private constructor(rpc: RpcPeer, options: MockHostOptions) {
    this.rpc = rpc;
    this.options = { autoPick: 0, autoInput: "fyx", ...options };
    rpc.onTraffic = (dir, message) => this.traffic.push({ direction: dir === "out" ? "host→sidecar" : "sidecar→host", message });
    this.registerHandlers();
  }

  /** Sidecar in a child process, optionally under Node's permission model. */
  static spawn(options: MockHostOptions & { permission?: { read: string[]; write: string[] }; cwd?: string; env?: Record<string, string> } = {}): MockHost {
    const main = join(import.meta.dirname, "main.ts");
    const flags = options.permission
      ? [
          "--permission",
          ...[import.meta.dirname, ...options.permission.read].flatMap(driveVariants).map((p) => `--allow-fs-read=${p}`),
          ...options.permission.write.flatMap(driveVariants).map((p) => `--allow-fs-write=${p}`),
        ]
      : [];
    // Under --permission even `existsSync("")` throws unless the cwd is readable, so default the
    // cwd to the first granted folder.
    const cwd = options.cwd ?? options.permission?.read[0];
    const child = spawn(process.execPath, [...flags, main], {
      stdio: ["pipe", "pipe", "pipe"],
      ...(cwd ? { cwd } : {}),
      ...(options.env ? { env: { ...process.env, ...options.env } } : {}),
    });
    const rpc = new RpcPeer((line) => void child.stdin.write(line));
    const host = new MockHost(rpc, options);
    host.child = child;
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => rpc.receive(chunk));
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk: string) => {
      host.stderr = (host.stderr + chunk).slice(-20_000);
    });
    child.on("exit", (code) => rpc.close(`sidecar exited with code ${code}`));
    return host;
  }

  /** Sidecar in this process (faster; no permission model). */
  static inProcess(options: MockHostOptions = {}): { host: MockHost; sidecar: ExtensionHost } {
    const [hostSide, sidecarSide] = pairedPeers();
    const sidecar = new ExtensionHost(sidecarSide);
    return { host: new MockHost(hostSide, options), sidecar };
  }

  get sidecarStderr(): string {
    return this.stderr;
  }

  // ----------------------------------------------------------------------------------------
  // Driving the sidecar

  async initialize(params: InitializeParams): Promise<InitializeResult> {
    const result = await this.rpc.request<InitializeResult>("initialize", params);
    for (const s of result.surfaces) this.surfaces.set(s.id, s);
    await this.rpc.request("initialized");
    return result;
  }

  async mount(surfaceId: string): Promise<Node[]> {
    const nodes = await this.rpc.request<Node[]>("mount", { "surface-id": surfaceId });
    this.check(`mount ${surfaceId}`, validateNodes(nodes));
    this.trees.set(surfaceId, nodes);
    return nodes;
  }

  async event(surfaceId: string, event: UiEvent): Promise<Update> {
    const update = await this.rpc.request<Update>("handle-event", { "surface-id": surfaceId, event });
    this.apply(surfaceId, update, "handle-event");
    return update;
  }

  async listRows(surfaceId: string, listNode: number, start: number, count: number): Promise<ListRow[]> {
    const rows = await this.rpc.request<ListRow[]>("list-rows", { "surface-id": surfaceId, "list-node": listNode, start, count });
    for (const row of rows) this.check(`list-rows ${surfaceId}#${row.index}`, validateNodes(row.nodes, { singleRoot: true }));
    return rows;
  }

  async openDocument(uri: string, text: string, languageId?: string): Promise<void> {
    this.buffers.set(uri, { text, version: 1 });
    await this.rpc.request("vscode/didOpenTextDocument", { uri, text, version: 1, ...(languageId ? { languageId } : {}) });
  }

  /** Host-side edit of a buffer (full-text sync), e.g. to reset it between runs. */
  async changeDocument(uri: string, text: string): Promise<void> {
    const buffer = this.buffers.get(uri) ?? { text, version: 0 };
    buffer.text = text;
    buffer.version += 100;
    this.buffers.set(uri, buffer);
    await this.rpc.request("vscode/didChangeTextDocument", { uri, version: buffer.version, text });
  }

  async setActiveEditor(uri: string | null, selections?: unknown[]): Promise<void> {
    await this.rpc.request("vscode/didChangeActiveEditor", { uri, ...(selections ? { selections } : {}) });
  }

  executeCommand(command: string, ...args: unknown[]): Promise<unknown> {
    return this.rpc.request("vscode/executeCommand", { command, args });
  }

  format(uri: string, options: Record<string, unknown> = { tabSize: 2, insertSpaces: true }): Promise<WireEdit[]> {
    return this.rpc.request<WireEdit[]>("vscode/provideFormatting", { uri, options });
  }

  coverage(): Promise<Record<string, any>> {
    return this.rpc.request("fyx/coverage");
  }

  /** Waits until pending QuickPick/InputBox automation and queued pushes have settled. */
  async settle(ms = 20): Promise<void> {
    for (let i = 0; i < 5; i++) {
      await new Promise((r) => setTimeout(r, ms));
      await Promise.allSettled(this.quickInputs);
    }
  }

  async close(): Promise<void> {
    try {
      await Promise.race([this.rpc.request("shutdown"), new Promise((r) => setTimeout(r, 2000))]);
    } catch {
      // already gone
    }
    if (this.child) {
      this.rpc.notify("exit");
      const child = this.child;
      await new Promise<void>((resolve) => {
        if (child.exitCode !== null) return resolve();
        const t = setTimeout(() => {
          child.kill();
          resolve();
        }, 2000);
        child.on("exit", () => {
          clearTimeout(t);
          resolve();
        });
      });
    }
    this.rpc.close();
  }

  writeTraffic(path: string): void {
    writeFileSync(path, this.traffic.map((t) => JSON.stringify(t)).join("\n") + "\n");
  }

  /** Count of messages by direction and method (responses are counted as `<response>`). */
  trafficSummary(): Record<string, number> {
    const out: Record<string, number> = {};
    for (const t of this.traffic) {
      const key = `${t.direction} ${"method" in t.message ? t.message.method : "<response>"}`;
      out[key] = (out[key] ?? 0) + 1;
    }
    return out;
  }

  // ----------------------------------------------------------------------------------------
  // Sidecar → host

  private check(where: string, errors: string[]): void {
    for (const e of errors) this.schemaErrors.push(`${where}: ${e}`);
  }

  /** Applies `update` as the Fyx host would; returns why it was refused, if it was. */
  private apply(surfaceId: string, update: Update, where: string): string | undefined {
    if (update !== "unchanged" && "patches" in update) {
      for (const p of update.patches) {
        if ("set-props" in p) this.check(where, validateProps(p["set-props"][1]));
        if ("insert" in p) this.check(where, validateNodes([{ ...p.insert.node, parent: null }]).filter((e) => !e.includes("does not precede")));
      }
    }
    const tree = this.trees.get(surfaceId);
    if (!tree) {
      if (update !== "unchanged" && !this.closedSurfaces.has(surfaceId)) this.schemaErrors.push(`${where}: update for unmounted surface ${surfaceId}`);
      return undefined;
    }
    if (this.refuseNextPush === surfaceId && where === "push-update") {
      this.refuseNextPush = undefined;
      return "refused by the test";
    }
    try {
      const next = applyUpdate(tree, update);
      this.check(`${where} ${surfaceId}`, validateNodes(next));
      this.trees.set(surfaceId, next);
      return undefined;
    } catch (err) {
      this.schemaErrors.push(`${where} ${surfaceId}: ${(err as Error).message}`);
      return (err as Error).message;
    }
  }

  private registerHandlers(): void {
    const rpc = this.rpc;
    rpc.on("push-update", (p: { "surface-id": string; update: Update }) => {
      const refused = this.apply(p["surface-id"], p.update, "push-update");
      if (refused) throw new RpcError(ErrorCodes.InvalidParams, `push-update refused: ${refused}`);
      return null;
    });
    rpc.on("log", (p: { level: string; message: string }) => void this.logs.push(p));
    rpc.on("fyx/surfacesChanged", (p: { added: Surface[]; removed: string[] }) => {
      for (const s of p.added) {
        this.surfaces.set(s.id, s);
        this.quickInputs.push(this.automateQuickInput(s));
      }
      for (const id of p.removed) {
        this.closedSurfaces.add(id);
        this.surfaces.delete(id);
        this.trees.delete(id);
      }
    });
    rpc.on("vscode/showMessage", (p: { severity: string; message: string; items: string[] }) => {
      this.messages.push(p);
      const ix = this.options.messageChoice;
      return ix !== undefined && p.items[ix] !== undefined ? p.items[ix] : null;
    });
    // All or nothing, refused when a document's version is not the buffer's (the user typed).
    rpc.on("vscode/applyEdit", (p: { edits: { uri: string; version: number | null; edits: WireEdit[] }[] }) => {
      if (this.refuseEdits) return false;
      for (const change of p.edits) {
        const buffer = this.buffers.get(change.uri);
        if (buffer && change.version !== null && change.version !== buffer.version) return false;
      }
      for (const change of p.edits) {
        const buffer = this.buffers.get(change.uri);
        if (!buffer) continue;
        buffer.text = applyWireEdits(buffer.text, change.edits);
        buffer.version++;
      }
      return true;
    });
    rpc.on("vscode/publishDiagnostics", (p: { uri: string; diagnostics: unknown[]; owner: string }) => void this.diagnostics.set(p.uri, p.diagnostics));
    rpc.on("vscode/createDecorationType", (p: { key: string; style: unknown }) => void this.decorationTypes.set(p.key, p.style));
    rpc.on("vscode/disposeDecorationType", (p: { key: string }) => void this.decorationTypes.delete(p.key));
    rpc.on("vscode/setDecorations", (p: { uri: string; key: string; ranges: unknown[] }) => void this.decorations.set(`${p.key} ${p.uri}`, p.ranges));
    rpc.on("vscode/executeCommand", (p: { command: string; args: unknown[] }) => {
      this.hostCommands.push(p);
      return null;
    });
    rpc.on("vscode/registerProvider", (p: { id: number }) => void this.providers.set(p.id, p));
    rpc.on("vscode/unregisterProvider", (p: { id: number }) => void this.providers.delete(p.id));
    rpc.on("vscode/setSelections", (p: { uri: string; selections: unknown[] }) => void this.selections.set(p.uri, p.selections));
    rpc.on("vscode/setEditorOptions", (p: { uri: string; options: Record<string, unknown> }) => void this.editorOptions.set(p.uri, { ...((this.editorOptions.get(p.uri) as object) ?? {}), ...p.options }));
    rpc.on("vscode/setContext", (p: { key: string; value: unknown }) => void this.contextKeys.set(p.key, p.value));
    rpc.on("vscode/showTextDocument", () => true);
    rpc.on("vscode/saveTextDocument", () => true);
    rpc.on("vscode/saveAll", () => true);
    rpc.on("vscode/insertSnippet", () => true);
    rpc.on("vscode/clipboard/readText", () => "");
    rpc.on("vscode/clipboard/writeText", () => null);
    rpc.on("vscode/openExternal", () => false);
    // Notifications the mock only records (see traffic).
    for (const m of ["vscode/registerCommand", "vscode/unregisterCommand", "vscode/updateConfiguration", "vscode/progress", "vscode/revealRange"]) rpc.on(m, () => null);
    rpc.onUnhandled((method) => {
      this.schemaErrors.push(`sidecar called unknown host method ${method}`);
      return null;
    });
  }

  private async automateQuickInput(surface: Surface): Promise<void> {
    const isPick = surface.id.includes("/quick-pick/");
    const isInput = surface.id.includes("/input-box/");
    if (!isPick && !isInput) return;
    // Let the sidecar finish populating items before the "user" acts.
    await new Promise((r) => setTimeout(r, 10));
    if (!this.surfaces.has(surface.id)) return;
    const nodes = await this.mount(surface.id).catch(() => undefined);
    if (!nodes) return;
    if (isPick && this.options.autoPick !== false) {
      const list = nodes.find((n) => "list" in n.props);
      if (list) {
        await this.listRows(surface.id, list.id, 0, 20).catch(() => []);
        await this.event(surface.id, { "row-click": [list.id, this.options.autoPick ?? 0] }).catch(() => undefined);
      }
    } else if (isInput && this.options.autoInput !== false) {
      const input = nodes.find((n) => "input" in n.props);
      if (input) await this.event(surface.id, { "input-submitted": [input.id, this.options.autoInput ?? ""] }).catch(() => undefined);
    }
  }
}

export function applyWireEdits(text: string, edits: readonly WireEdit[]): string {
  const lineStarts = [0];
  for (let i = 0; i < text.length; i++) if (text[i] === "\n") lineStarts.push(i + 1);
  const offset = (p: { line: number; character: number }) => {
    if (p.line >= lineStarts.length) return text.length;
    return Math.min(lineStarts[p.line] + p.character, p.line + 1 < lineStarts.length ? lineStarts[p.line + 1] - 1 : text.length);
  };
  const sorted = edits.map((e) => ({ start: offset(e.range.start), end: offset(e.range.end), text: e.newText })).sort((a, b) => b.start - a.start);
  for (const e of sorted) text = text.slice(0, e.start) + e.text + text.slice(e.end);
  return text;
}

/**
 * Node's permission grants compare Windows paths case-sensitively, drive letter included, while
 * VS Code's `Uri.fsPath` lowercases the drive (`c:\...`). Grant both spellings.
 */
function driveVariants(path: string): string[] {
  const m = /^([a-zA-Z]):(.*)$/.exec(path);
  if (!m) return [path];
  return [...new Set([`${m[1].toUpperCase()}:${m[2]}`, `${m[1].toLowerCase()}:${m[2]}`])];
}

export function tempDir(prefix: string): string {
  return mkdtempSync(join(tmpdir(), prefix));
}
