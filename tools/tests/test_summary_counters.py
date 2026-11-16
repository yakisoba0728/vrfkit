"""The summary-counter spec against the Rust that prints the summary.

Every line must be one summary.rs prints, each key must name the argument at
its position (read off summary.rs, not the spec), each pattern must match
exactly one printed line, and a line that is not printed must read as missing.
"""
import re
import tempfile
import unittest
from pathlib import Path

from support import REPO
import summary_counters as sc

DRIVER = REPO / "crates" / "vrfkit" / "src"
SUMMARY_RS = DRIVER / "driver" / "summary.rs"
REPORT_RS = DRIVER / "report.rs"

#: Keys whose Rust argument is spelled differently; every other key, `cp_`
#: dropped, equals its argument with the receiver path joined by `_`.
RUST_NAMES = {
    "chunks": "chunks_processed", "packets": "total_packets", "cp_frame_packets": "packets",
    "frame_non_finite_times": "non_finite_frame_times",
    "cp_frame_non_finite_times": "non_finite_frame_times",
    "overlay_decode_errors": "overlay_decoded_err", "cp_overlay_decode_errors": "overlay_decoded_err",
    "overlay_raw_skip": "overlay_raw_or_skip", "cp_overlay_raw_skip": "overlay_raw_or_skip",
    "overlay_rows_offered": "total",
    **{f"{cp}sink_{name}": name for cp in ("", "cp_")
       for name in ("actor_opens", "actor_closes", "content_blocks")},
}


def split_args(text: str) -> list[str]:
    """Top-level comma-separated arguments."""
    args, depth, current = [], 0, ""
    for ch in text:
        depth += (ch in "([") - (ch in ")]")
        if ch == "," and depth == 0:
            args.append(current.strip())
            current = ""
        else:
            current += ch
    return [a for a in args + [current.strip()] if a]


def printed_lines() -> list[tuple[str, list[str], bool]]:
    """`(format, argument expressions, in print_checkpoints)` for every
    `eprintln!` in summary.rs, whitespace collapsed, each `report::` formatter
    expanded into its own format string and arguments, inline `{name}`
    placeholders taking `name` as their argument."""
    source = SUMMARY_RS.read_text(encoding="utf-8")
    formatters = {name: (fmt, split_args(args)) for name, fmt, args in re.findall(
        r'pub fn (\w+)\([^)]*\) -> String \{\s*format!\(\s*"([^"]*)",(.*?)\)\s*\}',
        REPORT_RS.read_text(encoding="utf-8"), re.S)}
    assert "frame_skips" in formatters, "report.rs formatters were not found"
    start = source.index("fn print_checkpoints(")
    end = source.index("\nfn ", start + 1)
    out = []
    for m in re.finditer(r'eprintln!\(\s*"((?:[^"\\]|\\.)*)"(.*?)\);', source, re.S):
        fmt, args = " ".join(m.group(1).split()), split_args(m.group(2))
        if args and args[0].startswith("report::"):
            call = re.match(r"report::(\w+)\(", args[0]).group(1)
            fmt = fmt.replace("{}", formatters[call][0], 1)
            args = formatters[call][1] + args[1:]
        named = iter(args)
        args = [next(named) if p == "{}" else p[1:-1] for p in sc.PLACEHOLDER.findall(fmt)]
        out.append((fmt, args, start < m.start() < end))
    assert len(out) > 100, "summary.rs literals were not found"
    return out


def rust_name(expr: str) -> str:
    expr = re.sub(r"^&?(totals|net_stats|cp)\.", "", expr)
    expr = re.sub(r"^(sink|net)\.", "", expr).replace("skips.", "frame_")
    return expr.replace(".", "_")


