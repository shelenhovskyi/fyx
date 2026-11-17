// Assembles the `vscode` module an extension gets from `require("vscode")`.
//
// Every namespace is wrapped in a Proxy that records what the extension touches. A name that
// exists in VS Code (per src/api-catalog.json, generated from @types/vscode) but not here yields an
// explicit `NotImplementedInFyx` error naming the API: functions and events throw when called,
// values throw when read, classes throw when constructed. Names VS Code does not have either stay
// `undefined`, as in VS Code, so feature detection keeps working.

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { randomUUID } from "node:crypto";
import { join } from "node:path";
import type { HostContext } from "../context.ts";
import { NotImplementedInFyx } from "../coverage.ts";
import { OutputPanel } from "../ui/output.ts";
import { InputBoxImpl, QuickPickImpl, showInputBox, showQuickPick } from "../ui/quickpick.ts";
import { StatusBar } from "../ui/statusbar.ts";
import { TreeViewSurface } from "../ui/treeview.ts";
import { knownLanguageIds, type TextDocument } from "./documents.ts";
import { EditBuilder, EditorStore, TextEditorDecorationType } from "./editors.ts";
import { LanguageFeatures, matchSelector, type DocumentSelector } from "./languages.ts";
import * as types from "./types.ts";
import { encodeRange } from "./wire.ts";
import { WorkspaceApi } from "./workspace.ts";

const require = createRequire(import.meta.url);
const catalog = require("../api-catalog.json") as {
  classes: string[];
  enums: string[];
  namespaces: Record<string, { functions: string[]; values: string[]; events: string[] }>;
};

/** What `vscode.version` reports; extensions compare it against their `engines.vscode`. */
export const VSCODE_API_VERSION = "1.100.0";

export interface ShimState {
  api: Record<string, unknown>;
  commands: CommandRegistry;
  editors: EditorStore;
  workspace: WorkspaceApi;
  languages: LanguageFeatures;
  statusBar: StatusBar;
  output: OutputPanel;
  views: Map<string, TreeViewSurface>;
  contextKeys: Map<string, unknown>;
}

// ---------------------------------------------------------------------------------------------
// Commands

export class CommandRegistry {
  private ctx: HostContext;
  private handlers = new Map<string, { fn: (...args: unknown[]) => unknown; thisArg: unknown }>();
  readonly contextKeys = new Map<string, unknown>();
  /** Set by the shim: opens a document in an editor (`vscode.open`). */
  openHook: (uri: types.Uri, options?: unknown) => Promise<unknown> = async () => undefined;

  constructor(ctx: HostContext) {
    this.ctx = ctx;
  }

  register(id: string, fn: (...args: unknown[]) => unknown, thisArg?: unknown): types.Disposable {
    if (this.handlers.has(id)) throw new Error(`command '${id}' already exists`);
    this.handlers.set(id, { fn, thisArg });
    this.ctx.rpc.notify("vscode/registerCommand", { command: id });
    return new types.Disposable(() => {
      this.handlers.delete(id);
      this.ctx.rpc.notify("vscode/unregisterCommand", { command: id });
    });
  }

  has(id: string): boolean {
    return this.handlers.has(id);
  }

  list(): string[] {
    return [...this.handlers.keys()];
  }

  async execute(id: string, ...args: unknown[]): Promise<unknown> {
    if (!this.handlers.has(id)) await this.ctx.activate(`onCommand:${id}`);
    const h = this.handlers.get(id);
    if (h) return h.fn.apply(h.thisArg, args);
    switch (id) {
      case "setContext": {
        const [key, value] = args as [string, unknown];
        this.contextKeys.set(key, value);
        this.ctx.rpc.notify("vscode/setContext", { key, value: toWire(value) });
        return undefined;
      }
      case "vscode.open":
        return this.openHook(types.toUri(args[0]), args[1]);
    }
    // Everything else is a host (built-in) command.
    return this.ctx.rpc.request("vscode/executeCommand", { command: id, args: args.map(toWire) });
  }
}

