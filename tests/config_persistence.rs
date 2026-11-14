//! Selective settings persistence; every write is isolated in a temporary tree.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use riff::config::{SettingsUpdate, default_config_path, load_config, save_config};
use tempfile::TempDir;

fn wrap_update(wrap: bool) -> SettingsUpdate {
    SettingsUpdate {
        wrap: Some(wrap),
        ..SettingsUpdate::default()
    }
}

#[test]
fn settings_update_is_an_empty_cloneable_value_by_default() {
    let update = SettingsUpdate::default();
    assert_eq!(update, update.clone());
    assert_eq!(update.theme, None);
    assert_eq!(update.view, None);
    assert_eq!(update.wrap, None);
}

#[test]
fn selective_updates_create_and_reload_only_the_requested_fields() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("nested/riff/config.toml");
    assert_eq!(save_config(Some(&path), &wrap_update(true)).unwrap(), path);
    assert_eq!(fs::read_to_string(&path).unwrap(), "wrap = true\n");
    let settings = load_config(Some(&path)).unwrap();
    assert!(settings.wrap);
    assert_eq!(settings.theme, "critique");
    assert_eq!(settings.view, None);

    save_config(
        Some(&path),
        &SettingsUpdate {
            theme: Some("GitHub_Dark".to_string()),
            view: Some("UNIFIED".to_string()),
            wrap: None,
            watch: None,
            word_diff: None,
        },
    )
    .unwrap();
    let settings = load_config(Some(&path)).unwrap();
    assert_eq!(settings.theme, "github-dark");
    assert_eq!(settings.view.as_deref(), Some("unified"));
    assert!(settings.wrap);
    save_config(Some(&path), &wrap_update(false)).unwrap();
    assert!(!load_config(Some(&path)).unwrap().wrap);
}

#[test]
fn empty_update_does_not_create_or_touch_any_file() {
    let directory = TempDir::new().unwrap();
    let missing = directory.path().join("not-created/config.toml");
    assert_eq!(
        save_config(Some(&missing), &SettingsUpdate::default()).unwrap(),
        missing
    );
    assert!(!missing.parent().unwrap().exists());

    let path = directory.path().join("config.toml");
    let original = "# even malformed files are untouched\nnot TOML!\n";
    fs::write(&path, original).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    save_config(Some(&path), &SettingsUpdate::default()).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn preserves_comments_custom_palettes_and_unrelated_settings() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("config.toml");
    let original = "# My preferences\n  theme = 'nord' # selected theme\nview = 'split' # layout\nwrap = false # long lines\n\n# Personal colors\n[themes.'paper palette'] # custom\nextends = 'one dark'\nbackground = '#123456' # identity\nkeyword = '#abcdef'\n";
    fs::write(&path, original).unwrap();

    save_config(Some(&path), &wrap_update(true)).unwrap();
    let with_wrap = original.replace("wrap = false", "wrap = true");
    assert_eq!(fs::read_to_string(&path).unwrap(), with_wrap);

    save_config(
        Some(&path),
        &SettingsUpdate {
            theme: Some("PAPER_PALETTE".to_string()),
            ..SettingsUpdate::default()
        },
    )
    .unwrap();
    let expected = with_wrap.replace("theme = 'nord'", "theme = \"PAPER_PALETTE\"");
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    let settings = load_config(Some(&path)).unwrap();
    assert_eq!(settings.theme, "paper palette");
    assert_eq!(settings.view.as_deref(), Some("split"));
    assert!(settings.wrap);
    assert_eq!(settings.themes.last().unwrap().name, "paper palette");

    save_config(
        Some(&path),
        &SettingsUpdate {
            view: Some("auto".to_string()),
            ..SettingsUpdate::default()
        },
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        expected.replace("view = 'split'", "view = \"auto\"")
    );
}

#[test]
fn rereads_external_changes_instead_of_serializing_stale_settings() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("config.toml");
    fs::write(&path, "theme = 'nord'\nview = 'split'\n").unwrap();
    let _stale_settings = load_config(Some(&path)).unwrap();
    let external = "# changed elsewhere\ntheme = 'dracula'\nview = 'unified'\n\n[themes.mine]\nbackground = '#101010'\n";
    fs::write(&path, external).unwrap();
    save_config(Some(&path), &wrap_update(true)).unwrap();
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("theme = 'dracula'\nview = 'unified'"));
    assert!(saved.contains("# changed elsewhere"));
    assert!(saved.contains("[themes.mine]\nbackground = '#101010'"));
    assert!(load_config(Some(&path)).unwrap().wrap);

    let external = saved.replace("view = 'unified'", "view = 'auto'");
    fs::write(&path, &external).unwrap();
    save_config(
        Some(&path),
        &SettingsUpdate {
            theme: Some("mine".to_string()),
            ..SettingsUpdate::default()
        },
    )
    .unwrap();
    let settings = load_config(Some(&path)).unwrap();
    assert_eq!(settings.theme, "mine");
    assert_eq!(settings.view.as_deref(), Some("auto"));
    assert!(settings.wrap);
}

