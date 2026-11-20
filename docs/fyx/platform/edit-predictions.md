# Local and bring-your-own edit predictions (plan 6.5)

Status: design and first code (W5, 2026-11-20). Plan row: [plan.md P6](plan.md#p6-product-basics).

Fyx turns Zed's cloud off (`server_url` points to `http://127.0.0.1:0`, `assets/settings/fyx.json`),
and so sets `edit_predictions.provider` to `"none"`: Zed's own provider ("Zed AI", Zeta) needs a
zed.dev account. 6.5 asks which other providers upstream still ships, whether they work with the
cloud off, and what an OpenAI-compatible local endpoint (llama.cpp, Ollama) still needs.

## 1. What upstream ships (checked in this tree)

| Provider (`edit_predictions.provider`) | Where it sends requests | Account | Works cloud-off |
|---|---|---|---|
| `"zed"` (Zeta) | zed.dev LLM API (`build_zed_llm_url`, LLM token) | zed.dev | No |
| `"copilot"` | GitHub, through the Copilot language server | GitHub Copilot | Yes |
| `"codestral"` | `edit_predictions.codestral.api_url` (Mistral) | Mistral API key | Yes |
| `"mercury"` | `api.inceptionlabs.ai` (and its feedback endpoint) | Inception API key | Yes |
| `"ollama"` | `edit_predictions.ollama.api_url` + `/api/generate` | none | Yes, local |
| `"open_ai_compatible_api"` | `edit_predictions.open_ai_compatible_api.api_url` as written (e.g. `/v1/completions`), optional bearer key from the credential store | none or a key | Yes, local or remote |

The local providers already run without zed.dev: `fim::request_prediction` posts to the configured
URL only; accepted and rejected predictions are reported to zed.dev only for `"zed"` and to
Inception only for `"mercury"` (`EditPredictionStore::accept_current_prediction`,
`reject_prediction`); the LLM token is never refreshed for them; telemetry is off in `fyx.json`.
One exception: a local model with the `zeta` prompt format runs through `zeta.rs`, which queues a
"settled prediction" report for zed.dev (`send_settled_batches`); with Fyx's `server_url` that
request fails on loopback port 0 before anything is sent. The FIM formats (Qwen, StarCoder, ...)
queue nothing. Upstream has tests for both local paths against a fake server
(`edit_prediction_tests.rs`, `ollama.rs`). So 6.5 needs no new provider.

The prompt format is `prompt_format` (`infer`, `code_llama`, `star_coder`, `deepseek_coder`,
`qwen`, `code_gemma`, `codestral`, `glm`, `zeta`, `sweep`); `infer` reads it from `model`.

## 2. Gaps for a local llama.cpp server

1. **Format inference fails for llama.cpp model names.** `fim::infer_prompt_format` matches the
   model's base name exactly (`qwen2.5-coder`, `starcoder2`, ...). `llama-server` reports the GGUF
   file name, e.g. `qwen2.5-coder-1.5b-instruct-q8_0.gguf`, and users copy that into `model`. The
   inference returns `None` and the registry silently sets no provider (`// todo: notify user`):
   predictions just never appear.
2. **Nothing says why.** The same silent `None` happens for an empty `model` or `api_url`.
3. **"Zed AI" is offered in the provider menu** although it cannot work cloud-off: choosing it
   leads to a sign-in that goes nowhere.
4. **No how-to.** The user guide only says predictions are off.

## 3. Changes

- **Inference by family** (`crates/edit_prediction/src/fyx_prompt_format.rs`, one `fyx:` line in
  `fim::infer_prompt_format`): when the exact match fails, the model name is normalized (path and
  `.gguf` dropped, lower case) and matched by family prefix: `qwen2.5-coder*`, `qwen*coder*` →
  Qwen, `deepseek-coder*` → DeepSeek, `starcoder*` → StarCoder, `codellama*` → CodeLlama,
  `codegemma*` → CodeGemma, `codestral*` → Codestral, `glm-4*` → GLM. Exact upstream names keep
  their upstream result. Families are chosen by FIM tokens, so an instruct or quantized variant
  of the same family uses the same format.
- **A warning when no format is found** (one `fyx:` line in `edit_prediction_registry.rs`): the
  log says which model and to set `prompt_format`. (A notification in the status bar needs the
  button's UI and is left to the owner of AI settings.)
- **No "Zed AI" when the cloud is off** (`crates/edit_prediction_ui/src/fyx_providers.rs`, one
  `fyx:` line in `get_available_providers`): the provider list leaves out `"zed"` while
  `server_url` is Fyx's loopback port 0. Setting `"server_url": "https://zed.dev"` brings it back.
- **User guide**: "Edit predictions without zed.dev": a local `llama-server` (FIM model, the
  settings block, `prompt_format`), Ollama, Copilot, Codestral, Mercury, and what each sends where.

No Cargo change, no WIT change, no new setting.

```jsonc
// settings.json: a local llama.cpp server (llama-server -m <model>.gguf --port 8080)
"edit_predictions": {
  "provider": "open_ai_compatible_api",
  "open_ai_compatible_api": {
    "api_url": "http://127.0.0.1:8080/v1/completions",
    "model": "qwen2.5-coder-1.5b",
    "prompt_format": "infer",
    "max_output_tokens": 64
  }
}
```

## 4. Tests

- Unit: family inference for llama.cpp and Ollama names (with tags, paths, `.gguf`, upper case),
  exact names unchanged, unknown names still `None`.
- Unit: the provider list without and with the cloud off.
- Upstream's fake-server tests stay green (`edit_prediction`, `zed` registry tests).
- Done-when check (needs a build of the app, so W0 or the owner): with the network off, start
  `llama-server -m qwen2.5-coder-1.5b-*.gguf --port 8080`, apply the settings above, and type in a
  Rust file: a ghost suggestion appears and Tab accepts it. No model ships with Fyx; the user
  downloads one.

## 5. Owners

| Owner | Work |
|---|---|
| W5 (asked by the coordinator) | This design, the three `fyx:` hooks and their Fyx files, tests, user guide |
| W0 | Builds the app and runs the done-when check with a local `llama-server` |
| Owner of AI settings (later) | A status-bar notice when no prompt format is found |
