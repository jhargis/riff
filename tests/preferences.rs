//! Selective frontend persistence without touching a real user configuration.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use clap::Parser;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use riff::{
    app::App,
    cli::{Cli, ViewMode},
    config::{SettingsUpdate, load_config},
    patch::parse_patch,
    terminal::persist_settings,
    theme::{builtin_themes, find_theme},
    ui,
};

const PATCH: &str = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n";

fn app(theme: &str, view: ViewMode, wrap: bool) -> App {
    let themes = builtin_themes();
    let index = find_theme(theme, &themes).unwrap();
    let mut app = App::new(
        parse_patch(PATCH).unwrap(),
        themes,
        index,
        view,
        false,
        "test".into(),
    )
    .unwrap();
    app.resize(100, 24);
    app.set_wrap(wrap);
    app
}

fn key(app: &mut App, code: KeyCode) {
    app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

fn accept_next_theme(app: &mut App) {
    key(app, KeyCode::Char('t'));
    key(app, KeyCode::Down);
    key(app, KeyCode::Enter);
}

#[test]
fn pure_events_emit_only_explicitly_accepted_preferences_and_drain_once() {
    let mut app = app("dracula", ViewMode::Split, true);
    assert_eq!(app.take_settings_update(), SettingsUpdate::default());
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Down);
    assert_eq!(app.take_settings_update(), SettingsUpdate::default());
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.theme().name, "dracula");
    assert_eq!(app.take_settings_update(), SettingsUpdate::default());
    accept_next_theme(&mut app);
    assert_eq!(
        app.take_settings_update(),
        SettingsUpdate {
            theme: Some("catppuccin".into()),
            ..SettingsUpdate::default()
        }
    );
    assert_eq!(app.take_settings_update(), SettingsUpdate::default());
    key(&mut app, KeyCode::Char('v'));
    assert_eq!(
        app.take_settings_update(),
        SettingsUpdate {
            view: Some("unified".into()),
            ..SettingsUpdate::default()
        }
    );
    key(&mut app, KeyCode::Char('w'));
    assert_eq!(
        app.take_settings_update(),
        SettingsUpdate {
            wrap: Some(false),
            ..SettingsUpdate::default()
        }
    );
}

#[test]
fn startup_preview_cancel_and_quit_do_not_create_config_or_directories() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("not-created/config.toml");
    let mut app = app("dracula", ViewMode::Unified, true);
    persist_settings(&mut app, Some(&path));
    key(&mut app, KeyCode::Char('t'));
    key(&mut app, KeyCode::Down);
    persist_settings(&mut app, Some(&path));
    key(&mut app, KeyCode::Esc);
    persist_settings(&mut app, Some(&path));
    key(&mut app, KeyCode::Char('q'));
    persist_settings(&mut app, Some(&path));
    assert!(!path.parent().unwrap().exists());
}

#[test]
fn accepted_choices_save_immediately_and_load_in_the_next_session() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("riff/config.toml");
    let mut first = app("critique", ViewMode::Unified, false);
    accept_next_theme(&mut first);
    persist_settings(&mut first, Some(&path));
    assert_eq!(load_config(Some(&path)).unwrap().theme, "dracula");
    key(&mut first, KeyCode::Char('v'));
    persist_settings(&mut first, Some(&path));
    key(&mut first, KeyCode::Char('w'));
    persist_settings(&mut first, Some(&path));
    let settings = load_config(Some(&path)).unwrap();
    assert_eq!(settings.view.as_deref(), Some("split"));
    assert!(settings.wrap);
    let next = app(
        &settings.theme,
        ViewMode::parse_config(settings.view.as_deref().unwrap()).unwrap(),
        settings.wrap,
    );
    assert_eq!(next.theme().name, "dracula");
    assert!(next.cache.split && next.wrap);
}

#[test]
fn unrelated_one_run_overrides_are_never_written_incidentally() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("explicit.toml");
    let text = "# leave this alone\ntheme = 'nord' # saved theme\nview = 'auto'\nwrap = false\n\n[themes.mine]\nextends = 'sunset'\nbackground = '#010203' # custom\n";
    fs::write(&path, text).unwrap();
    // Pretend CLI/environment overrode all three settings for this run.
    let mut app = app("dracula", ViewMode::Split, true);
    key(&mut app, KeyCode::Char('v'));
    persist_settings(&mut app, Some(&path));
    let settings = load_config(Some(&path)).unwrap();
    assert_eq!(settings.theme, "nord");
    assert!(!settings.wrap);
    assert_eq!(settings.view.as_deref(), Some("unified"));
    accept_next_theme(&mut app);
    persist_settings(&mut app, Some(&path));
    let settings = load_config(Some(&path)).unwrap();
    assert_eq!(settings.theme, "catppuccin");
    assert!(!settings.wrap);
    assert_eq!(settings.view.as_deref(), Some("unified"));
    key(&mut app, KeyCode::Char('w'));
    persist_settings(&mut app, Some(&path));
    let saved = fs::read_to_string(&path).unwrap();
    for preserved in [
        "# leave this alone",
        "# saved theme",
        "background = '#010203' # custom",
        "[themes.mine]",
    ] {
        assert!(saved.contains(preserved), "{saved}");
    }
}

