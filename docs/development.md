# Development

## Local checks

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

## Testing in CI

- **Unit + integration tests** run on every push and pull request via `.github/workflows/ci.yml` on both `ubuntu-latest` and `macos-latest`:
  - `cargo fmt --all --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test --all-targets --all-features`
- **Nightly smoke-e2e** (`.github/workflows/smoke-e2e.yml`, `0 3 * * *` + `workflow_dispatch`) runs the built binary against real Lima VMs on self-hosted macOS and Linux runners, uploading artifacts from `e2e/artifacts`. It exercises:
  - prepared-base preflight,
  - sandbox `launch --seed` → `run` → `export` → `close --discard`,
  - repo `open` → `run` → `sync-back` → `open --base agbranch/<session>` → `close --sync`,
  - session controls (`ps`, `show`, `start`, `stop`, `shell --json`, `ssh --json`, `kill`),
  - parallel session isolation, blocked syncs with salvage-patch export, `watch`, `logs`, deterministic `repair`, and `gc`.
- The smoke suite deliberately does not launch real external agent CLIs — agent-behavior canaries are a separate, orthogonal concern.

Run the smoke suite locally:

```bash
cargo build
./target/debug/agbranch prepare
scripts/smoke-e2e.sh --binary ./target/debug/agbranch
```

## Repository layout

- `src/cli.rs` — clap definitions, single source of truth for command surface.
- `src/app.rs` — dispatch from parsed CLI into command handlers.
- `src/commands/` — one module per subcommand.
- `src/lima/`, `src/session/`, `src/git/`, `src/db/`, `src/policy/`, `src/provider/` — subsystem modules.
- `lima/safe-sync-{macos,linux}.yaml` + `lima/provision/*.sh` — Lima templates and provisioning scripts for the prepared base.
- `tests/` — Rust integration tests.
- `scripts/smoke-e2e.sh` — end-to-end runner used by the nightly workflow.
