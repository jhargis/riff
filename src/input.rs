//! Obtain patch text without invoking a shell, external diff driver, or textconv.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::patch::{parse_patch, sanitize};

#[derive(Clone, Debug)]
pub enum Input {
    WorkingTree,
    Staged,
    Commit(String),
    Stdin,
    Files(PathBuf, PathBuf),
}

/// Read exactly the requested source. Source auto-selection belongs to the CLI.
/// Patch bytes are not newline-normalized: CR in hunk content is significant.
pub fn load(source: &Input, paths: &[PathBuf]) -> Result<String, String> {
    validate(source, paths)?;
    if matches!(source, Input::Stdin) {
        let mut bytes = Vec::new();
        std::io::stdin()
            .lock()
            .read_to_end(&mut bytes)
            .map_err(|e| format!("Cannot read patch from stdin: {e}"))?;
        return decode_patch(bytes, "stdin");
    }
    load_git(source, paths, None)
}

/// Reload the same resolved query in its original directory, without changing
/// the process-wide cwd or ever consuming stdin again.
pub(crate) fn load_at(
    source: &Input,
    paths: &[PathBuf],
    directory: &Path,
) -> Result<String, String> {
    validate(source, paths)?;
    if matches!(source, Input::Stdin) {
        return Err("Stdin patches cannot be reloaded or watched.".into());
    }
    load_git(source, paths, Some(directory))
}

pub(crate) fn validate(source: &Input, paths: &[PathBuf]) -> Result<(), String> {
    if !paths.is_empty() && matches!(source, Input::Stdin | Input::Files(_, _)) {
        return Err("Path filters apply only to working-tree, staged, or commit diffs; remove the filters for stdin or a two-file comparison.".into());
    }
    if matches!(source, Input::Commit(reference) if reference.is_empty()) {
        return Err("Commit reference cannot be empty; supply a commit, tag, branch, or revision expression.".into());
    }
    Ok(())
}

fn arguments(
    source: &Input,
    paths: &[PathBuf],
    modern_merges: bool,
) -> Result<Vec<OsString>, String> {
    validate(source, paths)?;
    // `diff` can refresh stat data even with optional locks disabled. Explicitly
    // disable that refresh too: viewing must leave the index byte-for-byte intact.
    // Repeated watch reloads must not execute configured fsmonitor helpers either.
    let mut args: Vec<OsString> = [
        "--no-pager",
        "--no-optional-locks",
        "-c",
        "core.quotePath=false",
        "-c",
        "diff.suppressBlankEmpty=false",
        "-c",
        "diff.autoRefreshIndex=false",
        "-c",
        "core.fsmonitor=false",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    match source {
        Input::WorkingTree | Input::Staged => {
            args.push("diff".into());
            if matches!(source, Input::Staged) {
                args.push("--cached".into());
            }
        }
        Input::Commit(_) => {
            args.extend([
                "show".into(),
                "--format=".into(),
                "--no-show-signature".into(),
            ]);
            if modern_merges {
                args.push("--diff-merges=first-parent".into());
            } else {
                // Older Git can emit separate parent patches with -m, not a
                // combined diff; --first-parent constrains merge traversal.
                args.extend(["--first-parent".into(), "-m".into()]);
            }
        }
        Input::Files(_, _) => args.extend(["diff".into(), "--no-index".into()]),
        Input::Stdin => return Err("Internal input error: stdin is not a Git source.".into()),
    }
    args.extend(
        [
            "--patch",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--submodule=short",
            "--output-indicator-new=+",
            "--output-indicator-old=-",
            "--output-indicator-context= ",
        ]
        .into_iter()
        .map(OsString::from),
    );
    if let Input::Commit(reference) = source {
        // Unlike `--`, this terminates revision-option parsing, not revisions.
        // Peel annotated (including nested) tags and require a commit object.
        // Otherwise `show --format=` still emits annotated-tag headers.
        args.extend([
            "--end-of-options".into(),
            OsString::from(format!("{reference}^{{commit}}")),
        ]);
    }
    args.push("--".into());
    if let Input::Files(left, right) = source {
        args.push(file_operand(left));
        args.push(file_operand(right));
    } else {
        args.extend(paths.iter().map(|p| p.as_os_str().to_owned()));
    }
    Ok(args)
}

fn file_operand(path: &Path) -> OsString {
    // In --no-index mode, a lone `-` means stdin even after `--`.
    // Input::Files always means filesystem operands, never implicit stdin.
    if path.as_os_str() == "-" {
        Path::new(".").join(path).into_os_string()
    } else {
        path.as_os_str().to_owned()
    }
}

fn invoke(args: &[OsString], directory: Option<&Path>) -> Result<Output, String> {
    let mut command = Command::new("git");
    // Keep capability detection and actionable errors independent of locale.
    command.args(args).env("LC_ALL", "C");
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    command.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "Cannot run Git: executable not found. Install Git and ensure it is on PATH.".into()
        } else {
            format!("Cannot run Git: {}", sanitize(&e.to_string()))
        }
    })
}