#[test]
fn malformed_config_is_not_replaced_and_errors_are_visible_safe_and_nonfatal() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bad\u{1b}[31m.toml");
    fs::write(&path, "wrap = 'not a boolean'\n").unwrap();
    let mut app = app("critique", ViewMode::Unified, false);
    key(&mut app, KeyCode::Char('w'));
    persist_settings(&mut app, Some(&path));
    assert!(app.running && app.wrap);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "wrap = 'not a boolean'\n"
    );
    let error = app.save_error.as_ref().unwrap();
    assert!(error.chars().all(|ch| !ch.is_control()));
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("Save failed:"));
    assert_eq!(app.take_settings_update(), SettingsUpdate::default());
    key(&mut app, KeyCode::Char('v'));
    assert!(app.running);
}

#[cfg(unix)]
#[test]
fn read_only_parent_failure_is_reported_without_losing_the_live_choice() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("readonly");
    fs::create_dir(&parent).unwrap();
    let path = parent.join("config.toml");
    fs::write(&path, "theme = 'critique'\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();
    let mut app = app("critique", ViewMode::Unified, false);
    accept_next_theme(&mut app);
    persist_settings(&mut app, Some(&path));
    // Restore permissions before assertions so temporary-directory cleanup works.
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(app.theme().name, "dracula");
    assert!(app.running && app.save_error.is_some());
    assert_eq!(fs::read_to_string(&path).unwrap(), "theme = 'critique'\n");
}

#[test]
fn wrap_cli_flags_are_exclusive() {
    assert!(Cli::try_parse_from(["riff", "--wrap", "--no-wrap"]).is_err());
    assert!(Cli::try_parse_from(["riff", "--wrap"]).unwrap().wrap);
    assert!(Cli::try_parse_from(["riff", "--no-wrap"]).unwrap().no_wrap);
    let cli = Cli::try_parse_from(["riff"]).unwrap();
    assert!(!cli.wrap && !cli.no_wrap);
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
fn print_help_listing_and_cli_overrides_never_write_and_print_is_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let patch = format!(
        "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+{}\n",
        "word 界 ".repeat(150)
    );
    for color in ["never", "always"] {
        let mut baseline = None;
        for flag in [None, Some("--wrap"), Some("--no-wrap")] {
            let mut command = command(directory.path());
            command.args([
                "--print", "--color", color, "--theme", "dracula", "--view", "split",
            ]);
            if let Some(flag) = flag {
                command.arg(flag);
            }
            let mut child = command.spawn().unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(patch.as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success(), "{:?}", output.stderr);
            if let Some(baseline) = &baseline {
                assert_eq!(&output.stdout, baseline);
            } else {
                baseline = Some(output.stdout);
            }
        }
    }
    for flag in ["--help", "--list-themes"] {
        let output = command(directory.path())
            .args([flag, "--wrap"])
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    assert!(!directory.path().join("isolated").exists());
    assert!(!directory.path().join("home").exists());
    assert!(!directory.path().join("appdata").exists());
}

#[test]
fn configured_wrap_does_not_change_print_or_get_rewritten_by_cli_flags() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("explicit.toml");
    let code = format!("{}END_OF_COMPLETE_LINE", "word ".repeat(200));
    let patch = format!("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+{code}\n");
    for color in ["never", "always"] {
        let mut baseline = None;
        for wrap in [false, true] {
            let original = format!("# must not change\nwrap = {wrap}\nview = 'split'\n");
            fs::write(&config, &original).unwrap();
            for flag in [None, Some("--wrap"), Some("--no-wrap")] {
                let mut command = command(directory.path());
                command
                    .args(["--print", "--color", color, "--no-syntax", "--config"])
                    .arg(&config);
                if let Some(flag) = flag {
                    command.arg(flag);
                }
                let mut child = command.spawn().unwrap();
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(patch.as_bytes())
                    .unwrap();
                let output = child.wait_with_output().unwrap();
                assert!(output.status.success(), "{:?}", output.stderr);
                assert!(String::from_utf8_lossy(&output.stdout).contains(&code));
                if let Some(baseline) = &baseline {
                    assert_eq!(&output.stdout, baseline);
                } else {
                    baseline = Some(output.stdout);
                }
                assert_eq!(fs::read_to_string(&config).unwrap(), original);
            }
        }
    }
    let output = command(directory.path())
        .args(["--wrap", "--no-wrap"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
    assert!(!directory.path().join("isolated").exists());
}
