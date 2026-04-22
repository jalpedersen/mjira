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
    verbose: bool,
    very_verbose: bool,
    api_version: u8,
}

impl JiraClient {
    pub fn new(instance: &Instance, verbose: bool, very_verbose: bool) -> Result<Self> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self {
            http,
            api_base: instance.api_base(),
            base_url: instance.url.trim_end_matches('/').to_string(),
            auth: instance.auth_header()?,
            verbose,
            very_verbose,
            api_version: instance.api_version,
        })
    }

    fn log_request(&self, method: &str, url: &str, body: Option<&Value>) {
        if self.verbose {
            eprintln!("[verbose] {} {}", method, url);
            if let Some(b) = body {
                eprintln!("[verbose] body: {}", b);
            }
        }
    }

    pub fn search_path(&self) -> &'static str {
        if self.api_version >= 3 {
            "search/jql"
        } else {
            "search"
        }
    }

    /// In API v3, `GET /project` is deprecated and returns nothing; the correct
    /// endpoint is `GET /project/search` (paginated, `{"values": [...]}`).
    pub fn project_path(&self) -> &'static str {
        if self.api_version >= 3 {
            "project/search"
        } else {
            "project"
        }
    }

    pub fn api_version(&self) -> u8 {
        self.api_version
    }

    pub fn browse_url(&self, key: &str) -> String {
        format!("{}/browse/{}", self.base_url, key)
    }

    pub fn agile_url(&self, path: &str) -> String {
        format!(
            "{}/rest/agile/1.0/{}",
            self.base_url,
            path.trim_start_matches('/')
        )
    }

    pub async fn greenhopper_get<T: DeserializeOwned>(
        &self,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<T> {
        let url = format!(
            "{}/rest/greenhopper/1.0/{}",
            self.base_url,
            path.trim_start_matches('/')
        );
        if self.verbose {
            let query = params.iter().map(|(k, v)| format!("{}={}", k, v)).collect::<Vec<_>>().join("&");
            eprintln!("[verbose] GET {}?{}", url, query);
        }
        let resp = self
            .http
            .get(url)
            .header("Authorization", &self.auth)
            .header("Accept", "application/json")
            .query(params)
            .send()
            .await?;
        self.parse(resp).await
    }

    pub async fn agile_get_with_params<T: DeserializeOwned>(
        &self,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<T> {
        let url = self.agile_url(path);
        if self.verbose {
            let query = params
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join("&");
            eprintln!("[verbose] GET {}?{}", url, query);
        }
        let resp = self
            .http
            .get(url)
            .header("Authorization", &self.auth)
            .header("Accept", "application/json")
            .query(params)
            .send()
            .await?;
        self.parse(resp).await
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.api_base, path.trim_start_matches('/'))
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = self.url(path);
        self.log_request("GET", &url, None);
        let resp = self
            .http
            .get(url)
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
        let url = self.url(path);
        if self.verbose {
            let query = params
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join("&");
            eprintln!("[verbose] GET {}?{}", url, query);
        }
        let resp = self
            .http
            .get(url)
            .header("Authorization", &self.auth)
            .header("Accept", "application/json")
            .query(params)
            .send()
            .await?;
        self.parse(resp).await
    }

    pub async fn post<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T> {
        let url = self.url(path);
        self.log_request("POST", &url, Some(body));
        let resp = self
            .http
            .post(url)
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
        let url = self.url(path);
        self.log_request("POST", &url, Some(body));
        let resp = self
            .http
            .post(url)
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
        let url = self.url(path);
        self.log_request("PUT", &url, Some(body));
        let resp = self
            .http
            .put(url)
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

    pub async fn get_bytes_url(&self, url: &str) -> Result<Vec<u8>> {
        self.log_request("GET", url, None);
        let resp = self
            .http
            .get(url)
            .header("Authorization", &self.auth)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            bail!("Failed to download: HTTP {}", status);
        }
        Ok(resp.bytes().await?.to_vec())
    }

    async fn parse<T: DeserializeOwned>(&self, resp: Response) -> Result<T> {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("{}", jira_error_message(status.as_u16(), &text));
        }
        if self.verbose {
            if self.very_verbose {
                eprintln!("[verbose] response ({}): {}", status.as_u16(), text);
            } else {
                let preview: String = text.chars().take(2048).collect();
                eprintln!("[verbose] response ({}): {}", status.as_u16(), preview);
            }
        }
        serde_json::from_str(&text)
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
    format!(
        "Jira API error {}: {}",
        status,
        body.chars().take(200).collect::<String>()
    )
}
