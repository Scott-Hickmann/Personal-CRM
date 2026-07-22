mod cli;
mod commands;
mod config;
mod db;
mod error;
mod output;
mod repository;
mod source;
mod sync;

use clap::Parser;

fn main() {
    let cli = cli::Cli::parse();
    if let Err(error) = cli::run(cli) {
        output::print_error(&error);
        std::process::exit(error.exit_code());
    }
}
