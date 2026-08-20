use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use strsim::jaro_winkler;
use tempfile::TempDir;

use crate::{config::Config, git};

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
pub struct SkillEntry {
    pub name: String,
    pub source_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogSummary {
    pub first_party_count: usize,
    pub external_count: usize,
    pub group_count: usize,
    pub total_skill_count: usize,
}

pub struct Catalog {
    _temp: TempDir,
    root: PathBuf,
    externals: BTreeMap<String, ExternalSkill>,
    groups: BTreeMap<String, Vec<String>>,
    first_party: BTreeMap<String, PathBuf>,
}

impl Catalog {
    pub fn clone_from_config(cfg: &Config) -> Result<Self> {
        let temp = TempDir::new()?;
        let root = temp.path().join("catalog");
        git::clone_repository(&cfg.catalog_repository, Some(&cfg.catalog_ref), &root)
            .with_context(|| format!("downloading catalog from {}", cfg.catalog_repository))?;
        Self::open(temp, root)
    }

    pub fn open(temp: TempDir, root: PathBuf) -> Result<Self> {
        let externals = read_externals(&root)?;
        let groups = read_groups(&root)?;
        let first_party = read_first_party(&root)?;
        Ok(Self {
            _temp: temp,
            root,
            externals,
            groups,
            first_party,
        })
    }

    pub fn first_party_path(&self, name: &str) -> PathBuf {
        self.first_party
            .get(name)
            .cloned()
            .unwrap_or_else(|| self.root.join("skills").join(name))
    }
    pub fn has_first_party(&self, name: &str) -> bool {
        self.first_party.contains_key(name)
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
        names.extend(self.first_party.keys().cloned());
        names.sort();
        names.dedup();
        names
    }
    pub fn group_names(&self) -> Vec<String> {
        self.groups.keys().cloned().collect()
    }
    pub fn skills(&self) -> Vec<SkillEntry> {
        let mut skills: Vec<_> = self
            .first_party
            .keys()
            .map(|name| SkillEntry {
                name: name.clone(),
                source_type: "first-party".into(),
            })
            .chain(self.externals.keys().map(|name| SkillEntry {
                name: name.clone(),
                source_type: "external".into(),
            }))
            .collect();
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        skills
    }
    pub fn groups(&self) -> &BTreeMap<String, Vec<String>> {
        &self.groups
    }
    pub fn externals(&self) -> &BTreeMap<String, ExternalSkill> {
        &self.externals
    }
    pub fn summary(&self) -> CatalogSummary {
        CatalogSummary {
            first_party_count: self.first_party.len(),
            external_count: self.externals.len(),
            group_count: self.groups.len(),
            total_skill_count: self.first_party.len() + self.externals.len(),
        }
    }
    pub fn validate(&self) -> Result<CatalogSummary> {
        let mut issues = Vec::new();
        for name in self.first_party.keys() {
            if let Err(e) = validate_name(name, "skill") {
                issues.push(e.to_string());
            }
            if self.externals.contains_key(name) {
                issues.push(format!("duplicate first-party/external skill name: {name}"));
            }
            if !self.first_party_path(name).join("SKILL.md").is_file() {
                issues.push(format!("first-party skill {name} is missing SKILL.md"));
            }
        }
        for (name, ext) in &self.externals {
            if let Err(e) = validate_name(name, "skill") {
                issues.push(e.to_string());
            }
            if ext.source.trim().is_empty() {
                issues.push(format!("external skill {name} has empty source"));
            }
            if let Some(path) = ext
                .subdirectory
                .as_deref()
                .filter(|p| !p.is_empty() && *p != "-")
            {
                if let Err(e) = safe_relative_path(path) {
                    issues.push(format!("external skill {name}: {e}"));
                }
            }
        }
        for (group, members) in &self.groups {
            if let Err(e) = validate_name(group, "group") {
                issues.push(e.to_string());
            }
            if members.is_empty() {
                issues.push(format!("group {group} is empty"));
            }
            let mut seen = std::collections::BTreeSet::new();
            for member in members {
                if !seen.insert(member) {
                    issues.push(format!("group {group} has duplicate member {member}"));
                }
                if let Err(e) = validate_name(member, "skill") {
                    issues.push(format!("group {group}: {e}"));
                }
                if !self.has_skill(member) {
                    issues.push(format!("group {group} references missing skill {member}"));
                }
            }
        }
        if self.first_party.is_empty() && self.externals.is_empty() {
            issues.push("catalog contains zero skills".into());
        }
        if issues.is_empty() {
            Ok(self.summary())
        } else {
            Err(anyhow!(
                "catalog validation failed:\n- {}",
                issues.join("\n- ")
            ))
        }
    }
}

fn read_first_party(root: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut out = BTreeMap::new();
    let skills = root.join("skills");
    if !skills.exists() {
        return Ok(out);
    }
    let entries = fs::read_dir(&skills)
        .with_context(|| format!("reading first-party skills directory {}", skills.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| {
            format!(
                "reading an entry in first-party skills directory {}",
                skills.display()
            )
        })?;
        let file_type = entry.file_type().with_context(|| {
            format!(
                "reading file type for first-party skill candidate {}",
                entry.path().display()
            )
        })?;
        if file_type.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            out.insert(name, entry.path());
        }
    }
    Ok(out)
}

