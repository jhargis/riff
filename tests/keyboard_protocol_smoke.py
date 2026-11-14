#!/usr/bin/env python3
"""Regress Kitty shifted-key negotiation against the real release binary.

Run after `cargo build --release --locked`:
    python3 tests/keyboard_protocol_smoke.py [path/to/riff]

Only Python's standard library is used. Session supplies a controlling PTY and
an isolated temporary HOME/XDG_CONFIG_HOME/APPDATA, never the user's config.
Input follows the flags actually emitted by riff: without alternate-key reporting,
Shift+/ is CSI 47;2u, NOT a literal '?' or an unconditional CSI 47:63;2u.
The few explicit base/shifted pairs below are terminal-layout test inputs, not an
application-side punctuation translation table.
"""
from __future__ import annotations

from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import sys
import termios
import time

from pty_smoke import ARTIFACTS, ROOT, Session, full_screen, plain

CAPTURES: Path = ROOT / "target" / "keyboard-protocol-smoke"
PUSH_FLAGS: re.Pattern[bytes] = re.compile(rb"\x1b\[>(\d+)u")
ROW_RANGE: re.Pattern[str] = re.compile(r"rows (\d+)-(\d+)/(\d+)")
DISAMBIGUATE: int = 1
ALTERNATE_KEYS: int = 4
ALL_KEYS: int = 8
EXPECTED_FLAGS: int = DISAMBIGUATE | ALTERNATE_KEYS | ALL_KEYS
SEARCH_PROMPT: str = "Esc cancel · Ctrl-U clear"
HELP_TITLE: str = " Help ·"


@dataclass(frozen=True)
class KittyKeyboard:
    """A bounded terminal emulator for the negotiated all-keys CSI-u protocol."""

    flags: int

    @classmethod
    def advertised(cls, session: Session) -> KittyKeyboard:
        """Inspect actual startup bytes, rather than assuming the desired mask."""
        pushes: list[bytes] = PUSH_FLAGS.findall(bytes(session.transcript))
        assert len(pushes) == 1, f"expected one keyboard push, got {pushes!r}"
        flags: int = int(pushes[0])
        assert flags & (DISAMBIGUATE | ALL_KEYS) == DISAMBIGUATE | ALL_KEYS, flags
        return cls(flags)

    def key(self, base: str, *, shifted: str | None = None, shift: bool = False,
            caps_lock: bool = False, alt: bool = False, control: bool = False) -> bytes:
        """Encode one key using a terminal-supplied shifted glyph when requested.

        Kitty omits an alternate that is identical to the base (notably Space).
        There is no case/punctuation inference: the caller supplies layout data.
        No event types or associated-text fields are sent: riff requests neither.
        """
        assert len(base) == 1 and (shifted is None or len(shifted) == 1)
        is_shifted: bool = shift or shifted is not None
        codepoints: str = str(ord(base))
        if self.flags & ALTERNATE_KEYS and shifted is not None and shifted != base:
            codepoints += f":{ord(shifted)}"
        mask: int = 1 + int(is_shifted) + 2 * int(alt) + 4 * int(control) + 64 * int(caps_lock)
        modifier: str = f";{mask}" if mask != 1 else ""
        return f"\x1b[{codepoints}{modifier}u".encode("ascii")

    def text(self, value: str) -> bytes:
        """Encode unshifted characters; shifted glyphs must use explicit keys."""
        return b"".join(self.key(character) for character in value)


def patch_fixture() -> bytes:
    """Supply enough context rows for paging plus distinct searchable glyphs."""
    lines: list[str] = [f"line{number:03d}" for number in range(1, 81)]
    lines[0] = "first ?"
    lines[11] = "second ?"
    lines[29] = "glyphs { } _ + :"
    lines[59] = "two words"
    return ("--- a/keyboard.txt\n+++ b/keyboard.txt\n@@ -1,80 +1,80 @@\n"
            + "".join(f" {line}\n" for line in lines)).encode("utf-8")


def row_range(screen: str) -> tuple[int, int, int]:
    """Read the footer after a full repaint, avoiding stale incremental output."""
    match: re.Match[str] | None = ROW_RANGE.search(screen)
    assert match is not None, f"missing row range: {screen!r}"
    return int(match[1]), int(match[2]), int(match[3])


def help_toggle(session: Session, key: bytes, description: str) -> None:
    """Both edges of '?' must affect help, never the search editor."""
    session.send(key)
    screen: str = full_screen(session)
    assert HELP_TITLE in screen and SEARCH_PROMPT not in screen, (
        f"{description}: expected help, not search; sent {key!r}: {screen!r}")
    session.send(key)
    screen = full_screen(session)
    assert HELP_TITLE not in screen and SEARCH_PROMPT not in screen, screen
    assert "q quit" in screen, screen


