#!/usr/bin/env python3
"""Native submodule watch regressions in isolated, permission-locked fixtures."""
from __future__ import annotations

import json
from pathlib import Path
import sys
import tempfile

from pty_smoke import ARTIFACTS, ROOT, Session
from pty_watch_permissions import (
    git, lock_owned, session_at, signature, wait_screen, watch_on,
)


def repository(path: Path) -> Path:
    path.mkdir()
    git(path, "init", "-q")
    (path / "src/nested").mkdir(parents=True)
    (path / "src/nested/file").write_text("base\n")
    (path / ".gitignore").write_text("ignored-cache/\n")
    git(path, "add", ".")
    git(path, "commit", "-qm", "isolated permission fixture")
    return path


def fixture(root: Path) -> tuple[Path, Path]:
    origin: Path = repository(root / "origin")
    parent: Path = repository(root / "superproject")
    git(parent, "-c", "protocol.file.allow=always", "submodule", "add", "-q",
        "--", str(origin), "modules/dependency")
    git(parent, "commit", "-qm", "gitlink fixture")
    return parent, origin


def dirty(session: Session, oid: str) -> None:
    wait_screen(session, lambda screen: f"{oid}-dirty" in screen,
                "automatic submodule dirty refresh")


def clean(session: Session) -> None:
    wait_screen(session, lambda screen: "No differences." in screen,
                "automatic clean submodule refresh")


def sources_and_metadata(binary: Path, root: Path) -> dict[str, object]:
    parent: Path
    origin: Path
    parent, origin = fixture(root)
    child: Path = parent / "modules/dependency"
    oid: str = git(child, "rev-parse", "HEAD").decode().strip()
    blocked: Path = child / "ignored-cache/private"
    mode: int | None = lock_owned(blocked)
    assert mode is not None, "permission checks require an unprivileged uid"
    before: tuple[int, ...] = signature(blocked.stat())
    parent_index: bytes = (parent / ".git/index").read_bytes()
    child_index: bytes = (parent / ".git/modules/modules/dependency/index").read_bytes()
    session: Session = session_at(binary, parent, root / "preferences",
                                  ["--watch", "--view", "unified"])
    try:
        session.ready()
        watch_on(session)
        clean(session)
        (child / "src/nested/file").write_text("native submodule edit\n")
        dirty(session, oid)
        (child / "src/nested/file").write_text("base\n")
        clean(session)
        assert (parent / ".git/index").read_bytes() == parent_index
        assert (parent / ".git/modules/modules/dependency/index").read_bytes() == child_index
        tree: str = git(child, "rev-parse", "HEAD^{tree}").decode().strip()
        next_oid: str = git(child, "commit-tree", tree, "-p", oid,
                            "-m", "metadata-only fixture commit").decode().strip()
        git(child, "update-ref", "HEAD", next_oid)
        wait_screen(session, lambda screen: next_oid in screen and "watch:on" in screen,
                    "automatic metadata-only submodule HEAD refresh")
        assert signature(blocked.stat()) == before
        return session.finish(b"q", "watch-permission-submodule-source-metadata")
    finally:
        session.close()
        blocked.chmod(mode)


def nested_and_untracked(binary: Path, root: Path) -> dict[str, object]:
    parent: Path
    origin: Path
    parent, origin = fixture(root)
    child: Path = parent / "modules/dependency"
    git(child, "-c", "protocol.file.allow=always", "submodule", "add", "-q",
        "--", str(origin), "nested/child")
    git(child, "commit", "-qm", "nested gitlink")
    git(parent, "add", "--", "modules/dependency")
    git(parent, "commit", "-qm", "updated gitlink")
    git(parent, "config", "diff.ignoreSubmodules", "none")
    nested: Path = child / "nested/child"
    oid: str = git(child, "rev-parse", "HEAD").decode().strip()
    untracked: Path = nested / "new/deep/file"
    untracked.parent.mkdir(parents=True)
    untracked.write_text("initial untracked dirtiness\n")
    blocked: Path = nested / "new/ignored-cache/private"
    mode: int | None = lock_owned(blocked)
    assert mode is not None, "permission checks require an unprivileged uid"
    before: tuple[int, ...] = signature(blocked.stat())
    session: Session = session_at(binary, parent, root / "preferences",
                                  ["--watch", "--view", "unified"])
    try:
        session.ready()
        watch_on(session)
        dirty(session, oid)
        untracked.unlink()
        clean(session)
        # Empty untracked directories must remain covered for later creations.
        untracked.write_text("later untracked dirtiness\n")
        dirty(session, oid)
        untracked.unlink()
        clean(session)
        (nested / "src/nested/file").write_text("deep tracked edit\n")
        dirty(session, oid)
        assert signature(blocked.stat()) == before
        return session.finish(b"q", "watch-permission-nested-submodule-untracked")
    finally:
        session.close()
        blocked.chmod(mode)


def initialize_after_enable(binary: Path, root: Path) -> dict[str, object]:
    parent: Path
    origin: Path
    parent, origin = fixture(root)
    git(parent, "submodule", "--quiet", "deinit", "-f", "--", "modules/dependency")
    session: Session = session_at(binary, parent, root / "preferences",
                                  ["--watch", "--view", "unified"])
    try:
        session.ready()
        watch_on(session)
        clean(session)
        git(parent, "-c", "protocol.file.allow=always", "submodule", "update", "--init",
            "--", "modules/dependency")
        child: Path = parent / "modules/dependency"
        oid: str = git(child, "rev-parse", "HEAD").decode().strip()
        (child / "src/nested/file").write_text("edit during initialization\n")
        dirty(session, oid)
        (child / "src/nested/file").write_text("base\n")
        clean(session)
        (child / "src/nested/file").write_text("edit after initialization\n")
        dirty(session, oid)
        return session.finish(b"q", "watch-permission-submodule-initialization")
    finally:
        session.close()


def main() -> None:
    assert len(sys.argv) <= 2, "usage: pty_watch_permission_submodules.py [path/to/riff]"
    binary: Path = Path(sys.argv[1]).resolve() if len(sys.argv) == 2 else ROOT / "target/release/riff"
    assert binary.is_file(), f"build {binary} first"
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    results: list[dict[str, object]] = []
    with tempfile.TemporaryDirectory(prefix="watch-permission-submodules-", dir=ARTIFACTS) as temporary:
        root: Path = Path(temporary).resolve()
        case: str
        for case in ("sources", "nested", "initialization"):
            directory: Path = root / case
            directory.mkdir()
            if case == "sources":
                results.append(sources_and_metadata(binary, directory))
            elif case == "nested":
                results.append(nested_and_untracked(binary, directory))
            else:
                results.append(initialize_after_enable(binary, directory))
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
