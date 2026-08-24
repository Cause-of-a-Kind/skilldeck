use std::{collections::BTreeMap, fs, path::Path};

use anyhow::Result;
use serde::{Deserialize, Serialize};

const DIR: &str = ".skilldeck";
const FILE: &str = "installations.toml";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub skills: BTreeMap<String, Provenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Provenance {
    BuiltIn {
        name: String,
        skilldeck_version: String,
    },
    Catalog {
        name: String,
        catalog_repository: String,
        catalog_ref: String,
    },
    LocalCatalog {
        name: String,
        path: String,
    },
    DirectGit {
        repository: String,
        #[serde(default)]
        reference: Option<String>,
    },
}

pub fn load(root: &Path) -> Result<Manifest> {
    let path = root.join(DIR).join(FILE);
    if !path.exists() {
        return Ok(Manifest::default());
    }
    Ok(toml::from_str(&fs::read_to_string(path)?)?)
}

pub fn save(root: &Path, manifest: &Manifest) -> Result<()> {
    let dir = root.join(DIR);
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(FILE), toml::to_string_pretty(manifest)?)?;
    Ok(())
}

pub fn record(root: &Path, dir_name: &str, provenance: Provenance) -> Result<()> {
    let mut m = load(root)?;
    m.skills.insert(dir_name.to_string(), provenance);
    save(root, &m)
}

pub fn forget(root: &Path, dir_name: &str) -> Result<()> {
    let mut m = load(root)?;
    m.skills.remove(dir_name);
    save(root, &m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn provenance_round_trips() {
        let tmp = TempDir::new().unwrap();
        record(
            tmp.path(),
            "builtin",
            Provenance::BuiltIn {
                name: "skilldeck".into(),
                skilldeck_version: "0.2.0".into(),
            },
        )
        .unwrap();
        record(
            tmp.path(),
            "a",
            Provenance::Catalog {
                name: "alpha".into(),
                catalog_repository: "repo".into(),
                catalog_ref: "main".into(),
            },
        )
        .unwrap();
        record(
            tmp.path(),
            "local",
            Provenance::LocalCatalog {
                name: "local-skill".into(),
                path: "/tmp/catalog".into(),
            },
        )
        .unwrap();
        record(
            tmp.path(),
            "b",
            Provenance::DirectGit {
                repository: "git@example.com:o/r.git".into(),
                reference: None,
            },
        )
        .unwrap();
        let loaded = load(tmp.path()).unwrap();
        assert!(matches!(
            loaded.skills.get("builtin"),
            Some(Provenance::BuiltIn { .. })
        ));
        assert!(matches!(
            loaded.skills.get("a"),
            Some(Provenance::Catalog { .. })
        ));
        assert!(matches!(
            loaded.skills.get("local"),
            Some(Provenance::LocalCatalog { .. })
        ));
        assert!(matches!(
            loaded.skills.get("b"),
            Some(Provenance::DirectGit { .. })
        ));
        forget(tmp.path(), "a").unwrap();
        assert!(!load(tmp.path()).unwrap().skills.contains_key("a"));
    }
}
