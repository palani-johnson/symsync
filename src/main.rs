mod config;
mod plan;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use config::SymsyncConfig;
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
/// The expected layout is:
///
/// ```text
/// .symsync.state/
///   sources/          ← symlinks named by SHA-1, each pointing to a source file
///   targets.toml      ← tracks which targets are managed by symsync
/// ```
///
/// If the state directory does not exist the full layout is created. If it
/// exists but does not match the expected layout an error is returned.
///
/// Returns the absolute path to the state directory.
fn ensure_state_dir(config_path: &Path) -> Result<PathBuf> {
    let root = config_path.parent().unwrap_or(Path::new("."));
    let state_dir = root.join(".symsync.state");
    let sources_dir = state_dir.join("sources");
    let targets_file = state_dir.join("targets.toml");

    if !state_dir.exists() {
        std::fs::create_dir_all(&sources_dir).with_context(|| {
            format!(
                "Failed to create sources directory {}",
                sources_dir.display()
            )
        })?;
        std::fs::write(&targets_file, "")
            .with_context(|| format!("Failed to create targets file {}", targets_file.display()))?;
    }

    if state_dir.is_dir() && sources_dir.is_dir() && targets_file.is_file() {
        Ok(state_dir)
    } else {
        bail!(
            "State directory {} must not exist, or must exist with the expected structure:\n  {}/\n  {}",
            state_dir.display(),
            sources_dir.display(),
            targets_file.display(),
        )
    }
}

/// Sync the symlink farm by executing the computed plan.
///
/// Each step is executed in order. If any step fails — either because it is an
/// [`ErrorStep`] representing a conflict, or because an IO operation fails —
/// execution stops immediately and the error is returned.
///
/// On success, [`targets.toml`] is updated to reflect the current set of
/// managed targets.
///
/// [`ErrorStep`]: plan::ErrorStep
/// [`targets.toml`]: ensure_state_dir
fn cmd_sync() -> Result<()> {
    let config_path = get_config_path()?;
    let sync_config = SymsyncConfig::parse_file(&config_path)?;
    let root_dir = config_path.parent().unwrap_or(Path::new("."));

    let state_items = StateItem::generate(root_dir, &sync_config);
    let state_dir = ensure_state_dir(&config_path)?;

    for state_item in &state_items {
        for step in plan_link(state_item, &state_dir) {
            step.execute()?;
            println!("{}", step.message());
        }
    }

    write_targets_file(&state_dir, &state_items).context("Failed to update targets.toml")?;

    Ok(())
}

/// Write `targets.toml` inside `state_dir` with the current set of managed targets.
///
/// The file is a flat TOML map from each full target path (raw config target
/// joined with the source filename, e.g. `"out/file.txt"`) to its SHA-1 hash.
/// Multiple different target paths can share the same hash if the same source
/// file is linked to more than one location.
fn write_targets_file(state_dir: &Path, state_items: &[StateItem]) -> Result<()> {
    let mut map: BTreeMap<String, String> = BTreeMap::new();

    for item in state_items {
        let target_path = PathBuf::from(&item.target).join(&item.link);
        map.insert(
            target_path.to_string_lossy().into_owned(),
            item.state_hash.clone(),
        );
    }

    let content = toml::to_string_pretty(&map).context("Failed to serialize targets.toml")?;

    std::fs::write(state_dir.join("targets.toml"), content)
        .context("Failed to write targets.toml")?;

    Ok(())
}

/// Parse the config and print it as pretty JSON.
fn cmd_config() -> Result<()> {
    let config_path = get_config_path()?;
    let sync_config = SymsyncConfig::parse_file(&config_path)?;
    println!("{}", serde_json::to_string_pretty(&sync_config)?);
    Ok(())
}

/// Print the JSON schema for the symsync config format.
fn cmd_config_gen_schema() -> Result<()> {
    let schema = schemars::schema_for!(SymsyncConfig);
    println!("{}", serde_json::to_string_pretty(&schema)?);
    Ok(())
}

/// Generate state items, print them as pretty JSON, and ensure the state directory exists.
fn cmd_plan() -> Result<()> {
    let config_path = get_config_path()?;
    let sync_config = SymsyncConfig::parse_file(&config_path)?;
    let root_dir = config_path.parent().unwrap_or(Path::new("."));

    let state_items = StateItem::generate(root_dir, &sync_config);
    println!("{}", serde_json::to_string_pretty(&state_items)?);

    ensure_state_dir(&config_path)?;

    Ok(())
}
