// Port of `crates/theme_importer/src/vscode/converter.rs`: VS Code color theme → Zed theme JSON
// (`https://zed.dev/schema/themes/v0.2.0.json`). The color tables below mirror
// `convert_theme_colors` and `convert_status_colors` one to one. Differences from the Rust
// importer are deliberate and listed in README.md ("Differences from theme_importer").

import { own } from "./own.ts";
import { SYNTAX_FALLBACKS, ZED_SYNTAX_TOKENS, findBestTokenColorMatch, type VsCodeTokenColor } from "./syntax.ts";

export const ZED_THEME_SCHEMA_URL = "https://zed.dev/schema/themes/v0.2.0.json";

/** Stands for the foreground of the first `tokenColors` rule without a scope. */
const TOKEN_FOREGROUND = "$tokenForeground";

const ANSI = [
  "black",
  "red",
  "green",
  "yellow",
  "blue",
  "magenta",
  "cyan",
  "white",
] as const;

/** Zed theme color key → VS Code color ids, first present wins. */
export const THEME_COLOR_MAP: ReadonlyArray<readonly [string, readonly string[]]> = [
  ["border", ["panel.border"]],
  ["border.variant", ["panel.border"]],
  ["border.focused", ["focusBorder"]],
  ["border.selected", ["panel.border"]],
  ["border.transparent", ["panel.border"]],
  ["border.disabled", ["panel.border"]],
  ["elevated_surface.background", ["dropdown.background"]],
  ["surface.background", ["panel.background"]],
  ["background", ["editor.background"]],
  ["element.background", ["button.background"]],
  ["element.hover", ["list.hoverBackground"]],
  ["element.selected", ["list.activeSelectionBackground"]],
  ["drop_target.background", ["list.dropBackground"]],
  ["ghost_element.hover", ["list.hoverBackground"]],
  ["ghost_element.selected", ["list.activeSelectionBackground"]],
  ["text", ["foreground", TOKEN_FOREGROUND]],
  ["text.muted", ["tab.inactiveForeground"]],
  ["status_bar.background", ["statusBar.background"]],
  ["title_bar.background", ["titleBar.activeBackground"]],
  ["toolbar.background", ["breadcrumb.background", "editor.background"]],
  ["tab_bar.background", ["editorGroupHeader.tabsBackground"]],
  ["tab.inactive_background", ["tab.inactiveBackground"]],
  ["tab.active_background", ["tab.activeBackground", "tab.inactiveBackground"]],
  ["search.match_background", ["editor.findMatchBackground"]],
  ["panel.background", ["panel.background"]],
  ["pane_group.border", ["editorGroup.border"]],
  ["scrollbar.thumb.background", ["scrollbarSlider.background"]],
  ["scrollbar.thumb.hover_background", ["scrollbarSlider.hoverBackground"]],
  ["scrollbar.thumb.active_background", ["scrollbarSlider.activeBackground"]],
  ["scrollbar.thumb.border", ["scrollbarSlider.background"]],
  ["scrollbar.track.background", ["editor.background"]],
  ["scrollbar.track.border", ["editorOverviewRuler.border"]],
  ["minimap.thumb.background", ["minimapSlider.background"]],
  ["minimap.thumb.hover_background", ["minimapSlider.hoverBackground"]],
  ["minimap.thumb.active_background", ["minimapSlider.activeBackground"]],
  ["editor.foreground", ["editor.foreground", TOKEN_FOREGROUND]],
  ["editor.background", ["editor.background"]],
  ["editor.gutter.background", ["editor.background"]],
  ["editor.active_line.background", ["editor.lineHighlightBackground"]],
  ["editor.line_number", ["editorLineNumber.foreground"]],
  ["editor.active_line_number", ["editor.foreground"]],
  ["editor.wrap_guide", ["panel.border"]],
  ["editor.active_wrap_guide", ["panel.border"]],
  ["editor.document_highlight.bracket_background", ["editorBracketMatch.background"]],
  ["terminal.background", ["terminal.background"]],
  ...ANSI.flatMap((c) => {
    const cap = c[0].toUpperCase() + c.slice(1);
    return [
      [`terminal.ansi.${c}`, [`terminal.ansi${cap}`]] as const,
      [`terminal.ansi.bright_${c}`, [`terminal.ansiBright${cap}`]] as const,
    ];
  }),
  ["link_text.hover", ["textLink.activeForeground"]],
  ["vim.yank.background", ["editor.rangeHighlightBackground"]],
];

/** Zed status color key → VS Code color ids (`convert_status_colors`). */
export const STATUS_COLOR_MAP: ReadonlyArray<readonly [string, readonly string[]]> = [
  ["conflict", ["gitDecoration.conflictingResourceForeground"]],
  ["created", ["editorGutter.addedBackground"]],
  ["deleted", ["editorGutter.deletedBackground"]],
  ["error", ["editorError.foreground"]],
  ["error.background", ["editorError.background"]],
  ["error.border", ["editorError.border"]],
  ["hidden", ["tab.inactiveForeground"]],
  ["hint", ["editorInlayHint.foreground"]],
  ["hint.border", ["editorHint.border"]],
  ["ignored", ["gitDecoration.ignoredResourceForeground"]],
  ["info", ["editorInfo.foreground"]],
  ["info.background", ["editorInfo.background"]],
  ["info.border", ["editorInfo.border"]],
  ["modified", ["editorGutter.modifiedBackground"]],
  ["warning", ["editorWarning.foreground"]],
  ["warning.background", ["editorWarning.background"]],
  ["warning.border", ["editorWarning.border"]],
];

