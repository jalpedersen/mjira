use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Instance {
    /// Base URL of the Jira instance, e.g. https://mycompany.atlassian.net
    pub url: String,
    /// Username or email address
    pub username: String,
    /// API token (Jira Cloud) — takes precedence over password
    pub api_key: Option<String>,
    /// Password (Jira Server / Data Center)
    pub password: Option<String>,
    /// Personal Access Token (Jira Server 8.14+ / Data Center) — bypasses 2FA; takes precedence over password
    pub pat: Option<String>,
    /// REST API version to use (default: 2)
    #[serde(default = "default_api_version")]
    pub api_version: u8,
    /// Local git repository paths to search when running `issue commits`
    #[serde(default)]
    pub repos: Vec<String>,
    /// Map from Jira component name to git repository path.
    /// When an issue has components, the mapped repos are used instead of `repos`.
    #[serde(default)]
    pub component_repos: HashMap<String, String>,
}

fn default_api_version() -> u8 {
    2
}

impl Instance {
    pub fn auth_header(&self) -> Result<String> {
        if let Some(pat) = &self.pat {
            return Ok(format!("Bearer {}", pat));
        }
        let secret = if let Some(key) = &self.api_key {
            key.clone()
        } else if let Some(pass) = &self.password {
            pass.clone()
        } else {
            bail!("Instance has neither api_key, password, nor pat configured");
        };
        let credentials = format!("{}:{}", self.username, secret);
        Ok(format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(credentials)
        ))
    }

    pub fn api_base(&self) -> String {
        format!(
            "{}/rest/api/{}",
            self.url.trim_end_matches('/'),
            self.api_version
        )
    }
}

/// A predefined JQL search with optional defaults for limit and columns.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Query {
    /// JQL query string
    pub jql: String,
    /// Maximum results (overrides the CLI default of 25)
    pub limit: Option<u32>,
    /// Comma-separated columns to display (overrides the default set)
    pub columns: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct Config {
    /// Name of the instance to use when --instance is not specified
    pub default_instance: Option<String>,
    #[serde(default)]
    pub instances: HashMap<String, Instance>,
    /// Named JQL queries, runnable with `jira query <name>`
    #[serde(default)]
    pub queries: HashMap<String, Query>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        if !path.exists() {
            return Ok(Config::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config from {}", path.display()))?;
        toml::from_str(&content).context("Failed to parse config file")
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config dir {}", parent.display()))?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn get_instance<'a>(&'a self, name: Option<&'a str>) -> Result<(&'a str, &'a Instance)> {
        let name = match name {
            Some(n) => n,
            None => self
                .default_instance
                .as_deref()
                .context("No --instance specified and no default_instance set in config")?,
        };
        let instance = self
            .instances
            .get(name)
            .with_context(|| format!("Instance '{}' not found in config", name))?;
        Ok((name, instance))
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("jira-cli")
        .join("config.toml")
}
