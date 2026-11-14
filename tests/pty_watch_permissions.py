#!/usr/bin/env python3
"""Permission/watch PTYs using only isolated, test-owned repositories and files."""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from typing import Callable

from pty_smoke import ARTIFACTS, ROOT, Session, full_screen, plain

CTRL_W: bytes = b"\x1b[119;5u"
CTRL_R: bytes = b"\x1b[114;5u"
CHDIR_EXEC: str = (
    "import os, sys\n"
    "directory: str = sys.argv[1]\n"
    "executable: str = sys.argv[2]\n"
    "names: list[str] = list(os.environ)\n"
    "name: str\n"
    "for name in names:\n"
    "    if name.startswith('GIT_'): os.environ.pop(name)\n"
    "os.environ['GIT_CONFIG_NOSYSTEM'] = '1'\n"
    "os.environ['GIT_CONFIG_GLOBAL'] = os.devnull\n"
    "os.chdir(directory)\n"
    "os.execv(executable, sys.argv[2:])\n"
)


def git(root: Path, *args: str) -> bytes:
    environment: dict[str, str] = {key: value for key, value in os.environ.items()
                                    if not key.startswith("GIT_")}
    environment.update({"GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull})
    completed: subprocess.CompletedProcess[bytes] = subprocess.run(
        ["git", "--no-optional-locks", "-c", "core.fsmonitor=false", "-c",
         "core.hooksPath=/dev/null", "-c", "user.name=permission fixture", "-c",
         "user.email=watch@example.invalid", "-c", "commit.gpgSign=false", *args],
        cwd=root, env=environment, check=True, capture_output=True)
    return completed.stdout


def session_at(binary: Path, root: Path, config_root: Path, args: list[str]) -> Session:
    return Session(Path(sys.executable).resolve(),
                   ["-c", CHDIR_EXEC, str(root), str(binary), "--no-syntax", *args],
                   config_root=config_root)


def repaint(session: Session) -> str:
    width: int = session.width + (1 if session.width % 2 == 0 else -1)
    session.set_size(width, session.height)
    return plain(session.collect())


def wait_screen(session: Session, predicate: Callable[[str], bool], label: str) -> str:
    deadline: float = time.monotonic() + 10
    screen: str = ""
    while time.monotonic() < deadline:
        screen = repaint(session)
        if predicate(screen):
            screen = repaint(session)
            if predicate(screen):
                return screen
    raise AssertionError(f"{label}: {screen!r}; {plain(bytes(session.transcript))[-3000:]!r}")


def watch_on(session: Session) -> None:
    wait_screen(session, lambda screen: "watch:on" in screen and "loading" not in screen,
                "watch enable acknowledgement")


def automatic(session: Session, needle: str) -> None:
    deadline: float = time.monotonic() + 10
    output: str = ""
    while time.monotonic() < deadline:
        output += plain(session.collect(0.1))
        if needle in output:
            return
    raise AssertionError(f"no automatic update for {needle!r}: {output!r}")


def lock_owned(path: Path) -> int | None:
    path.mkdir(parents=True, exist_ok=True)
    original: int = path.stat().st_mode & 0o7777
    path.chmod(0)
    try:
        list(path.iterdir())
    except PermissionError:
        return original
    path.chmod(original)
    assert os.geteuid() == 0, "non-root must reproduce permission denial"
    print("SKIP: effective root bypasses mode000 permission fixture")
    return None


def fixture_sessions(binary: Path, root: Path) -> list[dict[str, object]]:
    repository: Path = root / "repository"
    repository.mkdir()
    git(repository, "init", "-q")
    source: Path = repository / "src/file[1].py"
    source.parent.mkdir()
    source.write_text("base\n")
    (repository / ".gitignore").write_text("ignored-cache/\n")
    git(repository, "add", ".")
    git(repository, "commit", "-qm", "fixture")
    source.write_text("initial difference\n")
    blocked: Path = repository / "ignored-cache/private"
    original: int | None = lock_owned(blocked)
    if original is None:
        return []
    before: tuple[int, ...] = signature(blocked.stat())
    index: bytes = (repository / ".git/index").read_bytes()
    config_root: Path = root / "preferences"
    config: Path = config_root / "riff/config.toml"
    config.parent.mkdir(parents=True)
    config_text: str = "# preserve comment\nwatch = false # accepted preference\nwrap = true\n"
    config.write_text(config_text)
    results: list[dict[str, object]] = []
    session: Session = session_at(binary, repository, config_root,
                                  ["--no-watch", "--view", "unified", "--", ":(literal)src/file[1].py"])
    try:
        session.ready()
        assert "ctrl+w watch" in full_screen(session)
        session.send(b"?")
        assert "ctrl+w (no Shift)" in full_screen(session)
        session.send(b"?")
        # Enhanced keyboard protocol can distinguish Shift; do not add aliases.
        keys: bytes
        for keys in [b"\x1b[119;6u", b"\x1b[87;5u", b"\x1b[87;6u", b"W", b"a"]:
            session.send(keys)
            assert "watch:off" in full_screen(session), f"unexpected alias {keys!r}"
            assert config.read_text() == config_text
        session.send(CTRL_W)
        watch_on(session)
        assert config.read_text() == config_text.replace("watch = false", "watch = true")
        source.write_text("PERMISSION_FIXTURE_AUTOMATICALLY_REFRESHED\n")
        automatic(session, "PERMISSION_FIXTURE_AUTOMATICALLY_REFRESHED")
        session.send(b"\x17")  # Legacy lowercase ctrl+w, with no Shift.
        wait_screen(session, lambda screen: "watch:off" in screen, "legacy ctrl+w off")
        assert config.read_text() == config_text
        session.send(b"\x17")
        watch_on(session)
        assert config.read_text() == config_text.replace("watch = false", "watch = true")
        results.append(session.finish(b"q", "watch-permissions-git-lowercase"))
        assert signature(blocked.stat()) == before
        assert (repository / ".git/index").read_bytes() == index
    finally:
        session.close()
        blocked.chmod(original)

    denied_config_root: Path = root / "denied-preferences"
    denied_config: Path = denied_config_root / "riff/config.toml"
    denied_config.parent.mkdir(parents=True)
    denied_config.write_text(config_text)
    session = session_at(binary, repository, denied_config_root,
                         ["--no-watch", "--view", "unified", "--", ":(literal)src/file[1].py"])
    required_original: int | None = None
    try:
        session.ready()
        required_original = lock_owned(source.parent)
        if required_original is not None:
            session.set_size(240, session.height)
            session.collect()
            session.send(CTRL_W)
            wait_screen(session, lambda screen: "Permission denied" in screen and "watch:off" in screen,
                        "failed enable remains nonfatal")
            assert denied_config.read_text() == config_text, "failed enable persisted a preference"
            source.parent.chmod(required_original)
            required_original = None
            source.write_text("MANUAL_RELOAD_AFTER_PERMISSION_FAILURE\n")
            session.send(CTRL_R)
            wait_screen(session, lambda screen: "MANUAL_RELOAD_AFTER_PERMISSION_FAILURE" in screen,
                        "manual reload remains usable")
            session.send(b"?")
            assert " Help " in full_screen(session)
            session.send(b"?")
        results.append(session.finish(b"q", "watch-permissions-failed-enable-manual"))
    finally:
        session.close()
        if required_original is not None:
            source.parent.chmod(required_original)

    pair: Path = root / "pair"
    pair.mkdir()
    left: Path = pair / "left"
    right: Path = pair / "right"
    left.write_text("base\n")
    right.write_text("initial\n")
    sibling: Path = pair / "unrelated/private"
    sibling_original: int | None = lock_owned(sibling)
    if sibling_original is None:
        return results
    session = session_at(binary, pair, root / "files-config",
                         ["--watch", "--view", "unified", "--files", "left", "right"])
    try:
        session.ready()
        watch_on(session)
        replacement: Path = pair / "replacement"
        replacement.write_text("ATOMIC_REGULAR_PAIR_UPDATE\n")
        replacement.replace(right)
        automatic(session, "ATOMIC_REGULAR_PAIR_UPDATE")
        results.append(session.finish(b"q", "watch-permissions-file-pair"))
    finally:
        session.close()
        sibling.chmod(sibling_original)
    return results


def signature(state: os.stat_result) -> tuple[int, ...]:
    return (state.st_dev, state.st_ino, state.st_mode, state.st_uid, state.st_gid,
            state.st_nlink, state.st_size, state.st_atime_ns, state.st_mtime_ns, state.st_ctime_ns)


def main() -> None:
    assert len(sys.argv) <= 2, "usage: pty_watch_permissions.py [path/to/riff]"
    binary: Path = Path(sys.argv[1]).resolve() if len(sys.argv) == 2 else ROOT / "target/release/riff"
    assert binary.is_file(), f"build {binary} first"
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    temporary: str
    with tempfile.TemporaryDirectory(prefix="watch-permissions-", dir=ARTIFACTS) as temporary:
        root: Path = Path(temporary).resolve()
        results: list[dict[str, object]] = fixture_sessions(binary, root)
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
