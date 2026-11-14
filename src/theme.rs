//! Built-in, terminal-native color palettes.

use ratatui::style::Color;

#[derive(Clone, Debug)]
pub struct Palette {
    pub background: Color,
    pub foreground: Color,
    pub header: Color,
    pub hunk: Color,
    pub border: Color,
    pub muted: Color,
    pub gutter: Color,
    pub gutter_foreground: Color,
    pub added_background: Color,
    pub removed_background: Color,
    pub added_word_background: Color,
    pub removed_word_background: Color,
    pub added_gutter: Color,
    pub removed_gutter: Color,
    pub added_foreground: Color,
    pub removed_foreground: Color,
    pub selection: Color,
    pub keyword: Color,
    pub string: Color,
    pub number: Color,
    pub comment: Color,
    pub function: Color,
    pub type_name: Color,
    pub variable: Color,
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub name: String,
    pub palette: Palette,
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn named(name: &str, palette: Palette) -> Theme {
    Theme {
        name: name.to_owned(),
        palette,
    }
}

/// All ten built-ins, with the default first. Catppuccin is Mocha;
/// Gruvbox is its dark variant. Every surface has an explicit RGB color.
pub fn builtin_themes() -> Vec<Theme> {
    vec![
        named(
            "critique",
            Palette {
                background: rgb(0x0f0f0f),
                foreground: rgb(0xe6edf3),
                header: rgb(0x000000),
                hunk: rgb(0x191919),
                border: rgb(0x303030),
                muted: rgb(0x666666),
                gutter: rgb(0x050505),
                gutter_foreground: rgb(0x666666),
                added_background: rgb(0x131a14),
                removed_background: rgb(0x2d0d0d),
                added_word_background: rgb(0x1b4628),
                removed_word_background: rgb(0x5e2020),
                added_gutter: rgb(0x003200),
                removed_gutter: rgb(0x3c0000),
                added_foreground: rgb(0xe6edf3),
                removed_foreground: rgb(0xe6edf3),
                selection: rgb(0x283649),
                keyword: rgb(0xff7b72),
                string: rgb(0xa5d6ff),
                number: rgb(0x79c0ff),
                comment: rgb(0x8b949e),
                function: rgb(0xd2a8ff),
                type_name: rgb(0x7ee787),
                variable: rgb(0xffa657),
            },
        ),
        named(
            "dracula",
            Palette {
                background: rgb(0x282a36),
                foreground: rgb(0xf8f8f2),
                header: rgb(0x21222c),
                hunk: rgb(0x303342),
                border: rgb(0x44475a),
                muted: rgb(0x8791b5),
                gutter: rgb(0x222430),
                gutter_foreground: rgb(0x6272a4),
                added_background: rgb(0x20382d),
                removed_background: rgb(0x44262f),
                added_word_background: rgb(0x24543a),
                removed_word_background: rgb(0x682e43),
                added_gutter: rgb(0x24543a),
                removed_gutter: rgb(0x682e43),
                added_foreground: rgb(0xa7f3ba),
                removed_foreground: rgb(0xffb3bd),
                selection: rgb(0x44475a),
                keyword: rgb(0xff79c6),
                string: rgb(0xf1fa8c),
                number: rgb(0xbd93f9),
                comment: rgb(0x6272a4),
                function: rgb(0x50fa7b),
                type_name: rgb(0x8be9fd),
                variable: rgb(0xffb86c),
            },
        ),
        named(
            "catppuccin",
            Palette {
                background: rgb(0x1e1e2e),
                foreground: rgb(0xcdd6f4),
                header: rgb(0x11111b),
                hunk: rgb(0x252537),
                border: rgb(0x45475a),
                muted: rgb(0x7f849c),
                gutter: rgb(0x181825),
                gutter_foreground: rgb(0x6c7086),
                added_background: rgb(0x26352e),
                removed_background: rgb(0x3b2635),
                added_word_background: rgb(0x354c3e),
                removed_word_background: rgb(0x593248),
                added_gutter: rgb(0x354c3e),
                removed_gutter: rgb(0x593248),
                added_foreground: rgb(0xa6e3a1),
                removed_foreground: rgb(0xf38ba8),
                selection: rgb(0x45475a),
                keyword: rgb(0xcba6f7),
                string: rgb(0xa6e3a1),
                number: rgb(0xfab387),
                comment: rgb(0x6c7086),
                function: rgb(0x89b4fa),
                type_name: rgb(0xf9e2af),
                variable: rgb(0xf2cdcd),
            },
        ),
        named(
            "nord",
            Palette {
                background: rgb(0x2e3440),
                foreground: rgb(0xd8dee9),
                header: rgb(0x242933),
                hunk: rgb(0x343e4d),
                border: rgb(0x4c566a),
                muted: rgb(0x8793a8),
                gutter: rgb(0x292e39),
                gutter_foreground: rgb(0x69758b),
                added_background: rgb(0x344238),
                removed_background: rgb(0x48343d),
                added_word_background: rgb(0x425a44),
                removed_word_background: rgb(0x663b48),
                added_gutter: rgb(0x425a44),
                removed_gutter: rgb(0x663b48),
                added_foreground: rgb(0xa3be8c),
                removed_foreground: rgb(0xbf616a),
                selection: rgb(0x434c5e),
                keyword: rgb(0x81a1c1),
                string: rgb(0xa3be8c),
                number: rgb(0xb48ead),
                comment: rgb(0x718096),
                function: rgb(0x88c0d0),
                type_name: rgb(0x8fbcbb),
                variable: rgb(0xd8dee9),
            },
        ),
        named(
            "gruvbox",
            Palette {
                background: rgb(0x282828),
                foreground: rgb(0xebdbb2),
                header: rgb(0x1d2021),
                hunk: rgb(0x32302f),
                border: rgb(0x504945),
                muted: rgb(0x928374),
                gutter: rgb(0x242424),
                gutter_foreground: rgb(0x7c6f64),
                added_background: rgb(0x30361f),
                removed_background: rgb(0x422822),
                added_word_background: rgb(0x424b20),
                removed_word_background: rgb(0x663126),
                added_gutter: rgb(0x424b20),
                removed_gutter: rgb(0x663126),
                added_foreground: rgb(0xb8bb26),
                removed_foreground: rgb(0xfb4934),
                selection: rgb(0x504945),
                keyword: rgb(0xfb4934),
                string: rgb(0xb8bb26),
                number: rgb(0xd3869b),
                comment: rgb(0x928374),
                function: rgb(0xb8bb26),
                type_name: rgb(0xfabd2f),
                variable: rgb(0x83a598),
            },
        ),
        named(
            "sunset",
            Palette {
                background: rgb(0x201724),
                foreground: rgb(0xf2d9cf),
                header: rgb(0x170f1d),
                hunk: rgb(0x322035),
                border: rgb(0x604458),
                muted: rgb(0xa17f91),
                gutter: rgb(0x1b1320),
                gutter_foreground: rgb(0x8a677d),
                added_background: rgb(0x293b30),
                removed_background: rgb(0x482332),
                added_word_background: rgb(0x3c5540),
                removed_word_background: rgb(0x703047),
                added_gutter: rgb(0x3c5540),
                removed_gutter: rgb(0x703047),
                added_foreground: rgb(0xb9d99a),
                removed_foreground: rgb(0xff8d98),
                selection: rgb(0x61405a),
                keyword: rgb(0xf6a35c),
                string: rgb(0xf3ce8b),
                number: rgb(0xec78a8),
                comment: rgb(0xa17f91),
                function: rgb(0xc5a7ee),
                type_name: rgb(0xeda6b1),
                variable: rgb(0xf0b493),
            },
        ),
        named(
            "one-dark",
            Palette {
                background: rgb(0x282c34),
                foreground: rgb(0xabb2bf),
                header: rgb(0x21252b),
                hunk: rgb(0x2c3441),
                border: rgb(0x4b5263),
                muted: rgb(0x7f8796),
                gutter: rgb(0x242830),
                gutter_foreground: rgb(0x5c6370),
                added_background: rgb(0x293b2e),
                removed_background: rgb(0x432b31),
                added_word_background: rgb(0x37553d),
                removed_word_background: rgb(0x663640),
                added_gutter: rgb(0x37553d),
                removed_gutter: rgb(0x663640),
                added_foreground: rgb(0x98c379),
                removed_foreground: rgb(0xe06c75),
                selection: rgb(0x3e4451),
                keyword: rgb(0xc678dd),
                string: rgb(0x98c379),
                number: rgb(0xd19a66),
                comment: rgb(0x5c6370),
                function: rgb(0x61afef),
                type_name: rgb(0xe5c07b),
                variable: rgb(0xe06c75),
            },
        ),
        named(
            "monokai",
            Palette {
                background: rgb(0x272822),
                foreground: rgb(0xf8f8f2),
                header: rgb(0x1e1f1c),
                hunk: rgb(0x34352d),
                border: rgb(0x49483e),
                muted: rgb(0x969581),
                gutter: rgb(0x22231e),
                gutter_foreground: rgb(0x75715e),
                added_background: rgb(0x303b24),
                removed_background: rgb(0x482732),
                added_word_background: rgb(0x465724),
                removed_word_background: rgb(0x70283f),
                added_gutter: rgb(0x465724),
                removed_gutter: rgb(0x70283f),
                added_foreground: rgb(0xa6e22e),
                removed_foreground: rgb(0xf92672),
                selection: rgb(0x49483e),
                keyword: rgb(0xf92672),
                string: rgb(0xe6db74),
                number: rgb(0xae81ff),
                comment: rgb(0x75715e),
                function: rgb(0xa6e22e),
                type_name: rgb(0x66d9ef),
                variable: rgb(0xfd971f),
            },
        ),
        named(
            "hacker-term",
            Palette {
                background: rgb(0x020904),
                foreground: rgb(0x39ff70),
                header: rgb(0x000000),
                hunk: rgb(0x082011),
                border: rgb(0x1e6738),
                muted: rgb(0x4d995f),
                gutter: rgb(0x010603),
                gutter_foreground: rgb(0x33864c),
                added_background: rgb(0x082811),
                removed_background: rgb(0x2d130c),
                added_word_background: rgb(0x0b4c1b),
                removed_word_background: rgb(0x5b2314),
                added_gutter: rgb(0x0b4c1b),
                removed_gutter: rgb(0x5b2314),
                added_foreground: rgb(0x66ff88),
                removed_foreground: rgb(0xff9a59),
                selection: rgb(0x145e2c),
                keyword: rgb(0x00ff66),
                string: rgb(0xafff70),
                number: rgb(0x72ffc1),
                comment: rgb(0x428655),
                function: rgb(0x00ffaa),
                type_name: rgb(0xd0ff9c),
                variable: rgb(0x39d85b),
            },
        ),
        named(
            "github-dark",
            Palette {
                background: rgb(0x0d1117),
                foreground: rgb(0xf0f6fc),
                header: rgb(0x151b23),
                hunk: rgb(0x121d2f),
                border: rgb(0x3d444d),
                muted: rgb(0x9198a1),
                gutter: rgb(0x090d12),
                gutter_foreground: rgb(0x9198a1),
                // This variant intentionally uses blue, not green, additions.
                added_background: rgb(0x13233a),
                removed_background: rgb(0x32161d),
                added_word_background: rgb(0x243e5d),
                removed_word_background: rgb(0x5a2029),
                added_gutter: rgb(0x243e5d),
                removed_gutter: rgb(0x5a2029),
                added_foreground: rgb(0xa5d6ff),
                removed_foreground: rgb(0xffb1af),
                selection: rgb(0x264f78),
                keyword: rgb(0xff8c00),
                string: rgb(0x79c0ff),
                number: rgb(0xa5d6ff),
                comment: rgb(0x9198a1),
                function: rgb(0xd2a8ff),
                type_name: rgb(0x7ee787),
                variable: rgb(0x79c0ff),
            },
        ),
    ]
}

/// Resolve names without case, space, hyphen, or underscore distinctions.
pub fn find_theme(name: &str, themes: &[Theme]) -> Option<usize> {
    let normalized = normalize(name);
    themes
        .iter()
        .position(|theme| normalize(&theme.name) == normalized)
}

fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_builtin_list_and_rgb_surfaces() {
        let themes = builtin_themes();
        let names: Vec<_> = themes.iter().map(|theme| theme.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "critique",
                "dracula",
                "catppuccin",
                "nord",
                "gruvbox",
                "sunset",
                "one-dark",
                "monokai",
                "hacker-term",
                "github-dark",
            ]
        );
        for theme in themes {
            let p = theme.palette;
            for color in [
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
                assert!(matches!(color, Color::Rgb(..)), "{}", theme.name);
            }
        }
    }

    #[test]
    fn reference_critique_colors() {
        let p = builtin_themes().remove(0).palette;
        assert_eq!(p.background, rgb(0x0f0f0f));
        assert_eq!(p.foreground, rgb(0xe6edf3));
        assert_eq!(p.header, rgb(0x000000));
        assert_eq!(p.gutter, rgb(0x050505));
        assert_eq!(p.gutter_foreground, rgb(0x666666));
        assert_eq!(p.muted, rgb(0x666666));
        assert_eq!(p.added_background, rgb(0x131a14));
        assert_eq!(p.removed_background, rgb(0x2d0d0d));
        assert_eq!(p.added_gutter, rgb(0x003200));
        assert_eq!(p.removed_gutter, rgb(0x3c0000));
        assert_eq!(p.comment, rgb(0x8b949e));
        assert_eq!(p.keyword, rgb(0xff7b72));
        assert_eq!(p.string, rgb(0xa5d6ff));
        assert_eq!(p.number, rgb(0x79c0ff));
        assert_eq!(p.function, rgb(0xd2a8ff));
        assert_eq!(p.type_name, rgb(0x7ee787));
        assert_eq!(p.variable, rgb(0xffa657));
    }

    #[test]
    fn github_additions_are_blue() {
        let themes = builtin_themes();
        let p = &themes[find_theme("GitHub Dark", &themes).unwrap()].palette;
        assert_eq!(p.background, rgb(0x0d1117));
        assert_eq!(p.header, rgb(0x151b23));
        assert_eq!(p.hunk, rgb(0x121d2f));
        assert_eq!(p.foreground, rgb(0xf0f6fc));
        assert_eq!(p.border, rgb(0x3d444d));
        assert_eq!(p.muted, rgb(0x9198a1));
        assert_eq!(p.added_background, rgb(0x13233a));
        assert_eq!(p.added_gutter, rgb(0x243e5d));
        assert_eq!(p.keyword, rgb(0xff8c00));
    }

    #[test]
    fn aliases_and_unknown_names() {
        let themes = builtin_themes();
        for aliases in [
            ["One Dark", "one-dark", "ONE_DARK", "onedark"],
            ["Hacker Term", "hacker-term", "HACKER_TERM", "hackerterm"],
            ["GitHub Dark", "github-dark", "GITHUB_DARK", "githubdark"],
        ] {
            let expected = find_theme(aliases[0], &themes).unwrap();
            for alias in aliases {
                assert_eq!(find_theme(alias, &themes), Some(expected));
            }
        }
        assert!(find_theme("missing", &themes).is_none());
        assert!(find_theme("", &themes).is_none());
        assert!(find_theme("critique", &[]).is_none());
    }
}