#[test]
fn invalid_existing_configuration_is_never_replaced_even_by_a_repairing_update() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("config.toml");
    let update = SettingsUpdate {
        theme: Some("nord".to_string()),
        view: Some("auto".to_string()),
        wrap: Some(true),
        watch: None,
        word_diff: None,
    };
    for original in [
        "not TOML",
        "unknown = true\n",
        "theme = 'absent'\n",
        "view = 'diagonal'\n",
        "wrap = 'true'\n",
        "wrap = 1\n",
        "wrap = true\nwrap = false\n",
        "[themes.mine]\nbackground = '#bad'\n",
        "[themes.mine]\nunknown = '#123456'\n",
        "[themes.mine]\nextends = 'absent'\n",
        "[themes.critique]\n",
    ] {
        fs::write(&path, original).unwrap();
        let error = save_config(Some(&path), &update).unwrap_err();
        assert!(error.contains(&path.display().to_string()), "{error}");
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
    fs::write(&path, [0xff, 0xfe]).unwrap();
    assert!(save_config(Some(&path), &update).is_err());
    assert_eq!(fs::read(&path).unwrap(), [0xff, 0xfe]);
}

#[test]
fn invalid_updates_do_not_write_or_create_directories() {
    let directory = TempDir::new().unwrap();
    let missing = directory.path().join("not-created/config.toml");
    let path = directory.path().join("existing.toml");
    let original = "# preserve me\ntheme = 'dracula'\n";
    fs::write(&path, original).unwrap();
    for update in [
        SettingsUpdate {
            theme: Some("missing".to_string()),
            ..SettingsUpdate::default()
        },
        SettingsUpdate {
            theme: Some("mine".to_string()),
            ..SettingsUpdate::default()
        },
        SettingsUpdate {
            view: Some("diagonal".to_string()),
            ..SettingsUpdate::default()
        },
    ] {
        assert!(save_config(Some(&missing), &update).is_err());
        assert!(!missing.parent().unwrap().exists());
        assert!(save_config(Some(&path), &update).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }
}

#[test]
fn externally_introduced_invalid_data_is_revalidated_on_save() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("config.toml");
    fs::write(&path, "theme = 'nord'\n").unwrap();
    let _settings = load_config(Some(&path)).unwrap();
    let invalid = "theme = 'nord'\nwrap = 'wrong type'\n";
    fs::write(&path, invalid).unwrap();
    assert!(save_config(Some(&path), &wrap_update(true)).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), invalid);
}

