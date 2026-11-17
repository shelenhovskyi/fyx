// Reads a .vsix (or an unpacked extension folder) and converts its declarative contributions:
// `contributes.themes`, `contributes.snippets`, `contributes.keybindings`.

import { readFileSync, statSync, readdirSync } from "node:fs";
import { join, posix, relative, sep } from "node:path";
import { parseJsonc } from "./jsonc.ts";
import { readZip } from "./zip.ts";
import { appearanceOf, convertTheme, themeFamily, type ConvertWarning, type VsCodeTheme, type ZedThemeFamily } from "./theme.ts";
import { convertSnippetFile, mergeSnippetFiles, type VsCodeSnippet, type ZedSnippet } from "./snippets.ts";
import { convertKeybindings, type Platform, type VsCodeKeybinding, type ZedKeymapSection } from "./keybindings.ts";

/** Files of an extension, keyed by POSIX path relative to the extension root. */
export interface ExtensionFiles {
  read(path: string): Buffer | undefined;
}

export function openExtension(path: string): ExtensionFiles {
  if (statSync(path).isDirectory()) {
    return { read: (p) => tryRead(join(path, ...posix.normalize(p).split("/"))) };
  }
  const entries = readZip(readFileSync(path));
  return { read: (p) => entries.get(posix.join("extension", posix.normalize(p))) };
}

function tryRead(path: string): Buffer | undefined {
  try {
    return readFileSync(path);
  } catch {
    return undefined;
  }
}

export interface ExtensionManifest {
  name: string;
  publisher?: string;
  displayName?: string;
  version?: string;
  description?: string;
  license?: string;
  repository?: string | { url?: string };
  main?: string;
  browser?: string;
  activationEvents?: string[];
  contributes?: {
    themes?: { label?: string; id?: string; uiTheme?: string; path: string }[];
    snippets?: { language?: string | string[]; path: string }[];
    keybindings?: VsCodeKeybinding | VsCodeKeybinding[];
    commands?: { command: string; title?: string } | { command: string; title?: string }[];
    [key: string]: unknown;
  };
  [key: string]: unknown;
}

export function readManifest(files: ExtensionFiles): ExtensionManifest {
  const raw = files.read("package.json");
  if (!raw) throw new Error("package.json not found (is this a VS Code extension?)");
  const manifest = JSON.parse(raw.toString("utf8")) as ExtensionManifest;
  const nlsRaw = files.read("package.nls.json");
  if (!nlsRaw) return manifest;
  const nls = parseJsonc(nlsRaw.toString("utf8")) as Record<string, string | { message: string }>;
  return localize(manifest, nls) as ExtensionManifest;
}

function localize(value: unknown, nls: Record<string, string | { message: string }>): unknown {
  if (typeof value === "string") {
    const m = /^%(.+)%$/.exec(value);
    const entry = m ? nls[m[1]] : undefined;
    if (entry === undefined) return value;
    return typeof entry === "string" ? entry : entry.message;
  }
  if (Array.isArray(value)) return value.map((v) => localize(v, nls));
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, localize(v, nls)]));
  }
  return value;
}

export interface ConversionResult {
  id: string;
  manifest: ExtensionManifest;
  themeFamily?: ZedThemeFamily;
  snippets: Map<string, Record<string, ZedSnippet>>;
  keymap?: ZedKeymapSection[];
  warnings: string[];
}

export function convertExtension(files: ExtensionFiles, options: { platform?: Platform } = {}): ConversionResult {
  const manifest = readManifest(files);
  const id = `${manifest.publisher ?? "unknown"}.${manifest.name}`;
  const contributes = manifest.contributes ?? {};
  const warnings: string[] = [];
  const result: ConversionResult = { id, manifest, snippets: new Map(), warnings };

  const themes = contributes.themes ?? [];
  if (themes.length > 0) {
    const converted = themes.flatMap((t) => {
      const themeWarnings: ConvertWarning[] = [];
      try {
        const theme = loadTheme(files, t.path);
        const out = convertTheme(theme, {
          name: t.label ?? t.id ?? theme.name,
          appearance: appearanceOf(theme, t.uiTheme),
          warnings: themeWarnings,
        });
        for (const w of themeWarnings) warnings.push(`theme ${t.path}: ${w.message}`);
        return [out];
      } catch (err) {
        warnings.push(`ERROR theme ${t.path}: ${(err as Error).message}`);
        return [];
      }
    });
    if (converted.length > 0) {
      result.themeFamily = themeFamily(manifest.displayName ?? manifest.name, manifest.publisher ?? "", converted);
    }
  }

  for (const s of contributes.snippets ?? []) {
    const raw = files.read(stripDot(s.path));
    if (!raw) {
      warnings.push(`ERROR snippets ${s.path}: file not found`);
      continue;
    }
    const file = parseJsonc(raw.toString("utf8")) as Record<string, VsCodeSnippet>;
    const languages = s.language === undefined ? [undefined] : Array.isArray(s.language) ? s.language : [s.language];
    for (const language of languages) {
      const snippetWarnings: string[] = [];
      mergeSnippetFiles(result.snippets, convertSnippetFile(file, language, snippetWarnings));
      for (const w of new Set(snippetWarnings)) warnings.push(`snippets ${s.path}: ${w}`);
    }
  }

  const keybindings = contributes.keybindings;
  if (keybindings) {
    const commands = contributes.commands ?? [];
    const extensionCommands = new Set((Array.isArray(commands) ? commands : [commands]).map((c) => c.command));
    const { keymap, warnings: keyWarnings } = convertKeybindings(Array.isArray(keybindings) ? keybindings : [keybindings], {
      platform: options.platform,
      extensionCommands,
    });
    result.keymap = keymap;
    for (const w of keyWarnings) warnings.push(`keybinding ${w}`);
  }
  return result;
}

