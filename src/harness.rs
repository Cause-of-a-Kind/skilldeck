use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use clap::ValueEnum;
use directories::BaseDirs;

use crate::cli::InstallTarget;

const BEGIN_EXCLUDES: &str = "# BEGIN skilldeck claude aliases";
const END_EXCLUDES: &str = "# END skilldeck claude aliases";

#[derive(Debug, Clone)]
pub struct InstallLocation {
    pub root: PathBuf,
    claude: Option<ClaudeLayout>,
    known_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct ClaudeLayout {
    canonical_root: PathBuf,
    claude_root: PathBuf,
    repository_root: Option<PathBuf>,
}

#[derive(Debug, Default)]
pub struct HarnessStatus {
    pub canonical: usize,
    pub linked: Vec<String>,
    pub missing: Vec<String>,
    pub conflicts: Vec<String>,
    pub stale: Vec<String>,
}

pub fn resolve_install_location(
    explicit: Option<&Path>,
    global: bool,
    claude: bool,
    target: InstallTarget,
) -> Result<InstallLocation> {
    if explicit.is_some() && global {
        return Err(anyhow!(
            "a custom install directory cannot be combined with --global"
        ));
    }
    if explicit.is_some() && claude {
        return Err(anyhow!(
            "--claude requires the standard project location or --global"
        ));
    }
    if explicit.is_some() && target != InstallTarget::Agents {
        return Err(anyhow!(
            "a custom install directory cannot be combined with --target"
        ));
    }
    if claude && target != InstallTarget::Agents {
        return Err(anyhow!(
            "--claude is a compatibility alias for --target agents and cannot be combined with another target"
        ));
    }
    if let Some(path) = explicit {
        return Ok(InstallLocation {
            root: path.to_path_buf(),
            claude: None,
            known_roots: Vec::new(),
        });
    }
    standard_location(global, claude, target)
}

pub fn standard_location(
    global: bool,
    claude: bool,
    target: InstallTarget,
) -> Result<InstallLocation> {
    let (base, repository_root) = if global {
        let home = BaseDirs::new()
            .ok_or_else(|| anyhow!("could not determine the user home directory"))?
            .home_dir()
            .to_path_buf();
        (home, None)
    } else {
        let cwd = env::current_dir()?;
        let root = crate::git::repository_root(&cwd).with_context(|| {
            "could not find a Git project; pass --global or an explicit install directory"
        })?;
        (root.clone(), Some(root))
    };
    let known_roots = InstallTarget::value_variants()
        .iter()
        .map(|target| target_root(&base, global, *target))
        .collect::<Vec<_>>();
    let root = target_root(&base, global, target);
    let claude_layout = claude.then(|| ClaudeLayout {
        canonical_root: root.clone(),
        claude_root: target_root(&base, global, InstallTarget::Claude),
        repository_root,
    });
    Ok(InstallLocation {
        root,
        claude: claude_layout,
        known_roots,
    })
}

fn target_root(base: &Path, global: bool, target: InstallTarget) -> PathBuf {
    let relative = match (target, global) {
        (InstallTarget::Agents, _) => ".agents/skills",
        (InstallTarget::Pi, false) => ".pi/skills",
        (InstallTarget::Pi, true) => ".pi/agent/skills",
        (InstallTarget::Codex, _) => ".codex/skills",
        (InstallTarget::Claude, _) => ".claude/skills",
        (InstallTarget::Gemini, _) => ".gemini/skills",
        (InstallTarget::Cursor, _) => ".cursor/skills",
        (InstallTarget::Opencode, false) => ".opencode/skills",
        (InstallTarget::Opencode, true) => ".config/opencode/skills",
    };
    base.join(relative)
}

impl InstallLocation {
    pub fn warn_name_collisions(&self, name: &str) {
        let selected = self.root.join(name);
        let mut conflicts = Vec::new();
        for root in &self.known_roots {
            let Ok(entries) = fs::read_dir(root) else {
                continue;
            };
            for entry in entries.flatten() {
                let candidate = entry.path();
                let skill_file = candidate.join("SKILL.md");
                if !skill_file.is_file() || candidate == selected {
                    continue;
                }
                let directory_matches = entry.file_name().to_string_lossy() == name;
                let metadata_matches = crate::skill::parse(&skill_file)
                    .ok()
                    .and_then(|parsed| parsed.frontmatter["name"].as_str().map(str::to_owned))
                    .is_some_and(|installed_name| installed_name == name);
                if (directory_matches || metadata_matches)
                    && !same_skill_path(&candidate, &selected)
                {
                    conflicts.push(candidate);
                }
            }
        }
        if !conflicts.is_empty() {
            eprintln!(
                "Warning: skill `{name}` is also installed in another harness-visible location:"
            );
            for path in conflicts {
                eprintln!("  {}", path.display());
            }
            eprintln!(
                "Harnesses that discover both locations may report a duplicate name or select one by precedence. Use a distinct skill name for a harness-specific adaptation."
            );
        }
    }

    pub fn preflight_claude(&self, name: &str) -> Result<()> {
        if let Some(layout) = &self.claude {
            preflight_alias(layout, name)?;
        }
        Ok(())
    }

