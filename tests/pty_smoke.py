#!/usr/bin/env python3
"""Unix PTY smoke test; no packages, real user terminal, or global changes needed."""
from __future__ import annotations

import errno
import fcntl
import json
import os
from pathlib import Path
import re
import select
import signal
import struct
import sys
import termios
import tempfile
import time
from typing import Any

ROOT: Path = Path(__file__).resolve().parents[1]
ARTIFACTS: Path = ROOT / "target" / "pty-smoke"
ANSI: re.Pattern[str] = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")


def plain(data: bytes) -> str:
    return ANSI.sub("", data.decode("utf-8", errors="replace"))


class Session:
    """Own one controlling PTY and preserve its before/after terminal settings."""

    def __init__(self, binary: Path, arguments: list[str], patch: bytes | None = None,
                 config_root: Path | None = None,
                 env_overrides: dict[str, str] | None = None) -> None:
        self.master: int
        self.slave: int
        self.master, self.slave = os.openpty()
        self.before: list[Any] = termios.tcgetattr(self.slave)
        self.transcript: bytearray = bytearray()
        self.height: int = 24
        self.width: int = 120
        self.status: int | None = None
        self.set_size(self.width, self.height)
        pipe: tuple[int, int] | None = os.pipe() if patch is not None else None
        self.temporary_config: tempfile.TemporaryDirectory[str] = tempfile.TemporaryDirectory(
            prefix="session-config-", dir=ARTIFACTS)
        self.config_root: Path = config_root or Path(self.temporary_config.name)
        environment: dict[str, str] = dict(os.environ)
        environment.update({"TERM": "xterm-256color", "COLORTERM": "truecolor",
                            "XDG_CONFIG_HOME": str(self.config_root),
                            "HOME": str(self.config_root), "APPDATA": str(self.config_root)})
        environment.pop("RIFF_THEME", None)
        if env_overrides:
            environment.update(env_overrides)
        self.pid: int = os.fork()
        if self.pid == 0:
            try:
                os.setsid()
                fcntl.ioctl(self.slave, termios.TIOCSCTTY, 0)
                os.dup2(pipe[0] if pipe else self.slave, 0)
                os.dup2(self.slave, 1)
                os.dup2(self.slave, 2)
                os.close(self.master)
                if self.slave > 2:
                    os.close(self.slave)
                if pipe:
                    os.close(pipe[0])
                    os.close(pipe[1])
                os.chdir(ROOT)
                os.execve(binary, [str(binary), *arguments], environment)
            finally:
                os._exit(127)
        if pipe:
            os.close(pipe[0])
            assert patch is not None
            os.write(pipe[1], patch)
            os.close(pipe[1])

    def set_size(self, width: int, height: int) -> None:
        self.width = width
        self.height = height
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
        if hasattr(self, "pid"):
            os.kill(self.pid, signal.SIGWINCH)

    def collect(self, seconds: float = 0.25) -> bytes:
        result: bytearray = bytearray()
        deadline: float = time.monotonic() + seconds
        while time.monotonic() < deadline:
            ready: list[int]
            ready, _, _ = select.select([self.master], [], [], max(0, deadline - time.monotonic()))
            if not ready:
                break
            try:
                chunk: bytes = os.read(self.master, 65536)
            except OSError as error:
                if error.errno == errno.EIO:
                    break
                raise
            if not chunk:
                break
            result.extend(chunk)
        self.transcript.extend(result)
        return bytes(result)

    def ready(self) -> None:
        # Grammar initialization can outlast a fixed sleep. Wait for the first
        # rendered footer, not just a live process or alternate-screen entry.
        deadline: float = time.monotonic() + 10
        while time.monotonic() < deadline:
            self.collect(0.05)
            waited, status = os.waitpid(self.pid, os.WNOHANG)
            if waited:
                self.status = status
                raise AssertionError(f"TUI exited before its first frame: {plain(bytes(self.transcript))!r}")
            if "q quit" in plain(bytes(self.transcript)):
                assert not termios.tcgetattr(self.slave)[3] & termios.ICANON, "TUI did not enter raw mode"
                return
        raise AssertionError(f"TUI did not render its first frame: {plain(bytes(self.transcript))!r}")

    def send(self, keys: bytes) -> bytes:
        os.write(self.master, keys)
        return self.collect()

    def repaint(self) -> str:
        # A real resize forces a complete repaint, allowing assertions without a
        # third-party VT emulator; incremental cursor/color updates are recorded.
        self.set_size(self.width, 25 if self.height == 24 else 24)
        return plain(self.collect())

    def expect(self, needle: str, keys: bytes | None = None) -> None:
        if keys is not None:
            self.send(keys)
        screen: str = self.repaint()
        assert needle in screen, f"missing {needle!r} in repaint: {screen!r}"

    def finish(self, keys: bytes, label: str, termination: int | None = None) -> dict[str, object]:
        if termination is not None:
            os.kill(self.pid, termination)
        self.send(keys)
        deadline: float = time.monotonic() + 5
        while time.monotonic() < deadline:
            waited: int
            status: int
            waited, status = os.waitpid(self.pid, os.WNOHANG)
            if waited:
                self.status = status
                break
            self.collect(0.05)
        assert self.status is not None, "TUI did not exit"
        self.collect(0.05)
        assert os.waitstatus_to_exitcode(self.status) == 0, plain(bytes(self.transcript))
        assert termios.tcgetattr(self.slave) == self.before, "terminal settings not restored"
        data: bytes = bytes(self.transcript)
        for code in [b"1049", b"1000", b"1002", b"1003", b"1006"]:
            if b"\x1b[?" + code + b"h" in data:
                assert b"\x1b[?" + code + b"l" in data, f"terminal mode {code!r} not disabled"
        assert b"\x1b[?1049h" in data and b"\x1b[?1049l" in data
        assert b"\x1b[?25l" in data and b"\x1b[?25h" in data
        pushes: list[bytes] = re.findall(rb"\x1b\[>(\d+)u", data)
        assert len(pushes) == 1 and int(pushes[0]) & 9 == 9, "keyboard flags not enabled"
        assert data.count(b"\x1b[<1u") == 1, "keyboard flags not popped exactly once"
        assert "panicked" not in plain(data)
        (ARTIFACTS / f"{label}.ansi").write_bytes(data)
        return {"session": label, "exit": 0, "terminal_restored": True, "bytes": len(data)}

    def close(self) -> None:
        if self.status is None:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        os.close(self.master)
        os.close(self.slave)
        self.temporary_config.cleanup()


