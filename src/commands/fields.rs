use anyhow::{bail, Result};
use colored::Colorize;
use serde::Deserialize;
use serde_json::Value;

use crate::client::JiraClient;
use super::as_jira_array;

// ── Types ────────────────────────────────────────────────────────────────────

/// Metadata about a field's data type, used to pick the right values endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct FieldSchema {
    #[serde(rename = "type")]
    pub field_type: String,
    /// Present on array fields, e.g. `"version"`, `"option"`, `"string"`.
    pub items: Option<String>,
    /// System field key, e.g. `"fixVersions"`.
    #[allow(dead_code)]
    pub system: Option<String>,
    /// Custom field type key, e.g. `"com.atlassian.jira.plugin.system.customfieldtypes:select"`.
    #[serde(rename = "custom")]
    pub custom_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct JiraField {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub custom: bool,
    #[serde(default)]
    pub navigable: bool,
    pub schema: Option<FieldSchema>,
}

/// A column resolved to everything needed for rendering.
pub struct ResolvedCol {
    /// The name the user typed (or the field's canonical display name for custom fields).
    pub label: String,
    /// Jira API field id to pass in `fields=` (e.g. "issuetype", "customfield_10016").
    pub api_id: String,
    /// True for server-side custom fields; drives the generic value extractor.
    pub custom: bool,
    /// Field schema — present for custom fields, used by the `values` command.
    pub schema: Option<FieldSchema>,
}

/// A single enumerable value for a field.
pub struct FieldValue {
    pub value: String,
    pub detail: Option<String>,
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
    ("parent",     "parent",     "Parent issue key"),
    ("components", "components", "Components"),
    ("labels",     "labels",     "Labels"),
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
                schema: None,
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
                schema: f.schema.clone(),
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

// ── Field values ──────────────────────────────────────────────────────────────

/// Fetch all enumerable values for a resolved column.
///
/// - Global fields (status, priority, issuetype, project) need no project key.
/// - Version and component fields require `--project`.
/// - Custom option/select/multi-select fields use `createmeta` and require `--project`.
pub async fn fetch_field_values(
    client: &JiraClient,
    col: &ResolvedCol,
    project: Option<&str>,
) -> Result<Vec<FieldValue>> {
    match col.api_id.as_str() {
        "issuetype" => issuetype_values(client).await,
        "status"    => status_values(client).await,
        "priority"  => priority_values(client).await,
        "project"   => project_values(client).await,
        "fixVersions" | "versions" | "affectsVersions" => {
            let p = need_project(project, "version fields")?;
            version_values(client, p).await
        }
        "components" => {
            let p = need_project(project, "component fields")?;
            component_values(client, p).await
        }
        _ => {
            // Use the schema stored in ResolvedCol to route custom fields.
            let schema = col.schema.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "Field '{}' has no schema information — cannot determine its value type.",
                    col.label
                )
            })?;

            let is_version = schema.field_type == "version"
                || (schema.field_type == "array" && schema.items.as_deref() == Some("version"));

            let is_option = schema.field_type == "option"
                || (schema.field_type == "array" && schema.items.as_deref() == Some("option"))
                || schema.custom_type.as_deref()
                    .map(|ct| ct.contains("select") || ct.contains("radio") || ct.contains("checkbox") || ct.contains("multicheckboxes"))
                    .unwrap_or(false);

            if is_version {
                let p = need_project(project, "version fields")?;
                version_values(client, p).await
            } else if is_option {
                let p = need_project(project, "custom option fields")?;
                createmeta_option_values(client, &col.api_id, p).await
            } else {
                bail!(
                    "Field '{}' (type: '{}') does not have a list of enumerable values.",
                    col.label,
                    schema.field_type
                )
            }
        }
    }
}

// ── Value fetch helpers ───────────────────────────────────────────────────────

fn need_project<'a>(project: Option<&'a str>, context: &str) -> Result<&'a str> {
    project.ok_or_else(|| {
        anyhow::anyhow!("--project <KEY> is required for {context}")
    })
}

async fn issuetype_values(client: &JiraClient) -> Result<Vec<FieldValue>> {
    let resp: Value = client.get("issuetype").await?;
    let arr = resp.as_array().ok_or_else(|| anyhow::anyhow!("Unexpected /issuetype response"))?;
    Ok(arr.iter().filter_map(|item| {
        let name = item["name"].as_str()?.to_string();
        let detail = if item["subtask"].as_bool().unwrap_or(false) {
            Some("sub-task".to_string())
        } else {
            None
        };
        Some(FieldValue { value: name, detail })
    }).collect())
}

