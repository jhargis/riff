//! End-to-end CLI behavior, with no terminal and no persistent Git mutations.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

use tempfile::TempDir;

const PATCH: &str = "diff --git a/example.rs b/example.rs\n--- a/example.rs\n+++ b/example.rs\n@@ -1,2 +1,2 @@\n-fn old() {}\n+fn new() {}\n \n";

fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_riff"));
    command
        .current_dir(directory)
        .env("XDG_CONFIG_HOME", directory.join("isolated-config"))
        .env_remove("RIFF_THEME")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", directory.join("missing-gitconfig"))
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn run(command: &mut Command, input: &str) -> Output {
    let mut child = command.spawn().expect("run riff");
    // Very small fixtures fit in the pipe; closing it also verifies EOF handling.
    if let Some(mut stdin) = child.stdin.take() {
        // A CLI argument error is allowed to exit before reading stdin.
        let _ = stdin.write_all(input.as_bytes());
    }
    child.wait_with_output().expect("wait for riff")
}

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}
fn error(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_success(output: &Output) {
    assert!(output.status.success(), "stderr: {}", error(output));
}

fn assert_plain(text: &str) {
    assert!(
        text.chars().all(|ch| ch == '\n' || !ch.is_control()),
        "unsafe output: {text:?}"
    );
}

#[test]
fn help_version_and_listing_work_outside_git() {
    let directory = TempDir::new().unwrap();
    for args in [vec!["--help"], vec!["--version"], vec!["--list-themes"]] {
        let output = run(command(directory.path()).args(args), "");
        assert_success(&output);
        assert!(!text(&output).is_empty());
        assert_plain(&text(&output));
    }
    let output = run(command(directory.path()).arg("--list-themes"), "");
    let names: Vec<String> = text(&output)
        .lines()
        .map(|line| line.to_lowercase().replace([' ', '-', '_'], ""))
        .collect();
    for expected in [
        "critique",
        "dracula",
        "catppuccin",
        "nord",
        "gruvbox",
        "sunset",
        "onedark",
        "monokai",
        "hackerterm",
        "githubdark",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "missing {expected}: {names:?}"
        );
    }
}

#[test]
fn automatic_stdin_and_non_tty_stdout_print_without_tui() {
    let directory = TempDir::new().unwrap();
    let output = run(&mut command(directory.path()), PATCH);
    assert_success(&output);
    let printed = text(&output);
    assert!(printed.contains("example.rs"));
    assert!(printed.contains("@@ -1,2 +1,2 @@"));
    assert!(printed.contains("1   - fn old() {}"));
    assert!(printed.contains("  1 + fn new() {}"));
    assert_plain(&printed);
}

#[test]
fn sources_are_exclusive_and_path_filters_are_validated() {
    let directory = TempDir::new().unwrap();
    for args in [
        vec!["--stdin", "--staged"],
        vec!["--commit", "HEAD", "HEAD~1"],
        vec!["--files", "old", "new", "--staged"],
        vec!["--stdin", "--", "some/path"],
    ] {
        let output = run(command(directory.path()).args(args), "");
        assert!(!output.status.success());
        assert!(!error(&output).is_empty());
    }
}

#[test]
fn requested_git_source_does_not_get_swallowed_by_piped_patch() {
    let directory = TempDir::new().unwrap();
    for args in [
        vec!["--staged"],
        vec!["--commit", "HEAD"],
        vec!["HEAD"],
        vec!["--", "example.rs"],
    ] {
        // Outside Git, an explicit Git request must fail rather than show PATCH.
        let output = run(command(directory.path()).args(args), PATCH);
        assert!(!output.status.success());
        assert!(!text(&output).contains("fn new"));
        assert!(!error(&output).is_empty());
    }
}

