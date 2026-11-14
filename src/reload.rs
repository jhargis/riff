//! Serial, asynchronous refresh of an already-resolved input query.
//!
//! An idle controller does no Git work. `notify` supplies filesystem events;
//! only event bursts are timed, never periodic Git polling. Dropping a controller
//! wakes the worker and releases its watcher without joining the worker. Git's
//! existing blocking loader cannot cancel a running Git process (including a
//! stuck configured clean filter); that one worker exits when the load returns.

use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, Weak, mpsc},
    thread,
    time::{Duration, Instant},
};

use notify::{
    Event, EventKind,
    event::{MetadataKind, ModifyKind},
};

use crate::{
    input::{self, Input},
    patch::{Diff, parse_patch},
};

mod watch;

/// Quiet time after a filesystem event burst, not a polling interval.
const WATCH_DEBOUNCE: Duration = Duration::from_millis(150);
const DISCONNECTED: &str = "Reload worker disconnected.";

#[derive(Debug)]
pub enum ReloadEvent {
    Reloading,
    Updated(Diff),
    Unchanged,
    Error(String),
    WatchChanged {
        enabled: bool,
        error: Option<String>,
    },
}

/// Owns one serial worker and, when enabled, one recommended OS watcher.
pub struct ReloadController {
    shared: Arc<Shared>,
    events: mpsc::Receiver<ReloadEvent>,
}

#[derive(Default)]
struct Pending {
    stopped: bool,
    reload: bool,
    watch_request: Option<bool>,
    changed: Option<Instant>,
    watch_error: Option<String>,
    reconfigure: Option<bool>,
    epoch: u64,
    watcher: Option<watch::Subscriptions>,
}

#[derive(Default)]
struct Shared {
    pending: Mutex<Pending>,
    wake: Condvar,
}

impl ReloadController {
    /// Capture the cwd and the resolved source exactly once. No Git command or
    /// stdin read occurs here; the supplied initial patch is already parsed by
    /// the caller and is the baseline for unchanged detection.
    pub fn new(source: Input, paths: Vec<PathBuf>, initial_patch: String) -> Result<Self, String> {
        if matches!(source, Input::Stdin) {
            return Err("Stdin patches cannot be reloaded or watched.".into());
        }
        input::validate(&source, &paths)?;
        let cwd =
            std::env::current_dir().map_err(|e| format!("Cannot capture reload directory: {e}"))?;
        let shared = Arc::new(Shared::default());
        let (sender, events) = mpsc::channel();
        let worker = Worker {
            shared: Arc::clone(&shared),
            events: sender,
            source,
            paths,
            cwd,
            last_patch: initial_patch,
        };
        thread::Builder::new()
            .name("riff-reload".into())
            .spawn(move || worker.run())
            .map_err(|e| format!("Cannot start reload worker: {e}"))?;
        Ok(Self { shared, events })
    }

    /// Nonblocking and bounded: many pending requests become one reload. A new
    /// request during a load schedules one more load, so the final edit is read.
    pub fn request_reload(&self) -> Result<(), String> {
        self.update(|pending| pending.reload = true)
    }

    /// The acknowledgement describes the actual watch mode, not merely the
    /// requested mode. Repeated pending toggles coalesce to the latest choice.
    pub fn set_watch(&self, enabled: bool) -> Result<(), String> {
        self.update(|pending| pending.watch_request = Some(enabled))
    }

    pub fn try_recv(&self) -> Result<Option<ReloadEvent>, String> {
        match self.events.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => Err(DISCONNECTED.into()),
        }
    }

    fn update(&self, change: impl FnOnce(&mut Pending)) -> Result<(), String> {
        let mut pending = self
            .shared
            .pending
            .lock()
            .map_err(|_| DISCONNECTED.to_string())?;
        if pending.stopped {
            return Err(DISCONNECTED.into());
        }
        change(&mut pending);
        self.shared.wake.notify_one();
        Ok(())
    }
}

impl Drop for ReloadController {
    fn drop(&mut self) {
        self.shared.stop();
    }
}

impl Shared {
    fn stop(&self) {
        let watcher = {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            pending.stopped = true;
            self.wake.notify_one();
            pending.watcher.take()
        };
        // Never drop a watcher while holding the callback's lock.
        drop(watcher);
    }

