use crate::error::process::ProcessError;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
}

pub trait CommandRunner {
    fn run(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
        env: &BTreeMap<String, String>,
    ) -> Result<CommandOutput, ProcessError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RealCommandRunner;

impl CommandRunner for RealCommandRunner {
    fn run(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
        env: &BTreeMap<String, String>,
    ) -> Result<CommandOutput, ProcessError> {
        let mut cmd = Command::new(program);
        cmd.args(args);
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        for (key, value) in env {
            cmd.env(key, value);
        }

        let output = cmd.output().map_err(|source| ProcessError::Spawn {
            program: program.to_owned(),
            source,
        })?;

        let stdout = String::from_utf8(output.stdout).map_err(|_| ProcessError::NonUtf8 {
            program: program.to_owned(),
        })?;
        let stderr = String::from_utf8(output.stderr).map_err(|_| ProcessError::NonUtf8 {
            program: program.to_owned(),
        })?;

        if !output.status.success() {
            let stderr = if stderr.trim().is_empty() {
                stdout.trim().to_owned()
            } else {
                stderr.trim().to_owned()
            };
            return Err(ProcessError::Failed {
                program: program.to_owned(),
                status: output.status.code().unwrap_or(1),
                stderr,
            });
        }

        Ok(CommandOutput { stdout, stderr })
    }
}
