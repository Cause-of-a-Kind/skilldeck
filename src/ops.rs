use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use tempfile::TempDir;

use crate::{
    builtins,
    catalog::{self, Catalog},
    cli::*,
    config::{self, Config, DEFAULT_REF},
    fsops, git,
    manifest::{self, Provenance},
};

pub fn init(args: InitArgs) -> Result<()> {
    let repo = args
        .repository
        .or(args.overrides.catalog_repository)
        .or_else(|| {
            if args.yes {
                None
            } else {
                prompt("Catalog repository URL/path")
            }
        });
    let repo =
        repo.ok_or_else(|| anyhow!("--repository is required with --yes/non-interactive init"))?;
    let repo = config::normalize_repository(&repo)?;
    let reference = args
        .reference
        .or(args.overrides.catalog_ref)
        .or_else(|| {
            if args.yes {
                Some(DEFAULT_REF.into())
            } else {
                prompt_default("Catalog ref", DEFAULT_REF)
            }
        })
        .unwrap_or_else(|| DEFAULT_REF.into());
    config::validate_registry_name(&args.name)?;
    let cfg = Config {
        catalog_repository: repo,
        catalog_ref: reference,
    };
    let catalog = Catalog::clone_from_config(&cfg)?;
    let summary = catalog.validate()?;
    let path = config::config_path()?;
    if path.exists()
        && !args.force
        && (args.yes
            || !confirm(&format!(
                "Replace existing Skilldeck config at {}?",
                path.display()
            ))?)
    {
        return Err(anyhow!(
            "config already exists at {} (use --force to replace it)",
            path.display()
        ));
    }
    let mut registries = std::collections::BTreeMap::new();
    registries.insert(
        args.name.clone(),
        config::Registry {
            repository: cfg.catalog_repository.clone(),
            reference: cfg.catalog_ref.clone(),
        },
    );
    let path = config::save_registries(&config::RegistrySet {
        default_registry: args.name.clone(),
        registries,
    })?;
    println!(
        "Configured skilldeck registry `{}` at {}",
        args.name,
        path.display()
    );
    println!("repository = {}", cfg.catalog_repository);
    println!("ref = {}", cfg.catalog_ref);
    println!(
        "Found {} skills and {} groups. ({} first-party, {} external)",
        summary.total_skill_count,
        summary.group_count,
        summary.first_party_count,
        summary.external_count
    );
    Ok(())
}

pub fn registry(args: RegistryArgs) -> Result<()> {
    match args.command {
        RegistryCommands::Add(args) => registry_add(args),
        RegistryCommands::List(args) => registry_list(args),
        RegistryCommands::Default(args) => registry_default(args),
        RegistryCommands::Rename(args) => registry_rename(args),
        RegistryCommands::Update(args) => registry_update(args),
        RegistryCommands::Remove(args) => registry_remove(args),
        RegistryCommands::Doctor(args) => registry_doctor(args),
    }
}

pub fn config_command(args: ConfigArgs) -> Result<()> {
    match args.command {
        ConfigCommands::Path => {
            println!("{}", config::config_path()?.display());
            Ok(())
        }
    }
}

pub fn docs(args: DocsArgs) -> Result<()> {
    match args.topic {
        None => {
            println!(
                "Documentation embedded in Skilldeck {}:",
                env!("CARGO_PKG_VERSION")
            );
            println!("  agent    Operational guidance for coding agents");
            println!("  recipes  Recipe, MiniJinja, input, and local-config reference");
            println!("  readme   Complete version-matched README");
            println!("\nRun `skilldeck docs <topic>`. No network access is required.");
        }
        Some(DocsTopic::Agent) => {
            print!(
                "{}",
                builtins::content("skilldeck").expect("built-in Skilldeck skill must exist")
            );
        }
        Some(DocsTopic::Recipes) => {
            const README: &str = include_str!("../README.md");
            let start = README
                .find("### Composable skill recipes")
                .ok_or_else(|| anyhow!("embedded README is missing the recipe reference"))?;
            let remainder = &README[start..];
            let end = remainder
                .find("\n### Maintain a local catalog")
                .unwrap_or(remainder.len());
            println!("{}", remainder[..end].trim_end());
        }
        Some(DocsTopic::Readme) => print!("{}", include_str!("../README.md")),
    }
    Ok(())
}

pub fn harness(args: HarnessArgs) -> Result<()> {
    match args.command {
        HarnessCommands::Sync(args) => {
            let status = match args.harness {
                HarnessKind::Claude => crate::harness::sync(args.global)?,
            };
            print_harness_status("Claude Code compatibility synchronized", &status);
            Ok(())
        }
        HarnessCommands::Status(args) => {
            let status = match args.harness {
                HarnessKind::Claude => crate::harness::status(args.global)?,
            };
            print_harness_status("Claude Code compatibility", &status);
            if status.conflicts.is_empty() && status.missing.is_empty() && status.stale.is_empty() {
                Ok(())
            } else {
                Err(anyhow!("Claude Code compatibility needs attention"))
            }
        }
        HarnessCommands::Remove(args) => {
            let removed = match args.harness {
                HarnessKind::Claude => crate::harness::remove(args.global)?,
            };
            println!("Removed {removed} Claude Code compatibility aliases");
            Ok(())
        }
    }
}

fn print_harness_status(label: &str, status: &crate::harness::HarnessStatus) {
    println!("{label}");
    println!("Canonical skills: {}", status.canonical);
    println!("Linked skills: {}", status.linked.len());
    if !status.missing.is_empty() {
        println!("Missing links:");
        for name in &status.missing {
            println!("  {name}");
        }
    }
    if !status.conflicts.is_empty() {
        println!("Conflicts:");
        for name in &status.conflicts {
            println!("  {name}");
        }
    }
    if !status.stale.is_empty() {
        println!("Stale links:");
        for name in &status.stale {
            println!("  {name}");
        }
    }
}

pub fn catalog_command(args: CatalogArgs) -> Result<()> {
    match args.command {
        CatalogCommands::Check(args) => catalog_check(args),
        CatalogCommands::Add(args) => catalog_add(args),
    }
}

fn catalog_check(args: CatalogCheckArgs) -> Result<()> {
    let root = fs::canonicalize(&args.path)
        .with_context(|| format!("opening local catalog {}", args.path.display()))?;
    let catalog = Catalog::open_path(root)?;
    let summary = catalog.validate()?;
    println!("Catalog structure: ok");
    println!(
        "Found {} skills and {} groups. ({} first-party, {} external)",
        summary.total_skill_count,
        summary.group_count,
        summary.first_party_count,
        summary.external_count
    );
    validate_catalog_metadata(&catalog, args.strict)?;
    if args.deep {
        deep_validate(&catalog, args.strict)?;
    }
    println!("Catalog check complete: ok");
    Ok(())
}

