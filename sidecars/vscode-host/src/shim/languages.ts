// `vscode.languages`: diagnostics and the language features Fyx forwards (formatting, completion,
// hover). Registrations are announced with `vscode/registerProvider` so the host knows which
// documents to ask about; the host then calls `vscode/provide*`.

import type { HostContext } from "../context.ts";
import type { TextDocument } from "./documents.ts";
import { decodePosition, decodeRange, encodeCompletions, encodeDiagnostic, encodeHover, encodeTextEdit, type WirePosition, type WireRange } from "./wire.ts";
import { CancellationTokenSource, CompletionTriggerKind, Diagnostic, Disposable, EventEmitter, Uri, toUri } from "./types.ts";

export type DocumentSelector = string | DocumentFilter | readonly (string | DocumentFilter)[];
interface DocumentFilter {
  language?: string;
  scheme?: string;
  pattern?: string | { pattern: string };
  notebookType?: string;
}

/** `languages.match`: 10 exact language, 5 wildcard, 0 no match (VS Code's scoring, simplified). */
export function matchSelector(selector: DocumentSelector, doc: TextDocument): number {
  const list = Array.isArray(selector) ? selector : [selector];
  let best = 0;
  for (const s of list as (string | DocumentFilter)[]) {
    let score = 0;
    if (typeof s === "string") score = s === doc.languageId ? 10 : s === "*" ? 5 : 0;
    else {
      score = 10;
      if (s.language !== undefined) score = s.language === doc.languageId ? 10 : s.language === "*" ? 5 : 0;
      if (score && s.scheme !== undefined) score = s.scheme === doc.uri.scheme || s.scheme === "*" ? score : 0;
      if (score && s.pattern !== undefined) {
        const pattern = typeof s.pattern === "string" ? s.pattern : s.pattern.pattern;
        score = globToRegExp(pattern).test(doc.uri.path) || globToRegExp(pattern).test(doc.fileName.replace(/\\/g, "/")) ? Math.max(score, 5) : 0;
      }
      if (s.language === undefined && s.scheme === undefined && s.pattern === undefined) score = 0;
    }
    best = Math.max(best, score);
  }
  return best;
}

/** Glob → RegExp supporting `**`, `*`, `?`, `{a,b}` and `[...]`; matches a whole POSIX path or its suffix after `/`. */
export function globToRegExp(glob: string): RegExp {
  let re = "";
  let inGroup = 0;
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i];
    if (c === "*") {
      if (glob[i + 1] === "*") {
        i++;
        if (glob[i + 1] === "/") {
          i++;
          re += "(?:.*/)?";
        } else re += ".*";
      } else re += "[^/]*";
    } else if (c === "?") re += "[^/]";
    else if (c === "{") {
      inGroup++;
      re += "(?:";
    } else if (c === "}" && inGroup) {
      inGroup--;
      re += ")";
    } else if (c === "," && inGroup) re += "|";
    else if (c === "[") {
      const end = glob.indexOf("]", i);
      if (end < 0) re += "\\[";
      else {
        re += `[${glob.slice(i + 1, end).replace(/^!/, "^").replace(/\\/g, "\\\\")}]`;
        i = end;
      }
    } else re += c.replace(/[.+^$()|\\]/g, "\\$&");
  }
  return new RegExp(glob.startsWith("**") || glob.startsWith("/") ? `^${re}$` : `(?:^|/)${re}$`);
}

export class DiagnosticCollectionImpl {
  readonly name: string;
  private ctx: HostContext;
  private registry: LanguageFeatures;
  private entries = new Map<string, { uri: Uri; diagnostics: readonly Diagnostic[] }>();

  constructor(ctx: HostContext, registry: LanguageFeatures, name: string) {
    this.ctx = ctx;
    this.registry = registry;
    this.name = name;
  }