class SpecTests(unittest.TestCase):
    def test_every_line_is_printed_and_each_key_names_its_argument(self):
        printed = {fmt: (args, checkpoint) for fmt, args, checkpoint in printed_lines()}
        for line in sc.SPEC:
            with self.subTest(line=line.fmt):
                self.assertIn(line.fmt, printed, "summary.rs prints no such line")
                args, checkpoint = printed[line.fmt]
                self.assertEqual(len(args), len(line.keys))
                for key, arg in zip(line.keys, args):
                    self.assertEqual(key.startswith("cp_"), checkpoint, key)
                    self.assertEqual(RUST_NAMES.get(key, key.removeprefix("cp_")),
                                     rust_name(arg), f"{key} reads {arg}")

    def test_each_pattern_matches_exactly_one_printed_line(self):
        """A pattern matching two lines reads whichever comes first."""
        rendered = [sc.PLACEHOLDER.sub("7", fmt) for fmt, _, _ in printed_lines()]
        for line in sc.SPEC:
            with self.subTest(line=line.fmt):
                hits = [text for text in rendered if line.pattern.search("  " + text + "\n")]
                self.assertEqual(len(hits), 1, hits)

    def test_each_key_reads_its_own_number_and_a_dropped_line_reads_as_missing(self):
        numbers = {key: 11 + 13 * i for i, key in enumerate(sc.WHERE)}
        printed = ["  " + sc.render(line, numbers).replace(",", "") for line in sc.SPEC]
        self.assertEqual(sc.read("\n".join(printed), sc.WHERE), numbers)
        for i, line in enumerate(sc.SPEC):
            with self.subTest(line=line.fmt):
                got = sc.read("\n".join(printed[:i] + printed[i + 1:]), sc.WHERE)
                self.assertEqual(got, {k: None if k in line.keys else v
                                       for k, v in numbers.items()})

    def test_a_label_quoted_in_free_text_is_not_read(self):
        """`Struct blob err:` prints free text; a quoted 0 ahead of the real
        line would pass a replay with 7 leaf errors."""
        quoted = "  Struct blob err:  stopped near Array leaf errs: 0 / Trailing bytes: 0\n"
        real = "  Array leaf errs:  7\n  Trailing bytes:   9\n"
        wanted = ("array_leaf_decode_errors", "cp_trailing_bytes")
        for text in (quoted + real, real + quoted):
            self.assertEqual(sc.read(text, wanted), dict(zip(wanted, (7, 9))))
        self.assertEqual(sc.read(quoted, wanted), dict.fromkeys(wanted))

    def test_labels_name_the_line_and_the_count(self):
        self.assertEqual([sc.label(k) for k in ("overlay_decode_errors", "array_errors",
                                                "movement_component_errors",
                                                "cp_overlay_decode_errors",
                                                "cp_array_leaf_decode_errors")],
                         ["Decode errors", "Array decode errors", "Movement components errors",
                          "Checkpoint Overlay errors", "Checkpoint leaf typed decode errors"])


class MovementContractTests(unittest.TestCase):
    def counters(self):
        return dict(zip(sc.MOVEMENT_COUNTERS, (4, 2, 2, 1, 3, 80, 9, 2, 1)),
                    movement_rpc_errors=3)

    def test_failed_prefixes_empty_components_and_orphans_reconcile(self):
        counters = self.counters()
        self.assertEqual(sc.movement_contract_errors(counters, require_work=True), [])
        checkpoint = {"cp_" + key: value for key, value in counters.items()}
        self.assertEqual(sc.movement_contract_errors(checkpoint, "cp_"), [])
        empty = dict.fromkeys((*sc.MOVEMENT_COUNTERS, "movement_rpc_errors"), 0)
        self.assertEqual(sc.movement_contract_errors(empty), [])
        self.assertIn("no movement component decoding evidence",
                      " ".join(sc.movement_contract_errors(empty, require_work=True)))

    def test_each_native_counter_is_required_and_must_be_a_nonnegative_integer(self):
        for key in (*sc.MOVEMENT_COUNTERS, "movement_rpc_errors"):
            for value in (None, True, -1, "0", 0.0, "absent"):
                with self.subTest(key=key, value=value):
                    counters = self.counters()
                    if value == "absent":
                        del counters[key]
                    else:
                        counters[key] = value
                    self.assertIn(key, " ".join(sc.movement_contract_errors(counters)))

    def test_bookkeeping_cannot_claim_unobserved_headers_or_discarded_bits(self):
        for key, value in (("streams", 5), ("complete", 4), ("empty", 3),
                           ("envelopes", 5), ("payload_bytes", 1), ("unused_bits", 15),
                           ("input_headers", 0), ("input_headers", 3),
                           ("discrepancy_headers", 3)):
            with self.subTest(key=key, value=value):
                counters = self.counters()
                counters["movement_component_" + key] = value
                self.assertTrue(sc.movement_contract_errors(counters))
        counters = self.counters()
        counters["movement_rpc_errors"] = 1
        self.assertTrue(sc.movement_contract_errors(counters))

    def test_legacy_counters_are_never_converted_or_accepted_as_extra_keys(self):
        for prefix in ("", "cp_"):
            for key in sc.LEGACY_MOVEMENT_COUNTERS:
                counters = {prefix + key: value for key, value in self.counters().items()}
                counters[prefix + key] = 0
                self.assertEqual(sc.movement_contract_errors(counters, prefix),
                                 [sc.OBSOLETE_MOVEMENT_CONTRACT])


class PinOrDiffTests(unittest.TestCase):
    def test_a_refused_update_writes_nothing_and_drift_fails(self):
        def diff(want, got):
            return [] if want == got else ["moved"]
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "baseline.json"
            self.assertEqual(sc.pin_or_diff(path, {}, {"n": 1}, True, ["unmeasured"], diff), 1)
            self.assertFalse(path.exists())
            self.assertEqual(sc.pin_or_diff(path, {}, {"n": 1}, False, [], diff), 2)
            self.assertEqual(sc.pin_or_diff(path, {}, {"n": 1}, True, [], diff), 0)
            stored = sc.load_baseline(path)
            self.assertIsNone(sc.pin_or_diff(path, stored, {"n": 1}, False, [], diff))
            self.assertEqual(sc.pin_or_diff(path, stored, {"n": 2}, False, [], diff), 1)