#[test]
fn filesystem_failures_are_contextual_and_do_not_destroy_existing_data() {
    let directory = TempDir::new().unwrap();
    let blocker = directory.path().join("not-a-directory");
    fs::write(&blocker, "keep this file").unwrap();
    let path = blocker.join("config.toml");
    let error = save_config(Some(&path), &wrap_update(true)).unwrap_err();
    assert!(error.contains(&path.display().to_string()), "{error}");
    assert_eq!(fs::read_to_string(&blocker).unwrap(), "keep this file");

    let path = directory.path().join("directory.toml");
    fs::create_dir(&path).unwrap();
    let sentinel = path.join("preserve.txt");
    fs::write(&sentinel, "preserve").unwrap();
    let error = save_config(Some(&path), &wrap_update(true)).unwrap_err();
    assert!(error.contains(&path.display().to_string()), "{error}");
    assert!(error.contains("not a regular file"), "{error}");
    assert_eq!(fs::read_to_string(sentinel).unwrap(), "preserve");
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    use super::*;

    #[test]
    fn atomic_replacement_preserves_existing_permissions_and_new_files_are_private() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(&path, "wrap = false\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let previous_inode = fs::metadata(&path).unwrap().ino();
        save_config(Some(&path), &wrap_update(true)).unwrap();
        let metadata = fs::metadata(&path).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o640);
        assert_ne!(metadata.ino(), previous_inode);
        assert!(load_config(Some(&path)).unwrap().wrap);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);

        let new = directory.path().join("new.toml");
        save_config(Some(&new), &wrap_update(false)).unwrap();
        assert_eq!(
            fs::metadata(&new).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn follows_relative_and_absolute_symlink_chains_without_replacing_links() {
        let directory = TempDir::new().unwrap();
        let dotfiles = directory.path().join("dotfiles");
        let config_dir = directory.path().join("config");
        fs::create_dir(&dotfiles).unwrap();
        fs::create_dir(&config_dir).unwrap();
        let target = dotfiles.join("riff.toml");
        fs::write(&target, "# managed palette\ntheme = 'nord'\nwrap = false\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        let intermediate = config_dir.join("linked.toml");
        symlink(&target, &intermediate).unwrap();
        let link = config_dir.join("config.toml");
        symlink("linked.toml", &link).unwrap();
        assert_eq!(save_config(Some(&link), &wrap_update(true)).unwrap(), link);
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("linked.toml"));
        assert_eq!(fs::read_link(&intermediate).unwrap(), target);
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "# managed palette\ntheme = 'nord'\nwrap = true\n"
        );
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert!(load_config(Some(&link)).unwrap().wrap);
    }

    #[test]
    fn dangling_symlinks_create_the_target_and_keep_the_link() {
        let directory = TempDir::new().unwrap();
        let link = directory.path().join("config.toml");
        let relative_target = Path::new("dotfiles/riff.toml");
        symlink(relative_target, &link).unwrap();
        save_config(Some(&link), &wrap_update(true)).unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), relative_target);
        assert!(
            load_config(Some(&directory.path().join(relative_target)))
                .unwrap()
                .wrap
        );
    }

    #[test]
    fn parent_directory_symlinks_are_preserved() {
        let directory = TempDir::new().unwrap();
        let real = directory.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = directory.path().join("linked-directory");
        symlink("real", &link).unwrap();
        save_config(Some(&link.join("config.toml")), &wrap_update(true)).unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("real"));
        assert!(load_config(Some(&real.join("config.toml"))).unwrap().wrap);
    }

    #[test]
    fn symlink_errors_leave_links_and_invalid_targets_untouched() {
        let directory = TempDir::new().unwrap();
        let target = directory.path().join("target.toml");
        let link = directory.path().join("config.toml");
        fs::write(&target, "unknown = true\n").unwrap();
        symlink("target.toml", &link).unwrap();
        let error = save_config(Some(&link), &wrap_update(true)).unwrap_err();
        assert!(error.contains(&link.display().to_string()), "{error}");
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("target.toml"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "unknown = true\n");

        let cycle = directory.path().join("cycle.toml");
        symlink("cycle.toml", &cycle).unwrap();
        let error = save_config(Some(&cycle), &wrap_update(true)).unwrap_err();
        assert!(error.contains(&cycle.display().to_string()), "{error}");
        assert!(error.contains("symlink"), "{error}");
        assert_eq!(fs::read_link(cycle).unwrap(), Path::new("cycle.toml"));
    }

    #[test]
    fn unwritable_directory_errors_leave_configuration_unchanged() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("config.toml");
        let original = "# untouched\nwrap = false\n";
        fs::write(&path, original).unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o500)).unwrap();
        // Root/capability-enabled test runners can bypass mode bits. Probe
        // without touching the config, then always restore directory access.
        let can_create = tempfile::NamedTempFile::new_in(directory.path()).is_ok();
        let result = save_config(Some(&path), &wrap_update(true));
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        if !can_create {
            let error = result.unwrap_err();
            assert!(error.contains(&path.display().to_string()), "{error}");
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
            assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        }
    }
}

// Environment-sensitive checks run in fresh test subprocesses. No process-wide
// environment mutation (unsafe in Rust 2024) or shared cwd changes are needed.
fn child(directory: &Path, case: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "isolated_config_environment_child",
            "--nocapture",
        ])
        .current_dir(directory)
        .env("RIFF_CONFIG_PERSISTENCE_CASE", case)
        .env("RIFF_CONFIG_PERSISTENCE_ROOT", directory)
        .env("HOME", directory.join("home"))
        .env_remove("XDG_CONFIG_HOME");
    command
}

