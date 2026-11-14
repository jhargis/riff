//! Unified diff parsing. Patch text is kept verbatim; sanitize only at the UI boundary.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
    NoNewline,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
    pub old_number: Option<u32>,
    pub new_number: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Hunk {
    pub header: String,
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug)]
pub struct FileDiff {
    pub old_path: String,
    pub new_path: String,
    pub metadata: Vec<String>,
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    pub fn display_path(&self) -> &str {
        if self.new_path.is_empty() || self.new_path == "/dev/null" {
            &self.old_path
        } else {
            &self.new_path
        }
    }

    pub fn additions(&self) -> usize {
        self.hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|line| line.kind == LineKind::Added)
            .count()
    }

    pub fn deletions(&self) -> usize {
        self.hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|line| line.kind == LineKind::Removed)
            .count()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Diff {
    pub files: Vec<FileDiff>,
}

/// Make terminal control characters visible, without changing printable Unicode.
/// Tabs always expand to four spaces (rather than to tab stops).
pub fn sanitize(text: &str) -> String {
    let mut safe = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\t' => safe.push_str("    "),
            '\0'..='\u{1f}' => {
                safe.push('^');
                safe.push(char::from_u32(ch as u32 + 64).unwrap());
            }
            '\u{7f}' => safe.push_str("^?"),
            '\u{80}'..='\u{9f}' => {
                use std::fmt::Write;
                let _ = write!(safe, "\\u{{{:04x}}}", ch as u32);
            }
            _ => safe.push(ch),
        }
    }
    safe
}

type Lines<'a> = std::iter::Peekable<std::iter::Enumerate<std::str::SplitTerminator<'a, char>>>;

struct FileBuilder {
    file: FileDiff,
    headers_seen: bool,
    binary_payload: bool,
    previous_ends: Option<(u64, u64)>,
    eof: (bool, bool),
    unresolved_paths: Option<(usize, u8)>,
    git_header: Option<String>,
}

impl FileBuilder {
    fn new(old_path: String, new_path: String) -> Self {
        Self {
            file: FileDiff {
                old_path,
                new_path,
                metadata: Vec::new(),
                hunks: Vec::new(),
            },
            headers_seen: false,
            binary_payload: false,
            previous_ends: None,
            eof: (false, false),
            unresolved_paths: None,
            git_header: None,
        }
    }

    fn resolve_path(&mut self, side: u8) {
        if let Some((_, resolved)) = &mut self.unresolved_paths {
            *resolved |= side;
            if *resolved == 3 {
                self.unresolved_paths = None;
            }
        }
    }
}

fn error(line: usize, message: impl std::fmt::Display) -> String {
    format!("Patch line {}: {}", line + 1, message)
}

fn finish(current: &mut Option<FileBuilder>, diff: &mut Diff) -> Result<(), String> {
    if let Some(builder) = current.take() {
        if let Some((line, _)) = builder.unresolved_paths {
            return Err(error(
                line,
                "ambiguous unquoted diff --git paths; expected ---/+++ headers, rename/copy records, or a binary summary to resolve them",
            ));
        }
        diff.files.push(builder.file);
    }
    Ok(())
}

fn is_combined(line: &str) -> bool {
    line == "diff --cc"
        || line.starts_with("diff --cc ")
        || line == "diff --combined"
        || line.starts_with("diff --combined ")
        || line.starts_with("@@@")
}

