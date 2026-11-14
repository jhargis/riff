//! Word edits are behavioral ranges/backgrounds, never incidental span counts.
use std::{
    fs,
    io::Write,
    ops::Range,
    process::{Command, Stdio},
};

use clap::Parser;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Terminal,
    backend::TestBackend,
    style::{Color, Style},
    text::Span,
};
use riff::{
    app::App,
    cli::{Cli, ViewMode},
    config::{SettingsUpdate, load_config, save_config},
    intraline::{
        self, MAX_LINE_BYTES, MAX_LINE_TOKENS, MAX_RUN_BYTES, MAX_RUN_PAIRS, MAX_RUN_WORK,
    },
    patch::{Diff, LineKind, parse_patch, sanitize},
    reload::ReloadEvent,
    render::{
        CodeCell, RenderCache, RowContent, SourceId, build_cache_with_word_diff, clip_spans,
        print_diff_with_word_diff,
    },
    theme::{Theme, builtin_themes},
    ui, wrap,
};
use unicode_segmentation::UnicodeSegmentation;

const PATCH: &str =
    "--- a/x.rs\n+++ b/x.rs\n@@ -1,2 +1,2 @@\n-timeout = 30\n+timeout = 60\n context = 30\n";

fn replacement(old: &[&str], new: &[&str]) -> Diff {
    let body: String = old
        .iter()
        .map(|s| format!("-{s}\n"))
        .chain(new.iter().map(|s| format!("+{s}\n")))
        .collect();
    parse_patch(&format!(
        "--- a/x.rs\n+++ b/x.rs\n@@ -1,{} +1,{} @@\n{body}",
        old.len(),
        new.len()
    ))
    .unwrap()
}

fn cells(cache: &RenderCache) -> Vec<&CodeCell> {
    cache
        .rows
        .iter()
        .flat_map(|row| match &row.content {
            RowContent::Unified(cell) => vec![cell],
            RowContent::Split { left, right } => left.iter().chain(right).collect(),
            _ => vec![],
        })
        .collect()
}

fn cache(diff: &Diff, theme: &Theme, view: ViewMode, syntax: bool, enabled: bool) -> RenderCache {
    build_cache_with_word_diff(diff, theme, view, 120, syntax, enabled)
}

fn marked(cell: &CodeCell) -> String {
    cell.spans
        .iter()
        .filter(|s| s.style.bg.is_some())
        .map(|s| s.content.as_ref())
        .collect()
}

fn bytes(cell: &CodeCell) -> Vec<(u8, Style)> {
    cell.spans
        .iter()
        .flat_map(|span| span.content.bytes().map(|b| (b, span.style)))
        .collect()
}

fn edits<'a>(text: &'a str, ranges: &[Range<usize>]) -> Vec<&'a str> {
    ranges.iter().map(|range| &text[range.clone()]).collect()
}

#[test]
fn exact_timeout_entire_numeric_tokens_all_themes_layouts_and_syntax_modes() {
    let diff = parse_patch(PATCH).unwrap();
    for theme in builtin_themes() {
        for view in [ViewMode::Unified, ViewMode::Split] {
            for syntax in [false, true] {
                let on = cache(&diff, &theme, view, syntax, true);
                let off = cache(&diff, &theme, view, syntax, false);
                assert_eq!(on.rows.len(), off.rows.len());
                assert_eq!(on.file_starts, off.file_starts);
                assert_eq!(on.hunk_starts, off.hunk_starts);
                for (a, b) in cells(&on).into_iter().zip(cells(&off)) {
                    assert_eq!(a.source, b.source);
                    assert_eq!((a.old_number, a.new_number), (b.old_number, b.new_number));
                    assert_eq!(a.text(), b.text());
                    let expected = match a.kind {
                        LineKind::Removed => "30",
                        LineKind::Added => "60",
                        _ => "",
                    };
                    assert_eq!(marked(a), expected, "{} {view:?}", theme.name);
                    for ((ch, style), (other, original)) in bytes(a).into_iter().zip(bytes(b)) {
                        assert_eq!(ch, other);
                        assert_eq!(style.fg, original.fg, "syntax foreground must survive");
                        assert_eq!(style.add_modifier, original.add_modifier);
                        assert_eq!(style.sub_modifier, original.sub_modifier);
                        if let Some(bg) = style.bg {
                            assert_eq!(
                                bg,
                                if a.kind == LineKind::Added {
                                    theme.palette.added_word_background
                                } else {
                                    theme.palette.removed_word_background
                                }
                            );
                        }
                    }
                }
                assert_eq!(
                    on.rows.iter().map(|r| &r.search_text).collect::<Vec<_>>(),
                    off.rows.iter().map(|r| &r.search_text).collect::<Vec<_>>()
                );
            }
        }
    }
}

