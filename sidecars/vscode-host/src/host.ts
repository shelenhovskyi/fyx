// The extension host: loads one unmodified VS Code extension (a .vsix or an unpacked folder),
// answers `require("vscode")` with the shim, activates it on its activation events and serves the
// JSON-RPC methods listed in docs/vscode-coverage.md ("Protocol").

import Module from "node:module";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve, sep } from "node:path";
import { tmpdir } from "node:os";
import { createHash } from "node:crypto";
import { Coverage, isNotImplemented, type CoverageReport } from "./coverage.ts";
import type { ExtensionInfo, HostContext } from "./context.ts";
import { ErrorCodes, RpcError, type RpcPeer } from "./rpc.ts";
import type { ListRow, Node, Surface, UiEvent, Update } from "./schema.ts";
import { createExtensionContext, createVscodeApi, type ShimState } from "./shim/api.ts";
import { ConfigStore } from "./shim/config.ts";
import { DocumentStore, languageIdForPath, type TextDocument } from "./shim/documents.ts";
import { Selection, Uri, toUri } from "./shim/types.ts";
import { decodeDiagnostic, decodeSelection } from "./shim/wire.ts";
import { SurfaceRegistry } from "./ui/surfaces.ts";
import { readZip } from "./zip.ts";

// ---------------------------------------------------------------------------------------------
// `require("vscode")`: one hook for the process, routed by the requiring file's extension root.

const apiByRoot = new Map<string, Record<string, unknown>>();
let hookInstalled = false;

function installRequireHook(): void {
  if (hookInstalled) return;
  hookInstalled = true;
  const M = Module as unknown as { _load: (request: string, parent: { filename?: string } | undefined, isMain: boolean) => unknown };
  const original = M._load;
  M._load = function (request, parent, isMain) {
    if (request === "vscode") {
      const file = parent?.filename ?? "";
      for (const [root, api] of apiByRoot) if (file === root || file.startsWith(root + sep)) return api;
      throw new Error(`require("vscode") from outside a loaded extension: ${file}`);
    }
    return original.call(this, request, parent, isMain);
  };
}

/** Unpacks a .vsix (`extension/` subtree) into `cacheDir`; returns the extension root. */
export function unpackVsix(vsixPath: string, cacheDir: string): string {
  const data = readFileSync(vsixPath);
  const digest = createHash("sha256").update(data).digest("hex").slice(0, 16);
  const root = join(cacheDir, digest);
  if (existsSync(join(root, "package.json"))) return root;
  for (const [name, content] of readZip(data)) {
    if (!name.startsWith("extension/")) continue;
    const target = join(root, ...name.slice("extension/".length).split("/"));
    if (!resolve(target).startsWith(resolve(root) + sep)) throw new Error(`vsix: entry escapes the extension folder: ${name}`);
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, content);
  }
  if (!existsSync(join(root, "package.json"))) throw new Error("vsix has no extension/package.json");
  return root;
}

export interface InitializeParams {
  /** A .vsix file or an unpacked extension folder. */
  extensionPath: string;
  workspaceFolders?: string[];
  settings?: Record<string, unknown>;
  /** Where unpacked .vsix files and extension storage go (default: OS temp). */
  storageDir?: string;
}

export interface InitializeResult {
  extension: { id: string; displayName: string; version: string; main: string | null; activationEvents: string[] };
  contributes: { commands: unknown[]; configuration: unknown; keybindings: unknown[]; views: unknown; menus: unknown };
  surfaces: Surface[];
}

export class ExtensionHost {
  private rpc: RpcPeer;
  private ctx?: HostContext;
  private shim?: ShimState;
  private coverage = new Coverage();
  private module?: { activate?: (context: unknown) => unknown; deactivate?: () => unknown };
  private activation?: Promise<void>;
  private activationEvents = new Set<string>();
  private extensionContext?: Record<string, unknown>;
  activated = false;
  activationError?: string;
  /** Every activation event the host saw, for the coverage report. */
  readonly seenEvents: string[] = [];

  constructor(rpc: RpcPeer) {
    this.rpc = rpc;
    this.registerHandlers();
  }

  private get c(): HostContext {
    if (!this.ctx) throw new RpcError(ErrorCodes.InvalidRequest, "not initialized");
    return this.ctx;
  }

  private get s(): ShimState {
    if (!this.shim) throw new RpcError(ErrorCodes.InvalidRequest, "not initialized");
    return this.shim;
  }