/** Plain JSON for values crossing to the host: URIs as strings, ranges/positions LSP-style. */
export function toWire(value: unknown, depth = 0): unknown {
  if (depth > 8) return null;
  if (value === undefined || value === null || typeof value === "string" || typeof value === "number" || typeof value === "boolean") return value ?? null;
  if (value instanceof types.Uri) return value.toString();
  if (value instanceof types.Range) return encodeRange(value);
  if (value instanceof types.Position) return { line: value.line, character: value.character };
  if (typeof value === "function" || typeof value === "symbol" || typeof value === "bigint") return null;
  if (Array.isArray(value)) return value.map((v) => toWire(v, depth + 1));
  if (typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value)) if (typeof v !== "function") out[k] = toWire(v, depth + 1);
    return out;
  }
  return null;
}

// ---------------------------------------------------------------------------------------------
// The module

export function createVscodeApi(ctx: HostContext): ShimState {
  const commands = new CommandRegistry(ctx);
  const editors = new EditorStore(ctx);
  const workspace = new WorkspaceApi(ctx, editors);
  const languages = new LanguageFeatures(ctx);
  const statusBar = new StatusBar(ctx);
  const output = new OutputPanel(ctx);
  const views = new Map<string, TreeViewSurface>();

  // Contributed views become panel surfaces up front (`surfaces()` is called once).
  const viewContainers = ctx.ext.manifest.contributes?.views ?? {};
  for (const [container, list] of Object.entries(viewContainers as Record<string, { id: string; name?: string; type?: string }[]>)) {
    for (const view of list ?? []) {
      if (view.type === "webview") continue;
      const position = container === "explorer" || container === "scm" || container === "debug" || container === "test" ? "left" : isPanelContainer(ctx, container) ? "bottom" : "left";
      const surface = new TreeViewSurface(ctx, view.id, view.name ?? view.id, position);
      views.set(view.id, surface);
      ctx.surfaces.add(surface);
    }
  }
  const viewFor = (viewId: string) => {
    let surface = views.get(viewId);
    if (!surface) {
      ctx.coverage.note("vscode.window.createTreeView", `view '${viewId}' is not in contributes.views; created a left panel for it`);
      surface = new TreeViewSurface(ctx, viewId, viewId, "left");
      views.set(viewId, surface);
      ctx.surfaces.add(surface);
    }
    return surface;
  };

  const showTextDocument = async (docOrUri: TextDocument | types.Uri, columnOrOptions?: number | { selection?: types.Range; preserveFocus?: boolean; preview?: boolean; viewColumn?: number }, preserveFocus?: boolean) => {
    const doc = docOrUri instanceof types.Uri ? await workspace.openTextDocument(docOrUri) : docOrUri;
    const options = typeof columnOrOptions === "object" ? columnOrOptions : { viewColumn: columnOrOptions, preserveFocus };
    return editors.show(doc, options);
  };
  commands.openHook = (uri) => showTextDocument(uri);

  const message = (severity: "info" | "warning" | "error") =>
    async (text: string, ...rest: unknown[]): Promise<unknown> => {
      let modal = false;
      let detail: string | undefined;
      if (rest.length && rest[0] && typeof rest[0] === "object" && !("title" in (rest[0] as object))) {
        const opts = rest.shift() as { modal?: boolean; detail?: string };
        modal = opts.modal ?? false;
        detail = opts.detail;
      }
      const items = rest as (string | { title: string })[];
      const chosen = await ctx.rpc.request<string | null>("vscode/showMessage", {
        severity,
        message: text,
        ...(detail ? { detail } : {}),
        modal,
        items: items.map((i) => (typeof i === "string" ? i : i.title)),
      });
      return chosen === null || chosen === undefined ? undefined : items.find((i) => (typeof i === "string" ? i : i.title) === chosen);
    };

  let progressSerial = 0;
  const windowImpl = {
    get activeTextEditor() {
      return editors.active;
    },
    get visibleTextEditors() {
      return editors.visible;
    },
    onDidChangeActiveTextEditor: editors.onDidChangeActive.event,
    onDidChangeVisibleTextEditors: editors.onDidChangeVisible.event,
    onDidChangeTextEditorSelection: editors.onDidChangeSelection.event,
    onDidChangeTextEditorOptions: editors.onDidChangeOptions.event,
    onDidChangeTextEditorVisibleRanges: editors.onDidChangeVisibleRanges.event,
    onDidChangeTextEditorViewColumn: editors.onDidChangeViewColumn.event,
    state: { focused: true, active: true },
    onDidChangeWindowState: new types.EventEmitter<unknown>().event,
    activeColorTheme: { kind: types.ColorThemeKind.Dark },
    onDidChangeActiveColorTheme: new types.EventEmitter<unknown>().event,
    showInformationMessage: message("info"),
    showWarningMessage: message("warning"),
    showErrorMessage: message("error"),
    showQuickPick: (items: unknown, options?: object, token?: object) => showQuickPick(ctx, items as never, options as never, token as never),
    showInputBox: (options?: object, token?: object) => showInputBox(ctx, options as never, token as never),
    createQuickPick: () => new QuickPickImpl(ctx),
    createInputBox: () => new InputBoxImpl(ctx),
    createStatusBarItem: (a?: string | number, b?: number, c?: number) => statusBar.create(a, b, c),
    setStatusBarMessage: (text: string, hide?: number | PromiseLike<unknown>) => statusBar.message(text, hide),
    createOutputChannel: (name: string, opts?: string | { log: true }) => output.create(name, opts),
    createTextEditorDecorationType: (options: Record<string, unknown>) => new TextEditorDecorationType(ctx, options),
    registerTreeDataProvider: (viewId: string, provider: never) => {
      viewFor(viewId).setProvider(provider);
      return new types.Disposable(() => {});
    },
    createTreeView: (viewId: string, options: { treeDataProvider: never; showCollapseAll?: boolean; canSelectMany?: boolean }) => {
      const surface = viewFor(viewId);
      surface.setProvider(options.treeDataProvider);
      return surface.view;
    },
    showTextDocument,
    withProgress: async <R>(options: { location: number | { viewId: string }; title?: string; cancellable?: boolean }, task: (progress: { report(v: { message?: string; increment?: number }): void }, token: types.CancellationToken) => PromiseLike<R>): Promise<R> => {
      const id = ++progressSerial;
      const source = new types.CancellationTokenSource();
      const location = typeof options.location === "number" ? options.location : `view:${options.location.viewId}`;
      ctx.rpc.notify("vscode/progress", { id, phase: "start", title: options.title ?? "", location, cancellable: options.cancellable ?? false });
      try {
        return await task({ report: (v) => ctx.rpc.notify("vscode/progress", { id, phase: "report", ...v }) }, source.token);
      } finally {
        ctx.rpc.notify("vscode/progress", { id, phase: "end" });
        source.dispose();
      }
    },
  };

  const workspaceImpl = {
    fs: workspace.fs,
    get workspaceFolders() {
      return workspace.folders.length ? workspace.folders : undefined;
    },
    get name() {
      return workspace.folders[0]?.name;
    },
    get rootPath() {
      return workspace.folders[0]?.uri.fsPath;
    },
    workspaceFile: undefined,
    get textDocuments() {
      return ctx.documents.all;
    },
    isTrusted: true,
    onDidGrantWorkspaceTrust: new types.EventEmitter<void>().event,
    getWorkspaceFolder: (uri: types.Uri) => workspace.getWorkspaceFolder(uri),
    asRelativePath: (p: string | types.Uri, include?: boolean) => workspace.asRelativePath(p, include),
    getConfiguration: (section?: string, _scope?: unknown) => ctx.config.get(section),
    onDidChangeConfiguration: ctx.config.onDidChange.event,
    openTextDocument: (arg?: never) => workspace.openTextDocument(arg),
    applyEdit: (edit: types.WorkspaceEdit) => workspace.applyEdit(edit),
    findFiles: (include: never, exclude?: never, max?: number, token?: never) => workspace.findFiles(include, exclude, max, token),
    saveAll: (includeUntitled?: boolean) => ctx.rpc.request<boolean>("vscode/saveAll", { includeUntitled: includeUntitled ?? false }),
    createFileSystemWatcher: (glob: unknown, a?: boolean, b?: boolean, c?: boolean) => workspace.createFileSystemWatcher(glob, a, b, c),
    registerTextDocumentContentProvider: (scheme: string, provider: never) => {
      workspace.contentProviders.set(scheme, provider);
      return new types.Disposable(() => workspace.contentProviders.delete(scheme));
    },
    onDidOpenTextDocument: ctx.documents.onDidOpen.event,
    onDidCloseTextDocument: ctx.documents.onDidClose.event,
    onDidChangeTextDocument: ctx.documents.onDidChange.event,
    onDidSaveTextDocument: ctx.documents.onDidSave.event,
    onWillSaveTextDocument: workspace.willSave.event,
    onDidChangeWorkspaceFolders: workspace.foldersChanged.event,
  };

  const languagesImpl = {
    createDiagnosticCollection: (name?: string) => languages.createDiagnosticCollection(name),
    getDiagnostics: (uri?: types.Uri) => languages.getDiagnostics(uri),
    onDidChangeDiagnostics: languages.diagnosticsChanged.event,
    registerDocumentFormattingEditProvider: (selector: DocumentSelector, provider: never) => languages.register("documentFormatting", selector, provider),
    registerDocumentRangeFormattingEditProvider: (selector: DocumentSelector, provider: never) => languages.register("documentRangeFormatting", selector, provider),
    registerOnTypeFormattingEditProvider: (selector: DocumentSelector, provider: never, first: string, ...more: string[]) =>
      languages.register("onTypeFormatting", selector, provider, { triggerCharacters: [first, ...more] }),
    registerCompletionItemProvider: (selector: DocumentSelector, provider: never, ...triggerCharacters: string[]) =>
      languages.register("completion", selector, provider, { triggerCharacters }),
    registerHoverProvider: (selector: DocumentSelector, provider: never) => languages.register("hover", selector, provider),
    match: (selector: DocumentSelector, doc: TextDocument) => matchSelector(selector, doc),
    getLanguages: async () => [...new Set([...knownLanguageIds(), ...((ctx.ext.manifest.contributes?.languages ?? []) as { id: string }[]).map((l) => l.id)])],
    setTextDocumentLanguage: async (doc: TextDocument, languageId: string) => ctx.documents.setLanguage(doc, languageId),
    setLanguageConfiguration: (language: string, configuration: unknown) => {
      ctx.coverage.note("vscode.languages.setLanguageConfiguration", "stored but not applied: Zed language config comes from its own config.toml");
      languages.languageConfigurations.set(language, configuration);
      return new types.Disposable(() => languages.languageConfigurations.delete(language));
    },
  };

  const commandsImpl = {
    registerCommand: (id: string, fn: (...args: unknown[]) => unknown, thisArg?: unknown) => commands.register(id, fn, thisArg),
    registerTextEditorCommand: (id: string, fn: (editor: unknown, edit: EditBuilder, ...args: unknown[]) => unknown, thisArg?: unknown) =>
      commands.register(id, async (...args: unknown[]) => {
        const editor = editors.active;
        if (!editor) {
          ctx.log("warn", `text editor command ${id} ignored: no active editor`);
          return undefined;
        }
        const builder = new EditBuilder(editor.document);
        const result = await fn.call(thisArg, editor, builder, ...args);
        await editors.applyEdits(editor.document, builder.edits);
        return result;
      }),
    executeCommand: (id: string, ...args: unknown[]) => commands.execute(id, ...args),
    getCommands: async (filterInternal?: boolean) => commands.list().filter((c) => !filterInternal || !c.startsWith("_")),
  };

  const machineId = randomUUID().replace(/-/g, "");
  const envImpl = {
    appName: "Fyx",
    appRoot: ctx.ext.path,
    appHost: "desktop",
    language: "en",
    machineId,
    sessionId: randomUUID(),
    uriScheme: "fyx",
    uiKind: types.UIKind.Desktop,
    remoteName: undefined,
    shell: process.env.SHELL ?? process.env.ComSpec ?? "",
    isTelemetryEnabled: false,
    isNewAppInstall: false,
    onDidChangeTelemetryEnabled: new types.EventEmitter<boolean>().event,
    onDidChangeShell: new types.EventEmitter<string>().event,
    logLevel: types.LogLevel.Info,
    onDidChangeLogLevel: new types.EventEmitter<number>().event,
    clipboard: namespace(ctx, "env.clipboard", {
      readText: () => ctx.rpc.request<string>("vscode/clipboard/readText").then((t) => t ?? ""),
      writeText: (text: string) => ctx.rpc.request<void>("vscode/clipboard/writeText", { text }).then(() => undefined),
    }),
    openExternal: (uri: types.Uri) => ctx.rpc.request<boolean>("vscode/openExternal", { uri: uri.toString() }).then((r) => r !== false),
    asExternalUri: async (uri: types.Uri) => uri,
  };

  const self = extensionObject(ctx);
  const extensionsImpl = {
    getExtension: (id: string) => (id.toLowerCase() === ctx.ext.id.toLowerCase() ? self : undefined),
    get all() {
      return [self];
    },
    onDidChange: new types.EventEmitter<void>().event,
  };

  const l10nImpl = {
    t: (...args: unknown[]) => {
      const first = args[0];
      const message = typeof first === "string" ? first : (first as { message: string }).message;
      const params = typeof first === "string" ? args.slice(1) : ((first as { args?: unknown[] | Record<string, unknown> }).args ?? []);
      return message.replace(/\{(\w+)\}/g, (m, key) => {
        const v = Array.isArray(params) ? params[Number(key)] : (params as Record<string, unknown>)[key];
        return v === undefined ? m : String(v);
      });
    },
    bundle: undefined,
    uri: undefined,
  };

  const top: Record<string, unknown> = {
    version: VSCODE_API_VERSION,
    commands: namespace(ctx, "commands", commandsImpl),
    window: namespace(ctx, "window", windowImpl),
    workspace: namespace(ctx, "workspace", workspaceImpl),
    languages: namespace(ctx, "languages", languagesImpl),
    env: namespace(ctx, "env", envImpl),
    extensions: namespace(ctx, "extensions", extensionsImpl),
    l10n: namespace(ctx, "l10n", l10nImpl),
  };
  for (const [name, value] of Object.entries(types)) {
    if (/^[A-Z]/.test(name)) top[name] = value;
  }
  top.TextEditorDecorationType = TextEditorDecorationType;

  return {
    api: moduleProxy(ctx, top),
    commands,
    editors,
    workspace,
    languages,
    statusBar,
    output,
    views,
    contextKeys: commands.contextKeys,
  };
}

