//! Scoped native subscriptions; ignored working-tree subtrees are never walked.

use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{input::Input, patch::sanitize};

#[derive(Default)]
pub(super) struct Plan {
    directories: BTreeSet<PathBuf>,
    // Ancestors observe only changes on the route to an input, not siblings.
    content: BTreeSet<PathBuf>,
    metadata: BTreeSet<PathBuf>,
}

impl Plan {
    fn directory(&mut self, directory: &Path, missing_ok: bool) -> Result<(), String> {
        let mut path = directory;
        loop {
            match fs::metadata(path) {
                Ok(metadata) if metadata.is_dir() => break,
                Ok(_) => return Err(format!("Cannot watch {}: not a directory", path.display())),
                Err(error) if missing_ok && error.kind() == io::ErrorKind::NotFound => {
                    path = path
                        .parent()
                        .ok_or("Watch directory has no existing ancestor.")?;
                }
                Err(error) => return Err(failure(path, error)),
            }
        }
        let canonical = path.canonicalize().map_err(|e| failure(path, e))?;
        self.content.insert(canonical.clone());
        // Both lexical and resolved ancestors matter when a symlink is replaced.
        for ancestor in path.ancestors().chain(canonical.ancestors()) {
            let resolved = ancestor.canonicalize().map_err(|e| failure(ancestor, e))?;
            self.directories.insert(resolved);
        }
        Ok(())
    }

    // Only selected comparison directories and Git metadata are enumerated.
    // Native watches remain nonrecursive so new directories are validated too.
    fn tree(
        &mut self,
        directory: &Path,
        visited: &mut BTreeSet<PathBuf>,
        metadata: bool,
    ) -> Result<(), String> {
        self.directory(directory, true)?;
        let canonical = match directory.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(failure(directory, error)),
        };
        if !visited.insert(canonical) {
            return Ok(());
        }
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(failure(directory, error)),
        };
        for entry in entries {
            let entry = entry.map_err(|e| failure(directory, e))?;
            let kind = entry.file_type().map_err(|e| failure(&entry.path(), e))?;
            // Git may relocate its metadata through directory symlinks. A
            // comparison's nested symlinks, however, are diffed as links by Git.
            let linked_directory = metadata
                && kind.is_symlink()
                && fs::metadata(entry.path())
                    .map_err(|e| failure(&entry.path(), e))?
                    .is_dir();
            if kind.is_dir() || linked_directory {
                self.tree(&entry.path(), visited, metadata)?;
            }
        }
        Ok(())
    }
}

pub(super) struct Subscriptions {
    watcher: RecommendedWatcher,
    directories: BTreeSet<PathBuf>,
    content: Arc<Mutex<BTreeSet<PathBuf>>>,
    metadata: BTreeSet<PathBuf>,
}

pub(super) fn start(
    source: &Input,
    cwd: &Path,
    mut callback: impl FnMut(notify::Result<Event>) + Send + 'static,
) -> Result<Subscriptions, String> {
    let plan = plan(source, cwd, true)?;
    let content = Arc::new(Mutex::new(plan.content.clone()));
    let observed = Arc::clone(&content);
    let watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
        let relevant = match &event {
            Ok(event) if !event.need_rescan() && !event.paths.is_empty() => {
                let content = observed.lock().unwrap_or_else(|e| e.into_inner());
                event.paths.iter().any(|path| {
                    path.parent().is_some_and(|parent| content.contains(parent))
                        || content.iter().any(|directory| directory.starts_with(path))
                })
            }
            _ => true,
        };
        if relevant {
            callback(event);
        }
    })
    .map_err(|e| format!("Cannot create filesystem watcher: {e}"))?;
    let mut subscriptions = Subscriptions {
        watcher,
        directories: BTreeSet::new(),
        content,
        metadata: BTreeSet::new(),
    };
    subscriptions.refresh(plan)?;
    Ok(subscriptions)
}

impl Subscriptions {
    /// Some(rebind) requests discovery; directory replacements also need a
    /// catch-up read even when the set of subscribed path names stays identical.
    pub(super) fn refresh_for(&self, event: &Event) -> Option<bool> {
        let rebind = event.need_rescan()
            || event.paths.is_empty()
            || event
                .paths
                .iter()
                .any(|path| self.directories.iter().any(|dir| dir.starts_with(path)));
        (rebind
            || !matches!(
                event.kind,
                notify::EventKind::Modify(notify::event::ModifyKind::Data(_))
            )
            || event
                .paths
                .iter()
                .any(|path| self.metadata.iter().any(|root| path.starts_with(root))))
        .then_some(rebind)
    }

