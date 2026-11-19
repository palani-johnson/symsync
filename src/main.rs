mod config;
mod paths;
mod settings;

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};

use config::LoadedConfig;

/// A stateful symlink farm manager.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// Path to the config file.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Path to the state directory.
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Show the changes that sync would make.
    Plan,

    /// Apply the plan for all packages in the config.
    Sync,

    /// Show all packages and the health of their targets.
    Status,

    /// Remove a package and all its links, using only the state.
    Remove {
        /// Name of the package.
        package: String,
    },

    /// Move a drifted target back into its source and restore the link.
    Adopt {
        /// Path of the drifted target.
        target: PathBuf,
    },

    /// Print the parsed config.
    Config,

    /// Print the JSON schema of the config.
    GenConfigSchema,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Plan
        | Command::Sync
        | Command::Status
        | Command::Remove { .. }
        | Command::Adopt { .. } => bail!("not implemented"),
        Command::Config => {
            let loaded = LoadedConfig::load(cli.config.as_deref(), &std::env::current_dir()?)?;
            println!("{}", serde_json::to_string_pretty(&loaded.config)?);
            Ok(())
        }
        Command::GenConfigSchema => {
            println!("{}", serde_json::to_string_pretty(&config::schema())?);
            Ok(())
        }
    }
}
