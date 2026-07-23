# Observability

Everything a session does is observable from outside the VM.

- **Catalog** — a local SQLite catalog tracks lifecycle state, sync state, lock owners, provider metadata, and an append-only event log per session.
- **`ps --json`** — array of session rows with lifecycle timestamps.
- **`show --json`** — single session with mode, review branch, provider, tmux socket, live VM status, and guest runtime probe.
- **`logs --source {events|provision|sync|guest|kernel} --follow`** — stream any of the five channels.
- **`watch --json`** — ndjson stream of snapshots + events; tails state transitions in real time.
- **`doctor --json`** — `{ ok, platform, lima_version, state_root, prepared_base, ... }` for health checks.
- **`repair`** — reads the session's lifecycle state and picks a deterministic recovery action (no-op, restart, finish-destroy); returns `Blocked` when manual intervention is required.
- **Launch timings** — `launch` and `open` expose ordered phase durations, total wall time, and the slowest phase in human and JSON output.

## State directory

Session catalog, logs, staging bundles, and filesystem locks live under a single state root:

| Platform | Default location                                                     |
|----------|----------------------------------------------------------------------|
| macOS    | `~/Library/Application Support/agbranch/`                            |
| Linux    | `$XDG_STATE_HOME/agbranch/` if set, else `~/.local/state/agbranch/`  |

Override with `AGBRANCH_STATE_ROOT=/some/path` — that directory becomes the state root verbatim.

Layout:

```
<state-root>/
├── state.db        # SQLite catalog (sessions, events, sync runs, provider preferences)
├── state.db-wal    # SQLite WAL journal
├── state.db-shm    # SQLite shared memory
├── logs/           # per-session log directories
├── staging/        # sync-back bundles + salvage patches
├── assets/         # extracted Lima asset cache, keyed by bundle fingerprint
└── locks/          # per-session locks plus base.lock and assets.lock
```

`agbranch doctor` prints the resolved state root on its last line; `agbranch doctor --json` returns it as `state_root`.

## Environment variables

| Variable                        | Purpose                                                                                     |
|---------------------------------|---------------------------------------------------------------------------------------------|
| `AGBRANCH_STATE_ROOT`           | Overrides the state directory verbatim (see above).                                         |
| `AGBRANCH_PREPARED_BASE_NAME`   | Overrides the Lima VM name for the prepared base. Surfaces as `name_source: env_override`. |
| `AGBRANCH_LIMA_ASSETS_DIR`      | Points at an alternate `lima/` tree to use instead of the binary's embedded bundle. The directory must contain every in-scope file; typos fail fast with an actionable error. Useful for editing provisioning scripts in place without rebuilding. |