fn catalog_add(args: CatalogAddArgs) -> Result<()> {
    catalog::validate_name(&args.name, "skill")?;
    if args.source.trim().is_empty() {
        return Err(anyhow!("external skill source cannot be empty"));
    }
    if let Some(path) = args.subdirectory.as_deref() {
        catalog::safe_relative_path(path)?;
    }
    if fsops::is_markdown_url(&args.source)
        && (args.subdirectory.is_some() || args.reference.is_some())
    {
        return Err(anyhow!(
            "direct Markdown sources cannot use --subdirectory or --reference"
        ));
    }
    let root = fs::canonicalize(&args.path)
        .with_context(|| format!("opening local catalog {}", args.path.display()))?;
    let existing = Catalog::open_path(root.clone())?;
    if existing
        .skill_names()
        .iter()
        .any(|name| name.eq_ignore_ascii_case(&args.name))
    {
        return Err(anyhow!(
            "skill name already exists in catalog (case-insensitive): {}",
            args.name
        ));
    }
    let external = catalog::ExternalSkill {
        source: args.source,
        subdirectory: args.subdirectory,
        reference: args.reference,
        recipe: None,
    };
    if !args.no_check {
        validate_external_package(&args.name, &external, true)?;
    }

    let path = root.join("external-skills.toml");
    let text = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let mut document = text
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("parsing {}", path.display()))?;
    if !document.contains_key("skills") {
        document["skills"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let mut entry = toml_edit::Table::new();
    entry["source"] = toml_edit::value(external.source.clone());
    if let Some(subdirectory) = &external.subdirectory {
        entry["subdirectory"] = toml_edit::value(subdirectory.clone());
    }
    if let Some(reference) = &external.reference {
        entry["ref"] = toml_edit::value(reference.clone());
    }
    document["skills"][&args.name] = toml_edit::Item::Table(entry);

    let original = text;
    fs::write(&path, document.to_string())?;
    let validation = Catalog::open_path(root).and_then(|catalog| catalog.validate());
    if let Err(error) = validation {
        fs::write(&path, original)
            .with_context(|| format!("restoring {} after validation failure", path.display()))?;
        return Err(error.context("catalog add rolled back because validation failed"));
    }
    println!("Added external skill `{}` to {}", args.name, path.display());
    Ok(())
}

fn registry_add(args: RegistryAddArgs) -> Result<()> {
    config::validate_registry_name(&args.name)?;
    let repository = config::normalize_repository(&args.repository)?;
    let cfg = Config {
        catalog_repository: repository.clone(),
        catalog_ref: args.reference.clone(),
    };
    let summary = Catalog::clone_from_config(&cfg)?.validate()?;

    let loaded = config::load_registries()?;
    let adding_first = loaded.is_none();
    let mut set = match loaded {
        Some(mut loaded) => {
            if loaded.legacy {
                let existing_name = match args.existing_as {
                    Some(name) => name,
                    None if args.yes => config::LEGACY_REGISTRY_NAME.into(),
                    None => prompt_default(
                        "Namespace for the existing registry",
                        config::LEGACY_REGISTRY_NAME,
                    )
                    .unwrap_or_else(|| config::LEGACY_REGISTRY_NAME.into()),
                };
                config::validate_registry_name(&existing_name)?;
                if existing_name != config::LEGACY_REGISTRY_NAME {
                    let existing = loaded
                        .set
                        .registries
                        .remove(config::LEGACY_REGISTRY_NAME)
                        .expect("legacy registry exists");
                    loaded
                        .set
                        .registries
                        .insert(existing_name.clone(), existing);
                    loaded.set.default_registry = existing_name;
                }
            }
            loaded.set
        }
        None => config::RegistrySet {
            default_registry: args.name.clone(),
            registries: std::collections::BTreeMap::new(),
        },
    };
    if set.registries.contains_key(&args.name) {
        return Err(anyhow!("registry already exists: {}", args.name));
    }
    set.registries.insert(
        args.name.clone(),
        config::Registry {
            repository,
            reference: args.reference,
        },
    );
    let make_default = args.default
        || adding_first
        || (!args.yes
            && confirm(&format!(
                "Make `{}` the default registry instead?",
                args.name
            ))?);
    if make_default {
        set.default_registry = args.name.clone();
    }
    let path = config::save_registries(&set)?;
    println!("Added registry `{}` in {}", args.name, path.display());
    println!(
        "Found {} skills and {} groups. ({} first-party, {} external)",
        summary.total_skill_count,
        summary.group_count,
        summary.first_party_count,
        summary.external_count
    );
    println!("Default registry: {}", set.default_registry);
    Ok(())
}

fn registry_list(args: RegistryListArgs) -> Result<()> {
    let loaded = config::load_registries()?
        .ok_or_else(|| anyhow!("no registries configured; run `skilldeck init`"))?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&loaded.set)?);
    } else {
        for (name, registry) in loaded.set.registries {
            let marker = if name == loaded.set.default_registry {
                "*"
            } else {
                " "
            };
            println!(
                "{marker} {name}: {} @ {}",
                registry.repository, registry.reference
            );
        }
    }
    Ok(())
}

fn registry_default(args: RegistryDefaultArgs) -> Result<()> {
    config::validate_registry_name(&args.name)?;
    let mut loaded = config::load_registries()?
        .ok_or_else(|| anyhow!("no registries configured; run `skilldeck init`"))?;
    if !loaded.set.registries.contains_key(&args.name) {
        return Err(anyhow!("registry not found: {}", args.name));
    }
    loaded.set.default_registry = args.name.clone();
    config::save_registries(&loaded.set)?;
    println!("Default registry: {}", args.name);
    Ok(())
}

fn registry_rename(args: RegistryRenameArgs) -> Result<()> {
    config::validate_registry_name(&args.old)?;
    config::validate_registry_name(&args.new)?;
    let mut loaded = config::load_registries()?
        .ok_or_else(|| anyhow!("no registries configured; run `skilldeck init`"))?;
    if loaded.set.registries.contains_key(&args.new) {
        return Err(anyhow!("registry already exists: {}", args.new));
    }
    let registry = loaded
        .set
        .registries
        .remove(&args.old)
        .ok_or_else(|| anyhow!("registry not found: {}", args.old))?;
    loaded.set.registries.insert(args.new.clone(), registry);
    if loaded.set.default_registry == args.old {
        loaded.set.default_registry = args.new.clone();
    }
    config::save_registries(&loaded.set)?;
    println!("Renamed registry `{}` to `{}`", args.old, args.new);
    Ok(())
}

fn registry_update(args: RegistryUpdateArgs) -> Result<()> {
    if args.repository.is_none() && args.reference.is_none() {
        return Err(anyhow!(
            "registry update requires --repository or --reference"
        ));
    }
    let mut loaded = config::load_registries()?
        .ok_or_else(|| anyhow!("no registries configured; run `skilldeck init`"))?;
    let registry = loaded
        .set
        .registries
        .get_mut(&args.name)
        .ok_or_else(|| anyhow!("registry not found: {}", args.name))?;
    if let Some(repository) = args.repository {
        registry.repository = config::normalize_repository(&repository)?;
    }
    if let Some(reference) = args.reference {
        registry.reference = reference;
    }
    Catalog::clone_from_config(&registry.as_config())?.validate()?;
    config::save_registries(&loaded.set)?;
    println!("Updated registry `{}`", args.name);
    Ok(())
}

fn registry_remove(args: RegistryRemoveArgs) -> Result<()> {
    let mut loaded = config::load_registries()?
        .ok_or_else(|| anyhow!("no registries configured; run `skilldeck init`"))?;
    if !loaded.set.registries.contains_key(&args.name) {
        return Err(anyhow!("registry not found: {}", args.name));
    }
    if loaded.set.registries.len() == 1 {
        return Err(anyhow!("cannot remove the only configured registry"));
    }
    if loaded.set.default_registry == args.name {
        let replacement = args.new_default.ok_or_else(|| {
            anyhow!("cannot remove the default registry without --new-default <name>")
        })?;
        if replacement == args.name || !loaded.set.registries.contains_key(&replacement) {
            return Err(anyhow!(
                "replacement default registry not found: {replacement}"
            ));
        }
        loaded.set.default_registry = replacement;
    }
    if !args.yes && !confirm(&format!("Remove registry `{}`?", args.name))? {
        return Err(anyhow!("registry removal cancelled"));
    }
    loaded.set.registries.remove(&args.name);
    config::save_registries(&loaded.set)?;
    println!("Removed registry `{}`", args.name);
    Ok(())
}

fn registry_doctor(args: RegistryDoctorArgs) -> Result<()> {
    let loaded = config::load_registries()?
        .ok_or_else(|| anyhow!("no registries configured; run `skilldeck init`"))?;
    let names: Vec<String> = if args.all {
        loaded.set.registries.keys().cloned().collect()
    } else {
        vec![args
            .name
            .unwrap_or_else(|| loaded.set.default_registry.clone())]
    };
    for name in names {
        let registry = loaded
            .set
            .registries
            .get(&name)
            .ok_or_else(|| anyhow!("registry not found: {name}"))?;
        let catalog = Catalog::clone_from_config(&registry.as_config())?;
        let summary = catalog.validate()?;
        println!(
            "Registry {name}: ok ({} skills, {} groups)",
            summary.total_skill_count, summary.group_count
        );
        validate_catalog_metadata(&catalog, args.strict)?;
        if args.deep {
            deep_validate(&catalog, args.strict)?;
        }
    }
    println!("Registry doctor complete: ok");
    Ok(())
}

fn prompt(label: &str) -> Option<String> {
    eprint!("{label}: ");
    io::stderr().flush().ok()?;
    let mut s = String::new();
    io::stdin().read_line(&mut s).ok()?;
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}
fn prompt_default(label: &str, default: &str) -> Option<String> {
    eprint!("{label} [{default}]: ");
    io::stderr().flush().ok()?;
    let mut s = String::new();
    io::stdin().read_line(&mut s).ok()?;
    let s = s.trim();
    Some(if s.is_empty() {
        default.into()
    } else {
        s.into()
    })
}

