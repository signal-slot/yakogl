#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Read and decode the A53 fault record the Zephyr samples leave in DDR.

The i.MX8MP board has no UART, so a Zephyr run reports through the panel and
through the time Linux takes to come back. Both are lossy: the panel needs
eyes, and the return time carries one small number. The fault record is the
third channel and the only exact one -- gc_sample.c writes the syndrome into a
page of carveout DDR at the FIRST instruction of the fatal handler, before any
printing, and Linux leaves that page no-map, so it survives the SoC reset and
can be read over ssh afterwards.

It answers the question M5's return time cannot. A hold of 15 s and a return of
22 s is consistent with Zephyr's fatal handler running, and equally with the
watchdog resetting a board that never reached the handler at all. The three
magic words separate those:

  A53F at word 0   the fatal handler RAN, and words 1..9 say why
  A53P at word 10  the image got as far as some progress mark (a breadcrumb
                   a hang leaves and a fault also leaves)
  A53M at word 15  the page was mapped and writable at init -- so an otherwise
                   empty record means "nothing was written", not "nothing could
                   be written"

The layout is read off the writer in
driver/zephyr/lib/board/imx8mp/gc_sample.c (fault_record_store,
gc_sample_progress, fault_record_map); if that changes, this must change with
it, and the self-test below is what catches the drift.

  tools/cts/fault-record.py --board torizon@<imx8mp>
  tools/cts/fault-record.py --file page.bin
  tools/cts/fault-record.py --self-test
