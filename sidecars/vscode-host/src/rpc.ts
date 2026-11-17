// JSON-RPC 2.0 peer over newline-delimited JSON (one message per line, like ACP). Both sides may
// send requests and notifications. Transport-agnostic: give it something to write lines to and
// feed it the lines you read.

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

export interface RpcRequest {
  jsonrpc: "2.0";
  id: number | string;
  method: string;
  params?: unknown;
}
export interface RpcNotification {
  jsonrpc: "2.0";
  method: string;
  params?: unknown;
}
export interface RpcResponse {
  jsonrpc: "2.0";
  id: number | string | null;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
}
export type RpcMessage = RpcRequest | RpcNotification | RpcResponse;

export const ErrorCodes = {
  ParseError: -32700,
  InvalidRequest: -32600,
  MethodNotFound: -32601,
  InvalidParams: -32602,
  InternalError: -32603,
  /** `result<_, string>` returned `err`: the WIT-level error. */
  ExtensionError: -32000,
  /** A `vscode.*` API the extension called is not implemented in Fyx. */
  NotImplemented: -32001,
} as const;

export class RpcError extends Error {
  code: number;
  data?: unknown;
  constructor(code: number, message: string, data?: unknown) {
    super(message);
    this.code = code;
    this.data = data;
  }
}

type Handler = (params: any) => unknown;

export class RpcPeer {
  private nextId = 1;
  private pending = new Map<number | string, { resolve: (v: any) => void; reject: (e: Error) => void }>();
  private handlers = new Map<string, Handler>();
  private fallback?: (method: string, params: unknown) => unknown;
  private buffer = "";
  private closed = false;
  private write: (line: string) => void;
  /** Called for every message in or out; the mock host uses it to record traffic. */
  onTraffic?: (direction: "in" | "out", message: RpcMessage) => void;

  constructor(write: (line: string) => void) {
    this.write = write;
  }

  on(method: string, handler: Handler): this {
    this.handlers.set(method, handler);
    return this;
  }

  onUnhandled(handler: (method: string, params: unknown) => unknown): this {
    this.fallback = handler;
    return this;
  }

  request<T = unknown>(method: string, params?: unknown): Promise<T> {
    if (this.closed) return Promise.reject(new Error(`rpc closed (request ${method})`));
    const id = this.nextId++;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({ jsonrpc: "2.0", id, method, ...(params === undefined ? {} : { params }) });
    });
  }

  notify(method: string, params?: unknown): void {
    if (this.closed) return;
    this.send({ jsonrpc: "2.0", method, ...(params === undefined ? {} : { params }) });
  }

  /** Feed raw transport data; complete lines are dispatched. */
  receive(chunk: string): void {
    this.buffer += chunk;
    let nl: number;
    while ((nl = this.buffer.indexOf("\n")) >= 0) {
      const line = this.buffer.slice(0, nl).trim();
      this.buffer = this.buffer.slice(nl + 1);
      if (line) this.dispatchLine(line);
    }
  }

  close(reason = "rpc closed"): void {
    this.closed = true;
    for (const { reject } of this.pending.values()) reject(new Error(reason));
    this.pending.clear();
  }

  private send(message: RpcMessage): void {
    this.onTraffic?.("out", message);
    this.write(`${JSON.stringify(message)}\n`);
  }

  private dispatchLine(line: string): void {
    let message: RpcMessage;
    try {
      message = JSON.parse(line);
    } catch {
      this.send({ jsonrpc: "2.0", id: null, error: { code: ErrorCodes.ParseError, message: "invalid JSON" } });
      return;
    }
    this.onTraffic?.("in", message);
    if ("method" in message) {
      void this.handle(message);
    } else if ("id" in message && message.id !== null) {
      const waiter = this.pending.get(message.id);
      if (!waiter) return;
      this.pending.delete(message.id);
      if (message.error) waiter.reject(new RpcError(message.error.code, message.error.message, message.error.data));
      else waiter.resolve(message.result);
    }
  }

  private async handle(message: RpcRequest | RpcNotification): Promise<void> {
    const isRequest = "id" in message;
    const handler = this.handlers.get(message.method);
    try {
      let result: unknown;
      if (handler) result = await handler(message.params);
      else if (this.fallback) result = await this.fallback(message.method, message.params);
      else throw new RpcError(ErrorCodes.MethodNotFound, `method not found: ${message.method}`);
      if (isRequest) this.send({ jsonrpc: "2.0", id: message.id, result: result === undefined ? null : result });
    } catch (err) {
      if (!isRequest) return;
      const e = err as Error & { code?: unknown; data?: unknown };
      const code = typeof e.code === "number" ? e.code : ErrorCodes.InternalError;
      this.send({
        jsonrpc: "2.0",
        id: message.id,
        error: { code, message: e.message ?? String(err), ...(e.data !== undefined ? { data: e.data } : {}) },
      });
    }
  }
}

/** Two peers wired back to back in one process (tests). */
export function pairedPeers(): [RpcPeer, RpcPeer] {
  let a!: RpcPeer;
  let b!: RpcPeer;
  a = new RpcPeer((line) => queueMicrotask(() => b.receive(line)));
  b = new RpcPeer((line) => queueMicrotask(() => a.receive(line)));
  return [a, b];
}
