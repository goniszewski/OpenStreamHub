# Git hooks

Enable once per clone:

```bash
git config core.hooksPath .githooks
```

- `pre-commit`: `cargo fmt --check` and `cargo clippy -D warnings` (both workspaces) when Rust files are staged; `tsc` and `vitest` when app frontend files are staged. Docs-only commits run nothing.
- `pre-push`: `cargo test` in both workspaces when the push contains Rust changes.

Both match what CI runs. Skip in an emergency with `--no-verify`.