    fn event(&self, epoch: u64, event: notify::Result<Event>) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if pending.stopped || pending.epoch != epoch {
            return;
        }
        match event {
            // Git reads its inputs: access events must not cause feedback.
            Ok(event)
                if matches!(
                    event.kind,
                    EventKind::Access(_)
                        | EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime))
                ) && !event.need_rescan() =>
            {
                return;
            }
            Ok(event) => {
                let refresh = pending
                    .watcher
                    .as_ref()
                    .map_or(Some(true), |watcher| watcher.refresh_for(&event));
                if let Some(rebind) = refresh {
                    pending.reconfigure = Some(pending.reconfigure.unwrap_or(false) || rebind);
                }
                pending.changed = Some(Instant::now());
            }
            Err(error) => {
                pending.watch_error = Some(format!(
                    "Filesystem watch failed: {error}; manual reload remains available."
                ));
                // Even if the OS lost events, attempt to read the latest state.
                pending.changed = Some(Instant::now());
            }
        }
        self.wake.notify_one();
    }
}

enum Work {
    Watch(bool),
    WatchError(String),
    Reload { reconfigure: Option<bool> },
}

struct Worker {
    shared: Arc<Shared>,
    events: mpsc::Sender<ReloadEvent>,
    source: Input,
    paths: Vec<PathBuf>,
    cwd: PathBuf,
    last_patch: String,
}

impl Worker {
    fn run(mut self) {
        while let Some(work) = self.next() {
            let event = match work {
                Work::Watch(enabled) => self.watch(enabled),
                Work::WatchError(error) => self.watch_failed(error),
                Work::Reload { reconfigure } => {
                    if self.events.send(ReloadEvent::Reloading).is_err() {
                        break;
                    }
                    self.reload(reconfigure)
                }
            };
            if self.events.send(event).is_err() {
                break;
            }
        }
    }

    fn next(&self) -> Option<Work> {
        let mut pending = self
            .shared
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        loop {
            if pending.stopped {
                return None;
            }
            if let Some(enabled) = pending.watch_request.take() {
                return Some(Work::Watch(enabled));
            }
            if let Some(error) = pending.watch_error.take() {
                return Some(Work::WatchError(error));
            }
            if std::mem::take(&mut pending.reload) {
                pending.changed = None;
                return Some(Work::Reload {
                    reconfigure: std::mem::take(&mut pending.reconfigure),
                });
            }
            if let Some(changed) = pending.changed {
                let remaining = WATCH_DEBOUNCE.saturating_sub(changed.elapsed());
                if remaining.is_zero() {
                    pending.changed = None;
                    return Some(Work::Reload {
                        reconfigure: std::mem::take(&mut pending.reconfigure),
                    });
                }
                pending = self
                    .shared
                    .wake
                    .wait_timeout(pending, remaining)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            } else {
                pending = self
                    .shared
                    .wake
                    .wait(pending)
                    .unwrap_or_else(|e| e.into_inner());
            }
        }
    }

    fn reload(&mut self, reconfigure: Option<bool>) -> ReloadEvent {
        let patch = match input::load_at(&self.source, &self.paths, &self.cwd) {
            Ok(patch) => patch,
            Err(error) => {
                return match self.refresh_watch(reconfigure) {
                    Ok(()) => ReloadEvent::Error(error),
                    Err(error) => self.watch_failed(error),
                };
            }
        };
        let diff = if patch == self.last_patch {
            Ok(None)
        } else {
            parse_patch(&patch).map(Some)
        };
        if let Err(error) = self.refresh_watch(reconfigure) {
            return self.watch_failed(error);
        }
        match diff {
            Ok(Some(diff)) => {
                self.last_patch = patch;
                ReloadEvent::Updated(diff)
            }
            Ok(None) => ReloadEvent::Unchanged,
            Err(error) => ReloadEvent::Error(format!("Cannot parse reloaded patch: {error}")),
        }
    }

    fn watch_failed(&self, error: String) -> ReloadEvent {
        self.watch(false);
        ReloadEvent::WatchChanged {
            enabled: false,
            error: Some(error),
        }
    }