fn unsupported_merge_option(output: &Output) -> bool {
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    !output.status.success()
        && stderr.contains("diff-merges")
        && [
            "unknown option",
            "unrecognized option",
            "unrecognized argument",
            "unknown switch",
        ]
        .iter()
        .any(|phrase| stderr.contains(phrase))
}

fn load_git(source: &Input, paths: &[PathBuf], directory: Option<&Path>) -> Result<String, String> {
    let mut output = invoke(&arguments(source, paths, true)?, directory)?;
    if matches!(source, Input::Commit(_)) && unsupported_merge_option(&output) {
        output = invoke(&arguments(source, paths, false)?, directory)?;
    }
    let label = match source {
        Input::WorkingTree => "Git working-tree diff",
        Input::Staged => "Git staged diff",
        Input::Commit(_) => "Git commit diff",
        Input::Files(_, _) => "Git file comparison",
        Input::Stdin => return Err("Internal input error: stdin is not a Git source.".into()),
    };
    let is_file_diff = matches!(source, Input::Files(_, _));
    if output.status.success() {
        return decode_patch(output.stdout, label);
    }
    // --no-index returns 1 for differences, but also for some I/O failures.
    // Require a real, well-formed patch, and reject any non-warning diagnostic;
    // do not turn partial output from a failed directory comparison into success.
    if is_file_diff && output.status.code() == Some(1) && warnings_only(&output.stderr) {
        let patch = decode_patch(std::mem::take(&mut output.stdout), label)?;
        let diff = parse_patch(&patch)
            .map_err(|e| format!("{label} produced an invalid or unsupported patch: {e}"))?;
        if !diff.files.is_empty() {
            return Ok(patch);
        }
    }
    let stderr = diagnostics(&output.stderr);
    let status = output
        .status
        .code()
        .map(|c| format!("exit {c}"))
        .unwrap_or_else(|| "terminated by signal".into());
    let mut message = format!("{label} failed ({status})");
    if !stderr.is_empty() {
        message.push_str(&format!(": {stderr}"));
    }
    if stderr.contains("not a git repository") {
        message
            .push_str("\nRun riff inside a Git work tree, or use a two-file comparison / stdin.");
    } else if stderr.contains("--end-of-options") {
        message.push_str("\nThis Git version cannot safely delimit revision options; upgrade Git.");
    } else if is_file_diff && stderr.is_empty() {
        message.push_str(
            ". Git did not produce a complete patch; check that both paths exist and are readable.",
        );
    }
    Err(message)
}

fn warnings_only(bytes: &[u8]) -> bool {
    String::from_utf8_lossy(bytes).lines().all(|line| {
        let line = line.trim();
        line.is_empty() || line.starts_with("warning:") || line.starts_with("hint:")
    })
}

fn diagnostics(bytes: &[u8]) -> String {
    sanitize(String::from_utf8_lossy(bytes).replace("\r\n", "\n").trim())
}