    /// Retain the same callback/epoch and ancestor coverage, and rebind existing
    /// paths too: a deleted/recreated directory may have the same spelling.
    pub(super) fn refresh(&mut self, mut plan: Plan) -> Result<bool, String> {
        let changed = self.directories != plan.directories;
        self.content
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(plan.content.iter().cloned());
        for path in self.directories.difference(&plan.directories) {
            if let Err(error) = self.watcher.unwatch(path)
                && !matches!(
                    error.kind,
                    notify::ErrorKind::WatchNotFound | notify::ErrorKind::PathNotFound
                )
            {
                return Err(failure(path, error));
            }
        }
        let mut raced = false;
        for path in &plan.directories.clone() {
            if let Err(error) = self.watcher.watch(path, RecursiveMode::NonRecursive) {
                // Removal during discovery is recoverable through the retained
                // ancestor. Never treat permission/resource failures as success.
                if matches!(&error.kind, notify::ErrorKind::PathNotFound)
                    || matches!(&error.kind, notify::ErrorKind::Io(e) if e.kind() == io::ErrorKind::NotFound)
                {
                    plan.directories.remove(path);
                    raced = true;
                } else {
                    return Err(failure(path, error));
                }
            }
        }
        *self.content.lock().unwrap_or_else(|e| e.into_inner()) = plan.content;
        self.directories = plan.directories;
        self.metadata = plan.metadata;
        Ok(changed || raced)
    }
}

pub(super) fn plan(source: &Input, cwd: &Path, initial: bool) -> Result<Plan, String> {
    let mut plan = Plan::default();
    match source {
        Input::Files(left, right) => {
            for operand in [left, right] {
                let path = cwd.join(operand);
                plan.directory(
                    path.parent()
                        .ok_or("Cannot watch a path without a parent.")?,
                    !initial,
                )?;
                match path.canonicalize() {
                    Ok(target) => {
                        plan.directory(
                            target.parent().ok_or("Watch target has no parent.")?,
                            true,
                        )?;
                        if target.is_dir() {
                            plan.tree(&target, &mut BTreeSet::new(), false)?;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(failure(&path, error)),
                }
            }
        }
        Input::WorkingTree | Input::Staged | Input::Commit(_) => {
            let git = Git {
                cwd,
                clear_env: &[],
            };
            // Git defines which exported variables belong to this repository,
            // not its submodules (notably GIT_INDEX_FILE and GIT_WORK_TREE).
            let local_env = git.run(&["rev-parse", "--local-env-vars"])?;
            let local_env: Vec<&str> = std::str::from_utf8(&local_env)
                .map_err(|_| "Git returned invalid environment variable names.")?
                .lines()
                .collect();
            plan.repository(
                &git,
                matches!(source, Input::Commit(_)),
                &local_env,
                &mut BTreeSet::new(),
            )?;
        }
        Input::Stdin => return Err("Stdin patches cannot be reloaded or watched.".into()),
    }
    Ok(plan)
}

fn failure(path: &Path, error: impl std::fmt::Display) -> String {
    format!(
        "Cannot watch {}: {error}. Check access to this required input or Git metadata; manual reload remains available.",
        path.display()
    )
}

impl Plan {
    fn untracked_tree(&mut self, directory: &Path, git: &Git<'_>) -> Result<(), String> {
        self.directory(directory, true)?;
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(failure(directory, error)),
        };
        for entry in entries {
            let entry = entry.map_err(|e| failure(directory, e))?;
            let path = entry.path();
            // Check the name BEFORE inspecting its type or descending. Git's
            // exclusion rules, including nested .gitignore files, remain authoritative.
            let output = git
                .command()
                .args(["check-ignore", "-q", "--"])
                .arg(&path)
                .output()
                .map_err(|e| failure(&path, e))?;
            match output.status.code() {
                Some(0) => continue,
                Some(1) => {}
                _ => {
                    return Err(failure(
                        &path,
                        sanitize(String::from_utf8_lossy(&output.stderr).trim()),
                    ));
                }
            }
            if entry.file_type().map_err(|e| failure(&path, e))?.is_dir() {
                self.untracked_tree(&path, git)?;
            }
        }
        Ok(())
    }

    fn repository(
        &mut self,
        git: &Git<'_>,
        allow_bare: bool,
        local_env: &[&str],
        repositories: &mut BTreeSet<PathBuf>,
    ) -> Result<(), String> {
        let git_dir = git.path(&["--absolute-git-dir"])?;
        if !repositories.insert(git_dir.canonicalize().map_err(|e| failure(&git_dir, e))?) {
            return Ok(());
        }
        let common_dir = git.path(&["--git-common-dir"])?;
        match git.path(&["--show-toplevel"]) {
            Ok(root) => {
                self.directory(&root, true)?;
                // Stage records identify gitlinks without probing ordinary files.
                // Split only the first tab: literal names may contain tabs/newlines.
                let files = git.run(&["ls-files", "--cached", "--stage", "--full-name", "-z"])?;
                let mut parents = BTreeSet::new();
                let mut submodules = BTreeSet::new();
                for record in files.split(|byte| *byte == 0).filter(|r| !r.is_empty()) {
                    let mut fields = record.splitn(2, |byte| *byte == b'\t');
                    let header = fields.next().ok_or("Missing Git index mode.")?;
                    let name = fields.next().ok_or("Missing Git index path.")?;
                    let path = root.join(bytes_path(name)?);
                    if header.starts_with(b"160000 ") {
                        submodules.insert(path.clone());
                    }
                    parents.insert(
                        path.parent()
                            .ok_or("Tracked file has no parent.")?
                            .to_path_buf(),
                    );
                }
                for parent in parents {
                    // Ancestor .gitattributes can affect a tracked input too.
                    // Observe entries, never recursively inspect their siblings.
                    for ancestor in parent.ancestors().take_while(|p| p.starts_with(&root)) {
                        self.directory(ancestor, true)?;
                    }
                }
                if !git.clear_env.is_empty() {
                    // Untracked files also make a submodule dirty in its parent.
                    // Let Git exclude ignored roots before inspecting any of them.
                    let others = git.run(&[
                        "ls-files",
                        "--others",
                        "--exclude-standard",
                        "--directory",
                        "--full-name",
                        "-z",
                    ])?;
                    for name in others.split(|byte| *byte == 0).filter(|n| !n.is_empty()) {
                        let path = root.join(bytes_path(name)?);
                        if name.ends_with(b"/") {
                            self.untracked_tree(&path, git)?;
                        } else {
                            for parent in path
                                .ancestors()
                                .skip(1)
                                .take_while(|p| p.starts_with(&root))
                            {
                                self.directory(parent, true)?;
                            }
                        }
                    }
                }
                for path in submodules {
                    // An empty/uninitialized gitlink can acquire .git later.
                    // Subscribe to its root, but never enumerate its siblings.
                    self.directory(&path, true)?;
                    let marker = path.join(".git");
                    match fs::symlink_metadata(&marker) {
                        Ok(_) => self.repository(
                            &Git {
                                cwd: &path,
                                clear_env: local_env,
                            },
                            false,
                            local_env,
                            repositories,
                        )?,
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(failure(&marker, error)),
                    }
                }
            }
            Err(_) if allow_bare => {}
            Err(error) => return Err(error),
        }
        let mut visited = BTreeSet::new();
        for directory in [&git_dir, &common_dir] {
            // HEAD, index, config, packed-refs and their atomic replacements.
            self.directory(directory, false)?;
            self.metadata.insert(
                directory
                    .canonicalize()
                    .map_err(|e| failure(directory, e))?,
            );
            // Do not walk hooks, unrelated worktrees, or inactive modules.
            for name in ["refs", "logs", "objects", "info", "reftable"] {
                self.tree(&directory.join(name), &mut visited, true)?;
            }
        }
        let objects = git.path(&["--git-path", "objects"])?;
        self.tree(&objects, &mut visited, true)?;
        self.metadata.insert(objects);
        // An external index parent must not expose its unrelated siblings.
        let index = git.path(&["--git-path", "index"])?;
        let parent = index.parent().ok_or("Git index has no parent directory.")?;
        self.directory(parent, true)?;
        self.metadata
            .insert(parent.canonicalize().map_err(|e| failure(parent, e))?);
        Ok(())
    }
}

struct Git<'a> {
    cwd: &'a Path,
    clear_env: &'a [&'a str],
}

