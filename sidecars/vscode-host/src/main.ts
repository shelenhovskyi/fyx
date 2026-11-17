#!/usr/bin/env node
// Sidecar entry point: JSON-RPC over stdin/stdout (newline-delimited JSON).
//
// stdout belongs to the protocol, so console output and stray `process.stdout.write` calls from
// extension code are redirected: console → `log` notifications (WIT `host.log`) and stderr.

import fs from "node:fs";
import { builtinModules } from "node:module";
import { format } from "node:util";
import { ExtensionHost, sandboxProblem } from "./host.ts";
import { enableProbeMode } from "./shim/api.ts";
import { RpcPeer } from "./rpc.ts";
import type { LogLevel } from "./schema.ts";

// Checked before the RPC loop starts and so before any extension can load: a launcher that forgets
// the flags must not run extensions unsandboxed.
const sandbox = sandboxProblem((process as unknown as { permission?: { has(scope: string, reference?: string): boolean } }).permission);
if (sandbox) {
  process.stderr.write(`[error] refusing to start: ${sandbox}\n`);
  process.exit(78);
}

const writeOut = process.stdout.write.bind(process.stdout);
const writeErr = process.stderr.write.bind(process.stderr);
process.stdout.write = ((chunk: string | Uint8Array, ...rest: unknown[]) => (writeErr as (c: string | Uint8Array, ...r: unknown[]) => boolean)(chunk, ...rest)) as typeof process.stdout.write;

// Under --permission, `process.binding` is denied. Bundled telemetry libraries (applicationinsights'
// diagnostic-channel) call `process.binding("natives")` at load time only to list builtin
// modules, so answer that one call from `module.builtinModules`.
const originalBinding = (process as unknown as { binding: (name: string) => unknown }).binding;
(process as unknown as { binding: (name: string) => unknown }).binding = (name: string) => {
  try {
    return originalBinding.call(process, name);
  } catch (err) {
    if (name === "natives") return Object.fromEntries(builtinModules.map((m) => [m, ""]));
    throw err;
  }
};

// `fs.existsSync` is documented never to throw, but under --permission it throws
// ERR_ACCESS_DENIED for paths outside the grant. Extensions probe home-directory paths with it
// (REST Client: ~/.rest-client), so restore the contract: an inaccessible path does not exist.
// This grants nothing.
const originalExistsSync = fs.existsSync;
fs.existsSync = ((path: fs.PathLike) => {
  try {
    return originalExistsSync(path);
  } catch {
    return false;
  }
}) as typeof fs.existsSync;

if (process.argv.includes("--fyx-coverage-probe")) {
  enableProbeMode();
  writeErr("[warn] coverage probe mode: missing vscode.* APIs return inert stubs instead of throwing\n");
} else if (process.env.FYX_VSCODE_PROBE) {
  writeErr("[warn] FYX_VSCODE_PROBE is ignored; probe mode needs --fyx-coverage-probe (coverage runs only)\n");
}

const rpc = new RpcPeer((line) => void writeOut(line));
const host = new ExtensionHost(rpc);

const levels: Record<string, LogLevel> = { log: "info", info: "info", debug: "debug", trace: "trace", warn: "warn", error: "error" };
for (const [method, level] of Object.entries(levels)) {
  (console as unknown as Record<string, (...args: unknown[]) => void>)[method] = (...args: unknown[]) => {
    const message = format(...args);
    writeErr(`[${level}] ${message}\n`);
    rpc.notify("log", { level, message });
  };
}

process.on("uncaughtException", (err) => {
  host.recordUncaught(err);
  writeErr(`[error] uncaught: ${err.stack ?? err}\n`);
});
process.on("unhandledRejection", (err) => {
  host.recordUncaught(err);
  writeErr(`[error] unhandled rejection: ${(err as Error)?.stack ?? err}\n`);
});

rpc.on("exit", () => {
  setImmediate(() => process.exit(0));
  return null;
});

process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk: string) => rpc.receive(chunk));
process.stdin.on("end", () => {
  void host.shutdown().finally(() => process.exit(0));
});