    pub fn link_claude(&self, name: &str) -> Result<()> {
        if let Some(layout) = &self.claude {
            ensure_alias(layout, name)?;
            update_local_excludes(layout, [name.to_string()], false)?;
            println!(
                "Linked Claude Code alias {}",
                layout.claude_root.join(name).display()
            );
        }
        Ok(())
    }
}

pub fn status(global: bool) -> Result<HarnessStatus> {
    let location = standard_location(global, true, InstallTarget::Agents)?;
    let layout = location.claude.expect("Claude layout requested");
    inspect(&layout)
}

pub fn sync(global: bool) -> Result<HarnessStatus> {
    let location = standard_location(global, true, InstallTarget::Agents)?;
    let layout = location.claude.expect("Claude layout requested");
    if !layout.canonical_root.is_dir() {
        return Err(anyhow!(
            "canonical skill directory does not exist: {}",
            layout.canonical_root.display()
        ));
    }
    let names = canonical_skill_names(&layout.canonical_root)?;
    for name in &names {
        preflight_alias(&layout, name)?;
    }
    for name in &names {
        ensure_alias(&layout, name)?;
    }
    update_local_excludes(&layout, names, false)?;
    inspect(&layout)
}

pub fn remove(global: bool) -> Result<usize> {
    let location = standard_location(global, true, InstallTarget::Agents)?;
    let layout = location.claude.expect("Claude layout requested");
    let mut removed = 0;
    if layout.claude_root.is_dir() {
        for entry in fs::read_dir(&layout.claude_root)? {
            let entry = entry?;
            let path = entry.path();
            if is_alias_into(&path, &layout.canonical_root)? {
                remove_directory_alias(&path)
                    .with_context(|| format!("removing Claude alias {}", path.display()))?;
                removed += 1;
            }
        }
    }
    update_local_excludes(&layout, std::iter::empty(), true)?;
    Ok(removed)
}

fn inspect(layout: &ClaudeLayout) -> Result<HarnessStatus> {
    let names = if layout.canonical_root.is_dir() {
        canonical_skill_names(&layout.canonical_root)?
    } else {
        Vec::new()
    };
    let mut status = HarnessStatus {
        canonical: names.len(),
        ..HarnessStatus::default()
    };
    for name in &names {
        let alias = layout.claude_root.join(name);
        if !alias.exists() && fs::symlink_metadata(&alias).is_err() {
            status.missing.push(name.clone());
        } else if alias_matches(&alias, &layout.canonical_root.join(name))? {
            status.linked.push(name.clone());
        } else {
            status.conflicts.push(name.clone());
        }
    }
    if layout.claude_root.is_dir() {
        let canonical = names.into_iter().collect::<BTreeSet<_>>();
        for entry in fs::read_dir(&layout.claude_root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !canonical.contains(&name) && is_alias_into(&entry.path(), &layout.canonical_root)? {
                status.stale.push(name);
            }
        }
    }
    Ok(status)
}

fn canonical_skill_names(root: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(root)
        .with_context(|| format!("reading canonical skills at {}", root.display()))?
    {
        let entry = entry?;
        if entry.file_type()?.is_dir() && entry.path().join("SKILL.md").is_file() {
            let name = entry.file_name().to_string_lossy().into_owned();
            crate::catalog::validate_name(&name, "skill")?;
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

fn preflight_alias(layout: &ClaudeLayout, name: &str) -> Result<()> {
    let alias = layout.claude_root.join(name);
    match fs::symlink_metadata(&alias) {
        Ok(_) if alias_matches(&alias, &layout.canonical_root.join(name))? => Ok(()),
        Ok(_) => Err(anyhow!(
            "Claude skill path already exists and is not the managed alias: {}",
            alias.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn ensure_alias(layout: &ClaudeLayout, name: &str) -> Result<()> {
    let alias = layout.claude_root.join(name);
    if fs::symlink_metadata(&alias).is_ok() {
        return Ok(());
    }
    fs::create_dir_all(&layout.claude_root)?;
    let target = layout.canonical_root.join(name);
    if !target.join("SKILL.md").is_file() {
        return Err(anyhow!(
            "cannot link Claude alias because canonical skill is missing: {}",
            target.display()
        ));
    }
    create_directory_alias(&target, &alias)
        .with_context(|| format!("creating Claude skill alias {}", alias.display()))
}

#[cfg(unix)]
fn create_directory_alias(target: &Path, alias: &Path) -> Result<()> {
    let relative = Path::new("../..").join(".agents").join("skills").join(
        target
            .file_name()
            .ok_or_else(|| anyhow!("canonical skill has no directory name"))?,
    );
    std::os::unix::fs::symlink(relative, alias)?;
    Ok(())
}

#[cfg(windows)]
fn create_directory_alias(target: &Path, alias: &Path) -> Result<()> {
    std::os::windows::fs::symlink_dir(target, alias)?;
    Ok(())
}

#[cfg(unix)]
fn remove_directory_alias(alias: &Path) -> Result<()> {
    fs::remove_file(alias)?;
    Ok(())
}

#[cfg(windows)]
fn remove_directory_alias(alias: &Path) -> Result<()> {
    fs::remove_dir(alias)?;
    Ok(())
}

fn same_skill_path(left: &Path, right: &Path) -> bool {
    matches!(
        (fs::canonicalize(left), fs::canonicalize(right)),
        (Ok(left), Ok(right)) if left == right
    )
}

fn alias_matches(alias: &Path, canonical: &Path) -> Result<bool> {
    let metadata = match fs::symlink_metadata(alias) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_symlink() {
        return Ok(false);
    }
    let target = fs::read_link(alias)?;
    let resolved = if target.is_absolute() {
        target
    } else {
        alias
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(target)
    };
    Ok(matches!(
        (fs::canonicalize(resolved), fs::canonicalize(canonical)),
        (Ok(actual), Ok(expected)) if actual == expected
    ))
}

fn is_alias_into(alias: &Path, canonical_root: &Path) -> Result<bool> {
    let metadata = match fs::symlink_metadata(alias) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_symlink() {
        return Ok(false);
    }
    let target = fs::read_link(alias)?;
    let resolved = if target.is_absolute() {
        target
    } else {
        alias
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(target)
    };
    if let (Ok(target), Ok(root)) = (fs::canonicalize(resolved), fs::canonicalize(canonical_root)) {
        Ok(target.starts_with(root))
    } else {
        // Broken aliases created by Skilldeck use this exact relative prefix.
        Ok(fs::read_link(alias)?
            .to_string_lossy()
            .replace('\\', "/")
            .starts_with("../../.agents/skills/"))
    }
}

fn update_local_excludes<I>(layout: &ClaudeLayout, names: I, remove_block: bool) -> Result<()>
where
    I: IntoIterator<Item = String>,
{
    let Some(repository_root) = &layout.repository_root else {
        return Ok(());
    };
    let path = crate::git::exclude_path(repository_root)?;
    let existing = if path.is_file() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let (outside, mut managed) = parse_exclude_block(&existing)?;
    if remove_block {
        managed.clear();
    } else {
        managed.extend(
            names
                .into_iter()
                .map(|name| format!("/.claude/skills/{name}")),
        );
    }
    let mut output = outside.trim_end_matches('\n').to_string();
    if !managed.is_empty() {
        if !output.is_empty() {
            output.push_str("\n\n");
        }
        output.push_str(BEGIN_EXCLUDES);
        output.push('\n');
        for entry in managed {
            output.push_str(&entry);
            output.push('\n');
        }
        output.push_str(END_EXCLUDES);
    }
    if !output.is_empty() {
        output.push('\n');
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, output)?;
    Ok(())
}

fn parse_exclude_block(text: &str) -> Result<(String, BTreeSet<String>)> {
    let mut outside = Vec::new();
    let mut managed = BTreeSet::new();
    let mut in_block = false;
    let mut found_end = false;
    for line in text.lines() {
        if line == BEGIN_EXCLUDES {
            if in_block {
                return Err(anyhow!("duplicate Skilldeck block in Git exclude file"));
            }
            in_block = true;
            found_end = false;
        } else if line == END_EXCLUDES {
            if !in_block {
                return Err(anyhow!("unmatched Skilldeck block in Git exclude file"));
            }
            in_block = false;
            found_end = true;
        } else if in_block {
            if !line.trim().is_empty() {
                managed.insert(line.to_string());
            }
        } else {
            outside.push(line);
        }
    }
    if in_block || (text.contains(BEGIN_EXCLUDES) && !found_end) {
        return Err(anyhow!("unterminated Skilldeck block in Git exclude file"));
    }
    Ok((outside.join("\n"), managed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclude_blocks_preserve_unrelated_lines() {
        let text = "*.tmp\n\n# BEGIN skilldeck claude aliases\n/.claude/skills/a\n# END skilldeck claude aliases\nlocal\n";
        let (outside, managed) = parse_exclude_block(text).unwrap();
        assert!(outside.contains("*.tmp"));
        assert!(outside.contains("local"));
        assert!(managed.contains("/.claude/skills/a"));
        assert!(parse_exclude_block(BEGIN_EXCLUDES).is_err());
    }

    #[test]
    fn native_target_roots_match_project_and_global_conventions() {
        let base = Path::new("/scope");
        let cases = [
            (InstallTarget::Agents, ".agents/skills", ".agents/skills"),
            (InstallTarget::Pi, ".pi/skills", ".pi/agent/skills"),
            (InstallTarget::Codex, ".codex/skills", ".codex/skills"),
            (InstallTarget::Claude, ".claude/skills", ".claude/skills"),
            (InstallTarget::Gemini, ".gemini/skills", ".gemini/skills"),
            (InstallTarget::Cursor, ".cursor/skills", ".cursor/skills"),
            (
                InstallTarget::Opencode,
                ".opencode/skills",
                ".config/opencode/skills",
            ),
        ];
        for (target, project, global) in cases {
            assert_eq!(target_root(base, false, target), base.join(project));
            assert_eq!(target_root(base, true, target), base.join(global));
        }
    }
}
