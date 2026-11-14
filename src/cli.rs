use std::{path::PathBuf, process::Command};

use clap::{Parser, ValueEnum};

use crate::input::Input;

/// An independent, read-only, interactive diff viewer.
#[derive(Debug, Parser)]
#[command(
    name = "riff",
    version,
    color = clap::ColorChoice::Never,
    about = "A read-only terminal diff viewer",
    after_help = "Without an explicit source, piped stdin is read as a patch; otherwise unstaged Git changes are shown.\nExplicit Git/file sources and bare paths take precedence over piped stdin.\nUse -- for multiple path filters or ambiguous names; use --commit to force a revision.\nMultiple bare paths are not supported; use riff -- PATH...\n\nExamples:\n  riff -- file1.rs file2.rs file3.rs\n  riff --staged -- file1.rs file2.rs\n  riff HEAD -- file1.rs file2.rs\n\nKeys: j/k scroll, n/p hunks, ]/[ files, / search, t themes, v view, w wrap, i word highlighting, ctrl+w (no Shift) watch, r/Ctrl-R reload, Space/Shift-Space page down/up, ? help, q quit."
)]
pub struct Cli {
    /// Show staged Git changes
    #[arg(long, group = "source")]
    pub staged: bool,

    /// Show a commit with git show
    #[arg(long, value_name = "REF", group = "source")]
    pub commit: Option<String>,

    /// Compare two files (differences are a successful result)
    #[arg(long, num_args = 2, value_names = ["OLD", "NEW"], group = "source")]
    pub files: Option<Vec<PathBuf>>,

    /// Read a unified patch from stdin, even when it is a terminal
    #[arg(long, group = "source")]
    pub stdin: bool,

    /// Commit to show, or a working-tree file/directory relative to the current directory
    #[arg(value_name = "REF_OR_PATH", group = "source")]
    pub reference: Option<String>,

    /// Git path filters. Place -- before this list.
    #[arg(last = true, value_name = "PATH")]
    pub paths: Vec<PathBuf>,

    /// Theme name (overrides RIFF_THEME and config)
    #[arg(long, value_name = "NAME")]
    pub theme: Option<String>,

    /// List built-in and configured custom themes without loading a diff
    #[arg(long)]
    pub list_themes: bool,

    /// Read this config file instead of the default location
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Layout; auto selects split at 100 columns
    #[arg(long, value_enum)]
    pub view: Option<ViewMode>,

    /// Wrap interactive code at word boundaries (overrides config)
    #[arg(long, conflicts_with = "no_wrap")]
    pub wrap: bool,

    /// Clip interactive code instead of wrapping (overrides config)
    #[arg(long, conflicts_with = "wrap")]
    pub no_wrap: bool,

    /// Emphasize changed words with theme backgrounds (default; overrides config)
    #[arg(long, conflicts_with = "no_word_diff")]
    pub word_diff: bool,

    /// Disable changed-word backgrounds for this run (overrides config)
    #[arg(long, conflicts_with = "word_diff")]
    pub no_word_diff: bool,

    /// Watch the original diff source in the TUI (overrides saved preference)
    #[arg(long, conflicts_with = "no_watch")]
    pub watch: bool,

    /// Disable automatic reload for this run (overrides saved preference)
    #[arg(long, conflicts_with = "watch")]
    pub no_watch: bool,

    /// Print a numbered unified view instead of opening the TUI
    #[arg(long = "print")]
    pub print: bool,

    /// ANSI output policy for --print (never affects TUI colors)
    #[arg(long, value_enum, default_value = "auto")]
    pub color: ColorMode,

    /// Disable syntax highlighting
    #[arg(long)]
    pub no_syntax: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ViewMode {
    Auto,
    Split,
    Unified,
}

impl ViewMode {
    pub fn parse_config(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "split" => Ok(Self::Split),
            "unified" => Ok(Self::Unified),
            _ => Err(format!(
                "invalid view {value:?}: expected auto, split, or unified"
            )),
        }
    }

