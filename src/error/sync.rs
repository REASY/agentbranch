use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("io error at `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Process(#[from] crate::error::process::ProcessError),
    #[error("failed to strip repo root `{root}` from `{path}`")]
    StripPrefix { root: PathBuf, path: PathBuf },
    #[error("unsafe sync path `{path}` escapes repo root `{root}`")]
    PathEscapesRepo { root: PathBuf, path: PathBuf },
    #[error("symlink `{path}` resolves outside the repo root: `{target}`")]
    UnsafeSymlink { path: PathBuf, target: PathBuf },
    #[error("patch export failed: {message}")]
    PatchExport { message: String },
    #[error("git diff failed with status {status}: {stderr}")]
    GitDiffFailed { status: i32, stderr: String },
}
