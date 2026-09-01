# Changelog

All notable changes to Skilldeck are documented here.

## [Unreleased]

## [0.4.0] - 2026-09-01

### Added

- `skilldeck installed [install-directory]` lists installed skills with project, `--global`, native `--target`, custom-root, and `--json` support.

## [0.3.1] - 2026-08-31

### Fixed

- `remove` and `remove-group` now use the same default project, `--global`, native `--target`, and optional custom-root resolution as installation. Portable removal also cleans the matching managed Claude compatibility alias.

## [0.3.0] - 2026-08-31

### Added

- Composable skill recipes using `recipe.toml`, MiniJinja `*.recipe.md` templates, nested rendered files, and catalog-level shared partials.
- Typed `string`, `boolean`, `integer`, and `choice` install inputs with interactive prompts, repeatable `--set KEY=VALUE`, group-qualified values, validation examples, and update-stable locked values.
- Machine-local recipe inputs backed by ignored `SKILL.local.toml` files, generated examples and invocation guidance, and preservation across updates.
- External skill recipe wrappers with `upstream.frontmatter` and `upstream.body` while retaining upstream assets.
- Canonical project installs at `<git-root>/.agents/skills` and global installs at `~/.agents/skills` when no custom path is supplied.
- Harness-native `--target agents|pi|codex|claude|gemini|cursor|opencode` installations, including `--global` support and cross-target skill-name collision warnings.
- Opt-in Claude Code compatibility aliases with `--claude` and `skilldeck harness sync|status|remove claude`, using exact repository-local Git exclusions and conflict-safe symlink management.
- Offline, version-matched `skilldeck docs agent|recipes|readme` output embedded in the binary.

### Changed

- Install and group-install destination arguments are optional while explicit custom paths remain supported.
- The bundled `builtin:skilldeck` skill now documents recipes, native targets, Claude compatibility, and the embedded documentation commands.

[Unreleased]: https://github.com/Cause-of-a-Kind/skilldeck/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/Cause-of-a-Kind/skilldeck/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/Cause-of-a-Kind/skilldeck/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/Cause-of-a-Kind/skilldeck/compare/v0.2.0...v0.3.0
