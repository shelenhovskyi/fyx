# Network audit 1: what Fyx can still send

**Scope.** `origin/integration` at `f3718ca`. This audit covers every outbound HTTP(S)/WebSocket
request the Fyx app (`crates/zed` binary) can make, with Fyx's defaults
(`assets/settings/default.json` + `assets/settings/fyx.json`, layered by `fyx_default_settings`,
`crates/zed/src/main.rs:505,1824`). Tests, docs and comments are out of scope. So are links that
only open in the system browser (listed at the end).

**Method.** Static reading only. Nothing was compiled or run. Four parts were read:
- the zed.dev paths (client, telemetry, crashes, auto-update, extension registry), read here;
- dependency downloads, read by a separate reviewer;
- AI providers, read by a separate reviewer;
- everything else, read by a separate reviewer.

The reviewers' load-bearing claims were re-checked against the code here: prettier on open,
registry on startup, Copilot env token, Anthropic `/v1/log/zed`, the Anthropic model fetch,
extension `fetch` with no capability check, and blame-popover avatars. Claims that were not
re-checked and depend on third-party server behaviour are marked *unverified*.

**Classes.**
- **(a)** at startup, with no user action;
- **(b)** on an explicit user action;
- **(c)** implicit or background later: opening a file or view, timers, settings changes;
- **(d)** dead in Fyx. The request is still built, but it goes to `http://127.0.0.1:0` and fails
  at `connect()` before a packet is sent.

## Verdict

W1's measurement (zero non-loopback connections at startup) holds for **a fresh profile with no
folder open and no credentials**. Every zed.dev path is dead (class d) because of the
`server_url` loopback trick. Telemetry, crash upload, auto-update, the extension registry and
edit predictions are all off by default.

The claim does not hold in three common situations:
1. **A folder is open (or restored) and an AI key exists** in the environment or keychain. The
   agent panel then authenticates every provider at startup, and several providers fetch their
   model list with the key (Anthropic, Google, OpenRouter, Vercel, OpenCode).
2. **`GH_COPILOT_TOKEN` / `GITHUB_COPILOT_TOKEN` is set.** Copilot Chat calls GitHub at startup
   with no folder open, even with `disable_ai: true`.
3. **A file is opened.** Built-in language servers check GitHub or npm for a newer version once
   per server per launch. Default prettier installs from npm. Git blame hover fetches author
   avatars. This includes opening Fyx's own `settings.json`, which starts json-language-server
   and the prettier install.

None of these goes to zed.dev, and all of them except the Anthropic log are features the user
opted into in some form. They are still requests the user did not ask for at that moment.

## How zed.dev is neutralised (verified here)

- `server_url: "http://127.0.0.1:0"` (`fyx.json`) feeds `Client::production`
  (`crates/client/src/client.rs:584-590`), and settings changes keep it in sync
  (`main.rs:827-829`).
- Every cloud URL is derived from it in `HttpClientWithUrl::build_zed_api_url` /
  `build_zed_cloud_url*` / `build_zed_llm_url` (`crates/http_client/src/http_client.rs:277-330`).
  Their `zed.dev → api./cloud.` mapping only fires for the literal `https://zed.dev`.
- Sign-in at startup (`main.rs:1380`) only runs with stored credentials. Those are keyed by
  `server_url` (`client.rs:371-374`), so a Zed login in the keychain is not picked up.
- The `ZED_SERVER_URL` and `ZED_RPC_URL` env vars still override `server_url`
  (`client.rs:63-65,119`). A user who exported them for Zed takes them into Fyx.

## Request table

### zed.dev and Zed-operated services