function isPanelContainer(ctx: HostContext, container: string): boolean {
  if (container === "panel") return true;
  const panel = (ctx.ext.manifest.contributes?.viewsContainers?.panel ?? []) as { id: string }[];
  return panel.some((c) => c.id === container);
}

function notImplementedFunction(ctx: HostContext, api: string): (...args: unknown[]) => never {
  return () => {
    ctx.coverage.missing(api);
    throw new NotImplementedInFyx(api);
  };
}

/** Wraps a namespace object so every access is recorded and unknown VS Code names fail loudly. */
export function namespace<T extends object>(ctx: HostContext, name: string, impl: T): T {
  const known = catalog.namespaces[name];
  return new Proxy(impl, {
    get(target, prop, receiver) {
      if (typeof prop === "symbol") return Reflect.get(target, prop, receiver);
      const api = `vscode.${name}.${prop}`;
      if (prop in target) {
        ctx.coverage.use(api);
        return Reflect.get(target, prop, receiver);
      }
      if (known?.functions.includes(prop) || known?.events.includes(prop)) return notImplementedFunction(ctx, api);
      if (known?.values.includes(prop)) {
        ctx.coverage.missing(api);
        throw new NotImplementedInFyx(api);
      }
      if (prop !== "then" && prop !== "toJSON") ctx.coverage.unknownName(api);
      return undefined;
    },
    has(target, prop) {
      if (typeof prop === "string" && known && (known.functions.includes(prop) || known.events.includes(prop) || known.values.includes(prop))) return true;
      return Reflect.has(target, prop);
    },
  });
}