impl Git<'_> {
    fn command(&self) -> Command {
        let mut command = Command::new("git");
        for name in self.clear_env {
            command.env_remove(name);
        }
        command.current_dir(self.cwd).env("LC_ALL", "C").args([
            "--no-pager",
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "diff.autoRefreshIndex=false",
        ]);
        command
    }

    fn run(&self, args: &[&str]) -> Result<Vec<u8>, String> {
        let output = self
            .command()
            .args(args)
            .output()
            .map_err(|e| format!("Cannot discover Git watch directories: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "Cannot discover Git watch directories: {}",
                sanitize(String::from_utf8_lossy(&output.stderr).trim())
            ));
        }
        Ok(output.stdout)
    }

    fn path(&self, args: &[&str]) -> Result<PathBuf, String> {
        let mut command = vec!["rev-parse"];
        command.extend_from_slice(args);
        let output = self.run(&command)?;
        // Strip only Git's record terminator, not whitespace in a directory name.
        let path = bytes_path(output.strip_suffix(b"\n").unwrap_or(&output))?;
        if path.as_os_str().is_empty() {
            return Err("Git returned an empty watch directory.".into());
        }
        Ok(self.cwd.join(path))
    }
}

fn bytes_path(bytes: &[u8]) -> Result<PathBuf, String> {
    #[cfg(unix)]
    {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        Ok(PathBuf::from(OsString::from_vec(bytes.to_vec())))
    }
    #[cfg(not(unix))]
    Ok(PathBuf::from(
        std::str::from_utf8(bytes).map_err(|_| "Git watch path is not valid UTF-8.")?,
    ))
}
