//! Every palette is exercised against the actual TUI test backend.
use ratatui::{Terminal, backend::TestBackend, style::Color};
use riff::{app::App, cli::ViewMode, patch::parse_patch, theme::builtin_themes, ui};

#[test]
fn typescript_and_tsx_reach_tui_cells_with_independent_old_new_state() {
    let diff = parse_patch(include_str!("fixtures/typescript.patch")).unwrap();
    let themes = builtin_themes();
    for (index, theme) in themes[..2].iter().enumerate() {
        for (view, width) in [(ViewMode::Unified, 100), (ViewMode::Split, 160)] {
            let mut app = App::new(
                diff.clone(),
                themes.clone(),
                index,
                view,
                true,
                "test".into(),
            )
            .unwrap();
            app.resize(width, 24);
            let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            let buffer = terminal.backend().buffer();
            let assert_token = |needle: &str, token: &str, colors: &[Color]| {
                let row = (0..24)
                    .find(|&y| {
                        (0..width)
                            .map(|x| buffer[(x, y)].symbol())
                            .collect::<String>()
                            .contains(needle)
                    })
                    .expect("code row is visible");
                let text: String = (0..width).map(|x| buffer[(x, row)].symbol()).collect();
                let matches: Vec<_> = text.match_indices(token).collect();
                assert_eq!(matches.len(), colors.len(), "{view:?}: {text}");
                for ((offset, _), color) in matches.into_iter().zip(colors) {
                    let x = text[..offset].chars().count() as u16;
                    for dx in 0..token.chars().count() as u16 {
                        assert_eq!(
                            buffer[(x + dx, row)].fg,
                            *color,
                            "{} {view:?}: {token}",
                            theme.name
                        );
                    }
                }
            };
            if view == ViewMode::Split {
                assert_token(
                    "interface",
                    "interface",
                    &[theme.palette.comment, theme.palette.keyword],
                );
            } else {
                assert_token("interface", "interface", &[theme.palette.keyword]);
            }
            let copies = if view == ViewMode::Split { 2 } else { 1 };
            let inherited = if view == ViewMode::Split {
                vec![theme.palette.comment, theme.palette.type_name]
            } else {
                vec![theme.palette.type_name]
            };
            assert_token("interface", "Base", &inherited);
            for needle in ["const checked", "const casted"] {
                assert_token(needle, "satisfies", &vec![theme.palette.keyword; copies]);
            }
            assert_token(
                "const label",
                "Person",
                &vec![theme.palette.type_name; copies],
            );
            assert_token("const label", "hello", &vec![theme.palette.string; copies]);
            assert_token("span title", "span", &[theme.palette.variable; 2]);
            assert_token("span title", "title", &[theme.palette.variable]);
            assert_token("span title", "ready", &[theme.palette.variable]);
            assert_token("span title", "雪", &[theme.palette.string]);
            assert_token("span title", "null", &[theme.palette.number]);
        }
    }
}

#[test]
fn every_theme_renders_both_layouts_with_its_own_surfaces_and_syntax() {
    let diff = parse_patch(
        "--- a/example.rs\n+++ b/example.rs\n@@ -1,2 +1,2 @@\n-fn before() {}\n+fn after() {}\n // unchanged\n",
    )
    .unwrap();
    let themes = builtin_themes();
    for (index, theme) in themes.iter().enumerate() {
        for (view, width) in [(ViewMode::Unified, 80), (ViewMode::Split, 120)] {
            let mut app = App::new(
                diff.clone(),
                themes.clone(),
                index,
                view,
                true,
                "test".into(),
            )
            .unwrap();
            app.resize(width, 24);
            let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            let cells = terminal.backend().buffer().content();
            for expected in [
                theme.palette.background,
                theme.palette.header,
                theme.palette.hunk,
                theme.palette.added_background,
                theme.palette.removed_background,
                theme.palette.added_gutter,
                theme.palette.removed_gutter,
            ] {
                assert!(
                    cells.iter().any(|cell| cell.bg == expected),
                    "{} {view:?}: missing {expected:?}",
                    theme.name
                );
            }
            assert!(
                cells
                    .iter()
                    .any(|cell| cell.fg == theme.palette.keyword && cell.symbol() == "f")
            );
            assert!(
                cells
                    .iter()
                    .any(|cell| cell.fg == theme.palette.comment && cell.symbol() == "/")
            );
            assert!(!cells.iter().any(|cell| cell.bg == Color::Reset));
        }
    }
}
