#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Sort every run of consecutive register-read and invalidate lines ("R ...",
"I ...") of a tools/pvr/km record.

The render service's diagnostic log lines read registers and invalidate
buffers inside their argument lists (pvr_render.c: render_debug_after, the
stall dump, the PVR_RENDER_VERBOSE dumps). C leaves the order of argument
evaluation unspecified (the host compiler evaluates them right to left), Rust
evaluates left to right, so the two records list those reads in different
orders. Writes, barriers, cleans, waits, log lines and results keep their
order; only the reads between two of them are compared as a set.

  rec_normalize.py < record > normalized
"""
import sys


def main():
    run = []
    out = sys.stdout
    for line in sys.stdin:
        if line.startswith("R ") or line.startswith("I "):
            run.append(line)
            continue
        if run:
            out.writelines(sorted(run))
            run = []
        out.write(line)
    if run:
        out.writelines(sorted(run))


if __name__ == "__main__":
    main()