/** `vscode_base_status_colors` in the Rust importer. */
const STATUS_DEFAULTS: Readonly<Record<string, string>> = { hint: "#969696ff" };

export type Appearance = "light" | "dark";

export interface VsCodeTheme {
  name?: string;
  type?: string;
  colors?: Record<string, string | null>;
  tokenColors?: VsCodeTokenColor[];
}

export interface HighlightStyle {
  color?: string;
  background_color?: string;
  font_style?: "italic" | "oblique";
  font_weight?: number;
}

export interface ZedTheme {
  name: string;
  appearance: Appearance;
  style: Record<string, unknown>;
}

export interface ZedThemeFamily {
  $schema: string;
  name: string;
  author: string;
  themes: ZedTheme[];
}

export interface ConvertWarning {
  message: string;
}

/** Appearance from a theme's `type` or the contributing `uiTheme` (`vs`, `vs-dark`, `hc-black`, `hc-light`). */
export function appearanceOf(theme: VsCodeTheme, uiTheme?: string): Appearance {
  const kind = (uiTheme ?? theme.type ?? "").toLowerCase();
  if (kind === "vs" || kind === "light" || kind === "hc-light" || kind === "hclight") return "light";
  return "dark";
}

/** Normalizes `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa` to lowercase `#rrggbbaa`; `undefined` if not a hex colour. */
export function normalizeColor(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const m = /^#([0-9a-f]{3,8})$/i.exec(value.trim());
  if (!m) return undefined;
  let hex = m[1].toLowerCase();
  if (hex.length === 3 || hex.length === 4) hex = [...hex].map((ch) => ch + ch).join("");
  if (hex.length === 6) hex += "ff";
  return hex.length === 8 ? `#${hex}` : undefined;
}

export function convertTheme(
  theme: VsCodeTheme,
  options: { name?: string; appearance?: Appearance; warnings?: ConvertWarning[] } = {},
): ZedTheme {
  const warnings = options.warnings ?? [];
  const colors = theme.colors ?? {};
  const tokenColors = (theme.tokenColors ?? []).filter((t) => t && typeof t === "object" && t.settings);
  const tokenForeground = tokenColors.find((t) => t.scope === undefined)?.settings.foreground;

  const lookup = (ids: readonly string[]): string | undefined => {
    for (const id of ids) {
      const raw = id === TOKEN_FOREGROUND ? tokenForeground : own(colors, id);
      if (raw === undefined || raw === null) continue;
      const color = normalizeColor(raw);
      if (color) return color;
      warnings.push({ message: `ignored invalid colour ${JSON.stringify(raw)} for ${id}` });
    }
    return undefined;
  };

  const style: Record<string, unknown> = {
    "background.appearance": "opaque",
    accents: [],
  };
  for (const [zedKey, ids] of THEME_COLOR_MAP) {
    const color = lookup(ids);
    if (color) style[zedKey] = color;
  }
  for (const [zedKey, ids] of STATUS_COLOR_MAP) {
    const color = lookup(ids) ?? own(STATUS_DEFAULTS, zedKey);
    if (color) style[zedKey] = color;
  }
  style.players = [];
  style.syntax = convertSyntax(tokenColors, warnings);

  return {
    name: options.name ?? theme.name ?? "Imported theme",
    appearance: options.appearance ?? appearanceOf(theme),
    style,
  };
}

function convertSyntax(tokenColors: VsCodeTokenColor[], warnings: ConvertWarning[]): Record<string, HighlightStyle> {
  const syntax: Record<string, HighlightStyle> = {};
  for (const [token] of ZED_SYNTAX_TOKENS) {
    const match =
      findBestTokenColorMatch(token, tokenColors) ??
      (own(SYNTAX_FALLBACKS, token) ?? []).map((f) => findBestTokenColorMatch(f, tokenColors)).find(Boolean);
    if (!match) continue;
    const highlight: HighlightStyle = {};
    const color = normalizeColor(match.settings.foreground);
    if (color) highlight.color = color;
    else if (match.settings.foreground !== undefined) {
      warnings.push({ message: `ignored invalid foreground for syntax token ${token}` });
    }
    const background = normalizeColor(match.settings.background);
    if (background) highlight.background_color = background;
    const fontStyle = match.settings.fontStyle;
    if (fontStyle?.includes("italic")) highlight.font_style = "italic";
    else if (fontStyle?.includes("oblique")) highlight.font_style = "oblique";
    if (fontStyle?.includes("bold")) highlight.font_weight = 700;
    if (Object.keys(highlight).length > 0) syntax[token] = highlight;
  }
  return syntax;
}

export function themeFamily(name: string, author: string, themes: ZedTheme[]): ZedThemeFamily {
  return { $schema: ZED_THEME_SCHEMA_URL, name, author, themes };
}
