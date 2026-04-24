use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("migration error: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error(
        "existing catalog at {path} pre-dates schema migrations; delete it (state is disposable during 0.x) and re-run, or restore from a post-unification agbranch install"
    )]
    PreMigrationCatalog { path: PathBuf },
    #[error("session lock is already held")]
    LockBusy,
}
