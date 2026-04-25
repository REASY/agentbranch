# Observability

Everything a session does is observable from outside the VM.

- **Catalog** — a local SQLite catalog tracks lifecycle state, sync state, lock owners, provider metadata, and an append-only event log per session.
- **`ps --json`** — array of session rows with lifecycle timestamps.
- **`show --json`** — single session with mode, review branch, provider, tmux socket, live VM status, and guest runtime probe.
- **`logs --source {events|provision|sync|guest|kernel} --follow`** — stream any of the five channels.
- **`watch --json`** — ndjson stream of snapshots + events; tails state transitions in real time.
- **`doctor --json`** — `{ ok, platform, lima_version, state_root, prepared_base, ... }` for health checks.
- **`repair`** — reads the session's lifecycle state and picks a deterministic recovery action (no-op, restart, finish-destroy); returns `Blocked` when manual intervention is required.

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
├── state.db        # SQLite catalog (sessions, sync_runs, session_events)
├── state.db-wal    # SQLite WAL journal
├── state.db-shm    # SQLite shared memory
├── logs/           # per-session log directories
├── staging/        # sync-back bundles + salvage patches
└── locks/          # per-session locks plus base.lock
```

`agbranch doctor` prints the resolved state root on its last line; `agbranch doctor --json` returns it as `state_root`.
