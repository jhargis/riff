//! Grammar-aware syntax colors, mapped to each palette's semantic fields.
//!
//! Input text is intentionally retained verbatim. Terminal frontends must
//! sanitize it before rendering, just as they sanitize unhighlighted patches.

use std::path::Path;
use std::sync::OnceLock;

use ratatui::style::Color;
use syntect::easy::HighlightLines;
use syntect::highlighting::{
    Color as SyntaxColor, StyleModifier, Theme as SyntaxTheme, ThemeItem, ThemeSettings,
};
use syntect::parsing::syntax_definition::Pattern;
use syntect::parsing::{Regex, SyntaxReference, SyntaxSet, SyntaxSetBuilder};

use crate::theme::Theme;

pub type HighlightedLine = Vec<(String, Color)>;

static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();

/// Highlight a sequence with shared parser state, retaining line count and
/// exact text (including tabs, Unicode, control characters, and line endings).
///
/// Unknown languages and parser failures degrade to the palette foreground.
/// Call once for each contiguous old/new source sequence, rather than mixing
/// removed and added lines, to preserve meaningful multiline parse state.
pub fn highlight_lines(path: &str, lines: &[String], theme: &Theme) -> Vec<HighlightedLine> {
    if lines.is_empty() {
        return Vec::new();
    }
    let syntaxes = SYNTAXES.get_or_init(load_syntaxes);
    let syntax = syntax_for(syntaxes, path, lines);
    let colors = semantic_theme(theme);
    let mut highlighter = HighlightLines::new(syntax, &colors);

    lines
        .iter()
        .map(|line| {
            // Default grammars expect newlines. Supply one to advance parser
            // state, but clip it out of the returned spans if it was synthetic.
            let terminated;
            let source = if line.ends_with('\n') {
                line.as_str()
            } else {
                terminated = format!("{line}\n");
                &terminated
            };
            match highlighter.highlight_line(source, syntaxes) {
                Ok(spans) => {
                    let mut output: HighlightedLine = Vec::new();
                    let mut remaining = line.len();
                    for (style, text) in spans {
                        let length = remaining.min(text.len());
                        if length == 0 {
                            continue;
                        }
                        let color =
                            Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);
                        let text = &text[..length];
                        if let Some((previous, previous_color)) = output.last_mut()
                            && *previous_color == color
                        {
                            previous.push_str(text);
                        } else {
                            output.push((text.to_owned(), color));
                        }
                        remaining -= length;
                    }
                    // A defensive fallback also protects exact text if a
                    // future grammar/highlighter fails to cover its input.
                    if remaining != 0 {
                        plain(line, theme.palette.foreground)
                    } else {
                        output
                    }
                }
                Err(_) => plain(line, theme.palette.foreground),
            }
        })
        .collect()
}

fn load_syntaxes() -> SyntaxSet {
    let bundled = two_face::syntax::extra_newlines().into_builder();
    let mut builder = SyntaxSetBuilder::new();
    for mut syntax in bundled.syntaxes().iter().cloned() {
        if matches!(
            syntax.scope.to_string().as_str(),
            "source.ts" | "source.tsx"
        ) {
            // The locked bat-0.26.1 grammars predate `satisfies`. Reuse their
            // `as` expression/type transitions, including the exit from a
            // preceding cast, rather than recoloring words after parsing.
            // Do not change `as const`, import aliases, or mapped-type `as`.
            for (name, context) in &mut syntax.contexts {
                if name == "expression-operators"
                    || name == "object-member"
                    || name.starts_with("#anon_expression-operators_")
                    || name.starts_with("#anon_object-member_")
                {
                    for pattern in &mut context.patterns {
                        if let Pattern::Match(pattern) = pattern {
                            let regex = pattern.regex.regex_str();
                            if regex.contains(r"(as)\s+") && !regex.contains("(const)") {
                                pattern.regex = Regex::new(regex.replace("(as)", "(as|satisfies)"));
                            }
                        }
                    }
                }
            }
        }
        // Preserve syntax order and context names so the bundle's already
        // linked Direct context IDs remain valid when the set is rebuilt.
        builder.add(syntax);
    }
    builder.build()
}

fn plain(line: &str, color: Color) -> HighlightedLine {
    vec![(line.to_owned(), color)]
}

