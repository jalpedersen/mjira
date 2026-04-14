use anyhow::Result;
use colored::Colorize;
use std::collections::HashMap;

use crate::client::JiraClient;
use crate::config::{config_path, Query};
use super::search;

/// Print all saved queries defined in the config file.
pub fn list(queries: &HashMap<String, Query>) {
    if queries.is_empty() {
        println!("{}", "No saved queries configured.".dimmed());
        println!();
        println!("Add queries to {}:", config_path().display());
        println!();
        println!("  [queries.my-work]");
        println!("  jql = \"assignee = currentUser() ORDER BY updated DESC\"");
        println!();
        println!("  [queries.release]");
        println!("  jql     = \"fixVersions = '2.0.0' AND status != Done\"");
        println!("  limit   = 50");
        println!("  columns = \"key,type,status,assignee,priority,summary\"");
        return;
    }

    let mut names: Vec<&str> = queries.keys().map(String::as_str).collect();
    names.sort_unstable();

    println!("Saved queries  (run with: jira query <name>)");
    println!();
    for name in names {
        let q = &queries[name];
        println!("  {}", name.bold());
        println!("    {}", q.jql.dimmed());
        let mut opts: Vec<String> = Vec::new();
        if let Some(l) = q.limit    { opts.push(format!("limit: {l}")); }
        if let Some(c) = &q.columns { opts.push(format!("columns: {c}")); }
        if !opts.is_empty() {
            println!("    {}", opts.join("  •  ").dimmed());
        }
        println!();
    }
}

/// Run a saved query by name, merging any CLI overrides on top of the stored defaults.
pub async fn run(
    client: &JiraClient,
    queries: &HashMap<String, Query>,
    name: &str,
    limit_override: Option<u32>,
    columns_override: Option<String>,
) -> Result<()> {
    let query = queries.get(name).ok_or_else(|| {
        let mut known: Vec<&str> = queries.keys().map(String::as_str).collect();
        known.sort_unstable();
        if known.is_empty() {
            anyhow::anyhow!(
                "No saved queries found. Run `jira query` to see setup instructions."
            )
        } else {
            anyhow::anyhow!(
                "No saved query named '{}'. Available: {}",
                name,
                known.join(", ")
            )
        }
    })?;

    // CLI overrides take precedence over saved defaults.
    let limit   = limit_override.unwrap_or(query.limit.unwrap_or(25));
    let columns = columns_override.or_else(|| query.columns.clone());

    search::run_search(client, &query.jql, limit, columns).await
}
