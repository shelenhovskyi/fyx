# Extension settings (plan 6.1, design wit-0.4 §D): host glue for W1 and W3

W2c's runtime side is on `claude/fyx-w2c-host-0-2-cbde2c`:
- the model `ui_ext::settings`;
- `UiManifest.settings` and `UiManifest::settings_specs`;
- `LoadOptions.settings` and `user_settings`;
- on `UiExtension`: `setting_specs`, `effective_settings`, `set_user_settings`.

What is left for the host:

## W1: the settings key (`settings_content`)

Add a user-level key that holds each extension's raw object, unchecked:

```rust
// settings_content/src/fyx.rs (or beside FyxAccountsSettingsContent), in UserSettingsContent
// or SettingsContent, with a fyx: comment where it touches upstream.
/// fyx: extension settings (plan 6.1): `fyx_extensions.<extension id>` = the extension's object.
pub fyx_extensions: Option<BTreeMap<String, serde_json::Value>>,
```

- **User level only** (§D.2). A project `.zed/settings.json` must not set it. If the key goes
  into the shared `SettingsContent`, the reader below takes the **user** file's value only.
  Never merge it with project values.
- The JSON schema may describe it as `object` with free-form values. The checks are the
  extension's declarations, applied by the runtime.

## W3: reading and pushing (`ui_ext_host`)

1. **At load,** in the scan and dev-path loads, beside `diagnostics_source`:
   `LoadOptions { user_settings: user_object(cx, &id), .. }`.
   `user_object` reads `SettingsStore::raw_user_settings()` → `fyx_extensions` → `get(id)` and
   converts it with `serde_json::from_value::<ui_ext::settings::RawValue>(value.clone()).ok()`.
2. **On every settings change,** observe `SettingsStore`. Then, for each loaded extension,
   `extension.set_user_settings(user_object(cx, &id).as_ref())`. A non-empty result is the
   `settings-changed` list. The runtime queues it for delivery once W0 stages the WIT, so the
   host does nothing more with it.
3. **The Settings tab (§D.3):**
   - Draw it from `extension.setting_specs()` (types, bounds, choices, titles).
   - Take values, errors and greyed-out unknown keys from `extension.effective_settings()`
     (`settings`, `errors` by key, `unknown`). An error under the empty key means the object
     is not a JSON object.
   - Write through the settings store as the settings UI does:
     `update_settings_file(..., |content| content.fyx_extensions[id][key] = value)`.
   - "Reset" removes the key. "Reset all" removes the extension's object.
4. **`settings.open`** waits for the WIT (W0). The runtime will check the event call and ask
   the host to show the tab.

## W0: WIT

Use the `settings` interface and `io-event::settings-changed(list<string>)` from design §D.4,
with `settings` in the V0_4 world lists. Then W2c links `get`/`all`/`open` in
`fyx_ui_v0_4.rs` over `SettingsHandle` and delivers the queued lists.
