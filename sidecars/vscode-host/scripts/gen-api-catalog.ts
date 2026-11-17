// Generates src/api-catalog.json from @types/vscode: every runtime name of the `vscode` module
// (classes, enums, namespace functions, values and events). The shim uses it to tell an API that
// exists in VS Code but not in Fyx (explicit "not implemented in Fyx" error) from a typo
// (undefined, as in VS Code).
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const typesDir = join(require.resolve("@types/vscode/package.json"), "..");
const version = JSON.parse(readFileSync(join(typesDir, "package.json"), "utf8")).version as string;
const lines = readFileSync(join(typesDir, "index.d.ts"), "utf8").split(/\r?\n/);

const classes = new Set<string>();
const enums = new Set<string>();
const namespaces: Record<string, { functions: Set<string>; values: Set<string>; events: Set<string> }> = {};
let current: string | undefined;

for (const line of lines) {
  const top = /^\t(?:export )?(class|enum|namespace) (\w+)/.exec(line);
  if (top) {
    const [, kind, name] = top;
    if (kind === "class") classes.add(name);
    else if (kind === "enum") enums.add(name);
    else namespaces[(current = name)] ??= { functions: new Set(), values: new Set(), events: new Set() };
    continue;
  }
  if (/^\t\}/.test(line)) {
    current = undefined;
    continue;
  }
  if (!current) continue;
  const member = /^\t\t(?:export )?(function|const|let) (\w+)\s*(?:<[^>]*>)?\s*([:(])\s*(.*)$/.exec(line);
  if (!member) continue;
  const [, kind, name, , rest] = member;
  const ns = namespaces[current];
  if (kind === "function") ns.functions.add(name);
  else if (/^Event</.test(rest)) ns.events.add(name);
  else ns.values.add(name);
}

const sorted = (s: Set<string>) => [...s].sort();
const catalog = {
  source: `@types/vscode@${version}`,
  classes: sorted(classes),
  enums: sorted(enums),
  namespaces: Object.fromEntries(
    Object.entries(namespaces)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([name, ns]) => [name, { functions: sorted(ns.functions), values: sorted(ns.values), events: sorted(ns.events) }]),
  ),
};
const out = join(import.meta.dirname, "..", "src", "api-catalog.json");
writeFileSync(out, `${JSON.stringify(catalog, null, 1)}\n`);
const count = Object.values(catalog.namespaces).reduce((n, ns) => n + ns.functions.length + ns.values.length + ns.events.length, 0);
console.log(`${out}: ${catalog.classes.length} classes, ${catalog.enums.length} enums, ${count} namespace members (${catalog.source})`);
