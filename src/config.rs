use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::cli::CatalogOverrideArgs;

pub const DEFAULT_REF: &str = "master";
pub const LEGACY_REGISTRY_NAME: &str = "default";

/// A single resolved catalog. This remains the type consumed by catalog operations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub catalog_repository: String,
    pub catalog_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Registry {
    pub repository: String,
    #[serde(default = "default_ref", rename = "ref")]
    pub reference: String,
}

impl Registry {
    pub fn as_config(&self) -> Config {
        Config {
            catalog_repository: self.repository.clone(),
            catalog_ref: self.reference.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistrySet {
    pub default_registry: String,
    pub registries: BTreeMap<String, Registry>,
}

#[derive(Debug, Deserialize)]
struct LegacyConfig {
    catalog_repository: String,
    #[serde(default = "default_ref")]
    catalog_ref: String,
}

fn default_ref() -> String {
    DEFAULT_REF.into()
}

#[derive(Debug, Clone)]
pub struct LoadedRegistries {
    pub set: RegistrySet,
    pub legacy: bool,
}

pub fn config_path() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("SKILLDECK_CONFIG_DIR") {
        return Ok(PathBuf::from(dir).join("config.toml"));
    }
    let dirs = ProjectDirs::from("org", "CauseOfAKind", "skilldeck")
        .ok_or_else(|| anyhow!("could not determine user config directory"))?;
    Ok(dirs.config_dir().join("config.toml"))
}

pub fn load_registries() -> Result<Option<LoadedRegistries>> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let value: toml::Value =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    if value.get("registries").is_some() || value.get("default_registry").is_some() {
        let set: RegistrySet = value
            .try_into()
            .with_context(|| format!("parsing multi-registry config at {}", path.display()))?;
        validate_registry_set(&set)?;
        Ok(Some(LoadedRegistries { set, legacy: false }))
    } else {
        let legacy: LegacyConfig = toml::from_str(&text)
            .with_context(|| format!("parsing legacy config at {}", path.display()))?;
        let mut registries = BTreeMap::new();
        registries.insert(
            LEGACY_REGISTRY_NAME.into(),
            Registry {
                repository: legacy.catalog_repository,
                reference: legacy.catalog_ref,
            },
        );
        Ok(Some(LoadedRegistries {
            set: RegistrySet {
                default_registry: LEGACY_REGISTRY_NAME.into(),
                registries,
            },
            legacy: true,
        }))
    }
}

pub fn save_registries(set: &RegistrySet) -> Result<PathBuf> {
    validate_registry_set(set)?;
    let path = config_path()?;
    write_config_preserving_symlink(&path, toml::to_string_pretty(set)?.as_bytes())?;
    Ok(path)
}

fn write_config_preserving_symlink(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // Stow commonly creates a file symlink. Replace the target atomically rather than
    // replacing the symlink itself.
    let target = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            let linked = fs::read_link(path)
                .with_context(|| format!("reading config symlink {}", path.display()))?;
            if linked.is_absolute() {
                linked
            } else {
                path.parent().unwrap_or_else(|| Path::new(".")).join(linked)
            }
        }
        _ => path.to_path_buf(),
    };
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("creating temporary config in {}", parent.display()))?;
    use std::io::Write;
    temp.write_all(contents)?;
    temp.flush()?;
    temp.persist(&target)
        .map_err(|e| e.error)
        .with_context(|| format!("writing {}", target.display()))?;
    Ok(())
}

pub fn normalize_repository(value: &str) -> Result<String> {
    if crate::fsops::is_git_url(value) {
        return Ok(value.to_string());
    }
    Ok(fs::canonicalize(value)
        .with_context(|| format!("resolving local repository path {value}"))?
        .to_string_lossy()
        .into_owned())
}

pub fn validate_registry_name(name: &str) -> Result<()> {
    crate::builtins::reject_reserved_registry(name)?;
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if valid && !matches!(name, "." | "..") {
        Ok(())
    } else {
        Err(anyhow!("invalid registry name: {name}"))
    }
}

pub fn validate_registry_set(set: &RegistrySet) -> Result<()> {
    if set.registries.is_empty() {
        return Err(anyhow!("configuration contains no registries"));
    }
    let mut portable_names = BTreeMap::<String, String>::new();
    for (name, registry) in &set.registries {
        validate_registry_name(name)?;
        let folded = name.to_ascii_lowercase();
        if let Some(previous) = portable_names.insert(folded, name.clone()) {
            if previous != *name {
                return Err(anyhow!(
                    "case-insensitive registry name collision: {previous} and {name}"
                ));
            }
        }
        if registry.repository.trim().is_empty() {
            return Err(anyhow!("registry {name} has an empty repository"));
        }
        if registry.reference.trim().is_empty() {
            return Err(anyhow!("registry {name} has an empty ref"));
        }
    }
    if !set.registries.contains_key(&set.default_registry) {
        return Err(anyhow!(
            "default registry `{}` is not configured",
            set.default_registry
        ));
    }
    Ok(())
}

