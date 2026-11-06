#!/usr/bin/env python3
"""Fail if an animation's batch evaluation has stopped vectorising here.

Noise is only cheap across many points when the compiler vectorises the loop
over them, and nothing else notices if it stops, because the values come out
the same. So this builds `tunnels_model` in release and counts packed float
multiplies in the functions holding those loops.

Each function holds two loops, plain and pulsed:

              both vectorised   one   neither
    x86_64          66         40-42     16
    arm64           50         24-26      0

x86 keeps some with neither, because the compiler still pairs a few operations
within one point. The minimums sit between the first two columns, so losing
either loop fails. Run from the repository root; pass an assembly file and a
machine name to check existing output instead of building.
"""

import os
import platform
import re
import subprocess
import sys
from pathlib import Path

FUNCTIONS = ("values_on_figure", "values_on_elements")

# Apple's assembly syntax puts the lane shape on the mnemonic.
ARM = (re.compile(r"\s+fmul(\.4s\s|\s+v\d+\.4s)"), 40)
PACKED_MULTIPLY = {
    "x86_64": (re.compile(r"\s+mulps\b"), 50),
    "arm64": ARM,
    "aarch64": ARM,
}

FUNCTION_LABEL = re.compile(r'^"?(_[^\s:"]+)"?:')


def build_assembly() -> Path:
    subprocess.run(
        ["cargo", "rustc", "--release", "-p", "tunnels_model", "--", "--emit=asm"],
        check=True,
    )
    deps = Path(os.environ.get("CARGO_TARGET_DIR", "target")) / "release" / "deps"
    assembly = max(
        deps.glob("tunnels_model-*.s"),
        key=lambda path: path.stat().st_mtime,
        default=None,
    )
    if assembly is None:
        sys.exit(f"no tunnels_model assembly in {deps}")
    return assembly


def packed_multiplies(assembly: Path, packed: re.Pattern) -> dict[str, int]:
    counts: dict[str, int] = {}
    current = None
    with assembly.open() as lines:
        for line in lines:
            label = FUNCTION_LABEL.match(line)
            if label:
                current = next((f for f in FUNCTIONS if f in label.group(1)), None)
                if current:
                    counts.setdefault(current, 0)
            elif current and packed.match(line):
                counts[current] += 1
    return counts


def main() -> int:
    if len(sys.argv) == 3:
        assembly, machine = Path(sys.argv[1]), sys.argv[2]
    else:
        assembly, machine = build_assembly(), platform.machine()
    packed, minimum = PACKED_MULTIPLY[machine]
    counts = packed_multiplies(assembly, packed)

    failed = False
    for function in FUNCTIONS:
        count = counts.get(function)
        if count is None:
            print(f"{function}: not found in {assembly}")
            failed = True
        elif count < minimum:
            print(f"{function}: {count} packed multiplies, under {minimum}: not vectorised")
            failed = True
        else:
            print(f"{function}: {count} packed multiplies")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
