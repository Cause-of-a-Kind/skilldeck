# Contributing

Thanks for helping improve Skilldeck.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Tests use local Git repositories and must not depend on network access.

## Guidelines

- Keep the CLI catalog-agnostic.
- Do not store credentials; rely on system Git for authentication.
- Preserve compatibility with `skills/`, `external-skills.toml`, and `skill-groups.toml`.
- Add tests for behavior changes, especially install/update/remove safeguards.
- Keep modules focused and rustfmt/clippy clean.
