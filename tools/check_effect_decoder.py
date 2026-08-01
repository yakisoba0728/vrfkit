#!/usr/bin/env python3
"""Self-check the live shot-effect decoder against pinned wire examples.

The first nine cases are every executable example currently in
``crates/vrf-decode/src/effect.rs``: six non-empty hex blobs and the three
one-byte empty arrays.  The Rust module is a format specification only; this
script deliberately calls the Python decoder that produces the valplay bundle.

The two ``reference_*`` cases use the C# reference bundle at
``valplay/pipeline/exports/02d4d478-1dfb-4412-9a77-29ca29105a9d/events.ndjson``:

* packet 39959, ``FloatValues``: adds ``FiringState.BurstShotNumber``;
* packet 15347, ``ObjectValues``: adds a singleton ``FXC.EffectContext``.

Run with:
    python tools/check_effect_decoder.py --check

For a deliberate-corruption demonstration (which must fail):
    python tools/check_effect_decoder.py --check --corrupt rust_float_sheriff_basic
"""

from __future__ import annotations

import argparse
import math
import struct
import sys
from pathlib import Path
from typing import NamedTuple


sys.path.insert(0, str(Path(__file__).parent))
import to_valplay_bundle as bundle  # noqa: E402


class Case(NamedTuple):
    """One independent wire blob and its decoded Python result."""

    name: str
    data: bytes
    bit_count: int
    spec: bundle._EffectArraySpec
    expected: tuple[tuple[int | None, object | None], ...]


def _hex(text: str) -> bytes:
    return bytes.fromhex("".join(text.split()))


# The exact f32 represented by the C# JSON number is the comparison value.
# System.Text.Json renders a shortest round-trippable decimal, which can differ
# textually from Python's f32 promoted to f64 (for example, random seeds).
def _reference_f32(value: float) -> float:
    return struct.unpack("<f", struct.pack("<f", value))[0]


RUST_FLOAT_SHERIFF = _hex("""
    08021020390412400000803f000410200f0412400000a040
    000610203d0412400000803f000810203b04124015f9b3ce0000
""")


CASES = (
    # Rust effect.rs: packet 4368, Sheriff FloatValues.
    Case(
        "rust_float_sheriff_basic", RUST_FLOAT_SHERIFF, 400, bundle._EFFECT_FLOATS,
        ((284, 1.0), (263, 5.0), (286, 1.0), (285, -1509722752.0)),
    ),
    # Rust effect.rs: packet 17421, Classic FloatValues with YawSwitch.
    Case(
        "rust_float_yaw_switch",
        _hex("""
            0a021020390412400000803f000410200f0412400000e040
            000610203d0412400000803f000810203b04124032cb82cc
            000a10203f041240000080410000
        """),
        496,
        bundle._EFFECT_FLOATS,
        ((284, 1.0), (263, 7.0), (286, 1.0),
         (285, _reference_f32(-68573580.0)), (287, 16.0)),
    ),
    # Rust effect.rs: packet 30968, Judge FloatValues without TracerOption.
    Case(
        "rust_float_shotgun",
        _hex("""
            060210203904124000004041000410200f04124000008040
            000610203b041240ebffe44d0000
        """),
        304,
        bundle._EFFECT_FLOATS,
        ((284, 12.0), (263, 4.0), (285, 480247136.0)),
    ),
    # Rust effect.rs: packet 4368 ObjectValues.
    Case(
        "rust_object_basic",
        _hex("""
            08022020370422201d300004202035042220190400
            062030ffff062220572a000820206504222075160000
        """),
        344,
        bundle._EFFECT_OBJECTS,
        ((283, 3086), (282, 268), (65535, 2731), (306, 1466)),
    ),
    # Rust effect.rs: packet 4368, one Sheriff attack vector.
    Case(
        "rust_vector_single",
        _hex("""
            0202182013041a81026b7b179c16f0e8bf11e6b45fc0eee33f
            9417c1fc5684b1bf0000
        """),
        280,
        bundle._EFFECT_VECTORS,
        ((265, (-0.7793076561609785, 0.6228944653768754, -0.06842559500463913)),),
    ),
    # Rust effect.rs: packet 30968, twelve Judge attack vectors.
    Case(
        "rust_vector_shotgun_12",
        _hex("""
            1802182013041a8102dbc17bd5d196e5bfe0a5c85f789ee73f
            2c6acea060d67cbf0004182023041a8102d928d6ec3252e4bf
            db684947f09ae83f8ff46bf204feb23f0006182025041a8102
            cab091674d89e3bfca826e887d57e93f329135a84dc986bf00
            08182027041a81021feb5fbfc5c9e4bfbe2eacda9150e83fd4
            e0ab8708b8993f000a182029041a8102eb1e013403a6e5bf70
            586cb99487e73f211db57f12dda43f000c18202b041a8102cf
            8ff307062fe5bfbc8f8d8712eae73f9ea909fb1d4fadbf000e
            18202d041a8102b2d91d6d819be4bf868429ea797ae83f82a1
            7b5833f487bf001018202f041a81024c18cf0e9622e6bfebed
            6acd4518e73f1643e10a3d219a3f0012182031041a81021e4a
            512e352ee5bf896bbcfad9fae73fa65e98e42cf892bf001418
            2015041a81023f37da4a7fa9e3bff6ce548e533ae93f0ec90b
            be86529f3f0016182017041a8102246b3d964f35e3bf7de713
            59cc90e93f0801a7f26937a33f0018182019041a81028e462c
            93744fe3bf46adf442357ae93f7f3bcd1e38b3a6bf0000
        """),
        3184,
        bundle._EFFECT_VECTORS,
        (
            (265, (-0.6746606034849337, 0.7380945082451795, -0.0070403837716193456)),
            (273, (-0.6350340486253813, 0.7689134018248994, 0.07418852728365465)),
            (274, (-0.6105105421861705, 0.7919299759560883, -0.011126143165434674)),
            (275, (-0.649630426195788, 0.7598351736973752, 0.02511609390327936)),
            (276, (-0.6765151992521771, 0.7353004094645836, 0.040749147500316114)),
            (277, (-0.6619901805211102, 0.747323288680938, -0.0572442406599578)),
            (278, (-0.6439826136764195, 0.7649507115821883, -0.011696244371208978)),
            (279, (-0.6917219437825763, 0.7217129718841852, 0.02551741961389372)),
            (280, (-0.6618905930175207, 0.7493715188205538, -0.01852483887895908)),
            (266, (-0.6144405805550618, 0.7883699207218047, 0.030588250493516482)),
            (267, (-0.600257676541649, 0.7989255657005142, 0.03753214919152198)),
            (268, (-0.6034491419288359, 0.796167975209223, -0.04433608413693601)),
        ),
    ),
    # Rust effect.rs: the three one-byte IntPacked-zero arrays.
    Case("rust_empty_float", b"\x00", 8, bundle._EFFECT_FLOATS, ()),
    Case("rust_empty_object", b"\x00", 8, bundle._EFFECT_OBJECTS, ()),
    Case("rust_empty_vector", b"\x00", 8, bundle._EFFECT_VECTORS, ()),
    # C# reference bundle packet 39959 FloatValues, fields listed in docstring.
    Case(
        "reference_float_burst",
        _hex("""
            0a021020390412400000803f00041020330412400000803f
            000610200f0412400000e041000810203d0412400000803f
            000a10203b0412408082c04e0000
        """),
        496,
        bundle._EFFECT_FLOATS,
        (
            (284, 1.0), (281, 1.0), (263, 28.0), (286, 1.0),
            (285, _reference_f32(1614889000.0)),
        ),
    ),
    # C# reference bundle packet 15347 ObjectValues, FXC.EffectContext=1368.
    Case(
        "reference_object_effect_context_only",
        _hex("0202202065042220b1140000"),
        96,
        bundle._EFFECT_OBJECTS,
        ((306, 1368),),
    ),
    # A truncated live blob pins Python's partial-list contract; Rust returns Err.
    Case(
        "python_truncated_float_partial",
        RUST_FLOAT_SHERIFF[:-4],
        (len(RUST_FLOAT_SHERIFF) - 4) * 8,
        bundle._EFFECT_FLOATS,
        ((284, 1.0), (263, 5.0), (286, 1.0), (285, None)),
    ),
)