#[test]
fn reference_palettes_keep_subtle_lines_and_restrained_word_contrast() {
    let themes = builtin_themes();
    for (name, expected) in [
        ("critique", [0x131a14_u32, 0x2d0d0d, 0x1b4628, 0x5e2020]),
        ("github-dark", [0x13233a, 0x32161d, 0x243e5d, 0x5a2029]),
    ] {
        let palette = &themes
            .iter()
            .find(|theme| theme.name == name)
            .unwrap()
            .palette;
        assert_eq!(
            [
                palette.added_background,
                palette.removed_background,
                palette.added_word_background,
                palette.removed_word_background,
            ],
            expected.map(|rgb| Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)),
            "{name}: preserve the approved red/green or red/blue backgrounds"
        );
    }
}

#[test]
fn multiple_identifiers_operators_inserted_deleted_tokens_and_whitespace() {
    for (old, new, removed, added) in [
        (
            "let timeout = 30 + retry;",
            "let timeout = 60 - retries;",
            vec!["30", "+", "retry"],
            vec!["60", "-", "retries"],
        ),
        ("value >= limit", "value != limit", vec![">="], vec!["!="]),
        ("call(value)", "call(extra, value)", vec![], vec!["extra, "]),
        ("call(extra, value)", "call(value)", vec!["extra, "], vec![]),
        ("let value = 1", "let  value = 1", vec![" "], vec!["  "]),
        ("", "  ", vec![], vec!["  "]),
        ("  ", "", vec!["  "], vec![]),
        ("", "", vec![], vec![]),
    ] {
        let mut budget = MAX_RUN_WORK;
        let (a, b) = intraline::pair(old, new, &mut budget).unwrap();
        assert_eq!(edits(old, &a), removed, "{old} -> {new}");
        assert_eq!(edits(new, &b), added, "{old} -> {new}");
    }
}

#[test]
fn unicode_graphemes_zwj_tabs_and_controls_use_sanitized_offsets() {
    let old = "let unchanged_identifier cafe\u{301} = \"👨‍👩‍👧‍👦\";\t\u{1b}a\u{85}";
    let new = "let unchanged_identifier cafe\u{300} = \"👩‍🚀\"; \u{1b}b\u{85}";
    let diff = replacement(&[old], &[new]);
    for syntax in [false, true] {
        let cache = cache(&diff, &builtin_themes()[0], ViewMode::Unified, syntax, true);
        for (cell, raw) in cells(&cache).into_iter().zip([old, new]) {
            let text = sanitize(raw);
            assert_eq!(cell.text(), text);
            assert!(!cell.text().chars().any(char::is_control));
            let mut offset = 0;
            for span in &cell.spans {
                assert!(text.is_char_boundary(offset));
                offset += span.content.len();
            }
            assert!(marked(cell).contains(if cell.kind == LineKind::Added {
                "cafe\u{300}"
            } else {
                "cafe\u{301}"
            }));
            assert!(marked(cell).contains(if cell.kind == LineKind::Added {
                "👩‍🚀"
            } else {
                "👨‍👩‍👧‍👦"
            }));
            let ranges = &intraline::hunk_ranges(&diff.files[0].hunks[0].lines)
                [if cell.kind == LineKind::Removed { 0 } else { 1 }];
            let boundaries: Vec<_> = text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .chain([text.len()])
                .collect();
            for range in ranges {
                assert!(boundaries.contains(&range.start) && boundaries.contains(&range.end));
            }
        }
    }
    let diff = replacement(&["value\t= 30"], &["value = 60"]);
    let ranges = intraline::hunk_ranges(&diff.files[0].hunks[0].lines);
    assert_eq!(edits(&sanitize("value\t= 30"), &ranges[0]), ["    ", "30"]);
}

