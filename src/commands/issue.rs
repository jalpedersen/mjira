use anyhow::{bail, Result};
use clap::Subcommand;
use colored::Colorize;
use serde_json::{json, Value};

use crate::client::JiraClient;
use super::{display_name, field_name, print_field, short_date, truncate};
use super::fields::{self, ResolvedCol, STATIC_COLS};

const DEFAULT_COLUMNS: &[&str] = &["key", "type", "status", "assignee", "updated", "summary"];

#[derive(Subcommand)]
pub enum IssueCommands {
    /// List issues (defaults to issues assigned to you)
    List {
        /// Filter by project key (e.g. PROJ)
        #[arg(short, long)]
        project: Option<String>,
        /// Filter by assignee username/email; use "me" for yourself
        #[arg(short, long)]
        assignee: Option<String>,
        /// Filter by status (e.g. "In Progress", "To Do")
        #[arg(short, long)]
        status: Option<String>,
        /// Filter by issue type (e.g. Bug, Story, Task)
        #[arg(short = 't', long = "type")]
        issue_type: Option<String>,
        /// Additional JQL clause appended to the query
        #[arg(long)]
        jql: Option<String>,
        /// Maximum number of results
        #[arg(short, long, default_value = "25")]
        limit: u32,
        /// Columns to display, comma-separated (use --list-columns to see options)
        #[arg(short = 'c', long)]
        columns: Option<String>,
        /// Print available columns and exit
        #[arg(long)]
        list_columns: bool,
    },
    /// Show detailed information about an issue
    Get {
        /// Issue key (e.g. PROJ-123)
        key: String,
    },
    /// Create a new issue
    Create {
        /// Project key (e.g. PROJ)
        #[arg(short, long)]
        project: String,
        /// Issue summary / title
        #[arg(short, long)]
        summary: String,
        /// Issue type (e.g. Bug, Story, Task)
        #[arg(short = 't', long = "type", default_value = "Task")]
        issue_type: String,
        /// Description text
        #[arg(short, long)]
        description: Option<String>,
        /// Priority (e.g. High, Medium, Low)
        #[arg(long)]
        priority: Option<String>,
        /// Assignee username or account ID
        #[arg(short, long)]
        assignee: Option<String>,
    },
    /// Add a comment to an issue
    Comment {
        /// Issue key (e.g. PROJ-123)
        key: String,
        /// Comment body text
        body: String,
    },
    /// Transition an issue to a new status
    Transition {
        /// Issue key (e.g. PROJ-123)
        key: String,
        /// Target status name (e.g. "In Progress"). Omit to list available transitions.
        status: Option<String>,
    },
    /// Assign an issue to a user
    Assign {
        /// Issue key (e.g. PROJ-123)
        key: String,
        /// Assignee account ID or username (use "-" to unassign)
        assignee: String,
    },
    /// List all possible values for a field
    Values {
        /// Field name (e.g. "status", "priority", "Target Version/s")
        field: String,
        /// Project key — required for version, component, and custom option fields
        #[arg(short, long)]
        project: Option<String>,
    },
}

pub async fn handle(cmd: IssueCommands, client: &JiraClient) -> Result<()> {
    match cmd {
        IssueCommands::List {
            project,
            assignee,
            status,
            issue_type,
            jql,
            limit,
            columns,
            list_columns,
        } => list(client, project, assignee, status, issue_type, jql, limit, columns, list_columns).await,

        IssueCommands::Get { key } => get(client, &key).await,

        IssueCommands::Create {
            project,
            summary,
            issue_type,
            description,
            priority,
            assignee,
        } => create(client, &project, &summary, &issue_type, description, priority, assignee).await,

        IssueCommands::Comment { key, body } => comment(client, &key, &body).await,

        IssueCommands::Transition { key, status } => transition(client, &key, status).await,

        IssueCommands::Assign { key, assignee } => assign(client, &key, &assignee).await,

        IssueCommands::Values { field, project } => values(client, &field, project).await,
    }
}

// ── List ────────────────────────────────────────────────────────────────────

