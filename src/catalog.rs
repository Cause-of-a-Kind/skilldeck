use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use strsim::jaro_winkler;
use tempfile::TempDir;

use crate::{config::Config, git};

#[derive(Debug, Clone, Deserialize)]
pub struct ExternalSkill {
    pub source: String,
    #[serde(default)]
    pub subdirectory: Option<String>,
    #[serde(default, rename = "ref")]
    pub reference: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExternalSkills {
    #[serde(default)]
    skills: BTreeMap<String, ExternalSkill>,
}
#[derive(Debug, Deserialize)]
struct SkillGroups {
    #[serde(default)]
    groups: BTreeMap<String, Group>,
}
#[derive(Debug, Deserialize)]
struct Group {
    skills: String,
}

pub struct Catalog {
    _temp: TempDir,
    root: PathBuf,
    externals: BTreeMap<String, ExternalSkill>,
    groups: BTreeMap<String, Vec<String>>,
}

impl Catalog {
    pub fn clone_from_config(cfg: &Config) -> Result<Self> {
        let temp = TempDir::new()?;
        let root = temp.path().join("catalog");
        git::clone_repository(&cfg.catalog_repository, Some(&cfg.catalog_ref), &root)
            .with_context(|| format!("downloading catalog from {}", cfg.catalog_repository))?;
        Self::open(temp, root)
    }

    fn open(temp: TempDir, root: PathBuf) -> Result<Self> {
        let externals = read_externals(&root)?;
        let groups = read_groups(&root)?;
        Ok(Self {
            _temp: temp,
            root,
            externals,
            groups,
        })
    }

    pub fn first_party_path(&self, name: &str) -> PathBuf {
        self.root.join("skills").join(name)
    }
    pub fn has_first_party(&self, name: &str) -> bool {
        self.first_party_path(name).is_dir()
    }
    pub fn external(&self, name: &str) -> Option<&ExternalSkill> {
        self.externals.get(name)
    }
    pub fn has_skill(&self, name: &str) -> bool {
        self.has_first_party(name) || self.externals.contains_key(name)
    }
    pub fn group(&self, name: &str) -> Option<&Vec<String>> {
        self.groups.get(name)
    }
    pub fn skill_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.externals.keys().cloned().collect();
        if let Ok(entries) = fs::read_dir(self.root.join("skills")) {
            for e in entries.flatten() {
                if e.path().is_dir() {
                    names.push(e.file_name().to_string_lossy().into_owned());
                }
            }
        }
        names.sort();
        names.dedup();
        names
    }
    pub fn group_names(&self) -> Vec<String> {
        self.groups.keys().cloned().collect()
    }
}

fn read_externals(root: &Path) -> Result<BTreeMap<String, ExternalSkill>> {
    let path = root.join("external-skills.toml");
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let parsed: ExternalSkills = toml::from_str(&fs::read_to_string(&path)?)
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok(parsed.skills)
}

fn read_groups(root: &Path) -> Result<BTreeMap<String, Vec<String>>> {
    let path = root.join("skill-groups.toml");
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let parsed: SkillGroups = toml::from_str(&fs::read_to_string(&path)?)
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok(parsed
        .groups
        .into_iter()
        .map(|(k, g)| (k, g.skills.split_whitespace().map(str::to_string).collect()))
        .collect())
}

pub fn not_found(kind: &str, name: &str, choices: &[String]) -> anyhow::Error {
    let best = choices
        .iter()
        .map(|c| (jaro_winkler(name, c), c))
        .max_by(|a, b| a.0.total_cmp(&b.0));
    if let Some((_score, choice)) = best.filter(|(s, _)| *s > 0.50) {
        anyhow!("{kind} not found: {name}. Did you mean `{choice}`?")
    } else {
        anyhow!("{kind} not found: {name}")
    }
}

pub fn validate_name(name: &str, kind: &str) -> Result<()> {
    let valid = !matches!(name, "." | "..")
        && !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if valid {
        Ok(())
    } else {
        Err(anyhow!("invalid {kind} name: {name}"))
    }
}

pub fn safe_relative_path(path: &str) -> Result<()> {
    let p = Path::new(path);
    if p.is_absolute()
        || p.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        Err(anyhow!("unsafe catalog subdirectory path: {path}"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn validation_suggestions_and_catalog_parsing() {
        assert!(validate_name("abc-1._", "skill").is_ok());
        assert!(validate_name("../bad", "skill").is_err());
        assert!(safe_relative_path("nested/skill").is_ok());
        assert!(safe_relative_path("../skill").is_err());
        assert!(not_found("skill", "alpah", &["alpha".into()])
            .to_string()
            .contains("alpha"));

        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("catalog");
        fs::create_dir_all(root.join("skills/first")).unwrap();
        fs::write(root.join("external-skills.toml"), "[skills.\"ext\"]\nsource = \"https://example.com/x.git\"\nsubdirectory = \"s\"\nref = \"main\"\n").unwrap();
        fs::write(
            root.join("skill-groups.toml"),
            "[groups.\"g\"]\nskills = \"first ext\"\n",
        )
        .unwrap();
        let catalog = Catalog::open(tmp, root).unwrap();
        assert!(catalog.has_skill("first"));
        assert_eq!(
            catalog.external("ext").unwrap().reference.as_deref(),
            Some("main")
        );
        assert_eq!(
            catalog.group("g").unwrap(),
            &vec!["first".to_string(), "ext".to_string()]
        );
    }
}
