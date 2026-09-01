use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{CrmError, Result};
use crate::jobs::{self, JobKind};
use crate::output::{self, Format};
use crate::{commands, review};

const LABEL: &str = "com.personal-crm.daemon";

pub fn start(format: Format, config_path: PathBuf) -> Result<()> {
    jobs::run(&config_path, JobKind::Contacts)?;
    let connection = commands::open_database(&config_path)?;
    let pending = review::pending_migration_count(&connection)?;
    if pending > 0 {
        return Err(CrmError::Contacts(format!(
            "{pending} CRM people require migration review; run `crm review` before `crm start`"
        )));
    }
    let plist = install_plist(&config_path)?;
    let domain = launch_domain()?;
    let service = format!("{domain}/{LABEL}");
    let loaded = Command::new("launchctl")
        .args(["print", &service])
        .output()
        .is_ok_and(|output| output.status.success());
    let status = if loaded {
        Command::new("launchctl")
            .args(["kickstart", "-k", &service])
            .status()
    } else {
        Command::new("launchctl")
            .args(["bootstrap", &domain, plist.to_str().unwrap()])
            .status()
    }
    .map_err(|error| {
        CrmError::InvalidConfig(format!("could not start launchd service: {error}"))
    })?;
    if !status.success() {
        return Err(CrmError::InvalidConfig(
            "launchd could not start the CRM daemon".into(),
        ));
    }
    output::emit(
        format,
        "start",
        serde_json::json!({"started": true}),
        "CRM daemon started".into(),
    )
}

pub fn stop(format: Format, config_path: PathBuf) -> Result<()> {
    let domain = launch_domain()?;
    let service = format!("{domain}/{LABEL}");
    let status = Command::new("launchctl")
        .args(["bootout", &service])
        .status()
        .map_err(|error| {
            CrmError::InvalidConfig(format!("could not stop launchd service: {error}"))
        })?;
    if !status.success() {
        return Err(CrmError::InvalidConfig("CRM daemon is not running".into()));
    }
    let connection = commands::open_database(&config_path)?;
    connection.execute(
        "UPDATE daemon_state SET pid=NULL, stopped_at=CURRENT_TIMESTAMP WHERE id=1",
        [],
    )?;
    output::emit(
        format,
        "stop",
        serde_json::json!({"stopped": true}),
        "CRM daemon stopped".into(),
    )
}

pub fn run_job(format: Format, config_path: PathBuf, kind: JobKind) -> Result<()> {
    jobs::run(&config_path, kind)?;
    output::emit(
        format,
        "run",
        serde_json::json!({"job": kind, "complete": true}),
        format!("{} complete", kind.as_str()),
    )
}

fn install_plist(config_path: &Path) -> Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| CrmError::InvalidConfig("cannot determine home directory".into()))?;
    let directory = home.join("Library/LaunchAgents");
    fs::create_dir_all(&directory).map_err(|source| CrmError::Io {
        path: directory.clone(),
        source,
    })?;
    let path = directory.join(format!("{LABEL}.plist"));
    let executable = std::env::current_exe().map_err(|source| CrmError::Io {
        path: PathBuf::from("current executable"),
        source,
    })?;
    let data = config_path.parent().unwrap();
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>{LABEL}</string>\n<key>ProgramArguments</key><array><string>{}</string><string>--config</string><string>{}</string><string>daemon</string></array>\n<key>KeepAlive</key><true/><key>RunAtLoad</key><true/>\n<key>StandardOutPath</key><string>{}/daemon.log</string>\n<key>StandardErrorPath</key><string>{}/daemon-error.log</string>\n</dict></plist>\n",
        xml(&executable.to_string_lossy()),
        xml(&config_path.to_string_lossy()),
        xml(&data.to_string_lossy()),
        xml(&data.to_string_lossy())
    );
    fs::write(&path, plist).map_err(|source| CrmError::Io {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

fn launch_domain() -> Result<String> {
    let output = Command::new("id").arg("-u").output().map_err(|error| {
        CrmError::InvalidConfig(format!("could not determine user id: {error}"))
    })?;
    if !output.status.success() {
        return Err(CrmError::InvalidConfig(
            "could not determine user id".into(),
        ));
    }
    Ok(format!(
        "gui/{}",
        String::from_utf8_lossy(&output.stdout).trim()
    ))
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
