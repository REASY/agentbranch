use crate::cli::PrepareArgs;
use crate::error::AppError;
use crate::lima::base::{
    ProvisionMarkers, prepare_base_report_with_progress_timeout, probe_provision_markers,
};
use crate::lima::instance;
use crate::platform::host::HostContext;
use crate::types::{Timestamp, VmName};
use crate::util::ids::prepared_base_name;
use crate::util::process::RealCommandRunner;
use crate::util::time::utc_now;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub fn render_prepare_json(
    prepared_base: &str,
    started_at: Timestamp,
    prepared_at: Timestamp,
    duration_ms: u64,
    steps: &[&str],
) -> Result<String, serde_json::Error> {
    render_prepare_json_with_versions(
        prepared_base,
        started_at,
        prepared_at,
        duration_ms,
        steps,
        &[],
    )
}

pub fn render_prepare_json_with_versions(
    prepared_base: &str,
    started_at: Timestamp,
    prepared_at: Timestamp,
    duration_ms: u64,
    steps: &[&str],
    agent_cli_versions: &[(&str, &str)],
) -> Result<String, serde_json::Error> {
    let versions = agent_cli_versions
        .iter()
        .map(|(name, version)| ((*name).to_owned(), (*version).to_owned()))
        .collect::<BTreeMap<_, _>>();
    serde_json::to_string(&serde_json::json!({
        "prepared_base": prepared_base,
        "started_at": started_at.to_string(),
        "prepared_at": prepared_at.to_string(),
        "duration_ms": duration_ms,
        "steps": steps,
        "agent_cli_versions": versions,
    }))
}

pub fn run(args: PrepareArgs) -> Result<(), AppError> {
    let host = HostContext::detect()?;
    let prepared_base = prepared_base_name(host.platform);
    let started_at = utc_now();
    let start = Instant::now();
    let mut progress = PrepareProgressLogger::new(prepared_base.as_str(), &host.home_dir, start);
    let report = prepare_base_report_with_progress_timeout(
        &RealCommandRunner,
        host.platform,
        args.rebuild,
        args.timeout,
        |step| {
            progress.on_step(step);
        },
    )?;
    progress.finish();
    let prepared_at = utc_now();
    let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    if args.json {
        println!(
            "{}",
            render_prepare_json_with_versions(
                prepared_base.as_str(),
                started_at,
                prepared_at,
                duration_ms,
                &report.steps,
                &report
                    .agent_cli_versions
                    .iter()
                    .map(|(name, version)| (name.as_str(), version.as_str()))
                    .collect::<Vec<_>>(),
            )
            .map_err(crate::error::observability::ObservabilityError::Json)?
        );
    } else {
        println!(
            "prepared {} via {} in {}ms",
            prepared_base,
            report.steps.join(" -> "),
            duration_ms
        );
    }
    Ok(())
}

fn prepare_log_path(home_dir: &Path, prepared_base: &str) -> PathBuf {
    home_dir
        .join(".lima")
        .join(prepared_base)
        .join("ha.stderr.log")
}

fn infer_prepare_start_phase(
    contents: &str,
    markers: Option<ProvisionMarkers>,
) -> Option<&'static str> {
    if let Some(markers) = markers {
        if !markers.system_done {
            return Some("system/bootstrap");
        }
        if !markers.agent_clis_done {
            return Some("agent-clis");
        }
        if !markers.docker_compose_done {
            return Some("docker");
        }
        return Some("readiness");
    }

    infer_prepare_start_phase_from_ha_stderr(contents)
}

fn infer_prepare_start_phase_from_ha_stderr(contents: &str) -> Option<&'static str> {
    const MARKERS: [(&str, &str); 6] = [
        ("Waiting for port to become available", "boot"),
        ("Waiting for the essential requirement", "boot"),
        ("Waiting for the optional requirement 1 of 2", "docker"),
        ("process.versions.node", "agent-clis"),
        ("codex --version", "agent-clis"),
        ("docker is not installed yet", "docker"),
    ];
    let mut best: Option<(usize, &'static str)> = None;
    for (needle, phase) in MARKERS {
        if let Some(index) = contents.rfind(needle)
            && best.is_none_or(|(best_index, _)| index > best_index)
        {
            best = Some((index, phase));
        }
    }
    for needle in ["claude --version", "gemini --version"] {
        if let Some(index) = contents.rfind(needle)
            && best.is_none_or(|(best_index, _)| index > best_index)
        {
            best = Some((index, "agent-clis"));
        }
    }
    if let Some(index) = contents.rfind("docker compose version")
        && best.is_none_or(|(best_index, _)| index > best_index)
    {
        best = Some((index, "docker"));
    }
    best.map(|(_, phase)| phase)
}

fn format_elapsed(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours}h{minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m{seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

struct StartPhaseWatcher {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl StartPhaseWatcher {
    fn start(prepared_base: &str, log_path: PathBuf, started_at: Instant) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let prepared_base = prepared_base.to_owned();
        let prepared_base_vm = VmName::new(prepared_base.clone());
        let log_offset = fs::metadata(&log_path)
            .ok()
            .and_then(|metadata| usize::try_from(metadata.len()).ok())
            .unwrap_or(0);
        let handle = thread::spawn(move || {
            let mut last_phase = Some("boot");
            eprintln!(
                "prepare {}: start (boot, {})",
                prepared_base,
                format_elapsed(started_at.elapsed())
            );
            while !stop_flag.load(Ordering::Relaxed) {
                let markers = instance::list_instances(&RealCommandRunner)
                    .ok()
                    .and_then(|instances| {
                        instances
                            .iter()
                            .find(|i| i.name == prepared_base_vm.as_str())
                            .cloned()
                    })
                    .and_then(|instance| {
                        if instance.is_running() {
                            probe_provision_markers(&RealCommandRunner, &prepared_base_vm)
                        } else {
                            None
                        }
                    });

                if let Ok(contents) = fs::read_to_string(&log_path)
                    && let Some(phase) = infer_prepare_start_phase(
                        prepare_log_since_offset(&contents, log_offset),
                        markers,
                    )
                    && last_phase != Some(phase)
                {
                    eprintln!(
                        "prepare {}: start ({}, {})",
                        prepared_base,
                        phase,
                        format_elapsed(started_at.elapsed())
                    );
                    last_phase = Some(phase);
                }
                thread::sleep(std::time::Duration::from_secs(1));
            }
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn prepare_log_since_offset(contents: &str, offset: usize) -> &str {
    if offset >= contents.len() {
        ""
    } else {
        &contents[offset..]
    }
}

impl Drop for StartPhaseWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

struct PrepareProgressLogger {
    prepared_base: String,
    log_path: PathBuf,
    started_at: Instant,
    start_watcher: Option<StartPhaseWatcher>,
}

impl PrepareProgressLogger {
    fn new(prepared_base: &str, home_dir: &Path, started_at: Instant) -> Self {
        Self {
            prepared_base: prepared_base.to_owned(),
            log_path: prepare_log_path(home_dir, prepared_base),
            started_at,
            start_watcher: None,
        }
    }

    fn on_step(&mut self, step: &'static str) {
        if step != "start" {
            self.stop_start_watcher();
        }
        eprintln!("prepare {}: {}", self.prepared_base, step);
        if step == "start" {
            self.stop_start_watcher();
            self.start_watcher = Some(StartPhaseWatcher::start(
                &self.prepared_base,
                self.log_path.clone(),
                self.started_at,
            ));
        }
    }

    fn finish(&mut self) {
        self.stop_start_watcher();
    }

    fn stop_start_watcher(&mut self) {
        if let Some(mut watcher) = self.start_watcher.take() {
            watcher.stop();
        }
    }
}

impl Drop for PrepareProgressLogger {
    fn drop(&mut self) {
        self.stop_start_watcher();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lima::base::ProvisionMarkers;
    use std::time::Duration;

    #[test]
    fn prepare_json_contains_agent_cli_versions() {
        let started = Timestamp::parse_rfc3339("2026-04-18T00:00:00Z").expect("timestamp");
        let prepared = Timestamp::parse_rfc3339("2026-04-18T00:01:00Z").expect("timestamp");
        let rendered = render_prepare_json_with_versions(
            "agbranch-base-macos",
            started,
            prepared,
            60_000,
            &["create", "start", "probe", "stop", "protect"],
            &[("codex", "1.2.3"), ("claude", "0.9.0"), ("gemini", "2.0.1")],
        )
        .expect("json");

        let value: serde_json::Value = serde_json::from_str(&rendered).expect("valid json");
        assert_eq!(value["agent_cli_versions"]["codex"], "1.2.3");
        assert_eq!(value["agent_cli_versions"]["claude"], "0.9.0");
        assert_eq!(value["agent_cli_versions"]["gemini"], "2.0.1");
    }

    #[test]
    fn infer_prepare_phase_maps_known_readiness_checks() {
        assert_eq!(
            infer_prepare_start_phase_from_ha_stderr(
                "{\"msg\":\"Waiting for port to become available on 192.168.5.15:22\"}\n"
            ),
            Some("boot")
        );
        assert_eq!(
            infer_prepare_start_phase_from_ha_stderr(
                "{\"msg\":\"Waiting for the optional requirement 1 of 2: \\\"user probe 1/2\\\"\"}\n"
            ),
            Some("docker")
        );
        assert_eq!(
            infer_prepare_start_phase_from_ha_stderr(
                "+ node -e 'process.exit(Number(process.versions.node.split(\".\")[0]) >= 20 ? 0 : 1)'\n"
            ),
            Some("agent-clis")
        );
        assert_eq!(
            infer_prepare_start_phase_from_ha_stderr("+ docker compose version\n"),
            Some("docker")
        );
    }

    #[test]
    fn infer_prepare_phase_ignores_removed_language_toolchain_checks() {
        assert_eq!(
            infer_prepare_start_phase_from_ha_stderr("+ test -x /home/demo/.cargo/bin/cargo\n"),
            None
        );
        assert_eq!(
            infer_prepare_start_phase_from_ha_stderr("+ test -x /home/demo/.local/bin/uv\n"),
            None
        );
        assert_eq!(
            infer_prepare_start_phase_from_ha_stderr(
                "+ test -f /home/demo/.agbranch/sdkman-jdks.env\n"
            ),
            None
        );
    }

    #[test]
    fn infer_prepare_phase_reports_system_bootstrap_before_agent_cli_marker_exists() {
        let log = "+ codex --version\n";
        let markers = ProvisionMarkers {
            system_done: false,
            agent_clis_done: false,
            docker_compose_done: false,
        };

        assert_eq!(
            infer_prepare_start_phase(log, Some(markers)),
            Some("system/bootstrap")
        );
    }

    #[test]
    fn infer_prepare_phase_advances_with_completed_markers() {
        let log = "+ docker compose version\n";
        let markers = ProvisionMarkers {
            system_done: true,
            agent_clis_done: true,
            docker_compose_done: false,
        };

        assert_eq!(
            infer_prepare_start_phase(log, Some(markers)),
            Some("docker")
        );
    }

    #[test]
    fn infer_prepare_phase_prefers_latest_probe_failure() {
        let log = "\
Waiting for the optional requirement 1 of 2
+ codex --version
 docker compose version";
        assert_eq!(
            infer_prepare_start_phase_from_ha_stderr(log),
            Some("docker")
        );
    }

    #[test]
    fn format_elapsed_renders_compact_human_duration() {
        assert_eq!(format_elapsed(Duration::from_secs(59)), "59s");
        assert_eq!(format_elapsed(Duration::from_secs(65)), "1m05s");
        assert_eq!(format_elapsed(Duration::from_secs(3660)), "1h01m");
    }

    #[test]
    fn prepare_log_since_offset_ignores_stale_prefix() {
        let contents =
            "old line\n+ codex --version\nnew line\nWaiting for the essential requirement\n";
        let offset = contents.find("new line").expect("offset");

        assert_eq!(
            prepare_log_since_offset(contents, offset),
            "new line\nWaiting for the essential requirement\n"
        );
        assert_eq!(
            infer_prepare_start_phase_from_ha_stderr(prepare_log_since_offset(contents, offset)),
            Some("boot")
        );
    }

    #[test]
    fn prepare_json_uses_rfc3339_timestamps_and_duration() {
        let started_at =
            Timestamp::parse_rfc3339("2026-04-15T16:14:20Z").expect("valid started timestamp");
        let prepared_at =
            Timestamp::parse_rfc3339("2026-04-15T16:14:28Z").expect("valid prepared timestamp");
        let rendered = render_prepare_json(
            "agbranch-base-macos",
            started_at,
            prepared_at,
            8_519,
            &["create", "start", "probe", "stop", "protect"],
        )
        .expect("prepare json");

        let value: serde_json::Value = serde_json::from_str(&rendered).expect("valid json");
        assert_eq!(value["prepared_base"], "agbranch-base-macos");
        assert_eq!(value["started_at"], "2026-04-15T16:14:20Z");
        assert_eq!(value["prepared_at"], "2026-04-15T16:14:28Z");
        assert_eq!(value["duration_ms"], 8_519);
        assert_eq!(
            value["steps"],
            serde_json::json!(["create", "start", "probe", "stop", "protect"])
        );
    }
}
