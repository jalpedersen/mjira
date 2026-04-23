mod snow_client;
mod snow_config;

use anyhow::Result;
use chrono::{Datelike, Local};
use clap::{Parser, Subcommand};
use colored::Colorize;
use serde_json::Value;

use snow_client::SnowClient;
use snow_config::SnowConfig;

#[derive(Parser)]
#[command(name = "snow", about = "ServiceNow CLI", version)]
struct Cli {
    /// Instance alias to use (from config)
    #[arg(short, long, global = true, env = "SNOW_INSTANCE")]
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
        /// Maximum number of records to return
        #[arg(short, long, default_value = "1000")]
        limit: u32,
    },
}

// --- Response parsing ---

#[derive(serde::Deserialize)]
struct TableResponse {
    result: Vec<Value>,
}

/// Extract the raw `value` from a ServiceNow field (which may be a plain string
/// or an object like `{"value": "...", "display_value": "..."}`).
fn field_value(record: &Value, key: &str) -> String {
    let v = &record[key];
    if let Some(s) = v.get("value").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    v.as_str().unwrap_or("").to_string()
}

/// Extract the `display_value` (falling back to `value`) from a ServiceNow field.
fn field_display(record: &Value, key: &str) -> String {
    let v = &record[key];
    if let Some(s) = v.get("display_value").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    field_value(record, key)
}

/// Parse a ServiceNow duration string ("D HH:MM:SS") to seconds.
fn parse_duration(s: &str) -> u64 {
    let s = s.trim();
    // Try plain integer seconds first
    if let Ok(n) = s.parse::<u64>() {
        return n;
    }
    // "D HH:MM:SS" format
    let (days, time) = match s.split_once(' ') {
        Some((d, t)) => (d.parse::<u64>().unwrap_or(0), t),
        None => (0, s),
    };
    let parts: Vec<u64> = time.split(':').filter_map(|p| p.parse().ok()).collect();
    let (h, m, sec) = match parts.as_slice() {
        [h, m, s] => (*h, *m, *s),
        [h, m] => (*h, *m, 0),
        [h] => (*h, 0, 0),
        _ => (0, 0, 0),
    };
    days * 86400 + h * 3600 + m * 60 + sec
}

// --- Display ---

struct WorkEntry {
    date: String,
    task: String,
    seconds: u64,
    notes: String,
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

fn print_entries(mut entries: Vec<WorkEntry>) {
    if entries.is_empty() {
        println!("No time registrations found.");
        return;
    }

    entries.sort_by(|a, b| b.date.cmp(&a.date).then(a.task.cmp(&b.task)));

    let date_w = entries.iter().map(|e| e.date.len()).max().unwrap_or(10).max(4);
    let task_w = entries.iter().map(|e| e.task.len()).max().unwrap_or(4).max(4);
    let dur_w = entries
        .iter()
        .map(|e| format_duration(e.seconds).len())
        .max()
        .unwrap_or(4)
        .max(4);

    println!(
        "{:<dw$}  {:<tw$}  {:<rw$}  {}",
        "Date".bold(), "Task".bold(), "Time".bold(), "Notes".bold(),
        dw = date_w, tw = task_w, rw = dur_w,
    );
    let sep = "─".repeat(date_w + 2 + task_w + 2 + dur_w + 2 + 40);
    println!("{}", sep);

    let total: u64 = entries.iter().map(|e| e.seconds).sum();
    for e in &entries {
        let notes: String = e.notes.chars().take(80).collect();
        println!(
            "{:<dw$}  {:<tw$}  {:<rw$}  {}",
            e.date, e.task.cyan().to_string(), format_duration(e.seconds), notes,
            dw = date_w, tw = task_w, rw = dur_w,
        );
    }
    println!("{}", "─".repeat(date_w + 2 + task_w + 2 + dur_w + 2 + 40));
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
    client: &SnowClient,
    table: &str,
    username: Option<&str>,
    from: &str,
    to: &str,
    limit: u32,
) -> Result<()> {
    let limit_str = limit.to_string();
    let query = match username {
        Some(u) => format!("sys_created_by={}^work_date>={}^work_date<={}", u, from, to),
        None => format!("work_date>={}^work_date<={}", from, to),
    };
    let params = [
        ("sysparm_query", query.as_str()),
        ("sysparm_fields", "work_date,time_worked,work_notes,task"),
        ("sysparm_display_value", "all"),
        ("sysparm_limit", limit_str.as_str()),
    ];

    let resp: TableResponse = client.get_table(table, &params).await?;

    let entries: Vec<WorkEntry> = resp
        .result
        .into_iter()
        .map(|r| {
            let date: String = field_value(&r, "work_date").chars().take(10).collect();
            let task = field_display(&r, "task");
            let seconds = parse_duration(&field_value(&r, "time_worked"));
            let notes = field_value(&r, "work_notes");
            WorkEntry { date, task, seconds, notes }
        })
        .collect();

    print_entries(entries);
    Ok(())
}

// --- Entry point ---

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = SnowConfig::load()?;
    let verbose = cli.verbose || cli.very_verbose;
    let (_, instance) = cfg.get_instance(cli.instance.as_deref())?;

    let client = SnowClient::new(instance, verbose, cli.very_verbose)?;

    match cli.command {
        Commands::Log { from, to, limit } => {
            let from = from.unwrap_or_else(week_start);
            let to = to.unwrap_or_else(today);
            let table = instance.time_table.as_deref().unwrap_or("task_time_worked");
            handle_log(&client, table, instance.username.as_deref(), &from, &to, limit).await?;
        }
    }

    Ok(())
}
