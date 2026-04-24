use crate::error::lima::LimaError;
use crate::types::{GuestPath, HostPath, VmName};
use crate::util::process::CommandRunner;
use std::collections::BTreeMap;
use std::path::Path;

pub fn copy_with_rsync(
    runner: &dyn CommandRunner,
    from: &HostPath,
    to: &str,
) -> Result<(), LimaError> {
    let args = build_host_copy_args(from.as_path(), to);
    runner.run("limactl", &args, None, &BTreeMap::new())?;
    Ok(())
}

pub fn copy_host_path_to_guest(
    runner: &dyn CommandRunner,
    host_path: &HostPath,
    instance_name: &VmName,
    guest_path: &GuestPath,
) -> Result<(), LimaError> {
    runner.run(
        "limactl",
        &[
            "shell".to_owned(),
            instance_name.as_str().to_owned(),
            "--".to_owned(),
            "mkdir".to_owned(),
            "-p".to_owned(),
            guest_path.as_path().display().to_string(),
        ],
        None,
        &BTreeMap::new(),
    )?;
    copy_with_rsync(
        runner,
        host_path,
        &format!(
            "{}:{}",
            instance_name.as_str(),
            guest_path.as_path().display()
        ),
    )
}

pub fn seed_repo(
    runner: &dyn CommandRunner,
    filtered_seed_root: &HostPath,
    instance_name: &VmName,
    guest_repo: &GuestPath,
) -> Result<(), LimaError> {
    runner.run(
        "limactl",
        &[
            "shell".to_owned(),
            instance_name.as_str().to_owned(),
            "--".to_owned(),
            "mkdir".to_owned(),
            "-p".to_owned(),
            guest_repo.as_path().display().to_string(),
        ],
        None,
        &BTreeMap::new(),
    )?;
    copy_with_rsync(
        runner,
        filtered_seed_root,
        &format!(
            "{}:{}",
            instance_name.as_str(),
            guest_repo.as_path().display()
        ),
    )
}

pub fn copy_guest_secret_file(
    runner: &dyn CommandRunner,
    host_secret_file: &HostPath,
    instance_name: &VmName,
    guest_secret_file: &GuestPath,
) -> Result<(), LimaError> {
    copy_host_file_to_guest(runner, host_secret_file, instance_name, guest_secret_file)
}

pub fn copy_host_file_to_guest(
    runner: &dyn CommandRunner,
    host_file: &HostPath,
    instance_name: &VmName,
    guest_file: &GuestPath,
) -> Result<(), LimaError> {
    runner.run(
        "limactl",
        &[
            "shell".to_owned(),
            instance_name.as_str().to_owned(),
            "--".to_owned(),
            "mkdir".to_owned(),
            "-p".to_owned(),
            guest_file
                .as_path()
                .parent()
                .unwrap_or(guest_file.as_path())
                .display()
                .to_string(),
        ],
        None,
        &BTreeMap::new(),
    )?;
    copy_with_rsync(
        runner,
        host_file,
        &format!(
            "{}:{}",
            instance_name.as_str(),
            guest_file.as_path().display()
        ),
    )
}

pub fn copy_repo_from_guest(
    runner: &dyn CommandRunner,
    instance_name: &VmName,
    guest_repo: &GuestPath,
    host_destination: &HostPath,
) -> Result<(), LimaError> {
    runner.run(
        "limactl",
        &[
            "copy".to_owned(),
            "--backend=rsync".to_owned(),
            "-r".to_owned(),
            format!(
                "{}:{}/.",
                instance_name.as_str(),
                guest_repo.as_path().display()
            ),
            format!("{}/", host_destination.as_path().display()),
        ],
        None,
        &BTreeMap::new(),
    )?;
    Ok(())
}

pub fn copy_guest_path_to_host(
    runner: &dyn CommandRunner,
    instance_name: &VmName,
    guest_path: &GuestPath,
    host_destination: &HostPath,
) -> Result<(), LimaError> {
    let guest_is_dir = guest_path_is_directory(runner, instance_name, guest_path)?;
    let args = build_guest_copy_args(
        instance_name,
        guest_path.as_path(),
        host_destination,
        guest_is_dir,
    );
    runner.run("limactl", &args, None, &BTreeMap::new())?;
    Ok(())
}

fn host_rsync_source(path: &Path) -> String {
    let display = path.display().to_string();
    if path.is_dir() {
        format!("{display}/.")
    } else {
        display
    }
}

fn guest_path_is_directory(
    runner: &dyn CommandRunner,
    instance_name: &VmName,
    guest_path: &GuestPath,
) -> Result<bool, LimaError> {
    let args = vec![
        "shell".to_owned(),
        instance_name.as_str().to_owned(),
        "--".to_owned(),
        "test".to_owned(),
        "-d".to_owned(),
        guest_path.as_path().display().to_string(),
    ];
    match runner.run("limactl", &args, None, &BTreeMap::new()) {
        Ok(_) => Ok(true),
        Err(crate::error::process::ProcessError::Failed { status: 1, .. }) => Ok(false),
        Err(err) => Err(LimaError::from(err)),
    }
}