"""
import argparse
import struct
import subprocess
import sys

FAULT_MAGIC = 0x46333541     # 'A53F'
PROGRESS_MAGIC = 0x50333541  # 'A53P'
CHANNEL_MAGIC = 0x4D333541   # 'A53M'
DEFAULT_ADDR = 0x81006000
RECORD_WORDS = 16

# ESR_EL1.EC, the values gc_sample.c's ENCODE_EC comment names, plus the ones
# a reader meets in practice. Anything else is printed as a bare number rather
# than guessed at.
EC_NAMES = {
    0x15: "SVC (k_panic / k_oops: the code chose to die)",
    0x20: "instruction abort, lower EL",
    0x21: "instruction abort, same EL (a jump into nothing)",
    0x24: "data abort, lower EL",
    0x25: "data abort, same EL (bad or unmapped pointer)",
    0x2F: "SError (asynchronous -- often a bus error under the GPU)",
    0x22: "PC alignment fault",
    0x26: "SP alignment fault",
    0x2C: "floating point exception",
}

# ESR_EL1.ISS[5:0] for an abort, the DFSC/IFSC.
def dfsc_name(dfsc):
    if 0x00 <= dfsc <= 0x03:
        return f"address size fault, level {dfsc & 3}"
    if 0x04 <= dfsc <= 0x07:
        return f"translation fault, level {dfsc & 3}"
    if 0x08 <= dfsc <= 0x0B:
        return f"access flag fault, level {dfsc & 3}"
    if 0x0C <= dfsc <= 0x0F:
        return f"permission fault, level {dfsc & 3}"
    if dfsc == 0x10:
        return "synchronous external abort"
    if dfsc == 0x21:
        return "alignment fault"
    return f"DFSC 0x{dfsc:02x}"


def decode(page):
    """page: at least 64 bytes. Returns (verdict, [lines])."""
    if len(page) < RECORD_WORDS * 4:
        return "SHORT", [f"only {len(page)} bytes: the record is {RECORD_WORDS * 4}"]
    w = struct.unpack("<16I", page[: RECORD_WORDS * 4])
    out = []
    faulted = w[0] == FAULT_MAGIC
    progressed = w[10] == PROGRESS_MAGIC
    mapped = w[15] == CHANNEL_MAGIC

    if faulted:
        esr = w[2] | (w[3] << 32)
        far = w[4] | (w[5] << 32)
        elr = w[6] | (w[7] << 32)
        ec = (esr >> 26) & 0x3F
        out.append(f"FAULT  reason {w[1]}")
        out.append(f"       ESR 0x{esr:016x}  EC 0x{ec:02x} = "
                   f"{EC_NAMES.get(ec, 'unknown exception class')}")
        if ec in (0x20, 0x21, 0x24, 0x25):
            out.append(f"       {dfsc_name(esr & 0x3F)}")
        out.append(f"       FAR 0x{far:016x}   (the address it touched)")
        out.append(f"       ELR 0x{elr:016x}   (the faulting PC: look it up in zephyr.elf)")
        out.append(f"       note 0x{w[8]:08x}   uptime {w[9]} ms")
    if progressed:
        out.append(f"MARK   last progress mark {w[11]} at {w[12]} ms, {w[13]} marks in total")
    if mapped:
        out.append(f"PAGE   mapped and writable at init, {w[14]} ms into the run")

    if faulted:
        verdict = "FAULT"
    elif progressed:
        verdict = "NO-FAULT"
        out.append("")
        out.append("The fatal handler did NOT run: the image reached a progress mark and")
        out.append("then stopped without faulting. The reset came from somewhere else --")
        out.append("WDOG1 unfed, or a reset path that bypasses Zephyr entirely.")
    elif mapped:
        verdict = "MAPPED-ONLY"
        out.append("")
        out.append("The page was writable but nothing else was ever written: the image")
        out.append("died before the first progress mark.")
    else:
        verdict = "EMPTY"
        out.append("no magic word at all: either this image has no fault record")
        out.append("(CONFIG_GC_SAMPLE_FAULT_RECORD), the address is wrong, or the page")
        out.append("did not survive -- and those are NOT the same as 'it did not fault'")
    return verdict, out


def read_board(host, addr, size):
    """The page is no-map and belongs to no driver, so this is /dev/mem, and
    /dev/mem is root.

    The scoped wrapper is tried first and is the supported route:
    fault-record-dump takes NO arguments and has the address baked in, so the
    sudoers rule it needs is not a read-anything primitive. Bare `sudo dd` is
    tried second only because a lab board may predate the wrapper."""
    import base64

    attempts = [
        ("fault-record-dump", "sudo -n /usr/local/sbin/fault-record-dump"),
        ("sudo dd", f"sudo -n dd if=/dev/mem bs=1 skip={addr} count={size} "
                    f"status=none 2>/dev/null | base64 -w0"),
    ]
    problems = []
    for name, cmd in attempts:
        r = subprocess.run(["ssh", "-o", "BatchMode=yes", host, cmd],
                           capture_output=True, text=True, timeout=120)
        if r.returncode == 0 and r.stdout.strip():
            try:
                blob = base64.b64decode(r.stdout.strip())
            except Exception as e:          # noqa: BLE001 - report, do not guess
                problems.append(f"{name}: output was not base64 ({e})")
                continue
            if len(blob) >= size:
                return blob
            problems.append(f"{name}: got {len(blob)} bytes, wanted {size}")
        else:
            problems.append(f"{name}: {r.stderr.strip()[:120] or 'no output'}")

    sys.exit(
        "fault-record: could not read the record on this board.\n"
        "  Tried: " + "; ".join(problems) + "\n"
        "  Install the wrapper with driver/tools/kexec-zephyr/board-sudo-setup.sh\n"
        "  (it adds /usr/local/sbin/fault-record-dump, no arguments, one fixed\n"
        "  address). THIS IS NOT EVIDENCE THAT THE BOARD DID NOT FAULT: a run\n"
        "  whose record could not be read has no reason recorded, and saying so\n"
        "  is the only honest report.")


def self_test():
    """The decoder is an instrument, so it gets checked against pages whose
    answer is known -- including the two that are easy to confuse, a record
    that never faulted and a record that was never written."""
    def page(**kw):
        w = [0] * RECORD_WORDS
        for k, v in kw.items():
            w[int(k[1:])] = v
        return struct.pack("<16I", *w)

    # A data abort at the same EL, translation fault level 2.
    esr = (0x25 << 26) | 0x06
    v, lines = decode(page(w0=FAULT_MAGIC, w1=4, w2=esr, w4=0xDEADBEEF,
                           w6=0x60001234, w9=1234, w10=PROGRESS_MAGIC,
                           w11=7, w15=CHANNEL_MAGIC))
    assert v == "FAULT", v
    body = "\n".join(lines)
    assert "EC 0x25" in body and "translation fault, level 2" in body, body
    assert "0x00000000deadbeef" in body, body

    v, lines = decode(page(w10=PROGRESS_MAGIC, w11=3, w13=9, w15=CHANNEL_MAGIC))
    assert v == "NO-FAULT", v
    assert "did NOT run" in "\n".join(lines)

    v, lines = decode(page(w14=42, w15=CHANNEL_MAGIC))
    assert v == "MAPPED-ONLY", v
    assert "42 ms into the run" in "\n".join(lines)

    v, lines = decode(page())
    assert v == "EMPTY", v
    assert "NOT the same as" in "\n".join(lines)

    v, _ = decode(b"\x00" * 8)
    assert v == "SHORT", v

    # An SVC: the k_panic shape, which is what a failed assertion looks like.
    v, lines = decode(page(w0=FAULT_MAGIC, w2=(0x15 << 26), w15=CHANNEL_MAGIC))
    assert v == "FAULT" and "SVC" in "\n".join(lines)
    print("fault-record: self-test OK (6 pages)")


# ---- the writer contract ------------------------------------------------
#
# --self-test proves the decoder is self-consistent. It cannot prove the
# decoder still matches gc_sample.c, because both the writer's offsets and this
# file's copy of them are typed by hand: move `far` from word 4 to word 5 and
# every test above still passes while the reader silently prints the wrong
# address. So this reads the WRITER and checks it word by word.
#
# The source is the authority. If gc_sample.c changes shape on purpose, this
# table is what has to be updated with it, and the failure names the word.
WRITER_SOURCE = "driver/zephyr/lib/board/imx8mp/gc_sample.c"

# word -> (name, a pattern the writer's right-hand side must match)
WRITER_WORDS = {
    0:  ("fault magic", r"FAULT_RECORD_MAGIC"),
    1:  ("reason", r"\breason\b"),
    2:  ("ESR low", r"\(uint32_t\)esr"),
    3:  ("ESR high", r"esr >> 32"),
    4:  ("FAR low", r"\(uint32_t\)far"),
    5:  ("FAR high", r"far >> 32"),
    6:  ("ELR low", r"\(uint32_t\)elr"),
    7:  ("ELR high", r"elr >> 32"),
    8:  ("note", r"s_fault_note"),
    9:  ("fault uptime", r"k_uptime_get_32"),
    10: ("progress magic", r"PROGRESS_MAGIC"),
    11: ("progress mark", r"\bmark\b"),
    12: ("progress uptime", r"k_uptime_get_32"),
    13: ("progress count", r"\+\+"),
    14: ("map uptime", r"k_uptime_get_32"),
    15: ("channel magic", r"CHANNEL_MAGIC"),
}


def check_writer(path):
    import os
    import re
    if not os.path.exists(path):
        sys.exit(f"fault-record: cannot read the writer at {path}; "
                 "this check is not optional -- fix the path")
    src = open(path).read()

    # 1. the magic values, as the writer defines them
    for name, want in (("FAULT_RECORD_MAGIC", FAULT_MAGIC),
                       ("PROGRESS_MAGIC", PROGRESS_MAGIC),
                       ("CHANNEL_MAGIC", CHANNEL_MAGIC)):
        m = re.search(rf"#define\s+{name}\s+(0x[0-9A-Fa-f]+)", src)
        if not m:
            sys.exit(f"fault-record: {name} is not defined in {path} any more")
        got = int(m.group(1), 16)
        if got != want:
            sys.exit(f"fault-record: {name} is 0x{got:08x} in the writer but "
                     f"0x{want:08x} here -- the decoder has drifted")

    # 2. every word the writer assigns, against what this file believes
    found = {}
    for m in re.finditer(r"(?:^|\W)(?:s_fault_rec|r)\[(\d+)\]\s*(=[^=]|\+\+)([^;]*);",
                         src, re.M):
        idx, op, rhs = int(m.group(1)), m.group(2), m.group(3)
        found.setdefault(idx, []).append(rhs if op.startswith("=") else "++")

    problems = []
    for idx, (name, pattern) in WRITER_WORDS.items():
        if idx not in found:
            problems.append(f"word {idx} ({name}) is no longer written by {path}")
            continue
        if not any(re.search(pattern, r) for r in found[idx]):
            problems.append(f"word {idx} ({name}): writer has "
                            f"{found[idx]!r}, which does not match /{pattern}/")
    for idx in sorted(set(found) - set(WRITER_WORDS)):
        problems.append(f"word {idx} is written by {path} and this decoder "
                        f"knows nothing about it: {found[idx]!r}")
    if problems:
        sys.exit("fault-record: the decoder and its writer have drifted\n  "
                 + "\n  ".join(problems))

    # 3. round trip: plant values at the WRITER's offsets and decode them
    esr = (0x25 << 26) | 0x06
    w = [0] * RECORD_WORDS
    w[0], w[1] = FAULT_MAGIC, 3
    w[2], w[3] = esr & 0xFFFFFFFF, 0
    w[4], w[5] = 0x1000BEEF, 0x77
    w[6], w[7] = 0xCAFE0000, 0x60
    w[8], w[9] = 0xABCD, 4242
    w[10], w[11], w[12], w[13] = PROGRESS_MAGIC, 5, 99, 2
    w[14], w[15] = 11, CHANNEL_MAGIC
    verdict, lines = decode(struct.pack("<16I", *w))
    body = "\n".join(lines)
    assert verdict == "FAULT", verdict
    # the 64-bit halves must be recombined the way the writer split them
    assert "0x000000771000beef" in body, body
    assert "0x00000060cafe0000" in body, body
    assert "EC 0x25" in body and "translation fault, level 2" in body, body
    assert "reason 3" in body and "4242 ms" in body, body
    assert "mark 5 at 99 ms, 2 marks" in body, body
    assert "11 ms into the run" in body, body
    print(f"fault-record: writer contract OK "
          f"({len(WRITER_WORDS)} words checked against {path})")


def main():
    ap = argparse.ArgumentParser()
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--board", metavar="user@host")
    g.add_argument("--file", metavar="page.bin")
    g.add_argument("--self-test", action="store_true")
    g.add_argument("--check-writer", nargs="?", const=WRITER_SOURCE,
                   metavar="gc_sample.c",
                   help="check the offsets and magics against the writer itself")
    ap.add_argument("--addr", type=lambda s: int(s, 0), default=DEFAULT_ADDR,
                    help="CONFIG_GC_SAMPLE_FAULT_RECORD_ADDR (default 0x81006000)")
    a = ap.parse_args()

    if a.self_test:
        self_test()
        return 0
    if a.check_writer:
        check_writer(a.check_writer)
        return 0
    page = read_board(a.board, a.addr, RECORD_WORDS * 4) if a.board \
        else open(a.file, "rb").read()
    verdict, lines = decode(page)
    print(f"fault record at 0x{a.addr:08x}: {verdict}")
    for line in lines:
        print("  " + line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