#[test]
fn changed_unicode_and_sanitized_text_keep_every_byte_and_style_when_wrapped() {
    let old = "let unchanged_identifier cafe\u{301} = \"👨‍👩‍👧‍👦\";\t\u{1b}a\u{85}";
    let new = "let unchanged_identifier cafe\u{300} = \"👩‍🚀\"; \u{1b}b\u{85}";
    let diff = replacement(&[old], &[new]);
    for view in [ViewMode::Unified, ViewMode::Split] {
        for syntax in [false, true] {
            let original = cache(&diff, &builtin_themes()[0], view, syntax, true);
            for width in [1, 8, 15, 40] {
                let mut wrapped = original.clone();
                wrap::wrap_cache(&mut wrapped, width);
                for logical in cells(&original) {
                    let fragments: Vec<_> = cells(&wrapped)
                        .into_iter()
                        .filter(|c| c.source == logical.source)
                        .collect();
                    assert_eq!(
                        fragments.iter().map(|c| c.text()).collect::<String>(),
                        logical.text()
                    );
                    assert_eq!(
                        fragments.iter().flat_map(|c| bytes(c)).collect::<Vec<_>>(),
                        bytes(logical)
                    );
                    let mut offset = 0;
                    let text = logical.text();
                    let boundaries: Vec<_> = text
                        .grapheme_indices(true)
                        .map(|(i, _)| i)
                        .chain([text.len()])
                        .collect();
                    for fragment in fragments {
                        assert_eq!(fragment.offset, offset);
                        assert!(boundaries.contains(&offset), "wrapped inside a grapheme");
                        offset += fragment.text().len();
                    }
                }
            }
        }
    }
}

#[test]
fn conservative_pairing_falls_back_for_pure_unequal_shifted_or_unrelated_runs() {
    for (old, new) in [
        (vec![], vec!["timeout = 60"]),
        (vec!["timeout = 30"], vec![]),
        (vec!["timeout = 30"], vec!["inserted = 1", "timeout = 60"]),
        (
            vec!["alpha = 30", "beta = 40"],
            vec!["beta = 40", "alpha = 60"],
        ),
        (vec!["alpha = 30"], vec!["omega = 60"]),
        (
            vec!["timeout = 30", "totally unrelated"],
            vec!["timeout = 60", "different words"],
        ),
    ] {
        let diff = replacement(&old, &new);
        let rendered = cache(&diff, &builtin_themes()[0], ViewMode::Unified, false, true);
        assert!(cells(&rendered).iter().all(|c| marked(c).is_empty()));
        let actual: Vec<_> = cells(&rendered).iter().map(|c| c.text()).collect();
        assert_eq!(
            actual,
            old.iter()
                .chain(&new)
                .map(|s| sanitize(s))
                .collect::<Vec<_>>()
        );
    }
    let diff = replacement(
        &["timeout = 30", "retry = 10"],
        &["timeout = 60", "retry = 20"],
    );
    for view in [ViewMode::Unified, ViewMode::Split] {
        let cache = cache(&diff, &builtin_themes()[0], view, false, true);
        let mut actual: Vec<_> = cells(&cache)
            .iter()
            .map(|c| (c.source, marked(c)))
            .collect();
        actual.sort_by_key(|(source, _)| match source {
            SourceId::Line(_, _, i) => *i,
            _ => 0,
        });
        assert_eq!(
            actual.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(),
            ["30", "10", "60", "20"]
        );
    }
}

#[test]
fn boundaries_and_eof_notes_are_never_code_or_cross_hunk_pairs() {
    let diff = parse_patch("--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n-timeout = 30\n context\n+timeout = 60\n@@ -5 +5,0 @@\n-timeout = 30\n--- a/y\n+++ b/y\n@@ -0,0 +1 @@\n+timeout = 60\n").unwrap();
    for view in [ViewMode::Unified, ViewMode::Split] {
        assert!(
            cells(&cache(&diff, &builtin_themes()[0], view, true, true))
                .iter()
                .all(|c| marked(c).is_empty())
        );
    }
    let diff = parse_patch("--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n-timeout = 30\n\\ No newline at end of file\n+timeout = 60\n\\ No newline at end of file\n").unwrap();
    for view in [ViewMode::Unified, ViewMode::Split] {
        let cache = cache(&diff, &builtin_themes()[0], view, true, true);
        for cell in cells(&cache) {
            assert_eq!(
                marked(cell),
                match cell.kind {
                    LineKind::Added => "60",
                    LineKind::Removed => "30",
                    _ => "",
                }
            );
        }
    }
}