function moduleProxy(ctx: HostContext, top: Record<string, unknown>): Record<string, unknown> {
  const namespaces = new Set(Object.keys(catalog.namespaces));
  const classes = new Set([...catalog.classes, ...catalog.enums]);
  const stubs = new Map<string, unknown>();
  return new Proxy(top, {
    get(target, prop, receiver) {
      if (typeof prop === "symbol") return Reflect.get(target, prop, receiver);
      const api = `vscode.${prop}`;
      if (prop in target) {
        if (typeof target[prop] !== "object" || target[prop] === null) ctx.coverage.use(api);
        return Reflect.get(target, prop, receiver);
      }
      if (namespaces.has(prop)) {
        let ns = stubs.get(prop);
        if (!ns) stubs.set(prop, (ns = namespace(ctx, prop, {})));
        return ns;
      }
      if (classes.has(prop)) {
        let stub = stubs.get(prop);
        if (!stub) {
          // A constructor that fails on use; static members fail too.
          const fail = () => {
            ctx.coverage.missing(api);
            throw new NotImplementedInFyx(api);
          };
          stub = new Proxy(function NotImplemented() {}, {
            construct: fail,
            apply: fail,
            get: (t, p) => (p === "prototype" || p === "name" || typeof p === "symbol" ? Reflect.get(t, p) : fail()),
          });
          stubs.set(prop, stub);
        }
        return stub;
      }
      if (prop !== "__esModule" && prop !== "default" && prop !== "then") ctx.coverage.unknownName(api);
      return undefined;
    },
    has(target, prop) {
      return Reflect.has(target, prop) || (typeof prop === "string" && (namespaces.has(prop) || classes.has(prop)));
    },
  });
}