fn decode_patch(bytes: Vec<u8>, source: &str) -> Result<String, String> {
    String::from_utf8(bytes).map_err(|_| format!(
        "{source} is not valid UTF-8. riff requires UTF-8 patch contents and file names; convert the input or use Git's binary-diff representation."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            // Keep test-created repositories inside this project, including cleanup.
            let base = option_env!("CARGO_MANIFEST_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap());
            loop {
                let path = base.join(format!(
                    "riff-input-tests-{}-{}",
                    std::process::id(),
                    NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
                ));
                match std::fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => panic!("cannot create test directory: {e}"),
                }
            }
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn has_git() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn git(directory: &Path, args: &[&str]) {
        let result = Command::new("git")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgSign=false",
                "-c",
                "merge.gpgSign=false",
                "-c",
                "merge.verifySignatures=false",
                "-c",
                "user.name=riff tests",
                "-c",
                "user.email=riff@example.invalid",
            ])
            .args(args)
            .current_dir(directory)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    fn strings(args: Vec<OsString>) -> Vec<String> {
        args.into_iter().map(|s| s.into_string().unwrap()).collect()
    }

    #[test]
    fn git_flags_and_option_boundaries() {
        let paths = [
            PathBuf::from("--output=not-an-option"),
            PathBuf::from("a file; $(false)"),
        ];
        let args = strings(arguments(&Input::WorkingTree, &paths, true).unwrap());
        for expected in [
            "--no-pager",
            "core.quotePath=false",
            "--no-optional-locks",
            "diff.autoRefreshIndex=false",
            "core.fsmonitor=false",
            "--patch",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
        ] {
            assert!(args.iter().any(|a| a == expected), "missing {expected}");
        }
        let boundary = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(
            &args[boundary + 1..],
            ["--output=not-an-option", "a file; $(false)"]
        );
        let staged = strings(arguments(&Input::Staged, &[], true).unwrap());
        assert!(staged.iter().any(|a| a == "--cached"));
    }

    #[test]
    fn references_cannot_become_options() {
        for reference in [
            "--output=/tmp/do-not-create",
            "-",
            "HEAD; echo unsafe",
            "$(false)",
        ] {
            let args = strings(arguments(&Input::Commit(reference.into()), &[], true).unwrap());
            let index = args.iter().position(|a| a == "--end-of-options").unwrap();
            assert_eq!(args[index + 1], format!("{reference}^{{commit}}"));
            assert_eq!(args[index + 2], "--");
            assert!(args.iter().any(|a| a == "--format="));
            assert!(args.iter().any(|a| a == "--diff-merges=first-parent"));
        }
        let legacy = strings(arguments(&Input::Commit("HEAD".into()), &[], false).unwrap());
        assert!(legacy.iter().any(|a| a == "-m"));
        assert!(legacy.iter().any(|a| a == "--first-parent"));
        assert!(!legacy.iter().any(|a| a.starts_with("--diff-merges")));
    }

    #[test]
    fn rejects_filters_for_non_git_sources_before_reading_stdin() {
        let paths = [PathBuf::from("a")];
        assert!(
            load(&Input::Stdin, &paths)
                .unwrap_err()
                .contains("Path filters")
        );
        assert!(
            load(&Input::Files("a".into(), "b".into()), &paths)
                .unwrap_err()
                .contains("Path filters")
        );
        assert!(arguments(&Input::Commit(String::new()), &[], true).is_err());
    }

    #[test]
    fn file_argument_names_are_not_options() {
        let source = Input::Files("--output=x".into(), "-R $(false)".into());
        let args = strings(arguments(&source, &[], true).unwrap());
        let boundary = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(&args[boundary + 1..], ["--output=x", "-R $(false)"]);
        assert!(args.iter().any(|a| a == "--no-index"));
        let dash = strings(arguments(&Input::Files("-".into(), "f".into()), &[], true).unwrap());
        assert_eq!(&dash[dash.len() - 2..], ["./-", "f"]);
    }

    #[test]
    fn compares_files_and_accepts_only_real_difference_exit_codes() {
        if !has_git() {
            return;
        }
        let dir = Directory::new();
        let left = dir.0.join("--left file");
        let right = dir.0.join("right; $(false)");
        std::fs::write(&left, "before\n").unwrap();
        std::fs::write(&right, "after\n").unwrap();
        let source = Input::Files(left.clone(), right.clone());
        let patch = load(&source, &[]).unwrap();
        // Also exercise leading hyphens as actual relative file operands.
        let relative = Input::Files("--left file".into(), "right; $(false)".into());
        assert!(!load_git(&relative, &[], Some(&dir.0)).unwrap().is_empty());
        std::fs::write(dir.0.join("-"), "literal dash file\n").unwrap();
        let dash = Input::Files("-".into(), "right; $(false)".into());
        assert!(
            load_git(&dash, &[], Some(&dir.0))
                .unwrap()
                .contains("-literal dash file")
        );
        let diff = parse_patch(&patch).unwrap();
        assert_eq!(diff.files.len(), 1);
        assert_eq!(
            (diff.files[0].additions(), diff.files[0].deletions()),
            (1, 1)
        );
        std::fs::write(&right, "before\n").unwrap();
        assert_eq!(load(&source, &[]).unwrap(), "");
        let error = load(&Input::Files(left, dir.0.join("missing")), &[]).unwrap_err();
        assert!(error.contains("failed"));
    }

    #[test]
    fn compares_binary_files_and_retains_carriage_returns() {
        if !has_git() {
            return;
        }
        let dir = Directory::new();
        let left_directory = dir.0.join("left and b");
        std::fs::create_dir(&left_directory).unwrap();
        let left = left_directory.join("part");
        let right = dir.0.join("right");
        std::fs::write(&left, b"a\0b").unwrap();
        std::fs::write(&right, b"a\0c").unwrap();
        let source = Input::Files(left.clone(), right.clone());
        let patch = load(&source, &[]).unwrap();
        let diff = parse_patch(&patch).unwrap();
        assert!(
            diff.files[0]
                .metadata
                .iter()
                .any(|m| m.contains("Binary files"))
        );
        assert!(diff.files[0].old_path.ends_with("left and b/part"));
        assert!(diff.files[0].new_path.ends_with("/right"));
        std::fs::write(&left, b"before\r\n").unwrap();
        std::fs::write(&right, b"after\r\n").unwrap();
        assert!(load(&source, &[]).unwrap().contains("-before\r\n"));
    }

    #[test]
    fn working_staged_commit_and_path_filter() {
        if !has_git() {
            return;
        }
        let dir = Directory::new();
        git(&dir.0, &["init", "-q"]);
        std::fs::write(dir.0.join("a file"), "initial\n\nlast\n").unwrap();
        std::fs::write(dir.0.join("other"), "initial\n").unwrap();
        git(&dir.0, &["add", "--", "a file", "other"]);
        git(&dir.0, &["commit", "-qm", "initial"]);
        // These nonexistent helpers make the test fail if Git ever invokes an
        // external diff or textconv, without executing any helper ourselves.
        git(
            &dir.0,
            &["config", "diff.external", "riff-forbidden-external-diff"],
        );
        git(
            &dir.0,
            &[
                "config",
                "diff.rifftest.textconv",
                "riff-forbidden-textconv",
            ],
        );
        std::fs::write(dir.0.join(".gitattributes"), "* diff=rifftest\n").unwrap();
        git(&dir.0, &["config", "diff.noprefix", "true"]);
        git(&dir.0, &["config", "diff.mnemonicPrefix", "true"]);
        git(&dir.0, &["config", "diff.suppressBlankEmpty", "true"]);
        std::fs::write(dir.0.join("a file"), "staged\n\nlast\n").unwrap();
        git(&dir.0, &["add", "--", "a file"]);
        std::fs::write(dir.0.join("other"), "working\n").unwrap();
        let staged = load_git(&Input::Staged, &[], Some(&dir.0)).unwrap();
        let staged_diff = parse_patch(&staged).unwrap();
        assert_eq!(staged_diff.files[0].display_path(), "a file");
        assert!(
            staged_diff.files[0].hunks[0]
                .lines
                .iter()
                .any(|line| line.kind == crate::patch::LineKind::Context && line.text.is_empty())
        );
        let working = load_git(&Input::WorkingTree, &[], Some(&dir.0)).unwrap();
        assert_eq!(
            parse_patch(&working).unwrap().files[0].display_path(),
            "other"
        );
        let filtered = load_git(&Input::WorkingTree, &["a file".into()], Some(&dir.0)).unwrap();
        assert!(filtered.is_empty());
        let commit = load_git(&Input::Commit("HEAD".into()), &[], Some(&dir.0)).unwrap();
        assert_eq!(parse_patch(&commit).unwrap().files.len(), 2);
        let invalid = load_git(
            &Input::Commit("--output=unexpected".into()),
            &[],
            Some(&dir.0),
        );
        assert!(invalid.is_err());
        assert!(!dir.0.join("unexpected").exists());
    }

    #[test]
    fn commit_merges_show_only_first_parent_changes() {
        if !has_git() {
            return;
        }
        let dir = Directory::new();
        git(&dir.0, &["init", "-q"]);
        std::fs::write(dir.0.join("base"), "base\n").unwrap();
        git(&dir.0, &["add", "--", "base"]);
        git(&dir.0, &["commit", "-qm", "base"]);
        git(&dir.0, &["branch", "-M", "primary"]);
        git(&dir.0, &["checkout", "-qb", "side"]);
        std::fs::write(dir.0.join("side file"), "side\n").unwrap();
        git(&dir.0, &["add", "--", "side file"]);
        git(&dir.0, &["commit", "-qm", "side"]);
        git(&dir.0, &["checkout", "-q", "primary"]);
        std::fs::write(dir.0.join("base"), "main\n").unwrap();
        git(&dir.0, &["commit", "-qam", "main"]);
        git(&dir.0, &["merge", "-q", "--no-ff", "--no-edit", "side"]);
        let patch = load_git(&Input::Commit("HEAD".into()), &[], Some(&dir.0)).unwrap();
        let diff = parse_patch(&patch).unwrap();
        assert_eq!(diff.files.len(), 1);
        assert_eq!(diff.files[0].display_path(), "side file");
    }

    #[test]
    fn diagnostics_are_terminal_safe_and_decoding_is_lossless() {
        assert_eq!(
            diagnostics(b"  fatal: \x1b[31munsafe\r\n"),
            "fatal: ^[[31munsafe"
        );
        assert!(
            decode_patch(vec![255], "stdin")
                .unwrap_err()
                .contains("UTF-8")
        );
        assert_eq!(
            decode_patch(b"line\r\n".to_vec(), "stdin").unwrap(),
            "line\r\n"
        );
        assert!(warnings_only(b"warning: line endings\n"));
        assert!(!warnings_only(b"error: unreadable file\n"));
    }
}
