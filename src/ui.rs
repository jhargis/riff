//! Drawing only: all expensive parsing and highlighting lives in the row cache.
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use unicode_width::UnicodeWidthStr;

use crate::{
    app::App,
    patch::sanitize,
    render::{
        CodeCell, LabelKind, RenderRow, RowContent, clip_spans, label_style, split_gutter,
        unified_gutter,
    },
    theme::Palette,
};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let palette = &app.theme().palette;
    frame.render_widget(
        Block::default().style(
            Style::default()
                .bg(palette.background)
                .fg(palette.foreground),
        ),
        area,
    );
    if area.width == 0 || area.height == 0 {
        return;
    }
    let header = format!(
        " riff · {} · {} · {} files  +{} -{}",
        app.reload_status(),
        sanitize(&app.source_label),
        app.diff.files.len(),
        app.additions,
        app.deletions
    );
    frame.render_widget(
        Paragraph::new(header).style(
            Style::default()
                .fg(palette.foreground)
                .bg(palette.header)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(area.x, area.y, area.width, 1),
    );

    let footer_height = area.height.saturating_sub(1).min(2);
    let body_height = area.height.saturating_sub(1 + footer_height);
    let body = Rect::new(area.x, area.y.saturating_add(1), area.width, body_height);
    if app.cache.rows.is_empty() && body.height > 0 {
        frame.render_widget(
            Paragraph::new(" No differences.").style(Style::default().fg(palette.muted)),
            body,
        );
    } else {
        for (visible, row) in app
            .cache
            .rows
            .iter()
            .skip(app.scroll)
            .take(usize::from(body.height))
            .enumerate()
        {
            let row_area = Rect::new(body.x, body.y.saturating_add(visible as u16), body.width, 1);
            draw_row(
                frame,
                row,
                row_area,
                app,
                app.current_match_row() == Some(app.scroll + visible),
            );
        }
    }

    if footer_height > 0 {
        let layout = if app.cache.split { "split" } else { "unified" };
        let layout = if app.view == crate::cli::ViewMode::Auto {
            format!("auto→{layout}")
        } else {
            layout.to_string()
        };
        let rows = app.cache.rows.len();
        let first = if rows == 0 { 0 } else { app.scroll + 1 };
        let last = (app.scroll + usize::from(body.height)).min(rows);
        let mut status = format!(
            " {layout} · {} · file {}/{} · rows {first}-{last}/{rows} · {}",
            sanitize(&app.theme().name),
            if app.diff.files.is_empty() {
                0
            } else {
                app.current_file() + 1
            },
            app.diff.files.len(),
            if app.wrap {
                "wrap:on".into()
            } else {
                format!("clip x={}", app.horizontal)
            }
        );
        if !app.query.is_empty() {
            status.push_str(&format!(
                " · /{} [{}/{}]",
                sanitize(&app.query),
                app.match_index.map_or(0, |n| n + 1),
                app.search_matches.len()
            ));
        }
        status.push_str(if app.word_diff {
            " · words:on"
        } else {
            " · words:off"
        });
        let status_y = area.y.saturating_add(area.height - footer_height);
        if let Some(error) = app.reload_error() {
            status = format!(" Reload error: {}", sanitize(error));
        }
        if let Some(error) = &app.save_error {
            status = format!(" Save failed: {}", sanitize(error));
        }
        frame.render_widget(
            Paragraph::new(status)
                .style(Style::default().fg(palette.foreground).bg(palette.gutter)),
            Rect::new(area.x, status_y, area.width, 1),
        );
        if footer_height == 2 {
            let keys = if app.editing_search() {
                format!(
                    " /{}▏  Enter accept · Esc cancel · Ctrl-U clear",
                    sanitize(&app.query)
                )
            } else {
                " q quit · ? help · r reload · ctrl+w watch · w wrap · i words · / search · Ctrl-P files · t themes · v view · n/p hunks · ]/[ files".into()
            };
            frame.render_widget(
                Paragraph::new(keys)
                    .style(Style::default().fg(palette.muted).bg(palette.background)),
                Rect::new(area.x, status_y.saturating_add(1), area.width, 1),
            );
        }
    }
    if app.picker_original.is_some() {
        draw_picker(frame, app);
    }
    if app.file_picker.is_some() {
        draw_files(frame, app);
    }
    if app.help {
        draw_help(frame, app);
    }
}

