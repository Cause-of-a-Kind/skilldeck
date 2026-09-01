# Skilldeck

Skilldeck installs agent skills from Git-backed catalogs. Point it at a catalog you trust, list what is available, install one skill or a group, and later update those installed skills from their recorded provenance.

Skilldeck is catalog-agnostic: catalogs can live in public or private Git repositories, and your system `git` handles SSH/HTTPS authentication. Multiple catalogs can be configured as named registries. Skilldeck does not store credentials; global config changes happen only through `init` and `registry` commands.

## Install

Install the latest published v0.4.0 GitHub Release on Linux/macOS:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/Cause-of-a-Kind/skilldeck/releases/download/v0.4.0/skilldeck-installer.sh | sh
```

On Windows PowerShell:

```powershell
irm https://github.com/Cause-of-a-Kind/skilldeck/releases/download/v0.4.0/skilldeck-installer.ps1 | iex
```

Security note: piping installers directly to a shell is convenient but optional. You can download the installer or archive first, inspect it, verify the `.sha256` file or `sha256.sum`, and run it locally.

Source fallback:

```sh
git clone https://github.com/Cause-of-a-Kind/skilldeck.git
cd skilldeck
cargo install --path .
```

Skilldeck is not published to npm or crates.io yet.

## Quickstart with an existing catalog

```sh
skilldeck init --name company --repository git@github.com:your-org/agent-skills.git --reference main --yes
skilldeck registry add personal git@github.com:you/personal-skills.git --reference main --yes
skilldeck registry list
skilldeck doctor
skilldeck list --all
skilldeck install company:fin
```

With no install directory, Skilldeck finds the current Git repository root and uses `.agents/skills`. Pass `--global` to use `~/.agents/skills`, or keep passing an explicit directory for a custom layout.

Registry-qualified selectors use `<registry>:<skill>` and `<registry>:<group>`. Unqualified names resolve only in the default registry, so adding another registry never changes an existing command through an ambiguous search. Use `skilldeck registry default <name>` to change that default.

For one-off testing, `--local [PATH]` replaces the selected registry source with a local working tree without changing config. It reads uncommitted catalog and first-party skill changes, similar to a path dependency:

```sh
skilldeck install coak:my-skill ./skills --local ../agent-skills
skilldeck install-group coak:rails ./skills --local ../agent-skills

cd ../agent-skills
skilldeck list coak --local
skilldeck doctor --registry coak --local --strict
```

A bare `--local` uses the current directory. Local installs record working-tree provenance, so bulk update keeps reading that path. A later normal qualified update, without `--local`, switches the installation back to the configured remote source and replaces its provenance.

## Portable, native, and Claude-compatible installs

Skilldeck uses the interoperable Agent Skills locations by default:

```sh
skilldeck install fin                 # <git-root>/.agents/skills
skilldeck install fin --global        # ~/.agents/skills
skilldeck install fin ./custom/skills # explicit custom root
```

The default project directory is found with Git, even when the command runs in a nested subdirectory. If `.agents/skills` does not exist, Skilldeck uses the same `Create it? [y/N]` safeguard as an explicit missing path. Outside a Git repository, pass `--global` or an explicit directory. `--global` conflicts with a custom directory.

For a skill intentionally adapted to one agent's workflow or capabilities, `--target` installs a real rendered skill in that harness's native directory:

| Target | Project | Global |
| --- | --- | --- |
| `agents` (default) | `.agents/skills` | `~/.agents/skills` |
| `pi` | `.pi/skills` | `~/.pi/agent/skills` |
| `codex` | `.codex/skills` | `~/.codex/skills` |
| `claude` | `.claude/skills` | `~/.claude/skills` |
| `gemini` | `.gemini/skills` | `~/.gemini/skills` |
| `cursor` | `.cursor/skills` | `~/.cursor/skills` |
| `opencode` | `.opencode/skills` | `~/.config/opencode/skills` |

```sh
skilldeck install herdr-review --target pi
skilldeck install subagent-review --target claude
skilldeck install company-codex --target codex --global
skilldeck install-group pi-workflows --target pi
skilldeck remove pi-workflow --target pi
```

Install, group-install, remove, and group-remove use the same project/global target resolution. Their install-root positional argument remains available only for custom layouts.

Native target installations are ordinary directories, are not automatically ignored, and can be committed to the repository. If the same skill name already exists in another known target, Skilldeck warns that harnesses discovering both locations may report a duplicate or choose one by precedence. Give adaptations distinct names when possible. `--target` cannot be combined with a custom positional install directory.

Pi, Codex, Gemini CLI, Cursor, and OpenCode can discover `.agents/skills`. Claude Code requires `.claude/skills`, so Claude users can instead opt into a local compatibility alias for a portable installation:

```sh
skilldeck install fin --claude
skilldeck install fin --global --claude
skilldeck install-group quickstart --claude
```

`--claude` means “install to the portable `agents` target and also create a compatibility alias”; it cannot be combined with another `--target`. Project aliases are relative symlinks such as `.claude/skills/fin -> ../../.agents/skills/fin`. Skilldeck adds only the exact generated alias paths to the repository-local Git exclude file (`git rev-parse --git-path info/exclude`); it does not modify the committed `.gitignore` or ignore the rest of `.claude`. Global aliases need no Git exclusion. Custom install directories cannot be combined with `--claude` because they do not define a standard project relationship.

After cloning a project whose canonical skills are already committed, synchronize or inspect Claude aliases with:

```sh
skilldeck harness sync claude
skilldeck harness status claude
skilldeck harness remove claude

