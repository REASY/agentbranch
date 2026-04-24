use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ValidationError {
    // ---- Platform / environment ----
    #[error("unsupported host platform")]
    UnsupportedHost,

    #[error("failed to parse macOS version")]
    MacosVersionParse,

    // ---- CLI argument shapes ----
    #[error(
        "session provided both positionally (`{positional}`) and via --session (`{flag}`); \
         choose one or make them match"
    )]
    SessionSelectorConflict { positional: String, flag: String },

    #[error("session is required")]
    SessionSelectorRequired,

    #[error("session name exceeds the {max}-character limit: `{name}`")]
    InvalidSessionName { name: String, max: usize },

    #[error("invalid memory size `{value}`: {reason}")]
    InvalidMemorySize { value: String, reason: String },

    #[error("invalid disk size `{value}`: {reason}")]
    InvalidDiskSize { value: String, reason: String },

    #[error("invalid environment entry `{0}`")]
    InvalidEnvEntry(String),

    #[error("choose exactly one of --shell or --agent")]
    AttachRequiresExactlyOne,

    // ---- Session lookup / state ----
    #[error("session `{0}` was not found")]
    SessionNotFound(String),

    #[error(
        "session `{name}` already exists in the local catalog (state: {state}, sync: {sync}); \
         choose a different session name"
    )]
    SessionAlreadyExists {
        name: String,
        state: String,
        sync: String,
    },

    #[error(
        "vm `{vm_name}` is already reserved by session `{owner}` in the local catalog \
         (state: {state}, sync: {sync}); choose a different session name"
    )]
    VmAlreadyReserved {
        vm_name: String,
        owner: String,
        state: String,
        sync: String,
    },

    #[error("session is missing tmux socket metadata")]
    SessionMissingTmuxSocket,

    #[error("cannot attach to {target} for session `{session}`: {reason}")]
    AttachTargetUnavailable {
        session: String,
        target: &'static str,
        reason: String,
    },

    // ---- Business rules ----
    #[error("sandbox sessions must export artifacts and then close with --discard")]
    CloseRequiresDiscardForSandbox,

    #[error("export is only valid for sandbox sessions")]
    ExportRequiresSandbox,

    #[error("guest export source must stay under ~/sandbox/<session>")]
    ExportPathOutsideSandbox,

    #[error("host export destination may not be inside .git")]
    ExportDestinationInsideGit,

    #[error("destination {path} already exists; pass --force to overwrite")]
    ExportDestinationExists { path: String },

    #[error("failed to derive guest home for export")]
    ExportGuestHomeDeriveFailure,

    #[error(
        "open requires a git repository; use `agbranch launch --seed ...` \
         for scratch directories"
    )]
    OpenRequiresGitRepo,

    #[error(
        "no git identity found; configure git user.name and user.email \
         before opening a repo session"
    )]
    OpenRequiresGitIdentity,

    #[error("sync-back is only valid for git-native repo sessions")]
    SyncBackRequiresGitNative,

    #[error("git-native session is missing {field}")]
    SyncBackMissingSessionField { field: &'static str },

    // ---- Providers ----
    #[error(
        "session already belongs to provider `{current}`; \
         open a new session to use `{requested}`"
    )]
    ProviderConflict { current: String, requested: String },

    #[error("unsupported provider")]
    UnsupportedProvider,

    #[error(
        "provider CLI `{name}` is unavailable in the prepared base; \
         run `agbranch prepare --rebuild`"
    )]
    ProviderCliMissing { name: String },

    // ---- Logs ----
    #[error("log source `{kind}` is not available for session `{session}`")]
    LogSourceNotAvailable { kind: String, session: String },

    #[error("json output is not supported with --follow for {channel} logs")]
    LogsFollowJsonUnsupported { channel: &'static str },

    // ---- Doctor aggregation ----
    #[error("{messages}")]
    DoctorReportIssues { messages: String },

    // ---- Guest process outcomes ----
    #[error("{program} exited with status {status}")]
    GuestCommandFailed { program: String, status: i32 },

    #[error("failed to resolve SSH host alias")]
    SshResolutionFailed,

    // ---- Composite / rollback paths ----
    #[error("{step}: {detail}")]
    StepFailed { step: &'static str, detail: String },

    #[error("{original}; {operation} rollback failed: {cleanup}")]
    RollbackFailed {
        original: String,
        cleanup: String,
        operation: &'static str,
    },

    #[error("{primary}; {cleanup}")]
    AgentBootstrapChainedFailure { primary: String, cleanup: String },

    #[error("failed to remove temporary secret file {path:?}: {source}")]
    TempSecretCleanupFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_selector_conflict_message() {
        let err = ValidationError::SessionSelectorConflict {
            positional: "foo".to_owned(),
            flag: "bar".to_owned(),
        };
        let msg = err.to_string();
        assert!(msg.contains("foo"));
        assert!(msg.contains("bar"));
        assert!(msg.contains("--session"));
    }

    #[test]
    fn macos_version_parse_is_a_single_short_message() {
        assert_eq!(
            ValidationError::MacosVersionParse.to_string(),
            "failed to parse macOS version"
        );
    }

    #[test]
    fn sync_back_missing_session_field_embeds_field_name() {
        let err = ValidationError::SyncBackMissingSessionField {
            field: "host git root",
        };
        assert_eq!(
            err.to_string(),
            "git-native session is missing host git root"
        );
    }

    #[test]
    fn step_failed_contains_step_and_detail() {
        let err = ValidationError::StepFailed {
            step: "start-vm",
            detail: "boom".to_owned(),
        };
        assert_eq!(err.to_string(), "start-vm: boom");
    }

    #[test]
    fn rollback_failed_renders_all_three_parts() {
        let err = ValidationError::RollbackFailed {
            original: "open failed".to_owned(),
            cleanup: "rm -rf failed".to_owned(),
            operation: "open",
        };
        assert_eq!(
            err.to_string(),
            "open failed; open rollback failed: rm -rf failed"
        );
    }

    #[test]
    fn temp_secret_cleanup_failed_preserves_source() {
        use std::error::Error;
        let err = ValidationError::TempSecretCleanupFailed {
            path: PathBuf::from("/tmp/secret"),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "no"),
        };
        assert!(err.source().is_some(), "source chain must be preserved");
    }
}
