// Value types and enums of the `vscode` module. Behaviour follows the VS Code API documentation;
// only what extensions observably rely on is implemented.

import { fileURLToPath } from "node:url";
import { isAbsolute } from "node:path";

// ---------------------------------------------------------------------------------------------
// Enums. TypeScript numeric enums carry a reverse mapping (`DiagnosticSeverity[0] === "Error"`),
// which some extensions use for labels, so we reproduce it.

function numericEnum<T extends Record<string, number>>(values: T): T & Record<number, string> {
  const out: Record<string | number, string | number> = { ...values };
  for (const [k, v] of Object.entries(values)) out[v] = k;
  return Object.freeze(out) as T & Record<number, string>;
}

export const DiagnosticSeverity = numericEnum({ Error: 0, Warning: 1, Information: 2, Hint: 3 });
export const DiagnosticTag = numericEnum({ Unnecessary: 1, Deprecated: 2 });
export const StatusBarAlignment = numericEnum({ Left: 1, Right: 2 });
export const TreeItemCollapsibleState = numericEnum({ None: 0, Collapsed: 1, Expanded: 2 });
export const ConfigurationTarget = numericEnum({ Global: 1, Workspace: 2, WorkspaceFolder: 3 });
export const EndOfLine = numericEnum({ LF: 1, CRLF: 2 });
export const OverviewRulerLane = numericEnum({ Left: 1, Center: 2, Right: 4, Full: 7 });
export const DecorationRangeBehavior = numericEnum({ OpenOpen: 0, ClosedClosed: 1, OpenClosed: 2, ClosedOpen: 3 });
export const TextEditorRevealType = numericEnum({ Default: 0, InCenter: 1, InCenterIfOutsideViewport: 2, AtTop: 3 });
export const ViewColumn = numericEnum({
  Active: -1,
  Beside: -2,
  One: 1,
  Two: 2,
  Three: 3,
  Four: 4,
  Five: 5,
  Six: 6,
  Seven: 7,
  Eight: 8,
  Nine: 9,
});
export const ProgressLocation = numericEnum({ SourceControl: 1, Window: 10, Notification: 15 });
export const FileType = numericEnum({ Unknown: 0, File: 1, Directory: 2, SymbolicLink: 64 });
export const FileChangeType = numericEnum({ Changed: 1, Created: 2, Deleted: 3 });
export const CompletionItemKind = numericEnum({
  Text: 0,
  Method: 1,
  Function: 2,
  Constructor: 3,
  Field: 4,
  Variable: 5,
  Class: 6,
  Interface: 7,
  Module: 8,
  Property: 9,
  Unit: 10,
  Value: 11,
  Enum: 12,
  Keyword: 13,
  Snippet: 14,
  Color: 15,
  File: 16,
  Reference: 17,
  Folder: 18,
  EnumMember: 19,
  Constant: 20,
  Struct: 21,
  Event: 22,
  Operator: 23,
  TypeParameter: 24,
  User: 25,
  Issue: 26,
});
export const CompletionItemTag = numericEnum({ Deprecated: 1 });
export const CompletionTriggerKind = numericEnum({ Invoke: 0, TriggerCharacter: 1, TriggerForIncompleteCompletions: 2 });
export const CodeActionTriggerKind = numericEnum({ Invoke: 1, Automatic: 2 });
export const SymbolKind = numericEnum({
  File: 0,
  Module: 1,
  Namespace: 2,
  Package: 3,
  Class: 4,
  Method: 5,
  Property: 6,
  Field: 7,
  Constructor: 8,
  Enum: 9,
  Interface: 10,
  Function: 11,
  Variable: 12,
  Constant: 13,
  String: 14,
  Number: 15,
  Boolean: 16,
  Array: 17,
  Object: 18,
  Key: 19,
  Null: 20,
  EnumMember: 21,
  Struct: 22,
  Event: 23,
  Operator: 24,
  TypeParameter: 25,
});
export const TextDocumentSaveReason = numericEnum({ Manual: 1, AfterDelay: 2, FocusOut: 3 });
export const TextDocumentChangeReason = numericEnum({ Undo: 1, Redo: 2 });
export const TextEditorSelectionChangeKind = numericEnum({ Keyboard: 1, Mouse: 2, Command: 3 });
export const TextEditorLineNumbersStyle = numericEnum({ Off: 0, On: 1, Relative: 2, Interval: 3 });
export const TextEditorCursorStyle = numericEnum({
  Line: 1,
  Block: 2,
  Underline: 3,
  LineThin: 4,
  BlockOutline: 5,
  UnderlineThin: 6,
});
export const ExtensionMode = numericEnum({ Production: 1, Development: 2, Test: 3 });
export const ExtensionKind = numericEnum({ UI: 1, Workspace: 2 });
export const UIKind = numericEnum({ Desktop: 1, Web: 2 });
export const ColorThemeKind = numericEnum({ Light: 1, Dark: 2, HighContrast: 3, HighContrastLight: 4 });
export const QuickPickItemKind = numericEnum({ Separator: -1, Default: 0 });
export const LogLevel = numericEnum({ Off: 0, Trace: 1, Debug: 2, Info: 3, Warning: 4, Error: 5 });
export const IndentAction = numericEnum({ None: 0, Indent: 1, IndentOutdent: 2, Outdent: 3 });
export const EnvironmentVariableMutatorType = numericEnum({ Replace: 1, Append: 2, Prepend: 3 });

