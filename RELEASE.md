# Releasing Skilldeck v0.2.0

Do not run these steps until the repository exists at `Cause-of-a-Kind/skilldeck` and CI is green.

## Prerequisites

- Rust stable and `rustup` installed.
- `dist` from cargo-dist 0.32.0 available locally:
  ```bash
  cargo install cargo-dist --locked
  ```
  The executable is `dist`.
- `cargo-llvm-cov` available for coverage checks:
  ```bash
  cargo install cargo-llvm-cov --locked
  rustup component add llvm-tools-preview
  ```
- GitHub repository `Cause-of-a-Kind/skilldeck` created with Actions enabled.

## Clean pre-release checks

Run from the repository root:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo llvm-cov --workspace --all-targets --all-features --summary-only --fail-under-lines 85
dist generate --mode ci --check --allow-dirty
dist plan --tag v0.2.0 --allow-dirty
```

`--allow-dirty` is only for local planning before the release commit exists. The release tag should be created from a clean, reviewed commit.

## Tag format

Use a semver version tag:

```bash
git tag v0.2.0
git push origin v0.2.0
```

The generated release workflow runs only for version tags matching cargo-dist's semver tag pattern.

## What cargo-dist creates

For v0.2.0, cargo-dist is configured to create GitHub Release assets for:

- `skilldeck-installer.sh`
- `skilldeck-installer.ps1`
- `sha256.sum`
- `source.tar.gz`
- `source.tar.gz.sha256`
- `skilldeck-x86_64-unknown-linux-gnu.tar.xz`
- `skilldeck-x86_64-unknown-linux-gnu.tar.xz.sha256`
- `skilldeck-aarch64-unknown-linux-gnu.tar.xz`
- `skilldeck-aarch64-unknown-linux-gnu.tar.xz.sha256`
- `skilldeck-x86_64-apple-darwin.tar.xz`
- `skilldeck-x86_64-apple-darwin.tar.xz.sha256`
- `skilldeck-aarch64-apple-darwin.tar.xz`
- `skilldeck-aarch64-apple-darwin.tar.xz.sha256`
- `skilldeck-x86_64-pc-windows-msvc.zip`
- `skilldeck-x86_64-pc-windows-msvc.zip.sha256`

The workflow creates/uploads release assets using the repository `GITHUB_TOKEN` with `contents: write` permission.

`skilldeck upgrade` depends on these archive and checksum asset names. After the release is public, smoke-test `skilldeck upgrade --check` and, from an isolated copied binary, `skilldeck upgrade --yes`. The command never runs generated installer scripts; it downloads/verifies/extracts archives directly.

## Post-release smoke tests

Use an isolated `CARGO_HOME` so the cargo-dist installer writes to a temporary Cargo-style home (`$CARGO_HOME/bin`) instead of replacing any existing binary.

Linux/macOS shell installer:

```bash
tmp="$(mktemp -d)"
export HOME="$tmp/home"
export CARGO_HOME="$tmp/cargo-home"
mkdir -p "$HOME" "$CARGO_HOME"
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/Cause-of-a-Kind/skilldeck/releases/download/v0.2.0/skilldeck-installer.sh \
  -o "$tmp/skilldeck-installer.sh"
sh "$tmp/skilldeck-installer.sh"
PATH="$CARGO_HOME/bin:$PATH" skilldeck version
PATH="$CARGO_HOME/bin:$PATH" skilldeck help
```

Windows PowerShell installer:

```powershell
$tmp = New-Item -ItemType Directory -Force ([System.IO.Path]::Combine([System.IO.Path]::GetTempPath(), [System.Guid]::NewGuid()))
$env:CARGO_HOME = Join-Path $tmp.FullName "cargo-home"
New-Item -ItemType Directory -Force $env:CARGO_HOME | Out-Null
$installer = Join-Path $tmp.FullName "skilldeck-installer.ps1"
Invoke-RestMethod https://github.com/Cause-of-a-Kind/skilldeck/releases/download/v0.2.0/skilldeck-installer.ps1 -OutFile $installer
& $installer
$env:PATH = "$(Join-Path $env:CARGO_HOME 'bin');$env:PATH"
skilldeck version
skilldeck help
```

Also verify checksums by downloading `sha256.sum` and the archive for the current platform, then running `sha256sum -c` or platform equivalent.
