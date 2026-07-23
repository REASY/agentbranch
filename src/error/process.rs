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
    #[error("`{program}` exceeded the {timeout:?} execution deadline")]
    Timeout {
        program: String,
        timeout: std::time::Duration,
    },
    #[error("`{program}` exceeded the {limit}-byte {stream} capture limit")]
    OutputLimit {
        program: String,
        stream: &'static str,
        limit: usize,
    },
    #[error("command runner for `{program}` does not support standard input")]
    InputUnsupported { program: String },
    #[error("`{program}` was interrupted")]
    Interrupted { program: String },
}
