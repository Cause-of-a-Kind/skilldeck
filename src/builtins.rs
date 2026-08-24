use std::{fs, path::PathBuf};

use anyhow::{anyhow, Result};
use tempfile::TempDir;

pub const REGISTRY_NAME: &str = "builtin";
const SKILLDECK_SKILL: &str = include_str!("../examples/skilldeck-skill/SKILL.md");
const NAMES: &[&str] = &["skilldeck"];

pub fn names() -> Vec<String> {
    NAMES.iter().map(|name| (*name).to_string()).collect()
}

pub fn selector_name(value: &str) -> Option<&str> {
    value
        .strip_prefix(REGISTRY_NAME)
        .and_then(|rest| rest.strip_prefix(':'))
}

pub fn content(name: &str) -> Option<&'static str> {
    match name {
        "skilldeck" => Some(SKILLDECK_SKILL),
        _ => None,
    }
}

pub fn materialize(name: &str) -> Result<(TempDir, PathBuf)> {
    let body =
        content(name).ok_or_else(|| crate::catalog::not_found("built-in skill", name, &names()))?;
    let temp = TempDir::new()?;
    let source = temp.path().join(name);
    fs::create_dir_all(&source)?;
    fs::write(source.join("SKILL.md"), body)?;
    Ok((temp, source))
}

pub fn reject_reserved_registry(name: &str) -> Result<()> {
    if name.eq_ignore_ascii_case(REGISTRY_NAME) {
        Err(anyhow!(
            "registry name `{name}` is reserved for skills bundled with Skilldeck"
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_skill_is_valid_and_materializes() {
        assert!(crate::skill::validate_text(content("skilldeck").unwrap(), "skilldeck").is_empty());
        let (_temp, path) = materialize("skilldeck").unwrap();
        assert!(path.join("SKILL.md").is_file());
        assert!(content("missing").is_none());
    }
}
