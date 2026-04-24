use crate::db::sessions::{find_session, find_session_by_vm_name};
use crate::error::{AppError, ValidationError};
use crate::types::{SessionName, VmName};
use rusqlite::Connection;

pub(crate) fn ensure_runtime_session_slot_available(
    conn: &Connection,
    session_name: &SessionName,
    vm_name: &VmName,
) -> Result<(), AppError> {
    if let Some(existing) = find_session(conn, session_name)? {
        return Err(AppError::Validation(
            ValidationError::SessionAlreadyExists {
                name: existing.name.as_str().to_owned(),
                state: existing.lifecycle_state.to_string(),
                sync: existing.sync_state.to_string(),
            },
        ));
    }

    if let Some(existing) = find_session_by_vm_name(conn, vm_name)? {
        return Err(AppError::Validation(ValidationError::VmAlreadyReserved {
            vm_name: existing.vm_name.as_str().to_owned(),
            owner: existing.name.as_str().to_owned(),
            state: existing.lifecycle_state.to_string(),
            sync: existing.sync_state.to_string(),
        }));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ensure_runtime_session_slot_available;
    use crate::db::connect::open_catalog;
    use crate::db::models::{SyncState, sync_state_name};
    use crate::db::sessions::{
        InsertSession, insert_session, update_lifecycle_state_with_timestamps,
    };
    use crate::testing::{test_repo_session, ts};
    use crate::types::{SessionName, Timestamp, VmName};
    use tempfile::tempdir;

    fn seed_session(conn: &rusqlite::Connection, session: &SessionName, vm_name: &VmName) {
        insert_session(
            conn,
            &InsertSession {
                vm_name: vm_name.clone(),
                ..test_repo_session(session.as_str(), ts("2026-04-20T00:00:00Z"))
            },
        )
        .expect("insert session");
    }

    #[test]
    fn allows_unused_session_slots() {
        let tmp = tempdir().expect("tempdir");
        let conn = open_catalog(&tmp.path().join("state.db")).expect("catalog");
        let session = SessionName::try_from("fresh").expect("session");
        let vm_name = VmName::for_session(&session);

        ensure_runtime_session_slot_available(&conn, &session, &vm_name).expect("slot available");
    }

    #[test]
    fn rejects_existing_session_name_with_clear_state() {
        let tmp = tempdir().expect("tempdir");
        let conn = open_catalog(&tmp.path().join("state.db")).expect("catalog");
        let session = SessionName::try_from("bleagle-3881-review").expect("session");
        let vm_name = VmName::for_session(&session);
        seed_session(&conn, &session, &vm_name);
        let closed_at = Timestamp::parse_rfc3339("2026-04-20T01:00:00Z").expect("timestamp");
        update_lifecycle_state_with_timestamps(
            &conn,
            &session,
            crate::db::models::LifecycleState::Closed,
            &closed_at,
            None,
            Some(&closed_at),
            Some(&closed_at),
        )
        .expect("close session");
        conn.execute(
            "UPDATE sessions SET sync_state = ?1 WHERE name = ?2",
            rusqlite::params![sync_state_name(SyncState::Discarded), &session],
        )
        .expect("update sync");

        let err = ensure_runtime_session_slot_available(&conn, &session, &vm_name)
            .expect_err("existing session should block reuse");
        assert!(
            err.to_string().contains(
                "session `bleagle-3881-review` already exists in the local catalog (state: closed, sync: discarded)"
            ),
            "unexpected message: {err}",
        );
    }

    #[test]
    fn rejects_existing_vm_name_with_clear_owner() {
        let tmp = tempdir().expect("tempdir");
        let conn = open_catalog(&tmp.path().join("state.db")).expect("catalog");
        let session = SessionName::try_from("other-session").expect("session");
        let reused_vm = VmName::new("agbranch-bleagle-3881-review");
        seed_session(&conn, &session, &reused_vm);

        let err = ensure_runtime_session_slot_available(
            &conn,
            &SessionName::try_from("bleagle-3881-review").expect("new session"),
            &reused_vm,
        )
        .expect_err("existing vm should block reuse");
        assert!(
            err.to_string().contains(
                "vm `agbranch-bleagle-3881-review` is already reserved by session `other-session`"
            ),
            "unexpected message: {err}",
        );
    }
}
