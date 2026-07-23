use crate::error::process::ProcessError;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use wait_timeout::ChildExt;

const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const DEFAULT_CAPTURE_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy)]
struct CommandPolicy<'a> {
    timeout: Duration,
    capture_limit: usize,
    accepted_statuses: &'a [i32],
}

const DEFAULT_COMMAND_POLICY: CommandPolicy<'static> = CommandPolicy {
    timeout: DEFAULT_COMMAND_TIMEOUT,
    capture_limit: DEFAULT_CAPTURE_LIMIT,
    accepted_statuses: &[0],
};

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
}

pub trait CommandRunner {
    fn run(
        &self,
        program: &str,
        _args: &[String],
        _cwd: Option<&Path>,
        _env: &BTreeMap<String, String>,
    ) -> Result<CommandOutput, ProcessError>;

    fn run_with_input(
        &self,
        program: &str,
        _args: &[String],
        _cwd: Option<&Path>,
        _env: &BTreeMap<String, String>,
        _input: &[u8],
    ) -> Result<CommandOutput, ProcessError> {
        Err(ProcessError::InputUnsupported {
            program: program.to_owned(),
        })
    }
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
        run_bounded(program, args, cwd, env, None, DEFAULT_COMMAND_POLICY)
    }

    fn run_with_input(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
        env: &BTreeMap<String, String>,
        input: &[u8],
    ) -> Result<CommandOutput, ProcessError> {
        run_bounded(program, args, cwd, env, Some(input), DEFAULT_COMMAND_POLICY)
    }
}

fn run_bounded(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    env: &BTreeMap<String, String>,
    input: Option<&[u8]>,
    policy: CommandPolicy<'_>,
) -> Result<CommandOutput, ProcessError> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    for (key, value) in env {
        cmd.env(key, value);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let mut child = cmd.spawn().map_err(|source| ProcessError::Spawn {
        program: program.to_owned(),
        source,
    })?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let stdout_reader = thread::spawn(move || read_capped(stdout, policy.capture_limit));
    let stderr_reader = thread::spawn(move || read_capped(stderr, policy.capture_limit));

    let input_writer = input.map(|input| {
        let mut stdin = child.stdin.take().expect("stdin is piped");
        let input = input.to_vec();
        thread::spawn(move || -> std::io::Result<()> {
            stdin.write_all(&input)?;
            stdin.flush()
        })
    });

    let deadline = Instant::now() + policy.timeout;
    let status = loop {
        if crate::util::signals::take_interrupt() {
            terminate_child_tree(&mut child);
            let _ = child.wait();
            join_writer(input_writer, program)?;
            let _ = join_reader(stdout_reader, program)?;
            let _ = join_reader(stderr_reader, program)?;
            return Err(ProcessError::Interrupted {
                program: program.to_owned(),
            });
        }
        let now = Instant::now();
        if now >= deadline {
            terminate_child_tree(&mut child);
            let _ = child.wait();
            join_writer(input_writer, program)?;
            let _ = join_reader(stdout_reader, program)?;
            let _ = join_reader(stderr_reader, program)?;
            return Err(ProcessError::Timeout {
                program: program.to_owned(),
                timeout: policy.timeout,
            });
        }
        let wait_for = deadline
            .saturating_duration_since(now)
            .min(Duration::from_millis(100));
        if let Some(status) =
            child
                .wait_timeout(wait_for)
                .map_err(|source| ProcessError::Spawn {
                    program: program.to_owned(),
                    source,
                })?
        {
            break status;
        }
    };

    join_writer(input_writer, program)?;
    let (stdout, stdout_overflow) = join_reader(stdout_reader, program)?;
    let (stderr, stderr_overflow) = join_reader(stderr_reader, program)?;
    if stdout_overflow {
        return Err(ProcessError::OutputLimit {
            program: program.to_owned(),
            stream: "stdout",
            limit: policy.capture_limit,
        });
    }
    if stderr_overflow {
        return Err(ProcessError::OutputLimit {
            program: program.to_owned(),
            stream: "stderr",
            limit: policy.capture_limit,
        });
    }

    let stdout = String::from_utf8(stdout).map_err(|_| ProcessError::NonUtf8 {
        program: program.to_owned(),
    })?;
    let stderr = String::from_utf8(stderr).map_err(|_| ProcessError::NonUtf8 {
        program: program.to_owned(),
    })?;

    let status_code = status.code().unwrap_or(1);
    if !policy.accepted_statuses.contains(&status_code) {
        let stderr = if stderr.trim().is_empty() {
            stdout.trim().to_owned()
        } else {
            stderr.trim().to_owned()
        };
        return Err(ProcessError::Failed {
            program: program.to_owned(),
            status: status_code,
            stderr,
        });
    }

    Ok(CommandOutput { stdout, stderr })
}

