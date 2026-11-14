//! Positional path/ref resolution through the CLI in tempfile-owned Git repositories.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

use tempfile::TempDir;

const PYTHON: &str = "sample/sample/routes.py";
const PATCH: &str =
    "--- a/piped-only.txt\n+++ b/piped-only.txt\n@@ -1 +1 @@\n-piped old\n+piped new\n";

struct Repo(TempDir);

impl Repo {
    fn new() -> Self {
        let repo = Self(tempfile::tempdir().unwrap());
        repo.git(&["init", "-q"]);
        for (path, contents) in [
            (PYTHON, "python base\n"),
            ("sample/sample/other.py", "other base\n"),
            ("a spaced directory/a file.py", "space base\n"),
            ("deleted.py", "deleted base\n"),
            ("gone/deleted.py", "gone base\n"),
            ("outside.txt", "outside base\n"),
        ] {
            repo.write(path, contents);
        }
        repo.git(&["add", "--", "."]);
        repo.git(&["commit", "-qm", "base"]);
        repo
    }

    fn root(&self) -> &Path {
        self.0.path()
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.root().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn isolate(&self, command: &mut Command, cwd: &Path) {
        assert!(cwd.starts_with(self.root()));
        command
            .current_dir(cwd)
            .env("HOME", self.root().join("home"))
            .env("XDG_CONFIG_HOME", self.root().join("config-home"))
            .env("APPDATA", self.root().join("appdata"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.root().join("no-global-config"))
            .env_remove("GIT_CONFIG_COUNT")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("RIFF_THEME");
    }

    // Git mutations are only possible within this fixture's tempfile directory.
    fn git(&self, args: &[&str]) {
        let mut command = Command::new("git");
        self.isolate(&mut command, self.root());
        let output = command
            .args([
                "--no-optional-locks",
                "-c",
                "diff.autoRefreshIndex=false",
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
            .args(args)
            .output()
            .expect("Git is required for these integration tests");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn run_from(&self, cwd: &Path, args: &[&str], stdin: Option<&str>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_riff"));
        self.isolate(&mut command, cwd);
        command
            .args(["--print", "--color", "never"])
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        let mut child = command.spawn().expect("run riff");
        if let Some(input) = stdin {
            // A rejected argument/source may exit without reading the pipe.
            let _ = child.stdin.take().unwrap().write_all(input.as_bytes());
        }
        child.wait_with_output().unwrap()
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_from(self.root(), args, Some(PATCH))
    }
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
    assert!(!text.contains("piped-only.txt"), "selected stdin, not Git");
    text
}

fn failure(output: Output) -> String {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let text = String::from_utf8(output.stderr).unwrap();
    assert!(!text.is_empty());
    assert!(text.chars().all(|ch| ch == '\n' || !ch.is_control()));
    text
}

#[test]
fn exact_nested_python_path_filters_working_tree_not_piped_stdin() {
    let repo = Repo::new();
    repo.write(PYTHON, "python working\n");
    repo.write("outside.txt", "outside working\n");
    let index = fs::read(repo.root().join(".git/index")).unwrap();
    let expected = success(repo.run(&["--", PYTHON]));
    assert!(expected.contains(PYTHON));
    assert!(expected.contains("+ python working"));
    assert!(!expected.contains("outside.txt"));
    assert_eq!(success(repo.run(&[PYTHON])), expected);
    assert_eq!(
        success(repo.run_from(repo.root(), &[PYTHON], None)),
        expected
    );
    assert_eq!(
        fs::read(repo.root().join(".git/index")).unwrap(),
        index,
        "path/ref recognition and diff must not refresh the index"
    );
}

#[test]
fn bare_directories_and_paths_with_spaces_retain_their_filters() {
    let repo = Repo::new();
    repo.write(PYTHON, "python working\n");
    repo.write("sample/sample/other.py", "other working\n");
    repo.write("a spaced directory/a file.py", "space working\n");
    repo.write("outside.txt", "outside working\n");
    for path in [
        "sample/sample",
        "a spaced directory",
        "a spaced directory/a file.py",
    ] {
        let expected = success(repo.run(&["--", path]));
        assert!(!expected.contains("No differences."));
        assert!(!expected.contains("outside.txt"));
        assert_eq!(success(repo.run(&[path])), expected);
    }
    let directory = success(repo.run(&["sample/sample"]));
    assert!(directory.contains(PYTHON));
    assert!(directory.contains("sample/sample/other.py"));
    assert!(!directory.contains("space working"));
}

#[test]
fn deleted_tracked_file_and_directory_are_still_path_filters() {
    let repo = Repo::new();
    fs::remove_file(repo.root().join("deleted.py")).unwrap();
    fs::remove_file(repo.root().join("gone/deleted.py")).unwrap();
    fs::remove_dir(repo.root().join("gone")).unwrap();
    repo.write("outside.txt", "outside working\n");
    let index = fs::read(repo.root().join(".git/index")).unwrap();
    for path in ["deleted.py", "gone/deleted.py", "gone"] {
        let expected = success(repo.run(&["--", path]));
        assert!(expected.contains("deleted.py"));
        assert!(!expected.contains("outside.txt"));
        assert_eq!(success(repo.run(&[path])), expected);
    }
    assert_eq!(fs::read(repo.root().join(".git/index")).unwrap(), index);
}

#[test]
fn paths_are_relative_to_invocation_cwd_not_repository_root() {
    let repo = Repo::new();
    repo.write(PYTHON, "python working\n");
    repo.write("sample/sample/other.py", "other working\n");
    repo.write("outside.txt", "outside working\n");
    let cwd = repo.root().join("sample");
    for path in ["sample/routes.py", "./sample/routes.py", "sample"] {
        let expected = success(repo.run_from(&cwd, &["--", path], Some(PATCH)));
        assert!(expected.contains("python working"));
        assert!(!expected.contains("outside.txt"));
        assert_eq!(success(repo.run_from(&cwd, &[path], Some(PATCH))), expected);
    }
    fs::remove_file(repo.root().join(PYTHON)).unwrap();
    let expected = success(repo.run_from(&cwd, &["--", "sample/routes.py"], Some(PATCH)));
    assert!(expected.contains("- python base"));
    assert!(!expected.contains("other.py"));
    assert_eq!(
        success(repo.run_from(&cwd, &["sample/routes.py"], Some(PATCH))),
        expected
    );
}

#[test]
fn commits_annotated_tags_slashed_branches_and_revision_syntax_still_work() {
    let repo = Repo::new();
    repo.write(PYTHON, "python committed\n");
    repo.git(&["commit", "-qam", "second"]);
    repo.git(&["tag", "-a", "release", "-m", "annotated release"]);
    repo.git(&["tag", "-a", "nested", "release", "-m", "nested release"]);
    repo.git(&["branch", "feature/admin-urls"]);
    repo.write(PYTHON, "python working\n");
    let expected = success(repo.run(&["--commit", "HEAD"]));
    assert!(expected.contains("+ python committed"));
    assert!(!expected.contains("python working"));
    for reference in [
        "HEAD",
        "release",
        "nested",
        "release^{}",
        "feature/admin-urls",
        "HEAD~0",
        "HEAD^{commit}",
        "@{0}",
    ] {
        assert_eq!(success(repo.run(&[reference])), expected, "{reference}");
        assert_eq!(
            success(repo.run(&[reference, "--", PYTHON])),
            expected,
            "{reference} with filter"
        );
    }
    assert_eq!(
        success(repo.run(&["HEAD~1"])),
        success(repo.run(&["--commit", "HEAD~1"]))
    );
    failure(repo.run(&["missing/revision"]));
    failure(repo.run(&["--commit", PYTHON]));
}

#[test]
fn explicit_filters_multiple_paths_and_staged_filters_keep_existing_meaning() {
    let repo = Repo::new();
    repo.write(PYTHON, "python staged\n");
    repo.write("outside.txt", "outside staged\n");
    repo.git(&["add", "--", PYTHON, "outside.txt"]);
    repo.write(PYTHON, "python working\n");
    repo.write("sample/sample/other.py", "other working\n");
    let staged = success(repo.run(&["--staged", "--", PYTHON]));
    assert!(staged.contains("+ python staged"));
    assert!(!staged.contains("python working"));
    assert!(!staged.contains("outside.txt"));
    let working = success(repo.run(&["--", PYTHON, "sample/sample/other.py"]));
    assert!(working.contains("+ python working"));
    assert!(working.contains("+ other working"));
    assert!(!working.contains("outside.txt"));
    assert_eq!(
        success(repo.run(&[PYTHON, "--", "sample/sample/other.py"])),
        working
    );
    // Keep these extra forms intentionally explicit, not partly interpreted.
    failure(repo.run(&[PYTHON, "sample/sample/other.py"]));
    failure(repo.run(&["--staged", PYTHON]));
}

#[test]
fn ambiguous_paths_and_revisions_require_explicit_disambiguation() {
    let repo = Repo::new();
    repo.write("HEAD", "head path base\n");
    repo.write("release", "tag path base\n");
    repo.write("feature/admin-urls/file.py", "branch directory base\n");
    repo.git(&["add", "--", "."]);
    repo.git(&["commit", "-qm", "ambiguous paths"]);
    repo.git(&["tag", "-a", "release", "-m", "release"]);
    repo.git(&["branch", "feature/admin-urls"]);
    repo.write("HEAD", "head path working\n");
    repo.write("release", "tag path working\n");
    repo.write("feature/admin-urls/file.py", "branch directory working\n");
    let commit = success(repo.run(&["--commit", "HEAD"]));
    for token in ["HEAD", "release", "feature/admin-urls"] {
        for args in [vec![token], vec![token, "--", PYTHON]] {
            let error = failure(repo.run(&args));
            assert!(error.contains("ambiguous"), "{error}");
            assert!(error.contains("--commit"), "{error}");
            assert!(error.contains("-- "), "{error}");
        }
        let path = success(repo.run(&["--", token]));
        assert!(path.contains("working"));
        assert_eq!(success(repo.run(&["--commit", token])), commit);
    }
    fs::remove_file(repo.root().join("release")).unwrap();
    assert!(failure(repo.run(&["release"])).contains("ambiguous"));
    assert!(success(repo.run(&["--", "release"])).contains("- tag path base"));
    assert_eq!(success(repo.run(&["--commit", "release"])), commit);
    let filtered = success(repo.run(&["--commit", "HEAD", "--", "HEAD"]));
    assert!(filtered.contains("+ head path base"));
    assert!(!filtered.contains("tag path"));
    assert!(!filtered.contains("working"));
}

#[test]
fn option_boundaries_and_exclusive_sources_are_not_weakened() {
    let repo = Repo::new();
    repo.write("--output=unexpected", "option path base\n");
    repo.write("-", "dash path base\n");
    repo.git(&["add", "--", "--output=unexpected", "-"]);
    repo.git(&["commit", "-qm", "option-like paths"]);
    repo.write("--output=unexpected", "option path working\n");
    repo.write("-", "dash path working\n");
    let explicit = success(repo.run(&["--", "--output=unexpected"]));
    assert!(explicit.contains("+ option path working"));
    assert_eq!(success(repo.run(&["./--output=unexpected"])), explicit);
    assert!(success(repo.run(&["./-"])).contains("+ dash path working"));
    failure(repo.run(&["--commit=--output=unexpected"]));
    failure(repo.run(&["--output=unexpected"]));
    assert!(!repo.root().join("unexpected").exists());
    for args in [
        vec!["--stdin", PYTHON],
        vec!["--files", "old", "new", PYTHON],
        vec!["--stdin", "--", PYTHON],
        vec!["--files", "old", "new", "--", PYTHON],
        vec!["--commit", "HEAD", PYTHON],
        vec!["--staged", "--commit", "HEAD"],
    ] {
        failure(repo.run(&args));
    }
}

#[test]
fn existing_bare_paths_are_literal_but_explicit_filters_remain_pathspecs() {
    let repo = Repo::new();
    for path in [
        "literal[1].txt",
        "literal1.txt",
        ":(exclude)outside.txt",
        "dir[1]/file.txt",
        "dir1/file.txt",
    ] {
        repo.write(path, "literal base\n");
    }
    repo.git(&["add", "--", "."]);
    repo.git(&["commit", "-qm", "literal paths"]);
    for path in [
        "literal[1].txt",
        "literal1.txt",
        ":(exclude)outside.txt",
        "dir[1]/file.txt",
        "dir1/file.txt",
    ] {
        repo.write(path, "literal working\n");
    }
    repo.write("outside.txt", "outside working\n");
    for path in ["literal[1].txt", ":(exclude)outside.txt", "dir[1]"] {
        let expected = success(repo.run(&["--", &format!(":(literal){path}")]));
        assert_eq!(expected.matches("── ").count(), 1, "{expected}");
        assert!(expected.contains("+ literal working"));
        assert_eq!(success(repo.run(&[path])), expected, "{path}");
        let absolute = repo.root().join(path);
        assert_eq!(success(repo.run(&[absolute.to_str().unwrap()])), expected);
    }
    let explicit_glob = success(repo.run(&["--", "literal[1].txt"]));
    assert!(explicit_glob.contains("── literal1.txt"));
    assert!(explicit_glob.contains("── literal[1].txt"));
    let explicit_exclusion = success(repo.run(&["--", ":(exclude)outside.txt"]));
    assert!(explicit_exclusion.contains("── literal1.txt"));
    assert!(!explicit_exclusion.contains("── outside.txt"));
    let combined = success(repo.run(&["literal[1].txt", "--", ":(literal)outside.txt"]));
    assert!(combined.contains("── literal[1].txt"));
    assert!(combined.contains("── outside.txt"));
    assert!(!combined.contains("── literal1.txt"));
}

#[test]
fn deleted_bare_paths_keep_literal_pathspec_characters() {
    let repo = Repo::new();
    for path in [
        "literal[1].txt",
        "literal1.txt",
        ":(exclude)outside.txt",
        "dir[1]/file.txt",
        "dir1/file.txt",
    ] {
        repo.write(path, "literal base\n");
    }
    repo.git(&["add", "--", "."]);
    repo.git(&["commit", "-qm", "literal paths"]);
    for path in ["literal[1].txt", ":(exclude)outside.txt", "dir[1]/file.txt"] {
        fs::remove_file(repo.root().join(path)).unwrap();
    }
    fs::remove_dir(repo.root().join("dir[1]")).unwrap();
    for path in ["literal1.txt", "dir1/file.txt", "outside.txt"] {
        repo.write(path, "unrelated working\n");
    }
    for path in ["literal[1].txt", ":(exclude)outside.txt", "dir[1]"] {
        let expected = success(repo.run(&["--", &format!(":(literal){path}")]));
        assert_eq!(expected.matches("── ").count(), 1, "{expected}");
        assert!(expected.contains("- literal base"));
        assert!(!expected.contains("unrelated working"));
        assert_eq!(success(repo.run(&[path])), expected, "{path}");
    }
}

#[cfg(unix)]
#[test]
fn deleted_path_recognition_does_not_execute_configured_fsmonitor() {
    use std::os::unix::fs::PermissionsExt;

    let repo = Repo::new();
    repo.git(&["branch", "deleted.py"]);
    fs::remove_file(repo.root().join("deleted.py")).unwrap();
    let marker = repo.root().join("fsmonitor-ran");
    let helper = repo.root().join("fsmonitor.sh");
    fs::write(
        &helper,
        format!(
            "#!/bin/sh\nprintf ran >> '{}'\nprintf 'token\\0'\n",
            marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
    repo.git(&["config", "core.fsmonitor", helper.to_str().unwrap()]);
    repo.git(&["config", "core.fsmonitorHookVersion", "2"]);
    let index = fs::read(repo.root().join(".git/index")).unwrap();
    // Ambiguity stops before input::load, isolating the new recognition probe.
    assert!(failure(repo.run(&["deleted.py"])).contains("ambiguous"));
    assert!(!marker.exists(), "recognition executed a configured helper");
    assert_eq!(fs::read(repo.root().join(".git/index")).unwrap(), index);
}

#[test]
fn resolution_probes_and_diff_do_not_refresh_stat_dirty_index_entries() {
    let repo = Repo::new();
    repo.git(&["config", "diff.autoRefreshIndex", "true"]);
    let index = fs::read(repo.root().join(".git/index")).unwrap();
    let times = fs::FileTimes::new()
        .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(60));
    fs::OpenOptions::new()
        .write(true)
        .open(repo.root().join(PYTHON))
        .unwrap()
        .set_times(times)
        .unwrap();
    assert_eq!(success(repo.run(&[PYTHON])), "No differences.\n");
    assert_eq!(fs::read(repo.root().join(".git/index")).unwrap(), index);
    fs::remove_file(repo.root().join("deleted.py")).unwrap();
    assert!(success(repo.run(&["deleted.py"])).contains("- deleted base"));
    assert_eq!(fs::read(repo.root().join(".git/index")).unwrap(), index);
}
