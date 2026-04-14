use anyhow::Result;
use clap::Subcommand;
use colored::Colorize;
use serde_json::Value;

use crate::client::JiraClient;
use super::truncate;

#[derive(Subcommand)]
pub enum ProjectCommands {
    /// List all accessible projects
    List {
        /// Search by project name or key
        #[arg(short, long)]
        query: Option<String>,
    },
}

pub async fn handle(cmd: ProjectCommands, client: &JiraClient) -> Result<()> {
    match cmd {
        ProjectCommands::List { query } => list(client, query).await,
    }
}

async fn list(client: &JiraClient, query: Option<String>) -> Result<()> {
    let projects: Value = client.get("project").await?;
    let projects = match projects.as_array() {
        Some(p) => p,
        None => {
            println!("No projects returned.");
            return Ok(());
        }
    };

    let filtered: Vec<&Value> = if let Some(q) = &query {
        let q_lower = q.to_lowercase();
        projects
            .iter()
            .filter(|p| {
                p["key"]
                    .as_str()
                    .map(|k| k.to_lowercase().contains(&q_lower))
                    .unwrap_or(false)
                    || p["name"]
                        .as_str()
                        .map(|n| n.to_lowercase().contains(&q_lower))
                        .unwrap_or(false)
            })
            .collect()
    } else {
        projects.iter().collect()
    };

    if filtered.is_empty() {
        println!("No projects found.");
        return Ok(());
    }

    println!(
        "{:<12} {:<16} {}",
        "KEY".bold(),
        "TYPE".bold(),
        "NAME".bold()
    );
    println!("{}", "─".repeat(70));

    let mut sorted = filtered.clone();
    sorted.sort_by_key(|p| p["key"].as_str().unwrap_or(""));

    for p in sorted {
        let key = p["key"].as_str().unwrap_or("?");
        let name = p["name"].as_str().unwrap_or("?");
        let ptype = p["projectTypeKey"].as_str().unwrap_or("?");
        println!("{:<12} {:<16} {}", key.cyan(), truncate(ptype, 15), name);
    }

    Ok(())
}