fn draw_row(frame: &mut Frame<'_>, row: &RenderRow, area: Rect, app: &App, selected: bool) {
    if area.width == 0 {
        return;
    }
    let palette = &app.theme().palette;
    match &row.content {
        RowContent::Label { text, kind } => {
            let mut style = label_style(*kind, palette);
            if *kind == LabelKind::File {
                style = style.add_modifier(Modifier::BOLD);
            }
            if selected {
                style = style.bg(palette.selection);
            }
            let line = clip_spans(
                &[Span::raw(text.clone())],
                app.horizontal,
                usize::from(area.width),
            );
            frame.render_widget(Paragraph::new(line).style(style), area);
        }
        RowContent::Unified(cell) => draw_cell(frame, Some(cell), area, app, None, selected),
        RowContent::Split { left, right } => {
            let left_width = area.width.saturating_sub(1) / 2;
            let separator_x = area.x.saturating_add(left_width);
            let right_width = area.width.saturating_sub(left_width + 1);
            draw_cell(
                frame,
                left.as_ref(),
                Rect::new(area.x, area.y, left_width, 1),
                app,
                Some(true),
                selected,
            );
            frame.render_widget(
                Paragraph::new("│")
                    .style(Style::default().fg(palette.border).bg(palette.background)),
                Rect::new(separator_x, area.y, 1, 1),
            );
            draw_cell(
                frame,
                right.as_ref(),
                Rect::new(separator_x.saturating_add(1), area.y, right_width, 1),
                app,
                Some(false),
                selected,
            );
        }
    }
}

fn draw_cell(
    frame: &mut Frame<'_>,
    cell: Option<&CodeCell>,
    area: Rect,
    app: &App,
    side: Option<bool>,
    selected: bool,
) {
    if area.width == 0 {
        return;
    }
    let palette = &app.theme().palette;
    let Some(cell) = cell else {
        let background = if selected {
            palette.selection
        } else {
            palette.background
        };
        frame.render_widget(
            Block::default().style(Style::default().bg(background)),
            area,
        );
        return;
    };
    let gutter = match side {
        Some(left) => split_gutter(cell, app.cache.digits, left),
        None => unified_gutter(cell, app.cache.digits),
    };
    let gutter_width = (gutter.len().min(usize::from(area.width))) as u16;
    let mut gutter_style = cell.gutter_style(palette);
    let mut code_style = cell.style(palette);
    if selected {
        gutter_style = gutter_style
            .bg(palette.selection)
            .add_modifier(Modifier::BOLD);
        code_style = code_style.bg(palette.selection);
    }
    frame.render_widget(
        Paragraph::new(gutter).style(gutter_style),
        Rect::new(area.x, area.y, gutter_width, 1),
    );
    let code_width = area.width.saturating_sub(gutter_width);
    if code_width > 0 {
        let line = clip_spans(&cell.spans, app.horizontal, usize::from(code_width));
        frame.render_widget(
            Paragraph::new(line).style(code_style),
            Rect::new(area.x.saturating_add(gutter_width), area.y, code_width, 1),
        );
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x.saturating_add((area.width - width) / 2),
        area.y.saturating_add((area.height - height) / 2),
        width,
        height,
    )
}

fn popup_block<'a>(title: &'a str, palette: &Palette) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(palette.border))
        .style(
            Style::default()
                .fg(palette.foreground)
                .bg(palette.background),
        )
}

