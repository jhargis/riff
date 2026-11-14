//! Backend tests run in child processes so cwd and Git environment are isolated.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

use riff::{
    input::{self, Input},
    patch::{Diff, LineKind, parse_patch},
    reload::{ReloadController, ReloadEvent},
};
use tempfile::TempDir;

const WAIT: Duration = Duration::from_secs(10);

fn run_case(case: &str) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "backend_child", "--ignored", "--nocapture"])
        .env("RIFF_RELOAD_CASE", case)
        .env("RIFF_RELOAD_ROOT", root)
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config-home"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", root.join("no-global-config"));
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_EXTERNAL_DIFF",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_SHALLOW_FILE",
        "GIT_NAMESPACE",
        "GIT_TEMPLATE_DIR",
        "GIT_CEILING_DIRECTORIES",
    ] {
        command.env_remove(name);
    }
    #[cfg(unix)]
    if matches!(
        case,
        "parse_error" | "coalesce" | "edit_during_reload" | "drop_busy"
    ) {
        install_git_fixture(&mut command, root);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{case}:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
fn install_git_fixture(command: &mut Command, root: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let real_git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("git"))
        .find(|path| path.is_file())
        .unwrap()
        .canonicalize()
        .unwrap();
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let script = bin.join("git");
    // Only this fixture uses a shell; production discovery/load use safe argv.
    // Control files are OUTSIDE the watched repository, preventing feedback.
    fs::write(
        &script,
        r##"#!/bin/sh
if test -f "$RIFF_RELOAD_ROOT/override"; then
    /bin/cat "$RIFF_RELOAD_ROOT/override"
    exit 0
fi
if test -f "$RIFF_RELOAD_ROOT/hold"; then
    "$RIFF_RELOAD_REAL_GIT" "$@" > "$RIFF_RELOAD_ROOT/snapshot"
    code=$?
    : > "$RIFF_RELOAD_ROOT/started"
    while test -f "$RIFF_RELOAD_ROOT/hold"; do sleep 0.01; done
    /bin/cat "$RIFF_RELOAD_ROOT/snapshot"
    exit "$code"
fi
exec "$RIFF_RELOAD_REAL_GIT" "$@"
"##,
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    command
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("RIFF_RELOAD_REAL_GIT", real_git);
}

macro_rules! cases {
    ($($name:ident),+ $(,)?) => { $(#[test] fn $name() { run_case(stringify!($name)); })+ };
}
cases!(
    manual_files,
    watch_toggle,
    atomic_save,
    deleted_recreated,
    empty_git_diff,
    original_filters_and_cwd,
    staged_index,
    linked_worktree,
    commit_head,
    dynamic_slashed_ref,
    invalid_utf8,
    setup_error,
    drop_idle,
    git_safety,
    directory_comparison
);
#[cfg(unix)]
cases!(parse_error, coalesce, edit_during_reload, drop_busy);

#[test]
fn stdin_rejected_without_consuming_it() {
    assert!(
        ReloadController::new(Input::Stdin, vec![], String::new())
            .err()
            .unwrap()
            .contains("Stdin")
    );
    assert!(
        ReloadController::new(
            Input::Files("a".into(), "b".into()),
            vec!["filter".into()],
            String::new()
        )
        .is_err()
    );
}

#[test]
#[ignore = "subprocess entry point"]
fn backend_child() {
    let root = PathBuf::from(std::env::var_os("RIFF_RELOAD_ROOT").unwrap());
    std::env::set_current_dir(&root).unwrap();
    match std::env::var("RIFF_RELOAD_CASE").unwrap().as_str() {
        "manual_files" => check_manual_files(&root),
        "watch_toggle" => check_watch_toggle(&root),
        "atomic_save" => check_atomic_save(&root),
        "deleted_recreated" => check_deleted_recreated(&root),
        "empty_git_diff" => check_empty_git_diff(&root),
        "original_filters_and_cwd" => check_original_filters_and_cwd(&root),
        "staged_index" => check_staged_index(&root),
        "linked_worktree" => check_linked_worktree(&root),
        "commit_head" => check_commit_head(&root),
        "dynamic_slashed_ref" => check_dynamic_slashed_ref(&root),
        "invalid_utf8" => check_invalid_utf8(&root),
        "setup_error" => check_setup_error(&root),
        "drop_idle" => check_drop_idle(&root),
        "git_safety" => check_git_safety(&root),
        "directory_comparison" => check_directory_comparison(&root),
        #[cfg(unix)]
        "parse_error" => check_parse_error(&root),
        #[cfg(unix)]
        "coalesce" => check_coalesce(&root),
        #[cfg(unix)]
        "edit_during_reload" => check_edit_during_reload(&root),
        #[cfg(unix)]
        "drop_busy" => check_drop_busy(&root),
        case => panic!("unknown backend case {case}"),
    }
}

fn eventually(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !condition() {
        assert!(Instant::now() < deadline, "condition did not become ready");
        thread::sleep(Duration::from_millis(5));
    }
}

fn event(
    controller: &ReloadController,
    mut matches: impl FnMut(&ReloadEvent) -> bool,
) -> ReloadEvent {
    let mut found = None;
    eventually(|| {
        if let Some(event) = controller.try_recv().unwrap() {
            assert!(
                !matches!(event, ReloadEvent::Error(_)),
                "unexpected {event:?}"
            );
            if matches(&event) {
                found = Some(event);
            }
        }
        found.is_some()
    });
    found.unwrap()
}

fn outcome(controller: &ReloadController) -> ReloadEvent {
    let mut found = None;
    eventually(|| {
        if let Some(event) = controller.try_recv().unwrap()
            && !matches!(
                event,
                ReloadEvent::Reloading | ReloadEvent::WatchChanged { .. }
            )
        {
            found = Some(event);
        }
        found.is_some()
    });
    found.unwrap()
}

fn updated(controller: &ReloadController, expected: &str) -> Diff {
    match event(
        controller,
        |event| matches!(event, ReloadEvent::Updated(diff) if added(diff, expected)),
    ) {
        ReloadEvent::Updated(diff) => diff,
        _ => unreachable!(),
    }
}

fn added(diff: &Diff, expected: &str) -> bool {
    diff.files
        .iter()
        .flat_map(|f| &f.hunks)
        .flat_map(|h| &h.lines)
        .any(|line| line.kind == LineKind::Added && line.text == expected)
}

fn watch(controller: &ReloadController, enabled: bool) {
    controller.set_watch(enabled).unwrap();
    let result = event(controller, |event| {
        matches!(event, ReloadEvent::WatchChanged { .. })
    });
    assert!(
        matches!(result, ReloadEvent::WatchChanged { enabled: actual, error: None } if actual == enabled),
        "{result:?}"
    );
    if enabled {
        assert!(matches!(outcome(controller), ReloadEvent::Unchanged));
    }
}

fn quiet(controller: &ReloadController) {
    // Absence is asserted only after the readiness acknowledgement/outcome.
    let until = Instant::now() + Duration::from_millis(550);
    while Instant::now() < until {
        assert!(
            controller.try_recv().unwrap().is_none(),
            "unexpected reload while idle/off"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn controller(source: Input, paths: Vec<PathBuf>) -> ReloadController {
    let patch = input::load(&source, &paths).unwrap();
    parse_patch(&patch).unwrap();
    ReloadController::new(source, paths, patch).unwrap()
}

fn files(root: &Path) -> (PathBuf, PathBuf, ReloadController) {
    let directory = root.join("files");
    fs::create_dir(&directory).unwrap();
    let left = directory.join("left");
    let right = directory.join("right");
    fs::write(&left, "base\n").unwrap();
    fs::write(&right, "initial\n").unwrap();
    let reload = controller(Input::Files(left.clone(), right.clone()), vec![]);
    (left, right, reload)
}

struct Repo {
    _directory: TempDir,
    root: PathBuf,
}
impl Repo {
    fn new(root: &Path) -> Self {
        let directory = tempfile::tempdir_in(root).unwrap();
        let repo = Self {
            root: directory.path().to_path_buf(),
            _directory: directory,
        };
        repo.git(&["init", "-q"]);
        repo.write("sub/a[1].txt", "base\n");
        repo.write("other", "base\n");
        repo.git(&["add", "--", "."]);
        repo.git(&["commit", "-qm", "base"]);
        std::env::set_current_dir(&repo.root).unwrap();
        repo
    }
    fn write(&self, name: &str, text: impl AsRef<[u8]>) {
        let path = self.root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn git(&self, args: &[&str]) -> Output {
        git(&self.root, args)
    }
}

fn git(cwd: &Path, args: &[&str]) -> Output {
    let output = Command::new("git")
        .current_dir(cwd)
        .args([
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=riff tests",
            "-c",
            "user.email=riff@example.invalid",
            "-c",
            "commit.gpgSign=false",
            "-c",
            "tag.gpgSign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn check_manual_files(root: &Path) {
    let (_, right, reload) = files(root);
    quiet(&reload); // new() is truly idle
    reload.request_reload().unwrap();
    assert!(matches!(outcome(&reload), ReloadEvent::Unchanged));
    fs::write(right, "manual\n").unwrap();
    reload.request_reload().unwrap();
    updated(&reload, "manual");
    reload.request_reload().unwrap();
    assert!(matches!(outcome(&reload), ReloadEvent::Unchanged));
}

fn check_watch_toggle(root: &Path) {
    let (_, right, reload) = files(root);
    watch(&reload, true);
    fs::write(&right, "watched\n").unwrap();
    updated(&reload, "watched");
    watch(&reload, false);
    fs::write(&right, "off\n").unwrap();
    quiet(&reload);
    reload.request_reload().unwrap();
    updated(&reload, "off");
    watch(&reload, true);
    fs::write(right, "again\n").unwrap();
    updated(&reload, "again");
}

fn check_atomic_save(root: &Path) {
    let (_, right, reload) = files(root);
    watch(&reload, true);
    let replacement = right.with_extension("new");
    fs::write(&replacement, "atomic\n").unwrap();
    fs::rename(replacement, &right).unwrap();
    updated(&reload, "atomic");
    fs::write(right, "after atomic\n").unwrap();
    updated(&reload, "after atomic");
}

fn check_deleted_recreated(root: &Path) {
    let (_, right, reload) = files(root);
    watch(&reload, true);
    fs::remove_file(&right).unwrap();
    assert!(matches!(outcome(&reload), ReloadEvent::Error(_)));
    fs::write(&right, "initial\n").unwrap();
    assert!(matches!(outcome(&reload), ReloadEvent::Unchanged));
    fs::write(right, "recovered\n").unwrap();
    updated(&reload, "recovered");
}

fn check_empty_git_diff(root: &Path) {
    let repo = Repo::new(root);
    let reload = controller(Input::WorkingTree, vec![]);
    watch(&reload, true);
    repo.write("other", "new difference\n");
    updated(&reload, "new difference");
    repo.write("other", "base\n");
    let result = event(
        &reload,
        |event| matches!(event, ReloadEvent::Updated(diff) if diff.files.is_empty()),
    );
    assert!(matches!(result, ReloadEvent::Updated(_)));
}

fn check_original_filters_and_cwd(root: &Path) {
    let repo = Repo::new(root);
    std::env::set_current_dir(repo.root.join("sub")).unwrap();
    let reload = controller(Input::WorkingTree, vec![":(literal)a[1].txt".into()]);
    std::env::set_current_dir(root).unwrap();
    watch(&reload, true);
    repo.write("other", "excluded\n");
    assert!(matches!(outcome(&reload), ReloadEvent::Unchanged));
    fs::remove_file(repo.root.join("sub/a[1].txt")).unwrap();
    let deletion = event(&reload, |event| matches!(event, ReloadEvent::Updated(_)));
    assert!(
        matches!(deletion, ReloadEvent::Updated(diff) if diff.files.len() == 1 && diff.files[0].deletions() == 1)
    );
    repo.write("sub/a[1].txt", "restored path not revision\n");
    let diff = updated(&reload, "restored path not revision");
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].display_path(), "sub/a[1].txt");

    // This Input is already resolved as a path even though a revision shares
    // its name; deletion/recreation must never cause source re-selection.
    repo.write("same-name", "tracked path\n");
    repo.git(&["add", "--", "same-name"]);
    repo.git(&["commit", "-qm", "path and revision"]);
    repo.git(&["branch", "same-name"]);
    std::env::set_current_dir(&repo.root).unwrap();
    let resolved = controller(Input::WorkingTree, vec![":(literal)same-name".into()]);
    watch(&resolved, true);
    fs::remove_file(repo.root.join("same-name")).unwrap();
    assert!(
        matches!(event(&resolved, |e| matches!(e, ReloadEvent::Updated(_))),
        ReloadEvent::Updated(diff) if diff.files.len() == 1 && diff.files[0].deletions() == 1)
    );
    repo.write("same-name", "still a path\n");
    updated(&resolved, "still a path");
}

fn check_staged_index(root: &Path) {
    let repo = Repo::new(root);
    let reload = controller(Input::Staged, vec!["other".into()]);
    watch(&reload, true);
    repo.write("other", "new staged\n");
    repo.git(&["add", "--", "other"]); // Git replaces index.lock -> index
    updated(&reload, "new staged");
    repo.write("other", "unstaged not included\n");
    assert!(matches!(outcome(&reload), ReloadEvent::Unchanged));
    repo.git(&["add", "--", "other"]);
    updated(&reload, "unstaged not included");
}

fn check_linked_worktree(root: &Path) {
    let repo = Repo::new(root);
    let linked = root.join("linked");
    repo.git(&[
        "worktree",
        "add",
        "-qb",
        "feature/linked",
        linked.to_str().unwrap(),
    ]);
    std::env::set_current_dir(&linked).unwrap();
    let reload = controller(Input::Staged, vec![]);
    watch(&reload, true);
    fs::write(linked.join("other"), "linked staged\n").unwrap();
    assert!(matches!(outcome(&reload), ReloadEvent::Unchanged));
    git(&linked, &["add", "--", "other"]);
    updated(&reload, "linked staged");
    drop(reload);
    let reload = controller(Input::Commit("HEAD".into()), vec!["other".into()]);
    watch(&reload, true);
    git(&linked, &["commit", "-qm", "linked update"]);
    updated(&reload, "linked staged");
    fs::write(linked.join("other"), "next linked\n").unwrap();
    git(&linked, &["commit", "-qam", "next linked"]);
    updated(&reload, "next linked");
    drop(reload);

    // Change only a common-directory ref from the original worktree. Nothing
    // under the linked working tree (or its private index/HEAD) is touched.
    repo.git(&["branch", "feature/external"]);
    let common = controller(
        Input::Commit("feature/external".into()),
        vec!["other".into()],
    );
    watch(&common, true);
    let head = git(&linked, &["rev-parse", "HEAD"]);
    let head = String::from_utf8(head.stdout).unwrap();
    repo.git(&["branch", "-f", "feature/external", head.trim()]);
    updated(&common, "next linked");
}

fn check_commit_head(root: &Path) {
    let repo = Repo::new(root);
    let reload = controller(Input::Commit("HEAD".into()), vec!["other".into()]);
    watch(&reload, true);
    repo.write("other", "new head\n");
    repo.git(&["commit", "-qam", "new head"]);
    updated(&reload, "new head");
    repo.write("other", "working only\n");
    assert!(matches!(outcome(&reload), ReloadEvent::Unchanged));
}

fn check_dynamic_slashed_ref(root: &Path) {
    let repo = Repo::new(root);
    repo.git(&["branch", "feature/slashed"]);
    repo.git(&["tag", "release/slashed"]);
    let branch = controller(
        Input::Commit("feature/slashed".into()),
        vec!["other".into()],
    );
    let tag = controller(
        Input::Commit("release/slashed".into()),
        vec!["other".into()],
    );
    watch(&branch, true);
    watch(&tag, true);
    repo.write("other", "ref moved\n");
    repo.git(&["commit", "-qam", "next"]);
    repo.git(&["branch", "-f", "feature/slashed", "HEAD"]);
    repo.git(&["tag", "-f", "release/slashed", "HEAD"]);
    updated(&branch, "ref moved");
    updated(&tag, "ref moved");
    repo.git(&["pack-refs", "--all", "--prune"]);
    assert!(matches!(outcome(&branch), ReloadEvent::Unchanged));
    repo.write("other", "packed ref moved\n");
    repo.git(&["commit", "-qam", "next packed"]);
    repo.git(&["branch", "-f", "feature/slashed", "HEAD"]);
    updated(&branch, "packed ref moved");
}

fn check_invalid_utf8(root: &Path) {
    let (_, right, reload) = files(root);
    watch(&reload, true);
    fs::write(&right, b"invalid \xff\n").unwrap();
    assert!(matches!(outcome(&reload), ReloadEvent::Error(error) if error.contains("UTF-8")));
    fs::write(&right, "initial\n").unwrap();
    assert!(matches!(outcome(&reload), ReloadEvent::Unchanged));
    fs::write(right, "valid again\n").unwrap();
    updated(&reload, "valid again");
}

fn check_setup_error(root: &Path) {
    let directory = root.join("not-yet");
    let left = directory.join("a");
    let right = directory.join("b");
    let reload = ReloadController::new(
        Input::Files(left.clone(), right.clone()),
        vec![],
        String::new(),
    )
    .unwrap();
    reload.set_watch(true).unwrap();
    let ack = event(&reload, |event| {
        matches!(event, ReloadEvent::WatchChanged { .. })
    });
    assert!(matches!(
        ack,
        ReloadEvent::WatchChanged {
            enabled: false,
            error: Some(_)
        }
    ));
    fs::create_dir(&directory).unwrap();
    fs::write(&left, "base\n").unwrap();
    fs::write(&right, "manual after failure\n").unwrap();
    reload.request_reload().unwrap();
    updated(&reload, "manual after failure");
    watch(&reload, true);
    fs::write(right, "watch recovered\n").unwrap();
    updated(&reload, "watch recovered");
}

fn check_directory_comparison(root: &Path) {
    fs::create_dir_all(root.join("dirs/left/nested")).unwrap();
    fs::create_dir_all(root.join("dirs/right/nested")).unwrap();
    fs::write(root.join("dirs/left/nested/file"), "same\n").unwrap();
    fs::write(root.join("dirs/right/nested/file"), "same\n").unwrap();
    let reload = controller(
        Input::Files("dirs/left".into(), "dirs/right".into()),
        vec![],
    );
    watch(&reload, true);
    fs::write(root.join("dirs/right/nested/file"), "nested change\n").unwrap();
    updated(&reload, "nested change");
}

fn check_git_safety(root: &Path) {
    let repo = Repo::new(root);
    let marker = root.join("fsmonitor-ran");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let helper = root.join("fsmonitor-helper");
        fs::write(
            &helper,
            "#!/bin/sh\n: > \"$RIFF_RELOAD_ROOT/fsmonitor-ran\"\nexit 0\n",
        )
        .unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
        repo.git(&["config", "core.fsmonitor", helper.to_str().unwrap()]);
    }
    #[cfg(not(unix))]
    repo.git(&["config", "core.fsmonitor", "riff-forbidden-fsmonitor"]);
    repo.git(&["config", "diff.external", "riff-forbidden-external-diff"]);
    repo.git(&["config", "diff.fixture.textconv", "riff-forbidden-textconv"]);
    repo.git(&["config", "diff.autoRefreshIndex", "true"]);
    repo.write(".gitattributes", "* diff=fixture\n");
    let before = fs::read(repo.root.join(".git/index")).unwrap();
    let reload = controller(Input::WorkingTree, vec![]);
    watch(&reload, true);
    for index in 0..3 {
        repo.write("other", format!("safe {index}\n"));
        updated(&reload, &format!("safe {index}"));
        assert_eq!(fs::read(repo.root.join(".git/index")).unwrap(), before);
    }
    // Reads by Git or by another reader must not feed the watcher back.
    fs::read(repo.root.join("other")).unwrap();
    fs::read(repo.root.join(".git/index")).unwrap();
    quiet(&reload);
    assert!(
        !marker.exists(),
        "viewing executed a configured fsmonitor helper"
    );
}

#[cfg(target_os = "linux")]
fn resources() -> (usize, usize) {
    let threads = fs::read_dir("/proc/self/task").unwrap().count();
    let watchers = fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| fs::read_link(entry.path()).ok())
        .filter(|path| path.as_os_str() == "anon_inode:inotify")
        .count();
    (threads, watchers)
}

fn check_drop_idle(root: &Path) {
    #[cfg(target_os = "linux")]
    let baseline = resources();
    let (_, _, reload) = files(root);
    watch(&reload, true);
    let start = Instant::now();
    drop(reload);
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "Drop blocked the caller"
    );
    #[cfg(target_os = "linux")]
    eventually(|| {
        let now = resources();
        now.0 <= baseline.0 && now.1 <= baseline.1
    });
}

#[cfg(unix)]
fn check_parse_error(root: &Path) {
    let repo = Repo::new(root);
    repo.write("other", "good\n");
    let initial = input::load(&Input::WorkingTree, &[]).unwrap();
    let reload = controller(Input::WorkingTree, vec![]);
    watch(&reload, true);
    fs::write(
        root.join("override"),
        "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-invalid\n",
    )
    .unwrap();
    repo.write("other", "invalid fixture output\n");
    assert!(matches!(outcome(&reload), ReloadEvent::Error(error) if error.contains("parse")));
    // Seeing the identical malformed output again is another error, not an
    // unchanged result: a failed parse must never advance the baseline.
    reload.request_reload().unwrap();
    assert!(matches!(outcome(&reload), ReloadEvent::Error(error) if error.contains("parse")));
    fs::write(root.join("override"), initial).unwrap();
    repo.write("other", "good\n");
    assert!(matches!(outcome(&reload), ReloadEvent::Unchanged));
    fs::remove_file(root.join("override")).unwrap();
    repo.write("other", "after parse error\n");
    updated(&reload, "after parse error");
}

#[cfg(unix)]
fn hold(root: &Path) {
    fs::write(root.join("hold"), "").unwrap();
}
#[cfg(unix)]
fn started(root: &Path) {
    eventually(|| root.join("started").exists());
}
#[cfg(unix)]
fn release(root: &Path) {
    fs::remove_file(root.join("hold")).unwrap();
}

#[cfg(unix)]
fn check_coalesce(root: &Path) {
    let repo = Repo::new(root);
    let reload = controller(Input::WorkingTree, vec![]);
    hold(root);
    repo.write("other", "first snapshot\n");
    reload.request_reload().unwrap();
    started(root);
    let start = Instant::now();
    for _ in 0..10_000 {
        reload.request_reload().unwrap();
    }
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "requests blocked on the loader"
    );
    repo.write("other", "final snapshot\n");
    release(root);
    updated(&reload, "first snapshot");
    updated(&reload, "final snapshot");
    quiet(&reload); // exactly one follow-up, not 10,000 queued loads
}

#[cfg(unix)]
fn check_edit_during_reload(root: &Path) {
    let repo = Repo::new(root);
    let reload = controller(Input::WorkingTree, vec![]);
    watch(&reload, true);
    hold(root);
    repo.write("other", "first edit\n");
    started(root);
    for index in 0..50 {
        repo.write("other", format!("burst {index}\n"));
    }
    repo.write("other", "final edit\n");
    release(root);
    updated(&reload, "first edit");
    updated(&reload, "final edit");
    quiet(&reload);
}

#[cfg(unix)]
fn check_drop_busy(root: &Path) {
    #[cfg(target_os = "linux")]
    let baseline = resources();
    let repo = Repo::new(root);
    let reload = controller(Input::WorkingTree, vec![]);
    watch(&reload, true);
    hold(root);
    repo.write("other", "blocked helper\n");
    started(root);
    let start = Instant::now();
    drop(reload);
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "Drop waited for Git"
    );
    #[cfg(target_os = "linux")]
    eventually(|| resources().1 <= baseline.1);
    release(root); // the documented uncancellable loader can now finish
    #[cfg(target_os = "linux")]
    eventually(|| resources().0 <= baseline.0);
}
