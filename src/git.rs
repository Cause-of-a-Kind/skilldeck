use std::{path::Path, process::Command};

use anyhow::{anyhow, Context, Result};

pub fn clone_repository(
    repository: &str,
    reference: Option<&str>,
    destination: &Path,
) -> Result<()> {
    let status = if let Some(reference) = reference.filter(|r| !r.is_empty() && *r != "-") {
        let status = Command::new("git")
            .args([
                "clone",
                "--quiet",
                "--depth",
                "1",
                "--no-checkout",
                repository,
            ])
            .arg(destination)
            .status()
            .with_context(|| "running git clone")?;
        if !status.success() {
            return Err(anyhow!("could not clone {repository}"));
        }
        let status = Command::new("git")
            .arg("-C")
            .arg(destination)
            .args(["fetch", "--quiet", "--depth", "1", "origin", reference])
            .status()
            .with_context(|| "running git fetch")?;
        if !status.success() {
            return Err(anyhow!("could not fetch ref {reference} from {repository}"));
        }
        let status = Command::new("git")
            .arg("-C")
            .arg(destination)
            .args(["checkout", "--quiet", "--detach", "FETCH_HEAD"])
            .status()
            .with_context(|| "running git checkout")?;
        if !status.success() {
            return Err(anyhow!("could not checkout ref {reference}"));
        }
        Command::new("git")
            .arg("-C")
            .arg(destination)
            .args([
                "submodule",
                "update",
                "--quiet",
                "--init",
                "--recursive",
                "--depth",
                "1",
            ])
            .status()
            .with_context(|| "running git submodule update")?
    } else {
        Command::new("git")
            .args([
                "clone",
                "--quiet",
                "--depth",
                "1",
                "--recurse-submodules",
                repository,
            ])
            .arg(destination)
            .status()
            .with_context(|| "running git clone")?
    };
    if !status.success() {
        return Err(anyhow!("git command failed for {repository}"));
    }
    Ok(())
}
