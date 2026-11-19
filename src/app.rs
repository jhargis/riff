//! Interactive state. Navigation never reparses or re-highlights a patch.
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseEventKind,
};
use unicode_width::UnicodeWidthStr;

use crate::{
    cli::ViewMode,
    config::SettingsUpdate,
    patch::Diff,
    render::{RenderCache, RowContent, build_cache, build_cache_with_word_diff},
    theme::Theme,
    wrap::{self, SearchHit, SourcePosition},
};

#[path = "app_reload.rs"]
mod reload_state;
pub use reload_state::ReloadRequests;

#[path = "file_picker.rs"]
mod files;
pub use files::FilePicker;

#[derive(Clone, Debug)]
struct SearchEdit {
    previous_query: String,
    previous_scroll: usize,
    previous_position: Option<SourcePosition>,
    previous_match: Option<SourcePosition>,
}

pub struct App {
    pub diff: Diff,
    pub themes: Vec<Theme>,
    pub theme_index: usize,
    pub view: ViewMode,
    pub syntax: bool,
    pub wrap: bool,
    pub word_diff: bool,
    pub save_error: Option<String>,
    pub source_label: String,
    pub additions: usize,
    pub deletions: usize,
    pub cache: RenderCache,
    pub scroll: usize,
    pub horizontal: usize,
    pub viewport_height: usize,
    pub dimensions: (u16, u16),
    pub query: String,
    pub search_matches: Vec<usize>,
    pub match_index: Option<usize>,
    pub picker_original: Option<usize>,
    pub file_picker: Option<FilePicker>,
    pub help: bool,
    pub running: bool,
    search_edit: Option<SearchEdit>,
    max_horizontal: usize,
    navigation_anchor: Option<(bool, usize)>,
    position_anchor: Option<SourcePosition>,
    position_scroll: usize,
    hits: Vec<SearchHit>,
    settings_update: SettingsUpdate,
    reload: reload_state::ReloadState,
}

impl App {
    pub fn new(
        diff: Diff,
        themes: Vec<Theme>,
        theme_index: usize,
        view: ViewMode,
        syntax: bool,
        source_label: String,
    ) -> Result<Self, String> {
        if theme_index >= themes.len() {
            return Err("no selected theme is available".into());
        }
        let cache = build_cache(&diff, &themes[theme_index], view, 0, syntax);
        let additions = diff.files.iter().map(|file| file.additions()).sum();
        let deletions = diff.files.iter().map(|file| file.deletions()).sum();
        let mut app = Self {
            diff,
            themes,
            theme_index,
            view,
            syntax,
            wrap: false,
            word_diff: true,
            save_error: None,
            source_label,
            additions,
            deletions,
            cache,
            scroll: 0,
            horizontal: 0,
            viewport_height: 0,
            dimensions: (0, 0),
            query: String::new(),
            search_matches: Vec::new(),
            match_index: None,
            picker_original: None,
            file_picker: None,
            help: false,
            running: true,
            search_edit: None,
            max_horizontal: 0,
            navigation_anchor: None,
            position_anchor: None,
            position_scroll: 0,
            hits: Vec::new(),
            settings_update: SettingsUpdate::default(),
            reload: reload_state::ReloadState::default(),
        };
        app.measure_horizontal();
        Ok(app)
    }

    /// Initialize wrapping without creating a persistence request.
    pub fn set_wrap(&mut self, enabled: bool) {
        if self.wrap != enabled {
            self.wrap = enabled;
            self.rebuild();
        }
    }

    /// Initialize word highlighting without requesting persistence or Git I/O.
    pub fn set_word_diff(&mut self, enabled: bool) {
        if self.word_diff != enabled {
            self.word_diff = enabled;
            self.rebuild();
        }
    }

    /// Drained by the terminal loop; handling events never performs disk I/O.
    pub fn take_settings_update(&mut self) -> SettingsUpdate {
        std::mem::take(&mut self.settings_update)
    }

