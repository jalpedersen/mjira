use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::client::JiraClient;
use crate::config::Config;

const CACHE_TTL: Duration = Duration::from_secs(300);

pub async fn issues(client: &JiraClient, instance_name: &str) -> Result<()> {
    let path = cache_path(instance_name);
    if let Some(cached) = read_cache(&path) {
        print!("{}", cached);
        return Ok(());
    }
    let data: Value = client
        .get_with_params(
            client.search_path(),
            &[
                ("jql", "updated >= -90d ORDER BY updated DESC"),
                ("fields", "summary"),
                ("maxResults", "200"),
            ],
        )
        .await?;
    let mut out = String::new();
    if let Some(issues) = data["issues"].as_array() {
        for issue in issues {
            let key = issue["key"].as_str().unwrap_or("");
            let summary = issue["fields"]["summary"]
                .as_str()
                .unwrap_or("")
                .replace('\t', " ");
            if !key.is_empty() {
                out.push_str(&format!("{}\t{}\n", key, summary));
            }
        }
    }
    write_cache(&path, &out);
    print!("{}", out);
    Ok(())
}

pub fn instances(cfg: &Config) {
    let mut names: Vec<&str> = cfg.instances.keys().map(|s| s.as_str()).collect();
    names.sort();
    for name in names {
        println!("{}", name);
    }
}

fn cache_path(instance_name: &str) -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("makrel")
        .join(instance_name)
        .join("issues")
}

fn read_cache(path: &PathBuf) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let age = SystemTime::now()
        .duration_since(meta.modified().ok()?)
        .ok()?;
    if age > CACHE_TTL {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn write_cache(path: &PathBuf, content: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, content);
}
