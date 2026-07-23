CREATE TABLE provider_preferences (
  provider TEXT PRIMARY KEY,
  import_auth INTEGER NOT NULL CHECK (import_auth IN (0, 1))
);
