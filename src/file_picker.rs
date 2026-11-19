//! File menu state. Filtering never rebuilds the diff or moves its reading position.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::App;
use crate::patch::{Diff, sanitize};

#[derive(Debug)]
pub struct FileEntry {
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
    match_path: String,
}

#[derive(Debug)]
pub struct FilePicker {
    pub query: String,
    pub entries: Vec<FileEntry>,
    /// Indices into entries, in diff order.
    pub matches: Vec<usize>,
    /// Index into matches, not entries.
    pub selected: Option<usize>,
}

impl FilePicker {
    pub(super) fn new(diff: &Diff, current_file: usize) -> Self {
        let entries: Vec<_> = diff
            .files
            .iter()
            .map(|file| FileEntry {
                path: sanitize(file.display_path()),
                additions: file.additions(),
                deletions: file.deletions(),
                match_path: file.display_path().to_lowercase(),
            })
            .collect();
        let selected = (!entries.is_empty()).then(|| current_file.min(entries.len() - 1));
        Self {
            query: String::new(),
            matches: (0..entries.len()).collect(),
            entries,
            selected,
        }
    }

    pub fn selected_file(&self) -> Option<usize> {
        self.selected
            .and_then(|selected| self.matches.get(selected).copied())
    }

    fn filter(&mut self) {
        let previous = self.selected_file();
        let query = self.query.to_lowercase();
        let terms: Vec<_> = query.split_whitespace().collect();
        self.matches = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                terms
                    .iter()
                    .all(|term| fuzzy_match(&entry.match_path, term))
            })
            .map(|(index, _)| index)
            .collect();
        self.selected = previous
            .and_then(|file| self.matches.iter().position(|index| *index == file))
            .or_else(|| (!self.matches.is_empty()).then_some(0));
    }

    fn step(&mut self, forward: bool) {
        let count = self.matches.len();
        if count == 0 {
            return;
        }
        self.selected = Some(match self.selected {
            Some(index) if forward => (index + 1) % count,
            Some(index) => (index + count - 1) % count,
            None if forward => 0,
            None => count - 1,
        });
    }
}

fn fuzzy_match(path: &str, term: &str) -> bool {
    let mut remaining = path.chars();
    term.chars()
        .all(|ch| remaining.by_ref().any(|candidate| candidate == ch))
}

impl App {
    pub(super) fn file_picker_key(&mut self, key: KeyEvent) {
        let Some(picker) = &mut self.file_picker else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.file_picker = None,
            KeyCode::Enter => {
                if let Some(file) = picker.selected_file() {
                    let row = self.cache.file_starts[file];
                    self.file_picker = None;
                    self.horizontal = 0;
                    self.jump_to_anchor(true, row);
                }
            }
            KeyCode::Up => picker.step(false),
            KeyCode::Down => picker.step(true),
            KeyCode::Home => picker.selected = (!picker.matches.is_empty()).then_some(0),
            KeyCode::End => picker.selected = picker.matches.len().checked_sub(1),
            KeyCode::Backspace => {
                picker.query.pop();
                picker.filter();
            }
            KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                picker.query.clear();
                picker.filter();
            }
            KeyCode::Char(ch)
                if !ch.is_control() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                picker.query.push(ch);
                picker.filter();
            }
            _ => {}
        }
    }
}