skilldeck harness sync claude --global
skilldeck harness status claude --global
skilldeck harness remove claude --global
```

Synchronization creates missing aliases but refuses to overwrite native Claude skills, files, directories, or unrelated symlinks. Status exits unsuccessfully when links are missing, conflicting, or stale, making it useful as a local setup check. Remove deletes only aliases that target the canonical `.agents/skills` tree and removes Skilldeck's local Git-exclude block. These ignored project aliases are intentionally machine-local and are not available in fresh Claude cloud environments.

Catalog selection precedence is:

1. A one-off local working tree selected by `--local [PATH]`
2. Ad-hoc CLI options: `--catalog-repository`, `--catalog-ref`
3. Environment: `SKILLDECK_CATALOG_REPOSITORY`, `SKILLDECK_CATALOG_REF`
4. A registry selected by `--registry`/`SKILLDECK_REGISTRY` or a qualified name
5. The default registry in global config

Legacy single-catalog config is read without modification. The first `registry add` migrates it to the multi-registry format, keeps it as the default, and interactively asks for its alias. In scripts, use `--existing-as <name> --yes`. If no alias is supplied non-interactively, the existing registry is named `default`.

If config already exists, `init` asks before replacing it. In scripts, pass `--force` explicitly. Set `SKILLDECK_CONFIG_DIR` to isolate config for tests or automation.

Useful registry operations:

```sh
skilldeck registry rename default company
skilldeck registry default company
skilldeck registry update company --reference stable
skilldeck registry doctor --all
skilldeck registry remove personal
skilldeck config path
```

Skilldeck also exposes a reserved virtual registry for skills bundled with the binary. It needs no config or network access:

```sh
skilldeck list --builtins
skilldeck install builtin:skilldeck ~/.agents/skills
skilldeck update builtin:skilldeck ~/.agents/skills
```

Bulk update also refreshes built-in skills from the currently installed Skilldeck binary. The alias `builtin` cannot be used for a Git-backed registry.

### Version-matched offline documentation

The binary embeds its agent guide, recipe reference, and complete README. Agents and scripts can read documentation matching the executable actually on the machine without locating package source files or fetching the website:

```sh
skilldeck docs              # list topics
skilldeck docs agent        # current agent operating guide
skilldeck docs recipes      # recipe.toml, MiniJinja, inputs, and local config
skilldeck docs readme       # complete README for this build
```

The bundled `builtin:skilldeck` skill contains a stable pointer telling agents to consult these commands for version-sensitive behavior. This avoids silently rewriting project or global skill files during a binary upgrade: an older installed copy of the skill can still retrieve current guidance from the new executable. Refresh the physical skill when desired with `skilldeck update builtin:skilldeck <install-root>` or a normal bulk update.

The multi-registry config format is:

```toml
default_registry = "company"

[registries.company]
repository = "git@github.com:your-org/agent-skills.git"
ref = "main"