fn confirm(label: &str) -> Result<bool> {
    eprint!("{label} [y/N] ");
    io::stderr().flush().ok();
    let mut s = String::new();
    io::stdin().read_line(&mut s)?;
    Ok(matches!(s.trim(), "y" | "Y" | "yes" | "YES" | "Yes"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BootstrapTemplate {
    Quickstart,
    Empty,
}

pub fn bootstrap(args: BootstrapArgs) -> Result<()> {
    let explicit_template = match (args.quickstart, args.empty) {
        (true, false) => Some(BootstrapTemplate::Quickstart),
        (false, true) => Some(BootstrapTemplate::Empty),
        (false, false) => None,
        (true, true) => unreachable!("clap rejects conflicting template flags"),
    };

    let path = match args.path {
        Some(path) => path,
        None => PathBuf::from(prompt_default_non_eof(
            "Where should the catalog be created?",
            "./skilldeck-catalog",
        )?),
    };
    let template = match explicit_template {
        Some(template) => template,
        None => prompt_bootstrap_template()?,
    };

    create_bootstrap_catalog(&path, template)?;
    if args.no_git {
        print_bootstrap_success(&path, template, false);
    } else if let Err(err) = git::initialize_catalog_repository(&path) {
        return Err(bootstrap_git_error(&path, err));
    } else {
        print_bootstrap_success(&path, template, true);
    }
    Ok(())
}

fn print_bootstrap_success(path: &Path, template: BootstrapTemplate, git_initialized: bool) {
    println!("Created Skilldeck catalog at {}", path.display());
    if git_initialized {
        println!("Git repository initialized on branch main with initial commit `Start Skilldeck catalog`.");
        println!("Next steps:");
        println!("  1. From inside the generated directory, optionally add a remote and push:");
        println!("     git remote add origin <your-catalog-git-url>");
        println!("     git push -u origin main");
        println!("  2. From inside the generated directory, configure Skilldeck:");
        println!("     skilldeck init --repository . --reference main");
        println!("     # or, after pushing: skilldeck init --repository <your-catalog-git-url> --reference main");
        println!("  3. Validate the catalog:");
        println!("     skilldeck doctor");
        if template == BootstrapTemplate::Quickstart {
            println!("  4. Try the quickstart group:");
            println!("     skilldeck install-group quickstart <install-directory>");
        }
    } else {
        println!("Git initialization skipped because --no-git was used.");
        println!("Next steps:");
        println!("  1. Review the generated files in {}", path.display());
        println!("  2. From inside that directory, initialize and publish the catalog with Git:");
        println!("     git init --initial-branch=main");
        println!("     git add .");
        println!("     git commit -m \"Start Skilldeck catalog\"");
        println!("     git remote add origin <your-catalog-git-url>");
        println!("     git push -u origin main");
        println!("  3. Configure Skilldeck:");
        println!("     skilldeck init --repository <your-catalog-git-url> --reference main");
        println!("  4. Validate the catalog:");
        println!("     skilldeck doctor");
        if template == BootstrapTemplate::Quickstart {
            println!("  5. Try the quickstart group:");
            println!("     skilldeck install-group quickstart <install-directory>");
        }
    }
}

fn bootstrap_git_error(path: &Path, err: git::BootstrapGitError) -> anyhow::Error {
    if err.step == git::BootstrapGitStep::Commit {
        if commit_failure_is_identity_error(&err.detail) {
            anyhow!(
                "{} failed while bootstrapping {} because Git identity is not configured. Catalog files exist and Git is initialized/staged. From inside the generated repository, configure identity with `git config user.name <name>` and `git config user.email <email>`, then run `git commit -m \"{}\"`. Details: {}",
                err.step,
                path.display(),
                git::initial_commit_message(),
                err.detail
            )
        } else {
            anyhow!(
                "{} failed while bootstrapping {}. Catalog files exist and Git is initialized/staged. Fix the Git error, then run `git commit -m \"{}\"` from inside the generated repository. Details: {}",
                err.step,
                path.display(),
                git::initial_commit_message(),
                err.detail
            )
        }
    } else {
        anyhow!(
            "{} failed while bootstrapping {}. Catalog files were left in place; fix the Git issue, then continue manually. Details: {}",
            err.step,
            path.display(),
            err.detail
        )
    }
}

fn commit_failure_is_identity_error(detail: &str) -> bool {
    let detail = detail.to_ascii_lowercase();
    detail.contains("author identity unknown")
        || detail.contains("committer identity unknown")
        || detail.contains("unable to auto-detect email")
        || detail.contains("empty ident")
}

fn prompt_bootstrap_template() -> Result<BootstrapTemplate> {
    loop {
        eprintln!("Choose a catalog template:");
        eprintln!("  1. Quickstart — working examples (recommended)");
        eprintln!("  2. Empty — catalog structure only");
        eprint!("Template [1]: ");
        io::stderr().flush().ok();
        let mut s = String::new();
        if io::stdin().read_line(&mut s)? == 0 {
            return Err(anyhow!(
                "non-interactive bootstrap requires a destination path and exactly one template flag (--quickstart or --empty)"
            ));
        }
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "1" | "q" | "quickstart" | "quick" => return Ok(BootstrapTemplate::Quickstart),
            "2" | "e" | "empty" => return Ok(BootstrapTemplate::Empty),
            other => {
                eprintln!("Unknown bootstrap template `{other}`. Use 1/quickstart or 2/empty.")
            }
        }
    }
}

fn prompt_default_non_eof(label: &str, default: &str) -> Result<String> {
    eprint!("{label} [{default}]: ");
    io::stderr().flush().ok();
    let mut s = String::new();
    if io::stdin().read_line(&mut s)? == 0 {
        return Err(anyhow!(
            "non-interactive bootstrap requires a destination path and exactly one template flag (--quickstart or --empty)"
        ));
    }
    let s = s.trim();
    Ok(if s.is_empty() {
        default.into()
    } else {
        s.into()
    })
}

fn create_bootstrap_catalog(path: &Path, template: BootstrapTemplate) -> Result<()> {
    ensure_bootstrap_destination(path)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("creating parent directory {}", parent.display()))?;
    let temp = tempfile::Builder::new()
        .prefix(".skilldeck-bootstrap-")
        .tempdir_in(parent)
        .with_context(|| format!("creating temporary catalog in {}", parent.display()))?;
    write_bootstrap_template(temp.path(), template)?;

    if path.exists() {
        move_dir_contents(temp.path(), path)?;
    } else {
        fs::rename(temp.path(), path)
            .with_context(|| format!("creating catalog directory {}", path.display()))?;
    }
    Ok(())
}

fn ensure_bootstrap_destination(path: &Path) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(anyhow!(
                "refusing to bootstrap into symlink {}",
                path.display()
            ));
        }
        if !meta.is_dir() {
            return Err(anyhow!(
                "bootstrap destination exists and is not a directory: {}",
                path.display()
            ));
        }
        if fs::read_dir(path)
            .with_context(|| format!("reading bootstrap destination {}", path.display()))?
            .next()
            .transpose()?
            .is_some()
        {
            return Err(anyhow!(
                "bootstrap destination is not empty: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn move_dir_contents(from: &Path, to: &Path) -> Result<()> {
    for entry in fs::read_dir(from).with_context(|| format!("reading {}", from.display()))? {
        let entry = entry?;
        fs::rename(entry.path(), to.join(entry.file_name())).with_context(|| {
            format!(
                "moving generated catalog entry {} into {}",
                entry.path().display(),
                to.display()
            )
        })?;
    }
    Ok(())
}

fn write_bootstrap_template(root: &Path, template: BootstrapTemplate) -> Result<()> {
    fs::create_dir_all(root.join("skills"))?;
    fs::create_dir_all(root.join("partials"))?;
    fs::write(root.join("partials/.gitkeep"), "")?;
    match template {
        BootstrapTemplate::Quickstart => {
            fs::create_dir_all(root.join("skills/hello-world"))?;
            fs::write(root.join("README.md"), QUICKSTART_README)?;
            fs::write(root.join("skills/hello-world/SKILL.md"), HELLO_WORLD_SKILL)?;
            fs::write(
                root.join("external-skills.toml"),
                QUICKSTART_EXTERNAL_SKILLS,
            )?;
            fs::write(root.join("skill-groups.toml"), QUICKSTART_SKILL_GROUPS)?;
        }
        BootstrapTemplate::Empty => {
            fs::write(root.join("README.md"), EMPTY_README)?;
            fs::write(root.join("skills/.gitkeep"), "")?;
            fs::write(root.join("external-skills.toml"), EMPTY_EXTERNAL_SKILLS)?;
            fs::write(root.join("skill-groups.toml"), EMPTY_SKILL_GROUPS)?;
        }
    }
    Ok(())
}

const QUICKSTART_README: &str = r#"# Skilldeck catalog quickstart

This catalog was generated by `skilldeck bootstrap --quickstart`.

## What's included

- `skills/hello-world/SKILL.md` — a first-party example skill you can edit or replace.
- `external-skills.toml` — an external `skilldeck` skill pinned to Skilldeck v0.2.0.
- `skill-groups.toml` — a `quickstart` group containing both skills.
- `partials/` — shared MiniJinja partials for optional skill recipes.

## Try it locally

By default, `skilldeck bootstrap` already initialized this directory as a Git repository on branch `main` and created the initial commit `Start Skilldeck catalog`.

```sh
skilldeck init --repository . --reference main
skilldeck doctor
skilldeck list
skilldeck install-group quickstart ./installed-skills
```

If this catalog was generated with `--no-git`, initialize it manually from inside this directory first:

```sh
git init --initial-branch=main
git add .
git commit -m "Start Skilldeck catalog"
```

## Make it yours

Edit `skills/hello-world/SKILL.md`, add more directories under `skills/`, then update `skill-groups.toml`. Push the catalog to your Git host if desired and point Skilldeck at either this local path or that URL with `skilldeck init`.
"#;

const HELLO_WORLD_SKILL: &str = r#"---
name: hello-world
description: Use when the user asks for a simple Skilldeck catalog example, wants to verify that installed agent skills are visible, or needs a safe first skill to test installation.
---

# Hello World Skill

When this skill is available, help the user confirm their Skilldeck setup without changing project files.

## What to do

1. Say that the `hello-world` skill loaded successfully.
2. Explain where the skill was installed if that context is available.
3. Suggest a harmless next step, such as running `skilldeck list`, `skilldeck doctor`, or editing this `SKILL.md` in the catalog.

## Safety

Do not commit, push, delete files, or change global configuration unless the user explicitly asks.
"#;

const QUICKSTART_EXTERNAL_SKILLS: &str = r#"[skills.skilldeck]
source = "https://github.com/Cause-of-a-Kind/skilldeck.git"
subdirectory = "examples/skilldeck-skill"
ref = "v0.2.0"
"#;

const QUICKSTART_SKILL_GROUPS: &str = r#"[groups.quickstart]
skills = "hello-world skilldeck"
"#;

const EMPTY_README: &str = r#"# Skilldeck catalog

A Skilldeck catalog is a Git repository containing first-party skills plus optional external skill and group indexes.

## Add a first-party skill

Create `skills/<name>/SKILL.md`.

To create a composable skill instead, add `skills/<name>/recipe.toml` and `skills/<name>/SKILL.recipe.md`. Put shared recipe partials under the catalog-root `partials/` directory. Skilldeck renders these into standard `SKILL.md` files during installation.

## Add external skills

Edit `external-skills.toml`:

```toml
# [skills.example]
# source = "https://github.com/example/repo.git"
# subdirectory = "path/to/skill"
# ref = "v1.0.0"
```

## Add groups

Edit `skill-groups.toml`:

```toml
# [groups.default]
# skills = "example another-skill"
```

By default, `skilldeck bootstrap` initializes this directory as a Git repository on branch `main` and creates the initial commit `Start Skilldeck catalog`. Run `skilldeck init --repository <path-or-url> --reference main` and `skilldeck doctor`. If generated with `--no-git`, initialize and commit manually first.
"#;

const EMPTY_EXTERNAL_SKILLS: &str = r#"# External skills are optional.
#
# [skills.example]
# source = "https://github.com/example/repo.git"
# subdirectory = "path/to/skill"
# ref = "v1.0.0"
"#;

const EMPTY_SKILL_GROUPS: &str = r#"# Skill groups are optional.
#
# [groups.default]
# skills = "example another-skill"
"#;

pub fn install(args: InstallArgs, require_existing: bool) -> Result<()> {
    let values = parse_set_values(&args.values)?;
    let location = crate::harness::resolve_install_location(
        args.install_directory.as_deref(),
        args.global,
        args.claude,
        args.target,
    )?;
    let expected_name = install_name(&args.name_or_git_url);
    catalog::validate_name(&expected_name, "skill")?;
    location.warn_name_collisions(&expected_name);
    location.preflight_claude(&expected_name)?;
    let name = install_named_or_git(
        &args.name_or_git_url,
        &location.root,
        args.force,
        require_existing,
        &args.overrides,
        args.local_catalog.local.as_deref(),
        RecipeInstallOptions {
            supplied: &values,
            locked: None,
            accept_defaults: args.yes,
        },
    )?;
    location.link_claude(&name)?;
    println!(
        "{} {} at {}",
        if require_existing {
            "Updated"
        } else {
            "Installed"
        },
        name,
        location.root.join(&name).display()
    );
    Ok(())
}

fn open_catalog(cfg: &Config, local: Option<&Path>) -> Result<(Catalog, Option<PathBuf>)> {
    if let Some(local) = local {
        let path = fs::canonicalize(local)
            .with_context(|| format!("opening local catalog {}", local.display()))?;
        Ok((Catalog::open_path(path.clone())?, Some(path)))
    } else {
        Ok((Catalog::clone_from_config(cfg)?, None))
    }
}

pub fn install_group(args: GroupInstallArgs) -> Result<()> {
    let group_values = parse_group_set_values(&args.values)?;
    let location = crate::harness::resolve_install_location(
        args.install_directory.as_deref(),
        args.global,
        args.claude,
        args.target,
    )?;
    let install_root = &location.root;
    let (group, overrides) = qualified_selector(&args.group, &args.overrides, "group")?;
    catalog::validate_name(&group, "group")?;
    let cfg = config::resolve(&overrides)?;
    let (catalog, local_path) = open_catalog(&cfg, args.local_catalog.local.as_deref())?;
    if local_path.is_some() {
        catalog.validate()?;
    }
    let skills = catalog
        .group(&group)
        .ok_or_else(|| catalog::not_found("group", &group, &catalog.group_names()))?
        .clone();
    for skill in &skills {
        catalog::validate_name(skill, "skill")?;
        if !catalog.has_skill(skill) {
            return Err(anyhow!(
                "skill group {} references missing skill: {}",
                group,
                skill
            ));
        }
    }
    for selected in group_values.keys() {
        if !skills.iter().any(|skill| skill == selected) {
            return Err(anyhow!(
                "recipe values target `{selected}`, which is not a member of group `{group}`"
            ));
        }
    }
    for skill in &skills {
        location.warn_name_collisions(skill);
        location.preflight_claude(skill)?;
    }
    fsops::ensure_install_root(install_root, args.yes)?;
    let mut installed = 0;
    let mut overwritten = 0;
    let mut skipped = 0;
    let mut planned = Vec::new();
    for skill in &skills {
        let dest = fsops::destination(install_root, skill);
        let mut overwrite = args.force;
        if dest.exists() && !args.force {
            if !dest.join("SKILL.md").is_file() {
                skipped += 1;
                println!(
                    "Skipped {}: refusing to overwrite {} because it does not contain SKILL.md",
                    skill,
                    dest.display()
                );
                continue;
            }
            if confirm(&format!(
                "Overwrite existing skill {} at {}?",
                skill,
                dest.display()
            ))? {
                overwrite = true;
            } else {
                skipped += 1;
                println!("Skipped {}: already installed", skill);
                continue;
            }
        }
        planned.push((skill.clone(), dest.exists(), overwrite));
    }

    let mut prepared = Vec::new();
    for (skill, existed, overwrite) in planned {
        let values = group_values.get(&skill).cloned().unwrap_or_default();
        let local_path = install_root.join(&skill).join("SKILL.local.toml");
        let local_values = crate::recipe::load_local_values(&local_path)?;
        prepared.push((
            prepare_catalog_source(
                &skill,
                &catalog,
                &values,
                None,
                local_values.as_ref(),
                args.yes,
            )?,
            existed,
            overwrite,
        ));
    }

    for (prepared, existed, overwrite) in prepared {
        let dest = fsops::destination(install_root, &prepared.name);
        let source = match local_path.as_deref() {
            Some(path) => CatalogInstallSource::Local(path),
            None => CatalogInstallSource::Remote(&cfg),
        };
        install_prepared_catalog(&prepared, install_root, overwrite, false, source)?;
        if existed {
            overwritten += 1;
            println!("Overwritten {} at {}", prepared.name, dest.display());
        } else {
            installed += 1;
            println!("Installed {} at {}", prepared.name, dest.display());
        }
    }
    for skill in &skills {
        if install_root.join(skill).join("SKILL.md").is_file() {
            location.link_claude(skill)?;
        }
    }
    println!(
        "Group install complete: {installed} installed, {overwritten} overwritten, {skipped} skipped in {}",
        install_root.display()
    );
    Ok(())
}

pub fn update(args: UpdateArgs) -> Result<()> {
    match (args.name_or_git_url, args.install_directory) {
        (Some(root), None) => {
            if !args.values.is_empty() {
                return Err(anyhow!("--set requires a single-skill update"));
            }
            update_all(PathBuf::from(root), &args.overrides)
        }
        (Some(name), Some(root)) => {
            let values = parse_set_values(&args.values)?;
            let installed_name = name.rsplit_once(':').map_or(name.as_str(), |(_, name)| name);
            let locked = manifest::load(&root)?
                .skills
                .get(installed_name)
                .and_then(provenance_render_values)
                .cloned();
            let installed = install_named_or_git(
                &name,
                &root,
                true,
                true,
                &args.overrides,
                None,
                RecipeInstallOptions {
                    supplied: &values,
                    locked: locked.as_ref(),
                    accept_defaults: false,
                },
            )?;
            println!("Updated {} at {}", installed, root.join(&installed).display());
            Ok(())
        }
        _ => Err(anyhow!("usage: skilldeck update <install-directory> OR skilldeck update <name-or-git-url> <install-directory>")),
    }
}

pub fn remove(args: RemoveArgs) -> Result<()> {
    catalog::validate_name(&args.name, "skill")?;
    let manage_claude_alias =
        args.install_directory.is_none() && args.target == InstallTarget::Agents;
    let location = crate::harness::resolve_install_location(
        args.install_directory.as_deref(),
        args.global,
        manage_claude_alias,
        args.target,
    )?;
    if remove_one(&args.name, &location.root, true)? {
        location.unlink_claude(&args.name)?;
        let _ = manifest::forget(&location.root, &args.name);
        println!("Removed {} from {}", args.name, location.root.display());
    }
    Ok(())
}

pub fn doctor(args: DoctorArgs) -> Result<()> {
    let cfg = config::resolve(&args.overrides)?;
    let (catalog, _) = open_catalog(&cfg, args.local_catalog.local.as_deref())?;
    let summary = catalog.validate()?;
    println!("Catalog structure: ok");
    println!(
        "Found {} skills and {} groups. ({} first-party, {} external)",
        summary.total_skill_count,
        summary.group_count,
        summary.first_party_count,
        summary.external_count
    );
    validate_catalog_metadata(&catalog, args.strict)?;
    if args.deep {
        deep_validate(&catalog, args.strict)?;
    }
    println!("Doctor complete: ok");
    Ok(())
}

#[derive(Serialize)]
struct ListOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    registry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference: Option<String>,
    counts: crate::catalog::CatalogSummary,
    skills: Vec<crate::catalog::SkillEntry>,
    groups: Vec<ListGroup>,
}

#[derive(Serialize)]
struct ListGroup {
    name: String,
    members: Vec<String>,
}

fn catalog_list_output(
    registry: Option<String>,
    repository: String,
    reference: String,
    catalog: &Catalog,
) -> Result<ListOutput> {
    let counts = catalog.validate()?;
    let skills = catalog.skills();
    let groups = catalog
        .groups()
        .iter()
        .map(|(name, members)| ListGroup {
            name: name.clone(),
            members: members.clone(),
        })
        .collect();
    Ok(ListOutput {
        registry,
        repository: Some(repository),
        reference: Some(reference),
        counts,
        skills,
        groups,
    })
}

fn list_output(registry: Option<String>, cfg: Config) -> Result<ListOutput> {
    let catalog = Catalog::clone_from_config(&cfg)?;
    catalog_list_output(registry, cfg.catalog_repository, cfg.catalog_ref, &catalog)
}

fn builtin_list_output() -> ListOutput {
    let skills = builtins::names()
        .into_iter()
        .map(|name| crate::catalog::SkillEntry {
            name,
            source_type: "built-in".into(),
        })
        .collect::<Vec<_>>();
    ListOutput {
        registry: Some(builtins::REGISTRY_NAME.into()),
        repository: None,
        reference: Some(env!("CARGO_PKG_VERSION").into()),
        counts: crate::catalog::CatalogSummary {
            built_in_count: skills.len(),
            first_party_count: 0,
            external_count: 0,
            group_count: 0,
            total_skill_count: skills.len(),
        },
        skills,
        groups: Vec::new(),
    }
}

fn print_list_output(output: &ListOutput, qualified: bool) {
    if let Some(registry) = &output.registry {
        match (&output.repository, &output.reference) {
            (Some(repository), Some(reference)) => {
                println!("Registry {registry}: {repository} @ {reference}")
            }
            (None, Some(version)) => {
                println!("Registry {registry}: bundled with Skilldeck {version}")
            }
            _ => println!("Registry {registry}"),
        }
    }
    if output.counts.built_in_count > 0 {
        let noun = if output.counts.built_in_count == 1 {
            "skill"
        } else {
            "skills"
        };
        println!("Found {} built-in {noun}.", output.counts.built_in_count);
    } else {
        println!(
            "Found {} skills and {} groups. ({} first-party, {} external)",
            output.counts.total_skill_count,
            output.counts.group_count,
            output.counts.first_party_count,
            output.counts.external_count
        );
    }
    println!("Skills:");
    for skill in &output.skills {
        let name = if qualified {
            format!(
                "{}:{}",
                output.registry.as_deref().unwrap_or("default"),
                skill.name
            )
        } else {
            skill.name.clone()
        };
        println!("  [{}] {name}", skill.source_type);
    }
    if !output.groups.is_empty() {
        println!("Groups:");
    }
    for group in &output.groups {
        let name = if qualified {
            format!(
                "{}:{}",
                output.registry.as_deref().unwrap_or("default"),
                group.name
            )
        } else {
            group.name.clone()
        };
        println!("  {name}: {}", group.members.join(" "));
    }
}

pub fn list(args: ListArgs) -> Result<()> {
    if args.builtins {
        if args.local_catalog.local.is_some()
            || args.overrides.catalog_repository.is_some()
            || args.overrides.registry.is_some()
        {
            return Err(anyhow!(
                "--builtins cannot be combined with --local, registry, or catalog overrides"
            ));
        }
        let output = builtin_list_output();
        if args.json {
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            print_list_output(&output, true);
        }
        return Ok(());
    }
    if args.all {
        if args.local_catalog.local.is_some()
            || args.overrides.catalog_repository.is_some()
            || args.overrides.registry.is_some()
        {
            return Err(anyhow!(
                "--all cannot be combined with --local, registry, or catalog overrides"
            ));
        }
        let mut outputs = vec![builtin_list_output()];
        if let Some(loaded) = config::load_registries()? {
            for (name, registry) in &loaded.set.registries {
                outputs.push(list_output(Some(name.clone()), registry.as_config())?);
            }
        }
        if args.json {
            println!("{}", serde_json::to_string_pretty(&outputs)?);
        } else {
            for output in outputs {
                print_list_output(&output, true);
            }
        }
        return Ok(());
    }
    let mut overrides = args.overrides.clone();
    if let Some(name) = args.registry_name {
        if let Some(selected) = &overrides.registry {
            if selected != &name {
                return Err(anyhow!(
                    "registry argument `{name}` conflicts with --registry `{selected}`"
                ));
            }
        }
        overrides.registry = Some(name);
    }
    if overrides.catalog_repository.is_none() && overrides.registry.is_none() {
        if let Some(loaded) = config::load_registries()? {
            overrides.registry = Some(loaded.set.default_registry);
        }
    }
    let cfg = config::resolve(&overrides)?;
    let output = if let Some(local) = args.local_catalog.local.as_deref() {
        let (catalog, path) = open_catalog(&cfg, Some(local))?;
        let path = path.expect("local catalog path is present");
        catalog_list_output(
            overrides.registry,
            path.display().to_string(),
            "working-tree".into(),
            &catalog,
        )?
    } else {
        list_output(overrides.registry, cfg)?
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print_list_output(&output, false);
    }
    Ok(())
}

fn validate_catalog_metadata(catalog: &Catalog, strict: bool) -> Result<()> {
    let issues = catalog.metadata_issues()?;
    for issue in &issues {
        println!("Warning: {issue}");
    }
    if strict && !issues.is_empty() {
        Err(anyhow!(
            "skill metadata validation failed:\n- {}",
            issues.join("\n- ")
        ))
    } else {
        Ok(())
    }
}

fn validate_external_package(
    name: &str,
    ext: &catalog::ExternalSkill,
    validate_metadata: bool,
) -> Result<()> {
    if fsops::is_markdown_url(&ext.source) {
        let body = reqwest::blocking::get(&ext.source)?
            .error_for_status()?
            .text()?;
        if body.trim().is_empty() {
            return Err(anyhow!("markdown content is empty"));
        }
        if validate_metadata {
            let issues = crate::skill::validate_text(&body, name);
            if !issues.is_empty() {
                return Err(anyhow!("invalid SKILL.md:\n- {}", issues.join("\n- ")));
            }
        }
        return Ok(());
    }
    let temp = TempDir::new()?;
    let source_root = temp.path().join("source");
    git::clone_repository(&ext.source, ext.reference.as_deref(), &source_root)?;
    let source = match ext
        .subdirectory
        .as_deref()
        .filter(|path| !path.is_empty() && *path != "-")
    {
        Some(path) => {
            catalog::safe_relative_path(path)?;
            source_root.join(path)
        }
        None => source_root,
    };
    if !source.is_dir() {
        return Err(anyhow!("subdirectory not found: {}", source.display()));
    }
    let skill_md = source.join("SKILL.md");
    if !skill_md.is_file() {
        return Err(anyhow!("resolved source missing SKILL.md"));
    }
    if validate_metadata {
        let issues = crate::skill::validate_file(&skill_md, name)?;
        if !issues.is_empty() {
            return Err(anyhow!("invalid SKILL.md:\n- {}", issues.join("\n- ")));
        }
    }
    Ok(())
}

fn deep_validate(catalog: &Catalog, strict: bool) -> Result<()> {
    let mut issues = Vec::new();
    for (name, ext) in catalog.externals() {
        print!("External {name}: ");
        match deep_validate_external(catalog, name, ext, strict) {
            Ok(()) => println!("ok"),
            Err(error) => {
                println!("failed");
                issues.push(format!("external {name}: {error:#}"));
            }
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "deep catalog validation failed:\n- {}",
            issues.join("\n- ")
        ))
    }
}

