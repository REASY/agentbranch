CREATE TABLE session_launch_retries (
  session_name TEXT PRIMARY KEY,
  checkpoint TEXT NOT NULL,
  last_error TEXT,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (session_name) REFERENCES sessions(name) ON DELETE CASCADE
);
