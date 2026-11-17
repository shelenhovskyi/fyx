// Runs every extension in extensions.json through the sidecar (one child process each, under
// Node's permission model) and the declarative converter, then writes per-extension JSON, the
// recorded RPC traffic and a markdown summary to .cache/coverage/. docs/vscode-coverage.md is
// written from that summary.
//
//   npm run fetch && npm run coverage
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { unpackVsix } from "../src/host.ts";
import { MockHost } from "../src/mock-host.ts";
import type { Node } from "../src/schema.ts";
import { Uri } from "../src/shim/types.ts";

const root = resolve(import.meta.dirname, "..");
const cache = join(root, ".cache");
const outDir = join(cache, "coverage");
const converterCli = resolve(root, "..", "..", "tools", "vsix-convert", "src", "cli.ts");
const list = JSON.parse(readFileSync(join(root, "extensions.json"), "utf8")).extensions as { id: string; version: string; kind: string }[];
const only = process.argv.slice(2);

/** Commands that would reach the network or the user's machine beyond the sandbox. */
const SKIP_COMMANDS: Record<string, RegExp> = {
  "humao.rest-client": /request|rerun|history|cookie|switch|environment|generate|copy/i,
};
const COMMAND_TIMEOUT_MS = 3000;

const SAMPLES: Record<string, { file: string; languageId: string; text: string }> = {
  plaintext: { file: "notes.txt", languageId: "plaintext", text: "hello world  \nTODO: fix this\n\tindented line \nFIXME later\nzebra\napple\n" },
  javascript: { file: "app.js", languageId: "javascript", text: "// TODO: tidy\nfunction helloWorld() {\n    const fooBar = './src/';\n  return fooBar;   \n}\n// ANCHOR: end\n" },
  html: { file: "page.html", languageId: "html", text: "<div>\n  <span>hi</span>\n</div>\n" },
  csv: { file: "data.csv", languageId: "csv", text: "name,age,city\nada,36,london\nalan,41,wilmslow\n" },
  http: { file: "api.http", languageId: "http", text: "GET https://example.invalid/ HTTP/1.1\n" },
  svg: { file: "icon.svg", languageId: "svg", text: '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>\n' },
  markdown: { file: "readme.md", languageId: "markdown", text: "# Title\n\nTODO: write\n" },
};

interface Result {
  id: string;
  version: string;
  kind: string;
  converter?: string;
  sidecar?: Record<string, unknown>;
  probe?: Record<string, unknown>;
}

mkdirSync(outDir, { recursive: true });
const results: Result[] = [];

for (const ext of list) {
  if (only.length && !only.includes(ext.id)) continue;
  const vsix = join(root, "vsix", `${ext.id}-${ext.version}.vsix`);
  if (!existsSync(vsix)) {
    console.error(`ERROR ${ext.id}: ${vsix} missing, run npm run fetch`);
    continue;
  }
  const result: Result = { id: ext.id, version: ext.version, kind: ext.kind };
  const conv = spawnSync(process.execPath, [converterCli, vsix, "-o", join(cache, "converted")], { encoding: "utf8" });
  result.converter = (conv.stdout || conv.stderr).trim().replace(/ → .*$/, "");
  const extRoot = unpackVsix(vsix, join(cache, "unpacked"));
  const manifest = JSON.parse(readFileSync(join(extRoot, "package.json"), "utf8"));
  if (manifest.main) {
    result.sidecar = await runSidecar(ext.id, extRoot, manifest, false);
    // Second pass with missing APIs stubbed, to list everything the extension would need.
    const probe = await runSidecar(ext.id, extRoot, manifest, true);
    result.probe = { activated: probe.activated, activationError: probe.activationError, notImplemented: probe.notImplemented };
  }
  results.push(result);
  writeFileSync(join(outDir, `${ext.id}.json`), JSON.stringify(result, null, 2));
  const s = result.sidecar as { activated?: boolean; notImplemented?: Record<string, number> } | undefined;
  console.log(`${ext.id}: ${result.converter}${s ? ` | activated=${s.activated} notImplemented=${Object.keys(s.notImplemented ?? {}).length}` : ""}`);
}
writeFileSync(join(outDir, "summary.json"), JSON.stringify(results, null, 2));
const markdown = renderMarkdown(results);
writeFileSync(join(outDir, "report.md"), markdown);
console.log(`wrote ${join(outDir, "report.md")}`);
// docs/vscode-coverage.md keeps its prose; only the block between the markers is regenerated.
const docs = resolve(root, "..", "..", "docs", "vscode-coverage.md");
if (!only.length && existsSync(docs)) {
  const text = readFileSync(docs, "utf8");
  const begin = "<!-- BEGIN GENERATED: npm run coverage -->";
  const end = "<!-- END GENERATED -->";
  const a = text.indexOf(begin);
  const b = text.indexOf(end);
  if (a >= 0 && b > a) {
    writeFileSync(docs, `${text.slice(0, a + begin.length)}\n\n${markdown}\n${text.slice(b)}`);
    console.log(`updated ${docs}`);
  }
}

