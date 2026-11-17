// Records which `vscode.*` APIs an extension touched, which ones are missing in Fyx, and what
// failed. The coverage report (docs/vscode-coverage.md) is built from these records.

import { ErrorCodes } from "./rpc.ts";

export class NotImplementedInFyx extends Error {
  readonly api: string;
  readonly code = ErrorCodes.NotImplemented;
  constructor(api: string, detail?: string, message?: string) {
    super(message ?? `${api} is not implemented in Fyx${detail ? ` (${detail})` : ""}`);
    this.name = "NotImplementedInFyx";
    this.api = api;
  }
}

/** An API that exists in Fyx, used in a way 0.1 cannot honour (for example a masked input). */
export function notSupported(api: string, what: string): NotImplementedInFyx {
  return new NotImplementedInFyx(api, undefined, `${api}: ${what} is not supported in Fyx 0.1`);
}

export function isNotImplemented(err: unknown): err is NotImplementedInFyx {
  return err instanceof Error && err.name === "NotImplementedInFyx" && typeof (err as NotImplementedInFyx).api === "string";
}

export interface CoverageReport {
  used: Record<string, number>;
  notImplemented: Record<string, number>;
  unknown: Record<string, number>;
  partial: Record<string, string>;
  errors: string[];
}

export class Coverage {
  private used = new Map<string, number>();
  private notImplemented = new Map<string, number>();
  private unknown = new Map<string, number>();
  private partial = new Map<string, string>();
  private errors: string[] = [];

  use(api: string): void {
    bump(this.used, api);
  }

  /** The extension called (or read) an API Fyx does not implement. */
  missing(api: string): void {
    bump(this.notImplemented, api);
  }

  /** A name that is not part of the VS Code API either (typo, proposed API, feature detection). */
  unknownName(api: string): void {
    bump(this.unknown, api);
  }

  /** Implemented, but with a known gap worth reporting (first note wins). */
  note(api: string, gap: string): void {
    if (!this.partial.has(api)) this.partial.set(api, gap);
  }

  error(message: string): void {
    if (this.errors.length < 50 && !this.errors.includes(message)) this.errors.push(message);
  }

  report(): CoverageReport {
    const obj = (m: Map<string, number>) => Object.fromEntries([...m].sort(([a], [b]) => a.localeCompare(b)));
    return {
      used: obj(this.used),
      notImplemented: obj(this.notImplemented),
      unknown: obj(this.unknown),
      partial: Object.fromEntries([...this.partial].sort(([a], [b]) => a.localeCompare(b))),
      errors: [...this.errors],
    };
  }
}

function bump(m: Map<string, number>, key: string): void {
  m.set(key, (m.get(key) ?? 0) + 1);
}
