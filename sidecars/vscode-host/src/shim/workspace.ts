// `vscode.workspace`: folders, file system (local Node fs, subject to the sidecar's permission
// flags), documents, `findFiles`, `applyEdit`, configuration and save events.

import { promises as fsp, statSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import type { HostContext } from "../context.ts";
import type { EditorStore } from "./editors.ts";
import { languageIdForPath, type TextDocument } from "./documents.ts";
import { globToRegExp } from "./languages.ts";
import { encodeTextEdit } from "./wire.ts";
import { Disposable, EventEmitter, FileSystemError, FileType, RelativePattern, TextDocumentSaveReason, TextEdit, Uri, WorkspaceEdit, toUri } from "./types.ts";

export class WorkspaceFolder {
  readonly uri: Uri;
  readonly name: string;
  readonly index: number;
  constructor(uri: Uri, index: number) {
    this.uri = uri;
    this.index = index;
    this.name = uri.path.split("/").filter(Boolean).pop() ?? uri.path;
  }
}

function fsError(err: unknown, uri: Uri): Error {
  const code = (err as NodeJS.ErrnoException).code;
  if (code === "ENOENT") return FileSystemError.FileNotFound(uri);
  if (code === "EEXIST") return FileSystemError.FileExists(uri);
  if (code === "ENOTDIR") return FileSystemError.FileNotADirectory(uri);
  if (code === "EISDIR") return FileSystemError.FileIsADirectory(uri);
  if (code === "EACCES" || code === "EPERM" || code === "ERR_ACCESS_DENIED") return FileSystemError.NoPermissions(uri);
  return err as Error;
}

function local(uri: Uri): string {
  if (uri.scheme !== "file") throw FileSystemError.Unavailable(`${uri.toString()} (Fyx only provides the file: scheme)`);
  return uri.fsPath;
}

/** `workspace.fs`. */
export function createFileSystem() {
  const wrap = async <T>(uri: Uri, f: (path: string) => Promise<T>): Promise<T> => {
    try {
      return await f(local(uri));
    } catch (err) {
      throw fsError(err, uri);
    }
  };
  return {
    stat: (uri: Uri) =>
      wrap(uri, async (p) => {
        const s = await fsp.lstat(p);
        const target = s.isSymbolicLink() ? await fsp.stat(p).catch(() => s) : s;
        const type = (target.isDirectory() ? FileType.Directory : target.isFile() ? FileType.File : FileType.Unknown) | (s.isSymbolicLink() ? FileType.SymbolicLink : 0);
        return { type, ctime: s.ctimeMs, mtime: s.mtimeMs, size: s.size };
      }),
    readDirectory: (uri: Uri) =>
      wrap(uri, async (p) =>
        (await fsp.readdir(p, { withFileTypes: true })).map((e) => [e.name, e.isDirectory() ? FileType.Directory : e.isFile() ? FileType.File : e.isSymbolicLink() ? FileType.SymbolicLink : FileType.Unknown] as [string, number]),
      ),
    createDirectory: (uri: Uri) => wrap(uri, async (p) => void (await fsp.mkdir(p, { recursive: true }))),
    readFile: (uri: Uri) => wrap(uri, async (p) => new Uint8Array(await fsp.readFile(p))),
    writeFile: (uri: Uri, content: Uint8Array) =>
      wrap(uri, async (p) => {
        await fsp.mkdir(dirname(p), { recursive: true });
        await fsp.writeFile(p, content);
      }),
    delete: (uri: Uri, options?: { recursive?: boolean; useTrash?: boolean }) => wrap(uri, (p) => fsp.rm(p, { recursive: options?.recursive ?? false })),
    rename: (source: Uri, target: Uri, options?: { overwrite?: boolean }) =>
      wrap(source, async (p) => {
        const to = local(target);
        if (!options?.overwrite && (await fsp.stat(to).then(() => true, () => false))) throw FileSystemError.FileExists(target);
        await fsp.rename(p, to);
      }),
    copy: (source: Uri, target: Uri, options?: { overwrite?: boolean }) => wrap(source, (p) => fsp.cp(p, local(target), { recursive: true, force: options?.overwrite ?? false, errorOnExist: !options?.overwrite })),
    isWritableFileSystem: (scheme: string) => (scheme === "file" ? true : undefined),
  };
}

export class WorkspaceApi {
  private ctx: HostContext;
  private editors: EditorStore;
  readonly fs = createFileSystem();
  readonly willSave = new EventEmitter<{ document: TextDocument; reason: number; waitUntil(t: PromiseLike<unknown>): void }>();
  readonly foldersChanged = new EventEmitter<{ added: WorkspaceFolder[]; removed: WorkspaceFolder[] }>();
  readonly contentProviders = new Map<string, { provideTextDocumentContent(uri: Uri, token: unknown): string | PromiseLike<string> }>();
  folders: WorkspaceFolder[];
  private watchers = new Set<FileWatcher>();

  constructor(ctx: HostContext, editors: EditorStore) {
    this.ctx = ctx;
    this.editors = editors;
    this.folders = ctx.workspaceFolders.map((u, i) => new WorkspaceFolder(u, i));
    ctx.documents.saveHook = (doc) => this.save(doc);
  }

  getWorkspaceFolder(uri: Uri): WorkspaceFolder | undefined {
    return this.folders
      .filter((f) => uri.path === f.uri.path || uri.path.startsWith(`${f.uri.path.replace(/\/$/, "")}/`))
      .sort((a, b) => b.uri.path.length - a.uri.path.length)[0];
  }

  asRelativePath(pathOrUri: string | Uri, includeWorkspaceFolder?: boolean): string {
    const uri = typeof pathOrUri === "string" ? (/^[a-z][\w+.-]+:/i.test(pathOrUri) && !/^[a-z]:[\\/]/i.test(pathOrUri) ? Uri.parse(pathOrUri) : Uri.file(pathOrUri)) : pathOrUri;
    const folder = this.getWorkspaceFolder(uri);
    if (!folder) return typeof pathOrUri === "string" ? pathOrUri : uri.fsPath;
    const rel = uri.path.slice(folder.uri.path.replace(/\/$/, "").length + 1);
    return (includeWorkspaceFolder ?? this.folders.length > 1) ? `${folder.name}/${rel}` : rel;
  }

  async openTextDocument(arg?: Uri | string | { language?: string; content?: string; encoding?: string }): Promise<TextDocument> {
    if (arg === undefined || (typeof arg === "object" && !(arg instanceof Uri) && !("scheme" in arg))) {
      const opts = (arg ?? {}) as { language?: string; content?: string };
      const uri = Uri.from({ scheme: "untitled", path: `Untitled-${Date.now()}` });
      return this.ctx.documents.open(uri, opts.language ?? "plaintext", 1, opts.content ?? "");
    }
    const uri = toUri(arg);
    const existing = this.ctx.documents.get(uri);
    if (existing) return existing;
    const provider = this.contentProviders.get(uri.scheme);
    if (provider) {
      const text = await provider.provideTextDocumentContent(uri, { isCancellationRequested: false, onCancellationRequested: () => new Disposable(() => {}) });
      return this.ctx.documents.open(uri, languageIdForPath(uri.path, this.ctx.ext.manifest.contributes?.languages), 1, text ?? "");
    }
    const bytes = await this.fs.readFile(uri);
    const text = new TextDecoder("utf-8").decode(bytes);
    const doc = this.ctx.documents.open(uri, languageIdForPath(uri.path, this.ctx.ext.manifest.contributes?.languages), 1, text);
    doc.isDirty = false;
    return doc;
  }

  async applyEdit(edit: WorkspaceEdit): Promise<boolean> {
    const changes: { uri: string; version: number | null; edits: unknown[] }[] = [];
    const local: { doc: TextDocument; version: number; edits: TextEdit[] }[] = [];
    for (const [uri, edits] of edit.entries()) {
      const doc = this.ctx.documents.get(uri) ?? (await this.openTextDocument(uri).catch(() => undefined));
      changes.push({ uri: uri.toString(), version: doc?.version ?? null, edits: edits.map(encodeTextEdit) });
      if (doc) local.push({ doc, version: doc.version, edits: edits as TextEdit[] });
    }
    const fileOperations = edit.fileOperations.map((op) => ({ kind: op.kind, uri: op.uri.toString(), ...(op.newUri ? { newUri: op.newUri.toString() } : {}) }));
    if (changes.length === 0 && fileOperations.length === 0) return true;
    // The host applies a workspace edit all or nothing; local documents change only after it did.
    const ok = await this.ctx.rpc.request<boolean>("vscode/applyEdit", { edits: changes, ...(fileOperations.length ? { fileOperations } : {}) });
    if (ok !== true) return false;
    for (const { doc, version, edits } of local) this.ctx.documents.applyConfirmed(doc, version, edits);
    return true;
  }

  async findFiles(include: string | RelativePattern, exclude?: string | RelativePattern | null, maxResults?: number, token?: { isCancellationRequested: boolean }): Promise<Uri[]> {
    const roots = include instanceof RelativePattern ? [include.baseUri] : this.folders.map((f) => f.uri);
    const includeRe = globToRegExp(include instanceof RelativePattern ? include.pattern : include);
    const excludes: RegExp[] = [];
    if (exclude === undefined) {
      const filesExclude = this.ctx.config.get("files").get<Record<string, boolean>>("exclude", {}) ?? {};
      for (const [glob, on] of Object.entries(filesExclude)) if (on) excludes.push(globToRegExp(glob));
    } else if (exclude) excludes.push(globToRegExp(exclude instanceof RelativePattern ? exclude.pattern : exclude));
    const out: Uri[] = [];
    const limit = maxResults ?? Number.POSITIVE_INFINITY;
    for (const root of roots) {
      const base = root.fsPath;
      const walk = async (dir: string): Promise<void> => {
        if (out.length >= limit || token?.isCancellationRequested) return;
        let entries;
        try {
          entries = await fsp.readdir(dir, { withFileTypes: true });
        } catch {
          return;
        }
        for (const e of entries) {
          const full = join(dir, e.name);
          const rel = relative(base, full).split(sep).join("/");
          if (excludes.some((re) => re.test(rel) || re.test(`${rel}/`))) continue;
          if (e.isDirectory()) {
            if (e.name === "node_modules" && exclude === undefined) continue;
            await walk(full);
          } else if (includeRe.test(rel)) {
            out.push(Uri.file(full));
            if (out.length >= limit) return;
          }
        }
      };
      await walk(base);
    }
    return out;
  }

  async save(doc: TextDocument): Promise<boolean> {
    const extra: TextEdit[] = [];
    const pending: PromiseLike<unknown>[] = [];
    this.willSave.fire({ document: doc, reason: TextDocumentSaveReason.Manual, waitUntil: (t) => void pending.push(t) });
    for (const result of await Promise.all(pending)) if (Array.isArray(result)) extra.push(...result);
    if (extra.length) await this.editors.applyEdits(doc, extra);
    const ok = await this.ctx.rpc.request<boolean>("vscode/saveTextDocument", { uri: doc.uri.toString() });
    if (ok !== false) {
      doc.isDirty = false;
      this.ctx.documents.onDidSave.fire(doc);
    }
    return ok !== false;
  }

  /** Host → sidecar: the user is saving; returns edits the extension wants applied first. */
  async willSaveFromHost(doc: TextDocument, reason: number): Promise<unknown[]> {
    const pending: PromiseLike<unknown>[] = [];
    this.willSave.fire({ document: doc, reason, waitUntil: (t) => void pending.push(t) });
    const edits: TextEdit[] = [];
    for (const result of await Promise.all(pending)) if (Array.isArray(result)) edits.push(...result.filter((e) => e instanceof TextEdit));
    return edits.map(encodeTextEdit);
  }

  /**
   * File events come from the host (`vscode/didChangeWatchedFiles`), which watches the workspace;
   * the sidecar filters them per watcher by glob and the ignore flags.
   */
  createFileSystemWatcher(glob: string | RelativePattern, ignoreCreate = false, ignoreChange = false, ignoreDelete = false) {
    const pattern = glob instanceof RelativePattern ? glob.pattern : glob;
    const re = globToRegExp(pattern);
    const base = glob instanceof RelativePattern ? glob.baseUri.path.replace(/\/$/, "") : undefined;
    const matches = (uri: Uri) => {
      if (base === undefined) return re.test(uri.path);
      return uri.path.startsWith(`${base}/`) && re.test(uri.path.slice(base.length + 1));
    };
    const watcher: FileWatcher = {
      matches,
      ignore: [ignoreCreate, ignoreChange, ignoreDelete],
      emitters: [new EventEmitter<Uri>(), new EventEmitter<Uri>(), new EventEmitter<Uri>()],
    };
    this.watchers.add(watcher);
    const [created, changed, deleted] = watcher.emitters;
    const watchers = this.watchers;
    return {
      ignoreCreateEvents: ignoreCreate,
      ignoreChangeEvents: ignoreChange,
      ignoreDeleteEvents: ignoreDelete,
      onDidCreate: created.event,
      onDidChange: changed.event,
      onDidDelete: deleted.event,
      dispose() {
        watchers.delete(watcher);
        for (const emitter of watcher.emitters) emitter.dispose();
      },
    };
  }

  /** `vscode/didChangeWatchedFiles`; `type` is LSP's FileChangeType: 1 created, 2 changed, 3 deleted. */
  didChangeWatchedFiles(changes: readonly { uri: string; type: number }[]): void {
    for (const change of changes) {
      const kind = change.type - 1;
      if (kind < 0 || kind > 2) throw new Error(`didChangeWatchedFiles: unknown change type ${change.type}`);
      const uri = toUri(change.uri);
      for (const watcher of [...this.watchers]) {
        if (!watcher.ignore[kind] && watcher.matches(uri)) watcher.emitters[kind].fire(uri);
      }
    }
  }
}

interface FileWatcher {
  matches(uri: Uri): boolean;
  /** Indexed by change kind: created, changed, deleted. */
  ignore: [boolean, boolean, boolean];
  emitters: [EventEmitter<Uri>, EventEmitter<Uri>, EventEmitter<Uri>];
}

export function isDirectory(path: string): boolean {
  try {
    return statSync(path).isDirectory();
  } catch {
    return false;
  }
}