def search_glyph(session: Session, keyboard: KittyKeyboard, base: str, shifted: str,
                 matches: int = 1) -> None:
    """Verify the terminal's shifted glyph, not the base key, becomes the query."""
    session.send(keyboard.key("\x1b") + keyboard.key("g") + keyboard.key("/"))
    screen: str = full_screen(session)
    assert SEARCH_PROMPT in screen and "/▏" in screen and HELP_TITLE not in screen, screen
    session.send(keyboard.key(base, shifted=shifted))
    screen = full_screen(session)
    assert f"/{shifted} [1/{matches}]" in screen and f"/{shifted}▏" in screen, screen
    assert SEARCH_PROMPT in screen and HELP_TITLE not in screen, screen
    session.send(keyboard.key("\r"))
    screen = full_screen(session)
    assert f"/{shifted} [1/{matches}]" in screen and SEARCH_PROMPT not in screen, screen


def exercise_protocol(session: Session, keyboard: KittyKeyboard) -> None:
    """Exercise parser-to-dispatch behavior, including negotiation's regressions."""
    question: bytes = keyboard.key("/", shifted="?")
    # Deliberately check behavior before requiring mask 13, so an old binary
    # reproduces the actual wrong-search dispatch with its advertised mask 9.
    help_toggle(session, question, f"Shift+/ with advertised flags={keyboard.flags}")
    assert keyboard.flags == EXPECTED_FLAGS, (
        f"expected keyboard flags {EXPECTED_FLAGS}, got {keyboard.flags}")
    assert question == b"\x1b[47:63;2u", question

    # Plain slash starts search; shifted slash inside it inserts literal '?'.
    search_glyph(session, keyboard, "/", "?", matches=2)
    session.send(keyboard.key("\r"))
    screen: str = full_screen(session)
    assert "/? [2/2]" in screen, screen
    session.send(keyboard.key("n", shifted="N"))
    screen = full_screen(session)
    assert "/? [1/2]" in screen and SEARCH_PROMPT not in screen, screen

    # Representative punctuation must come from the shifted alternate codepoint.
    search_glyph(session, keyboard, "[", "{")
    search_glyph(session, keyboard, "]", "}")
    search_glyph(session, keyboard, "-", "_")
    search_glyph(session, keyboard, "=", "+", matches=4)  # Also occurs in patch metadata.
    search_glyph(session, keyboard, ";", ":")

    # Shift+g must still be G (last row), not the lower-case first-row command.
    session.send(keyboard.key("\x1b") + keyboard.key("g"))
    assert row_range(full_screen(session))[0] == 1
    session.send(keyboard.key("g", shifted="G"))
    first: int
    last: int
    total: int
    first, last, total = row_range(full_screen(session))
    page: int = session.height - 3
    assert first == max(1, total - page + 1) and last == total, (first, last, total)
    session.send(keyboard.key("g"))
    assert row_range(full_screen(session))[0] == 1

    # Real Kitty Caps+g uses the base letter with lock bit 64, not literal G.
    caps_g: bytes = keyboard.key("g", caps_lock=True)
    assert caps_g == b"\x1b[103;65u", caps_g
    session.send(caps_g)
    assert row_range(full_screen(session))[0] == max(1, total - page + 1)
    session.send(keyboard.key("g", shifted="G", caps_lock=True))
    assert row_range(full_screen(session))[0] == 1, "Shift+Caps must produce lowercase g"

    # Unsupported chords cannot navigate, quit, open modals, or write settings.
    session.send(keyboard.key(" "))
    for character in "wvt/q":
        session.send(keyboard.key(character, alt=True))
    session.send(keyboard.key(" ", shift=True, alt=True))
    assert row_range(full_screen(session))[0] == page + 1
    session.send(keyboard.key("g"))

    # Space has no distinct shifted codepoint. Real Kitty input retains SHIFT
    # without an alternate; injecting 32:32 would wrongly hide this behavior.
    shift_space: bytes = keyboard.key(" ", shifted=" ")
    assert shift_space == b"\x1b[32;2u", shift_space
    session.send(keyboard.key(" "))
    assert row_range(full_screen(session))[0] == page + 1
    session.send(shift_space)
    assert row_range(full_screen(session))[0] == 1
    session.send(shift_space)
    assert row_range(full_screen(session))[0] == 1, "reverse page did not clamp"

    # In search, that same Shift+Space must insert a space, not navigate.
    session.send(keyboard.key("/") + keyboard.text("two") + shift_space + keyboard.text("words"))
    screen = full_screen(session)
    assert "/two words [1/1]" in screen and "/two words▏" in screen, screen
    session.send(keyboard.key("\r") + keyboard.key("\x1b"))

    # Lock normalization also applies to search text, but not Control shortcuts.
    session.send(keyboard.key("/") + caps_g
                 + keyboard.key("g", shifted="G", caps_lock=True))
    screen = full_screen(session)
    assert "/Gg▏" in screen and SEARCH_PROMPT in screen, screen
    session.send(keyboard.key("u", control=True, caps_lock=True))
    assert "/▏" in full_screen(session)
    session.send(keyboard.key("\x1b"))


