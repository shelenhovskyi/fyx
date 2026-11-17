// `workspace.getConfiguration`. Values come from three layers, lowest first: VS Code defaults the
// shim knows, the extension's `contributes.configuration` defaults, and settings the host sends
// (`initialize.settings`, `vscode/didChangeConfiguration`), keyed by dotted name.

import { ConfigurationTarget, EventEmitter } from "./types.ts";

type Settings = Record<string, unknown>;

/** Defaults for core VS Code settings extensions commonly read. */
const CORE_DEFAULTS: Settings = {
  "editor.tabSize": 4,
  "editor.insertSpaces": true,
  "editor.detectIndentation": true,
  "editor.fontSize": 14,
  "editor.fontFamily": "monospace",
  "editor.lineHeight": 0,
  "editor.wordWrap": "off",
  "editor.formatOnSave": false,
  "editor.renderWhitespace": "selection",
  "editor.rulers": [],
  "editor.wordSeparators": "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?",
  "files.encoding": "utf8",
  "files.eol": "auto",
  "files.insertFinalNewline": false,
  "files.trimTrailingWhitespace": false,
  "files.trimFinalNewlines": false,
  "files.exclude": { "**/.git": true, "**/.svn": true, "**/.hg": true, "**/CVS": true, "**/.DS_Store": true, "**/Thumbs.db": true },
  "files.associations": {},
  "search.exclude": { "**/node_modules": true, "**/bower_components": true, "**/*.code-search": true },
  "workbench.colorTheme": "Fyx Dark",
  "http.proxy": "",
  "http.proxyStrictSSL": true,
};

export interface ConfigurationChangeEvent {
  affectsConfiguration(section: string, scope?: unknown): boolean;
}

export class ConfigStore {
  private defaults: Settings = { ...CORE_DEFAULTS };
  private user: Settings = {};
  private workspace: Settings = {};
  readonly onDidChange = new EventEmitter<ConfigurationChangeEvent>();
  /** Called on `update()`, so the host can persist the change. */
  onUpdate: (key: string, value: unknown, target: number) => void = () => {};

  addContributedDefaults(configuration: unknown): void {
    const blocks = Array.isArray(configuration) ? configuration : configuration ? [configuration] : [];
    for (const block of blocks) {
      const properties = (block as { properties?: Record<string, { default?: unknown; type?: string | string[] }> }).properties ?? {};
      for (const [key, schema] of Object.entries(properties)) {
        if (!schema) continue;
        // Like VS Code's configuration registry: a property without `default` gets its type's default.
        this.defaults[key] = "default" in schema ? schema.default : typeDefault(schema.type);
      }
    }
  }

  /** Replaces host settings (nested objects are flattened to dotted keys) and fires a change event. */
  setUserSettings(settings: Settings, fire = true): void {
    const before = this.user;
    this.user = flatten(settings);
    if (fire) this.fireChanged(changedKeys(before, this.user));
  }

  get(section?: string): WorkspaceConfiguration {
    return makeConfiguration(this, section);
  }

  /** @internal */
  lookup(key: string): { found: boolean; value?: unknown; defaultValue?: unknown; globalValue?: unknown; workspaceValue?: unknown } {
    const layers = [this.defaults, this.user, this.workspace];
    let found = false;
    let value: unknown;
    for (const layer of layers) {
      const v = readKey(layer, key);
      if (v !== undefined) {
        found = true;
        value = isPlainObject(value) && isPlainObject(v) ? { ...value, ...v } : v;
      }
    }
    return { found, value, defaultValue: readKey(this.defaults, key), globalValue: readKey(this.user, key), workspaceValue: readKey(this.workspace, key) };
  }

  /** @internal */
  keysUnder(prefix: string): string[] {
    const all = new Set([...Object.keys(this.defaults), ...Object.keys(this.user), ...Object.keys(this.workspace)]);
    return [...all].filter((k) => !prefix || k.startsWith(`${prefix}.`)).map((k) => (prefix ? k.slice(prefix.length + 1) : k));
  }

  /** @internal */
  update(key: string, value: unknown, target: number): void {
    const layer = target === ConfigurationTarget.Global ? this.user : this.workspace;
    for (const k of Object.keys(layer)) if (k.startsWith(`${key}.`)) delete layer[k];
    if (value === undefined) delete layer[key];
    else layer[key] = value;
    this.onUpdate(key, value, target);
    this.fireChanged([key]);
  }

