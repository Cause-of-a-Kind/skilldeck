use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct Frontmatter {
    name: Option<serde_yaml::Value>,
    description: Option<serde_yaml::Value>,
}

#[derive(Debug, Serialize)]
pub struct ParsedSkill {
    pub frontmatter: serde_json::Value,
    pub body: String,
}

pub fn parse(path: impl AsRef<Path>) -> Result<ParsedSkill> {
    let path = path.as_ref();
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading upstream skill at {}", path.display()))?;
    parse_text(&text).with_context(|| format!("parsing upstream skill at {}", path.display()))
}

pub fn parse_text(text: &str) -> Result<ParsedSkill> {
    let without_bom = text.strip_prefix('\u{feff}').unwrap_or(text);
    let normalized = without_bom.replace("\r\n", "\n");
    let rest = normalized
        .strip_prefix("---\n")
        .ok_or_else(|| anyhow::anyhow!("SKILL.md is missing YAML frontmatter"))?;
    let end = rest
        .find("\n---\n")
        .ok_or_else(|| anyhow::anyhow!("SKILL.md has unterminated YAML frontmatter"))?;
    let frontmatter: serde_yaml::Value = serde_yaml::from_str(&rest[..end])?;
    Ok(ParsedSkill {
        frontmatter: serde_json::to_value(frontmatter)?,
        body: rest[end + "\n---\n".len()..].to_string(),
    })
}

/// Return portable Agent Skill metadata issues without making catalog loading depend on
/// a particular validation policy.
pub fn validate_file(path: &Path, expected_name: &str) -> Result<Vec<String>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading skill metadata at {}", path.display()))?;
    Ok(validate_text(&text, expected_name))
}

pub fn validate_text(text: &str, expected_name: &str) -> Vec<String> {
    let mut issues = Vec::new();
    let without_bom = text.strip_prefix('\u{feff}').unwrap_or(text);
    let normalized = without_bom.replace("\r\n", "\n");
    let Some(rest) = normalized.strip_prefix("---\n") else {
        return vec!["SKILL.md is missing YAML frontmatter".into()];
    };
    let Some(end) = rest.find("\n---") else {
        return vec!["SKILL.md has unterminated YAML frontmatter".into()];
    };
    let yaml = &rest[..end];
    let frontmatter: Frontmatter = match serde_yaml::from_str(yaml) {
        Ok(value) => value,
        Err(error) => return vec![format!("SKILL.md has malformed YAML frontmatter: {error}")],
    };
    match frontmatter.name {
        Some(serde_yaml::Value::String(name)) if !name.trim().is_empty() => {
            if let Err(error) = crate::catalog::validate_name(&name, "skill") {
                issues.push(error.to_string());
            }
            if name != expected_name {
                issues.push(format!(
                    "SKILL.md name `{name}` does not match catalog name `{expected_name}`"
                ));
            }
        }
        Some(_) => issues.push("SKILL.md frontmatter `name` must be a non-empty string".into()),
        None => issues.push("SKILL.md frontmatter is missing `name`".into()),
    }
    match frontmatter.description {
        Some(serde_yaml::Value::String(description)) if !description.trim().is_empty() => {}
        Some(_) => {
            issues.push("SKILL.md frontmatter `description` must be a non-empty string".into())
        }
        None => issues.push("SKILL.md frontmatter is missing `description`".into()),
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsed_skill_separates_frontmatter_and_body() {
        let parsed =
            parse_text("---\r\nname: demo\r\ndescription: Demo.\r\n---\r\nBody text.\r\n").unwrap();
        assert_eq!(parsed.frontmatter["name"], "demo");
        assert_eq!(parsed.body, "Body text.\n");
        assert!(parse_text("plain markdown").is_err());
        assert!(parse_text("---\nname: demo\n").is_err());
    }

    #[test]
    fn metadata_validation_reports_missing_and_mismatched_fields() {
        assert!(validate_text("plain markdown", "demo")[0].contains("missing YAML"));
        let issues = validate_text("---\nname: other\ndescription: ''\n---\n# Demo\n", "demo");
        assert!(issues.iter().any(|issue| issue.contains("does not match")));
        assert!(issues.iter().any(|issue| issue.contains("description")));
    }
}
