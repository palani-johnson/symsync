mod config;
mod plan;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use config::SymsyncConfig;
use plan::{StateItem, plan_link, plan_unlink};

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
/// Stale targets — entries present in `targets.toml` that no longer appear in
/// the config — are unlinked first, then the current set of links is created.
/// On success, [`targets.toml`] is updated to reflect the current set of
/// managed targets.
///
/// [`ErrorStep`]: plan::ErrorStep
/// [`targets.toml`]: ensure_state_dir
fn cmd_sync() -> Result<()> {
    let config_path = get_config_path()?;
    let sync_config = SymsyncConfig::parse_file(&config_path)?;
    let root_dir = config_path.parent().unwrap_or(Path::new("."));

    let state_items = StateItem::generate(root_dir, &sync_config)?;
    let state_dir = ensure_state_dir(&config_path)?;

    // --- Unlink stale targets ---
    let old_targets = get_targets(&state_dir);
    let new_target_paths: std::collections::HashSet<String> = state_items
        .iter()
        .map(|item| {
            PathBuf::from(&item.target)
                .join(&item.link)
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let active_hashes: std::collections::HashSet<&str> = state_items
        .iter()
        .map(|item| item.state_hash.as_str())
        .collect();

    let mut did_anything = false;
    let mut printed = false;

    for (target_path_str, state_hash) in &old_targets {
        if !new_target_paths.contains(target_path_str) {
            let target_path = PathBuf::from(target_path_str);
            // Only remove the state link when no active target shares this hash.
            let hash_still_needed = active_hashes.contains(state_hash.as_str());
            for step in plan_unlink(&target_path, state_hash, &state_dir, hash_still_needed) {
                step.execute()?;
                if !step.is_noop() {
                    did_anything = true;
                }
                if let Some(msg) = step.message() {
                    println!("{msg}");
                    printed = true;
                }
            }
            // Notify the user if the parent directory is now empty.
            if let Some(parent) = target_path.parent() {
                if parent.is_dir()
                    && std::fs::read_dir(parent)
                        .map(|mut d| d.next().is_none())
                        .unwrap_or(false)
                {
                    println!(
                        "note: {} is now empty, you may want to remove it",
                        parent.display()
                    );
                }
            }
        }
    }

    // --- Link current targets ---
    for state_item in &state_items {
        for step in plan_link(state_item, &state_dir) {
            step.execute()?;
            if !step.is_noop() {
                did_anything = true;
            }
            if let Some(msg) = step.message() {
                println!("{msg}");
                printed = true;
            }
        }
    }

    if !did_anything {
        println!("already in sync");
    } else if !printed {
        println!("synced");
    }

    write_targets_file(&state_dir, &state_items).context("Failed to update targets.toml")?;

    Ok(())
}

/// Read `targets.toml` from `state_dir` and return its contents as a map from
/// target path to state hash. Returns an empty map if the file is missing or
/// unreadable, or if parsing fails.
fn get_targets(state_dir: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(state_dir.join("targets.toml"))
        .ok()
        .and_then(|content| toml::from_str(&content).ok())
        .unwrap_or_default()
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

/// Show the full plan: unlink steps for stale targets followed by link steps for
/// current targets. Each step is printed as a single human-readable line.
///
/// If no `targets.toml` exists yet (first run), the unlink phase is skipped.
fn cmd_plan() -> Result<()> {
    let config_path = get_config_path()?;
    let sync_config = SymsyncConfig::parse_file(&config_path)?;
    let root_dir = config_path
        .parent()
        .with_context(|| "Failed to get config directory")?;

    let state_items = StateItem::generate(root_dir, &sync_config)?;

    // Try to locate the state directory so we can read old targets. It is fine
    // if it does not exist yet — that just means there is nothing to unlink.
    let state_dir_opt = {
        let candidate = root_dir.join(".symsync.state");
        if candidate.is_dir() {
            Some(candidate)
        } else {
            None
        }
    };

    // --- Plan: unlink stale targets ---
    if let Some(ref state_dir) = state_dir_opt {
        let old_targets = get_targets(state_dir);
        let new_target_paths: std::collections::HashSet<String> = state_items
            .iter()
            .map(|item| {
                PathBuf::from(&item.target)
                    .join(&item.link)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        let active_hashes: std::collections::HashSet<&str> = state_items
            .iter()
            .map(|item| item.state_hash.as_str())
            .collect();

        for (target_path_str, state_hash) in &old_targets {
            if !new_target_paths.contains(target_path_str) {
                let target_path = PathBuf::from(target_path_str);
                let hash_still_needed = active_hashes.contains(state_hash.as_str());
                for step in plan_unlink(&target_path, state_hash, state_dir, hash_still_needed) {
                    if let Some(msg) = step.message() {
                        println!("{msg}");
                    }
                }
            }
        }
    }

    // --- Plan: link current targets ---
    let state_dir_for_plan = state_dir_opt.unwrap_or_else(|| root_dir.join(".symsync.state"));
    let mut did_anything = false;
    for state_item in &state_items {
        for step in plan_link(state_item, &state_dir_for_plan) {
            if let Some(msg) = step.message() {
                println!("{msg}");
                did_anything = true;
            }
        }
    }

    if !did_anything {
        println!("nothing to do");
    }

    Ok(())
}