/// Parse Git patches and ordinary unified diffs. Combined merge diffs are not
/// unified diffs and are deliberately rejected. Binary bodies are summarized,
/// not decoded; the `GIT binary patch` metadata entry records their presence.
pub fn parse_patch(text: &str) -> Result<Diff, String> {
    let mut diff = Diff::default();
    let mut current: Option<FileBuilder> = None;
    let mut lines = text.split_terminator('\n').enumerate().peekable();
    while let Some((number, raw)) = lines.next() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if is_combined(line) {
            return Err(error(
                number,
                "combined merge diffs are unsupported; use a first-parent or individual-parent diff",
            ));
        }
        if let Some(paths) = line.strip_prefix("diff --git ") {
            finish(&mut current, &mut diff)?;
            let (old, new, ambiguous) = git_paths(paths).map_err(|e| error(number, e))?;
            let mut file = FileBuilder::new(old, new);
            file.git_header = Some(paths.to_owned());
            if ambiguous {
                file.unresolved_paths = Some((number, 0));
            }
            current = Some(file);
            continue;
        }
        if line == "diff --git" {
            return Err(error(number, "expected two paths in diff --git header"));
        }
        if current.as_ref().is_some_and(|f| f.binary_payload) {
            // Git binary payload records have a separate encoding, with no text hunks.
            continue;
        }
        if let Some(path) = line.strip_prefix("--- ") {
            if current.as_ref().is_some_and(|f| {
                f.headers_seen || f.file.metadata.iter().any(|m| binary_summary(m))
            }) {
                finish(&mut current, &mut diff)?;
            }
            let old = header_path(path, 'a').map_err(|e| error(number, e))?;
            let (next_number, next_raw) = lines
                .next()
                .ok_or_else(|| error(number, "missing +++ file header after --- header"))?;
            let next = next_raw.strip_suffix('\r').unwrap_or(next_raw);
            if is_combined(next) {
                return Err(error(next_number, "combined merge diffs are unsupported"));
            }
            let path = next.strip_prefix("+++ ").ok_or_else(|| {
                error(
                    next_number,
                    "expected +++ file header immediately after --- header",
                )
            })?;
            let new = header_path(path, 'b').map_err(|e| error(next_number, e))?;
            let file = current.get_or_insert_with(|| FileBuilder::new(old.clone(), new.clone()));
            file.file.old_path = old;
            file.file.new_path = new;
            file.headers_seen = true;
            file.unresolved_paths = None;
            continue;
        }
        if line.starts_with("@@") {
            let file = current
                .as_mut()
                .filter(|f| f.headers_seen)
                .ok_or_else(|| error(number, "hunk requires preceding --- and +++ file headers"))?;
            let range = hunk_range(line).map_err(|e| error(number, e))?;
            let old_begin = boundary(range.old_start, range.old_count);
            let new_begin = boundary(range.new_start, range.new_count);
            if let Some((old_end, new_end)) = file.previous_ends
                && (old_begin < old_end || new_begin < new_end)
            {
                return Err(error(number, "hunk ranges overlap or are out of order"));
            }
            let hunk = parse_hunk(number, raw, range, &mut lines, &mut file.eof)?;
            file.previous_ends = Some((
                old_begin + u64::from(range.old_count),
                new_begin + u64::from(range.new_count),
            ));
            file.file.hunks.push(hunk);
            continue;
        }
        if line.is_empty() {
            continue;
        }
        if binary_summary(line)
            && current.as_ref().is_none_or(|file| {
                file.git_header.is_none()
                    || file.headers_seen
                    || file.file.metadata.iter().any(|m| binary_summary(m))
            })
        {
            finish(&mut current, &mut diff)?;
            let (old, new) = binary_paths(line, None).map_err(|e| error(number, e))?;
            let mut file = FileBuilder::new(old, new);
            file.file.metadata.push(raw.to_owned());
            current = Some(file);
            continue;
        }
        if let Some(file) = current.as_mut()
            && metadata(line)
        {
            if let Some(path) = line
                .strip_prefix("rename from ")
                .or_else(|| line.strip_prefix("copy from "))
            {
                file.file.old_path = metadata_path(path).map_err(|e| error(number, e))?;
                file.resolve_path(1);
            } else if let Some(path) = line
                .strip_prefix("rename to ")
                .or_else(|| line.strip_prefix("copy to "))
            {
                file.file.new_path = metadata_path(path).map_err(|e| error(number, e))?;
                file.resolve_path(2);
            } else if line.starts_with("new file mode ") {
                file.file.old_path = "/dev/null".into();
            } else if line.starts_with("deleted file mode ") {
                file.file.new_path = "/dev/null".into();
            } else if line.starts_with("Binary files ") && file.unresolved_paths.is_some() {
                // Unlike diff --git's single space, ` and ` is a distinct
                // separator and often disambiguates names in no-index diffs.
                let (old, new) =
                    binary_paths(line, file.git_header.as_deref()).map_err(|e| error(number, e))?;
                file.file.old_path = old;
                file.file.new_path = new;
                file.unresolved_paths = None;
            }
            file.binary_payload = line == "GIT binary patch";
            file.file.metadata.push(raw.to_owned());
            continue;
        }
        if line.starts_with("diff ")
            || line.starts_with("Index: ")
            || (line.len() >= 3 && line.bytes().all(|c| c == b'='))
        {
            // These are ordinary diff / patch-tool preambles, not file metadata.
            finish(&mut current, &mut diff)?;
            continue;
        }
        let detail = if line.starts_with([' ', '+', '-', '\\']) {
            "unexpected diff line outside a hunk (check the preceding hunk's line counts)"
                .to_owned()
        } else {
            format!("unrecognized patch record: {}", sanitize(line))
        };
        return Err(error(number, detail));
    }
    finish(&mut current, &mut diff)?;
    Ok(diff)
}