  async initialize(params: InitializeParams): Promise<InitializeResult> {
    if (this.ctx) throw new RpcError(ErrorCodes.InvalidRequest, "already initialized");
    const storageDir = params.storageDir ?? join(tmpdir(), "fyx-vscode-host");
    const root = params.extensionPath.toLowerCase().endsWith(".vsix") ? unpackVsix(params.extensionPath, join(storageDir, "unpacked")) : resolve(params.extensionPath);
    const manifest = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
    localizeManifest(root, manifest);
    const id = extensionId(manifest);
    const ext: ExtensionInfo = { id, displayName: manifest.displayName ?? manifest.name, version: manifest.version ?? "0.0.0", path: root, uri: Uri.file(root), manifest };

    const rpc = this.rpc;
    const surfaces = new SurfaceRegistry({
      pushUpdate: (surfaceId, update) => rpc.notify("push-update", { "surface-id": surfaceId, update }),
      surfacesChanged: (added, removed) => rpc.notify("fyx/surfacesChanged", { added, removed }),
    });
    const documents = new DocumentStore();
    const config = new ConfigStore();
    config.addContributedDefaults(manifest.contributes?.configuration);
    config.setUserSettings(params.settings ?? {}, false);
    config.onUpdate = (key, value, target) => rpc.notify("vscode/updateConfiguration", { key, value: value ?? null, target: target === 1 ? "global" : "workspace" });

    const storage = join(storageDir, "storage", id);
    const ctx: HostContext = {
      rpc,
      coverage: this.coverage,
      ext,
      surfaces,
      documents,
      config,
      workspaceFolders: (params.workspaceFolders ?? []).map((f) => toUri(f)),
      storagePath: join(storage, "workspace"),
      globalStoragePath: join(storage, "global"),
      logPath: join(storage, "logs"),
      activate: (event) => this.activateOn(event),
      executeCommand: (command, ...args) => this.s.commands.execute(command, ...args),
      log: (level, message) => rpc.notify("log", { level, message }),
    };
    this.ctx = ctx;
    this.shim = createVscodeApi(ctx);
    this.computeActivationEvents(manifest);
    documents.onDidOpen.event((doc) => void this.activateOn(`onLanguage:${doc.languageId}`));

    return {
      extension: { id, displayName: ext.displayName, version: ext.version, main: manifest.main ?? null, activationEvents: [...this.activationEvents] },
      contributes: {
        commands: arrayOf(manifest.contributes?.commands),
        configuration: manifest.contributes?.configuration ?? null,
        keybindings: arrayOf(manifest.contributes?.keybindings),
        views: manifest.contributes?.views ?? null,
        menus: manifest.contributes?.menus ?? null,
      },
      surfaces: surfaces.declared(),
    };
  }

  /** Startup activation: `*`, `onStartupFinished`, `workspaceContains:`, and documents already open. */
  async startup(): Promise<void> {
    await this.activateOn("*");
    await this.activateOn("onStartupFinished");
    for (const event of this.activationEvents) {
      if (!event.startsWith("workspaceContains:")) continue;
      const pattern = event.slice("workspaceContains:".length);
      const found = await this.s.workspace.findFiles(pattern.includes("/") || pattern.includes("*") ? pattern : `**/${pattern}`, undefined, 1).catch(() => []);
      if (found.length) await this.activateOn(event);
    }
  }

  private computeActivationEvents(manifest: Record<string, any>): void {
    for (const e of manifest.activationEvents ?? []) this.activationEvents.add(e);
    // Implicit since VS Code 1.74: contributed commands, views, languages and custom editors.
    for (const c of arrayOf(manifest.contributes?.commands)) this.activationEvents.add(`onCommand:${(c as { command: string }).command}`);
    for (const list of Object.values(manifest.contributes?.views ?? {}) as { id: string }[][]) for (const v of list ?? []) this.activationEvents.add(`onView:${v.id}`);
    for (const l of arrayOf(manifest.contributes?.languages)) this.activationEvents.add(`onLanguage:${(l as { id: string }).id}`);
  }

  private matches(event: string): boolean {
    if (this.activationEvents.has(event)) return true;
    if (event === "*" || event === "onStartupFinished") return false;
    if (event.startsWith("onLanguage:") && this.activationEvents.has("onLanguage")) return true;
    return false;
  }

  activateOn(event: string): Promise<void> {
    if (!this.seenEvents.includes(event)) this.seenEvents.push(event);
    if (this.activation) return this.activation;
    if (!this.matches(event)) return Promise.resolve();
    this.activation = this.activate(event);
    return this.activation;
  }

