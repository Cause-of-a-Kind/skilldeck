# Skilldeck

Skilldeck installs agent skills from Git-backed catalogs. Point it at a catalog you trust, list what is available, install one skill or a group, and later update those installed skills from their recorded provenance.

Skilldeck is catalog-agnostic: catalogs can live in public or private Git repositories, and your system `git` handles SSH/HTTPS authentication. Skilldeck does not store credentials or make global config changes except when you explicitly run `skilldeck init`.

## Install

Install the latest published v0.1.2 GitHub Release on Linux/macOS:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/Cause-of-a-Kind/skilldeck/releases/download/v0.1.2/skilldeck-installer.sh | sh
```

On Windows PowerShell:

```powershell
irm https://github.com/Cause-of-a-Kind/skilldeck/releases/download/v0.1.2/skilldeck-installer.ps1 | iex
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
skilldeck init --repository git@github.com:your-org/agent-skills.git --reference main --yes
skilldeck doctor
skilldeck list
skilldeck install-group default ./skills
```

Catalog selection precedence is:

1. CLI options: `--catalog-repository`, `--catalog-ref`
2. Environment: `SKILLDECK_CATALOG_REPOSITORY`, `SKILLDECK_CATALOG_REF`
3. Global per-user config written by `skilldeck init`

If config already exists, `init` asks before replacing it. In scripts, pass `--force` to replace existing config explicitly. Set `SKILLDECK_CONFIG_DIR` to isolate config for tests or automation.

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

`bootstrap` creates a missing destination or uses an existing genuinely empty directory. It refuses symlinks, non-empty destinations, and never overwrites; there is no `--force`.

Typical next steps:

```sh
cd skilldeck-catalog
git init --initial-branch=main
git add .
git commit -m "Start Skilldeck catalog"
git remote add origin <your-catalog-url>
git push -u origin main
skilldeck init --repository <your-catalog-url> --reference main
skilldeck doctor
skilldeck install-group quickstart ../skills
```

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

Groups install or remove a whitespace-separated list of catalog skill names.

## Practical command reference

```sh
skilldeck bootstrap ./catalog --quickstart
skilldeck init --repository <catalog-url-or-path> --reference main
skilldeck list
skilldeck list --json
skilldeck doctor
skilldeck doctor --deep

skilldeck install fin ./skills
skilldeck install --yes fin ./new-skills
skilldeck install --force fin ./skills
skilldeck install-group rails ./skills

skilldeck update fin ./skills
skilldeck update ./skills
skilldeck remove fin ./skills
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
- Installed `.git` directories/files are stripped.
- `remove` refuses to delete directories without `SKILL.md`.
- Bulk update and group removal list updated/removed/skipped entries and summaries.
- Unrelated directories are left untouched.
- Skilldeck never stores Git credentials. Do not put secrets in repository URLs.

Skilldeck writes `<install-root>/.skilldeck/installations.toml` recording managed installs as `catalog` or `direct-git` sources. Bulk update uses this provenance instead of guessing from the current global config.

## Upgrading Skilldeck

`skilldeck upgrade` checks the latest stable GitHub Release for `Cause-of-a-Kind/skilldeck`, ignores drafts/prereleases, chooses the cargo-dist archive for the current supported target, downloads the archive plus checksum, verifies it, extracts the expected `skilldeck`/`skilldeck.exe` binary, and replaces the current executable without running installer scripts.

Supported self-upgrade archives are Linux x86_64/aarch64, macOS x86_64/aarch64, and Windows x86_64 MSVC. If the executable or its directory is not writable, Skilldeck fails with a package-manager caveat instead of escalating privileges.

`skilldeck upgrade --check` reports status and never installs. Passive update notices are shown only on interactive stderr, never for JSON output, scripts/tests, `upgrade`, or when `SKILLDECK_NO_UPDATE_CHECK=1`. Checks are cached with offline-friendly backoff. No telemetry is sent.

## Development and release

Useful local checks:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo llvm-cov --workspace --all-targets --all-features --summary-only --fail-under-lines 85
git diff --check
cargo dist generate
cargo dist plan
```

GitHub Releases are configured with cargo-dist. Pushing a version tag such as `v0.1.2` runs the release workflow, builds archives for supported Linux/macOS/Windows targets, generates shell and PowerShell installers, and uploads checksums. See [`RELEASE.md`](./RELEASE.md) for the exact process.

crates.io publishing remains disabled while the CLI surface stabilizes.
