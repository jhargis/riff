#!/usr/bin/env python3
"""Release word-background controls/search/wrap/theme/watch in disposable PTYs."""
from __future__ import annotations

import json
from pathlib import Path
import sys
import tempfile

from pty_smoke import ARTIFACTS, ROOT, Session, plain
from pty_reload import atomic_text, wait_screen, watch_on

CAPTURES: Path = ROOT / "target" / "intraline-pty"
CRITIQUE: bytes = b"48;2;27;70;40"
GITHUB: bytes = b"48;2;36;62;93"


def frame(session: Session) -> bytes:
    """Force a fresh full repaint, retaining the actual SGR color sequences."""
    session.set_size(121 if session.width == 120 else 120, session.height)
    return session.collect()


def run(binary: Path, root: Path, view: str, syntax: bool) -> dict[str, object]:
    """Use one-run overrides, then accept only individual interactive settings."""
    old: Path = root / "before.rs"
    new: Path = root / "after.rs"
    prefix: str = "let timeout = "
    old.write_text(prefix + "30; // stable explanatory comment with extra words\n")
    new.write_text(prefix + "60; // stable explanatory comment with extra words\n")
    config: Path = root / "config.toml"
    original: str = ("# isolated preferences\ntheme = 'critique'\nview = 'auto'\n"
                     "wrap = false\nwatch = false\nword_diff = false # token backgrounds\n")
    config.write_text(original)
    args: list[str] = ["--files", str(old), str(new), "--config", str(config),
                       "--view", view, "--word-diff", "--watch"]
    if not syntax:
        args.append("--no-syntax")
    session: Session = Session(binary, args, config_root=root / "isolated")
    label: str = f"intraline-{view}-{'syntax' if syntax else 'plain'}"
    try:
        session.ready()
        watch_on(session)
        data: bytes = frame(session)
        assert "words:on" in plain(data), plain(data)
        assert CRITIQUE in data, data
        assert config.read_text() == original, "startup saved CLI overrides"
        session.send(b"/60\r")
        assert "/60 [1/1]" in plain(frame(session))
        session.send(b"i")
        data = frame(session)
        assert "words:off" in plain(data) and CRITIQUE not in data, plain(data)
        assert "/60 [1/1]" in plain(data), plain(data)
        assert config.read_text() == original, "i saved unrelated CLI overrides"
        atomic_text(new, prefix + "160; // stable explanatory comment with extra words\n")
        wait_screen(session, lambda screen: "160" in screen and "loading" not in screen,
                    "watch while word diff disabled")
        data = frame(session)
        assert "words:off" in plain(data) and CRITIQUE not in data, plain(data)
        session.send(b"i")
        data = frame(session)
        assert "words:on" in plain(data) and CRITIQUE in data, plain(data)
        assert config.read_text() == original.replace("word_diff = false", "word_diff = true")
        session.send(b"w")
        assert "wrap:on" in plain(frame(session))
        session.send(b"t\x1b[F\r")  # End selects the last builtin, github-dark.
        data = frame(session)
        assert "github-dark" in plain(data) and GITHUB in data, plain(data)
        assert "/60 [1/1]" in plain(data), plain(data)
        atomic_text(new, prefix + "260; // stable explanatory comment with extra words\n")
        wait_screen(session, lambda screen: "260" in screen and "loading" not in screen,
                    "watch with wrapped word backgrounds")
        data = frame(session)
        assert GITHUB in data and "words:on" in plain(data), plain(data)
        assert "/60 [1/1]" in plain(data), plain(data)
        session.send(b"r")
        wait_screen(session, lambda screen: "loading" not in screen and "260" in screen,
                    "manual reload retains word diff")
        session.send(b"v")
        assert GITHUB in frame(session), "view rebuild lost token backgrounds"
        expected: str = original.replace("word_diff = false", "word_diff = true")
        expected = expected.replace("wrap = false", "wrap = true")
        expected = expected.replace("theme = 'critique'", 'theme = "github-dark"')
        expected = expected.replace("view = 'auto'", f'view = "{ "split" if view == "unified" else "unified" }"')
        assert config.read_text() == expected, config.read_text()
        return session.finish(b"q", label)
    finally:
        (CAPTURES / f"{label}.ansi").write_bytes(bytes(session.transcript))
        session.close()


def main() -> None:
    """All fixture files/configuration and controlling terminals are owned here."""
    binary: Path = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / "target/release/riff"
    assert binary.is_file(), f"build {binary} first"
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    CAPTURES.mkdir(parents=True, exist_ok=True)
    results: list[dict[str, object]] = []
    view: str
    syntax: bool
    temporary: str
    for view in ("unified", "split"):
        for syntax in (True, False):
            with tempfile.TemporaryDirectory(prefix="intraline-", dir=CAPTURES) as temporary:
                results.append(run(binary, Path(temporary), view, syntax))
    report: str = json.dumps(results, indent=2)
    (CAPTURES / "results.json").write_text(report + "\n")
    print(report)


if __name__ == "__main__":
    main()
