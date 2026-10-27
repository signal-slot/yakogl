#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Cut a board-trace excerpt for src/mali/trace.rs out of a pandecode dump.

    tools/mali/board_dump_cut.py [--from <line>] <dump.pandecode> <source> <va>...

Prints, each block headed `## <source> lines a-b` (the line range in the
dump, the header src/mali/trace.rs reads):

- the command-stream words of the first draw: from the first REQ_RESOURCE
  line to the first RUN_IDVS line, both included;
- for each <va> (hex, e.g. 7fffffffd000), the dump's memory block of the
  buffer at that GPU VA: from its `Buffer: memory_<va>` line to the line
  before the next `Buffer:` line (or the end of the file).

--from <line> (1-based) starts every search at that line of the dump: a
dump of several frames (mesa-board-ref.sh concatenates one file a frame)
names the same buffers once per frame, and the excerpt is of a later one.
The line ranges printed stay the dump's own.

Refuses (exit 1) a dump without those lines or a VA no buffer starts at, so
an excerpt never silently leaves a block out.
"""
import sys


def main():
    args = sys.argv[1:]
    origin = 0
    if args[:1] == ["--from"]:
        if len(args) < 2 or not args[1].isdigit() or int(args[1]) < 1:
            print(__doc__, file=sys.stderr)
            return 2
        origin, args = int(args[1]) - 1, args[2:]
    if len(args) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    path, source, vas = args[0], args[1], args[2:]
    lines = open(path, encoding="utf-8", errors="replace").read().split("\n")

    def first(pred, start=origin):
        return next((i for i in range(start, len(lines)) if pred(lines[i])), None)

    a = first(lambda l: "REQ_RESOURCE" in l)
    b = None if a is None else first(lambda l: "RUN_IDVS" in l, a)
    if a is None or b is None:
        print(f"board_dump_cut: {path}: no REQ_RESOURCE ... RUN_IDVS", file=sys.stderr)
        return 1
    blocks = [(a, b)]
    for va in vas:
        start = first(lambda l: l.startswith(f"Buffer: memory_{va.lower()} "))
        if start is None:
            print(f"board_dump_cut: {path}: no buffer at {va}", file=sys.stderr)
            return 1
        end = first(lambda l: l.startswith("Buffer:"), start + 1)
        end = (len(lines) if end is None else end) - 1
        while end > start and not lines[end].strip():
            end -= 1
        blocks.append((start, end))
    for s, e in blocks:
        print(f"## {source} lines {s + 1}-{e + 1}")
        print("\n".join(lines[s:e + 1]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
