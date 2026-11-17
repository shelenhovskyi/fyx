// VS Code colours → `fyx:ui` colour tokens. `fyx:ui` carries semantic slots only, never raw
// colours, so `ThemeColor` ids map to the closest slot and raw CSS colours are dropped (and
// reported in the coverage notes).

import type { ColorToken } from "../schema.ts";

const EXACT: Readonly<Record<string, ColorToken>> = {
  foreground: "text",
  "editor.foreground": "text",
  "statusBar.foreground": "text",
  "statusBarItem.foreground": "text",
  descriptionForeground: "text-muted",
  disabledForeground: "text-muted",
  "editorLineNumber.foreground": "text-muted",
  "editorCodeLens.foreground": "text-muted",
  "editorGhostText.foreground": "text-muted",
  "editorInlayHint.foreground": "text-muted",
  "editorWhitespace.foreground": "text-muted",
  "editorIndentGuide.background": "text-muted",
  "tab.inactiveForeground": "text-muted",
  "textLink.foreground": "text-accent",
  focusBorder: "text-accent",
  "button.background": "text-accent",
  "charts.blue": "text-accent",
  "statusBarItem.prominentBackground": "text-accent",
  "statusBarItem.remoteBackground": "text-accent",
  "editorInfo.foreground": "text-accent",
  "charts.green": "success",
  "testing.iconPassed": "success",
  "gitDecoration.addedResourceForeground": "success",
  "editorGutter.addedBackground": "success",
  "terminal.ansiGreen": "success",
  "panel.border": "border",
  contrastBorder: "border",
  "editorGroup.border": "border",
  "editorIndentGuide.activeBackground": "border",
  "editor.background": "surface",
  "panel.background": "surface",
  "sideBar.background": "surface",
  "statusBar.background": "surface",
  "editorWidget.background": "elevated-surface",
  "editorHoverWidget.background": "elevated-surface",
  "dropdown.background": "elevated-surface",
  "quickInput.background": "elevated-surface",
};

export function tokenForThemeColor(id: string): ColorToken | null {
  if (Object.hasOwn(EXACT, id)) return EXACT[id];
  if (/error/i.test(id)) return "error";
  if (/warning/i.test(id)) return "warning";
  if (/(added|success|passed)/i.test(id)) return "success";
  if (/(info|link|accent|highlight|selection|findMatch|prominent)/i.test(id)) return "text-accent";
  if (/border/i.test(id)) return "border";
  if (/(widget|hover|dropdown|quickInput|menu|notification)/i.test(id)) return "elevated-surface";
  if (/background/i.test(id)) return "surface";
  if (/foreground/i.test(id)) return "text";
  return null;
}

/** A `ThemeColor` maps to a token; a raw colour string yields `null` and is reported by the caller. */
export function tokenForColor(value: unknown): { token: ColorToken | null; dropped?: string } {
  if (value === undefined || value === null) return { token: null };
  if (typeof value === "object" && typeof (value as { id?: unknown }).id === "string") {
    return { token: tokenForThemeColor((value as { id: string }).id) };
  }
  return { token: null, dropped: String(value) };
}
