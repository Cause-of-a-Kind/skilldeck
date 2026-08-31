use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use minijinja::{Environment, UndefinedBehavior};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use walkdir::WalkDir;

use crate::{catalog, fsops};

pub const MANIFEST_FILE: &str = "recipe.toml";
const RECIPE_SUFFIX: &str = ".recipe.md";

#[derive(Debug, Clone, Deserialize)]
pub struct Recipe {
    pub version: u32,
    pub template: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, Input>,
    #[serde(default)]
    pub local_inputs: BTreeMap<String, Input>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Input {
    #[serde(rename = "type")]
    pub kind: InputType,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<toml::Value>,
    #[serde(default)]
    pub example: Option<toml::Value>,
    #[serde(default)]
    pub choices: Vec<String>,
    #[serde(default = "default_true")]
    pub allow_invocation_override: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InputType {
    String,
    Boolean,
    Integer,
    Choice,
}

pub type Values = BTreeMap<String, toml::Value>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RenderState {
    pub format_version: u32,
    #[serde(default)]
    pub values: Values,
}

pub struct RenderedPackage {
    pub source: PathBuf,
    pub state: RenderState,
    _temp: TempDir,
}

pub fn is_recipe_dir(path: &Path) -> bool {
    path.join(MANIFEST_FILE).is_file()
}

pub fn load(path: &Path) -> Result<Recipe> {
    let manifest = path.join(MANIFEST_FILE);
    let recipe: Recipe = toml::from_str(
        &fs::read_to_string(&manifest)
            .with_context(|| format!("reading recipe manifest {}", manifest.display()))?,
    )
    .with_context(|| format!("parsing recipe manifest {}", manifest.display()))?;
    validate_schema(&recipe, path)?;
    Ok(recipe)
}

fn validate_schema(recipe: &Recipe, recipe_dir: &Path) -> Result<()> {
    if recipe.version != 1 {
        return Err(anyhow!(
            "unsupported recipe version {} in {} (supported: 1)",
            recipe.version,
            recipe_dir.join(MANIFEST_FILE).display()
        ));
    }
    catalog::safe_relative_path(&recipe.template)?;
    if !recipe.template.ends_with(RECIPE_SUFFIX) {
        return Err(anyhow!(
            "recipe template must end in {RECIPE_SUFFIX}: {}",
            recipe.template
        ));
    }
    if !recipe_dir.join(&recipe.template).is_file() {
        return Err(anyhow!(
            "recipe template not found: {}",
            recipe_dir.join(&recipe.template).display()
        ));
    }
    for name in recipe.inputs.keys() {
        if recipe.local_inputs.contains_key(name) {
            return Err(anyhow!(
                "recipe input `{name}` cannot be both a render input and a local input"
            ));
        }
    }
    for (name, input) in recipe.inputs.iter().chain(&recipe.local_inputs) {
        validate_input_name(name)?;
        if matches!(input.kind, InputType::Choice) && input.choices.is_empty() {
            return Err(anyhow!("choice input `{name}` must declare choices"));
        }
        if !matches!(input.kind, InputType::Choice) && !input.choices.is_empty() {
            return Err(anyhow!("only choice inputs may declare choices (`{name}`)"));
        }
        if let Some(value) = &input.default {
            validate_value(name, input, value)?;
        }
        if let Some(value) = &input.example {
            validate_value(name, input, value)?;
        }
        if input.required && input.default.is_none() && input.example.is_none() {
            return Err(anyhow!(
                "required recipe input `{name}` needs an example for catalog validation"
            ));
        }
    }
    Ok(())
}

fn validate_input_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    if !matches!(chars.next(), Some('a'..='z' | 'A'..='Z' | '_'))
        || !chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(anyhow!("recipe input `{name}` must be an ASCII identifier"));
    }
    if matches!(name, "skill" | "values" | "upstream") {
        return Err(anyhow!("recipe input name `{name}` is reserved"));
    }
    Ok(())
}

fn validate_value(name: &str, input: &Input, value: &toml::Value) -> Result<()> {
    let valid_type = match input.kind {
        InputType::String | InputType::Choice => value.is_str(),
        InputType::Boolean => value.is_bool(),
        InputType::Integer => value.is_integer(),
    };
    if !valid_type {
        return Err(anyhow!(
            "recipe input `{name}` has a value that does not match its declared type"
        ));
    }
    if matches!(input.kind, InputType::Choice) {
        let selected = value.as_str().expect("choice type checked");
        if !input.choices.iter().any(|choice| choice == selected) {
            return Err(anyhow!(
                "recipe input `{name}` must be one of: {}",
                input.choices.join(", ")
            ));
        }
    }
    Ok(())
}

