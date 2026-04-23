use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::de::DeserializeOwned;

use crate::snow_config::{Auth, SnowInstance};

pub struct SnowClient {
    http: Client,
    base_url: String,
    auth: Auth,
    verbose: bool,
    very_verbose: bool,
}

impl SnowClient {
    pub fn new(instance: &SnowInstance, verbose: bool, very_verbose: bool) -> Result<Self> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self {
            http,
            base_url: format!("{}/api/now", instance.url.trim_end_matches('/')),
            auth: instance.auth()?,
            verbose,
            very_verbose,
        })
    }

    pub async fn get_table<T: DeserializeOwned>(
        &self,
        table: &str,
        params: &[(&str, &str)],
    ) -> Result<T> {
        let url = format!("{}/table/{}", self.base_url, table);
        if self.verbose {
            let query = params
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join("&");
            eprintln!("[verbose] GET {}?{}", url, query);
            match &self.auth {
                Auth::Header { name, .. } => eprintln!("[verbose] auth: {} header", name),
                Auth::Cookie { x_user_token, .. } => eprintln!(
                    "[verbose] auth: Cookie header{}",
                    if x_user_token.is_some() { " + X-UserToken" } else { "" }
                ),
            }
        }
        let req = self.http.get(&url).header("Accept", "application/json");
        let req = match &self.auth {
            Auth::Header { name, value } => req.header(*name, value),
            Auth::Cookie { cookie, x_user_token } => {
                let req = req.header("Cookie", cookie);
                match x_user_token {
                    Some(token) => req.header("X-UserToken", token),
                    None => req,
                }
            }
        };
        let resp = req.query(params).send().await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!(
                "ServiceNow API error {}: {}",
                status.as_u16(),
                text.chars().take(400).collect::<String>()
            );
        }
        if self.verbose && self.very_verbose {
            eprintln!("[verbose] response ({}): {}", status.as_u16(), text);
        }
        serde_json::from_str(&text).context("Failed to parse ServiceNow response as JSON")
    }
}
