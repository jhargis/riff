#!/usr/bin/env python3
"""Real watch/reload PTYs, using only tempfile-owned files, repos, and config.

Run after building: python3 tests/pty_reload.py target/debug/riff
Session retains the existing raw-mode, keyboard-stack, and shutdown assertions.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
from typing import Callable

from pty_smoke import ARTIFACTS, ROOT, Session, full_screen, plain, rendered_rows

CHDIR_EXEC: str = (
    "import os, sys\n"
    "directory: str = sys.argv[1]\n"
    "executable: str = sys.argv[2]\n"
    "arguments: list[str] = sys.argv[2:]\n"
    "os.chdir(directory)\n"
    "os.execv(executable, arguments)\n"
)
CTRL_W: bytes = b"\x1b[119;5u"
CTRL_R: bytes = b"\x1b[114;5u"


def session_at(binary: Path, args: list[str], config: Path, cwd: Path,
               env_overrides: dict[str, str] | None = None) -> Session:
    return Session(Path(sys.executable).resolve(),
                   ["-c", CHDIR_EXEC, str(cwd), str(binary), "--no-syntax", *args],
                   config_root=config,
                   env_overrides={"GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
                                  **(env_overrides or {})})


def wait_screen(session: Session, predicate: Callable[[str], bool], label: str) -> str:
    """Bound a failing test, not the implementation's reload operation."""
    deadline: float = time.monotonic() + 10
    screen: str = ""
    while time.monotonic() < deadline:
        screen = full_screen(session)
        if predicate(screen):
            # A result can arrive during the resize repaint; discard that
            # initial frame plus incremental update before inspecting totals.
            screen = full_screen(session)
            if predicate(screen):
                return screen
    raise AssertionError(f"{label}: {screen!r}; transcript: {plain(bytes(session.transcript))[-3000:]!r}")


def wait_idle_output(session: Session, needle: str) -> None:
    """Require a background update without sending keys or resize events."""
    deadline: float = time.monotonic() + 10
    output: str = ""
    while time.monotonic() < deadline:
        output += plain(session.collect(0.1))
        if needle in output:
            return
    raise AssertionError(f"idle watch did not render {needle!r}: {output!r}")


def watch_on(session: Session) -> str:
    return wait_screen(session, lambda screen: "watch:on" in screen and "loading" not in screen,
                       "watch acknowledgement")


def git(root: Path, *args: str) -> str:
    completed: subprocess.CompletedProcess[str] = subprocess.run(
        ["git", "-C", str(root), *args], check=True, capture_output=True, text=True,
        env={**os.environ, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull})
    return completed.stdout.strip()


def atomic_text(path: Path, text: str) -> None:
    replacement: Path = path.with_name(path.name + ".replacement")
    replacement.write_text(text)
    replacement.replace(path)