pub fn validation_values(recipe: &Recipe) -> Values {
    recipe
        .inputs
        .iter()
        .filter_map(|(name, input)| {
            input
                .default
                .clone()
                .or_else(|| input.example.clone())
                .map(|value| (name.clone(), value))
        })
        .collect()
}

pub fn validation_local_values(recipe: &Recipe) -> Values {
    example_values(&recipe.local_inputs)
}

fn example_values(inputs: &BTreeMap<String, Input>) -> Values {
    inputs
        .iter()
        .filter_map(|(name, input)| {
            input
                .default
                .clone()
                .or_else(|| input.example.clone())
                .map(|value| (name.clone(), value))
        })
        .collect()
}

#[cfg(test)]
fn resolve_values(
    recipe: &Recipe,
    supplied: &BTreeMap<String, String>,
    locked: Option<&Values>,
    accept_defaults: bool,
) -> Result<Values> {
    resolve_declared_values(&recipe.inputs, supplied, locked, accept_defaults)
}

pub fn resolve_all_values(
    recipe: &Recipe,
    supplied: &BTreeMap<String, String>,
    locked: Option<&Values>,
    local: Option<&Values>,
    accept_defaults: bool,
) -> Result<(Values, Values)> {
    let mut render_supplied = BTreeMap::new();
    let mut local_supplied = BTreeMap::new();
    for (name, value) in supplied {
        if recipe.inputs.contains_key(name) {
            render_supplied.insert(name.clone(), value.clone());
        } else if recipe.local_inputs.contains_key(name) {
            local_supplied.insert(name.clone(), value.clone());
        } else {
            return Err(anyhow!("unknown recipe input `{name}`"));
        }
    }
    Ok((
        resolve_declared_values(&recipe.inputs, &render_supplied, locked, accept_defaults)?,
        resolve_declared_values(
            &recipe.local_inputs,
            &local_supplied,
            local,
            accept_defaults,
        )?,
    ))
}

fn resolve_declared_values(
    inputs: &BTreeMap<String, Input>,
    supplied: &BTreeMap<String, String>,
    locked: Option<&Values>,
    accept_defaults: bool,
) -> Result<Values> {
    for name in supplied.keys() {
        if !inputs.contains_key(name) {
            return Err(anyhow!("unknown recipe input `{name}`"));
        }
    }
    let mut values = Values::new();
    for (name, input) in inputs {
        let value = if let Some(raw) = supplied.get(name) {
            Some(parse_value(name, input, raw)?)
        } else if let Some(value) = locked.and_then(|values| values.get(name)).cloned() {
            validate_value(name, input, &value)?;
            Some(value)
        } else if accept_defaults {
            input.default.clone()
        } else {
            prompt_value(name, input)?
        };

        let value = value.or_else(|| input.default.clone());
        if let Some(value) = value {
            validate_value(name, input, &value)?;
            values.insert(name.clone(), value);
        } else if input.required {
            return Err(anyhow!(
                "missing required recipe input `{name}`; provide it with --set {name}=<value>"
            ));
        }
    }
    Ok(values)
}

pub fn load_local_values(path: &Path) -> Result<Option<Values>> {
    if !path.is_file() {
        return Ok(None);
    }
    let values = toml::from_str(&fs::read_to_string(path)?)
        .with_context(|| format!("parsing local skill configuration {}", path.display()))?;
    Ok(Some(values))
}

fn parse_value(name: &str, input: &Input, raw: &str) -> Result<toml::Value> {
    let value = match input.kind {
        InputType::String | InputType::Choice => toml::Value::String(raw.to_string()),
        InputType::Boolean => match raw.to_ascii_lowercase().as_str() {
            "true" | "yes" | "y" | "1" => toml::Value::Boolean(true),
            "false" | "no" | "n" | "0" => toml::Value::Boolean(false),
            _ => return Err(anyhow!("recipe input `{name}` expects a boolean")),
        },
        InputType::Integer => toml::Value::Integer(
            raw.parse()
                .with_context(|| format!("recipe input `{name}` expects an integer"))?,
        ),
    };
    validate_value(name, input, &value)?;
    Ok(value)
}

