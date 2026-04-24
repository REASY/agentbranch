use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct ArtifactPolicy {
    patterns: Vec<String>,
}

impl ArtifactPolicy {
    pub fn load(repo_root: &Path) -> std::io::Result<Self> {
        let mut patterns = load_patterns(include_str!("../../policy/sync-excludes.txt"));
        let repo_ignore = repo_root.join(".agbranchignore");
        if repo_ignore.exists() {
            patterns.extend(load_patterns(&fs::read_to_string(repo_ignore)?));
        }
        Ok(Self { patterns })
    }

    pub fn is_excluded(&self, path: &Path) -> bool {
        let rendered = path.to_string_lossy();
        self.patterns
            .iter()
            .any(|pattern| rendered.contains(pattern))
    }
}

pub fn path_is_excluded(path: &Path) -> bool {
    ArtifactPolicy {
        patterns: load_patterns(include_str!("../../policy/sync-excludes.txt")),
    }
    .is_excluded(path)
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

pub struct FilteredSeedTree {
    path: PathBuf,
}

impl FilteredSeedTree {
    pub fn materialize(repo_root: &Path, policy: &ArtifactPolicy) -> std::io::Result<Self> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("agbranch-seed-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path)?;
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
                if source.is_dir() {
                    fs::create_dir_all(&destination)?;
                } else if source.is_file() {
                    fs::copy(&source, &destination)?;
                }
                Ok(())
            },
            &mut |_| Ok(()),
        )?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub fn scrub_tree(root: &Path, policy: &ArtifactPolicy) -> std::io::Result<()> {
    scrub_tree_inner(root, root, policy)
}

impl Drop for FilteredSeedTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
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
        if policy.is_excluded(relative) {
            if entry.file_type()?.is_dir() {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
            continue;
        }

        if entry.file_type()?.is_dir() {
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
        if policy.is_excluded(relative) {
            exclude(relative)?;
            continue;
        }
        include(relative)?;
        if entry.file_type()?.is_dir() {
            visit_tree(repo_root, &path, policy, include, exclude)?;
        }
    }
    Ok(())
}

fn load_patterns(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.trim_matches('*').trim_end_matches('/').to_owned())
        .filter(|line| !line.is_empty())
        .collect()
}