async fn status_values(client: &JiraClient) -> Result<Vec<FieldValue>> {
    let resp: Value = client.get("status").await?;
    let arr = resp.as_array().ok_or_else(|| anyhow::anyhow!("Unexpected /status response"))?;
    Ok(arr.iter().filter_map(|item| {
        let name = item["name"].as_str()?.to_string();
        let detail = item["statusCategory"]["name"].as_str().map(|s| s.to_string());
        Some(FieldValue { value: name, detail })
    }).collect())
}

async fn priority_values(client: &JiraClient) -> Result<Vec<FieldValue>> {
    let resp: Value = client.get("priority").await?;
    let arr = resp.as_array().ok_or_else(|| anyhow::anyhow!("Unexpected /priority response"))?;
    Ok(arr.iter().filter_map(|item| {
        Some(FieldValue {
            value: item["name"].as_str()?.to_string(),
            detail: None,
        })
    }).collect())
}

async fn project_values(client: &JiraClient) -> Result<Vec<FieldValue>> {
    let resp: Value = client.get(client.project_path()).await?;
    let arr = as_jira_array(&resp).ok_or_else(|| anyhow::anyhow!("Unexpected /project response"))?;
    Ok(arr.iter().filter_map(|item| {
        let key = item["key"].as_str()?.to_string();
        let name = item["name"].as_str().map(|s| s.to_string());
        Some(FieldValue { value: key, detail: name })
    }).collect())
}

async fn version_values(client: &JiraClient, project_key: &str) -> Result<Vec<FieldValue>> {
    let resp: Value = client.get(&format!("project/{project_key}/versions")).await?;
    let arr = resp.as_array().ok_or_else(|| anyhow::anyhow!("Unexpected /versions response"))?;
    Ok(arr.iter().filter_map(|item| {
        let name = item["name"].as_str()?.to_string();
        let released = item["released"].as_bool().unwrap_or(false);
        let date = item["releaseDate"].as_str();
        let detail = Some(match (released, date) {
            (true,  Some(d)) => format!("released {d}"),
            (true,  None)    => "released".to_string(),
            (false, Some(d)) => format!("unreleased, due {d}"),
            (false, None)    => "unreleased".to_string(),
        });
        Some(FieldValue { value: name, detail })
    }).collect())
}

async fn component_values(client: &JiraClient, project_key: &str) -> Result<Vec<FieldValue>> {
    let resp: Value = client.get(&format!("project/{project_key}/components")).await?;
    let arr = resp.as_array().ok_or_else(|| anyhow::anyhow!("Unexpected /components response"))?;
    Ok(arr.iter().filter_map(|item| {
        let name = item["name"].as_str()?.to_string();
        let detail = item["description"].as_str()
            .filter(|d| !d.is_empty())
            .map(|d| d.to_string());
        Some(FieldValue { value: name, detail })
    }).collect())
}

/// Fetch option values for a custom select/multi-select field via `createmeta`.
async fn createmeta_option_values(
    client: &JiraClient,
    field_id: &str,
    project_key: &str,
) -> Result<Vec<FieldValue>> {
    let params = [
        ("projectKeys", project_key),
        ("expand", "projects.issuetypes.fields"),
    ];
    let resp: Value = client.get_with_params("issue/createmeta", &params).await?;

    let mut seen = std::collections::HashSet::new();
    let mut values = Vec::new();

    if let Some(projects) = resp["projects"].as_array() {
        for proj in projects {
            if let Some(issuetypes) = proj["issuetypes"].as_array() {
                for itype in issuetypes {
                    if let Some(allowed) = itype["fields"][field_id]["allowedValues"].as_array() {
                        for opt in allowed {
                            // Option fields use "value"; version-like use "name"
                            let val = opt.get("value")
                                .or_else(|| opt.get("name"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("?");
                            if seen.insert(val.to_string()) {
                                values.push(FieldValue { value: val.to_string(), detail: None });
                            }
                        }
                    }
                }
            }
        }
    }

    if values.is_empty() {
        bail!(
            "No options found for this field on project '{project_key}'. \
             The field may not be configured for that project."
        );
    }
    Ok(values)
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
        "parent"     => 14,
        "components" => 20,
        "labels"     => 20,
        _            => 15,
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
