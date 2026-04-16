use anyhow::{bail, Result};
use clap::Subcommand;
use colored::Colorize;
use std::io::{self, Write};

use crate::config::{config_path, Config, Instance};

#[derive(Subcommand)]
pub enum InstanceCommands {
    /// List all configured instances
    List,
    /// Add or update a Jira instance
    Add {
        /// Alias for this instance (used with --instance flag)
        name: String,
        /// Base URL, e.g. https://mycompany.atlassian.net
        #[arg(long)]
        url: Option<String>,
        /// Username or email
        #[arg(long)]
        username: Option<String>,
        /// API token (Jira Cloud) — preferred over password
        #[arg(long)]
        api_key: Option<String>,
        /// Password (Jira Server)
        #[arg(long)]
        password: Option<String>,
        /// REST API version (default: 2)
        #[arg(long, default_value = "2")]
        api_version: u8,
    },
    /// Remove a configured instance
    Remove {
        name: String,
    },
    /// Set the default instance
    SetDefault {
        name: String,
    },
    /// Show config file path
    Path,
}

pub async fn handle(cmd: InstanceCommands, mut cfg: Config) -> Result<()> {
    match cmd {
        InstanceCommands::List => list(&cfg),

        InstanceCommands::Add {
            name,
            url,
            username,
            api_key,
            password,
            api_version,
        } => {
            let url = match url {
                Some(u) => u,
                None => prompt("URL (e.g. https://mycompany.atlassian.net)")?,
            };
            let username = match username {
                Some(u) => u,
                None => prompt("Username / Email")?,
            };
            let (api_key, password) = if api_key.is_none() && password.is_none() {
                let choice = prompt("Auth type — [a]pi-key or [p]assword? [a]")?;
                if choice.trim().eq_ignore_ascii_case("p") {
                    (None, Some(prompt_secret("Password")?))
                } else {
                    (Some(prompt_secret("API key")?), None)
                }
            } else {
                (api_key, password)
            };

            if api_key.is_none() && password.is_none() {
                bail!("Must provide either --api-key or --password");
            }

            cfg.instances.insert(
                name.clone(),
                Instance {
                    url,
                    username,
                    api_key,
                    password,
                    pat: None,
                    api_version,
                    default_assignee: None,
                    repos: Vec::new(),
                    component_repos: std::collections::HashMap::new(),
                },
            );
            if cfg.default_instance.is_none() {
                cfg.default_instance = Some(name.clone());
                println!("Set '{}' as default instance.", name.green());
            }
            cfg.save()?;
            println!("Instance '{}' saved.", name.green());
        }

        InstanceCommands::Remove { name } => {
            if cfg.instances.remove(&name).is_none() {
                bail!("Instance '{}' not found", name);
            }
            if cfg.default_instance.as_deref() == Some(&name) {
                cfg.default_instance = None;
            }
            cfg.save()?;
            println!("Instance '{}' removed.", name.yellow());
        }

        InstanceCommands::SetDefault { name } => {
            if !cfg.instances.contains_key(&name) {
                bail!("Instance '{}' not found", name);
            }
            cfg.default_instance = Some(name.clone());
            cfg.save()?;
            println!("Default instance set to '{}'.", name.green());
        }

        InstanceCommands::Path => {
            println!("{}", config_path().display());
        }
    }
    Ok(())
}

fn list(cfg: &Config) {
    if cfg.instances.is_empty() {
        println!("No instances configured. Run: jira instance add <name>");
        return;
    }
    let mut names: Vec<&String> = cfg.instances.keys().collect();
    names.sort();
    println!("{:<20} {:<8} {}", "NAME".bold(), "API VER".bold(), "URL".bold());
    println!("{}", "─".repeat(60));
    for name in names {
        let inst = &cfg.instances[name];
        let is_default = cfg.default_instance.as_deref() == Some(name.as_str());
        let label = if is_default {
            format!("{} {}", name.green(), "(default)".dimmed())
        } else {
            name.normal().to_string()
        };
        println!("{:<28} v{:<6} {}", label, inst.api_version, inst.url);
    }
}

fn prompt(label: &str) -> Result<String> {
    print!("{}: ", label.bold());
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

fn prompt_secret(label: &str) -> Result<String> {
    // Use rpassword if available; fall back to plain prompt
    print!("{} (hidden): ", label.bold());
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}
