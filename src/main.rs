mod config;
mod desired;
mod execute;
mod health;
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
        Command::Status => cmd_status(&cli),
        Command::Remove { package } => cmd_remove(&cli, package),
        Command::Adopt { target } => cmd_adopt(&cli, target),
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
    warnings: Vec<String>,
}

impl Context {
    fn load(cli: &Cli) -> Result<Self> {
        let env = Env::from_process()?;
        let loaded = LoadedConfig::load(cli.config.as_deref(), &env.cwd)?;
        let state_dir = settings::state_dir(cli.state_dir.as_deref(), &env, Some(&loaded));
        let link_type = settings::link_type(&env, Some(&loaded))?;
        let expansion = desired::expand(&loaded, &env.home, &state_dir)?;

        Ok(Self {
            env,
            loaded,
            state_dir,
            link_type,
            desired: expansion.packages,
            warnings: expansion.warnings,
        })
    }

    fn print_warnings(&self) {
        for warning in &self.warnings {
            println!("warning: {warning}");
        }
    }

    fn plan(&self, state: &State) -> plan::Plan {
        plan::plan(
            state,
            &self.loaded.path,
            self.link_type,
            &self.desired,
            &self.env.home,
            &still_defines,
        )
    }
}

/// Whether the config at `config` still defines `package`. A config that cannot be read counts as `true`, so that sync never takes a package by mistake.
fn still_defines(config: &std::path::Path, package: &str) -> bool {
    if !config.exists() {
        return false;
    }
    match LoadedConfig::load(Some(config), std::path::Path::new("/")) {
        Ok(loaded) => loaded.config.packages.contains_key(package),
        Err(_) => true,
    }
}

fn print_findings(plan: &plan::Plan, home: &std::path::Path) {
    for note in &plan.notes {
        println!("note: {note}");
    }
    for conflict in &plan.conflicts {
        println!("conflict: {}", conflict.message(home));
    }
}

fn cmd_plan(cli: &Cli) -> Result<()> {
    let ctx = Context::load(cli)?;
    let state = State::read(&ctx.state_dir)?;
    let plan = ctx.plan(&state);
    ctx.print_warnings();

    for message in plan.steps.iter().filter_map(|s| s.message.as_deref()) {
        println!("{message}");
    }
    print_findings(&plan, &ctx.env.home);

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
    ctx.print_warnings();

    if !plan.conflicts.is_empty() {
        print_findings(&plan, &ctx.env.home);
        bail!(
            "sync stopped by {} conflict(s), nothing was changed",
            plan.conflicts.len()
        );
    }

    execute::run_steps(&plan.steps, |message| println!("{message}"))?;
    print_findings(&plan, &ctx.env.home);
    if plan.steps.is_empty() {
        println!("already in sync");
    }

    execute::update_records(&mut state, plan.records);
    state.save()
}

/// The state directory for commands that do not need a config. A config is used only if one is given or found.
fn optional_state_dir(cli: &Cli, env: &Env) -> Result<PathBuf> {
    let loaded = LoadedConfig::load_optional(cli.config.as_deref(), &env.cwd)?;
    Ok(settings::state_dir(
        cli.state_dir.as_deref(),
        env,
        loaded.as_ref(),
    ))
}

fn cmd_status(cli: &Cli) -> Result<()> {
    let env = Env::from_process()?;
    let state_dir = optional_state_dir(cli, &env)?;
    let state = State::read(&state_dir)?;
    let show = |path: &std::path::Path| paths::display(path, &env.home);

    println!("state: {}", show(&state_dir));
    if state.packages.is_empty() {
        println!("no packages");
        return Ok(());
    }

    for (name, record) in &state.packages {
        let orphaned = if still_defines(&record.config, name) {
            ""
        } else {
            "orphaned, "
        };
        println!("{name} ({orphaned}{})", show(&record.config));
        for (target, source) in &record.targets {
            let health = health::check(&state, target, source);
            println!("  {:<18} {}", health.to_string(), show(target));
        }
    }
    Ok(())
}

fn cmd_remove(cli: &Cli, package: &str) -> Result<()> {
    let env = Env::from_process()?;
    let state_dir = optional_state_dir(cli, &env)?;
    if !state_dir.exists() {
        bail!("no package {package} in {}", state_dir.display());
    }

    let mut state = State::open(&state_dir)?;
    let Some(record) = state.packages.get(package).cloned() else {
        bail!("no package {package} in {}", state_dir.display());
    };
    let plan = plan::plan_remove(&state, package, &record, &env.home);

    if !plan.conflicts.is_empty() {
        print_findings(&plan, &env.home);
        bail!(
            "remove stopped by {} conflict(s), nothing was changed",
            plan.conflicts.len()
        );
    }

    execute::run_steps(&plan.steps, |message| println!("{message}"))?;
    print_findings(&plan, &env.home);
    execute::update_records(&mut state, plan.records);
    state.save()?;

    println!("removed package {package}");
    if record.config.exists() && still_defines(&record.config, package) {
        println!(
            "note: {} still defines {package}, the next sync from it adds the package again",
            paths::display(&record.config, &env.home)
        );
    }
    Ok(())
}

fn cmd_adopt(cli: &Cli, target: &std::path::Path) -> Result<()> {
    let env = Env::from_process()?;
    let target = paths::normalize(&env.cwd.join(target));
    let state_dir = optional_state_dir(cli, &env)?;
    let not_managed = || anyhow::anyhow!("{} is not managed by symsync", target.display());
    if !state_dir.exists() {
        return Err(not_managed());
    }

    let state = State::open(&state_dir)?;
    let (name, record) = state
        .packages
        .iter()
        .find(|(_, record)| record.targets.contains_key(&target))
        .map(|(name, record)| (name.clone(), record.clone()))
        .ok_or_else(not_managed)?;

    let health = health::check(&state, &target, &record.targets[&target]);
    if health != health::Health::Drifted {
        bail!(
            "{} is not drifted, its status is: {health}",
            target.display()
        );
    }

    let plan = plan::plan_adopt(&state, &name, &record, &target, &env.home);
    if !plan.conflicts.is_empty() {
        print_findings(&plan, &env.home);
        bail!(
            "adopt stopped by {} conflict(s), nothing was changed",
            plan.conflicts.len()
        );
    }

    execute::run_steps(&plan.steps, |message| println!("{message}"))?;
    print_findings(&plan, &env.home);
    Ok(())
}
