use crate::error::lima::LimaError;
use crate::lima::{copy, inspect::LimaInstance, instance};
use crate::types::{DiskSize, GuestPath, HostPath, MemorySize, VmName};
use crate::util::process::{CommandOutput, CommandRunner};
use std::collections::BTreeMap;

pub trait LimaClient {
    fn list_instances(&self) -> Result<Vec<LimaInstance>, LimaError>;

    fn clone_instance(
        &self,
        source: &VmName,
        target: &VmName,
        cpus: Option<u16>,
        memory: Option<&MemorySize>,
        disk: Option<&DiskSize>,
    ) -> Result<(), LimaError>;

    fn start_instance(&self, name: &VmName) -> Result<(), LimaError>;

    fn stop_instance(&self, name: &VmName) -> Result<(), LimaError>;

    fn delete_instance(&self, name: &VmName) -> Result<(), LimaError>;

    fn bash(&self, vm: &VmName, command: &str) -> Result<CommandOutput, LimaError>;

    fn copy_host_path_to_guest(
        &self,
        host_path: &HostPath,
        instance_name: &VmName,
        guest_path: &GuestPath,
    ) -> Result<(), LimaError>;

    fn seed_repo(
        &self,
        filtered_seed_root: &HostPath,
        instance_name: &VmName,
        guest_repo: &GuestPath,
    ) -> Result<(), LimaError>;

    fn copy_host_file_to_guest(
        &self,
        host_file: &HostPath,
        instance_name: &VmName,
        guest_file: &GuestPath,
    ) -> Result<(), LimaError>;

    fn copy_guest_secret_file(
        &self,
        host_secret_file: &HostPath,
        instance_name: &VmName,
        guest_secret_file: &GuestPath,
    ) -> Result<(), LimaError>;
}

pub struct LimactlClient<'a, R: CommandRunner> {
    runner: &'a R,
}

impl<'a, R: CommandRunner> LimactlClient<'a, R> {
    pub fn new(runner: &'a R) -> Self {
        Self { runner }
    }
}

impl<R: CommandRunner> LimaClient for LimactlClient<'_, R> {
    fn list_instances(&self) -> Result<Vec<LimaInstance>, LimaError> {
        instance::list_instances(self.runner)
    }

    fn clone_instance(
        &self,
        source: &VmName,
        target: &VmName,
        cpus: Option<u16>,
        memory: Option<&MemorySize>,
        disk: Option<&DiskSize>,
    ) -> Result<(), LimaError> {
        instance::clone_instance(self.runner, source, target, cpus, memory, disk)
    }

    fn start_instance(&self, name: &VmName) -> Result<(), LimaError> {
        instance::start_instance(self.runner, name)
    }

    fn stop_instance(&self, name: &VmName) -> Result<(), LimaError> {
        instance::stop_instance(self.runner, name)
    }

    fn delete_instance(&self, name: &VmName) -> Result<(), LimaError> {
        instance::delete_instance(self.runner, name)
    }

    fn bash(&self, vm: &VmName, command: &str) -> Result<CommandOutput, LimaError> {
        let args = vec![
            "shell".to_owned(),
            vm.as_str().to_owned(),
            "--".to_owned(),
            "bash".to_owned(),
            "-lc".to_owned(),
            command.to_owned(),
        ];
        self.runner
            .run("limactl", &args, None, &BTreeMap::new())
            .map_err(LimaError::from)
    }

    fn copy_host_path_to_guest(
        &self,
        host_path: &HostPath,
        instance_name: &VmName,
        guest_path: &GuestPath,
    ) -> Result<(), LimaError> {
        copy::copy_host_path_to_guest(self.runner, host_path, instance_name, guest_path)
    }

    fn seed_repo(
        &self,
        filtered_seed_root: &HostPath,
        instance_name: &VmName,
        guest_repo: &GuestPath,
    ) -> Result<(), LimaError> {
        copy::seed_repo(self.runner, filtered_seed_root, instance_name, guest_repo)
    }

    fn copy_host_file_to_guest(
        &self,
        host_file: &HostPath,
        instance_name: &VmName,
        guest_file: &GuestPath,
    ) -> Result<(), LimaError> {
        copy::copy_host_file_to_guest(self.runner, host_file, instance_name, guest_file)
    }

    fn copy_guest_secret_file(
        &self,
        host_secret_file: &HostPath,
        instance_name: &VmName,
        guest_secret_file: &GuestPath,
    ) -> Result<(), LimaError> {
        copy::copy_guest_secret_file(
            self.runner,
            host_secret_file,
            instance_name,
            guest_secret_file,
        )
    }
}
