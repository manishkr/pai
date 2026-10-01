use std::env;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::{Context, Result};

use crate::documents::DocumentAccessRules;

// =============================================================================
// Runtime Configuration
// =============================================================================
//
// The server is intentionally strict about required configuration because the
// happy path needs both state stores and the Anthropic key to work. Failing
// fast keeps missing environment problems obvious during local setup.

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub anthropic_api_key: Option<String>,
    pub anthropic_model: String,
    pub anthropic_mock: bool,
    pub database_url: String,
    pub redis_url: String,
    pub cookie_secret: String,
    pub google_client_id: String,
    pub google_client_secret: String,
    pub google_redirect_uri: String,
    pub google_hosted_domain: String,
    pub sharepoint_graph: Option<SharePointGraphConfig>,
    #[allow(dead_code)]
    pub document_access_rules_raw: String,
    pub document_access_rules: DocumentAccessRules,
    pub bind_address: SocketAddr,
    pub public_origin: String,
}

#[derive(Clone, Debug)]
pub struct SharePointGraphConfig {
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: String,
    pub site_id: String,
    pub drive_id: String,
    pub root_item_id: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|key| env::var(key).ok())
    }

    fn from_lookup<F>(lookup: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let anthropic_mock = lookup("ANTHROPIC_MOCK")
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        let anthropic_api_key = match lookup("ANTHROPIC_API_KEY") {
            Some(value) => Some(value),
            None if anthropic_mock => None,
            None => {
                return Err(anyhow::anyhow!("environment variable not found"))
                    .context("while attempting to read ANTHROPIC_API_KEY");
            }
        };
        let anthropic_model =
            lookup("ANTHROPIC_MODEL").context("while attempting to read ANTHROPIC_MODEL")?;
        let database_url =
            lookup("DATABASE_URL").context("while attempting to read DATABASE_URL")?;
        let redis_url = lookup("REDIS_URL").context("while attempting to read REDIS_URL")?;
        let cookie_secret =
            lookup("COOKIE_SECRET").context("while attempting to read COOKIE_SECRET")?;
        let google_client_id =
            lookup("GOOGLE_CLIENT_ID").context("while attempting to read GOOGLE_CLIENT_ID")?;
        let google_client_secret = lookup("GOOGLE_CLIENT_SECRET")
            .context("while attempting to read GOOGLE_CLIENT_SECRET")?;
        let google_redirect_uri = lookup("GOOGLE_REDIRECT_URI")
            .context("while attempting to read GOOGLE_REDIRECT_URI")?;
        let google_hosted_domain = lookup("GOOGLE_HOSTED_DOMAIN")
            .context("while attempting to read GOOGLE_HOSTED_DOMAIN")?;
        let sharepoint_graph = SharePointGraphConfig::from_lookup(&lookup)?;
        let (document_access_rules_raw, document_access_rules) = if let Some(
            document_access_rules_file,
        ) =
            lookup("DOCUMENT_ACCESS_RULES_FILE")
        {
            let raw = fs::read_to_string(&document_access_rules_file).with_context(|| {
                    format!(
                        "while attempting to read DOCUMENT_ACCESS_RULES_FILE at {document_access_rules_file}"
                    )
                })?;
            let parsed = DocumentAccessRules::from_yaml(&raw)?;
            (raw, parsed)
        } else {
            let raw = lookup("DOCUMENT_ACCESS_RULES")
                .context("while attempting to read DOCUMENT_ACCESS_RULES")?;
            let parsed = DocumentAccessRules::from_json(&raw)?;
            (raw, parsed)
        };

        let bind_host = lookup("BIND_HOST").unwrap_or_else(|| Ipv4Addr::LOCALHOST.to_string());
        let bind_port = lookup("PORT")
            .and_then(|port| port.parse::<u16>().ok())
            .unwrap_or(3000);
        let bind_address = SocketAddr::new(
            bind_host
                .parse::<IpAddr>()
                .context("while attempting to parse BIND_HOST")?,
            bind_port,
        );
        let public_origin =
            lookup("PUBLIC_ORIGIN").unwrap_or_else(|| format!("http://{bind_address}"));

        Ok(Self {
            anthropic_api_key,
            anthropic_model,
            anthropic_mock,
            database_url,
            redis_url,
            cookie_secret,
            google_client_id,
            google_client_secret,
            google_redirect_uri,
            google_hosted_domain,
            sharepoint_graph,
            document_access_rules_raw,
            document_access_rules,
            bind_address,
            public_origin,
        })
    }
}