#[test]
fn all_budgets_fall_back_without_truncating_any_text() {
    let long = format!("value = {}", "x".repeat(MAX_LINE_BYTES));
    let tokens = format!("value = {}", "a ".repeat(MAX_LINE_TOKENS));
    let expanded = format!("value = {}", "\t".repeat(MAX_LINE_BYTES / 2));
    let bytes = "a".repeat(MAX_RUN_BYTES / (MAX_RUN_PAIRS * 2) + 1);
    for (old, new) in [
        (vec![long.as_str()], vec!["value = y"]),
        (vec![tokens.as_str()], vec!["value = b"]),
        (vec![expanded.as_str()], vec!["value = y"]),
        (
            vec!["value = 30"; MAX_RUN_PAIRS + 1],
            vec!["value = 60"; MAX_RUN_PAIRS + 1],
        ),
        (
            vec![bytes.as_str(); MAX_RUN_PAIRS],
            vec![bytes.as_str(); MAX_RUN_PAIRS],
        ),
    ] {
        let diff = replacement(&old, &new);
        let cache = cache(&diff, &builtin_themes()[0], ViewMode::Unified, false, true);
        assert_eq!(
            cells(&cache).iter().map(|c| c.text()).collect::<Vec<_>>(),
            old.iter()
                .chain(&new)
                .map(|s| sanitize(s))
                .collect::<Vec<_>>()
        );
        assert!(cells(&cache).iter().all(|c| marked(c).is_empty()));
    }
    assert!(intraline::pair("value = 30", "value = 60", &mut 0).is_none());
    // Distinct lines prevent the shifted-exact-line guard from hiding the work limit.
    let old: Vec<_> = (0..MAX_RUN_PAIRS)
        .map(|i| format!("line{i} {}30", "a + ".repeat(85)))
        .collect();
    let new: Vec<_> = (0..MAX_RUN_PAIRS)
        .map(|i| format!("line{i} {}60", "a + ".repeat(85)))
        .collect();
    let mut budget = MAX_RUN_WORK;
    assert!(intraline::pair(&old[0], &new[0], &mut budget).is_some());
    let old: Vec<_> = old.iter().map(String::as_str).collect();
    let new: Vec<_> = new.iter().map(String::as_str).collect();
    let diff = replacement(&old, &new);
    assert!(
        intraline::hunk_ranges(&diff.files[0].hunks[0].lines)
            .iter()
            .all(Vec::is_empty)
    );
}

#[test]
fn overlay_intersects_styles_without_changing_them_or_text() {
    let a = Style::default().fg(Color::Blue);
    let b = Style::default().fg(Color::Yellow);
    let original = vec![Span::styled("timeout = 3", a), Span::styled("0", b)];
    let over = intraline::overlay(original, std::slice::from_ref(&(10..12)), Color::Red);
    assert_eq!(
        over.iter().map(|s| s.content.as_ref()).collect::<String>(),
        "timeout = 30"
    );
    let flat: Vec<_> = over
        .iter()
        .flat_map(|s| s.content.chars().map(|ch| (ch, s.style)))
        .collect();
    assert!(flat[..10].iter().all(|(_, s)| s.bg.is_none()));
    assert_eq!(flat[10].1, a.bg(Color::Red));
    assert_eq!(flat[11].1, b.bg(Color::Red));
}

