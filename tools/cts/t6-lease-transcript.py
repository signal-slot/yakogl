#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Turn the lease's serial_read output into the plain transcript the
NanoPC-T6 U-Boot tools (prompt-reads.py, prompt-go.py) read.

usage: t6-lease-transcript.py OUT IN... [--from-offset N]
           [--from-line REGEX] [--to-line REGEX]

IN is a saved serial_read result: plain text (one chunk, no offsets) or a
JSON object {"text"|"data"|"output"|"content": str, "offset"|"start_offset"
|"since": N, "next_offset": M, "truncated": bool}. The text may carry
control bytes as \\xNN escapes; they are decoded back to bytes.
Chunks are joined in offset order; a chunk must start where the previous
one's next_offset ended.

Output: LF lines, CR, NUL and 0xFF removed, cut from the first line matching
--from-line (default 'Hit any key to stop autoboot') to the end, or to the
first later line matching --to-line (included).

Exit: 0 ok, 2 usage, 3 input unusable (unreadable, no from-line match),
4 offset gap or overlap (or offsets missing with several inputs).
"""
import json
import re
import sys

DEFAULT_FROM = r"Hit any key to stop autoboot"
TEXT_KEYS = ("text", "data", "output", "content")
START_KEYS = ("offset", "start_offset", "since", "from_offset")
NEXT_KEYS = ("next_offset", "end_offset")


class Fail(Exception):
    def __init__(self, code, msg):
        super().__init__(msg)
        self.code = code


def unescape(s):
    """Text -> bytes; \\xNN becomes the byte, \\\\ one backslash."""
    out = bytearray()
    i, n = 0, len(s)
    while i < n:
        c = s[i]
        if c == "\\" and i + 3 < n and s[i + 1] == "x" and \
                re.fullmatch(r"[0-9a-fA-F]{2}", s[i + 2:i + 4]):
            out.append(int(s[i + 2:i + 4], 16))
            i += 4
        elif c == "\\" and i + 1 < n and s[i + 1] == "\\" and \
                s[i + 2:i + 3] == "x":
            out.append(0x5c)
            i += 2
        else:
            out.extend(c.encode("utf-8", "surrogateescape"))
            i += 1
    return bytes(out)


def first(d, keys):
    for k in keys:
        if k in d and d[k] is not None:
            return d[k]
    return None


def load(path):
    try:
        with open(path, "rb") as f:
            raw = f.read()
    except OSError as e:
        raise Fail(3, f"{path}: {e}")
    text = raw.decode("utf-8", "surrogateescape")
    obj = None
    if text.lstrip().startswith("{"):
        try:
            obj = json.loads(text)
        except ValueError:
            obj = None
    if not isinstance(obj, dict):
        return {"path": path, "start": None, "next": None,
                "data": unescape(text), "truncated": False}
    body = first(obj, TEXT_KEYS)
    if not isinstance(body, str):
        raise Fail(3, f"{path}: JSON has none of {TEXT_KEYS}")
    data = unescape(body)
    start = first(obj, START_KEYS)
    nxt = first(obj, NEXT_KEYS)
    for v in (start, nxt):
        if v is not None and not isinstance(v, int):
            raise Fail(3, f"{path}: offset is not an integer")
    if start is None and nxt is not None:
        start = nxt - len(data)
    if nxt is None and start is not None:
        nxt = start + len(data)
    if start is not None and nxt != start + len(data):
        raise Fail(4, f"{path}: next_offset {nxt} != offset {start} + "
                      f"{len(data)} bytes")
    return {"path": path, "start": start, "next": nxt, "data": data,
            "truncated": bool(obj.get("truncated"))}


def join(chunks, from_offset):
    if len(chunks) > 1 and any(c["start"] is None for c in chunks):
        raise Fail(4, "several inputs, but one has no offsets")
    if len(chunks) > 1:
        chunks = sorted(chunks, key=lambda c: (c["start"], c["next"]))
    buf = bytearray()
    pos = None
    for c in chunks:
        if c["start"] is not None:
            if pos is not None and c["start"] != pos:
                kind = "overlap" if c["start"] < pos else "gap"
                raise Fail(4, f"offset {kind}: {c['path']} starts at "
                              f"{c['start']}, previous ended at {pos}")
            pos = c["next"]
        buf.extend(c["data"])
    base = chunks[0]["start"]
    if from_offset is not None and base is not None:
        buf = buf[max(0, from_offset - base):]
    return bytes(buf)


def normalize(data):
    data = data.replace(b"\r", b"").replace(b"\x00", b"").replace(b"\xff", b"")
    return data.decode("utf-8", "replace").split("\n")


def main(argv):
    out = None
    ins, opts = [], {"--from-line": DEFAULT_FROM, "--to-line": None,
                     "--from-offset": None}
    i = 0
    args = argv[1:]
    while i < len(args):
        a = args[i]
        if a in opts:
            if i + 1 >= len(args):
                raise Fail(2, f"{a} needs a value")
            opts[a] = args[i + 1]
            i += 2
        elif a.startswith("--"):
            raise Fail(2, f"unknown option {a}")
        else:
            if out is None:
                out = a
            else:
                ins.append(a)
            i += 1
    if out is None or not ins:
        raise Fail(2, "usage: t6-lease-transcript.py OUT IN... "
                      "[--from-offset N] [--from-line REGEX] "
                      "[--to-line REGEX]")
    try:
        fo = None if opts["--from-offset"] is None else int(opts["--from-offset"])
        rf = re.compile(opts["--from-line"])
        rt = None if opts["--to-line"] is None else re.compile(opts["--to-line"])
    except (ValueError, re.error) as e:
        raise Fail(2, str(e))
    lines = normalize(join([load(p) for p in ins], fo))
    for k, ln in enumerate(lines):
        if rf.search(ln):
            lines = lines[k:]
            break
    else:
        raise Fail(3, f"no line matches --from-line {opts['--from-line']!r}")
    if rt is not None:
        for k, ln in enumerate(lines):
            if k > 0 and rt.search(ln):
                lines = lines[:k + 1]
                break
        else:
            raise Fail(3, f"no line after the start matches --to-line "
                          f"{opts['--to-line']!r}")
    while lines and lines[-1] == "":
        lines.pop()
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines) + "\n")
    print(f"t6-lease-transcript: {len(lines)} lines -> {out}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv))
    except Fail as e:
        print(f"t6-lease-transcript: {e}", file=sys.stderr)
        sys.exit(e.code)
