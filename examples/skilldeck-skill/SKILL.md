---
name: skilldeck
description: Use when the user asks to manage agent skills with Skilldeck, inspect or install a Skilldeck catalog, install or remove skills or skill groups, update installed skills, run Skilldeck diagnostics, or understand the difference between skilldeck update and skilldeck upgrade.
---

# Skilldeck CLI Skill

Use this skill to help a user operate `skilldeck` safely and predictably. Skilldeck installs agent skills from Git-backed catalogs into a local install directory.

## Core commands

- `skilldeck list` — show catalog skills and groups. Use `--json` for machine-readable output.
- `skilldeck doctor` — validate the configured catalog. Add `--deep` only when the user wants external Git/Markdown sources resolved too.
- `skilldeck install <skill-or-git-url> <install-directory>` — install one catalog skill or direct Git source.
- `skilldeck install-group <group> <install-directory>` — install every skill in a catalog group.
- `skilldeck update <install-directory>` — bulk update already installed skills under an install root.
- `skilldeck update <skill-or-git-url> <install-directory>` — update one installed skill.
- `skilldeck remove <name> <install-directory>` — remove one installed skill by directory name.
- `skilldeck remove-group <group> <install-directory>` — remove currently installed members of a catalog group.
- `skilldeck init --repository <url-or-path> --reference <ref>` — configure the user's catalog.
- `skilldeck bootstrap <path> --quickstart|--empty` — create a new catalog scaffold and, by default, initialize/commit it as a local Git repository on branch `main`.
- `skilldeck bootstrap <path> --quickstart|--empty --no-git` — generate catalog files only.

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

## Helpful workflow

When a user asks to get started:

```sh
skilldeck bootstrap ./skilldeck-catalog --quickstart
skilldeck init --repository ./skilldeck-catalog --reference main
skilldeck doctor
skilldeck install-group quickstart ./installed-skills
```

If the user chooses `--no-git`, tell them to run `git init --initial-branch=main`, `git add .`, and `git commit -m "Start Skilldeck catalog"` from inside the generated catalog before configuring Skilldeck. If automatic commit fails because Git identity is missing, explain that files exist and are staged; configure `user.name` and `user.email`, then run that commit command.

Adjust paths for the user's shell and operating system. Keep output portable; avoid shell-specific syntax unless the user is already using that shell.
