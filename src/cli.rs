use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::error::Result;
use crate::output::Format;
use crate::query::Entity;
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
    Query(QueryArgs),
    History(HistoryArgs),
    Analyze(AnalyzeArgs),
    Explain(PersonReference),
    Graph(GraphArgs),
    Face {
        #[command(subcommand)]
        command: FaceCommand,
    },
    Photos {
        #[command(subcommand)]
        command: PhotosCommand,
    },
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
}

#[derive(Subcommand)]
pub(crate) enum FaceCommand {
    Match(FaceMatchArgs),
}

#[derive(Args)]
pub(crate) struct FaceMatchArgs {
    pub photo: PathBuf,
    #[arg(long)]
    pub library: Option<PathBuf>,
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u32).range(1..=100))]
    pub limit: u32,
}

#[derive(Subcommand)]
pub(crate) enum PhotosCommand {
    Status,
    Review(PhotosReviewArgs),
    Reconcile(PhotosLibraryArgs),
}

#[derive(Args)]
pub(crate) struct PhotosLibraryArgs {
    #[arg(long)]
    pub library: Option<PathBuf>,
}

#[derive(Args)]
pub(crate) struct PhotosReviewArgs {
    #[arg(long)]
    pub person: Option<String>,
    #[arg(long, requires = "person")]
    pub photo: Option<PathBuf>,
    #[arg(long)]
    pub library: Option<PathBuf>,
}

#[derive(Args)]
pub(crate) struct HistoryArgs {
    pub person: String,
    #[arg(long)]
    pub channel: Option<String>,
    #[arg(long, default_value_t = 50)]
    pub limit: u32,
}

#[derive(Args)]
pub(crate) struct AnalyzeArgs {
    #[arg(long, default_value_t = 20)]
    pub limit: u32,
}

#[derive(Args)]
pub(crate) struct GraphArgs {
    pub person: Option<String>,
    #[arg(long, default_value_t = 0.7)]
    pub min_confidence: f64,
}

#[derive(Subcommand)]
pub(crate) enum AuthCommand {
    Gmail {
        #[command(subcommand)]
        command: GmailCommand,
    },
}

#[derive(Subcommand)]
pub(crate) enum GmailCommand {
    Add {
        #[arg(long)]
        credentials: PathBuf,
    },
    List,
    Remove {
        account: String,
    },
}

#[derive(Args)]
pub(crate) struct QueryArgs {
    #[arg(value_enum)]
    pub entity: Entity,
    #[arg(long)]
    pub select: Option<String>,
    #[arg(long = "filter")]
    pub filter: Option<String>,
    #[arg(long)]
    pub sort: Option<String>,
    #[arg(long)]
    pub group: Option<String>,
    #[arg(long, default_value_t = 100)]
    pub limit: u32,
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
