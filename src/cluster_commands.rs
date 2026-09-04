use crate::{
    clusters,
    commands::open_database,
    error::Result,
    output::{self, Format},
};
use clap::Subcommand;
use std::path::Path;

#[derive(Subcommand)]
pub(crate) enum ClusterCommand {
    /// Compute or read cached clusters and evaluation at all detail levels.
    List,
    /// Persist a name for an existing cluster.
    Rename { id: String, name: String },
    /// Restore the evidence-backed suggested name.
    ResetName { id: String },
}

pub(crate) fn run(format: Format, config: &Path, command: ClusterCommand) -> Result<()> {
    let connection = open_database(config)?;
    match command {
        ClusterCommand::List => {
            let levels = clusters::load(&connection)?;
            let text = levels
                .iter()
                .map(|l| {
                    format!(
                        "{}: {} clusters; seed agreement {:.0}%; raw-weight agreement {:.0}%",
                        l.level,
                        l.clusters.len(),
                        l.seed_agreement * 100.0,
                        l.raw_weight_agreement * 100.0
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            output::emit(format, "cluster.list", &levels, text)
        }
        ClusterCommand::Rename { id, name } => {
            clusters::rename(&connection, &id, Some(&name))?;
            output::emit(
                format,
                "cluster.rename",
                &serde_json::json!({"id":id,"name":name.trim()}),
                "Cluster renamed".into(),
            )
        }
        ClusterCommand::ResetName { id } => {
            clusters::rename(&connection, &id, None)?;
            output::emit(
                format,
                "cluster.reset-name",
                &serde_json::json!({"id":id}),
                "Suggested name restored".into(),
            )
        }
    }
}
