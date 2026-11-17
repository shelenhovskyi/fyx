// Text documents. The host owns buffers; the sidecar keeps full-text mirrors that the host syncs
// with `vscode/didOpenTextDocument` / `vscode/didChangeTextDocument` (full text, like LSP's
// `TextDocumentSyncKind.Full`). Edits made by the extension are applied locally first and sent to
// the host with `vscode/applyEdit`.

import { extname, basename } from "node:path";
import { EndOfLine, EventEmitter, Position, Range, TextEdit, Uri } from "./types.ts";

export class TextLine {
  readonly lineNumber: number;
  readonly text: string;
  readonly range: Range;
  readonly rangeIncludingLineBreak: Range;
  constructor(lineNumber: number, text: string, hasBreak: boolean) {
    this.lineNumber = lineNumber;
    this.text = text;
    this.range = new Range(lineNumber, 0, lineNumber, text.length);
    this.rangeIncludingLineBreak = hasBreak ? new Range(lineNumber, 0, lineNumber + 1, 0) : this.range;
  }
  get firstNonWhitespaceCharacterIndex(): number {
    const m = /\S/.exec(this.text);
    return m ? m.index : this.text.length;
  }
  get isEmptyOrWhitespace(): boolean {
    return this.firstNonWhitespaceCharacterIndex === this.text.length;
  }
}

export interface ContentChange {
  range: Range;
  rangeOffset: number;
  rangeLength: number;
  text: string;
}

export class TextDocument {
  readonly uri: Uri;
  languageId: string;
  version: number;
  isDirty = false;
  isClosed = false;
  private text = "";
  private lineStarts: number[] = [0];
  private saveHook: (doc: TextDocument) => Promise<boolean>;

  constructor(uri: Uri, languageId: string, version: number, text: string, saveHook: (doc: TextDocument) => Promise<boolean>) {
    this.uri = uri;
    this.languageId = languageId;
    this.version = version;
    this.saveHook = saveHook;
    this.setText(text);
  }

  get fileName(): string {
    return this.uri.scheme === "file" ? this.uri.fsPath : this.uri.path;
  }
  get isUntitled(): boolean {
    return this.uri.scheme === "untitled";
  }
  get lineCount(): number {
    return this.lineStarts.length;
  }
  get eol(): number {
    return this.text.includes("\r\n") ? EndOfLine.CRLF : EndOfLine.LF;
  }
  get encoding(): string {
    return "utf8";
  }

  save(): Promise<boolean> {
    return this.saveHook(this);
  }

  getText(range?: Range): string {
    if (!range) return this.text;
    const r = this.validateRange(range);
    return this.text.slice(this.offsetAt(r.start), this.offsetAt(r.end));
  }

  lineAt(lineOrPosition: number | Position): TextLine {
    const line = typeof lineOrPosition === "number" ? lineOrPosition : lineOrPosition.line;
    if (line < 0 || line >= this.lineCount || !Number.isInteger(line)) throw new Error(`Illegal value for \`line\`: ${line}`);
    const start = this.lineStarts[line];
    const nextStart = line + 1 < this.lineCount ? this.lineStarts[line + 1] : this.text.length;
    let end = nextStart;
    if (line + 1 < this.lineCount) end -= this.text[nextStart - 2] === "\r" ? 2 : 1;
    return new TextLine(line, this.text.slice(start, end), line + 1 < this.lineCount);
  }

  offsetAt(position: Position): number {
    const p = this.validatePosition(position);
    return this.lineStarts[p.line] + p.character;
  }

