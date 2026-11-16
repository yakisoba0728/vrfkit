"""Run the retained checkpoint public-API regression against a supplied checkout."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True, type=Path)
    parser.add_argument("--cargo", default="cargo")
    parser.add_argument("--target-dir", type=Path)
    args = parser.parse_args()
    source = args.repo.resolve() / "crates" / "vrf-schema"
    if not (source / "Cargo.toml").is_file():
        parser.error("--repo must name a vrfkit source checkout")
    evidence = Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory(prefix="vrfkit-path-budget-") as temporary:
        project = Path(temporary)
        (project / "src").mkdir()
        (project / "Cargo.toml").write_text(
            '[package]\nname = "checkpoint-path-budget-probe"\n'
            'version = "0.0.0"\nedition = "2024"\n\n[dependencies]\n'
            f"vrf-schema = {{ path = {json.dumps(str(source))} }}\n",
            encoding="utf-8",
        )
        shutil.copyfile(evidence / "checkpoint_path_budget_probe.rs", project / "src/main.rs")
        shutil.copyfile(evidence / "probe.Cargo.lock", project / "Cargo.lock")
        environment = dict(os.environ)
        environment["CARGO_BUILD_JOBS"] = "1"
        if args.target_dir is not None:
            environment["CARGO_TARGET_DIR"] = str(args.target_dir.resolve())
        return subprocess.run(
            [args.cargo, "+1.86.0", "run", "--locked", "--manifest-path", str(project / "Cargo.toml")],
            env=environment,
            check=False,
        ).returncode


if __name__ == "__main__":
    raise SystemExit(main())
