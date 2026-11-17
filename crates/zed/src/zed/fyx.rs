//! Fyx-only startup glue. Upstream files call into this module with one `fyx:` line each, so the
//! fork's logic lives here instead of inside upstream functions.

use db::kvp::KeyValueStore;
use gpui::{App, UpdateGlobal as _};
use release_channel::ReleaseChannel;
use settings::SettingsStore;
use util::ResultExt as _;

/// Key-value entries that Zed's onboarding writes once the user has seen it. Fyx sets them up front:
/// both views are built around a zed.dev account (sign-in, plans, hosted models).
const SKIPPED_ONBOARDING: &[(&str, &str)] = &[
    // First-run onboarding page (`onboarding::FIRST_OPEN`); Fyx opens the welcome page instead.
    (onboarding::FIRST_OPEN, "false"),
    // The agent panel's new-user card (`OnboardingUpsell` in `agent_ui`, a private type).
    ("dismissed-trial-upsell", "1"),
];

/// Call right after `settings::init`, before any window opens.
pub fn init(cx: &mut App) {
    let channel = ReleaseChannel::global(cx);
    // The Windows AppUserModelID (taskbar grouping, toasts) is `dev.fyx.Fyx…`, not one derived from
    // the executable's path.
    cx.set_app_identity(channel.app_id(), channel.display_name());
    layer_default_settings(cx);
    skip_zed_onboarding(cx);
}

/// `fyx://` is the public URL scheme; `zed://` stays the internal link namespace (mentions, schemas,
/// settings links), so incoming links are rewritten at the entry points.
pub fn url_to_internal(url: String) -> String {
    match url.strip_prefix("fyx://") {
        Some(rest) => format!("zed://{rest}"),
        None => url,
    }
}

/// Rewrites `fyx://` command-line arguments, for the first instance and for the ones forwarded to a
/// running instance.
pub fn rewrite_url_args(args: &mut [String]) {
    for arg in args {
        *arg = url_to_internal(std::mem::take(arg));
    }
}

/// Layers Fyx's defaults (`assets/settings/fyx.json`) over Zed's `default.json`. Only the app does
/// this, so `default.json` stays upstream's and upstream tests keep Zed's defaults.
fn layer_default_settings(cx: &mut App) {
    let parse = |path: &str| {
        let content = util::asset_str::<settings::SettingsAssets>(path);
        settings::parse_json_with_comments::<serde_json::Value>(&content)
            .unwrap_or_else(|error| panic!("failed to parse {path}: {error}"))
    };
    let mut defaults = parse("settings/default.json");
    util::merge_non_null_json_value_into(parse("settings/fyx.json"), &mut defaults);
    SettingsStore::update_global(cx, |store, cx| {
        store
            .set_default_settings(&defaults.to_string(), cx)
            .expect("Fyx default settings are invalid");
    });
}

fn skip_zed_onboarding(cx: &mut App) {
    let kvp = KeyValueStore::global(cx);
    for (key, value) in SKIPPED_ONBOARDING {
        if matches!(kvp.read_kvp(key), Ok(None)) {
            // Synchronous, so the startup code that reads these keys sees them.
            cx.foreground_executor()
                .block_on(kvp.write_kvp(key.to_string(), value.to_string()))
                .log_err();
        }
    }
}