fn deep_validate_external(
    catalog: &Catalog,
    name: &str,
    ext: &catalog::ExternalSkill,
    strict: bool,
) -> Result<()> {
    let temp = TempDir::new()?;
    let source_root = temp.path().join("source");
    if fsops::is_markdown_url(&ext.source) {
        fs::create_dir_all(&source_root)?;
        let body = reqwest::blocking::get(&ext.source)?
            .error_for_status()?
            .text()?;
        if body.trim().is_empty() {
            return Err(anyhow!("markdown content is empty"));
        }
        fs::write(source_root.join("SKILL.md"), body)?;
    } else {
        git::clone_repository(&ext.source, ext.reference.as_deref(), &source_root)?;
    }
    let source = match ext
        .subdirectory
        .as_deref()
        .filter(|path| !path.is_empty() && *path != "-")
    {
        Some(path) => source_root.join(path),
        None => source_root,
    };
    if !source.is_dir() {
        return Err(anyhow!("subdirectory not found: {}", source.display()));
    }
    if !source.join("SKILL.md").is_file() {
        return Err(anyhow!("resolved source missing SKILL.md"));
    }

    if let Some(recipe_path) = ext.recipe.as_deref() {
        let manifest = catalog.root().join(recipe_path);
        let recipe_dir = manifest
            .parent()
            .ok_or_else(|| anyhow!("external recipe has no parent directory"))?;
        let recipe = crate::recipe::load(recipe_dir)?;
        crate::recipe::render(
            &source,
            recipe_dir,
            catalog.root(),
            name,
            crate::recipe::validation_values(&recipe),
            crate::recipe::validation_local_values(&recipe),
        )?;
    } else if strict {
        let metadata = crate::skill::validate_file(&source.join("SKILL.md"), name)?;
        if !metadata.is_empty() {
            return Err(anyhow!("invalid SKILL.md: {}", metadata.join("; ")));
        }
    }
    Ok(())
}