  set(uriOrEntries: Uri | readonly [Uri, readonly Diagnostic[] | undefined][], diagnostics?: readonly Diagnostic[] | undefined): void {
    if (Array.isArray(uriOrEntries)) {
      const grouped = new Map<string, { uri: Uri; diagnostics: Diagnostic[] }>();
      for (const [uri, diags] of uriOrEntries as [Uri, readonly Diagnostic[] | undefined][]) {
        const g = grouped.get(uri.toString()) ?? { uri, diagnostics: [] };
        if (diags === undefined) g.diagnostics = [];
        else g.diagnostics.push(...diags);
        grouped.set(uri.toString(), g);
      }
      for (const g of grouped.values()) this.publish(g.uri, g.diagnostics);
    } else this.publish(uriOrEntries as Uri, diagnostics ?? []);
  }

  delete(uri: Uri): void {
    this.publish(uri, []);
  }

  clear(): void {
    for (const { uri } of [...this.entries.values()]) this.publish(uri, []);
  }

  forEach(callback: (uri: Uri, diagnostics: readonly Diagnostic[], collection: this) => unknown, thisArg?: unknown): void {
    for (const { uri, diagnostics } of this.entries.values()) callback.call(thisArg, uri, diagnostics, this);
  }

  get(uri: Uri): readonly Diagnostic[] | undefined {
    return this.entries.get(uri.toString())?.diagnostics;
  }

  has(uri: Uri): boolean {
    return this.entries.has(uri.toString());
  }

  dispose(): void {
    this.clear();
    this.registry.collections.delete(this);
  }

  *[Symbol.iterator](): IterableIterator<[Uri, readonly Diagnostic[]]> {
    for (const { uri, diagnostics } of this.entries.values()) yield [uri, diagnostics];
  }

  private publish(uri: Uri, diagnostics: readonly Diagnostic[]): void {
    const key = uri.toString();
    if (diagnostics.length === 0) this.entries.delete(key);
    else this.entries.set(key, { uri, diagnostics: [...diagnostics] });
    this.ctx.rpc.notify("vscode/publishDiagnostics", { owner: `${this.ctx.ext.id}/${this.name}`, uri: key, diagnostics: diagnostics.map(encodeDiagnostic) });
    this.registry.diagnosticsChanged.fire({ uris: [uri] });
  }
}

type ProviderKind = "documentFormatting" | "documentRangeFormatting" | "onTypeFormatting" | "completion" | "hover";

interface Registration {
  id: number;
  kind: ProviderKind;
  selector: DocumentSelector;
  provider: Record<string, (...args: any[]) => any>;
  extra?: Record<string, unknown>;
}

export class LanguageFeatures {
  private ctx: HostContext;
  private registrations = new Map<number, Registration>();
  private serial = 0;
  readonly collections = new Set<DiagnosticCollectionImpl>();
  readonly diagnosticsChanged = new EventEmitter<{ uris: readonly Uri[] }>();
  readonly languageConfigurations = new Map<string, unknown>();

  constructor(ctx: HostContext) {
    this.ctx = ctx;
  }

  createDiagnosticCollection(name?: string): DiagnosticCollectionImpl {
    const c = new DiagnosticCollectionImpl(this.ctx, this, name ?? `diagnostics${this.collections.size + 1}`);
    this.collections.add(c);
    return c;
  }

  getDiagnostics(uri?: Uri): unknown {
    if (uri) return [...this.collections].flatMap((c) => [...(c.get(uri) ?? [])]);
    const all = new Map<string, { uri: Uri; diags: Diagnostic[] }>();
    for (const c of this.collections) {
      for (const [u, diags] of c) {
        const e = all.get(u.toString()) ?? { uri: u, diags: [] };
        e.diags.push(...diags);
        all.set(u.toString(), e);
      }
    }
    return [...all.values()].map((e) => [e.uri, e.diags]);
  }

  register(kind: ProviderKind, selector: DocumentSelector, provider: Registration["provider"], extra?: Record<string, unknown>): Disposable {
    const id = ++this.serial;
    this.registrations.set(id, { id, kind, selector, provider, extra });
    this.ctx.rpc.notify("vscode/registerProvider", { id, kind, selector: encodeSelector(selector), ...(extra ?? {}) });
    return new Disposable(() => {
      this.registrations.delete(id);
      this.ctx.rpc.notify("vscode/unregisterProvider", { id });
    });
  }

  private best(kind: ProviderKind, doc: TextDocument): Registration | undefined {
    let best: Registration | undefined;
    let bestScore = 0;
    for (const r of this.registrations.values()) {
      if (r.kind !== kind) continue;
      const score = matchSelector(r.selector, doc);
      if (score > 0 && score >= bestScore) {
        best = r;
        bestScore = score;
      }
    }
    return best;
  }

