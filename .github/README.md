# GitHub Actions

- **CI** (`workflows/ci.yml`): on every push and pull request to `main`, runs `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test` inside the Nix dev shell, and builds the Nix package (`nix build`).
- **PR Labeler** (`workflows/pr-labeler.yml`): labels pull requests by the files they touch (`labeler.yml`).
- **Dependabot** (`dependabot.yml`): weekly updates for Cargo dependencies and GitHub Actions.

Run the same checks locally:

```sh
nix develop -c cargo fmt --check
nix develop -c cargo clippy --all-targets -- -D warnings
nix develop -c cargo test
nix build
```
