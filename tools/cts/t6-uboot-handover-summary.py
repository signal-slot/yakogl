#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The measured U-Boot hand-over of the NanoPC-T6 as one plain-text block.

    python3 -I tools/cts/t6-uboot-handover-summary.py READS GO

READS is a prompt-reads record (driver/tools/uboot-t6/prompt-reads.sh): the
directory, or its answers.txt ("## command" and the command's output, as
prompt-reads.py writes it). GO is a t6_cold record of prompt-go.sh: the
directory, or its image.log (the verdict.txt next to it is read too).

The block gives
  - the prompt reads and what they say: PD_GPU (the PMU words, decided as
    prompt-reads.py decides it), the GPU clock words, CLKGATE_CON(15) with
    WDT0's two gates (bits 0 and 1, open when both read 0), GLB_RST_ST and
    GLB_RST_CON (raw: section 6 of docs/notes/nanopc-t6-uboot-handover.md
    has no bit decoding of them yet), WDT0's CR and TORR and its identity
    words (read only when the gates read open);
  - which of keep, move or refuse gpu_clock_set took, with the words written
    and read back (the image's "clock set:" line, board::nanopc_t6_gpu);
  - the supply read (the RK806 line of the hand-over) and the enable the
    image sent, PD_GPU at hand-over and after (t6_z1.c's lines), the stop
    line when the image stopped, the verdict;
  - the names of the two records it rests on.

After those, three more blocks (appended): the UART2 lines of the prompt reads
("# UART2 ..." in answers.txt), the image's "t6: GIC active state" line (a
"no redistributor with affinity" line means no frame matched or the wake
timed out), and the feeder's "first feed at N ms" line, reported absent when
missing (the feeder timer never ticked).

A value that was not read or whose line is absent prints "not read". Nothing
is guessed: keep, move and refuse follow gpu_clock_set's own outcomes
(ClockSet.kept; stop 9 NoTableRate and stop 10 ReadBack or Rate are the
refusals), and a clock line that fits none says "not decided".

Input that holds an address is refused, not echoed: an rfc2217:// URL, a
/dev/tty path, a MAC, a 100.64/10 address. The scan covers the md.l
answers of READS (not printenv or bdinfo, which carry the board's own MAC
and IP by nature), all of image.log and verdict.txt, and the names of the
two records. Only the record directories' base names are printed.

Exit: 0 the block was printed; 2 usage; 3 refused (an address in the input);
4 a record could not be read (no answers.txt or image.log).
"""

import os
import re
import sys

PMU = 0xFD8D8000
PWR_GATE, IDLE_REQ, IDLE_ACK, IDLE_ST, REPAIR_ST = 0x14C, 0x10C, 0x118, 0x120, 0x290
CLKGATE_CON15 = 0xFD7C083C
GLB_RST_ST, GLB_RST_CON = 0xFD7C0C04, 0xFD7C0C10
WDT0 = 0xFEAF0000
NR = "not read"

ADDRESS = (
    ("an rfc2217 URL", re.compile(r"rfc2217://", re.I)),
    ("a /dev/tty path", re.compile(r"/dev/tty")),
    ("a MAC address", re.compile(r"(?<![0-9a-f])(?:[0-9a-f]{2}[:-]){5}[0-9a-f]{2}(?![0-9a-f])", re.I)),
    ("a 100.64/10 address", re.compile(r"(?<![\d.])100\.(?:6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.\d{1,3}\.\d{1,3}(?![\d])")),
)


class Refused(Exception):
    pass


class Unreadable(Exception):
    pass


def scan(text, what):
    for kind, rx in ADDRESS:
        if rx.search(text):
            raise Refused("refused: %s holds %s; not echoed, redact the record first" % (what, kind))


def slurp(path):
    try:
        with open(path, "r", errors="replace") as f:
            return f.read()
    except OSError as e:
        raise Unreadable("cannot read %s: %s" % (os.path.basename(path), e.strerror))


def locate(arg, name, extra=()):
    """(record name, {file: text or None}) for a directory or a file."""
    arg = arg.rstrip("/") or arg
    if os.path.isdir(arg):
        d, rec = arg, os.path.basename(os.path.abspath(arg))
        main = os.path.join(d, name)
    else:
        main, rec = arg, os.path.basename(os.path.dirname(os.path.abspath(arg)))
        d = os.path.dirname(os.path.abspath(arg))
        if os.path.basename(arg) != name:
            rec = os.path.basename(arg)
    scan(rec, "a record name")
    files = {name: slurp(main)}
    for e in extra:
        p = os.path.join(d, e)
        files[e] = slurp(p) if os.path.isfile(p) else None
    return rec, files


# ---- prompt reads ---------------------------------------------------------

def sections(text):
    """{command: output} of answers.txt; '# ' note lines are kept apart."""
    out, notes, cur = {}, [], None
    for line in text.splitlines():
        if line.startswith("## "):
            cur = line[3:]
            out[cur] = []
        elif line.startswith("# ") and cur is not None and not out[cur]:
            notes.append(line[2:])
        elif cur is not None:
            out[cur].append(line)
    return {k: "\n".join(v) for k, v in out.items()}, notes


def md_words(answer, addr, count):
    """The count words md.l printed from addr, or None if any is missing."""
    words = {}
    for line in answer.splitlines():
        m = re.match(r"([0-9a-f]{8}): ((?:[0-9a-f]{8}(?: |$)){1,4})", line)
        if not m:
            continue
        a = int(m.group(1), 16)
        for i, w in enumerate(m.group(2).split()):
            words[a + 4 * i] = int(w, 16)
    got = [words.get(addr + 4 * i) for i in range(count)]
    return None if None in got else got


def cmd_md(addr, count):
    return "md.l 0x%08x %x" % (addr, count)


def read(ans, addr, count=1):
    return md_words(ans.get(cmd_md(addr, count), ""), addr, count)


def hx(w):
    return NR if w is None else "0x%08x" % w


def reads_block(ans, notes):
    L = []

    def word(name, addr, decode=None):
        r = read(ans, addr)
        w = None if r is None else r[0]
        s = "  %-22s %s: %s" % (name, "0x%08x" % addr, hx(w))
        if w is not None and decode:
            s += "  " + decode(w)
        L.append(s)
        return w

    gate = word("PMU PWR_GATE", PMU + PWR_GATE, lambda w: "(bit 0 %s)" % ("set: off" if w & 1 else "clear"))
    for n, a in (("PMU BUS_IDLE_REQ", IDLE_REQ), ("PMU BUS_IDLE_ACK", IDLE_ACK), ("PMU BUS_IDLE_ST", IDLE_ST)):
        word(n, PMU + a)
    rep = word("PMU REPAIR_STATUS", PMU + REPAIR_ST, lambda w: "(bit 1 %s)" % ("set" if w & 2 else "clear"))
    if gate is None or rep is None:
        pd = NR
    else:
        pd = "on" if gate & 1 == 0 and rep & 2 == 2 else "off"
    L.append("  PD_GPU at the prompt: %s (PWR_GATE bit 0 clear and REPAIR_STATUS bit 1 set = on)" % pd)
    if pd == "on":
        qos = []
        for base in (0xFDF35000, 0xFDF35200, 0xFDF35400, 0xFDF35600):
            w = read(ans, base + 8, 5)
            qos.append("qos@0x%08x: %s" % (base, NR if w is None else " ".join("0x%08x" % x for x in w)))
        L += ["  " + q for q in qos]
    else:
        L.append("  GPU QoS words: %s (read only when PD_GPU reads on)" % NR)

    word("CLKSEL_CON(158)", 0xFD7C0578)
    word("CLKSEL_CON(159)", 0xFD7C057C)
    word("CLKGATE_CON(66)", 0xFD7C0908)
    word("MODE_CON0", 0xFD7C0280)
    for n, a in (("CPLL PLL_CON(0..6)", 0xFD7C01A0), ("GPLL PLL_CON(0..6)", 0xFD7C01C0)):
        w = read(ans, a, 7)
        L.append("  %-22s 0x%08x: %s" % (n, a, NR if w is None else " ".join("%08x" % x for x in w)))

    g = word("CLKGATE_CON(15)", CLKGATE_CON15)
    if g is None:
        gates = NR
        L.append("  WDT0 gates (PCLK_WDT0 bit 0, TCLK_WDT0 bit 1): %s" % NR)
    else:
        gates = "open" if g & 3 == 0 else "closed"
        L.append("  WDT0 gates (PCLK_WDT0 bit 0, TCLK_WDT0 bit 1): PCLK %s, TCLK %s -> %s" % (
            "gated" if g & 1 else "open", "gated" if g & 2 else "open", gates))
    word("GLB_RST_ST", GLB_RST_ST)
    word("GLB_RST_CON", GLB_RST_CON)

    w = read(ans, WDT0, 2) if gates == "open" else None
    if gates != "open":
        L.append("  WDT0 WDT_CR / WDT_TORR: %s (gates %s: WDT0 is not read)" % (NR, gates if g is not None else NR))
    elif w is None:
        L.append("  WDT0 WDT_CR / WDT_TORR: %s" % NR)
    else:
        L.append("  WDT0 WDT_CR 0x%08x (WDT_EN %d, RMOD %d, RPL %d), WDT_TORR 0x%08x (TOP %d)" % (
            w[0], w[0] & 1, w[0] >> 1 & 1, w[0] >> 2 & 7, w[1], w[1] & 0xF))
    i = read(ans, WDT0 + 0xF4, 3) if gates == "open" else None
    if i is None:
        L.append("  WDT0 COMP_PARAM_1 / COMP_VERSION / COMP_TYPE: %s" % NR)
    else:
        L.append("  WDT0 COMP_PARAM_1 0x%08x, COMP_VERSION 0x%08x, COMP_TYPE 0x%08x" % tuple(i))
    for n in notes:
        if n.startswith("WDT0") or n.startswith("PD_GPU"):
            L.append("  prompt-reads: " + n)
    return L


# ---- the go record --------------------------------------------------------

HEX = r"0x([0-9a-f]+)"
PD_RX = re.compile(r"^t6_\w+: PD_GPU (at hand-over|after): PWR_GATE 0x([0-9a-f]{8}) STATUS 0x([0-9a-f]{8}) "
                   r"BUS_IDLE_REQ 0x([0-9a-f]{8}) ACK 0x([0-9a-f]{8}) ST 0x([0-9a-f]{8}) "
                   r"REPAIR_STATUS 0x([0-9a-f]{8}): (on|off or idle)\s*$", re.M)
CRU_RX = re.compile(r"^t6_\w+: CRU at hand-over: CLKSEL_CON\(158\) 0x([0-9a-f]{8}) CLKSEL_CON\(159\) 0x([0-9a-f]{8}) "
                    r"CLKGATE_CON\(66\) 0x([0-9a-f]{8}) MODE_CON0 0x([0-9a-f]{8}) PLL_CON 0x([0-9a-f]{8}) "
                    r"0x([0-9a-f]{8}) 0x([0-9a-f]{8}): clk_gpu_src (\d+) Hz\s*$", re.M)
RK_RX = re.compile(r"^t6_\w+: RK806 at hand-over: POWER_EN0 0x([0-9a-f]+) POWER_SLP_EN0 0x([0-9a-f]+) "
                   r"BUCK1_CONFIG 0x([0-9a-f]+) BUCK1_ON_VSEL 0x([0-9a-f]+): vdd_gpu_s0 (on|off), (\d+) uV\s*$", re.M)
CLK_RX = re.compile(r"^t6_\w+: clock set: kept (\d+), CPLL (\d+) Hz, wrote CLKSEL_CON\(158\) 0x([0-9a-f]{8}) "
                    r"\(159\) 0x([0-9a-f]{8}), now 0x([0-9a-f]{8}) 0x([0-9a-f]{8}): clk_gpu_src (\d+) Hz\s*$", re.M)
EN_RX = re.compile(r"^t6_\w+: vdd_gpu_s0 enable: (frame sent|not sent); POWER_EN0 0x([0-9a-f]+) "
                   r"BUCK1_ON_VSEL 0x([0-9a-f]+) after, waited (\d+) us\s*$", re.M)
STOP_RX = re.compile(r"^t6_\w+: Z1 stopped: stop (\d+) \((.*)\), refusal (\d+), detail 0x([0-9a-f]{16})\s*$", re.M)
PDON_RX = re.compile(r"^t6_\w+: PD_GPU on: gates opened 0x([0-9a-f]+), was on (\d+), waited (\d+)/(\d+)/(\d+) us\s*$", re.M)
PLL_RX = re.compile(r"^t6_\w+: GPU PLL on: PLL_CON6 0x([0-9a-f]{8}) at hand-over, was on (\d+), waited (\d+) us, "
                    r"PLL_CON6 0x([0-9a-f]{8})\s*$", re.M)
WDT_RX = re.compile(r"^t6: WDT0 armed: CR 0x([0-9a-f]{8}) -> 0x([0-9a-f]{8}) \((was \w+)\), TORR 0x([0-9a-f]{8}) -> "
                    r"0x([0-9a-f]{8}), CCVR 0x([0-9a-f]{8}), PARAM_1 0x([0-9a-f]{8}), version 0x([0-9a-f]{8}), "
                    r"flags 0x([0-9a-f]+): period (\d+) ms \(TOP (\d+)\)", re.M)
ARM_RX = re.compile(r"^T6:A3 WDT0 arm rc (-?\d+) CR 0x([0-9a-f]{8}) period_ms (\d+)\s*$", re.M)

CLOCK_STOPS = {9: "refuse: NoTableRate, nothing written (the CPLL is not 1500 MHz; detail is its rate)",
               10: "refuse: the clock set did not read back or the rate is not the set rate"}


def clock_block(img):
    L = []
    cl, stop = CLK_RX.search(img), STOP_RX.search(img)
    if cl is None:
        L.append("  gpu_clock_set: %s (no 'clock set:' line: the image stopped before it or printed none)" % NR)
        if stop and int(stop.group(1)) in CLOCK_STOPS:
            L.append("  stop %s: %s" % (stop.group(1), CLOCK_STOPS[int(stop.group(1))]))
        return L
    kept, cpll = int(cl.group(1)), int(cl.group(2))
    w158, w159, n158, n159 = (int(cl.group(i), 16) for i in (3, 4, 5, 6))
    hz = int(cl.group(7))
    st = int(stop.group(1)) if stop else None
    if st in CLOCK_STOPS:
        took = CLOCK_STOPS[st]
        if st == 9:
            took += " [CPLL %d Hz]" % int(stop.group(4), 16)
    elif kept == 1:
        took = "keep: the clock was on an OPP rate and was left as found, nothing written"
    elif w158 != 0:
        took = "move: CLKSEL_CON(158) written%s, both words read back" % (" and (159)" if w159 else "")
    else:
        took = "not decided (kept 0 but no word written and no clock refusal in the record)"
    L.append("  gpu_clock_set took: %s" % took)
    L.append("  CPLL %d Hz; words written: CLKSEL_CON(158) 0x%08x, (159) 0x%08x; read back: 0x%08x 0x%08x; "
             "clk_gpu_src %d Hz" % (cpll, w158, w159, n158, n159, hz))
    pl = PLL_RX.search(img)
    if pl:
        L.append("  GPU PLL: PLL_CON6 0x%s at hand-over, was on %s, waited %s us, PLL_CON6 0x%s after" % (
            pl.group(1), pl.group(2), pl.group(3), pl.group(4)))
    return L


def supply_block(img):
    L = []
    rk, en = RK_RX.search(img), EN_RX.search(img)
    if rk is None:
        L.append("  RK806 at hand-over: %s" % NR)
    else:
        L.append("  RK806 at hand-over: POWER_EN0 0x%s POWER_SLP_EN0 0x%s BUCK1_CONFIG 0x%s BUCK1_ON_VSEL 0x%s: "
                 "vdd_gpu_s0 %s, %s uV" % (rk.group(1), rk.group(2), rk.group(3), rk.group(4), rk.group(5), rk.group(6)))
    if en is None:
        L.append("  vdd_gpu_s0 enable: %s" % NR)
    else:
        L.append("  vdd_gpu_s0 enable: %s; POWER_EN0 0x%s BUCK1_ON_VSEL 0x%s after, waited %s us" % (
            en.group(1), en.group(2), en.group(3), en.group(4)))
    return L


def pd_block(img):
    L = []
    got = {m.group(1): m for m in PD_RX.finditer(img)}
    for when in ("at hand-over", "after"):
        m = got.get(when)
        if m is None:
            L.append("  PD_GPU %s: %s" % (when, NR))
        else:
            L.append("  PD_GPU %s: PWR_GATE 0x%s STATUS 0x%s BUS_IDLE_REQ 0x%s ACK 0x%s ST 0x%s REPAIR_STATUS 0x%s: %s" % (
                (when,) + m.groups()[1:]))
        if when == "at hand-over":
            on = PDON_RX.search(img)
            L.append("  PD_GPU on step: %s" % (NR if on is None else "gates opened 0x%s, was on %s, waited %s/%s/%s us" % on.groups()))
    return L


def wdt_block(img):
    L = []
    a, w = ARM_RX.search(img), WDT_RX.search(img)
    L.append("  image's WDT0 arm (T6:A3): %s" % (NR if a is None else "rc %s, CR 0x%s, period %s ms" % (a.group(1), a.group(2), a.group(3))))
    if w is None:
        L.append("  image's WDT0 line: %s" % NR)
    else:
        L.append("  image's WDT0 line: CR 0x%s -> 0x%s (%s), TORR 0x%s -> 0x%s, CCVR 0x%s, PARAM_1 0x%s, version 0x%s, "
                 "flags 0x%s, period %s ms (TOP %s)" % w.groups())
    return L


def go_block(img, verdict):
    L = ["Hand-over as the image found and set it (go record)"]
    L.append("gpu_clock_set:")
    L += clock_block(img)
    L.append("GPU supply:")
    L += supply_block(img)
    L.append("PD_GPU:")
    L += pd_block(img)
    cru = CRU_RX.search(img)
    L.append("CRU at hand-over: %s" % (NR if cru is None else "CLKSEL_CON(158) 0x%s (159) 0x%s CLKGATE_CON(66) 0x%s "
                                       "MODE_CON0 0x%s PLL_CON 0x%s 0x%s 0x%s: clk_gpu_src %s Hz" % cru.groups()))
    L.append("WDT0 in the image:")
    L += wdt_block(img)
    st = STOP_RX.search(img)
    L.append("image stop: %s" % ("none printed" if st is None else "stop %s (%s), refusal %s, detail 0x%s" % st.groups()))
    last = [x for x in (verdict or "").splitlines() if x.strip()]
    L.append("verdict: %s" % (NR if not last else last[-1].strip()))
    return L


GIC_RX = re.compile(r"^t6: GIC active state: (.*)$", re.M)
FEED_RX = re.compile(r"^yakogl-wdt: nanopc_t6 first feed at (-?\d+) ms\s*$", re.M)
UART2_RX = re.compile(r"^# (UART2.*)$", re.M)


def uart2_block(reads_text):
    """The UART2 lines prompt-reads.py wrote as '# UART2 ...' notes."""
    got = UART2_RX.findall(reads_text)
    if not got:
        return ["  UART2 at the prompt: %s (no 'UART2' lines in answers.txt)" % NR]
    return ["  " + x for x in got]


def tail_block(reads_text, img):
    L = ["", "UART2 at the prompt (prompt reads)"]
    L += uart2_block(reads_text)
    L.append("GIC redistributor wake and active reset (image)")
    g = GIC_RX.search(img)
    if g is None:
        L.append("  GIC active state line: %s" % NR)
    else:
        L.append("  GIC active state: " + g.group(1).rstrip())
        if "no redistributor with affinity" in g.group(1):
            L.append("  no redistributor matched: the wake timed out or the affinity has no frame; "
                     "SGIs and PPIs were not scanned")
        else:
            L.append("  a redistributor matched and was woken or found awake")
    L.append("WDT0 feeder (image)")
    f = FEED_RX.search(img)
    if f is None:
        L.append("  first feed: absent (no 'yakogl-wdt: nanopc_t6 first feed' line: the feeder timer never ticked)")
    else:
        L.append("  first feed at %s ms" % f.group(1))
    return L


def summary(reads_arg, go_arg):
    rname, rf = locate(reads_arg, "answers.txt")
    gname, gf = locate(go_arg, "image.log", ("verdict.txt",))
    ans, notes = sections(rf["answers.txt"])
    # Only the md.l answers are scanned: printenv and bdinfo carry the board's
    # own MAC and IP, and are never read here.
    scan("\n".join(v for k, v in ans.items() if k.startswith("md.l ")), "the md.l answers")
    scan(gf["image.log"], "image.log")
    if gf["verdict.txt"]:
        scan(gf["verdict.txt"], "verdict.txt")
    out = ["NanoPC-T6 U-Boot hand-over, measured",
           "records: prompt reads %s; go %s" % (rname, gname),
           "",
           "Prompt reads (U-Boot's state before go)"]
    out += reads_block(ans, notes)
    out.append("")
    out += go_block(gf["image.log"], gf["verdict.txt"])
    out += tail_block(rf["answers.txt"], gf["image.log"])
    return "\n".join(out) + "\n"


def main(argv):
    if len(argv) != 3 or argv[1] in ("-h", "--help"):
        sys.stderr.write(__doc__)
        return 2
    try:
        sys.stdout.write(summary(argv[1], argv[2]))
    except Refused as e:
        sys.stderr.write("t6-uboot-handover-summary: %s\n" % e)
        return 3
    except Unreadable as e:
        sys.stderr.write("t6-uboot-handover-summary: %s\n" % e)
        return 4
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