fn assert_child(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "child stdout: {}\nchild stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn relative_explicit_paths_are_not_redirected_to_the_default() {
    let directory = TempDir::new().unwrap();
    assert_child(&mut child(directory.path(), "relative-explicit"));
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn xdg_absolute_override_and_home_fallbacks_are_isolated_and_lazy() {
    for xdg in [Some("absolute"), Some("relative"), Some(""), None] {
        let directory = TempDir::new().unwrap();
        let root = directory.path();
        let expected = if xdg == Some("absolute") {
            root.join("xdg/riff/config.toml")
        } else {
            root.join("home/.config/riff/config.toml")
        };
        let mut command = child(root, "default-round-trip");
        command.env("RIFF_CONFIG_PERSISTENCE_EXPECTED", expected);
        if let Some(xdg) = xdg {
            if xdg == "absolute" {
                command.env("XDG_CONFIG_HOME", root.join("xdg"));
            } else {
                command.env("XDG_CONFIG_HOME", xdg);
            }
        }
        assert_child(&mut command);
        assert!(!root.join("relative").exists());
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn unavailable_home_path_returns_a_helpful_error_without_relative_writes() {
    let directory = TempDir::new().unwrap();
    assert_child(
        child(directory.path(), "no-directory")
            .env("HOME", "relative-home")
            .env("XDG_CONFIG_HOME", "relative-xdg"),
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn unset_or_empty_home_uses_native_home_discovery_without_writing() {
    let directory = TempDir::new().unwrap();
    assert_child(child(directory.path(), "native-home").env_remove("HOME"));
    assert_child(child(directory.path(), "native-home").env("HOME", ""));
}

#[cfg(target_os = "macos")]
#[test]
fn macos_uses_application_support_and_ignores_xdg() {
    let directory = TempDir::new().unwrap();
    assert_child(
        child(directory.path(), "default-round-trip")
            .env("XDG_CONFIG_HOME", directory.path().join("ignored-xdg"))
            .env(
                "RIFF_CONFIG_PERSISTENCE_EXPECTED",
                directory
                    .path()
                    .join("home/Library/Application Support/riff/config.toml"),
            ),
    );
}

#[cfg(windows)]
#[test]
fn windows_uses_native_roaming_appdata_and_ignores_xdg() {
    let directory = TempDir::new().unwrap();
    assert_child(
        child(directory.path(), "native-windows")
            .env("XDG_CONFIG_HOME", directory.path().join("ignored-xdg")),
    );
}

#[test]
fn isolated_config_environment_child() {
    let Ok(case) = std::env::var("RIFF_CONFIG_PERSISTENCE_CASE") else {
        return;
    };
    let root = PathBuf::from(std::env::var_os("RIFF_CONFIG_PERSISTENCE_ROOT").unwrap());
    match case.as_str() {
        "default-round-trip" => {
            let expected =
                PathBuf::from(std::env::var_os("RIFF_CONFIG_PERSISTENCE_EXPECTED").unwrap());
            assert_eq!(default_config_path(), Some(expected.clone()));
            assert!(!expected.parent().unwrap().exists());
            assert!(!load_config(None).unwrap().wrap);
            assert!(!expected.parent().unwrap().exists());
            assert_eq!(
                save_config(None, &SettingsUpdate::default()).unwrap(),
                expected
            );
            assert!(!expected.parent().unwrap().exists());
            assert_eq!(save_config(None, &wrap_update(true)).unwrap(), expected);
            assert_eq!(fs::read_to_string(&expected).unwrap(), "wrap = true\n");
            assert!(load_config(None).unwrap().wrap);
            let explicit = root.join("explicit/config.toml");
            save_config(Some(&explicit), &wrap_update(false)).unwrap();
            assert!(!load_config(Some(&explicit)).unwrap().wrap);
            assert!(load_config(None).unwrap().wrap);
        }
        "relative-explicit" => {
            let relative = Path::new("nested/config.toml");
            assert_eq!(
                save_config(Some(relative), &wrap_update(true)).unwrap(),
                relative
            );
            assert!(load_config(Some(relative)).unwrap().wrap);
            assert!(root.join(relative).is_file());
            assert!(!root.join("home").exists());
        }
        "no-directory" => {
            assert_eq!(default_config_path(), None);
            assert!(!load_config(None).unwrap().wrap);
            let error = save_config(None, &wrap_update(true)).unwrap_err();
            assert!(error.contains("configuration directory"), "{error}");
            assert!(error.contains("--config"), "{error}");
        }
        "native-home" => {
            let expected = dirs::home_dir()
                .filter(|home| home.is_absolute())
                .map(|home| home.join(".config/riff/config.toml"));
            assert_eq!(default_config_path(), expected);
        }
        "native-windows" => {
            assert_eq!(
                default_config_path(),
                dirs::config_dir().map(|root| root.join("riff/config.toml"))
            );
        }
        _ => panic!("unknown isolated config case: {case}"),
    }
}