    fn match_position(&self) -> Option<SourcePosition> {
        self.match_index
            .and_then(|index| self.hits.get(index))
            .map(|hit| hit.position)
    }

    pub fn theme(&self) -> &Theme {
        &self.themes[self.theme_index]
    }

    pub fn editing_search(&self) -> bool {
        self.search_edit.is_some()
    }

    fn modal_open(&self) -> bool {
        self.help
            || self.picker_original.is_some()
            || self.file_picker.is_some()
            || self.editing_search()
    }

    pub fn current_match_row(&self) -> Option<usize> {
        self.match_index
            .and_then(|index| self.search_matches.get(index).copied())
    }

    pub fn current_file(&self) -> usize {
        let row = self.navigation_anchor.map_or(self.scroll, |(_, row)| row);
        self.cache
            .file_starts
            .iter()
            .rposition(|start| *start <= row)
            .unwrap_or(0)
    }

    pub fn resize(&mut self, width: u16, height: u16) {
        let width_changed = self.dimensions.0 != width;
        let height_changed = self.dimensions.1 != height;
        self.dimensions = (width, height);
        self.viewport_height = usize::from(height.saturating_sub(3));
        if width_changed {
            self.rebuild();
        } else if height_changed {
            let position = self.reading_position();
            // A taller viewport may clamp the visible row. Keep the logical
            // reading position so shrinking the height can restore it too.
            if let Some(row) = position.and_then(|source| self.source_row(source)) {
                self.scroll = row;
            }
            self.position_anchor = position;
            self.clamp();
            self.position_scroll = self.scroll;
        }
        self.clamp();
    }

    fn reading_position(&self) -> Option<SourcePosition> {
        self.position_anchor
            .filter(|_| self.position_scroll == self.scroll)
            .or_else(|| {
                self.cache
                    .rows
                    .get(self.scroll)
                    .map(|row| wrap::row_position_at_column(row, self.horizontal))
            })
    }

    fn row_source(&self, row: usize) -> Option<SourcePosition> {
        self.cache.rows.get(row).map(wrap::row_position)
    }

    fn source_row(&self, source: SourcePosition) -> Option<usize> {
        wrap::source_row(&self.cache, source)
    }

    fn rebuild(&mut self) {
        let was_wrapped = !self.cache.search_rows.is_empty();
        let position = self.reading_position();
        let selected = self.match_position();
        let navigation = self
            .navigation_anchor
            .and_then(|(file, row)| self.row_source(row).map(|source| (file, source)));
        self.cache = build_cache_with_word_diff(
            &self.diff,
            self.theme(),
            self.view,
            self.dimensions.0,
            self.syntax,
            self.word_diff,
        );
        if self.wrap {
            wrap::wrap_cache(&mut self.cache, self.dimensions.0);
        }
        self.measure_horizontal();
        self.update_matches(false);
        if let Some(position) = selected {
            self.match_index = self.hits.iter().position(|hit| hit.position == position);
        }
        if let Some(row) = position.and_then(|source| self.source_row(source)) {
            self.scroll = row;
        }
        self.position_anchor = position;
        self.navigation_anchor =
            navigation.and_then(|(file, source)| self.source_row(source).map(|row| (file, row)));
        let previous_row = self
            .search_edit
            .as_ref()
            .and_then(|edit| edit.previous_position)
            .and_then(|source| self.source_row(source));
        if let (Some(edit), Some(row)) = (&mut self.search_edit, previous_row) {
            edit.previous_scroll = row;
        }
        if position.is_some() && position == selected {
            self.reveal_horizontal();
        } else if was_wrapped
            && !self.wrap
            && let Some(position) = position
            && let Some(text) = wrap::source_text(&self.cache, position.source)
        {
            self.horizontal = text[..position.offset.min(text.len())].width();
        }
        self.clamp();
        self.position_scroll = self.scroll;
    }

