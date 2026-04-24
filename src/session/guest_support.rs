use crate::error::lima::LimaError;
use crate::lima::{client::LimaClient, tmux};
use crate::session::paths::{provider_shim_path, shellenv_path};
use crate::types::{GuestPath, ProviderKind, SessionName, VmName};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub fn install_guest_support_files(
    client: &dyn LimaClient,
    vm_name: &VmName,
    host_home_dir: &Path,
) -> Result<(), LimaError> {
    let host_shellenv = crate::types::HostPath::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("lima")
            .join("guest")
            .join("shellenv.sh"),
    );
    let guest_shellenv = shellenv_path(host_home_dir);
    client.copy_host_file_to_guest(&host_shellenv, vm_name, &guest_shellenv)?;

    for provider in [
        ProviderKind::Codex,
        ProviderKind::Claude,
        ProviderKind::Gemini,
    ] {
        let rendered = crate::provider::shims::render_provider_shim(provider);
        let mut temp =
            tempfile::NamedTempFile::new().map_err(|source| LimaError::GuestSupport {
                path: "temporary provider shim".to_owned(),
                source,
            })?;
        temp.write_all(rendered.as_bytes())
            .map_err(|source| LimaError::GuestSupport {
                path: temp.path().display().to_string(),
                source,
            })?;
        let mut perms = temp
            .as_file()
            .metadata()
            .map_err(|source| LimaError::GuestSupport {
                path: temp.path().display().to_string(),
                source,
            })?
            .permissions();
        perms.set_mode(0o755);
        temp.as_file()
            .set_permissions(perms)
            .map_err(|source| LimaError::GuestSupport {
                path: temp.path().display().to_string(),
                source,
            })?;

        let host_path = crate::types::HostPath::new(temp.path().to_path_buf());
        let guest_path = provider_shim_path(host_home_dir, provider);
        client.copy_host_file_to_guest(&host_path, vm_name, &guest_path)?;
    }

    Ok(())
}

pub fn ensure_workspace_and_shell(
    client: &dyn LimaClient,
    vm_name: &VmName,
    session: &SessionName,
    tmux_socket: &GuestPath,
    workspace: &GuestPath,
) -> Result<(), LimaError> {
    let bootstrap = tmux::ensure_shell_window_commands(
        tmux_socket,
        session.as_str(),
        "shell",
        session.as_str(),
        &workspace.to_string(),
    )
    .join(" && ");
    client.bash(vm_name, &format!("mkdir -p {} && {}", workspace, bootstrap))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lima::inspect::LimaInstance;
    use crate::types::{DiskSize, HostPath, MemorySize};
    use crate::util::process::CommandOutput;
    use std::cell::RefCell;

    #[derive(Default)]
    struct RecordingClient {
        bash_commands: RefCell<Vec<String>>,
        copy_targets: RefCell<Vec<String>>,
    }

    impl LimaClient for RecordingClient {
        fn list_instances(&self) -> Result<Vec<LimaInstance>, LimaError> {
            unreachable!("list_instances is not used by guest support tests");
        }

        fn clone_instance(
            &self,
            _source: &VmName,
            _target: &VmName,
            _cpus: Option<u16>,
            _memory: Option<&MemorySize>,
            _disk: Option<&DiskSize>,
        ) -> Result<(), LimaError> {
            unreachable!("clone_instance is not used by guest support tests");
        }

        fn start_instance(&self, _name: &VmName) -> Result<(), LimaError> {
            unreachable!("start_instance is not used by guest support tests");
        }

        fn stop_instance(&self, _name: &VmName) -> Result<(), LimaError> {
            unreachable!("stop_instance is not used by guest support tests");
        }

        fn delete_instance(&self, _name: &VmName) -> Result<(), LimaError> {
            unreachable!("delete_instance is not used by guest support tests");
        }

        fn bash(&self, _vm: &VmName, command: &str) -> Result<CommandOutput, LimaError> {
            self.bash_commands.borrow_mut().push(command.to_owned());
            Ok(CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
            })
        }

        fn copy_host_path_to_guest(
            &self,
            _host_path: &HostPath,
            _instance_name: &VmName,
            _guest_path: &GuestPath,
        ) -> Result<(), LimaError> {
            unreachable!("copy_host_path_to_guest is not used by guest support tests");
        }

        fn seed_repo(
            &self,
            _filtered_seed_root: &HostPath,
            _instance_name: &VmName,
            _guest_repo: &GuestPath,
        ) -> Result<(), LimaError> {
            unreachable!("seed_repo is not used by guest support tests");
        }

        fn copy_host_file_to_guest(
            &self,
            _host_file: &HostPath,
            _instance_name: &VmName,
            guest_file: &GuestPath,
        ) -> Result<(), LimaError> {
            self.copy_targets.borrow_mut().push(guest_file.to_string());
            Ok(())
        }

        fn copy_guest_secret_file(
            &self,
            _host_secret_file: &HostPath,
            _instance_name: &VmName,
            _guest_secret_file: &GuestPath,
        ) -> Result<(), LimaError> {
            unreachable!("copy_guest_secret_file is not used by guest support tests");
        }
    }

    #[test]
    fn install_guest_support_files_copies_shellenv_and_provider_shims_under_agbranch_home() {
        let client = RecordingClient::default();
        let vm_name = VmName::new("agbranch-demo");

        install_guest_support_files(&client, &vm_name, Path::new("/Users/abalaian"))
            .expect("guest support install should succeed");

        let copy_targets = client.copy_targets.borrow();

        assert!(
            copy_targets
                .iter()
                .any(|target| target.ends_with("/home/abalaian.guest/.agbranch/shellenv.sh")),
            "shellenv should be copied under ~/.agbranch"
        );

        for provider in [
            ProviderKind::Codex,
            ProviderKind::Claude,
            ProviderKind::Gemini,
        ] {
            let expected = format!("/home/abalaian.guest/.agbranch/bin/{}", provider.as_str());
            assert!(
                copy_targets
                    .iter()
                    .any(|target| target.ends_with(&expected)),
                "provider shim for {} should be copied under ~/.agbranch/bin",
                provider.as_str()
            );
        }
    }

    #[test]
    fn ensure_workspace_and_shell_bootstraps_tmux_shell_window_in_workspace() {
        let client = RecordingClient::default();
        let session = SessionName::try_from("agbranch-smoke-happy").expect("session");
        let tmux_socket =
            GuestPath::new("/home/abalaian.guest/.agbranch/tmux/agbranch-smoke-happy.sock");
        let workspace = GuestPath::new("/home/abalaian.guest/sandbox/agbranch-smoke-happy");

        ensure_workspace_and_shell(
            &client,
            &VmName::new("agbranch-demo"),
            &session,
            &tmux_socket,
            &workspace,
        )
        .expect("workspace bootstrap should succeed");

        let bash_commands = client.bash_commands.borrow();
        let command = bash_commands.last().expect("bash payload");

        assert!(
            command.contains("mkdir -p /home/abalaian.guest/sandbox/agbranch-smoke-happy"),
            "workspace directory should be created before tmux bootstrap"
        );
        assert!(
            command.contains(
                "tmux -S '/home/abalaian.guest/.agbranch/tmux/agbranch-smoke-happy.sock'"
            ),
            "tmux bootstrap should target the session socket"
        );
        assert!(
            command.contains("-c /home/abalaian.guest/sandbox/agbranch-smoke-happy"),
            "tmux shell window should start in the requested workspace"
        );
    }
}
