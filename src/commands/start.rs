use crate::cli::SessionArgs;
use crate::commands::{find_existing_session, resolve_session_name};
use crate::db::connect::open_catalog;
use crate::db::models::LifecycleState;
use crate::db::sessions::update_lifecycle_state_with_timestamps;
use crate::error::AppError;
use crate::lima::client::{LimaClient, LimactlClient};
use crate::platform::host::HostContext;
use crate::session::guest_support;
use crate::session::orchestration::run_step;
use crate::session::state::transition_after_start;
use crate::util::process::RealCommandRunner;
use crate::util::time::utc_now;
use std::time::Instant;

pub fn run(args: SessionArgs) -> Result<(), AppError> {
    let (session_name_raw, session_name) = resolve_session_name(&args.session)?;
    let host = HostContext::detect()?;
    let conn = open_catalog(&host.state_roots.db)?;
    let session = find_existing_session(&conn, &session_name, &session_name_raw)?;
    let runner = RealCommandRunner;
    let lima = LimactlClient::new(&runner);
    let total_start = Instant::now();
    run_step(&session_name, "start", "start-vm", &total_start, || {
        Ok(lima.start_instance(&session.vm_name)?)
    })?;
    if let Some(tmux_socket) = session.guest_tmux_socket_path.as_ref() {
        run_step(&session_name, "start", "ensure-shell", &total_start, || {
            Ok(guest_support::ensure_workspace_and_shell(
                &lima,
                &session.vm_name,
                &session_name,
                tmux_socket,
                &session.guest_workspace_path,
            )?)
        })?;
    }
    run_step(&session_name, "start", "update-state", &total_start, || {
        let now = utc_now();
        let started_at = if session.lifecycle_state == LifecycleState::Running {
            None
        } else {
            Some(&now)
        };
        update_lifecycle_state_with_timestamps(
            &conn,
            &session_name,
            transition_after_start(),
            &now,
            started_at,
            None,
            None,
        )?;
        Ok(())
    })?;
    Ok(())
}
