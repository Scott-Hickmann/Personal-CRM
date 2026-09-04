use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::error::Result;
use crate::output::Format;
use crate::query::Entity;

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
    Ui(UiArgs),
    #[command(hide = true)]
    UiData {
        #[command(subcommand)]
        command: UiDataCommand,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Doctor,
    Start,
    Stop,
    Status(StatusArgs),
    Review(ReviewArgs),
    Run(RunArgs),
    #[command(hide = true)]
    Daemon,
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
    Followup {
        #[command(subcommand)]
        command: FollowupCommand,
    },
    Affinity {
        #[command(subcommand)]
        command: AffinityCommand,
    },
    Query(QueryArgs),
    History(HistoryArgs),
    Explain(PersonReference),
    Graph(GraphArgs),
    Cluster {
        #[command(subcommand)]
        command: crate::cluster_commands::ClusterCommand,
    },
    Face {
        #[command(subcommand)]
        command: FaceCommand,
    },
    Photos {
        #[command(subcommand)]
        command: PhotosCommand,
    },
    Contacts {
        #[command(subcommand)]
        command: ContactsCommand,
    },
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
}

#[derive(Args)]
pub(crate) struct UiArgs {
    #[arg(long, default_value_t = 3000)]
    pub port: u16,
}

#[derive(Args)]
pub(crate) struct StatusArgs {
    /// Continuously show daemon progress and recent activity
    #[arg(long)]
    pub live: bool,
}

#[derive(Subcommand)]
pub(crate) enum UiDataCommand {
    Overview,
    Image(PersonReference),
    Person(PersonDetailArgs),
    Interaction(InteractionReference),
}

#[derive(Args)]
pub(crate) struct PersonDetailArgs {
    pub person: String,
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..=1000))]
    pub history_limit: u32,
}

#[derive(Args)]
pub(crate) struct InteractionReference {
    pub interaction: String,
}

#[derive(Subcommand)]
pub(crate) enum ContactsCommand {
    Containers,
    Configure(ContactsConfigureArgs),
    Status,
}

#[derive(Args)]
pub(crate) struct ContactsConfigureArgs {
    #[arg(long)]
    pub source_container: String,
    #[arg(long)]
    pub personal_account: String,
    #[arg(long)]
    pub workspace_account: String,
    #[arg(long = "work-domain", required = true)]
    pub work_domains: Vec<String>,
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
pub(crate) struct GraphArgs {
    pub person: Option<String>,
}

#[derive(Args)]
pub(crate) struct ReviewArgs {
    pub id: Option<String>,
    #[arg(long, conflicts_with_all = ["approve", "reject", "delete_person"])]
    pub link_icloud: Option<String>,
    #[arg(long, conflicts_with_all = ["reject", "delete_person"])]
    pub approve: bool,
    #[arg(long, conflicts_with = "delete_person")]
    pub reject: bool,
    #[arg(long)]
    pub delete_person: bool,
}

#[derive(Args)]
pub(crate) struct RunArgs {
    #[arg(value_enum)]
    pub work: crate::coordinator::WorkKind,
}

#[derive(Subcommand)]
pub(crate) enum AuthCommand {
    Gmail {
        #[command(subcommand)]
        command: GmailCommand,
    },
    Contacts {
        #[command(subcommand)]
        command: ContactsAuthCommand,
    },
}

#[derive(Subcommand)]
pub(crate) enum ContactsAuthCommand {
    Add {
        #[arg(long)]
        credentials: Option<PathBuf>,
    },
    List,
    Remove {
        account: String,
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
    pub include_retired: bool,
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
    #[arg(long = "self-phone")]
    pub self_phones: Vec<String>,
}

#[derive(Subcommand)]
pub(crate) enum PersonCommand {
    Show(PersonReference),
    Delete(PersonReference),
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

#[derive(Subcommand)]
pub(crate) enum FollowupCommand {
    Add(FollowupAddArgs),
}

#[derive(Subcommand)]
pub(crate) enum AffinityCommand {
    Rate(AffinityRateArgs),
    Clear(AffinityClearArgs),
}

#[derive(Args)]
pub(crate) struct AffinityRateArgs {
    #[arg(long)]
    pub person: String,
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=7))]
    pub rating: u8,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub(crate) struct AffinityClearArgs {
    #[arg(long)]
    pub person: String,
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

#[derive(Args)]
pub(crate) struct FollowupAddArgs {
    #[arg(long)]
    pub person: String,
    #[arg(long)]
    pub text: String,
    #[arg(long)]
    pub due: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
}

pub fn run(cli: Cli) -> Result<()> {
    crate::commands::run(cli.format, cli.config, cli.command)
}