pub fn remove_group(args: RemoveGroupArgs) -> Result<()> {
    let manage_claude_alias =
        args.install_directory.is_none() && args.target == InstallTarget::Agents;
    let location = crate::harness::resolve_install_location(
        args.install_directory.as_deref(),
        args.global,
        manage_claude_alias,
        args.target,
    )?;
    let install_root = &location.root;
    let (group, overrides) = qualified_selector(&args.group, &args.overrides, "group")?;
    catalog::validate_name(&group, "group")?;
    let cfg = config::resolve(&overrides)?;
    let catalog = Catalog::clone_from_config(&cfg)?;
    let skills = catalog
        .group(&group)
        .ok_or_else(|| catalog::not_found("group", &group, &catalog.group_names()))?
        .clone();
    let mut removed = 0;
    let mut skipped = 0;
    for skill in skills {
        match remove_one(&skill, install_root, false) {
            Ok(true) => {
                removed += 1;
                if let Err(error) = location.unlink_claude(&skill) {
                    println!(
                        "Warning: removed {skill}, but could not clean its Claude alias: {error:#}"
                    );
                }
                manifest::forget(install_root, &skill).ok();
                println!("Removed {} from {}", skill, install_root.display());
            }
            Ok(false) => {
                skipped += 1;
                println!("Skipped {}: not installed or missing SKILL.md", skill);
            }
            Err(e) => {
                skipped += 1;
                println!("Skipped {}: {}", skill, e);
            }
        }
    }
    println!(
        "Group removal complete: {removed} removed, {skipped} skipped from {}",
        install_root.display()
    );
    if removed == 0 {
        Err(anyhow!(
            "no installed skills from group {} were removed",
            group
        ))
    } else {
        Ok(())
    }
}

