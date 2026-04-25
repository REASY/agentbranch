use crate::lima::base_info::{
    BaseMetadata, format_rfc3339_seconds, metadata_path, write_metadata_atomic,
};
use crate::lima::fingerprint::CURRENT_PROVISION_FINGERPRINT;
use crate::lima::inspect::{LimaInstance, LimaInstanceStatus};
use crate::platform::detect::HostPlatform;
use crate::provider::registry::{provider_spec, supported_providers};
use crate::types::VmName;
use crate::util::ids::prepared_base_name;
use crate::util::process::CommandRunner;
use crate::util::time::utc_now;
use crate::{error::lima::LimaError, error::process::ProcessError, lima::instance};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedBaseReport {
    pub steps: Vec<&'static str>,
    pub agent_cli_versions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProvisionMarkers {
    pub system_done: bool,
    pub agent_clis_done: bool,
    pub docker_compose_done: bool,
}

pub fn prepare_steps(rebuild: bool) -> Vec<&'static str> {
    let mut steps = Vec::new();
    if rebuild {
        steps.push("unprotect");
        steps.push("delete");
    }
    steps.extend(["create", "start", "probe", "stop", "protect"]);
    steps
}

pub fn prepared_base_requires_rebuild(instance: &LimaInstance) -> bool {
    instance.has_host_mounts() || instance.has_deprecated_top_level_rosetta()
}

pub fn prepare_steps_for_existing(
    existing: Option<&LimaInstance>,
    rebuild: bool,
) -> Vec<&'static str> {
    match existing {
        None => prepare_steps(false),
        Some(instance) if rebuild || prepared_base_requires_rebuild(instance) => {
            prepare_steps(true)
        }
        Some(instance) => match instance.status {
            LimaInstanceStatus::Running => vec!["stop", "start", "probe", "stop", "protect"],
            LimaInstanceStatus::Stopped | LimaInstanceStatus::Other => {
                vec!["start", "probe", "stop", "protect"]
            }
        },
    }
}

pub fn safe_sync_template(platform: HostPlatform) -> PathBuf {
    let name = match platform {
        HostPlatform::Macos => "safe-sync-macos.yaml",
        HostPlatform::Linux => "safe-sync-linux.yaml",
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("lima")
        .join(name)
}

pub fn prepared_base_vm_name(platform: HostPlatform) -> VmName {
    prepared_base_name(platform)
}

pub fn prepare_base(
    runner: &dyn CommandRunner,
    platform: HostPlatform,
    rebuild: bool,
) -> Result<Vec<&'static str>, LimaError> {
    Ok(prepare_base_report_with_progress(runner, platform, rebuild, |_| {})?.steps)
}

