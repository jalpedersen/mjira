use anyhow::Result;
use clap::{Parser, Subcommand};

mod client;
mod commands;
mod config;

use commands::{fields, instance, issue, project, search};

#[derive(Parser)]
#[command(
    name = "jira",
    about = "Jira CLI — manage issues across multiple Jira instances",
    version
)]
struct Cli {
    /// Instance alias to use (overrides default_instance in config)
    #[arg(short, long, global = true, env = "JIRA_INSTANCE")]
    instance: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage configured Jira instances
    #[command(subcommand_required = true)]
    Instance {
        #[command(subcommand)]
        command: instance::InstanceCommands,
    },
    /// Work with Jira issues
    #[command(subcommand_required = true)]
    Issue {
        #[command(subcommand)]
        command: issue::IssueCommands,
    },
    /// Work with Jira projects
    #[command(subcommand_required = true)]
    Project {
        #[command(subcommand)]
        command: project::ProjectCommands,
    },
    /// Search issues using JQL
    Search {
        /// JQL query string (e.g. 'project = PROJ AND status = "In Progress"')
        #[arg(required_unless_present = "list_columns")]
        jql: Option<String>,
        /// Maximum results to return
        #[arg(short, long, default_value = "25")]
        limit: u32,
        /// Columns to display, comma-separated (use --list-columns to see options)
        #[arg(short = 'c', long)]
        columns: Option<String>,
        /// Print available columns and exit
        #[arg(long)]
        list_columns: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = config::Config::load()?;

    match cli.command {
        Commands::Instance { command } => {
            instance::handle(command, cfg).await?;
        }
        Commands::Issue { command } => {
            let (_, inst) = cfg.get_instance(cli.instance.as_deref())?;
            let client = client::JiraClient::new(inst)?;
            issue::handle(command, &client).await?;
        }
        Commands::Project { command } => {
            let (_, inst) = cfg.get_instance(cli.instance.as_deref())?;
            let client = client::JiraClient::new(inst)?;
            project::handle(command, &client).await?;
        }
        Commands::Search { jql, limit, columns, list_columns } => {
            let (_, inst) = cfg.get_instance(cli.instance.as_deref())?;
            let client = client::JiraClient::new(inst)?;
            if list_columns {
                fields::print_columns(&client, fields::STATIC_COLS).await?;
            } else {
                search::run_search(&client, jql.as_deref().unwrap(), limit, columns).await?;
            }
        }
    }

    Ok(())
}