pub(crate) fn run_with_limits(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    timeout: Duration,
    capture_limit: usize,
    accepted_statuses: &[i32],
) -> Result<CommandOutput, ProcessError> {
    run_bounded(
        program,
        args,
        cwd,
        &BTreeMap::new(),
        None,
        CommandPolicy {
            timeout,
            capture_limit,
            accepted_statuses,
        },
    )
}

fn read_capped(mut reader: impl Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut captured = Vec::new();
    let mut overflow = false;
    let mut chunk = [0_u8; 8192];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(captured.len());
        let retained = remaining.min(read);
        captured.extend_from_slice(&chunk[..retained]);
        overflow |= retained < read;
    }
    Ok((captured, overflow))
}

fn join_reader(
    reader: thread::JoinHandle<std::io::Result<(Vec<u8>, bool)>>,
    program: &str,
) -> Result<(Vec<u8>, bool), ProcessError> {
    reader
        .join()
        .map_err(|_| ProcessError::Spawn {
            program: program.to_owned(),
            source: std::io::Error::other("output reader thread panicked"),
        })?
        .map_err(|source| ProcessError::Spawn {
            program: program.to_owned(),
            source,
        })
}

fn join_writer(
    writer: Option<thread::JoinHandle<std::io::Result<()>>>,
    program: &str,
) -> Result<(), ProcessError> {
    let Some(writer) = writer else {
        return Ok(());
    };
    writer
        .join()
        .map_err(|_| ProcessError::Spawn {
            program: program.to_owned(),
            source: std::io::Error::other("input writer thread panicked"),
        })?
        .map_err(|source| ProcessError::Spawn {
            program: program.to_owned(),
            source,
        })
}

fn terminate_child_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let process_group = -(child.id() as i32);
        // SAFETY: `process_group` refers to the child group created immediately
        // before spawn. Failure is harmless because `Child::kill` is the fallback.
        unsafe {
            libc::kill(process_group, libc::SIGKILL);
        }
    }
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_runner_writes_stdin() {
        let output = run_bounded(
            "sh",
            &["-c".to_owned(), "cat".to_owned()],
            None,
            &BTreeMap::new(),
            Some(b"transaction\n"),
            CommandPolicy {
                timeout: Duration::from_secs(2),
                capture_limit: 1024,
                accepted_statuses: &[0],
            },
        )
        .expect("cat");
        assert_eq!(output.stdout, "transaction\n");
    }

    #[test]
    fn bounded_runner_times_out_and_kills_process_group() {
        let err = run_bounded(
            "sh",
            &["-c".to_owned(), "sleep 10".to_owned()],
            None,
            &BTreeMap::new(),
            None,
            CommandPolicy {
                timeout: Duration::from_millis(50),
                capture_limit: 1024,
                accepted_statuses: &[0],
            },
        )
        .expect_err("timeout");
        assert!(matches!(err, ProcessError::Timeout { .. }));
    }

    #[test]
    fn bounded_runner_rejects_excess_output() {
        let err = run_bounded(
            "sh",
            &["-c".to_owned(), "printf 12345".to_owned()],
            None,
            &BTreeMap::new(),
            None,
            CommandPolicy {
                timeout: Duration::from_secs(2),
                capture_limit: 4,
                accepted_statuses: &[0],
            },
        )
        .expect_err("output limit");
        assert!(matches!(
            err,
            ProcessError::OutputLimit {
                stream: "stdout",
                limit: 4,
                ..
            }
        ));
    }
}
