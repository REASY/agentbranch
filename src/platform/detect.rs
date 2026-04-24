use crate::error::ValidationError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPlatform {
    Macos,
    Linux,
}

impl HostPlatform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Linux => "linux",
        }
    }

    pub fn current() -> Result<Self, ValidationError> {
        if cfg!(target_os = "macos") {
            Ok(Self::Macos)
        } else if cfg!(target_os = "linux") {
            Ok(Self::Linux)
        } else {
            Err(ValidationError::UnsupportedHost)
        }
    }
}
