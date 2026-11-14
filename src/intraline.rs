//! Bounded, conservative word edits on sanitized logical lines.
//!
//! Equal-sized replacement runs pair by ordinal, exactly like split rendering.
//! Unequal runs, shifted exact lines, unrelated pairs, and exhausted budgets
//! fall back to existing line tints for the WHOLE run. No text is truncated.
//! EOF notes participate in neither pairing nor tokenization.
use std::ops::Range;

use ratatui::{style::Color, text::Span};
use similar::{Algorithm, DiffTag, capture_diff_slices};
use unicode_segmentation::UnicodeSegmentation;

use crate::patch::{DiffLine, LineKind, sanitize};

pub const MAX_RUN_PAIRS: usize = 32;
pub const MAX_RUN_BYTES: usize = 65_536;
pub const MAX_LINE_BYTES: usize = 4_096;
pub const MAX_LINE_TOKENS: usize = 512;
/// Sum of squared combined token counts; bounds worst-case Myers work per run.
pub const MAX_RUN_WORK: usize = 1_048_576;

pub type ChangedRanges = Vec<Range<usize>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Operator,
    Other,
}

fn class(glyph: &str) -> Class {
    if glyph.chars().all(char::is_whitespace) {
        Class::Space
    } else if glyph
        .chars()
        .next()
        .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
    {
        Class::Word
    } else if glyph.chars().all(|ch| "+-*/%=!<>|&^~?:".contains(ch)) {
        Class::Operator
    } else {
        Class::Other
    }
}

fn tokens(text: &str) -> Option<Vec<&str>> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut previous = Class::Other;
    for (offset, glyph) in text.grapheme_indices(true) {
        let current = class(glyph);
        if offset > 0 && (current != previous || current == Class::Other) {
            result.push(&text[start..offset]);
            if result.len() >= MAX_LINE_TOKENS {
                return None;
            }
            start = offset;
        }
        previous = current;
    }
    if !text.is_empty() {
        result.push(&text[start..]);
    }
    Some(result)
}

fn token_range(tokens: &[&str], range: Range<usize>) -> Option<Range<usize>> {
    if range.is_empty() {
        return None;
    }
    let start = tokens[..range.start].iter().map(|s| s.len()).sum::<usize>();
    Some(start..start + tokens[range].iter().map(|s| s.len()).sum::<usize>())
}

/// Return byte ranges at whole-token/grapheme boundaries, never character diffs.
/// An unchanged word (or non-punctuation Unicode symbol) must account for at
/// least half the smaller line's substantive text. Whitespace-only pairs are
/// also meaningful; punctuation/indentation alone cannot align unrelated code.
pub fn pair(old: &str, new: &str, work: &mut usize) -> Option<(ChangedRanges, ChangedRanges)> {
    if old.len().max(new.len()) > MAX_LINE_BYTES {
        return None;
    }
    let a = tokens(old)?;
    let b = tokens(new)?;
    let cost = (a.len() + b.len()).pow(2);
    *work = work.checked_sub(cost)?;
    let ops = capture_diff_slices(Algorithm::Myers, &a, &b);
    let substantive = |token: &&str| {
        matches!(class(token), Class::Word)
            || (class(token) == Class::Other && token.chars().any(|ch| !ch.is_ascii_punctuation()))
    };
    let weight = |tokens: &[&str]| {
        tokens
            .iter()
            .filter(|token| substantive(token))
            .map(|s| s.chars().count())
            .sum::<usize>()
    };
    let common = ops
        .iter()
        .filter(|op| op.tag() == DiffTag::Equal)
        .map(|op| weight(&a[op.old_range()]))
        .sum::<usize>();
    let minimum = weight(&a).min(weight(&b));
    let spaces_only = old.chars().chain(new.chars()).all(char::is_whitespace);
    if !spaces_only && (common == 0 || common * 2 < minimum) {
        return None;
    }
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    for op in ops.into_iter().filter(|op| op.tag() != DiffTag::Equal) {
        removed.extend(token_range(&a, op.old_range()));
        added.extend(token_range(&b, op.new_range()));
    }
    Some((removed, added))
}

/// Compute once at cache construction, independently of layout and syntax.
/// Returned offsets refer to sanitize(line.text), not raw patch byte offsets.
pub fn hunk_ranges(lines: &[DiffLine]) -> Vec<ChangedRanges> {
    let mut result = vec![Vec::new(); lines.len()];
    let mut start = 0;
    while start < lines.len() {
        if !matches!(lines[start].kind, LineKind::Removed | LineKind::Added) {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < lines.len() && lines[end].kind != LineKind::Context {
            end += 1;
        }
        mark_run(lines, start..end, &mut result);
        start = end;
    }
    result
}

fn mark_run(lines: &[DiffLine], run: Range<usize>, result: &mut [ChangedRanges]) {
    let indices = |kind| {
        run.clone()
            .filter(|&i| lines[i].kind == kind)
            .collect::<Vec<_>>()
    };
    let old = indices(LineKind::Removed);
    let new = indices(LineKind::Added);
    if old.is_empty() || old.len() != new.len() || old.len() > MAX_RUN_PAIRS {
        return;
    }
    let bytes = old
        .iter()
        .chain(&new)
        .map(|&i| lines[i].text.len())
        .sum::<usize>();
    if bytes > MAX_RUN_BYTES
        || old
            .iter()
            .chain(&new)
            .any(|&i| lines[i].text.len() > MAX_LINE_BYTES)
    {
        return;
    }
    let a: Vec<_> = old.iter().map(|&i| sanitize(&lines[i].text)).collect();
    let b: Vec<_> = new.iter().map(|&i| sanitize(&lines[i].text)).collect();
    if a.iter().enumerate().any(|(i, line)| {
        b.iter()
            .enumerate()
            .any(|(j, other)| i != j && line == other)
    }) {
        return;
    }
    let mut work = MAX_RUN_WORK;
    let mut changes = Vec::new();
    for (a, b) in a.iter().zip(&b) {
        let Some(ranges) = pair(a, b, &mut work) else {
            return;
        };
        changes.push(ranges);
    }
    for ((&old, &new), (removed, added)) in old.iter().zip(&new).zip(changes) {
        result[old] = removed;
        result[new] = added;
    }
}

/// Intersect sanitized syntax spans with sorted changed ranges. Only background
/// is patched: foreground, modifiers, source IDs, and all text remain intact.
pub fn overlay(
    spans: Vec<Span<'static>>,
    ranges: &[Range<usize>],
    background: Color,
) -> Vec<Span<'static>> {
    if ranges.is_empty() {
        return spans;
    }
    let mut result = Vec::new();
    let (mut offset, mut index) = (0, 0);
    for span in spans {
        let end = offset + span.content.len();
        let mut cursor = offset;
        while cursor < end {
            while index < ranges.len() && ranges[index].end <= cursor {
                index += 1;
            }
            let range = ranges.get(index);
            let changed = range.is_some_and(|range| range.start <= cursor);
            let next = range.map_or(end, |range| {
                if changed {
                    range.end.min(end)
                } else {
                    range.start.min(end)
                }
            });
            let style = if changed {
                span.style.bg(background)
            } else {
                span.style
            };
            result.push(Span::styled(
                span.content[cursor - offset..next - offset].to_owned(),
                style,
            ));
            cursor = next;
        }
        offset = end;
    }
    result
}
