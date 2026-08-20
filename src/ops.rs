use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use tempfile::TempDir;

use crate::{
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
    let path = config::save(&cfg)?;
    println!("Configured skilldeck catalog at {}", path.display());
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
- `external-skills.toml` — an external `skilldeck` skill pinned to Skilldeck v0.1.4.
- `skill-groups.toml` — a `quickstart` group containing both skills.

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
ref = "v0.1.4"
"#;

const QUICKSTART_SKILL_GROUPS: &str = r#"[groups.quickstart]
skills = "hello-world skilldeck"
"#;

const EMPTY_README: &str = r#"# Skilldeck catalog

A Skilldeck catalog is a Git repository containing first-party skills plus optional external skill and group indexes.

## Add a first-party skill

Create `skills/<name>/SKILL.md`.

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
    install_named_or_git(
        &args.name_or_git_url,
        &args.install_directory,
        args.force,
        require_existing,
        &args.overrides,
        args.yes,
    )
    .map(|name| {
        println!(
            "{} {} at {}",
            if require_existing {
                "Updated"
            } else {
                "Installed"
            },
            name,
            args.install_directory.join(&name).display()
        );
    })
}

pub fn install_group(args: GroupInstallArgs) -> Result<()> {
    catalog::validate_name(&args.group, "group")?;
    let cfg = config::resolve(&args.overrides)?;
    let catalog = Catalog::clone_from_config(&cfg)?;
    let skills = catalog
        .group(&args.group)
        .ok_or_else(|| catalog::not_found("group", &args.group, &catalog.group_names()))?
        .clone();
    for skill in &skills {
        catalog::validate_name(skill, "skill")?;
        if !catalog.has_skill(skill) {
            return Err(anyhow!(
                "skill group {} references missing skill: {}",
                args.group,
                skill
            ));
        }
    }
    fsops::ensure_install_root(&args.install_directory, args.yes)?;
    let mut installed = 0;
    let mut overwritten = 0;
    let mut skipped = 0;
    let mut planned = Vec::new();
    for skill in &skills {
        let dest = fsops::destination(&args.install_directory, skill);
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
        prepared.push((
            prepare_catalog_source(&skill, &catalog)?,
            existed,
            overwrite,
        ));
    }

    for (prepared, existed, overwrite) in prepared {
        let dest = fsops::destination(&args.install_directory, &prepared.name);
        install_prepared_catalog(&prepared, &args.install_directory, overwrite, false, &cfg)?;
        if existed {
            overwritten += 1;
            println!("Overwritten {} at {}", prepared.name, dest.display());
        } else {
            installed += 1;
            println!("Installed {} at {}", prepared.name, dest.display());
        }
    }
    println!(
        "Group install complete: {installed} installed, {overwritten} overwritten, {skipped} skipped in {}",
        args.install_directory.display()
    );
    Ok(())
}

pub fn update(args: UpdateArgs) -> Result<()> {
    match (args.name_or_git_url, args.install_directory) {
        (Some(root), None) => update_all(PathBuf::from(root), &args.overrides),
        (Some(name), Some(root)) => {
            install_named_or_git(&name, &root, true, true, &args.overrides, false)?;
            println!("Updated {} at {}", dir_name_for(&name), root.join(dir_name_for(&name)).display());
            Ok(())
        },
        _ => Err(anyhow!("usage: skilldeck update <install-directory> OR skilldeck update <name-or-git-url> <install-directory>")),
    }
}

pub fn remove(args: RemoveArgs) -> Result<()> {
    catalog::validate_name(&args.name, "skill")?;
    remove_one(&args.name, &args.install_directory, true).map(|removed| {
        if removed {
            let _ = manifest::forget(&args.install_directory, &args.name);
            println!(
                "Removed {} from {}",
                args.name,
                args.install_directory.display()
            );
        }
    })
}

pub fn doctor(args: DoctorArgs) -> Result<()> {
    let cfg = config::resolve(&args.overrides)?;
    let catalog = Catalog::clone_from_config(&cfg)?;
    let summary = catalog.validate()?;
    println!("Catalog structure: ok");
    println!(
        "Found {} skills and {} groups. ({} first-party, {} external)",
        summary.total_skill_count,
        summary.group_count,
        summary.first_party_count,
        summary.external_count
    );
    if args.deep {
        deep_validate(&catalog)?;
    }
    println!("Doctor complete: ok");
    Ok(())
}

#[derive(Serialize)]
struct ListOutput {
    repository: String,
    reference: String,
    counts: crate::catalog::CatalogSummary,
    skills: Vec<crate::catalog::SkillEntry>,
    groups: Vec<ListGroup>,
}

#[derive(Serialize)]
struct ListGroup {
    name: String,
    members: Vec<String>,
}

pub fn list(args: ListArgs) -> Result<()> {
    let cfg = config::resolve(&args.overrides)?;
    let catalog = Catalog::clone_from_config(&cfg)?;
    let counts = catalog.validate()?;
    let skills = catalog.skills();
    let groups: Vec<_> = catalog
        .groups()
        .iter()
        .map(|(name, members)| ListGroup {
            name: name.clone(),
            members: members.clone(),
        })
        .collect();
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&ListOutput {
                repository: cfg.catalog_repository,
                reference: cfg.catalog_ref,
                counts,
                skills,
                groups
            })?
        );
    } else {
        println!(
            "Found {} skills and {} groups. ({} first-party, {} external)",
            counts.total_skill_count,
            counts.group_count,
            counts.first_party_count,
            counts.external_count
        );
        println!("Skills:");
        for skill in skills {
            println!("  [{}] {}", skill.source_type, skill.name);
        }
        println!("Groups:");
        for group in groups {
            println!("  {}: {}", group.name, group.members.join(" "));
        }
    }
    Ok(())
}

