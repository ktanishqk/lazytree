mod cli;
mod doctor;
mod filesystem;
mod git;
mod locking;
mod metadata;
mod repository;
mod runtime;
mod semantic;
mod session;

use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

use cli::Cli;

fn main() -> Result<ExitCode> {
    let cli = Cli::parse();
    cli::run(cli)
}