[registries.personal]
repository = "git@github.com:you/personal-skills.git"
ref = "main"
```

## Create your own catalog

Generate a starter catalog:

```sh
skilldeck bootstrap ./skilldeck-catalog --quickstart
```

Or create only the structure:

```sh
skilldeck bootstrap ./skilldeck-catalog --empty
```

Without arguments in an interactive terminal, `bootstrap` asks where to create the catalog and whether to use the quickstart or empty template. In non-interactive use, provide both a path and exactly one template flag.

By default, `bootstrap` uses system Git to initialize the generated catalog as an immediately clonable local repository on branch `main`, stages the generated files, and creates the initial commit `Start Skilldeck catalog`. It does not create remotes, push, or change global Skilldeck config. Use `--no-git` to generate files only.

`bootstrap` creates a missing destination or uses an existing genuinely empty directory. It refuses symlinks, non-empty destinations, and never overwrites; there is no `--force`.

Typical default flow:

```sh
skilldeck bootstrap ./skilldeck-catalog --quickstart
skilldeck init --repository ./skilldeck-catalog --reference main
skilldeck doctor
skilldeck install-group quickstart ./skills
```

Optionally publish the catalog after bootstrap:

```sh
cd skilldeck-catalog
git remote add origin <your-catalog-url>
git push -u origin main
skilldeck init --repository <your-catalog-url> --reference main
```

If you pass `--no-git`, initialize manually from inside the generated directory:

```sh
git init --initial-branch=main
git add .
git commit -m "Start Skilldeck catalog"
```

If the automatic commit fails because Git identity is missing, the catalog files remain in place and are already initialized/staged. Configure `user.name` and `user.email`, then run `git commit -m "Start Skilldeck catalog"` from inside the catalog directory.

## Catalog format

A catalog repository may contain:

- first-party skill directories at `skills/<name>/SKILL.md`
- `external-skills.toml`
- `skill-groups.toml`

First-party skill names are directory names. Names must contain only ASCII letters, numbers, dots, underscores, and hyphens.

External skill example:

```toml
[skills."layered-rails"]
source = "https://github.com/palkan/skills.git"
subdirectory = "layered-rails/skills/layered-rails"
ref = "f4e8cd90ae388339d53bc05a3826034d0df56255"
```

`source` may be a Git URL or a direct Markdown URL. `subdirectory` and `ref` are optional. Subdirectories are validated as safe relative paths.

Group example:

```toml
[groups."rails"]
skills = "layered-rails object-oriented-design rails-feature-review rails-project-review"
```

Groups install or remove a whitespace-separated list of catalog-local skill names. Qualify the group itself when selecting another registry, for example `company:rails`.

### Composable skill recipes

A first-party skill may use a Skilldeck recipe instead of a static `SKILL.md`. Skilldeck renders the recipe during installation and installs an ordinary, portable Agent Skill; using the installed skill does not require Skilldeck or a template engine.

```text
partials/
  reviews/output.recipe.md
skills/
  code-review/
    recipe.toml
    SKILL.recipe.md
    references/checklist.recipe.md
```

`skills/code-review/recipe.toml` declares the entry template and install-time inputs:

```toml
version = 1
template = "SKILL.recipe.md"

[inputs.review_style]
type = "choice"
prompt = "Preferred review style"
choices = ["concise", "detailed", "teaching"]
default = "concise"

[inputs.ticket_system]
type = "string"
prompt = "Ticket system"
required = true
example = "Linear"
```

Templates use MiniJinja syntax:

```jinja2
---
name: code-review
description: Review changes using project-specific conventions.
---

Use a {{ review_style }} review style.

{% include "partials/reviews/output.recipe.md" %}

When the user supplies `ticket=<id>`, interpret it as a {{ ticket_system }} ticket.
```

Every `*.recipe.md` beneath the skill recipe directory is rendered. Skilldeck removes `.recipe` from its installed name, so `SKILL.recipe.md` becomes `SKILL.md` and `references/checklist.recipe.md` becomes `references/checklist.md`. Ordinary supporting files are copied unchanged. `recipe.toml`, source recipes, and catalog-level partials are not installed. A static output file and recipe output may not claim the same path.

Catalog-root `partials/` are shared by every recipe and may be nested for namespacing. Partials use inputs declared by the consuming skill's single `recipe.toml`; they do not have separate input manifests. Prefer MiniJinja macros with explicit arguments when a shared partial has a reusable parameter contract.

Supported input types are `string`, `boolean`, `integer`, and `choice`. Required inputs without defaults must declare an `example` so catalog validation can render the skill. Input names must be ASCII identifiers and cannot use the reserved names `skill`, `values`, or `upstream`.

Install interactively or provide repeatable values for automation:

```sh
skilldeck install code-review ./skills
skilldeck install code-review ./skills --yes \
  --set review_style=detailed \
  --set ticket_system=Linear
```

`--yes` accepts declared defaults but never invents required values. Resolved values are stored with provenance in `.skilldeck/installations.toml` and reused by single and bulk updates. A single update can change a locked value with `--set`. Group values must be qualified as `SKILL.KEY=VALUE`.

#### User- and machine-local inputs

Values that should vary between collaborators must not be rendered into tracked files. Declare them under `local_inputs` instead:

```toml
[local_inputs.model]
type = "choice"
prompt = "Default delegated-agent model"
choices = ["gpt", "claude-opus", "claude-sonnet"]
default = "claude-sonnet"
allow_invocation_override = true
```

Skilldeck prompts for local inputs during installation and writes them next to the installed skill:

```text
code-review/
  SKILL.md
  SKILL.local.example.toml
  SKILL.local.toml
  .gitignore
