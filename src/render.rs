//! Cached, sanitized presentation rows shared by the TUI and the printer.
use std::io::{self, Write};

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    cli::ViewMode,
    highlight::{HighlightedLine, highlight_lines},
    intraline,
    patch::{Diff, DiffLine, LineKind, sanitize},
    theme::{Palette, Theme},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelKind {
    File,
    Metadata,
    Hunk,
}

/// Stable patch identity, independent of the number of presentation rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceId {
    Header(usize, usize),
    Hunk(usize, usize),
    Line(usize, usize, usize),
}

#[derive(Clone, Debug)]
pub struct CodeCell {
    pub source: SourceId,
    pub kind: LineKind,
    pub old_number: Option<u32>,
    pub new_number: Option<u32>,
    pub spans: Vec<Span<'static>>,
    /// Byte offset of this visual fragment in its sanitized logical code line.
    pub offset: usize,
}

impl CodeCell {
    pub fn text(&self) -> String {
        self.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    pub fn sign(&self) -> char {
        if self.offset > 0 {
            return ' ';
        }
        match self.kind {
            LineKind::Context => ' ',
            LineKind::Added => '+',
            LineKind::Removed => '-',
            LineKind::NoNewline => '\\',
        }
    }

    pub fn style(&self, palette: &Palette) -> Style {
        let background = match self.kind {
            LineKind::Added => palette.added_background,
            LineKind::Removed => palette.removed_background,
            _ => palette.background,
        };
        Style::default()
            .fg(code_foreground(self.kind, palette))
            .bg(background)
    }

    pub fn gutter_style(&self, palette: &Palette) -> Style {
        let background = match self.kind {
            LineKind::Added => palette.added_gutter,
            LineKind::Removed => palette.removed_gutter,
            _ => palette.gutter,
        };
        Style::default()
            .fg(palette.gutter_foreground)
            .bg(background)
    }
}

#[derive(Clone, Debug)]
pub enum RowContent {
    Label {
        text: String,
        kind: LabelKind,
    },
    Unified(CodeCell),
    Split {
        left: Option<CodeCell>,
        right: Option<CodeCell>,
    },
}

#[derive(Clone, Debug)]
pub struct RenderRow {
    pub source: SourceId,
    pub content: RowContent,
    /// Sanitized logical text, including offscreen columns. Wrapped visual rows
    /// leave this empty and search the originals in RenderCache::search_rows.
    pub search_text: String,
}

impl RenderRow {
    fn label(text: String, kind: LabelKind, source: SourceId) -> Self {
        Self {
            source,
            search_text: text.clone(),
            content: RowContent::Label { text, kind },
        }
    }

    fn unified(cell: CodeCell) -> Self {
        Self {
            source: cell.source,
            search_text: cell.text(),
            content: RowContent::Unified(cell),
        }
    }

    fn split(left: Option<CodeCell>, right: Option<CodeCell>) -> Self {
        let search_text = match (&left, &right) {
            (Some(left), Some(right)) if left.text() == right.text() => left.text(),
            _ => format!(
                "{} │ {}",
                left.as_ref().map(CodeCell::text).unwrap_or_default(),
                right.as_ref().map(CodeCell::text).unwrap_or_default()
            ),
        };
        Self {
            source: right
                .as_ref()
                .or(left.as_ref())
                .expect("nonempty split row")
                .source,
            search_text,
            content: RowContent::Split { left, right },
        }
    }

    pub fn source_for_query(&self, query: &str) -> SourceId {
        if !query.is_empty()
            && let RowContent::Split {
                left: Some(left),
                right,
            } = &self.content
            && left.text().to_lowercase().contains(query)
            && !right
                .as_ref()
                .is_some_and(|cell| cell.text().to_lowercase().contains(query))
        {
            return left.source;
        }
        self.source
    }

