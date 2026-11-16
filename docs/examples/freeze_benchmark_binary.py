"""Freeze a newly built CLI selected from Cargo JSON, without running it.

Use the isolated-target build procedure in docs/PHASE_B_BENCHMARK_HANDOFF.md.
The receipt and build logs are private build provenance, not a benchmark.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tomllib


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def git(source: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(source), *args], text=True).strip()


def source_identity(source: Path, revision: str) -> dict:
    if not re.fullmatch(r"[0-9a-f]{40}", revision) or git(source, "rev-parse", "HEAD") != revision:
        raise ValueError("checkout must be at the requested full commit ID")
    if git(source, "status", "--porcelain", "--untracked-files=all"):
        raise ValueError("source checkout must be clean")
    names = git(source, "ls-files", "-z", "--", "Cargo.toml", "Cargo.lock", "crates", "dev",
                "tools/extract_component_classes", ".cargo").split("\0")
    files = {name: sha256(source / name) for name in names if name}
    if "Cargo.lock" not in files:
        raise ValueError("tracked Cargo.lock is required")
    return {"commit": revision, "tree": git(source, "rev-parse", "HEAD^{tree}"),
            "files": files, "files_sha256": hashlib.sha256(
                json.dumps(files, sort_keys=True, separators=(",", ":")).encode()).hexdigest()}


def select_artifact(messages: Path, source: Path, target: Path) -> tuple[dict, list[dict]]:
    records = [json.loads(line) for line in messages.read_text(encoding="utf-8").splitlines() if line]
    finished = [row for row in records if row.get("reason") == "build-finished"]
    if len(finished) != 1 or finished[0].get("success") is not True:
        raise ValueError("Cargo messages must contain one successful build-finished record")
    manifests = {path.resolve() for path in (source / "crates").glob("*/Cargo.toml")}
    artifacts = [row for row in records if row.get("reason") == "compiler-artifact"
                 and Path(row.get("manifest_path", "")).resolve() in manifests]
    seen = {Path(row["manifest_path"]).resolve() for row in artifacts}
    if not manifests or seen != manifests:
        raise ValueError("every runtime crate must have an artifact from this checkout")
    for row in artifacts:
        if row.get("fresh") is not False or row["profile"].get("test") is not False:
            raise ValueError("runtime artifacts must be newly compiled, non-test artifacts")
        if not Path(row["target"]["src_path"]).resolve().is_relative_to(source):
            raise ValueError("runtime artifact points outside the source checkout")
    binaries = [row for row in artifacts if row["target"]["name"] == "vrfkit"
                and row["target"]["kind"] == ["bin"] and row.get("executable")]
    if len(binaries) != 1:
        raise ValueError("exactly one vrfkit binary artifact is required")
    binary = binaries[0]
    if (Path(binary["manifest_path"]).resolve() != source / "crates/vrfkit/Cargo.toml"
            or Path(binary["target"]["src_path"]).resolve() != source / "crates/vrfkit/src/main.rs"):
        raise ValueError("binary artifact must identify the vrfkit CLI manifest and source")
    if (binary["profile"].get("opt_level") != "3"
            or binary["profile"].get("debuginfo") != 1
            or binary["profile"].get("debug_assertions") is not False
            or sorted(binary.get("features", [])) != ["default", "export"]):
        raise ValueError("expected release optimization and default export features")
    executable = Path(binary["executable"]).resolve()
    if not executable.is_relative_to(target) or not executable.is_file():
        raise ValueError("binary artifact must exist in the declared isolated target directory")
    return binary, artifacts


def freeze(args: argparse.Namespace) -> None:
    source, target = args.source.resolve(), args.target_dir.resolve()
    identity = source_identity(source, args.revision)
    binary, artifacts = select_artifact(args.cargo_messages, source, target)
    profile = tomllib.loads((source / "Cargo.toml").read_text(encoding="utf-8"))["profile"]["release"]
    expected_profile = {"opt-level": 3, "lto": "fat", "codegen-units": 1, "debug": 1}
    if any(profile.get(key) != value for key, value in expected_profile.items()):
        raise ValueError("source must declare the documented release profile")
    rustc = args.rustc_info.read_text(encoding="utf-8")
    cargo = args.cargo_info.read_text(encoding="utf-8")
    if "release: 1.86.0\n" not in rustc or not cargo.startswith("cargo 1.86.0 "):
        raise ValueError("compiler records must identify Rust and Cargo 1.86.0")
    executable = Path(binary["executable"]).resolve()
    original_hash = sha256(executable)
    args.output_dir.mkdir(mode=0o700)
    frozen = args.output_dir / executable.name
    shutil.copy2(executable, frozen)
    if sha256(frozen) != original_hash or sha256(executable) != original_hash:
        raise ValueError("executable changed while being frozen")
    if source_identity(source, args.revision) != identity:
        raise ValueError("source changed while recording provenance")
    env_names = {"RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
                 "CARGO_INCREMENTAL", "CARGO_BUILD_JOBS", "CC", "CXX", "CFLAGS", "CXXFLAGS", "LDFLAGS"}
    build_env = {name: value for name, value in os.environ.items()
                 if name in env_names or name.startswith("CARGO_PROFILE_")
                 or (name.startswith("CARGO_TARGET_") and name.endswith(("_RUSTFLAGS", "_LINKER")))}
    receipt = {
        "schema_version": 1, "source": identity,
        "compiler": {"rustc_verbose": rustc, "cargo_version": cargo},
        "declared_release_profile": profile, "build_environment": build_env,
        "build_command": args.build_command.read_text(encoding="utf-8"),
        "cargo_messages_sha256": sha256(args.cargo_messages),
        "binary_artifact": binary, "runtime_artifacts": artifacts,
        "frozen_executable": {"name": frozen.name, "bytes": frozen.stat().st_size,
                              "sha256": original_hash},
    }
    with (args.output_dir / "build.json").open("x", encoding="utf-8") as stream:
        json.dump(receipt, stream, indent=2)
        stream.write("\n")
    print(f"frozen {frozen.name}: {original_hash}; source {args.revision}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ("source", "target-dir", "cargo-messages", "rustc-info", "cargo-info",
                 "build-command", "output-dir"):
        parser.add_argument("--" + flag, type=Path, required=True)
    parser.add_argument("--revision", required=True)
    args = parser.parse_args()
    try:
        freeze(args)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as exc:
        parser.exit(1, f"No accepted build receipt: {exc}\n")


if __name__ == "__main__":
    main()
