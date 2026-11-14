//! Cross-patch identities and UI-only request handling; no worker internals.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use riff::{
    app::App,
    cli::ViewMode,
    config::SettingsUpdate,
    patch::{Diff, parse_patch},
    reload::ReloadEvent,
    render::{RowContent, SourceId},
    theme::builtin_themes,
    ui, wrap,
};

fn file(name: &str, start: u32, lines: &[&str]) -> String {
    format!(
        "--- a/{name}\n+++ b/{name}\n@@ -{start},{} +{start},{} @@\n{}",
        lines.len(),
        lines.len(),
        lines
            .iter()
            .map(|line| format!(" {line}\n"))
            .collect::<String>()
    )
}

fn app(patch: &str) -> App {
    let mut app = App::new(
        parse_patch(patch).unwrap(),
        builtin_themes(),
        0,
        ViewMode::Unified,
        false,
        "test".into(),
    )
    .unwrap();
    app.resize(45, 5);
    app.configure_reload(true, false);
    app
}

fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_event(Event::Key(KeyEvent::new(code, modifiers)));
}

fn press(app: &mut App, ch: char) {
    key(app, KeyCode::Char(ch), KeyModifiers::NONE);
}

fn search(app: &mut App, query: &str) {
    press(app, '/');
    for ch in query.chars() {
        press(app, ch);
    }
    key(app, KeyCode::Enter, KeyModifiers::NONE);
}

fn update(app: &mut App, patch: &str) {
    app.handle_reload_event(ReloadEvent::Updated(parse_patch(patch).unwrap()));
}

fn top(app: &App) -> wrap::SourcePosition {
    wrap::row_position(&app.cache.rows[app.scroll])
}

#[test]
fn inserted_file_and_hunk_do_not_steal_reading_or_selected_match_identity() {
    let original = file("b", 30, &["needle b", "tail b", "end b"]);
    let mut app = app(&original);
    search(&mut app, "needle");
    let revised = format!(
        "{}{}@@ -10,1 +10,1 @@\n needle earlier\n@@ -30,3 +30,3 @@\n needle b\n tail b\n end b\n",
        file("a", 1, &["needle a"]),
        "--- a/b\n+++ b/b\n"
    );
    update(&mut app, &revised);
    assert_eq!(app.diff.files[app.current_file()].display_path(), "b");
    assert_eq!(top(&app).source, SourceId::Line(1, 1, 0));
    assert_eq!(app.current_match_row(), Some(app.scroll));
    assert_eq!(app.match_index, Some(2));
    assert_eq!(app.search_matches.len(), 3);
    assert_eq!(app.query, "needle");
    press(&mut app, 'n');
    assert_eq!(top(&app).source, SourceId::Hunk(0, 0));
}

#[test]
fn selected_match_and_wrap_continuation_survive_shifted_lines_and_files() {
    let long = format!("{}needle {}", "prefix ".repeat(30), "tail ".repeat(30));
    for view in [ViewMode::Unified, ViewMode::Split] {
        let original = file("b", 30, &[&long, "needle second", "tail", "end"]);
        let mut app = app(&original);
        app.view = view;
        app.set_wrap(true);
        search(&mut app, "needle");
        assert!(top(&app).offset > 0);
        let before = top(&app).offset;
        let revised = format!(
            "{}{}",
            file("a", 1, &["needle inserted"]),
            file("b", 31, &[&long, "needle second", "tail", "end"])
        );
        update(&mut app, &revised);
        assert_eq!(top(&app).source, SourceId::Line(1, 0, 0));
        assert_eq!(top(&app).offset, before);
        assert_eq!(app.match_index, Some(1));
        assert!(app.wrap && app.view == view);
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.match_index, Some(2));
    }
}

#[test]
fn selected_match_follows_its_new_offset_when_the_line_text_changes() {
    let mut app = app(&file("b", 30, &["needle here", "tail", "end"]));
    app.set_wrap(true);
    search(&mut app, "needle");
    let changed = format!("{}needle here", "inserted prefix ".repeat(20));
    update(&mut app, &file("b", 30, &[&changed, "tail", "end"]));
    assert_eq!(app.current_match_row(), Some(app.scroll));
    assert!(top(&app).offset > 0);
    assert_eq!(app.match_index, Some(0));
}

fn target_hunk(old_start: u32, new_start: u32) -> String {
    format!(
        "@@ -{old_start},7 +{new_start},7 @@\n LINE_097\n LINE_098\n LINE_099\n-LINE_100\n+TARGET_EDITED\n SHARED_CONTEXT\n LINE_102\n LINE_103\n"
    )
}