```

`SKILL.local.toml` is automatically ignored by the installed skill's nested `.gitignore`; Skilldeck does not write its values to `installations.toml`. The example, ignore rule, and generated instructions are safe to commit. Existing local values are validated and preserved across updates.

The generated `SKILL.md` includes a **Before starting: local overrides** section telling the invoking agent to resolve values in this order:

1. Explicit invocation `key=value`, when `allow_invocation_override` permits it
2. `SKILL.local.toml`
3. The default declared in `recipe.toml`

For example, `skilldeck install code-review ./skills --set model=gpt` writes `model` only to the ignored local file. A later invocation such as `<code-review> ticket=123 effort=high` can override an allowed local default for that invocation without involving Skilldeck. Local values are also plaintext and must not contain secrets.

External catalog skills may be wrapped by a recipe:

```toml
[skills."company-code-review"]
source = "https://github.com/example/review-skills.git"
subdirectory = "skills/code-review"
ref = "v2.1.0"
recipe = "recipes/company-code-review/recipe.toml"
```

External wrapper templates receive `upstream.frontmatter` and `upstream.body`, preserve other upstream package files, and may use the catalog's shared partials. `catalog check --deep` resolves and renders these wrappers.

Recipes have strict undefined-value handling and cannot execute commands, read arbitrary host files, access environment variables, or fetch network resources. Includes resolve only from the recipe directory and catalog `partials/`. Render inputs and outputs are plaintext: never use recipe values for secrets or credentials.

Invocation-time text such as `ticket=12344` is interpreted by the installed skill and agent. Skilldeck documents allowed local-input overrides in the generated skill but is not involved at invocation time.

### Maintain a local catalog

Validate a working tree without committing or cloning it:

```sh
skilldeck catalog check .
skilldeck catalog check . --strict
skilldeck catalog check . --deep
```

`--strict` fails for missing, malformed, or mismatched `SKILL.md` YAML frontmatter. Without it, metadata findings are warnings so existing catalogs can migrate gradually. `--deep` resolves external Git and Markdown sources. The same metadata policy is available on `doctor` and `registry doctor`.

Add an external package with duplicate checks, remote resolution, metadata validation, and rollback if the resulting catalog is invalid:

```sh
skilldeck catalog add layered-rails \
  --source https://github.com/palkan/skills.git \
  --subdirectory layered-rails/skills/layered-rails \
  --reference f4e8cd90ae388339d53bc05a3826034d0df56255 \
  --path .
```

Use `--no-check` only when intentionally adding a source that is currently unavailable. Exact and case-insensitive collisions within a catalog are rejected; the same package name in different registries is allowed.

## Practical command reference

```sh
skilldeck bootstrap ./catalog --quickstart
skilldeck init --name company --repository <catalog-url-or-path> --reference main
skilldeck registry add personal <catalog-url-or-path> --reference main
skilldeck registry list
skilldeck list
skilldeck list company
skilldeck list --all
skilldeck list --all --json
skilldeck doctor
skilldeck doctor --strict
skilldeck registry doctor --all --deep --strict
skilldeck catalog check . --strict

skilldeck install fin
skilldeck install fin --global
skilldeck install fin --claude
skilldeck install fin --global --claude
skilldeck install pi-herdr-flow --target pi
skilldeck install claude-subagents --target claude
skilldeck install company:fin ./skills
skilldeck install company:fin ./skills --local ../company-skills
skilldeck install code-review --set review_style=detailed
skilldeck install builtin:skilldeck --global
skilldeck install --yes fin ./new-skills
skilldeck harness sync claude
skilldeck harness status claude
skilldeck docs agent
skilldeck docs recipes
skilldeck install --force fin ./skills
skilldeck install-group rails ./skills

skilldeck installed
skilldeck installed --global
skilldeck installed --target pi
skilldeck installed ./skills
skilldeck installed --json

skilldeck update fin ./skills
skilldeck update ./skills
skilldeck remove fin
skilldeck remove fin --global
skilldeck remove pi-herdr-flow --target pi
skilldeck remove fin ./skills
skilldeck remove-group rails
skilldeck remove-group rails ./skills

