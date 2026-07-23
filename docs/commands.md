# Command reference

Every `agbranch` subcommand, grouped by what you're trying to do. For full flag lists, run `agbranch <command> --help`.

| Group         | Command                  | What it does                                                                                 |
|---------------|--------------------------|----------------------------------------------------------------------------------------------|
| **Setup**     | `base prepare`           | Build or refresh the prepared base VM for your host (`--rebuild`, `--timeout`, `--json`).   |
|               | `doctor`                 | Validate host: `limactl` version, platform prerequisites, orphaned VMs.                      |
| **Create**    | `launch`                 | Start a sandbox session, optionally `--seed`, `--agent`, and `--auth`.                       |
|               | `open`                   | Start a git-native repo session, optionally `--base`, `--agent`, and `--auth`.               |
| **Inspect**   | `ps`                     | List sessions with live status; `--all`, `--search`, `--state`, `--sort`.                    |
|               | `ports`                  | List published localhost ports and whether guest services are listening.                     |
|               | `base show`              | Show the current prepared base VM, readiness, fingerprint, and sizing (`--json`).            |
|               | `show`                   | Structured detail for a single session (mode, refs, provider, tmux, VM).                    |
|               | `logs`                   | Stream one of: `events`, `provision`, `sync`, `guest`, `kernel`. `--follow` tails.           |
|               | `watch`                  | Continuous state stream; snapshots on change plus event lines.                               |
| **Enter**     | `attach`                 | Open the session's tmux `shell` or `agent` window.                                           |
|               | `shell`                  | Fresh tmux shell in the guest (`--forward-ssh-agent`, `--env`, `--env-file`).                |
|               | `ssh`                    | Raw SSH into the guest without tmux.                                                         |
|               | `run`                    | Execute a one-shot command in the guest (`--` separates the command).                        |
| **Providers** | `agent start --provider` | Bootstrap a provider in an existing session; supports `--auth import\|none\|ask`.            |
|               | `agent stop`             | Stop the session's agent window.                                                             |
|               | `kill`                   | Force-stop the agent (and optionally the VM with `--force`).                                 |
| **VM power**  | `start` / `stop`         | Power the session VM on or off without closing the session.                                  |
| **Finish**    | `sync-back`              | Repo sessions: bundle guest HEAD back to the host review branch.                             |
|               | `export`                 | Sandbox sessions: copy files out of `~/sandbox/<session>` to the host.                       |
|               | `close`                  | Destroy the session — requires `--sync` or `--discard` plus `--yes`.                         |
| **Recover**   | `repair`                 | Deterministic recovery for stuck sessions, driven by lifecycle state.                        |
|               | `gc`                     | Reclaim staging dirs, log dirs, and obsolete base VMs.                                       |

Commands print human output by default. `--json` enables machine-readable output where supported; streaming commands may emit line-delimited JSON instead of a single document. Most session-scoped commands accept either the `SESSION` positional or `--session <name>`; `launch` and `open` require `--session`, and `watch` uses `--session` only.

For `launch --agent`, `open --agent`, and `agent start`, `--auth import` imports detected host credentials without prompting, `--auth none` skips imports, and `--auth ask` forces an interactive prompt when credentials are detected. The resulting import/none decision is remembered per provider in the state catalog. With no flag, the remembered choice is reused; if none exists, interactive commands prompt once and non-interactive commands import nothing.

`launch` and `open` accept repeatable `--publish` mappings. `--publish 3000` maps host localhost port 3000 to guest port 3000; `--publish 8080:3000` maps a different host port; append `/udp` for UDP. Published ports never bind to non-loopback host addresses. Use `agbranch ports SESSION [--json]` to inspect configured endpoints and live guest listener state.

## Launch and open timings

Each `launch` and `open` phase is announced immediately, then reports elapsed phase and total time with millisecond precision when it completes. Human output ends with an ordered phase breakdown, each phase's percentage of wall time, and the slowest phase. The `start-vm` measurement covers the complete `limactl start` call, including Lima waiting for the guest to become ready.

With `--json`, the same data is returned as:

```json
{
  "timings": {
    "total_ms": 14250,
    "phases": [
      { "name": "clone-vm", "duration_ms": 1000 },
      { "name": "start-vm", "duration_ms": 12000 }
    ],
    "slowest_phase": { "name": "start-vm", "duration_ms": 12000 }
  }
}
```

## Exit codes

`agbranch` returns one of the following codes on exit:

| Code | Meaning                                          |
|------|--------------------------------------------------|
| 0    | Success                                          |
| 1    | User input / config error (`--help` for options) |
| 2    | Internal error (observability, not-implemented)  |
| 3    | Action required (e.g. blocked sync)              |
| 4    | Interrupted (signal)                             |
| 5    | Catalog / database error                         |
| 6    | VM runtime (Lima) error                          |
| 7    | Command runner error                             |
| 8    | Filesystem I/O error                             |
| 9    | Sync subsystem error                             |

Scripts that only care about success/failure can keep grepping on non-zero. Consumers that want to react differently to "DB corrupt" versus "Lima unavailable" can discriminate on these codes.
