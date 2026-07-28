use std::path::PathBuf;

use clap::{Parser, Subcommand};

mod audio;
mod chunk;
mod commands;
mod config;
mod convert;
mod domain;
mod embed;
mod hashing;
mod ignore_spec;
mod lock;
mod mcp_server;
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
mod ort_runtime;
mod scan;
mod skill;
mod store;
mod tokenizer;
mod types;
mod vector_store;
mod walk;
mod whisper_model;
mod work_file;

#[derive(Parser)]
#[command(name = "brd", version, about = "Local knowledge/research MCP server")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize .brained/, config, and the Claude skill file in the current directory
    Init,
    /// Print the `claude mcp add` command to register this binary as an MCP server
    Install,
    /// Scan a folder once (defaults to the current directory)
    Scan { path: Option<PathBuf> },
    /// Run the MCP server, with a background rescanning thread (defaults to the current directory)
    Mcp { path: Option<PathBuf> },
    /// Show model cache location, version, and indexed file count
    Info,
    /// Human override: lock the tech domain (defaults to the current directory)
    Locktech { path: Option<PathBuf> },
    /// Human override: unlock the tech domain (defaults to the current directory)
    Unlocktech { path: Option<PathBuf> },
    /// Human override: lock the business domain (defaults to the current directory)
    Lockbusiness { path: Option<PathBuf> },
    /// Human override: unlock the business domain (defaults to the current directory)
    Unlockbusiness { path: Option<PathBuf> },
}

fn resolve_root(path: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    match path {
        Some(p) => Ok(p),
        None => std::env::current_dir().map_err(Into::into),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;
    match cli.command {
        Commands::Init => commands::cmd_init(&cwd),
        Commands::Install => commands::cmd_install(&cwd),
        Commands::Scan { path } => commands::cmd_scan(&resolve_root(path)?).await,
        Commands::Mcp { path } => commands::cmd_mcp(&resolve_root(path)?).await,
        Commands::Info => commands::cmd_info(&cwd),
        Commands::Locktech { path } => commands::cmd_cli_lock(&resolve_root(path)?, domain::Domain::Tech),
        Commands::Unlocktech { path } => commands::cmd_cli_unlock(&resolve_root(path)?, domain::Domain::Tech),
        Commands::Lockbusiness { path } => commands::cmd_cli_lock(&resolve_root(path)?, domain::Domain::Business),
        Commands::Unlockbusiness { path } => commands::cmd_cli_unlock(&resolve_root(path)?, domain::Domain::Business),
    }
}
