mod config;
mod desired;
mod execute;
mod paths;
mod plan;
mod settings;
mod state;

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};

use config::{LinkType, LoadedConfig};
use desired::Targets;
use settings::Env;
use state::State;

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

    match &cli.command {
        Command::Plan => cmd_plan(&cli),
        Command::Sync => cmd_sync(&cli),
        Command::Status | Command::Remove { .. } | Command::Adopt { .. } => {
            bail!("not implemented")
        }
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

/// Everything that planning for the current config needs.
struct Context {
    env: Env,
    loaded: LoadedConfig,
    state_dir: PathBuf,
    link_type: LinkType,
    desired: std::collections::BTreeMap<String, Targets>,
}

impl Context {
    fn load(cli: &Cli) -> Result<Self> {
        let env = Env::from_process()?;
        let loaded = LoadedConfig::load(cli.config.as_deref(), &env.cwd)?;
        let state_dir = settings::state_dir(cli.state_dir.as_deref(), &env, Some(&loaded));
        let link_type = settings::link_type(&env, Some(&loaded))?;
        let desired = desired::expand(&loaded, &env.home)?;

        Ok(Self {
            env,
            loaded,
            state_dir,
            link_type,
            desired,
        })
    }

    fn plan(&self, state: &State) -> plan::Plan {
        plan::plan(
            state,
            &self.loaded.path,
            self.link_type,
            &self.desired,
            &self.env.home,
        )
    }
}

fn print_findings(plan: &plan::Plan) {
    for note in &plan.notes {
        println!("note: {note}");
    }
    for conflict in &plan.conflicts {
        println!("conflict: {conflict}");
    }
}

fn cmd_plan(cli: &Cli) -> Result<()> {
    let ctx = Context::load(cli)?;
    let state = State::read(&ctx.state_dir)?;
    let plan = ctx.plan(&state);

    for message in plan.steps.iter().filter_map(|s| s.message.as_deref()) {
        println!("{message}");
    }
    print_findings(&plan);

    if !plan.conflicts.is_empty() {
        bail!("sync is blocked by {} conflict(s)", plan.conflicts.len());
    }
    if plan.steps.is_empty() {
        println!("nothing to do");
    }
    Ok(())
}

fn cmd_sync(cli: &Cli) -> Result<()> {
    let ctx = Context::load(cli)?;
    let mut state = State::open(&ctx.state_dir)?;
    let plan = ctx.plan(&state);

    if !plan.conflicts.is_empty() {
        print_findings(&plan);
        bail!(
            "sync stopped by {} conflict(s), nothing was changed",
            plan.conflicts.len()
        );
    }

    execute::run_steps(&plan.steps, |message| println!("{message}"))?;
    print_findings(&plan);
    if plan.steps.is_empty() {
        println!("already in sync");
    }

    execute::update_records(&mut state, plan.records);
    state.save()
}