  positionAt(offset: number): Position {
    offset = Math.max(0, Math.min(Math.floor(offset), this.text.length));
    let lo = 0;
    let hi = this.lineStarts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this.lineStarts[mid] <= offset) lo = mid;
      else hi = mid - 1;
    }
    const lineLength = this.lineAt(lo).text.length;
    return new Position(lo, Math.min(offset - this.lineStarts[lo], lineLength));
  }

  validatePosition(position: Position): Position {
    if (position.line >= this.lineCount) {
      const last = this.lineCount - 1;
      return new Position(last, this.lineAt(last).text.length);
    }
    const length = this.lineAt(position.line).text.length;
    return position.character > length ? new Position(position.line, length) : position;
  }

  validateRange(range: Range): Range {
    const start = this.validatePosition(range.start);
    const end = this.validatePosition(range.end);
    return start === range.start && end === range.end ? range : new Range(start, end);
  }

  getWordRangeAtPosition(position: Position, regex?: RegExp): Range | undefined {
    const line = this.lineAt(this.validatePosition(position).line);
    const re = new RegExp(regex?.source ?? /(-?\d*\.\d\w*)|([^`~!@#$%^&*()\-=+[{\]}\\|;:'",.<>/?\s]+)/.source, `${regex?.flags.replace("g", "") ?? ""}g`);
    for (const m of line.text.matchAll(re)) {
      const start = m.index!;
      const end = start + m[0].length;
      if (start <= position.character && position.character <= end && m[0].length > 0) {
        return new Range(line.lineNumber, start, line.lineNumber, end);
      }
    }
    return undefined;
  }

  /** @internal Replaces the whole text. */
  setText(text: string): void {
    this.text = text;
    this.lineStarts = [0];
    for (let i = 0; i < text.length; i++) if (text[i] === "\n") this.lineStarts.push(i + 1);
  }

  /** @internal Applies edits (in original coordinates) and returns the content changes. */
  applyEdits(edits: readonly TextEdit[]): ContentChange[] {
    const resolved = edits
      .map((e, ix) => {
        const range = this.validateRange(e.range);
        return { ix, range, start: this.offsetAt(range.start), end: this.offsetAt(range.end), text: e.newText };
      })
      .sort((a, b) => b.start - a.start || b.ix - a.ix);
    const changes: ContentChange[] = resolved.map((e) => ({ range: e.range, rangeOffset: e.start, rangeLength: e.end - e.start, text: e.text }));
    let text = this.text;
    for (const e of resolved) text = text.slice(0, e.start) + e.text + text.slice(e.end);
    this.setText(text);
    return changes;
  }
}

const EXTENSION_LANGUAGES: Readonly<Record<string, string>> = {
  ".js": "javascript",
  ".mjs": "javascript",
  ".cjs": "javascript",
  ".jsx": "javascriptreact",
  ".ts": "typescript",
  ".mts": "typescript",
  ".cts": "typescript",
  ".tsx": "typescriptreact",
  ".json": "json",
  ".jsonc": "jsonc",
  ".md": "markdown",
  ".py": "python",
  ".rs": "rust",
  ".go": "go",
  ".c": "c",
  ".h": "c",
  ".cpp": "cpp",
  ".cc": "cpp",
  ".hpp": "cpp",
  ".cs": "csharp",
  ".java": "java",
  ".html": "html",
  ".htm": "html",
  ".css": "css",
  ".scss": "scss",
  ".less": "less",
  ".yaml": "yaml",
  ".yml": "yaml",
  ".toml": "toml",
  ".xml": "xml",
  ".svg": "xml",
  ".sh": "shellscript",
  ".bash": "shellscript",
  ".ps1": "powershell",
  ".rb": "ruby",
  ".php": "php",
  ".csv": "csv",
  ".tsv": "tsv",
  ".txt": "plaintext",
  ".http": "http",
  ".rest": "http",
  ".ini": "ini",
  ".sql": "sql",
  ".vue": "vue",
  ".lua": "lua",
};

const FILENAME_LANGUAGES: Readonly<Record<string, string>> = {
  ".editorconfig": "properties",
  Dockerfile: "dockerfile",
  Makefile: "makefile",
};

/** Language id for a path; `extraLanguages` comes from `contributes.languages`. */
export function languageIdForPath(path: string, extraLanguages: readonly { id: string; extensions?: string[]; filenames?: string[] }[] = []): string {
  const name = basename(path);
  const ext = extname(path).toLowerCase();
  for (const lang of extraLanguages) {
    if (lang.filenames?.includes(name)) return lang.id;
    if (lang.extensions?.some((e) => e.toLowerCase() === ext)) return lang.id;
  }
  if (Object.hasOwn(FILENAME_LANGUAGES, name)) return FILENAME_LANGUAGES[name];
  return Object.hasOwn(EXTENSION_LANGUAGES, ext) ? EXTENSION_LANGUAGES[ext] : "plaintext";
}

export function knownLanguageIds(): string[] {
  return [...new Set([...Object.values(EXTENSION_LANGUAGES), ...Object.values(FILENAME_LANGUAGES)])].sort();
}

export interface DocumentChangeEvent {
  document: TextDocument;
  contentChanges: ContentChange[];
  reason: number | undefined;
}

export class DocumentStore {
  private docs = new Map<string, TextDocument>();
  readonly onDidOpen = new EventEmitter<TextDocument>();
  readonly onDidClose = new EventEmitter<TextDocument>();
  readonly onDidChange = new EventEmitter<DocumentChangeEvent>();
  readonly onDidSave = new EventEmitter<TextDocument>();
  saveHook: (doc: TextDocument) => Promise<boolean> = async () => false;

  get all(): TextDocument[] {
    return [...this.docs.values()];
  }

  get(uri: Uri): TextDocument | undefined {
    return this.docs.get(uri.toString());
  }

  open(uri: Uri, languageId: string, version: number, text: string): TextDocument {
    const existing = this.get(uri);
    if (existing) {
      if (version > existing.version || existing.getText() !== text) this.replace(uri, version, text);
      return existing;
    }
    const doc = new TextDocument(uri, languageId, version, text, (d) => this.saveHook(d));
    this.docs.set(uri.toString(), doc);
    this.onDidOpen.fire(doc);
    return doc;
  }

  /**
   * Full-text sync from the host, which owns versions and text. A version older than the local
   * one is a message that crossed a newer sync and is dropped; the local version is only ever the
   * host's, or one past it after an edit the host confirmed (see `applyConfirmed`).
   */
  replace(uri: Uri, version: number, text: string): void {
    const doc = this.get(uri);
    if (!doc || version < doc.version) return;
    const old = doc.getText();
    doc.version = version;
    if (old === text) return;
    const end = doc.positionAt(old.length);
    doc.setText(text);
    doc.isDirty = true;
    this.onDidChange.fire({
      document: doc,
      contentChanges: [{ range: new Range(new Position(0, 0), end), rangeOffset: 0, rangeLength: old.length, text }],
      reason: undefined,
    });
  }

  /**
   * Applies edits the host has just confirmed it applied on top of `baseVersion`. If a host sync
   * arrived first (the version moved on), that text already holds the edits.
   */
  applyConfirmed(doc: TextDocument, baseVersion: number, edits: readonly TextEdit[]): void {
    if (edits.length === 0 || doc.isClosed || doc.version !== baseVersion) return;
    const changes = doc.applyEdits(edits);
    doc.version = baseVersion + 1;
    doc.isDirty = true;
    this.onDidChange.fire({ document: doc, contentChanges: changes, reason: undefined });
  }

  close(uri: Uri): void {
    const doc = this.get(uri);
    if (!doc) return;
    this.docs.delete(uri.toString());
    doc.isClosed = true;
    this.onDidClose.fire(doc);
  }

  setLanguage(doc: TextDocument, languageId: string): TextDocument {
    if (doc.languageId === languageId) return doc;
    this.close(doc.uri);
    return this.open(doc.uri, languageId, doc.version, doc.getText());
  }
}