#[test]
fn files_differing_and_equal_both_exit_successfully() {
    let directory = TempDir::new().unwrap();
    let old = directory.path().join("old.txt");
    let new = directory.path().join("new.txt");
    fs::write(&old, "old\n").unwrap();
    fs::write(&new, "new\n").unwrap();
    let output = run(
        command(directory.path())
            .arg("--files")
            .arg(&old)
            .arg(&new)
            .args(["--color", "never"]),
        PATCH,
    );
    assert_success(&output);
    assert!(text(&output).contains("- old"));
    assert!(text(&output).contains("+ new"));
    fs::write(&new, "old\n").unwrap();
    let output = run(
        command(directory.path()).arg("--files").arg(&old).arg(&new),
        "",
    );
    assert_success(&output);
    assert!(text(&output).contains("No differences"));
}

#[test]
fn empty_patch_is_a_successful_no_difference_result() {
    let directory = TempDir::new().unwrap();
    let output = run(command(directory.path()).args(["--stdin", "--print"]), "");
    assert_success(&output);
    assert!(text(&output).contains("No differences"));
}

#[test]
fn config_custom_palette_and_explicit_color_are_used() {
    let directory = TempDir::new().unwrap();
    let config = directory.path().join("custom.toml");
    fs::write(&config, "theme = 'my palette'\nview = 'split'\n[themes.'my palette']\nextends = 'critique'\nbackground = '#101010'\nkeyword = '#ff0000'\n").unwrap();
    let output = run(
        command(directory.path())
            .arg("--config")
            .arg(&config)
            .args(["--list-themes"]),
        "",
    );
    assert_success(&output);
    assert!(text(&output).contains("my palette"));
    let output = run(
        command(directory.path())
            .arg("--config")
            .arg(&config)
            .args(["--stdin", "--print", "--color", "always"]),
        PATCH,
    );
    assert_success(&output);
    assert!(text(&output).contains("\x1b[48;2;16;16;16m"));
    assert!(text(&output).contains("\x1b[38;2;255;0;0m"));
    // Unspecified removal surfaces inherit the built-in unchanged.
    assert!(text(&output).contains("\x1b[48;2;45;13;13m"));
    let output = run(
        command(directory.path())
            .arg("--config")
            .arg(&config)
            .args(["--stdin", "--print", "--color", "never", "--no-syntax"]),
        PATCH,
    );
    assert_success(&output);
    assert_plain(&text(&output));
}

#[test]
fn unknown_themes_missing_configs_and_invalid_config_are_errors() {
    let directory = TempDir::new().unwrap();
    let output = run(
        command(directory.path()).args(["--theme", "no-such-palette", "--stdin"]),
        "",
    );
    assert!(!output.status.success());
    assert!(error(&output).contains("unknown theme"));
    let output = run(
        command(directory.path()).args(["--config", "missing.toml", "--list-themes"]),
        "",
    );
    assert!(!output.status.success());
    for config_text in [
        "[themes.bad]\nextends = 'critique'\nbackground = '#gg0000'\n",
        "[themes.critique]\nbackground = '#000000'\n",
        "[themes.bad]\nextends = 'critique'\nunknown_color = '#123456'\n",
    ] {
        let config = directory.path().join("invalid.toml");
        fs::write(&config, config_text).unwrap();
        let output = run(
            command(directory.path())
                .arg("--config")
                .arg(&config)
                .arg("--list-themes"),
            "",
        );
        assert!(!output.status.success(), "accepted {config_text}");
        assert!(!error(&output).is_empty());
    }
}

#[test]
fn theme_aliases_and_environment_precedence() {
    let directory = TempDir::new().unwrap();
    for theme in ["DRACULA", "one_dark", "hacker-term", "github dark"] {
        let output = run(
            command(directory.path()).args(["--stdin", "--theme", theme]),
            "",
        );
        assert_success(&output);
    }
    let output = run(
        command(directory.path())
            .env("RIFF_THEME", "not-a-theme")
            .arg("--stdin"),
        "",
    );
    assert!(!output.status.success());
    let output = run(
        command(directory.path())
            .env("RIFF_THEME", "not-a-theme")
            .args(["--stdin", "--theme", "nord"]),
        "",
    );
    assert_success(&output);
}

