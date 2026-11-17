import { own } from "./own.ts";

// VS Code snippets → Zed snippet files.
//
// Zed already reads the VS Code snippet JSON shape (`crates/snippet_provider/src/format.rs`):
// `{ name: { prefix, body, description } }`, one file per language whose stem is the language's
// `snippet_scope_id()` (lowercased Zed language name, `snippets.json` = global). What does not
// carry over:
//   * `.code-snippets` files mix languages through a per-snippet `scope`; we split them per file.
//   * VS Code language ids differ from Zed language names (`typescriptreact` → `tsx`, ...).
//   * Zed's body parser (`crates/snippet/src/snippet.rs`) only knows `$n`, `${n}`, `${n:...}` and
//     `${n|a,b|}`. Variables (`$TM_FILENAME`, `${CURRENT_YEAR}`) and transforms make Zed drop the
//     whole snippet with a parse error, so we rewrite them the way VS Code treats unknown variables:
//     as a placeholder holding the variable's default or name.

export interface VsCodeSnippet {
  prefix?: string | string[];
  body: string | string[];
  description?: string | string[];
  scope?: string;
  isFileTemplate?: boolean;
}

export interface ZedSnippet {
  prefix?: string | string[];
  body: string | string[];
  description?: string;
}

/** VS Code language id → Zed snippet scope id. Unlisted ids pass through lowercased. */
export const LANGUAGE_TO_ZED_SCOPE: Readonly<Record<string, string>> = {
  javascript: "javascript",
  javascriptreact: "javascript",
  typescript: "typescript",
  typescriptreact: "tsx",
  python: "python",
  rust: "rust",
  go: "go",
  c: "c",
  cpp: "c++",
  csharp: "c#",
  css: "css",
  json: "json",
  jsonc: "jsonc",
  markdown: "markdown",
  yaml: "yaml",
  shellscript: "shell script",
  plaintext: "plaintext",
  diff: "diff",
  "git-commit": "git commit",
  html: "html",
  vue: "vue.js",
  dockerfile: "dockerfile",
};

/** Zed language names known to ship with Zed (`crates/grammars/src/*\/config.toml`). */
const ZED_BUILTIN_SCOPES = new Set([
  "c",
  "c++",
  "css",
  "diff",
  "env",
  "git commit",
  "go",
  "go mod",
  "go work",
  "javascript",
  "json",
  "jsonc",
  "markdown",
  "python",
  "rust",
  "shell script",
  "tsx",
  "typescript",
  "yaml",
  "plaintext",
]);

export const GLOBAL_SCOPE = "snippets";

export function zedScopeFor(languageId: string, warnings?: string[]): string {
  const id = languageId.trim();
  if (id === "" || id === "*" || id === "all") return GLOBAL_SCOPE;
  const mapped = own(LANGUAGE_TO_ZED_SCOPE, id) ?? id.toLowerCase().replace(/[/\\]/g, "");
  if (!ZED_BUILTIN_SCOPES.has(mapped)) {
    warnings?.push(`language "${id}" → "${mapped}": not built into Zed, needs a language extension`);
  }
  return mapped;
}

/**
 * Converts one VS Code snippet file. `language` is the contributing language (from
 * `contributes.snippets[].language`); without it, each snippet's `scope` decides, and a snippet
 * without scope is global.
 */
export function convertSnippetFile(
  file: Record<string, VsCodeSnippet>,
  language: string | undefined,
  warnings: string[] = [],
): Map<string, Record<string, ZedSnippet>> {
  const out = new Map<string, Record<string, ZedSnippet>>();
  const add = (scope: string, name: string, snippet: ZedSnippet) => {
    let bucket = out.get(scope);
    if (!bucket) out.set(scope, (bucket = {}));
    bucket[name] = snippet;
  };
  for (const [name, snippet] of Object.entries(file)) {
    if (!snippet || typeof snippet !== "object" || snippet.body === undefined) {
      warnings.push(`snippet "${name}": no body, skipped`);
      continue;
    }
    if (snippet.isFileTemplate) warnings.push(`snippet "${name}": isFileTemplate has no Zed equivalent`);
    const bodyLines = Array.isArray(snippet.body) ? snippet.body : [snippet.body];
    const { body, rewrites } = toZedBody(bodyLines.join("\n"));
    for (const r of rewrites) warnings.push(`snippet "${name}": ${r}`);
    const zed: ZedSnippet = {
      ...(snippet.prefix !== undefined ? { prefix: snippet.prefix } : {}),
      body: Array.isArray(snippet.body) ? body.split("\n") : body,
      ...(snippet.description !== undefined
        ? {
            description: Array.isArray(snippet.description)
              ? snippet.description.join("\n")
              : snippet.description,
          }
        : {}),
    };
    const scopes = language !== undefined ? [language] : (snippet.scope ?? "").split(",");
    for (const scope of new Set(scopes.map((s) => zedScopeFor(s, warnings)))) add(scope, name, zed);
  }
  return out;
}

/** Merges per-scope snippet maps; later names win, as Zed does when two files share a scope. */
export function mergeSnippetFiles(
  into: Map<string, Record<string, ZedSnippet>>,
  from: Map<string, Record<string, ZedSnippet>>,
): void {
  for (const [scope, snippets] of from) into.set(scope, { ...(into.get(scope) ?? {}), ...snippets });
}

// ---------------------------------------------------------------------------------------------
// Body rewriting

interface Rewrite {
  body: string;
  rewrites: string[];
}