| Path (file:line) | Host | Trigger | Class | Fyx default / setting |
|---|---|---|---|---|
| `client/src/telemetry.rs:566-617,644-703` | api.zed.dev `/telemetry/events` | event queue flush (5 min / queue full / quit) | d | `telemetry.metrics: false` returns early at :571 before queueing. Also loopback |
| `zed/src/reliability.rs:51-55,67,259-300` | `ZED_MINIDUMP_ENDPOINT` (Sentry) | startup upload of old minidumps; crash | d | `telemetry.diagnostics: false`. The endpoint is a build-time or env var. Upstream `release_nightly.yml` bakes it from a secret the fork lacks |
| `zed/src/reliability/hang_detection.rs:77` | via telemetry | hang reports | d | goes through `telemetry::send_event`, so gated by `metrics` |
| `auto_update/src/auto_update.rs:281-296,490-512,708` | cloud.zed.dev releases | hourly poll (15 min nightly) | d | `auto_update: false`. A manual "Check for Updates" goes to loopback |
| `auto_update_ui/src/auto_update_ui.rs:108-122` | zed.dev release notes | "View release notes" | d | loopback. The startup "updated" toast makes no request |
| `extension_host/src/extension_host.rs:464-470,778-842` | api.zed.dev `/extensions/updates` | **every startup** (`check_for_updates` after index load) | a→d | loopback, attempted every launch. `auto_install_extensions.html: false` stops the install at :746 |
| `extension_host/src/extension_host.rs:661-739,975-1040`; `extensions_ui/src/extensions_ui.rs:654-696` | api.zed.dev | Extensions page, install | d | loopback |
| `client/src/client.rs:1297` (+ `/rpc`) | collab WebSocket | sign-in / reconnect | d | `title_bar.show_sign_in: false`; loopback. `call`, `channel`, `notifications` and LiveKit only use URLs that zed.dev RPC supplies (`call/src/call_impl/room.rs:1761`) |
| `language_models_cloud/src/language_models_cloud.rs:261,629`; `web_search_providers/src/cloud.rs:74`; `edit_prediction/src/edit_prediction.rs:1157,1933,2391,2974,3057` | llm.zed.dev / cloud.zed.dev | zed.dev model, web search, zeta | d | loopback; `edit_predictions.provider: "none"` |
| `remote/src/transport/ssh.rs:1105-1113`, `wsl.rs:282`, `docker.rs:299,327` → `auto_update.rs:707` | cloud.zed.dev remote-server download | SSH/WSL/Docker connect with no server binary on the host | d | loopback. Release builds have no build-from-source fallback (`transport.rs:261`), so **remote dev fails** unless `zed-remote-server-*` is already on the host |

### AI providers (startup and background)

The agent panel loads unless `disable_ai` is set (`zed/src/zed.rs:836-847`). When a worktree is
open it creates the native agent (`agent_ui/src/agent_panel.rs:2958-2971`). That calls
`authenticate_providers` (`agent/src/agent.rs:427`; `language_model/src/registry.rs:545-576`).
Keys come from the env first, then the keychain (`language_model/src/api_key.rs:235-251`).

