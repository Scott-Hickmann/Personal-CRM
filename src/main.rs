mod analysis;
mod analytics_commands;
mod auth_commands;
mod cli;
mod commands;
mod config;
mod db;
mod error;
mod gmail;
mod graph;
mod history_commands;
mod ollama;
mod output;
mod query;
mod repository;
mod scoring;
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
