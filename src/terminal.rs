//! Owns terminal side effects, including cleanup after partial setup and panics.
use std::{
    io::{self, Stdout},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use crossterm::{
    cursor::{Hide, Show},
    event::{
        self, DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::{
    app::App,
    config::{self, SettingsUpdate},
    patch::sanitize,
    reload::{ReloadController, ReloadEvent},
    ui,
};

static ACTIVE: AtomicBool = AtomicBool::new(false);
static KEYBOARD_ACTIVE: AtomicBool = AtomicBool::new(false);

fn restore_keyboard() {
    // Popping a stack is not idempotent: the panic hook and Drop must not each
    // pop, or we would also remove the caller's previous keyboard settings.
    if KEYBOARD_ACTIVE.swap(false, Ordering::SeqCst) {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    }
}

/// Handlers only set an atomic flag; all I/O and termios cleanup stays on the
/// normal event-loop thread, never in an async-signal context.
#[cfg(unix)]
struct ShutdownSignals {
    requested: std::sync::Arc<AtomicBool>,
    handlers: Vec<signal_hook::SigId>,
}

#[cfg(unix)]
impl ShutdownSignals {
    fn install() -> io::Result<Self> {
        use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
        let mut signals = Self {
            requested: std::sync::Arc::new(AtomicBool::new(false)),
            handlers: Vec::new(),
        };
        for signal in [SIGTERM, SIGINT, SIGHUP] {
            signals.handlers.push(signal_hook::flag::register(
                signal,
                std::sync::Arc::clone(&signals.requested),
            )?);
        }
        Ok(signals)
    }

    fn requested(&self) -> bool {
        self.requested.load(Ordering::Relaxed)
    }
}

#[cfg(unix)]
impl Drop for ShutdownSignals {
    fn drop(&mut self) {
        for handler in self.handlers.drain(..) {
            signal_hook::low_level::unregister(handler);
        }
    }
}

struct Session {
    terminal: Option<Terminal<CrosstermBackend<Stdout>>>,
    raw: bool,
    alternate: bool,
    mouse: bool,
    cursor_hidden: bool,
    keyboard: bool,
}

impl Session {
    fn open() -> io::Result<Self> {
        let mut session = Self {
            terminal: None,
            raw: false,
            alternate: false,
            mouse: false,
            cursor_hidden: false,
            keyboard: false,
        };
        ACTIVE.store(true, Ordering::SeqCst);
        // Mark each attempted operation before doing it: writes can fail after
        // partially reaching the terminal, and cleanup must still be attempted.
        session.raw = true;
        enable_raw_mode()?;
        session.alternate = true;
        execute!(io::stdout(), EnterAlternateScreen)?;
        session.keyboard = true;
        KEYBOARD_ACTIVE.store(true, Ordering::SeqCst);
        match execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
            )
        ) {
            Ok(()) => {}
            // Crossterm's legacy Windows backend reports native modifiers but
            // explicitly rejects this ANSI-only command; do not break that TUI.
            Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                session.keyboard = false;
                KEYBOARD_ACTIVE.store(false, Ordering::SeqCst);
            }
            Err(error) => return Err(error),
        }
        session.mouse = true;
        execute!(io::stdout(), EnableMouseCapture)?;
        session.cursor_hidden = true;
        execute!(io::stdout(), Hide)?;
        session.terminal = Some(Terminal::new(CrosstermBackend::new(io::stdout()))?);
        Ok(session)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.raw {
            let _ = disable_raw_mode();
        }
        if self.keyboard {
            restore_keyboard();
        }
        if self.mouse {
            let _ = execute!(io::stdout(), DisableMouseCapture);
        }
        if self.alternate {
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
        }
        if self.cursor_hidden {
            let _ = execute!(io::stdout(), Show);
        }
        ACTIVE.store(false, Ordering::SeqCst);
    }
}

fn install_panic_cleanup() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Panic hooks run BEFORE stack unwinding, so restore before the default
        // hook prints diagnostics. The Session guard then repeats cleanup safely.
        if ACTIVE.swap(false, Ordering::SeqCst) {
            let _ = disable_raw_mode();
            restore_keyboard();
            let _ = execute!(
                io::stdout(),
                DisableMouseCapture,
                LeaveAlternateScreen,
                Show
            );
        }
        previous(info);
    }));
}

pub fn run(app: &mut App) -> io::Result<()> {
    run_with_config(app, None)
}

/// Persist only explicitly accepted interactive choices; failures stay in the TUI.
pub fn persist_settings(app: &mut App, path: Option<&Path>) {
    let update = app.take_settings_update();
    if update == SettingsUpdate::default() {
        return;
    }
    app.save_error = config::save_config(path, &update)
        .err()
        .map(|error| sanitize(&error));
}

pub fn run_with_config(app: &mut App, path: Option<&Path>) -> io::Result<()> {
    run_with_reload(app, path, None)
}

/// Execute queued commands and consume ready results without waiting for Git.
/// Watch setup failures leave the controller available for manual reload.
pub fn service_reload(app: &mut App, controller: &ReloadController) {
    let requests = app.take_reload_requests();
    if let Some(enabled) = requests.watch
        && let Err(error) = controller.set_watch(enabled)
    {
        app.handle_reload_event(ReloadEvent::WatchChanged {
            enabled: app.watch_enabled(),
            error: Some(error),
        });
    }
    if requests.reload
        && let Err(error) = controller.request_reload()
    {
        app.handle_reload_event(ReloadEvent::Error(error));
    }
    loop {
        match controller.try_recv() {
            Ok(Some(event)) => app.handle_reload_event(event),
            Ok(None) => break,
            Err(error) => {
                app.reload_unavailable(&format!("reload worker disconnected: {error}"));
                app.handle_reload_event(ReloadEvent::Error(error));
                break;
            }
        }
    }
}

pub fn run_with_reload(
    app: &mut App,
    path: Option<&Path>,
    controller: Option<&ReloadController>,
) -> io::Result<()> {
    install_panic_cleanup();
    #[cfg(unix)]
    let shutdown = ShutdownSignals::install()?;
    let mut session = Session::open()?;
    let terminal = session
        .terminal
        .as_mut()
        .expect("successful terminal initialization");
    while app.running {
        #[cfg(unix)]
        if shutdown.requested() {
            app.running = false;
            break;
        }
        if let Some(controller) = controller {
            service_reload(app, controller);
        }
        persist_settings(app, path);
        terminal.draw(|frame| {
            let area = frame.area();
            app.resize(area.width, area.height);
            ui::draw(frame, app);
        })?;
        if event::poll(Duration::from_millis(50))? {
            app.handle_event(event::read()?);
        }
    }
    Ok(())
}
