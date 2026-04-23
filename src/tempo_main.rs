#[allow(dead_code)]
mod client;
#[allow(dead_code)]
mod config;
mod tempo_client;

use anyhow::Result;
use chrono::{Datelike, Local};
use clap::{Parser, Subcommand};
use colored::Colorize;
use serde::Deserialize;

use client::JiraClient;
use config::{Config, Instance};
use tempo_client::TempoClient;

#[derive(Parser)]
#[command(name = "tempo", about = "Tempo time tracking CLI", version)]
struct Cli {
    /// Instance alias to use (from config)
    #[arg(short, long, global = true, env = "JIRA_INSTANCE")]
    instance: Option<String>,
    /// Print each HTTP request to stderr
    #[arg(short, long, global = true)]
    verbose: bool,
    /// Print full request + response bodies (implies -v)
    #[arg(long, global = true)]
    very_verbose: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List your time registrations
    Log {
        /// Start date (YYYY-MM-DD), defaults to Monday of the current week
        #[arg(long)]
        from: Option<String>,
        /// End date (YYYY-MM-DD), defaults to today
        #[arg(long)]
        to: Option<String>,
        /// Maximum number of worklogs to return
        #[arg(short, long, default_value = "1000")]
        limit: u32,
    },
}

// --- Jira API ---

#[derive(Deserialize)]
struct Myself {
    #[serde(rename = "accountId")]
    account_id: String,
}

#[derive(Deserialize)]
struct IssueSearchResponse {
    issues: Vec<IssueRef>,
}

#[derive(Deserialize)]
struct IssueRef {
    id: String,
    key: String,
}

// --- Tempo Cloud response (api.tempo.io/4) ---

#[derive(Deserialize)]
struct CloudResponse {
    results: Vec<CloudWorklog>,
}

#[derive(Deserialize)]
struct CloudWorklog {
    issue: CloudWorklogIssue,
    author: CloudAuthor,
    #[serde(rename = "timeSpentSeconds")]
    time_spent_seconds: u64,
    #[serde(rename = "startDate")]
    start_date: String,
    description: Option<String>,
}

// Cloud returns only the numeric id, not the key
#[derive(Deserialize)]
struct CloudWorklogIssue {
    id: u64,
}

#[derive(Deserialize)]
struct CloudAuthor {
    #[serde(rename = "accountId")]
    account_id: String,
}

// --- Tempo Data Center/Server response (tempo-timesheets/4) ---

type DcResponse = Vec<DcWorklog>;

#[derive(Deserialize)]
struct DcWorklog {
    issue: WorklogIssue,
    #[serde(rename = "timeSpentSeconds")]
    time_spent_seconds: u64,
    // Older DC versions use "dateStarted", newer may use "startDate"
    #[serde(rename = "dateStarted", alias = "startDate")]
    date_started: String,
    comment: Option<String>,
}

#[derive(Deserialize)]
struct WorklogIssue {
    key: String,
}

// --- Unified display ---

struct Worklog {
    date: String,
    issue_key: String,
    seconds: u64,
    description: String,
}

fn format_duration(seconds: u64) -> String {
    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    match (h, m) {
        (0, m) => format!("{}m", m),
        (h, 0) => format!("{}h", h),
        (h, m) => format!("{}h {}m", h, m),
    }
}

fn print_worklogs(mut worklogs: Vec<Worklog>) {
    if worklogs.is_empty() {
        println!("No time registrations found.");
        return;
    }

    worklogs.sort_by(|a, b| b.date.cmp(&a.date).then(a.issue_key.cmp(&b.issue_key)));

    let date_w = worklogs.iter().map(|w| w.date.len()).max().unwrap_or(10).max(4);
    let key_w = worklogs.iter().map(|w| w.issue_key.len()).max().unwrap_or(5).max(5);
    let dur_w = worklogs
        .iter()
        .map(|w| format_duration(w.seconds).len())
        .max()
        .unwrap_or(4)
        .max(4);

    println!(
        "{:<dw$}  {:<kw$}  {:<tw$}  {}",
        "Date".bold(),
        "Issue".bold(),
        "Time".bold(),
        "Description".bold(),
        dw = date_w,
        kw = key_w,
        tw = dur_w,
    );
    let sep_len = date_w + 2 + key_w + 2 + dur_w + 2 + 40;
    let sep = "─".repeat(sep_len);
    println!("{}", sep);

    let total: u64 = worklogs.iter().map(|w| w.seconds).sum();
    for w in &worklogs {
        let desc: String = w.description.chars().take(80).collect();
        println!(
            "{:<dw$}  {:<kw$}  {:<tw$}  {}",
            w.date,
            w.issue_key.cyan().to_string(),
            format_duration(w.seconds),
            desc,
            dw = date_w,
            kw = key_w,
            tw = dur_w,
        );
    }
    println!("{}", sep);
    println!("Total: {}", format_duration(total).bold());
}

