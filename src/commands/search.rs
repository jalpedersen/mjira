use anyhow::Result;
use colored::Colorize;
use serde_json::Value;

use crate::client::JiraClient;
use super::{display_name, field_name, short_date, truncate};
use super::fields::{self, ResolvedCol, STATIC_COLS};

const DEFAULT_COLUMNS: &[&str] = &["key", "type", "status", "assignee", "updated", "summary"];

pub async fn run_search(
    client: &JiraClient,
    jql: &str,
    limit: u32,
    columns: Option<String>,
) -> Result<()> {
    let col_names: Vec<&str> = columns
        .as_deref()
        .map(|s| s.split(',').map(str::trim).filter(|c| !c.is_empty()).collect())
        .unwrap_or_else(|| DEFAULT_COLUMNS.to_vec());

    let active_cols: Vec<ResolvedCol> =
        fields::resolve_columns(&col_names, client, STATIC_COLS).await?;

    let fields_str = {
        let mut seen = std::collections::HashSet::new();
        let mut parts: Vec<&str> = Vec::new();
        for col in &active_cols {
            let id = col.api_id.as_str();
            if id == "key" { continue; }
            if seen.insert(id) {
                parts.push(id);
            }
        }
        parts.join(",")
    };

    let max = limit.to_string();
    let params = [
        ("jql", jql),
        ("maxResults", max.as_str()),
        ("fields", fields_str.as_str()),
    ];
    let result: Value = client.get_with_params(client.search_path(), &params).await?;
    let issues = result["issues"].as_array().map(|v| v.as_slice()).unwrap_or(&[]);

    if issues.is_empty() {
        println!("No issues found.");
        return Ok(());
    }

    let total = result["total"].as_u64().unwrap_or(0);
    println!(
        "{}",
        format!("Showing {} of {} issues  •  JQL: {}", issues.len(), total, jql).dimmed()
    );
    println!();

    // Header
    let last = active_cols.len().saturating_sub(1);
    for (i, col) in active_cols.iter().enumerate() {
        let w = fields::col_width(col);
        let header = fields::col_header(col);
        if i == last {
            print!("{}", header.as_str().bold());
        } else {
            print!("{:<w$} ", header.as_str().bold(), w = w);
        }
    }
    println!();
    println!("{}", "─".repeat(100));

    // Rows
    for issue in issues {
        let key = issue["key"].as_str().unwrap_or("?");
        let f = &issue["fields"];

        for (i, col) in active_cols.iter().enumerate() {
            let is_last = i == last;
            let w = fields::col_width(col);

            if col.custom {
                let raw = fields::extract_custom_value(&f[col.api_id.as_str()]);
                let v = truncate(&raw, w.saturating_sub(1));
                if is_last { print!("{}", v); } else { print!("{:<w$} ", v, w = w); }
            } else {
                match col.label.as_str() {
                    "key" => {
                        if is_last { print!("{}", key.cyan()); }
                        else { print!("{:<w$} ", key.cyan(), w = w); }
                    }
                    "status" => {
                        let v = truncate(field_name(f, "status"), w.saturating_sub(1));
                        if is_last { print!("{}", fields::status_colored(&v)); }
                        else { print!("{:<w$} ", fields::status_colored(&v), w = w); }
                    }
                    _ => {
                        let v = cell_value(f, col, w);
                        if is_last { print!("{}", v); } else { print!("{:<w$} ", v, w = w); }
                    }
                }
            }
        }
        println!();
    }

    Ok(())
}

fn cell_value(f: &Value, col: &ResolvedCol, w: usize) -> String {
    match col.label.as_str() {
        "type"     => truncate(field_name(f, "issuetype"), w.saturating_sub(1)),
        "assignee" => truncate(display_name(f, "assignee"), w.saturating_sub(1)),
        "priority" => truncate(field_name(f, "priority"), w.saturating_sub(1)),
        "updated"  => short_date(f["updated"].as_str().unwrap_or("")).to_string(),
        "summary"  => truncate(f["summary"].as_str().unwrap_or("(no summary)"), w.saturating_sub(1)),
        "project"  => truncate(field_name(f, "project"), w.saturating_sub(1)),
        _          => truncate(field_name(f, col.api_id.as_str()), w.saturating_sub(1)),
    }
}
