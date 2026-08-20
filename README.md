# Skilldeck

Skilldeck is a public, open-source CLI for installing agent skills from Git-backed catalogs. It is catalog-agnostic: configure any public or private Git repository that follows the supported format, and let your system `git` handle SSH/HTTPS authentication. Skilldeck never stores credentials.

## Install

Install the latest v0.1.2 GitHub Release on Linux/macOS with the generated shell installer:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/Cause-of-a-Kind/skilldeck/releases/download/v0.1.2/skilldeck-installer.sh | sh
```

On Windows PowerShell:

```powershell
irm https://github.com/Cause-of-a-Kind/skilldeck/releases/download/v0.1.2/skilldeck-installer.ps1 | iex
```

Security note: piping installers directly to a shell is convenient but optional. If you prefer, download the installer or release archive first, inspect it, verify the `.sha256` file or `sha256.sum`, and then run it locally.

Source fallback:

```bash
git clone https://github.com/Cause-of-a-Kind/skilldeck.git
cd skilldeck
cargo install --path .
```

Future npm distribution may wrap official binaries, but will not use unsafe postinstall scripts.

## Configure a catalog

```bash
skilldeck init --repository git@github.com:your-org/agent-skills.git --reference main --yes
```

If config already exists, `init` asks before replacing it. In non-interactive use, pass `--force` to replace existing config explicitly.

Config is global per user in the platform config directory (`directories` crate). Override location for tests/automation with `SKILLDECK_CONFIG_DIR`.

Precedence for catalog selection:

1. CLI options on catalog commands: `--catalog-repository`, `--catalog-ref`
2. Environment: `SKILLDECK_CATALOG_REPOSITORY`, `SKILLDECK_CATALOG_REF`
3. Global config from `skilldeck init`

## Catalog format

Skilldeck supports the existing registry layout exactly:

- first-party skill directories as immediate children of `skills/<name>/`, each containing `SKILL.md`
- `external-skills.toml`
- `skill-groups.toml`

External skill example:

```toml
[skills."layered-rails"]
source = "https://github.com/palkan/skills.git"
subdirectory = "layered-rails/skills/layered-rails"
ref = "f4e8cd90ae388339d53bc05a3826034d0df56255"
```

`source` may be a Git URL or a direct Markdown URL. `subdirectory` and `ref` are optional; use `-` for repository root/default ref.

Group example:

```toml
[groups."rails"]
skills = "layered-rails object-oriented-design rails-feature-review rails-project-review"
```

Names must contain only letters, numbers, dots, underscores, and hyphens. Paths are validated against traversal.

## Usage

```bash
skilldeck install fin ./skills
skilldeck install --force fin ./skills
skilldeck install --yes fin ./new-skills
skilldeck install-group rails ./skills
skilldeck update fin ./skills
skilldeck update ./skills
skilldeck upgrade --check
skilldeck upgrade
skilldeck upgrade --yes
skilldeck remove fin ./skills
skilldeck remove-group rails ./skills
skilldeck list
skilldeck list --json
skilldeck doctor
skilldeck doctor --deep
skilldeck help
skilldeck version
```

Direct Git installs are supported:

```bash
skilldeck install https://github.com/example/my-skill.git ./skills
```

The repository basename becomes the skill directory name. Direct Markdown URLs are supported only through catalog entries, where the catalog name supplies the destination directory.

## Safeguards

- Existing destinations are not overwritten unless `--force` is passed.
- Missing install roots prompt `Create it? [y/N]`; default is No. Use `--yes` in scripts/CI.
- Installed `.git` directories/files are stripped.
- `remove` refuses to delete directories without `SKILL.md`.
- Bulk update and group removal list updated/removed/skipped entries and summaries.
- Unrelated directories are left untouched.

## Upgrading Skilldeck

`skilldeck upgrade` updates the Skilldeck binary itself; `skilldeck update` remains for installed skills. It checks the latest stable GitHub Release for `Cause-of-a-Kind/skilldeck`, ignores drafts/prereleases, chooses the cargo-dist archive for the current supported target, downloads the archive plus SHA-256 sidecar or `sha256.sum`, verifies the checksum, extracts the expected `skilldeck`/`skilldeck.exe` binary, and replaces the current executable without running installer scripts.

Supported self-upgrade archives are Linux x86_64/aarch64, macOS x86_64/aarch64, and Windows x86_64 MSVC. If the executable or its directory is not writable, Skilldeck fails with a package-manager caveat instead of escalating privileges.

`skilldeck upgrade --check` only reports status, never prompts or installs, and exits 0 when the check succeeds whether or not an update is available. `--yes` installs without prompting.

For normal interactive commands, Skilldeck may print a passive stderr notice when a newer stable release is known. Notices are shown only when stderr is a terminal, never for JSON output, scripts/tests, `upgrade`, or when `SKILLDECK_NO_UPDATE_CHECK=1`. Successful checks are cached for 24 hours in the platform cache directory; failed attempts are cached for a one-hour backoff so offline use does not repeatedly wait on timeouts. Stale checks use a short timeout and failures are silent. No telemetry is sent.

## Provenance manifest

Skilldeck writes an unobtrusive manifest at `<install-root>/.skilldeck/installations.toml`. It records each Skilldeck-managed directory as either:

- `catalog`: original catalog name, catalog repository, and catalog ref
- `direct-git`: original repository and optional ref

This lets bulk update preserve the original source/catalog instead of guessing from the current global config. The manifest contains no credentials beyond the repository strings you supplied. Do not put secrets in repository URLs.

## Test coverage

Behavioral coverage is enforced with unit/integration tests and measured with [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov):

```bash
cargo llvm-cov --workspace --all-targets --all-features --summary-only --fail-under-lines 85
```

The current measured coverage on Linux is 86.23% line coverage, 80.00% function coverage, and 83.21% region coverage. Branch coverage is not reported by the current Rust/LLVM setup for this crate. CI enforces the line threshold while prioritizing meaningful release-risk behavior over artificial assertions.

## Release approach

GitHub Releases are configured with cargo-dist 0.32.0. Pushing a version tag such as `v0.1.2` runs the release workflow, builds archives for the supported Linux/macOS/Windows targets, generates shell and PowerShell installers, and uploads checksums. See [`RELEASE.md`](./RELEASE.md) for the exact process.

crates.io publishing remains disabled for the initial release while the CLI surface stabilizes.