pub fn resolve(overrides: &CatalogOverrideArgs) -> Result<Config> {
    // The legacy ad-hoc repository flags intentionally retain highest precedence.
    if let Some(repo) = overrides.catalog_repository.clone() {
        let configured_ref = load_registries()?.and_then(|loaded| {
            let name = overrides
                .registry
                .as_deref()
                .unwrap_or(&loaded.set.default_registry);
            loaded
                .set
                .registries
                .get(name)
                .map(|registry| registry.reference.clone())
        });
        return Ok(Config {
            catalog_repository: repo,
            catalog_ref: overrides
                .catalog_ref
                .clone()
                .or(configured_ref)
                .unwrap_or_else(|| DEFAULT_REF.to_string()),
        });
    }

    let loaded = load_registries()?.ok_or_else(|| {
        anyhow!("catalog repository is not configured; run `skilldeck init` or `skilldeck registry add`")
    })?;
    let name = overrides
        .registry
        .as_deref()
        .unwrap_or(&loaded.set.default_registry);
    validate_registry_name(name)?;
    let registry = loaded.set.registries.get(name).ok_or_else(|| {
        let choices = loaded
            .set
            .registries
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        anyhow!("registry not found: {name}. Configured registries: {choices}")
    })?;
    let mut cfg = registry.as_config();
    if let Some(reference) = &overrides.catalog_ref {
        cfg.catalog_ref = reference.clone();
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_normalization_preserves_urls_and_absolutizes_paths() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert_eq!(
            normalize_repository("git@example.com:org/repo.git").unwrap(),
            "git@example.com:org/repo.git"
        );
        assert_eq!(
            normalize_repository(tmp.path().to_str().unwrap()).unwrap(),
            fs::canonicalize(tmp.path())
                .unwrap()
                .to_string_lossy()
                .into_owned()
        );
    }

    #[test]
    fn registry_set_validation_reports_invalid_shapes() {
        let empty = RegistrySet {
            default_registry: "default".into(),
            registries: BTreeMap::new(),
        };
        assert!(validate_registry_set(&empty)
            .unwrap_err()
            .to_string()
            .contains("no registries"));
        assert!(validate_registry_name("bad name").is_err());
        assert!(validate_registry_name("builtin")
            .unwrap_err()
            .to_string()
            .contains("reserved"));

        let registry = |repository: &str, reference: &str| Registry {
            repository: repository.into(),
            reference: reference.into(),
        };
        let mut registries = BTreeMap::new();
        registries.insert("Company".into(), registry("repo", "main"));
        registries.insert("company".into(), registry("repo", "main"));
        assert!(validate_registry_set(&RegistrySet {
            default_registry: "Company".into(),
            registries,
        })
        .unwrap_err()
        .to_string()
        .contains("case-insensitive"));

        for (repository, reference, expected) in
            [("", "main", "empty repository"), ("repo", "", "empty ref")]
        {
            let mut registries = BTreeMap::new();
            registries.insert("company".into(), registry(repository, reference));
            assert!(validate_registry_set(&RegistrySet {
                default_registry: "company".into(),
                registries,
            })
            .unwrap_err()
            .to_string()
            .contains(expected));
        }

        let mut registries = BTreeMap::new();
        registries.insert("company".into(), registry("repo", "main"));
        assert!(validate_registry_set(&RegistrySet {
            default_registry: "missing".into(),
            registries,
        })
        .unwrap_err()
        .to_string()
        .contains("not configured"));
    }

    #[test]
    fn legacy_config_becomes_default_registry() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var("SKILLDECK_CONFIG_DIR", tmp.path());
        fs::write(
            tmp.path().join("config.toml"),
            "catalog_repository = 'repo'\ncatalog_ref = 'main'\n",
        )
        .unwrap();
        let loaded = load_registries().unwrap().unwrap();
        assert!(loaded.legacy);
        assert_eq!(loaded.set.default_registry, "default");
        assert_eq!(loaded.set.registries["default"].repository, "repo");
        std::env::remove_var("SKILLDECK_CONFIG_DIR");
    }
}
