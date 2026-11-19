//! File filtering, modal isolation, cached navigation, and actual popup rendering.
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseEvent, MouseEventKind,
};
use ratatui::{Terminal, backend::TestBackend};
use riff::{
    app::App,
    cli::ViewMode,
    config::SettingsUpdate,
    patch::{Diff, parse_patch},
    reload::ReloadEvent,
    theme::builtin_themes,
    ui,
};

fn file(path: &str) -> String {
    format!("--- a/{path}\n+++ b/{path}\n@@ -1,2 +1,3 @@\n-old\n+new\n+extra\n needle\n")
}

fn app(paths: &[&str]) -> App {
    patch_app(&paths.iter().map(|path| file(path)).collect::<String>())
}

fn patch_app(patch: &str) -> App {
    let mut app = App::new(
        parse_patch(patch).unwrap(),
        builtin_themes(),
        0,
        ViewMode::Unified,
        false,
        "test".into(),
    )
    .unwrap();
    app.resize(80, 8);
    app.configure_reload(true, false);
    app
}

fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_event(Event::Key(KeyEvent::new(code, modifiers)));
}

fn press(app: &mut App, code: KeyCode) {
    key(app, code, KeyModifiers::NONE);
}

fn text(app: &mut App, text: &str) {
    for ch in text.chars() {
        press(app, KeyCode::Char(ch));
    }
}

fn open(app: &mut App) {
    key(app, KeyCode::Char('p'), KeyModifiers::CONTROL);
}

fn matched_paths(app: &App) -> Vec<&str> {
    let picker = app.file_picker.as_ref().unwrap();
    picker
        .matches
        .iter()
        .map(|index| picker.entries[*index].path.as_str())
        .collect()
}