#[test]
fn controls_are_visible_not_executed_and_newline_metadata_survives() {
    let directory = TempDir::new().unwrap();
    let patch = "--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+\u{1b}[31m\t界\u{85}\n\\ No newline at end of file\n";
    let output = run(
        command(directory.path()).args(["--stdin", "--color", "never"]),
        patch,
    );
    assert_success(&output);
    assert!(text(&output).contains("No newline at end of file"));
    assert!(text(&output).contains('界'));
    assert_plain(&text(&output));
    let output = run(command(directory.path()).arg("--unknown-\u{1b}[31m"), "");
    assert!(!output.status.success());
    assert_plain(&error(&output));
}

#[test]
fn listing_recovers_from_invalid_theme_overrides() {
    let directory = TempDir::new().unwrap();
    let output = run(
        command(directory.path())
            .env("RIFF_THEME", "not-a-theme")
            .args(["--theme", "also-invalid", "--list-themes"]),
        "",
    );
    assert_success(&output);
    assert!(text(&output).lines().any(|line| line == "critique"));
}

#[test]
fn every_theme_prints_plain_or_its_own_ansi_palette() {
    use ratatui::style::Color;
    let directory = TempDir::new().unwrap();
    let mut plain = None;
    for theme in riff::theme::builtin_themes() {
        let output = run(
            command(directory.path()).args([
                "--stdin",
                "--print",
                "--theme",
                &theme.name,
                "--color",
                "never",
            ]),
            PATCH,
        );
        assert_success(&output);
        assert_plain(&text(&output));
        let expected = plain.get_or_insert_with(|| text(&output));
        assert_eq!(&text(&output), expected);
        let output = run(
            command(directory.path()).args([
                "--stdin",
                "--print",
                "--theme",
                &theme.name,
                "--color",
                "always",
            ]),
            PATCH,
        );
        assert_success(&output);
        let printed = text(&output);
        for (color, selector) in [
            (theme.palette.keyword, 38),
            (theme.palette.added_background, 48),
            (theme.palette.removed_background, 48),
            (theme.palette.header, 48),
        ] {
            let Color::Rgb(r, g, b) = color else {
                panic!("built-in RGB palette")
            };
            assert!(
                printed.contains(&format!("\x1b[{selector};2;{r};{g};{b}m")),
                "{}",
                theme.name
            );
        }
    }
}

#[test]
fn typescript_and_tsx_print_semantic_colors_and_honor_no_syntax() {
    use ratatui::style::Color;
    let directory = TempDir::new().unwrap();
    for theme in &riff::theme::builtin_themes()[..2] {
        for syntax in [true, false] {
            let mut command = command(directory.path());
            command.args([
                "--stdin",
                "--print",
                "--color",
                "always",
                "--theme",
                &theme.name,
            ]);
            if !syntax {
                command.arg("--no-syntax");
            }
            let output = run(&mut command, include_str!("fixtures/typescript.patch"));
            assert_success(&output);
            let printed = text(&output);
            for (token, color) in [
                ("interface", theme.palette.keyword),
                ("Person", theme.palette.type_name),
                ("Base", theme.palette.type_name),
                ("satisfies", theme.palette.keyword),
                ("hello", theme.palette.string),
                ("unchanged", theme.palette.comment),
                ("span", theme.palette.variable),
                ("ready", theme.palette.variable),
                ("雪", theme.palette.string),
                ("null", theme.palette.number),
            ] {
                let Color::Rgb(r, g, b) = color else {
                    panic!("built-in RGB palette")
                };
                let prefix = format!("\x1b[38;2;{r};{g};{b}m");
                // Each printed span has its own foreground and trailing reset.
                let colored = printed.split("\x1b[0m").any(|span| {
                    span.trim_start_matches('\n').starts_with(&prefix) && span.contains(token)
                });
                assert_eq!(colored, syntax, "{} {token}: {printed:?}", theme.name);
                assert!(printed.contains(token));
            }
        }
    }
}

#[test]
fn malformed_patch_is_reported_instead_of_opening_a_tui() {
    let directory = TempDir::new().unwrap();
    let output = run(command(directory.path()).arg("--stdin"), "not a patch\n");
    assert!(!output.status.success());
    assert!(!error(&output).is_empty());
}
