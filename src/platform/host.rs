use crate::error::AppError;
use crate::error::ValidationError;
use crate::platform::detect::HostPlatform;
use crate::platform::paths::StateRoots;
use crate::util::process::CommandRunner;
use semver::Version;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct HostContext {
    pub platform: HostPlatform,
    pub home_dir: PathBuf,
    pub xdg_state_home: Option<PathBuf>,
    pub state_roots: StateRoots,
}

impl HostContext {
    pub fn detect() -> Result<Self, ValidationError> {
        let platform = HostPlatform::current()?;
        let home_dir = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or(ValidationError::UnsupportedHost)?;
        let xdg_state_home = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from);
        let state_roots = std::env::var_os("AGBRANCH_STATE_ROOT")
            .map(PathBuf::from)
            .map(|path| StateRoots::from_base(&path))
            .unwrap_or_else(|| {
                StateRoots::from_parts(platform, &home_dir, xdg_state_home.as_deref())
            });

        Ok(Self {
            platform,
            home_dir,
            xdg_state_home,
            state_roots,
        })
    }
}

#[derive(Debug, Clone)]
pub struct HostPrereqs {
    pub platform: HostPlatform,
    pub lima_available: bool,
    pub lima_version: Option<Version>,
    pub qemu_available: bool,
    pub kvm_available: bool,
    pub macos_major: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct DoctorChecks {
    pub ok: bool,
    pub messages: Vec<String>,
}

impl DoctorChecks {
    pub fn from_prereqs(prereqs: HostPrereqs) -> Self {
        let mut messages = Vec::new();
        let mut ok = true;

        if !prereqs.lima_available {
            ok = false;
            messages.push("limactl is not available on PATH".to_owned());
        } else if prereqs
            .lima_version
            .as_ref()
            .is_none_or(|version| version < &Version::new(2, 1, 0))
        {
            ok = false;
            messages.push("Lima 2.1.0 or later is required".to_owned());
        }

        match prereqs.platform {
            HostPlatform::Macos => {
                if prereqs.macos_major.unwrap_or_default() < 13 {
                    ok = false;
                    messages.push("macOS 13 or later is required for vmType=vz".to_owned());
                }
            }
            HostPlatform::Linux => {
                if !prereqs.qemu_available {
                    ok = false;
                    messages.push("QEMU is not available on PATH".to_owned());
                }
                if !prereqs.kvm_available {
                    ok = false;
                    messages.push("/dev/kvm is not accessible to the current user".to_owned());
                }
            }
        }

        Self { ok, messages }
    }
}

pub fn collect_host_prereqs(runner: &dyn CommandRunner) -> Result<HostPrereqs, ValidationError> {
    let platform = HostPlatform::current()?;
    let env = BTreeMap::new();

    let lima_version_output = runner.run("limactl", &["--version".to_owned()], None, &env);

    let (lima_available, lima_version) = match lima_version_output {
        Ok(output) => (
            true,
            parse_semver_from_output(&format!("{}\n{}", output.stdout, output.stderr)),
        ),
        Err(_) => (false, None),
    };

    let qemu_available = match platform {
        HostPlatform::Macos => false,
        HostPlatform::Linux => {
            command_available(runner, "qemu-system-x86_64", &["--version".to_owned()])
                || command_available(runner, "qemu-system-aarch64", &["--version".to_owned()])
        }
    };

    let kvm_available = matches!(platform, HostPlatform::Linux) && is_kvm_accessible();
    let macos_major = if matches!(platform, HostPlatform::Macos) {
        read_macos_major_version(runner).ok()
    } else {
        None
    };

    Ok(HostPrereqs {
        platform,
        lima_available,
        lima_version,
        qemu_available,
        kvm_available,
        macos_major,
    })
}

fn command_available(runner: &dyn CommandRunner, program: &str, args: &[String]) -> bool {
    runner
        .run(program, args, None, &BTreeMap::new())
        .map(|_| true)
        .unwrap_or(false)
}

fn is_kvm_accessible() -> bool {
    let path = Path::new("/dev/kvm");
    path.exists()
        && std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .is_ok()
}

fn read_macos_major_version(runner: &dyn CommandRunner) -> Result<u32, AppError> {
    let output = runner.run(
        "sw_vers",
        &["-productVersion".to_owned()],
        None,
        &BTreeMap::new(),
    )?;
    let raw = output.stdout.trim();
    raw.split('.')
        .next()
        .ok_or(ValidationError::MacosVersionParse)?
        .parse::<u32>()
        .map_err(|_| ValidationError::MacosVersionParse.into())
}

fn parse_semver_from_output(output: &str) -> Option<Version> {
    output
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '-'))
        .find_map(|token| {
            let trimmed = token.trim_matches(|ch: char| !ch.is_ascii_digit());
            if trimmed.is_empty() {
                None
            } else {
                Version::parse(trimmed).ok()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_context_uses_agbranch_state_root_override() {
        temp_env::with_vars(
            [
                ("AGBRANCH_STATE_ROOT", Some("/tmp/agbranch-smoke-state")),
                ("HOME", Some("/Users/alice")),
                ("XDG_STATE_HOME", None),
            ],
            || {
                let context = HostContext::detect().expect("host context");
                assert_eq!(
                    context.state_roots.base,
                    Path::new("/tmp/agbranch-smoke-state")
                );
            },
        );
    }

    #[test]
    fn linux_doctor_requires_kvm() {
        let prereqs = HostPrereqs {
            platform: HostPlatform::Linux,
            lima_available: true,
            lima_version: Some(Version::new(2, 1, 0)),
            qemu_available: true,
            kvm_available: false,
            macos_major: None,
        };

        let report = DoctorChecks::from_prereqs(prereqs);
        assert!(!report.ok);
        assert!(report.messages.iter().any(|line| line.contains("/dev/kvm")));
    }

    #[test]
    fn doctor_rejects_lima_below_the_supported_floor() {
        let prereqs = HostPrereqs {
            platform: HostPlatform::Macos,
            lima_available: true,
            lima_version: Some(Version::new(2, 0, 3)),
            qemu_available: false,
            kvm_available: false,
            macos_major: Some(14),
        };

        let report = DoctorChecks::from_prereqs(prereqs);
        assert!(!report.ok);
        assert!(report.messages.iter().any(|line| line.contains("2.1.0")));
    }
}
