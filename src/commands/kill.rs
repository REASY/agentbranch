use crate::cli::KillArgs;
use crate::db::connect::open_catalog;
use crate::db::sessions::find_session;
use crate::error::{AppError, ValidationError};
use crate::lima::{
    client::{LimaClient, LimactlClient},
    tmux,
};
use crate::platform::host::HostContext;
use crate::types::SessionName;
use crate::util::ids::session_vm_name;
use crate::util::process::RealCommandRunner;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KillPlan {
    pub session_name: SessionName,
    pub vm_name: String,
    pub tmux_session_name: String,
    pub window_name: String,
    pub stop_vm: bool,
}

pub fn build_kill_plan(session: &str, force: bool) -> Result<KillPlan, ValidationError> {
    let session_name = SessionName::try_from(session)?;
    Ok(KillPlan {
        vm_name: session_vm_name(&session_name).to_string(),
        tmux_session_name: session_name.to_string(),
        window_name: "agent".to_owned(),
        stop_vm: force,
        session_name,
    })
}

pub fn run(args: KillArgs) -> Result<(), AppError> {
    let session_name_raw = args.session.resolve_owned()?;
    let plan = build_kill_plan(&session_name_raw, args.force)?;
    let host = HostContext::detect()?;
    let conn = open_catalog(&host.state_roots.db)?;
    let session = find_session(&conn, &plan.session_name)?.ok_or_else(|| {
        AppError::Validation(ValidationError::SessionNotFound(session_name_raw.clone()))
    })?;

    let runner = RealCommandRunner;
    let lima = LimactlClient::new(&runner);
    lima.bash(
        &session.vm_name,
        &tmux::kill_window_command(
            session
                .guest_tmux_socket_path
                .as_ref()
                .ok_or(ValidationError::SessionMissingTmuxSocket)?,
            &plan.tmux_session_name,
            &plan.window_name,
        ),
    )?;

    if plan.stop_vm {
        lima.stop_instance(&session.vm_name)?;
    }

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "session": plan.session_name,
                "vm_name": plan.vm_name,
                "force": plan.stop_vm,
            })
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kill_without_force_only_stops_guest_agent() {
        let plan = build_kill_plan("research", false).expect("kill plan");
        assert_eq!(plan.tmux_session_name, "research");
        assert_eq!(plan.vm_name, "agbranch-research");
        assert!(!plan.stop_vm);
    }

    #[test]
    fn kill_with_force_escalates_to_vm_stop() {
        let plan = build_kill_plan("research", true).expect("kill plan");
        assert!(plan.stop_vm);
    }
}
