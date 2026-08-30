mod cli;
mod cursor_integration;
mod doctor;
mod filesystem;
mod fsmonitor;
mod git;
mod locking;
mod metadata;
mod repository;
mod runtime;
mod semantic;
mod session;
mod util;

use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

use cli::Cli;

fn main() -> Result<ExitCode> {
    let cli = Cli::parse();
    cli::run(cli)
}
