"""Independently review an immutable metadata-only Phase D candidate."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tomllib


def git(repo: Path, *args: str) -> bytes:
    return subprocess.check_output(["git", "-C", str(repo), *args])


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    assert not args.output.exists(), "Refuse to overwrite an earlier review"
    commits = {}
    inventories = {}
    for label, revision in (("baseline", args.baseline), ("candidate", args.candidate)):
        commit = git(args.repo, "rev-parse", f"{revision}^{{commit}}").decode().strip()
        tree = git(args.repo, "rev-parse", f"{revision}^{{tree}}").decode().strip()
        names = git(args.repo, "ls-tree", "-r", "--name-only", revision).decode().splitlines()
        inventories[label] = {name: git(args.repo, "show", f"{revision}:{name}") for name in names}
        commits[label] = {"commit": commit, "tree": tree}
    old, new = inventories["baseline"], inventories["candidate"]
    changed = sorted(name for name in old.keys() | new.keys() if old.get(name) != new.get(name))
    allowed_prefixes = ("docs/",)
    allowed_files = {"Cargo.toml", "Cargo.lock", "README.md", "CHANGELOG.md"}
    assert all(name in allowed_files or name.startswith(allowed_prefixes) for name in changed), changed
    old_cargo, new_cargo = (tomllib.loads(data["Cargo.toml"].decode()) for data in (old, new))
    assert old_cargo["workspace"]["package"]["version"] == "0.5.0"
    assert new_cargo["workspace"]["package"]["version"] == "0.6.0"
    old_cargo["workspace"]["package"]["version"] = "0.6.0"
    assert old_cargo == new_cargo, "Unexpected Cargo.toml change"
    production = []
    development = []
    for name in sorted(new):
        if name.startswith("crates/") and name.endswith("/Cargo.toml"):
            package = tomllib.loads(new[name].decode())["package"]
            assert package["version"] == {"workspace": True}, name
            production.append(package["name"])
        if name in ("dev/vrf-testkit/Cargo.toml", "tools/extract_component_classes/Cargo.toml"):
            package = tomllib.loads(new[name].decode())["package"]
            assert package.get("version", "0.0.0") == "0.0.0" and package["publish"] is False
            development.append(package["name"])
    assert len(production) == 10 and len(development) == 2
    old_lock, new_lock = (tomllib.loads(data["Cargo.lock"].decode()) for data in (old, new))
    changed_packages = []
    for package in old_lock["package"]:
        if package["name"] in production and "source" not in package:
            assert package["version"] == "0.5.0"
            package["version"] = "0.6.0"
            changed_packages.append(package["name"])
    assert sorted(changed_packages) == sorted(production)
    assert old_lock == new_lock, "Dependency, checksum, or lock metadata drift"
    runtime_names = [name for name in old if name.startswith(("crates/", "dev/", ".cargo/", ".github/", "tools/"))]
    candidate_runtime = [name for name in new if name.startswith(("crates/", "dev/", ".cargo/", ".github/", "tools/"))]
    assert set(runtime_names) == set(candidate_runtime), "Runtime/build/tool path-set drift"
    assert all(old[name] == new[name] for name in runtime_names), "Runtime/build/tool content drift"
    source_map = {name: digest(new[name]) for name in sorted(runtime_names)}
    result = {
        "schema_version": 1,
        "decision": "ACCEPT_METADATA_AND_CURRENT_CONTRACT_DOCS_PENDING_EXECUTED_FINAL_GATES",
        "identities": commits,
        "changed_paths": changed,
        "production_packages": sorted(production),
        "development_packages_unchanged": sorted(development),
        "runtime_build_and_tool_files_equal": len(runtime_names),
        "runtime_build_and_tool_files_sha256": digest(json.dumps(source_map, sort_keys=True, separators=(",", ":")).encode()),
        "runtime_build_and_tool_files": source_map,
        "cargo_lock_before_sha256": digest(old["Cargo.lock"]),
        "cargo_lock_after_sha256": digest(new["Cargo.lock"]),
        "dependency_resolution_unchanged": True,
        "release_profile_unchanged": True,
        "registry_publication_prepared": False,
        "runtime_tests_executed_by_this_script": 0,
        "limitations": ["This is an immutable source/metadata review, not current CI or corpus acceptance."],
    }
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps({key: result[key] for key in ("decision", "runtime_build_and_tool_files_equal", "cargo_lock_before_sha256", "cargo_lock_after_sha256")}, indent=2))


if __name__ == "__main__":
    main()