def full_screen(session: Session) -> str:
    """Force a repaint without changing the page height used by key assertions."""
    session.set_size(121 if session.width == 120 else 120, session.height)
    return plain(session.collect())


def rendered_rows(data: bytes, width: int, height: int) -> list[str]:
    """Decode the cursor/clear subset emitted by Ratatui for an ASCII code fixture.

    Unlike stripping ANSI, this retains physical rows and gutter columns. This
    is not a general terminal emulator; Unicode code/style fidelity is separately
    checked by the Rust TestBackend and lossless grapheme regression tests.
    """
    grid: list[list[str]] = [[" "] * width for _ in range(height)]
    x: int = 0
    y: int = 0
    text: str = data.decode("utf-8")
    previous: int = 0
    for match in ANSI.finditer(text):
        for character in text[previous:match.start()]:
            if character == "\r":
                x = 0
            elif character == "\n":
                y += 1
            else:
                assert not character.isascii() or character.isprintable(), repr(character)
                if x >= width:
                    x = 0
                    y += 1
                if 0 <= y < height:
                    grid[y][x] = character
                x += 1
        sequence: str = match[0]
        command: str = sequence[-1]
        parameters: str = sequence[2:-1]
        if command in ("H", "f"):
            values: list[str] = parameters.split(";")
            y = int(values[0] or "1") - 1
            x = int(values[1] or "1") - 1 if len(values) > 1 else 0
        elif command == "J":
            assert parameters in ("", "0", "2"), sequence
            if parameters == "2":
                grid = [[" "] * width for _ in range(height)]
            elif 0 <= y < height:
                grid[y][x:] = [" "] * (width - x)
                for row in range(y + 1, height):
                    grid[row] = [" "] * width
        elif command == "K":
            assert parameters in ("", "0", "2"), sequence
            if 0 <= y < height:
                start: int = 0 if parameters == "2" else x
                grid[y][start:] = [" "] * (width - start)
        else:
            assert command in ("m", "h", "l", "u"), sequence
        previous = match.end()
    assert not text[previous:], "expected the frame to end with a cursor/style command"
    return ["".join(row) for row in grid]


