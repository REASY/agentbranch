use crate::error::db::DbError;
use crate::ports::{PortProtocol, PublishedPort};
use crate::types::SessionName;
use rusqlite::{Connection, params};

pub fn insert_session_ports(
    conn: &Connection,
    session: &SessionName,
    ports: &[PublishedPort],
) -> Result<(), DbError> {
    for port in ports {
        conn.execute(
            "INSERT INTO session_ports (session_name, host_port, guest_port, protocol)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                session,
                port.host_port,
                port.guest_port,
                port.protocol.as_str()
            ],
        )?;
    }
    Ok(())
}

pub fn list_session_ports(
    conn: &Connection,
    session: &SessionName,
) -> Result<Vec<PublishedPort>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT host_port, guest_port, protocol
         FROM session_ports
         WHERE session_name = ?1
         ORDER BY host_port, protocol",
    )?;
    let rows = stmt.query_map(params![session], |row| {
        let protocol: String = row.get(2)?;
        let protocol = PortProtocol::parse(&protocol).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                format!("invalid port protocol `{protocol}`").into(),
            )
        })?;
        Ok(PublishedPort {
            host_port: row.get(0)?,
            guest_port: row.get(1)?,
            protocol,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connect::open_catalog;
    use crate::db::sessions::{InsertSession, insert_session};
    use tempfile::tempdir;

    #[test]
    fn inserts_lists_and_cascades_session_ports() {
        let dir = tempdir().expect("tempdir");
        let conn = open_catalog(&dir.path().join("state.db")).expect("catalog");
        let session = SessionName::try_from("ports-demo").expect("session");
        insert_session(
            &conn,
            &InsertSession {
                name: session.clone(),
                ..crate::testing::test_sandbox_session(
                    "ports-demo",
                    crate::testing::ts("2026-07-23T00:00:00Z"),
                )
            },
        )
        .expect("session");
        let ports = vec![
            "3000".parse::<PublishedPort>().expect("port"),
            "8080:80".parse::<PublishedPort>().expect("port"),
        ];
        insert_session_ports(&conn, &session, &ports).expect("insert");
        assert_eq!(list_session_ports(&conn, &session).expect("list"), ports);

        conn.execute("DELETE FROM sessions WHERE name = ?1", params![session])
            .expect("delete session");
        assert!(
            list_session_ports(&conn, &session)
                .expect("list after delete")
                .is_empty()
        );
    }
}
