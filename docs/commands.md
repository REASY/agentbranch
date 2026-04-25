# Command reference

Every `agbranch` subcommand, grouped by what you're trying to do. For full flag lists, run `agbranch <command> --help`.

| Group         | Command                  | What it does                                                                                 |
|---------------|--------------------------|----------------------------------------------------------------------------------------------|
| **Setup**     | `base prepare`           | Build or refresh the prepared base VM for your host (`--rebuild`, `--timeout`, `--json`).   |
|               | `doctor`                 | Validate host: `limactl` version, platform prerequisites, orphaned VMs.                      |
| **Create**    | `launch`                 | Start a sandbox session, optionally `--seed` and `--agent`.                                  |
|               | `open`                   | Start a git-native repo session on `--repo <path>`, optionally `--base <ref>` and `--agent`. |
| **Inspect**   | `ps`                     | List sessions with live status; `--all`, `--search`, `--state`, `--sort`.                    |
|               | `base show`              | Show the current prepared base VM, readiness, fingerprint, and sizing (`--json`).            |
|               | `show`                   | Structured detail for a single session (mode, refs, provider, tmux, VM).                    |
|               | `logs`                   | Stream one of: `events`, `provision`, `sync`, `guest`, `kernel`. `--follow` tails.           |
|               | `watch`                  | Continuous state stream; snapshots on change plus event lines.                               |
| **Enter**     | `attach`                 | Open the session's tmux `shell` or `agent` window.                                           |
|               | `shell`                  | Fresh tmux shell in the guest (`--forward-ssh-agent`, `--env`, `--env-file`).                |
|               | `ssh`                    | Raw SSH into the guest without tmux.                                                         |
|               | `run`                    | Execute a one-shot command in the guest (`--` separates the command).                        |
| **Providers** | `agent start --provider` | Bootstrap Codex/Claude/Gemini inside an existing session.                                    |
|               | `agent stop`             | Stop the session's agent window.                                                             |
|               | `kill`                   | Force-stop the agent (and optionally the VM with `--force`).                                 |
| **VM power**  | `start` / `stop`         | Power the session VM on or off without closing the session.                                  |
| **Finish**    | `sync-back`              | Repo sessions: bundle guest HEAD back to the host review branch.                             |
|               | `export`                 | Sandbox sessions: copy files out of `~/sandbox/<session>` to the host.                       |
|               | `close`                  | Destroy the session — requires `--sync` or `--discard` plus `--yes`.                         |
| **Recover**   | `repair`                 | Deterministic recovery for stuck sessions, driven by lifecycle state.                        |
|               | `gc`                     | Reclaim staging dirs, log dirs, and obsolete base VMs.                                       |

Commands print human output by default. `--json` enables machine-readable output where supported; streaming commands may emit line-delimited JSON instead of a single document. Most session-scoped commands accept either the `SESSION` positional or `--session <name>`; `launch` and `open` require `--session`, and `watch` uses `--session` only.

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
