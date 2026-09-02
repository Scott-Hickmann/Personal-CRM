mod affinity_calibration;
mod affinity_commands;
mod analysis;
mod analytics_commands;
mod auth_commands;
mod cli;
mod commands;
mod config;
mod contact_commands;
mod contact_label;
mod contact_publish;
mod daemon;
mod daemon_commands;
mod db;
mod error;
mod face_commands;
mod face_matching;
mod gmail;
mod google_contacts;
mod graph;
mod history_commands;
mod job_runner;
mod jobs;
mod ollama;
mod output;
mod person_cleanup;
mod phone;
mod photo_links;
mod photos_commands;
mod photos_faces;
mod photos_import;
mod photos_library;
mod photos_names;
mod photos_prompt;
mod photos_review;
mod progress;
mod query;
mod relationship_signals;
mod repository;
mod review;
mod review_candidates;
#[cfg(test)]
mod review_candidates_tests;
mod review_commands;
mod scoring;
mod source;
mod status_commands;
mod sync;
mod ui_commands;
mod ui_data;

use clap::Parser;

fn main() {
    let cli = cli::Cli::parse();
    if let Err(error) = cli::run(cli) {
        output::print_error(&error);
        std::process::exit(error.exit_code());
    }
}
