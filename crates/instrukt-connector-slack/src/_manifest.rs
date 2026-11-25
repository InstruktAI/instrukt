//! The manifest the company's Slack app is created from (`slack-connector`
//! spec, The Slack app), and the bot scopes it requests.

/// The manifest the company's Slack app is created from, in full; its bot
/// scopes are the ones admission requires.
pub(crate) const MANIFEST: &str = include_str!("app-manifest.yaml");

/// The bot scopes the app's manifest requests: the scopes admission requires
/// the bot token to hold (`slack-connector` spec, Connection).
pub(crate) fn manifest_bot_scopes() -> Result<Vec<String>, serde_yaml_ng::Error> {
    #[derive(serde::Deserialize)]
    struct Manifest {
        oauth_config: OauthConfig,
    }
    #[derive(serde::Deserialize)]
    struct OauthConfig {
        scopes: Scopes,
    }
    #[derive(serde::Deserialize)]
    struct Scopes {
        bot: Vec<String>,
    }
    serde_yaml_ng::from_str::<Manifest>(MANIFEST).map(|manifest| manifest.oauth_config.scopes.bot)
}

#[cfg(test)]
mod tests {
    use super::manifest_bot_scopes;

    #[test]
    fn the_shipped_manifest_names_the_bot_scopes_admission_requires() {
        assert!(!manifest_bot_scopes().unwrap().is_empty());
    }
}