fn binary_summary(line: &str) -> bool {
    line.starts_with("Binary files ") && line.trim_end_matches('\r').ends_with(" differ")
}

fn metadata(line: &str) -> bool {
    [
        "index ",
        "old mode ",
        "new mode ",
        "new file mode ",
        "deleted file mode ",
        "similarity index ",
        "dissimilarity index ",
        "rename from ",
        "rename to ",
        "copy from ",
        "copy to ",
        "Submodule ",
    ]
    .iter()
    .any(|p| line.starts_with(p))
        || binary_summary(line)
        || line == "GIT binary patch"
}

fn strip_prefix(path: String, prefix: char) -> String {
    if path.starts_with(prefix) && path.as_bytes().get(1) == Some(&b'/') {
        path[2..].to_owned()
    } else {
        path
    }
}

fn header_path(text: &str, prefix: char) -> Result<String, String> {
    let path = if text.starts_with('"') {
        let (path, used) = quoted_path(text)?;
        if !text[used..].is_empty() && !text[used..].starts_with(['\t', ' ']) {
            return Err("unexpected text after quoted file name".into());
        }
        path
    } else {
        // GNU diff separates timestamps with a TAB, not a space.
        text.split('\t').next().unwrap_or(text).to_owned()
    };
    let path = strip_prefix(path, prefix);
    if path.is_empty() {
        return Err("empty file name in patch header".into());
    }
    Ok(path)
}

fn metadata_path(text: &str) -> Result<String, String> {
    if text.is_empty() {
        return Err("empty rename/copy file name".into());
    }
    if text.starts_with('"') {
        let (path, used) = quoted_path(text)?;
        if used != text.len() {
            return Err("unexpected text after quoted file name".into());
        }
        if path.is_empty() {
            return Err("empty quoted file name".into());
        }
        Ok(path)
    } else {
        Ok(text.to_owned())
    }
}

fn git_paths(text: &str) -> Result<(String, String, bool), String> {
    let (old, new, ambiguous) = if text.starts_with('"') {
        let (old, used) = quoted_path(text)?;
        let rest = text[used..]
            .strip_prefix(' ')
            .ok_or("expected two paths in diff --git header")?;
        (old, metadata_path(rest)?, false)
    } else if let Some((old, new)) = text.split_once(" \"") {
        (old.to_owned(), metadata_path(&format!("\"{new}"))?, false)
    } else {
        // Git does not quote spaces. Prefer the exact same path on both sides;
        // otherwise a/b prefixes delimit the pair. ---/+++ and rename records
        // later provide authoritative names for changed/renamed files.
        let splits: Vec<usize> = text.match_indices(' ').map(|(i, _)| i).collect();
        let same = splits.iter().copied().find(|&i| {
            let a = text[..i].strip_prefix("a/").unwrap_or(&text[..i]);
            let b = text[i + 1..].strip_prefix("b/").unwrap_or(&text[i + 1..]);
            a == b
        });
        let mut prefixed = splits
            .iter()
            .copied()
            .filter(|&i| text[i + 1..].starts_with("b/"));
        let first_prefixed = prefixed.next();
        let ambiguous = same.is_none()
            && if first_prefixed.is_some() {
                prefixed.next().is_some()
            } else {
                splits.len() > 1
            };
        let split = same
            .or(first_prefixed)
            .or_else(|| splits.first().copied())
            .ok_or("expected two paths in diff --git header")?;
        (
            text[..split].to_owned(),
            text[split + 1..].to_owned(),
            ambiguous,
        )
    };
    let (old, new) = (strip_prefix(old, 'a'), strip_prefix(new, 'b'));
    if !ambiguous && (old.is_empty() || new.is_empty()) {
        return Err("empty file name in diff --git header".into());
    }
    Ok((old, new, ambiguous))
}

