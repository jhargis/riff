# Configuration and themes

[README](../README.md) · [Usage](usage.md) · [Example config](../examples/config.toml)

## Configuration location

Riff uses the native per-user configuration location:

- Linux/other Unix: absolute, nonempty `$XDG_CONFIG_HOME/riff/config.toml`,
  otherwise `~/.config/riff/config.toml`. Relative XDG paths are ignored.
- macOS: `~/Library/Application Support/riff/config.toml`.
- Windows: roaming AppData, normally `%APPDATA%\riff\config.toml`.

A missing default is fine. It is created only after an accepted interactive
change, never on startup, help, listing, print, CLI overrides, or quit alone.
`--config PATH` selects both reading and persistence; an explicit missing file
is an error.

Saves preserve comments, custom palettes, and other settings. Malformed existing
configuration is never replaced. Atomic saves follow file symlinks (up to 40
links), preserving links and target permissions. Save errors appear in the TUI
without closing it or discarding the live choice.

Configuration is strictly validated, including when listing themes. Unknown
settings, invalid colors/types, unknown themes, and built-in name overrides are
errors. `wrap`, `word_diff`, and `watch` must be booleans.

## Settings and precedence

| Setting | Highest to lowest precedence |
| --- | --- |
| Theme | `--theme NAME` → `RIFF_THEME` → config `theme` → `critique` |
| Layout | `--view auto\|split\|unified` → config `view` → `auto` |
| Word wrap | `--wrap` / `--no-wrap` → config `wrap` → `false` |
| Word highlighting | `--word-diff` / `--no-word-diff` → config `word_diff` → `true` |
| Watch | `--watch` / `--no-watch` → config `watch` → `false` |

Auto layout splits at 100 columns. The wrap flags are mutually exclusive, as
are the watch and word-diff flag pairs. Wrapping affects only the TUI, not printed output. Watching
is available only for interactive Git/file sources.

Theme preview is immediate; Enter accepts and saves only the theme. Esc or
`q` cancels without saving. Interactive `v` saves only layout; `w` saves only
wrapping; `i` saves only word highlighting; an acknowledged `ctrl+w` toggle (without Shift) saves only watch mode.
CLI/environment overrides are one-run choices unless that specific setting is
accepted interactively; unrelated overrides are never written incidentally.

## Built-in themes

- **critique** (default)
- dracula
- catppuccin (Mocha)
- nord
- gruvbox (dark)
- sunset
- one-dark
- monokai
- hacker-term
- github-dark

Theme lookup is case-insensitive and ignores spaces, hyphens, and underscores:
`"GitHub Dark"`, `github-dark`, and `github_dark` resolve identically.

```sh
riff --list-themes
RIFF_THEME=nord riff
riff --theme "one dark" --view split
riff --theme sunset --no-syntax
```

Syntax highlighting uses Syntect and two-face's bat-curated grammar bundle,
including TypeScript (`.ts`) and TSX (`.tsx`), with each palette's semantic
colors and separate multiline parser state for old/new source. Unknown
languages fall back to plain text. `--no-syntax` retains diff colors without
grammar highlighting. See [third-party notices](../THIRD-PARTY-NOTICES.md) for
grammar licenses and riff's load-time TypeScript/TSX correction.

## Custom palettes

```toml
theme = 'my midnight'
view = 'auto'
wrap = false
word_diff = true
watch = false

[themes.'my midnight']
extends = 'critique'
background = '#101018'
keyword = '#ff79c6'
string = '#a5d6ff'
added_background = '#14241a'
removed_background = '#341a22'
added_word_background = '#1b4628'
removed_word_background = '#5e2020'
```

Each custom theme inherits a built-in only; omitted `extends` uses `critique`.
Unspecified colors are inherited. Names cannot override or alias built-ins or
other custom names. Colors must be exactly `#RRGGBB`: no short hex, alpha,
named colors, or terminal escapes.

Supported color keys (exact spelling):

| Surface / role | Keys |
| --- | --- |
| Main text | `background`, `foreground`, `muted` |
| Headers and borders | `header`, `hunk`, `border` |
| Line numbers | `gutter`, `gutter_foreground` |
| Change backgrounds | `added_background`, `removed_background` |
| Changed-token backgrounds | `added_word_background`, `removed_word_background` |
| Change gutters | `added_gutter`, `removed_gutter` |
| Change text | `added_foreground`, `removed_foreground` |
| Search / picker | `selection` |
| Syntax | `keyword`, `string`, `number`, `comment`, `function`, `type_name`, `variable` |

Changed-token roles strengthen only changed words, never their syntax foreground.
They inherit independently of line backgrounds. GitHub-dark uses blue additions;
critique uses restrained green/red. See [pairing and limits](usage.md#changed-word-highlighting).

`header` and `hunk` are background colors with `foreground` text. `gutter`
and `selection` are backgrounds too. See the
[example config](../examples/config.toml) for a complete palette and a second
inheritance example.
