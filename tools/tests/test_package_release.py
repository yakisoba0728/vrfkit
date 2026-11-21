"""Release assets must fail verification when their bytes or identity change."""
import contextlib
import hashlib
import io
import json
import os
import re
from pathlib import Path
import struct
import subprocess
import unittest
from unittest.mock import patch
import zipfile

from support import TempDirTestCase
import package_release as release


def pe_image():
    data = bytearray(256)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 60, 128)
    data[128:132] = b"PE\0\0"
    struct.pack_into("<H", data, 132, 0x8664)
    struct.pack_into("<H", data, 152, 0x20B)
    return bytes(data)


def write_cargo(root, version):
    (root / "Cargo.toml").write_text(
        f'[workspace]\nmembers = []\n\n[workspace.package]\nversion = "{version}"\n',
        encoding="utf-8")


class WorkspaceVersionTests(TempDirTestCase):
    def check(self, tag, version):
        root = self.tmp()
        write_cargo(root, version)
        release.require_workspace_version(tag, root)

    def test_tag_must_name_the_workspace_version(self):
        for tag, version in (("v0.6.0", "0.6.0"), ("v0.6.0-rc.1", "0.6.0-rc.1"),
                             ("v0.6.0+build.5", "0.6.0"), ("v0.6.0-rc.1+ci.abc", "0.6.0-rc.1"),
                             ("v0.6.0", "0.6.0+meta")):
            with self.subTest(tag=tag, version=version):
                self.check(tag, version)
        for tag, version in (("v0.5.2", "0.6.0"), ("v0.6.0-rc.1", "0.6.0"),
                             ("v0.6.0", "0.6.0-rc.1"), ("v0.6.0-rc.2", "0.6.0-rc.1"),
                             ("v0.6.1+0.6.0", "0.6.0"), ("v1.6.0", "0.6.0")):
            with self.subTest(tag=tag, version=version):
                with self.assertRaisesRegex(ValueError, "does not name the Cargo workspace version"):
                    self.check(tag, version)

    def test_missing_version_is_an_error_not_a_match(self):
        root = self.tmp()
        (root / "Cargo.toml").write_text("[workspace]\nmembers = []\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, r"no \[workspace.package\] version"):
            release.require_workspace_version("v0.6.0", root)

    def test_version_command_prints_this_repository_version(self):
        # Read independently of the tool: a regex, not tomllib.
        text = (release.REPO / "Cargo.toml").read_text(encoding="utf-8")
        section = text.split("[workspace.package]", 1)[1].split("\n[", 1)[0]
        expected = re.search(r'(?m)^version\s*=\s*"([^"]+)"', section)[1]
        with contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(release.main(["version"]), 0)
        self.assertEqual(out.getvalue(), expected + "\n")


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


class ReleasePackageTests(TempDirTestCase):
    TAG = "v1.2.3-rc.1"
    COMMIT = "a" * 40
    HELP = "\n".join("vrfkit " + command for command in ("inspect", "validate", "diag", "export"))

    def setUp(self):
        self.root = self.tmp()
        self.exe = self.root / "source.exe"
        self.exe.write_bytes(pe_image())
        for name in ("LICENSE", "NOTICE.md"):
            (self.root / name).write_text("Notice for " + name, encoding="utf-8")
        write_cargo(self.root, self.TAG[1:])
        self.output = self.root / "output"

    def verify(self, tag, commit, **kwargs):
        return release.verify_package(self.output, tag, commit, repository=self.root, **kwargs)

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
        self.assertEqual(self.verify(self.TAG, self.COMMIT), archive)
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
            self.verify(self.TAG, self.COMMIT)

    def test_checksum_cannot_refer_to_a_different_filename(self):
        archive = self.create()
        checksum = archive.with_suffix(".zip.sha256")
        checksum.write_text(checksum.read_text(encoding="ascii").replace(archive.name, "other.zip"), encoding="ascii")
        with self.assertRaisesRegex(ValueError, "filename"):
            self.verify(self.TAG, self.COMMIT)

    def test_package_cannot_be_reused_for_another_commit(self):
        self.create()
        with self.assertRaisesRegex(ValueError, "provenance"):
            self.verify(self.TAG, "b" * 40)

    def test_changed_metadata_is_rejected_even_with_a_matching_zip_checksum(self):
        self.create()
        self.rewrite(lambda entries: entries.update({"build-info.json": b'{}'}))
        with self.assertRaisesRegex(ValueError, "provenance"):
            self.verify(self.TAG, self.COMMIT)

    def test_changed_binary_is_rejected_even_with_a_matching_zip_checksum(self):
        self.create()
        self.rewrite(lambda entries: entries.update({"vrfkit.exe": pe_image() + b"changed"}))
        with self.assertRaisesRegex(ValueError, "provenance"):
            self.verify(self.TAG, self.COMMIT)

    def test_extra_path_in_zip_is_rejected_before_extraction(self):
        self.create()
        self.rewrite(lambda entries: entries.update({"../extra.exe": pe_image()}))
        with self.assertRaisesRegex(ValueError, "entries"):
            self.verify(self.TAG, self.COMMIT, smoke=True)

    def test_missing_zip_entry_is_rejected(self):
        self.create()
        self.rewrite(lambda entries: entries.pop("LICENSE"))
        with self.assertRaisesRegex(ValueError, "entries"):
            self.verify(self.TAG, self.COMMIT)

    def test_empty_notice_is_rejected(self):
        self.create()
        self.rewrite(lambda entries: entries.update({"NOTICE.md": b" \n"}))
        with self.assertRaisesRegex(ValueError, "empty"):
            self.verify(self.TAG, self.COMMIT)

    def test_failed_or_incomplete_help_cannot_pass_smoke_test(self):
        self.create()
        for result in (subprocess.CompletedProcess([], 1, self.HELP, "failure"),
                       subprocess.CompletedProcess([], 0, "vrfkit inspect", "")):
            with self.subTest(result=result), patch.object(release.subprocess, "run", return_value=result):
                with self.assertRaisesRegex(ValueError, "smoke"):
                    self.verify(self.TAG, self.COMMIT, smoke=True)

    def test_smoke_timeout_is_not_success(self):
        self.create()
        with patch.object(release.subprocess, "run", side_effect=subprocess.TimeoutExpired("vrfkit", 30)):
            with self.assertRaises(subprocess.TimeoutExpired):
                self.verify(self.TAG, self.COMMIT, smoke=True)

    def test_cli_verification_failure_exits_nonzero(self):
        # The repository's own version, so the failure is the missing package.
        tag = "v" + release.workspace_version()
        with self.assertRaises(SystemExit) as raised, contextlib.redirect_stderr(io.StringIO()) as err:
            release.main(["verify", "--tag", tag, "--commit", self.COMMIT,
                          "--directory", str(self.output)])
        self.assertEqual(raised.exception.code, 1)
        self.assertNotIn("workspace version", err.getvalue())

    def test_mismatched_tag_is_refused_before_creating_output(self):
        write_cargo(self.root, "1.2.4")
        with self.assertRaisesRegex(ValueError, "workspace version 1.2.4"):
            self.create()
        self.assertFalse(self.output.exists())

    def test_verification_rejects_a_package_whose_tag_is_not_the_workspace_version(self):
        self.create()
        self.assertTrue(self.verify(self.TAG, self.COMMIT))
        write_cargo(self.root, "1.2.3")
        with self.assertRaisesRegex(ValueError, "workspace version 1.2.3"):
            self.verify(self.TAG, self.COMMIT)

    def test_create_and_verify_cli_without_identity_is_a_usage_error(self):
        for command in ("create", "verify"):
            with self.subTest(command=command), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as raised:
                    release.main([command])
                self.assertEqual(raised.exception.code, 2)
