# Usage reference

[README](../README.md) · [Configuration](configuration.md)

## Sources and paths

`riff` shows unstaged tracked Git changes when stdin is a terminal. The source
options `--staged`, `--commit REF`, positional `REF_OR_PATH`, `--files OLD NEW`,
and `--stdin` are mutually exclusive.

A single bare file or directory filters unstaged changes, including tracked
paths deleted from disk. Bare paths are literal, including wildcard characters
and Git pathspec-looking names: `riff 'literal[1].txt'` selects only that filename.
Paths are relative to the invocation directory. Other bare arguments select a
commit, branch, tag, or commit-resolving revision such as `HEAD~1`; branch names
containing `/` are supported. This is `git show`, not a two-revision comparison.
For merge commits, Git sources show the first-parent diff.

If a name identifies both a path and a revision, riff reports an ambiguity:
use `riff -- HEAD` for the path or `riff --commit HEAD` for the revision.

For multiple paths, staged filters, or commit filters, put Git path filters
after `--`:

```sh
riff -- src tests
riff --staged -- src
riff HEAD -- src
riff --commit HEAD -- src/main.rs
```

Multiple bare paths and `riff --staged PATH` are not supported. Filters after
`--` retain Git pathspec syntax; for a literal filter use, for example,
`riff -- ':(literal)literal[1].txt'`. Filters cannot be used with `--files` or
`--stdin`.

`--files OLD NEW` compares two files using `git diff --no-index`, including
outside a repository. A file named `-` is a literal filesystem operand in this
mode, not stdin. Differences are a successful result.

### Stdin selection

Without an explicit source, **non-terminal stdin is automatically a patch**.
An explicit Git/file source takes precedence over piped stdin, so
`producer | riff --staged` still shows staged changes. Path filters explicitly
select Git. To request unstaged Git changes from a noninteractive caller,
provide a filter such as `riff -- .`.

`--stdin` explicitly reads until EOF, even from terminal stdin. A patch can open
the TUI when stdout is a terminal, but the patch is read completely first and
cannot be reloaded. Reload/watch keys explain the limitation without changing
the saved watch preference.

### One-shot output

Non-terminal stdout always prints and never opens the TUI. `--print` forces the
same numbered unified output on a terminal; `--view` affects only the TUI.
Both old and new line numbers, file/hunk metadata, and visible `+`/`-` markers
are included. This is a review format, not a raw patch for `git apply`.

```sh
riff --print --staged
riff --stdin --print --color always < change.patch
riff --staged --color never > review.txt
```

`--color auto|always|never` controls printed ANSI colors. Auto enables colors
only for terminal stdout and honors `NO_COLOR`; explicit `always` overrides it.
TUI colors use the selected palette regardless of this option. Wrapping does
not alter printed output: it always retains complete lines. ANSI printing includes
changed-token backgrounds by default; `--no-word-diff` disables them. With
`--color never`, readable output is byte-identical regardless of word-diff mode
and contains no ANSI codes.

No differences is a successful `No differences.` result; actual differences
also exit **0**. Errors exit nonzero. Closing an output pipe early is not an
error. An empty initial diff prints and exits unless interactive watch is enabled;
becoming empty after reload never closes an already running viewer.

### Supported patches

Standard unified and Git patches must have UTF-8 contents and filenames.
Rename/mode/binary metadata is displayed; binary bodies are not decoded.
Combined merge patches are rejected rather than flattened. Highlighting across
omitted hunk context is best effort because the missing source is unavailable.

Riff does not request editing, staging, or Git write operations. Git queries
disable optional index-stat refresh and fsmonitor, along with external diff and
textconv. They are not sandboxed: configured clean/process filters may still
execute with side effects or network access. To avoid running Git, supply an
already obtained patch through stdin.

## Interactive controls

Auto layout splits at **100 terminal columns**; narrower terminals use unified
rows. Split rows align ordered removal/addition runs while retaining blank
context, old/new numbers, and no-final-newline notes.