def assert_code_pane(rows: list[str], gutter_width: int,
                     expected: list[tuple[str, str]]) -> None:
    """Read all visual fragments, requiring each source gutter exactly once."""
    index: int = -1
    remaining: str = ""
    for row in rows:
        gutter: str = row[:gutter_width]
        code: str = row[gutter_width:]
        if any(character.isdigit() for character in gutter):
            assert not remaining, f"code clipped before the next source line: {remaining!r}"
            index += 1
            assert index < len(expected), f"unexpected source gutter: {gutter!r}"
            wanted_gutter: str
            wanted_gutter, remaining = expected[index]
            assert gutter == wanted_gutter, (gutter, wanted_gutter)
        elif gutter.strip():
            # The EOF note has no source number and follows the complete code.
            assert "\\" in gutter and index == len(expected) - 1 and not remaining, row
            continue
        elif index < 0 or not remaining:
            assert not code.strip(), row
            continue
        # Compare exact code, permitting only right-hand screen padding. A long
        # unbroken token must reconstruct without inserted/lost characters.
        count: int = 0
        while count < min(len(code), len(remaining)) and code[count] == remaining[count]:
            count += 1
        assert count or not remaining, (code, remaining)
        assert not code[count:].strip(), (code, remaining)
        remaining = remaining[count:]
    assert index == len(expected) - 1 and not remaining, (index, remaining)


def wrapping_sessions(binary: Path, results: list[dict[str, object]]) -> None:
    common: str = "context alpha beta gamma delta " + "context_token_" * 7 + " end"
    removed: str = "old alpha beta gamma delta " + "R" * 73 + " old tail"
    added: str = "new alpha beta gamma delta " + "N" * 79 + " new tail"
    patch: bytes = ("--- a/wrap.txt\n+++ b/wrap.txt\n@@ -997,4 +997,4 @@\n"
                    f" {common}\n-{removed}\n+{added}\n \n end1000\n"
                    "\\ No newline at end of file\n").encode()
    for view in ("unified", "split"):
        with tempfile.TemporaryDirectory(prefix="wrap-screen-", dir=ARTIFACTS) as temporary:
            root: Path = Path(temporary)
            config: Path = root / "riff/config.toml"
            config.parent.mkdir()
            original: str = "# one-run overrides must not save\nwrap = false\nview = 'auto'\n"
            config.write_text(original)
            session: Session = Session(binary, ["--stdin", "--wrap", "--view", view], patch,
                                       config_root=root)
            try:
                session.set_size(80, 40)
                session.ready()
                session.set_size(80, 41)
                rows: list[str] = rendered_rows(session.collect(), 80, 41)
                assert "wrap:on" in rows[-2] and f" {view} ·" in rows[-2], rows[-2]
                hunk: int = next(index for index, row in enumerate(rows) if row.startswith("@@"))
                note: int = next(index for index, row in enumerate(rows)
                                 if "No newline at end of file" in row)
                body: list[str] = rows[hunk + 1:note]
                if view == "unified":
                    expected: list[tuple[str, str]] = [
                        (" 997  997   ", common), (" 998      - ", removed),
                        ("      998 + ", added), (" 999  999   ", ""),
                        ("1000 1000   ", "end1000"),
                    ]
                    assert_code_pane(body, 12, expected)
                else:
                    assert all(row[39] == "│" for row in body if row.strip()), body
                    assert_code_pane([row[:39] for row in body], 7, [
                        (" 997   ", common), (" 998 - ", removed),
                        (" 999   ", ""), ("1000   ", "end1000"),
                    ])
                    assert_code_pane([row[40:] for row in body], 7, [
                        (" 997   ", common), (" 998 + ", added),
                        (" 999   ", ""), ("1000   ", "end1000"),
                    ])
                assert config.read_text() == original, "--wrap override wrote configuration"
                results.append(session.finish(b"q", f"word-wrap-{view}-complete-gutters"))
                assert config.read_text() == original, "quit persisted a one-run override"
            finally:
                session.close()


