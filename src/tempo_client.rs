use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::de::DeserializeOwned;

use crate::config::Instance;

pub struct TempoClient {
    http: Client,
    base_url: String,
    auth: String,
    verbose: bool,
    very_verbose: bool,
    pub is_cloud: bool,
}

impl TempoClient {
    pub fn new(instance: &Instance, verbose: bool, very_verbose: bool) -> Result<Self> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        let is_cloud = instance.api_version >= 3;
        let (base_url, auth) = if is_cloud {
            let token = instance
                .tempo_token
                .as_deref()
                .context("Tempo Cloud requires 'tempo_token' in the instance config (generate a Tempo API token in your Tempo settings)")?;
            (
                "https://api.tempo.io/4".to_string(),
                format!("Bearer {}", token),
            )
        } else {
            (
                format!(
                    "{}/rest/tempo-timesheets/4",
                    instance.url.trim_end_matches('/')
                ),
                instance.auth_header()?,
            )
        };

        Ok(Self {
            http,
            base_url,
            auth,
            verbose,
            very_verbose,
            is_cloud,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    pub async fn get_with_params<T: DeserializeOwned>(
        &self,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<T> {
        let url = self.url(path);
        if self.verbose {
            let query = params
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join("&");
            if query.is_empty() {
                eprintln!("[verbose] GET {}", url);
            } else {
                eprintln!("[verbose] GET {}?{}", url, query);
            }
        }
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth)
            .header("Accept", "application/json")
            .query(params)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!(
                "Tempo API error {}: {}",
                status.as_u16(),
                text.chars().take(400).collect::<String>()
            );
        }
        if self.verbose && self.very_verbose {
            eprintln!("[verbose] response ({}): {}", status.as_u16(), text);
        }
        serde_json::from_str(&text).context("Failed to parse Tempo response as JSON")
    }
}
