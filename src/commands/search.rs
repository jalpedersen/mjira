use anyhow::{bail, Result};
use colored::Colorize;
use serde_json::Value;

use crate::client::JiraClient;
use super::{display_name, field_name, short_date, truncate};

const AVAILABLE_COLUMNS: &[(&str, &str)] = &[
    ("key",      "Issue key (e.g. PROJ-123)"),
    ("type",     "Issue type (Bug, Story, Task, etc.)"),
    ("status",   "Current status"),
    ("assignee", "Assigned user"),
    ("priority", "Priority level"),
    ("updated",  "Last updated date"),
    ("summary",  "Issue summary / title"),
    ("project",  "Project name"),
];

const DEFAULT_COLUMNS: &str = "key,type,status,assignee,updated,summary";

pub fn print_available_columns() {
    println!("Available columns for 'search':");
    for (name, desc) in AVAILABLE_COLUMNS {
        println!("  {:<10} {}", name, desc);
    }
}

pub async fn run_search(
    client: &JiraClient,
    jql: &str,
    limit: u32,
    columns: Option<String>,
) -> Result<()> {
    let active_cols: Vec<&str> = columns
        .as_deref()
        .unwrap_or(DEFAULT_COLUMNS)
        .split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .collect();

    for col in &active_cols {
        if !AVAILABLE_COLUMNS.iter().any(|(n, _)| *n == *col) {
            bail!("Unknown column '{}'. Run with --list-columns to see options.", col);
        }
    }

    let fields_str = {
        let mut seen = std::collections::HashSet::new();
        let mut parts: Vec<&str> = Vec::new();
        for &col in &active_cols {
            let field = match col {
                "key"     => continue,
                "type"    => "issuetype",
                "status"  => "status",
                "assignee"=> "assignee",
                "priority"=> "priority",
                "updated" => "updated",
                "summary" => "summary",
                "project" => "project",
                _         => continue,
            };
            if seen.insert(field) {
                parts.push(field);
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
    let result: Value = client.get_with_params("search", &params).await?;
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
    for (i, &col) in active_cols.iter().enumerate() {
        let w = col_width(col);
        if i == last {
            print!("{}", col_header(col).bold());
        } else {
            print!("{:<w$} ", col_header(col).bold(), w = w);
        }
    }
    println!();
    println!("{}", "─".repeat(100));

    for issue in issues {
        let key = issue["key"].as_str().unwrap_or("?");
        let f = &issue["fields"];

        for (i, &col) in active_cols.iter().enumerate() {
            let is_last = i == last;
            let w = col_width(col);
            match col {
                "key" => {
                    if is_last { print!("{}", key.cyan()); }
                    else { print!("{:<w$} ", key.cyan(), w = w); }
                }
                "type" => {
                    let v = truncate(field_name(f, "issuetype"), w.saturating_sub(1));
                    if is_last { print!("{}", v); }
                    else { print!("{:<w$} ", v, w = w); }
                }
                "status" => {
                    let v = truncate(field_name(f, "status"), w.saturating_sub(1));
                    if is_last { print!("{}", v); }
                    else { print!("{:<w$} ", v, w = w); }
                }
                "assignee" => {
                    let v = truncate(display_name(f, "assignee"), w.saturating_sub(1));
                    if is_last { print!("{}", v); }
                    else { print!("{:<w$} ", v, w = w); }
                }
                "priority" => {
                    let v = truncate(field_name(f, "priority"), w.saturating_sub(1));
                    if is_last { print!("{}", v); }
                    else { print!("{:<w$} ", v, w = w); }
                }
                "updated" => {
                    let v = short_date(f["updated"].as_str().unwrap_or(""));
                    if is_last { print!("{}", v); }
                    else { print!("{:<w$} ", v, w = w); }
                }
                "summary" => {
                    let v = truncate(f["summary"].as_str().unwrap_or("(no summary)"), w.saturating_sub(1));
                    if is_last { print!("{}", v); }
                    else { print!("{:<w$} ", v, w = w); }
                }
                "project" => {
                    let v = truncate(field_name(f, "project"), w.saturating_sub(1));
                    if is_last { print!("{}", v); }
                    else { print!("{:<w$} ", v, w = w); }
                }
                _ => {}
            }
        }
        println!();
    }

    Ok(())
}

fn col_header(col: &str) -> &'static str {
    match col {
        "key"      => "KEY",
        "type"     => "TYPE",
        "status"   => "STATUS",
        "assignee" => "ASSIGNEE",
        "priority" => "PRIORITY",
        "updated"  => "UPDATED",
        "summary"  => "SUMMARY",
        "project"  => "PROJECT",
        _          => "?",
    }
}

fn col_width(col: &str) -> usize {
    match col {
        "key"      => 14,
        "type"     => 10,
        "status"   => 14,
        "assignee" => 22,
        "priority" => 10,
        "updated"  => 10,
        "summary"  => 60,
        "project"  => 14,
        _          => 12,
    }
}