def file_sessions(binary: Path, root: Path, results: list[dict[str, object]]) -> None:
    old: Path = root / "before.txt"
    new: Path = root / "after.txt"
    old.write_text("same\n")
    new.write_text("same\n")
    config_root: Path = root / "file-config"
    config: Path = config_root / "riff/config.toml"
    args: list[str] = ["--files", str(old), str(new), "--view", "unified"]
    session: Session = session_at(binary, [*args, "--watch"], config_root, root)
    try:
        session.ready()
        screen: str = watch_on(session)
        assert "No differences." in screen, screen
        assert not config.exists(), "--watch wrote a startup override"
        atomic_text(new, "FILE_WATCH_FIRST\n")
        wait_idle_output(session, "FILE_WATCH_FIRST")
        wait_screen(session, lambda text: "FILE_WATCH_FIRST" in text, "initial empty watch")
        new.unlink()
        wait_screen(session, lambda text: "Reload error:" in text, "missing file error")
        screen = full_screen(session)
        assert "FILE_WATCH_FIRST" in screen and "watch:on" in screen, screen
        atomic_text(new, "FILE_WATCH_RECOVERED\n")
        wait_screen(session, lambda text: "FILE_WATCH_RECOVERED" in text and "Reload error:" not in text,
                    "watch deletion/recreation recovery")
        session.send(CTRL_W)
        wait_screen(session, lambda text: "watch:off" in text, "disable watch")
        assert config.read_text() == "watch = false\n", config.read_text()
        atomic_text(new, "FILE_MANUAL_ONLY\n")
        session.collect(0.5)
        assert "FILE_MANUAL_ONLY" not in full_screen(session), "watch off still reloaded"
        session.send(b"r")
        wait_screen(session, lambda text: "FILE_MANUAL_ONLY" in text, "bare r reload")
        atomic_text(new, "FILE_CONTROL_R\n")
        session.send(CTRL_R)
        wait_screen(session, lambda text: "FILE_CONTROL_R" in text, "enhanced Ctrl-R reload")
        atomic_text(new, "same\n")
        session.send(b"\x12")  # Legacy Ctrl-R byte is also supported.
        wait_screen(session, lambda text: "No differences." in text, "empty reload stays open")
        session.send(b"\x17")  # Legacy Ctrl-W, persist true for the next launch.
        watch_on(session)
        assert config.read_text() == "watch = true\n"
        results.append(session.finish(b"q", "reload-files-empty-watch-recovery-manual"))
    finally:
        session.close()

    session = session_at(binary, args, config_root, root)
    try:
        session.ready()  # Empty, but remembered watch must keep the viewer open.
        watch_on(session)
        atomic_text(new, "FILE_REMEMBERED_WATCH\n")
        wait_screen(session, lambda text: "FILE_REMEMBERED_WATCH" in text, "saved watch across restart")
        assert config.read_text() == "watch = true\n"
        results.append(session.finish(b"q", "reload-watch-remembered"))
    finally:
        session.close()

    session = session_at(binary, [*args, "--no-watch"], config_root, root)
    try:
        session.ready()
        assert "watch:off" in full_screen(session)
        atomic_text(new, "FILE_NO_WATCH_OVERRIDE\n")
        session.collect(0.5)
        assert "FILE_NO_WATCH_OVERRIDE" not in full_screen(session)
        assert config.read_text() == "watch = true\n", "--no-watch rewrote saved watch"
        session.send(CTRL_R)
        wait_screen(session, lambda text: "FILE_NO_WATCH_OVERRIDE" in text, "manual with --no-watch")
        results.append(session.finish(b"q", "reload-no-watch-override"))
    finally:
        session.close()

    explicit: Path = root / "explicit.toml"
    original: str = ("# preserve unrelated settings\ntheme = 'nord' # custom choice\n"
                     "view = 'auto'\nwrap = true\nwatch = true # automatic\n"
                     "[themes.mine]\nextends = 'sunset'\nbackground = '#010203'\n")
    explicit.write_text(original)
    session = session_at(binary, [*args, "--config", str(explicit), "--no-wrap", "--theme", "dracula"],
                         config_root, root)
    try:
        session.ready()
        watch_on(session)
        session.send(CTRL_W)
        wait_screen(session, lambda text: "watch:off" in text, "explicit config watch save")
        assert explicit.read_text() == original.replace("watch = true", "watch = false")
        assert config.read_text() == "watch = true\n", "explicit config changed default config"
        results.append(session.finish(b"q", "reload-explicit-config-selective"))
    finally:
        session.close()


