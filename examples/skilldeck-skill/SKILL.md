---
name: skilldeck
description: Use when the user asks to manage Skilldeck registries or agent skills, inspect or maintain a catalog, install built-in, remote, or local-working-tree skills, update installed skills, run diagnostics, or understand the difference between skilldeck update and skilldeck upgrade.
---

# Skilldeck CLI Skill

Use this skill to help a user operate `skilldeck` safely and predictably. Skilldeck installs agent skills from Git-backed catalogs into a local install directory.

For version-sensitive syntax or behavior, run `skilldeck docs agent` and follow the guidance embedded in the currently installed binary. For recipe authoring, MiniJinja templates, inputs, or local configuration, run `skilldeck docs recipes`. These commands are offline and always match the active Skilldeck version, even when this installed skill file has not yet been refreshed.

## Core commands

- `skilldeck list` — show default-registry skills and groups. Use `--all` for every registry, `--builtins` for bundled skills, and `--json` for machine-readable output.
- `skilldeck doctor` — validate the configured default registry. Use `skilldeck registry doctor --all` for all registries.
- `skilldeck registry list|add|rename|default|update|remove|doctor` — manage named Git-backed registries.
- `skilldeck catalog check <path> --strict` — validate a local catalog and its skill metadata before committing.
- `skilldeck catalog add <name> --source <url> --path <path>` — safely add a validated external package.
- `skilldeck install <skill-or-git-url> [install-directory]` — install one catalog skill or direct Git source. Without a directory, use `<git-root>/.agents/skills`; use `--global` for `~/.agents/skills`. Use `--target pi|codex|claude|gemini|cursor|opencode` for a real, commit-ready harness-native installation. Add `--local [catalog-path]` to test a registry working tree, including uncommitted changes, without changing config. Composable recipes accept repeatable `--set KEY=VALUE` inputs.
- `skilldeck install builtin:skilldeck [install-directory]` — install this bundled skill without a configured registry or network access; use `--global` for the standard global location.
- `skilldeck install-group <group> [install-directory]` — install every skill in a catalog group, with the same project/global defaults.
- `skilldeck installed [install-directory]` — list installed skills using the same project/global `--target` and custom-root resolution. Use `--json` for machine-readable output.
- `skilldeck harness sync|status|remove claude [--global]` — manage opt-in, machine-local Claude Code aliases from canonical `.agents/skills`.
- `skilldeck docs agent|recipes|readme` — print documentation embedded in the active binary without network access.
- `skilldeck update <install-directory>` — bulk update already installed skills under an install root.
- `skilldeck update <skill-or-git-url> <install-directory>` — update one installed skill.
- `skilldeck remove <name> [install-directory]` — remove one installed skill, using the same project/global `--target` defaults as installation. Portable removal also cleans a matching managed Claude alias.
- `skilldeck remove-group <group> [install-directory]` — remove currently installed group members with the same location resolution.
- `skilldeck init --name <registry> --repository <url-or-path> --reference <ref>` — configure the user's initial/default registry.
- `skilldeck bootstrap <path> --quickstart|--empty` — create a new catalog scaffold and, by default, initialize/commit it as a local Git repository on branch `main`.
- `skilldeck bootstrap <path> --quickstart|--empty --no-git` — generate catalog files only.

## Composable recipes

A catalog skill may contain one `recipe.toml` and one or more `*.recipe.md` templates instead of `SKILL.md`. Shared partials live under the catalog-root `partials/` directory. Skilldeck prompts for unresolved install-time inputs, stores resolved plaintext values in `.skilldeck/installations.toml`, renders all recipe files, and installs an ordinary `SKILL.md` package. `--yes` accepts defaults but does not invent required values. Never put secrets in recipe inputs.

Use `[local_inputs.<name>]` for user- or machine-specific defaults that must not change tracked rendered files. Skilldeck writes these values to an ignored `SKILL.local.toml`, generates a committed example and nested `.gitignore`, preserves the local file during updates, and adds invocation instructions to `SKILL.md`. Explicit `key=value` prompt arguments take precedence when the input allows invocation overrides. Local values are plaintext and are not suitable for secrets.

Runtime arguments such as `ticket=12344` are user prompt content interpreted by the installed skill; Skilldeck is not involved during invocation.

## Update vs upgrade

- `skilldeck update` changes installed skills.
- `skilldeck upgrade` changes the Skilldeck binary itself. Do not run it when the user only asked to refresh skills.

## Safety rules

1. Prefer `skilldeck list` and `skilldeck doctor` before installation when the catalog state is unclear.
2. Respect prompts and confirmation safeguards. Do not add `--force` or `--yes` unless the user explicitly asked for non-interactive behavior and the destination is understood.
3. Treat `install --force`, `install-group --force`, `remove`, `remove-group`, and `upgrade --yes` as destructive or high-impact. State what will change before suggesting them.
4. Do not overwrite a user's catalog, global config, or installed skills unless they explicitly request it.
5. Do not commit, push, tag, release, or change private repositories unless the user explicitly asks and grants access.
6. Pin external catalog refs to branches or tags the user trusts; prefer immutable tags for shared catalogs.
7. Claude aliases are local symlinks excluded through `.git/info/exclude`. Never replace an existing `.claude/skills` entry that is not the matching managed alias.
8. Treat `--target claude` as a real native installation, distinct from the portable-install-plus-alias behavior of `--claude`. Warn about duplicate skill names across known target directories.

## Helpful workflow

When a user asks to get started:

```sh
skilldeck bootstrap ./skilldeck-catalog --quickstart
skilldeck init --name default --repository ./skilldeck-catalog --reference main
skilldeck doctor
skilldeck install-group quickstart
```

If the user chooses `--no-git`, tell them to run `git init --initial-branch=main`, `git add .`, and `git commit -m "Start Skilldeck catalog"` from inside the generated catalog before configuring Skilldeck. If automatic commit fails because Git identity is missing, explain that files exist and are staged; configure `user.name` and `user.email`, then run that commit command.

Registry-qualified selectors use `<registry>:<skill>`; unqualified names use only the default registry. The reserved `builtin` registry contains skills embedded in the current Skilldeck binary. A one-off `--local [PATH]` acts like a path dependency and records local-working-tree provenance; omit it on a later qualified update to switch back to the configured remote. Legacy single-catalog config migrates when a second registry is added, with the original kept as default.

The global config can be symlinked from a dotfiles repository; use `skilldeck config path` to locate it. Repository authentication remains machine-local.

Adjust paths for the user's shell and operating system. Keep output portable; avoid shell-specific syntax unless the user is already using that shell.
