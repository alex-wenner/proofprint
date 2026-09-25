//! ProofPrint command-line entry point.

mod args;
mod commands;

use clap::Parser;

fn main() -> anyhow::Result<()> {
    commands::run(&args::Cli::parse())
}
