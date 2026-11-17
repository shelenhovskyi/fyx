# vscode-host

A Node 24 sidecar that runs an unmodified VS Code extension (`.vsix` or unpacked folder) against a
partial `vscode.*` shim, and talks to the Fyx host over JSON-RPC 2.0 (newline-delimited JSON on
stdin/stdout). UI-ish APIs render as `fyx:ui@0.1.0` trees (the JSON mirror of
`crates/ui_ext/wit/fyx-ui.wit`); everything else is a `vscode/*` method. Layer 6 of
ARCHITECTURE.md, prototyped with the host mocked.

Coverage, protocol and findings: [docs/vscode-coverage.md](../../docs/vscode-coverage.md).

```bash
npm install
npm test                # unit + end-to-end tests against the kitchen-sink fixture extension
npm run fetch           # download the pinned Open VSX extensions into vsix/ (gitignored)
npm run coverage        # run them all; writes .cache/coverage/ and the generated part of the doc
npm run gen-catalog     # regenerate src/api-catalog.json from @types/vscode
```

Run the sidecar by hand (it waits for `initialize` on stdin):

```bash
node --permission --allow-fs-read=<ext dir> --allow-fs-read=<this dir> --allow-fs-read=<workspace> --allow-fs-write=<workspace> src/main.ts
```

`main.ts` refuses to start (exit code 78, reason on stderr) without `--permission`, or with a grant
that voids the sandbox: `--allow-fs-read=*`, `--allow-fs-write=*`, `--allow-child-process`,
`--allow-worker`, `--allow-wasi` or `--allow-addons`. Whoever launches it, the Rust host included,
must pass path-scoped grants.

## Layout

| Path | Role |
|---|---|
| `src/main.ts` | stdio entry; keeps stdout for the protocol (console → `log` notifications) |
| `src/host.ts` | `ExtensionHost`: unpack, `require("vscode")` hook, activation events, RPC handlers |
| `src/schema.ts` | `fyx:ui` JSON mirror, builders, validation, `diff` (tree → patches), `applyUpdate` |
| `src/rpc.ts` | JSON-RPC peer over NDJSON |
| `src/shim/` | the `vscode` module: value types, documents, configuration, editors/decorations, languages, workspace, assembly with not-implemented handling (`api.ts`) |
| `src/ui/` | status bar, tree view, QuickPick/InputBox and output channel as `fyx:ui` surfaces |
| `src/mock-host.ts` | the host stand-in: records traffic, keeps retained trees, validates every node and patch |
| `src/coverage.ts` | per-extension API usage and "not implemented in Fyx" records |
| `scripts/` | catalog generator, Open VSX fetcher, coverage runner |
| `extensions.json` | the pinned extension list |

`src/zip.ts` is a copy of `tools/vsix-convert/src/zip.ts`; the two packages stay independent.

## Safety

Downloaded extensions run only in a child process under Node's permission model: read access to the
extension and its sandbox folder, write access to the sandbox folder, no child processes, workers,
addons or inspector. Node 24 has no network permission, so the coverage runner skips commands that
send network requests (REST Client). `npm run fetch` accepts only `open-vsx.org` downloads, checks
the published SHA-256 and requires a permissive license.
