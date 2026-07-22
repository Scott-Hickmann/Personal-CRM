use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::error::Result;
use crate::output::Format;
use crate::sync::SyncTarget;

#[derive(Parser)]
#[command(name = "crm", version, about)]
pub struct Cli {
    #[arg(long, value_enum, default_value = "table", global = true)]
    format: Format,
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Doctor,
    Status,
    Person {
        #[command(subcommand)]
        command: PersonCommand,
    },
    Note {
        #[command(subcommand)]
        command: NoteCommand,
    },
    Fact {
        #[command(subcommand)]
        command: FactCommand,
    },
    Tag {
        #[command(subcommand)]
        command: TagCommand,
    },
    Sync {
        #[arg(value_enum, default_value = "all")]
        target: SyncTarget,
    },
}

#[derive(Subcommand)]
pub(crate) enum ConfigCommand {
    Init(InitArgs),
    Show,
}

#[derive(Args)]
pub(crate) struct InitArgs {
    #[arg(long)]
    pub self_name: String,
    #[arg(long = "self-email")]
    pub self_emails: Vec<String>,
    #[arg(long = "self-phone")]
    pub self_phones: Vec<String>,
}

#[derive(Subcommand)]
pub(crate) enum PersonCommand {
    Add(PersonAddArgs),
    Show(PersonReference),
}

#[derive(Subcommand)]
pub(crate) enum NoteCommand {
    Add(TextMutationArgs),
}

#[derive(Subcommand)]
pub(crate) enum FactCommand {
    Set(FactSetArgs),
}

#[derive(Subcommand)]
pub(crate) enum TagCommand {
    Add(TagAddArgs),
}

#[derive(Args)]
pub(crate) struct PersonAddArgs {
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub(crate) struct PersonReference {
    pub person: String,
}

#[derive(Args)]
pub(crate) struct TextMutationArgs {
    #[arg(long)]
    pub person: String,
    #[arg(long)]
    pub text: String,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub(crate) struct FactSetArgs {
    #[arg(long)]
    pub person: String,
    #[arg(long)]
    pub key: String,
    #[arg(long)]
    pub value: String,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub(crate) struct TagAddArgs {
    #[arg(long)]
    pub person: String,
    #[arg(long)]
    pub tag: String,
    #[arg(long)]
    pub dry_run: bool,
}

pub fn run(cli: Cli) -> Result<()> {
    crate::commands::run(cli.format, cli.config, cli.command)
}