  private all(kind: ProviderKind, doc: TextDocument): Registration[] {
    return [...this.registrations.values()].filter((r) => r.kind === kind && matchSelector(r.selector, doc) > 0);
  }

  // Host → sidecar requests -------------------------------------------------------------------

  async provideFormatting(params: { uri: string; options: Record<string, unknown>; range?: WireRange }): Promise<unknown[]> {
    const doc = this.document(params.uri);
    const token = new CancellationTokenSource().token;
    const options = { tabSize: 4, insertSpaces: true, ...params.options };
    if (params.range) {
      const r = this.best("documentRangeFormatting", doc);
      if (!r) return [];
      const edits = await r.provider.provideDocumentRangeFormattingEdits(doc, decodeRange(params.range), options, token);
      return (edits ?? []).map(encodeTextEdit);
    }
    const r = this.best("documentFormatting", doc) ?? this.best("documentRangeFormatting", doc);
    if (!r) return [];
    const edits =
      r.kind === "documentFormatting"
        ? await r.provider.provideDocumentFormattingEdits(doc, options, token)
        : await r.provider.provideDocumentRangeFormattingEdits(doc, doc.validateRange(decodeRange({ start: { line: 0, character: 0 }, end: { line: doc.lineCount, character: 0 } })), options, token);
    return (edits ?? []).map(encodeTextEdit);
  }

  async provideOnTypeFormatting(params: { uri: string; position: WirePosition; ch: string; options: Record<string, unknown> }): Promise<unknown[]> {
    const doc = this.document(params.uri);
    const r = this.all("onTypeFormatting", doc).find((x) => (x.extra?.triggerCharacters as string[] | undefined)?.includes(params.ch));
    if (!r) return [];
    const edits = await r.provider.provideOnTypeFormattingEdits(doc, decodePosition(params.position), params.ch, { tabSize: 4, insertSpaces: true, ...params.options }, new CancellationTokenSource().token);
    return (edits ?? []).map(encodeTextEdit);
  }

  async provideCompletion(params: { uri: string; position: WirePosition; triggerCharacter?: string }): Promise<unknown> {
    const doc = this.document(params.uri);
    const items: Record<string, unknown>[] = [];
    let isIncomplete = false;
    for (const r of this.all("completion", doc)) {
      const triggers = (r.extra?.triggerCharacters as string[] | undefined) ?? [];
      if (params.triggerCharacter && !triggers.includes(params.triggerCharacter)) continue;
      const context = { triggerKind: params.triggerCharacter ? CompletionTriggerKind.TriggerCharacter : CompletionTriggerKind.Invoke, triggerCharacter: params.triggerCharacter };
      const result = encodeCompletions(await r.provider.provideCompletionItems(doc, decodePosition(params.position), new CancellationTokenSource().token, context));
      items.push(...result.items);
      isIncomplete ||= result.isIncomplete;
    }
    return { isIncomplete, items };
  }

  async provideHover(params: { uri: string; position: WirePosition }): Promise<unknown> {
    const doc = this.document(params.uri);
    const contents: unknown[] = [];
    let range: unknown;
    for (const r of this.all("hover", doc)) {
      const hover = encodeHover(await r.provider.provideHover(doc, decodePosition(params.position), new CancellationTokenSource().token));
      if (hover) {
        contents.push(...(hover.contents as unknown[]));
        range ??= hover.range;
      }
    }
    return contents.length ? { contents, ...(range ? { range } : {}) } : null;
  }

  private document(uri: string): TextDocument {
    const doc = this.ctx.documents.get(toUri(uri));
    if (!doc) throw Object.assign(new Error(`document not open: ${uri}`), { code: -32602 });
    return doc;
  }
}

function encodeSelector(selector: DocumentSelector): unknown {
  const list = Array.isArray(selector) ? selector : [selector];
  return list.map((s) => (typeof s === "string" ? { language: s } : { ...s, ...(typeof s.pattern === "object" && s.pattern ? { pattern: s.pattern.pattern } : {}) }));
}