pub fn prepare_base_with_progress<F>(
    runner: &dyn CommandRunner,
    platform: HostPlatform,
    rebuild: bool,
    on_step: F,
) -> Result<Vec<&'static str>, LimaError>
where
    F: FnMut(&'static str),
{
    prepare_base_with_progress_timeout(
        runner,
        platform,
        rebuild,
        Duration::from_secs(20 * 60),
        on_step,
    )
}

pub fn prepare_base_with_progress_timeout<F>(
    runner: &dyn CommandRunner,
    platform: HostPlatform,
    rebuild: bool,
    start_timeout: Duration,
    mut on_step: F,
) -> Result<Vec<&'static str>, LimaError>
where
    F: FnMut(&'static str),
{
    let base = prepared_base_name(platform);
    let template = safe_sync_template(platform);
    let instances = instance::list_instances(runner)?;
    let existing = instances.iter().find(|item| item.name == base.as_str());
    let steps = prepare_steps_for_existing(existing, rebuild);
    for step in &steps {
        on_step(step);
        match *step {
            "unprotect" => instance::unprotect_instance(runner, &base)?,
            "delete" => instance::delete_instance(runner, &base)?,
            "create" => instance::create_instance(runner, &base, &template)?,
            "start" => start_instance_for_prepare(runner, &base, start_timeout)?,
            "probe" => instance::probe_instance(runner, &base)?,
            "stop" => instance::stop_instance(runner, &base)?,
            "protect" => instance::protect_instance(runner, &base)?,
            _ => {}
        }
    }
    Ok(steps)
}

pub fn prepare_base_report_with_progress<F>(
    runner: &dyn CommandRunner,
    platform: HostPlatform,
    rebuild: bool,
    on_step: F,
) -> Result<PreparedBaseReport, LimaError>
where
    F: FnMut(&'static str),
{
    prepare_base_report_with_progress_timeout(
        runner,
        platform,
        rebuild,
        Duration::from_secs(20 * 60),
        on_step,
    )
}

pub fn prepare_base_report_with_progress_timeout<F>(
    runner: &dyn CommandRunner,
    platform: HostPlatform,
    rebuild: bool,
    start_timeout: Duration,
    mut on_step: F,
) -> Result<PreparedBaseReport, LimaError>
where
    F: FnMut(&'static str),
{
    let base = prepared_base_name(platform);
    let template = safe_sync_template(platform);
    let instances = instance::list_instances(runner)?;
    let existing = instances.iter().find(|item| item.name == base.as_str());
    let steps = prepare_report_steps_for_existing(existing, rebuild);
    let mut agent_cli_versions = BTreeMap::new();
    for step in &steps {
        on_step(step);
        match *step {
            "unprotect" => instance::unprotect_instance(runner, &base)?,
            "delete" => instance::delete_instance(runner, &base)?,
            "create" => instance::create_instance(runner, &base, &template)?,
            "start" => start_instance_for_prepare(runner, &base, start_timeout)?,
            "probe" => {
                instance::probe_instance(runner, &base)?;
                agent_cli_versions = collect_agent_cli_versions(runner, &base);
            }
            "metadata" => {
                if let Err(err) = write_base_metadata(runner, &base, &agent_cli_versions) {
                    let _ = instance::stop_instance(runner, &base);
                    return Err(err);
                }
            }
            "stop" => instance::stop_instance(runner, &base)?,
            "protect" => instance::protect_instance(runner, &base)?,
            _ => {}
        }
    }
    Ok(PreparedBaseReport {
        steps,
        agent_cli_versions,
    })
}

fn prepare_report_steps_for_existing(
    existing: Option<&LimaInstance>,
    rebuild: bool,
) -> Vec<&'static str> {
    let mut steps = prepare_steps_for_existing(existing, rebuild);
    if let Some(index) = steps.iter().position(|step| *step == "probe") {
        steps.insert(index + 1, "metadata");
    }
    steps
}

fn write_base_metadata(
    runner: &dyn CommandRunner,
    base: &VmName,
    agent_cli_versions: &BTreeMap<String, String>,
) -> Result<(), LimaError> {
    let instances = instance::list_instances(runner)?;
    let instance = instances
        .iter()
        .find(|instance| instance.name == base.as_str())
        .ok_or_else(|| LimaError::MissingPreparedBase(base.as_str().to_owned()))?;
    let instance_dir = PathBuf::from(&instance.instance_dir);
    let metadata = BaseMetadata {
        schema_version: 1,
        prepared_at: format_rfc3339_seconds(utc_now().as_offset_date_time()),
        provision_fingerprint: CURRENT_PROVISION_FINGERPRINT.to_owned(),
        agent_cli_versions: agent_cli_versions.clone(),
    };
    write_metadata_atomic(&instance_dir, &metadata).map_err(|source| LimaError::BaseMetadata {
        path: metadata_path(&instance_dir).display().to_string(),
        source,
    })
}

fn collect_agent_cli_versions(
    runner: &dyn CommandRunner,
    base: &VmName,
) -> BTreeMap<String, String> {
    let mut versions = BTreeMap::new();
    for kind in supported_providers() {
        let spec = provider_spec(kind);
        let mut args = vec![
            "shell".to_owned(),
            base.as_str().to_owned(),
            "--".to_owned(),
        ];
        args.push(spec.binary_name.to_owned());
        args.extend(spec.version_args.iter().map(|arg| (*arg).to_owned()));
        let version = match runner.run("limactl", &args, None, &BTreeMap::new()) {
            Ok(output) => {
                let rendered = if output.stdout.trim().is_empty() {
                    output.stderr.trim()
                } else {
                    output.stdout.trim()
                };
                first_output_line(rendered)
            }
            Err(_) => "unavailable".to_owned(),
        };
        versions.insert(kind.as_str().to_owned(), version);
    }
    versions
}

pub fn probe_provision_markers(
    runner: &dyn CommandRunner,
    base: &VmName,
) -> Option<ProvisionMarkers> {
    let command = r#"for pair in \
system:/var/lib/agbranch/provision/00-system.done \
agent_clis:/var/lib/agbranch/provision/10-agent-clis.done \
docker_compose:/var/lib/agbranch/provision/20-docker-compose.done; do
  key="${pair%%:*}"
  path="${pair#*:}"
  if [ -f "$path" ]; then
    printf '%s=1\n' "$key"
  else
    printf '%s=0\n' "$key"
  fi
done"#;
    let output = instance::shell_bash(runner, base, command).ok()?;
    parse_provision_markers(&output.stdout)
}

pub(crate) fn parse_provision_markers(output: &str) -> Option<ProvisionMarkers> {
    let mut markers = BTreeMap::new();
    for line in output.lines() {
        let (key, value) = line.split_once('=')?;
        markers.insert(key.trim(), value.trim() == "1");
    }
    Some(ProvisionMarkers {
        system_done: *markers.get("system")?,
        agent_clis_done: *markers.get("agent_clis")?,
        docker_compose_done: *markers.get("docker_compose")?,
    })
}

fn first_output_line(output: &str) -> String {
    output
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .unwrap_or_default()
        .to_owned()
}

fn start_instance_for_prepare(
    runner: &dyn CommandRunner,
    base: &VmName,
    timeout: Duration,
) -> Result<(), LimaError> {
    match instance::start_instance_with_timeout(runner, base, Some(timeout)) {
        Ok(()) => Ok(()),
        Err(err) => Err(enrich_prepare_start_error(runner, base, err)),
    }
}

fn enrich_prepare_start_error(
    runner: &dyn CommandRunner,
    base: &VmName,
    err: LimaError,
) -> LimaError {
    let LimaError::Process(ProcessError::Failed { program, .. }) = &err else {
        return err;
    };
    if program != "limactl" {
        return err;
    }

    let Some(cloud_init_output) = read_cloud_init_output(runner, base) else {
        return err;
    };
    let Some((script, detail)) = extract_agbranch_provision_failure(&cloud_init_output) else {
        return err;
    };
    LimaError::ProvisionFailed { script, detail }
}

fn read_cloud_init_output(runner: &dyn CommandRunner, base: &VmName) -> Option<String> {
    let args = vec![
        "shell".to_owned(),
        base.as_str().to_owned(),
        "--".to_owned(),
        "sudo".to_owned(),
        "bash".to_owned(),
        "-lc".to_owned(),
        "tail -n 400 /var/log/cloud-init-output.log".to_owned(),
    ];
    runner
        .run("limactl", &args, None, &BTreeMap::new())
        .ok()
        .map(|output| output.stdout)
}

fn extract_agbranch_provision_failure(cloud_init_output: &str) -> Option<(String, String)> {
    let mut block: Vec<&str> = Vec::new();
    let mut script_name: Option<&'static str> = None;
    let mut failure: Option<(String, String)> = None;

    for line in cloud_init_output.lines() {
        if line.contains("Executing /mnt/lima-cidata/provision.") {
            block.clear();
            script_name = None;
        }
        block.push(line);
        script_name = script_name.or_else(|| provision_script_name_from_line(line));
        if line.contains("WARNING: Failed to execute /mnt/lima-cidata/provision.")
            && let Some(failed_script_name) = script_name
        {
            let detail = summarize_provision_failure_detail(&block);
            failure = Some((failed_script_name.to_owned(), detail));
            block.clear();
            script_name = None;
        }
    }

    failure
}

fn provision_script_name_from_line(line: &str) -> Option<&'static str> {
    const MARKERS: [(&str, &str); 3] = [
        ("00-system.done", "00-system.sh"),
        ("05-network-guard.done", "05-network-guard.sh"),
        ("10-agent-clis.done", "10-agent-clis.sh"),
    ];

    MARKERS
        .iter()
        .find_map(|(needle, script)| line.contains(needle).then_some(*script))
        .or_else(|| {
            line.contains("20-docker-compose.done")
                .then_some("20-docker-compose.sh")
        })
}

fn summarize_provision_failure_detail(block: &[&str]) -> String {
    let mut candidates = block
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with('+'))
        .filter(|line| !line.starts_with("LIMA "))
        .filter(|line| !line.starts_with("Cloud-init "))
        .collect::<Vec<_>>();

    for needle in [
        "E: ",
        "Please install",
        "command failed",
        "Connection timed out",
        "failed to resolve",
        "Not found.",
    ] {
        if let Some(line) = candidates.iter().rev().find(|line| line.contains(needle)) {
            return (*line).to_owned();
        }
    }

    candidates
        .drain(candidates.len().saturating_sub(3)..)
        .collect::<Vec<_>>()
        .join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::process::ProcessError;
    use crate::lima::base_info::read_metadata;
    use crate::lima::inspect::{LimaConfig, LimaInstance, LimaInstanceStatus, LimaMount};
    use crate::platform::detect::HostPlatform;
    use crate::util::process::{CommandOutput, CommandRunner};
    use std::cell::RefCell;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn rebuild_prepare_flow_stops_and_protects_the_base() {
        let steps = prepare_steps(true);
        assert_eq!(
            steps,
            vec![
                "unprotect",
                "delete",
                "create",
                "start",
                "probe",
                "stop",
                "protect"
            ]
        );
    }

    #[test]
    fn rebuild_without_existing_base_skips_unprotect_and_delete() {
        let steps = prepare_steps_for_existing(None, true);
        assert_eq!(steps, vec!["create", "start", "probe", "stop", "protect"]);
    }

    fn instance(
        status: LimaInstanceStatus,
        mounts: Vec<LimaMount>,
        top_level_rosetta: bool,
    ) -> LimaInstance {
        LimaInstance {
            name: "agbranch-base-macos".to_owned(),
            instance_dir: "/tmp/base".to_owned(),
            ssh_config_file: "/tmp/base/ssh.config".to_owned(),
            vm_type: "vz".to_owned(),
            raw_status: match status {
                LimaInstanceStatus::Running => "Running",
                LimaInstanceStatus::Stopped => "Stopped",
                LimaInstanceStatus::Other => "Unknown",
            }
            .to_owned(),
            arch: None,
            cpus: None,
            memory: None,
            disk: None,
            protected: false,
            ssh_local_port: None,
            ssh_address: None,
            config: LimaConfig {
                mounts,
                rosetta: top_level_rosetta.then_some(serde_json::json!({"enabled": true})),
            },
            status,
        }
    }

    #[test]
    fn existing_stopped_base_is_started_probed_stopped_and_protected() {
        let steps = prepare_steps_for_existing(
            Some(&instance(LimaInstanceStatus::Stopped, vec![], false)),
            false,
        );
        assert_eq!(steps, vec!["start", "probe", "stop", "protect"]);
    }

    #[test]
    fn existing_running_base_is_restarted_before_probe_and_protect() {
        let steps = prepare_steps_for_existing(
            Some(&instance(LimaInstanceStatus::Running, vec![], false)),
            false,
        );
        assert_eq!(steps, vec!["stop", "start", "probe", "stop", "protect"]);
    }

    #[test]
    fn insecure_existing_base_is_rebuilt_even_without_rebuild_flag() {
        let existing = instance(
            LimaInstanceStatus::Stopped,
            vec![LimaMount {
                location: "/Users/tester".to_owned(),
            }],
            true,
        );
        assert!(prepared_base_requires_rebuild(&existing));
        let steps = prepare_steps_for_existing(Some(&existing), false);
        assert_eq!(
            steps,
            vec![
                "unprotect",
                "delete",
                "create",
                "start",
                "probe",
                "stop",
                "protect"
            ]
        );
    }

    #[derive(Default)]
    struct RecordingRunner {
        calls: RefCell<Vec<(String, Vec<String>)>>,
    }

    impl CommandRunner for RecordingRunner {
        fn run(
            &self,
            program: &str,
            args: &[String],
            _cwd: Option<&Path>,
            _env: &BTreeMap<String, String>,
        ) -> Result<CommandOutput, ProcessError> {
            self.calls
                .borrow_mut()
                .push((program.to_owned(), args.to_vec()));
            Ok(CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    struct VersionFailingRunner {
        calls: RefCell<Vec<(String, Vec<String>)>>,
        list_calls: RefCell<usize>,
        base_dir: TempDir,
    }

    impl Default for VersionFailingRunner {
        fn default() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                list_calls: RefCell::new(0),
                base_dir: tempfile::tempdir().expect("tempdir"),
            }
        }
    }

    impl CommandRunner for VersionFailingRunner {
        fn run(
            &self,
            program: &str,
            args: &[String],
            _cwd: Option<&Path>,
            _env: &BTreeMap<String, String>,
        ) -> Result<CommandOutput, ProcessError> {
            self.calls
                .borrow_mut()
                .push((program.to_owned(), args.to_vec()));
            if program == "limactl" && args == ["list", "--json"] {
                let mut list_calls = self.list_calls.borrow_mut();
                let stdout = if *list_calls == 0 {
                    "[]".to_owned()
                } else {
                    format!(
                        r#"[{{"name":"agbranch-base-macos","status":"Stopped","vmType":"vz","dir":"{}","sshConfigFile":"/tmp/base/ssh.config"}}]"#,
                        self.base_dir.path().display()
                    )
                };
                *list_calls += 1;
                return Ok(CommandOutput {
                    stdout,
                    stderr: String::new(),
                });
            }
            if program == "limactl"
                && args.first().map(String::as_str) == Some("shell")
                && args.iter().any(|arg| arg == "codex")
            {
                return Err(ProcessError::Failed {
                    program: program.to_owned(),
                    status: 1,
                    stderr: "missing codex".to_owned(),
                });
            }
            Ok(CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn prepare_reports_each_step_in_execution_order() {
        let runner = RecordingRunner::default();
        let mut seen_steps = Vec::new();

        let executed = prepare_base_with_progress(&runner, HostPlatform::Macos, false, |step| {
            seen_steps.push(step.to_owned());
        })
        .expect("prepare succeeds");

        assert_eq!(
            executed,
            vec!["create", "start", "probe", "stop", "protect"]
        );
        assert_eq!(seen_steps, executed);
    }

    #[test]
    fn legacy_prepare_path_does_not_probe_provider_versions() {
        let runner = RecordingRunner::default();

        let executed = prepare_base_with_progress(&runner, HostPlatform::Macos, false, |_| {})
            .expect("prepare succeeds");

        assert_eq!(
            executed,
            vec!["create", "start", "probe", "stop", "protect"]
        );

        let calls = runner.calls.borrow();
        let shell_calls = calls
            .iter()
            .filter(|(program, args)| {
                program == "limactl" && args.first().map(String::as_str) == Some("shell")
            })
            .count();
        assert_eq!(
            shell_calls, 1,
            "legacy prepare path should only run the readiness probe shell call"
        );
    }

    #[test]
    fn prepare_report_keeps_cleanup_steps_even_when_version_probe_fails() {
        let runner = VersionFailingRunner::default();

        let report = prepare_base_report_with_progress(&runner, HostPlatform::Macos, false, |_| {})
            .expect("prepare report succeeds");

        assert_eq!(
            report.steps,
            vec!["create", "start", "probe", "metadata", "stop", "protect"]
        );
        assert_eq!(
            report.agent_cli_versions.get("codex").map(String::as_str),
            Some("unavailable")
        );

        let calls = runner.calls.borrow();
        assert!(
            calls
                .iter()
                .any(|(_, args)| args.first().map(String::as_str) == Some("stop")),
            "prepare report should still stop the base after a failed version probe"
        );
        assert!(
            calls
                .iter()
                .any(|(_, args)| args.first().map(String::as_str) == Some("protect")),
            "prepare report should still protect the base after a failed version probe"
        );
        let metadata_path = runner.base_dir.path().join("agbranch-base.json");
        assert!(
            metadata_path.exists(),
            "prepare report should write metadata"
        );
        let (metadata, valid) = read_metadata(runner.base_dir.path()).expect("read metadata");
        assert!(valid, "freshly written metadata should be valid");
        let metadata = metadata.expect("metadata should exist");
        assert!(
            !metadata.prepared_at.contains('.'),
            "prepared_at should be second-precision RFC3339, got {}",
            metadata.prepared_at
        );
        assert_eq!(
            fs::read_to_string(metadata_path)
                .expect("metadata file")
                .matches("\"prepared_at\"")
                .count(),
            1,
            "metadata file should contain exactly one prepared_at field"
        );
    }

    #[test]
    fn extracts_system_provision_failure_with_package_error() {
        let log = r#"
LIMA 2026-04-20T18:15:23+08:00| Executing /mnt/lima-cidata/provision.system/00000004
+ MARKER_FILE=/var/lib/agbranch/provision/00-system.done
 apt-get install -y bash build-essential unzip
 E: Failed to fetch http://ports.ubuntu.com/gcc-13-aarch64-linux-gnu.deb  Connection timed out [IP: 91.189.91.104 80]
LIMA 2026-04-20T18:04:11+08:00| WARNING: Failed to execute /mnt/lima-cidata/provision.system/00000004
"#;

        let failure = extract_agbranch_provision_failure(log);
        assert_eq!(
            failure,
            Some((
                "00-system.sh".to_owned(),
                "E: Failed to fetch http://ports.ubuntu.com/gcc-13-aarch64-linux-gnu.deb  Connection timed out [IP: 91.189.91.104 80]".to_owned()
            ))
        );
    }

    #[test]
    fn extracts_agent_cli_provision_failure_with_package_detail() {
        let log = r#"
LIMA 2026-04-20T18:04:37+08:00| Executing /mnt/lima-cidata/provision.system/00000009
+ MARKER_FILE=/var/lib/agbranch/provision/10-agent-clis.done
 npm install -g @openai/codex @anthropic-ai/claude-code @google/gemini-cli
 command failed while refreshing NodeSource keyring
LIMA 2026-04-20T18:04:38+08:00| WARNING: Failed to execute /mnt/lima-cidata/provision.system/00000009
"#;

        let failure = extract_agbranch_provision_failure(log);
        assert_eq!(
            failure,
            Some((
                "10-agent-clis.sh".to_owned(),
                "command failed while refreshing NodeSource keyring".to_owned()
            ))
        );
    }

    #[test]
    fn parse_provision_markers_reads_shell_probe_output() {
        let markers = parse_provision_markers("system=1\nagent_clis=0\ndocker_compose=0\n")
            .expect("markers should parse");

        assert_eq!(
            markers,
            ProvisionMarkers {
                system_done: true,
                agent_clis_done: false,
                docker_compose_done: false,
            }
        );
    }

    #[test]
    fn parse_provision_markers_accepts_minimal_builtin_marker_set() {
        let markers = parse_provision_markers("system=1\nagent_clis=1\ndocker_compose=0\n")
            .expect("markers should parse");
        assert_eq!(
            markers,
            ProvisionMarkers {
                system_done: true,
                agent_clis_done: true,
                docker_compose_done: false,
            },
            "minimal built-in provisioning markers should parse without language toolchains"
        );
    }
}
