"""Reproduce the Reader's same-executable input comparison.

The work directory contains private commands and logs. Publish only the
aggregate report after reviewing its input hashes. Full callback fingerprints
are checked separately with reader_input_probe --fingerprint; timing them
would measure diagnostic text formatting rather than normal Reader use.
"""

import argparse
import json
import platform
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
import benchmark_resources as bench  # noqa: E402


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("input", type=Path)
    parser.add_argument("work", type=Path)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--control", action="store_true")
    args = parser.parse_args()
    if args.repeats < 1:
        parser.error("--repeats must be positive")
    binary, replay, work = (p.resolve() for p in (args.binary, args.input, args.work))
    work.mkdir(mode=0o700)
    input_size = replay.stat().st_size
    report = {
        "status": "running",
        "equivalent": False,
        "platform": platform.platform(),
        "input_bytes": input_size,
        "input_sha256": bench.sha256_file(replay),
        "binary_sha256": bench.sha256_file(binary),
        "comparison": "file/file control" if args.control else "buffered/file",
        "samples": [],
    }
    expected = None
    report_path = work / "report.json"
    for phase, round_number, variant in bench.plan(1, args.repeats):
        mode = "file" if args.control or variant == "candidate" else "buffered"
        directory = work / f"{phase}-{round_number:02}-{variant}"
        directory.mkdir()
        sample = bench.run_worker([str(binary), mode, str(replay)], directory, 120)
        if sample["status"] != "completed" or sample["exit_code"] != 0:
            raise RuntimeError(f"failed Reader sample: {sample}")
        values = sample["measurements"]
        bench.validate_measurements(values)
        output = (directory / "parser.log").read_bytes()
        if expected is None:
            expected = output
        if output != expected:
            raise RuntimeError("Reader output counts differ")
        sample.update(phase=phase, round=round_number, variant=variant,
                      output_bytes=len(output),
                      output_sha256=bench.sha256_file(directory / "parser.log"))
        values["throughput_mib_s"] = input_size / (1024 * 1024 * values["wall_seconds"])
        report["samples"].append(sample)
        report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(phase, round_number, variant, values["wall_seconds"], flush=True)
    if (report["input_sha256"] != bench.sha256_file(replay)
            or report["binary_sha256"] != bench.sha256_file(binary)):
        raise RuntimeError("input or executable changed during measurement")
    report.update(status="completed", equivalent=True,
                  summary=bench.summarize(report["samples"]),
                  paired_ratios=bench.paired_ratios(report["samples"]))
    report["throughput_mib_s"] = {
        variant: statistics.median(
            sample["measurements"]["throughput_mib_s"] for sample in report["samples"]
            if sample["phase"] == "measured" and sample["variant"] == variant
        ) for variant in ("baseline", "candidate")
    }
    report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