    pub fn contains_source(&self, source: SourceId) -> bool {
        self.source == source
            || matches!(&self.content, RowContent::Split { left, right }
                if [left, right].iter().any(|cell| cell.as_ref().is_some_and(|cell| cell.source == source)))
    }
}

#[derive(Clone, Debug, Default)]
pub struct RenderCache {
    pub rows: Vec<RenderRow>,
    /// Original logical rows when wrapped; empty in the backward-compatible cache.
    pub search_rows: Vec<RenderRow>,
    pub file_starts: Vec<usize>,
    pub hunk_starts: Vec<usize>,
    pub digits: usize,
    pub split: bool,
}

pub fn build_cache(
    diff: &Diff,
    theme: &Theme,
    view: ViewMode,
    width: u16,
    syntax: bool,
) -> RenderCache {
    build_cache_with_word_diff(diff, theme, view, width, syntax, true)
}

pub fn build_cache_with_word_diff(
    diff: &Diff,
    theme: &Theme,
    view: ViewMode,
    width: u16,
    syntax: bool,
    word_diff: bool,
) -> RenderCache {
    let maximum = diff
        .files
        .iter()
        .flat_map(|file| &file.hunks)
        .flat_map(|hunk| &hunk.lines)
        .flat_map(|line| [line.old_number, line.new_number])
        .flatten()
        .max()
        .unwrap_or(1);
    let mut cache = RenderCache {
        digits: maximum.to_string().len().max(1),
        split: view.is_split(width),
        ..RenderCache::default()
    };
    for (file_index, file) in diff.files.iter().enumerate() {
        cache.file_starts.push(cache.rows.len());
        cache.rows.push(RenderRow::label(
            format!(
                "── {}  +{} -{}",
                sanitize(file.display_path()),
                file.additions(),
                file.deletions()
            ),
            LabelKind::File,
            SourceId::Header(file_index, 0),
        ));
        cache.rows.push(RenderRow::label(
            format!("--- {}", sanitize(&file.old_path)),
            LabelKind::Metadata,
            SourceId::Header(file_index, 1),
        ));
        cache.rows.push(RenderRow::label(
            format!("+++ {}", sanitize(&file.new_path)),
            LabelKind::Metadata,
            SourceId::Header(file_index, 2),
        ));
        for (record, metadata) in file.metadata.iter().enumerate() {
            cache.rows.push(RenderRow::label(
                sanitize(metadata),
                LabelKind::Metadata,
                SourceId::Header(file_index, record + 3),
            ));
        }
        // Both versions get independent multiline parser state. Missing context between
        // hunks cannot be reconstructed from a patch, so highlighting is best effort.
        let old_text: Vec<String> = file
            .hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| matches!(l.kind, LineKind::Context | LineKind::Removed))
            .map(|l| sanitize(&l.text))
            .collect();
        let new_text: Vec<String> = file
            .hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| matches!(l.kind, LineKind::Context | LineKind::Added))
            .map(|l| sanitize(&l.text))
            .collect();
        let old_highlights = if syntax {
            highlight_lines(&file.old_path, &old_text, theme)
        } else {
            vec![]
        };
        let new_highlights = if syntax {
            highlight_lines(&file.new_path, &new_text, theme)
        } else {
            vec![]
        };
        let (mut old_index, mut new_index) = (0, 0);
        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            cache.hunk_starts.push(cache.rows.len());
            cache.rows.push(RenderRow::label(
                sanitize(&hunk.header),
                LabelKind::Hunk,
                SourceId::Hunk(file_index, hunk_index),
            ));
            let changes = word_diff.then(|| intraline::hunk_ranges(&hunk.lines));
            let mut cells = Vec::with_capacity(hunk.lines.len());
            for (line_index, line) in hunk.lines.iter().enumerate() {
                let source = SourceId::Line(file_index, hunk_index, line_index);
                let highlighted = match line.kind {
                    LineKind::Removed => old_highlights.get(old_index),
                    LineKind::Context | LineKind::Added => new_highlights.get(new_index),
                    LineKind::NoNewline => None,
                };
                let old_context = (cache.split && line.kind == LineKind::Context).then(|| {
                    make_cell(line, old_highlights.get(old_index), &theme.palette, source)
                });
                let mut cell = make_cell(line, highlighted, &theme.palette, source);
                if let Some(changes) = &changes {
                    let background = if line.kind == LineKind::Added {
                        theme.palette.added_word_background
                    } else {
                        theme.palette.removed_word_background
                    };
                    cell.spans = intraline::overlay(cell.spans, &changes[line_index], background);
                }
                cells.push((cell, old_context));
                if matches!(line.kind, LineKind::Context | LineKind::Removed) {
                    old_index += 1;
                }
                if matches!(line.kind, LineKind::Context | LineKind::Added) {
                    new_index += 1;
                }
            }
            if cache.split {
                split_hunk(&mut cache.rows, cells);
            } else {
                cache
                    .rows
                    .extend(cells.into_iter().map(|(cell, _)| RenderRow::unified(cell)));
            }
        }
    }
    cache
}

fn code_foreground(kind: LineKind, palette: &Palette) -> Color {
    match kind {
        LineKind::Added => palette.added_foreground,
        LineKind::Removed => palette.removed_foreground,
        LineKind::NoNewline => palette.muted,
        LineKind::Context => palette.foreground,
    }
}

