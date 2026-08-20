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
