use std::{
    io::{self, IsTerminal, Write},
    process::ExitCode,
};

use clap::Parser;
use riff::{
    app::App,
    cli::{Cli, ColorMode, ViewMode},
    config::load_config,
    input::{self, Input},
    patch::{parse_patch, sanitize},
    reload::ReloadController,
    render::print_diff_with_word_diff,
    terminal,
    theme::find_theme,
};

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            // Clap errors can echo user-provided argv. Never replay their controls.
            let text = safe_multiline(&error.to_string());
            let result = if error.use_stderr() {
                io::stderr().lock().write_all(text.as_bytes())
            } else {
                io::stdout().lock().write_all(text.as_bytes())
            };
            if result.is_err_and(|e| e.kind() != io::ErrorKind::BrokenPipe) {
                return ExitCode::FAILURE;
            }
            return ExitCode::from(error.exit_code() as u8);
        }
    };
    match execute(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr().lock(), "riff: {}", sanitize(&error));
            ExitCode::FAILURE
        }
    }
}

fn execute(cli: Cli) -> Result<(), String> {
    let settings = load_config(cli.config.as_deref())?;
    if cli.list_themes {
        let mut output = io::stdout().lock();
        for theme in &settings.themes {
            if let Err(error) = writeln!(output, "{}", sanitize(&theme.name)) {
                return output_result(Err(error));
            }
        }
        return Ok(());
    }
    let selected = cli
        .theme
        .clone()
        .or_else(|| std::env::var("RIFF_THEME").ok())
        .unwrap_or(settings.theme);
    let theme_index = find_theme(&selected, &settings.themes)
        .ok_or_else(|| format!("unknown theme {selected:?}; use --list-themes"))?;
    let view = match cli.view {
        Some(view) => view,
        None => ViewMode::parse_config(settings.view.as_deref().unwrap_or("auto"))?,
    };
    let wrap = if cli.wrap {
        true
    } else if cli.no_wrap {
        false
    } else {
        settings.wrap
    };
    let word_diff = cli.word_diff || (!cli.no_word_diff && settings.word_diff);
    let (source, paths) = cli.resolve_input(io::stdin().is_terminal())?;
    let stdout_is_terminal = io::stdout().is_terminal();
    let watch = cli.effective_watch(settings.watch, &source, stdout_is_terminal)?;
    let text = input::load(&source, &paths)?;
    let diff = parse_patch(&text)?;
    if cli.print || !stdout_is_terminal || (diff.files.is_empty() && !watch) {
        let color = match cli.color {
            ColorMode::Always => true,
            ColorMode::Never => false,
            ColorMode::Auto => stdout_is_terminal && std::env::var_os("NO_COLOR").is_none(),
        };
        return output_result(print_diff_with_word_diff(
            &mut io::stdout().lock(),
            &diff,
            &settings.themes[theme_index],
            !cli.no_syntax,
            color,
            word_diff,
        ));
    }
    let source_label = match &source {
        Input::WorkingTree => "unstaged changes".to_string(),
        Input::Staged => "staged changes".to_string(),
        Input::Commit(reference) => format!("commit {reference}"),
        Input::Stdin => "stdin patch".to_string(),
        Input::Files(old, new) => format!("{} ↔ {}", old.display(), new.display()),
    };
    let mut app = App::new(
        diff,
        settings.themes,
        theme_index,
        view,
        !cli.no_syntax,
        source_label,
    )?;
    app.set_word_diff(word_diff);
    app.set_wrap(wrap);
    // Preserve the resolved source and literal/pathspec filters once. A deleted
    // positional path must never be reinterpreted as a revision on reload.
    let controller = if matches!(source, Input::Stdin) {
        app.reload_unavailable("stdin patch cannot be reread; reload/watch unavailable");
        None
    } else {
        match ReloadController::new(source, paths, text) {
            Ok(controller) => {
                app.configure_reload(true, watch);
                Some(controller)
            }
            Err(error) => {
                app.reload_unavailable(&error);
                app.handle_reload_event(riff::reload::ReloadEvent::Error(error));
                None
            }
        }
    };
    terminal::run_with_reload(&mut app, cli.config.as_deref(), controller.as_ref())
        .map_err(|e| format!("terminal: {e}"))
}

fn output_result(result: io::Result<()>) -> Result<(), String> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(format!("output: {error}")),
    }
}

/// Keep our own layout newlines, but neutralize all other terminal controls.
fn safe_multiline(text: &str) -> String {
    text.split('\n')
        .map(sanitize)
        .collect::<Vec<_>>()
        .join("\n")
}
