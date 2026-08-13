"""Small fail-closed helpers for generated files and output directories."""

from __future__ import annotations

import os
import shutil
import tempfile
from pathlib import Path


def require_descendant(path: Path, root: Path, *, allow_root: bool = False) -> Path:
    """Resolve *path* and require it to stay below the resolved *root*."""
    resolved_root = root.resolve()
    resolved_path = path.resolve()
    if resolved_path == resolved_root:
        if allow_root:
            return resolved_path
        raise ValueError(f"refusing to operate on containment root: {resolved_path}")
    if not resolved_path.is_relative_to(resolved_root):
        raise ValueError(
            f"path escapes containment root {resolved_root}: {resolved_path}"
        )
    return resolved_path


def remove_tree(path: Path, root: Path) -> None:
    """Remove a directory only after resolving and checking containment."""
    resolved = require_descendant(path, root)
    if resolved.exists():
        shutil.rmtree(resolved)


def atomic_write_text(
    path: Path,
    content: str,
    *,
    encoding: str = "utf-8",
) -> None:
    """Replace a text file atomically, leaving the old file on failure."""
    path.parent.mkdir(parents=True, exist_ok=True)
    parent = path.parent.resolve()
    target = require_descendant(path, parent)
    fd, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(fd, "w", encoding=encoding, newline="") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, target)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise
