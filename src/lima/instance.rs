use crate::error::lima::LimaError;
use crate::lima::inspect::{LimaInstance, parse_instances};
use crate::types::{DiskSize, MemorySize, VmName};
use crate::util::process::{CommandOutput, CommandRunner};
use std::collections::BTreeMap;
use std::path::Path;
use std::thread;
use std::time::Duration;

pub fn list_instances(runner: &dyn CommandRunner) -> Result<Vec<LimaInstance>, LimaError> {
    let output = runner.run(
        "limactl",
        &["list".to_owned(), "--json".to_owned()],
        None,
        &BTreeMap::new(),
    )?;
    parse_instances(&output.stdout)
}

pub fn create_instance(
    runner: &dyn CommandRunner,
    name: &VmName,
    template: &Path,
) -> Result<(), LimaError> {
    let args = build_create_args(name, template);
    runner.run("limactl", &args, None, &BTreeMap::new())?;
    Ok(())
}

pub fn build_create_args(name: &VmName, template: &Path) -> Vec<String> {
    vec![
        "create".to_owned(),
        "--mount-none".to_owned(),
        "--name".to_owned(),
        name.as_str().to_owned(),
        template.display().to_string(),
    ]
}

pub fn start_instance(runner: &dyn CommandRunner, name: &VmName) -> Result<(), LimaError> {
    start_instance_with_timeout(runner, name, None)
}

pub fn start_instance_with_timeout(
    runner: &dyn CommandRunner,
    name: &VmName,
    timeout: Option<Duration>,
) -> Result<(), LimaError> {
    let args = build_start_args(name, timeout);
    match run_simple(runner, &args) {
        Ok(()) => Ok(()),
        Err(err) if should_retry_early_start_failure(&err) => {
            let _ = stop_instance(runner, name);
            thread::sleep(Duration::from_secs(1));
            run_simple(runner, &args)
        }
        Err(err) => Err(err),
    }
}

pub fn stop_instance(runner: &dyn CommandRunner, name: &VmName) -> Result<(), LimaError> {
    run_simple(runner, &["stop".to_owned(), name.as_str().to_owned()])
}

pub fn delete_instance(runner: &dyn CommandRunner, name: &VmName) -> Result<(), LimaError> {
    run_simple(
        runner,
        &[
            "delete".to_owned(),
            "--force".to_owned(),
            name.as_str().to_owned(),
        ],
    )
}

pub fn protect_instance(runner: &dyn CommandRunner, name: &VmName) -> Result<(), LimaError> {
    run_simple(runner, &["protect".to_owned(), name.as_str().to_owned()])
}

pub fn unprotect_instance(runner: &dyn CommandRunner, name: &VmName) -> Result<(), LimaError> {
    run_simple(runner, &["unprotect".to_owned(), name.as_str().to_owned()])
}

pub fn probe_instance(runner: &dyn CommandRunner, name: &VmName) -> Result<(), LimaError> {
    run_simple(
        runner,
        &[
            "shell".to_owned(),
            name.as_str().to_owned(),
            "--".to_owned(),
            "true".to_owned(),
        ],
    )
}

pub fn shell_bash(
    runner: &dyn CommandRunner,
    name: &VmName,
    command: &str,
) -> Result<CommandOutput, LimaError> {
    let args = vec![
        "shell".to_owned(),
        name.as_str().to_owned(),
        "--".to_owned(),
        "bash".to_owned(),
        "-lc".to_owned(),
        command.to_owned(),
    ];
    runner
        .run("limactl", &args, None, &BTreeMap::new())
        .map_err(LimaError::from)
}

pub fn clone_instance(
    runner: &dyn CommandRunner,
    source: &VmName,
    target: &VmName,
    cpus: Option<u16>,
    memory: Option<&MemorySize>,
    disk: Option<&DiskSize>,
) -> Result<(), LimaError> {
    let args = build_clone_args(source, target, cpus, memory, disk);
    runner.run("limactl", &args, None, &BTreeMap::new())?;
    Ok(())
}

pub fn build_clone_args(
    source: &VmName,
    target: &VmName,
    cpus: Option<u16>,
    memory: Option<&MemorySize>,
    disk: Option<&DiskSize>,
) -> Vec<String> {
    let mut args = vec![
        "clone".to_owned(),
        "--mount-none".to_owned(),
        source.as_str().to_owned(),
        target.as_str().to_owned(),
    ];
    if let Some(cpus) = cpus {
        args.push("--cpus".to_owned());
        args.push(cpus.to_string());
    }
    if let Some(memory) = memory {
        args.push("--memory".to_owned());
        args.push(memory.to_lima_gib_arg());
    }
    if let Some(disk) = disk {
        args.push("--disk".to_owned());
        args.push(disk.to_lima_gib_arg());
    }
    args
}

