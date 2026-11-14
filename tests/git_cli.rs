//! Real Git sources exercised through the built CLI, isolated from user config.
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

use tempfile::TempDir;

fn isolated(command: &mut Command, directory: &Path) {
    command
        .current_dir(directory)
        .env("XDG_CONFIG_HOME", directory.join("config-home"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", directory.join("no-global-config"))
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("RIFF_THEME");
}

fn git(directory: &Path, arguments: &[&str]) {
    let mut command = Command::new("git");
    isolated(&mut command, directory);
    let output = command
        .args([
            "-c",
            "user.name=riff tests",
            "-c",
            "user.email=riff@example.invalid",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgSign=false",
            "-c",
            "tag.gpgSign=false",
        ])
        .args(arguments)
        .output()
        .expect("Git must be available for Git integration tests");
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn riff(directory: &Path, arguments: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_riff"));
    isolated(&mut command, directory);
    command
        .args(["--print", "--color", "never"])
        .args(arguments)
        .stdin(Stdio::null())
        .output()
        .expect("run riff")
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "riff: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.chars().all(|ch| ch == '\n' || !ch.is_control()));
    text
}

fn repository() -> TempDir {
    let directory = TempDir::new_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("target")).unwrap();
    git(directory.path(), &["init", "-q"]);
    fs::write(directory.path().join("f"), "same\n").unwrap();
    git(directory.path(), &["add", "f"]);
    git(directory.path(), &["commit", "-qm", "base"]);
    directory
}

#[test]
fn viewing_stat_dirty_unchanged_files_never_refreshes_the_index() {
    let directory = repository();
    let root = directory.path();
    git(root, &["config", "diff.autoRefreshIndex", "true"]);
    let index = root.join(".git/index");
    let before = fs::read(&index).unwrap();
    let times = fs::FileTimes::new()
        .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(60));
    fs::OpenOptions::new()
        .write(true)
        .open(root.join("f"))
        .unwrap()
        .set_times(times)
        .unwrap();
    assert_eq!(success(riff(root, &["--", "."])), "No differences.\n");
    assert_eq!(
        fs::read(&index).unwrap(),
        before,
        "viewing refreshed index stat data"
    );
    assert_eq!(success(riff(root, &["--staged"])), "No differences.\n");
    assert_eq!(fs::read(index).unwrap(), before);
}

#[test]
fn annotated_and_nested_tags_show_the_commit_patch_without_tag_headers() {
    let directory = repository();
    let root = directory.path();
    git(root, &["tag", "-a", "v1", "-m", "release"]);
    git(root, &["tag", "-a", "nested", "v1", "-m", "nested release"]);
    git(root, &["tag", "lightweight"]);
    let expected = success(riff(root, &["--commit", "HEAD"]));
    for reference in ["v1", "nested", "lightweight", "v1^{}"] {
        assert_eq!(success(riff(root, &["--commit", reference])), expected);
        assert_eq!(success(riff(root, &[reference, "--", "f"])), expected);
    }
    assert!(!riff(root, &["--commit", "HEAD:f"]).status.success());
}

#[test]
fn staged_unstaged_commit_filters_renames_binary_and_eof() {
    let directory = TempDir::new().unwrap();
    let root = directory.path();
    git(root, &["init", "-q"]);
    fs::write(root.join("source.rs"), "fn initial() {}\n").unwrap();
    fs::write(root.join("old name.txt"), "rename this unchanged file\n").unwrap();
    fs::write(root.join("eof.txt"), "before").unwrap();
    fs::write(root.join("binary.dat"), b"before\0binary").unwrap();
    git(root, &["add", "--", "."]);
    git(root, &["commit", "-qm", "initial"]);
    fs::write(root.join("source.rs"), "fn staged() {}\n").unwrap();
    fs::write(root.join("eof.txt"), "after").unwrap();
    fs::write(root.join("binary.dat"), b"after\0binary").unwrap();
    git(root, &["mv", "--", "old name.txt", "new name.txt"]);
    git(root, &["add", "--", "source.rs", "eof.txt", "binary.dat"]);
    fs::write(root.join("source.rs"), "fn unstaged() {}\n").unwrap();

    let staged = success(riff(root, &["--staged"]));
    assert!(staged.contains("fn staged()"));
    assert!(!staged.contains("fn unstaged()"));
    assert!(staged.contains("rename from old name.txt"));
    assert!(staged.contains("rename to new name.txt"));
    assert!(staged.contains("Binary files a/binary.dat and b/binary.dat differ"));
    assert_eq!(staged.matches("No newline at end of file").count(), 2);
    let working = success(riff(root, &["--", "."]));
    assert!(working.contains("- fn staged()"));
    assert!(working.contains("+ fn unstaged()"));
    assert!(!working.contains("binary.dat"));
    let filtered = success(riff(root, &["--staged", "--", "source.rs"]));
    assert!(filtered.contains("source.rs"));
    assert!(!filtered.contains("eof.txt"));
    assert!(!filtered.contains("new name.txt"));
    assert_eq!(success(riff(root, &["--", "eof.txt"])), "No differences.\n");

    git(root, &["commit", "-qm", "staged snapshot"]);
    let commit = success(riff(root, &["--commit", "HEAD"]));
    assert_eq!(commit, staged);
    assert!(!commit.contains("fn unstaged()"));
    assert_eq!(success(riff(root, &["HEAD", "--", "source.rs"])), filtered);
    // A read-only invocation must leave both working-tree contents and index intact.
    assert_eq!(
        fs::read_to_string(root.join("source.rs")).unwrap(),
        "fn unstaged() {}\n"
    );
    assert_eq!(success(riff(root, &["--staged"])), "No differences.\n");
}