skilldeck upgrade --check
skilldeck upgrade
skilldeck upgrade --yes
skilldeck version
skilldeck help
```

Direct Git installs are supported:

```sh
skilldeck install https://github.com/example/my-skill.git ./skills
```

The repository basename becomes the skill directory name. Direct Markdown URLs are supported only through catalog entries, where the catalog name supplies the destination directory.

Remember: `skilldeck update` refreshes installed skills; `skilldeck upgrade` updates the Skilldeck binary.

## Safeguards, security, and provenance

- Existing skill destinations are not overwritten unless `--force` is passed.
- Missing install roots prompt `Create it? [y/N]`; default is No. Use `--yes` only in scripts/CI where the path is intentional.
- Project installs default to `<git-root>/.agents/skills`; global installs selected with `--global` use `~/.agents/skills`.
- `installed` lists skill directories containing `SKILL.md` with the same project, global, native-target, and custom-root resolution; `--json` provides stable machine-readable output.
- Claude compatibility aliases are opt-in, locally Git-excluded, and never overwrite existing Claude skill entries.
- Native `--target` installs are real, commit-ready skill directories; Skilldeck warns when the same name exists in another known harness location.
- Installed `.git` directories/files are stripped.
- `remove` and `remove-group` use the same default, global, native-target, and custom-root resolution as installation.
- Portable default/global removal also cleans only the matching managed Claude alias; native Claude skills and unrelated paths are untouched.
- `remove` refuses to delete directories without `SKILL.md`.
- Bulk update and group removal list updated/removed/skipped entries and summaries.
- Unrelated directories are left untouched.
- Skilldeck never stores Git credentials. Do not put secrets in repository URLs.
- Recipe input values and rendered files are plaintext. Never use recipe inputs for secrets.

Skilldeck writes `<install-root>/.skilldeck/installations.toml` recording managed installs as `built-in`, `catalog`, `local-catalog`, or `direct-git` sources. Bulk update uses the recorded source. Remote catalog installs retain repository and ref rather than a mutable registry alias, so renaming or removing an alias does not break updates. Local-catalog installs retain the absolute working-tree path until explicitly updated from a configured registry.

The global registry config is normally `~/.config/skilldeck/config.toml` on Linux; use `skilldeck config path` to print the active platform path. The file is suitable for a dotfiles repository, and registry mutations preserve Stow-style file symlinks. Local repository paths supplied to `init`, `registry add`, or `registry update` are stored as absolute paths so behavior does not depend on the current directory; use remote Git URLs for cross-machine config. Repository credentials are deliberately not included, so each machine still needs its own Git/SSH authentication.

Commit `.skilldeck/installations.toml` alongside a project’s installed skills. It acts as the project’s Skilldeck provenance manifest, allowing other contributors and CI to update from the same recorded sources without relying on their global catalog configuration. The manifest contains repository strings and refs but no stored credentials; never embed credentials in repository URLs. For stronger reproducibility, prefer immutable commit refs or release tags over moving branches such as `main`.

## Upgrading Skilldeck

`skilldeck upgrade` checks the latest stable GitHub Release for `Cause-of-a-Kind/skilldeck`, ignores drafts/prereleases, chooses the cargo-dist archive for the current supported target, downloads the archive plus checksum, verifies it, extracts the expected `skilldeck`/`skilldeck.exe` binary, and replaces the current executable without running installer scripts.

Supported self-upgrade archives are Linux x86_64/aarch64, macOS x86_64/aarch64, and Windows x86_64 MSVC. If the executable or its directory is not writable, Skilldeck fails with a package-manager caveat instead of escalating privileges.

`skilldeck upgrade --check` reports status and never installs. Passive update notices are shown only on interactive stderr, never for JSON output, documentation output, scripts/tests, `upgrade`, or when `SKILLDECK_NO_UPDATE_CHECK=1`. Checks are cached with offline-friendly backoff. No telemetry is sent.

Upgrading the executable immediately updates `skilldeck docs` because those documents are embedded at build time. Skilldeck deliberately does not rewrite installed or committed skill directories during a binary upgrade.

## Development and release

Useful local checks:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo llvm-cov --workspace --all-targets --all-features --summary-only --fail-under-lines 85
git diff --check
dist generate --mode ci --check --allow-dirty
dist plan --tag v0.4.0 --allow-dirty
```

GitHub Releases are configured with cargo-dist. Pushing a version tag such as `v0.4.0` runs the release workflow, builds archives for supported Linux/macOS/Windows targets, generates shell and PowerShell installers, and uploads checksums. See [`RELEASE.md`](./RELEASE.md) for the exact process.

crates.io publishing remains disabled while the CLI surface stabilizes.