fn app(view: ViewMode, width: u16) -> App {
    let mut app = App::new(
        parse_patch(PATCH).unwrap(),
        builtin_themes(),
        0,
        view,
        true,
        "test".into(),
    )
    .unwrap();
    app.resize(width, 24);
    app
}
fn key(app: &mut App, code: KeyCode) {
    app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

#[test]
fn wrap_clip_buffers_and_selected_search_preserve_word_backgrounds_and_syntax() {
    for view in [ViewMode::Unified, ViewMode::Split] {
        let mut app = app(view, 120);
        for wrapped in [false, true] {
            app.set_wrap(wrapped);
            app.resize(if wrapped { 30 } else { 120 }, 24);
            key(&mut app, KeyCode::Char('/'));
            for ch in "60".chars() {
                key(&mut app, KeyCode::Char(ch));
            }
            key(&mut app, KeyCode::Enter);
            assert_eq!(app.search_matches.len(), 1);
            let mut terminal = Terminal::new(TestBackend::new(app.dimensions.0, 24)).unwrap();
            terminal.draw(|f| ui::draw(f, &app)).unwrap();
            let p = &app.theme().palette;
            for ch in ["6", "0"] {
                assert!(
                    terminal
                        .backend()
                        .buffer()
                        .content()
                        .iter()
                        .any(|c| c.symbol() == ch
                            && c.bg == p.added_word_background
                            && c.fg == p.number)
                );
            }
            let cell = cells(&app.cache)
                .into_iter()
                .find(|c| c.kind == LineKind::Added && marked(c).contains("60"))
                .unwrap();
            let offset = cell.text().find("60").unwrap();
            let clipped = clip_spans(&cell.spans, offset + 1, 1);
            assert_eq!(clipped.to_string(), "0");
            assert_eq!(clipped.spans[0].style.bg, Some(p.added_word_background));
            key(&mut app, KeyCode::Esc);
        }
    }
}

#[test]
fn toggle_keeps_reading_search_wrap_and_scroll_and_never_requests_git() {
    let text = "prefix ".repeat(12);
    let diff = replacement(
        &[&format!("{text}timeout = 30")],
        &[&format!("{text}timeout = 60")],
    );
    for view in [ViewMode::Unified, ViewMode::Split] {
        let mut app = app(view, 40);
        app.handle_reload_event(ReloadEvent::Updated(diff.clone()));
        app.set_word_diff(false);
        assert_eq!(app.take_settings_update(), SettingsUpdate::default());
        app.configure_reload(true, false);
        app.set_wrap(true);
        key(&mut app, KeyCode::Char('/'));
        for ch in "60".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        key(&mut app, KeyCode::Enter);
        let before = (
            app.scroll,
            app.horizontal,
            app.query.clone(),
            app.search_matches.clone(),
            app.match_index,
            wrap::row_position(&app.cache.rows[app.scroll]),
        );
        key(&mut app, KeyCode::Char('i'));
        assert!(app.word_diff && app.wrap);
        assert_eq!(
            before,
            (
                app.scroll,
                app.horizontal,
                app.query.clone(),
                app.search_matches.clone(),
                app.match_index,
                wrap::row_position(&app.cache.rows[app.scroll])
            )
        );
        assert_eq!(
            app.take_settings_update(),
            SettingsUpdate {
                word_diff: Some(true),
                ..SettingsUpdate::default()
            }
        );
        assert_eq!(app.take_reload_requests(), Default::default());
        assert!(cells(&app.cache).iter().any(|c| marked(c) == "60"));
        key(&mut app, KeyCode::Char('i'));
        app.handle_reload_event(ReloadEvent::Updated(parse_patch(PATCH).unwrap()));
        assert!(!app.word_diff);
        assert!(cells(&app.cache).iter().all(|c| marked(c).is_empty()));
        key(&mut app, KeyCode::Char('i'));
        app.handle_reload_event(ReloadEvent::Updated(diff.clone()));
        assert!(cells(&app.cache).iter().any(|c| marked(c) == "60"));
    }
}

#[test]
fn toggle_has_no_modal_modifier_or_release_leaks() {
    let mut app = app(ViewMode::Unified, 80);
    for modifier in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::SHIFT,
        KeyModifiers::SUPER,
    ] {
        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Char('i'), modifier)));
        assert!(app.word_diff);
    }
    app.handle_event(Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char('i'),
        KeyModifiers::NONE,
        KeyEventKind::Release,
    )));
    for modal in ['?', 't', '/'] {
        key(&mut app, KeyCode::Char(modal));
        key(&mut app, KeyCode::Char('i'));
        assert!(app.word_diff);
        key(&mut app, KeyCode::Esc);
    }
    assert_eq!(app.take_settings_update(), SettingsUpdate::default());
    key(&mut app, KeyCode::Char('i'));
    assert!(!app.word_diff);
}

