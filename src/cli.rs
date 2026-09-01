use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

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
    /// Create a new local Skilldeck catalog scaffold.
    Bootstrap(BootstrapArgs),
    /// Configure the initial global per-user registry.
    Init(InitArgs),
    /// Manage configured package registries.
    Registry(RegistryArgs),
    /// Inspect Skilldeck configuration.
    Config(ConfigArgs),
    /// Maintain and validate a local catalog working tree.
    Catalog(CatalogArgs),
    /// Install a catalog skill or direct Git repository.
    Install(InstallArgs),
    /// Install every skill in a catalog group.
    InstallGroup(GroupInstallArgs),
    /// Manage local compatibility with agent harnesses.
    Harness(HarnessArgs),
    /// Print version-matched documentation embedded in this binary.
    Docs(DocsArgs),
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
    /// Configured registry alias (defaults to the configured default registry; env SKILLDECK_REGISTRY).
    #[arg(long, global = true, env = "SKILLDECK_REGISTRY")]
    pub registry: Option<String>,
    /// Catalog Git repository URL/path (ad-hoc override; env SKILLDECK_CATALOG_REPOSITORY).
    #[arg(long, global = true, env = "SKILLDECK_CATALOG_REPOSITORY")]
    pub catalog_repository: Option<String>,
    /// Catalog Git ref/branch/tag (overrides config; env SKILLDECK_CATALOG_REF).
    #[arg(long, global = true, env = "SKILLDECK_CATALOG_REF")]
    pub catalog_ref: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct LocalCatalogArgs {
    /// Use a local catalog working tree, including uncommitted changes (defaults to `.`).
    #[arg(long, num_args = 0..=1, default_missing_value = ".", value_name = "PATH")]
    pub local: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct BootstrapArgs {
    /// Destination directory to create or populate if genuinely empty.
    pub path: Option<PathBuf>,
    /// Create a catalog with working example skills and a quickstart group.
    #[arg(long, conflicts_with = "empty")]
    pub quickstart: bool,
    /// Create only the catalog structure and commented example formats.
    #[arg(long, conflicts_with = "quickstart")]
    pub empty: bool,
    /// Generate files only; do not initialize or commit a local Git repository.
    #[arg(long)]
    pub no_git: bool,
}

#[derive(Args, Debug)]
pub struct InitArgs {
    #[command(flatten)]
    pub overrides: CatalogOverrideArgs,
    /// Alias for the initial registry.
    #[arg(long, default_value = "default")]
    pub name: String,
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
    #[command(flatten)]
    pub local_catalog: LocalCatalogArgs,
    /// Replace an existing destination directory.
    #[arg(long)]
    pub force: bool,
    /// Create a missing install root and accept recipe defaults without prompting.
    #[arg(long)]
    pub yes: bool,
    /// Set an install-time recipe input (repeatable).
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub values: Vec<String>,
    /// Install in the selected target's global skill directory.
    #[arg(long, conflicts_with = "install_directory")]
    pub global: bool,
    /// Install into a harness-native directory instead of the portable .agents directory.
    #[arg(long, value_enum, default_value_t = InstallTarget::Agents)]
    pub target: InstallTarget,
    /// Also create a local Claude Code alias for a portable agents-target installation.
    #[arg(long, conflicts_with = "install_directory")]
    pub claude: bool,
    pub name_or_git_url: String,
    /// Custom install root. When omitted, uses the selected target under the Git root.
    pub install_directory: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct GroupInstallArgs {
    #[command(flatten)]
    pub overrides: CatalogOverrideArgs,
    #[command(flatten)]
    pub local_catalog: LocalCatalogArgs,
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub yes: bool,
    /// Set recipe inputs; use SKILL.KEY=VALUE to target one group member.
    #[arg(long = "set", value_name = "[SKILL.]KEY=VALUE")]
    pub values: Vec<String>,
    /// Install in the selected target's global skill directory.
    #[arg(long, conflicts_with = "install_directory")]
    pub global: bool,
    /// Install into a harness-native directory instead of the portable .agents directory.
    #[arg(long, value_enum, default_value_t = InstallTarget::Agents)]
    pub target: InstallTarget,
    /// Also create local Claude Code aliases for portable agents-target installations.
    #[arg(long, conflicts_with = "install_directory")]
    pub claude: bool,
    pub group: String,
    /// Custom install root. When omitted, uses the selected target under the Git root.
    pub install_directory: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct HarnessArgs {
    #[command(subcommand)]
    pub command: HarnessCommands,
}

#[derive(Subcommand, Debug)]
pub enum HarnessCommands {
    /// Create missing local aliases for canonical skills.
    Sync(HarnessScopeArgs),
    /// Report linked, missing, conflicting, and stale aliases.
    Status(HarnessScopeArgs),
    /// Remove only aliases managed from the canonical skill directory.
    Remove(HarnessScopeArgs),
}

#[derive(Args, Debug)]
pub struct HarnessScopeArgs {
    #[arg(value_enum)]
    pub harness: HarnessKind,
    /// Operate on ~/.agents/skills and ~/.claude/skills.
    #[arg(long)]
    pub global: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum HarnessKind {
    Claude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InstallTarget {
    /// Portable Agent Skills directory shared by interoperable harnesses.
    Agents,
    /// Pi's native skill directory.
    Pi,
    /// Codex's native skill directory.
    Codex,
    /// Claude Code's native skill directory.
    Claude,
    /// Gemini CLI's native skill directory.
    Gemini,
    /// Cursor's native skill directory.
    Cursor,
    /// OpenCode's native skill directory.
    Opencode,
}

#[derive(Args, Debug)]
pub struct DocsArgs {
    /// Documentation topic. Omit to list available topics.
    #[arg(value_enum)]
    pub topic: Option<DocsTopic>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DocsTopic {
    /// Current operational guidance intended for coding agents.
    Agent,
    /// Recipe manifests, MiniJinja templates, inputs, and local configuration.
    Recipes,
    /// Complete README for this exact Skilldeck build.
    Readme,
}

#[derive(Args, Debug)]
pub struct UpdateArgs {
    #[command(flatten)]
    pub overrides: CatalogOverrideArgs,
    /// Override an install-time recipe input for a single-skill update.
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub values: Vec<String>,
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
    /// Installed skill directory name.
    pub name: String,
    /// Custom install root. When omitted, uses the selected target under the Git root.
    pub install_directory: Option<PathBuf>,
    /// Remove from the selected target's global skill directory.
    #[arg(long, conflicts_with = "install_directory")]
    pub global: bool,
    /// Remove from a harness-native directory instead of the portable .agents directory.
    #[arg(long, value_enum, default_value_t = InstallTarget::Agents)]
    pub target: InstallTarget,
}

#[derive(Args, Debug)]
pub struct RemoveGroupArgs {
    #[command(flatten)]
    pub overrides: CatalogOverrideArgs,
    pub group: String,
    /// Custom install root. When omitted, uses the selected target under the Git root.
    pub install_directory: Option<PathBuf>,
    /// Remove from the selected target's global skill directory.
    #[arg(long, conflicts_with = "install_directory")]
    pub global: bool,
    /// Remove from a harness-native directory instead of the portable .agents directory.
    #[arg(long, value_enum, default_value_t = InstallTarget::Agents)]
    pub target: InstallTarget,
}

#[derive(Args, Debug)]
pub struct DoctorArgs {
    #[command(flatten)]
    pub overrides: CatalogOverrideArgs,
    #[command(flatten)]
    pub local_catalog: LocalCatalogArgs,
    /// Resolve all external Git/Markdown sources too.
    #[arg(long)]
    pub deep: bool,
    /// Fail when SKILL.md frontmatter is missing or malformed.
    #[arg(long)]
    pub strict: bool,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    #[command(flatten)]
    pub overrides: CatalogOverrideArgs,
    #[command(flatten)]
    pub local_catalog: LocalCatalogArgs,
    /// List every configured registry, including built-in skills.
    #[arg(long, conflicts_with_all = ["registry_name", "builtins"])]
    pub all: bool,
    /// List only skills bundled with the Skilldeck binary.
    #[arg(long, conflicts_with_all = ["registry_name", "all"])]
    pub builtins: bool,
    /// Registry alias to list (the default registry when omitted).
    pub registry_name: Option<String>,
    /// Print stable machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct RegistryArgs {
    #[command(subcommand)]
    pub command: RegistryCommands,
}

#[derive(Subcommand, Debug)]
pub enum RegistryCommands {
    /// Add and validate a registry.
    Add(RegistryAddArgs),
    /// List configured registries.
    List(RegistryListArgs),
    /// Select the registry used by unqualified package names.
    Default(RegistryDefaultArgs),
    /// Rename a local registry alias.
    Rename(RegistryRenameArgs),
    /// Change a registry repository or ref after validating it.
    Update(RegistryUpdateArgs),
    /// Remove a registry.
    Remove(RegistryRemoveArgs),
    /// Validate one or every configured registry.
    Doctor(RegistryDoctorArgs),
}

#[derive(Args, Debug)]
pub struct RegistryAddArgs {
    pub name: String,
    pub repository: String,
    #[arg(long, default_value = "master")]
    pub reference: String,
    /// Name to give a legacy single registry while migrating.
    #[arg(long)]
    pub existing_as: Option<String>,
    /// Make the newly added registry the default.
    #[arg(long)]
    pub default: bool,
    /// Do not prompt during legacy migration.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Args, Debug)]
pub struct RegistryListArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct RegistryDefaultArgs {
    pub name: String,
}

#[derive(Args, Debug)]
pub struct RegistryRenameArgs {
    pub old: String,
    pub new: String,
}

#[derive(Args, Debug)]
pub struct RegistryUpdateArgs {
    pub name: String,
    #[arg(long)]
    pub repository: Option<String>,
    #[arg(long = "reference")]
    pub reference: Option<String>,
}

#[derive(Args, Debug)]
pub struct RegistryRemoveArgs {
    pub name: String,
    /// Remove without prompting.
    #[arg(long)]
    pub yes: bool,
    /// Select another default when removing the current default.
    #[arg(long)]
    pub new_default: Option<String>,
}

#[derive(Args, Debug)]
pub struct RegistryDoctorArgs {
    pub name: Option<String>,
    #[arg(long, conflicts_with = "name")]
    pub all: bool,
    #[arg(long)]
    pub deep: bool,
    /// Fail when SKILL.md frontmatter is missing or malformed.
    #[arg(long)]
    pub strict: bool,
}

#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommands,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// Print the active global configuration path.
    Path,
}

#[derive(Args, Debug)]
pub struct CatalogArgs {
    #[command(subcommand)]
    pub command: CatalogCommands,
}

#[derive(Subcommand, Debug)]
pub enum CatalogCommands {
    /// Validate a local catalog working tree without cloning it.
    Check(CatalogCheckArgs),
    /// Add and validate an external package in a local catalog.
    Add(CatalogAddArgs),
}

#[derive(Args, Debug)]
pub struct CatalogCheckArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Resolve external Git and Markdown sources too.
    #[arg(long)]
    pub deep: bool,
    /// Fail when SKILL.md frontmatter is missing or malformed.
    #[arg(long)]
    pub strict: bool,
}

#[derive(Args, Debug)]
pub struct CatalogAddArgs {
    pub name: String,
    /// Git repository or direct Markdown URL.
    #[arg(long)]
    pub source: String,
    #[arg(long)]
    pub subdirectory: Option<String>,
    #[arg(long = "reference")]
    pub reference: Option<String>,
    /// Local catalog working tree.
    #[arg(long, default_value = ".")]
    pub path: PathBuf,
    /// Skip resolving the remote source before adding it.
    #[arg(long)]
    pub no_check: bool,
}