def _same_value(actual: object | None, expected: object | None,
                spec: bundle._EffectArraySpec) -> bool:
    if actual is None or expected is None:
        return actual is expected
    if spec is bundle._EFFECT_FLOATS:
        return actual == expected
    if spec is bundle._EFFECT_OBJECTS:
        return actual == expected
    if isinstance(actual, tuple) and isinstance(expected, tuple):
        return len(actual) == len(expected) and all(
            math.isclose(a, e, rel_tol=0.0, abs_tol=1e-15)
            for a, e in zip(actual, expected)
        )
    return False


def _corrupt(case: Case) -> Case:
    """Flip one actual wire byte so a check run proves the guard is live."""
    if not case.data:
        raise ValueError(f"cannot corrupt empty data for {case.name}")
    data = bytearray(case.data)
    mask = 2 if case.data == b"\x00" and case.expected == () else 1
    data[0] ^= mask
    return case._replace(data=bytes(data))


def _preview(elements: tuple[tuple[int | None, object | None], ...]) -> str:
    """Keep a corruption failure readable when a bad count allocates 256 slots."""
    if len(elements) <= 4:
        return repr(elements)
    return f"{elements[:4]!r} ... ({len(elements)} total)"


def check(corrupt_name: str | None = None) -> list[str]:
    """Return one failure per Python-decoder disagreement."""
    failures = []
    names = {case.name for case in CASES}
    if corrupt_name is not None and corrupt_name not in names:
        return [f"unknown case for --corrupt: {corrupt_name}"]
    for original in CASES:
        case = _corrupt(original) if original.name == corrupt_name else original
        actual = tuple(bundle._decode_effect_elements(case.data, case.bit_count, case.spec))
        if len(actual) != len(case.expected):
            failures.append(
                f"{case.name}: expected {len(case.expected)} elements, got {len(actual)}: "
                f"{_preview(actual)}"
            )
            continue
        for index, ((actual_tag, actual_value), (expected_tag, expected_value)) in enumerate(
            zip(actual, case.expected)
        ):
            if actual_tag != expected_tag or not _same_value(actual_value, expected_value, case.spec):
                failures.append(
                    f"{case.name}[{index}]: expected {(expected_tag, expected_value)!r}, "
                    f"got {(actual_tag, actual_value)!r}"
                )
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify pinned cases")
    parser.add_argument("--corrupt", metavar="CASE", help="flip one byte in CASE")
    args = parser.parse_args()
    failures = check(args.corrupt)
    if failures:
        print(f"FAILED: {len(failures)} effect decoder check(s)", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1
    print(f"OK: {len(CASES)} live effect decoder cases")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
