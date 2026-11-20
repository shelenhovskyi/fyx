//! Fyx (plan 6.5, `docs/fyx/platform/edit-predictions.md`): the provider menu leaves out
//! "Zed AI" while Zed's cloud is off. Fyx's `server_url` is loopback port 0, where every zed.dev
//! request fails before it is sent, so Zed's provider could only lead to a sign-in that goes
//! nowhere. With `"server_url": "https://zed.dev"` it is offered again.

use client::ClientSettings;
use gpui::App;
use settings::{EditPredictionProvider, Settings as _};

/// Whether `server_url` is a loopback address on port 0, as Fyx sets it.
pub(crate) fn is_cloud_off(server_url: &str) -> bool {
    let rest = server_url
        .split_once("://")
        .map_or(server_url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let Some((host, port)) = authority.rsplit_once(':') else {
        return false;
    };
    port == "0" && matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

/// Drops the providers that cannot work with the current `server_url`.
pub(crate) fn retain_reachable(providers: &mut Vec<EditPredictionProvider>, cx: &App) {
    if is_cloud_off(&ClientSettings::get_global(cx).server_url) {
        providers.retain(|provider| *provider != EditPredictionProvider::Zed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_port_zero_is_cloud_off() {
        assert!(is_cloud_off("http://127.0.0.1:0"));
        assert!(is_cloud_off("http://localhost:0/"));
        assert!(is_cloud_off("http://[::1]:0"));
        assert!(!is_cloud_off("https://zed.dev"));
        assert!(!is_cloud_off("http://127.0.0.1:8080"));
        assert!(!is_cloud_off("http://example.com:0"));
        assert!(!is_cloud_off(""));
    }
}
