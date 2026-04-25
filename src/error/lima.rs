use thiserror::Error;

#[derive(Debug, Error)]
pub enum LimaError {
    #[error(transparent)]
    Process(#[from] crate::error::process::ProcessError),
    #[error("failed to parse limactl JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("lima provision script `{script}` failed: {detail}")]
    ProvisionFailed { script: String, detail: String },
    #[error("failed to prepare guest support file `{path}`: {source}")]
    GuestSupport {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("prepared base instance `{0}` was not found")]
    MissingPreparedBase(String),
    #[error("failed to write prepared base metadata at `{path}`: {source}")]
    BaseMetadata {
        path: String,
        #[source]
        source: std::io::Error,
    },
}
