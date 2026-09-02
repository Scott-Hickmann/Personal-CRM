use std::path::Path;
use std::process::Command;

use crate::error::{CrmError, Result};

pub fn run(port: u16) -> Result<()> {
    let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("web");
    if !web.join("package.json").exists() {
        return Err(CrmError::Ui(format!(
            "web app not found at {}",
            web.display()
        )));
    }
    let executable = std::env::current_exe().map_err(|error| CrmError::Ui(error.to_string()))?;
    let status = Command::new("pnpm")
        .args(["dev", "--", "--port", &port.to_string()])
        .current_dir(web)
        .env("CRM_CLI_PATH", executable)
        .status()
        .map_err(|error| CrmError::Ui(format!("could not start pnpm: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(CrmError::Ui(format!("pnpm exited with {status}")))
    }
}