fn make_cell(
    line: &DiffLine,
    highlighted: Option<&HighlightedLine>,
    palette: &Palette,
    source: SourceId,
) -> CodeCell {
    let foreground = code_foreground(line.kind, palette);
    let spans = if let Some(highlighted) = highlighted {
        highlighted
            .iter()
            .map(|(text, color)| {
                let color = if *color == palette.foreground {
                    foreground
                } else {
                    *color
                };
                // Sanitize defensively at the terminal boundary, not in the patch model.
                Span::styled(sanitize(text), Style::default().fg(color))
            })
            .collect()
    } else {
        let text = if line.kind == LineKind::NoNewline {
            let text = line.text.trim_start_matches('\\').trim_start();
            if text.is_empty() {
                "No newline at end of file"
            } else {
                text
            }
        } else {
            &line.text
        };
        vec![Span::styled(
            sanitize(text),
            Style::default().fg(foreground),
        )]
    };
    CodeCell {
        source,
        kind: line.kind,
        old_number: line.old_number,
        new_number: line.new_number,
        spans,
        offset: 0,
    }
}

fn split_hunk(rows: &mut Vec<RenderRow>, cells: Vec<(CodeCell, Option<CodeCell>)>) {
    let (mut left, mut right) = (Vec::new(), Vec::new());
    let mut previous = LineKind::Context;
    for (cell, old_context) in cells {
        match cell.kind {
            LineKind::Removed => {
                previous = cell.kind;
                left.push(cell);
            }
            LineKind::Added => {
                previous = cell.kind;
                right.push(cell);
            }
            LineKind::NoNewline if previous == LineKind::Removed => left.push(cell),
            LineKind::NoNewline if previous == LineKind::Added => right.push(cell),
            LineKind::NoNewline => {
                flush_changes(rows, &mut left, &mut right);
                rows.push(RenderRow::label(
                    format!("\\ {}", cell.text()),
                    LabelKind::Metadata,
                    cell.source,
                ));
            }
            LineKind::Context => {
                flush_changes(rows, &mut left, &mut right);
                previous = LineKind::Context;
                rows.push(RenderRow::split(old_context, Some(cell)));
            }
        }
    }
    flush_changes(rows, &mut left, &mut right);
}

fn flush_changes(rows: &mut Vec<RenderRow>, left: &mut Vec<CodeCell>, right: &mut Vec<CodeCell>) {
    let mut left = std::mem::take(left).into_iter().peekable();
    let mut right = std::mem::take(right).into_iter().peekable();
    while left.peek().is_some() || right.peek().is_some() {
        rows.push(RenderRow::split(left.next(), right.next()));
        // EOF notes belong to the preceding code row, not the next paired line.
        let left_note = left.next_if(|cell| cell.kind == LineKind::NoNewline);
        let right_note = right.next_if(|cell| cell.kind == LineKind::NoNewline);
        if left_note.is_some() || right_note.is_some() {
            rows.push(RenderRow::split(left_note, right_note));
        }
    }
}

pub fn label_style(kind: LabelKind, palette: &Palette) -> Style {
    match kind {
        LabelKind::File => Style::default().fg(palette.foreground).bg(palette.header),
        LabelKind::Metadata => Style::default().fg(palette.muted).bg(palette.background),
        LabelKind::Hunk => Style::default().fg(palette.foreground).bg(palette.hunk),
    }
}

fn number(number: Option<u32>, digits: usize) -> String {
    match number {
        Some(n) => format!("{n:>digits$}"),
        None => " ".repeat(digits),
    }
}

pub fn unified_gutter(cell: &CodeCell, digits: usize) -> String {
    format!(
        "{} {} {} ",
        number(cell.old_number, digits),
        number(cell.new_number, digits),
        cell.sign()
    )
}

pub fn split_gutter(cell: &CodeCell, digits: usize, left: bool) -> String {
    let n = if left {
        cell.old_number
    } else {
        cell.new_number
    };
    format!("{} {} ", number(n, digits), cell.sign())
}