def git_sessions(binary: Path, root: Path, results: list[dict[str, object]]) -> None:
    repository: Path = root / "repo"
    repository.mkdir()
    git(repository, "init", "-q")
    git(repository, "config", "user.name", "Reload Test")
    git(repository, "config", "user.email", "reload@example.invalid")
    tracked: Path = repository / "tracked.txt"
    other: Path = repository / "other.txt"
    tracked.write_text("original\n")
    other.write_text("other\n")
    git(repository, "add", ".")
    git(repository, "commit", "-qm", "isolated fixture")
    config_root: Path = root / "git-config"
    session: Session = session_at(binary, ["tracked.txt", "--watch", "--view", "unified"],
                                  config_root, repository)
    try:
        session.ready()
        assert "No differences." in watch_on(session)
        other.write_text("UNRELATED_MUST_NOT_APPEAR\n")
        tracked.write_text("GIT_FILTERED_WATCH\n")
        wait_idle_output(session, "GIT_FILTERED_WATCH")
        screen: str = wait_screen(session, lambda text: "GIT_FILTERED_WATCH" in text,
                                  "working tree with original positional filter")
        assert "UNRELATED_MUST_NOT_APPEAR" not in screen and "1 files" in screen, screen
        tracked.unlink()
        wait_screen(session, lambda text: "+0 -1" in text and "Reload error:" not in text,
                    "deleted positional path remains a path")
        tracked.write_text("GIT_RECREATED_PATH\n")
        wait_screen(session, lambda text: "GIT_RECREATED_PATH" in text, "recreated positional path")
        results.append(session.finish(b"q", "reload-git-original-path-filter"))
    finally:
        session.close()

    session = session_at(binary, ["--staged", "--watch", "--view", "unified", "--", "tracked.txt"],
                         config_root, repository)
    try:
        session.ready()
        assert "No differences." in watch_on(session)
        git(repository, "add", "tracked.txt")
        wait_screen(session, lambda text: "GIT_RECREATED_PATH" in text, "staged index watch")
        tracked.write_text("GIT_UNSTAGED_NOT_SHOWN\n")
        session.collect(0.5)
        assert "GIT_UNSTAGED_NOT_SHOWN" not in full_screen(session), "staged query changed"
        session.send(CTRL_W)
        wait_screen(session, lambda text: "watch:off" in text, "staged manual mode")
        git(repository, "add", "tracked.txt")
        session.send(CTRL_R)
        wait_screen(session, lambda text: "GIT_UNSTAGED_NOT_SHOWN" in text, "manual original staged query")
        results.append(session.finish(b"q", "reload-git-staged-index"))
    finally:
        session.close()


def stdin_session(binary: Path, root: Path, results: list[dict[str, object]]) -> None:
    config_root: Path = root / "stdin-config"
    config: Path = config_root / "riff/config.toml"
    config.parent.mkdir(parents=True)
    config.write_text("watch = true # keep saved preference\n")
    patch: bytes = b"--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+STDIN_KEEP_DIFF\n"
    session: Session = Session(binary, ["--stdin", "--no-syntax"], patch, config_root=config_root)
    try:
        session.ready()
        assert "watch:unavailable" in full_screen(session)
        for command in [b"r", CTRL_R, CTRL_W]:
            session.send(command)
            screen: str = full_screen(session)
            assert "stdin patch cannot be reread" in screen and "STDIN_KEEP_DIFF" in screen, screen
            assert config.read_text() == "watch = true # keep saved preference\n"
        results.append(session.finish(b"q", "reload-stdin-unavailable-nonfatal"))
    finally:
        session.close()


def reject_stdin_watch_before_read(binary: Path, root: Path) -> None:
    # An open terminal stdin would block forever if the viewer read it before
    # rejecting explicit watch. No keystrokes or EOF are supplied here.
    session: Session = Session(binary, ["--stdin", "--watch"], config_root=root / "rejected-config")
    try:
        deadline: float = time.monotonic() + 10
        while time.monotonic() < deadline:
            session.collect(0.05)
            waited: int
            status: int
            waited, status = os.waitpid(session.pid, os.WNOHANG)
            if waited:
                session.status = status
                break
        assert session.status is not None, "explicit stdin watch blocked on stdin"
        assert os.waitstatus_to_exitcode(session.status) != 0
        message: str = plain(bytes(session.transcript))
        assert "--watch is unavailable for stdin patches" in message, message
        assert b"\x1b[?1049h" not in session.transcript, "rejected request opened a TUI"
        assert not (session.config_root / "riff/config.toml").exists()
    finally:
        session.close()