| Keys | Action |
| --- | --- |
| `j/k`, `↓/↑`, mouse wheel | Scroll |
| `PgDn/PgUp`, `Space/Shift-Space` | Full page down / up |
| `Ctrl-D/Ctrl-U` | Half page down / up |
| `g/G`, `Home/End` | First / last row |
| `h/l`, `←/→`, horizontal wheel | Horizontal scroll, four columns |
| `0` | Reset horizontal scroll |
| `n/p` | Next / previous hunk, wrapping |
| `]/[` | Next / previous file, wrapping |
| `Ctrl-P` | Open the fuzzy file picker with addition/removal counts |
| File picker: `↓/↑`, `Home/End` | Next / previous match, wrapping; first / last match |
| File picker: `Enter` / `Esc` / `Ctrl-U` | Open selected file / cancel / clear filter |
| `/` | Start live, case-insensitive literal search |
| `Enter/Shift-Enter`, `F3/Shift-F3` | Next / previous search match, wrapping |
| `N` | Previous search match |
| `Esc` | Clear accepted search |
| `t` | Open live-preview theme picker |
| Theme picker: `j/k`, `↓/↑`, `PgDn/PgUp`, `g/G`, `Home/End` | Preview themes |
| Theme picker: `Enter` / `Esc` or `q` | Accept and save / restore original theme |
| Search entry: `Enter` / `Esc` / `Ctrl-U` | Accept / restore previous search / clear entry |
| `v` | Toggle split / unified and save layout |
| `w` | Toggle code word wrap and save wrapping |
| `i` | Toggle changed-word backgrounds and save only `word_diff` |
| `ctrl+w` (no Shift) | Toggle automatic reload and save watch mode |
| `r` / `Ctrl-R` | Reload the original Git/file source asynchronously |
| `?` | Help; `?`, `Esc`, or `q` closes help |
| `q` / `Ctrl-C` | Quit; Ctrl-C works from dialogs too |

Search counts matching logical code lines and metadata rows, not occurrences
or wrap fragments. Split context copies count once; distinct changed
counterparts can each match. Queries can span wrap boundaries, and late matches
are revealed. Hunk navigation remains `n/p` while a search is accepted.

### File picker

Press `Ctrl-P` to list the files in the current diff. Each entry shows its total
`+` additions and `-` removals across all hunks. Deleted files use their old path;
renamed files use their new path. Binary and metadata-only changes show `+0 -0`
when the patch has no added or removed text lines.

Type to filter the list immediately. Matching is case-insensitive and fuzzy:
letters in each term must occur in order anywhere in the path, but do not need
to be adjacent. For example, `sapp` matches `src/app.rs`. Space-separated terms
must all match, in any term order: `app src` also matches `src/app.rs`.
This is not glob or regular-expression matching. Results stay in diff order.

Use `↑/↓` to select a match. Use `Home/End` to select the first or last match.
Press `Enter` to close the picker and navigate to the selected file.
Press `Esc` to cancel without moving the diff. `Backspace` removes the last
character, and `Ctrl-U` clears the filter. With no matches, Enter leaves the
picker open. Other printable keys, including `q`, enter filter text.

Opening and filtering do not change the accepted code search, saved preferences,
or diff position. File selection resets horizontal scrolling to show the header.
Reload results wait until the picker closes, as they do for the other dialogs.

### Scrolling and wrapping

Code is clipped by default. Horizontal scrolling keeps gutters fixed and clips
at Unicode grapheme/column boundaries, padding partially visible wide glyphs.

`w` or `--wrap` wraps code at word boundaries and hard-breaks long tokens while
retaining whitespace, syntax colors, and change backgrounds. Split counterparts
wrap independently and pad the shorter side. Line numbers and +/- appear only
on the first fragment. Enabling wrap retains the horizontally scrolled reading
position; resize roundtrips retain the logical continuation unless you navigate
elsewhere. Wrapping disables horizontal offset. The status shows `clip x=…` or
`wrap:on`. File/hunk/metadata labels remain clipped. With no code area, text is
retained until the terminal is widened.

Tabs display as four spaces. Terminal C0/C1 controls, including ESC, are made
visible rather than replayed; original patch text is retained internally.

### Changed-word highlighting

Enabled by default in both layouts and ANSI output, independently of syntax
highlighting. `timeout = 30` → `timeout = 60` emphasizes the entire `30`/`60`
tokens, leaving `timeout`, equals, and spacing on the original line tint.
Only backgrounds change; syntax foregrounds are retained. Search selection
still marks the row while token backgrounds remain visible. Wrapping and
horizontal clipping retain token styles and search uses original logical lines.

`i` toggles without rereading Git or resetting reading/search/wrap position.
It saves only the strict boolean `word_diff`. Mutually exclusive
`--word-diff` / `--no-word-diff` override config for one run, never at startup.
Status shows `words:on/off`. Help/search/theme dialogs do not toggle it.

