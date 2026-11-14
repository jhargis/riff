//! Watch settings and one-shot behavior, isolated from real user configuration.
use clap::Parser;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use riff::{
    app::App,
    cli::{Cli, ViewMode},
    config::{SettingsUpdate, load_config, save_config},
    input::Input,
    patch::parse_patch,
    reload::ReloadEvent,
    terminal::persist_settings,
    theme::builtin_themes,
};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

const PATCH: &str = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n";

fn watch_update(watch: bool) -> SettingsUpdate {
    SettingsUpdate {
        watch: Some(watch),
        ..SettingsUpdate::default()
    }
}

#[test]
fn watch_is_strictly_boolean_defaults_false_and_roundtrips() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    fs::write(&path, "").unwrap();
    assert!(!load_config(Some(&path)).unwrap().watch);
    for value in [true, false, true] {
        save_config(Some(&path), &watch_update(value)).unwrap();
        assert_eq!(load_config(Some(&path)).unwrap().watch, value);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("watch = {value}\n")
        );
    }
    for value in ["'true'", "'false'", "0", "1", "[]", "{}"] {
        let text = format!("watch = {value}\n");
        fs::write(&path, &text).unwrap();
        assert!(
            load_config(Some(&path))
                .unwrap_err()
                .contains("invalid TOML")
        );
        assert!(save_config(Some(&path), &watch_update(true)).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
    }
}

#[test]
fn selective_watch_saves_preserve_comments_palettes_and_other_overrides() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("explicit.toml");
    let text = "# preferences\ntheme = 'nord' # theme\nview = 'auto'\nwrap = true\nwatch = false # automatic refresh\n\n[themes.mine]\nextends = 'sunset'\nbackground = '#010203' # custom\n";
    fs::write(&path, text).unwrap();
    let mut app = App::new(
        parse_patch(PATCH).unwrap(),
        builtin_themes(),
        1,
        ViewMode::Split,
        false,
        "test".into(),
    )
    .unwrap();
    app.set_wrap(false);
    app.configure_reload(true, false);
    persist_settings(&mut app, Some(&path));
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::Char('w'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(app.take_reload_requests().watch, Some(true));
    app.handle_reload_event(ReloadEvent::WatchChanged {
        enabled: true,
        error: None,
    });
    persist_settings(&mut app, Some(&path));
    let expected = text.replace("watch = false", "watch = true");
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    assert!(load_config(Some(&path)).unwrap().watch);
    // Ignoring a saved true value for an unreplayable source is not acceptance.
    app.reload_unavailable("stdin patch cannot be reread");
    app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::Char('w'),
        KeyModifiers::CONTROL,
    )));
    persist_settings(&mut app, Some(&path));
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    assert!(!directory.path().join("riff/config.toml").exists());
}

#[cfg(unix)]
#[test]
fn atomic_watch_update_follows_symlinks_preserves_permissions_and_rereads() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("managed.toml");
    let link = directory.path().join("config.toml");
    fs::write(&target, "watch = false # keep\nwrap = true\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    symlink("managed.toml", &link).unwrap();
    let inode = fs::metadata(&target).unwrap().ino();
    let _stale = load_config(Some(&link)).unwrap();
    fs::write(&target, "watch = false # external\nwrap = false\n").unwrap();
    save_config(Some(&link), &watch_update(true)).unwrap();
    assert_eq!(fs::read_link(&link).unwrap(), Path::new("managed.toml"));
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "watch = true # external\nwrap = false\n"
    );
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_ne!(fs::metadata(&target).unwrap().ino(), inode);
}

#[test]
fn cli_watch_precedence_and_unsupported_explicit_requests() {
    assert!(Cli::try_parse_from(["riff", "--watch", "--no-watch"]).is_err());
    for saved in [false, true] {
        for (args, expected) in [
            (vec!["riff"], saved),
            (vec!["riff", "--watch"], true),
            (vec!["riff", "--no-watch"], false),
        ] {
            let cli = Cli::try_parse_from(args).unwrap();
            assert_eq!(
                cli.effective_watch(saved, &Input::WorkingTree, true)
                    .unwrap(),
                expected
            );
        }
    }
    let cli = Cli::try_parse_from(["riff"]).unwrap();
    assert!(!cli.effective_watch(true, &Input::Stdin, true).unwrap());
    assert!(
        !cli.effective_watch(true, &Input::WorkingTree, false)
            .unwrap()
    );
    let cli = Cli::try_parse_from(["riff", "--print"]).unwrap();
    assert!(
        !cli.effective_watch(true, &Input::WorkingTree, true)
            .unwrap()
    );
    let cli = Cli::try_parse_from(["riff", "--watch"]).unwrap();
    assert!(
        cli.effective_watch(false, &Input::Stdin, true)
            .unwrap_err()
            .contains("stdin")
    );
    assert!(
        cli.effective_watch(false, &Input::WorkingTree, false)
            .unwrap_err()
            .contains("interactive")
    );
    let cli = Cli::try_parse_from(["riff", "--watch", "--print"]).unwrap();
    assert!(
        cli.effective_watch(false, &Input::WorkingTree, true)
            .unwrap_err()
            .contains("--print")
    );
}

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_riff"));
    command
        .current_dir(root)
        .env("XDG_CONFIG_HOME", root.join("isolated"))
        .env("HOME", root.join("home"))
        .env("APPDATA", root.join("appdata"))
        .env_remove("RIFF_THEME")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

#[test]
fn saved_watch_does_not_change_plain_or_ansi_printing_or_write_on_startup() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("explicit.toml");
    for color in ["always", "never"] {
        let mut baseline = None;
        for saved in [false, true] {
            let text = format!("watch = {saved} # unchanged\n");
            fs::write(&path, &text).unwrap();
            for flag in [None, Some("--no-watch")] {
                let mut command = command(directory.path());
                command
                    .args(["--print", "--color", color, "--config"])
                    .arg(&path);
                if let Some(flag) = flag {
                    command.arg(flag);
                }
                let mut child = command.spawn().unwrap();
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(PATCH.as_bytes())
                    .unwrap();
                let output = child.wait_with_output().unwrap();
                assert!(output.status.success(), "{:?}", output.stderr);
                if let Some(expected) = &baseline {
                    assert_eq!(&output.stdout, expected);
                } else {
                    baseline = Some(output.stdout);
                }
                assert_eq!(fs::read_to_string(&path).unwrap(), text);
            }
        }
    }
    assert!(!directory.path().join("isolated").exists());
}

#[test]
fn explicit_noninteractive_watch_fails_before_consuming_open_stdin() {
    let directory = tempfile::tempdir().unwrap();
    for flags in [vec!["--watch"], vec!["--watch", "--print"]] {
        let mut child = command(directory.path()).args(flags).spawn().unwrap();
        // Keep stdin open and empty; wait would hang if startup tried to read it.
        let input = child.stdin.take().unwrap();
        let output = child.wait_with_output().unwrap();
        drop(input);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("interactive TUI"));
        assert!(output.stdout.is_empty());
    }
    for flag in ["--help", "--version", "--list-themes"] {
        let output = command(directory.path())
            .args([flag, "--watch"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output.stderr);
    }
    assert!(!directory.path().join("isolated").exists());
    assert!(!directory.path().join("home").exists());
    assert!(!directory.path().join("appdata").exists());
}