/** Loads a theme file, following `include` (base theme) and string `tokenColors` (path to a JSON file). */
export function loadTheme(files: ExtensionFiles, path: string, depth = 0): VsCodeTheme {
  if (depth > 8) throw new Error("theme include chain too deep");
  const p = stripDot(path);
  const raw = files.read(p);
  if (!raw) throw new Error(`file not found: ${path}`);
  if (/\.tmTheme$/i.test(p)) throw new Error("TextMate .tmTheme themes are not supported");
  const theme = parseJsonc(raw.toString("utf8")) as Omit<VsCodeTheme, "tokenColors"> & { include?: string; tokenColors?: unknown };
  const dir = posix.dirname(p);
  let tokenColors: unknown = theme.tokenColors;
  if (typeof tokenColors === "string") {
    const tokenPath = posix.join(dir, tokenColors);
    if (/\.tmTheme$/i.test(tokenPath)) throw new Error("tokenColors in a .tmTheme file are not supported");
    const tokenFile = parseJsonc(files.read(tokenPath)?.toString("utf8") ?? "null") as { tokenColors?: unknown } | unknown[] | null;
    tokenColors = Array.isArray(tokenFile) ? tokenFile : tokenFile?.tokenColors;
  }
  let merged: VsCodeTheme = { ...theme, tokenColors: (tokenColors as VsCodeTheme["tokenColors"]) ?? [] };
  if (theme.include) {
    const base = loadTheme(files, posix.join(dir, theme.include), depth + 1);
    merged = {
      ...base,
      ...merged,
      colors: { ...(base.colors ?? {}), ...(merged.colors ?? {}) },
      tokenColors: [...(base.tokenColors ?? []), ...(merged.tokenColors ?? [])],
    };
  }
  return merged;
}

function stripDot(path: string): string {
  return posix.normalize(path.replace(/\\/g, "/")).replace(/^(\.\/)+/, "");
}

/** Files the converter writes, keyed by POSIX path relative to the output folder. */
export function outputFiles(result: ConversionResult): Map<string, string> {
  const files = new Map<string, string>();
  const json = (v: unknown) => `${JSON.stringify(v, null, 2)}\n`;
  const slug = result.id.toLowerCase().replace(/[^a-z0-9]+/g, "-");
  const themes: string[] = [];
  if (result.themeFamily) {
    const p = `themes/${slug}.json`;
    files.set(p, json(result.themeFamily));
    themes.push(p);
  }
  const snippetPaths: string[] = [];
  for (const [scope, snippets] of [...result.snippets].sort(([a], [b]) => a.localeCompare(b))) {
    const p = `snippets/${scope}.json`;
    files.set(p, json(snippets));
    snippetPaths.push(p);
  }
  if (result.keymap) files.set("keymap.json", json(result.keymap));

  if (themes.length > 0 || snippetPaths.length > 0) {
    const m = result.manifest;
    const repo = typeof m.repository === "string" ? m.repository : m.repository?.url;
    const toml = [
      `id = ${tomlString(slug)}`,
      `name = ${tomlString(m.displayName ?? m.name)}`,
      `version = ${tomlString(m.version ?? "0.0.0")}`,
      "schema_version = 1",
      `authors = [${tomlString(m.publisher ?? "unknown")}]`,
      `description = ${tomlString(`${m.description ?? ""} (converted from VS Code extension ${result.id})`.trim())}`,
      ...(repo ? [`repository = ${tomlString(repo.replace(/^git\+/, ""))}`] : []),
      ...(snippetPaths.length > 0 ? [`snippets = [${snippetPaths.map(tomlString).join(", ")}]`] : []),
      "",
    ];
    files.set("extension.toml", toml.join("\n"));
  }
  if (result.warnings.length > 0) files.set("CONVERSION-WARNINGS.txt", `${result.warnings.join("\n")}\n`);
  return files;
}

function tomlString(s: string): string {
  return JSON.stringify(s);
}

/** Lists a folder recursively as POSIX paths (used by tests). */
export function listFiles(dir: string): string[] {
  const out: string[] = [];
  const walk = (d: string) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const full = join(d, e.name);
      if (e.isDirectory()) walk(full);
      else out.push(relative(dir, full).split(sep).join("/"));
    }
  };
  walk(dir);
  return out.sort();
}