  private async activate(reason: string): Promise<void> {
    const ctx = this.c;
    const main = ctx.ext.manifest.main as string | undefined;
    if (!main) {
      this.activated = true;
      return;
    }
    try {
      installRequireHook();
      apiByRoot.set(ctx.ext.path, this.s.api);
      const entry = resolveMain(ctx.ext.path, main);
      const requireFromExt = Module.createRequire(entry);
      this.module = requireFromExt(entry);
      this.extensionContext = createExtensionContext(ctx);
      ctx.log("info", `activating ${ctx.ext.id} (${reason})`);
      await this.module?.activate?.(this.extensionContext);
      this.activated = true;
    } catch (err) {
      this.activationError = describeError(err);
      this.coverage.error(`activate: ${this.activationError}`);
      ctx.log("error", `activation of ${ctx.ext.id} failed: ${this.activationError}`);
    }
  }

  async shutdown(): Promise<void> {
    try {
      await this.module?.deactivate?.();
    } catch (err) {
      this.coverage.error(`deactivate: ${describeError(err)}`);
    }
    for (const d of (this.extensionContext?.subscriptions as { dispose(): unknown }[] | undefined) ?? []) {
      try {
        d.dispose();
      } catch {
        // ignore
      }
    }
    this.ctx?.surfaces.flush();
  }

  coverageReport(): CoverageReport & { activated: boolean; activationError?: string; activationEvents: string[]; commands: string[]; contextKeys: Record<string, unknown> } {
    return {
      ...this.coverage.report(),
      activated: this.activated,
      ...(this.activationError ? { activationError: this.activationError } : {}),
      activationEvents: [...this.activationEvents],
      commands: this.shim?.commands.list() ?? [],
      contextKeys: Object.fromEntries(this.shim?.contextKeys ?? []),
    };
  }

  /** Records an error thrown by extension code outside a request (timers, listeners). */
  recordUncaught(err: unknown): void {
    this.coverage.error(`uncaught: ${describeError(err)}`);
  }

  private registerHandlers(): void {
    const rpc = this.rpc;
    const wrap = <P, R>(f: (p: P) => R | Promise<R>) => async (p: P) => {
      try {
        return await f(p);
      } catch (err) {
        if (isNotImplemented(err)) throw new RpcError(ErrorCodes.NotImplemented, err.message, { api: err.api });
        if (err instanceof RpcError) throw err;
        throw new RpcError(ErrorCodes.ExtensionError, describeError(err));
      }
    };
    // fyx:ui world exports
    rpc.on("initialize", wrap((p: InitializeParams) => this.initialize(p)));
    rpc.on("initialized", wrap(() => this.startup()));
    rpc.on("surfaces", wrap((): Surface[] => this.c.surfaces.declared()));
    rpc.on("mount", wrap((p: { "surface-id": string }): Promise<Node[]> => this.c.surfaces.mount(p["surface-id"])));
    rpc.on("handle-event", wrap((p: { "surface-id": string; event: UiEvent }): Promise<Update> => this.c.surfaces.handleEvent(p["surface-id"], p.event)));
    rpc.on("list-rows", wrap((p: { "surface-id": string; "list-node": number; start: number; count: number }): Promise<ListRow[]> => this.c.surfaces.listRows(p["surface-id"], p["list-node"], p.start, p.count)));
    rpc.on("fyx/surfaceClosed", wrap((p: { "surface-id": string }) => this.c.surfaces.closed(p["surface-id"])));
    rpc.on("shutdown", wrap(() => this.shutdown()));
    rpc.on("fyx/coverage", wrap(() => this.coverageReport()));

    // VS Code services
    rpc.on("vscode/executeCommand", wrap((p: { command: string; args?: unknown[] }) => this.s.commands.execute(p.command, ...(p.args ?? [])).then(toWireResult)));
    rpc.on("vscode/didOpenTextDocument", wrap((p: { uri: string; languageId?: string; version?: number; text: string }) => {
      const uri = toUri(p.uri);
      const doc = this.c.documents.open(uri, p.languageId ?? languageIdForPath(uri.path, this.c.ext.manifest.contributes?.languages), p.version ?? 1, p.text);
      doc.isDirty = false;
    }));
    rpc.on("vscode/didChangeTextDocument", wrap((p: { uri: string; version: number; text: string }) => this.c.documents.replace(toUri(p.uri), p.version, p.text)));
    rpc.on("vscode/didCloseTextDocument", wrap((p: { uri: string }) => this.c.documents.close(toUri(p.uri))));
    rpc.on("vscode/didSaveTextDocument", wrap((p: { uri: string }) => {
      const doc = this.c.documents.get(toUri(p.uri));
      if (!doc) return;
      doc.isDirty = false;
      this.c.documents.onDidSave.fire(doc);
    }));
    rpc.on("vscode/willSaveTextDocument", wrap((p: { uri: string; reason?: number }) => {
      const doc = this.c.documents.get(toUri(p.uri));
      return doc ? this.s.workspace.willSaveFromHost(doc, p.reason ?? 1) : [];
    }));
    rpc.on("vscode/didChangeActiveEditor", wrap((p: { uri: string | null; selections?: { anchor: { line: number; character: number }; active: { line: number; character: number } }[] }) => {
      if (!p.uri) return this.s.editors.setActive(undefined);
      const doc = this.c.documents.get(toUri(p.uri));
      if (!doc) throw new RpcError(ErrorCodes.InvalidParams, `document not open: ${p.uri}`);
      const editor = this.s.editors.editorFor(doc);
      if (p.selections) editor.setSelectionsFromHost(p.selections.map((sel) => clampSelection(doc, decodeSelection(sel))));
      this.s.editors.setActive(editor);
    }));
    rpc.on("vscode/didChangeSelection", wrap((p: { uri: string; selections: { anchor: { line: number; character: number }; active: { line: number; character: number } }[]; kind?: number }) => {
      const doc = this.c.documents.get(toUri(p.uri));
      if (!doc) return;
      this.s.editors.setSelections(this.s.editors.editorFor(doc), p.selections.map((sel) => clampSelection(doc, decodeSelection(sel))), p.kind);
    }));
    rpc.on("vscode/didChangeDiagnostics", wrap((p: { uri: string; diagnostics: never[] }) => this.s.languages.setHostDiagnostics(toUri(p.uri), p.diagnostics.map(decodeDiagnostic))));
    rpc.on("vscode/didChangeConfiguration", wrap((p: { settings: Record<string, unknown> }) => this.c.config.setUserSettings(p.settings)));
    rpc.on("vscode/provideFormatting", wrap((p: any) => this.s.languages.provideFormatting(p)));
    rpc.on("vscode/provideOnTypeFormatting", wrap((p: any) => this.s.languages.provideOnTypeFormatting(p)));
    rpc.on("vscode/provideCompletion", wrap((p: any) => this.s.languages.provideCompletion(p)));
    rpc.on("vscode/provideHover", wrap((p: any) => this.s.languages.provideHover(p)));
  }
}