fn prompt_value(name: &str, input: &Input) -> Result<Option<toml::Value>> {
    let label = input.prompt.as_deref().unwrap_or(name);
    let choices = if input.choices.is_empty() {
        String::new()
    } else {
        format!(" [{}]", input.choices.join("/"))
    };
    let default = input
        .default
        .as_ref()
        .map(display_value)
        .map(|value| format!(" ({value})"))
        .unwrap_or_default();
    eprint!("{label}{choices}{default}: ");
    io::stderr().flush().ok();
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer)? == 0 || answer.trim().is_empty() {
        return Ok(input.default.clone());
    }
    parse_value(name, input, answer.trim()).map(Some)
}

fn display_value(value: &toml::Value) -> String {
    match value {
        toml::Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

pub fn render(
    source: &Path,
    recipe_dir: &Path,
    catalog_root: &Path,
    skill_name: &str,
    values: Values,
    local_values: Values,
) -> Result<RenderedPackage> {
    let recipe = load(recipe_dir)?;
    for (name, value) in &values {
        let input = recipe
            .inputs
            .get(name)
            .ok_or_else(|| anyhow!("unknown recipe input `{name}`"))?;
        validate_value(name, input, value)?;
    }
    for (name, input) in &recipe.inputs {
        if input.required && !values.contains_key(name) {
            return Err(anyhow!("missing required recipe input `{name}`"));
        }
    }
    for (name, input) in &recipe.local_inputs {
        let Some(value) = local_values.get(name) else {
            if input.required {
                return Err(anyhow!("missing required local recipe input `{name}`"));
            }
            continue;
        };
        validate_value(name, input, value)?;
    }

    let temp = TempDir::new()?;
    let package = temp.path().join("package");
    fsops::copy_dir_clean(source, &package)?;
    let recipe_is_source = source == recipe_dir;
    if recipe_is_source {
        remove_build_sources(&package)?;
    }

    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    let templates = template_files(recipe_dir, catalog_root)?;
    for (name, path) in &templates {
        let body = fs::read_to_string(path)
            .with_context(|| format!("reading recipe template {}", path.display()))?;
        env.add_template_owned(name.clone(), body)
            .with_context(|| format!("loading recipe template {}", path.display()))?;
    }

    let mut context = serde_json::Map::new();
    let values_json = serde_json::to_value(&values)?;
    context.insert("values".into(), values_json.clone());
    context.insert(
        "skill".into(),
        serde_json::json!({
            "name": skill_name,
            "source_type": if recipe_is_source { "first-party" } else { "external" }
        }),
    );
    for (name, value) in &values {
        context.insert(name.clone(), serde_json::to_value(value)?);
    }
    if !recipe_is_source {
        let upstream = crate::skill::parse(source.join("SKILL.md"))?;
        context.insert("upstream".into(), serde_json::to_value(upstream)?);
    }

    for (name, path) in templates
        .iter()
        .filter(|(name, _)| !name.starts_with("partials/"))
    {
        let relative = path.strip_prefix(recipe_dir)?;
        let output_relative = rendered_path(relative)?;
        let output = package.join(&output_relative);
        if output.exists() && (recipe_is_source || output_relative != Path::new("SKILL.md")) {
            return Err(anyhow!(
                "recipe output collides with an existing file: {}",
                output_relative.display()
            ));
        }
        let rendered = env
            .get_template(name)?
            .render(&context)
            .with_context(|| format!("rendering recipe template {}", path.display()))?;
        if rendered.len() > 1024 * 1024 {
            return Err(anyhow!(
                "rendered recipe output exceeds 1 MiB: {}",
                output_relative.display()
            ));
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&output, rendered)?;
    }

    if !package.join("SKILL.md").is_file() {
        return Err(anyhow!("recipe did not produce SKILL.md"));
    }
    if !recipe.local_inputs.is_empty() {
        write_local_configuration(&package, &recipe, &local_values)?;
    }
    let issues = crate::skill::validate_file(&package.join("SKILL.md"), skill_name)?;
    if !issues.is_empty() {
        return Err(anyhow!(
            "rendered SKILL.md is invalid:\n- {}",
            issues.join("\n- ")
        ));
    }

    Ok(RenderedPackage {
        source: package,
        state: RenderState {
            format_version: 1,
            values,
        },
        _temp: temp,
    })
}

fn write_local_configuration(package: &Path, recipe: &Recipe, values: &Values) -> Result<()> {
    let skill_path = package.join("SKILL.md");
    let mut skill = fs::read_to_string(&skill_path)?;
    if !skill.ends_with('\n') {
        skill.push('\n');
    }
    skill.push_str("\n## Before starting: local overrides\n\n");
    skill.push_str(
        "1. Read `SKILL.local.toml` next to this file when it exists.\n\
2. Apply its configured local defaults.\n\
3. Treat explicit `key=value` arguments from the user as overrides only where allowed below.\n\
4. Fall back to the documented defaults when no local or invocation value is present.\n\n",
    );
    skill.push_str("Supported local values:\n\n");
    for (name, input) in &recipe.local_inputs {
        let label = input.prompt.as_deref().unwrap_or(name);
        skill.push_str(&format!("- `{name}` — {label}."));
        if let Some(default) = &input.default {
            skill.push_str(&format!(" Default: `{}`.", display_value(default)));
        }
        if !input.choices.is_empty() {
            skill.push_str(&format!(" Choices: `{}`.", input.choices.join("`, `")));
        }
        if input.allow_invocation_override {
            skill.push_str(" May be overridden for one invocation with `key=value`.");
        } else {
            skill.push_str(" Invocation-time overrides are not allowed.");
        }
        skill.push('\n');
    }
    skill.push_str(
        "\nDo not modify or commit `SKILL.local.toml` unless the user explicitly asks.\n",
    );
    fs::write(skill_path, skill)?;

    fs::write(
        package.join("SKILL.local.toml"),
        toml::to_string_pretty(values)?,
    )?;
    let examples = example_values(&recipe.local_inputs);
    let mut example = String::from(
        "# Copy this file to SKILL.local.toml to customize this skill for your machine.\n",
    );
    example.push_str(&toml::to_string_pretty(&examples)?);
    fs::write(package.join("SKILL.local.example.toml"), example)?;

    let ignore_path = package.join(".gitignore");
    let mut ignore = if ignore_path.is_file() {
        fs::read_to_string(&ignore_path)?
    } else {
        String::new()
    };
    if !ignore
        .lines()
        .any(|line| line.trim() == "/SKILL.local.toml")
    {
        if !ignore.is_empty() && !ignore.ends_with('\n') {
            ignore.push('\n');
        }
        ignore.push_str("/SKILL.local.toml\n");
        fs::write(ignore_path, ignore)?;
    }
    Ok(())
}

fn template_files(recipe_dir: &Path, catalog_root: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut templates = BTreeMap::new();
    collect_templates(recipe_dir, recipe_dir, "", &mut templates)?;
    let partials = catalog_root.join("partials");
    if partials.is_dir() {
        collect_templates(&partials, catalog_root, "", &mut templates)?;
    }
    Ok(templates)
}

fn collect_templates(
    directory: &Path,
    logical_root: &Path,
    _prefix: &str,
    output: &mut BTreeMap<String, PathBuf>,
) -> Result<()> {
    for entry in WalkDir::new(directory).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file()
            || !entry.file_name().to_string_lossy().ends_with(RECIPE_SUFFIX)
        {
            continue;
        }
        let relative = entry.path().strip_prefix(logical_root)?;
        let name = relative.to_string_lossy().replace('\\', "/");
        if output
            .insert(name.clone(), entry.path().to_path_buf())
            .is_some()
        {
            return Err(anyhow!("duplicate recipe template name `{name}`"));
        }
    }
    Ok(())
}

fn rendered_path(path: &Path) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("recipe template has a non-portable filename"))?;
    let stem = file_name
        .strip_suffix(RECIPE_SUFFIX)
        .ok_or_else(|| anyhow!("recipe template must end in {RECIPE_SUFFIX}"))?;
    Ok(path.with_file_name(format!("{stem}.md")))
}

