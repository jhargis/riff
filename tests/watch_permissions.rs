//! Native-event regressions with subprocess-local cwd and Git configuration.
#![cfg(unix)]
use riff::{
    input::{self, Input},
    patch::{Diff, LineKind},
    reload::{ReloadController, ReloadEvent},
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

fn run_case(case: &str) {
    let temporary = tempfile::tempdir().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    if case == "discovery_race" {
        discovery_fixture(&mut command, temporary.path());
    }
    let output = command
        .args(["--exact", "permissions_child", "--ignored", "--nocapture"])
        .current_dir(temporary.path())
        .env("RIFF_WATCH_PERMISSION_CASE", case)
        .env("HOME", temporary.path().join("home"))
        .env("XDG_CONFIG_HOME", temporary.path().join("config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{case}:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

macro_rules! cases {
    ($($name:ident => $case:literal),+ $(,)?) => { $(
        #[test] fn $name() { run_case($case); }
    )+ };
}
cases!(
    ignored_unreadable_tree_allows_watch_and_updates => "ignored",
    regular_files_do_not_visit_siblings_and_follow_link_targets => "files",
    tracked_directory_deletion_recreation_and_replacement => "recreated",
    index_introduces_new_watched_directory => "new_index",
    explicitly_tracked_ignored_files_are_still_watched => "force_tracked",
    required_unreadable_inputs_fail_nonfatally => "required",
    selected_directory_comparison_does_not_visit_siblings => "directories",
    external_index_parent_does_not_visit_siblings => "external_index",
    required_unreadable_git_metadata_is_not_silently_skipped => "metadata",
    bare_repository_commit_watch_still_works => "bare",
    discovery_and_reconfiguration_keep_final_edits => "discovery_race",
    initialized_submodules_watch_tracked_sources_and_metadata => "submodule",
    nested_submodules_watch_without_visiting_ignored_siblings => "nested_submodule",
    submodule_initialization_after_enable_reconciles_subscriptions => "submodule_init",
    submodule_discovery_does_not_inherit_superproject_index => "submodule_index",
    submodule_untracked_dirtiness_watches_only_nonignored_paths => "submodule_untracked",
    required_submodule_source_permissions_fail_visibly => "submodule_required",
);

#[test]
#[ignore = "subprocess entry point"]
fn permissions_child() {
    match std::env::var("RIFF_WATCH_PERMISSION_CASE")
        .unwrap()
        .as_str()
    {
        "ignored" => ignored(),
        "files" => files(),
        "recreated" => recreated(),
        "new_index" => new_index(),
        "force_tracked" => force_tracked(),
        "required" => required(),
        "directories" => directories(),
        "external_index" => external_index(),
        "index_worker" => index_worker(),
        "metadata" => metadata(),
        "bare" => bare(),
        "discovery_race" => discovery_race(),
        "submodule" => submodule(),
        "nested_submodule" => nested_submodule(),
        "submodule_init" => submodule_init(),
        "submodule_index" => submodule_index(),
        "submodule_index_worker" => submodule_index_worker(),
        "submodule_untracked" => submodule_untracked(),
        "submodule_required" => submodule_required(),
        case => panic!("unknown case {case}"),
    }
}

struct Locked(PathBuf, fs::Permissions);
impl Locked {
    fn new(path: impl AsRef<Path>) -> Option<Self> {
        let path = path.as_ref().to_path_buf();
        fs::create_dir_all(&path).unwrap();
        let original = fs::metadata(&path).unwrap().permissions();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let locked = Self(path, original);
        match fs::read_dir(&locked.0) {
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Some(locked),
            _ => {
                let uid = Command::new("id").arg("-u").output().unwrap();
                assert_eq!(
                    String::from_utf8(uid.stdout).unwrap().trim(),
                    "0",
                    "non-root must reproduce permission denial"
                );
                eprintln!("SKIP: effective root bypasses the isolated mode000 fixture");
                None
            }
        }
    }
}
impl Drop for Locked {
    fn drop(&mut self) {
        fs::set_permissions(&self.0, self.1.clone()).unwrap();
    }
}
fn git(args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .args([
            "--no-optional-locks",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=watch fixture",
            "-c",
            "user.email=watch@example.invalid",
            "-c",
            "commit.gpgSign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
fn repo() {
    git(&["init", "-q"]);
    fs::create_dir_all("src/nested").unwrap();
    fs::write("src/nested/file", "base\n").unwrap();
    fs::write(".gitignore", "ignored-cache/\nignored-tracked/\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "fixture"]);
}
fn controller(source: Input, paths: Vec<PathBuf>) -> ReloadController {
    let patch = input::load(&source, &paths).unwrap();
    ReloadController::new(source, paths, patch).unwrap()
}
fn event(controller: &ReloadController, predicate: impl Fn(&ReloadEvent) -> bool) -> ReloadEvent {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(event) = controller.try_recv().unwrap() {
            if predicate(&event) {
                return event;
            }
            assert!(
                !matches!(
                    event,
                    ReloadEvent::Error(_) | ReloadEvent::WatchChanged { error: Some(_), .. }
                ),
                "unexpected {event:?}"
            );
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for native watch event"
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn enabled(controller: &ReloadController) {
    controller.set_watch(true).unwrap();
    let ack = event(controller, |e| {
        matches!(e, ReloadEvent::WatchChanged { .. })
    });
    assert!(
        matches!(
            ack,
            ReloadEvent::WatchChanged {
                enabled: true,
                error: None
            }
        ),
        "{ack:?}"
    );
    event(controller, |e| matches!(e, ReloadEvent::Unchanged));
}
fn contains(diff: &Diff, text: &str) -> bool {
    diff.files
        .iter()
        .flat_map(|f| &f.hunks)
        .flat_map(|h| &h.lines)
        .any(|line| line.kind == LineKind::Added && line.text == text)
}
fn updated(controller: &ReloadController, text: &str) {
    event(
        controller,
        |e| matches!(e, ReloadEvent::Updated(diff) if contains(diff, text)),
    );
}
fn ignored() {
    repo();
    fs::write("src/nested/file", "initial difference\n").unwrap();
    let Some(locked) = Locked::new("ignored-cache/private") else {
        return;
    };
    git(&["check-ignore", "ignored-cache/private"]);
    let before = snapshot(&locked.0);
    let parent_before = snapshot(Path::new("ignored-cache"));
    let index = fs::read(".git/index").unwrap();
    let reload = controller(Input::WorkingTree, vec![]);
    // Exercise the real acknowledgement through the UI persistence state too.
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use riff::{app::App, cli::ViewMode, patch::parse_patch, theme::builtin_themes};
    let mut app = App::new(
        parse_patch(&input::load(&Input::WorkingTree, &[]).unwrap()).unwrap(),
        builtin_themes(),
        0,
        ViewMode::Unified,
        false,
        "fixture".into(),
    )
    .unwrap();
    app.configure_reload(true, false);
    app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::Char('w'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(app.take_reload_requests().watch, Some(true));
    assert_eq!(app.take_settings_update().watch, None);
    reload.set_watch(true).unwrap();
    let ack = event(&reload, |e| matches!(e, ReloadEvent::WatchChanged { .. }));
    assert!(
        matches!(
            &ack,
            ReloadEvent::WatchChanged {
                enabled: true,
                error: None
            }
        ),
        "{ack:?}"
    );
    app.handle_reload_event(ack);
    assert_eq!(app.take_settings_update().watch, Some(true));
    event(&reload, |e| matches!(e, ReloadEvent::Unchanged));
    fs::write("src/nested/file", "after enabled\n").unwrap();
    updated(&reload, "after enabled");
    quiet(&reload);
    assert_eq!(snapshot(&locked.0), before);
    assert_eq!(snapshot(Path::new("ignored-cache")), parent_before);
    assert_eq!(fs::read(".git/index").unwrap(), index);
}

fn snapshot(path: &Path) -> (u64, u64, u32, i64, i64, i64, i64, i64, i64) {
    use std::os::unix::fs::MetadataExt;
    let m = fs::metadata(path).unwrap();
    (
        m.dev(),
        m.ino(),
        m.mode(),
        m.atime(),
        m.atime_nsec(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    )
}

fn quiet(reload: &ReloadController) {
    // Drain bounded reconfiguration catch-up work, then ensure reads do not loop.
    let settle = Instant::now() + Duration::from_millis(400);
    while Instant::now() < settle {
        if let Some(event) = reload.try_recv().unwrap() {
            assert!(
                matches!(event, ReloadEvent::Reloading | ReloadEvent::Unchanged),
                "{event:?}"
            );
        }
        thread::sleep(Duration::from_millis(5));
    }
    let until = Instant::now() + Duration::from_millis(250);
    while Instant::now() < until {
        assert!(reload.try_recv().unwrap().is_none(), "watch/read feedback");
        thread::sleep(Duration::from_millis(5));
    }
}

fn files() {
    use std::os::unix::fs::symlink;
    fs::create_dir("pair").unwrap();
    fs::create_dir("target").unwrap();
    fs::write("pair/-left[1]", "base\n").unwrap();
    fs::write("pair/right", "initial\n").unwrap();
    fs::write("target/right", "initial\n").unwrap();
    // Git follows symlinked parent directories, but diffs a final symlink as
    // link text. Test the loader's real follow behavior, not an invented one.
    symlink("../target", "pair/link").unwrap();
    let Some(locked) = Locked::new("pair/unrelated/private") else {
        return;
    };
    let before = snapshot(&locked.0);
    let reload = controller(
        Input::Files("pair/-left[1]".into(), "pair/right".into()),
        vec![],
    );
    enabled(&reload);
    fs::write("pair/replacement", "atomic regular file\n").unwrap();
    fs::rename("pair/replacement", "pair/right").unwrap();
    updated(&reload, "atomic regular file");
    drop(reload);
    let reload = controller(
        Input::Files("pair/-left[1]".into(), "pair/link/right".into()),
        vec![],
    );
    enabled(&reload);
    fs::write("target/replacement", "atomic target\n").unwrap();
    fs::rename("target/replacement", "target/right").unwrap();
    updated(&reload, "atomic target");
    fs::write("target/right", "after target save\n").unwrap();
    updated(&reload, "after target save");
    fs::create_dir("retarget").unwrap();
    fs::write("retarget/right", "new link target\n").unwrap();
    symlink("../retarget", "pair/new-link").unwrap();
    fs::rename("pair/new-link", "pair/link").unwrap();
    updated(&reload, "new link target");
    quiet(&reload);
    fs::write("retarget/right", "after retarget\n").unwrap();
    updated(&reload, "after retarget");
    assert_eq!(snapshot(&locked.0), before);
}

fn recreated() {
    repo();
    let reload = controller(Input::WorkingTree, vec![":(literal)src/nested/file".into()]);
    enabled(&reload);
    fs::remove_dir_all("src").unwrap();
    event(
        &reload,
        |e| matches!(e, ReloadEvent::Updated(diff) if diff.files.iter().any(|f| f.deletions() > 0)),
    );
    fs::create_dir_all("src/nested").unwrap();
    fs::write("src/nested/file", "recreated\n").unwrap();
    updated(&reload, "recreated");
    quiet(&reload);
    fs::write("src/nested/file", "after recreation\n").unwrap();
    updated(&reload, "after recreation");
    // Replace an entire watched directory between debounced loads.
    fs::rename("src/nested", "src/old").unwrap();
    fs::create_dir("src/nested").unwrap();
    fs::write("src/nested/file", "replacement directory\n").unwrap();
    updated(&reload, "replacement directory");
    quiet(&reload);
    fs::write("src/nested/file", "final edit\n").unwrap();
    updated(&reload, "final edit");
}

fn new_index() {
    repo();
    let working = controller(Input::WorkingTree, vec![]);
    let staged = controller(Input::Staged, vec![]);
    enabled(&working);
    enabled(&staged);
    fs::create_dir_all("new/deep").unwrap();
    fs::write("new/deep/file", "staged addition\n").unwrap();
    git(&["add", "--", "new/deep/file"]);
    updated(&staged, "staged addition");
    // Observe a complete reload after the index replacement before the edit.
    event(&working, |e| matches!(e, ReloadEvent::Unchanged));
    quiet(&working);
    fs::write("new/deep/file", "unstaged after index\n").unwrap();
    updated(&working, "unstaged after index");
    git(&["add", "--", "new/deep/file"]);
    updated(&staged, "unstaged after index");
    git(&["rm", "-qf", "--", "new/deep/file"]);
    event(
        &working,
        |e| matches!(e, ReloadEvent::Updated(diff) if diff.files.is_empty()),
    );
    quiet(&working);
}

fn force_tracked() {
    repo();
    fs::create_dir("ignored-tracked").unwrap();
    fs::write("ignored-tracked/file", "base\n").unwrap();
    git(&["add", "-f", "--", "ignored-tracked/file"]);
    git(&["commit", "-qm", "explicitly tracked ignored input"]);
    let Some(_locked) = Locked::new("ignored-tracked/unrelated/private") else {
        return;
    };
    let reload = controller(Input::WorkingTree, vec![]);
    enabled(&reload);
    fs::write("ignored-tracked/file", "tracked despite ignore\n").unwrap();
    updated(&reload, "tracked despite ignore");
}

fn rejected(reload: &ReloadController) {
    reload.set_watch(true).unwrap();
    let ack = event(reload, |e| matches!(e, ReloadEvent::WatchChanged { .. }));
    assert!(
        matches!(ack, ReloadEvent::WatchChanged { enabled: false, error: Some(ref error) }
        if error.contains("Permission denied") && error.contains("manual reload")),
        "{ack:?}"
    );
}

fn required() {
    repo();
    let reload = controller(Input::WorkingTree, vec![]);
    let Some(locked) = Locked::new("src/nested") else {
        return;
    };
    rejected(&reload);
    drop(locked);
    fs::write("src/nested/file", "manual after failed enable\n").unwrap();
    reload.request_reload().unwrap();
    updated(&reload, "manual after failed enable");
    enabled(&reload);
    let Some(locked) = Locked::new("src/nested") else {
        return;
    };
    let ack = event(&reload, |e| {
        matches!(
            e,
            ReloadEvent::WatchChanged {
                enabled: false,
                error: Some(_)
            }
        )
    });
    assert!(
        matches!(ack, ReloadEvent::WatchChanged { error: Some(ref error), .. } if error.contains("Permission denied"))
    );
    drop(locked);
    fs::write("src/nested/file", "manual after runtime denial\n").unwrap();
    reload.request_reload().unwrap();
    updated(&reload, "manual after runtime denial");
    enabled(&reload);
    fs::write("src/nested/file", "re-enabled\n").unwrap();
    updated(&reload, "re-enabled");
}

fn directories() {
    fs::create_dir_all("compare/left/nested").unwrap();
    fs::create_dir_all("compare/right/nested").unwrap();
    fs::write("compare/left/nested/file", "base\n").unwrap();
    fs::write("compare/right/nested/file", "base\n").unwrap();
    let Some(_locked) = Locked::new("compare/unrelated/private") else {
        return;
    };
    let reload = controller(
        Input::Files("compare/left".into(), "compare/right".into()),
        vec![],
    );
    enabled(&reload);
    fs::create_dir_all("compare/right/new/deep").unwrap();
    fs::write("compare/right/new/deep/file", "new nested file\n").unwrap();
    updated(&reload, "new nested file");
    quiet(&reload);
    fs::write("compare/right/new/deep/file", "after new directory\n").unwrap();
    updated(&reload, "after new directory");
}

fn external_index() {
    repo();
    fs::create_dir("external").unwrap();
    fs::copy(".git/index", "external/index").unwrap();
    let Some(_locked) = Locked::new("external/unrelated/private") else {
        return;
    };
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "permissions_child", "--ignored", "--nocapture"])
        .env("RIFF_WATCH_PERMISSION_CASE", "index_worker")
        .env(
            "GIT_INDEX_FILE",
            fs::canonicalize("external/index").unwrap(),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn index_worker() {
    let reload = controller(Input::Staged, vec![]);
    enabled(&reload);
    fs::write("src/nested/file", "external index update\n").unwrap();
    git(&["add", "--", "src/nested/file"]);
    updated(&reload, "external index update");
    quiet(&reload);
}

fn metadata() {
    repo();
    let reload = controller(Input::WorkingTree, vec![]);
    let Some(locked) = Locked::new(".git/refs/required-private") else {
        return;
    };
    rejected(&reload);
    drop(locked);
    fs::write("src/nested/file", "manual after metadata failure\n").unwrap();
    reload.request_reload().unwrap();
    updated(&reload, "manual after metadata failure");
}

fn discovery_fixture(command: &mut Command, root: &Path) {
    let real_git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("git"))
        .find(|path| path.is_file())
        .unwrap()
        .canonicalize()
        .unwrap();
    let bin = root.join("bin");
    let control = root.join("control");
    fs::create_dir(&bin).unwrap();
    fs::create_dir(&control).unwrap();
    let wrapper = bin.join("git");
    fs::write(
        &wrapper,
        r#"#!/bin/sh
for argument in "$@"; do
    if test "$argument" = ls-files && test -f "$RIFF_WATCH_CONTROL/hold"; then
        "$RIFF_WATCH_REAL_GIT" "$@" > "$RIFF_WATCH_CONTROL/snapshot"
        code=$?
        : > "$RIFF_WATCH_CONTROL/discovered"
        while test -f "$RIFF_WATCH_CONTROL/hold"; do sleep 0.01; done
        /bin/cat "$RIFF_WATCH_CONTROL/snapshot"
        exit "$code"
    fi
done
exec "$RIFF_WATCH_REAL_GIT" "$@"
"#,
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    command
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("RIFF_WATCH_CONTROL", control)
        .env("RIFF_WATCH_REAL_GIT", real_git);
}

fn discovered(control: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !control.join("discovered").exists() {
        assert!(Instant::now() < deadline, "discovery did not start");
        thread::sleep(Duration::from_millis(5));
    }
}

struct ReleaseDiscovery(PathBuf);
impl Drop for ReleaseDiscovery {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0.join("hold"));
    }
}
fn discovery_race() {
    let root = std::env::current_dir().unwrap();
    let control = root.join("control");
    fs::create_dir("repository").unwrap();
    std::env::set_current_dir("repository").unwrap();
    repo();
    let reload = controller(Input::WorkingTree, vec![]);
    fs::write(control.join("hold"), "").unwrap();
    let _release = ReleaseDiscovery(control.clone());
    reload.set_watch(true).unwrap();
    discovered(&control);
    // Index changes AFTER discovery's snapshot and BEFORE any native watch.
    fs::create_dir_all("during-setup/deep").unwrap();
    fs::write("during-setup/deep/file", "staged during setup\n").unwrap();
    git(&["add", "--", "during-setup/deep/file"]);
    fs::remove_file(control.join("hold")).unwrap();
    let ack = event(&reload, |e| matches!(e, ReloadEvent::WatchChanged { .. }));
    assert!(
        matches!(
            ack,
            ReloadEvent::WatchChanged {
                enabled: true,
                error: None
            }
        ),
        "{ack:?}"
    );
    event(&reload, |e| matches!(e, ReloadEvent::Unchanged));
    quiet(&reload);
    fs::write("during-setup/deep/file", "after setup race\n").unwrap();
    updated(&reload, "after setup race");
    quiet(&reload);
    fs::remove_file(control.join("discovered")).unwrap();
    fs::write(control.join("hold"), "").unwrap();
    fs::create_dir_all("during-refresh/deep").unwrap();
    fs::write("during-refresh/deep/file", "staged during refresh\n").unwrap();
    git(&["add", "--", "during-refresh/deep/file"]);
    discovered(&control);
    // The first diff was loaded already, but this new directory has not been
    // subscribed yet. No event for this write can reach the native watcher.
    fs::write(
        "during-refresh/deep/file",
        "final edit across subscription gap\n",
    )
    .unwrap();
    fs::remove_file(control.join("hold")).unwrap();
    updated(&reload, "final edit across subscription gap");
    quiet(&reload);
    fs::write("during-refresh/deep/file", "after refresh race\n").unwrap();
    updated(&reload, "after refresh race");
}

fn submodule_fixture() {
    repo();
    fs::create_dir("origin").unwrap();
    let root = std::env::current_dir().unwrap();
    std::env::set_current_dir("origin").unwrap();
    repo();
    std::env::set_current_dir(root).unwrap();
    git(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        "-q",
        "--",
        "./origin",
        "modules/dependency",
    ]);
    git(&["commit", "-qm", "tracked submodule"]);
}

fn submodule_oid(path: &str) -> String {
    String::from_utf8(git(&["-C", path, "rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned()
}

fn clean_update(reload: &ReloadController) {
    event(
        reload,
        |e| matches!(e, ReloadEvent::Updated(diff) if diff.files.is_empty()),
    );
    quiet(reload);
}

fn submodule() {
    submodule_fixture();
    let path = "modules/dependency";
    let oid = submodule_oid(path);
    let Some(locked) = Locked::new(format!("{path}/ignored-cache/private")) else {
        return;
    };
    let before = snapshot(&locked.0);
    let index = fs::read(".git/index").unwrap();
    let child_index = fs::read(".git/modules/modules/dependency/index").unwrap();
    let reload = controller(Input::WorkingTree, vec![]);
    enabled(&reload);
    fs::write(format!("{path}/src/nested/file"), "native child edit\n").unwrap();
    updated(&reload, &format!("Subproject commit {oid}-dirty"));
    fs::write(format!("{path}/src/nested/file"), "base\n").unwrap();
    clean_update(&reload);
    assert_eq!(fs::read(".git/index").unwrap(), index);
    assert_eq!(
        fs::read(".git/modules/modules/dependency/index").unwrap(),
        child_index
    );
    // A metadata-only HEAD move must reload even without a working-file event.
    let tree = String::from_utf8(git(&["-C", path, "rev-parse", "HEAD^{tree}"])).unwrap();
    let next = String::from_utf8(git(&[
        "-C",
        path,
        "commit-tree",
        tree.trim(),
        "-p",
        &oid,
        "-m",
        "metadata only",
    ]))
    .unwrap();
    quiet(&reload);
    git(&["-C", path, "update-ref", "HEAD", next.trim()]);
    updated(&reload, &format!("Subproject commit {}", next.trim()));
    assert_eq!(snapshot(&locked.0), before);
}

fn nested_submodule() {
    submodule_fixture();
    let root = std::env::current_dir().unwrap();
    let origin = root.join("origin");
    git(&[
        "-C",
        "modules/dependency",
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        "-q",
        "--",
        origin.to_str().unwrap(),
        "nested/child[1]\tname",
    ]);
    git(&[
        "-C",
        "modules/dependency",
        "commit",
        "-qm",
        "nested gitlink",
    ]);
    git(&["add", "--", "modules/dependency"]);
    git(&["commit", "-qm", "updated submodule"]);
    let oid = submodule_oid("modules/dependency");
    let Some(locked) =
        Locked::new("modules/dependency/nested/child[1]\tname/ignored-cache/private")
    else {
        return;
    };
    let before = snapshot(&locked.0);
    let reload = controller(Input::WorkingTree, vec![]);
    enabled(&reload);
    fs::write(
        "modules/dependency/nested/child[1]\tname/src/nested/file",
        "deep native child edit\n",
    )
    .unwrap();
    updated(&reload, &format!("Subproject commit {oid}-dirty"));
    assert_eq!(snapshot(&locked.0), before);
}

fn submodule_init() {
    submodule_fixture();
    git(&[
        "submodule",
        "--quiet",
        "deinit",
        "-f",
        "--",
        "modules/dependency",
    ]);
    let reload = controller(Input::WorkingTree, vec![]);
    enabled(&reload);
    git(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "update",
        "--init",
        "--",
        "modules/dependency",
    ]);
    event(&reload, |e| matches!(e, ReloadEvent::Unchanged));
    quiet(&reload);
    let oid = submodule_oid("modules/dependency");
    fs::write(
        "modules/dependency/src/nested/file",
        "initialized after enable\n",
    )
    .unwrap();
    updated(&reload, &format!("Subproject commit {oid}-dirty"));
}

fn submodule_index() {
    submodule_fixture();
    fs::create_dir("external").unwrap();
    fs::copy(".git/index", "external/index").unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "permissions_child", "--ignored", "--nocapture"])
        .env("RIFF_WATCH_PERMISSION_CASE", "submodule_index_worker")
        .env(
            "GIT_INDEX_FILE",
            fs::canonicalize("external/index").unwrap(),
        )
        .env("GIT_DIR", fs::canonicalize(".git").unwrap())
        .env("GIT_WORK_TREE", std::env::current_dir().unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn submodule_untracked() {
    submodule_fixture();
    // Git's default diff ignores untracked submodule dirtiness, but an explicit
    // repository preference makes it part of the existing input query.
    git(&["config", "diff.ignoreSubmodules", "none"]);
    let path = "modules/dependency/new/deep";
    fs::create_dir_all(path).unwrap();
    fs::write(format!("{path}/file"), "untracked dirtiness\n").unwrap();
    let Some(locked) = Locked::new("modules/dependency/ignored-cache/private") else {
        return;
    };
    let before = snapshot(&locked.0);
    let reload = controller(Input::WorkingTree, vec![]);
    enabled(&reload);
    fs::remove_file(format!("{path}/file")).unwrap();
    clean_update(&reload);
    let oid = submodule_oid("modules/dependency");
    fs::create_dir_all(format!("{path}/later/nested")).unwrap();
    fs::write(format!("{path}/later/nested/file"), "new dirtiness\n").unwrap();
    updated(&reload, &format!("Subproject commit {oid}-dirty"));
    quiet(&reload);
    fs::remove_file(format!("{path}/later/nested/file")).unwrap();
    clean_update(&reload);
    assert_eq!(snapshot(&locked.0), before);
}

fn submodule_required() {
    submodule_fixture();
    let reload = controller(Input::WorkingTree, vec![]);
    let Some(locked) = Locked::new("modules/dependency/src/nested") else {
        return;
    };
    rejected(&reload);
    drop(locked);
    let oid = submodule_oid("modules/dependency");
    fs::write(
        "modules/dependency/src/nested/file",
        "manual child recovery\n",
    )
    .unwrap();
    reload.request_reload().unwrap();
    updated(&reload, &format!("Subproject commit {oid}-dirty"));
    enabled(&reload);
}

fn submodule_index_worker() {
    // The child's Git queries must not accidentally rediscover this parent's
    // index, worktree or metadata when repository-local variables are exported.
    let oid = String::from_utf8(git(&["rev-parse", "HEAD:modules/dependency"])).unwrap();
    let reload = controller(Input::WorkingTree, vec![]);
    enabled(&reload);
    fs::write(
        "modules/dependency/src/nested/file",
        "external parent index\n",
    )
    .unwrap();
    updated(&reload, &format!("Subproject commit {}-dirty", oid.trim()));
}

fn bare() {
    repo();
    fs::write("src/nested/file", "committed difference\n").unwrap();
    git(&["commit", "-qam", "difference"]);
    git(&["clone", "-q", "--bare", ".", "bare.git"]);
    std::env::set_current_dir("bare.git").unwrap();
    let reload = controller(Input::Commit("HEAD".into()), vec![]);
    enabled(&reload);
    git(&["update-ref", "HEAD", "HEAD^"]);
    event(&reload, |e| matches!(e, ReloadEvent::Updated(_)));
}
