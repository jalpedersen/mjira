use anyhow::Result;
use clap::Subcommand;
use colored::Colorize;
use serde_json::Value;

use crate::client::JiraClient;
use super::fields::{self, ResolvedCol, STATIC_COLS};
use super::{display_name, field_name, short_date, truncate};

const DEFAULT_COLUMNS: &[&str] = &["key", "type", "status", "assignee", "updated", "summary"];

#[derive(Subcommand)]
pub enum BoardCommands {
    /// List accessible boards
    List {
        /// Filter by project key or ID
        #[arg(short, long)]
        project: Option<String>,
        /// Filter by board name (substring match)
        #[arg(short, long)]
        name: Option<String>,
        /// Maximum number of boards to return (default: fetch all)
        #[arg(short, long)]
        limit: Option<u32>,
    },
    /// List issues on a board
    Issues {
        /// Board ID
        id: u64,
        /// Maximum results to return
        #[arg(short, long, default_value = "25")]
        limit: u32,
        /// Columns to display, comma-separated
        #[arg(short = 'c', long)]
        columns: Option<String>,
        /// Additional JQL to filter issues
        #[arg(short, long)]
        jql: Option<String>,
        /// Apply a quick filter by ID (see board quick-filters <id>)
        #[arg(short = 'q', long)]
        quick_filter: Option<u64>,
    },
    /// List quick filters for a board
    QuickFilters {
        /// Board ID
        id: u64,
    },
}

pub async fn handle(cmd: BoardCommands, client: &JiraClient) -> Result<()> {
    match cmd {
        BoardCommands::List { project, name, limit } => list(client, project, name, limit).await,
        BoardCommands::Issues { id, limit, columns, jql, quick_filter } => {
            issues(client, id, limit, columns, jql, quick_filter).await
        }
        BoardCommands::QuickFilters { id } => quick_filters(client, id).await,
    }
}

async fn list(client: &JiraClient, project: Option<String>, name_filter: Option<String>, limit: Option<u32>) -> Result<()> {
    let page_size: u32 = limit.unwrap_or(50).min(50);
    let mut all_boards: Vec<Value> = Vec::new();
    let mut start_at: u32 = 0;
    let mut total_reported: Option<u64> = None;
    loop {
        let start_str = start_at.to_string();
        let size_str = page_size.to_string();
        let mut params: Vec<(&str, String)> = vec![
            ("maxResults", size_str.clone()),
            ("startAt", start_str.clone()),
        ];
        if let Some(p) = &project {
            params.push(("projectKeyOrId", p.clone()));
        }
        if let Some(n) = &name_filter {
            params.push(("name", n.clone()));
        }

        let params_ref: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let result: Value = client.agile_get_with_params("board", &params_ref).await?;

        let total = result["total"].as_u64().unwrap_or(0);
        let page = match result["values"].as_array() {
            Some(v) if !v.is_empty() => v.clone(),
            _ => break,
        };
        total_reported = Some(total);
        let fetched = page.len() as u32;
        all_boards.extend(page);

        if let Some(lim) = limit {
            if all_boards.len() >= lim as usize {
                all_boards.truncate(lim as usize);
                break;
            }
        }

        start_at += fetched;
        if fetched < page_size || all_boards.len() as u64 >= total {
            break;
        }
    }

    if all_boards.is_empty() {
        println!("No boards found.");
        return Ok(());
    }

    let total_reported = total_reported.unwrap_or(all_boards.len() as u64);
    println!("{}", format!("Showing {} of {} boards", all_boards.len(), total_reported).dimmed());
    println!();
    println!(
        "{:<6} {:<10} {:<14} {}",
        "ID".bold(),
        "TYPE".bold(),
        "PROJECT".bold(),
        "NAME".bold()
    );
    println!("{}", "─".repeat(70));

    for b in &all_boards {
        let id = b["id"].as_u64().map(|n| n.to_string()).unwrap_or_else(|| "?".to_string());
        let btype = b["type"].as_str().unwrap_or("?");
        let proj_key = b["location"]["projectKey"].as_str().unwrap_or("—");
        let bname = b["name"].as_str().unwrap_or("?");
        println!(
            "{:<6} {:<10} {:<14} {}",
            id.cyan(),
            btype,
            proj_key,
            bname
        );
    }

    Ok(())
}