def exercise_legacy(session: Session) -> None:
    """A terminal ignoring Kitty negotiation must still support ordinary '?' ."""
    help_toggle(session, b"?", "legacy literal question mark")
    session.send(b"/")
    screen: str = full_screen(session)
    assert SEARCH_PROMPT in screen and "/▏" in screen and HELP_TITLE not in screen, screen
    session.send(b"?")
    screen = full_screen(session)
    assert "/? [1/2]" in screen and "/?▏" in screen and HELP_TITLE not in screen, screen
    session.send(b"\r\x1b")
    help_toggle(session, b"?", "legacy question mark after search")


def finish(session: Session, quit_key: bytes, label: str, flags: int) -> dict[str, object]:
    """Check exit/mode restoration against this test's negotiated keyboard mask.

    Startup rendering and bounded exit polling use the shared fixture.
    """
    session.send(quit_key)
    deadline: float = time.monotonic() + 5
    while time.monotonic() < deadline:
        waited: int
        status: int
        waited, status = os.waitpid(session.pid, os.WNOHANG)
        if waited:
            session.status = status
            break
        session.collect(0.05)
    assert session.status is not None, "TUI did not exit"
    session.collect(0.05)
    data: bytes = bytes(session.transcript)
    assert os.waitstatus_to_exitcode(session.status) == 0, plain(data)
    assert termios.tcgetattr(session.slave) == session.before, "terminal settings not restored"
    assert PUSH_FLAGS.findall(data) == [str(flags).encode("ascii")], "unexpected keyboard push"
    assert data.count(b"\x1b[<1u") == 1, "keyboard flag stack not restored"
    for code in (b"1049", b"1000", b"1002", b"1003", b"1006", b"25"):
        enable: bytes = b"\x1b[?" + code + b"h"
        disable: bytes = b"\x1b[?" + code + b"l"
        if code == b"25":
            assert disable in data and enable in data, "cursor not restored"
        elif enable in data:
            assert disable in data, f"terminal mode {code!r} not restored"
    assert b"\x1b[?1049h" in data and b"\x1b[?1049l" in data
    assert "panicked" not in plain(data), plain(data)
    assert not list(session.config_root.rglob("*")), "keyboard actions wrote configuration"
    return {"session": label, "advertised_flags": flags, "exit": 0,
            "terminal_restored": True, "config_untouched": True, "bytes": len(data)}


def main() -> None:
    """Run negotiated and legacy paths with separate isolated PTY sessions."""
    assert len(sys.argv) <= 2, "usage: keyboard_protocol_smoke.py [path/to/riff]"
    binary: Path = Path(sys.argv[1]).resolve() if len(sys.argv) == 2 else ROOT / "target/release/riff"
    assert binary.is_file(), f"build {binary} first"
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    CAPTURES.mkdir(parents=True, exist_ok=True)
    results: list[dict[str, object]] = []
    label: str
    for label in ("kitty-alternate-keys", "legacy-question-mark"):
        session: Session = Session(binary, ["--stdin", "--view", "unified", "--no-wrap"],
                                   patch_fixture())
        try:
            session.ready()
            keyboard: KittyKeyboard = KittyKeyboard.advertised(session)
            if label == "kitty-alternate-keys":
                exercise_protocol(session, keyboard)
                results.append(finish(session, keyboard.key("q"), label, keyboard.flags))
            else:
                assert keyboard.flags == EXPECTED_FLAGS, keyboard.flags
                exercise_legacy(session)
                results.append(finish(session, b"q", label, keyboard.flags))
        finally:
            # Preserve failed sessions too, but clean up only this test's PTY/config.
            (CAPTURES / f"{label}.ansi").write_bytes(bytes(session.transcript))
            session.close()
    report: str = json.dumps(results, indent=2)
    (CAPTURES / "results.json").write_text(report + "\n", encoding="utf-8")
    print(report)


if __name__ == "__main__":
    main()
