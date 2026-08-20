use std::{fs, path::PathBuf};

use anyhow::{anyhow, Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::cli::CatalogOverrideArgs;

pub const DEFAULT_REF: &str = "master";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub catalog_repository: String,
    pub catalog_ref: String,
}

pub fn config_path() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("SKILLDECK_CONFIG_DIR") {
        return Ok(PathBuf::from(dir).join("config.toml"));
    }
    let dirs = ProjectDirs::from("org", "CauseOfAKind", "skilldeck")
        .ok_or_else(|| anyhow!("could not determine user config directory"))?;
    Ok(dirs.config_dir().join("config.toml"))
}

pub fn load() -> Result<Option<Config>> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let cfg: Config =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(cfg))
}

pub fn save(cfg: &Config) -> Result<PathBuf> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, toml::to_string_pretty(cfg)?)?;
    Ok(path)
}

pub fn resolve(overrides: &CatalogOverrideArgs) -> Result<Config> {
    let file = load()?;
    let repo = overrides
        .catalog_repository
        .clone()
        .or_else(|| file.as_ref().map(|c| c.catalog_repository.clone()))
        .ok_or_else(|| anyhow!("catalog repository is not configured; run `skilldeck init` or pass --catalog-repository"))?;
    let reference = overrides
        .catalog_ref
        .clone()
        .or_else(|| file.as_ref().map(|c| c.catalog_ref.clone()))
        .unwrap_or_else(|| DEFAULT_REF.to_string());
    Ok(Config {
        catalog_repository: repo,
        catalog_ref: reference,
    })
}