/** An object whose every member throws `NotImplementedInFyx` (ExtensionContext.secrets and friends). */
export function notImplementedObject(ctx: HostContext, api: string): object {
  return new Proxy(
    {},
    {
      get(_t, prop) {
        if (typeof prop === "symbol" || prop === "then" || prop === "toJSON") return undefined;
        const name = `${api}.${prop}`;
        ctx.coverage.missing(name);
        throw new NotImplementedInFyx(name);
      },
    },
  );
}

class Memento {
  private file: string;
  private data: Record<string, unknown>;
  private syncKeys: readonly string[] = [];
  constructor(file: string) {
    this.file = file;
    try {
      this.data = JSON.parse(readFileSync(file, "utf8"));
    } catch {
      this.data = {};
    }
  }
  keys(): readonly string[] {
    return Object.keys(this.data);
  }
  get<T>(key: string, defaultValue?: T): T | undefined {
    return Object.hasOwn(this.data, key) ? structuredClone(this.data[key] as T) : defaultValue;
  }
  async update(key: string, value: unknown): Promise<void> {
    if (value === undefined) delete this.data[key];
    else this.data[key] = structuredClone(value);
    try {
      mkdirSync(join(this.file, ".."), { recursive: true });
      writeFileSync(this.file, JSON.stringify(this.data));
    } catch {
      // Storage is best-effort (the permission model may forbid it); values stay in memory.
    }
  }
  setKeysForSync(keys: readonly string[]): void {
    this.syncKeys = keys;
  }
}