Pairing is intentionally conservative, not a source-code alignment engine:
within each hunk, contiguous removal/addition runs with equal code-line counts
pair by ordinal in both views. EOF notes are excluded. Unequal counts, shifted
exact lines, or any unrelated pair fall back to existing line-level colors for
the whole run. No pairing crosses context, hunks, or files. Pure added/removed
runs are not marked word-by-word.

Tokens group identifiers/numbers (including combining marks), whitespace, and
operator sequences; other punctuation and emoji use whole graphemes, including
ZWJ sequences. Matching words or non-punctuation Unicode symbols must cover at
least half the smaller line's substantive character count, with at least one
such match; whitespace-only pairs are allowed. Punctuation/indentation alone
cannot make unrelated code appear aligned. Whitespace changes are highlighted
as whole whitespace tokens; tabs are compared after four-space expansion.
Terminal controls are compared only after visible sanitization.

Named limits in `src/intraline.rs`: at most 32 pairs, 65,536 raw bytes per run,
4,096 bytes per raw or sanitized line, 512 tokens per line, and 1,048,576 summed
squared combined token counts per run. Exceeding any limit falls back for the
whole run, never truncating text. Token calculations happen at cache rebuild,
not per frame. Ambiguous or substantial rewrites can therefore legitimately
have line tints without word backgrounds.

### Keyboard and terminal behavior

Riff requests Kitty/CSI-u keyboard enhancements so supporting terminals can
report Shift-Space distinctly. Legacy terminals that send the same byte for
Space and Shift-Space cannot distinguish them; use PgUp or Ctrl-U instead.
Search entry still inserts a space, and Shift-Space does not navigate
pickers/help.

Unsupported Alt/Super/Hyper/Meta chords are ignored, including in dialogs;
they do not trigger bare-key preferences. Enhanced-input Caps Lock is honored
for ASCII letters, including search text. Shifted punctuation follows the
terminal's keyboard layout.

Terminal guards restore raw mode, alternate screen, mouse mode, keyboard flags,
and cursor after normal exits, setup/drawing errors, and Rust panics. On Unix,
external SIGINT, SIGTERM, and SIGHUP request graceful exit (status 0) through the
event loop, including from dialogs. Uncatchable termination such as SIGKILL
cannot be cleaned up. Runtime verification is Linux-only, not a full terminal
emulator or OS matrix.

## Watch and reload

`ctrl+w` (lowercase w, without Shift) toggles automatic refresh and remembers
watch mode across launches. `ctrl+shift+w` may close the terminal window and is
not a riff shortcut.
`r` or `Ctrl-R` reloads manually even with watching off; `w` still toggles wrap.
These controls apply outside dialogs. Mutually exclusive `--watch` /
`--no-watch` override config `watch` (default `false`) for one run without
saving.

The header shows watch on/off/unavailable and loading. Errors appear in status
without discarding the last valid diff. Later edits or manual reload can recover
without turning watch off. An initially empty watched diff stays open.

Reload reuses the original resolved Git query/file pair, invocation directory,
and path filters. A deleted positional path is not reinterpreted as a revision.
It preserves reading position, search selection, wrap continuation, horizontal
offset, theme, and layout where possible. Missing content falls back to a nearby
surviving line/file; indistinguishable duplicates can make anchoring best effort.
Results arriving during search, help, file selection, or theme preview wait
until the dialog closes; only the latest good result is applied.

Watching uses native OS filesystem events with a 150 ms debounce, not periodic
Git polling. File-pair parents are watched for atomic replacement/recreation;
Git watches include the repository and relevant Git/index/HEAD/ref directories.
Unrelated writes may cause unchanged checks; read/access events are ignored.
Watcher setup failures leave manual reload available. Filesystems with unreliable
events may require manual reload; a watcher overflow/fatal-stream failure has not
been forced in runtime validation.

Saved watch is ignored for stdin patches and plain/ANSI one-shot output.
Explicit `--watch` with `--print`, non-terminal stdout, or stdin is rejected
before consuming stdin. A piped-patch TUI never rereads stdin.

Reload loads and parses off the UI thread, serializing and coalescing requests.
A running Git/configured helper cannot be cancelled. Quit does not wait for that
worker, but a stuck helper may outlive the viewer until it returns. Initial
loading happens before the TUI opens and can also block on a helper.
