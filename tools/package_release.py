"""Package and verify the Windows x64 CLI before publishing a tagged release.

The ZIP is deterministic for identical inputs. Verification binds its contents
to the requested tag and source commit; smoke tests run the extracted binary.
No command in this tool creates tags, releases, or uploads assets.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess
import tempfile
import zipfile

REPO = Path(__file__).resolve().parents[1]
TAG = re.compile(r"v(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
                 r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
                 r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?", re.ASCII)
CONTENTS = {"vrfkit.exe", "LICENSE", "NOTICE.md", "build-info.json"}


def validate_identity(tag: str, commit: str) -> bool:
    """Return whether a valid SemVer tag is a prerelease; reject unsafe labels."""
    match = TAG.fullmatch(tag)
    if not match or any(part.isdigit() and len(part) > 1 and part[0] == "0"
                        for part in (match[1] or "").split(".")):
        raise ValueError("tag must be vMAJOR.MINOR.PATCH with optional SemVer suffixes")
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("commit must be the full lowercase Git SHA")
    return match[1] is not None


def require_windows_x64(data: bytes) -> None:
    if len(data) < 64 or data[:2] != b"MZ":
        raise ValueError("executable is not a Windows PE image")
    offset = struct.unpack_from("<I", data, 60)[0]
    if (offset + 26 > len(data) or data[offset:offset + 4] != b"PE\0\0"
            or struct.unpack_from("<H", data, offset + 4)[0] != 0x8664
            or struct.unpack_from("<H", data, offset + 24)[0] != 0x20B):
        raise ValueError("executable must be Windows x64 (AMD64, PE32+)")


def archive_name(tag: str) -> str:
    return f"vrfkit-{tag}-windows-x64.zip"


def verify_package(directory: Path, tag: str, commit: str, *, smoke: bool = False) -> Path:
    validate_identity(tag, commit)
    archive = directory / archive_name(tag)
    expected = hashlib.sha256(archive.read_bytes()).hexdigest() + "  " + archive.name + "\n"
    if archive.with_suffix(".zip.sha256").read_text(encoding="ascii") != expected:
        raise ValueError("ZIP checksum or checksum filename does not match")
    with zipfile.ZipFile(archive) as bundle:
        names = bundle.namelist()
        if len(names) != len(CONTENTS) or set(names) != CONTENTS:
            raise ValueError("ZIP has unexpected, missing, or duplicate entries")
        data = {name: bundle.read(name) for name in names}
    require_windows_x64(data["vrfkit.exe"])
    metadata = {"tag": tag, "commit": commit, "target": "x86_64-pc-windows-msvc",
                "executable_sha256": hashlib.sha256(data["vrfkit.exe"]).hexdigest()}
    if json.loads(data["build-info.json"]) != metadata:
        raise ValueError("ZIP provenance does not match the tag, commit, or executable")
    if any(not data[name].strip() for name in ("LICENSE", "NOTICE.md")):
        raise ValueError("ZIP license or notice is empty")
    if smoke:
        with tempfile.TemporaryDirectory(prefix="vrfkit-release-") as temporary:
            executable = Path(temporary) / "vrfkit.exe"
            executable.write_bytes(data["vrfkit.exe"])
            run = subprocess.run([str(executable), "--help"], cwd=temporary,
                                 capture_output=True, text=True, encoding="utf-8",
                                 errors="replace", timeout=30)
            if run.returncode != 0 or any(f"vrfkit {command}" not in run.stdout
                                          for command in ("inspect", "validate", "diag", "export")):
                raise ValueError("extracted executable failed its --help smoke test")
    return archive


def create_package(executable: Path, directory: Path, tag: str, commit: str,
                   *, repository: Path = REPO) -> Path:
    validate_identity(tag, commit)
    binary = executable.read_bytes()
    require_windows_x64(binary)
    metadata = {"tag": tag, "commit": commit, "target": "x86_64-pc-windows-msvc",
                "executable_sha256": hashlib.sha256(binary).hexdigest()}
    data = {"vrfkit.exe": binary, "LICENSE": (repository / "LICENSE").read_bytes(),
            "NOTICE.md": (repository / "NOTICE.md").read_bytes(),
            "build-info.json": (json.dumps(metadata, indent=2, sort_keys=True) + "\n").encode()}
    directory.mkdir(parents=True, exist_ok=False)
    archive = directory / archive_name(tag)
    with zipfile.ZipFile(archive, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as bundle:
        for name, payload in sorted(data.items()):
            entry = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            entry.create_system = 3
            entry.external_attr = 0o100644 << 16
            bundle.writestr(entry, payload, compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(".zip.sha256").write_text(f"{digest}  {archive.name}\n", encoding="ascii", newline="\n")
    return verify_package(directory, tag, commit, smoke=True)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("create", "verify"))
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--exe", type=Path)
    parser.add_argument("--smoke", action="store_true", help="run the extracted EXE (Windows only)")
    args = parser.parse_args(argv)
    try:
        if args.command == "create":
            if args.exe is None:
                parser.error("create requires --exe")
            archive = create_package(args.exe, args.directory, args.tag, args.commit)
        else:
            archive = verify_package(args.directory, args.tag, args.commit, smoke=args.smoke)
    except (OSError, ValueError, zipfile.BadZipFile, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"release package: {error}\n")
    print(f"Verified: {archive.name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
