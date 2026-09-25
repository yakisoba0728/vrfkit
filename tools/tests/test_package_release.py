"""Release assets must fail verification when their bytes or identity change."""
from contextlib import redirect_stderr
import hashlib
from io import StringIO
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import package_release as release


def pe_image():
    data = bytearray(256)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 60, 128)
    data[128:132] = b"PE\0\0"
    struct.pack_into("<H", data, 132, 0x8664)
    struct.pack_into("<H", data, 152, 0x20B)
    return bytes(data)


class ReleaseIdentityTests(unittest.TestCase):
    def test_release_and_prerelease_tags(self):
        for tag, expected in (("v0.1.0", False), ("v1.2.3+build-4", False),
                              ("v1.2.3-rc.1", True), ("v1.2.3-0+build.4", True)):
            with self.subTest(tag=tag):
                self.assertEqual(release.validate_identity(tag, "a" * 40), expected)

    def test_unsafe_and_non_semver_tags_are_rejected(self):
        for tag in ("v1", "1.2.3", "v01.2.3", "v1.2.3-01", "v1.2.3-rc..1",
                    "v1.2.3+", "v1.2.3\n", "v1.2.3/../file", "v1.2.3;echo bad",
                    "v1.2.3$(echo bad)", "v1.2.3\"", "v1.2.3-\u00e9"):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                release.validate_identity(tag, "a" * 40)

    def test_commit_must_be_a_complete_git_sha(self):
        for commit in ("a" * 7, "A" * 40, "g" * 40, "a" * 40 + "\n", ""):
            with self.subTest(commit=commit), self.assertRaises(ValueError):
                release.validate_identity("v1.2.3", commit)

    def test_wrong_or_truncated_pe_architecture_is_rejected(self):
        bad = [b"", b"MZ", pe_image()[:153]]
        for offset, value in ((0, b"NO"), (60, b"\xff" * 4), (128, b"NO\0\0"),
                              (132, b"\x4c\x01"), (132, b"\x64\xaa"), (152, b"\x0b\x01")):
            image = bytearray(pe_image())
            image[offset:offset + len(value)] = value
            bad.append(bytes(image))
        for image in bad:
            with self.subTest(image=image[:4]), self.assertRaises(ValueError):
                release.require_windows_x64(image)


