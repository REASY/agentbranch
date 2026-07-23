use crate::types::SessionName;
use crate::util::process::CommandRunner;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRefs {
    pub base: String,
    pub head: String,
}

pub fn hidden_ref_names(session: &SessionName) -> SessionRefs {
    SessionRefs {
        base: format!("refs/agbranch/sessions/{session}/base"),
        head: format!("refs/agbranch/sessions/{session}/head"),
    }
}

pub fn review_branch_name(session: &SessionName) -> String {
    format!("agbranch/{session}")
}

pub fn incoming_ref_name(session: &SessionName) -> String {
    format!("refs/agbranch/sessions/{session}/incoming")
}

pub fn resolve_base_ref(explicit: Option<&str>, current: &str) -> String {
    explicit.unwrap_or(current).to_owned()
}

pub fn resolve_ref_oid(
    runner: &dyn CommandRunner,
    repo_root: &Path,
    reference: &str,
) -> Result<String, crate::error::process::ProcessError> {
    let output = runner.run(
        "git",
        &["rev-parse".to_owned(), reference.to_owned()],
        Some(repo_root),
        &BTreeMap::new(),
    )?;
    Ok(output.stdout.trim().to_owned())
}

pub fn initialize_session_refs(
    runner: &dyn CommandRunner,
    repo_root: &Path,
    refs: &SessionRefs,
    base_oid: &str,
) -> Result<(), crate::error::process::ProcessError> {
    for reference in [&refs.base, &refs.head] {
        runner.run(
            "git",
            &[
                "update-ref".to_owned(),
                reference.to_owned(),
                base_oid.to_owned(),
            ],
            Some(repo_root),
            &BTreeMap::new(),
        )?;
    }
    Ok(())
}

pub fn ref_exists(
    runner: &dyn CommandRunner,
    repo_root: &Path,
    reference: &str,
) -> Result<bool, crate::error::process::ProcessError> {
    match runner.run(
        "git",
        &[
            "show-ref".to_owned(),
            "--verify".to_owned(),
            "--quiet".to_owned(),
            reference.to_owned(),
        ],
        Some(repo_root),
        &BTreeMap::new(),
    ) {
        Ok(_) => Ok(true),
        Err(crate::error::process::ProcessError::Failed { .. }) => Ok(false),
        Err(err) => Err(err),
    }
}

pub fn is_ancestor(
    runner: &dyn CommandRunner,
    repo_root: &Path,
    ancestor: &str,
    descendant: &str,
) -> Result<bool, crate::error::process::ProcessError> {
    match runner.run(
        "git",
        &[
            "merge-base".to_owned(),
            "--is-ancestor".to_owned(),
            ancestor.to_owned(),
            descendant.to_owned(),
        ],
        Some(repo_root),
        &BTreeMap::new(),
    ) {
        Ok(_) => Ok(true),
        Err(crate::error::process::ProcessError::Failed { status: 1, .. }) => Ok(false),
        Err(err) => Err(err),
    }
}

pub fn delete_ref_if_exists(
    runner: &dyn CommandRunner,
    repo_root: &Path,
    reference: &str,
) -> Result<(), crate::error::process::ProcessError> {
    if !ref_exists(runner, repo_root, reference)? {
        return Ok(());
    }
    runner.run(
        "git",
        &[
            "update-ref".to_owned(),
            "-d".to_owned(),
            reference.to_owned(),
        ],
        Some(repo_root),
        &BTreeMap::new(),
    )?;
    Ok(())
}

