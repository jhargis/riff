//! Strict TOML configuration with built-in-only custom palette inheritance.
//!
//! Missing default configuration is fine; an explicitly requested file must
//! exist. Precedence between CLI, environment, and these settings belongs to
//! the frontend.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ratatui::style::Color;
use serde::Deserialize;

use crate::theme::{Palette, Theme, builtin_themes, find_theme};

#[path = "config_persistence.rs"]
mod persistence;
pub use persistence::save_config;

/// Only explicitly accepted interactive changes are persisted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsUpdate {
    pub theme: Option<String>,
    pub view: Option<String>,
    pub wrap: Option<bool>,
    pub word_diff: Option<bool>,
    pub watch: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub theme: String,
    pub view: Option<String>,
    pub wrap: bool,
    pub word_diff: bool,
    pub watch: bool,
    pub themes: Vec<Theme>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawConfig {
    theme: Option<String>,
    view: Option<String>,
    wrap: Option<bool>,
    word_diff: Option<bool>,
    watch: Option<bool>,
    themes: BTreeMap<String, RawTheme>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawTheme {
    extends: Option<String>,
    background: Option<String>,
    foreground: Option<String>,
    header: Option<String>,
    hunk: Option<String>,
    border: Option<String>,
    muted: Option<String>,
    gutter: Option<String>,
    gutter_foreground: Option<String>,
    added_background: Option<String>,
    removed_background: Option<String>,
    added_word_background: Option<String>,
    removed_word_background: Option<String>,
    added_gutter: Option<String>,
    removed_gutter: Option<String>,
    added_foreground: Option<String>,
    removed_foreground: Option<String>,
    selection: Option<String>,
    keyword: Option<String>,
    string: Option<String>,
    number: Option<String>,
    comment: Option<String>,
    function: Option<String>,
    type_name: Option<String>,
    variable: Option<String>,
}

/// The native per-user configuration directory, followed by `riff/config.toml`.
///
/// Unix (except macOS) honors only absolute `XDG_CONFIG_HOME` paths, falling
/// back to the native home directory's `.config`. macOS uses Application
/// Support; Windows uses roaming AppData. Discovery never creates anything.
pub fn default_config_path() -> Option<PathBuf> {
    #[cfg(all(unix, not(target_os = "macos")))]
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|root| root.is_absolute())
        .or_else(|| {
            dirs::home_dir()
                .filter(|home| home.is_absolute())
                .map(|home| home.join(".config"))
        });
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    let root = dirs::config_dir().filter(|root| root.is_absolute());

    root.map(|root| root.join("riff").join("config.toml"))
}

pub fn load_config(path: Option<&Path>) -> Result<Settings, String> {
    match path {
        Some(path) => load_path(path, true),
        None => match default_config_path() {
            Some(path) => load_path(&path, false),
            None => parse_config(""),
        },
    }
}

fn load_path(path: &Path, explicit: bool) -> Result<Settings, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if !explicit && error.kind() == std::io::ErrorKind::NotFound => {
            return parse_config("");
        }
        Err(error) => {
            return Err(format!(
                "could not read configuration {}: {error}",
                path.display()
            ));
        }
    };
    parse_config(&text).map_err(|error| format!("configuration {}: {error}", path.display()))
}