  private fireChanged(keys: string[]): void {
    if (keys.length === 0) return;
    this.onDidChange.fire({
      affectsConfiguration: (section: string) => keys.some((k) => k === section || k.startsWith(`${section}.`) || section.startsWith(`${k}.`)),
    });
  }
}

export interface WorkspaceConfiguration {
  get<T>(key: string, defaultValue?: T): T | undefined;
  has(key: string): boolean;
  inspect(key: string): unknown;
  update(key: string, value: unknown, target?: boolean | number | null): Promise<void>;
  readonly [key: string]: unknown;
}

function makeConfiguration(store: ConfigStore, section?: string): WorkspaceConfiguration {
  const full = (key: string) => (section ? `${section}.${key}` : key);
  const api = {
    get(key: string, defaultValue?: unknown) {
      const { found, value } = store.lookup(full(key));
      return found ? structuredClone(value) : defaultValue;
    },
    has(key: string) {
      return store.lookup(full(key)).found;
    },
    inspect(key: string) {
      const r = store.lookup(full(key));
      return { key: full(key), defaultValue: r.defaultValue, globalValue: r.globalValue, workspaceValue: r.workspaceValue };
    },
    async update(key: string, value: unknown, target?: boolean | number | null) {
      const t = target === true || target === ConfigurationTarget.Global ? ConfigurationTarget.Global : target === false || target === undefined || target === null ? ConfigurationTarget.Workspace : target;
      store.update(full(key), value, t);
    },
  };
  // A WorkspaceConfiguration also exposes its values as properties (`config.enable`).
  const values: Record<string, unknown> = {};
  for (const key of store.keysUnder(section ?? "")) {
    const top = key.split(".")[0];
    if (!(top in api) && !(top in values)) values[top] = api.get(top);
  }
  return Object.freeze(Object.assign(Object.create(null), values, api)) as WorkspaceConfiguration;
}

/** Reads `key` from flat dotted settings, assembling an object from deeper keys when needed. */
function readKey(layer: Settings, key: string): unknown {
  if (Object.hasOwn(layer, key)) return layer[key];
  const prefix = `${key}.`;
  let out: Record<string, unknown> | undefined;
  for (const [k, v] of Object.entries(layer)) {
    if (!k.startsWith(prefix)) continue;
    out ??= {};
    setPath(out, k.slice(prefix.length).split("."), v);
  }
  // A parent key holding an object also answers for its children (`files.exclude` → `files`).
  if (out === undefined) {
    const parts = key.split(".");
    for (let i = parts.length - 1; i > 0; i--) {
      const parent = parts.slice(0, i).join(".");
      if (Object.hasOwn(layer, parent) && isPlainObject(layer[parent])) {
        let v: unknown = layer[parent];
        for (const p of parts.slice(i)) v = isPlainObject(v) ? (v as Record<string, unknown>)[p] : undefined;
        return v;
      }
    }
  }
  return out;
}

function setPath(obj: Record<string, unknown>, path: string[], value: unknown): void {
  let o = obj;
  for (const p of path.slice(0, -1)) o = (o[p] = isPlainObject(o[p]) ? o[p] : {}) as Record<string, unknown>;
  o[path[path.length - 1]] = value;
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/**
 * Flattens nested settings (`{ editor: { tabSize: 2 } }`) into dotted keys. Keys that already
 * contain a dot, language overrides (`[python]`) and objects whose keys are not identifiers
 * (`files.exclude` globs) are kept as values.
 */
export function flatten(settings: Settings, prefix = "", out: Settings = {}): Settings {
  for (const [k, v] of Object.entries(settings)) {
    const key = prefix ? `${prefix}.${k}` : k;
    const nested =
      isPlainObject(v) &&
      !k.includes(".") &&
      !k.startsWith("[") &&
      Object.keys(v).length > 0 &&
      Object.keys(v).every((c) => /^[A-Za-z_$][\w$-]*$/.test(c));
    if (nested) flatten(v, key, out);
    else out[key] = v;
  }
  return out;
}

function typeDefault(type: string | string[] | undefined): unknown {
  switch (Array.isArray(type) ? type[0] : type) {
    case "string":
      return "";
    case "boolean":
      return false;
    case "number":
    case "integer":
      return 0;
    case "array":
      return [];
    case "object":
      return {};
    default:
      return null;
  }
}

function changedKeys(a: Settings, b: Settings): string[] {
  const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
  return [...keys].filter((k) => JSON.stringify(a[k]) !== JSON.stringify(b[k]));
}