    fn measure_horizontal(&mut self) {
        if self.wrap {
            self.horizontal = 0;
            self.max_horizontal = 0;
            return;
        }
        let width = usize::from(self.dimensions.0);
        let unified_width = width.saturating_sub(self.cache.digits * 2 + 4);
        let left_width = width.saturating_sub(1) / 2;
        let right_width = width.saturating_sub(left_width + 1);
        let left_code = left_width.saturating_sub(self.cache.digits + 3);
        let right_code = right_width.saturating_sub(self.cache.digits + 3);
        self.max_horizontal = self
            .cache
            .rows
            .iter()
            .map(|row| match &row.content {
                RowContent::Label { text, .. } => text.width().saturating_sub(width),
                RowContent::Unified(cell) => cell.text().width().saturating_sub(unified_width),
                RowContent::Split { left, right } => {
                    let left = left
                        .as_ref()
                        .map_or(0, |cell| cell.text().width().saturating_sub(left_code));
                    let right = right
                        .as_ref()
                        .map_or(0, |cell| cell.text().width().saturating_sub(right_code));
                    left.max(right)
                }
            })
            .max()
            .unwrap_or(0);
    }

    pub fn maximum_scroll(&self) -> usize {
        self.cache
            .rows
            .len()
            .saturating_sub(self.viewport_height.max(1))
    }

    fn clamp(&mut self) {
        self.scroll = self.scroll.min(self.maximum_scroll());
        self.horizontal = self.horizontal.min(self.max_horizontal);
    }

    fn scroll_by(&mut self, delta: isize) {
        self.position_anchor = None;
        self.navigation_anchor = None;
        self.scroll = self.scroll.saturating_add_signed(delta);
        self.clamp();
    }

    fn horizontal_by(&mut self, delta: isize) {
        let previous = self.horizontal;
        self.horizontal = self
            .horizontal
            .saturating_add_signed(delta)
            .min(self.max_horizontal);
        if self.horizontal != previous {
            self.position_anchor = None;
        }
    }

    fn jump_anchor(&mut self, file: bool, forward: bool) {
        let anchors = if file {
            &self.cache.file_starts
        } else {
            &self.cache.hunk_starts
        };
        if anchors.is_empty() {
            return;
        }
        let previous = self
            .navigation_anchor
            .filter(|(kind, _)| *kind == file)
            .and_then(|(_, row)| anchors.iter().position(|anchor| *anchor == row));
        let target = if let Some(index) = previous {
            anchors[if forward {
                (index + 1) % anchors.len()
            } else {
                (index + anchors.len() - 1) % anchors.len()
            }]
        } else if forward {
            anchors
                .iter()
                .copied()
                .find(|row| *row > self.scroll)
                .unwrap_or(anchors[0])
        } else {
            anchors
                .iter()
                .rev()
                .copied()
                .find(|row| *row < self.scroll)
                .unwrap_or(*anchors.last().unwrap())
        };
        self.jump_to_anchor(file, target);
    }

    fn jump_to_anchor(&mut self, file: bool, target: usize) {
        self.scroll = target;
        self.position_anchor = None;
        // An anchor may be below the last full page. Keep its logical position
        // so repeated navigation can wrap instead of getting stuck at the clamp.
        self.navigation_anchor = Some((file, target));
        self.clamp();
    }

    fn update_matches(&mut self, jump: bool) {
        let query = self.query.to_lowercase();
        let old_position = self.match_position();
        self.hits = wrap::search_hits(&self.cache, &query);
        self.search_matches = self.hits.iter().map(|hit| hit.row).collect();
        self.match_index = if jump {
            self.search_matches
                .iter()
                .position(|row| *row >= self.scroll)
                .or_else(|| (!self.search_matches.is_empty()).then_some(0))
        } else {
            old_position
                .and_then(|position| self.hits.iter().position(|hit| hit.position == position))
                .or_else(|| (!self.search_matches.is_empty()).then_some(0))
        };
        if jump {
            self.reveal_match();
        }
    }

    fn reveal_match(&mut self) {
        self.navigation_anchor = None;
        if let Some(row) = self.current_match_row() {
            self.scroll = row;
            self.position_anchor = self.match_position();
            self.reveal_horizontal();
            self.clamp();
            self.position_scroll = self.scroll;
        }
    }