fn binary_paths(line: &str, hint: Option<&str>) -> Result<(String, String), String> {
    let text = line
        .strip_prefix("Binary files ")
        .and_then(|s| s.strip_suffix(" differ"))
        .ok_or("malformed binary-file summary")?;
    let (old, new) = if text.starts_with('"') {
        let (old, used) = quoted_path(text)?;
        let new = text[used..]
            .strip_prefix(" and ")
            .ok_or("malformed binary-file separator")?;
        (old, metadata_path(new)?)
    } else if let Some((old, new)) = text.split_once(" and \"") {
        (old.to_owned(), metadata_path(&format!("\"{new}"))?)
    } else {
        let splits: Vec<usize> = text.match_indices(" and ").map(|(i, _)| i).collect();
        let hinted = hint
            .map(|header| binary_split_hint(text, &splits, header))
            .transpose()?;
        let same = splits.iter().copied().find(|&i| {
            let old = text[..i].strip_prefix("a/").unwrap_or(&text[..i]);
            let new = text[i + 5..].strip_prefix("b/").unwrap_or(&text[i + 5..]);
            old == new
        });
        let mut prefixed = splits
            .iter()
            .copied()
            .filter(|&i| text[i + 5..].starts_with("b/"));
        let first_prefixed = prefixed.next();
        let unique_prefixed = first_prefixed.filter(|_| prefixed.next().is_none());
        let unique = if splits.len() == 1 {
            splits.first().copied()
        } else {
            None
        };
        let split = hinted.or(same).or(unique_prefixed).or(unique).ok_or(
            "ambiguous or malformed binary-file paths; supply a Git patch with file headers",
        )?;
        (text[..split].to_owned(), text[split + 5..].to_owned())
    };
    let (old, new) = (strip_prefix(old, 'a'), strip_prefix(new, 'b'));
    if old.is_empty() || new.is_empty() {
        return Err("empty binary-file path".into());
    }
    Ok((old, new))
}

