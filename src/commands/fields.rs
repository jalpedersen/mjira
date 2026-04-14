use anyhow::{bail, Result};
use colored::Colorize;
use serde::Deserialize;
use serde_json::Value;

use crate::client::JiraClient;

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct JiraField {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub custom: bool,
    #[serde(default)]
    pub navigable: bool,
}

/// A column resolved to everything needed for rendering.
pub struct ResolvedCol {
    /// The name the user typed (or the field's canonical display name for custom fields).
    pub label: String,
    /// Jira API field id to pass in `fields=` (e.g. "issuetype", "customfield_10016").
    pub api_id: String,
    /// True for server-side custom fields; drives the generic value extractor.
    pub custom: bool,
}

// ── Static column table ───────────────────────────────────────────────────────

/// Built-in columns: (user-facing name, Jira API field id, description).
pub const STATIC_COLS: &[(&str, &str, &str)] = &[
    ("key",      "key",       "Issue key (e.g. PROJ-123)"),
    ("type",     "issuetype", "Issue type (Bug, Story, Task, etc.)"),
    ("status",   "status",    "Current status"),
    ("assignee", "assignee",  "Assigned user"),
    ("priority", "priority",  "Priority level"),
    ("updated",  "updated",   "Last updated date"),
    ("summary",  "summary",   "Issue summary / title"),
    ("project",  "project",   "Project name"),
];

// ── Network ───────────────────────────────────────────────────────────────────

/// Fetch all fields from the Jira instance (`GET /field`).
pub async fn fetch_fields(client: &JiraClient) -> Result<Vec<JiraField>> {
    let mut fields: Vec<JiraField> = client.get("field").await?;
    fields.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(fields)
}

// ── Discovery ─────────────────────────────────────────────────────────────────

/// Print built-in columns followed by all navigable custom fields from the server.
pub async fn print_columns(
    client: &JiraClient,
    static_cols: &[(&str, &str, &str)],
) -> Result<()> {
    println!("Built-in columns:");
    for (name, _, desc) in static_cols {
        println!("  {:<10} {}", name, desc);
    }

    let server = fetch_fields(client).await?;
    let custom: Vec<&JiraField> = server
        .iter()
        .filter(|f| f.custom && f.navigable)
        .collect();

    if custom.is_empty() {
        println!();
        println!("{}", "No custom fields found on this instance.".dimmed());
    } else {
        println!();
        println!("Custom fields (from server):");
        for f in custom {
            println!("  {:<32} {}", f.name, f.id.dimmed());
        }
    }
    Ok(())
}

// ── Resolution ────────────────────────────────────────────────────────────────

/// Resolve user-supplied column names into `ResolvedCol`.
///
/// Matches case-insensitively against `static_cols` first; only fetches the
/// server's field list when at least one name doesn't match a built-in.
pub async fn resolve_columns(
    names: &[&str],
    client: &JiraClient,
    static_cols: &[(&str, &str, &str)],
) -> Result<Vec<ResolvedCol>> {
    let mut resolved = Vec::new();
    let mut server_fields: Option<Vec<JiraField>> = None;

    for &name in names {
        // 1. Check built-in columns (case-insensitive).
        if let Some(&(col_name, api_id, _)) = static_cols
            .iter()
            .find(|(n, _, _)| n.eq_ignore_ascii_case(name))
        {
            resolved.push(ResolvedCol {
                label: col_name.to_string(),
                api_id: api_id.to_string(),
                custom: false,
            });
            continue;
        }

        // 2. Lazy-fetch server fields on first custom-column request.
        if server_fields.is_none() {
            server_fields = Some(fetch_fields(client).await?);
        }
        let fields = server_fields.as_ref().unwrap();

        // Match by display name, then by id (case-insensitive).
        if let Some(f) = fields.iter().find(|f| {
            f.name.eq_ignore_ascii_case(name) || f.id.eq_ignore_ascii_case(name)
        }) {
            resolved.push(ResolvedCol {
                label: f.name.clone(),
                api_id: f.id.clone(),
                custom: true,
            });
        } else {
            bail!(
                "Unknown column '{}'. Run with --list-columns to see available columns.",
                name
            );
        }
    }

    Ok(resolved)
}

// ── Rendering helpers ─────────────────────────────────────────────────────────

/// Display width for a column.  Custom / unknown columns get a sensible default.
pub fn col_width(col: &ResolvedCol) -> usize {
    match col.label.as_str() {
        "key"      => 14,
        "type"     => 12,
        "status"   => 12,
        "assignee" => 22,
        "priority" => 10,
        "updated"  => 10,
        "summary"  => 60,
        "project"  => 14,
        _          => 15,
    }
}

/// Column header label (uppercased; dynamic for custom fields).
pub fn col_header(col: &ResolvedCol) -> String {
    col.label.to_uppercase()
}

/// Best-effort value extractor for custom field JSON values.
///
/// Handles common Jira field shapes:
/// - bare scalars (number, string, bool)
/// - objects with `.value`, `.name`, or `.displayName` (option/user/priority-like)
/// - arrays of any of the above (joined with ", ")
pub fn extract_custom_value(v: &Value) -> String {
    match v {
        Value::Null => "—".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if f.fract() == 0.0 {
                    return format!("{}", f as i64);
                }
                return format!("{}", f);
            }
            n.to_string()
        }
        Value::String(s) => s.clone(),
        Value::Object(_) => {
            for key in &["value", "name", "displayName"] {
                if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
                    return s.to_string();
                }
            }
            "—".to_string()
        }
        Value::Array(arr) => {
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|item| {
                    if let Some(s) = item.as_str() {
                        return Some(s.to_string());
                    }
                    for key in &["value", "name", "displayName"] {
                        if let Some(s) = item.get(key).and_then(|x| x.as_str()) {
                            return Some(s.to_string());
                        }
                    }
                    None
                })
                .collect();
            if parts.is_empty() {
                "—".to_string()
            } else {
                parts.join(", ")
            }
        }
    }
}

/// Color-code a status string (shared by both `issue list` and `search`).
pub fn status_colored(s: &str) -> colored::ColoredString {
    let lower = s.to_lowercase();
    if lower.contains("done") || lower.contains("closed") || lower.contains("resolved") {
        s.green()
    } else if lower.contains("progress") || lower.contains("review") {
        s.yellow()
    } else if lower.contains("blocked") {
        s.red()
    } else {
        s.normal()
    }
}
