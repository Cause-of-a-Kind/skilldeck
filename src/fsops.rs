use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use walkdir::WalkDir;

pub fn ensure_install_root(path: &Path, yes: bool) -> Result<()> {
    if path.exists() {
        if path.is_dir() {
            return Ok(());
        }
        return Err(anyhow!(
            "install location is not a directory: {}",
            path.display()
        ));
    }
    if yes || prompt_create(path)? {
        fs::create_dir_all(path).with_context(|| format!("creating {}", path.display()))?;
        Ok(())
    } else {
        Err(anyhow!(
            "install directory does not exist: {}",
            path.display()
        ))
    }
}

fn prompt_create(path: &Path) -> Result<bool> {
    eprint!(
        "Install directory {} does not exist. Create it? [y/N] ",
        path.display()
    );
    io::stderr().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(matches!(input.trim(), "y" | "Y" | "yes" | "YES" | "Yes"))
}

pub fn copy_dir_clean(source: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for entry in WalkDir::new(source)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
    {
        let entry = entry?;
        let rel = entry.path().strip_prefix(source)?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        if rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(anyhow!("unsafe source path"));
        }
        let target = dest.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(p) = target.parent() {
                fs::create_dir_all(p)?;
            }
            fs::copy(entry.path(), target)?;
        }
    }
    remove_git_metadata(dest)?;
    Ok(())
}

pub fn remove_git_metadata(dest: &Path) -> Result<()> {
    for entry in WalkDir::new(dest)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_name() == ".git")
        .collect::<Vec<_>>()
    {
        let p = entry.path();
        if p.is_dir() {
            fs::remove_dir_all(p)?;
        } else {
            fs::remove_file(p)?;
        }
    }
    Ok(())
}

pub fn package_name_from_url(value: &str) -> String {
    let value = value
        .split(['?', '#'])
        .next()
        .unwrap_or(value)
        .trim_end_matches('/');
    let last = value.rsplit(['/', ':']).next().unwrap_or(value);
    last.strip_suffix(".git").unwrap_or(last).to_string()
}

pub fn is_git_url(s: &str) -> bool {
    s.contains("://")
        || s.split_once(':').is_some_and(|(host, path)| {
            host.contains('@') && !host.contains(['/', '\\']) && !path.is_empty()
        })
}
pub fn is_markdown_url(s: &str) -> bool {
    s.split(['?', '#']).next().unwrap_or(s).ends_with(".md")
}
pub fn destination(root: &Path, name: &str) -> PathBuf {
    root.join(name)
}

pub fn cleanup_path(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn swap_staged_into(stage: &Path, dest: &Path, backup: &Path) -> Result<()> {
    swap_staged_with_hook(stage, dest, backup, || Ok(()))
}

pub fn swap_staged_with_hook<F>(
    stage: &Path,
    dest: &Path,
    backup: &Path,
    before_final_rename: F,
) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    cleanup_path(backup)?;
    if dest.exists() {
        fs::rename(dest, backup).with_context(|| {
            format!(
                "moving existing {} to backup {}",
                dest.display(),
                backup.display()
            )
        })?;
    }

    if let Err(err) = before_final_rename() {
        let _ = cleanup_path(dest);
        if backup.exists() {
            let _ = fs::rename(backup, dest);
        }
        let _ = cleanup_path(stage);
        return Err(err)
            .with_context(|| format!("preparing to move staged skill into {}", dest.display()));
    }

    match fs::rename(stage, dest) {
        Ok(()) => {
            cleanup_path(backup)?;
            Ok(())
        }
        Err(err) => {
            let _ = cleanup_path(dest);
            if backup.exists() {
                let _ = fs::rename(backup, dest);
            }
            let _ = cleanup_path(stage);
            Err(err).with_context(|| format!("moving staged skill into {}", dest.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn package_name_parsing_handles_common_urls() {
        assert_eq!(
            package_name_from_url("https://x/y/my-skill.git"),
            "my-skill"
        );
        assert_eq!(package_name_from_url("git@github.com:o/repo.git"), "repo");
        assert_eq!(package_name_from_url("file:///tmp/repo/"), "repo");
    }

    #[test]
    fn swap_rollback_restores_existing_on_final_rename_failure() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("skill");
        let stage = tmp.path().join("stage");
        let backup = tmp.path().join("backup");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("SKILL.md"), "old").unwrap();
        fs::create_dir_all(&stage).unwrap();
        fs::write(stage.join("SKILL.md"), "new").unwrap();

        let result = swap_staged_with_hook(&stage, &dest, &backup, || {
            fs::create_dir_all(&dest)?;
            fs::write(dest.join("blocking-child"), "block final rename")?;
            Ok(())
        });

        assert!(result.is_err());
        assert_eq!(fs::read_to_string(dest.join("SKILL.md")).unwrap(), "old");
        assert!(!backup.exists());
    }

    #[test]
    fn swap_rollback_restores_existing_on_hook_failure() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("skill");
        let stage = tmp.path().join("stage");
        let backup = tmp.path().join("backup");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("SKILL.md"), "old").unwrap();
        fs::create_dir_all(&stage).unwrap();
        fs::write(stage.join("SKILL.md"), "new").unwrap();

        let result = swap_staged_with_hook(&stage, &dest, &backup, || {
            Err(anyhow!("injected hook failure"))
        });

        assert!(result.is_err());
        assert_eq!(fs::read_to_string(dest.join("SKILL.md")).unwrap(), "old");
        assert!(!backup.exists());
        assert!(!stage.exists());
    }
}