fn syntax_for<'a>(syntaxes: &'a SyntaxSet, path: &str, lines: &[String]) -> &'a SyntaxReference {
    let path = Path::new(path);
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let extension = path.extension().and_then(|extension| extension.to_str());
    extension
        .and_then(|extension| {
            syntaxes
                .find_syntax_by_extension(extension)
                .or_else(|| syntaxes.find_syntax_by_extension(&extension.to_ascii_lowercase()))
        })
        .or_else(|| syntaxes.find_syntax_by_extension(filename))
        .or_else(|| syntaxes.find_syntax_by_extension(&filename.to_ascii_lowercase()))
        .or_else(|| syntaxes.find_syntax_by_extension(filename.trim_start_matches('.')))
        .or_else(|| {
            lines.first().and_then(|line| {
                syntaxes.find_syntax_by_first_line(line.trim_end_matches(['\r', '\n']))
            })
        })
        .unwrap_or_else(|| syntaxes.find_syntax_plain_text())
}

fn semantic_theme(theme: &Theme) -> SyntaxTheme {
    let palette = &theme.palette;
    let rules = [
        (
            "variable, entity.name.tag, entity.other.attribute-name",
            palette.variable,
        ),
        ("keyword, storage.type, storage.modifier", palette.keyword),
        ("string", palette.string),
        (
            "constant.numeric, constant.language, constant.character, constant.other",
            palette.number,
        ),
        ("comment, punctuation.definition.comment", palette.comment),
        (
            "entity.name.function, support.function, variable.function",
            palette.function,
        ),
        (
            "entity.name.type, entity.name.class, entity.name.struct, entity.name.enum, entity.name.trait, entity.other.inherited-class, support.type, support.class",
            palette.type_name,
        ),
    ];
    SyntaxTheme {
        name: Some(theme.name.clone()),
        settings: ThemeSettings {
            foreground: Some(syntax_color(palette.foreground)),
            ..ThemeSettings::default()
        },
        scopes: rules
            .into_iter()
            .map(|(scope, color)| ThemeItem {
                scope: scope
                    .parse()
                    .expect("static semantic scope selectors are valid"),
                style: StyleModifier {
                    foreground: Some(syntax_color(color)),
                    ..StyleModifier::default()
                },
            })
            .collect(),
        ..SyntaxTheme::default()
    }
}

fn syntax_color(color: Color) -> SyntaxColor {
    let (r, g, b) = match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(index) => indexed_rgb(index),
        Color::Black => (0, 0, 0),
        Color::Red => (128, 0, 0),
        Color::Green => (0, 128, 0),
        Color::Yellow => (128, 128, 0),
        Color::Blue => (0, 0, 128),
        Color::Magenta => (128, 0, 128),
        Color::Cyan => (0, 128, 128),
        Color::Gray => (192, 192, 192),
        Color::DarkGray => (128, 128, 128),
        Color::LightRed => (255, 0, 0),
        Color::LightGreen => (0, 255, 0),
        Color::LightYellow => (255, 255, 0),
        Color::LightBlue => (0, 0, 255),
        Color::LightMagenta => (255, 0, 255),
        Color::LightCyan => (0, 255, 255),
        Color::White => (255, 255, 255),
        Color::Reset => (230, 237, 243),
    };
    SyntaxColor { r, g, b, a: 255 }
}

