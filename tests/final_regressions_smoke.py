#!/usr/bin/env python3
"""Production-binary PTY regressions for clipped/wrapped reading-position anchors."""
from __future__ import annotations

import json
from pathlib import Path
import sys

from pty_smoke import ARTIFACTS, ROOT, Session, rendered_rows

CAPTURES: Path = ROOT / "target" / "final-regressions-smoke"


def fixture() -> bytes:
    """Five-cell words make display columns and wrapped source offsets explicit."""
    text: str = " ".join(f"T{number:03d}" for number in range(80)) + " "
    return f"--- a/position.txt\n+++ b/position.txt\n@@ -1 +1 @@\n-old\n+{text}\n".encode()


def screen(session: Session, label: str) -> list[str]:
    """Reconstruct physical cells without resizing away the position under test."""
    session.collect()
    rows: list[str] = rendered_rows(bytes(session.transcript), session.width, session.height)
    (CAPTURES / f"{label}.screen").write_text("\n".join(rows) + "\n", encoding="utf-8")
    return rows


def clip_to_wrap(binary: Path) -> dict[str, object]:
    """Column 80 must remain visible when wrapping replaces horizontal scrolling."""
    session: Session = Session(binary, ["--stdin", "--view", "unified", "--no-wrap"], fixture())
    try:
        session.set_size(26, 4)
        session.ready()
        session.send(b"G" + b"l" * 20)
        before: list[str] = screen(session, "clip-before")
        assert before[1][6:].startswith("T016"), before
        session.send(b"w")
        after: list[str] = screen(session, "wrap-after")
        assert after[1][6:].startswith("T016"), after
        config: Path = session.config_root / "riff/config.toml"
        assert config.read_text(encoding="utf-8").strip() == "wrap = true"
        return session.finish(b"q", "review-clip-to-wrap")
    finally:
        (CAPTURES / "clip-to-wrap.ansi").write_bytes(bytes(session.transcript))
        session.close()


def height_roundtrip(binary: Path) -> dict[str, object]:
    """Height clamping must not discard a logical continuation's source offset."""
    session: Session = Session(binary, ["--stdin", "--view", "unified", "--wrap"], fixture())
    try:
        session.set_size(25, 5)
        session.ready()
        session.send(b"j" * 13)
        before: list[str] = screen(session, "height-before")
        assert before[1][6:].startswith("T024"), before
        session.set_size(25, 200)
        session.collect()
        session.set_size(25, 5)
        after: list[str] = screen(session, "height-after")
        assert after[1] == before[1], (before, after)
        assert not list(session.config_root.rglob("*")), "resize persisted a setting"
        return session.finish(b"q", "review-height-roundtrip")
    finally:
        (CAPTURES / "height-roundtrip.ansi").write_bytes(bytes(session.transcript))
        session.close()


def main() -> None:
    """Use only isolated configuration and disposable controlling PTYs."""
    assert len(sys.argv) <= 2, "usage: final_regressions_smoke.py [path/to/riff]"
    binary: Path = Path(sys.argv[1]).resolve() if len(sys.argv) == 2 else ROOT / "target/release/riff"
    assert binary.is_file(), f"build {binary} first"
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    CAPTURES.mkdir(parents=True, exist_ok=True)
    results: list[dict[str, object]] = [clip_to_wrap(binary), height_roundtrip(binary)]
    report: str = json.dumps(results, indent=2)
    (CAPTURES / "results.json").write_text(report + "\n", encoding="utf-8")
    print(report)


if __name__ == "__main__":
    main()