    pub fn is_split(self, width: u16) -> bool {
        match self {
            Self::Auto => width >= 100,
            Self::Split => true,
            Self::Unified => false,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Split => "split",
            Self::Unified => "unified",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

impl Cli {
    /// Saved watch is meaningful only for replayable, interactive sources.
    /// Validate explicit requests before reading stdin or starting a controller.
    pub fn effective_watch(
        &self,
        saved: bool,
        source: &Input,
        stdout_is_terminal: bool,
    ) -> Result<bool, String> {
        if self.watch && (self.print || !stdout_is_terminal) {
            return Err(
                "--watch requires an interactive TUI; remove --print and use terminal stdout"
                    .into(),
            );
        }
        if self.watch && matches!(source, Input::Stdin) {
            return Err("--watch is unavailable for stdin patches, which cannot be reread".into());
        }
        Ok(!self.print
            && stdout_is_terminal
            && !matches!(source, Input::Stdin)
            && !self.no_watch
            && (self.watch || saved))
    }

    /// Resolve source and filters together so a positional path cannot be lost.
    pub fn resolve_input(&self, stdin_is_terminal: bool) -> Result<(Input, Vec<PathBuf>), String> {
        if !self.paths.is_empty() && (self.stdin || self.files.is_some()) {
            return Err("path filters apply only to Git sources, not --stdin or --files".into());
        }
        let mut paths = self.paths.clone();
        let source = if let Some(files) = &self.files {
            Input::Files(files[0].clone(), files[1].clone())
        } else if self.stdin {
            Input::Stdin
        } else if self.staged {
            Input::Staged
        } else if let Some(reference) = &self.commit {
            Input::Commit(reference.clone())
        } else if let Some(token) = &self.reference {
            // Query the index too: deleted tracked files/directories still name
            // paths. Literal lookup avoids mistaking a wildcard for a real path.
            let path = PathBuf::from(token);
            let is_path = path.exists()
                || git_succeeds(&[
                    "--literal-pathspecs",
                    "ls-files",
                    "--error-unmatch",
                    "--",
                    token,
                ])?;
            if is_path {
                if git_succeeds(&[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "--end-of-options",
                    &format!("{token}^{{commit}}"),
                ])? {
                    return Err(format!(
                        "ambiguous argument {token:?}: both a path and a revision; use '-- {token}' for a path or '--commit {token}' for a revision"
                    ));
                }
                // Recognition above is literal; retain that identity when Git
                // loads the diff. Only explicit filters after `--` are pathspecs.
                paths.insert(0, PathBuf::from(format!(":(literal){token}")));
                Input::WorkingTree
            } else {
                // Preserve Git's revision syntax and diagnostics for unknown refs.
                Input::Commit(token.clone())
            }
        } else if stdin_is_terminal || !paths.is_empty() {
            // A path filter explicitly requests Git, never a stdin patch filter.
            Input::WorkingTree
        } else {
            Input::Stdin
        };
        Ok((source, paths))
    }
}

/// Recognition probes must neither refresh the index nor run its fsmonitor hook.
fn git_succeeds(args: &[&str]) -> Result<bool, String> {
    Command::new("git")
        .args([
            "--no-pager",
            "--no-optional-locks",
            "-c",
            "diff.autoRefreshIndex=false",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .map(|output| output.status.success())
        .map_err(|error| format!("Cannot run Git: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_flags_are_exclusive() {
        for args in [
            vec!["riff", "--staged", "--stdin"],
            vec!["riff", "--commit", "HEAD", "HEAD~1"],
            vec!["riff", "--files", "a", "b", "--staged"],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn explicit_source_beats_pipe() {
        let args = Cli::try_parse_from(["riff", "--staged"]).unwrap();
        assert!(matches!(
            args.resolve_input(false).unwrap().0,
            Input::Staged
        ));
        let args = Cli::try_parse_from(["riff"]).unwrap();
        assert!(matches!(args.resolve_input(false).unwrap().0, Input::Stdin));
        assert!(matches!(
            args.resolve_input(true).unwrap().0,
            Input::WorkingTree
        ));
    }

    #[test]
    fn paths_after_separator_are_not_references() {
        let args = Cli::try_parse_from(["riff", "--", "src/main.rs"]).unwrap();
        assert!(args.reference.is_none());
        assert_eq!(args.paths, vec![PathBuf::from("src/main.rs")]);
        let (source, paths) = args.resolve_input(false).unwrap();
        assert!(matches!(source, Input::WorkingTree));
        assert_eq!(paths, vec![PathBuf::from("src/main.rs")]);
    }
}