#[test]
fn earlier_large_insertion_on_either_side_keeps_the_hunk_and_navigation_anchor() {
    let original = format!("--- a/b\n+++ b/b\n{}", target_hunk(97, 97));
    for view in [ViewMode::Unified, ViewMode::Split] {
        for wrapped in [false, true] {
            for left_changed in [false, true] {
                let mut app = app(&original);
                app.view = view;
                app.resize(120, 6);
                app.set_wrap(wrapped);
                press(&mut app, 'n');
                assert_eq!(top(&app).source, SourceId::Hunk(0, 0));
                let (first_header, sign, old_start, new_start) = if left_changed {
                    ("@@ -1,503 +1,3 @@", '-', 597, 97)
                } else {
                    ("@@ -1,3 +1,503 @@", '+', 97, 597)
                };
                let revised = format!(
                    "--- a/b\n+++ b/b\n{first_header}\n{} LINE_001\n LINE_002\n LINE_003\n{}",
                    format!("{sign}PREPENDED\n").repeat(500),
                    target_hunk(old_start, new_start)
                );
                update(&mut app, &revised);
                assert_eq!(top(&app).source, SourceId::Hunk(0, 1));
                press(&mut app, 'n');
                assert_eq!(top(&app).source, SourceId::Hunk(0, 0));
                press(&mut app, 'p');
                assert_eq!(top(&app).source, SourceId::Hunk(0, 1));
            }
        }
    }
}

#[test]
fn inserted_duplicate_does_not_steal_selected_context_on_either_side() {
    let original = format!("--- a/b\n+++ b/b\n{}", target_hunk(97, 97));
    for view in [ViewMode::Unified, ViewMode::Split] {
        for wrapped in [false, true] {
            for left_changed in [false, true] {
                let mut app = app(&original);
                app.view = view;
                app.resize(120, 6);
                app.set_wrap(wrapped);
                search(&mut app, "SHARED_CONTEXT");
                assert_eq!(app.match_index, Some(0));
                let (header, sign) = if left_changed {
                    ("@@ -95,29 +95,9 @@", '-')
                } else {
                    ("@@ -95,9 +95,29 @@", '+')
                };
                let inserted: String = (0..20)
                    .map(|index| {
                        if index == 3 {
                            format!("{sign}SHARED_CONTEXT\n")
                        } else {
                            format!("{sign}INSERT_{index}\n")
                        }
                    })
                    .collect();
                let revised = format!(
                    "--- a/b\n+++ b/b\n{header}\n LINE_095\n LINE_096\n LINE_097\n{inserted} LINE_098\n LINE_099\n-LINE_100\n+TARGET_EDITED\n SHARED_CONTEXT\n LINE_102\n LINE_103\n"
                );
                update(&mut app, &revised);
                assert_eq!(app.search_matches.len(), 2);
                assert_eq!(app.match_index, Some(1));
                assert_eq!(app.current_match_row(), Some(app.scroll));
                let SourceId::Line(_, hunk, line) = top(&app).source else {
                    panic!("expected surviving context line");
                };
                let line = &app.diff.files[0].hunks[hunk].lines[line];
                assert_eq!(line.kind, riff::patch::LineKind::Context);
                let expected = if left_changed { (121, 101) } else { (101, 121) };
                assert_eq!(
                    (line.old_number, line.new_number),
                    (Some(expected.0), Some(expected.1))
                );
                key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
                assert_eq!(app.match_index, Some(0));
            }
        }
    }
}

#[test]
fn rapid_watch_toggles_serialize_and_save_only_the_final_acknowledged_choice() {
    let mut app = app(&file("b", 1, &["line"]));
    key(&mut app, KeyCode::Char('w'), KeyModifiers::CONTROL);
    assert_eq!(app.take_reload_requests().watch, Some(true));
    key(&mut app, KeyCode::Char('w'), KeyModifiers::CONTROL);
    assert_eq!(app.take_reload_requests().watch, None);
    app.handle_reload_event(ReloadEvent::WatchChanged {
        enabled: true,
        error: None,
    });
    assert_eq!(app.take_settings_update(), SettingsUpdate::default());
    assert_eq!(app.take_reload_requests().watch, Some(false));
    app.handle_reload_event(ReloadEvent::WatchChanged {
        enabled: false,
        error: None,
    });
    assert_eq!(app.take_settings_update().watch, Some(false));
    assert!(!app.watch_enabled());
}

#[test]
fn horizontal_theme_and_layout_survive_successful_reload() {
    let long = "word ".repeat(60);
    let original = file("b", 1, &[&long, "tail", "end"]);
    let mut app = app(&original);
    search(&mut app, "word");
    press(&mut app, 'l');
    press(&mut app, 'l');
    press(&mut app, 't');
    press(&mut app, 'j');
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    let horizontal = app.horizontal;
    let theme = app.theme().name.clone();
    update(&mut app, &format!("{}{}", file("a", 1, &["new"]), original));
    assert_eq!(app.horizontal, horizontal);
    assert!(horizontal > 0);
    assert_eq!(app.theme().name, theme);
    assert_eq!(app.view, ViewMode::Unified);
}

