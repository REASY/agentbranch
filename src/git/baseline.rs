use crate::error::process::ProcessError;
use crate::util::process::CommandRunner;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoBaseline {
    pub head_oid: Option<String>,
    pub head_ref: Option<String>,
    pub dirty: bool,
}

pub fn capture_repo_baseline(
    runner: &dyn CommandRunner,
    repo_root: &Path,
) -> Result<RepoBaseline, ProcessError> {
    let env = BTreeMap::new();
    let in_git_repo = runner
        .run(
            "git",
            &["rev-parse".to_owned(), "--is-inside-work-tree".to_owned()],
            Some(repo_root),
            &env,
        )
        .ok()
        .map(|out| out.stdout.trim() == "true")
        .unwrap_or(false);

    if !in_git_repo {
        return Ok(RepoBaseline {
            head_oid: None,
            head_ref: None,
            dirty: false,
        });
    }

    let head_oid = runner
        .run(
            "git",
            &["rev-parse".to_owned(), "HEAD".to_owned()],
            Some(repo_root),
            &env,
        )
        .ok()
        .map(|out| out.stdout.trim().to_owned())
        .filter(|value| !value.is_empty());

    let head_ref = runner
        .run(
            "git",
            &[
                "symbolic-ref".to_owned(),
                "--quiet".to_owned(),
                "HEAD".to_owned(),
            ],
            Some(repo_root),
            &env,
        )
        .ok()
        .map(|out| out.stdout.trim().to_owned())
        .filter(|value| !value.is_empty());

    let dirty = runner
        .run(
            "git",
            &["status".to_owned(), "--porcelain".to_owned()],
            Some(repo_root),
            &env,
        )?
        .stdout
        .lines()
        .next()
        .is_some();

    Ok(RepoBaseline {
        head_oid,
        head_ref,
        dirty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::process::CommandOutput;
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::Mutex;

    struct ExpectedCall {
        program: String,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        result: Result<CommandOutput, ProcessError>,
    }

    struct ExpectationRunner {
        calls: Mutex<VecDeque<ExpectedCall>>,
    }

    impl ExpectationRunner {
        fn with(calls: Vec<ExpectedCall>) -> Self {
            Self {
                calls: Mutex::new(calls.into()),
            }
        }

        fn assert_no_remaining_calls(&self) {
            let pending = self.calls.lock().expect("lock").len();
            assert_eq!(pending, 0, "unused expected git calls remain: {pending}");
        }
    }

    fn git_call(
        args: &[&str],
        cwd: &Path,
        result: Result<CommandOutput, ProcessError>,
    ) -> ExpectedCall {
        ExpectedCall {
            program: "git".to_owned(),
            args: args.iter().map(|value| (*value).to_owned()).collect(),
            cwd: Some(cwd.to_path_buf()),
            result,
        }
    }

    impl CommandRunner for ExpectationRunner {
        fn run(
            &self,
            program: &str,
            args: &[String],
            cwd: Option<&Path>,
            env: &BTreeMap<String, String>,
        ) -> Result<CommandOutput, ProcessError> {
            let expected = self
                .calls
                .lock()
                .expect("lock")
                .pop_front()
                .expect("unexpected command invocation");

            assert_eq!(program, expected.program, "unexpected program");
            assert_eq!(args, expected.args, "unexpected args");
            assert_eq!(cwd.map(Path::to_path_buf), expected.cwd, "unexpected cwd");
            assert!(env.is_empty(), "expected empty env");

            expected.result
        }
    }

    #[test]
    fn non_git_repo_returns_empty_baseline() {
        let repo_root = Path::new("/tmp/non-git");
        let runner = ExpectationRunner::with(vec![git_call(
            &["rev-parse", "--is-inside-work-tree"],
            repo_root,
            Err(ProcessError::Failed {
                program: "git".to_owned(),
                status: 128,
                stderr: "not a git repo".to_owned(),
            }),
        )]);

        let baseline = capture_repo_baseline(&runner, repo_root).expect("baseline");
        assert_eq!(baseline.head_oid, None);
        assert_eq!(baseline.head_ref, None);
        assert!(!baseline.dirty);
        runner.assert_no_remaining_calls();
    }

    #[test]
    fn branch_head_captures_oid_and_ref() {
        let repo_root = Path::new("/tmp/repo");
        let runner = ExpectationRunner::with(vec![
            git_call(
                &["rev-parse", "--is-inside-work-tree"],
                repo_root,
                Ok(CommandOutput {
                    stdout: "true\n".into(),
                    stderr: String::new(),
                }),
            ),
            git_call(
                &["rev-parse", "HEAD"],
                repo_root,
                Ok(CommandOutput {
                    stdout: "abc123\n".into(),
                    stderr: String::new(),
                }),
            ),
            git_call(
                &["symbolic-ref", "--quiet", "HEAD"],
                repo_root,
                Ok(CommandOutput {
                    stdout: "refs/heads/main\n".into(),
                    stderr: String::new(),
                }),
            ),
            git_call(
                &["status", "--porcelain"],
                repo_root,
                Ok(CommandOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                }),
            ),
        ]);

        let baseline = capture_repo_baseline(&runner, repo_root).expect("baseline");
        assert_eq!(baseline.head_oid.as_deref(), Some("abc123"));
        assert_eq!(baseline.head_ref.as_deref(), Some("refs/heads/main"));
        assert!(!baseline.dirty);
        runner.assert_no_remaining_calls();
    }

    #[test]
    fn detached_head_keeps_oid_and_omits_ref() {
        let repo_root = Path::new("/tmp/repo");
        let runner = ExpectationRunner::with(vec![
            git_call(
                &["rev-parse", "--is-inside-work-tree"],
                repo_root,
                Ok(CommandOutput {
                    stdout: "true\n".into(),
                    stderr: String::new(),
                }),
            ),
            git_call(
                &["rev-parse", "HEAD"],
                repo_root,
                Ok(CommandOutput {
                    stdout: "abc123\n".into(),
                    stderr: String::new(),
                }),
            ),
            git_call(
                &["symbolic-ref", "--quiet", "HEAD"],
                repo_root,
                Err(ProcessError::Failed {
                    program: "git".to_owned(),
                    status: 1,
                    stderr: String::new(),
                }),
            ),
            git_call(
                &["status", "--porcelain"],
                repo_root,
                Ok(CommandOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                }),
            ),
        ]);

        let baseline = capture_repo_baseline(&runner, repo_root).expect("baseline");
        assert_eq!(baseline.head_oid.as_deref(), Some("abc123"));
        assert_eq!(baseline.head_ref, None);
        assert!(!baseline.dirty);
        runner.assert_no_remaining_calls();
    }

    #[test]
    fn dirty_repo_sets_dirty_flag() {
        let repo_root = Path::new("/tmp/repo");
        let runner = ExpectationRunner::with(vec![
            git_call(
                &["rev-parse", "--is-inside-work-tree"],
                repo_root,
                Ok(CommandOutput {
                    stdout: "true\n".into(),
                    stderr: String::new(),
                }),
            ),
            git_call(
                &["rev-parse", "HEAD"],
                repo_root,
                Ok(CommandOutput {
                    stdout: "abc123\n".into(),
                    stderr: String::new(),
                }),
            ),
            git_call(
                &["symbolic-ref", "--quiet", "HEAD"],
                repo_root,
                Ok(CommandOutput {
                    stdout: "refs/heads/main\n".into(),
                    stderr: String::new(),
                }),
            ),
            git_call(
                &["status", "--porcelain"],
                repo_root,
                Ok(CommandOutput {
                    stdout: " M src/main.rs\n".into(),
                    stderr: String::new(),
                }),
            ),
        ]);

        let baseline = capture_repo_baseline(&runner, repo_root).expect("baseline");
        assert_eq!(baseline.head_oid.as_deref(), Some("abc123"));
        assert_eq!(baseline.head_ref.as_deref(), Some("refs/heads/main"));
        assert!(baseline.dirty);
        runner.assert_no_remaining_calls();
    }
}
