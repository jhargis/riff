#!/usr/bin/env python3
"""Exercise file selection with legacy and Kitty keyboard input in real PTYs."""
from __future__ import annotations

import json
from pathlib import Path
import re
import sys

from keyboard_protocol_smoke import KittyKeyboard
from pty_smoke import ARTIFACTS, ROOT, Session, full_screen

CAPTURES: Path = ROOT / "target" / "file-picker-pty"
ROW_RANGE: re.Pattern[str] = re.compile(r"rows (\d+)-(\d+)/(\d+)")


def patch_fixture() -> bytes:
    """Keep each file taller than the viewport to verify real navigation."""
    paths: tuple[str, ...] = ("docs/notes.md", "src/app.rs", "tests/app.rs")
    context: str = " needle\n" + "".join(f" context {index:02}\n" for index in range(39))
    return "".join(
        f"--- a/{path}\n+++ b/{path}\n@@ -1,41 +1,42 @@\n-old\n+new\n+extra\n{context}"
        for path in paths
    ).encode("utf-8")


def row_range(screen: str) -> tuple[int, int, int]:
    """Read the visible row range after a full repaint."""
    match: re.Match[str] | None = ROW_RANGE.search(screen)
    assert match is not None, screen
    return int(match[1]), int(match[2]), int(match[3])


def run(binary: Path, view: str, enhanced: bool) -> dict[str, object]:
    """Filter, select, accept, cancel, and quit without writing preferences."""
    session: Session = Session(binary, ["--stdin", "--no-syntax", "--view", view], patch_fixture())
    label: str = f"file-picker-{view}-{'kitty' if enhanced else 'legacy'}"
    try:
        session.ready()
        keyboard: KittyKeyboard = KittyKeyboard.advertised(session)
        ctrl_p: bytes = keyboard.key("p", control=True) if enhanced else b"\x10"
        ctrl_u: bytes = keyboard.key("u", control=True) if enhanced else b"\x15"
        enter: bytes = keyboard.key("\r") if enhanced else b"\r"
        escape: bytes = keyboard.key("\x1b") if enhanced else b"\x1b"

        def encode(text: str) -> bytes:
            return keyboard.text(text) if enhanced else text.encode("utf-8")

        session.send(encode("/needle") + enter + encode("g"))
        before: tuple[int, int, int] = row_range(full_screen(session))
        session.send(ctrl_p)
        screen: str = full_screen(session)
        assert "Files · 3/3" in screen and "+2 -1  src/app.rs" in screen, screen
        assert row_range(screen) == before
        session.send(encode("APP"))
        screen = full_screen(session)
        assert "Files · 2/3" in screen and "› +2 -1  src/app.rs" in screen, screen
        session.send(b"\x1b[B")
        screen = full_screen(session)
        assert "› +2 -1  tests/app.rs" in screen, screen
        session.send(b"\x1b[A" + encode(" src"))
        screen = full_screen(session)
        assert "Files · 1/3" in screen and "› +2 -1  src/app.rs" in screen, screen
        assert row_range(screen) == before, "filtering moved the underlying diff"
        session.send(enter)
        screen = full_screen(session)
        assert "Files ·" not in screen and "file 2/3" in screen, screen
        assert "/needle [1/3]" in screen, "file selection replaced accepted code search"
        session.send(encode("]"))
        screen = full_screen(session)
        assert "file 3/3" in screen, screen
        before = row_range(screen)
        session.send(ctrl_p + encode("zzzz"))
        screen = full_screen(session)
        assert "Files · 0/3" in screen and "No matching files." in screen, screen
        session.send(enter)
        assert "Files · 0/3" in full_screen(session), "Enter closed an empty picker"
        session.send(ctrl_u + encode("q"))
        screen = full_screen(session)
        assert "Files · 0/3" in screen and "> q▏" in screen, "q did not become filter text"
        session.send(ctrl_u)
        assert "Files · 3/3" in full_screen(session)
        session.send(escape)
        screen = full_screen(session)
        assert "Files ·" not in screen and row_range(screen) == before, screen
        assert not (session.config_root / "riff").exists(), "picker wrote preferences"
        session.send(ctrl_p)
        ctrl_c: bytes = keyboard.key("c", control=True) if enhanced else b"\x03"
        return session.finish(ctrl_c, label)
    finally:
        (CAPTURES / f"{label}.ansi").write_bytes(bytes(session.transcript))
        session.close()


def main() -> None:
    """Use only owned fixture configuration, transcripts, and controlling PTYs."""
    binary: Path = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / "target/release/riff"
    assert binary.is_file(), f"build {binary} first"
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    CAPTURES.mkdir(parents=True, exist_ok=True)
    results: list[dict[str, object]] = [
        run(binary, view, enhanced)
        for view in ("unified", "split")
        for enhanced in (False, True)
    ]
    report: str = json.dumps(results, indent=2)
    (CAPTURES / "results.json").write_text(report + "\n")
    print(report)


if __name__ == "__main__":
    main()
