mod catalog;
mod cli;
mod config;
mod fsops;
mod git;
mod manifest;
mod ops;
mod upgrade;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};

fn main() {
    if let Err(err) = run() {
        eprintln!("skilldeck: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let should_notify = should_notify_after(&cli.command);
    let result = match cli.command {
        Commands::Bootstrap(args) => ops::bootstrap(args),
        Commands::Init(args) => ops::init(args),
        Commands::Install(args) => ops::install(args, false),
        Commands::InstallGroup(args) => ops::install_group(args),
        Commands::Update(args) => ops::update(args),
        Commands::Upgrade(args) => upgrade::run(upgrade::UpgradeOptions {
            yes: args.yes,
            check: args.check,
        }),
        Commands::Remove(args) => ops::remove(args),
        Commands::RemoveGroup(args) => ops::remove_group(args),
        Commands::Doctor(args) => ops::doctor(args),
        Commands::List(args) => ops::list(args),
        Commands::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    };
    if result.is_ok() && should_notify {
        upgrade::maybe_notify();
    }
    result
}

fn should_notify_after(command: &Commands) -> bool {
    match command {
        Commands::Upgrade(_) | Commands::Version => false,
        Commands::List(args) if args.json => false,
        _ => true,
    }
}
