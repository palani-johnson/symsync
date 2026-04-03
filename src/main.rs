mod config;
mod plan;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use config::SymsyncSyncConfig;
use plan::{StateItem, plan_link};

/// A symlink farm manager.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Show the plan of sync operations.
    Plan,

    /// Sync the symlink farm.
    Sync,

    /// Print the symsync config.
    Config,

    /// Generate the JSON schema for the symsync config format.
    GenConfigSchema,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Plan => cmd_plan(),
        Command::Sync => cmd_sync(),
        Command::Config => cmd_config(),
        Command::GenConfigSchema => cmd_config_gen_schema(),
    }
}

/// Find the symsync config file in the current directory.
///
/// Tries each supported filename in order:
/// `symsync.toml`, `symsync.json`, `symsync.jsonc`, `symsync.json5`, `symsync.yaml`, `symsync.yml`
fn get_config_path() -> Result<PathBuf> {
    let candidates = [
        "symsync.toml",
        "symsync.json",
        "symsync.jsonc",
        "symsync.json5",
        "symsync.yaml",
        "symsync.yml",
    ];

    candidates
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No config file found. Tried:\n{}",
                candidates
                    .iter()
                    .map(|p| format!("  - {p}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        })
}

/// Ensure the `.symsync.state` directory exists next to the config file.
///
/// The state directory is a flat directory — symsync manages its contents
/// directly with no subdirectories. If the directory does not exist it is
/// created. If a non-directory already exists at that path an error is returned.
///
/// Returns the absolute path to the state directory.
fn ensure_state_dir(config_path: &Path) -> Result<PathBuf> {
    let root = config_path.parent().unwrap_or(Path::new("."));
    let state_dir = root.join(".symsync.state");

    if !state_dir.exists() {
        std::fs::create_dir_all(&state_dir)
            .with_context(|| format!("Failed to create state directory {}", state_dir.display()))?;
    }

    if state_dir.is_dir() {
        Ok(state_dir)
    } else {
        bail!(
            "State path {} exists but is not a directory",
            state_dir.display()
        )
    }
}

/// Run the sync
fn cmd_sync() -> Result<()> {
    let config_path = get_config_path()?;
    let sync_config = SymsyncSyncConfig::parse_file(&config_path)?;
    let root_dir = config_path.parent().unwrap_or(Path::new("."));

    let state_items = StateItem::generate(root_dir, &sync_config);
    let state_dir = ensure_state_dir(&config_path)?;

    for state_item in &state_items {
        for step in plan_link(state_item, &state_dir) {
            println!("{}", step.message());
        }
    }

    Ok(())
}

/// Parse the config and print it as pretty JSON.
fn cmd_config() -> Result<()> {
    let config_path = get_config_path()?;
    let sync_config = SymsyncSyncConfig::parse_file(&config_path)?;
    println!("{}", serde_json::to_string_pretty(&sync_config)?);
    Ok(())
}

/// Print the JSON schema for the symsync config format.
fn cmd_config_gen_schema() -> Result<()> {
    let schema = schemars::schema_for!(SymsyncSyncConfig);
    println!("{}", serde_json::to_string_pretty(&schema)?);
    Ok(())
}

/// Generate state items, print them as pretty JSON, and ensure the state directory exists.
fn cmd_plan() -> Result<()> {
    let config_path = get_config_path()?;
    let sync_config = SymsyncSyncConfig::parse_file(&config_path)?;
    let root_dir = config_path.parent().unwrap_or(Path::new("."));

    let state_items = StateItem::generate(root_dir, &sync_config);
    println!("{}", serde_json::to_string_pretty(&state_items)?);

    ensure_state_dir(&config_path)?;

    Ok(())
}