fn build_guest_copy_args(
    instance_name: &VmName,
    guest_path: &Path,
    host_destination: &HostPath,
    recursive: bool,
) -> Vec<String> {
    let mut args = vec!["copy".to_owned(), "--backend=rsync".to_owned()];
    if recursive {
        args.push("-r".to_owned());
    }
    let source = if recursive {
        format!("{}:{}/.", instance_name.as_str(), guest_path.display())
    } else {
        format!("{}:{}", instance_name.as_str(), guest_path.display())
    };
    args.push(source);
    args.push(host_destination.as_path().display().to_string());
    args
}

fn build_host_copy_args(from: &Path, to: &str) -> Vec<String> {
    let mut args = vec!["copy".to_owned(), "--backend=rsync".to_owned()];
    if from.is_dir() {
        args.push("-r".to_owned());
    }
    args.push(host_rsync_source(from));
    args.push(to.to_owned());
    args
}

#[cfg(test)]
mod tests {
    use super::{build_host_copy_args, copy_guest_path_to_host, host_rsync_source, seed_repo};
    use crate::error::process::ProcessError;
    use crate::types::{GuestPath, HostPath, VmName};
    use crate::util::process::{CommandOutput, CommandRunner};
    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    #[derive(Default)]
    struct RecordingRunner {
        calls: RefCell<Vec<(String, Vec<String>)>>,
        directory_paths: BTreeSet<String>,
    }

    impl CommandRunner for RecordingRunner {
        fn run(
            &self,
            program: &str,
            args: &[String],
            _cwd: Option<&Path>,
            _env: &BTreeMap<String, String>,
        ) -> Result<CommandOutput, crate::error::process::ProcessError> {
            self.calls
                .borrow_mut()
                .push((program.to_owned(), args.to_vec()));
            if program == "limactl"
                && args.len() >= 6
                && args[0] == "shell"
                && args[2] == "--"
                && args[3] == "test"
                && args[4] == "-d"
                && !self.directory_paths.contains(&args[5])
            {
                return Err(ProcessError::Failed {
                    program: program.to_owned(),
                    status: 1,
                    stderr: String::new(),
                });
            }
            Ok(CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn adds_trailing_slash_for_directory_sources() {
        let dir = tempdir().expect("tempdir");
        assert!(host_rsync_source(dir.path()).ends_with("/."));
    }

    #[test]
    fn leaves_file_sources_without_trailing_slash() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("payload.sh");
        fs::write(&file, "#!/bin/sh\n").expect("fixture file");

        assert_eq!(host_rsync_source(&file), file.display().to_string());
    }

    #[test]
    fn omits_recursive_flag_for_file_sources() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("payload.sh");
        fs::write(&file, "#!/bin/sh\n").expect("fixture file");

        let args = build_host_copy_args(&file, "vm:/tmp/payload.sh");
        assert!(
            !args.iter().any(|arg| arg == "-r"),
            "file uploads should not use recursive copy mode"
        );
    }

    #[test]
    fn keeps_recursive_flag_for_directory_sources() {
        let dir = tempdir().expect("tempdir");

        let args = build_host_copy_args(dir.path(), "vm:/tmp/workspace");
        assert!(
            args.iter().any(|arg| arg == "-r"),
            "directory uploads should remain recursive"
        );
    }

    #[test]
    fn seed_repo_creates_repo_directory_before_copying_contents() {
        let dir = tempdir().expect("tempdir");
        let runner = RecordingRunner::default();
        let host_seed = HostPath::new(dir.path());
        let vm_name = VmName::new("agbranch-manual-happy");
        let guest_repo = GuestPath::new("/home/tester.guest/workspaces/manual-happy/repo");

        seed_repo(&runner, &host_seed, &vm_name, &guest_repo).expect("seed repo");

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 2, "seed_repo should mkdir then copy");
        assert_eq!(calls[0].0, "limactl");
        assert_eq!(
            calls[0].1,
            vec![
                "shell",
                "agbranch-manual-happy",
                "--",
                "mkdir",
                "-p",
                "/home/tester.guest/workspaces/manual-happy/repo",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        );
        assert_eq!(calls[1].0, "limactl");
        assert!(
            calls[1].1.iter().any(|arg| arg == "-r"),
            "directory seed copy should remain recursive"
        );
    }

    #[test]
    fn guest_file_export_omits_recursive_flag() {
        let runner = RecordingRunner::default();
        let vm_name = VmName::new("agbranch-export-file");
        let guest_file = GuestPath::new("/home/tester.guest/sandbox/demo/README.md");
        let host_path = HostPath::new("/tmp/exported-readme.md");

        copy_guest_path_to_host(&runner, &vm_name, &guest_file, &host_path)
            .expect("guest file export");

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 2, "export should probe then copy");
        assert_eq!(
            calls[0].1,
            vec![
                "shell",
                "agbranch-export-file",
                "--",
                "test",
                "-d",
                "/home/tester.guest/sandbox/demo/README.md",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        );
        assert!(
            !calls[1].1.iter().any(|arg| arg == "-r"),
            "file exports should not use recursive copy"
        );
        assert_eq!(
            calls[1].1,
            vec![
                "copy",
                "--backend=rsync",
                "agbranch-export-file:/home/tester.guest/sandbox/demo/README.md",
                "/tmp/exported-readme.md",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        );
    }
}
