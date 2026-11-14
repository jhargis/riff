//! Visual-row, identity, style, search, and modified-key regressions.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend, style::Style};
use riff::{
    app::App,
    cli::ViewMode,
    patch::{LineKind, parse_patch, sanitize},
    render::{CodeCell, RowContent, SourceId, build_cache, split_gutter, unified_gutter},
    theme::builtin_themes,
    ui, wrap,
};
use unicode_width::UnicodeWidthStr;

fn app(text: &str, view: ViewMode, width: u16, height: u16) -> App {
    let patch = format!("--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n-old short\n+{text}\n");
    let mut app = App::new(
        parse_patch(&patch).unwrap(),
        builtin_themes(),
        0,
        view,
        true,
        "test".into(),
    )
    .unwrap();
    app.resize(width, height);
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

fn cells(app: &App, source: SourceId) -> Vec<&CodeCell> {
    app.cache
        .rows
        .iter()
        .flat_map(|row| match &row.content {
            RowContent::Unified(cell) => vec![cell],
            RowContent::Split { left, right } => left.iter().chain(right).collect(),
            _ => vec![],
        })
        .filter(|cell| cell.source == source)
        .collect()
}

fn characters(cells: &[&CodeCell]) -> Vec<(char, Style)> {
    cells
        .iter()
        .flat_map(|cell| &cell.spans)
        .flat_map(|span| span.content.chars().map(|ch| (ch, span.style)))
        .collect()
}

#[test]
fn wraps_at_words_without_losing_whitespace_unicode_tabs_or_styles() {
    let text = "fn hello() {\tlet snow = \"界e\u{301}👨‍👩‍👧‍👦\";   trailing   }  ";
    for view in [ViewMode::Unified, ViewMode::Split] {
        let mut app = app(text, view, 39, 30);
        let source = SourceId::Line(0, 0, 1);
        let original = characters(&cells(&app, source));
        app.set_wrap(true);
        let fragments = cells(&app, source);
        assert!(fragments.len() > 1);
        assert_eq!(characters(&fragments), original);
        assert_eq!(
            fragments.iter().map(|cell| cell.text()).collect::<String>(),
            sanitize(text)
        );
        let mut offset = 0;
        for (index, cell) in fragments.iter().enumerate() {
            assert_eq!(cell.offset, offset);
            offset += cell.text().len();
            assert_eq!(cell.kind, LineKind::Added);
            assert_eq!(
                cell.style(&app.theme().palette).bg,
                Some(app.theme().palette.added_background)
            );
            if index > 0 {
                assert_eq!(cell.old_number, None);
                assert_eq!(cell.new_number, None);
                assert_eq!(cell.sign(), ' ');
                assert!(!unified_gutter(cell, app.cache.digits).contains('+'));
                assert!(!split_gutter(cell, app.cache.digits, false).contains('+'));
            }
        }
    }
    let mut app = app("alpha beta gamma delta", ViewMode::Unified, 18, 10);
    app.set_wrap(true);
    assert_eq!(
        cells(&app, SourceId::Line(0, 0, 1))[0].text(),
        "alpha beta "
    );
}

#[test]
fn hard_breaks_tokens_and_preserves_wide_graphemes_at_tiny_widths() {
    for width in 0..24 {
        let text = "long_unbroken_token_界🦀e\u{301}_tail";
        let mut app = app(text, ViewMode::Unified, width, 4);
        app.set_wrap(true);
        let fragments = cells(&app, SourceId::Line(0, 0, 1));
        assert_eq!(
            fragments.iter().map(|cell| cell.text()).collect::<String>(),
            text
        );
        if width >= 8 {
            assert!(
                fragments
                    .iter()
                    .all(|cell| cell.text().width() <= usize::from(width) - 6)
            );
        }
        app.resize(width, 0);
        assert!(app.scroll <= app.maximum_scroll());
    }
}

#[test]
fn split_sides_wrap_independently_with_padding_and_fixed_divider() {
    let mut app = app(
        "fn long_name() { let value = 123456789; }",
        ViewMode::Split,
        41,
        30,
    );
    app.set_wrap(true);
    let rows: Vec<_> = app
        .cache
        .rows
        .iter()
        .filter_map(|row| match &row.content {
            RowContent::Split { left, right } => Some((left, right)),
            _ => None,
        })
        .collect();
    assert!(rows.len() > 2);
    assert!(rows[0].0.is_some() && rows[0].1.is_some());
    assert!(
        rows[1..]
            .iter()
            .all(|(left, right)| left.is_none() && right.is_some())
    );
    let mut terminal = Terminal::new(TestBackend::new(41, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let first = app
        .cache
        .rows
        .iter()
        .position(|row| matches!(row.content, RowContent::Split { .. }))
        .unwrap();
    for index in 0..rows.len() {
        let y = (first + index + 1) as u16;
        assert_eq!(terminal.backend().buffer()[(20, y)].symbol(), "│");
        assert_eq!(
            terminal.backend().buffer()[(40, y)].bg,
            app.theme().palette.added_background
        );
        assert_eq!(
            terminal.backend().buffer()[(21, y)].bg,
            app.theme().palette.added_gutter
        );
        if index > 0 {
            assert_eq!(
                terminal.backend().buffer()[(0, y)].bg,
                app.theme().palette.background
            );
        }
    }
}

#[test]
fn search_counts_logical_lines_and_finds_across_boundaries_and_late_fragments() {
    let mut app = app(
        "alpha beta gamma delta epsilon needle omega",
        ViewMode::Unified,
        18,
        6,
    );
    app.set_wrap(true);
    search(&mut app, "beta gamma");
    assert_eq!(app.search_matches.len(), 1);
    let row = app.current_match_row().unwrap();
    assert_eq!(app.scroll, row);
    search(&mut app, "needle");
    assert_eq!(app.search_matches.len(), 1);
    let row = app.current_match_row().unwrap();
    assert!(row > app.cache.hunk_starts[0] + 2);
    assert!(row >= app.scroll && row < app.scroll + app.viewport_height);
    assert!(
        matches!(&app.cache.rows[row].content, RowContent::Unified(cell) if cell.text().contains("needle"))
    );
    for width in [23, 120, 18, 31, 18] {
        app.resize(width, 6);
        assert_eq!(app.search_matches.len(), 1);
        let row = app.current_match_row().unwrap();
        assert!(row >= app.scroll && row < app.scroll + app.viewport_height);
    }
    press(&mut app, 'w');
    assert!(!app.wrap);
    assert!(
        app.horizontal > 0,
        "clipped mode must reveal the late match too"
    );
    press(&mut app, 'w');
    assert_eq!(app.horizontal, 0);
    let row = app.current_match_row().unwrap();
    assert!(row >= app.scroll && row < app.scroll + app.viewport_height);
}

#[test]
fn selected_old_side_and_search_cancel_survive_wrap_layout_and_theme_rebuilds() {
    let patch = "--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n-old alpha alpha needle end\n-old beta beta needle end\n+new alpha alpha needle end\n+new beta beta needle end\n";
    let mut app = App::new(
        parse_patch(patch).unwrap(),
        builtin_themes(),
        0,
        ViewMode::Unified,
        false,
        "test".into(),
    )
    .unwrap();
    app.resize(20, 5);
    app.set_wrap(true);
    search(&mut app, "needle");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    let selected = wrap::search_hits(&app.cache, "needle")[app.match_index.unwrap()].position;
    assert_eq!(selected.source, SourceId::Line(0, 0, 1));
    for width in [27, 120, 20] {
        app.resize(width, 5);
        assert_eq!(
            wrap::search_hits(&app.cache, "needle")[app.match_index.unwrap()].position,
            selected
        );
    }
    press(&mut app, '/');
    press(&mut app, 'z');
    app.resize(120, 5);
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(
        wrap::search_hits(&app.cache, "needle")[app.match_index.unwrap()].position,
        selected
    );
    press(&mut app, 'v');
    press(&mut app, 't');
    key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(
        wrap::search_hits(&app.cache, "needle")[app.match_index.unwrap()].position,
        selected
    );
}

#[test]
fn continuation_reading_position_survives_roundtrip_resizes_and_toggles() {
    let mut app = app(&"word ".repeat(80), ViewMode::Unified, 25, 5);
    app.set_wrap(true);
    for _ in 0..13 {
        press(&mut app, 'j');
    }
    let original = wrap::row_position(&app.cache.rows[app.scroll]);
    assert!(original.offset > 0);
    for width in [31, 48, 25] {
        app.resize(width, 5);
    }
    assert_eq!(wrap::row_position(&app.cache.rows[app.scroll]), original);
    press(&mut app, 'w');
    assert!(app.horizontal > 0);
    press(&mut app, 'w');
    assert_eq!(wrap::row_position(&app.cache.rows[app.scroll]), original);
}

#[test]
fn wrapped_navigation_does_not_rebuild_and_anchors_remain_valid() {
    let mut app = app(&"word ".repeat(80), ViewMode::Unified, 25, 7);
    app.set_wrap(true);
    let pointer = app.cache.rows.as_ptr();
    press(&mut app, ' ');
    assert_eq!(app.cache.rows.as_ptr(), pointer);
    assert_eq!(app.scroll, 4);
    press(&mut app, 'n');
    assert_eq!(app.scroll, app.cache.hunk_starts[0]);
    assert!(matches!(
        app.cache.rows[app.cache.hunk_starts[0]].source,
        SourceId::Hunk(0, 0)
    ));
    press(&mut app, ']');
    assert_eq!(app.scroll, app.cache.file_starts[0]);
    press(&mut app, 'l');
    assert_eq!(app.horizontal, 0);
}

#[test]
fn shifted_space_is_exact_reverse_page_and_ignores_releases() {
    let mut app = app(&"word ".repeat(80), ViewMode::Unified, 25, 7);
    app.set_wrap(true);
    for height in [0, 1, 7, 12] {
        app.resize(25, height);
        app.scroll = 0;
        press(&mut app, ' ');
        let page = app.viewport_height.max(1);
        assert_eq!(app.scroll, page.min(app.maximum_scroll()));
        key(&mut app, KeyCode::Char(' '), KeyModifiers::SHIFT);
        assert_eq!(app.scroll, 0);
        key(&mut app, KeyCode::Char(' '), KeyModifiers::SHIFT);
        assert_eq!(app.scroll, 0);
        app.scroll = app.maximum_scroll();
        let end = app.scroll;
        key(&mut app, KeyCode::Char(' '), KeyModifiers::SHIFT);
        assert_eq!(app.scroll, end.saturating_sub(page));
    }
    let scroll = app.scroll;
    app.handle_event(Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char(' '),
        KeyModifiers::SHIFT,
        KeyEventKind::Release,
    )));
    assert_eq!(app.scroll, scroll);
}

#[test]
fn shifted_space_keeps_search_picker_help_and_ctrl_c_semantics() {
    let mut app = app(&"word ".repeat(80), ViewMode::Unified, 25, 7);
    app.set_wrap(true);
    press(&mut app, ' ');
    let scroll = app.scroll;
    press(&mut app, 't');
    key(&mut app, KeyCode::Char(' '), KeyModifiers::SHIFT);
    assert_eq!(app.scroll, scroll);
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut app, '/');
    key(&mut app, KeyCode::Char(' '), KeyModifiers::SHIFT);
    assert_eq!(app.query, " ");
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    press(&mut app, '?');
    key(&mut app, KeyCode::Char(' '), KeyModifiers::SHIFT);
    assert_eq!(app.scroll, scroll);
    press(&mut app, '?');
    key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(!app.running);
}