#[test]
fn missing_line_uses_nearest_same_side_and_missing_file_uses_old_neighbor() {
    let original = format!(
        "{}{}",
        file("b", 10, &["before", "gone", "after", "end"]),
        file("c", 1, &["c", "tail", "end"])
    );
    let mut app = app(&original);
    search(&mut app, "gone");
    let revised = format!(
        "{}{}{}",
        file("a", 1, &["new"]),
        file("b", 12, &["after", "end"]),
        file("c", 1, &["c", "tail", "end"])
    );
    update(&mut app, &revised);
    assert_eq!(top(&app).source, SourceId::Line(1, 0, 0));
    assert_eq!(app.search_matches.len(), 0);
    update(
        &mut app,
        &format!(
            "{}{}",
            file("a", 1, &["new"]),
            file("c", 1, &["c", "tail", "end"])
        ),
    );
    // Both a and c now survive; b's nearest old neighbor is a (not a stale line
    // number in c, which happens to take b's former index).
    assert_eq!(top(&app).source, SourceId::Header(0, 0));
    assert_eq!(app.diff.files[app.current_file()].display_path(), "a");
}

#[test]
fn vanished_original_file_prefers_surviving_neighbor_over_new_numeric_occupant() {
    let original = format!(
        "{}{}",
        file("b", 1, &["needle", "tail"]),
        file("c", 1, &["c", "tail"])
    );
    let mut app = app(&original);
    search(&mut app, "needle");
    update(
        &mut app,
        &format!(
            "{}{}",
            file("a", 1, &["inserted"]),
            file("c", 1, &["c", "tail"])
        ),
    );
    assert_eq!(app.diff.files[app.current_file()].display_path(), "c");
    assert_eq!(top(&app).source, SourceId::Header(1, 0));
}

#[test]
fn modal_updates_keep_latest_result_and_cancel_restores_search_before_remapping() {
    let original = file("b", 1, &["needle one", "needle two", "tail", "end"]);
    let mut app = app(&original);
    search(&mut app, "needle");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    press(&mut app, '/');
    press(&mut app, 'z');
    update(&mut app, &file("discarded", 1, &["outdated"]));
    update(
        &mut app,
        &format!("{}{}", file("a", 1, &["needle inserted"]), original),
    );
    app.handle_reload_event(ReloadEvent::Unchanged);
    assert_eq!(app.diff.files.len(), 1);
    assert!(app.editing_search());
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(app.query, "needle");
    assert_eq!(app.diff.files.len(), 2);
    assert_eq!(app.match_index, Some(2));
    assert_eq!(top(&app).source, SourceId::Line(1, 0, 1));
    press(&mut app, 't');
    press(&mut app, 'j');
    let preview = app.theme().name.clone();
    update(
        &mut app,
        &file("b", 1, &["needle one", "needle two", "tail", "end"]),
    );
    assert_eq!(app.theme().name, preview);
    assert!(app.picker_original.is_some());
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.diff.files.len(), 1);
    assert_eq!(app.theme().name, preview);
    press(&mut app, '?');
    update(&mut app, "");
    assert!(!app.diff.files.is_empty() && app.help);
    press(&mut app, '?');
    assert!(app.diff.files.is_empty() && app.running);
}

#[test]
fn errors_and_unchanged_leave_cache_intact_and_good_results_recover_counts() {
    let mut app = app("--- a/b\n+++ b/b\n@@ -1 +1 @@\n-old\n+new\n");
    let allocation = app.cache.rows.as_ptr();
    app.handle_reload_event(ReloadEvent::WatchChanged {
        enabled: true,
        error: None,
    });
    app.handle_reload_event(ReloadEvent::Reloading);
    app.handle_reload_event(ReloadEvent::Error("bad\u{1b}[31m\nloader".into()));
    assert_eq!(app.cache.rows.as_ptr(), allocation);
    assert!(app.watch_enabled() && app.running);
    assert!(
        app.reload_error()
            .unwrap()
            .chars()
            .all(|ch| !ch.is_control())
    );
    assert!(!app.reload_status().contains("loading"));
    app.handle_reload_event(ReloadEvent::Unchanged);
    assert_eq!(app.cache.rows.as_ptr(), allocation);
    assert!(app.reload_error().is_none());
    assert_eq!((app.additions, app.deletions), (1, 1));
    app.handle_reload_event(ReloadEvent::Updated(Diff::default()));
    assert_eq!((app.additions, app.deletions), (0, 0));
    assert!(app.cache.rows.is_empty() && app.search_matches.is_empty() && app.running);
    for width in [0, 1, 2, 10] {
        for height in [0, 1, 2, 4] {
            app.resize(width, height);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        }
    }
}