fn indexed_rgb(index: u8) -> (u8, u8, u8) {
    const BASE: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (128, 0, 0),
        (0, 128, 0),
        (128, 128, 0),
        (0, 0, 128),
        (128, 0, 128),
        (0, 128, 128),
        (192, 192, 192),
        (128, 128, 128),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (0, 0, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    match index {
        0..=15 => BASE[usize::from(index)],
        16..=231 => {
            let index = index - 16;
            let levels = [0, 95, 135, 175, 215, 255];
            (
                levels[usize::from(index / 36)],
                levels[usize::from((index / 6) % 6)],
                levels[usize::from(index % 6)],
            )
        }
        232..=255 => {
            let gray = 8 + (index - 232) * 10;
            (gray, gray, gray)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{builtin_themes, find_theme};

    fn strings(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| (*line).to_owned()).collect()
    }

    fn joined(line: &HighlightedLine) -> String {
        line.iter().map(|(text, _)| text.as_str()).collect()
    }

    #[test]
    fn rust_has_grammar_aware_semantic_colors() {
        let theme = &builtin_themes()[0];
        let lines = strings(&[
            "fn greet() {",
            "    let n = 42;",
            "    let message = \"hello\"; // a comment",
            "}",
        ]);
        let highlighted = highlight_lines("src/main.rs", &lines, theme);
        assert_eq!(highlighted.len(), lines.len());
        for (actual, expected) in highlighted.iter().zip(&lines) {
            assert_eq!(&joined(actual), expected);
        }
        assert!(
            highlighted[0]
                .iter()
                .any(|(text, color)| text.contains("fn") && *color == theme.palette.keyword)
        );
        assert!(
            highlighted[0]
                .iter()
                .any(|(text, color)| text.contains("greet") && *color == theme.palette.function)
        );
        assert!(
            highlighted[1]
                .iter()
                .any(|(text, color)| text.contains("42") && *color == theme.palette.number)
        );
        assert!(
            highlighted[2]
                .iter()
                .any(|(text, color)| text.contains("hello") && *color == theme.palette.string)
        );
        assert!(
            highlighted[2]
                .iter()
                .any(|(text, color)| text.contains("a comment") && *color == theme.palette.comment)
        );
    }

    fn assert_token(line: &HighlightedLine, token: &str, expected: Color) {
        let text = joined(line);
        assert!(text.contains(token), "fixture contains token");
        for (start, _) in text.match_indices(token) {
            let end = start + token.len();
            let mut offset = 0;
            for (text, color) in line {
                let next = offset + text.len();
                if offset < end && next > start {
                    assert_eq!(*color, expected, "{token:?} in {line:?}");
                }
                offset = next;
            }
        }
    }

    #[test]
    fn typescript_keywords_strings_comments_and_types_use_each_palette() {
        let themes = builtin_themes();
        let lines = strings(&[
            "interface Person { name: string; }",
            "const person: Person = { name: \"雪\" }; // greeting",
            "type People = Array<Person>;",
        ]);
        let mut previous = None;
        for name in ["critique", "dracula"] {
            let theme = &themes[find_theme(name, &themes).unwrap()];
            for path in ["src/person.ts", "src/person.TS"] {
                let output = highlight_lines(path, &lines, theme);
                assert_eq!(output.len(), lines.len());
                for (actual, expected) in output.iter().zip(&lines) {
                    assert_eq!(&joined(actual), expected);
                }
                assert_token(&output[0], "interface", theme.palette.keyword);
                assert_token(&output[0], "Person", theme.palette.type_name);
                assert_token(&output[1], "const", theme.palette.keyword);
                assert_token(&output[1], "雪", theme.palette.string);
                assert_token(&output[1], "greeting", theme.palette.comment);
                assert_token(&output[2], "People", theme.palette.type_name);
            }
            let output = highlight_lines("person.ts", &lines, theme);
            if let Some(previous) = previous {
                assert_ne!(output, previous);
            }
            previous = Some(output);
        }
    }

    #[test]
    fn typescript_inherited_types_use_type_colors() {
        for theme in &builtin_themes()[..2] {
            for path in ["person.ts", "person.tsx"] {
                let lines = strings(&[
                    "interface Person extends Base {}",
                    "class Person extends Base {}",
                ]);
                let output = highlight_lines(path, &lines, theme);
                for (actual, expected) in output.iter().zip(&lines) {
                    assert_eq!(&joined(actual), expected);
                    assert_token(actual, "Base", theme.palette.type_name);
                }
            }
        }
    }

    #[test]
    fn typescript_satisfies_is_a_keyword_in_expressions_and_after_casts() {
        let lines = strings(&[
            "const answer = person satisfies Person;",
            "const answer = person as Person satisfies Person;",
            "const answer = { name: \"雪\" } satisfies Person;",
            "const answer = { person: person satisfies Person };",
            "const answer = { person: person as Person satisfies Person };",
            "const answer = { name: \"雪\" } as const satisfies Person;",
            "const answer = person satisfies Array<Person>;",
        ]);
        for theme in &builtin_themes()[..2] {
            for path in ["person.ts", "person.tsx"] {
                let output = highlight_lines(path, &lines, theme);
                for (actual, expected) in output.iter().zip(&lines) {
                    assert_eq!(&joined(actual), expected);
                    assert_token(actual, "satisfies", theme.palette.keyword);
                    assert_token(actual, "Person", theme.palette.type_name);
                }
            }
        }
    }

    #[test]
    fn typescript_satisfies_fix_leaves_nonoperators_and_other_languages_alone() {
        let cases = [
            ("const satisfies = 1;", "satisfies", "variable"),
            ("person.satisfies;", "satisfies", "variable"),
            ("person?.satisfies;", "satisfies", "variable"),
            (
                "const person = { satisfies: true };",
                "satisfies",
                "foreground",
            ),
            (
                "const answer = satisfiesMore + 1;",
                "satisfiesMore",
                "variable",
            ),
            ("const answer = $satisfies + 1;", "$satisfies", "variable"),
            ("const answer = _satisfies + 1;", "_satisfies", "variable"),
            ("const answer = 雪satisfies + 1;", "雪satisfies", "variable"),
            ("const answer = satisfiesλ + 1;", "satisfiesλ", "variable"),
            ("const text = 'satisfies Person';", "satisfies", "string"),
            ("const text = `satisfies Person`;", "satisfies", "string"),
            ("const pattern = /satisfies Person/;", "satisfies", "string"),
            ("// satisfies Person", "satisfies", "comment"),
            ("/* satisfies Person */", "satisfies", "comment"),
        ];
        for theme in &builtin_themes()[..2] {
            for path in ["person.ts", "person.tsx"] {
                for (source, token, role) in cases {
                    let output = highlight_lines(path, &strings(&[source]), theme);
                    assert_eq!(joined(&output[0]), source);
                    let expected = match role {
                        "string" => theme.palette.string,
                        "comment" => theme.palette.comment,
                        "foreground" => theme.palette.foreground,
                        _ => theme.palette.variable,
                    };
                    assert_token(&output[0], token, expected);
                }
            }
            let source = strings(&["const answer = person satisfies Person;"]);
            let js = highlight_lines("person.js", &source, theme);
            assert_token(&js[0], "satisfies", theme.palette.variable);
            let unknown = highlight_lines("person.riff-unknown", &source, theme);
            assert_token(&unknown[0], "satisfies", theme.palette.foreground);
        }
    }

    #[test]
    fn tsx_tags_attributes_and_embedded_expressions_use_each_palette() {
        let themes = builtin_themes();
        let lines = strings(&[
            "const view = <section title=\"hello\">",
            "  {ready ? <span>{\"雪\"}</span> : null}",
            "</section>; // rendered",
        ]);
        let mut previous = None;
        for name in ["critique", "dracula"] {
            let theme = &themes[find_theme(name, &themes).unwrap()];
            for path in ["src/view.tsx", "src/view.TSX"] {
                let output = highlight_lines(path, &lines, theme);
                assert_eq!(output.len(), lines.len());
                for (actual, expected) in output.iter().zip(&lines) {
                    assert_eq!(&joined(actual), expected);
                }
                assert_token(&output[0], "const", theme.palette.keyword);
                assert_token(&output[0], "section", theme.palette.variable);
                assert_token(&output[0], "title", theme.palette.variable);
                assert_token(&output[0], "hello", theme.palette.string);
                assert_token(&output[1], "ready", theme.palette.variable);
                assert_token(&output[1], "span", theme.palette.variable);
                assert_token(&output[1], "雪", theme.palette.string);
                assert_token(&output[1], "null", theme.palette.number);
                assert_token(&output[2], "section", theme.palette.variable);
                assert_token(&output[2], "rendered", theme.palette.comment);
            }
            let output = highlight_lines("view.tsx", &lines, theme);
            if let Some(previous) = previous {
                assert_ne!(output, previous);
            }
            previous = Some(output);
        }
    }

    #[test]
    fn typescript_and_tsx_multiline_state_resets_between_calls() {
        let theme = &builtin_themes()[0];
        for path in ["file.ts", "file.tsx"] {
            let old = highlight_lines(path, &strings(&["/* open", "const value = 42;"]), theme);
            assert_token(&old[1], "const value = 42;", theme.palette.comment);
            let new = highlight_lines(
                path,
                &strings(&[
                    "// closed",
                    "const value = 42;",
                    "/* open",
                    "end */ const next = 7;",
                ]),
                theme,
            );
            assert_token(&new[1], "const", theme.palette.keyword);
            assert_token(&new[3], "end", theme.palette.comment);
            assert_token(&new[3], "const", theme.palette.keyword);
            let template = highlight_lines(
                path,
                &strings(&["const message = `hello", "雪 ${42}`;"]),
                theme,
            );
            assert_token(&template[1], "雪", theme.palette.string);
            assert_token(&template[1], "42", theme.palette.number);
        }
    }

    #[test]
    fn multiline_comments_and_line_comments_keep_correct_state() {
        let theme = &builtin_themes()[0];
        let lines = strings(&[
            "/* open",
            "still a comment",
            "*/ fn after() {}",
            "// only this line",
            "fn next() {}",
        ]);
        let highlighted = highlight_lines("file.rs", &lines, theme);
        assert!(
            highlighted[1]
                .iter()
                .all(|(_, color)| *color == theme.palette.comment)
        );
        for index in [2, 4] {
            assert!(
                highlighted[index]
                    .iter()
                    .any(|(text, color)| text.contains("fn") && *color == theme.palette.keyword)
            );
        }
        for (actual, expected) in highlighted.iter().zip(&lines) {
            assert_eq!(&joined(actual), expected);
        }
    }

    #[test]
    fn theme_colors_are_not_a_fixed_syntect_palette() {
        let themes = builtin_themes();
        let lines = strings(&["fn hello() { let value = 17; }"]);
        let mut prior = None;
        for name in ["critique", "dracula", "github dark"] {
            let theme = &themes[find_theme(name, &themes).unwrap()];
            let output = highlight_lines("a.rs", &lines, theme);
            assert!(
                output[0]
                    .iter()
                    .any(|(text, color)| text.contains("fn") && *color == theme.palette.keyword)
            );
            if let Some(previous) = prior {
                assert_ne!(output[0], previous);
            }
            prior = Some(output[0].clone());
        }
    }

    #[test]
    fn custom_semantic_colors_are_honored() {
        let mut theme = builtin_themes().remove(0);
        theme.palette.keyword = Color::Rgb(1, 2, 3);
        theme.palette.function = Color::Rgb(4, 5, 6);
        theme.palette.foreground = Color::Rgb(7, 8, 9);
        let output = highlight_lines("file.rs", &strings(&["fn mine() {}"]), &theme);
        assert!(
            output[0]
                .iter()
                .any(|(text, color)| text.contains("fn") && *color == Color::Rgb(1, 2, 3))
        );
        assert!(
            output[0]
                .iter()
                .any(|(text, color)| text.contains("mine") && *color == Color::Rgb(4, 5, 6))
        );
        let plain = highlight_lines("unknown.riff-unknown", &strings(&["plain words"]), &theme);
        assert!(
            plain[0]
                .iter()
                .all(|(_, color)| *color == Color::Rgb(7, 8, 9))
        );
    }

    #[test]
    fn exact_text_including_unicode_tabs_line_endings_and_controls() {
        let lines = strings(&[
            "let café = \"雪 🦀\";\r\n",
            "\t// λ\n",
            "",
            "\u{1b}[31m\u{0}\u{85}",
            "trailing spaces   ",
        ]);
        for path in [
            "source.rs",
            "source.ts",
            "source.tsx",
            "unknown.riff-unknown",
        ] {
            let result = highlight_lines(path, &lines, &builtin_themes()[0]);
            assert_eq!(result.len(), lines.len());
            for (actual, expected) in result.iter().zip(&lines) {
                assert_eq!(&joined(actual), expected);
            }
        }
    }

    #[test]
    fn unknown_syntax_is_plaintext_and_empty_input_is_empty() {
        let lines = strings(&[
            "const view: Person = <span>{\"雪\"}</span>; // not code",
            "",
        ]);
        for theme in &builtin_themes()[..2] {
            let result = highlight_lines("file.unrecognized-riff", &lines, theme);
            assert_eq!(result.len(), lines.len());
            for (actual, expected) in result.iter().zip(&lines) {
                assert_eq!(&joined(actual), expected);
                assert!(
                    actual
                        .iter()
                        .all(|(_, color)| *color == theme.palette.foreground)
                );
            }
            assert!(highlight_lines("file.tsx", &[], theme).is_empty());
        }
    }

    #[test]
    fn extensionless_shebang_uses_a_grammar() {
        let theme = &builtin_themes()[0];
        let lines = strings(&["#!/usr/bin/env python3", "def greet():", "    return 42"]);
        let result = highlight_lines("script", &lines, theme);
        assert_eq!(result.len(), lines.len());
        assert!(
            result[1]
                .iter()
                .any(|(text, color)| text.contains("def") && *color == theme.palette.keyword)
        );
    }

    #[test]
    fn grammar_rebuild_keeps_non_typescript_definitions_unchanged() {
        let original = two_face::syntax::extra_newlines().into_builder();
        let patched = load_syntaxes().into_builder();
        assert_eq!(original.syntaxes().len(), patched.syntaxes().len());
        for (before, after) in original.syntaxes().iter().zip(patched.syntaxes()) {
            assert_eq!(before.name, after.name);
            if !matches!(
                before.scope.to_string().as_str(),
                "source.ts" | "source.tsx"
            ) {
                assert_eq!(before, after, "{} grammar changed", before.name);
            }
        }
    }

    #[test]
    fn syntax_set_is_cached_across_calls() {
        let theme = &builtin_themes()[0];
        highlight_lines("a.rs", &strings(&["fn a() {}"]), theme);
        let first = SYNTAXES.get().unwrap() as *const SyntaxSet;
        for path in ["a.js", "a.ts", "a.tsx"] {
            highlight_lines(path, &strings(&["const a = 1;"]), theme);
            assert_eq!(first, SYNTAXES.get().unwrap() as *const SyntaxSet);
        }
    }
}
