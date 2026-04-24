use crate::error::process::ProcessError;
use crate::util::process::CommandRunner;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitIdentity {
    pub name: String,
    pub email: String,
}

impl GitIdentity {
    pub fn new(name: &str, email: &str) -> Self {
        Self {
            name: name.to_owned(),
            email: email.to_owned(),
        }
    }
}

pub fn resolve_identity(
    repo_local: Option<GitIdentity>,
    global: Option<GitIdentity>,
) -> Option<GitIdentity> {
    repo_local.or(global)
}

pub fn detect_identity(
    runner: &dyn CommandRunner,
    repo_root: &Path,
) -> Result<Option<GitIdentity>, ProcessError> {
    let repo_local = read_identity_scope(runner, Some(repo_root), &["config", "--get"])?;
    let global = read_identity_scope(runner, None, &["config", "--global", "--get"])?;

    let name = repo_local.name.or(global.name);
    let email = repo_local.email.or(global.email);

    Ok(match (name, email) {
        (Some(name), Some(email)) => Some(GitIdentity { name, email }),
        _ => None,
    })
}

#[derive(Debug, Default)]
struct PartialGitIdentity {
    name: Option<String>,
    email: Option<String>,
}

fn read_identity_scope(
    runner: &dyn CommandRunner,
    cwd: Option<&Path>,
    prefix: &[&str],
) -> Result<PartialGitIdentity, ProcessError> {
    Ok(PartialGitIdentity {
        name: git_config_value(runner, cwd, prefix, "user.name")?,
        email: git_config_value(runner, cwd, prefix, "user.email")?,
    })
}

fn git_config_value(
    runner: &dyn CommandRunner,
    cwd: Option<&Path>,
    prefix: &[&str],
    key: &str,
) -> Result<Option<String>, ProcessError> {
    let env = BTreeMap::new();
    let mut args = prefix
        .iter()
        .map(|arg| (*arg).to_owned())
        .collect::<Vec<_>>();
    args.push(key.to_owned());

    match runner.run("git", &args, cwd, &env) {
        Ok(output) => match output.stdout.trim() {
            "" => Ok(None),
            value => Ok(Some(value.to_owned())),
        },
        Err(ProcessError::Failed { .. }) => Ok(None),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::process::CommandOutput;
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[test]
    fn repo_identity_wins_over_global_identity() {
        let identity = resolve_identity(
            Some(GitIdentity::new("Repo User", "repo@example.com")),
            Some(GitIdentity::new("Global User", "global@example.com")),
        )
        .expect("identity");

        assert_eq!(identity.name, "Repo User");
        assert_eq!(identity.email, "repo@example.com");
    }

    #[derive(Default)]
    struct StubRunner {
        by_scope: HashMap<(Option<PathBuf>, String), String>,
    }

    impl StubRunner {
        fn with_value(mut self, cwd: Option<&Path>, key: &str, value: &str) -> Self {
            self.by_scope.insert(
                (cwd.map(Path::to_path_buf), key.to_owned()),
                value.to_owned(),
            );
            self
        }
    }

    impl CommandRunner for StubRunner {
        fn run(
            &self,
            program: &str,
            args: &[String],
            cwd: Option<&Path>,
            _env: &BTreeMap<String, String>,
        ) -> Result<CommandOutput, crate::error::process::ProcessError> {
            assert_eq!(program, "git");
            let key = args.last().expect("config key").to_owned();
            match self.by_scope.get(&(cwd.map(Path::to_path_buf), key)) {
                Some(value) => Ok(CommandOutput {
                    stdout: format!("{value}\n"),
                    stderr: String::new(),
                }),
                None => Err(crate::error::process::ProcessError::Failed {
                    program: program.to_owned(),
                    status: 1,
                    stderr: "missing".to_owned(),
                }),
            }
        }
    }

    #[test]
    fn detect_identity_merges_repo_and_global_fields_per_key() {
        let repo = Path::new("/tmp/repo");
        let runner = StubRunner::default()
            .with_value(None, "user.name", "Global User")
            .with_value(None, "user.email", "global@example.com")
            .with_value(Some(repo), "user.email", "repo@example.com");

        let identity = detect_identity(&runner, repo)
            .expect("detect identity")
            .expect("identity");

        assert_eq!(identity.name, "Global User");
        assert_eq!(identity.email, "repo@example.com");
    }
}