fn screen(app: &mut App, width: u16, height: u16) -> String {
    app.resize(width, height);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn fuzzy_terms_match_nonadjacent_letters_anywhere_in_any_term_order() {
    let mut app = app(&[
        "docs/app.md",
        "src/app.rs",
        "src/apple.rs",
        "src/main.rs",
        "src/Ä雪.rs",
    ]);
    open(&mut app);
    text(&mut app, "APP");
    assert_eq!(
        matched_paths(&app),
        ["docs/app.md", "src/app.rs", "src/apple.rs"]
    );
    text(&mut app, " src");
    assert_eq!(matched_paths(&app), ["src/app.rs", "src/apple.rs"]);
    key(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    text(&mut app, "  aP.rS  src  ");
    assert_eq!(matched_paths(&app), ["src/app.rs", "src/apple.rs"]);
    key(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    text(&mut app, "ppa");
    assert!(
        matched_paths(&app).is_empty(),
        "letter order within a term matters"
    );
    key(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    text(&mut app, "ä雪");
    assert_eq!(matched_paths(&app), ["src/Ä雪.rs"]);
    press(&mut app, KeyCode::Backspace);
    assert_eq!(app.file_picker.as_ref().unwrap().query, "ä");
    assert_eq!(matched_paths(&app), ["src/Ä雪.rs"]);
}

#[test]
fn selection_uses_filtered_file_identity_and_keeps_surviving_selection() {
    let mut app = app(&["docs/notes", "src/app.rs", "src/apple.rs", "tests/app.rs"]);
    press(&mut app, KeyCode::Char(']'));
    assert_eq!(app.current_file(), 1);
    open(&mut app);
    assert_eq!(app.file_picker.as_ref().unwrap().selected_file(), Some(1));
    text(&mut app, "app");
    assert_eq!(app.file_picker.as_ref().unwrap().selected, Some(0));
    press(&mut app, KeyCode::Up);
    assert_eq!(app.file_picker.as_ref().unwrap().selected_file(), Some(3));
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    assert_eq!(app.file_picker.as_ref().unwrap().selected_file(), Some(2));
    text(&mut app, " src");
    assert_eq!(app.file_picker.as_ref().unwrap().selected_file(), Some(2));
    press(&mut app, KeyCode::Enter);
    assert!(app.file_picker.is_none());
    assert_eq!(app.current_file(), 2);
    assert_eq!(app.scroll, app.cache.file_starts[2]);
    open(&mut app);
    assert!(app.file_picker.as_ref().unwrap().query.is_empty());
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char(']'));
    assert_eq!(app.current_file(), 3);
}

#[test]
fn navigation_survives_resize_wrapping_and_last_page_clamping_without_rebuilding() {
    for view in [ViewMode::Unified, ViewMode::Split] {
        for wrap in [false, true] {
            let mut app = app(&["first", "middle", "last"]);
            app.view = view;
            app.set_wrap(wrap);
            app.resize(40, 5);
            open(&mut app);
            text(&mut app, "last");
            app.resize(60, 100);
            let allocation = app.cache.rows.as_ptr();
            press(&mut app, KeyCode::Enter);
            assert_eq!(app.current_file(), 2);
            assert_eq!(app.scroll, 0);
            assert_eq!(app.cache.rows.as_ptr(), allocation);
            press(&mut app, KeyCode::Char('['));
            assert_eq!(app.current_file(), 1);
            press(&mut app, KeyCode::Char(']'));
            assert_eq!(app.current_file(), 2);
            press(&mut app, KeyCode::Char(']'));
            assert_eq!(app.current_file(), 0);
        }
    }
}

#[test]
fn cancel_and_typing_leave_diff_search_preferences_and_reload_requests_unchanged() {
    let mut app = app(&["src/quite-wide-path.rs", "src/other.rs"]);
    app.resize(12, 8);
    press(&mut app, KeyCode::Char('/'));
    text(&mut app, "needle");
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Right);
    assert!(app.horizontal > 0);
    let before = (
        app.scroll,
        app.horizontal,
        app.match_index,
        app.current_file(),
    );
    let allocation = app.cache.rows.as_ptr();
    open(&mut app);
    text(&mut app, "q?twirv/jk ");
    assert!(app.running && !app.help && !app.wrap && app.word_diff);
    assert_eq!(app.file_picker.as_ref().unwrap().query, "q?twirv/jk ");
    key(&mut app, KeyCode::Char('w'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('r'), KeyModifiers::CONTROL);
    app.handle_event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(
        (
            app.scroll,
            app.horizontal,
            app.match_index,
            app.current_file()
        ),
        before
    );
    assert_eq!(app.cache.rows.as_ptr(), allocation);
    assert_eq!(app.take_settings_update(), SettingsUpdate::default());
    assert_eq!(app.take_reload_requests(), Default::default());
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.query, "needle");
    assert_eq!(
        (
            app.scroll,
            app.horizontal,
            app.match_index,
            app.current_file()
        ),
        before
    );
    open(&mut app);
    text(&mut app, "other");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.current_file(), 1);
    assert_eq!(app.horizontal, 0);
    assert_eq!(app.query, "needle");
    assert_eq!(app.cache.rows.as_ptr(), allocation);
}

#[test]
fn empty_matches_ignore_enter_and_arrows_and_recover_on_edit() {
    let mut empty = app(&[]);
    let mut app = app(&["src/app.rs", "tests/app.rs"]);
    open(&mut app);
    text(&mut app, "zz");
    for code in [
        KeyCode::Enter,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Home,
        KeyCode::End,
    ] {
        press(&mut app, code);
        assert!(app.file_picker.as_ref().unwrap().selected.is_none());
    }
    assert!(screen(&mut app, 100, 24).contains("No matching files."));
    press(&mut app, KeyCode::Backspace);
    press(&mut app, KeyCode::Backspace);
    assert_eq!(matched_paths(&app), ["src/app.rs", "tests/app.rs"]);
    press(&mut app, KeyCode::End);
    assert_eq!(app.file_picker.as_ref().unwrap().selected_file(), Some(1));
    press(&mut app, KeyCode::Home);
    assert_eq!(app.file_picker.as_ref().unwrap().selected_file(), Some(0));
    text(&mut app, "zz");
    key(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    assert_eq!(matched_paths(&app).len(), 2);
    open(&mut empty);
    press(&mut empty, KeyCode::Enter);
    assert!(screen(&mut empty, 100, 24).contains("No files in diff."));
    press(&mut empty, KeyCode::Esc);
    assert!(empty.running && empty.file_picker.is_none());
}

#[test]
fn counts_and_display_paths_include_deleted_renamed_binary_and_multiple_hunks() {
    let patch = format!(
        "{}@@ -20 +21,2 @@\n-another\n+replacement\n+extra\n\
         --- a/deleted.rs\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-one\n-two\n\
         diff --git a/old.rs b/new.rs\nsimilarity index 100%\nrename from old.rs\nrename to new.rs\n\
         diff --git a/image.png b/image.png\nBinary files a/image.png and b/image.png differ\n",
        file("src/app.rs")
    );
    let mut app = patch_app(&patch);
    app.resize(100, 24);
    open(&mut app);
    let entries = &app.file_picker.as_ref().unwrap().entries;
    let summary: Vec<_> = entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry.additions, entry.deletions))
        .collect();
    assert_eq!(
        summary,
        [
            ("src/app.rs", 4, 2),
            ("deleted.rs", 0, 2),
            ("new.rs", 0, 0),
            ("image.png", 0, 0)
        ]
    );
    let rendered = screen(&mut app, 100, 24);
    for expected in [
        "+4 -2  src/app.rs",
        "+0 -2  deleted.rs",
        "+0 -0  new.rs",
        "+0 -0  image.png",
    ] {
        assert!(rendered.contains(expected), "{rendered}");
    }
    text(&mut app, "dlt");
    assert_eq!(matched_paths(&app), ["deleted.rs"]);
    text(&mut app, "d");
    assert_eq!(matched_paths(&app), ["deleted.rs"]);
}

