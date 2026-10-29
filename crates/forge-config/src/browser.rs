//! `[browser]`: attach the `browser` tool to an already-running, logged-in Chromium-family browser
//! instead of launching Forge's own. See docs/features/browser-attach.md.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserConfig {
    /// DevTools HTTP endpoint of a browser started with `--remote-debugging-port`, e.g.
    /// `http://127.0.0.1:9222`. Empty = launch Forge's own browser. `FORGE_BROWSER_CDP` overrides.
    pub attach: Option<String>,
    /// Alias of `attach`, for people who think of it as "the CDP URL".
    pub cdp_url: Option<String>,
    /// Let the model run arbitrary JavaScript in the page (`eval`, `replay`). Off by default.
    pub allow_eval: bool,
    /// Let the model read the browser's cookies (`cookies`). Off by default.
    pub allow_cookie_access: bool,
}

impl BrowserConfig {
    /// The configured endpoint, `attach` first.
    pub fn endpoint(&self) -> Option<&str> {
        [self.attach.as_deref(), self.cdp_url.as_deref()]
            .into_iter()
            .flatten()
            .map(str::trim)
            .find(|url| !url.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attach_wins_over_cdp_url_and_blank_means_unset() {
        assert_eq!(BrowserConfig::default().endpoint(), None);
        let both = BrowserConfig {
            attach: Some("http://a:1".into()),
            cdp_url: Some("http://b:2".into()),
            ..BrowserConfig::default()
        };
        assert_eq!(both.endpoint(), Some("http://a:1"));
        let alias = BrowserConfig {
            attach: Some("  ".into()),
            cdp_url: Some("http://b:2".into()),
            ..BrowserConfig::default()
        };
        assert_eq!(alias.endpoint(), Some("http://b:2"));
        let parsed: BrowserConfig = toml::from_str("cdp_url = \"9222\"").unwrap();
        assert_eq!(parsed.endpoint(), Some("9222"));
    }

    #[test]
    fn eval_and_cookie_access_default_off() {
        let config = BrowserConfig::default();
        assert!(!config.allow_eval && !config.allow_cookie_access);
        let parsed: BrowserConfig =
            toml::from_str("allow_eval = true\nallow_cookie_access = true").unwrap();
        assert!(parsed.allow_eval && parsed.allow_cookie_access);
    }
}