async function runSidecar(id: string, extRoot: string, manifest: Record<string, any>, probe: boolean): Promise<Record<string, any>> {
  const work = join(cache, "work", probe ? `${id}.probe` : id);
  rmSync(work, { recursive: true, force: true });
  const ws = join(work, "workspace");
  const storage = join(work, "storage");
  mkdirSync(ws, { recursive: true });
  mkdirSync(storage, { recursive: true });
  for (const s of Object.values(SAMPLES)) writeFileSync(join(ws, s.file), s.text);
  writeFileSync(join(ws, ".editorconfig"), "root = true\n\n[*]\nindent_style = space\nindent_size = 2\ntrim_trailing_whitespace = true\n");

  // Each extension gets a private home directory inside its sandbox, so probes and caches under
  // ~ (REST Client keeps history in ~/.rest-client) land in the grant instead of failing.
  const home = join(work, "home");
  mkdirSync(home, { recursive: true });
  const host = MockHost.spawn({
    permission: { read: [extRoot, work], write: [work] },
    cwd: work,
    probe,
    env: { HOME: home, USERPROFILE: home, XDG_CACHE_HOME: join(home, ".cache"), XDG_CONFIG_HOME: join(home, ".config") },
  });
  const started = Date.now();
  const commandResults: Record<string, string> = {};
  const surfaceSummaries: Record<string, string> = {};
  const featureResults: Record<string, string> = {};
  try {
    const init = await withTimeout(
      host.initialize({ extensionPath: extRoot, workspaceFolders: [Uri.file(ws).toString()], storageDir: storage, settings: {} }),
      10_000,
      "initialize",
    );

    // Open every sample document; the active editor is a code file unless the extension names
    // languages we have samples for (some extensions skip plain text by default).
    const declared: string[] = [];
    for (const e of init.extension.activationEvents) if (e.startsWith("onLanguage:")) declared.push(e.slice("onLanguage:".length));
    for (const l of manifest.contributes?.languages ?? []) declared.push(l.id);
    const opened: { uri: string; languageId: string; text: string }[] = [];
    for (const [lang, sample] of Object.entries(SAMPLES)) {
      const uri = Uri.file(join(ws, sample.file)).toString();
      await host.openDocument(uri, sample.text, lang);
      opened.push({ uri, languageId: lang, text: sample.text });
    }
    const primary = opened.find((o) => declared.includes(o.languageId)) ?? opened.find((o) => o.languageId === "javascript")!;
    // A diagnostic from a "language server" on the host, for extensions that render diagnostics.
    await host.rpc.request("vscode/didChangeDiagnostics", {
      uri: primary.uri,
      diagnostics: [{ range: { start: { line: 1, character: 9 }, end: { line: 1, character: 19 } }, message: "'helloWorld' is declared but never used.", severity: "warning", source: "ts" }],
    });
    await host.setActiveEditor(primary.uri, [{ anchor: { line: 0, character: 0 }, active: { line: 3, character: 0 } }]);
    await host.settle(50);

    // Mount every surface; click status items; pull tree rows.
    for (const [sid] of host.surfaces) {
      const nodes = await withTimeout(host.mount(sid), 5000, `mount ${sid}`).catch((e: Error) => {
        surfaceSummaries[sid] = `mount failed: ${e.message}`;
        return undefined;
      });
      if (!nodes) continue;
      surfaceSummaries[sid] = describeTree(nodes);
      const list = nodes.find((n) => "list" in n.props);
      if (list) {
        const rows = await host.listRows(sid, list.id, 0, 10).catch(() => []);
        surfaceSummaries[sid] += `; rows: ${rows.map((r) => textOf(r.nodes).join(" ")).join(" | ") || "none"}`;
      }
    }

    // Run every contributed command.
    const skip = SKIP_COMMANDS[id];
    const commands = [...new Set([...(manifest.contributes?.commands ?? []).map((c: { command: string }) => c.command)])] as string[];
    for (const command of commands) {
      if (skip?.test(command)) {
        commandResults[command] = "skipped (network)";
        continue;
      }
      // Each command starts from the same buffers and editor state.
      for (const o of opened) if (host.buffers.get(o.uri)?.text !== o.text) await host.changeDocument(o.uri, o.text).catch(() => undefined);
      await host.setActiveEditor(primary.uri, [{ anchor: { line: 0, character: 0 }, active: { line: 3, character: 0 } }]).catch(() => undefined);
      try {
        await withTimeout(host.executeCommand(command), COMMAND_TIMEOUT_MS, command);
        commandResults[command] = "ok";
      } catch (err) {
        commandResults[command] = `error: ${scrubPaths((err as Error).message.split("\n")[0]).slice(0, 160)}`;
      }
      await host.settle(10);
    }

    // Status bar clicks after commands ran (items may have appeared).
    for (const side of ["status-left", "status-right"]) {
      const sid = `${init.extension.id}/${side}`;
      const tree = host.trees.get(sid);
      for (const b of tree?.filter((n) => "button" in n.props) ?? []) {
        await withTimeout(host.event(sid, { click: b.id }), COMMAND_TIMEOUT_MS, "status click").catch(() => undefined);
      }
      if (tree) surfaceSummaries[sid] = describeTree(host.trees.get(sid) ?? tree);
    }
    for (const sid of [`${init.extension.id}/output`]) if (host.trees.has(sid)) surfaceSummaries[sid] = describeTree(host.trees.get(sid)!);

    // Language features the extension registered.
    const kinds = new Set([...host.providers.values()].map((p) => (p as { kind: string }).kind));
    for (const doc of opened) {
      if (kinds.has("documentFormatting") || kinds.has("documentRangeFormatting")) {
        featureResults[`format ${doc.languageId}`] = await withTimeout(host.format(doc.uri), COMMAND_TIMEOUT_MS, "format").then(
          (edits) => `${edits.length} edit(s)`,
          (e: Error) => `error: ${e.message.slice(0, 120)}`,
        );
      }
      if (kinds.has("completion")) {
        const line = doc.text.split("\n").findIndex((l) => l.includes("'./"));
        const position = line >= 0 ? { line, character: doc.text.split("\n")[line].indexOf("'./") + 3 } : { line: 0, character: 0 };
        featureResults[`completion ${doc.languageId}`] = await withTimeout(host.rpc.request<{ items: unknown[] }>("vscode/provideCompletion", { uri: doc.uri, position }), COMMAND_TIMEOUT_MS, "completion").then(
          (r) => `${r.items.length} item(s)`,
          (e: Error) => `error: ${e.message.slice(0, 120)}`,
        );
      }
      if (kinds.has("hover")) {
        featureResults[`hover ${doc.languageId}`] = await withTimeout(host.rpc.request("vscode/provideHover", { uri: doc.uri, position: { line: 0, character: 1 } }), COMMAND_TIMEOUT_MS, "hover").then(
          (h) => (h ? "hover" : "no hover"),
          (e: Error) => `error: ${e.message.slice(0, 120)}`,
        );
      }
    }
    await host.settle(30);

    const coverage = await withTimeout(host.coverage(), 5000, "coverage");
    if (!probe) host.writeTraffic(join(outDir, `${id}.traffic.jsonl`));
    const edited = [...new Set(host.traffic.filter((t) => "method" in t.message && t.message.method === "vscode/applyEdit").flatMap((t) => ((t.message as { params: { edits: { uri: string }[] } }).params.edits ?? []).map((e) => opened.find((o) => o.uri === e.uri)?.languageId ?? e.uri)))];
    return {
      ...coverage,
      durationMs: Date.now() - started,
      surfaces: surfaceSummaries,
      commandResults,
      features: featureResults,
      effects: {
        diagnostics: [...host.diagnostics.values()].reduce((n, d) => n + d.length, 0),
        decorationTypes: host.decorationTypes.size,
        decoratedRanges: peakDecorations(host),
        providers: [...kinds],
        messages: host.messages.map((m) => scrubPaths(`${m.severity}: ${m.message}`).slice(0, 100)),
        editedDocuments: edited,
        hostCommands: [...new Set(host.hostCommands.map((c) => c.command))],
        contextKeys: [...host.contextKeys.keys()],
        editorOptions: Object.fromEntries(host.editorOptions),
      },
      schemaErrors: host.schemaErrors,
      traffic: host.trafficSummary(),
      stderrTail: host.sidecarStderr.split("\n").filter((l) => /error|denied|not implemented/i.test(l)).slice(-8),
    };
  } catch (err) {
    return { activated: false, runnerError: (err as Error).message, stderrTail: host.sidecarStderr.split("\n").slice(-12) };
  } finally {
    await host.close();
  }
}