fn remove_one(name: &str, root: &Path, strict: bool) -> Result<bool> {
    let dest = root.join(name);
    if !dest.exists() {
        if strict {
            return Err(anyhow!("{} is not installed in {}", name, root.display()));
        }
        return Ok(false);
    }
    if !dest.join("SKILL.md").is_file() {
        if strict {
            return Err(anyhow!(
                "refusing to remove {} because it does not contain SKILL.md",
                dest.display()
            ));
        }
        return Ok(false);
    }
    fs::remove_dir_all(dest)?;
    Ok(true)
}

fn install_builtin(
    name: &str,
    root: &Path,
    force: bool,
    require_existing: bool,
    yes: bool,
) -> Result<String> {
    catalog::validate_name(name, "built-in skill")?;
    let (_temp, source) = builtins::materialize(name)?;
    fsops::ensure_install_root(root, yes)?;
    install_source(name, &source, root, force, require_existing)?;
    manifest::record(
        root,
        name,
        Provenance::BuiltIn {
            name: name.into(),
            skilldeck_version: env!("CARGO_PKG_VERSION").into(),
        },
    )?;
    Ok(name.into())
}

#[derive(Clone, Copy)]
struct RecipeInstallOptions<'a> {
    supplied: &'a std::collections::BTreeMap<String, String>,
    locked: Option<&'a crate::recipe::Values>,
    accept_defaults: bool,
}

fn install_named_or_git(
    requested: &str,
    root: &Path,
    force: bool,
    require_existing: bool,
    overrides: &CatalogOverrideArgs,
    local_catalog: Option<&Path>,
    recipe_options: RecipeInstallOptions<'_>,
) -> Result<String> {
    if let Some(name) = builtins::selector_name(requested) {
        if local_catalog.is_some() {
            return Err(anyhow!("--local can only be used with catalog skills"));
        }
        if overrides.catalog_repository.is_some()
            || overrides.catalog_ref.is_some()
            || overrides.registry.is_some()
        {
            return Err(anyhow!(
                "built-in skills cannot be combined with registry or catalog overrides"
            ));
        }
        if !recipe_options.supplied.is_empty() {
            return Err(anyhow!("--set can only be used with recipe skills"));
        }
        return install_builtin(
            name,
            root,
            force,
            require_existing,
            recipe_options.accept_defaults,
        );
    }
    if fsops::is_git_url(requested) {
        if local_catalog.is_some() {
            return Err(anyhow!("--local can only be used with catalog skills"));
        }
        if fsops::is_markdown_url(requested) {
            return Err(anyhow!(
                "direct Markdown URLs must be installed through a catalog entry"
            ));
        }
        if !recipe_options.supplied.is_empty() {
            return Err(anyhow!("--set can only be used with recipe skills"));
        }
        let name = dir_name_for(requested);
        catalog::validate_name(&name, "skill")?;
        let temp = TempDir::new()?;
        let src = temp.path().join("source");
        git::clone_repository(requested, None, &src)?;
        fsops::ensure_install_root(root, recipe_options.accept_defaults)?;
        install_source(&name, &src, root, force, require_existing)?;
        manifest::record(
            root,
            &name,
            Provenance::DirectGit {
                repository: requested.into(),
                reference: None,
            },
        )?;
        Ok(name)
    } else {
        let (name, selected_overrides) = qualified_selector(requested, overrides, "skill")?;
        let cfg = config::resolve(&selected_overrides)?;
        let (catalog, local_path) = open_catalog(&cfg, local_catalog)?;
        if local_path.is_some() {
            catalog.validate()?;
        }
        // Resolve the catalog entry before prompting/creating a missing install root.
        if !catalog.has_skill(&name) {
            catalog::validate_name(&name, "skill")?;
            return Err(catalog::not_found("skill", &name, &catalog.skill_names()));
        }
        fsops::ensure_install_root(root, recipe_options.accept_defaults)?;
        let source = match local_path.as_deref() {
            Some(path) => CatalogInstallSource::Local(path),
            None => CatalogInstallSource::Remote(&cfg),
        };
        install_from_catalog(
            &name,
            root,
            force,
            require_existing,
            source,
            &catalog,
            recipe_options,
        )
    }
}