function extensionObject(ctx: HostContext): Record<string, unknown> {
  return {
    id: ctx.ext.id,
    extensionUri: ctx.ext.uri,
    extensionPath: ctx.ext.path,
    get isActive() {
      return true;
    },
    packageJSON: ctx.ext.manifest,
    extensionKind: types.ExtensionKind.Workspace,
    exports: undefined,
    activate: async () => undefined,
  };
}

export function createExtensionContext(ctx: HostContext): Record<string, unknown> {
  const storageUri = types.Uri.file(ctx.storagePath);
  const globalStorageUri = types.Uri.file(ctx.globalStoragePath);
  const logUri = types.Uri.file(ctx.logPath);
  const context = {
    subscriptions: [] as { dispose(): unknown }[],
    workspaceState: new Memento(join(ctx.storagePath, "workspaceState.json")),
    globalState: new Memento(join(ctx.globalStoragePath, "globalState.json")),
    secrets: notImplementedObject(ctx, "vscode.ExtensionContext.secrets"),
    environmentVariableCollection: notImplementedObject(ctx, "vscode.ExtensionContext.environmentVariableCollection"),
    languageModelAccessInformation: notImplementedObject(ctx, "vscode.ExtensionContext.languageModelAccessInformation"),
    extensionUri: ctx.ext.uri,
    extensionPath: ctx.ext.path,
    asAbsolutePath: (rel: string) => join(ctx.ext.path, rel),
    storageUri,
    storagePath: ctx.storagePath,
    globalStorageUri,
    globalStoragePath: ctx.globalStoragePath,
    logUri,
    logPath: ctx.logPath,
    extensionMode: types.ExtensionMode.Production,
    extension: extensionObject(ctx),
  };
  return namespaceLike(ctx, "ExtensionContext", context);
}

/** Records access to ExtensionContext members like a namespace (not in the catalog). */
function namespaceLike<T extends object>(ctx: HostContext, name: string, impl: T): T {
  return new Proxy(impl, {
    get(target, prop, receiver) {
      if (typeof prop === "string" && prop in target) ctx.coverage.use(`vscode.${name}.${prop}`);
      return Reflect.get(target, prop, receiver);
    },
  });
}