#[test]
fn requests_are_deferred_coalesced_and_watch_is_saved_only_after_successful_ack() {
    let mut app = app(&file("b", 1, &["line"]));
    app.configure_reload(true, true);
    assert_eq!(app.take_reload_requests().watch, Some(true));
    app.handle_reload_event(ReloadEvent::WatchChanged {
        enabled: true,
        error: None,
    });
    assert_eq!(app.take_settings_update(), SettingsUpdate::default());
    for _ in 0..4 {
        press(&mut app, 'r');
    }
    key(&mut app, KeyCode::Char('r'), KeyModifiers::CONTROL);
    assert!(app.take_reload_requests().reload);
    assert!(!app.take_reload_requests().reload);
    key(&mut app, KeyCode::Char('w'), KeyModifiers::CONTROL);
    assert!(!app.wrap);
    assert_eq!(app.take_reload_requests().watch, Some(false));
    assert_eq!(app.take_settings_update(), SettingsUpdate::default());
    app.handle_reload_event(ReloadEvent::WatchChanged {
        enabled: false,
        error: None,
    });
    assert_eq!(app.take_settings_update().watch, Some(false));
    key(&mut app, KeyCode::Char('w'), KeyModifiers::CONTROL);
    assert_eq!(app.take_reload_requests().watch, Some(true));
    app.handle_reload_event(ReloadEvent::WatchChanged {
        enabled: false,
        error: Some("watch failed".into()),
    });
    assert_eq!(app.take_settings_update(), SettingsUpdate::default());
    press(&mut app, 'r');
    assert!(app.take_reload_requests().reload);
    press(&mut app, 'w');
    assert!(app.wrap);
    assert_eq!(app.take_settings_update().wrap, Some(true));
}

#[test]
fn unsupported_actions_modifiers_and_modals_never_persist_watch_or_leak_bindings() {
    let mut app = app(&file("b", 1, &["line"]));
    app.reload_unavailable("stdin patch cannot be reread");
    for (ch, modifiers) in [
        ('r', KeyModifiers::NONE),
        ('r', KeyModifiers::CONTROL),
        ('w', KeyModifiers::CONTROL),
    ] {
        key(&mut app, KeyCode::Char(ch), modifiers);
        assert!(app.reload_error().unwrap().contains("stdin"));
        assert_eq!(app.take_reload_requests(), Default::default());
        assert_eq!(app.take_settings_update(), SettingsUpdate::default());
    }
    assert!(app.running);
    app.configure_reload(true, false);
    for modifiers in [
        KeyModifiers::ALT,
        KeyModifiers::SUPER,
        KeyModifiers::HYPER,
        KeyModifiers::META,
        KeyModifiers::CONTROL | KeyModifiers::ALT,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ] {
        for ch in ['r', 'w'] {
            key(&mut app, KeyCode::Char(ch), modifiers);
        }
    }
    assert_eq!(app.take_reload_requests(), Default::default());
    app.handle_event(Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char('r'),
        KeyModifiers::NONE,
        KeyEventKind::Release,
    )));
    assert_eq!(app.take_reload_requests(), Default::default());
    for opener in ['?', 't', '/'] {
        press(&mut app, opener);
        key(&mut app, KeyCode::Char('w'), KeyModifiers::CONTROL);
        key(&mut app, KeyCode::Char('r'), KeyModifiers::CONTROL);
        press(&mut app, 'r');
        assert_eq!(app.take_reload_requests(), Default::default());
        assert_eq!(app.take_settings_update(), SettingsUpdate::default());
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    }
    app.handle_event(Event::Key(KeyEvent {
        code: KeyCode::Char('w'),
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::CAPS_LOCK,
    }));
    assert!(!app.wrap);
    press(&mut app, '?');
    key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(!app.running);
}

#[test]
fn split_reload_maps_the_selected_old_side_not_its_new_counterpart() {
    let original = "--- a/b\n+++ b/b\n@@ -1,3 +1,3 @@\n-needle old\n+needle new\n tail\n end\n";
    let mut app = app(original);
    app.view = ViewMode::Split;
    app.resize(46, 5);
    search(&mut app, "needle old");
    update(
        &mut app,
        &format!("{}{original}", file("a", 1, &["needle inserted"])),
    );
    assert_eq!(
        app.cache.rows[app.current_match_row().unwrap()].source,
        SourceId::Line(1, 0, 1)
    );
    let row = &app.cache.rows[app.current_match_row().unwrap()];
    assert!(
        matches!(&row.content, RowContent::Split { left: Some(cell), .. } if cell.source == SourceId::Line(1, 0, 0))
    );
    assert_eq!(app.search_matches.len(), 1);
}
