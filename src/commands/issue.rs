use anyhow::{bail, Result};
use clap::Subcommand;
use colored::Colorize;
use serde_json::{json, Value};

use crate::client::JiraClient;
use super::{display_name, field_name, print_field, short_date, truncate};

const AVAILABLE_COLUMNS: &[(&str, &str)] = &[
    ("key",      "Issue key (e.g. PROJ-123)"),
    ("type",     "Issue type (Bug, Story, Task, etc.)"),
    ("status",   "Current status"),
    ("assignee", "Assigned user"),
    ("priority", "Priority level"),
    ("updated",  "Last updated date"),
    ("summary",  "Issue summary / title"),
];

const DEFAULT_COLUMNS: &str = "key,type,status,assignee,updated,summary";

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
        println!("Available columns for 'issue list':");
        for (name, desc) in AVAILABLE_COLUMNS {
            println!("  {:<10} {}", name, desc);
        }
        return Ok(());
    }

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
        for &col in &active_cols {
            let field = match col {
                "key"      => continue,
                "type"     => "issuetype",
                "status"   => "status",
                "assignee" => "assignee",
                "priority" => "priority",
                "updated"  => "updated",
                "summary"  => "summary",
                _          => continue,
            };
            if seen.insert(field) {
                parts.push(field);
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
                    let raw = field_name(f, "status");
                    let v = truncate(raw, w.saturating_sub(1));
                    if is_last { print!("{}", status_colored(&v)); }
                    else { print!("{:<w$} ", status_colored(&v), w = w); }
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
                _ => {}
            }
        }
        println!();
    }
    Ok(())
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

// ── Helpers ──────────────────────────────────────────────────────────────────

fn col_header(col: &str) -> &'static str {
    match col {
        "key"      => "KEY",
        "type"     => "TYPE",
        "status"   => "STATUS",
        "assignee" => "ASSIGNEE",
        "priority" => "PRIORITY",
        "updated"  => "UPDATED",
        "summary"  => "SUMMARY",
        _          => "?",
    }
}

fn col_width(col: &str) -> usize {
    match col {
        "key"      => 14,
        "type"     => 12,
        "status"   => 12,
        "assignee" => 22,
        "priority" => 10,
        "updated"  => 10,
        "summary"  => 60,
        _          => 12,
    }
}

fn status_colored(s: &str) -> colored::ColoredString {
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