async fn list(
    client: &JiraClient,
    project: Option<String>,
    assignee: Option<String>,
    status: Option<String>,
    issue_type: Option<String>,
    extra_jql: Option<String>,
    limit: u32,
    columns: Option<String>,
    list_columns: bool,
) -> Result<()> {
    if list_columns {
        return fields::print_columns(client, STATIC_COLS).await;
    }

    let col_names: Vec<&str> = columns
        .as_deref()
        .map(|s| s.split(',').map(str::trim).filter(|c| !c.is_empty()).collect())
        .unwrap_or_else(|| DEFAULT_COLUMNS.to_vec());

    let active_cols: Vec<ResolvedCol> =
        fields::resolve_columns(&col_names, client, STATIC_COLS).await?;

    let mut clauses: Vec<String> = Vec::new();

    if let Some(proj) = project {
        clauses.push(format!("project = \"{}\"", proj));
    }
    match assignee.as_deref() {
        Some("me") => clauses.push("assignee = currentUser()".into()),
        Some(a) => clauses.push(format!("assignee = \"{}\"", a)),
        None => {}
    }
    if let Some(s) = status {
        clauses.push(format!("status = \"{}\"", s));
    }
    if let Some(t) = issue_type {
        clauses.push(format!("issuetype = \"{}\"", t));
    }
    if let Some(j) = extra_jql {
        clauses.push(j);
    }

    let jql = if clauses.is_empty() {
        "assignee = currentUser() ORDER BY updated DESC".to_string()
    } else {
        format!("{} ORDER BY updated DESC", clauses.join(" AND "))
    };

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
        ("jql", jql.as_str()),
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
        format!("Showing {} of {} issues", issues.len(), total).dimmed()
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

// ── Values ────────────────────────────────────────────────────────────────────

async fn values(client: &JiraClient, field_name: &str, project: Option<String>) -> Result<()> {
    let name_slice = [field_name];
    let resolved = fields::resolve_columns(&name_slice, client, STATIC_COLS).await?;
    let col = &resolved[0];

    let mut field_values =
        fields::fetch_field_values(client, col, project.as_deref()).await?;

    field_values.sort_by(|a, b| a.value.cmp(&b.value));

    println!("Values for '{}':", col.label.bold());
    println!("{}", "─".repeat(60));

    let value_width = field_values.iter().map(|v| v.value.len()).max().unwrap_or(0);
    for fv in &field_values {
        match &fv.detail {
            Some(d) => println!("  {:<width$}  {}", fv.value, d.dimmed(), width = value_width),
            None    => println!("  {}", fv.value),
        }
    }

    println!();
    println!("{}", format!("{} values", field_values.len()).dimmed());
    Ok(())
}

/// Extract a plain-text cell value for non-key, non-status static columns.
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

// ── Get ─────────────────────────────────────────────────────────────────────

async fn get(client: &JiraClient, key: &str) -> Result<()> {
    let issue: Value = client
        .get(&format!("issue/{key}?fields=summary,description,status,assignee,reporter,issuetype,priority,created,updated,project,comment,labels,fixVersions"))
        .await?;

    let f = &issue["fields"];

    println!();
    println!("{} — {}", issue["key"].as_str().unwrap_or(key).cyan().bold(),
        f["summary"].as_str().unwrap_or("(no summary)").bold());
    println!("{}", "─".repeat(80));

    print_field("Type",     field_name(f, "issuetype"));
    print_field("Status",   field_name(f, "status"));
    print_field("Priority", field_name(f, "priority"));
    print_field("Project",  field_name(f, "project"));
    print_field("Assignee", display_name(f, "assignee"));
    print_field("Reporter", display_name(f, "reporter"));
    print_field("Created",  short_date(f["created"].as_str().unwrap_or("")));
    print_field("Updated",  short_date(f["updated"].as_str().unwrap_or("")));

    // Labels
    if let Some(labels) = f["labels"].as_array() {
        let ls: Vec<&str> = labels.iter().filter_map(|l| l.as_str()).collect();
        if !ls.is_empty() {
            print_field("Labels", &ls.join(", "));
        }
    }

    // Fix versions
    if let Some(fvs) = f["fixVersions"].as_array() {
        let vs: Vec<&str> = fvs.iter().filter_map(|v| v["name"].as_str()).collect();
        if !vs.is_empty() {
            print_field("Fix Version", &vs.join(", "));
        }
    }

    // Description
    println!();
    println!("{}", "Description:".bold());
    match f["description"].as_str() {
        Some(desc) if !desc.is_empty() => println!("{}", desc),
        _ => println!("{}", "(no description)".dimmed()),
    }

    // Comments
    let comments = f["comment"]["comments"].as_array();
    if let Some(comments) = comments {
        if !comments.is_empty() {
            println!();
            println!(
                "{} ({})",
                "Comments:".bold(),
                comments.len().to_string().dimmed()
            );
            println!("{}", "─".repeat(80));
            for c in comments {
                let author = c["author"]["displayName"].as_str().unwrap_or("?");
                let created = short_date(c["created"].as_str().unwrap_or(""));
                let body = c["body"].as_str().unwrap_or("");
                println!("{} — {}", author.bold(), created.dimmed());
                println!("{}", body);
                println!();
            }
        }
    }

    Ok(())
}

// ── Create ──────────────────────────────────────────────────────────────────

async fn create(
    client: &JiraClient,
    project: &str,
    summary: &str,
    issue_type: &str,
    description: Option<String>,
    priority: Option<String>,
    assignee: Option<String>,
) -> Result<()> {
    let mut fields = json!({
        "project": { "key": project },
        "summary": summary,
        "issuetype": { "name": issue_type },
    });

    if let Some(desc) = description {
        fields["description"] = json!(desc);
    }
    if let Some(prio) = priority {
        fields["priority"] = json!({ "name": prio });
    }
    if let Some(a) = assignee {
        // Jira Cloud uses accountId; Jira Server uses name
        fields["assignee"] = json!({ "name": a });
    }

    let body = json!({ "fields": fields });
    let resp: Value = client.post("issue", &body).await?;
    let key = resp["key"].as_str().unwrap_or("?");
    println!("Created {} ✓", key.green().bold());
    Ok(())
}

// ── Comment ─────────────────────────────────────────────────────────────────

async fn comment(client: &JiraClient, key: &str, body_text: &str) -> Result<()> {
    let body = json!({ "body": body_text });
    let resp: Value = client.post(&format!("issue/{key}/comment"), &body).await?;
    let id = resp["id"].as_str().unwrap_or("?");
    println!("Comment {} added to {} ✓", id.dimmed(), key.green());
    Ok(())
}

// ── Transition ───────────────────────────────────────────────────────────────

async fn transition(client: &JiraClient, key: &str, target: Option<String>) -> Result<()> {
    let resp: Value = client.get(&format!("issue/{key}/transitions")).await?;
    let transitions = resp["transitions"].as_array().cloned().unwrap_or_default();

    match target {
        None => {
            // List available transitions
            println!("Available transitions for {}:", key.cyan());
            for t in &transitions {
                let id = t["id"].as_str().unwrap_or("?");
                let name = t["name"].as_str().unwrap_or("?");
                println!("  {:>4}  {}", id.dimmed(), name.bold());
            }
        }
        Some(status_name) => {
            // Find matching transition (case-insensitive)
            let t = transitions.iter().find(|t| {
                t["name"]
                    .as_str()
                    .map(|n| n.eq_ignore_ascii_case(&status_name))
                    .unwrap_or(false)
            });
            match t {
                None => {
                    let names: Vec<&str> = transitions
                        .iter()
                        .filter_map(|t| t["name"].as_str())
                        .collect();
                    bail!(
                        "No transition named '{}'. Available: {}",
                        status_name,
                        names.join(", ")
                    );
                }
                Some(t) => {
                    let id = t["id"].as_str().unwrap_or("?");
                    let body = json!({ "transition": { "id": id } });
                    client
                        .post_no_body(&format!("issue/{key}/transitions"), &body)
                        .await?;
                    println!(
                        "{} transitioned to '{}' ✓",
                        key.green(),
                        t["name"].as_str().unwrap_or(&status_name).bold()
                    );
                }
            }
        }
    }
    Ok(())
}

// ── Assign ───────────────────────────────────────────────────────────────────

async fn assign(client: &JiraClient, key: &str, assignee: &str) -> Result<()> {
    let body = if assignee == "-" {
        json!({ "name": null })
    } else {
        json!({ "name": assignee })
    };
    client
        .put_no_body(&format!("issue/{key}/assignee"), &body)
        .await?;
    if assignee == "-" {
        println!("{} unassigned ✓", key.green());
    } else {
        println!("{} assigned to '{}' ✓", key.green(), assignee.bold());
    }
    Ok(())
}
