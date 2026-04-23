use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SnowInstance {
    /// Base URL, e.g. https://mycompany.service-now.com
    pub url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    /// Bearer token (alternative to password)
    pub api_key: Option<String>,
    /// Raw cookie string for SSO-protected instances (copy from browser DevTools)
    /// e.g. "JSESSIONID=abc123; glide_session_store=xyz789"
    pub cookie: Option<String>,
    /// CSRF token required by some instances alongside cookie auth (run `g_ck` in browser console)
    pub x_user_token: Option<String>,
    /// Table used for time entries (default: task_time_worked)
    pub time_table: Option<String>,
}

pub enum Auth {
    Header { name: &'static str, value: String },
    Cookie { cookie: String, x_user_token: Option<String> },
}

impl SnowInstance {
    pub fn auth(&self) -> Result<Auth> {
        if let Some(cookie) = &self.cookie {
            return Ok(Auth::Cookie {
                cookie: cookie.clone(),
                x_user_token: self.x_user_token.clone(),
            });
        }
        if let Some(key) = &self.api_key {
            return Ok(Auth::Header { name: "Authorization", value: format!("Bearer {}", key) });
        }
        if let Some(pass) = &self.password {
            let username = self.username.as_deref().unwrap_or("");
            let credentials = format!("{}:{}", username, pass);
            return Ok(Auth::Header {
                name: "Authorization",
                value: format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode(credentials)
                ),
            });
        }
        bail!("Instance has no authentication configured (cookie, api_key, or password required)");
    }
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct SnowConfig {
    pub default_instance: Option<String>,
    #[serde(default)]
    pub instances: HashMap<String, SnowInstance>,
}

impl SnowConfig {
    pub fn load() -> Result<Self> {
        let path = config_path();
        if !path.exists() {
            return Ok(SnowConfig::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config from {}", path.display()))?;
        toml::from_str(&content).context("Failed to parse ServiceNow config file")
    }

    pub fn get_instance<'a>(&'a self, name: Option<&'a str>) -> Result<(&'a str, &'a SnowInstance)> {
        let name = match name {
            Some(n) => n,
            None => self
                .default_instance
                .as_deref()
                .context("No --instance specified and no default_instance set in snow config")?,
        };
        let instance = self
            .instances
            .get(name)
            .with_context(|| format!("Instance '{}' not found in snow config", name))?;
        Ok((name, instance))
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("makrel")
        .join("snow.toml")
}
