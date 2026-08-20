use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "skilldeck",
    version,
    about = "Install agent skills from Git-backed catalogs"
)]
#[command(help_template = "{about}\n\nUsage: {usage}\n\n{all-args}{after-help}\n")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Configure the global per-user catalog repository.
    Init(InitArgs),
    /// Install a catalog skill or direct Git repository.
    Install(InstallArgs),
    /// Install every skill in a catalog group.
    InstallGroup(GroupInstallArgs),
    /// Update one installed skill, or bulk update an install root.
    Update(UpdateArgs),
    /// Upgrade the Skilldeck binary from the latest stable GitHub release.
    Upgrade(UpgradeArgs),
    /// Remove one installed skill by directory name.
    Remove(RemoveArgs),
    /// Remove currently installed members of a catalog group.
    RemoveGroup(RemoveGroupArgs),
    /// Validate the configured catalog.
    Doctor(DoctorArgs),
    /// List catalog skills and groups.
    List(ListArgs),
    /// Print the Skilldeck version.
    Version,
}

#[derive(Args, Debug, Clone)]
pub struct CatalogOverrideArgs {
    /// Catalog Git repository URL/path (overrides config; env SKILLDECK_CATALOG_REPOSITORY).
    #[arg(long, global = true, env = "SKILLDECK_CATALOG_REPOSITORY")]
    pub catalog_repository: Option<String>,
    /// Catalog Git ref/branch/tag (overrides config; env SKILLDECK_CATALOG_REF).
    #[arg(long, global = true, env = "SKILLDECK_CATALOG_REF")]
    pub catalog_ref: Option<String>,
}

#[derive(Args, Debug)]
pub struct InitArgs {
    #[command(flatten)]
    pub overrides: CatalogOverrideArgs,
    /// Catalog Git repository URL/path for non-interactive setup.
    #[arg(long)]
    pub repository: Option<String>,
    /// Catalog Git ref/branch/tag for non-interactive setup.
    #[arg(long)]
    pub reference: Option<String>,
    /// Accept defaults without prompting.
    #[arg(long)]
    pub yes: bool,
    /// Replace an existing global Skilldeck config without confirmation.
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct InstallArgs {
    #[command(flatten)]
    pub overrides: CatalogOverrideArgs,
    /// Replace an existing destination directory.
    #[arg(long)]
    pub force: bool,
    /// Create a missing install root without prompting.
    #[arg(long)]
    pub yes: bool,
    pub name_or_git_url: String,
    pub install_directory: PathBuf,
}

#[derive(Args, Debug)]
pub struct GroupInstallArgs {
    #[command(flatten)]
    pub overrides: CatalogOverrideArgs,
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub yes: bool,
    pub group: String,
    pub install_directory: PathBuf,
}

#[derive(Args, Debug)]
pub struct UpdateArgs {
    #[command(flatten)]
    pub overrides: CatalogOverrideArgs,
    /// Skill/catalog name or Git URL. Omit for bulk update of an install root.
    pub name_or_git_url: Option<String>,
    /// Install root for single updates. For bulk update, pass it as the only positional argument.
    pub install_directory: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct UpgradeArgs {
    /// Report whether an upgrade is available; never prompt or install. Exits 0 if the check succeeds.
    #[arg(long)]
    pub check: bool,
    /// Download and install without prompting.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Args, Debug)]
pub struct RemoveArgs {
    pub name: String,
    pub install_directory: PathBuf,
}

#[derive(Args, Debug)]
pub struct RemoveGroupArgs {
    #[command(flatten)]
    pub overrides: CatalogOverrideArgs,
    pub group: String,
    pub install_directory: PathBuf,
}

#[derive(Args, Debug)]
pub struct DoctorArgs {
    #[command(flatten)]
    pub overrides: CatalogOverrideArgs,
    /// Resolve all external Git/Markdown sources too.
    #[arg(long)]
    pub deep: bool,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    #[command(flatten)]
    pub overrides: CatalogOverrideArgs,
    /// Print stable machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}
