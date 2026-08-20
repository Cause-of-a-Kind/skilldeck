use std::{fmt, path::Path, process::Command};

use anyhow::{anyhow, Context, Result};

const INITIAL_COMMIT_MESSAGE: &str = "Start Skilldeck catalog";

#[derive(Debug)]
pub struct BootstrapGitError {
    pub step: BootstrapGitStep,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapGitStep {
    Init,
    Add,
    Commit,
}

impl fmt::Display for BootstrapGitStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Init => "git init",
            Self::Add => "git add",
            Self::Commit => "git commit",
        })
    }
}

pub fn initial_commit_message() -> &'static str {
    INITIAL_COMMIT_MESSAGE
}

pub fn initialize_catalog_repository(catalog: &Path) -> std::result::Result<(), BootstrapGitError> {
    run_catalog_git(
        catalog,
        BootstrapGitStep::Init,
        &["init", "--initial-branch=main"],
    )?;
    run_catalog_git(catalog, BootstrapGitStep::Add, &["add", "."])?;
    run_catalog_git(
        catalog,
        BootstrapGitStep::Commit,
        &["commit", "-m", INITIAL_COMMIT_MESSAGE],
    )?;
    Ok(())
}

fn run_catalog_git(
    catalog: &Path,
    step: BootstrapGitStep,
    args: &[&str],
) -> std::result::Result<(), BootstrapGitError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(catalog)
        .args(args)
        .output()
        .map_err(|err| BootstrapGitError {
            step,
            detail: err.to_string(),
        })?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Err(BootstrapGitError {
            step,
            detail: if !stderr.is_empty() { stderr } else { stdout },
        })
    }
}

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
