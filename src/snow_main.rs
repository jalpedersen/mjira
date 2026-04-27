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
    /// Open a browser window, complete SSO login, and save session cookies to config
    Login {},
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

// --- Login ---

fn find_chrome() -> Option<std::path::PathBuf> {
    if let Ok(path) = std::env::var("CHROME_PATH") {
        let p = std::path::PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }
    let candidates: &[&str] = &[
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/snap/bin/chromium",
    ];
    candidates.iter().map(std::path::PathBuf::from).find(|p| p.exists())
}

async fn handle_login(instance_name: &str, instance: &snow_config::SnowInstance) -> Result<()> {
    use chromiumoxide::{Browser, BrowserConfig};
    use futures::StreamExt;

    let chrome_path = find_chrome().ok_or_else(|| {
        anyhow::anyhow!(
            "Chrome/Chromium not found. Install Google Chrome or set the CHROME_PATH env var."
        )
    })?;

    eprintln!("Launching Chrome at {} ...", instance.url);

    let config = BrowserConfig::builder()
        .chrome_executable(chrome_path)
        .with_head()
        .build()
        .map_err(|e| anyhow::anyhow!("Browser config error: {}", e))?;

    let (browser, mut handler) = Browser::launch(config).await?;

    tokio::spawn(async move {
        while handler.next().await.is_some() {}
    });

    let page = browser.new_page(&instance.url).await?;

    println!("Complete sign-in in the browser window.");
    println!("Press Enter when you are fully logged in...");
    let mut buf = String::new();
    std::io::stdin().read_line(&mut buf)?;

    use chromiumoxide::cdp::browser_protocol::network::GetCookiesParams;
    let resp = page
        .execute(GetCookiesParams { urls: Some(vec![instance.url.clone()]) })
        .await?;

    let cookies: Vec<String> = resp
        .cookies
        .iter()
        .map(|c| format!("{}={}", c.name, c.value))
        .collect();

    if cookies.is_empty() {
        anyhow::bail!("No cookies found for {}. Did you complete the sign-in?", instance.url);
    }

    let csrf: Option<String> = page
        .evaluate("window.g_ck || ''")
        .await
        .ok()
        .and_then(|v| v.into_value::<String>().ok())
        .filter(|s| !s.is_empty());

    let mut cfg = snow_config::SnowConfig::load()?;
    let inst = cfg
        .instances
        .get_mut(instance_name)
        .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found in config", instance_name))?;
    inst.cookie = Some(cookies.join("; "));
    inst.x_user_token = csrf.clone();
    cfg.save()?;

    println!("Saved {} session cookie(s).", cookies.len());
    if csrf.is_some() {
        println!("Saved X-UserToken (g_ck).");
    }

    Ok(())
}

// --- Entry point ---

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = SnowConfig::load()?;
    let verbose = cli.verbose || cli.very_verbose;
    let (instance_name, instance) = cfg.get_instance(cli.instance.as_deref())?;

    match cli.command {
        Commands::Login {} => {
            handle_login(instance_name, instance).await?;
        }
        Commands::Log { from, to, limit } => {
            let client = SnowClient::new(instance, verbose, cli.very_verbose)?;
            let from = from.unwrap_or_else(week_start);
            let to = to.unwrap_or_else(today);
            let table = instance.time_table.as_deref().unwrap_or("task_time_worked");
            handle_log(&client, table, instance.username.as_deref(), &from, &to, limit).await?;
        }
    }

    Ok(())
}