fn read_externals(root: &Path) -> Result<BTreeMap<String, ExternalSkill>> {
    let path = root.join("external-skills.toml");
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let parsed: ExternalSkills = toml::from_str(&fs::read_to_string(&path)?)
        .with_context(|| format!("parsing external-skills.toml at {}", path.display()))?;
    Ok(parsed.skills)
}

fn read_groups(root: &Path) -> Result<BTreeMap<String, Vec<String>>> {
    let path = root.join("skill-groups.toml");
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let parsed: SkillGroups = toml::from_str(&fs::read_to_string(&path)?)
        .with_context(|| format!("parsing skill-groups.toml at {}", path.display()))?;
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
        fs::write(root.join("skills/first/SKILL.md"), "first").unwrap();
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

    #[test]
    fn first_party_missing_skill_md_is_reported_by_name() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("catalog");
        fs::create_dir_all(root.join("skills/broken")).unwrap();
        fs::write(root.join("external-skills.toml"), "").unwrap();
        fs::write(root.join("skill-groups.toml"), "").unwrap();
        let catalog = Catalog::open(tmp, root).unwrap();
        let err = catalog.validate().unwrap_err().to_string();
        assert!(err.contains("first-party skill broken is missing SKILL.md"));
    }

    #[test]
    fn nested_skill_md_does_not_create_extra_catalog_skill() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("catalog");
        fs::create_dir_all(root.join("skills/parent/nested")).unwrap();
        fs::write(root.join("skills/parent/SKILL.md"), "parent").unwrap();
        fs::write(root.join("skills/parent/nested/SKILL.md"), "nested").unwrap();
        fs::write(root.join("external-skills.toml"), "").unwrap();
        fs::write(root.join("skill-groups.toml"), "").unwrap();
        let catalog = Catalog::open(tmp, root).unwrap();
        let summary = catalog.validate().unwrap();
        assert_eq!(summary.first_party_count, 1);
        assert_eq!(catalog.skill_names(), vec!["parent".to_string()]);
        assert_eq!(catalog.skills()[0].name, "parent");
    }

    #[test]
    fn invalid_immediate_first_party_name_is_reported() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("catalog");
        fs::create_dir_all(root.join("skills/bad name")).unwrap();
        fs::write(root.join("skills/bad name/SKILL.md"), "bad").unwrap();
        fs::write(root.join("external-skills.toml"), "").unwrap();
        fs::write(root.join("skill-groups.toml"), "").unwrap();
        let catalog = Catalog::open(tmp, root).unwrap();
        let err = catalog.validate().unwrap_err().to_string();
        assert!(err.contains("invalid skill name: bad name"));
    }

    #[test]
    fn first_party_read_dir_errors_are_contextual() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("catalog");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("skills"), "not a directory").unwrap();
        fs::write(root.join("external-skills.toml"), "").unwrap();
        fs::write(root.join("skill-groups.toml"), "").unwrap();
        let err = match Catalog::open(tmp, root) {
            Ok(_) => panic!("expected first-party read_dir error"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("reading first-party skills directory"));
    }
}