// The character set `vsce` accepts for `name` and `publisher`. The id names the extension's
// storage folder, and it comes from an untrusted package.json.
const ID_SEGMENT = /^[a-z0-9][a-z0-9._-]*$/i;

export function extensionId(manifest: { publisher?: unknown; name?: unknown }): string {
  const publisher = manifest.publisher ?? "unknown";
  for (const [field, value] of [["publisher", publisher], ["name", manifest.name]] as const) {
    if (typeof value !== "string" || !ID_SEGMENT.test(value) || value.includes("..")) {
      throw new Error(`package.json ${field} ${JSON.stringify(value)} is not a valid extension name`);
    }
  }
  return `${publisher}.${manifest.name}`;
}

/** VS Code never hands an extension a selection outside its document. */
function clampSelection(doc: TextDocument, sel: Selection): Selection {
  return new Selection(doc.validatePosition(sel.anchor), doc.validatePosition(sel.active));
}

function toWireResult(v: unknown): unknown {
  try {
    return v === undefined ? null : JSON.parse(JSON.stringify(v));
  } catch {
    return null;
  }
}

function arrayOf(v: unknown): unknown[] {
  return v === undefined || v === null ? [] : Array.isArray(v) ? v : [v];
}

function resolveMain(root: string, main: string): string {
  const base = resolve(root, main);
  for (const candidate of [base, `${base}.js`, join(base, "index.js")]) if (existsSync(candidate) && !candidate.endsWith(sep)) return candidate;
  throw new Error(`main entry not found: ${main}`);
}

function describeError(err: unknown): string {
  if (isNotImplemented(err)) return err.message;
  if (err instanceof Error) {
    const where = err.stack?.split("\n").slice(1, 3).map((l) => l.trim()).join(" | ");
    return `${err.name}: ${err.message}${where ? ` (${where})` : ""}`;
  }
  return String(err);
}

/** Resolves `%key%` placeholders from package.nls.json, as VS Code does at install time. */
function localizeManifest(root: string, manifest: Record<string, unknown>): void {
  const file = join(root, "package.nls.json");
  if (!existsSync(file)) return;
  let nls: Record<string, string | { message: string }>;
  try {
    nls = JSON.parse(readFileSync(file, "utf8"));
  } catch {
    return;
  }
  const walk = (v: unknown): unknown => {
    if (typeof v === "string") {
      const m = /^%(.+)%$/.exec(v);
      const entry = m && Object.hasOwn(nls, m[1]) ? nls[m[1]] : undefined;
      return entry === undefined ? v : typeof entry === "string" ? entry : entry.message;
    }
    if (Array.isArray(v)) return v.map(walk);
    if (v && typeof v === "object") {
      for (const [k, x] of Object.entries(v)) (v as Record<string, unknown>)[k] = walk(x);
    }
    return v;
  };
  walk(manifest);
}
