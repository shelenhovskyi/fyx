#!/usr/bin/env node
// Sidecar entry point: JSON-RPC over stdin/stdout (newline-delimited JSON).
//
// stdout belongs to the protocol, so console output and stray `process.stdout.write` calls from
// extension code are redirected: console → `log` notifications (WIT `host.log`) and stderr.

import { format } from "node:util";
import { ExtensionHost } from "./host.ts";
import { RpcPeer } from "./rpc.ts";
import type { LogLevel } from "./schema.ts";

const writeOut = process.stdout.write.bind(process.stdout);
const writeErr = process.stderr.write.bind(process.stderr);
process.stdout.write = ((chunk: string | Uint8Array, ...rest: unknown[]) => (writeErr as (c: string | Uint8Array, ...r: unknown[]) => boolean)(chunk, ...rest)) as typeof process.stdout.write;

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
