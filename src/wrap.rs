//! Cached word wrapping and source-offset mapping for interactive presentation.
//!
//! Ratatui's Paragraph wrapper is private and drops boundary whitespace (and
//! overwide glyphs), even with trim=false. This small adapter instead partitions
//! native styled graphemes without dropping any sanitized code or syntax style.
use std::collections::HashMap;

use ratatui::{style::Style, text::Span};
use unicode_width::UnicodeWidthStr;

use crate::render::{CodeCell, RenderCache, RenderRow, RowContent, SourceId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourcePosition {
    pub source: SourceId,
    /// UTF-8 offset in the sanitized logical code line, not a fragment index.
    pub offset: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct SearchHit {
    pub position: SourcePosition,
    pub row: usize,
}

fn fragments(cell: &CodeCell, width: usize) -> Vec<CodeCell> {
    if width == 0 {
        return vec![cell.clone()];
    }
    let graphemes: Vec<_> = cell
        .spans
        .iter()
        .flat_map(|span| span.styled_graphemes(Style::default()))
        .collect();
    let mut result = Vec::new();
    let mut start = 0;
    let mut offset = 0;
    while start < graphemes.len() {
        let (mut end, mut columns, mut boundary) = (start, 0usize, start);
        while end < graphemes.len() {
            let glyph = &graphemes[end];
            if end > start
                && graphemes[end - 1].symbol.chars().all(char::is_whitespace)
                && !glyph.symbol.chars().all(char::is_whitespace)
            {
                boundary = end;
            }
            let next = columns.saturating_add(glyph.symbol.width());
            if next > width && end > start {
                break;
            }
            columns = next;
            end += 1;
        }
        if end < graphemes.len() && boundary > start {
            end = boundary;
        }
        let mut fragment = CodeCell {
            source: cell.source,
            kind: cell.kind,
            old_number: if offset == 0 { cell.old_number } else { None },
            new_number: if offset == 0 { cell.new_number } else { None },
            spans: Vec::new(),
            offset,
        };
        for glyph in &graphemes[start..end] {
            offset += glyph.symbol.len();
            if let Some(last) = fragment.spans.last_mut()
                && last.style == glyph.style
            {
                last.content.to_mut().push_str(glyph.symbol);
            } else {
                fragment
                    .spans
                    .push(Span::styled(glyph.symbol.to_owned(), glyph.style));
            }
        }
        result.push(fragment);
        start = end;
    }
    if result.is_empty() {
        result.push(cell.clone());
    }
    result
}

/// Turn logical rows into visual rows once per width/layout/theme/wrap rebuild.
/// Labels deliberately stay single-row and clipped; gutters are never wrapped.
pub fn wrap_cache(cache: &mut RenderCache, width: u16) {
    let width = usize::from(width);
    let unified = width.saturating_sub(cache.digits * 2 + 4);
    let left = width.saturating_sub(1) / 2;
    let right = width.saturating_sub(left + 1);
    let gutter = cache.digits + 3;
    let logical = std::mem::take(&mut cache.rows);
    let mut remap = Vec::with_capacity(logical.len());
    for row in &logical {
        remap.push(cache.rows.len());
        match &row.content {
            RowContent::Label { .. } => cache.rows.push(row.clone()),
            RowContent::Unified(cell) => {
                for fragment in fragments(cell, unified) {
                    cache.rows.push(RenderRow {
                        source: fragment.source,
                        content: RowContent::Unified(fragment),
                        // Search uses logical rows, not this fragment's text.
                        search_text: String::new(),
                    });
                }
            }
            RowContent::Split { left: a, right: b } => {
                let a = a
                    .as_ref()
                    .map(|cell| fragments(cell, left.saturating_sub(gutter)))
                    .unwrap_or_default();
                let b = b
                    .as_ref()
                    .map(|cell| fragments(cell, right.saturating_sub(gutter)))
                    .unwrap_or_default();
                for index in 0..a.len().max(b.len()) {
                    let left = a.get(index).cloned();
                    let right = b.get(index).cloned();
                    cache.rows.push(RenderRow {
                        source: right.as_ref().or(left.as_ref()).unwrap().source,
                        content: RowContent::Split { left, right },
                        search_text: String::new(),
                    });
                }
            }
        }
    }
    for anchor in cache.file_starts.iter_mut().chain(&mut cache.hunk_starts) {
        *anchor = remap[*anchor];
    }
    cache.search_rows = logical;
}

pub fn row_position(row: &RenderRow) -> SourcePosition {
    let cell = match &row.content {
        RowContent::Unified(cell) => Some(cell),
        RowContent::Split { left, right } => right.as_ref().or(left.as_ref()),
        RowContent::Label { .. } => None,
    };
    SourcePosition {
        source: row.source,
        offset: cell.map_or(0, |cell| cell.offset),
    }
}

/// Include the clipped reading column when moving into wrapped presentation.
/// Columns are display cells; the resulting offset stays on a UTF-8 grapheme
/// boundary, including when the viewport clips through a wide glyph.
pub fn row_position_at_column(row: &RenderRow, column: usize) -> SourcePosition {
    let mut position = row_position(row);
    if column == 0 {
        return position;
    }
    if let Some(cell) = cells(row).next() {
        let mut columns = 0;
        for glyph in cell
            .spans
            .iter()
            .flat_map(|span| span.styled_graphemes(Style::default()))
        {
            let next = columns + glyph.symbol.width();
            if next > column {
                break;
            }
            columns = next;
            position.offset += glyph.symbol.len();
        }
    }
    position
}

fn cells(row: &RenderRow) -> impl Iterator<Item = &CodeCell> {
    let (a, b) = match &row.content {
        RowContent::Unified(cell) => (Some(cell), None),
        RowContent::Split { left, right } => (right.as_ref(), left.as_ref()),
        RowContent::Label { .. } => (None, None),
    };
    a.into_iter().chain(b)
}

fn cell_end(cell: &CodeCell) -> usize {
    cell.offset
        + cell
            .spans
            .iter()
            .map(|span| span.content.len())
            .sum::<usize>()
}

pub fn source_row(cache: &RenderCache, position: SourcePosition) -> Option<usize> {
    let mut fallback = None;
    for (index, row) in cache.rows.iter().enumerate() {
        if matches!(&row.content, RowContent::Label { .. }) && row.source == position.source {
            return Some(index);
        }
        for cell in cells(row).filter(|cell| cell.source == position.source) {
            if cell.offset <= position.offset {
                fallback = Some(index);
                if position.offset < cell_end(cell) {
                    return Some(index);
                }
            }
        }
    }
    fallback
}

/// Lowercase matching mapped back to original UTF-8 offsets, including characters
/// whose lowercase representation changes byte length or expands to two scalars.
fn match_offset(text: &str, query: &str) -> Option<usize> {
    let lowered = text.to_lowercase();
    let found = lowered.find(query)?;
    let mut offset = 0;
    for (original, ch) in text.char_indices() {
        let length: usize = ch.to_lowercase().map(char::len_utf8).sum();
        if offset + length > found {
            return Some(original);
        }
        offset += length;
    }
    Some(text.len())
}

pub fn search_hits(cache: &RenderCache, query: &str) -> Vec<SearchHit> {
    if query.is_empty() {
        return Vec::new();
    }
    let logical = if cache.search_rows.is_empty() {
        &cache.rows
    } else {
        &cache.search_rows
    };
    // Index fragments once, rather than rescanning the entire visual cache per
    // match. Search runs on whole logical lines, including boundary-spanning text.
    let mut locations: HashMap<SourceId, Vec<(usize, usize, usize)>> = HashMap::new();
    for (index, row) in cache.rows.iter().enumerate() {
        if matches!(row.content, RowContent::Label { .. }) {
            locations
                .entry(row.source)
                .or_default()
                .push((0, usize::MAX, index));
        } else {
            for cell in cells(row) {
                let entries = locations.entry(cell.source).or_default();
                let entry = (cell.offset, cell_end(cell), index);
                if entries.last() != Some(&entry) {
                    entries.push(entry);
                }
            }
        }
    }
    let mut hits = Vec::new();
    for row in logical {
        let candidates: Vec<_> = if let RowContent::Label { text, .. } = &row.content {
            match_offset(text, query)
                .map(|offset| SourcePosition {
                    source: row.source,
                    offset,
                })
                .into_iter()
                .collect()
        } else {
            let mut candidates = Vec::new();
            for cell in cells(row) {
                if let Some(offset) = match_offset(&cell.text(), query) {
                    let position = SourcePosition {
                        source: cell.source,
                        offset,
                    };
                    // Old/new context copies share one source and count once.
                    if !candidates.contains(&position) {
                        candidates.push(position);
                    }
                }
            }
            candidates
        };
        for position in candidates {
            if let Some(entries) = locations.get(&position.source) {
                // Unequal split pane widths give context copies overlapping,
                // non-monotonic offsets. Select the first containing visual row,
                // exactly like source_row, rather than binary-searching offsets.
                if let Some((_, _, row)) = entries
                    .iter()
                    .find(|(start, end, _)| *start <= position.offset && position.offset < *end)
                {
                    hits.push(SearchHit {
                        position,
                        row: *row,
                    });
                }
            }
        }
    }
    // Split counterparts may start on different visual fragments.
    hits.sort_by_key(|hit| hit.row);
    hits
}

pub fn source_text(cache: &RenderCache, source: SourceId) -> Option<String> {
    let rows = if cache.search_rows.is_empty() {
        &cache.rows
    } else {
        &cache.search_rows
    };
    rows.iter().find_map(|row| {
        if let RowContent::Label { text, .. } = &row.content
            && row.source == source
        {
            return Some(text.clone());
        }
        cells(row)
            .find(|cell| cell.source == source)
            .map(CodeCell::text)
    })
}
