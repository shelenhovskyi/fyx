// Checks that every Zed action the keybinding converter emits exists in this Zed checkout.
import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { COMMAND_TO_ZED_ACTION } from "../src/keybindings.ts";

const repo = join(import.meta.dirname, "..", "..", "..");
const inRepo = existsSync(join(repo, "crates", "editor")) && existsSync(join(repo, "assets", "keymaps"));

function* walk(dir: string, ext: string): Generator<string> {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, e.name);
    if (e.isDirectory()) {
      if (e.name !== "target" && e.name !== "node_modules") yield* walk(full, ext);
    } else if (e.name.endsWith(ext)) yield full;
  }
}

test("every mapped Zed action exists", { skip: !inRepo && "not inside the Zed/Fyx checkout" }, () => {
  const wanted = new Set(
    Object.values(COMMAND_TO_ZED_ACTION)
      .filter((a) => a !== null)
      .map((a) => (typeof a === "string" ? a : a[0])),
  );
  const keymapText = [...walk(join(repo, "assets", "keymaps"), ".json")].map((f) => readFileSync(f, "utf8")).join("\n");
  let missing = [...wanted].filter((a) => !keymapText.includes(`"${a}"`));
  if (missing.length > 0) {
    const sources = [...walk(join(repo, "crates"), ".rs")]
      .map((f) => readFileSync(f, "utf8"))
      .filter((s) => s.includes("actions!") || s.includes("namespace ="));
    missing = missing.filter((action) => {
      const [ns, name] = action.split("::");
      const macro = new RegExp(`actions!\\(\\s*${ns}\\s*,[\\s\\S]*?\\b${name}\\b[\\s\\S]*?\\]\\s*\\)`);
      const derive = new RegExp(`namespace\\s*=\\s*${ns}\\b[\\s\\S]{0,400}?(struct|enum)\\s+${name}\\b`);
      return !sources.some((s) => macro.test(s) || derive.test(s));
    });
  }
  assert.deepEqual(missing, []);
});