fn parse_config(text: &str) -> Result<Settings, String> {
    let raw: RawConfig = toml::from_str(text).map_err(|error| format!("invalid TOML: {error}"))?;
    let mut themes = builtin_themes();
    let builtin_count = themes.len();

    // BTreeMap keeps custom-theme ordering reproducible after all built-ins.
    for (name, custom) in raw.themes {
        if name.chars().all(|c| matches!(c, ' ' | '-' | '_')) || name.chars().any(char::is_control)
        {
            return Err(format!(
                "invalid custom theme name {name:?}: use a nonempty name without control characters"
            ));
        }
        if let Some(index) = find_theme(&name, &themes) {
            return Err(if index < builtin_count {
                format!(
                    "custom theme {name:?} cannot override built-in theme {:?}",
                    themes[index].name
                )
            } else {
                format!(
                    "duplicate custom theme {name:?}: it aliases {:?}",
                    themes[index].name
                )
            });
        }
        let extends = custom.extends.as_deref().unwrap_or("critique");
        let base = find_theme(extends, &themes[..builtin_count]).ok_or_else(|| {
            format!(
                "custom theme {name:?} extends {extends:?}, which is not a built-in theme; custom themes may inherit built-ins only"
            )
        })?;
        let mut palette = themes[base].palette.clone();
        custom.apply(&mut palette, &name)?;
        themes.push(Theme { name, palette });
    }

    let requested = raw.theme.as_deref().unwrap_or("critique");
    let index = find_theme(requested, &themes).ok_or_else(|| {
        let available = themes
            .iter()
            .map(|theme| theme.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        format!("unknown theme {requested:?}; available themes: {available}")
    })?;
    let view = raw
        .view
        .map(|view| {
            let normalized = view.to_ascii_lowercase();
            match normalized.as_str() {
                "auto" | "unified" | "split" => Ok(normalized),
                _ => Err(format!(
                    "unknown view {view:?}; expected auto, unified, or split"
                )),
            }
        })
        .transpose()?;
    Ok(Settings {
        theme: themes[index].name.clone(),
        view,
        wrap: raw.wrap.unwrap_or(false),
        word_diff: raw.word_diff.unwrap_or(true),
        watch: raw.watch.unwrap_or(false),
        themes,
    })
}

impl RawTheme {
    fn apply(&self, palette: &mut Palette, name: &str) -> Result<(), String> {
        macro_rules! apply_colors {
            ($($field:ident),+ $(,)?) => {
                $(if let Some(value) = &self.$field {
                    palette.$field = parse_color(value).map_err(|error| {
                        format!("theme {name:?}, color {}: {error}", stringify!($field))
                    })?;
                })+
            };
        }
        apply_colors!(
            background,
            foreground,
            header,
            hunk,
            border,
            muted,
            gutter,
            gutter_foreground,
            added_background,
            removed_background,
            added_word_background,
            removed_word_background,
            added_gutter,
            removed_gutter,
            added_foreground,
            removed_foreground,
            selection,
            keyword,
            string,
            number,
            comment,
            function,
            type_name,
            variable,
        );
        Ok(())
    }
}

fn parse_color(value: &str) -> Result<Color, String> {
    let bytes = value.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' || !bytes[1..].iter().all(u8::is_ascii_hexdigit) {
        return Err(format!(
            "{value:?} must be exactly #RRGGBB (six hexadecimal digits)"
        ));
    }
    // The ASCII/length checks above make each pair a valid UTF-8 boundary.
    let component = |start| u8::from_str_radix(&value[start..start + 2], 16);
    match (component(1), component(3), component(5)) {
        (Ok(red), Ok(green), Ok(blue)) => Ok(Color::Rgb(red, green, blue)),
        _ => Err(format!("invalid hexadecimal color {value:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_configuration_has_all_builtins_and_default_theme() {
        let settings = parse_config("").unwrap();
        assert_eq!(settings.theme, "critique");
        assert_eq!(settings.view, None);
        assert!(!settings.wrap);
        assert_eq!(settings.themes.len(), 10);
        assert_eq!(settings.themes[0].name, "critique");
    }

    #[test]
    fn wrap_requires_a_boolean_and_defaults_to_false() {
        assert!(!parse_config("").unwrap().wrap);
        assert!(!parse_config("wrap = false").unwrap().wrap);
        assert!(parse_config("wrap = true").unwrap().wrap);
        for value in ["'true'", "'false'", "0", "1", "[]", "{}"] {
            let error = parse_config(&format!("wrap = {value}")).unwrap_err();
            assert!(error.contains("invalid TOML"), "{value}: {error}");
        }
    }

    #[test]
    fn aliases_are_canonicalized() {
        let settings = parse_config("theme = 'GitHub_Dark'\nview = 'split'").unwrap();
        assert_eq!(settings.theme, "github-dark");
        assert_eq!(settings.view.as_deref(), Some("split"));
        assert_eq!(
            parse_config("view = 'auto'").unwrap().view.as_deref(),
            Some("auto")
        );
        assert_eq!(
            parse_config("view = 'UNIFIED'").unwrap().view.as_deref(),
            Some("unified")
        );
    }

    #[test]
    fn custom_inherits_a_builtin_and_can_be_selected_by_alias() {
        let settings = parse_config(
            "theme = 'MY_THEME'\n[themes.'my theme']\nextends = 'One Dark'\nbackground = '#101010'\nkeyword = '#ff0000'",
        )
        .unwrap();
        assert_eq!(settings.theme, "my theme");
        assert_eq!(settings.themes.len(), 11);
        assert_eq!(settings.themes[10].name, "my theme");
        let base = &settings.themes[find_theme("one dark", &settings.themes).unwrap()].palette;
        let custom = &settings.themes[10].palette;
        assert_eq!(custom.background, Color::Rgb(0x10, 0x10, 0x10));
        assert_eq!(custom.keyword, Color::Rgb(0xff, 0, 0));
        assert_eq!(custom.foreground, base.foreground);
        assert_eq!(custom.function, base.function);
        assert_eq!(custom.added_background, base.added_background);
    }

    #[test]
    fn omitted_extends_defaults_to_critique() {
        let settings = parse_config("[themes.mine]\nforeground = '#abcdef'").unwrap();
        let custom = &settings.themes[10].palette;
        assert_eq!(custom.foreground, Color::Rgb(0xab, 0xcd, 0xef));
        assert_eq!(custom.background, settings.themes[0].palette.background);
    }

    #[test]
    fn every_palette_field_is_overridable() {
        let fields = [
            "background",
            "foreground",
            "header",
            "hunk",
            "border",
            "muted",
            "gutter",
            "gutter_foreground",
            "added_background",
            "removed_background",
            "added_word_background",
            "removed_word_background",
            "added_gutter",
            "removed_gutter",
            "added_foreground",
            "removed_foreground",
            "selection",
            "keyword",
            "string",
            "number",
            "comment",
            "function",
            "type_name",
            "variable",
        ];
        let mut text = String::from("[themes.mine]\n");
        for field in fields {
            text.push_str(&format!("{field} = '#123aBC'\n"));
        }
        let settings = parse_config(&text).unwrap();
        let p = &settings.themes[10].palette;
        for actual in [
            p.background,
            p.foreground,
            p.header,
            p.hunk,
            p.border,
            p.muted,
            p.gutter,
            p.gutter_foreground,
            p.added_background,
            p.removed_background,
            p.added_word_background,
            p.removed_word_background,
            p.added_gutter,
            p.removed_gutter,
            p.added_foreground,
            p.removed_foreground,
            p.selection,
            p.keyword,
            p.string,
            p.number,
            p.comment,
            p.function,
            p.type_name,
            p.variable,
        ] {
            assert_eq!(actual, Color::Rgb(0x12, 0x3a, 0xbc));
        }
    }

    #[test]
    fn strict_colors_do_not_accept_shorthand_names_alpha_or_unicode() {
        for value in [
            "red",
            "fff",
            "#fff",
            "#12345678",
            "#12g456",
            " #123456",
            "#123456 ",
            "#１２３",
            "",
            "#",
        ] {
            assert!(parse_color(value).is_err(), "accepted {value:?}");
        }
        assert_eq!(parse_color("#00aAFF").unwrap(), Color::Rgb(0, 0xaa, 0xff));
    }

    #[test]
    fn malformed_or_unknown_configuration_is_rejected() {
        for (text, expected) in [
            ("not toml", "invalid TOML"),
            ("theme = 7", "invalid TOML"),
            ("theme = 'missing'", "unknown theme"),
            ("view = 'diagonal'", "unknown view"),
            ("theem = 'nord'", "unknown field"),
            ("[themes.mine]\nbackgrond = '#000000'", "unknown field"),
            ("[themes.mine]\nkeyword = 7", "invalid TOML"),
            ("[themes.mine]\nkeyword = '#xyzxyz'", "color keyword"),
            ("[themes.mine]\nextends = 'missing'", "not a built-in"),
            ("[themes.critique]", "cannot override"),
            ("[themes.'GitHub Dark']", "cannot override"),
            (
                "[themes.'my theme']\n[themes.my_theme]",
                "duplicate custom theme",
            ),
            ("[themes.'']", "invalid custom theme name"),
            ("[themes.'---']", "invalid custom theme name"),
            (
                "[themes.'a']\n[themes.'b']\nextends = 'a'",
                "built-ins only",
            ),
            ("theme = 'nord'\ntheme = 'gruvbox'", "invalid TOML"),
            (
                "[themes.mine]\nkeyword = '#000000'\nkeyword = '#ffffff'",
                "invalid TOML",
            ),
        ] {
            let error = parse_config(text).unwrap_err();
            assert!(error.contains(expected), "{text:?}: {error}");
        }
    }

    #[test]
    fn explicit_missing_file_errors_but_missing_default_is_fine() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.toml");
        assert!(
            load_config(Some(&missing))
                .unwrap_err()
                .contains("could not read")
        );
        assert_eq!(load_path(&missing, false).unwrap().theme, "critique");
    }

    #[test]
    fn explicit_files_include_their_path_in_errors() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        std::fs::write(&path, "theme = 'dracula'\n").unwrap();
        assert_eq!(load_config(Some(&path)).unwrap().theme, "dracula");
        std::fs::write(&path, "bogus = true\n").unwrap();
        let error = load_config(Some(&path)).unwrap_err();
        assert!(error.contains(path.to_str().unwrap()));
        assert!(error.contains("unknown field"));
    }
}
