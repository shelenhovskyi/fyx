// What every part of the shim shares for one loaded extension.

import type { Coverage } from "./coverage.ts";
import type { RpcPeer } from "./rpc.ts";
import type { LogLevel } from "./schema.ts";
import type { ConfigStore } from "./shim/config.ts";
import type { DocumentStore } from "./shim/documents.ts";
import type { Uri } from "./shim/types.ts";
import type { SurfaceRegistry } from "./ui/surfaces.ts";

export interface ExtensionInfo {
  /** `publisher.name` */
  id: string;
  displayName: string;
  version: string;
  /** Root folder of the unpacked extension. */
  path: string;
  uri: Uri;
  manifest: Record<string, any>;
}

export interface HostContext {
  rpc: RpcPeer;
  coverage: Coverage;
  ext: ExtensionInfo;
  surfaces: SurfaceRegistry;
  documents: DocumentStore;
  config: ConfigStore;
  workspaceFolders: Uri[];
  storagePath: string;
  globalStoragePath: string;
  logPath: string;
  /** Runs `activate()` if `event` is one of the extension's activation events (or implied). */
  activate(event: string): Promise<void>;
  /** Executes a command the way `vscode.commands.executeCommand` does. */
  executeCommand(command: string, ...args: unknown[]): Promise<unknown>;
  log(level: LogLevel, message: string): void;
}
