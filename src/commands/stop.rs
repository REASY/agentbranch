use crate::cli::SessionArgs;
use crate::commands::{find_existing_session, resolve_session_name};
use crate::db::connect::open_catalog;
use crate::db::models::LifecycleState;
use crate::db::sessions::update_lifecycle_state_with_timestamps;
use crate::error::AppError;
use crate::lima::instance::stop_instance;
use crate::platform::host::HostContext;
use crate::session::state::transition_after_stop;
use crate::util::process::RealCommandRunner;
use crate::util::time::utc_now;

pub fn run(args: SessionArgs) -> Result<(), AppError> {
    let (session_name_raw, session_name) = resolve_session_name(&args.session)?;
    let host = HostContext::detect()?;
    let conn = open_catalog(&host.state_roots.db)?;
    let session = find_existing_session(&conn, &session_name, &session_name_raw)?;
    stop_instance(&RealCommandRunner, &session.vm_name)?;
    let now = utc_now();
    let stopped_at = if matches!(
        session.lifecycle_state,
        LifecycleState::Stopped | LifecycleState::Closed
    ) {
        None
    } else {
        Some(&now)
    };
    update_lifecycle_state_with_timestamps(
        &conn,
        &session_name,
        transition_after_stop(),
        &now,
        None,
        stopped_at,
        None,
    )?;
    Ok(())
}
