import { test } from "node:test";
import assert from "node:assert/strict";
import { ErrorCodes, RpcError, RpcPeer, pairedPeers } from "../src/rpc.ts";

test("requests, notifications and errors round-trip", async () => {
  const [a, b] = pairedPeers();
  const notes: unknown[] = [];
  b.on("add", (p: { x: number; y: number }) => p.x + p.y);
  b.on("fail", () => {
    throw new RpcError(ErrorCodes.ExtensionError, "nope", { why: 1 });
  });
  b.on("note", (p) => void notes.push(p));
  assert.equal(await a.request("add", { x: 2, y: 3 }), 5);
  await assert.rejects(a.request("fail"), (e: RpcError) => e.code === ErrorCodes.ExtensionError && e.message === "nope" && (e.data as { why: number }).why === 1);
  await assert.rejects(a.request("missing"), (e: RpcError) => e.code === ErrorCodes.MethodNotFound);
  a.notify("note", { n: 1 });
  await new Promise((r) => setTimeout(r, 5));
  assert.deepEqual(notes, [{ n: 1 }]);
});

test("newline framing survives split and merged chunks", async () => {
  const lines: string[] = [];
  const peer = new RpcPeer((l) => lines.push(l));
  const got: unknown[] = [];
  peer.on("m", (p) => void got.push(p));
  const msg = (n: number) => JSON.stringify({ jsonrpc: "2.0", method: "m", params: n }) + "\n";
  const all = msg(1) + msg(2) + msg(3);
  peer.receive(all.slice(0, 7));
  peer.receive(all.slice(7, 40));
  peer.receive(all.slice(40));
  await new Promise((r) => setTimeout(r, 5));
  assert.deepEqual(got, [1, 2, 3]);
  peer.receive("not json\n");
  assert.match(lines.at(-1)!, /-32700/);
});