fn draw_picker(frame: &mut Frame<'_>, app: &App) {
    let palette = &app.theme().palette;
    let height = (app.themes.len().saturating_add(2).min(22)) as u16;
    let area = centered(frame.area(), 76, height);
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(Clear, area);
    let items: Vec<_> = app
        .themes
        .iter()
        .map(|theme| {
            ListItem::new(Line::from(vec![
                Span::raw(format!("{}  ", sanitize(&theme.name))),
                Span::styled(
                    " +added ",
                    Style::default()
                        .fg(theme.palette.added_foreground)
                        .bg(theme.palette.added_background),
                ),
                Span::styled(
                    " -removed ",
                    Style::default()
                        .fg(theme.palette.removed_foreground)
                        .bg(theme.palette.removed_background),
                ),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(popup_block(
            " Themes · ↑/↓ preview · Enter accept · Esc restore ",
            palette,
        ))
        .highlight_symbol("› ")
        .highlight_style(
            Style::default()
                .bg(palette.selection)
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default().with_selected(Some(app.theme_index));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_files(frame: &mut Frame<'_>, app: &App) {
    let Some(picker) = &app.file_picker else {
        return;
    };
    let palette = &app.theme().palette;
    let height = picker.entries.len().saturating_add(4).clamp(5, 24) as u16;
    let area = centered(frame.area(), 96, height);
    let title = format!(
        " Files · {}/{} ",
        picker.matches.len(),
        picker.entries.len()
    );
    let block = popup_block(&title, palette);
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let query = format!(" > {}▏", sanitize(&picker.query));
    let offset = query.width().saturating_sub(usize::from(inner.width));
    frame.render_widget(
        Paragraph::new(clip_spans(
            &[Span::raw(query)],
            offset,
            usize::from(inner.width),
        )),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    let list_area = Rect::new(
        inner.x,
        inner.y.saturating_add(1),
        inner.width,
        inner.height.saturating_sub(2),
    );
    if picker.matches.is_empty() {
        let message = if picker.entries.is_empty() {
            " No files in diff."
        } else {
            " No matching files."
        };
        frame.render_widget(
            Paragraph::new(message).style(Style::default().fg(palette.muted)),
            list_area,
        );
    } else {
        let items = picker.matches.iter().map(|index| {
            let entry = &picker.entries[*index];
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("+{} ", entry.additions),
                    Style::default().fg(palette.added_foreground),
                ),
                Span::styled(
                    format!("-{}  ", entry.deletions),
                    Style::default().fg(palette.removed_foreground),
                ),
                Span::raw(entry.path.clone()),
            ]))
        });
        let list = List::new(items).highlight_symbol("› ").highlight_style(
            Style::default()
                .bg(palette.selection)
                .add_modifier(Modifier::BOLD),
        );
        let mut state = ListState::default().with_selected(picker.selected);
        frame.render_stateful_widget(list, list_area, &mut state);
    }
    if inner.height > 1 {
        frame.render_widget(
            Paragraph::new(" ↑/↓ select · Enter open · Esc cancel · Ctrl-U clear")
                .style(Style::default().fg(palette.muted)),
            Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1),
        );
    }
}

fn draw_help(frame: &mut Frame<'_>, app: &App) {
    let area = centered(frame.area(), 80, 30);
    if area.width == 0 || area.height == 0 {
        return;
    }
    let palette = &app.theme().palette;
    let text = [
        "riff — read-only diff viewer",
        "",
        "j/k, ↑/↓             Scroll one row; mouse wheel scrolls too",
        "Space / Shift-Space  Page down / up (PgDn/PgUp also work)",
        "Ctrl-U / Ctrl-D      Scroll half a page",
        "g/G, Home/End        First / last row",
        "h/l, ←/→             Horizontal scroll by four columns",
        "0                    Reset horizontal scroll",
        "n/p                  Next / previous hunk (wraps)",
        "]/[                  Next / previous file (wraps)",
        "Ctrl-P               Files: type fuzzy terms; ↑/↓ select, Enter open",
        "/                    Search text, case-insensitive, live results",
        "Enter / Shift-Enter  Next / previous search result (wraps)",
        "F3 / Shift-F3        Next / previous search result; N also goes back",
        "Esc                  Clear search; cancel active search or picker",
        "t                    Theme picker: ↑/↓ or j/k preview, Enter accept",
        "v                    Toggle split / unified; auto splits at 100 columns",
        "w                    Toggle code word wrap (horizontal scroll off)",
        "i                    Toggle changed-word backgrounds and save preference",
        "ctrl+w (no Shift)    Toggle automatic reload and save watch preference",
        "r / Ctrl-R           Reload original source; stdin cannot be reread",
        "?                    Toggle this help; Esc or q closes it",
        "q / Ctrl-C           Quit (Ctrl-C also quits from dialogs)",
        "",
        "+/- and old/new line numbers convey changes without relying on color.",
        "Theme Enter, v, w, and ctrl+w save only that accepted preference.",
        "Reload results wait for dialogs to close; failures keep the last good diff.",
        "Labels stay clipped. Gutters stay fixed. Diff files/Git are never modified.",
        "Shift-Space needs a terminal that reports Shift (Kitty/CSI-u supported).",
    ]
    .join("\n");
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(text)
            .block(popup_block(" Help · ? / Esc / q close ", palette))
            .wrap(Wrap { trim: false }),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cli::ViewMode, patch::parse_patch, theme::builtin_themes};
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn tiny_and_resized_buffers_do_not_overflow() {
        let diff = parse_patch("--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n-old界\n+new🦀\n").unwrap();
        let mut app = App::new(
            diff,
            builtin_themes(),
            0,
            ViewMode::Auto,
            true,
            "test".into(),
        )
        .unwrap();
        for width in [1, 2, 10, 79, 99, 100, 160] {
            for height in [1, 2, 3, 4, 24] {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                app.resize(width, height);
                terminal.draw(|frame| draw(frame, &app)).unwrap();
                app.handle_event(Event::Key(KeyEvent::new(
                    KeyCode::Char('t'),
                    KeyModifiers::NONE,
                )));
                terminal.draw(|frame| draw(frame, &app)).unwrap();
                app.handle_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
                app.help = true;
                terminal.draw(|frame| draw(frame, &app)).unwrap();
                app.help = false;
            }
        }
    }

    #[test]
    fn change_backgrounds_fill_code_rows() {
        let diff = parse_patch("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n").unwrap();
        let mut app = App::new(
            diff,
            builtin_themes(),
            0,
            ViewMode::Unified,
            false,
            "test".into(),
        )
        .unwrap();
        app.resize(80, 24);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| cell.bg == app.theme().palette.added_background)
        );
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| cell.bg == app.theme().palette.removed_background)
        );
    }
}
