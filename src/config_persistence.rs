//! Comment-preserving, selective settings updates with atomic replacement.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use toml_edit::{DocumentMut, Item, Value};

use super::{SettingsUpdate, default_config_path, parse_config};

/// Persist only fields present in `update`, re-reading and validating the file
/// on every call. An empty update performs no configuration I/O. Missing paths
/// are created only for a real update; invalid existing files are never replaced.
///
/// Existing file symlinks are followed, including dangling links, so atomic
/// replacement changes the target rather than a dotfile-manager symlink. The
/// target's permissions are preserved; new files use tempfile's private mode.
/// Returns the requested (or discovered default) path, not the symlink target.
pub fn save_config(path: Option<&Path>, update: &SettingsUpdate) -> Result<PathBuf, String> {
    let path = path
        .map(Path::to_path_buf)
        .or_else(default_config_path)
        .ok_or_else(|| {
            "could not discover a per-user configuration directory; provide --config PATH"
                .to_string()
        })?;
    if update == &SettingsUpdate::default() {
        return Ok(path);
    }
    save_path(&path, update)
        .map_err(|error| format!("could not save configuration {}: {error}", path.display()))?;
    Ok(path)
}

fn save_path(path: &Path, update: &SettingsUpdate) -> Result<(), String> {
    let target = resolve_target(path).map_err(|error| format!("resolve target: {error}"))?;
    let (text, permissions) = match fs::metadata(&target) {
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err("configuration target is not a regular file".to_string());
            }
            let text = fs::read_to_string(&target)
                .map_err(|error| format!("read existing configuration: {error}"))?;
            (text, Some(metadata.permissions()))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => (String::new(), None),
        Err(error) => return Err(format!("inspect existing configuration: {error}")),
    };
    // Validate before making changes: an update must not silently repair or
    // discard an unknown key, malformed palette, or invalid selected setting.
    parse_config(&text)?;
    let mut document = text
        .parse::<DocumentMut>()
        .map_err(|error| format!("invalid TOML: {error}"))?;
    if let Some(theme) = &update.theme {
        replace_value(&mut document, "theme", Value::from(theme.as_str()));
    }
    if let Some(view) = &update.view {
        replace_value(&mut document, "view", Value::from(view.as_str()));
    }
    if let Some(wrap) = update.wrap {
        replace_value(&mut document, "wrap", Value::from(wrap));
    }
    if let Some(word_diff) = update.word_diff {
        replace_value(&mut document, "word_diff", Value::from(word_diff));
    }
    if let Some(watch) = update.watch {
        replace_value(&mut document, "watch", Value::from(watch));
    }
    let updated = document.to_string();
    // Reuse the loader's theme aliases, custom theme resolution, and validation.
    parse_config(&updated)?;

    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "create configuration directory {}: {error}",
            parent.display()
        )
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("create temporary configuration: {error}"))?;
    temporary
        .write_all(updated.as_bytes())
        .map_err(|error| format!("write temporary configuration: {error}"))?;
    if let Some(permissions) = permissions {
        temporary
            .as_file()
            .set_permissions(permissions)
            .map_err(|error| format!("preserve configuration permissions: {error}"))?;
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("sync temporary configuration: {error}"))?;
    temporary
        .persist(&target)
        .map_err(|error| format!("replace configuration atomically: {}", error.error))?;
    Ok(())
}

fn replace_value(document: &mut DocumentMut, key: &str, mut value: Value) {
    if let Some(existing) = document.get_mut(key).and_then(Item::as_value_mut) {
        // Assignment alone discards inline comments and surrounding whitespace.
        *value.decor_mut() = existing.decor().clone();
        *existing = value;
    } else {
        document[key] = Item::Value(value);
    }
}

fn resolve_target(path: &Path) -> io::Result<PathBuf> {
    let mut target = path.to_path_buf();
    // Follow the final component ourselves: canonicalize rejects dangling
    // symlinks, whose missing target can legitimately be created on first save.
    // Parent-directory symlinks are followed naturally by filesystem operations.
    for followed in 0..=40 {
        match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                if followed == 40 {
                    break;
                }
                let link = fs::read_link(&target)?;
                target = if link.is_absolute() {
                    link
                } else {
                    target.parent().unwrap_or_else(|| Path::new(".")).join(link)
                };
            }
            Ok(_) => return Ok(target),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(target),
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other("too many configuration symlinks"))
}