#[test]
fn strict_boolean_custom_colors_inheritance_and_selective_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    fs::write(&path, "").unwrap();
    assert!(load_config(Some(&path)).unwrap().word_diff);
    for invalid in ["'true'", "1", "[]", "{}"] {
        fs::write(&path, format!("word_diff = {invalid}")).unwrap();
        assert!(load_config(Some(&path)).is_err());
    }
    for invalid in ["#fff", "red", "#12345678", "#xx0000"] {
        fs::write(
            &path,
            format!("[themes.mine]\nadded_word_background = '{invalid}'"),
        )
        .unwrap();
        assert!(load_config(Some(&path)).is_err());
    }
    let original = "# settings\ntheme = 'mine'\nword_diff = false # preference\nwrap = true\nwatch = true\n\n[themes.mine]\nextends = 'github-dark'\nadded_word_background = '#123456' # token\n";
    fs::write(&path, original).unwrap();
    let settings = load_config(Some(&path)).unwrap();
    let p = &settings.themes.last().unwrap().palette;
    assert_eq!(p.added_word_background, Color::Rgb(0x12, 0x34, 0x56));
    assert_eq!(
        p.removed_word_background,
        builtin_themes()[9].palette.removed_word_background
    );
    let rendered = cache(
        &parse_patch(PATCH).unwrap(),
        settings.themes.last().unwrap(),
        ViewMode::Unified,
        true,
        true,
    );
    for cell in cells(&rendered)
        .into_iter()
        .filter(|c| c.kind == LineKind::Added)
    {
        assert!(cell.spans.iter().any(|s| s.content == "60"
            && s.style.bg == Some(p.added_word_background)
            && s.style.fg == Some(p.number)));
    }
    save_config(
        Some(&path),
        &SettingsUpdate {
            word_diff: Some(true),
            ..SettingsUpdate::default()
        },
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        original.replace("word_diff = false", "word_diff = true")
    );
    assert!(load_config(Some(&path)).unwrap().word_diff);
    fs::write(&path, "word_diff = 'broken'\n").unwrap();
    assert!(
        save_config(
            Some(&path),
            &SettingsUpdate {
                word_diff: Some(false),
                ..SettingsUpdate::default()
            }
        )
        .is_err()
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "word_diff = 'broken'\n");
}

#[test]
fn print_plain_is_byte_identical_and_ansi_uses_background_only() {
    let diff = parse_patch(PATCH).unwrap();
    for syntax in [false, true] {
        let mut plain = Vec::new();
        for enabled in [false, true] {
            let mut out = Vec::new();
            print_diff_with_word_diff(
                &mut out,
                &diff,
                &builtin_themes()[0],
                syntax,
                false,
                enabled,
            )
            .unwrap();
            assert!(!out.contains(&27));
            if !enabled {
                plain = out;
            } else {
                assert_eq!(out, plain);
            }
        }
        assert_eq!(
            String::from_utf8(plain).unwrap(),
            "── x.rs  +1 -1\n--- x.rs\n+++ x.rs\n@@ -1,2 +1,2 @@\n1   - timeout = 30\n  1 + timeout = 60\n2 2   context = 30\n"
        );
        let mut out = Vec::new();
        print_diff_with_word_diff(&mut out, &diff, &builtin_themes()[0], syntax, true, true)
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("\x1b[48;2;27;70;40m60\x1b[0m"));
        assert!(text.contains("\x1b[48;2;94;32;32m30\x1b[0m"));
        assert!(!text.contains("\x1b[1m"));
    }
}

#[test]
fn cli_precedence_exclusivity_and_no_startup_persistence_in_print() {
    assert!(Cli::try_parse_from(["riff", "--word-diff", "--no-word-diff"]).is_err());
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    for saved in [false, true] {
        let original = format!("# untouched\nword_diff = {saved}\n");
        fs::write(&config, &original).unwrap();
        for flag in [None, Some("--word-diff"), Some("--no-word-diff")] {
            for color in ["always", "never"] {
                let mut command = Command::new(env!("CARGO_BIN_EXE_riff"));
                command
                    .args(["--stdin", "--print", "--color", color, "--config"])
                    .arg(&config)
                    .current_dir(dir.path())
                    .env_remove("RIFF_THEME")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
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
                let out = child.wait_with_output().unwrap();
                assert!(
                    out.status.success(),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                let text = String::from_utf8(out.stdout).unwrap();
                let enabled = flag.map_or(saved, |s| s == "--word-diff");
                assert_eq!(
                    text.contains("\x1b[48;2;27;70;40m60"),
                    enabled && color == "always"
                );
                if color == "never" {
                    assert!(!text.contains('\x1b'));
                }
                assert_eq!(fs::read_to_string(&config).unwrap(), original);
            }
        }
    }
}