class ReleasePackageTests(unittest.TestCase):
    TAG = "v1.2.3-rc.1"
    COMMIT = "a" * 40
    HELP = "\n".join("vrfkit " + command for command in ("inspect", "validate", "diag", "export"))

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.exe = self.root / "source.exe"
        self.exe.write_bytes(pe_image())
        for name in ("LICENSE", "NOTICE.md"):
            (self.root / name).write_text("Notice for " + name, encoding="utf-8")
        self.output = self.root / "output"

    def create(self, directory=None):
        def execute(args, **kwargs):
            # This is the extracted file, not the build-tree executable.
            self.assertNotEqual(Path(args[0]), self.exe)
            self.assertEqual(Path(args[0]).read_bytes(), self.exe.read_bytes())
            self.assertEqual(args[1:], ["--help"])
            self.assertEqual(kwargs["timeout"], 30)
            return subprocess.CompletedProcess(args, 0, self.HELP, "")
        with patch.object(release.subprocess, "run", side_effect=execute) as run:
            archive = release.create_package(self.exe, directory or self.output,
                                             self.TAG, self.COMMIT, repository=self.root)
        self.assertEqual(run.call_count, 1)
        return archive

    def rewrite(self, edit):
        archive = self.output / release.archive_name(self.TAG)
        with zipfile.ZipFile(archive) as bundle:
            entries = {name: bundle.read(name) for name in bundle.namelist()}
        edit(entries)
        with zipfile.ZipFile(archive, "w") as bundle:
            for name, data in entries.items():
                bundle.writestr(name, data)
        checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
        archive.with_suffix(".zip.sha256").write_text(f"{checksum}  {archive.name}\n", encoding="ascii")

    def test_round_trip_verifies_provenance_and_required_files(self):
        archive = self.create()
        self.assertEqual(release.verify_package(self.output, self.TAG, self.COMMIT), archive)
        with zipfile.ZipFile(archive) as bundle:
            self.assertEqual(set(bundle.namelist()), release.CONTENTS)
            metadata = json.loads(bundle.read("build-info.json"))
            self.assertEqual(metadata["commit"], self.COMMIT)
            self.assertEqual(metadata["tag"], self.TAG)
            self.assertEqual(bundle.read("LICENSE"), (self.root / "LICENSE").read_bytes())

    def test_archive_is_reproducible_across_source_timestamp_changes(self):
        first = self.create()
        os.utime(self.exe, (1_600_000_000, 1_600_000_000))
        second = self.create(self.root / "second")
        self.assertEqual(first.read_bytes(), second.read_bytes())

    def test_existing_output_is_never_overwritten(self):
        archive = self.create()
        original = archive.read_bytes()
        with self.assertRaises(FileExistsError):
            release.create_package(self.exe, self.output, self.TAG, self.COMMIT, repository=self.root)
        self.assertEqual(archive.read_bytes(), original)

    def test_missing_notice_fails_before_creating_output(self):
        (self.root / "NOTICE.md").unlink()
        with self.assertRaises(FileNotFoundError):
            release.create_package(self.exe, self.output, self.TAG, self.COMMIT, repository=self.root)
        self.assertFalse(self.output.exists())

    def test_changed_archive_fails_checksum_verification(self):
        archive = self.create()
        archive.write_bytes(archive.read_bytes() + b"changed")
        with self.assertRaisesRegex(ValueError, "checksum"):
            release.verify_package(self.output, self.TAG, self.COMMIT)

    def test_checksum_cannot_refer_to_a_different_filename(self):
        archive = self.create()
        checksum = archive.with_suffix(".zip.sha256")
        checksum.write_text(checksum.read_text(encoding="ascii").replace(archive.name, "other.zip"), encoding="ascii")
        with self.assertRaisesRegex(ValueError, "filename"):
            release.verify_package(self.output, self.TAG, self.COMMIT)

    def test_package_cannot_be_reused_for_another_commit(self):
        self.create()
        with self.assertRaisesRegex(ValueError, "provenance"):
            release.verify_package(self.output, self.TAG, "b" * 40)

    def test_changed_metadata_is_rejected_even_with_a_matching_zip_checksum(self):
        self.create()
        self.rewrite(lambda entries: entries.update({"build-info.json": b'{}'}))
        with self.assertRaisesRegex(ValueError, "provenance"):
            release.verify_package(self.output, self.TAG, self.COMMIT)

    def test_changed_binary_is_rejected_even_with_a_matching_zip_checksum(self):
        self.create()
        self.rewrite(lambda entries: entries.update({"vrfkit.exe": pe_image() + b"changed"}))
        with self.assertRaisesRegex(ValueError, "provenance"):
            release.verify_package(self.output, self.TAG, self.COMMIT)

    def test_extra_path_in_zip_is_rejected_before_extraction(self):
        self.create()
        self.rewrite(lambda entries: entries.update({"../extra.exe": pe_image()}))
        with self.assertRaisesRegex(ValueError, "entries"):
            release.verify_package(self.output, self.TAG, self.COMMIT, smoke=True)

    def test_missing_zip_entry_is_rejected(self):
        self.create()
        self.rewrite(lambda entries: entries.pop("LICENSE"))
        with self.assertRaisesRegex(ValueError, "entries"):
            release.verify_package(self.output, self.TAG, self.COMMIT)

    def test_empty_notice_is_rejected(self):
        self.create()
        self.rewrite(lambda entries: entries.update({"NOTICE.md": b" \n"}))
        with self.assertRaisesRegex(ValueError, "empty"):
            release.verify_package(self.output, self.TAG, self.COMMIT)

    def test_failed_or_incomplete_help_cannot_pass_smoke_test(self):
        self.create()
        for result in (subprocess.CompletedProcess([], 1, self.HELP, "failure"),
                       subprocess.CompletedProcess([], 0, "vrfkit inspect", "")):
            with self.subTest(result=result), patch.object(release.subprocess, "run", return_value=result):
                with self.assertRaisesRegex(ValueError, "smoke"):
                    release.verify_package(self.output, self.TAG, self.COMMIT, smoke=True)

    def test_smoke_timeout_is_not_success(self):
        self.create()
        with patch.object(release.subprocess, "run", side_effect=subprocess.TimeoutExpired("vrfkit", 30)):
            with self.assertRaises(subprocess.TimeoutExpired):
                release.verify_package(self.output, self.TAG, self.COMMIT, smoke=True)

    def test_cli_verification_failure_exits_nonzero(self):
        with redirect_stderr(StringIO()), self.assertRaises(SystemExit) as raised:
            release.main(["verify", "--tag", self.TAG, "--commit", self.COMMIT,
                          "--directory", str(self.output)])
        self.assertEqual(raised.exception.code, 1)


if __name__ == "__main__":
    unittest.main()
