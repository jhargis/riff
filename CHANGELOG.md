# Changelog

## 0.1.0 (unreleased)

Initial version.

- Read-only viewing of unstaged/staged Git changes, commits, file pairs, and
  UTF-8 unified/Git patches from stdin.
- Split/unified layouts with automatic width selection, aligned change rows,
  line numbers, and file/hunk metadata.
- Syntax highlighting, including TypeScript/TSX, ten built-in themes, a live
  theme picker, and custom TOML palettes.
- Default-on word-level backgrounds, preserving syntax foregrounds and subtle
  line tints in both layouts and ANSI print. Whole `30`/`60` tokens are marked
  in `timeout = 30` → `timeout = 60`, not just the first digits. Critique uses
  restrained red/green contrast; github-dark uses red/blue. All ten built-ins
  and custom palettes expose added/removed word backgrounds.
- `i` saves only `word_diff`; mutually exclusive `--word-diff` / `--no-word-diff`
  override config for one run. Plain output is byte-identical in either mode.
- Grapheme-safe token edits pair equal-sized replacement runs by ordinal;
  pure, unequal, shifted-exact, or unrelated runs retain line styling. Limits
  are 32 pairs, 65,536 raw bytes per run, 4,096 raw/sanitized bytes and 512 tokens
  per line, and 1,048,576 summed squared combined token counts per run. Any
  limit or unrelated pair falls back for the whole run without truncation.
- File/hunk navigation, live literal search, Unicode-aware horizontal scrolling,
  and optional word wrapping.
- `Ctrl-P` opens a file picker with per-file addition/removal counts, live fuzzy
  filtering with terms in any order, arrow selection, and Enter navigation.
- Manual reload and debounced native filesystem watching, with best-effort
  reading/search position preservation and recoverable reload errors.
- Selective persistence of accepted theme, layout, wrap, word-diff, and watch preferences
  at native OS configuration locations.
- Numbered plain-text/ANSI output, explicit color policy, visible terminal
  controls, and terminal cleanup on exit.
- MIT licensing for original code, with bundled grammar notices preserved
  separately. Token diffing adds Similar 3.2.1 (Apache-2.0; license included)
  and reuses locked unicode-segmentation 1.13.3 (MIT OR Apache-2.0).

Runtime verification is Linux-only. See [usage limits](docs/usage.md).
