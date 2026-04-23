use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};

mod client;
mod commands;
mod config;

use commands::{board, fields, instance, issue, project, query, search};

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

    /// Print each HTTP request to stderr
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Print full request + response bodies to stderr (implies -v)
    #[arg(short = 'V', long, global = true)]
    very_verbose: bool,

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
    /// Work with Jira boards (requires Agile/Software plugin)
    #[command(subcommand_required = true)]
    Board {
        #[command(subcommand)]
        command: board::BoardCommands,
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
    /// Run a saved query from the config file (omit name to list all)
    Query {
        /// Name of the saved query to run
        name: Option<String>,
        /// Override the result limit
        #[arg(short, long)]
        limit: Option<u32>,
        /// Override the columns to display
        #[arg(short = 'c', long)]
        columns: Option<String>,
        /// Print available columns and exit
        #[arg(long)]
        list_columns: bool,
    },
    /// Generate shell completion script
    #[command(hide = true)]
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Commands::Completions { shell } = cli.command {
        clap_complete::generate(shell, &mut Cli::command(), "mjira", &mut std::io::stdout());
        return Ok(());
    }

    let cfg = config::Config::load()?;

    match cli.command {
        Commands::Instance { command } => {
            instance::handle(command, cfg).await?;
        }
        Commands::Issue { command } => {
            let (_, inst) = cfg.get_instance(cli.instance.as_deref())?;
            let client = client::JiraClient::new(inst, cli.verbose || cli.very_verbose, cli.very_verbose)?;
            issue::handle(command, &client, inst).await?;
        }
        Commands::Project { command } => {
            let (_, inst) = cfg.get_instance(cli.instance.as_deref())?;
            let client = client::JiraClient::new(inst, cli.verbose || cli.very_verbose, cli.very_verbose)?;
            project::handle(command, &client).await?;
        }
        Commands::Board { command } => {
            let (_, inst) = cfg.get_instance(cli.instance.as_deref())?;
            let client = client::JiraClient::new(inst, cli.verbose || cli.very_verbose, cli.very_verbose)?;
            board::handle(command, &client).await?;
        }
        Commands::Search { jql, limit, columns, list_columns } => {
            let (_, inst) = cfg.get_instance(cli.instance.as_deref())?;
            let client = client::JiraClient::new(inst, cli.verbose || cli.very_verbose, cli.very_verbose)?;
            if list_columns {
                fields::print_columns(&client, fields::STATIC_COLS).await?;
            } else {
                search::run_search(&client, jql.as_deref().unwrap(), limit, columns).await?;
            }
        }
        Commands::Query { name, limit, columns, list_columns } => {
            match name {
                None => query::list(&cfg.queries),
                Some(name) => {
                    let (_, inst) = cfg.get_instance(cli.instance.as_deref())?;
                    let client = client::JiraClient::new(inst, cli.verbose || cli.very_verbose, cli.very_verbose)?;
                    if list_columns {
                        fields::print_columns(&client, fields::STATIC_COLS).await?;
                    } else {
                        query::run(&client, &cfg.queries, &name, limit, columns).await?;
                    }
                }
            }
        }
        Commands::Completions { .. } => unreachable!(),
    }

    Ok(())
}