fn remove_build_sources(package: &Path) -> Result<()> {
    let mut files = WalkDir::new(package)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.path().to_path_buf())
        .filter(|path| {
            path.file_name().is_some_and(|name| name == MANIFEST_FILE)
                || path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(RECIPE_SUFFIX))
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for file in files {
        fs::remove_file(file)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(kind: InputType) -> Input {
        Input {
            kind,
            prompt: None,
            required: false,
            default: None,
            example: None,
            choices: Vec::new(),
            allow_invocation_override: true,
        }
    }

    #[test]
    fn recipe_schema_rejects_invalid_manifests() {
        let temp = TempDir::new().unwrap();
        let directory = temp.path();
        fs::write(directory.join("SKILL.recipe.md"), "template").unwrap();

        fs::write(
            directory.join(MANIFEST_FILE),
            "version = 2\ntemplate = \"SKILL.recipe.md\"\n",
        )
        .unwrap();
        assert!(load(directory).unwrap_err().to_string().contains("version"));

        fs::write(
            directory.join(MANIFEST_FILE),
            "version = 1\ntemplate = \"SKILL.md\"\n",
        )
        .unwrap();
        assert!(load(directory)
            .unwrap_err()
            .to_string()
            .contains(RECIPE_SUFFIX));

        fs::write(
            directory.join(MANIFEST_FILE),
            "version = 1\ntemplate = \"missing.recipe.md\"\n",
        )
        .unwrap();
        assert!(load(directory)
            .unwrap_err()
            .to_string()
            .contains("not found"));

        fs::write(
            directory.join(MANIFEST_FILE),
            "version = 1\ntemplate = \"../SKILL.recipe.md\"\n",
        )
        .unwrap();
        assert!(load(directory).unwrap_err().to_string().contains("unsafe"));

        let mut recipe = Recipe {
            version: 1,
            template: "SKILL.recipe.md".into(),
            inputs: BTreeMap::from([("choice".into(), input(InputType::Choice))]),
            local_inputs: BTreeMap::new(),
        };
        assert!(validate_schema(&recipe, directory)
            .unwrap_err()
            .to_string()
            .contains("declare choices"));
        recipe.inputs.insert(
            "bad-name".into(),
            Input {
                choices: vec!["x".into()],
                ..input(InputType::String)
            },
        );
        assert!(validate_schema(&recipe, directory)
            .unwrap_err()
            .to_string()
            .contains("ASCII identifier"));
        recipe.inputs = BTreeMap::from([(
            "required".into(),
            Input {
                required: true,
                ..input(InputType::Integer)
            },
        )]);
        assert!(validate_schema(&recipe, directory)
            .unwrap_err()
            .to_string()
            .contains("needs an example"));
        recipe.inputs = BTreeMap::from([("upstream".into(), input(InputType::String))]);
        assert!(validate_schema(&recipe, directory)
            .unwrap_err()
            .to_string()
            .contains("reserved"));
        recipe.inputs = BTreeMap::from([("model".into(), input(InputType::String))]);
        recipe.local_inputs = BTreeMap::from([("model".into(), input(InputType::String))]);
        assert!(validate_schema(&recipe, directory)
            .unwrap_err()
            .to_string()
            .contains("both a render input and a local input"));
    }

    #[test]
    fn value_resolution_rejects_unknown_missing_and_invalid_values() {
        let recipe = Recipe {
            version: 1,
            template: "SKILL.recipe.md".into(),
            inputs: BTreeMap::from([(
                "count".into(),
                Input {
                    required: true,
                    example: Some(1.into()),
                    ..input(InputType::Integer)
                },
            )]),
            local_inputs: BTreeMap::new(),
        };
        assert!(resolve_values(
            &recipe,
            &BTreeMap::from([("other".into(), "1".into())]),
            None,
            true,
        )
        .unwrap_err()
        .to_string()
        .contains("unknown"));
        assert!(resolve_values(&recipe, &BTreeMap::new(), None, true)
            .unwrap_err()
            .to_string()
            .contains("missing required"));
        assert!(resolve_values(
            &recipe,
            &BTreeMap::from([("count".into(), "nope".into())]),
            None,
            true,
        )
        .unwrap_err()
        .to_string()
        .contains("integer"));
        let values = resolve_values(
            &recipe,
            &BTreeMap::from([("count".into(), "4".into())]),
            None,
            true,
        )
        .unwrap();
        assert_eq!(values["count"].as_integer(), Some(4));
    }

    #[test]
    fn typed_values_are_parsed_and_validated() {
        let recipe = Recipe {
            version: 1,
            template: "SKILL.recipe.md".into(),
            inputs: BTreeMap::from([
                (
                    "enabled".into(),
                    Input {
                        kind: InputType::Boolean,
                        prompt: None,
                        required: true,
                        default: None,
                        example: Some(true.into()),
                        choices: Vec::new(),
                        allow_invocation_override: true,
                    },
                ),
                (
                    "style".into(),
                    Input {
                        kind: InputType::Choice,
                        prompt: None,
                        required: false,
                        default: Some("concise".into()),
                        example: None,
                        choices: vec!["concise".into(), "detailed".into()],
                        allow_invocation_override: true,
                    },
                ),
            ]),
            local_inputs: BTreeMap::new(),
        };
        let supplied = BTreeMap::from([
            ("enabled".into(), "yes".into()),
            ("style".into(), "detailed".into()),
        ]);
        let values = resolve_values(&recipe, &supplied, None, true).unwrap();
        assert_eq!(values["enabled"].as_bool(), Some(true));
        assert_eq!(values["style"].as_str(), Some("detailed"));
    }
}
