mod clips;
mod config;
mod daemon;
mod installer;
mod ipc;
mod job;
mod logging;
mod notify;
mod obs;
mod obs_profile;
mod processor;
mod queue;
mod setup;
mod share;
mod tui;
mod updater;
mod wizard;

use anyhow::Result;
use clap::{Parser, Subcommand};
use config::Config;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "rscapt", about = "OBS replay buffer → 1440p clip processor")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Start the background daemon (default if already configured)
    Daemon,
    /// Open the TUI to monitor jobs and manage clips
    Tui,
    /// Re-run the setup wizard (reconfigure OBS, output folder, etc.)
    Setup,
    /// Install: Start Menu shortcut + autostart (setup wizard does this automatically)
    Install,
    /// Uninstall: remove shortcuts, autostart, and PATH entry
    Uninstall,
    /// Check for updates and install if a newer version is available
    Update,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // ── First-run: no subcommand + no config → launch setup wizard ────────────
    if cli.command.is_none() && !Config::path().exists() {
        return first_run();
    }

    // ── Normal boot ───────────────────────────────────────────────────────────
    let config = Config::load()?;

    let log_dir = PathBuf::from(&config.output_dir)
        .parent()
        .unwrap_or(&PathBuf::from("."))
        .join("rscapt-logs");

    let _guard = logging::init(&log_dir)?;

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        obs = %format!("{}:{}", config.obs_host, config.obs_port),
        output_dir = %config.output_dir,
        encoder = %config.encoder,
        "rscapt starting"
    );

    match cli.command.unwrap_or(Command::Daemon) {
        Command::Daemon    => daemon::run(config).await,
        Command::Tui       => tui::run(&config).await,
        Command::Setup     => run_wizard(),
        Command::Install   => { installer::install()?; Ok(()) }
        Command::Uninstall => { installer::uninstall()?; Ok(()) }
        Command::Update    => updater::run_update().await,
    }
}

// ── First run ─────────────────────────────────────────────────────────────────

fn first_run() -> Result<()> {
    match wizard::run() {
        None => {
            println!("Setup cancelled. Run `rscapt setup` to configure at any time.");
            Ok(())
        }
        Some(opts) => {
            // Wizard exited cleanly — TUI is gone, back to plain terminal
            println!();
            setup::apply(&opts)?;
            println!();
            if opts.autostart {
                println!("All done! rscapt will start automatically on your next login.");
                println!("Run `rscapt daemon` now to start immediately.");
            } else {
                println!("All done! Run `rscapt daemon` to start the background processor.");
                println!("Run `rscapt tui` to open the clip manager.");
            }
            Ok(())
        }
    }
}

// ── Reconfigure (rscapt setup) ────────────────────────────────────────────────

fn run_wizard() -> Result<()> {
    match wizard::run() {
        None => {
            println!("Setup cancelled — existing config unchanged.");
            Ok(())
        }
        Some(opts) => {
            println!();
            setup::apply(&opts)?;
            println!();
            println!("Configuration updated.");
            Ok(())
        }
    }
}