/** Largest number of ranges each decoration type held at once, summed (commands may clear them later). */
function peakDecorations(host: MockHost): number {
  const peak = new Map<string, number>();
  for (const t of host.traffic) {
    if (!("method" in t.message) || t.message.method !== "vscode/setDecorations") continue;
    const p = t.message.params as { key: string; uri: string; ranges: unknown[] };
    const k = `${p.key} ${p.uri}`;
    peak.set(k, Math.max(peak.get(k) ?? 0, p.ranges.length));
  }
  return [...peak.values()].reduce((a, b) => a + b, 0);
}

function withTimeout<T>(p: Promise<T>, ms: number, what: string): Promise<T> {
  return Promise.race([p, new Promise<T>((_, reject) => setTimeout(() => reject(new Error(`timeout after ${ms} ms: ${what}`)), ms).unref())]);
}

function textOf(nodes: Node[]): string[] {
  return nodes.flatMap((n) =>
    "text" in n.props ? [n.props.text.content] : "button" in n.props ? [`[${n.props.button.icon ? `$(${n.props.button.icon}) ` : ""}${n.props.button.label}]`] : "input" in n.props ? [`<input ${n.props.input.placeholder}>`] : [],
  );
}

function describeTree(nodes: Node[]): string {
  const kinds = nodes.map((n) => Object.keys(n.props)[0]);
  const counts = Object.entries(kinds.reduce<Record<string, number>>((m, k) => ((m[k] = (m[k] ?? 0) + 1), m), {}))
    .map(([k, v]) => `${v} ${k}`)
    .join(", ");
  const texts = textOf(nodes)
    .map((t) => scrubPaths(t).replace(/\s+/g, " ").trim())
    .filter(Boolean)
    .map((t) => (t.length > 60 ? `${t.slice(0, 57)}…` : t));
  return `${counts}${texts.length ? `: ${texts.slice(0, 6).join(" · ")}` : ""}`;
}

