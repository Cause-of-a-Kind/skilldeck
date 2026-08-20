mod catalog;
mod cli;
mod config;
mod fsops;
mod git;
mod manifest;
mod ops;

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
    match cli.command {
        Commands::Init(args) => ops::init(args),
        Commands::Install(args) => ops::install(args, false),
        Commands::InstallGroup(args) => ops::install_group(args),
        Commands::Update(args) => ops::update(args),
        Commands::Remove(args) => ops::remove(args),
        Commands::RemoveGroup(args) => ops::remove_group(args),
        Commands::Doctor(args) => ops::doctor(args),
        Commands::List(args) => ops::list(args),
        Commands::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}