/// Clip terminal columns, not byte offsets. A wide glyph crossing either edge is
/// replaced by spaces; combining marks are retained only with their visible base.
pub fn clip_spans(spans: &[Span<'static>], offset: usize, width: usize) -> Line<'static> {
    if width == 0 {
        return Line::default();
    }
    let end = offset.saturating_add(width);
    let mut column = 0usize;
    let mut base_visible = false;
    let mut result = Vec::new();
    for span in spans {
        let mut chunk = String::new();
        let mut finished = false;
        for grapheme in span.styled_graphemes(Style::default()) {
            let w = grapheme.symbol.width();
            if w == 0 {
                if base_visible {
                    chunk.push_str(grapheme.symbol);
                }
                continue;
            }
            if column >= end {
                finished = true;
                break;
            }
            let next = column.saturating_add(w);
            if column >= offset && next <= end {
                chunk.push_str(grapheme.symbol);
                base_visible = true;
            } else {
                let visible = next.min(end).saturating_sub(column.max(offset));
                chunk.push_str(&" ".repeat(visible));
                base_visible = false;
            }
            column = next;
        }
        if !chunk.is_empty() {
            result.push(Span::styled(chunk, span.style));
        }
        if finished || column > end {
            break;
        }
    }
    Line::from(result)
}

pub fn print_diff<W: Write>(
    writer: &mut W,
    diff: &Diff,
    theme: &Theme,
    syntax: bool,
    color: bool,
) -> io::Result<()> {
    print_diff_with_word_diff(writer, diff, theme, syntax, color, true)
}

pub fn print_diff_with_word_diff<W: Write>(
    writer: &mut W,
    diff: &Diff,
    theme: &Theme,
    syntax: bool,
    color: bool,
    word_diff: bool,
) -> io::Result<()> {
    let cache = build_cache_with_word_diff(
        diff,
        theme,
        ViewMode::Unified,
        120,
        syntax,
        word_diff && color,
    );
    if cache.rows.is_empty() {
        write_styled(
            writer,
            "No differences.",
            Style::default()
                .fg(theme.palette.muted)
                .bg(theme.palette.background),
            color,
        )?;
        return writeln!(writer);
    }
    for (index, row) in cache.rows.iter().enumerate() {
        match &row.content {
            RowContent::Label { text, kind } => {
                if *kind == LabelKind::File && index != 0 {
                    writeln!(writer)?;
                }
                write_styled(writer, text, label_style(*kind, &theme.palette), color)?;
            }
            RowContent::Unified(cell) => {
                write_styled(
                    writer,
                    &unified_gutter(cell, cache.digits),
                    cell.gutter_style(&theme.palette),
                    color,
                )?;
                for span in &cell.spans {
                    write_styled(
                        writer,
                        &span.content,
                        cell.style(&theme.palette).patch(span.style),
                        color,
                    )?;
                }
            }
            RowContent::Split { .. } => unreachable!("printer always builds unified rows"),
        }
        writeln!(writer)?;
    }
    Ok(())
}

fn write_styled<W: Write>(writer: &mut W, text: &str, style: Style, color: bool) -> io::Result<()> {
    if !color {
        return write!(writer, "{text}");
    }
    if let Some(foreground) = style.fg {
        write_color(writer, foreground, false)?;
    }
    if let Some(background) = style.bg {
        write_color(writer, background, true)?;
    }
    write!(writer, "{text}\x1b[0m")
}

fn write_color<W: Write>(writer: &mut W, color: Color, background: bool) -> io::Result<()> {
    let selector = if background { 48 } else { 38 };
    match color {
        Color::Rgb(r, g, b) => write!(writer, "\x1b[{selector};2;{r};{g};{b}m"),
        Color::Indexed(index) => write!(writer, "\x1b[{selector};5;{index}m"),
        Color::Reset => write!(writer, "\x1b[{}m", if background { 49 } else { 39 }),
        other => {
            let index = match other {
                Color::Black => 0,
                Color::Red => 1,
                Color::Green => 2,
                Color::Yellow => 3,
                Color::Blue => 4,
                Color::Magenta => 5,
                Color::Cyan => 6,
                Color::Gray => 7,
                Color::DarkGray => 8,
                Color::LightRed => 9,
                Color::LightGreen => 10,
                Color::LightYellow => 11,
                Color::LightBlue => 12,
                Color::LightMagenta => 13,
                Color::LightCyan => 14,
                Color::White => 15,
                _ => 7,
            };
            write!(writer, "\x1b[{selector};5;{index}m")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{patch::parse_patch, theme::builtin_themes};
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn clipping_is_unicode_safe_and_column_bounded() {
        let spans = vec![Span::raw("a界e\u{301}🦀z👨‍👩‍👧‍👦")];
        for offset in 0..10 {
            for width in 0..10 {
                let line = clip_spans(&spans, offset, width);
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                assert!(
                    text.width() <= width,
                    "offset={offset}, width={width}: {text:?}"
                );
            }
        }
        let text = clip_spans(&spans, 2, 3).to_string();
        assert_eq!(text, " e\u{301} ");
    }

    #[test]
    fn split_pairs_changes_and_keeps_blank_context() {
        let diff =
            parse_patch("--- a/a.rs\n+++ b/a.rs\n@@ -1,3 +1,4 @@\n-old\n+new\n+extra\n \n end\n")
                .unwrap();
        let theme = builtin_themes().remove(0);
        let cache = build_cache(&diff, &theme, ViewMode::Split, 100, false);
        let rows: Vec<_> = cache
            .rows
            .iter()
            .filter_map(|row| match &row.content {
                RowContent::Split { left, right } => Some((left, right)),
                _ => None,
            })
            .collect();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].0.as_ref().unwrap().text(), "old");
        assert_eq!(rows[0].1.as_ref().unwrap().text(), "new");
        assert!(rows[1].0.is_none());
        assert_eq!(rows[2].0.as_ref().unwrap().text(), "");
        assert_eq!(rows[2].1.as_ref().unwrap().new_number, Some(3));
    }

    #[test]
    fn split_eof_notes_do_not_displace_paired_code() {
        for patch in [
            "--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n-old one\n-old two\n\\ No newline at end of file\n+new one\n+new two\n",
            "--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n-old one\n-old two\n+new one\n+new two\n\\ No newline at end of file\n",
            "--- a/f\n+++ b/f\n@@ -1 +1,2 @@\n-old one\n\\ No newline at end of file\n+new one\n+new two\n",
            "--- a/f\n+++ b/f\n@@ -1,2 +1 @@\n-old one\n-old two\n+new one\n\\ No newline at end of file\n",
        ] {
            let diff = parse_patch(patch).unwrap();
            let theme = builtin_themes().remove(0);
            let cache = build_cache(&diff, &theme, ViewMode::Split, 100, false);
            let rows: Vec<_> = cache
                .rows
                .iter()
                .filter_map(|row| match &row.content {
                    RowContent::Split { left, right } => Some((left, right)),
                    _ => None,
                })
                .collect();
            assert_eq!(rows.len(), 3);
            let mut code_pairs = Vec::new();
            for (left, right) in rows {
                let is_note = |cell: &Option<CodeCell>| {
                    cell.as_ref()
                        .is_some_and(|cell| cell.kind == LineKind::NoNewline)
                };
                if is_note(left) || is_note(right) {
                    assert!(left.is_none() || is_note(left));
                    assert!(right.is_none() || is_note(right));
                } else {
                    code_pairs.push((
                        left.as_ref().and_then(|cell| cell.old_number),
                        right.as_ref().and_then(|cell| cell.new_number),
                    ));
                }
            }
            let old_count = diff.files[0].deletions() as u32;
            let new_count = diff.files[0].additions() as u32;
            let expected: Vec<_> = (1..=old_count.max(new_count))
                .map(|line| {
                    (
                        (line <= old_count).then_some(line),
                        (line <= new_count).then_some(line),
                    )
                })
                .collect();
            assert_eq!(code_pairs, expected);
        }
    }

    #[test]
    fn cached_code_uses_the_selected_semantic_palette() {
        let diff = parse_patch(
            "--- a/example.rs\n+++ b/example.rs\n@@ -1 +1 @@\n-fn old() {}\n+fn new() {}\n",
        )
        .unwrap();
        let mut theme = builtin_themes().remove(0);
        let marker = Color::Rgb(1, 23, 245);
        theme.palette.keyword = marker;
        let cache = build_cache(&diff, &theme, ViewMode::Unified, 80, true);
        let colored = |cache: &RenderCache| {
            cache.rows.iter().any(|row| {
            matches!(&row.content, RowContent::Unified(cell) if cell.spans.iter().any(|span| span.style.fg == Some(marker)))
        })
        };
        assert!(colored(&cache));
        let plain = build_cache(&diff, &theme, ViewMode::Unified, 80, false);
        assert!(!colored(&plain));
    }

    #[test]
    fn printing_plain_does_not_leak_controls() {
        let diff =
            parse_patch("--- a/a\n+++ b/a\n@@ -1 +1 @@\n-a\n+\u{1b}[31m\t界\u{85}\n").unwrap();
        let theme = builtin_themes().remove(0);
        let mut output = Vec::new();
        print_diff(&mut output, &diff, &theme, false, false).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains('界'));
        assert!(
            output
                .chars()
                .all(|c| c == '\n' || (!c.is_control() && c != '\u{85}'))
        );
        let mut colored = Vec::new();
        print_diff(&mut colored, &diff, &theme, false, true).unwrap();
        assert!(colored.contains(&0x1b));
    }
}
