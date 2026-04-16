use anyhow::{bail, Context, Result};
use reqwest::{Client, Response};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::config::Instance;

pub struct JiraClient {
    http: Client,
    api_base: String,
    pub base_url: String,
    auth: String,
}

impl JiraClient {
    pub fn new(instance: &Instance) -> Result<Self> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self {
            http,
            api_base: instance.api_base(),
            base_url: instance.url.trim_end_matches('/').to_string(),
            auth: instance.auth_header()?,
        })
    }

    pub fn browse_url(&self, key: &str) -> String {
        format!("{}/browse/{}", self.base_url, key)
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.api_base, path.trim_start_matches('/'))
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self
            .http
            .get(self.url(path))
            .header("Authorization", &self.auth)
            .header("Accept", "application/json")
            .send()
            .await?;
        self.parse(resp).await
    }

    pub async fn get_with_params<T: DeserializeOwned>(
        &self,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<T> {
        let resp = self
            .http
            .get(self.url(path))
            .header("Authorization", &self.auth)
            .header("Accept", "application/json")
            .query(params)
            .send()
            .await?;
        self.parse(resp).await
    }

    pub async fn post<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T> {
        let resp = self
            .http
            .post(self.url(path))
            .header("Authorization", &self.auth)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(body)
            .send()
            .await?;
        self.parse(resp).await
    }

    /// POST that expects no response body (e.g. transitions, comments that return 204)
    pub async fn post_no_body(&self, path: &str, body: &Value) -> Result<()> {
        let resp = self
            .http
            .post(self.url(path))
            .header("Authorization", &self.auth)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!("{}", jira_error_message(status.as_u16(), &text));
        }
        Ok(())
    }

    pub async fn put_no_body(&self, path: &str, body: &Value) -> Result<()> {
        let resp = self
            .http
            .put(self.url(path))
            .header("Authorization", &self.auth)
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!("{}", jira_error_message(status.as_u16(), &text));
        }
        Ok(())
    }

    async fn parse<T: DeserializeOwned>(&self, resp: Response) -> Result<T> {
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("{}", jira_error_message(status.as_u16(), &text));
        }
        resp.json::<T>()
            .await
            .context("Failed to parse Jira response as JSON")
    }
}

fn jira_error_message(status: u16, body: &str) -> String {
    if let Ok(json) = serde_json::from_str::<Value>(body) {
        // Extract errorMessages array
        if let Some(msgs) = json.get("errorMessages").and_then(|v| v.as_array()) {
            let text: Vec<&str> = msgs.iter().filter_map(|m| m.as_str()).collect();
            if !text.is_empty() {
                return format!("Jira error ({}): {}", status, text.join("; "));
            }
        }
        // Extract errors object
        if let Some(errors) = json.get("errors").and_then(|v| v.as_object()) {
            let parts: Vec<String> = errors
                .iter()
                .map(|(k, v)| format!("{}: {}", k, v.as_str().unwrap_or(&v.to_string())))
                .collect();
            if !parts.is_empty() {
                return format!("Jira error ({}): {}", status, parts.join("; "));
            }
        }
        // Fallback: message field
        if let Some(msg) = json.get("message").and_then(|v| v.as_str()) {
            return format!("Jira error ({}): {}", status, msg);
        }
    }
    format!("Jira API error {}: {}", status, body.chars().take(200).collect::<String>())
}