// A binary summary inserts `and ` into a diff --git header. Common prefix and
// suffix lengths locate that insertion in linear time, even if either filename
// itself contains ` and b/`. Do not guess if both records are still ambiguous.
fn binary_split_hint(text: &str, splits: &[usize], header: &str) -> Result<usize, String> {
    if header.len().checked_add(4) != Some(text.len()) {
        return Err("binary summary paths disagree with diff --git header".into());
    }
    let prefix = text
        .bytes()
        .zip(header.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = text
        .bytes()
        .rev()
        .zip(header.bytes().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let mut matches = splits.iter().copied().filter(|&i| {
        let insertion = i + 1;
        insertion <= prefix && insertion <= header.len() && header.len() - insertion <= suffix
    });
    let first = matches
        .next()
        .ok_or("binary summary paths disagree with diff --git header")?;
    if matches.next().is_some() {
        return Err(
            "ambiguous binary-file paths even with diff --git header; use quoted file names".into(),
        );
    }
    Ok(first)
}

/// Decode Git's C-style quoted paths, including octal UTF-8 byte sequences.
fn quoted_path(text: &str) -> Result<(String, usize), String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                return String::from_utf8(out)
                    .map(|path| (path, i + 1))
                    .map_err(|_| "quoted file name is not valid UTF-8".into());
            }
            b'\\' => {
                i += 1;
                let byte = *bytes
                    .get(i)
                    .ok_or("unfinished escape in quoted file name")?;
                if (b'0'..=b'7').contains(&byte) {
                    let mut value = 0u16;
                    let mut digits = 0;
                    while i < bytes.len() && (b'0'..=b'7').contains(&bytes[i]) && digits < 3 {
                        value = value * 8 + u16::from(bytes[i] - b'0');
                        i += 1;
                        digits += 1;
                    }
                    out.push(u8::try_from(value).map_err(|_| "octal filename escape exceeds 255")?);
                    continue;
                }
                out.push(match byte {
                    b'a' => 7,
                    b'b' => 8,
                    b't' => b'\t',
                    b'n' => b'\n',
                    b'v' => 11,
                    b'f' => 12,
                    b'r' => b'\r',
                    b'\\' => b'\\',
                    b'"' => b'"',
                    _ => return Err("unknown escape in quoted file name".into()),
                });
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    Err("unterminated quoted file name".into())
}

#[derive(Clone, Copy)]
struct HunkRange {
    old_start: u32,
    old_count: u32,
    new_start: u32,
    new_count: u32,
}

fn boundary(start: u32, count: u32) -> u64 {
    u64::from(start) - u64::from(count != 0)
}

fn take_number(text: &str, position: &mut usize) -> Result<u32, String> {
    let start = *position;
    let mut number = 0u32;
    while let Some(byte @ b'0'..=b'9') = text.as_bytes().get(*position) {
        number = number
            .checked_mul(10)
            .and_then(|n| n.checked_add(u32::from(byte - b'0')))
            .ok_or("hunk line number/count exceeds u32")?;
        *position += 1;
    }
    if *position == start {
        return Err("expected a decimal hunk line number/count".into());
    }
    Ok(number)
}

fn take_range(text: &str, position: &mut usize) -> Result<(u32, u32), String> {
    let start = take_number(text, position)?;
    let count = if text.as_bytes().get(*position) == Some(&b',') {
        *position += 1;
        take_number(text, position)?
    } else {
        1
    };
    if count > 0 && (start == 0 || start.checked_add(count - 1).is_none()) {
        return Err("nonempty hunk range must be within lines 1..=4294967295".into());
    }
    Ok((start, count))
}

fn hunk_range(text: &str) -> Result<HunkRange, String> {
    if !text.starts_with("@@ -") {
        return Err("malformed hunk header; expected @@ -old[,count] +new[,count] @@".into());
    }
    let mut position = 4;
    let (old_start, old_count) = take_range(text, &mut position)?;
    if !text[position..].starts_with(" +") {
        return Err("malformed hunk header: missing new-file range".into());
    }
    position += 2;
    let (new_start, new_count) = take_range(text, &mut position)?;
    if !text[position..].starts_with(" @@") {
        return Err("malformed hunk header: missing closing @@".into());
    }
    position += 3;
    if !text[position..].is_empty() && !text[position..].starts_with(' ') {
        return Err("malformed hunk header: expected a space before section text".into());
    }
    Ok(HunkRange {
        old_start,
        old_count,
        new_start,
        new_count,
    })
}

fn parse_hunk(
    header_number: usize,
    header: &str,
    range: HunkRange,
    lines: &mut Lines<'_>,
    eof: &mut (bool, bool),
) -> Result<Hunk, String> {
    let mut hunk = Hunk {
        header: header.to_owned(),
        old_start: range.old_start,
        new_start: range.new_start,
        lines: Vec::new(),
    };
    let (mut old, mut new) = (u64::from(range.old_start), u64::from(range.new_start));
    let (mut old_left, mut new_left) = (range.old_count, range.new_count);
    let mut marker_allowed = false;
    loop {
        if let Some(&(number, raw)) = lines.peek()
            && raw.strip_suffix('\r').unwrap_or(raw) == "\\ No newline at end of file"
        {
            if !marker_allowed {
                return Err(error(
                    number,
                    "EOF newline marker must follow a content line, and cannot be repeated",
                ));
            }
            lines.next();
            match hunk.lines.last().map(|line| line.kind) {
                Some(LineKind::Removed) => eof.0 = true,
                Some(LineKind::Added) => eof.1 = true,
                Some(LineKind::Context) => *eof = (true, true),
                _ => unreachable!("marker_allowed requires a preceding content line"),
            }
            hunk.lines.push(DiffLine {
                kind: LineKind::NoNewline,
                text: raw.to_owned(),
                old_number: None,
                new_number: None,
            });
            marker_allowed = false;
            continue;
        }
        if old_left == 0 && new_left == 0 {
            break;
        }
        let (number, raw) = lines.next().ok_or_else(|| {
            error(
                header_number,
                format!("truncated hunk: still expected {old_left} old and {new_left} new lines"),
            )
        })?;
        if is_combined(raw) {
            return Err(error(number, "combined merge diffs are unsupported"));
        }
        let (kind, consumes_old, consumes_new) = match raw.as_bytes().first() {
            Some(b' ') => (LineKind::Context, true, true),
            Some(b'-') => (LineKind::Removed, true, false),
            Some(b'+') => (LineKind::Added, false, true),
            _ => {
                return Err(error(
                    number,
                    format!(
                        "truncated or malformed hunk: expected {old_left} old and {new_left} new lines; content must begin with space, +, or -"
                    ),
                ));
            }
        };
        if (consumes_old && eof.0) || (consumes_new && eof.1) {
            return Err(error(
                number,
                "content follows the corresponding side's EOF marker",
            ));
        }
        if (consumes_old && old_left == 0) || (consumes_new && new_left == 0) {
            return Err(error(
                number,
                "hunk contains more lines than its declared old/new counts",
            ));
        }
        hunk.lines.push(DiffLine {
            kind,
            text: raw[1..].to_owned(),
            old_number: consumes_old.then_some(old as u32),
            new_number: consumes_new.then_some(new as u32),
        });
        if consumes_old {
            old += 1;
            old_left -= 1;
        }
        if consumes_new {
            new += 1;
            new_left -= 1;
        }
        marker_allowed = true;
    }
    Ok(hunk)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(body: &str) -> Diff {
        parse_patch(body).unwrap()
    }

    #[test]
    fn numbers_blank_lines_and_separate_hunks() {
        let diff = parse(
            "--- a/file\n+++ b/file\n@@ -2,3 +2,4 @@ section\n same\n-\n+new\n+\n tail\n@@ -10 +11 @@\n-last\n+end\n",
        );
        let file = &diff.files[0];
        assert_eq!((file.additions(), file.deletions()), (3, 2));
        let lines = &file.hunks[0].lines;
        assert_eq!(
            lines
                .iter()
                .map(|l| (l.old_number, l.new_number))
                .collect::<Vec<_>>(),
            vec![
                (Some(2), Some(2)),
                (Some(3), None),
                (None, Some(3)),
                (None, Some(4)),
                (Some(4), Some(5))
            ]
        );
        assert_eq!(lines[1].text, "");
        assert_eq!(lines[3].text, "");
        assert_eq!(file.hunks[1].lines[1].new_number, Some(11));
    }

    #[test]
    fn eof_markers_do_not_advance_numbers() {
        let diff = parse(
            "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n",
        );
        let lines = &diff.files[0].hunks[0].lines;
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[1].kind, LineKind::NoNewline);
        assert_eq!((lines[1].old_number, lines[1].new_number), (None, None));
        assert_eq!(lines[2].new_number, Some(1));
    }

    #[test]
    fn additions_deletions_and_empty_ranges() {
        let diff = parse(
            "diff --git a/new b/new\nnew file mode 100644\n--- /dev/null\n+++ b/new\n@@ -0,0 +1,2 @@\n+one\n+two\ndiff --git a/old b/old\ndeleted file mode 100644\n--- a/old\n+++ /dev/null\n@@ -1 +0,0 @@\n-gone\n",
        );
        assert_eq!(diff.files[0].old_path, "/dev/null");
        assert_eq!(diff.files[1].display_path(), "old");
        assert_eq!(diff.files[1].hunks[0].lines[0].new_number, None);
        assert!(parse_patch("--- a/f\n+++ b/f\n@@ -0 +1 @@\n-x\n+y\n").is_err());
        assert!(parse_patch("--- a/f\n+++ b/f\n@@ -0,0 +0,0 @@\n").is_ok());
    }

    #[test]
    fn file_headers_inside_hunks_are_content() {
        let diff = parse(
            "--- a/one\n+++ b/one\n@@ -1,2 +1,2 @@\n--- a/not-a-file\n+++ b/not-a-file\n diff --git a/x b/x\n--- a/two\n+++ b/two\n@@ -0,0 +1 @@\n+next\n",
        );
        assert_eq!(diff.files.len(), 2);
        assert_eq!(diff.files[0].hunks[0].lines[0].text, "-- a/not-a-file");
        assert_eq!(diff.files[1].display_path(), "two");
    }

    #[test]
    fn quoted_unicode_and_control_names() {
        let diff = parse(
            "diff --git \"a/caf\\303\\251\\t\\\"x\" \"b/caf\\303\\251\\t\\\"x\"\nold mode 100644\nnew mode 100755\n",
        );
        assert_eq!(diff.files[0].display_path(), "café\t\"x");
        assert_eq!(diff.files[0].metadata.len(), 2);
        assert!(parse_patch("diff --git \"a/\\777\" b/f\n").is_err());
        assert!(parse_patch("diff --git \"a/\\377\" b/f\n").is_err());
        assert!(parse_patch("diff --git \"a/no end b/f\n").is_err());
    }

    #[test]
    fn spaces_timestamps_and_authoritative_rename_paths() {
        let diff = parse(
            "diff --git a/a b/c d b/a b/c d\nold mode 100644\nnew mode 100755\ndiff --git a/old name b/new name\nsimilarity index 100%\nrename from old name\nrename to new name\n--- old file name\t2026-01-01 00:00:00 +0000\n+++ new file name\t2026-01-01 00:00:00 +0000\n@@ -1 +1 @@\n-a\n+b\n",
        );
        assert_eq!(diff.files[0].old_path, "a b/c d");
        assert_eq!(diff.files[0].new_path, "a b/c d");
        assert_eq!(diff.files[1].old_path, "old file name");
        assert_eq!(diff.files[1].new_path, "new file name");
    }

    #[test]
    fn binary_mode_and_rename_summaries_have_no_duplicate_headers() {
        let diff = parse(
            "diff --git a/old b/new\nsimilarity index 100%\nrename from old\nrename to new\ndiff --git a/bin b/bin\nindex 111..222 100644\nBinary files a/bin and b/bin differ\ndiff --git a/empty b/empty\nnew file mode 100644\nindex 000..111\ndiff --git a/data b/data\nGIT binary patch\nliteral 3\nKc$`a12345\n\nliteral 0\nHcmV?d00001\n",
        );
        assert_eq!(diff.files.len(), 4);
        assert_eq!(diff.files[0].display_path(), "new");
        assert_eq!(diff.files[1].metadata.len(), 2);
        assert_eq!(diff.files[2].old_path, "/dev/null");
        assert_eq!(diff.files[3].metadata, vec!["GIT binary patch"]);
        assert!(diff.files.iter().all(|f| f.hunks.is_empty()));
        assert_eq!(parse("Binary files old and new differ\n").files.len(), 1);
    }

    #[test]
    fn rejects_malformed_truncated_or_overflowing_hunks() {
        for body in [
            "@@ -1 +1 @@\n-old\n",
            "@@ -1,2 +1 @@\n-old\n+new\n",
            "@@ -1 +1 @@\n-old\n+new\n+extra\n",
            "@@ -1 +1 @@\n\n",
            "@@ -1 +1 @\n",
            "@@ -1 +x @@\n",
            "@@ -4294967296 +1 @@\n",
            "@@ -4294967295,2 +1 @@\n",
            "@@ -1 +1 @@bad\n",
            "@@ -0,0 +0,0 @@\n\\ No newline at end of file\n",
            "@@ -1 +1 @@\n-a\n+b\n\\ No newline at end of file\n\\ No newline at end of file\n",
        ] {
            let result = parse_patch(&format!("--- a/f\n+++ b/f\n{body}"));
            assert!(result.is_err(), "accepted {body:?}");
            assert!(result.unwrap_err().contains("Patch line"));
        }
        assert!(
            parse_patch("--- a/f\n")
                .unwrap_err()
                .contains("missing +++")
        );
        assert!(parse_patch("@@ -1 +1 @@\n-a\n+b\n").is_err());
    }

    #[test]
    fn overlapping_hunks_and_combined_diffs_fail_explicitly() {
        assert!(
            parse_patch("--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n@@ -1 +1 @@\n-a\n+b\n")
                .unwrap_err()
                .contains("overlap")
        );
        for patch in [
            "diff --cc f\n",
            "diff --combined f\n",
            "--- a/f\n+++ b/f\n@@@ -1 -1 +1 @@@\n",
        ] {
            assert!(parse_patch(patch).unwrap_err().contains("combined"));
        }
    }

    #[test]
    fn raw_text_and_printable_unicode_are_preserved() {
        let diff = parse("--- a/f\n+++ b/f\n@@ -1 +1 @@\n-\u{1b}[31m\t☃\r\n+é\r\n");
        assert_eq!(diff.files[0].hunks[0].lines[0].text, "\u{1b}[31m\t☃\r");
        assert_eq!(diff.files[0].hunks[0].lines[1].text, "é\r");
        let controls: String = (0..=159).filter_map(char::from_u32).collect();
        assert!(!sanitize(&controls).chars().any(char::is_control));
        assert_eq!(
            sanitize("é\t☃\u{1b}[31m\r\n\u{85}"),
            "é    ☃^[[31m^M^J\\u{0085}"
        );
    }

    #[test]
    fn ambiguous_git_path_hints_are_resolved_by_authoritative_records() {
        let diff = parse(
            "diff --git a/old b/part b/new\n--- a/old b/part\n+++ b/new\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/from b/part b/to\nsimilarity index 100%\nrename from from b/part\nrename to to\ndiff --git a/left b/part b/right\nBinary files a/left b/part and b/right differ\ndiff --git a/left and b/part b/right\nBinary files a/left and b/part and b/right differ\n",
        );
        assert_eq!(diff.files[0].old_path, "old b/part");
        assert_eq!(diff.files[1].old_path, "from b/part");
        assert_eq!(diff.files[2].old_path, "left b/part");
        assert_eq!(diff.files[2].new_path, "right");
        assert_eq!(diff.files[3].old_path, "left and b/part");
        assert_eq!(diff.files[3].new_path, "right");
        assert!(parse_patch("Binary files left and first and right differ\n").is_err());
        assert!(
            parse_patch("diff --git a/old b/part b/new\nold mode 100644\nnew mode 100755\n")
                .unwrap_err()
                .contains("ambiguous")
        );
        let quoted =
            parse("Binary files \"a/old and \\\"name\" and \"b/new and \\\"name\" differ\n");
        assert_eq!(quoted.files[0].old_path, "old and \"name");
        assert_eq!(quoted.files[0].new_path, "new and \"name");
    }

    #[test]
    fn maximum_line_number_and_zero_length_ranges_remain_exact() {
        let diff = parse("--- a/f\n+++ b/f\n@@ -4294967295 +4294967295 @@\n-old\n+new\n");
        let lines = &diff.files[0].hunks[0].lines;
        assert_eq!(lines[0].old_number, Some(u32::MAX));
        assert_eq!(lines[1].new_number, Some(u32::MAX));
        let diff = parse("--- a/f\n+++ b/f\n@@ -0,0 +1 @@\n+one\n@@ -0,0 +2 @@\n+two\n");
        assert_eq!(diff.files[0].hunks[1].lines[0].new_number, Some(2));
        for patch in [
            "diff --git a/ b/f\n",
            "--- a/\n+++ b/f\n",
            "diff --git a/f b/f\nrename to \"\"\n",
        ] {
            assert!(parse_patch(patch).is_err());
        }
    }

    #[test]
    fn empty_and_ordinary_diff_preambles() {
        assert!(parse("").files.is_empty());
        assert!(parse("\n\n").files.is_empty());
        let diff = parse("diff -u old new\n--- old\n+++ new\n@@ -1 +1 @@\n-a\n+b");
        assert_eq!(diff.files[0].display_path(), "new");
        assert!(parse_patch("not a patch\n").is_err());
    }
}