// ---------------------------------------------------------------------------------------------
// Lifetime and events

export class Disposable {
  private callOnDispose: (() => unknown) | undefined;
  constructor(callOnDispose: () => unknown) {
    this.callOnDispose = callOnDispose;
  }
  static from(...disposables: { dispose(): unknown }[]): Disposable {
    return new Disposable(() => {
      for (const d of disposables) d?.dispose?.();
    });
  }
  dispose(): unknown {
    const f = this.callOnDispose;
    this.callOnDispose = undefined;
    return f?.();
  }
}

export type Event<T> = (listener: (e: T) => unknown, thisArgs?: unknown, disposables?: Disposable[]) => Disposable;

export class EventEmitter<T> {
  private listeners = new Set<(e: T) => unknown>();
  private disposed = false;
  readonly event: Event<T> = (listener, thisArgs, disposables) => {
    const bound = thisArgs === undefined ? listener : listener.bind(thisArgs);
    this.listeners.add(bound);
    const d = new Disposable(() => this.listeners.delete(bound));
    disposables?.push(d);
    return d;
  };
  fire(data: T): void {
    if (this.disposed) return;
    for (const l of [...this.listeners]) {
      try {
        l(data);
      } catch (err) {
        // VS Code logs listener errors without breaking the emitter.
        console.error("[vscode-host] event listener threw:", err);
      }
    }
  }
  /** Fires and waits for listeners that return promises (onWillSave and friends). */
  async fireAsync(data: T): Promise<void> {
    for (const l of [...this.listeners]) await l(data);
  }
  get listenerCount(): number {
    return this.listeners.size;
  }
  dispose(): void {
    this.disposed = true;
    this.listeners.clear();
  }
}

export class CancellationError extends Error {
  constructor() {
    super("Canceled");
    this.name = "Canceled";
  }
}

export interface CancellationToken {
  isCancellationRequested: boolean;
  onCancellationRequested: Event<unknown>;
}

export class CancellationTokenSource {
  private emitter = new EventEmitter<unknown>();
  readonly token: CancellationToken;
  constructor() {
    const emitter = this.emitter;
    this.token = { isCancellationRequested: false, onCancellationRequested: emitter.event };
  }
  cancel(): void {
    if (this.token.isCancellationRequested) return;
    this.token.isCancellationRequested = true;
    this.emitter.fire(undefined);
  }
  dispose(): void {
    this.emitter.dispose();
  }
}

export const noneToken: CancellationToken = Object.freeze({
  isCancellationRequested: false,
  onCancellationRequested: () => new Disposable(() => {}),
});

// ---------------------------------------------------------------------------------------------
// Uri

