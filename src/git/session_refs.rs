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

pub fn update_ref(
    runner: &dyn CommandRunner,
    repo_root: &Path,
    reference: &str,
    oid: &str,
) -> Result<(), crate::error::process::ProcessError> {
    runner.run(
        "git",
        &[
            "update-ref".to_owned(),
            reference.to_owned(),
            oid.to_owned(),
        ],
        Some(repo_root),
        &BTreeMap::new(),
    )?;
    Ok(())
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

pub fn fast_forward_review_branch(
    runner: &dyn CommandRunner,
    repo_root: &Path,
    review_branch: &str,
    source_ref: &str,
) -> Result<bool, crate::error::process::ProcessError> {
    let review_ref = format!("refs/heads/{review_branch}");
    if !ref_exists(runner, repo_root, &review_ref)? {
        update_ref(
            runner,
            repo_root,
            &review_ref,
            &resolve_ref_oid(runner, repo_root, source_ref)?,
        )?;
        return Ok(true);
    }

    let review_oid = resolve_ref_oid(runner, repo_root, &review_ref)?;
    let source_oid = resolve_ref_oid(runner, repo_root, source_ref)?;
    if !is_ancestor(runner, repo_root, &review_oid, &source_oid)? {
        return Ok(false);
    }

    update_ref(runner, repo_root, &review_ref, &source_oid)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::bundle::fetch_bundle_ref;
    use crate::util::process::CommandOutput;
    use std::cell::RefCell;
    use std::path::PathBuf;
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
}
