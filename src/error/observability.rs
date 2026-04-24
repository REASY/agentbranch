use crate::error::{db::DbError, process::ProcessError};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ObservabilityError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Process(#[from] ProcessError),
    #[error("io error at `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
