use crate::db::launch_retries::{save_launch_checkpoint, save_launch_error};
use crate::error::AppError;
use crate::types::SessionName;
use crate::util::time::utc_now;
use rusqlite::Connection;

pub const VM_CLONED: &str = "vm_cloned";
pub const PORTS_CONFIGURED: &str = "ports_configured";
pub const VM_STARTED: &str = "vm_started";
pub const GUEST_SUPPORT_INSTALLED: &str = "guest_support_installed";
pub const WORKSPACE_SEEDED: &str = "workspace_seeded";
pub const SHELL_READY: &str = "shell_ready";
pub const GIT_IDENTITY_CONFIGURED: &str = "git_identity_configured";
pub const AGENT_STARTED: &str = "agent_started";

pub fn checkpoint(
    catalog: &Connection,
    session: &SessionName,
    checkpoint: &'static str,
) -> Result<(), AppError> {
    save_launch_checkpoint(catalog, session, checkpoint, &utc_now())?;
    Ok(())
}

pub fn preserve_failure(
    catalog: &Connection,
    session: &SessionName,
    err: &AppError,
) -> Result<AppError, AppError> {
    save_launch_error(catalog, session, &err.to_string(), &utc_now())?;
    Ok(AppError::Blocked(format!(
        "session `{session}` was preserved after a failed launch: {err}\n\
         resume it with `agbranch retry {session}`"
    )))
}

pub fn remaining_stages<'a>(
    stages: &'a [&'static str],
    completed: &str,
) -> Option<&'a [&'static str]> {
    stages
        .iter()
        .position(|stage| *stage == completed)
        .map(|index| &stages[index + 1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaining_stages_start_after_the_checkpoint() {
        let stages = [VM_CLONED, VM_STARTED, SHELL_READY];
        assert_eq!(remaining_stages(&stages, VM_CLONED), Some(&stages[1..]));
        assert_eq!(remaining_stages(&stages, SHELL_READY), Some(&stages[3..]));
    }

    #[test]
    fn unknown_checkpoint_is_rejected() {
        assert_eq!(remaining_stages(&[VM_CLONED], "future_checkpoint"), None);
    }
}