// --- Date helpers ---

fn week_start() -> String {
    let today = Local::now().date_naive();
    let monday = today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64);
    monday.format("%Y-%m-%d").to_string()
}

fn today() -> String {
    Local::now().date_naive().format("%Y-%m-%d").to_string()
}

// --- Command handler ---

async fn handle_log(
    jira: &JiraClient,
    tempo: &TempoClient,
    instance: &Instance,
    from: &str,
    to: &str,
    limit: u32,
) -> Result<()> {
    let limit_str = limit.to_string();

    let worklogs: Vec<Worklog> = if tempo.is_cloud {
        let myself: Myself = jira.get("myself").await?;
        let params = [
            ("from", from),
            ("to", to),
            ("authorAccountId", myself.account_id.as_str()),
            ("limit", limit_str.as_str()),
        ];
        let resp: CloudResponse = tempo.get_with_params("worklogs", &params).await?;

        // authorAccountId query param is ignored by the API — filter client-side
        let results: Vec<CloudWorklog> = resp.results
            .into_iter()
            .filter(|w| w.author.account_id == myself.account_id)
            .collect();

        // Tempo v4 Cloud returns only issue.id — resolve keys via Jira search
        let unique_ids: Vec<String> = {
            let mut ids: Vec<u64> = results.iter().map(|w| w.issue.id).collect();
            ids.sort_unstable();
            ids.dedup();
            ids.into_iter().map(|id| id.to_string()).collect()
        };
        let key_map: std::collections::HashMap<u64, String> = if unique_ids.is_empty() {
            Default::default()
        } else {
            let jql = format!("id in ({})", unique_ids.join(","));
            let max = unique_ids.len().to_string();
            let search: IssueSearchResponse = jira
                .get_with_params(jira.search_path(), &[("jql", &jql), ("fields", "key"), ("maxResults", &max)])
                .await?;
            search.issues.into_iter()
                .filter_map(|i| i.id.parse::<u64>().ok().map(|id| (id, i.key)))
                .collect()
        };

        results
            .into_iter()
            .map(|w| {
                let key = key_map.get(&w.issue.id).cloned().unwrap_or_else(|| w.issue.id.to_string());
                Worklog {
                    date: w.start_date,
                    issue_key: key,
                    seconds: w.time_spent_seconds,
                    description: w.description.unwrap_or_default(),
                }
            })
            .collect()
    } else {
        let params = [
            ("dateFrom", from),
            ("dateTo", to),
            ("worker", instance.username.as_str()),
            ("limit", limit_str.as_str()),
        ];
        let resp: DcResponse = tempo.get_with_params("worklogs", &params).await?;
        resp.into_iter()
            .map(|w| Worklog {
                date: w.date_started.chars().take(10).collect(),
                issue_key: w.issue.key,
                seconds: w.time_spent_seconds,
                description: w.comment.unwrap_or_default(),
            })
            .collect()
    };

    print_worklogs(worklogs);
    Ok(())
}

// --- Entry point ---

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = Config::load()?;
    let verbose = cli.verbose || cli.very_verbose;
    let (_, instance) = cfg.get_instance(cli.instance.as_deref())?;

    let jira = JiraClient::new(instance, verbose, cli.very_verbose)?;
    let tempo = TempoClient::new(instance, verbose, cli.very_verbose)?;

    match cli.command {
        Commands::Log { from, to, limit } => {
            let from = from.unwrap_or_else(week_start);
            let to = to.unwrap_or_else(today);
            handle_log(&jira, &tempo, instance, &from, &to, limit).await?;
        }
    }

    Ok(())
}
