use agbranch::git::diff::{PatchChangeKind, PatchEntry, PatchEntryKind, export_patch_from_entries};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::thread;
use tempfile::tempdir;

#[test]
fn export_patch_rewrites_headers_to_repo_relative_paths() {
    let dir = tempdir().expect("tempdir");
    let host = dir.path().join("host");
    let staging = dir.path().join("staging");
    fs::create_dir_all(host.join("src")).expect("host src dir");
    fs::create_dir_all(staging.join("src")).expect("staging src dir");
    fs::write(host.join("src/lib.rs"), "pub fn version() -> u8 { 1 }\n").expect("host file");
    fs::write(staging.join("src/lib.rs"), "pub fn version() -> u8 { 2 }\n").expect("staging file");
    init_git_repo(&host);

    let patch_path = dir.path().join("feat-a.patch");
    let entries = vec![PatchEntry {
        relative_path: Path::new("src/lib.rs").to_path_buf(),
        change: PatchChangeKind::Modify,
        kind: PatchEntryKind::File,
    }];
    export_patch_from_entries(&host, &staging, &entries, &patch_path).expect("patch export");

    let patch = fs::read_to_string(&patch_path).expect("patch contents");
    assert!(patch.contains("--- a/src/lib.rs"));
    assert!(patch.contains("+++ b/src/lib.rs"));
    assert!(!patch.contains(host.to_string_lossy().as_ref()));
    assert!(!patch.contains(staging.to_string_lossy().as_ref()));
    assert!(!patch.contains(".git/"));

    let status = Command::new("git")
        .arg("-C")
        .arg(&host)
        .arg("apply")
        .arg("--check")
        .arg(&patch_path)
        .status()
        .expect("git apply --check");
    assert!(status.success(), "git apply --check should succeed");
}

#[test]
fn export_patch_keeps_delete_headers_repo_relative() {
    let dir = tempdir().expect("tempdir");
    let host = dir.path().join("host");
    let staging = dir.path().join("staging");
    fs::create_dir_all(host.join("src")).expect("host src dir");
    fs::create_dir_all(staging.join("src")).expect("staging src dir");
    fs::write(host.join("README.md"), "before\n").expect("host readme");
    fs::write(staging.join("README.md"), "before\nafter\n").expect("staging readme");
    fs::write(host.join("src/lib.rs"), "pub fn present() {}\n").expect("host lib");
    init_git_repo(&host);

    let patch_path = dir.path().join("blocked.patch");
    let entries = vec![
        PatchEntry {
            relative_path: Path::new("README.md").to_path_buf(),
            change: PatchChangeKind::Modify,
            kind: PatchEntryKind::File,
        },
        PatchEntry {
            relative_path: Path::new("src/lib.rs").to_path_buf(),
            change: PatchChangeKind::Delete,
            kind: PatchEntryKind::File,
        },
    ];
    export_patch_from_entries(&host, &staging, &entries, &patch_path).expect("patch export");

    let patch = fs::read_to_string(&patch_path).expect("patch contents");
    assert!(patch.contains("diff --git a/src/lib.rs b/src/lib.rs"));
    assert!(!patch.contains("b/old/src/lib.rs"));
    assert!(!patch.contains("a/new/src/lib.rs"));

    let status = Command::new("git")
        .arg("-C")
        .arg(&host)
        .arg("apply")
        .arg("--check")
        .arg(&patch_path)
        .status()
        .expect("git apply --check");
    assert!(status.success(), "git apply --check should succeed");
}

#[cfg(unix)]
#[test]
fn export_patch_preserves_repo_relative_symlink_paths() {
    let dir = tempdir().expect("tempdir");
    let host = dir.path().join("host");
    let staging = dir.path().join("staging");
    fs::create_dir_all(host.join("src")).expect("host src");
    fs::create_dir_all(staging.join("src")).expect("staging src");
    fs::write(host.join("src/lib.rs"), "pub fn current() {}\n").expect("host file");
    fs::write(staging.join("src/lib.rs"), "pub fn current() {}\n").expect("staging file");
    std::os::unix::fs::symlink("lib.rs", staging.join("src/current-link")).expect("symlink");
    init_git_repo(&host);

    let patch_path = dir.path().join("symlink.patch");
    let entries = vec![PatchEntry {
        relative_path: Path::new("src/current-link").to_path_buf(),
        change: PatchChangeKind::Add,
        kind: PatchEntryKind::Symlink,
    }];

    export_patch_from_entries(&host, &staging, &entries, &patch_path).expect("patch");

    let patch = fs::read_to_string(&patch_path).expect("patch contents");
    assert!(patch.contains("diff --git a/src/current-link b/src/current-link"));
    assert!(patch.contains("new file mode 120000"));
    assert!(patch.contains("+++ b/src/current-link"));
    assert!(!patch.contains(host.to_string_lossy().as_ref()));
    assert!(!patch.contains(staging.to_string_lossy().as_ref()));

    let status = Command::new("git")
        .arg("-C")
        .arg(&host)
        .arg("apply")
        .arg("--check")
        .arg(&patch_path)
        .status()
        .expect("git apply --check");
    assert!(status.success(), "git apply --check should succeed");
}

#[test]
fn export_patch_parallel_exports_do_not_share_scratch_state() {
    let dir = tempdir().expect("tempdir");
    let barrier = Arc::new(Barrier::new(16));
    let mut handles = Vec::new();

    for i in 0..16 {
        let root = dir.path().join(format!("case-{i}"));
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let host = root.join("host");
            let staging = root.join("staging");
            fs::create_dir_all(host.join("src")).expect("host src dir");
            fs::create_dir_all(staging.join("src")).expect("staging src dir");
            fs::write(
                host.join("src/lib.rs"),
                format!("pub fn version() -> u8 {{ {i} }}\n"),
            )
            .expect("host file");
            fs::write(
                staging.join("src/lib.rs"),
                format!("pub fn version() -> u8 {{ {} }}\n", i + 1),
            )
            .expect("staging file");
            init_git_repo(&host);

            let patch_path = root.join("parallel.patch");
            let entries = vec![PatchEntry {
                relative_path: Path::new("src/lib.rs").to_path_buf(),
                change: PatchChangeKind::Modify,
                kind: PatchEntryKind::File,
            }];

            barrier.wait();
            export_patch_from_entries(&host, &staging, &entries, &patch_path).expect("patch");

            let patch = fs::read_to_string(&patch_path).expect("patch contents");
            assert!(patch.contains("--- a/src/lib.rs"));
            assert!(patch.contains("+++ b/src/lib.rs"));
            assert!(
                !patch.contains("README.md"),
                "parallel patch should not contain another test's file headers: {patch}"
            );

            let status = Command::new("git")
                .arg("-C")
                .arg(&host)
                .arg("apply")
                .arg("--check")
                .arg(&patch_path)
                .status()
                .expect("git apply --check");
            assert!(status.success(), "git apply --check should succeed");
        }));
    }

    for handle in handles {
        handle.join().expect("parallel export thread");
    }
}

fn init_git_repo(path: &Path) {
    let status = Command::new("git")
        .arg("init")
        .arg(path)
        .status()
        .expect("git init");
    assert!(status.success(), "git init should succeed");
}