fn parse_set_values(raw_values: &[String]) -> Result<std::collections::BTreeMap<String, String>> {
    let mut values = std::collections::BTreeMap::new();
    for raw in raw_values {
        let (name, value) = raw
            .split_once('=')
            .ok_or_else(|| anyhow!("recipe value must use KEY=VALUE: `{raw}`"))?;
        catalog::validate_name(name, "recipe input")?;
        if values.insert(name.to_string(), value.to_string()).is_some() {
            return Err(anyhow!("recipe input `{name}` was provided more than once"));
        }
    }
    Ok(values)
}

fn parse_group_set_values(
    raw_values: &[String],
) -> Result<std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>> {
    let mut grouped =
        std::collections::BTreeMap::<String, std::collections::BTreeMap<String, String>>::new();
    for raw in raw_values {
        let (selector, value) = raw
            .split_once('=')
            .ok_or_else(|| anyhow!("group recipe value must use SKILL.KEY=VALUE: `{raw}`"))?;
        let (skill, input) = selector.split_once('.').ok_or_else(|| {
            anyhow!("group recipe values must be qualified as SKILL.KEY=VALUE: `{raw}`")
        })?;
        catalog::validate_name(skill, "skill")?;
        catalog::validate_name(input, "recipe input")?;
        if grouped
            .entry(skill.to_string())
            .or_default()
            .insert(input.to_string(), value.to_string())
            .is_some()
        {
            return Err(anyhow!(
                "recipe input `{selector}` was provided more than once"
            ));
        }
    }
    Ok(grouped)
}

fn provenance_render_values(provenance: &Provenance) -> Option<&crate::recipe::Values> {
    match provenance {
        Provenance::Catalog {
            render: Some(render),
            ..
        }
        | Provenance::LocalCatalog {
            render: Some(render),
            ..
        } => Some(&render.values),
        _ => None,
    }
}

fn qualified_selector(
    value: &str,
    overrides: &CatalogOverrideArgs,
    kind: &str,
) -> Result<(String, CatalogOverrideArgs)> {
    let Some((registry, name)) = value.split_once(':') else {
        return Ok((value.to_string(), overrides.clone()));
    };
    config::validate_registry_name(registry)?;
    catalog::validate_name(name, kind)?;
    if let Some(selected) = &overrides.registry {
        if selected != registry {
            return Err(anyhow!(
                "qualified {kind} selects registry `{registry}`, but --registry selects `{selected}`"
            ));
        }
    }
    if overrides.catalog_repository.is_some() {
        return Err(anyhow!(
            "qualified {kind} names cannot be combined with --catalog-repository"
        ));
    }
    let mut selected = overrides.clone();
    selected.registry = Some(registry.to_string());
    Ok((name.to_string(), selected))
}

struct PreparedCatalogSource {
    name: String,
    source: PathBuf,
    render: Option<crate::recipe::RenderState>,
    _temp: Option<TempDir>,
    _rendered: Option<crate::recipe::RenderedPackage>,
}

fn prepare_catalog_source(
    name: &str,
    catalog: &Catalog,
    supplied_values: &std::collections::BTreeMap<String, String>,
    locked_values: Option<&crate::recipe::Values>,
    local_values: Option<&crate::recipe::Values>,
    accept_defaults: bool,
) -> Result<PreparedCatalogSource> {
    catalog::validate_name(name, "skill")?;
    if catalog.has_first_party(name) {
        let source = catalog.first_party_path(name);
        if crate::recipe::is_recipe_dir(&source) {
            let recipe = crate::recipe::load(&source)?;
            let (values, local_values) = crate::recipe::resolve_all_values(
                &recipe,
                supplied_values,
                locked_values,
                local_values,
                accept_defaults,
            )?;
            let rendered = crate::recipe::render(
                &source,
                &source,
                catalog.root(),
                name,
                values,
                local_values,
            )?;
            return Ok(PreparedCatalogSource {
                name: name.into(),
                source: rendered.source.clone(),
                render: Some(rendered.state.clone()),
                _temp: None,
                _rendered: Some(rendered),
            });
        }
        if !supplied_values.is_empty() {
            return Err(anyhow!("skill `{name}` does not define recipe inputs"));
        }
        return Ok(PreparedCatalogSource {
            name: name.into(),
            source,
            render: None,
            _temp: None,
            _rendered: None,
        });
    }
    if let Some(ext) = catalog.external(name) {
        let temp = TempDir::new()?;
        let source_root = temp.path().join("source");
        if fsops::is_markdown_url(&ext.source) {
            fs::create_dir_all(&source_root)?;
            let body = reqwest::blocking::get(&ext.source)?
                .error_for_status()?
                .text()?;
            fs::write(source_root.join("SKILL.md"), body)?;
        } else {
            git::clone_repository(&ext.source, ext.reference.as_deref(), &source_root)?;
        }
        let source = match ext
            .subdirectory
            .as_deref()
            .filter(|p| !p.is_empty() && *p != "-")
        {
            Some(p) => {
                catalog::safe_relative_path(p)?;
                source_root.join(p)
            }
            None => source_root,
        };
        if !source.is_dir() {
            return Err(anyhow!(
                "catalog path not found for {name}: {}",
                source.display()
            ));
        }
        if let Some(recipe_path) = ext.recipe.as_deref() {
            catalog::safe_relative_path(recipe_path)?;
            let manifest = catalog.root().join(recipe_path);
            if manifest.file_name().and_then(|name| name.to_str())
                != Some(crate::recipe::MANIFEST_FILE)
            {
                return Err(anyhow!("external recipe path must point to recipe.toml"));
            }
            let recipe_dir = manifest
                .parent()
                .ok_or_else(|| anyhow!("external recipe has no parent directory"))?;
            let recipe = crate::recipe::load(recipe_dir)?;
            let (values, local_values) = crate::recipe::resolve_all_values(
                &recipe,
                supplied_values,
                locked_values,
                local_values,
                accept_defaults,
            )?;
            let rendered = crate::recipe::render(
                &source,
                recipe_dir,
                catalog.root(),
                name,
                values,
                local_values,
            )?;
            return Ok(PreparedCatalogSource {
                name: name.into(),
                source: rendered.source.clone(),
                render: Some(rendered.state.clone()),
                _temp: Some(temp),
                _rendered: Some(rendered),
            });
        }
        if !supplied_values.is_empty() {
            return Err(anyhow!("skill `{name}` does not define recipe inputs"));
        }
        return Ok(PreparedCatalogSource {
            name: name.into(),
            source,
            render: None,
            _temp: Some(temp),
            _rendered: None,
        });
    }
    Err(catalog::not_found("skill", name, &catalog.skill_names()))
}