/** Rewrites a VS Code snippet body into the subset Zed's parser accepts. */
export function toZedBody(source: string): Rewrite {
  const rewrites: string[] = [];
  let nextIndex = maxTabstop(source) + 1;
  const variableIndex = new Map<string, number>();
  const placeholderFor = (name: string) => {
    let ix = variableIndex.get(name);
    if (ix === undefined) variableIndex.set(name, (ix = nextIndex++));
    return ix;
  };

  const parse = (s: string, nested: boolean): [string, string] => {
    let out = "";
    while (s.length > 0) {
      const c = s[0];
      if (c === "\\") {
        if (s.length > 1 && "$\\}".includes(s[1])) {
          out += s.slice(0, 2);
          s = s.slice(2);
        } else {
          out += c;
          s = s.slice(1);
        }
      } else if (c === "}" && nested) {
        return [out, s];
      } else if (c === "$") {
        const [text, rest] = parseDollar(s.slice(1));
        out += text;
        s = rest;
      } else {
        const end = s.search(nested ? /[\\$}]/ : /[\\$]/);
        const chunk = end < 0 ? s : s.slice(0, end || 1);
        out += chunk;
        s = s.slice(chunk.length);
      }
    }
    return [out, s];
  };

  const parseDollar = (s: string): [string, string] => {
    let m = /^\d+/.exec(s);
    if (m) return [`$${m[0]}`, s.slice(m[0].length)];
    m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(s);
    if (m) {
      const ix = placeholderFor(m[0]);
      rewrites.push(`variable $${m[0]} → placeholder \${${ix}:${m[0]}}`);
      return [`\${${ix}:${m[0]}}`, s.slice(m[0].length)];
    }
    if (!s.startsWith("{")) return ["\\$", s]; // a lone `$` is literal in VS Code
    s = s.slice(1);
    m = /^\d+/.exec(s);
    if (m) {
      const ix = m[0];
      s = s.slice(ix.length);
      if (s.startsWith("}")) return [`\${${ix}}`, s.slice(1)];
      if (s.startsWith(":")) {
        const [inner, rest] = parse(s.slice(1), true);
        return [`\${${ix}:${inner}}`, rest.slice(1)];
      }
      if (s.startsWith("|")) {
        const end = s.indexOf("|}", 1);
        if (end > 0) return [`\${${ix}${s.slice(0, end + 1)}}`, s.slice(end + 2)];
      }
      if (s.startsWith("/")) {
        rewrites.push(`transform on tabstop $${ix} dropped (Zed has no snippet transforms)`);
        return [`$${ix}`, skipTransform(s)];
      }
      rewrites.push(`malformed tabstop \${${ix}... kept as text`);
      return [`\\\${${ix}`, s];
    }
    m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(s);
    if (m) {
      const name = m[0];
      s = s.slice(name.length);
      const ix = placeholderFor(name);
      if (s.startsWith("}")) {
        rewrites.push(`variable \${${name}} → placeholder \${${ix}:${name}}`);
        return [`\${${ix}:${name}}`, s.slice(1)];
      }
      if (s.startsWith(":")) {
        const [inner, rest] = parse(s.slice(1), true);
        rewrites.push(`variable \${${name}:…} → placeholder \${${ix}:…}`);
        return [`\${${ix}:${inner}}`, rest.slice(1)];
      }
      if (s.startsWith("/")) {
        rewrites.push(`variable transform \${${name}/…} → placeholder \${${ix}:${name}}`);
        return [`\${${ix}:${name}}`, skipTransform(s)];
      }
    }
    rewrites.push("unparseable `${` kept as literal text");
    return ["\\${", s];
  };

  const [body] = parse(source, false);
  return { body, rewrites };
}

function maxTabstop(source: string): number {
  let max = 0;
  for (const m of source.matchAll(/\$\{?(\d+)/g)) max = Math.max(max, Number(m[1]));
  return max;
}

/** Skips `/regex/format/options}` and returns what follows the closing brace. */
function skipTransform(s: string): string {
  let slashes = 0;
  let depth = 0;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (c === "\\") i++;
    else if (c === "$" && s[i + 1] === "{") {
      depth++;
      i++;
    } else if (c === "}") {
      if (depth > 0) depth--;
      else if (slashes >= 3) return s.slice(i + 1);
    } else if (c === "/" && depth === 0) slashes++;
  }
  return "";
}

/**
 * A port of Zed's `Snippet::parse` acceptance rules, used to check converted bodies. Returns the
 * error Zed would report, or `undefined` if Zed accepts the body.
 */
export function zedSnippetParseError(source: string): string | undefined {
  try {
    zedParse(source, false);
    return undefined;
  } catch (err) {
    return (err as Error).message;
  }
}

function zedParse(s: string, nested: boolean): string {
  while (s.length > 0) {
    const c = s[0];
    if (c === "$") s = zedTabstop(s.slice(1));
    else if (c === "\\") s = s.length > 1 && "$\\}".includes(s[1]) ? s.slice(2) : s.slice(1);
    else if (c === "}") {
      if (nested) return s;
      s = s.slice(1);
    } else {
      const end = s.search(/[}$\\]/);
      s = end < 0 ? "" : s.slice(end);
    }
  }
  return "";
}

function zedTabstop(s: string): string {
  const int = (t: string) => {
    const m = /^\d+/.exec(t);
    if (!m) throw new Error("expected an integer");
    return t.slice(m[0].length);
  };
  if (!s.startsWith("{")) return int(s);
  s = int(s.slice(1));
  if (s.startsWith("|")) {
    const end = s.indexOf("|", 1);
    if (end < 0) throw new Error("Placeholder choice doesn't contain closing pipe-character '|'");
    s = s.slice(end + 1);
  }
  if (s.startsWith(":")) s = zedParse(s.slice(1), true);
  if (!s.startsWith("}")) throw new Error("expected a closing brace");
  return s.slice(1);
}