def parse_recovery_session(binary: Path, root: Path, results: list[dict[str, object]]) -> None:
    directory: Path = root / "parse-recovery"
    directory.mkdir()
    old: Path = directory / "old.txt"
    new: Path = directory / "new.txt"
    old.write_text("baseline\n")
    new.write_text("PARSE_LAST_GOOD\n")
    override: Path = directory / "override.patch"
    invocations: Path = directory / "invocations"
    git_executable: str | None = shutil.which("git")
    assert git_executable is not None, "Git is required for file comparisons"
    real_git: Path = Path(git_executable).resolve()
    shim: Path = directory / "bin/git"
    shim.parent.mkdir()
    # A process-local Git shim supplies malformed output; no real Git/config is
    # replaced. The actual controller/parser/UI path still handles every result.
    shim.write_text(
        f"#!{sys.executable}\n"
        "from pathlib import Path\nimport os, sys\n"
        f"override: Path = Path({str(override)!r})\n"
        f"invocations: Path = Path({str(invocations)!r})\n"
        f"real_git: str = {str(real_git)!r}\n"
        "if override.exists():\n"
        "    count: int = int(invocations.read_text()) if invocations.exists() else 0\n"
        "    invocations.write_text(str(count + 1))\n"
        "    sys.stdout.buffer.write(override.read_bytes())\n"
        "    sys.exit(0)\n"
        "os.execv(real_git, [real_git, *sys.argv[1:]])\n")
    shim.chmod(0o700)
    session: Session = session_at(
        binary, ["--files", old.name, new.name, "--no-watch", "--view", "unified"],
        root / "parse-config", directory,
        {"PATH": str(shim.parent) + os.pathsep + os.environ["PATH"]})
    try:
        session.ready()
        override.write_text("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-incomplete\n")
        for attempt in [1, 2]:
            session.send(CTRL_R)
            screen: str = wait_screen(
                session,
                lambda text: (invocations.exists() and invocations.read_text() == str(attempt)
                              and "Reload error: Cannot parse reloaded patch:" in text
                              and "PARSE_LAST_GOOD" in text and "loading" not in text),
                "malformed reload retains last good diff on every attempt")
            assert "watch:off" in screen and "1 files" in screen, screen
        override.unlink()
        new.write_text("PARSE_RECOVERED\n")
        session.send(b"r")
        wait_screen(session, lambda text: "PARSE_RECOVERED" in text and "Reload error:" not in text,
                    "manual recovery after repeated parse errors")
        assert not (session.config_root / "riff/config.toml").exists()
        results.append(session.finish(b"q", "reload-parse-error-retains-and-recovers"))
    finally:
        session.close()


def anchor_rows(session: Session) -> list[str]:
    # Drain incremental output, then resize horizontally for one complete frame.
    session.collect(0.1)
    session.set_size(121 if session.width == 120 else 120, session.height)
    return rendered_rows(session.collect(), session.width, session.height)


def wait_anchor_rows(session: Session, predicate: Callable[[list[str]], bool],
                     label: str) -> list[str]:
    deadline: float = time.monotonic() + 10
    rows: list[str] = []
    while time.monotonic() < deadline:
        rows = anchor_rows(session)
        if predicate(rows):
            return rows
    raise AssertionError(f"{label}: {rows!r}")


def anchor_session(binary: Path, root: Path, results: list[dict[str, object]],
                   case: str, view: str, wrapped: bool, watched: bool) -> None:
    label: str = (f"reload-anchor-{case}-{view}-"
                  f"{'wrapped' if wrapped else 'clipped'}-{'watch' if watched else 'manual'}")
    directory: Path = root / label
    directory.mkdir()
    lines: list[str] = [f"LINE_{index:03d}\n" for index in range(1, 221)]
    lines[100] = "SHARED_CONTEXT\n"
    old: Path = directory / "left"
    new: Path = directory / "right"
    old.write_text("".join(lines))
    lines[99] = "TARGET_EDITED\n"
    new.write_text("".join(lines))
    session: Session = session_at(
        binary, ["--files", old.name, new.name, "--view", view,
                 "--wrap" if wrapped else "--no-wrap", "--watch" if watched else "--no-watch"],
        directory / "config", directory)
    try:
        session.ready()
        if watched:
            watch_on(session)
        session.set_size(120, 6)
        session.collect()
        session.send(b"n" if case == "hunk" else b"/SHARED_CONTEXT\r")
        before: list[str] = anchor_rows(session)
        if case == "hunk":
            assert "@@ -97,7 +97,7 @@" in before[1], before
            revision: str = "PREPENDED\n" * 500 + "".join(lines)
        else:
            assert "SHARED_CONTEXT" in before[1] and "[1/1]" in before[-2], before
            inserted: list[str] = [f"INSERT_{index}\n" for index in range(20)]
            inserted[3] = "SHARED_CONTEXT\n"
            revision = "".join(lines[:97] + inserted + lines[97:])
        atomic_text(new, revision)
        if not watched:
            session.send(CTRL_R)
        if case == "hunk":
            wait_anchor_rows(session, lambda rows: "@@ -97,7 +597,7 @@" in rows[1],
                             "earlier insertion stole the current hunk")
            session.send(b"n")
            rows: list[str] = anchor_rows(session)
            assert "@@ -1,3 +1,503 @@" in rows[1], rows
            session.send(b"p")
            rows = anchor_rows(session)
            assert "@@ -97,7 +597,7 @@" in rows[1], rows
        else:
            rows = wait_anchor_rows(
                session, lambda rows: "SHARED_CONTEXT" in rows[1] and "[2/2]" in rows[-2],
                "inserted duplicate stole the selected context")
            if view == "unified":
                assert "101 121   SHARED_CONTEXT" in rows[1], rows
            else:
                assert "101   SHARED_CONTEXT" in rows[1] and "121   SHARED_CONTEXT" in rows[1], rows
            session.send(b"\r")
            assert "[1/2]" in anchor_rows(session)[-2]
        assert not (session.config_root / "riff/config.toml").exists(), "reload saved overrides"
        results.append(session.finish(b"q", label))
    finally:
        session.close()


