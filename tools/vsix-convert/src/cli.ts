#!/usr/bin/env node
// vsix-convert <extension.vsix | extension-folder> [-o out-dir] [--platform macos|linux|windows]
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { convertExtension, openExtension, outputFiles } from "./vsix.ts";
import type { Platform } from "./keybindings.ts";

const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    out: { type: "string", short: "o" },
    platform: { type: "string" },
    help: { type: "boolean", short: "h" },
  },
});

if (values.help || positionals.length === 0) {
  console.log("usage: vsix-convert <extension.vsix | folder>... [-o out-dir] [--platform macos|linux|windows]");
  process.exit(positionals.length === 0 && !values.help ? 1 : 0);
}

const platform = values.platform as Platform | undefined;
if (platform && !["macos", "linux", "windows"].includes(platform)) {
  console.error(`ERROR unknown platform ${platform}`);
  process.exit(1);
}

let failed = false;
for (const input of positionals) {
  try {
    const result = convertExtension(openExtension(resolve(input)), { platform });
    const files = outputFiles(result);
    const outDir = resolve(values.out ?? "out", result.id);
    for (const [path, content] of files) {
      const full = join(outDir, ...path.split("/"));
      mkdirSync(dirname(full), { recursive: true });
      writeFileSync(full, content);
    }
    const counts = [
      result.themeFamily ? `${result.themeFamily.themes.length} theme(s)` : undefined,
      result.snippets.size > 0 ? `${[...result.snippets.values()].reduce((n, s) => n + Object.keys(s).length, 0)} snippet(s) in ${result.snippets.size} scope(s)` : undefined,
      result.keymap ? `${result.keymap.reduce((n, s) => n + Object.keys(s.bindings).length, 0)} keybinding(s)` : undefined,
    ].filter(Boolean);
    console.log(`${result.id}: ${counts.join(", ") || "nothing to convert"}, ${result.warnings.length} warning(s) → ${outDir}`);
  } catch (err) {
    failed = true;
    console.error(`ERROR ${input}: ${(err as Error).message}`);
  }
}
process.exit(failed ? 1 : 0);
