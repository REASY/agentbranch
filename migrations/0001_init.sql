CREATE TABLE IF NOT EXISTS sessions (
  name TEXT PRIMARY KEY,
  vm_name TEXT NOT NULL UNIQUE,
  session_mode TEXT NOT NULL,
  repo_sync_mode TEXT,
  host_context_path TEXT,
  guest_workspace_path TEXT NOT NULL,
  seed_host_path TEXT,
  host_git_root TEXT,
  host_head_oid_at_open TEXT,
  host_head_ref_at_open TEXT,
  host_dirty_at_open INTEGER NOT NULL,
  base_ref TEXT,
  review_branch TEXT,
  session_ref_base TEXT,
  session_ref_head TEXT,
  provider_kind TEXT,
  imported_provider_files_json TEXT NOT NULL DEFAULT '[]',
  guest_tmux_socket_path TEXT,
  shell_window_name TEXT,
  agent_window_name TEXT,
  agent_launch_preset TEXT,
  lifecycle_state TEXT NOT NULL,
  sync_state TEXT NOT NULL,
  lock_owner_pid INTEGER,
  lock_operation TEXT,
  created_at TEXT NOT NULL,
  last_started_at TEXT,
  last_stopped_at TEXT,
  last_used_at TEXT NOT NULL,
  closed_at TEXT
);

CREATE TABLE IF NOT EXISTS sync_runs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_name TEXT NOT NULL,
  direction TEXT NOT NULL,
  result TEXT NOT NULL,
  started_at TEXT NOT NULL,
  finished_at TEXT,
  staging_path TEXT,
  patch_path TEXT,
  error_text TEXT,
  FOREIGN KEY (session_name) REFERENCES sessions(name) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS session_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_name TEXT NOT NULL,
  at TEXT NOT NULL,
  level TEXT NOT NULL,
  kind TEXT NOT NULL,
  message TEXT NOT NULL,
  FOREIGN KEY (session_name) REFERENCES sessions(name) ON DELETE CASCADE
);