#[test]
fn popup_only_renders_matching_entries_and_scrolls_to_selected_file() {
    let names: Vec<_> = (0..40).map(|index| format!("path-{index:02}.rs")).collect();
    let paths: Vec<_> = names.iter().map(String::as_str).collect();
    let mut app = app(&paths);
    open(&mut app);
    press(&mut app, KeyCode::End);
    let rendered = screen(&mut app, 100, 24);
    assert!(rendered.contains("› +2 -1  path-39.rs"), "{rendered}");
    text(&mut app, "39");
    let rendered = screen(&mut app, 100, 24);
    assert!(rendered.contains("Files · 1/40"));
    assert!(rendered.contains("path-39.rs"));
    assert!(!rendered.contains("path-38.rs"));
    for width in [0, 1, 2, 3, 8, 40, 100] {
        for height in [0, 1, 2, 3, 4, 5, 24] {
            screen(&mut app, width, height);
        }
    }
}

#[test]
fn labels_sanitize_terminal_controls_and_long_unicode_queries_keep_the_caret_visible() {
    let mut app = patch_app(
        "diff --git \"a/unsafe\\033[31m.rs\" \"b/unsafe\\033[31m.rs\"\nold mode 100644\nnew mode 100755\n",
    );
    open(&mut app);
    let rendered = screen(&mut app, 100, 24);
    assert!(rendered.contains("unsafe^[[31m.rs"), "{rendered}");
    assert!(!rendered.contains('\u{1b}'));
    assert_eq!(matched_paths(&app), ["unsafe^[[31m.rs"]);
    text(&mut app, &format!("{}界🦀", "long-query".repeat(20)));
    let rendered = screen(&mut app, 40, 12);
    let query_row = rendered
        .lines()
        .find(|row| row.contains("long-query"))
        .unwrap();
    assert!(
        query_row.contains('界') && query_row.contains('🦀'),
        "{rendered}"
    );
    assert!(query_row.ends_with("▏│"), "{rendered}");
}

#[test]
fn pending_reload_waits_for_picker_then_remaps_accepted_file_identity() {
    for accept in [false, true] {
        let mut app = app(&["first", "target", "last"]);
        open(&mut app);
        text(&mut app, "target");
        app.handle_reload_event(ReloadEvent::Updated(Diff::default()));
        let revised = format!(
            "{}{}{}{}",
            file("inserted"),
            file("first"),
            file("target"),
            file("last")
        );
        app.handle_reload_event(ReloadEvent::Updated(parse_patch(&revised).unwrap()));
        assert_eq!(app.diff.files.len(), 3);
        assert_eq!(matched_paths(&app), ["target"]);
        press(&mut app, if accept { KeyCode::Enter } else { KeyCode::Esc });
        assert_eq!(app.diff.files.len(), 4);
        let expected = if accept { "target" } else { "first" };
        assert_eq!(app.diff.files[app.current_file()].display_path(), expected);
        if accept {
            assert_eq!(app.scroll, app.cache.file_starts[2]);
        }
    }
}

#[test]
fn shortcut_respects_existing_modals_modifiers_release_caps_lock_and_ctrl_c() {
    let mut app = app(&["src/app.rs"]);
    for modifiers in [
        KeyModifiers::NONE,
        KeyModifiers::ALT,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ] {
        key(&mut app, KeyCode::Char('p'), modifiers);
        assert!(app.file_picker.is_none());
    }
    let mut release = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
    release.kind = KeyEventKind::Release;
    app.handle_event(Event::Key(release));
    assert!(app.file_picker.is_none());
    for modal in ['?', 't', '/'] {
        press(&mut app, KeyCode::Char(modal));
        open(&mut app);
        assert!(app.file_picker.is_none());
        press(&mut app, KeyCode::Esc);
    }
    open(&mut app);
    let mut caps = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
    caps.state = KeyEventState::CAPS_LOCK;
    app.handle_event(Event::Key(caps));
    assert_eq!(app.file_picker.as_ref().unwrap().query, "A");
    assert_eq!(matched_paths(&app), ["src/app.rs"]);
    key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(!app.running);
}
