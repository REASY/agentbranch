use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Config(#[from] crate::error::config::ConfigError),
    #[error(transparent)]
    Db(#[from] crate::error::db::DbError),
    #[error(transparent)]
    Lima(#[from] crate::error::lima::LimaError),
    #[error(transparent)]
    Observability(#[from] crate::error::observability::ObservabilityError),
    #[error(transparent)]
    Process(#[from] crate::error::process::ProcessError),
    #[error(transparent)]
    Sync(#[from] crate::error::sync::SyncError),
    #[error(transparent)]
    Validation(#[from] crate::error::validation::ValidationError),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Blocked(String),
    #[error("operation interrupted")]
    Interrupted,
    #[error("{0}")]
    NotImplemented(&'static str),
}

impl AppError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Config(_) | Self::Validation(_) => 1,
            Self::Observability(_) | Self::NotImplemented(_) => 2,
            Self::Blocked(_) => 3,
            Self::Interrupted => 4,
            Self::Db(_) => 5,
            Self::Lima(_) => 6,
            Self::Process(_) => 7,
            Self::Io(_) => 8,
            Self::Sync(_) => 9,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::db::DbError;
    use crate::error::lima::LimaError;
    use crate::error::process::ProcessError;
    use crate::error::sync::SyncError;
    use std::io;

    #[test]
    fn db_error_routes_via_from() {
        let err: AppError = DbError::LockBusy.into();
        assert!(matches!(err, AppError::Db(DbError::LockBusy)));
        assert_eq!(err.exit_code(), 5);
    }

    #[test]
    fn lima_error_routes_via_from() {
        let err: AppError = LimaError::MissingPreparedBase("agbranch-base-macos".to_owned()).into();
        assert!(matches!(
            err,
            AppError::Lima(LimaError::MissingPreparedBase(_))
        ));
        assert_eq!(err.exit_code(), 6);
    }

    #[test]
    fn process_error_routes_via_from() {
        let err: AppError = ProcessError::Failed {
            program: "git".to_owned(),
            status: 1,
            stderr: String::new(),
        }
        .into();
        assert!(matches!(
            err,
            AppError::Process(ProcessError::Failed { .. })
        ));
        assert_eq!(err.exit_code(), 7);
    }

    #[test]
    fn io_error_routes_via_from() {
        let err: AppError = io::Error::new(io::ErrorKind::NotFound, "missing").into();
        assert!(matches!(err, AppError::Io(_)));
        assert_eq!(err.exit_code(), 8);
    }

    #[test]
    fn sync_error_routes_via_from() {
        let err: AppError = SyncError::PatchExport {
            message: "boom".to_owned(),
        }
        .into();
        assert!(matches!(err, AppError::Sync(SyncError::PatchExport { .. })));
        assert_eq!(err.exit_code(), 9);
    }

    #[test]
    fn legacy_exit_codes_unchanged() {
        assert_eq!(AppError::Blocked("x".to_owned()).exit_code(), 3);
        assert_eq!(AppError::Interrupted.exit_code(), 4);
        assert_eq!(AppError::NotImplemented("x").exit_code(), 2);
    }

    #[test]
    fn io_error_display_includes_prefix() {
        let err: AppError = io::Error::new(io::ErrorKind::NotFound, "missing").into();
        assert!(err.to_string().contains("I/O:"), "{err}");
    }

    #[test]
    fn typed_error_display_is_transparent() {
        let err: AppError = DbError::LockBusy.into();
        assert_eq!(err.to_string(), DbError::LockBusy.to_string());
    }
}