pub fn publish_synced_refs(
    runner: &dyn CommandRunner,
    repo_root: &Path,
    review_branch: &str,
    source_oid: &str,
    session_head_ref: &str,
    expected_session_head: &str,
) -> Result<bool, crate::error::process::ProcessError> {
    let review_ref = format!("refs/heads/{review_branch}");
    let expected_review_oid = if ref_exists(runner, repo_root, &review_ref)? {
        let oid = resolve_ref_oid(runner, repo_root, &review_ref)?;
        if !is_ancestor(runner, repo_root, &oid, source_oid)? {
            return Ok(false);
        }
        oid
    } else {
        "0000000000000000000000000000000000000000".to_owned()
    };

    let transaction = format!(
        "start\nupdate {review_ref} {source_oid} {expected_review_oid}\n\
         update {session_head_ref} {source_oid} {expected_session_head}\nprepare\ncommit\n"
    );
    match runner.run_with_input(
        "git",
        &["update-ref".to_owned(), "--stdin".to_owned()],
        Some(repo_root),
        &BTreeMap::new(),
        transaction.as_bytes(),
    ) {
        Ok(_) => Ok(true),
        Err(crate::error::process::ProcessError::Failed { .. }) => Ok(false),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::bundle::fetch_bundle_ref;
    use crate::util::process::CommandOutput;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::tempdir;

    #[test]
    fn review_branch_name_matches_session() {
        let session = SessionName::try_from("feature-x").expect("session");
        assert_eq!(review_branch_name(&session), "agbranch/feature-x");
    }

    #[test]
    fn hidden_ref_names_match_session_namespace() {
        let session = SessionName::try_from("feature-x").expect("session");
        let refs = hidden_ref_names(&session);

        assert_eq!(refs.base, "refs/agbranch/sessions/feature-x/base");
        assert_eq!(refs.head, "refs/agbranch/sessions/feature-x/head");
    }

    #[test]
    fn explicit_base_ref_wins_over_current_branch() {
        let resolved = resolve_base_ref(Some("agbranch/other"), "refs/heads/main");
        assert_eq!(resolved, "agbranch/other");
    }

    #[test]
    fn review_branch_name_is_fast_forward_target() {
        let session = SessionName::try_from("feature-x").expect("session");
        let refs = hidden_ref_names(&session);

        assert_eq!(review_branch_name(&session), "agbranch/feature-x");
        assert_eq!(refs.head, "refs/agbranch/sessions/feature-x/head");
    }

    #[derive(Default)]
    struct RecordingRunner {
        calls: RefCell<Vec<RecordedCall>>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RecordedCall {
        program: String,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        input: Option<Vec<u8>>,
    }

    impl CommandRunner for RecordingRunner {
        fn run(
            &self,
            program: &str,
            args: &[String],
            cwd: Option<&Path>,
            _env: &BTreeMap<String, String>,
        ) -> Result<CommandOutput, crate::error::process::ProcessError> {
            self.calls.borrow_mut().push(RecordedCall {
                program: program.to_owned(),
                args: args.to_vec(),
                cwd: cwd.map(Path::to_path_buf),
                input: None,
            });
            Ok(CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
            })
        }

        fn run_with_input(
            &self,
            program: &str,
            args: &[String],
            cwd: Option<&Path>,
            _env: &BTreeMap<String, String>,
            input: &[u8],
        ) -> Result<CommandOutput, crate::error::process::ProcessError> {
            self.calls.borrow_mut().push(RecordedCall {
                program: program.to_owned(),
                args: args.to_vec(),
                cwd: cwd.map(Path::to_path_buf),
                input: Some(input.to_vec()),
            });
            Ok(CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn fetch_bundle_uses_session_scoped_incoming_ref_instead_of_fetch_head() {
        let dir = tempdir().expect("tempdir");
        let repo_root = dir.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo dir");
        let bundle_path = dir.path().join("sync.bundle");
        std::fs::write(&bundle_path, b"bundle").expect("bundle file");
        let session = SessionName::try_from("feature-x").expect("session");
        let incoming_ref = incoming_ref_name(&session);
        let runner = RecordingRunner::default();

        fetch_bundle_ref(
            &runner,
            &crate::types::HostPath::new(&repo_root),
            &crate::types::HostPath::new(&bundle_path),
            "HEAD",
            &incoming_ref,
        )
        .expect("fetch bundle");

        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].program, "git");
        assert_eq!(calls[0].cwd.as_deref(), Some(repo_root.as_path()));
        assert_eq!(
            calls[0].args,
            vec![
                "fetch".to_owned(),
                "--quiet".to_owned(),
                bundle_path.display().to_string(),
                format!("HEAD:{incoming_ref}"),
            ]
        );
    }

    #[test]
    fn publish_synced_refs_uses_one_ref_transaction() {
        let runner = RecordingRunner::default();
        let repo = Path::new("/tmp/repo");
        let source = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let review = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let session = "cccccccccccccccccccccccccccccccccccccccc";

        // The generic recording runner returns empty resolutions, so exercise
        // the exact transaction format through a runner with deterministic refs.
        struct RefRunner(RecordingRunner);
        impl CommandRunner for RefRunner {
            fn run(
                &self,
                program: &str,
                args: &[String],
                cwd: Option<&Path>,
                env: &BTreeMap<String, String>,
            ) -> Result<CommandOutput, crate::error::process::ProcessError> {
                let stdout = match args {
                    [cmd, reference]
                        if cmd == "rev-parse" && reference == "refs/heads/agbranch/demo" =>
                    {
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n".to_owned()
                    }
                    _ => String::new(),
                };
                self.0.calls.borrow_mut().push(RecordedCall {
                    program: program.to_owned(),
                    args: args.to_vec(),
                    cwd: cwd.map(Path::to_path_buf),
                    input: None,
                });
                let _ = env;
                Ok(CommandOutput {
                    stdout,
                    stderr: String::new(),
                })
            }

            fn run_with_input(
                &self,
                program: &str,
                args: &[String],
                cwd: Option<&Path>,
                _env: &BTreeMap<String, String>,
                input: &[u8],
            ) -> Result<CommandOutput, crate::error::process::ProcessError> {
                self.0.calls.borrow_mut().push(RecordedCall {
                    program: program.to_owned(),
                    args: args.to_vec(),
                    cwd: cwd.map(Path::to_path_buf),
                    input: Some(input.to_vec()),
                });
                Ok(CommandOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                })
            }
        }

        let runner = RefRunner(runner);
        assert!(
            publish_synced_refs(
                &runner,
                repo,
                "agbranch/demo",
                source,
                "refs/agbranch/sessions/demo/head",
                session,
            )
            .expect("publish")
        );
        let calls = runner.0.calls.borrow();
        let transaction = calls.last().expect("transaction");
        assert_eq!(transaction.args, ["update-ref", "--stdin"]);
        let input = String::from_utf8(transaction.input.clone().expect("input")).expect("utf8");
        assert!(input.starts_with("start\n"));
        assert!(input.contains(&format!(
            "update refs/heads/agbranch/demo {source} {review}"
        )));
        assert!(input.contains(&format!(
            "update refs/agbranch/sessions/demo/head {source} {session}"
        )));
        assert!(input.ends_with("prepare\ncommit\n"));
    }

    #[test]
    fn ref_transaction_does_not_partially_publish_after_concurrent_change() {
        let dir = tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        assert!(
            Command::new("git")
                .args(["init", "-b", "main"])
                .arg(&repo)
                .status()
                .expect("git init")
                .success()
        );
        std::fs::write(repo.join("state"), "a").expect("state a");
        git_commit(&repo, "a");
        let oid_a = git_oid(&repo, "HEAD");
        git_ref(&repo, "refs/heads/agbranch/demo", &oid_a);
        git_ref(&repo, "refs/agbranch/sessions/demo/head", &oid_a);

        std::fs::write(repo.join("state"), "b").expect("state b");
        git_commit(&repo, "b");
        let oid_b = git_oid(&repo, "HEAD");
        git_ref(&repo, "refs/agbranch/sessions/demo/incoming", &oid_b);

        std::fs::write(repo.join("state"), "c").expect("state c");
        git_commit(&repo, "c");
        let oid_c = git_oid(&repo, "HEAD");
        git_ref(&repo, "refs/agbranch/sessions/demo/head", &oid_c);

        let published = publish_synced_refs(
            &crate::util::process::RealCommandRunner,
            &repo,
            "agbranch/demo",
            &oid_b,
            "refs/agbranch/sessions/demo/head",
            &oid_a,
        )
        .expect("publish result");

        assert!(!published);
        assert_eq!(git_oid(&repo, "refs/heads/agbranch/demo"), oid_a);
        assert_eq!(git_oid(&repo, "refs/agbranch/sessions/demo/head"), oid_c);
    }

    fn git_commit(repo: &Path, message: &str) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(["add", "."])
                .status()
                .expect("git add")
                .success()
        );
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(repo)
                .args([
                    "-c",
                    "user.name=agbranch-tests",
                    "-c",
                    "user.email=agbranch@example.invalid",
                    "commit",
                    "-m",
                    message,
                ])
                .status()
                .expect("git commit")
                .success()
        );
    }

    fn git_ref(repo: &Path, reference: &str, oid: &str) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(["update-ref", reference, oid])
                .status()
                .expect("git update-ref")
                .success()
        );
    }

    fn git_oid(repo: &Path, reference: &str) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["rev-parse", reference])
            .output()
            .expect("git rev-parse");
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .expect("utf8")
            .trim()
            .to_owned()
    }
}
