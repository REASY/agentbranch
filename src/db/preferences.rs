use crate::error::db::DbError;
use crate::types::ProviderKind;
use rusqlite::{Connection, OptionalExtension, params};

pub fn remembered_auth_import(
    conn: &Connection,
    provider: ProviderKind,
) -> Result<Option<bool>, DbError> {
    conn.query_row(
        "SELECT import_auth FROM provider_preferences WHERE provider = ?1",
        params![provider.as_str()],
        |row| row.get::<_, bool>(0),
    )
    .optional()
    .map_err(DbError::from)
}

pub fn remember_auth_import(
    conn: &Connection,
    provider: ProviderKind,
    import: bool,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO provider_preferences (provider, import_auth)
         VALUES (?1, ?2)
         ON CONFLICT(provider) DO UPDATE SET import_auth = excluded.import_auth",
        params![provider.as_str(), import],
    )?;
    Ok(())
}

pub fn forget_auth_import(conn: &Connection, provider: ProviderKind) -> Result<bool, DbError> {
    let deleted = conn.execute(
        "DELETE FROM provider_preferences WHERE provider = ?1",
        params![provider.as_str()],
    )?;
    Ok(deleted > 0)
}

pub fn clear_auth_imports(conn: &Connection) -> Result<usize, DbError> {
    conn.execute("DELETE FROM provider_preferences", [])
        .map_err(DbError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connect::open_catalog;
    use tempfile::tempdir;

    #[test]
    fn remembers_and_updates_auth_choice_per_provider() {
        let dir = tempdir().expect("tempdir");
        let conn = open_catalog(&dir.path().join("state.db")).expect("catalog");

        assert_eq!(
            remembered_auth_import(&conn, ProviderKind::Codex).expect("lookup"),
            None
        );
        remember_auth_import(&conn, ProviderKind::Codex, true).expect("remember import");
        remember_auth_import(&conn, ProviderKind::Claude, false).expect("remember none");
        assert_eq!(
            remembered_auth_import(&conn, ProviderKind::Codex).expect("lookup"),
            Some(true)
        );
        assert_eq!(
            remembered_auth_import(&conn, ProviderKind::Claude).expect("lookup"),
            Some(false)
        );

        remember_auth_import(&conn, ProviderKind::Codex, false).expect("update");
        assert_eq!(
            remembered_auth_import(&conn, ProviderKind::Codex).expect("lookup"),
            Some(false)
        );
    }

    #[test]
    fn forgets_one_or_all_auth_choices() {
        let dir = tempdir().expect("tempdir");
        let conn = open_catalog(&dir.path().join("state.db")).expect("catalog");
        remember_auth_import(&conn, ProviderKind::Codex, true).expect("codex");
        remember_auth_import(&conn, ProviderKind::Claude, false).expect("claude");

        assert!(forget_auth_import(&conn, ProviderKind::Codex).expect("forget codex"));
        assert!(!forget_auth_import(&conn, ProviderKind::Codex).expect("forget missing"));
        assert_eq!(
            remembered_auth_import(&conn, ProviderKind::Codex).expect("lookup codex"),
            None
        );
        assert_eq!(clear_auth_imports(&conn).expect("clear"), 1);
        assert_eq!(
            remembered_auth_import(&conn, ProviderKind::Claude).expect("lookup claude"),
            None
        );
    }
}
