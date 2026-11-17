// JSON shapes for the non-UI `vscode/*` methods. Positions and ranges follow LSP
// (`{ line, character }`, zero-based, UTF-16 code units), so the host can reuse its LSP plumbing.

import { CompletionItem, CompletionList, Diagnostic, DiagnosticSeverity, Hover, MarkdownString, Position, Range, Selection, SnippetString, TextEdit } from "./types.ts";

export interface WirePosition {
  line: number;
  character: number;
}
export interface WireRange {
  start: WirePosition;
  end: WirePosition;
}

export function encodeRange(r: Range): WireRange {
  return { start: { line: r.start.line, character: r.start.character }, end: { line: r.end.line, character: r.end.character } };
}

export function decodePosition(p: WirePosition): Position {
  return new Position(p.line, p.character);
}

export function decodeRange(r: WireRange): Range {
  return new Range(decodePosition(r.start), decodePosition(r.end));
}

export function decodeSelection(s: { anchor: WirePosition; active: WirePosition }): Selection {
  return new Selection(decodePosition(s.anchor), decodePosition(s.active));
}

export function encodeTextEdit(e: TextEdit): { range: WireRange; newText: string } {
  return { range: encodeRange(e.range), newText: e.newText };
}

const SEVERITY = ["error", "warning", "information", "hint"] as const;

export function decodeDiagnostic(d: { range: WireRange; message: string; severity?: string; source?: string; code?: string | number }): Diagnostic {
  const severity = SEVERITY.indexOf((d.severity ?? "error") as (typeof SEVERITY)[number]);
  const diag = new Diagnostic(decodeRange(d.range), d.message, severity < 0 ? DiagnosticSeverity.Error : severity);
  if (d.source !== undefined) diag.source = d.source;
  if (d.code !== undefined) diag.code = d.code;
  return diag;
}

export function encodeDiagnostic(d: Diagnostic): Record<string, unknown> {
  const code = typeof d.code === "object" && d.code !== null ? { value: d.code.value, target: d.code.target.toString() } : d.code;
  return {
    range: encodeRange(d.range),
    message: d.message,
    severity: SEVERITY[d.severity ?? DiagnosticSeverity.Error] ?? "error",
    ...(d.source !== undefined ? { source: d.source } : {}),
    ...(code !== undefined ? { code } : {}),
    ...(d.tags?.length ? { tags: d.tags } : {}),
    ...(d.relatedInformation?.length
      ? { relatedInformation: d.relatedInformation.map((r) => ({ uri: r.location.uri.toString(), range: encodeRange(r.location.range), message: r.message })) }
      : {}),
  };
}

function markup(v: unknown): string {
  if (v instanceof MarkdownString) return v.value;
  if (typeof v === "string") return v;
  if (v && typeof v === "object" && "language" in v && "value" in v) {
    const { language, value } = v as { language: string; value: string };
    return `\`\`\`${language}\n${value}\n\`\`\``;
  }
  return "";
}

export function encodeHover(h: Hover | null | undefined): Record<string, unknown> | null {
  if (!h) return null;
  return { contents: h.contents.map(markup).filter(Boolean), ...(h.range ? { range: encodeRange(h.range) } : {}) };
}

export function encodeCompletions(result: CompletionItem[] | CompletionList | null | undefined): { isIncomplete: boolean; items: Record<string, unknown>[] } {
  if (!result) return { isIncomplete: false, items: [] };
  const list = Array.isArray(result) ? new CompletionList(result) : result;
  return {
    isIncomplete: list.isIncomplete,
    items: list.items.map((item) => {
      const label = typeof item.label === "string" ? item.label : item.label.label;
      const insert = item.insertText instanceof SnippetString ? { insertText: item.insertText.value, insertTextFormat: "snippet" } : item.insertText !== undefined ? { insertText: item.insertText } : {};
      const range = item.range instanceof Range ? encodeRange(item.range) : item.range ? { inserting: encodeRange(item.range.inserting), replacing: encodeRange(item.range.replacing) } : undefined;
      return {
        label,
        ...(item.kind !== undefined ? { kind: item.kind } : {}),
        ...(item.detail ? { detail: item.detail } : {}),
        ...(item.documentation ? { documentation: markup(item.documentation) } : {}),
        ...(item.sortText ? { sortText: item.sortText } : {}),
        ...(item.filterText ? { filterText: item.filterText } : {}),
        ...insert,
        ...(range ? { range } : {}),
        ...(item.additionalTextEdits?.length ? { additionalTextEdits: item.additionalTextEdits.map(encodeTextEdit) } : {}),
      };
    }),
  };
}