    fn reveal_horizontal(&mut self) {
        if self.wrap {
            self.horizontal = 0;
            return;
        }
        if let Some(position) = self.match_position()
            && let Some(text) = wrap::source_text(&self.cache, position.source)
        {
            let column = text[..position.offset.min(text.len())].width();
            let width = usize::from(self.dimensions.0);
            let code_width = if self.cache.split {
                width.saturating_sub(1) / 2
            } else {
                width
            }
            .saturating_sub(if self.cache.split {
                self.cache.digits + 3
            } else {
                self.cache.digits * 2 + 4
            });
            let end = column.saturating_add(self.query.width().min(code_width));
            if column < self.horizontal || end > self.horizontal.saturating_add(code_width) {
                self.horizontal = column.min(self.max_horizontal);
            }
        }
    }

    fn step_match(&mut self, forward: bool) {
        let count = self.search_matches.len();
        if count == 0 {
            return;
        }
        self.match_index = Some(match self.match_index {
            Some(index) if forward => (index + 1) % count,
            Some(index) => (index + count - 1) % count,
            None if forward => 0,
            None => count - 1,
        });
        self.reveal_match();
    }

    fn begin_search(&mut self) {
        self.search_edit = Some(SearchEdit {
            previous_query: self.query.clone(),
            previous_scroll: self.scroll,
            previous_position: self.reading_position(),
            previous_match: self.match_position(),
        });
        self.query.clear();
        self.update_matches(false);
    }

