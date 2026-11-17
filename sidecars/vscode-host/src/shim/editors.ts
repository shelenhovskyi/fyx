// Text editors and decorations. The host tells the sidecar which document is active and where the
// selections are (`vscode/didChangeActiveEditor`, `vscode/didChangeSelection`); decorations go
// back as `vscode/createDecorationType` + `vscode/setDecorations` (layer 4 of the architecture,
// owned by W6; not part of `fyx:ui@0.1.0`). Colours are reduced to theme tokens.

import type { HostContext } from "../context.ts";
import { tokenForColor } from "../ui/theme.ts";
import type { TextDocument } from "./documents.ts";
import { encodeRange, encodeTextEdit } from "./wire.ts";
import { EndOfLine, EventEmitter, Position, Range, Selection, SnippetString, TextEdit, TextEditorCursorStyle, TextEditorLineNumbersStyle, Uri } from "./types.ts";

let decorationSerial = 0;

export class TextEditorDecorationType {
  readonly key: string;
  private ctx: HostContext;
  constructor(ctx: HostContext, options: Record<string, unknown>) {
    this.ctx = ctx;
    this.key = `${ctx.ext.id}/decoration/${++decorationSerial}`;
    const style = encodeDecorationOptions(ctx, options);
    ctx.rpc.notify("vscode/createDecorationType", { key: this.key, style });
  }
  dispose(): void {
    this.ctx.rpc.notify("vscode/disposeDecorationType", { key: this.key });
  }
}

const COLOR_KEYS = ["color", "backgroundColor", "borderColor", "outlineColor", "overviewRulerColor"] as const;

/** Keeps what `fyx:ui` can express: tokens, font weight/style, text decoration, before/after text. */
export function encodeDecorationOptions(ctx: HostContext, options: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  const dropped: string[] = [];
  const encodeBlock = (block: Record<string, unknown> | undefined, prefix: string) => {
    if (!block) return undefined;
    const b: Record<string, unknown> = {};
    for (const key of COLOR_KEYS) {
      if (block[key] === undefined) continue;
      const { token, dropped: raw } = tokenForColor(block[key]);
      if (token) b[key] = token;
      if (raw) dropped.push(`${prefix}${key}=${raw}`);
    }
    for (const key of ["fontWeight", "fontStyle", "textDecoration", "contentText", "border", "borderRadius", "borderStyle", "borderWidth", "opacity", "cursor", "letterSpacing", "margin", "width", "height"]) {
      if (typeof block[key] === "string" || typeof block[key] === "number") b[key] = block[key];
    }
    if (block.gutterIconPath !== undefined) dropped.push(`${prefix}gutterIconPath`);
    for (const nested of ["before", "after"]) {
      const inner = encodeBlock(block[nested] as Record<string, unknown> | undefined, `${prefix}${nested}.`);
      if (inner) b[nested] = inner;
    }
    return b;
  };
  Object.assign(out, encodeBlock(options, ""));
  if (typeof options.isWholeLine === "boolean") out.isWholeLine = options.isWholeLine;
  if (typeof options.rangeBehavior === "number") out.rangeBehavior = options.rangeBehavior;
  if (typeof options.overviewRulerLane === "number") out.overviewRulerLane = options.overviewRulerLane;
  for (const theme of ["light", "dark"]) {
    const t = encodeBlock(options[theme] as Record<string, unknown> | undefined, `${theme}.`);
    if (t && Object.keys(t).length > 0) out[theme] = t;
  }
  if (dropped.length > 0) {
    ctx.coverage.note("vscode.window.createTextEditorDecorationType", `raw colours/icons dropped (fyx:ui carries theme tokens only): ${dropped.slice(0, 4).join(", ")}${dropped.length > 4 ? ", …" : ""}`);
  }
  return out;
}

export interface EditorOptions {
  tabSize: number;
  insertSpaces: boolean;
  cursorStyle: number;
  lineNumbers: number;
  indentSize?: number;
}

export class TextEditor {
  readonly document: TextDocument;
  private ctx: HostContext;
  private editors: EditorStore;
  private selectionState: Selection[] = [new Selection(0, 0, 0, 0)];
  visibleRanges: Range[] = [];
  viewColumn = 1;
  private optionsState: EditorOptions;

