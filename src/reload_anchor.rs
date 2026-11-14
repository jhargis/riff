//! SourceId indexes are valid within one patch only. Cross-patch anchors use
//! actual path identities, line side/number/text, and surviving old neighbors.
use crate::{
    patch::{Diff, DiffLine, FileDiff, sanitize},
    render::SourceId,
    wrap::SourcePosition,
};

fn file_index(source: SourceId) -> usize {
    match source {
        SourceId::Header(file, _) | SourceId::Hunk(file, _) | SourceId::Line(file, _, _) => file,
    }
}

fn real_path(path: &str) -> bool {
    !path.is_empty() && path != "/dev/null"
}

fn find_file(diff: &Diff, old: &FileDiff) -> Option<usize> {
    diff.files
        .iter()
        .position(|file| file.old_path == old.old_path && file.new_path == old.new_path)
        .or_else(|| {
            diff.files.iter().position(|file| {
                [&old.old_path, &old.new_path].into_iter().any(|path| {
                    real_path(path) && (path == &file.old_path || path == &file.new_path)
                })
            })
        })
}

fn header(file: usize) -> SourcePosition {
    SourcePosition {
        source: SourceId::Header(file, 0),
        offset: 0,
    }
}

fn nearest_file(old: &Diff, new: &Diff, index: usize) -> Option<SourcePosition> {
    // Prefer a surviving neighbor in the original patch, not a new file that
    // happens to occupy the removed file's numeric slot.
    (0..old.files.len())
        .filter(|candidate| *candidate != index)
        .filter_map(|candidate| {
            find_file(new, &old.files[candidate]).map(|file| (candidate.abs_diff(index), file))
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, file)| header(file))
        .or_else(|| (!new.files.is_empty()).then(|| header(0)))
}

fn line_distance(old: &DiffLine, candidate: &DiffLine) -> (u32, u64) {
    // Either side can stay fixed while edits shift the other (including file
    // pairs whose left side changes). Never compare old to new coordinates.
    [
        old.old_number.zip(candidate.old_number),
        old.new_number.zip(candidate.new_number),
    ]
    .into_iter()
    .flatten()
    .map(|(old, new)| old.abs_diff(new))
    .fold((u32::MAX, 0), |(nearest, total), distance| {
        (nearest.min(distance), total + u64::from(distance))
    })
}

pub(super) fn remap(old: &Diff, new: &Diff, position: SourcePosition) -> Option<SourcePosition> {
    let old_index = file_index(position.source);
    let old_file = old.files.get(old_index)?;
    let Some(file_index) = find_file(new, old_file) else {
        return nearest_file(old, new, old_index);
    };
    let file = &new.files[file_index];
    let (source, text) = match position.source {
        SourceId::Header(_, record) => {
            let record = if record < 3 {
                record
            } else {
                old_file
                    .metadata
                    .get(record - 3)
                    .and_then(|text| file.metadata.iter().position(|candidate| candidate == text))
                    .map_or(0, |index| index + 3)
            };
            return Some(SourcePosition {
                source: SourceId::Header(file_index, record),
                offset: 0,
            });
        }
        SourceId::Hunk(_, hunk) => {
            let old_hunk = old_file.hunks.get(hunk)?;
            let Some((index, hunk)) = file.hunks.iter().enumerate().min_by_key(|(_, hunk)| {
                let old_distance = hunk.old_start.abs_diff(old_hunk.old_start);
                let new_distance = hunk.new_start.abs_diff(old_hunk.new_start);
                // A large insertion on one side must not outweigh the surviving
                // hunk's unchanged coordinate on the other side.
                (
                    old_distance.min(new_distance),
                    u64::from(old_distance) + u64::from(new_distance),
                )
            }) else {
                return Some(header(file_index));
            };
            (SourceId::Hunk(file_index, index), sanitize(&hunk.header))
        }
        SourceId::Line(_, hunk, line) => {
            let old_line = old_file.hunks.get(hunk)?.lines.get(line)?;
            let candidate = file
                .hunks
                .iter()
                .enumerate()
                .flat_map(|(hunk, value)| {
                    value
                        .lines
                        .iter()
                        .enumerate()
                        .map(move |(line, value)| (hunk, line, value))
                })
                .filter(|(_, _, line)| {
                    old_line.old_number.zip(line.old_number).is_some()
                        || old_line.new_number.zip(line.new_number).is_some()
                        || (old_line.old_number.is_none()
                            && old_line.new_number.is_none()
                            && line.kind == old_line.kind)
                })
                .min_by_key(|(_, _, line)| {
                    (
                        line.text != old_line.text,
                        // An inserted duplicate is not the surviving context
                        // merely because it inherited that context's number.
                        line.kind != old_line.kind,
                        line_distance(old_line, line),
                    )
                });
            let Some((hunk, line, value)) = candidate else {
                return Some(header(file_index));
            };
            (
                SourceId::Line(file_index, hunk, line),
                sanitize(&value.text),
            )
        }
    };
    let mut offset = position.offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    Some(SourcePosition { source, offset })
}
