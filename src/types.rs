use crate::error::ValidationError;
use byte_unit::Byte;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, Value, ValueRef};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionName(String);

impl SessionName {
    pub const MAX_LEN: usize = 48;

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SessionName {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() > Self::MAX_LEN {
            return Err(ValidationError::InvalidSessionName {
                name: value,
                max: Self::MAX_LEN,
            });
        }
        Ok(Self(value))
    }
}

impl TryFrom<&str> for SessionName {
    type Error = ValidationError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_owned())
    }
}

impl fmt::Display for SessionName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl ToSql for SessionName {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl FromSql for SessionName {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let raw = <String as FromSql>::column_result(value)?;
        SessionName::try_from(raw).map_err(|err| FromSqlError::Other(Box::new(err)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VmName(String);

impl VmName {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn for_session(session: &SessionName) -> Self {
        Self(format!("agbranch-{}", session.as_str()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VmName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl ToSql for VmName {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl FromSql for VmName {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        Ok(Self(<String as FromSql>::column_result(value)?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HostPath(PathBuf);

impl HostPath {
    pub fn new(value: impl Into<PathBuf>) -> Self {
        Self(value.into())
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for HostPath {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl fmt::Display for HostPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.display().fmt(f)
    }
}

impl ToSql for HostPath {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Owned(Value::Text(
            self.0.display().to_string(),
        )))
    }
}

impl FromSql for HostPath {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        Ok(Self(PathBuf::from(<String as FromSql>::column_result(
            value,
        )?)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GuestPath(PathBuf);

impl GuestPath {
    pub fn new(value: impl Into<PathBuf>) -> Self {
        Self(value.into())
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for GuestPath {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl fmt::Display for GuestPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.display().fmt(f)
    }
}

impl ToSql for GuestPath {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Owned(Value::Text(
            self.0.display().to_string(),
        )))
    }
}

impl FromSql for GuestPath {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        Ok(Self(PathBuf::from(<String as FromSql>::column_result(
            value,
        )?)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(OffsetDateTime);

impl Timestamp {
    pub fn now_utc() -> Self {
        Self(OffsetDateTime::now_utc())
    }

    pub fn parse_rfc3339(value: &str) -> Result<Self, time::error::Parse> {
        Ok(Self(OffsetDateTime::parse(value, &Rfc3339)?))
    }

    pub fn as_rfc3339(&self) -> String {
        self.0
            .format(&Rfc3339)
            .expect("RFC3339 formatting should succeed for valid timestamps")
    }

    pub fn as_offset_date_time(&self) -> OffsetDateTime {
        self.0
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_rfc3339())
    }
}

impl ToSql for Timestamp {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Owned(Value::Text(self.as_rfc3339())))
    }
}

impl FromSql for Timestamp {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let raw = <String as FromSql>::column_result(value)?;
        Timestamp::parse_rfc3339(&raw).map_err(|err| FromSqlError::Other(Box::new(err)))
    }
}

impl Serialize for Timestamp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.as_rfc3339())
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Timestamp::parse_rfc3339(&raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MemorySize(Byte);

impl MemorySize {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        parse_size(value, SizeKind::Memory).map(Self)
    }

    pub fn as_bytes(&self) -> u64 {
        self.0.as_u64()
    }

    pub fn to_lima_gib_arg(&self) -> String {
        format_gib_argument(self.0)
    }
}

impl fmt::Display for MemorySize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_lima_gib_arg())
    }
}

impl FromStr for MemorySize {
    type Err = ValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DiskSize(Byte);

impl DiskSize {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        parse_size(value, SizeKind::Disk).map(Self)
    }

    pub fn as_bytes(&self) -> u64 {
        self.0.as_u64()
    }

    pub fn to_lima_gib_arg(&self) -> String {
        format_gib_argument(self.0)
    }
}

impl fmt::Display for DiskSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_lima_gib_arg())
    }
}

impl FromStr for DiskSize {
    type Err = ValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[derive(Debug, Clone, Copy)]
enum SizeKind {
    Memory,
    Disk,
}

fn parse_size(value: &str, kind: SizeKind) -> Result<Byte, ValidationError> {
    let parsed = Byte::parse_str(value, true).map_err(|err| match kind {
        SizeKind::Memory => ValidationError::InvalidMemorySize {
            value: value.to_owned(),
            reason: err.to_string(),
        },
        SizeKind::Disk => ValidationError::InvalidDiskSize {
            value: value.to_owned(),
            reason: err.to_string(),
        },
    })?;
    if parsed.as_u64() == 0 {
        return Err(match kind {
            SizeKind::Memory => ValidationError::InvalidMemorySize {
                value: value.to_owned(),
                reason: "value must be greater than zero".to_owned(),
            },
            SizeKind::Disk => ValidationError::InvalidDiskSize {
                value: value.to_owned(),
                reason: "value must be greater than zero".to_owned(),
            },
        });
    }
    Ok(parsed)
}

fn format_gib_argument(value: Byte) -> String {
    const GIB_BYTES: f64 = 1024.0 * 1024.0 * 1024.0;

    let gib = value.as_u64() as f64 / GIB_BYTES;
    let mut rendered = format!("{gib:.9}");
    while rendered.contains('.') && rendered.ends_with('0') {
        rendered.pop();
    }
    if rendered.ends_with('.') {
        rendered.pop();
    }
    rendered
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionMode {
    Sandbox,
    Repo,
}

impl SessionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Repo => "repo",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "sandbox" => Self::Sandbox,
            "repo" => Self::Repo,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepoSyncMode {
    GitNative,
}

impl RepoSyncMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GitNative => "git_native",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "git_native" => Self::GitNative,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderKind {
    Codex,
    Claude,
    Gemini,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Gemini => "gemini",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "codex" => Self::Codex,
            "claude" => Self::Claude,
            "gemini" => Self::Gemini,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentLaunchPreset {
    Unrestricted,
}

impl AgentLaunchPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unrestricted => "unrestricted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "unrestricted" => Self::Unrestricted,
            _ => return None,
        })
    }
}

macro_rules! impl_string_sql_enum {
    ($ty:ty, $parse:ident, $as_str:ident) => {
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

impl fmt::Display for SessionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for RepoSyncMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for AgentLaunchPreset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl_string_sql_enum!(SessionMode, parse, as_str);
impl_string_sql_enum!(RepoSyncMode, parse, as_str);
impl_string_sql_enum!(ProviderKind, parse, as_str);
impl_string_sql_enum!(AgentLaunchPreset, parse, as_str);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_name_rejects_values_longer_than_forty_eight_chars() {
        let too_long = "a".repeat(49);
        let err = SessionName::try_from(too_long.clone()).expect_err("must reject > 48 chars");
        assert!(matches!(
            err,
            ValidationError::InvalidSessionName { max: 48, .. }
        ));
        assert!(err.to_string().contains(&too_long));
    }
}