  constructor(ctx: HostContext, editors: EditorStore, document: TextDocument) {
    this.ctx = ctx;
    this.editors = editors;
    this.document = document;
    const cfg = ctx.config.get("editor");
    this.optionsState = {
      tabSize: cfg.get<number>("tabSize", 4)!,
      insertSpaces: cfg.get<boolean>("insertSpaces", true)!,
      cursorStyle: TextEditorCursorStyle.Line,
      lineNumbers: TextEditorLineNumbersStyle.On,
    };
    this.visibleRanges = [new Range(0, 0, Math.max(0, document.lineCount - 1), 0)];
  }

  get options(): EditorOptions {
    return this.optionsState;
  }
  /** Assigning options (EditorConfig does) changes the host editor's indentation settings. */
  set options(value: Partial<EditorOptions>) {
    this.optionsState = { ...this.optionsState, ...value };
    this.ctx.rpc.notify("vscode/setEditorOptions", { uri: this.document.uri.toString(), options: value });
    this.editors.onDidChangeOptions.fire({ textEditor: this, options: this.optionsState });
  }

  get selections(): readonly Selection[] {
    return this.selectionState;
  }
  /** Assigned by the extension: moves the host's cursors. */
  set selections(value: readonly Selection[]) {
    this.selectionState = value.length ? [...value] : [new Selection(0, 0, 0, 0)];
    this.ctx.rpc.notify("vscode/setSelections", { uri: this.document.uri.toString(), selections: this.selectionState.map(encodeSelection) });
  }

  get selection(): Selection {
    return this.selectionState[0];
  }
  set selection(s: Selection) {
    this.selections = [s];
  }

  /** @internal Selections reported by the host (no echo back). */
  setSelectionsFromHost(selections: readonly Selection[]): void {
    this.selectionState = selections.length ? [...selections] : [new Selection(0, 0, 0, 0)];
  }

  async edit(callback: (builder: EditBuilder) => void, _options?: { undoStopBefore: boolean; undoStopAfter: boolean }): Promise<boolean> {
    const builder = new EditBuilder(this.document);
    callback(builder);
    if (builder.eol !== undefined) {
      this.ctx.coverage.note("vscode.TextEditorEdit.setEndOfLine", "converts line endings locally; the host receives a full-text edit");
    }
    return this.editors.applyEdits(this.document, builder.edits);
  }

  async insertSnippet(snippet: SnippetString, location?: Position | Range | readonly (Position | Range)[]): Promise<boolean> {
    const targets = location === undefined ? this.selections : Array.isArray(location) ? location : [location];
    return this.ctx.rpc.request<boolean>("vscode/insertSnippet", {
      uri: this.document.uri.toString(),
      snippet: snippet.value,
      ranges: targets.map((t) => encodeRange(t instanceof Position ? new Range(t, t) : (t as Range))),
    });
  }

  setDecorations(type: TextEditorDecorationType, rangesOrOptions: readonly (Range | { range: Range; hoverMessage?: unknown; renderOptions?: Record<string, unknown> })[]): void {
    this.ctx.rpc.notify("vscode/setDecorations", {
      uri: this.document.uri.toString(),
      key: type.key,
      ranges: rangesOrOptions.map((r) =>
        r instanceof Range
          ? { range: encodeRange(r) }
          : {
              range: encodeRange(r.range),
              ...(hoverText(r.hoverMessage) !== undefined ? { hoverMessage: hoverText(r.hoverMessage) } : {}),
              ...(r.renderOptions ? { renderOptions: encodeDecorationOptions(this.ctx, r.renderOptions) } : {}),
            },
      ),
    });
  }

  revealRange(range: Range, revealType?: number): void {
    this.ctx.rpc.notify("vscode/revealRange", { uri: this.document.uri.toString(), range: encodeRange(range), revealType: revealType ?? 0 });
  }

  show(): void {
    void this.editors.show(this.document);
  }
  hide(): void {}
}

/** `hoverMessage` is a string, a MarkdownString, an array of those, or null/undefined. */
function hoverText(message: unknown): string | undefined {
  if (message === null || message === undefined) return undefined;
  if (Array.isArray(message)) {
    const parts = message.map(hoverText).filter((m): m is string => m !== undefined);
    return parts.length ? parts.join("\n\n") : undefined;
  }
  if (typeof message === "object" && typeof (message as { value?: unknown }).value === "string") return (message as { value: string }).value;
  return typeof message === "string" ? message : undefined;
}

