use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};

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
        | Command::Adopt { .. }
        | Command::Config
        | Command::GenConfigSchema => bail!("not implemented"),
    }
}