function renderMarkdown(rs: Result[]): string {
  const lines: string[] = [];
  lines.push("| Extension | Converter | Activated | APIs used | Not implemented | Commands ok/run | Probe: all missing APIs |");
  lines.push("|---|---|---|---|---|---|---|");
  for (const r of rs) {
    const s = r.sidecar as any;
    const cmds = s?.commandResults ? Object.values(s.commandResults as Record<string, string>) : [];
    const ran = cmds.filter((c) => !c.startsWith("skipped"));
    lines.push(
      `| ${r.id}@${r.version} | ${(r.converter ?? "").replace(/^[^:]+: /, "").replace(/\|/g, "/")} | ${s ? (s.activated ? "yes" : `**no**${s.activationError ? `: ${escape(s.activationError).slice(0, 120)}` : ""}`) : "n/a (no code)"} | ${s ? Object.keys(s.used ?? {}).length : ""} | ${s ? Object.keys(s.notImplemented ?? {}).map((a) => `\`${a}\``).join(", ") || "none" : ""} | ${s ? `${ran.filter((c) => c === "ok").length}/${ran.length}` : ""} | ${r.probe ? Object.keys((r.probe as any).notImplemented ?? {}).map((a) => `\`${a.replace(/^vscode\./, "")}\``).join(", ") || "none" : ""} |`,
    );
  }
  for (const r of rs) {
    const s = r.sidecar as any;
    if (!s) continue;
    lines.push("", `### ${r.id}`, "");
    if (s.runnerError) lines.push(`Runner error: ${s.runnerError}`);
    if (s.activationError) lines.push(`Activation error: ${escape(s.activationError)}`, "");
    lines.push(`APIs used: ${Object.keys(s.used ?? {}).map((a) => `\`${a.replace(/^vscode\./, "")}\``).join(", ") || "none"}`, "");
    if (Object.keys(s.notImplemented ?? {}).length) lines.push(`Threw "not implemented in Fyx": ${Object.entries(s.notImplemented).map(([a, n]) => `\`${a}\` (${n}×)`).join(", ")}`, "");
    if (Object.keys(s.partial ?? {}).length) lines.push(`Partial: ${Object.entries(s.partial).map(([a, g]) => `\`${a}\`: ${escape(String(g))}`).join("; ")}`, "");
    if (Object.keys(s.unknown ?? {}).length) lines.push(`Names VS Code lacks too (proposed/feature detection): ${Object.keys(s.unknown).map((a) => `\`${a}\``).join(", ")}`, "");
    const e = s.effects ?? {};
    lines.push(`Effects seen by the host: ${e.diagnostics ?? 0} diagnostics,${e.editorOptions && Object.keys(e.editorOptions).length ? ` editor options ${escape(JSON.stringify(Object.values(e.editorOptions)[0]))},` : ""} ${e.decorationTypes ?? 0} decoration types / ${e.decoratedRanges ?? 0} ranges, providers [${(e.providers ?? []).join(", ")}], edited [${(e.editedDocuments ?? []).join(", ")}], host commands [${(e.hostCommands ?? []).join(", ")}]${e.messages?.length ? `, messages: ${e.messages.map(escape).join(" / ")}` : ""}`, "");
    for (const [sid, d] of Object.entries(s.surfaces ?? {})) {
      if (/^1 flex$|No output yet\.$/.test(String(d))) continue;
      lines.push(`- surface \`${sid.replace(`${r.id}/`, "")}\`: ${escape(String(d))}`);
    }
    const cmds = Object.entries((s.commandResults ?? {}) as Record<string, string>);
    if (cmds.length) {
      const ok = cmds.filter(([, v]) => v === "ok").length;
      lines.push("", `Commands: ${ok} of ${cmds.length} ran without error.`, ...cmds.filter(([, v]) => v !== "ok").map(([c, v]) => `- \`${c}\`: ${escape(v)}`));
    }
    const feat = Object.entries(s.features ?? {}).map(([c, v]) => `- ${c}: ${escape(String(v))}`);
    if (feat.length) lines.push("", "Language features:", ...feat);
    if (s.errors?.length) lines.push("", "Errors:", ...s.errors.slice(0, 6).map((x: string) => `- ${escape(x).slice(0, 220)}`));
    if (s.schemaErrors?.length) lines.push("", "Schema violations:", ...s.schemaErrors.map((x: string) => `- ${escape(x)}`));
    lines.push("", `RPC traffic: ${Object.entries(s.traffic ?? {}).map(([k, v]) => `${k} ×${v}`).join(", ")}`);
  }
  return `${lines.join("\n")}\n`;
}

/** Markdown-safe, with machine-specific paths replaced by placeholders. */
function escape(s: string): string {
  return scrubPaths(s).replace(/\|/g, "\\|").replace(/\r?\n/g, " ").replace(/</g, "&lt;");
}

/** Machine-specific absolute paths (and file: URIs) become `…/<last segment>`. */
function scrubPaths(text: string): string {
  return text.replace(/(?:file:\/{2,3})?(?:[a-zA-Z](?::|%3A)|\/(?=Users\/|home\/|projects\/))[\\/][^\s"'`|()]*/g, (m) => {
    const parts = m.split(/[\\/]/).filter(Boolean);
    return `…/${parts[parts.length - 1] ?? ""}`;
  });
}