pub fn build_start_args(name: &VmName, timeout: Option<Duration>) -> Vec<String> {
    let mut args = vec!["start".to_owned()];
    if let Some(timeout) = timeout {
        args.push("--timeout".to_owned());
        args.push(format_lima_timeout(timeout));
    }
    args.push(name.as_str().to_owned());
    args
}

fn run_simple(runner: &dyn CommandRunner, args: &[String]) -> Result<(), LimaError> {
    runner.run("limactl", args, None, &BTreeMap::new())?;
    Ok(())
}

fn format_lima_timeout(duration: Duration) -> String {
    let total_secs = duration.as_secs().max(1);
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    let mut rendered = String::new();
    if hours > 0 {
        rendered.push_str(&format!("{hours}h"));
    }
    if minutes > 0 {
        rendered.push_str(&format!("{minutes}m"));
    }
    if seconds > 0 || rendered.is_empty() {
        rendered.push_str(&format!("{seconds}s"));
    }
    rendered
}

fn should_retry_early_start_failure(err: &LimaError) -> bool {
    match err {
        LimaError::Process(crate::error::process::ProcessError::Failed { program, stderr, .. })
            if program == "limactl" =>
        {
            stderr.contains(
                r#"level=fatal msg="exiting, status={Running:false Degraded:false Exiting:true Errors:[]"#,
            )
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::lima::LimaError;
    use crate::error::process::ProcessError;
    use crate::types::{DiskSize, MemorySize, VmName};
    use std::path::Path;
    use std::time::Duration;

    #[test]
    fn clone_args_keep_source_target_and_resource_sizes_typed() {
        let source = VmName::new("agbranch-base-macos");
        let target = VmName::new("agbranch-feat-a");
        let memory = MemorySize::parse("8GiB").expect("memory should parse");
        let disk = DiskSize::parse("50GiB").expect("disk should parse");

        let args = build_clone_args(&source, &target, Some(8), Some(&memory), Some(&disk));

        assert_eq!(
            args,
            vec![
                "clone",
                "--mount-none",
                "agbranch-base-macos",
                "agbranch-feat-a",
                "--cpus",
                "8",
                "--memory",
                "8",
                "--disk",
                "50",
            ]
        );
    }

    #[test]
    fn create_args_request_mountless_instances() {
        let name = VmName::new("agbranch-base-macos");
        let args = build_create_args(&name, Path::new("/tmp/safe-sync-macos.yaml"));

        assert_eq!(
            args,
            vec![
                "create",
                "--mount-none",
                "--name",
                "agbranch-base-macos",
                "/tmp/safe-sync-macos.yaml",
            ]
        );
    }

    #[test]
    fn start_args_propagate_timeout_when_present() {
        let name = VmName::new("agbranch-base-macos");
        let args = build_start_args(&name, Some(Duration::from_secs(20 * 60)));

        assert_eq!(
            args,
            vec!["start", "--timeout", "20m", "agbranch-base-macos"]
        );
    }

    #[test]
    fn retries_immediate_vz_exit_before_boot() {
        let err = LimaError::Process(ProcessError::Failed {
            program: "limactl".to_owned(),
            status: 1,
            stderr: r#"time="2026-04-22T06:07:20+08:00" level=info msg="[hostagent] Starting VZ (hint: to watch the boot progress, see "/Users/abalaian/.lima/agbranch-smoke-base/serial*.log")"
time="2026-04-22T06:07:20+08:00" level=fatal msg="exiting, status={Running:false Degraded:false Exiting:true Errors:[] SSHLocalPort:0 CloudInitProgress:<nil> PortForward:<nil> Vsock:<nil>} (hint: see "/Users/abalaian/.lima/agbranch-smoke-base/ha.stderr.log")""#
                .to_owned(),
        });

        assert!(should_retry_early_start_failure(&err));
    }

    #[test]
    fn does_not_retry_degraded_readiness_failures() {
        let err = LimaError::Process(ProcessError::Failed {
            program: "limactl".to_owned(),
            status: 1,
            stderr: r#"time="2026-04-20T21:27:41+08:00" level=error msg="[failed to satisfy the optional requirement 2 of 2 "agbranch runtime readiness"...]"
time="2026-04-20T21:27:41+08:00" level=fatal msg="degraded, status={Running:true Degraded:true Exiting:false Errors:[failed to satisfy the optional requirement 2 of 2 "agbranch runtime readiness"...] SSHLocalPort:55399 CloudInitProgress:<nil> PortForward:<nil> Vsock:<nil>}""#
                .to_owned(),
        });

        assert!(!should_retry_early_start_failure(&err));
    }
}