def anchor_sessions(binary: Path, root: Path, results: list[dict[str, object]]) -> None:
    for case in ["hunk", "context"]:
        for view in ["unified", "split"]:
            for wrapped in [False, True]:
                for watched in [False, True]:
                    anchor_session(binary, root, results, case, view, wrapped, watched)


def slow_loader_session(binary: Path, root: Path, results: list[dict[str, object]]) -> None:
    repository: Path = root / "slow-repo"
    repository.mkdir()
    git(repository, "init", "-q")
    git(repository, "config", "user.name", "Reload Test")
    git(repository, "config", "user.email", "reload@example.invalid")
    tracked: Path = repository / "tracked.txt"
    tracked.write_text("baseline\n")
    (repository / ".gitattributes").write_text("tracked.txt filter=slow\n")
    git(repository, "add", ".")
    git(repository, "commit", "-qm", "isolated fixture")
    tracked.write_text("initial difference\n")
    marker: Path = root / "helper-started"
    release: Path = root / "helper-release"
    done: Path = root / "helper-done"
    helper: Path = root / "slow-helper.py"
    helper.write_text(
        "from pathlib import Path\nimport signal, sys, time\n"
        "signal.signal(signal.SIGHUP, signal.SIG_IGN)\n"
        "marker: Path = Path(sys.argv[1])\nrelease: Path = Path(sys.argv[2])\n"
        "done: Path = Path(sys.argv[3])\n"
        "data: bytes = sys.stdin.buffer.read()\n"
        "marker.write_text('started')\n"
        "while not release.exists():\n    time.sleep(0.02)\n"
        "done.write_text('released')\n"
        "sys.stdout.buffer.write(data)\n")
    session: Session = session_at(binary, ["--no-watch", "--view", "unified"],
                                  root / "slow-config", repository)
    try:
        session.ready()
        # Configure the helper only after the initial load. Git itself invokes
        # configured filters through a shell; all test paths are shell-quoted.
        import shlex
        command: str = " ".join(shlex.quote(str(path)) for path in
                                 [Path(sys.executable), helper, marker, release, done])
        git(repository, "config", "filter.slow.clean", command)
        tracked.write_text("slow reloaded difference\n")
        session.send(CTRL_R)
        deadline: float = time.monotonic() + 10
        while not marker.exists() and time.monotonic() < deadline:
            session.collect(0.05)
        assert marker.exists(), "reload helper never started"
        assert "loading" in full_screen(session)
        session.send(b"?")
        assert " Help " in full_screen(session), "loader blocked UI input"
        results.append(session.finish(b"\x03", "reload-slow-loader-responsive-quit"))
    finally:
        release.write_text("release owned helper")
        session.close()
        deadline = time.monotonic() + 10
        while marker.exists() and not done.exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        assert not marker.exists() or done.exists(), "owned helper did not release"


def main() -> None:
    binary: Path = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / "target/debug/riff"
    assert binary.is_file(), f"build {binary} first"
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    results: list[dict[str, object]] = []
    with tempfile.TemporaryDirectory(prefix="riff-reload-", dir=ARTIFACTS) as temporary:
        root: Path = Path(temporary).resolve()
        file_sessions(binary, root, results)
        git_sessions(binary, root, results)
        stdin_session(binary, root, results)
        reject_stdin_watch_before_read(binary, root)
        parse_recovery_session(binary, root, results)
        anchor_sessions(binary, root, results)
        slow_loader_session(binary, root, results)
    (ARTIFACTS / "reload-results.json").write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
