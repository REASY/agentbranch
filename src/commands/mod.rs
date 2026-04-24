pub mod agent;
pub mod attach;
pub mod close;
pub mod doctor;
pub mod export;
pub mod gc;
pub mod kill;
pub mod launch;
pub mod logs;
pub mod open;
pub mod prepare;
pub mod ps;
pub mod repair;
pub mod run;
pub(crate) mod session_slot;
pub mod shell;
pub mod show;
pub mod ssh;
pub mod start;
pub mod stop;
pub mod sync_back;
pub mod watch;

pub(crate) fn resolve_session_name(
    selector: &crate::cli::SessionSelector,
) -> Result<(String, crate::types::SessionName), crate::error::AppError> {
    let raw = selector.resolve_owned()?;
    let parsed = crate::types::SessionName::try_from(raw.as_str())?;
    Ok((raw, parsed))
}

pub(crate) fn find_existing_session(
    conn: &rusqlite::Connection,
    session_name: &crate::types::SessionName,
    raw_name: &str,
) -> Result<crate::db::sessions::SessionRow, crate::error::AppError> {
    crate::db::sessions::find_session(conn, session_name)?.ok_or_else(|| {
        crate::error::AppError::Validation(crate::error::ValidationError::SessionNotFound(
            raw_name.to_owned(),
        ))
    })
}