export class EditBuilder {
  readonly edits: TextEdit[] = [];
  eol?: number;
  private document: TextDocument;
  constructor(document: TextDocument) {
    this.document = document;
  }
  replace(location: Position | Range | Selection, value: string): void {
    this.edits.push(new TextEdit(location instanceof Position ? new Range(location, location) : location, value));
  }
  insert(location: Position, value: string): void {
    this.edits.push(TextEdit.insert(location, value));
  }
  delete(location: Range | Selection): void {
    this.edits.push(TextEdit.delete(location));
  }
  setEndOfLine(eol: number): void {
    this.eol = eol;
    const text = this.document.getText();
    const converted = eol === EndOfLine.CRLF ? text.replace(/\r?\n/g, "\r\n") : text.replace(/\r\n/g, "\n");
    if (converted !== text) this.edits.push(new TextEdit(new Range(new Position(0, 0), this.document.positionAt(text.length)), converted));
  }
}

export function encodeSelection(s: Selection): unknown {
  return { anchor: { line: s.anchor.line, character: s.anchor.character }, active: { line: s.active.line, character: s.active.character } };
}

export class EditorStore {
  private ctx: HostContext;
  private editors = new Map<string, TextEditor>();
  active: TextEditor | undefined;
  readonly onDidChangeActive = new EventEmitter<TextEditor | undefined>();
  readonly onDidChangeVisible = new EventEmitter<readonly TextEditor[]>();
  readonly onDidChangeSelection = new EventEmitter<{ textEditor: TextEditor; selections: readonly Selection[]; kind: number | undefined }>();
  readonly onDidChangeOptions = new EventEmitter<{ textEditor: TextEditor; options: EditorOptions }>();
  readonly onDidChangeVisibleRanges = new EventEmitter<{ textEditor: TextEditor; visibleRanges: readonly Range[] }>();
  readonly onDidChangeViewColumn = new EventEmitter<unknown>();

  constructor(ctx: HostContext) {
    this.ctx = ctx;
    ctx.documents.onDidClose.event((doc) => {
      const key = doc.uri.toString();
      const editor = this.editors.get(key);
      if (!editor) return;
      this.editors.delete(key);
      if (this.active === editor) this.setActive(undefined);
      this.onDidChangeVisible.fire(this.visible);
    });
  }

  get visible(): TextEditor[] {
    return [...this.editors.values()];
  }

  editorFor(doc: TextDocument): TextEditor {
    const key = doc.uri.toString();
    let editor = this.editors.get(key);
    if (!editor || editor.document !== doc) {
      editor = new TextEditor(this.ctx, this, doc);
      this.editors.set(key, editor);
      this.onDidChangeVisible.fire(this.visible);
    }
    return editor;
  }

  setActive(editor: TextEditor | undefined): void {
    if (this.active === editor) return;
    this.active = editor;
    this.onDidChangeActive.fire(editor);
  }

  setSelections(editor: TextEditor, selections: Selection[], kind?: number): void {
    editor.setSelectionsFromHost(selections);
    this.onDidChangeSelection.fire({ textEditor: editor, selections: editor.selections, kind });
  }

  /** `window.showTextDocument`: the host opens it, the sidecar treats it as active right away. */
  async show(doc: TextDocument, options?: { selection?: Range; preserveFocus?: boolean; preview?: boolean; viewColumn?: number }): Promise<TextEditor> {
    const editor = this.editorFor(doc);
    if (options?.selection) editor.setSelectionsFromHost([new Selection(options.selection.start, options.selection.end)]);
    await this.ctx.rpc.request("vscode/showTextDocument", {
      uri: doc.uri.toString(),
      ...(options?.selection ? { selection: encodeRange(options.selection) } : {}),
      preserveFocus: options?.preserveFocus ?? false,
    });
    if (!options?.preserveFocus) this.setActive(editor);
    return editor;
  }

  /**
   * Asks the host to apply edits to its buffer, and changes the local document only once the host
   * confirmed them: a refused edit never shows up in the extension's view of the text.
   */
  async applyEdits(doc: TextDocument, edits: readonly TextEdit[]): Promise<boolean> {
    if (edits.length === 0) return true;
    const version = doc.version;
    const ok = await this.ctx.rpc.request<boolean>("vscode/applyEdit", { edits: [{ uri: doc.uri.toString(), version, edits: edits.map(encodeTextEdit) }] });
    if (ok !== true) return false;
    this.ctx.documents.applyConfirmed(doc, version, edits);
    return true;
  }
}

export function uriKey(uri: Uri): string {
  return uri.toString();
}
