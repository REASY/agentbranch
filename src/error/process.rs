use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("failed to spawn `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`{program}` exited with status {status}: {stderr}")]
    Failed {
        program: String,
        status: i32,
        stderr: String,
    },
    #[error("`{program}` produced non-utf8 output")]
    NonUtf8 { program: String },
}