    fn search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                if let Some(previous) = self.search_edit.take() {
                    self.query = previous.previous_query;
                    self.scroll = previous.previous_scroll;
                    self.position_anchor = previous.previous_position;
                    self.update_matches(false);
                    self.match_index = previous.previous_match.and_then(|position| {
                        self.hits.iter().position(|hit| hit.position == position)
                    });
                    self.reveal_horizontal();
                    self.clamp();
                    self.position_scroll = self.scroll;
                }
            }
            KeyCode::Enter => {
                self.search_edit = None;
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.update_matches(true);
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.clear();
                self.update_matches(true);
            }
            KeyCode::Char(ch)
                if !ch.is_control()
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.query.push(ch);
                self.update_matches(true);
            }
            _ => {}
        }
    }

    fn preview_theme(&mut self, index: usize) {
        if index < self.themes.len() && index != self.theme_index {
            self.theme_index = index;
            self.rebuild();
        }
    }

    fn theme_key(&mut self, key: KeyEvent) {
        let count = self.themes.len();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                if let Some(original) = self.picker_original.take() {
                    self.preview_theme(original);
                }
            }
            KeyCode::Enter => {
                self.picker_original = None;
                self.settings_update.theme = Some(self.theme().name.clone());
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.preview_theme((self.theme_index + count - 1) % count)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.preview_theme((self.theme_index + 1) % count)
            }
            KeyCode::PageDown => self.preview_theme((self.theme_index + 8).min(count - 1)),
            KeyCode::PageUp => self.preview_theme(self.theme_index.saturating_sub(8)),
            KeyCode::Home | KeyCode::Char('g') => self.preview_theme(0),
            KeyCode::End | KeyCode::Char('G') => self.preview_theme(count - 1),
            _ => {}
        }
    }

    pub fn handle_event(&mut self, event: Event) {
        match event {
            Event::Resize(width, height) => self.resize(width, height),
            Event::Mouse(mouse) if !self.modal_open() => match mouse.kind {
                MouseEventKind::ScrollDown => self.scroll_by(3),
                MouseEventKind::ScrollUp => self.scroll_by(-3),
                MouseEventKind::ScrollLeft => self.horizontal_by(-4),
                MouseEventKind::ScrollRight => self.horizontal_by(4),
                _ => {}
            },
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
            _ => {}
        }
        self.apply_pending_reload();
    }

    fn key(&mut self, mut key: KeyEvent) {
        // None of our bindings use these modifiers, including inside modals.
        // In particular Alt-W must not become a persistent bare-W preference.
        if key.modifiers.intersects(
            KeyModifiers::ALT | KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META,
        ) {
            return;
        }
        // Kitty all-key encoding reports the base letter plus lock state.
        // Crossterm applies the shifted alternate, but not Caps Lock. Toggle
        // ASCII case once here for commands and search, leaving Ctrl keys alone.
        if key.state.contains(KeyEventState::CAPS_LOCK)
            && !key.modifiers.contains(KeyModifiers::CONTROL)
            && let KeyCode::Char(ch) = &mut key.code
        {
            *ch = if ch.is_ascii_lowercase() {
                ch.to_ascii_uppercase()
            } else {
                ch.to_ascii_lowercase()
            };
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.running = false;
            return;
        }
        if self.help {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
            ) {
                self.help = false;
            }
            return;
        }
        if self.picker_original.is_some() {
            self.theme_key(key);
            return;
        }
        if self.file_picker.is_some() {
            self.file_picker_key(key);
            return;
        }
        if self.editing_search() {
            self.search_key(key);
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            let half_page = (self.viewport_height / 2).max(1) as isize;
            match key.code {
                KeyCode::Char('d') => self.scroll_by(half_page),
                KeyCode::Char('u') => self.scroll_by(-half_page),
                KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                    self.file_picker = Some(FilePicker::new(&self.diff, self.current_file()));
                }
                KeyCode::Char('w') if key.modifiers == KeyModifiers::CONTROL => self.toggle_watch(),
                KeyCode::Char('r') if key.modifiers == KeyModifiers::CONTROL => {
                    self.request_reload()
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.running = false,
            KeyCode::Down | KeyCode::Char('j') => self.scroll_by(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll_by(-1),
            KeyCode::Char(' ') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.scroll_by(-(self.viewport_height.max(1) as isize))
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.scroll_by(self.viewport_height.max(1) as isize)
            }
            KeyCode::PageUp => self.scroll_by(-(self.viewport_height.max(1) as isize)),
            KeyCode::Home | KeyCode::Char('g') => {
                self.navigation_anchor = None;
                self.position_anchor = None;
                self.scroll = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.navigation_anchor = None;
                self.position_anchor = None;
                self.scroll = self.maximum_scroll();
            }
            KeyCode::Left | KeyCode::Char('h') => self.horizontal_by(-4),
            KeyCode::Right | KeyCode::Char('l') => self.horizontal_by(4),
            KeyCode::Char('0') => {
                self.horizontal = 0;
                self.position_anchor = None;
            }
            KeyCode::Char('n') => self.jump_anchor(false, true),
            KeyCode::Char('p') => self.jump_anchor(false, false),
            KeyCode::Char(']') => self.jump_anchor(true, true),
            KeyCode::Char('[') => self.jump_anchor(true, false),
            KeyCode::Char('/') => self.begin_search(),
            KeyCode::Enter | KeyCode::F(3) => {
                self.step_match(!key.modifiers.contains(KeyModifiers::SHIFT))
            }
            KeyCode::Char('N') => self.step_match(false),
            KeyCode::Esc => {
                self.query.clear();
                self.update_matches(false);
            }
            KeyCode::Char('t') => self.picker_original = Some(self.theme_index),
            KeyCode::Char('v') => {
                self.view = if self.cache.split {
                    ViewMode::Unified
                } else {
                    ViewMode::Split
                };
                self.rebuild();
                self.settings_update.view = Some(self.view.name().into());
            }
            KeyCode::Char('r') if key.modifiers.is_empty() => self.request_reload(),
            KeyCode::Char('w') => {
                self.set_wrap(!self.wrap);
                self.settings_update.wrap = Some(self.wrap);
            }
            KeyCode::Char('i') if key.modifiers.is_empty() => {
                self.set_word_diff(!self.word_diff);
                self.settings_update.word_diff = Some(self.word_diff);
            }
            KeyCode::Char('?') => self.help = true,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{patch::parse_patch, theme::builtin_themes};

    fn app() -> App {
        let diff =
            parse_patch("--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n-old\n+needle\n keep\n end\n").unwrap();
        App::new(
            diff,
            builtin_themes(),
            0,
            ViewMode::Auto,
            false,
            "test".into(),
        )
        .unwrap()
    }

    fn key(app: &mut App, code: KeyCode) {
        app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    #[test]
    fn scrolling_does_not_rebuild_cache() {
        let mut app = app();
        app.resize(80, 5);
        let allocation = app.cache.rows.as_ptr();
        key(&mut app, KeyCode::Char('j'));
        assert_eq!(app.cache.rows.as_ptr(), allocation);
        assert_eq!(app.scroll, 1);
    }

    #[test]
    fn resize_and_view_changes_clamp_scroll() {
        let mut app = app();
        app.resize(80, 4);
        key(&mut app, KeyCode::End);
        app.resize(120, 100);
        assert!(app.cache.split);
        assert_eq!(app.scroll, 0);
        key(&mut app, KeyCode::Char('v'));
        assert!(!app.cache.split);
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn picker_cancel_restores_original_and_accept_keeps_preview() {
        let mut app = app();
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Down);
        assert_eq!(app.theme_index, 1);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.theme_index, 0);
        key(&mut app, KeyCode::Char('t'));
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.theme_index, 1);
        assert!(app.picker_original.is_none());
    }

    #[test]
    fn search_updates_and_escape_restores() {
        let mut app = app();
        app.resize(80, 5);
        key(&mut app, KeyCode::Char('/'));
        for ch in "needle".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        assert_eq!(app.search_matches.len(), 1);
        assert!(app.current_match_row().is_some());
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('/'));
        key(&mut app, KeyCode::Char('z'));
        assert!(app.search_matches.is_empty());
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.query, "needle");
        assert_eq!(app.search_matches.len(), 1);
    }

    #[test]
    fn hunk_navigation_wraps_even_when_everything_fits_one_page() {
        let diff =
            parse_patch("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n@@ -5 +5 @@\n-c\n+d\n").unwrap();
        let mut app = App::new(
            diff,
            builtin_themes(),
            0,
            ViewMode::Unified,
            false,
            "test".into(),
        )
        .unwrap();
        app.resize(120, 100);
        let first = app.cache.hunk_starts[0];
        let second = app.cache.hunk_starts[1];
        key(&mut app, KeyCode::Char('n'));
        assert_eq!(app.navigation_anchor, Some((false, first)));
        key(&mut app, KeyCode::Char('n'));
        assert_eq!(app.navigation_anchor, Some((false, second)));
        key(&mut app, KeyCode::Char('n'));
        assert_eq!(app.navigation_anchor, Some((false, first)));
        key(&mut app, KeyCode::Char('p'));
        assert_eq!(app.navigation_anchor, Some((false, second)));
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn horizontal_scroll_clamps_after_resize() {
        let patch = format!(
            "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+{}\n",
            "界".repeat(100)
        );
        let diff = parse_patch(&patch).unwrap();
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
        for _ in 0..100 {
            key(&mut app, KeyCode::Right);
        }
        assert!(app.horizontal > 0);
        app.resize(300, 24);
        assert_eq!(app.horizontal, 0);
    }

    #[test]
    fn empty_diff_and_zero_size_are_safe() {
        let mut app = App::new(
            Diff::default(),
            builtin_themes(),
            0,
            ViewMode::Split,
            true,
            "empty".into(),
        )
        .unwrap();
        app.resize(0, 0);
        for code in [
            KeyCode::End,
            KeyCode::Char('n'),
            KeyCode::Char('p'),
            KeyCode::Char(']'),
            KeyCode::Right,
        ] {
            key(&mut app, code);
        }
        assert_eq!(app.scroll, 0);
        assert_eq!(app.horizontal, 0);
    }
}