const URI_RE = /^(([^:/?#]+?):)?(\/\/([^/?#]*))?([^?#]*)(\?([^#]*))?(#(.*))?/;

export class Uri {
  readonly scheme: string;
  readonly authority: string;
  readonly path: string;
  readonly query: string;
  readonly fragment: string;

  constructor(scheme: string, authority: string, path: string, query: string, fragment: string) {
    this.scheme = scheme;
    this.authority = authority;
    this.path = path;
    this.query = query;
    this.fragment = fragment;
  }

  static parse(value: string, _strict = false): Uri {
    const m = URI_RE.exec(value);
    if (!m) return new Uri("", "", "", "", "");
    const dec = (s: string | undefined) => (s ? safeDecode(s) : "");
    return new Uri(m[2] ?? "", dec(m[4]), dec(m[5]), dec(m[7]), dec(m[9]));
  }

  static file(fsPath: string): Uri {
    if (process.platform === "win32" || /^[a-zA-Z]:[\\/]/.test(fsPath)) fsPath = fsPath.replace(/\\/g, "/");
    let authority = "";
    if (fsPath.startsWith("//")) {
      const ix = fsPath.indexOf("/", 2);
      authority = ix < 0 ? fsPath.slice(2) : fsPath.slice(2, ix);
      fsPath = ix < 0 ? "/" : fsPath.slice(ix);
    }
    if (!fsPath.startsWith("/")) fsPath = `/${fsPath}`;
    if (/^\/[A-Z]:/.test(fsPath)) fsPath = `/${fsPath[1].toLowerCase()}${fsPath.slice(2)}`;
    return new Uri("file", authority, fsPath, "", "");
  }

  static joinPath(base: Uri, ...segments: string[]): Uri {
    let path = base.path || "/";
    for (const seg of segments) path = `${path.replace(/\/+$/, "")}/${seg.replace(/^\/+/, "")}`;
    return base.with({ path: normalizePosix(path) });
  }

  static from(c: { scheme: string; authority?: string; path?: string; query?: string; fragment?: string }): Uri {
    return new Uri(c.scheme, c.authority ?? "", c.path ?? "", c.query ?? "", c.fragment ?? "");
  }

  static isUri(thing: unknown): thing is Uri {
    return thing instanceof Uri;
  }

  get fsPath(): string {
    let p: string;
    if (this.authority && this.path.length > 1 && this.scheme === "file") p = `//${this.authority}${this.path}`;
    else if (/^\/[a-zA-Z]:/.test(this.path)) p = `${this.path[1].toLowerCase()}${this.path.slice(2)}`;
    else p = this.path;
    return process.platform === "win32" ? p.replace(/\//g, "\\") : p;
  }

  with(change: { scheme?: string; authority?: string | null; path?: string | null; query?: string | null; fragment?: string | null }): Uri {
    return new Uri(
      change.scheme ?? this.scheme,
      change.authority === undefined ? this.authority : (change.authority ?? ""),
      change.path === undefined ? this.path : (change.path ?? ""),
      change.query === undefined ? this.query : (change.query ?? ""),
      change.fragment === undefined ? this.fragment : (change.fragment ?? ""),
    );
  }

  toString(_skipEncoding = false): string {
    let s = this.scheme ? `${this.scheme}:` : "";
    if (this.authority || this.scheme === "file") s += `//${this.authority}`;
    s += this.path.split("/").map(encodeURIComponent).join("/");
    if (this.query) s += `?${encodeURIComponent(this.query)}`;
    if (this.fragment) s += `#${encodeURIComponent(this.fragment)}`;
    return s;
  }

  toJSON(): unknown {
    return { $mid: 1, scheme: this.scheme, authority: this.authority, path: this.path, query: this.query, fragment: this.fragment, fsPath: this.fsPath, external: this.toString() };
  }
}

function safeDecode(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

function normalizePosix(p: string): string {
  const out: string[] = [];
  for (const seg of p.split("/")) {
    if (seg === "..") out.pop();
    else if (seg !== "." && seg !== "") out.push(seg);
  }
  return `/${out.join("/")}`;
}

/** Accepts a `Uri`, a `file:` URI string or a filesystem path. */
export function toUri(value: unknown): Uri {
  if (value instanceof Uri) return value;
  if (typeof value === "string") {
    if (/^[a-zA-Z][\w+.-]+:\/\//.test(value)) {
      if (value.startsWith("file:")) return Uri.file(fileURLToPath(value));
      return Uri.parse(value);
    }
    if (isAbsolute(value)) return Uri.file(value);
  }
  if (value && typeof value === "object" && "scheme" in value) return Uri.from(value as { scheme: string });
  throw new TypeError(`not a Uri: ${String(value)}`);
}

// ---------------------------------------------------------------------------------------------
// Positions and ranges

export class Position {
  readonly line: number;
  readonly character: number;
  constructor(line: number, character: number) {
    if (line < 0 || character < 0) throw new Error("Illegal argument: line and character must be non-negative");
    this.line = line;
    this.character = character;
  }
  isBefore(o: Position): boolean {
    return this.line < o.line || (this.line === o.line && this.character < o.character);
  }
  isBeforeOrEqual(o: Position): boolean {
    return this.isBefore(o) || this.isEqual(o);
  }
  isAfter(o: Position): boolean {
    return !this.isBeforeOrEqual(o);
  }
  isAfterOrEqual(o: Position): boolean {
    return !this.isBefore(o);
  }
  isEqual(o: Position): boolean {
    return this.line === o.line && this.character === o.character;
  }
  compareTo(o: Position): number {
    return this.isBefore(o) ? -1 : this.isEqual(o) ? 0 : 1;
  }
  translate(lineDelta: number | { lineDelta?: number; characterDelta?: number } = 0, characterDelta = 0): Position {
    if (typeof lineDelta === "object") return this.translate(lineDelta.lineDelta ?? 0, lineDelta.characterDelta ?? 0);
    return new Position(this.line + lineDelta, this.character + characterDelta);
  }
  with(line: number | { line?: number; character?: number } = this.line, character = this.character): Position {
    if (typeof line === "object") return new Position(line.line ?? this.line, line.character ?? this.character);
    return new Position(line, character);
  }
  toJSON(): unknown {
    return { line: this.line, character: this.character };
  }
}

export class Range {
  readonly start: Position;
  readonly end: Position;
  constructor(a: Position | number, b: Position | number, c?: number, d?: number) {
    const start = typeof a === "number" ? new Position(a, b as number) : a;
    const end = typeof a === "number" ? new Position(c!, d!) : (b as Position);
    if (start.isAfter(end)) {
      this.start = end;
      this.end = start;
    } else {
      this.start = start;
      this.end = end;
    }
  }
  get isEmpty(): boolean {
    return this.start.isEqual(this.end);
  }
  get isSingleLine(): boolean {
    return this.start.line === this.end.line;
  }
  contains(x: Position | Range): boolean {
    if (x instanceof Range) return this.contains(x.start) && this.contains(x.end);
    return x.isAfterOrEqual(this.start) && x.isBeforeOrEqual(this.end);
  }
  isEqual(o: Range): boolean {
    return this.start.isEqual(o.start) && this.end.isEqual(o.end);
  }
  intersection(o: Range): Range | undefined {
    const start = this.start.isAfter(o.start) ? this.start : o.start;
    const end = this.end.isBefore(o.end) ? this.end : o.end;
    return start.isAfter(end) ? undefined : new Range(start, end);
  }
  union(o: Range): Range {
    return new Range(this.start.isBefore(o.start) ? this.start : o.start, this.end.isAfter(o.end) ? this.end : o.end);
  }
  with(start: Position | { start?: Position; end?: Position } = this.start, end: Position = this.end): Range {
    if (start instanceof Position) return new Range(start, end);
    return new Range(start.start ?? this.start, start.end ?? this.end);
  }
  toJSON(): unknown {
    return { start: this.start.toJSON(), end: this.end.toJSON() };
  }
}

export class Selection extends Range {
  readonly anchor: Position;
  readonly active: Position;
  constructor(a: Position | number, b: Position | number, c?: number, d?: number) {
    const anchor = typeof a === "number" ? new Position(a, b as number) : a;
    const active = typeof a === "number" ? new Position(c!, d!) : (b as Position);
    super(anchor, active);
    this.anchor = anchor;
    this.active = active;
  }
  get isReversed(): boolean {
    return this.anchor.isAfter(this.active);
  }
}

export class Location {
  uri: Uri;
  range: Range;
  constructor(uri: Uri, rangeOrPosition: Range | Position) {
    this.uri = uri;
    this.range = rangeOrPosition instanceof Position ? new Range(rangeOrPosition, rangeOrPosition) : rangeOrPosition;
  }
}

// ---------------------------------------------------------------------------------------------
// Edits

export class TextEdit {
  range: Range;
  newText: string;
  newEol?: number;
  constructor(range: Range, newText: string) {
    this.range = range;
    this.newText = newText;
  }
  static replace(range: Range, newText: string): TextEdit {
    return new TextEdit(range, newText);
  }
  static insert(position: Position, newText: string): TextEdit {
    return new TextEdit(new Range(position, position), newText);
  }
  static delete(range: Range): TextEdit {
    return new TextEdit(range, "");
  }
  static setEndOfLine(eol: number): TextEdit {
    const e = new TextEdit(new Range(0, 0, 0, 0), "");
    e.newEol = eol;
    return e;
  }
}

export interface FileOperation {
  kind: "create" | "delete" | "rename";
  uri: Uri;
  newUri?: Uri;
  options?: unknown;
}

export class WorkspaceEdit {
  private edits = new Map<string, { uri: Uri; edits: TextEdit[] }>();
  readonly fileOperations: FileOperation[] = [];
  get size(): number {
    return this.edits.size;
  }
  private bucket(uri: Uri): TextEdit[] {
    const key = uri.toString();
    let b = this.edits.get(key);
    if (!b) this.edits.set(key, (b = { uri, edits: [] }));
    return b.edits;
  }
  replace(uri: Uri, range: Range, newText: string): void {
    this.bucket(uri).push(new TextEdit(range, newText));
  }
  insert(uri: Uri, position: Position, newText: string): void {
    this.bucket(uri).push(TextEdit.insert(position, newText));
  }
  delete(uri: Uri, range: Range): void {
    this.bucket(uri).push(TextEdit.delete(range));
  }
  has(uri: Uri): boolean {
    return this.edits.has(uri.toString());
  }
  set(uri: Uri, edits: readonly (TextEdit | [TextEdit, unknown])[] | undefined | null): void {
    const list = this.bucket(uri);
    list.length = 0;
    for (const e of edits ?? []) list.push(Array.isArray(e) ? e[0] : e);
  }
  get(uri: Uri): TextEdit[] {
    return [...(this.edits.get(uri.toString())?.edits ?? [])];
  }
  entries(): [Uri, TextEdit[]][] {
    return [...this.edits.values()].map((b) => [b.uri, [...b.edits]]);
  }
  createFile(uri: Uri, options?: unknown): void {
    this.fileOperations.push({ kind: "create", uri, options });
  }
  deleteFile(uri: Uri, options?: unknown): void {
    this.fileOperations.push({ kind: "delete", uri, options });
  }
  renameFile(oldUri: Uri, newUri: Uri, options?: unknown): void {
    this.fileOperations.push({ kind: "rename", uri: oldUri, newUri, options });
  }
}

export class SnippetString {
  value: string;
  private tabstop = 1;
  constructor(value = "") {
    this.value = value;
  }
  appendText(s: string): this {
    this.value += s.replace(/[$}\\]/g, "\\$&");
    return this;
  }
  appendTabstop(n = this.tabstop++): this {
    this.value += `$${n}`;
    return this;
  }
  appendPlaceholder(value: string | ((s: SnippetString) => unknown), n = this.tabstop++): this {
    if (typeof value === "function") {
      const nested = new SnippetString();
      nested.tabstop = this.tabstop;
      value(nested);
      this.tabstop = nested.tabstop;
      this.value += `\${${n}:${nested.value}}`;
    } else this.value += `\${${n}:${value.replace(/[$}\\]/g, "\\$&")}}`;
    return this;
  }
  appendChoice(values: string[], n = this.tabstop++): this {
    this.value += `\${${n}|${values.map((v) => v.replace(/[,|\\]/g, "\\$&")).join(",")}|}`;
    return this;
  }
  appendVariable(name: string, defaultValue?: string | ((s: SnippetString) => unknown)): this {
    if (defaultValue === undefined) this.value += `\${${name}}`;
    else if (typeof defaultValue === "string") this.value += `\${${name}:${defaultValue}}`;
    else {
      const nested = new SnippetString();
      defaultValue(nested);
      this.value += `\${${name}:${nested.value}}`;
    }
    return this;
  }
}

// ---------------------------------------------------------------------------------------------
// Presentation

export class MarkdownString {
  value: string;
  isTrusted?: boolean | { enabledCommands: string[] };
  supportThemeIcons?: boolean;
  supportHtml?: boolean;
  baseUri?: Uri;
  constructor(value = "", supportThemeIcons = false) {
    this.value = value;
    this.supportThemeIcons = supportThemeIcons;
  }
  appendText(value: string): this {
    this.value += value.replace(/[\\`*_{}[\]()#+\-.!|<>]/g, "\\$&").replace(/\n/g, "\n\n");
    return this;
  }
  appendMarkdown(value: string): this {
    this.value += value;
    return this;
  }
  appendCodeblock(code: string, language = ""): this {
    this.value += `\n\`\`\`${language}\n${code}\n\`\`\`\n`;
    return this;
  }
}

export class ThemeColor {
  readonly id: string;
  constructor(id: string) {
    this.id = id;
  }
}

export class ThemeIcon {
  static readonly File = new ThemeIcon("file");
  static readonly Folder = new ThemeIcon("folder");
  readonly id: string;
  readonly color?: ThemeColor;
  constructor(id: string, color?: ThemeColor) {
    this.id = id;
    this.color = color;
  }
}

export class Diagnostic {
  range: Range;
  message: string;
  severity: number;
  source?: string;
  code?: string | number | { value: string | number; target: Uri };
  relatedInformation?: DiagnosticRelatedInformation[];
  tags?: number[];
  constructor(range: Range, message: string, severity: number = DiagnosticSeverity.Error) {
    this.range = range;
    this.message = message;
    this.severity = severity;
  }
}

export class DiagnosticRelatedInformation {
  location: Location;
  message: string;
  constructor(location: Location, message: string) {
    this.location = location;
    this.message = message;
  }
}

export class TreeItem {
  label?: string | { label: string; highlights?: [number, number][] };
  id?: string;
  iconPath?: string | Uri | ThemeIcon | { light: string | Uri; dark: string | Uri };
  description?: string | boolean;
  resourceUri?: Uri;
  tooltip?: string | MarkdownString;
  command?: { command: string; title: string; arguments?: unknown[] };
  collapsibleState?: number;
  contextValue?: string;
  accessibilityInformation?: unknown;
  checkboxState?: unknown;
  constructor(labelOrUri: string | { label: string } | Uri, collapsibleState: number = TreeItemCollapsibleState.None) {
    if (labelOrUri instanceof Uri) this.resourceUri = labelOrUri;
    else this.label = labelOrUri;
    this.collapsibleState = collapsibleState;
  }
}

export class CompletionItem {
  label: string | { label: string; detail?: string; description?: string };
  kind?: number;
  detail?: string;
  documentation?: string | MarkdownString;
  sortText?: string;
  filterText?: string;
  insertText?: string | SnippetString;
  range?: Range | { inserting: Range; replacing: Range };
  command?: unknown;
  commitCharacters?: string[];
  tags?: number[];
  preselect?: boolean;
  keepWhitespace?: boolean;
  additionalTextEdits?: TextEdit[];
  constructor(label: string | { label: string }, kind?: number) {
    this.label = label;
    this.kind = kind;
  }
}

export class CompletionList {
  isIncomplete: boolean;
  items: CompletionItem[];
  constructor(items: CompletionItem[] = [], isIncomplete = false) {
    this.items = items;
    this.isIncomplete = isIncomplete;
  }
}

export class Hover {
  contents: (MarkdownString | string | { language: string; value: string })[];
  range?: Range;
  constructor(contents: MarkdownString | string | (MarkdownString | string)[], range?: Range) {
    this.contents = Array.isArray(contents) ? contents : [contents];
    this.range = range;
  }
}

export class CodeActionKind {
  static readonly Empty = new CodeActionKind("");
  static readonly QuickFix = new CodeActionKind("quickfix");
  static readonly Refactor = new CodeActionKind("refactor");
  static readonly RefactorExtract = new CodeActionKind("refactor.extract");
  static readonly RefactorInline = new CodeActionKind("refactor.inline");
  static readonly RefactorRewrite = new CodeActionKind("refactor.rewrite");
  static readonly Source = new CodeActionKind("source");
  static readonly SourceOrganizeImports = new CodeActionKind("source.organizeImports");
  static readonly SourceFixAll = new CodeActionKind("source.fixAll");
  readonly value: string;
  constructor(value: string) {
    this.value = value;
  }
  append(part: string): CodeActionKind {
    return new CodeActionKind(this.value ? `${this.value}.${part}` : part);
  }
  contains(other: CodeActionKind): boolean {
    return this.value === "" || other.value === this.value || other.value.startsWith(`${this.value}.`);
  }
  intersects(other: CodeActionKind): boolean {
    return this.contains(other) || other.contains(this);
  }
}

export class CodeAction {
  title: string;
  kind?: CodeActionKind;
  edit?: WorkspaceEdit;
  command?: unknown;
  diagnostics?: Diagnostic[];
  isPreferred?: boolean;
  constructor(title: string, kind?: CodeActionKind) {
    this.title = title;
    this.kind = kind;
  }
}

export class CodeLens {
  range: Range;
  command?: unknown;
  constructor(range: Range, command?: unknown) {
    this.range = range;
    this.command = command;
  }
  get isResolved(): boolean {
    return this.command !== undefined;
  }
}

export class DocumentLink {
  range: Range;
  target?: Uri;
  tooltip?: string;
  constructor(range: Range, target?: Uri) {
    this.range = range;
    this.target = target;
  }
}

export class Color {
  readonly red: number;
  readonly green: number;
  readonly blue: number;
  readonly alpha: number;
  constructor(red: number, green: number, blue: number, alpha: number) {
    this.red = red;
    this.green = green;
    this.blue = blue;
    this.alpha = alpha;
  }
}

export class ColorInformation {
  range: Range;
  color: Color;
  constructor(range: Range, color: Color) {
    this.range = range;
    this.color = color;
  }
}

export class FoldingRange {
  start: number;
  end: number;
  kind?: number;
  constructor(start: number, end: number, kind?: number) {
    this.start = start;
    this.end = end;
    this.kind = kind;
  }
}

export class RelativePattern {
  baseUri: Uri;
  pattern: string;
  constructor(base: Uri | string | { uri: Uri }, pattern: string) {
    this.baseUri = base instanceof Uri ? base : typeof base === "string" ? Uri.file(base) : base.uri;
    this.pattern = pattern;
  }
  get base(): string {
    return this.baseUri.fsPath;
  }
}

export class FileSystemError extends Error {
  readonly code: string;
  constructor(messageOrUri?: string | Uri, code = "Unknown") {
    super(messageOrUri instanceof Uri ? messageOrUri.toString() : (messageOrUri ?? code));
    this.code = code;
    this.name = `${code} (FileSystemError)`;
  }
  static FileNotFound(m?: string | Uri): FileSystemError {
    return new FileSystemError(m, "FileNotFound");
  }
  static FileExists(m?: string | Uri): FileSystemError {
    return new FileSystemError(m, "FileExists");
  }
  static FileNotADirectory(m?: string | Uri): FileSystemError {
    return new FileSystemError(m, "FileNotADirectory");
  }
  static FileIsADirectory(m?: string | Uri): FileSystemError {
    return new FileSystemError(m, "FileIsADirectory");
  }
  static NoPermissions(m?: string | Uri): FileSystemError {
    return new FileSystemError(m, "NoPermissions");
  }
  static Unavailable(m?: string | Uri): FileSystemError {
    return new FileSystemError(m, "Unavailable");
  }
}

export class TabInputText {
  readonly uri: Uri;
  constructor(uri: Uri) {
    this.uri = uri;
  }
}
