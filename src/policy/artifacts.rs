use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct ArtifactPolicy {
    matcher: Gitignore,
}

impl ArtifactPolicy {
    pub fn load(repo_root: &Path) -> std::io::Result<Self> {
        let mut builder = GitignoreBuilder::new(repo_root);
        add_patterns(
            &mut builder,
            None,
            include_str!("../../policy/sync-excludes.txt"),
        )?;
        let repo_ignore = repo_root.join(".agbranchignore");
        if repo_ignore.exists() {
            add_patterns(
                &mut builder,
                Some(&repo_ignore),
                &fs::read_to_string(&repo_ignore)?,
            )?;
        }
        let matcher = builder.build().map_err(std::io::Error::other)?;
        Ok(Self { matcher })
    }

    pub fn is_excluded(&self, path: &Path) -> bool {
        self.is_excluded_with_type(path, false)
    }

    fn is_excluded_with_type(&self, path: &Path, is_dir: bool) -> bool {
        self.matcher
            .matched_path_or_any_parents(path, is_dir)
            .is_ignore()
    }
}

pub fn collect_excluded_paths(
    repo_root: &Path,
    policy: &ArtifactPolicy,
) -> std::io::Result<Vec<PathBuf>> {
    let mut excluded = Vec::new();
    visit_tree(
        repo_root,
        repo_root,
        policy,
        &mut |_| Ok(()),
        &mut |relative| {
            excluded.push(relative.to_path_buf());
            Ok(())
        },
    )?;
    Ok(excluded)
}

#[derive(Debug)]
pub struct FilteredSeedTree {
    dir: tempfile::TempDir,
}

impl FilteredSeedTree {
    pub fn materialize(repo_root: &Path, policy: &ArtifactPolicy) -> std::io::Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix("agbranch-seed-")
            .tempdir()?;
        let path = dir.path();
        let canonical_root = repo_root.canonicalize()?;
        visit_tree(
            repo_root,
            repo_root,
            policy,
            &mut |relative| {
                let source = repo_root.join(relative);
                let destination = path.join(relative);
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)?;
                }
                let source_type = fs::symlink_metadata(&source)?.file_type();
                if source_type.is_symlink() {
                    let canonical_target = source.canonicalize()?;
                    if !canonical_target.starts_with(&canonical_root) {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            format!(
                                "seed symlink `{}` resolves outside `{}`",
                                source.display(),
                                repo_root.display()
                            ),
                        ));
                    }
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(fs::read_link(&source)?, &destination)?;
                    #[cfg(not(unix))]
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "seed symlinks are only supported on Unix hosts",
                    ));
                } else if source_type.is_dir() {
                    fs::create_dir_all(&destination)?;
                } else if source_type.is_file() {
                    fs::copy(&source, &destination)?;
                }
                Ok(())
            },
            &mut |_| Ok(()),
        )?;
        Ok(Self { dir })
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

pub fn scrub_tree(root: &Path, policy: &ArtifactPolicy) -> std::io::Result<()> {
    scrub_tree_inner(root, root, policy)
}

fn scrub_tree_inner(
    repo_root: &Path,
    current: &Path,
    policy: &ArtifactPolicy,
) -> std::io::Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(repo_root)
            .map_err(std::io::Error::other)?;
        let file_type = entry.file_type()?;
        if policy.is_excluded_with_type(relative, file_type.is_dir()) {
            if file_type.is_dir() {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
            continue;
        }

        if file_type.is_dir() {
            scrub_tree_inner(repo_root, &path, policy)?;
        }
    }
    Ok(())
}

fn visit_tree(
    repo_root: &Path,
    current: &Path,
    policy: &ArtifactPolicy,
    include: &mut dyn FnMut(&Path) -> std::io::Result<()>,
    exclude: &mut dyn FnMut(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(repo_root)
            .map_err(std::io::Error::other)?;
        let file_type = entry.file_type()?;
        if policy.is_excluded_with_type(relative, file_type.is_dir()) {
            exclude(relative)?;
            continue;
        }
        include(relative)?;
        if file_type.is_dir() {
            visit_tree(repo_root, &path, policy, include, exclude)?;
        }
    }
    Ok(())
}

fn add_patterns(
    builder: &mut GitignoreBuilder,
    source: Option<&Path>,
    raw: &str,
) -> std::io::Result<()> {
    for line in raw.lines() {
        if let Err(err) = builder.add_line(source.map(Path::to_path_buf), line) {
            return Err(std::io::Error::other(err));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn materialized_seed_honors_builtin_and_repo_patterns() {
        let source = tempdir().expect("source");
        fs::create_dir_all(source.path().join("nested/target")).expect("target dir");
        fs::create_dir_all(source.path().join("nested/src")).expect("src dir");
        fs::write(source.path().join("nested/target/cache"), "generated").expect("cache");
        fs::write(source.path().join("nested/src/lib.rs"), "source").expect("source");
        fs::write(source.path().join("local.secret"), "secret").expect("secret");
        fs::write(source.path().join(".agbranchignore"), "local.secret\n").expect("ignore");

        let policy = ArtifactPolicy::load(source.path()).expect("policy");
        let filtered = FilteredSeedTree::materialize(source.path(), &policy).expect("filtered");

        assert!(!filtered.path().join("nested/target").exists());
        assert!(!filtered.path().join("local.secret").exists());
        assert!(filtered.path().join("nested/src/lib.rs").is_file());
        assert!(filtered.path().join(".agbranchignore").is_file());
    }

    #[test]
    fn matching_uses_paths_instead_of_substrings() {
        let source = tempdir().expect("source");
        fs::create_dir_all(source.path().join("retargeting")).expect("dir");
        fs::write(source.path().join("retargeting/notes"), "keep").expect("notes");

        let policy = ArtifactPolicy::load(source.path()).expect("policy");
        assert!(!policy.is_excluded(Path::new("retargeting/notes")));
    }

    #[cfg(unix)]
    #[test]
    fn materialized_seed_refuses_symlinks_outside_the_seed_root() {
        let source = tempdir().expect("source");
        let outside = tempdir().expect("outside");
        fs::write(outside.path().join("credentials"), "secret").expect("credentials");
        std::os::unix::fs::symlink(
            outside.path().join("credentials"),
            source.path().join("credentials-link"),
        )
        .expect("symlink");

        let policy = ArtifactPolicy::load(source.path()).expect("policy");
        let err = FilteredSeedTree::materialize(source.path(), &policy)
            .expect_err("external symlink must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    }
}
