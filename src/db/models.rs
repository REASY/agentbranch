pub use crate::types::{AgentLaunchPreset, ProviderKind, RepoSyncMode, SessionMode, Timestamp};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleState {
    Running,
    Stopped,
    Error,
    Closed,
    PreparingBase,
    Cloning,
    Starting,
    Seeding,
    Syncing,
    Staging,
    Applying,
    Destroying,
    Repairing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncState {
    Pending,
    Clean,
    Blocked,
    Error,
    Discarded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncDirection {
    Seed,
    SyncBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncRunResult {
    Success,
    Blocked,
    Error,
}

impl LifecycleState {
    pub fn as_str(self) -> &'static str {
        match self {
            LifecycleState::Running => "running",
            LifecycleState::Stopped => "stopped",
            LifecycleState::Error => "error",
            LifecycleState::Closed => "closed",
            LifecycleState::PreparingBase => "preparing_base",
            LifecycleState::Cloning => "cloning",
            LifecycleState::Starting => "starting",
            LifecycleState::Seeding => "seeding",
            LifecycleState::Syncing => "syncing",
            LifecycleState::Staging => "staging",
            LifecycleState::Applying => "applying",
            LifecycleState::Destroying => "destroying",
            LifecycleState::Repairing => "repairing",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "running" => Self::Running,
            "stopped" => Self::Stopped,
            "error" => Self::Error,
            "closed" => Self::Closed,
            "preparing_base" => Self::PreparingBase,
            "cloning" => Self::Cloning,
            "starting" => Self::Starting,
            "seeding" => Self::Seeding,
            "syncing" => Self::Syncing,
            "staging" => Self::Staging,
            "applying" => Self::Applying,
            "destroying" => Self::Destroying,
            "repairing" => Self::Repairing,
            _ => return None,
        })
    }
}

impl fmt::Display for LifecycleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl SyncState {
    pub fn as_str(self) -> &'static str {
        match self {
            SyncState::Pending => "pending",
            SyncState::Clean => "clean",
            SyncState::Blocked => "blocked",
            SyncState::Error => "error",
            SyncState::Discarded => "discarded",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "clean" => Self::Clean,
            "blocked" => Self::Blocked,
            "error" => Self::Error,
            "discarded" => Self::Discarded,
            _ => return None,
        })
    }
}

impl fmt::Display for SyncState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl EventLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            EventLevel::Info => "info",
            EventLevel::Warn => "warn",
            EventLevel::Error => "error",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "info" => Self::Info,
            "warn" => Self::Warn,
            "error" => Self::Error,
            _ => return None,
        })
    }
}

impl SyncDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            SyncDirection::Seed => "seed",
            SyncDirection::SyncBack => "sync_back",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "seed" => Self::Seed,
            "sync_back" => Self::SyncBack,
            _ => return None,
        })
    }
}

impl SyncRunResult {
    pub fn as_str(self) -> &'static str {
        match self {
            SyncRunResult::Success => "success",
            SyncRunResult::Blocked => "blocked",
            SyncRunResult::Error => "error",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "success" => Self::Success,
            "blocked" => Self::Blocked,
            "error" => Self::Error,
            _ => return None,
        })
    }
}

impl fmt::Display for EventLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for SyncDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for SyncRunResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

macro_rules! impl_string_sql {
    ($ty:ty, $as_str:ident, $parse:ident) => {
        impl ToSql for $ty {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                self.$as_str().to_sql()
            }
        }

        impl FromSql for $ty {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                let raw = <String as FromSql>::column_result(value)?;
                <$ty>::$parse(&raw).ok_or_else(|| {
                    FromSqlError::Other(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("invalid {} value `{raw}`", stringify!($ty)),
                    )))
                })
            }
        }
    };
}

impl_string_sql!(LifecycleState, as_str, parse);
impl_string_sql!(SyncState, as_str, parse);
impl_string_sql!(EventLevel, as_str, parse);
impl_string_sql!(SyncDirection, as_str, parse);
impl_string_sql!(SyncRunResult, as_str, parse);

pub fn lifecycle_state_name(state: LifecycleState) -> &'static str {
    state.as_str()
}

pub fn sync_state_name(state: SyncState) -> &'static str {
    state.as_str()
}

#[allow(dead_code)]
pub fn timestamp_as_rfc3339(value: Timestamp) -> String {
    value.to_string()
}