fn deep_validate(catalog: &Catalog) -> Result<()> {
    let mut issues = Vec::new();
    for (name, ext) in catalog.externals() {
        print!("External {name}: ");
        if fsops::is_markdown_url(&ext.source) {
            match reqwest::blocking::get(&ext.source)
                .and_then(|r| r.error_for_status())
                .and_then(|r| r.text())
            {
                Ok(body) if !body.trim().is_empty() => println!("ok"),
                Ok(_) => {
                    println!("empty markdown");
                    issues.push(format!("external {name}: markdown content is empty"));
                }
                Err(e) => {
                    println!("failed");
                    issues.push(format!("external {name}: {e}"));
                }
            }
            continue;
        }
        let temp = TempDir::new()?;
        let source_root = temp.path().join("source");
        match git::clone_repository(&ext.source, ext.reference.as_deref(), &source_root) {
            Ok(()) => {
                let source = match ext
                    .subdirectory
                    .as_deref()
                    .filter(|p| !p.is_empty() && *p != "-")
                {
                    Some(p) => source_root.join(p),
                    None => source_root,
                };
                if !source.is_dir() {
                    println!("missing subdirectory");
                    issues.push(format!("external {name}: subdirectory not found"));
                } else if !source.join("SKILL.md").is_file() {
                    println!("missing SKILL.md");
                    issues.push(format!("external {name}: resolved source missing SKILL.md"));
                } else {
                    println!("ok");
                }
            }
            Err(e) => {
                println!("failed");
                issues.push(format!("external {name}: {e:#}"));
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

pub fn remove_group(args: RemoveGroupArgs) -> Result<()> {
    catalog::validate_name(&args.group, "group")?;
    let cfg = config::resolve(&args.overrides)?;
    let catalog = Catalog::clone_from_config(&cfg)?;
    let skills = catalog
        .group(&args.group)
        .ok_or_else(|| catalog::not_found("group", &args.group, &catalog.group_names()))?
        .clone();
    let mut removed = 0;
    let mut skipped = 0;
    for skill in skills {
        match remove_one(&skill, &args.install_directory, false) {
            Ok(true) => {
                removed += 1;
                manifest::forget(&args.install_directory, &skill).ok();
                println!(
                    "Removed {} from {}",
                    skill,
                    args.install_directory.display()
                );
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
        args.install_directory.display()
    );
    if removed == 0 {
        Err(anyhow!(
            "no installed skills from group {} were removed",
            args.group
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

fn install_named_or_git(
    requested: &str,
    root: &Path,
    force: bool,
    require_existing: bool,
    overrides: &CatalogOverrideArgs,
    yes: bool,
) -> Result<String> {
    if fsops::is_git_url(requested) {
        if fsops::is_markdown_url(requested) {
            return Err(anyhow!(
                "direct Markdown URLs must be installed through a catalog entry"
            ));
        }
        let name = dir_name_for(requested);
        catalog::validate_name(&name, "skill")?;
        let temp = TempDir::new()?;
        let src = temp.path().join("source");
        git::clone_repository(requested, None, &src)?;
        fsops::ensure_install_root(root, yes)?;
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
        let cfg = config::resolve(overrides)?;
        let catalog = Catalog::clone_from_config(&cfg)?;
        // Resolve the catalog entry before prompting/creating a missing install root.
        if !catalog.has_skill(requested) {
            catalog::validate_name(requested, "skill")?;
            return Err(catalog::not_found(
                "skill",
                requested,
                &catalog.skill_names(),
            ));
        }
        fsops::ensure_install_root(root, yes)?;
        install_from_catalog(requested, root, force, require_existing, &cfg, &catalog)
    }
}

struct PreparedCatalogSource {
    name: String,
    source: PathBuf,
    _temp: Option<TempDir>,
}

fn prepare_catalog_source(name: &str, catalog: &Catalog) -> Result<PreparedCatalogSource> {
    catalog::validate_name(name, "skill")?;
    if catalog.has_first_party(name) {
        return Ok(PreparedCatalogSource {
            name: name.into(),
            source: catalog.first_party_path(name),
            _temp: None,
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
        return Ok(PreparedCatalogSource {
            name: name.into(),
            source,
            _temp: Some(temp),
        });
    }
    Err(catalog::not_found("skill", name, &catalog.skill_names()))
}

fn install_prepared_catalog(
    prepared: &PreparedCatalogSource,
    root: &Path,
    force: bool,
    require_existing: bool,
    cfg: &Config,
) -> Result<String> {
    install_source(
        &prepared.name,
        &prepared.source,
        root,
        force,
        require_existing,
    )?;
    manifest::record(
        root,
        &prepared.name,
        Provenance::Catalog {
            name: prepared.name.clone(),
            catalog_repository: cfg.catalog_repository.clone(),
            catalog_ref: cfg.catalog_ref.clone(),
        },
    )?;
    Ok(prepared.name.clone())
}

fn install_from_catalog(
    name: &str,
    root: &Path,
    force: bool,
    require_existing: bool,
    cfg: &Config,
    catalog: &Catalog,
) -> Result<String> {
    let prepared = prepare_catalog_source(name, catalog)?;
    install_prepared_catalog(&prepared, root, force, require_existing, cfg)
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
            Some(Provenance::Catalog {
                name: cat_name,
                catalog_repository,
                catalog_ref,
            }) => {
                let cfg2 = Config {
                    catalog_repository,
                    catalog_ref,
                };
                let cat2 = Catalog::clone_from_config(&cfg2)?;
                install_from_catalog(&cat_name, &root, true, true, &cfg2, &cat2)?;
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
                        install_from_catalog(&name, &root, true, true, cfg, catalog)?;
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