    fn refresh_watch(&self, reconfigure: Option<bool>) -> Result<(), String> {
        let Some(rebind) = reconfigure else {
            return Ok(());
        };
        if self
            .shared
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .watcher
            .is_none()
        {
            return Ok(());
        }
        // Discovery can run Git; leave the watcher in Shared so Drop can still
        // release it immediately while a Git process is blocked.
        let plan = watch::plan(&self.source, &self.cwd, false)?;
        let watcher = self
            .shared
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .watcher
            .take();
        if let Some(mut watcher) = watcher {
            // Never call native watcher methods under the callback's lock.
            let changed = watcher.refresh(plan)?;
            let mut pending = self
                .shared
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if pending.stopped {
                drop(pending);
                drop(watcher);
            } else {
                pending.watcher = Some(watcher);
                // Read again after installing newly discovered subscriptions.
                // Events during the load/reconfiguration remain queued as well.
                pending.reload |= changed || rebind;
            }
        }
        Ok(())
    }

    fn watch(&self, enabled: bool) -> ReloadEvent {
        let epoch = {
            let mut pending = self
                .shared
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if enabled && pending.watcher.is_some() {
                return ReloadEvent::WatchChanged {
                    enabled: true,
                    error: None,
                };
            }
            pending.epoch = pending.epoch.wrapping_add(1);
            pending.changed = None;
            pending.watch_error = None;
            pending.reconfigure = None;
            let epoch = pending.epoch;
            let old = pending.watcher.take();
            drop(pending);
            drop(old);
            epoch
        };
        if !enabled {
            return ReloadEvent::WatchChanged {
                enabled: false,
                error: None,
            };
        }
        let weak: Weak<Shared> = Arc::downgrade(&self.shared);
        let watcher = watch::start(&self.source, &self.cwd, move |event| {
            if let Some(shared) = weak.upgrade() {
                shared.event(epoch, event);
            }
        });
        match watcher {
            Ok(watcher) => {
                let mut pending = self
                    .shared
                    .pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if pending.stopped {
                    drop(pending);
                    drop(watcher);
                    return ReloadEvent::WatchChanged {
                        enabled: false,
                        error: None,
                    };
                }
                pending.watcher = Some(watcher);
                // Close the initial-load/setup race, including an empty diff
                // and index changes between discovery and native subscription.
                pending.reconfigure = Some(pending.reconfigure.unwrap_or(false));
                pending.reload = true;
                ReloadEvent::WatchChanged {
                    enabled: true,
                    error: None,
                }
            }
            Err(error) => ReloadEvent::WatchChanged {
                enabled: false,
                error: Some(error),
            },
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Also runs on panic, so the receiver reports a real disconnect and an
        // idle controller cannot leave a native watcher behind.
        self.shared.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, AccessMode, Flag};

    #[test]
    fn reads_and_access_times_do_not_feed_back() {
        let shared = Shared::default();
        for kind in [
            EventKind::Access(AccessKind::Read),
            EventKind::Access(AccessKind::Open(AccessMode::Read)),
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)),
        ] {
            shared.event(0, Ok(Event::new(kind)));
        }
        assert!(shared.pending.lock().unwrap().changed.is_none());
        shared.event(0, Ok(Event::new(EventKind::Any).set_flag(Flag::Rescan)));
        assert!(shared.pending.lock().unwrap().changed.is_some());
    }

    #[test]
    fn watch_error_schedules_recovery_without_discarding_error() {
        let shared = Shared::default();
        shared.event(0, Err(notify::Error::generic("event stream failed")));
        let pending = shared.pending.lock().unwrap();
        assert!(pending.changed.is_some());
        assert!(
            pending
                .watch_error
                .as_ref()
                .unwrap()
                .contains("event stream failed")
        );
    }

    #[test]
    fn stale_callbacks_and_callbacks_after_stop_are_ignored() {
        let shared = Shared::default();
        shared.pending.lock().unwrap().epoch = 1;
        shared.event(0, Ok(Event::new(EventKind::Any)));
        assert!(shared.pending.lock().unwrap().changed.is_none());
        shared.stop();
        shared.event(1, Ok(Event::new(EventKind::Any)));
        assert!(shared.pending.lock().unwrap().changed.is_none());
    }

    #[test]
    fn a_disconnected_worker_is_not_an_empty_queue() {
        let (sender, events) = mpsc::channel();
        let controller = ReloadController {
            shared: Arc::default(),
            events,
        };
        assert!(controller.try_recv().unwrap().is_none());
        drop(sender);
        assert!(controller.try_recv().unwrap_err().contains("disconnected"));
        controller.shared.stop();
        assert!(controller.request_reload().is_err());
        assert!(controller.set_watch(true).is_err());
    }
}