impl SharePointGraphConfig {
    fn from_lookup<F>(lookup: &F) -> Result<Option<Self>>
    where
        F: Fn(&str) -> Option<String>,
    {
        let tenant_id = lookup("SHAREPOINT_GRAPH_TENANT_ID");
        let client_id = lookup("SHAREPOINT_GRAPH_CLIENT_ID");
        let client_secret = lookup("SHAREPOINT_GRAPH_CLIENT_SECRET");
        let site_id = lookup("SHAREPOINT_GRAPH_SITE_ID");
        let drive_id = lookup("SHAREPOINT_GRAPH_DRIVE_ID");
        let root_item_id = lookup("SHAREPOINT_GRAPH_ROOT_ITEM_ID")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        let any_set = [
            tenant_id.as_ref(),
            client_id.as_ref(),
            client_secret.as_ref(),
            site_id.as_ref(),
            drive_id.as_ref(),
            root_item_id.as_ref(),
        ]
        .iter()
        .any(|value| value.is_some());

        if !any_set {
            return Ok(None);
        }

        Ok(Some(Self {
            tenant_id: tenant_id.context("while attempting to read SHAREPOINT_GRAPH_TENANT_ID")?,
            client_id: client_id.context("while attempting to read SHAREPOINT_GRAPH_CLIENT_ID")?,
            client_secret: client_secret
                .context("while attempting to read SHAREPOINT_GRAPH_CLIENT_SECRET")?,
            site_id: site_id.context("while attempting to read SHAREPOINT_GRAPH_SITE_ID")?,
            drive_id: drive_id.context("while attempting to read SHAREPOINT_GRAPH_DRIVE_ID")?,
            root_item_id,
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::AppConfig;

    #[test]
    fn config_requires_cookie_secret() {
        let vars = HashMap::from([
            ("ANTHROPIC_API_KEY", "key".to_string()),
            ("ANTHROPIC_MODEL", "claude-test".to_string()),
            ("DATABASE_URL", "postgres://localhost/test".to_string()),
            ("REDIS_URL", "redis://localhost".to_string()),
            ("GOOGLE_CLIENT_ID", "client".to_string()),
            ("GOOGLE_CLIENT_SECRET", "secret".to_string()),
            (
                "GOOGLE_REDIRECT_URI",
                "http://localhost/callback".to_string(),
            ),
            ("GOOGLE_HOSTED_DOMAIN", "company.com".to_string()),
            (
                "DOCUMENT_ACCESS_RULES",
                r#"{"default":{"documents":[],"categories":[]}}"#.to_string(),
            ),
        ]);

        let error = AppConfig::from_lookup(|key| vars.get(key).cloned())
            .expect_err("missing COOKIE_SECRET must fail");
        assert!(error.to_string().contains("COOKIE_SECRET"));
    }

    #[test]
    fn config_allows_missing_api_key_in_mock_mode() {
        let vars = HashMap::from([
            ("ANTHROPIC_MODEL", "claude-test".to_string()),
            ("ANTHROPIC_MOCK", "true".to_string()),
            ("DATABASE_URL", "postgres://localhost/test".to_string()),
            ("REDIS_URL", "redis://localhost".to_string()),
            ("COOKIE_SECRET", "dev-secret".to_string()),
            ("GOOGLE_CLIENT_ID", "client".to_string()),
            ("GOOGLE_CLIENT_SECRET", "secret".to_string()),
            (
                "GOOGLE_REDIRECT_URI",
                "http://localhost/callback".to_string(),
            ),
            ("GOOGLE_HOSTED_DOMAIN", "company.com".to_string()),
            (
                "DOCUMENT_ACCESS_RULES",
                r#"{"default":{"documents":[],"categories":[]}}"#.to_string(),
            ),
        ]);

        let config = AppConfig::from_lookup(|key| vars.get(key).cloned())
            .expect("mock mode should allow a missing API key");
        assert!(config.anthropic_mock);
        assert_eq!(config.anthropic_api_key, None);
    }

    #[test]
    fn config_rejects_invalid_document_access_rules() {
        let vars = HashMap::from([
            ("ANTHROPIC_MODEL", "claude-test".to_string()),
            ("ANTHROPIC_MOCK", "true".to_string()),
            ("DATABASE_URL", "postgres://localhost/test".to_string()),
            ("REDIS_URL", "redis://localhost".to_string()),
            ("COOKIE_SECRET", "dev-secret".to_string()),
            ("GOOGLE_CLIENT_ID", "client".to_string()),
            ("GOOGLE_CLIENT_SECRET", "secret".to_string()),
            (
                "GOOGLE_REDIRECT_URI",
                "http://localhost/callback".to_string(),
            ),
            ("GOOGLE_HOSTED_DOMAIN", "company.com".to_string()),
            ("DOCUMENT_ACCESS_RULES", "{".to_string()),
        ]);

        let error = AppConfig::from_lookup(|key| vars.get(key).cloned())
            .expect_err("invalid document rules must fail");
        assert!(error.to_string().contains("DOCUMENT_ACCESS_RULES"));
    }

    #[test]
    fn config_requires_complete_sharepoint_graph_settings_once_started() {
        let vars = HashMap::from([
            ("ANTHROPIC_MODEL", "claude-test".to_string()),
            ("ANTHROPIC_MOCK", "true".to_string()),
            ("DATABASE_URL", "postgres://localhost/test".to_string()),
            ("REDIS_URL", "redis://localhost".to_string()),
            ("COOKIE_SECRET", "dev-secret".to_string()),
            ("GOOGLE_CLIENT_ID", "client".to_string()),
            ("GOOGLE_CLIENT_SECRET", "secret".to_string()),
            (
                "GOOGLE_REDIRECT_URI",
                "http://localhost/callback".to_string(),
            ),
            ("GOOGLE_HOSTED_DOMAIN", "company.com".to_string()),
            (
                "DOCUMENT_ACCESS_RULES",
                r#"{"default":{"documents":[],"categories":[]}}"#.to_string(),
            ),
            ("SHAREPOINT_GRAPH_TENANT_ID", "tenant".to_string()),
        ]);

        let error = AppConfig::from_lookup(|key| vars.get(key).cloned())
            .expect_err("partial SharePoint Graph settings must fail");
        assert!(error.to_string().contains("SHAREPOINT_GRAPH_CLIENT_ID"));
    }
}
