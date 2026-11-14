//! UI-only reload state, requests, and atomic replacement of the last good diff.
use super::*;
use crate::{patch::sanitize, reload::ReloadEvent};

#[path = "reload_anchor.rs"]
mod anchor;

#[derive(Default)]
pub(super) struct ReloadState {
    available: bool,
    unavailable: Option<String>,
    watching: bool,
    desired: bool,
    loading: bool,
    error: Option<String>,
    reload_requested: bool,
    watch_requested: Option<bool>,
    watch_in_flight: Option<bool>,
    persist_watch: bool,
    pending: Option<Diff>,
}

/// Drained by the terminal loop, never executed by a key handler.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReloadRequests {
    pub reload: bool,
    pub watch: Option<bool>,
}

impl App {
    /// Initialize replayability and the one-run watch choice without saving it.
    pub fn configure_reload(&mut self, available: bool, watch: bool) {
        self.reload = ReloadState {
            available,
            desired: available && watch,
            watch_requested: (available && watch).then_some(true),
            ..ReloadState::default()
        };
    }

    pub fn reload_unavailable(&mut self, reason: &str) {
        self.reload.available = false;
        self.reload.error = None;
        self.reload.unavailable = Some(sanitize(reason));
        self.reload.watching = false;
        self.reload.desired = false;
        self.reload.loading = false;
        self.reload.watch_requested = None;
        self.reload.watch_in_flight = None;
        self.reload.persist_watch = false;
    }

    pub fn watch_enabled(&self) -> bool {
        self.reload.watching
    }

    pub fn reload_status(&self) -> String {
        if !self.reload.available {
            return "watch:unavailable".into();
        }
        let mut status = format!("watch:{}", if self.reload.watching { "on" } else { "off" });
        if self.reload.loading || self.reload.watch_in_flight.is_some() {
            status.push_str(" · loading");
        }
        if self.reload.pending.is_some() {
            status.push_str(" · update pending");
        }
        status
    }

    pub fn reload_error(&self) -> Option<&str> {
        self.reload.error.as_deref()
    }

    fn unsupported_reload(&mut self) -> bool {
        if self.reload.available {
            return false;
        }
        self.reload.error = Some(
            self.reload
                .unavailable
                .clone()
                .unwrap_or_else(|| "reload requires a replayable Git or file source".into()),
        );
        true
    }

    pub(super) fn request_reload(&mut self) {
        if !self.unsupported_reload() {
            self.reload.reload_requested = true;
        }
    }

    pub(super) fn toggle_watch(&mut self) {
        if !self.unsupported_reload() {
            self.reload.desired = !self.reload.desired;
            self.reload.watch_requested = Some(self.reload.desired);
            self.reload.persist_watch = true;
        }
    }

    pub fn take_reload_requests(&mut self) -> ReloadRequests {
        let watch = if self.reload.watch_in_flight.is_none() {
            self.reload.watch_requested.take()
        } else {
            None
        };
        if let Some(enabled) = watch {
            self.reload.watch_in_flight = Some(enabled);
        }
        let reload = std::mem::take(&mut self.reload.reload_requested);
        if reload {
            self.reload.loading = true;
        }
        ReloadRequests { reload, watch }
    }

    pub fn handle_reload_event(&mut self, event: ReloadEvent) {
        match event {
            ReloadEvent::Reloading => self.reload.loading = true,
            ReloadEvent::Updated(diff) => {
                self.reload.loading = false;
                self.reload.error = None;
                // Keep only the latest successful update while a modal owns its
                // original search/cancel anchors or theme preview cache.
                self.reload.pending = Some(diff);
            }
            ReloadEvent::Unchanged => {
                self.reload.loading = false;
                self.reload.error = None;
            }
            ReloadEvent::Error(error) => {
                self.reload.loading = false;
                self.reload.error = Some(sanitize(&error));
            }
            ReloadEvent::WatchChanged { enabled, error } => {
                self.reload.watching = enabled;
                self.reload.watch_in_flight = None;
                if self.reload.watch_requested.is_none() {
                    self.reload.desired = enabled;
                    if self.reload.persist_watch && error.is_none() {
                        self.settings_update.watch = Some(enabled);
                    }
                    self.reload.persist_watch = false;
                }
                self.reload.error = error.map(|error| sanitize(&error));
            }
        }
        self.apply_pending_reload();
    }

    pub(super) fn apply_pending_reload(&mut self) {
        if self.help || self.picker_original.is_some() || self.editing_search() {
            return;
        }
        if let Some(diff) = self.reload.pending.take() {
            self.replace_diff(diff);
        }
    }

    fn replace_diff(&mut self, diff: Diff) {
        let previous_position = self.reading_position();
        let previous_match = self.match_position();
        let following_match = previous_position.is_some() && previous_position == previous_match;
        let map = |position| anchor::remap(&self.diff, &diff, position);
        let mut position = previous_position.and_then(map);
        let selected = previous_match.and_then(map);
        let navigation = self.navigation_anchor.and_then(|(kind, row)| {
            self.row_source(row)
                .and_then(map)
                .map(|position| (kind, position))
        });
        let horizontal = self.horizontal;
        self.diff = diff;
        self.additions = self.diff.files.iter().map(|file| file.additions()).sum();
        self.deletions = self.diff.files.iter().map(|file| file.deletions()).sum();
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
        // Do not let the old index-shaped IDs select a newly inserted file.
        self.match_index = None;
        self.hits.clear();
        self.measure_horizontal();
        self.update_matches(false);
        if let Some(selected) = selected {
            self.match_index = self
                .hits
                .iter()
                .enumerate()
                .filter(|(_, hit)| hit.position.source == selected.source)
                .min_by_key(|(_, hit)| hit.position.offset.abs_diff(selected.offset))
                .map(|(index, _)| index)
                .or_else(|| {
                    let row = self.source_row(selected)?;
                    self.hits
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, hit)| hit.row.abs_diff(row))
                        .map(|(index, _)| index)
                });
        }
        if following_match {
            position = self.match_position().or(position);
        }
        self.scroll = position
            .and_then(|position| self.source_row(position))
            .unwrap_or(0);
        self.position_anchor = position;
        self.navigation_anchor = navigation
            .and_then(|(kind, position)| self.source_row(position).map(|row| (kind, row)));
        self.horizontal = horizontal;
        if following_match {
            self.reveal_horizontal();
        }
        self.clamp();
        self.position_scroll = self.scroll;
    }
}