#[derive(Clone, Copy)]
enum CatalogInstallSource<'a> {
    Remote(&'a Config),
    Local(&'a Path),
}

fn install_prepared_catalog(
    prepared: &PreparedCatalogSource,
    root: &Path,
    force: bool,
    require_existing: bool,
    source: CatalogInstallSource<'_>,
) -> Result<String> {
    install_source(
        &prepared.name,
        &prepared.source,
        root,
        force,
        require_existing,
    )?;
    let provenance = match source {
        CatalogInstallSource::Remote(cfg) => Provenance::Catalog {
            name: prepared.name.clone(),
            catalog_repository: cfg.catalog_repository.clone(),
            catalog_ref: cfg.catalog_ref.clone(),
            render: prepared.render.clone(),
        },
        CatalogInstallSource::Local(path) => Provenance::LocalCatalog {
            name: prepared.name.clone(),
            path: path.to_string_lossy().into_owned(),
            render: prepared.render.clone(),
        },
    };
    manifest::record(root, &prepared.name, provenance)?;
    Ok(prepared.name.clone())
}

fn install_from_catalog(
    name: &str,
    root: &Path,
    force: bool,
    require_existing: bool,
    source: CatalogInstallSource<'_>,
    catalog: &Catalog,
    recipe_options: RecipeInstallOptions<'_>,
) -> Result<String> {
    let local_path = root.join(name).join("SKILL.local.toml");
    let local_values = crate::recipe::load_local_values(&local_path)?;
    let prepared = prepare_catalog_source(
        name,
        catalog,
        recipe_options.supplied,
        recipe_options.locked,
        local_values.as_ref(),
        recipe_options.accept_defaults,
    )?;
    install_prepared_catalog(&prepared, root, force, require_existing, source)
}

fn install_source(
    name: &str,
    source: &Path,
    root: &Path,
    force: bool,
    require_existing: bool,
) -> Result<()> {
    let dest = root.join(name);
    if dest.exists() {
        if !force {
            return Err(anyhow!(
                "{} already exists (use --force to replace it)",
                dest.display()
            ));
        }
    } else if require_existing {
        return Err(anyhow!("{} is not installed", dest.display()));
    }

    if dest.exists() && force && !dest.join("SKILL.md").is_file() {
        let managed = manifest::load(root)
            .map(|m| m.skills.contains_key(name))
            .unwrap_or(false);
        if !managed {
            return Err(anyhow!(
                "refusing to replace {} because it does not contain SKILL.md and is not managed by Skilldeck",
                dest.display()
            ));
        }
    }

    let stage = root.join(format!(".skilldeck-stage-{name}-{}", std::process::id()));
    let backup = root.join(format!(".skilldeck-backup-{name}-{}", std::process::id()));
    fsops::cleanup_path(&stage)?;
    fsops::cleanup_path(&backup)?;

    fsops::copy_dir_clean(source, &stage)
        .with_context(|| format!("copying {} to staging directory", source.display()))?;
    fsops::swap_staged_into(&stage, &dest, &backup)
}

fn update_all(root: PathBuf, overrides: &CatalogOverrideArgs) -> Result<()> {
    if !root.is_dir() {
        return Err(anyhow!(
            "install directory does not exist: {}",
            root.display()
        ));
    }
    let man = manifest::load(&root)?;
    let mut current_catalog: Option<(Config, Catalog)> = None;
    let mut updated = 0;
    let mut skipped = 0;
    let no_values = std::collections::BTreeMap::new();
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".skilldeck" {
            continue;
        }
        if !entry.path().join("SKILL.md").is_file() {
            continue;
        }
        match man.skills.get(&name).cloned() {
            Some(Provenance::BuiltIn {
                name: builtin_name, ..
            }) => {
                install_builtin(&builtin_name, &root, true, true, false)?;
                updated += 1;
                println!("Updated {name}: built-in skill {builtin_name}");
            }
            Some(Provenance::LocalCatalog {
                name: cat_name,
                path,
                render,
            }) => {
                let local_path = fs::canonicalize(&path)
                    .with_context(|| format!("opening local catalog {path}"))?;
                let catalog = Catalog::open_path(local_path.clone())?;
                catalog.validate()?;
                install_from_catalog(
                    &cat_name,
                    &root,
                    true,
                    true,
                    CatalogInstallSource::Local(&local_path),
                    &catalog,
                    RecipeInstallOptions {
                        supplied: &no_values,
                        locked: render.as_ref().map(|render| &render.values),
                        accept_defaults: true,
                    },
                )?;
                updated += 1;
                println!("Updated {name}: local catalog skill {cat_name}");
            }
            Some(Provenance::Catalog {
                name: cat_name,
                catalog_repository,
                catalog_ref,
                render,
            }) => {
                let cfg2 = Config {
                    catalog_repository,
                    catalog_ref,
                };
                let cat2 = Catalog::clone_from_config(&cfg2)?;
                install_from_catalog(
                    &cat_name,
                    &root,
                    true,
                    true,
                    CatalogInstallSource::Remote(&cfg2),
                    &cat2,
                    RecipeInstallOptions {
                        supplied: &no_values,
                        locked: render.as_ref().map(|render| &render.values),
                        accept_defaults: true,
                    },
                )?;
                updated += 1;
                println!("Updated {name}: catalog skill {cat_name}");
            }
            Some(Provenance::DirectGit {
                repository,
                reference,
            }) => {
                let temp = TempDir::new()?;
                let src = temp.path().join("source");
                git::clone_repository(&repository, reference.as_deref(), &src)?;
                install_source(&name, &src, &root, true, true)?;
                updated += 1;
                println!("Updated {name}: direct Git {repository}");
            }
            None => {
                if current_catalog.is_none() {
                    if let Ok(cfg) = config::resolve(overrides) {
                        if let Ok(catalog) = Catalog::clone_from_config(&cfg) {
                            current_catalog = Some((cfg, catalog));
                        }
                    }
                }
                if let Some((cfg, catalog)) = &current_catalog {
                    if catalog.has_skill(&name) {
                        install_from_catalog(
                            &name,
                            &root,
                            true,
                            true,
                            CatalogInstallSource::Remote(cfg),
                            catalog,
                            RecipeInstallOptions {
                                supplied: &no_values,
                                locked: None,
                                accept_defaults: true,
                            },
                        )?;
                        updated += 1;
                        println!("Updated {name}: catalog match");
                    } else {
                        skipped += 1;
                        println!("Skipped {name}: no provenance and no matching catalog entry");
                    }
                } else {
                    skipped += 1;
                    println!("Skipped {name}: no provenance and no configured catalog");
                }
            }
        }
    }
    println!(
        "Bulk update complete: {updated} updated, {skipped} skipped in {}",
        root.display()
    );
    if updated == 0 {
        Err(anyhow!(
            "no installed skills in {} could be updated",
            root.display()
        ))
    } else {
        Ok(())
    }
}

fn install_name(requested: &str) -> String {
    if let Some(name) = builtins::selector_name(requested) {
        name.to_string()
    } else if fsops::is_git_url(requested) {
        fsops::package_name_from_url(requested)
    } else {
        requested
            .split_once(':')
            .map_or(requested, |(_, name)| name)
            .to_string()
    }
}

fn dir_name_for(requested: &str) -> String {
    if fsops::is_git_url(requested) {
        fsops::package_name_from_url(requested)
    } else {
        requested.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_failure_identity_classification_known_git_phrases() {
        assert!(commit_failure_is_identity_error("Author identity unknown"));
        assert!(commit_failure_is_identity_error(
            "fatal: unable to auto-detect email address"
        ));
        assert!(commit_failure_is_identity_error(
            "empty ident name (for <>) not allowed"
        ));
        assert!(commit_failure_is_identity_error(
            "Committer identity unknown"
        ));
        assert!(!commit_failure_is_identity_error(
            "gpg failed to sign the data"
        ));
    }

    #[test]
    fn recipe_set_parsing_validates_single_and_group_values() {
        let values = parse_set_values(&["style=detailed".into(), "enabled=true".into()]).unwrap();
        assert_eq!(values["style"], "detailed");
        assert!(parse_set_values(&["missing".into()]).is_err());
        assert!(parse_set_values(&["style=a".into(), "style=b".into()]).is_err());

        let grouped =
            parse_group_set_values(&["review.style=detailed".into(), "audit.enabled=true".into()])
                .unwrap();
        assert_eq!(grouped["review"]["style"], "detailed");
        assert!(parse_group_set_values(&["style=detailed".into()]).is_err());
        assert!(
            parse_group_set_values(&["review.style=a".into(), "review.style=b".into()]).is_err()
        );
    }

    #[test]
    fn bootstrap_git_error_formats_identity_and_generic_commit_failures() {
        let path = Path::new("catalog with spaces");
        let identity = bootstrap_git_error(
            path,
            git::BootstrapGitError {
                step: git::BootstrapGitStep::Commit,
                detail: "Author identity unknown".into(),
            },
        )
        .to_string();
        assert!(identity.contains("because Git identity is not configured"));
        assert!(identity.contains("git config user.name <name>"));
        assert!(identity.contains("git config user.email <email>"));
        assert!(identity.contains("From inside the generated repository"));

        let generic = bootstrap_git_error(
            path,
            git::BootstrapGitError {
                step: git::BootstrapGitStep::Commit,
                detail: "gpg failed to sign the data".into(),
            },
        )
        .to_string();
        assert!(generic.contains("Fix the Git error"));
        assert!(generic.contains("gpg failed to sign the data"));
        assert!(!generic.contains("user.name"));
        assert!(!generic.contains("user.email"));
    }
}
