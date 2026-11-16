"""Reconstruct the measured source trees and relocate copies of archived runners.

Creates new detached worktrees and local commits only. Does not build, run
benchmarks, download replays, modify an existing worktree, or push anything.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def git(repo: Path, *args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=repo, text=True).strip()


def reconstruct(repo: Path, output: Path) -> dict:
    archive = Path(__file__).resolve().parent
    manifest = json.loads((archive / "reconstruction.json").read_text(encoding="utf-8"))
    repo, output = repo.resolve(), output.resolve()
    if output.exists():
        raise ValueError("reconstruction output must be a new directory")
    if output.is_relative_to(repo):
        raise ValueError("reconstruction output must be outside the existing clone")
    base = manifest["public_base"]
    if git(repo, "rev-parse", base["commit"] + "^{tree}") != base["tree"]:
        raise ValueError("public baseline tree differs from the recorded baseline")
    for step in manifest["snapshots"]:
        if sha256(archive / step["patch"]) != step["patch_sha256"]:
            raise ValueError("source patch hash mismatch")
    for item in manifest["harnesses"]:
        if sha256(archive / item["file"]) != item["sha256"]:
            raise ValueError("archived harness hash mismatch")

    output.mkdir(parents=True)
    receipt = {
        "schema_version": 1,
        "public_base": base,
        "reconstructor_sha256": sha256(Path(__file__)),
        "snapshots": [],
        "harnesses": [],
        "timings_run": False,
    }
    variants = (("schema-budget", 1), ("schema-reuse-baseline", 2), ("schema-reuse", 3))
    for name, count in variants:
        worktree = output / "worktrees" / name
        git(repo, "-c", "core.autocrlf=false", "worktree", "add", "--detach",
            str(worktree), base["commit"])
        for step in manifest["snapshots"][:count]:
            if git(worktree, "write-tree") != step["expected_parent_tree"]:
                raise ValueError("patch parent tree mismatch")
            patch = str(archive / step["patch"])
            git(worktree, "apply", "--check", "--index", patch)
            git(worktree, "apply", "--index", patch)
            if git(worktree, "write-tree") != step["expected_tree"]:
                raise ValueError("reconstructed source tree mismatch")
        expected = manifest["snapshots"][count - 1]["expected_tree"]
        # These new local commit IDs keep the copied runner's HEAD provenance
        # accurate. Their metadata need not match the historical experiment.
        git(worktree, "-c", "user.name=vrfkit reproduction",
            "-c", "user.email=reproduction@localhost.invalid", "commit", "-m",
            "Reconstruct measured source tree " + expected)
        if git(worktree, "rev-parse", "HEAD^{tree}") != expected:
            raise ValueError("reconstruction commit changed the source tree")
        if sha256(worktree / "Cargo.lock") != manifest["cargo_lock_sha256"]:
            raise ValueError("reconstructed lockfile hash mismatch")
        receipt["snapshots"].append({
            "worktree": name,
            "source_tree": expected,
            "local_commit": git(worktree, "rev-parse", "HEAD"),
        })

    example = manifest["registration_example"]
    source = output / "worktrees/schema-reuse" / example["candidate_path"]
    if sha256(source) != example["sha256"]:
        raise ValueError("registration example hash mismatch")
    destination = output / "worktrees/schema-reuse-baseline" / example["candidate_path"]
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    receipt["baseline_example_overlay"] = {
        "path": example["candidate_path"], "sha256": sha256(destination),
        "tracked": False,
    }

    for item in manifest["harnesses"]:
        source = archive / item["file"]
        destination = output / item["restore_path"]
        destination.parent.mkdir(parents=True, exist_ok=True)
        data = source.read_bytes()
        relocated = item["restore_path"].startswith("evidence/")
        if relocated:
            lines = data.decode("utf-8").splitlines(keepends=True)
            matches = [i for i, line in enumerate(lines) if line.startswith("ROOT = Path(")]
            if len(matches) != 1:
                raise ValueError("expected exactly one archived ROOT assignment")
            lines[matches[0]] = "ROOT = Path(" + repr(str(output)) + ")\n"
            data = "".join(lines).encode("utf-8")
        destination.write_bytes(data)
        receipt["harnesses"].append({
            "path": item["restore_path"],
            "archived_sha256": item["sha256"],
            "restored_sha256": sha256(destination),
            "root_assignment_relocated": relocated,
        })
    receipt_path = output / "evidence/reproduction-receipt.json"
    receipt_path.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    return receipt


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True, help="clone containing the public base")
    parser.add_argument("--out", type=Path, required=True, help="new experiment directory")
    args = parser.parse_args()
    receipt = reconstruct(args.repo, args.out)
    for snapshot in receipt["snapshots"]:
        print(snapshot["worktree"], snapshot["source_tree"])
    print("Verified source trees and harness hashes; no build or timing was run.")


if __name__ == "__main__":
    main()
