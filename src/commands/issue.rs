use anyhow::{bail, Result};
use clap::Subcommand;
use colored::Colorize;
use serde_json::{json, Value};

use super::fields::{self, ResolvedCol, STATIC_COLS};
use super::{display_name, field_name, print_field, short_date, to_adf_doc, truncate};
use crate::client::JiraClient;
use crate::config::Instance;

const DEFAULT_COLUMNS: &[&str] = &[
    "key",
    "parent",
    "type",
    "status",
    "assignee",
    "updated",
    "components",
    "summary",
];

#[derive(Subcommand)]
pub enum IssueCommands {
    /// List issues (defaults to issues assigned to you)
    List {
        /// Filter by project key (e.g. PROJ)
        #[arg(short, long)]
        project: Option<String>,
        /// Filter by assignee username/email
        #[arg(short, long)]
        assignee: Option<String>,
        /// Show issues for all assignees (clears the default assignee filter)
        #[arg(long)]
        any_assignee: bool,
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
        /// Display image attachments using kitty icat
        #[arg(long)]
        images: bool,
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
    /// List git commits mentioning an issue key across configured repositories
    Commits {
        /// Issue key (e.g. PROJ-123)
        key: String,
        /// Additional repository paths to search (supplements config repos)
        #[arg(short, long = "repo")]
        repos: Vec<String>,
        /// Show repos with no commits found
        #[arg(short, long)]
        verbose: bool,
    },
    /// Show git diff for commits mentioning an issue key
    Diff {
        /// Issue key (e.g. PROJ-123)
        key: String,
        /// Show diff for a specific commit hash only
        #[arg(short, long)]
        commit: Option<String>,
        /// Additional repository paths to search (supplements config repos)
        #[arg(short, long = "repo")]
        repos: Vec<String>,
        /// Show repos with no commits found
        #[arg(short, long)]
        verbose: bool,
    },
}

pub async fn handle(cmd: IssueCommands, client: &JiraClient, instance: &Instance) -> Result<()> {
    match cmd {
        IssueCommands::List {
            project,
            assignee,
            any_assignee,
            status,
            issue_type,
            jql,
            limit,
            columns,
            list_columns,
        } => {
            list(
                client,
                instance,
                project,
                assignee,
                any_assignee,
                status,
                issue_type,
                jql,
                limit,
                columns,
                list_columns,
            )
            .await
        }

        IssueCommands::Get { key, images } => get(client, &key, images).await,

        IssueCommands::Create {
            project,
            summary,
            issue_type,
            description,
            priority,
            assignee,
        } => {
            create(
                client,
                &project,
                &summary,
                &issue_type,
                description,
                priority,
                assignee,
            )
            .await
        }

        IssueCommands::Comment { key, body } => comment(client, &key, &body).await,

        IssueCommands::Transition { key, status } => transition(client, &key, status).await,

        IssueCommands::Assign { key, assignee } => assign(client, &key, &assignee).await,

        IssueCommands::Values { field, project } => values(client, &field, project).await,

        IssueCommands::Commits {
            key,
            repos,
            verbose,
        } => commits(client, instance, &key, repos, verbose).await,

        IssueCommands::Diff {
            key,
            commit,
            repos,
            verbose,
        } => diff(client, instance, &key, commit.as_deref(), repos, verbose).await,
    }
}

// ── List ────────────────────────────────────────────────────────────────────

async fn list(
    client: &JiraClient,
    instance: &Instance,
    project: Option<String>,
    assignee: Option<String>,
    any_assignee: bool,
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
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .collect()
        })
        .unwrap_or_else(|| DEFAULT_COLUMNS.to_vec());

    let active_cols: Vec<ResolvedCol> =
        fields::resolve_columns(&col_names, client, STATIC_COLS).await?;

    let mut clauses: Vec<String> = Vec::new();

    if let Some(proj) = project {
        clauses.push(format!("project = \"{}\"", proj));
    }
    if !any_assignee {
        if let Some(a) = assignee.as_deref().or(instance.default_assignee.as_deref()) {
            clauses.push(format!("assignee = {}", a));
        }
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

    let jql = format!("{} ORDER BY updated DESC", clauses.join(" AND "));

    let fields_str = {
        let mut seen = std::collections::HashSet::new();
        let mut parts: Vec<&str> = Vec::new();
        for col in &active_cols {
            let id = col.api_id.as_str();
            if id == "key" {
                continue;
            }
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
    let result: Value = client
        .get_with_params(client.search_path(), &params)
        .await?;
    let issues = result["issues"]
        .as_array()
        .map(|v| v.as_slice())
        .unwrap_or(&[]);

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
                if is_last {
                    print!("{}", v);
                } else {
                    print!("{:<w$} ", v, w = w);
                }
            } else {
                match col.label.as_str() {
                    "key" => {
                        if is_last {
                            print!("{}", key.cyan());
                        } else {
                            print!("{:<w$} ", key.cyan(), w = w);
                        }
                    }
                    "status" => {
                        let v = truncate(field_name(f, "status"), w.saturating_sub(1));
                        if is_last {
                            print!("{}", fields::status_colored(&v));
                        } else {
                            print!("{:<w$} ", fields::status_colored(&v), w = w);
                        }
                    }
                    _ => {
                        let v = cell_value(f, col, w);
                        if is_last {
                            print!("{}", v);
                        } else {
                            print!("{:<w$} ", v, w = w);
                        }
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

    let mut field_values = fields::fetch_field_values(client, col, project.as_deref()).await?;

    field_values.sort_by(|a, b| a.value.cmp(&b.value));

    println!("Values for '{}':", col.label.bold());
    println!("{}", "─".repeat(60));

    let value_width = field_values
        .iter()
        .map(|v| v.value.len())
        .max()
        .unwrap_or(0);
    for fv in &field_values {
        match &fv.detail {
            Some(d) => println!(
                "  {:<width$}  {}",
                fv.value,
                d.dimmed(),
                width = value_width
            ),
            None => println!("  {}", fv.value),
        }
    }

    println!();
    println!("{}", format!("{} values", field_values.len()).dimmed());
    Ok(())
}

/// Extract a plain-text cell value for non-key, non-status static columns.
fn cell_value(f: &Value, col: &ResolvedCol, w: usize) -> String {
    match col.label.as_str() {
        "type" => truncate(field_name(f, "issuetype"), w.saturating_sub(1)),
        "assignee" => truncate(display_name(f, "assignee"), w.saturating_sub(1)),
        "priority" => truncate(field_name(f, "priority"), w.saturating_sub(1)),
        "updated" => short_date(f["updated"].as_str().unwrap_or("")).to_string(),
        "summary" => truncate(
            f["summary"].as_str().unwrap_or("(no summary)"),
            w.saturating_sub(1),
        ),
        "project" => truncate(field_name(f, "project"), w.saturating_sub(1)),
        "parent" => truncate(
            f["parent"]["key"].as_str().unwrap_or("—"),
            w.saturating_sub(1),
        ),
        "components" => {
            let names: Vec<&str> = f["components"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|c| c["name"].as_str()).collect())
                .unwrap_or_default();
            let joined = names.join(", ");
            truncate(
                if names.is_empty() { "—" } else { &joined },
                w.saturating_sub(1),
            )
        }
        "labels" => {
            let labels: Vec<&str> = f["labels"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|l| l.as_str()).collect())
                .unwrap_or_default();
            let joined = labels.join(", ");
            truncate(
                if labels.is_empty() { "—" } else { &joined },
                w.saturating_sub(1),
            )
        }
        _ => truncate(field_name(f, col.api_id.as_str()), w.saturating_sub(1)),
    }
}

// ── Get ─────────────────────────────────────────────────────────────────────

async fn get(client: &JiraClient, key: &str, show_images: bool) -> Result<()> {
    let issue: Value = client
        .get(&format!("issue/{key}?fields=summary,description,status,assignee,reporter,issuetype,priority,created,updated,project,comment,labels,fixVersions,attachment&expand=changelog"))
        .await?;

    let f = &issue["fields"];
    let display_key = issue["key"].as_str().unwrap_or(key);

    println!();
    println!(
        "{} — {}",
        display_key.cyan().bold(),
        f["summary"].as_str().unwrap_or("(no summary)").bold()
    );
    println!("{}", "─".repeat(80));
    print_field("URL", &client.browse_url(display_key));

    print_field("Type", field_name(f, "issuetype"));
    print_field("Status", field_name(f, "status"));
    print_field("Priority", field_name(f, "priority"));
    print_field("Project", field_name(f, "project"));
    print_field("Assignee", display_name(f, "assignee"));
    print_field("Reporter", display_name(f, "reporter"));
    print_field("Created", f["created"].as_str().unwrap_or(""));
    print_field("Updated", f["updated"].as_str().unwrap_or(""));

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

    // Attachments
    if let Some(attachments) = f["attachment"].as_array() {
        if !attachments.is_empty() {
            println!();
            println!(
                "{} ({})",
                "Attachments:".bold(),
                attachments.len().to_string().dimmed()
            );
            for a in attachments {
                let filename = a["filename"].as_str().unwrap_or("?");
                let url = a["content"].as_str().unwrap_or("");
                println!("  {} — {}", filename.bold(), url.dimmed());
            }
            if show_images {
                show_image_attachments(client, attachments).await?;
            }
        }
    }

    // Description
    println!();
    println!("{}", "Description:".bold());
    let desc_text = f["description"]
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| adf_to_text(&f["description"]));
    match desc_text.as_deref() {
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
                let created = c["created"].as_str().unwrap_or("");
                let body_owned = c["body"]
                    .as_str()
                    .map(|s| s.to_string())
                    .or_else(|| adf_to_text(&c["body"]))
                    .unwrap_or_default();
                let body = body_owned.as_str();
                println!("{} — {}", author.bold(), created.dimmed());
                println!("{}", body);
                println!();
            }
        }
    }

    // Assignee history
    let histories = issue["changelog"]["histories"].as_array();
    if let Some(histories) = histories {
        let assignee_events: Vec<(String, String, String)> = histories
            .iter()
            .flat_map(|h| {
                let date = h["created"].as_str().unwrap_or("").to_string();
                let actor = h["author"]["displayName"]
                    .as_str()
                    .unwrap_or("?")
                    .to_string();
                h["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|item| item["field"].as_str() == Some("assignee"))
                    .map(|item| {
                        let to = item["toString"]
                            .as_str()
                            .unwrap_or("(unassigned)")
                            .to_string();
                        (date.clone(), actor.clone(), to)
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        if !assignee_events.is_empty() {
            println!();
            println!("{}", "Assignee History:".bold());
            println!("{}", "─".repeat(80));
            for (date, actor, to) in &assignee_events {
                println!(
                    "{}  {} assigned to {}",
                    date.as_str().dimmed(),
                    actor.bold(),
                    to.cyan()
                );
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
        fields["description"] = if client.api_version() >= 3 {
            to_adf_doc(&desc)
        } else {
            json!(desc)
        };
    }
    if let Some(prio) = priority {
        fields["priority"] = json!({ "name": prio });
    }
    if let Some(a) = assignee {
        fields["assignee"] = if client.api_version() >= 3 {
            json!({ "accountId": a })
        } else {
            json!({ "name": a })
        };
    }

    let body = json!({ "fields": fields });
    let resp: Value = client.post("issue", &body).await?;
    let key = resp["key"].as_str().unwrap_or("?");
    println!("Created {} ✓", key.green().bold());
    Ok(())
}

// ── Comment ─────────────────────────────────────────────────────────────────

async fn comment(client: &JiraClient, key: &str, body_text: &str) -> Result<()> {
    let body = if client.api_version() >= 3 {
        json!({ "body": to_adf_doc(body_text) })
    } else {
        json!({ "body": body_text })
    };
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

// ── Commits ──────────────────────────────────────────────────────────────────

async fn commits(
    client: &JiraClient,
    instance: &Instance,
    key: &str,
    extra_repos: Vec<String>,
    verbose: bool,
) -> Result<()> {
    // Fetch the issue's components to look up component-specific repos.
    let issue: Value = client
        .get(&format!("issue/{key}?fields=components,summary"))
        .await?;
    let f = &issue["fields"];

    let component_names: Vec<&str> = f["components"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|c| c["name"].as_str()).collect())
        .unwrap_or_default();

    // Resolve repos: prefer component mappings; fall back to instance.repos.
    let mut repos: Vec<String> = {
        let mapped: Vec<String> = component_names
            .iter()
            .filter_map(|name| instance.component_repos.get(*name).cloned())
            .collect();

        // Deduplicate while preserving order.
        let mut seen = std::collections::HashSet::new();
        let mut deduped: Vec<String> = Vec::new();
        for r in mapped {
            if seen.insert(r.clone()) {
                deduped.push(r);
            }
        }

        if deduped.is_empty() {
            instance.repos.clone()
        } else {
            deduped
        }
    };

    // Append any --repo overrides (deduplicated).
    let mut seen: std::collections::HashSet<String> = repos.iter().cloned().collect();
    for r in extra_repos {
        if seen.insert(r.clone()) {
            repos.push(r);
        }
    }

    if repos.is_empty() {
        println!("No repositories configured. Add repos or component_repos to the instance in config, or use --repo.");
        return Ok(());
    }

    let summary = f["summary"].as_str().unwrap_or("");
    println!();
    println!("Commits mentioning {}", key.cyan().bold());
    if !summary.is_empty() {
        println!("{}", summary.dimmed());
    }
    if !component_names.is_empty() {
        println!("Components: {}", component_names.join(", ").bold());
    }

    for repo_path in &repos {
        let display = std::path::Path::new(repo_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(repo_path.as_str());

        let output = std::process::Command::new("git")
            .args([
                "-C",
                repo_path,
                "log",
                "--all",
                "--format=%h\t%as\t%an\t%s",
                &format!("--grep={}", key),
            ])
            .output();

        match output {
            Err(e) => {
                println!();
                println!("{} {}", display.bold(), repo_path.dimmed());
                println!("{}", "─".repeat(80));
                println!("  {} {}", "error:".red(), e);
            }
            Ok(out) if !out.status.success() => {
                let msg = String::from_utf8_lossy(&out.stderr);
                println!();
                println!("{} {}", display.bold(), repo_path.dimmed());
                println!("{}", "─".repeat(80));
                println!("  {} {}", "git error:".red(), msg.trim());
            }
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let lines: Vec<&str> = stdout.lines().collect();
                if lines.is_empty() {
                    if verbose {
                        println!();
                        println!("{} {}", display.bold(), repo_path.dimmed());
                        println!("{}", "─".repeat(80));
                        println!("  {}", "(no commits found)".dimmed());
                    }
                } else {
                    println!();
                    println!("{} {}", display.bold(), repo_path.dimmed());
                    println!("{}", "─".repeat(80));
                    for line in &lines {
                        let parts: Vec<&str> = line.splitn(4, '\t').collect();
                        match parts.as_slice() {
                            [hash, date, author, subject] => {
                                let branches = branches_containing(repo_path, hash);
                                let branch_str = if branches.is_empty() {
                                    String::new()
                                } else {
                                    format!("  [{}]", branches.join(", "))
                                };
                                println!(
                                    "  {}  {}  {}  {}{}",
                                    hash.yellow(),
                                    date.dimmed(),
                                    author.bold(),
                                    subject,
                                    branch_str.magenta()
                                );
                            }
                            _ => println!("  {}", line),
                        }
                    }
                    println!();
                    println!("  {}", format!("{} commit(s)", lines.len()).dimmed());
                }
            }
        }
    }

    println!();
    Ok(())
}

// ── Diff ─────────────────────────────────────────────────────────────────────

async fn diff(
    client: &JiraClient,
    instance: &Instance,
    key: &str,
    commit: Option<&str>,
    extra_repos: Vec<String>,
    verbose: bool,
) -> Result<()> {
    let issue: Value = client
        .get(&format!("issue/{key}?fields=components,summary"))
        .await?;
    let f = &issue["fields"];

    let component_names: Vec<&str> = f["components"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|c| c["name"].as_str()).collect())
        .unwrap_or_default();

    let mut repos: Vec<String> = {
        let mapped: Vec<String> = component_names
            .iter()
            .filter_map(|name| instance.component_repos.get(*name).cloned())
            .collect();

        let mut seen = std::collections::HashSet::new();
        let mut deduped: Vec<String> = Vec::new();
        for r in mapped {
            if seen.insert(r.clone()) {
                deduped.push(r);
            }
        }

        if deduped.is_empty() {
            instance.repos.clone()
        } else {
            deduped
        }
    };

    let mut seen: std::collections::HashSet<String> = repos.iter().cloned().collect();
    for r in extra_repos {
        if seen.insert(r.clone()) {
            repos.push(r);
        }
    }

    if repos.is_empty() {
        println!("No repositories configured. Add repos or component_repos to the instance in config, or use --repo.");
        return Ok(());
    }

    let summary = f["summary"].as_str().unwrap_or("");
    println!();
    println!("Diff for {}", key.cyan().bold());
    if !summary.is_empty() {
        println!("{}", summary.dimmed());
    }

    for repo_path in &repos {
        let display = std::path::Path::new(repo_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(repo_path.as_str());

        if let Some(hash) = commit {
            // Show diff for a single specific commit.
            let branches = branches_containing(repo_path, hash);
            let branch_str = if branches.is_empty() {
                String::new()
            } else {
                format!("  [{}]", branches.join(", "))
            };
            println!();
            println!(
                "{} {} — {}{}",
                display.bold(),
                repo_path.dimmed(),
                hash.yellow(),
                branch_str.magenta()
            );
            println!("{}", "─".repeat(80));
            show_commit_diff(repo_path, hash);
        } else {
            // Find all commits mentioning the key and show each diff.
            let output = std::process::Command::new("git")
                .args([
                    "-C",
                    repo_path,
                    "log",
                    "--all",
                    "--format=%H\t%as\t%an\t%s",
                    &format!("--grep={}", key),
                ])
                .output();

            match output {
                Err(e) => {
                    println!();
                    println!(
                        "{} {} {}",
                        display.bold(),
                        repo_path.dimmed(),
                        format!("error: {e}").red()
                    );
                }
                Ok(out) if !out.status.success() => {
                    let msg = String::from_utf8_lossy(&out.stderr);
                    println!();
                    println!(
                        "{} {} {}",
                        display.bold(),
                        repo_path.dimmed(),
                        format!("git error: {}", msg.trim()).red()
                    );
                }
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let lines: Vec<&str> = stdout.lines().collect();
                    if lines.is_empty() {
                        if verbose {
                            println!();
                            println!("{} {}", display.bold(), repo_path.dimmed());
                            println!("{}", "─".repeat(80));
                            println!("  {}", "(no commits found)".dimmed());
                        }
                    } else {
                        for line in &lines {
                            let parts: Vec<&str> = line.splitn(4, '\t').collect();
                            match parts.as_slice() {
                                [hash, date, author, subject] => {
                                    let branches = branches_containing(repo_path, hash);
                                    let branch_str = if branches.is_empty() {
                                        String::new()
                                    } else {
                                        format!("  [{}]", branches.join(", "))
                                    };
                                    println!();
                                    println!(
                                        "{} {}  {}  {}  {}{}",
                                        display.bold(),
                                        repo_path.dimmed(),
                                        hash[..8.min(hash.len())].yellow(),
                                        date.dimmed(),
                                        author.bold(),
                                        branch_str.magenta()
                                    );
                                    println!("  {}", subject);
                                    println!("{}", "─".repeat(80));
                                    show_commit_diff(repo_path, hash);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }

    println!();
    Ok(())
}

fn branches_containing(repo_path: &str, hash: &str) -> Vec<String> {
    let out = std::process::Command::new("git")
        .args(["-C", repo_path, "branch", "--all", "--contains", hash])
        .output()
        .ok();
    out.filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim_start_matches([' ', '*']).trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

async fn show_image_attachments(client: &JiraClient, attachments: &[Value]) -> Result<()> {
    let images: Vec<&Value> = attachments
        .iter()
        .filter(|a| {
            a["mimeType"]
                .as_str()
                .map(|m| m.starts_with("image/"))
                .unwrap_or(false)
        })
        .collect();

    if images.is_empty() {
        return Ok(());
    }

    println!();
    println!("{}", "Images:".bold());

    for img in images {
        let filename = img["filename"].as_str().unwrap_or("?");
        let url = img["content"].as_str().unwrap_or("");
        if url.is_empty() {
            continue;
        }

        println!("  {}", filename.bold());

        let bytes = match client.get_bytes_url(url).await {
            Ok(b) => b,
            Err(e) => {
                println!("  {}", format!("download failed: {e}").red());
                continue;
            }
        };

        let ext = std::path::Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin");
        let tmp = std::env::temp_dir().join(format!(
            "mjira_img_{}.{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            ext
        ));

        if let Err(e) = std::fs::write(&tmp, &bytes) {
            println!("  {}", format!("write failed: {e}").red());
            continue;
        }

        let status = std::process::Command::new("kitty")
            .args(["+kitten", "icat", "--scale-up", tmp.to_str().unwrap_or("")])
            .status();

        let _ = std::fs::remove_file(&tmp);

        if let Err(e) = status {
            println!("  {}", format!("kitty icat not available: {e}").red());
        }
    }

    Ok(())
}

/// Best-effort plain-text extraction from Atlassian Document Format (ADF).
/// ADF is the JSON-based rich text format used by Jira Cloud API v3.
fn adf_to_text(node: &Value) -> Option<String> {
    if node.is_null() {
        return None;
    }
    let mut out = String::new();
    collect_adf_text(node, &mut out);
    if out.is_empty() {
        None
    } else {
        Some(out.trim_end().to_string())
    }
}

fn collect_adf_text(node: &Value, out: &mut String) {
    match node["type"].as_str() {
        Some("text") => {
            if let Some(t) = node["text"].as_str() {
                out.push_str(t);
            }
        }
        Some("hardBreak") => out.push('\n'),
        Some("paragraph") | Some("heading") => {
            if let Some(content) = node["content"].as_array() {
                for child in content {
                    collect_adf_text(child, out);
                }
            }
            out.push('\n');
        }
        Some("bulletList") | Some("orderedList") => {
            if let Some(items) = node["content"].as_array() {
                for item in items {
                    out.push_str("  - ");
                    if let Some(content) = item["content"].as_array() {
                        for child in content {
                            collect_adf_text(child, out);
                        }
                    }
                }
            }
        }
        Some("codeBlock") => {
            if let Some(content) = node["content"].as_array() {
                for child in content {
                    collect_adf_text(child, out);
                }
            }
            out.push('\n');
        }
        _ => {
            if let Some(content) = node["content"].as_array() {
                for child in content {
                    collect_adf_text(child, out);
                }
            }
        }
    }
}

fn show_commit_diff(repo_path: &str, hash: &str) {
    let output = std::process::Command::new("git")
        .args(["-C", repo_path, "show", "--stat", "--patch", hash])
        .output();

    match output {
        Err(e) => println!("  {} {}", "error:".red(), e),
        Ok(out) if !out.status.success() => {
            let msg = String::from_utf8_lossy(&out.stderr);
            println!("  {} {}", "git error:".red(), msg.trim());
        }
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                if line.starts_with('+') && !line.starts_with("+++") {
                    println!("{}", line.green());
                } else if line.starts_with('-') && !line.starts_with("---") {
                    println!("{}", line.red());
                } else if line.starts_with("@@") {
                    println!("{}", line.cyan());
                } else {
                    println!("{}", line);
                }
            }
        }
    }
}

// ── Assign ───────────────────────────────────────────────────────────────────

async fn assign(client: &JiraClient, key: &str, assignee: &str) -> Result<()> {
    let body = if client.api_version() >= 3 {
        if assignee == "-" {
            json!({ "accountId": null })
        } else {
            json!({ "accountId": assignee })
        }
    } else {
        if assignee == "-" {
            json!({ "name": null })
        } else {
            json!({ "name": assignee })
        }
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