#[test]
fn unequal_context_panes_map_matches_to_the_same_visual_row_as_source_positions() {
    let text = format!("{}needle{}", "abcde".repeat(17), "xyz".repeat(20));
    let patch = format!("--- a/f\n+++ b/f\n@@ -1 +1 @@\n {text}\n");
    let mut app = App::new(
        parse_patch(&patch).unwrap(),
        builtin_themes(),
        0,
        ViewMode::Split,
        false,
        "test".into(),
    )
    .unwrap();
    app.resize(40, 6);
    app.set_wrap(true);
    search(&mut app, "needle");
    for width in [40, 42, 44, 46, 48, 50, 39, 40] {
        app.resize(width, 6);
        let hits = wrap::search_hits(&app.cache, "needle");
        assert_eq!(hits.len(), 1, "shared context must not be counted twice");
        assert_eq!(
            wrap::source_row(&app.cache, hits[0].position),
            Some(hits[0].row)
        );
        assert_eq!(app.current_match_row(), Some(hits[0].row));
        assert!(hits[0].row >= app.scroll && hits[0].row < app.scroll + app.viewport_height);
    }
}

#[test]
fn unicode_lowercase_offsets_and_long_tokens_are_lossless() {
    let text = format!("{}İneedle界{}", "x".repeat(10_000), "tail".repeat(300));
    let mut app = app(&text, ViewMode::Unified, 18, 5);
    app.set_wrap(true);
    search(&mut app, "i\u{307}needle");
    let hits = wrap::search_hits(&app.cache, "i\u{307}needle");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].position.offset, 10_000);
    assert_eq!(
        wrap::source_row(&app.cache, hits[0].position),
        Some(hits[0].row)
    );
    assert_eq!(
        cells(&app, hits[0].position.source)
            .iter()
            .map(|cell| cell.text())
            .collect::<String>(),
        text
    );
}

#[test]
fn wrapped_tiny_test_backends_are_safe_and_legacy_cache_stays_unwrapped() {
    for view in [ViewMode::Unified, ViewMode::Split] {
        let mut app = app("界abc \tlong_token_xxxxx", view, 20, 8);
        let legacy = build_cache(&app.diff, app.theme(), view, 20, true);
        assert!(legacy.search_rows.is_empty());
        app.set_wrap(true);
        for width in 1..15 {
            for height in 1..6 {
                app.resize(width, height);
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            }
        }
    }
}