async fn fetch_quick_filters(client: &JiraClient, board_id: u64) -> Result<Vec<Value>> {
    // Try the dedicated quick-filter sub-resource first (works on some versions).
    const PAGE: u32 = 50;
    let mut all: Vec<Value> = Vec::new();
    let mut start_at: u32 = 0;
    let mut use_config = false;

    loop {
        let start_str = start_at.to_string();
        let size_str = PAGE.to_string();
        let path = format!("board/{}/quickfilter", board_id);
        let result_res: Result<Value> = client
            .agile_get_with_params(&path, &[("maxResults", &size_str), ("startAt", &start_str)])
            .await;

        let result = match result_res {
            Ok(v) => v,
            Err(e) if e.to_string().contains("404") && start_at == 0 => {
                use_config = true;
                break;
            }
            Err(e) => return Err(e),
        };

        let page = match result["values"].as_array() {
            Some(v) if !v.is_empty() => v.clone(),
            _ => break,
        };
        let fetched = page.len() as u32;
        all.extend(page);

        let is_last = result["isLast"].as_bool().unwrap_or(false);
        if is_last || fetched < PAGE {
            break;
        }
        start_at += fetched;
    }

    if use_config || all.is_empty() {
        // Try the board configuration endpoint first (modern Jira Software versions
        // include a top-level "quickFilters" array there).
        let path = format!("board/{}/configuration", board_id);
        let result: Value = client.agile_get_with_params(&path, &[]).await?;
        if let Some(filters) = result["quickFilters"].as_array() {
            all.extend(filters.clone());
        }

        // Older Jira Server/DC omits quickFilters from the configuration response.
        // Try the editmodel endpoint which includes the full board config with quick filters.
        if all.is_empty() {
            let id_str = board_id.to_string();
            if let Ok(result) = client
                .greenhopper_get::<Value>("rapidviewconfig/editmodel.json", &[("rapidViewId", &id_str)])
                .await
            {
                if let Some(filters) = result["quickFilterConfig"]["quickFilters"].as_array() {
                    all.extend(filters.clone());
                }
            }
        }

        // Final fallback: the internal board-data endpoint that the Jira UI itself uses.
        if all.is_empty() {
            let id_str = board_id.to_string();
            let result: Value = client
                .greenhopper_get("xboard/work/allData.json", &[("rapidViewId", &id_str)])
                .await?;
            // Response shape: { "quickFilters": { "quickFilters": [...] } }
            let inner = &result["quickFilters"];
            let list = inner["quickFilters"]
                .as_array()
                .or_else(|| inner.as_array())
                .cloned()
                .unwrap_or_default();
            all.extend(list);
        }
    }

    Ok(all)
}

async fn quick_filters(client: &JiraClient, board_id: u64) -> Result<()> {
    let all = fetch_quick_filters(client, board_id).await?;

    if all.is_empty() {
        println!("No quick filters found for board {}.", board_id);
        return Ok(());
    }

    println!("{}", format!("{} quick filter(s) on board {}", all.len(), board_id).dimmed());
    println!();
    println!("{:<6} {:<24} {}", "ID".bold(), "NAME".bold(), "JQL".bold());
    println!("{}", "─".repeat(90));
    for f in &all {
        let id = f["id"].as_u64().map(|n| n.to_string()).unwrap_or_else(|| "?".to_string());
        let name = f["name"].as_str().unwrap_or("?");
        let query = f["query"].as_str().unwrap_or("—");
        println!("{:<6} {:<24} {}", id.cyan(), name, query);
    }

    Ok(())
}

async fn issues(client: &JiraClient, board_id: u64, limit: u32, columns: Option<String>, jql: Option<String>, quick_filter: Option<u64>) -> Result<()> {
    let jql = match quick_filter {
        None => jql,
        Some(qf_id) => {
            let qf_jql = {
                let filters = fetch_quick_filters(client, board_id).await?;
                filters.iter()
                    .find(|f| f["id"].as_u64() == Some(qf_id))
                    .and_then(|f| f["query"].as_str())
                    .unwrap_or("")
                    .to_string()
            };
            // Adding a JQL parameter to board/{id}/issue replaces the built-in sprint
            // scope, so we must re-add it explicitly.
            let sprint_scope = "sprint in openSprints()";
            let base = match jql {
                Some(existing) if !existing.is_empty() => format!("({}) AND ({})", existing, qf_jql),
                _ => qf_jql,
            };
            Some(format!("({}) AND {}", base, sprint_scope))
        }
    };
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
    let path = format!("board/{}/issue", board_id);
    let mut params: Vec<(&str, &str)> = vec![
        ("maxResults", max.as_str()),
        ("fields", fields_str.as_str()),
    ];
    if let Some(j) = &jql {
        params.push(("jql", j.as_str()));
    }
    let result: Value = client.agile_get_with_params(&path, &params).await?;

    let issues = result["issues"].as_array().map(|v| v.as_slice()).unwrap_or(&[]);
    if issues.is_empty() {
        println!("No issues found.");
        return Ok(());
    }

    let total = result["total"].as_u64().unwrap_or(0);
    let header_suffix = match &jql {
        Some(j) => format!("  •  JQL: {}", j),
        None => String::new(),
    };
    println!(
        "{}",
        format!("Showing {} of {} issues  •  board {}{}", issues.len(), total, board_id, header_suffix).dimmed()
    );
    println!();

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