def preference_sessions(binary: Path, arguments: list[str],
                        results: list[dict[str, object]]) -> None:
    with tempfile.TemporaryDirectory(prefix="preferences-", dir=ARTIFACTS) as temporary:
        root: Path = Path(temporary)
        config: Path = root / "riff/config.toml"
        session: Session = Session(binary, [*arguments, "--view", "unified", "--no-wrap"],
                                   config_root=root)
        try:
            session.ready()
            assert not config.exists(), "startup wrote a default configuration"
            session.send(b"tj\x1b")
            assert not config.exists(), "theme preview/cancel wrote configuration"
            # Actual Kitty/CSI-u delivery, not a synthesized Rust KeyEvent.
            session.send(b"\x1b[32u")
            screen: str = full_screen(session)
            assert "rows 22-" in screen, screen
            session.send(b"\x1b[32;2u")
            screen = full_screen(session)
            assert "rows 1-" in screen, screen
            session.send(b"\x1b[32;2u")
            assert "rows 1-" in full_screen(session), "reverse page failed to clamp"
            session.send(b"\x1b[119u")  # w, encoded as an ordinary CSI-u key
            assert "wrap:on" in full_screen(session)
            saved: str = config.read_text()
            assert "wrap = true" in saved and "theme =" not in saved and "view =" not in saved, saved
            session.send(b"/needle_070\r")
            assert "/needle_070 [1/1]" in full_screen(session)
            session.send(b"\x1b[116u\x1b[106u\x1b")  # t, j, Esc: cancel
            assert config.read_text() == saved
            session.send(b"\x1b[116u\x1b[106u\x1b[13u")  # accept dracula
            saved = config.read_text()
            assert 'theme = "dracula"' in saved and "wrap = true" in saved and "view =" not in saved, saved
            # ESC-v is Alt+v, not two keys; encode Esc and v unambiguously.
            session.send(b"\x1b[27u\x1b[118u")  # clear search, then explicitly accept split
            assert 'view = "split"' in config.read_text()
            results.append(session.finish(b"\x1b[99;5u", "csi-u-wrap-shift-space-persist"))
        finally:
            session.close()

        session = Session(binary, arguments, config_root=root)
        try:
            session.ready()
            screen = full_screen(session)
            assert " split · dracula ·" in screen and "wrap:on" in screen, screen
            session.send(b"w")
            assert "clip x=0" in full_screen(session)
            saved = config.read_text()
            assert "wrap = false" in saved and 'theme = "dracula"' in saved and 'view = "split"' in saved, saved
            results.append(session.finish(b"q", "preferences-next-session"))
        finally:
            session.close()

        default_before: str = config.read_text()
        explicit: Path = root / "explicit.toml"
        original: str = ("# custom settings\ntheme = 'nord' # keep\nview = 'auto'\nwrap = true\n"
                         "[themes.mine]\nextends = 'sunset'\nbackground = '#010203'\n")
        explicit.write_text(original)
        session = Session(binary, [*arguments, "--config", str(explicit), "--view", "unified", "--no-wrap"],
                          config_root=root, env_overrides={"RIFF_THEME": "dracula"})
        try:
            session.ready()
            screen = full_screen(session)
            assert " unified · dracula ·" in screen and "clip x=0" in screen, screen
            assert explicit.read_text() == original
            session.send(b"w")
            saved = explicit.read_text()
            assert "theme = 'nord' # keep" in saved and "view = 'auto'" in saved, saved
            assert "[themes.mine]" in saved and "background = '#010203'" in saved, saved
            session.send(b"w")  # Make the persisted value differ from the original.
            saved = explicit.read_text()
            assert "wrap = false" in saved and "theme = 'nord' # keep" in saved, saved
            assert "view = 'auto'" in saved and "[themes.mine]" in saved, saved
            assert config.read_text() == default_before, "--config wrote to the default path"
            results.append(session.finish(b"q", "explicit-selective-overrides"))
        finally:
            session.close()

        readonly: Path = root / "readonly"
        readonly.mkdir()
        blocked: Path = readonly / "bad\x1b[31m.toml"
        blocked.write_text("wrap = false\n")
        session = Session(binary, [*arguments, "--config", str(blocked)], config_root=root)
        try:
            session.ready()
            blocked.chmod(0o444)
            readonly.chmod(0o555)
            session.send(b"w")
            session.set_size(320, 24)
            screen = plain(session.collect())
            assert "Save failed:" in screen and "bad^[[31m.toml" in screen, screen
            assert b"bad\x1b[31m.toml" not in session.transcript, "save error replayed raw controls"
            assert blocked.read_text() == "wrap = false\n"
            readonly.chmod(0o755)
            blocked.chmod(0o644)
            session.send(b"v")
            screen = full_screen(session)
            assert "Save failed:" not in screen and "wrap:on" in screen, screen
            assert "wrap = false" in blocked.read_text(), "failed update was retried incidentally"
            results.append(session.finish(b"q", "save-failure-nonfatal"))
        finally:
            readonly.chmod(0o755)
            blocked.chmod(0o644)
            session.close()

    session = Session(binary, [*arguments, "--wrap", "--theme", "dracula", "--view", "split"])
    try:
        session.ready()
        config = session.config_root / "riff/config.toml"
        assert not config.exists(), "CLI-only override wrote configuration"
        session.send(b"tj\x1b")
        assert " split · dracula ·" in full_screen(session), "preview cancel did not restore theme"
        assert not config.exists(), "preview cancel created configuration"
        results.append(session.finish(b"q", "override-preview-cancel-no-write"))
        assert not config.exists(), "quit wrote configuration"
    finally:
        session.close()


