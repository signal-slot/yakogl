#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Read or write the case slot of the STM32MP257F-DK's dEQP image.

The DK's board layer has no SNVS record, so its dEQP image cannot take the
i.MX8MP's run-time selector. Instead driver/samples/zephyr/deqp (Kconfig
DEQP_CASE_SLOT, src/main.cpp) carries a fixed block in .rodata, which this
tool finds by its magic in zephyr.bin (or zephyr.elf) and writes after the
build, before tools/board-stm32mp2.sh stages the image:

  offset  0  magic     24 bytes, "yakogl-deqp-case-slot/1" and a NUL
  offset 24  size      uint32 LE, CONFIG_DEQP_CASE_SLOT_SIZE (the filter's room)
  offset 28  selector  int32 LE, -1 none, else an index into DEQP_CASE_TABLE
  offset 32  length    uint32 LE, 0 none, else the filter's length in bytes
  offset 36  filter    `size` bytes, a --deqp-case value, NUL-terminated

The image runs the filter when one is written, else the DEQP_CASE_TABLE entry
the selector names (tools/cts/board-stage.sh builds the DK image with the
29-row tools/cts/imx8mp-full-tree.txt as its table), else CONFIG_DEQP_CASE.

  mp2-deqp-slot.py show IMAGE
  mp2-deqp-slot.py write IMAGE --selector N [--table TREE] [-o OUT]
  mp2-deqp-slot.py write IMAGE --filter 'dEQP-GLES2.functional.a.*,...' [-o OUT]
  mp2-deqp-slot.py write IMAGE --case-list FILE [-o OUT]
  mp2-deqp-slot.py write IMAGE --clear [-o OUT]

--case-list reads one case name or pattern per line ('#' starts a comment,
blank lines are skipped) and joins them with ','. Without -o the image is
written in place. Every write is read back and checked; nothing outside the
slot changes. Exit 0 done, 1 the slot or the request does not fit, 2 usage.
"""
import argparse
import os
import struct
import sys

MAGIC = b"yakogl-deqp-case-slot/1\0"
HEADER = struct.Struct("<24sIiI")  # magic, size, selector, length
assert HEADER.size == 36


class SlotError(Exception):
    pass


def find(data):
    """The slot's offset and header fields; exactly one magic in `data`."""
    at = data.find(MAGIC)
    if at < 0:
        raise SlotError("no case slot in the image (built without CONFIG_DEQP_CASE_SLOT?)")
    if data.find(MAGIC, at + 1) >= 0:
        raise SlotError("the slot's magic occurs more than once in the image")
    if at + HEADER.size > len(data):
        raise SlotError("the slot's header runs past the end of the image")
    _, size, selector, length = HEADER.unpack_from(data, at)
    if size == 0 or at + HEADER.size + size > len(data):
        raise SlotError(f"slot size {size} does not fit the image")
    return at, size, selector, length


def read(data):
    at, size, selector, length = find(data)
    raw = bytes(data[at + HEADER.size: at + HEADER.size + size])
    if length:
        if length >= size or raw[length] != 0 or b"\0" in raw[:length]:
            raise SlotError(f"slot filter length {length} does not hold together")
        flt = raw[:length].decode("ascii")
    else:
        flt = ""
    return {"offset": at, "size": size, "selector": selector, "filter": flt}


def table_rows(path):
    with open(path, encoding="utf-8") as f:
        return [ln.strip() for ln in f if ln.strip() and not ln.lstrip().startswith("#")]


def case_list(path):
    out = []
    with open(path, encoding="utf-8") as f:
        for ln in f:
            ln = ln.split("#", 1)[0].strip()
            if ln:
                out.append(ln)
    return out


def check_filter(flt):
    if not flt:
        raise SlotError("an empty filter: use --clear")
    try:
        flt.encode("ascii")
    except UnicodeEncodeError:
        raise SlotError("the filter is not ASCII")
    for p in flt.split(","):
        if not p or p != p.strip() or any(c.isspace() for c in p):
            raise SlotError(f"bad pattern {p!r} in the filter (empty, or with whitespace)")
        if not p.startswith("dEQP-"):
            raise SlotError(f"pattern {p!r} does not start with dEQP-")


def write(data, selector, flt):
    """Write selector and filter into `data` (a bytearray) in place."""
    at, size, _, _ = find(data)
    enc = flt.encode("ascii")
    if len(enc) >= size:
        raise SlotError(f"the filter is {len(enc)} bytes, the slot holds {size - 1} (CONFIG_DEQP_CASE_SLOT_SIZE)")
    HEADER.pack_into(data, at, MAGIC, size, selector, len(enc))
    body = at + HEADER.size
    data[body: body + size] = enc + b"\0" * (size - len(enc))


def show(s, out=sys.stdout):
    print(f"mp2-deqp-slot: slot at 0x{s['offset']:x}, room {s['size'] - 1} bytes", file=out)
    print(f"mp2-deqp-slot: selector {s['selector']}", file=out)
    if s["filter"]:
        pats = s["filter"].split(",")
        print(f"mp2-deqp-slot: filter {len(s['filter'])} bytes, {len(pats)} pattern(s)", file=out)
        for p in pats:
            print(f"  {p}", file=out)
    else:
        print("mp2-deqp-slot: filter none", file=out)
    if s["filter"]:
        print("mp2-deqp-slot: the image runs the filter", file=out)
    elif s["selector"] >= 0:
        print(f"mp2-deqp-slot: the image runs DEQP_CASE_TABLE entry {s['selector']}", file=out)
    else:
        print("mp2-deqp-slot: the image runs CONFIG_DEQP_CASE (slot not written)", file=out)


def main(argv):
    ap = argparse.ArgumentParser(prog="mp2-deqp-slot.py", description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    sp = sub.add_parser("show")
    sp.add_argument("image")
    wp = sub.add_parser("write")
    wp.add_argument("image")
    g = wp.add_mutually_exclusive_group(required=True)
    g.add_argument("--selector", type=int)
    g.add_argument("--filter")
    g.add_argument("--case-list")
    g.add_argument("--clear", action="store_true")
    wp.add_argument("--table", help="the tree file the image's DEQP_CASE_TABLE was built from: checks the index")
    wp.add_argument("-o", "--out", help="write here instead of in place")
    a = ap.parse_args(argv)

    try:
        with open(a.image, "rb") as f:
            data = bytearray(f.read())
        if a.cmd == "show":
            show(read(data))
            return 0
        selector, flt = -1, ""
        if a.selector is not None:
            if a.selector < 0:
                raise SlotError("a selector is 0 or more (use --clear for none)")
            if a.table:
                rows = table_rows(a.table)
                if a.selector >= len(rows):
                    raise SlotError(f"selector {a.selector} is past {a.table}'s {len(rows)} rows")
                print(f"mp2-deqp-slot: row {a.selector} of {a.table}: {rows[a.selector]}")
            selector = a.selector
        elif a.filter is not None:
            flt = a.filter
            check_filter(flt)
        elif a.case_list:
            names = case_list(a.case_list)
            if not names:
                raise SlotError(f"{a.case_list} names no case")
            flt = ",".join(names)
            check_filter(flt)
        before = bytes(data)
        write(data, selector, flt)
        back = read(data)
        if back["selector"] != selector or back["filter"] != flt:
            raise SlotError("read-back differs from what was written")
        at, size = back["offset"], back["size"]
        end = at + HEADER.size + size
        if before[:at] != bytes(data[:at]) or before[end:] != bytes(data[end:]):
            raise SlotError("bytes outside the slot changed")
        out = a.out or a.image
        tmp = out + ".slot-tmp"
        with open(tmp, "wb") as f:
            f.write(data)
        os.replace(tmp, out)
        with open(out, "rb") as f:
            again = read(f.read())
        if again["selector"] != selector or again["filter"] != flt:
            raise SlotError(f"{out}: read-back from the file differs from what was written")
        print(f"mp2-deqp-slot: wrote {out}")
        show(again)
        return 0
    except SlotError as e:
        print(f"mp2-deqp-slot: {e}", file=sys.stderr)
        return 1
    except OSError as e:
        print(f"mp2-deqp-slot: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