| Path (file:line) | Host | Trigger | Class | Fyx default / setting |
|---|---|---|---|---|
| `language_models/src/provider/anthropic.rs:87-125` | api.anthropic.com `/v1/models` | startup with a folder open, if `ANTHROPIC_API_KEY` or a keychain key; api_url change | a/c | **not disabled.** Only `disable_ai: true` |
| `provider/google.rs:63-139` | generativelanguage.googleapis.com | same, `GEMINI_API_KEY` / `GOOGLE_AI_API_KEY` | a/c | not disabled; `disable_ai` |
| `provider/open_router.rs:80-115` | openrouter.ai | same, `OPENROUTER_API_KEY` | a/c | not disabled; `disable_ai` |
| `provider/vercel_ai_gateway.rs:73-121` | ai-gateway.vercel.sh | same, `VERCEL_AI_GATEWAY_API_KEY` | a/c | not disabled; `disable_ai` |
| `provider/opencode.rs:122-214` | models.opencode.ai | same, `OPENCODE_API_KEY`; **then hourly** (:204-208) | a/c | not disabled; `disable_ai` |
| `copilot_chat/src/copilot_chat.rs:584-627`, init `zed/src/main.rs:692` | api.github.com/graphql, api.githubcopilot.com | startup, **no folder needed**, if `GH_COPILOT_TOKEN`/`GITHUB_COPILOT_TOKEN` is set (**ignores `disable_ai`**) or a keychain token exists | a | **not disabled.** Code change only for the env path |
| `openai_subscribed/src/openai_subscribed.rs:119-160,243-262` | auth.openai.com, chatgpt.com | startup if a ChatGPT-subscription login is stored: token refresh plus model list | a | not disabled; `disable_ai` (the keychain read waits on it) |
| `provider/lmstudio.rs:222-230`; `provider/llama_cpp.rs:519-521` | localhost:1234, localhost:8080 (llama.cpp router mode: SSE that reconnects every 5 s) | **every startup**, in the provider constructor, ignores `disable_ai` | a | loopback only (these are W1's two probes). Code change to gate them |
| `provider/ollama.rs:94-113` | localhost:11434 | via authenticate (folder open) | a/c | loopback; `disable_ai` |
| `provider/open_ai.rs`, `mistral.rs`, `deepseek.rs`, `x_ai.rs`, `api_compatible.rs`, `bedrock.rs`; `x_ai_subscribed` | the provider | only when a message is sent | b | — |
| `provider/anthropic/telemetry.rs:80-123` ← `agent_ui/src/inline_assistant.rs:415,1021`, `buffer_codegen.rs:854`, `terminal_codegen.rs:103` | api.anthropic.com **`/v1/log/zed`** | inline-assist shown / accepted / rejected with an Anthropic key | b (side channel) | **not gated by `telemetry.*`.** Usage telemetry to a third party |
| `copilot/src/copilot.rs:448-459,1348-1371` | registry.npmjs.org, Copilot LSP | edit prediction provider `copilot` | b | off: `edit_predictions.provider: "none"` |
| `edit_prediction/src/mercury.rs`, `codestral/src/codestral.rs`, `edit_prediction/src/ollama.rs` | inceptionlabs.ai, codestral.mistral.ai, custom | typing, when selected | b | off: provider `none` |
| `project/src/context_server_store.rs:1782-1830`; `context_server/src/oauth.rs:387-445` | user MCP servers | startup if configured | a | `context_servers: {}` (user's own config) |
| `agent/src/tools/fetch_tool.rs:80-84`; `agent_ui/src/mention_set.rs:1566-1581` | any | agent tool call / URL @-mention | b | tool permissions |
| `agent_ui/src/agent_ui.rs:104` | any image URL in model output | rendering an agent thread | c | code change only |

### Agent registry and external agents

| Path (file:line) | Host | Trigger | Class | Fyx default / setting |
|---|---|---|---|---|
| `project/src/agent_registry_store.rs:20,265-273`; `agent_server_store.rs:318-322` | cdn.agentclientprotocol.com, raw.githubusercontent.com (icons) | W1 removed the startup fetch. It **still fires at startup when a project opens if settings contain a `type: "registry"` agent**, then on settings changes if stale (>1 h), on connect (`agent_servers/src/custom.rs:224-227`) and on the registry page | a (if configured)/c/b | `agent_servers: {}` by default; `disable_ai` |
| `agent_server_store.rs:1232-1273` | api.github.com, archive host | first start of an archive agent version | b | — |
| `agent_server_store.rs:1405,1473` | registry.npmjs.org (`npm install`) | **every** connect of an npx registry agent | b/c | — |

### Language servers, Node, formatters, debuggers

The version check runs if a server is not on PATH. The cached binary is used, and
`try_fetch_server_binary` still runs **once per server per launch, at the first start**
(`language/src/language.rs:855-924`). Downloads cannot be turned off globally:
`allow_binary_download` is hard-coded (`project/src/lsp_store.rs:585`; node at `main.rs:546`,
"TODO: Expose this setting"). Only `lsp.<server>.binary.path` avoids it (`lsp_store.rs:802-803`).
It waits for worktree trust, but invisible worktrees (e.g. a lone settings file) skip trust
(`project/src/trusted_worktrees.rs:478`).

| Path (file:line) | Host | Trigger | Class | Fyx default / setting |
|---|---|---|---|---|
| `languages/src/json.rs:163,230` | registry.npmjs.org (`vscode-langservers-extracted`) | open any JSON/JSONC file, **including `settings.json`/`keymap.json`** | c | not disabled; `lsp.json-language-server.binary.path` |
| `languages/src/json.rs:537,602` | api.github.com, github.com (package-version-server) | open any JSON file | c | `lsp.package-version-server.binary.path` |
| `languages/src/rust.rs:818`; `c.rs:35`; `go.rs:101,168` (`go install` → proxy.golang.org) | GitHub releases | open .rs / C / Go file without the server on PATH | c | per-server `binary.path` |
| `languages/src/yaml.rs:49`; `css.rs:46`; `bash.rs:140`; `tailwind.rs:53`; `tailwindcss.rs:49`; `vtsls.rs:104`; `python.rs:2226` (basedpyright) | registry.npmjs.org | open YAML / CSS / shell / HTML-JS-TS (tailwind is attached to 12 languages) / Python | c | per-server `binary.path` or `languages.<L>.language_servers` |
| `languages/src/python.rs:2590` (ruff); `eslint.rs:87-148` | GitHub releases; github.com archive + npm | open Python / JS-TS file | c | per-server `binary.path` |
| `node_runtime/src/node_runtime.rs:703` | nodejs.org | first npm-based server when no `node` on PATH | c | `node.path`; download is hard-coded on |
| `project/src/prettier_store.rs:600-625,933` ← `lsp_store.rs:5270-5292,5798-5810` | registry.npmjs.org (`prettier@latest`) | **opening** (not formatting) a file whose language has `prettier.allowed: true` (JSON, JSONC, Markdown, CSS, JS/TS, HTML…) with formatter `auto` | c | not disabled. Per-language `prettier.allowed: false` or `formatter: "language_server"` |
| `yaml-language-server`, `vscode-json-language-server` (third-party behaviour) | json.schemastore.org, any `$schema` URL | YAML/JSON files | c | *unverified*: `lsp.yaml-language-server.settings.yaml.schemaStore.enable: false`. Fyx's own settings schemas are served in-process via `zed://` (`json_schema_store/src/json_schema_store.rs:141-169`) |
| `github_releases/src/releases.rs:144` | api.github.com | every GitHub release lookup above | — | sends `$GITHUB_TOKEN` as Bearer if set |
| `dap_adapters/src/{codelldb,javascript,python,go}.rs` | GitHub, npm, pypi.org, `go install` | starting a debug session | b | `dap.<adapter>.binary` |

### Git, images, extensions, other

| Path (file:line) | Host | Trigger | Class | Fyx default / setting |
|---|---|---|---|---|
| `git_ui/src/blame_ui.rs:326-336` → `commit_tooltip.rs:152-213` → `git_hosting_providers/src/providers/github.rs:75-90,134-145` | avatars.githubusercontent.com `?email=<author>`; api.github.com (with `GITHUB_TOKEN`) | **hovering inline blame** (on by default, `git.inline_blame.enabled: true`) | c | not disabled. `git.blame.show_avatar` only covers the gutter (`blame_ui.rs:141,176`). The popover is unconditional |
| `git_hosting_providers/src/providers/{gitlab,bitbucket,gitee,gitea,forgejo}.rs` | the origin's forge API | same, for non-GitHub origins: leaks owner/repo/SHA | c | code change only |
| `git_ui/src/git_panel.rs:7971`, `git_graph.rs:2119,3066`, `commit_view.rs:658` | as above, one per listed commit | History tab, git graph, commit view | b/c | code change only |
| `project/src/git_store.rs:8617-8705` | git remotes | periodic auto-fetch | c | off: `git.auto_fetch.enabled: false` (upstream default) |
| `zed/src/main.rs:1445-1463` | user's SSH/WSL/Docker hosts | session restore | a | user's own hosts; `restore_on_startup: "none"` |
| `gpui/src/elements/img.rs:627-642` via `markdown_preview/src/markdown_preview_view.rs:1128,1539-1542` | any image host in the document | opening a Markdown preview with remote images | b/c | code change only. Hover docs and `ui_ext_host` markdown have no remote resolver (`markdown/src/markdown.rs:2766-2770`) |
| `extension_host/src/wasm_host/wit/since_v0_8_0.rs:633-667` (`fetch`, `fetch_stream`) | **any** | installed extension code (language server start on file open, slash commands…) | c | **no capability check at all** (verified). Only `download_file` (:1084) and npm are gated, by `granted_extension_capabilities`, whose default is `host: "*"` (`default.json:2376-2380`) |
| `since_v0_8_0.rs:768-866` | registry.npmjs.org, api.github.com | the same | c | npm: `npm:install` capability; GitHub release calls are not gated |
| `repl/src/repl_store.rs:220-243`, `kernels/remote_kernels.rs:44-286` | `$JUPYTER_SERVER` (HTTP + WS, UA `Zed/repl`) | kernel picker / run | b/c | only with `JUPYTER_SERVER` + `JUPYTER_TOKEN` env |
| `dev_container/src/oci.rs:35-105`, `devcontainer_manifest.rs:538-581` | ghcr.io, feature registries | templates / build dev container | b | — |
| `settings_ui/src/pages/skill_creator.rs:541,982,1023,1105` | raw.githubusercontent.com (retries with `GITHUB_TOKEN`) | import skill from URL | b | — |
| `ui_ext`, `ui_ext_host`, `ui_ext_sdk`, `agent_review`, `extensions/fyx_*` | none | — | — | no network code. The Fyx UI WASI context does not call `inherit_network` (`ui_ext_host/src/fyx_ui.rs:340,452`). That it denies sockets in wasmtime-wasi 48 is *unverified* |

Checked and found no auto-started network work: onboarding, workspace, theme_selector, feedback,
install_cli, cli, terminal, sandbox, http_proxy, proxy_handshake, oauth_callback_server,
feature_flags (flags arrive over zed.dev RPC, so dead).

## Still phones home (startup or implicit), with fixes

Fixes prefer settings in `fyx.json`. Where no setting exists, the smallest `fyx:` hook is named.

1. **Copilot env token at startup.** `copilot_chat.rs:584-627`. It hits GitHub with no folder open
   and ignores `disable_ai`. *Fix (code, one hook):* in `CopilotChat::new`, skip the load or model
   fetch while `DisableAiSettings` is set, and only fetch models once the Copilot Chat provider is
   used or its settings page is opened.
2. **Model-list fetches with BYO keys at startup** (Anthropic, Google, OpenRouter, Vercel,
   OpenCode hourly; ChatGPT subscription refresh). They need a folder open and a key.
   *Setting:* `disable_ai: true` stops them, but it also removes BYO-key AI, which Fyx keeps, so
   it cannot be a default. *Fix (code):* defer `fetch_models` from `authenticate` to the first use
   of a provider (model picker opened, or message sent). Alternatively, only authenticate
   providers that have a `language_models.<provider>` entry in user settings, instead of any env
   var that happens to be set.
3. **Anthropic `/v1/log/zed` usage events** (`anthropic/telemetry.rs:80-92`). This is telemetry to
   a third party that ignores `telemetry.metrics: false`. *Fix (code, one line):* return early in
   `AnthropicEventReporter::report` unless `TelemetrySettings::metrics`. Fyx then needs no
   settings change.
4. **Git avatars on blame hover** (author email to GitHub; repo/SHA to other forges). *Setting:*
   add `"git": { "blame": { "show_avatar": false } }` to `fyx.json`. That covers the gutter only.
   *Fix (code, one hook):* make `CommitAvatar::avatar` (`git_ui/src/commit_tooltip.rs:152`)
   return the placeholder when `git.blame.show_avatar` is false. The popover, history, graph and
   commit view would then all follow the one setting.
5. **LSP version checks and downloads on file open**, including `settings.json`. These are by
   design and needed for IDE features, but not visible as network use. *Setting:* none global
   (only per-server `lsp.<name>.binary.path`). *Fix:* expose the existing `allow_binary_download`
   (`main.rs:546` TODO; `lsp_store.rs:585`) as one Fyx setting, e.g. `lsp_downloads`, keeping
   `true` as the default. Separately, consider skipping the per-launch "latest version" check when
   a cached binary exists. That is the request users don't expect.
6. **Default prettier `npm install` on file open.** *Setting (trade-off):* add
   `"languages": { "JSONC": { "prettier": { "allowed": false } } }` to `fyx.json`
   (`default.json:2539-2543` allows it). That covers Fyx's own settings and keymap files, and
   `formatter: auto` then falls back to json-language-server for them. Leave other languages
   alone, where prettier is the formatter users expect.
7. **Agent registry at startup** when any `type: "registry"` agent is configured
   (`agent_server_store.rs:318-322`). This is acceptable (user-configured) and refreshes are
   throttled to 1 h. No change needed. Documented here because W1's "no startup fetch" applies
   only to an empty `agent_servers`.
8. **Extension `fetch`/`fetch_stream` with no capability gate** (`since_v0_8_0.rs:633-667`). Today
   only extensions the user installed by hand can reach it, because the registry is dead. *Setting
   (partial):* `granted_extension_capabilities` does not cover `fetch`. *Fix (code):* call
   `capability_granter.grant_download_file(&url)` (or a new `http_fetch` capability) in `fetch` and
   `fetch_stream`. This is worth raising upstream.
9. **Startup extension update check** (`extension_host.rs:468`). Dead (loopback), but attempted
   every launch, and live the moment a user sets `server_url` back to zed.dev. No change needed.
   Note it if Fyx gets its own registry.

Loopback only, no action needed: LM Studio, llama.cpp and Ollama probes. To be tidy, gate the
first two on `disable_ai` (`lmstudio.rs:228`, `llama_cpp.rs:519`).

## Side notes

- `crates/remote_server` does not layer `fyx.json`. That is harmless today, because it downloads
  Zed's remote server, which is dead.
- `GITHUB_TOKEN` from the environment is sent to api.github.com on every LSP/DAP release lookup
  (`github_releases/src/releases.rs:144`) and by the GitHub avatar fallback. This is upstream
  behaviour.
- The REPL WebSocket still sends `User-Agent: Zed/repl` (`remote_kernels.rs:163-169`).
- *Suggested W1 re-measurement:* open a folder containing `.rs`, `.json` and `.md` files, with
  `ANTHROPIC_API_KEY` set, and record the connections. That run should show the class (a)/(c)
  rows above (api.anthropic.com, api.github.com, registry.npmjs.org, nodejs.org when Node is not
  on PATH).

**Browser-only links** (`cx.open_url`, not requests by Fyx): zed.dev docs/account/blog,
status.zed.dev, github.com (issues, permalinks, PR creation, Copilot device flow),
auth.openai.com, auth.x.ai, opencode.ai/auth, provider dashboards, agentclientprotocol.com,
twitter.com.