def main() -> None:
    binary: Path = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / "target/release/riff"
    assert binary.is_file(), f"build {binary} first"
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    old: Path = ARTIFACTS / "before.rs"
    new: Path = ARTIFACTS / "after.rs"
    old.write_text("".join(f"fn before_{index:03d}() {{}} // {'old ' * 45}\n" for index in range(120)))
    new.write_text("".join(f"fn needle_{index:03d}() {{}} // {'new ' * 45}\n" for index in range(120)))
    arguments: list[str] = ["--files", str(old), str(new)]
    results: list[dict[str, object]] = []
    session: Session = Session(binary, arguments)
    try:
        session.ready()
        session.expect("auto→split")
        session.expect("rows 2-", b"j")
        session.expect("rows 1-", b"k")
        session.expect("rows 1-", b"Gg")
        session.expect("x=4", b"l")
        session.expect("x=0", b"0")
        session.send(b"n")
        session.send(b"p")
        session.send(b"]")
        session.send(b"[")
        session.set_size(80, 24)
        session.collect()
        session.expect("auto→unified")
        session.set_size(120, 24)
        session.collect()
        session.expect("auto→split")
        session.expect(" unified ·", b"v")
        session.expect(" Themes ·", b"t")
        session.send(b"j")
        session.expect(" dracula ·", b"\r")
        session.send(b"t")
        session.send(b"j")
        session.expect(" dracula ·", b"\x1b")
        session.expect("/needle_070 [1/1]", b"/needle_070")
        session.send(b"\r")
        session.send(b"/")
        session.send(b"missing")
        session.expect("/needle_070 [1/1]", b"\x1b")
        session.expect(" Help ·", b"?")
        session.send(b"?")
        results.append(session.finish(b"q", "interactive-quit"))
    finally:
        session.close()

    session = Session(binary, arguments)
    try:
        session.ready()
        session.expect(" Themes ·", b"t")
        results.append(session.finish(b"\x03", "dialog-ctrl-c"))
    finally:
        session.close()

    patch: bytes = b"--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-fn old() {}\n+fn new() {}\n"
    session = Session(binary, [], patch)
    try:
        session.ready()
        session.expect("stdin patch")
        session.expect(" Themes ·", b"t")
        session.send(b"\x1b")
        results.append(session.finish(b"q", "piped-stdin"))
    finally:
        session.close()
    # A deliberately slow exec catches fixed-sleep startup assumptions. Keep
    # the PID so signal delivery still exercises the viewer, not a wrapper.
    delayed_arguments: list[str] = [
        "-c", "import os, sys, time; time.sleep(1); os.execv(sys.argv[1], sys.argv[1:])",
        str(binary), *arguments,
    ]
    session = Session(Path(sys.executable).resolve(), delayed_arguments)
    try:
        session.ready()
        session.expect(" Themes ·", b"t")
        results.append(session.finish(b"", "delayed-startup-sigterm", signal.SIGTERM))
    finally:
        session.close()

    # Reproduce the review's compact viewport scenarios, forcing real repaints.
    replacement: bytes = ("--- a/f\n+++ b/f\n@@ -1,80 +1,80 @@\n" +
                          "".join(f"-old{line:03d}\n" for line in range(1, 81)) +
                          "".join(f"+new{line:03d}\n" for line in range(1, 81))).encode()
    session = Session(binary, ["--stdin"], replacement)
    try:
        session.set_size(80, 10)
        session.ready()
        session.send(b"/new041\r")
        session.set_size(120, 10)
        screen: str = plain(session.collect(0.6))
        assert "auto→split" in screen and "new041" in screen and "new074" not in screen, (screen, plain(bytes(session.transcript))[-4000:])
        assert "/new041 [1/1]" in screen, screen
        session.set_size(80, 10)
        screen = plain(session.collect(0.6))
        assert "auto→unified" in screen and "new041" in screen, screen
        results.append(session.finish(b"q", "adaptive-search-position"))
    finally:
        session.close()

    needles: bytes = ("--- a/f\n+++ b/f\n@@ -1,30 +1,30 @@\n" +
                      "".join(f" {'needle' if line in [2, 15] else 'line'}{line}\n"
                              for line in range(1, 31))).encode()
    session = Session(binary, ["--stdin"], needles)
    try:
        session.set_size(80, 10)
        session.ready()
        session.send(b"/needle\r\r")
        session.set_size(81, 10)
        screen = plain(session.collect())
        assert "/needle [2/2]" in screen and "needle15" in screen, screen
        session.send(b"/zzzz\x1b")
        session.set_size(80, 10)
        screen = plain(session.collect())
        assert "/needle [2/2]" in screen and "needle15" in screen, screen
        session.send(b"\r")
        session.set_size(81, 10)
        screen = plain(session.collect())
        assert "/needle [1/2]" in screen and "needle2" in screen and "needle15" not in screen, screen
        results.append(session.finish(b"q", "cancel-search-selection"))
    finally:
        session.close()

    wrapping_sessions(binary, results)
    preference_sessions(binary, arguments, results)

    failures: list[str] = []
    for termination in [signal.SIGTERM, signal.SIGINT, signal.SIGHUP]:
        session = Session(binary, arguments)
        label: str = f"external-{signal.Signals(termination).name.lower()}"
        try:
            session.ready()
            session.expect(" Themes ·", b"t")
            results.append(session.finish(b"", label, termination))
        except AssertionError as error:
            failures.append(f"{label}: {error}")
        finally:
            session.close()
    assert not failures, "\n".join(failures)
    report: str = json.dumps(results, indent=2)
    (ARTIFACTS / "results.json").write_text(report + "\n")
    print(report)


if __name__ == "__main__":
    main()
