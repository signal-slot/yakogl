#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""pvr-fwtrace-pr.py - cross-check the firmware's SPM/MAIN partial-render trace
addresses against the Z/S addresses of a stall dump, from an AM62P serial log.

    python3 -I tools/cts/pvr-fwtrace-pr.py [--spm-scratch ADDR] [LOG]
    python3 -I tools/cts/pvr-fwtrace-pr.py --selftest

LOG is a serial log (default stdin). Exit status:

    0  the log has partial-render trace lines and every SPM 17-20 address
       equals a dumped Z/S address (or the scratch of --spm-scratch); trace
       lines with no stall dump are printed as an unchecked timeline and
       still exit 0.
    1  no partial-render trace lines in the log, or an address line equals
       none of the dumped Z/S addresses and not the scratch.
    2  the log or the arguments are unusable (unreadable LOG, a bad
       --spm-scratch value, too many arguments).

The SPM 17-20 pair formats the DDK's "0x%08x%08x" with the high dword then
the low dword (driver/src/pvr_fw_trace_decode.c:86-88, :169), so the two
dwords rebuild one 64-bit device address (hi << 32 | lo).

Every trace line pattern is taken from driver/src/pvr_fw_trace_decode.c's
format strings (its header pvr_fw_trace_decode.h:45 documents the shape
"T<t>-<GROUP>> [<ts>] <message>"):

    SPM 17-20, the Z/S store and load addresses (2 parameters):
      "T%u-SPM> [%llu] PR <ZStore|SStore|ZLoad|SLoad> address 0x%08x%08x"
        pvr_fw_trace_decode.c:167 (labels: ZStore :93, SStore :94,
        ZLoad :95, SLoad :96; the entry advances 5 dwords, :172). The IDs
        are the DDK's SPM 17-20 (rgx_fwif_sf.h:1533, :1535, :1537, :1539;
        cited as the range :1533-1540 by the decode file's own comment
        at :84-89). The printed "PR <label> address" is the decode file's
        replacement of the DDK string (:84-99).
    MAIN 47/48, a partial render without a ZS buffer (no parameters):
      "T%u-MAIN> [%llu] PR: no ZS buffer (store|load)"
        pvr_fw_trace_decode.c:178 (IDs matched at :175-176; the DDK's
        rgx_fwif_sf.h:208-213, cited at :177).
    SPM 1/2, no parameters, keeping the DDK's own strings through the
    generic format:
      "T%u-SPM> [%llu] <Force Z-Load|Force Z-Store> for partial render"
        pvr_fw_trace_decode.c:200 with the strings at
        third_party/ti-img-rogue-ddk-25.2/include/rgx_fwif_sf.h:1499-1502
        (cited by the decode file at :88-89).
    Section context line:
      "=== FW trace: log_type=0x%08x size=%u dw ==="
        pvr_fw_trace_decode.c:221.
    Group names "SPM" / "MAIN": the decode file's group table (:29-33,
    :68-73) names the DDK's RGXFW_LOG_SFGROUPLIST (rgx_fwif_sf.h:58 MAIN,
    :63 SPM).
    On the board the lines are prefixed "FWT| " (driver/src/pvr_selftest.c:111);
    every pattern is searched inside the line, so any serial prefix matches.

The stall dump's command word lines come from driver/rust/src/gpu/pvr/render.rs:
" c3d[NN]: <8 dwords>" printed after "render: last FRAG cmd3d (seq N), fence F:"
(render.rs:1930, :1937), "render: cmd3d[NN]: ..." at kick time in verbose mode
(render.rs:1546), and the not-yet-wired pr_diag.rs decodable dump
"render: stall: c3d[NN]: ..." (pr_diag.rs:166). The word-line shape and the
Z/S dword names are those of tools/cts/pvr-cmd3d-decode.py (:74-78): dword
26-27 isp_zload_store_base and 28-29 isp_stencil_load_store_base, written
64-bit lo/hi at render.rs:1498-1499. A packed D24S8 frame carries the same
address in both pairs (src/pvr/backend.rs), so one address may match both.

The SPM scratch address is not printed by any board line today (the backend
keeps it in the kick's frag_pr.spm_scratch_gpu, render.rs:173; the PR's PBE
state words carry it into the EOT PDS data, src/pvr/spm.rs, which the stall
dump does not print), so pass it with --spm-scratch ADDR (hex); an address
then also matches when it equals the scratch. With no stall dump and no
--spm-scratch the timeline is printed unchecked and the run still exits 0.
"""
import re
import sys

# SPM 17-20 (decode :167, labels :93-96): high dword, low dword.
ADDR_RE = re.compile(
    r"T(\d+)-SPM> \[(\d+)\] PR (ZStore|SStore|ZLoad|SLoad) address 0x([0-9a-fA-F]{8})([0-9a-fA-F]{8})")
# MAIN 47/48 (decode :178).
NOZS_RE = re.compile(r"T(\d+)-MAIN> \[(\d+)\] PR: no ZS buffer \((store|load)\)")
# SPM 1/2 (decode :200 with the DDK strings, rgx_fwif_sf.h:1499-1502).
FORCE_RE = re.compile(
    r"T(\d+)-SPM> \[(\d+)\] (Force Z-Load|Force Z-Store) for partial render")
# Section context (decode :221).
HDR_RE = re.compile(r"=== FW trace: log_type=0x([0-9a-fA-F]{8}) size=(\d+) dw ===")
# The stall dump's command word lines, the shape pvr-cmd3d-decode.py reads
# (render.rs:1937 c3d, :1546 cmd3d; geometry commands carry no Z/S words).
CMD_RE = re.compile(r"\b(c3d|cmd3d|cgeom)\[\s*(\d+)\]:((?: [0-9a-fA-F]{8}){8})\s*$")
ZLS_DW, SLS_DW = 26, 28  # isp_zload_store_base, isp_stencil_load_store_base


def parse(text):
    """(entries, records) for a log: decoded PR trace entries in log order,
    and the stall dump's command records (kind, words) rebuilt from the
    word lines (the assembly is pvr-cmd3d-decode.py's)."""
    entries, recs = [], []
    for line in text.splitlines():
        line = line.rstrip("\r\n")
        m = ADDR_RE.search(line)
        if m:
            hi, lo = int(m.group(4), 16), int(m.group(5), 16)
            entries.append(("addr", m.group(1), "SPM", int(m.group(2)),
                            m.group(3), (hi << 32) | lo))
            continue
        m = NOZS_RE.search(line)
        if m:
            entries.append(("noz", m.group(1), "MAIN", int(m.group(2)),
                            "no ZS buffer (%s)" % m.group(3), None))
            continue
        m = FORCE_RE.search(line)
        if m:
            entries.append(("force", m.group(1), "SPM", int(m.group(2)), m.group(3), None))
            continue
        m = HDR_RE.search(line)
        if m:
            entries.append(("hdr", "-", "-", None,
                            "FW trace log_type=0x%s size=%s dw" % (m.group(1), m.group(2)), None))
            continue
        m = CMD_RE.search(line)
        if m:
            kind, idx = m.group(1), int(m.group(2))
            if idx == 0 or not recs or recs[-1][0] != kind or len(recs[-1][1]) != idx:
                recs.append([kind, []])
            recs[-1][1].extend(int(x, 16) for x in m.group(3).split())
    return entries, recs


def zs_bases(recs):
    """[(record number, "Z"|"S", address)] for every dumped 3D command pair;
    geometry records (cgeom) have no Z/S words."""
    out = []
    for n, (kind, words) in enumerate(recs, 1):
        if kind == "cgeom":
            continue
        for dw, name in ((ZLS_DW, "Z"), (SLS_DW, "S")):
            if len(words) >= dw + 2:
                out.append((n, name, words[dw] | words[dw + 1] << 32))
    return out


def run(text, scratch):
    """(lines, failure reasons) for a log; scratch None or a u64."""
    entries, recs = parse(text)
    pr = [e for e in entries if e[0] in ("addr", "noz", "force")]
    bases = zs_bases(recs)
    lines, bad = [], []
    lines.append("pvr-fwtrace-pr: %d partial-render trace lines, %d stall dump records, scratch %s"
                 % (len(pr), len(recs), "none" if scratch is None else "0x%016x" % scratch))
    lines.append("timeline:")
    for kind, thread, group, ts, what, addr in entries:  # the log order
        if kind == "hdr":
            lines.append("  (%s)" % what)
            continue
        if kind == "addr":
            hits = ["%s base (dump %d)" % (name, n) for n, name, a in bases if a == addr]
            if scratch is not None and addr == scratch:
                hits.append("scratch")
            if hits:
                tag = "= " + ", ".join(hits)
            elif bases:
                tag = "no match"
                bad.append("address 0x%016x matches no dumped Z/S address or scratch" % addr)
            else:
                tag = "unchecked (no dumped Z/S words)"
            lines.append("  [%s] T%s-%s> %s address 0x%016x  [%s]" % (ts, thread, group, what, addr, tag))
        else:
            lines.append("  [%s] T%s-%s> %s" % (ts, thread, group, what))
    if bases:
        lines.append("dumped Z/S: " + "; ".join(
            "dump %d %s=0x%016x" % (n, name, a) for n, name, a in bases))
    if not pr:
        bad.append("no SPM/MAIN partial-render trace lines in the log")
    lines.append("pvr-fwtrace-pr: OK" if not bad else "pvr-fwtrace-pr: FAIL " + "; ".join(bad))
    return lines, bool(bad)


# Synthetic logs in the formats above, behind a serial log prefix. FWT| is the
# board prefix (pvr_selftest.c:111). Z=0x400e9e80 is a packed D24S8 buffer:
# the same address in the Z and S pairs (src/pvr/backend.rs).
SERIAL = "[00:00:12.345,000] <inf> pvr: "
ZBASE_HI, ZBASE_LO = "00000000", "400e9e80"
TRACE_HDR = SERIAL + "=== FW trace: log_type=0x00000000 size=12000 dw ===\n"
ADDR_LINE = SERIAL + "FWT| T0-SPM> [%s] PR %s address 0x%s%s\n"
NOZS_LINE = SERIAL + "FWT| T0-MAIN> [%s] PR: no ZS buffer (%s)\n"
FORCE_LINE = SERIAL + "FWT| T0-SPM> [%s] %s for partial render\n"


def c3d_lines(z_lo, s_lo):
    """The 80 words of a c3d dump (render.rs:1930,1937): dword n =
    0xa0000000|n, except the Z/S pairs at 26-29."""
    w = ["%08x" % (0xa0000000 | i) for i in range(80)]
    w[26], w[27], w[28], w[29] = z_lo, ZBASE_HI, s_lo, ZBASE_HI
    out = [SERIAL + "render: last FRAG cmd3d (seq 42), fence 41:\n"]
    for i in range(0, 80, 8):
        out.append(SERIAL + " c3d[%2d]: %s\n" % (i, " ".join(w[i:i + 8])))
    return "".join(out)


LOG_MATCH = (TRACE_HDR
             + FORCE_LINE % ("123450", "Force Z-Load")
             + ADDR_LINE % ("123451", "ZLoad", ZBASE_HI, ZBASE_LO)
             + ADDR_LINE % ("123452", "SLoad", ZBASE_HI, ZBASE_LO)
             + FORCE_LINE % ("123453", "Force Z-Store")
             + ADDR_LINE % ("123454", "ZStore", ZBASE_HI, ZBASE_LO)
             + ADDR_LINE % ("123455", "SStore", ZBASE_HI, ZBASE_LO)
             + c3d_lines(ZBASE_LO, ZBASE_LO))
LOG_MISMATCH = (TRACE_HDR
                + ADDR_LINE % ("123451", "ZLoad", ZBASE_HI, ZBASE_LO)
                + ADDR_LINE % ("123452", "ZStore", ZBASE_HI, "40123456")  # not the command's base
                + ADDR_LINE % ("123453", "SStore", ZBASE_HI, ZBASE_LO)
                + c3d_lines(ZBASE_LO, ZBASE_LO))
LOG_NOTRACE = (TRACE_HDR
               + SERIAL + "FWT| T0-SPM> [123456] 3D MemFree: Local FL 0x00000000\n"  # not a PR line
               + c3d_lines(ZBASE_LO, ZBASE_LO))
LOG_NOZS = TRACE_HDR + NOZS_LINE % ("123451", "store") + NOZS_LINE % ("123452", "load")
LOG_NO_DUMP = (TRACE_HDR + FORCE_LINE % ("123450", "Force Z-Load")
               + ADDR_LINE % ("123451", "ZLoad", ZBASE_HI, ZBASE_LO)
               + ADDR_LINE % ("123452", "ZStore", ZBASE_HI, ZBASE_LO))


def selftest():
    scratch = 0x40123456
    cases = [
        ("match, expect OK", LOG_MATCH, None, 0, "= Z base (dump 1), S base (dump 1)",
         "pvr-fwtrace-pr: OK"),
        ("mismatch fails", LOG_MISMATCH, None, 1, "matches no dumped Z/S address or scratch",
         "pvr-fwtrace-pr: FAIL"),
        ("the mismatched address is the scratch", LOG_MISMATCH, scratch, 0, "= scratch",
         "pvr-fwtrace-pr: OK"),
        ("no PR trace lines fails", LOG_NOTRACE, None, 1,
         "no SPM/MAIN partial-render trace lines in the log", "pvr-fwtrace-pr: FAIL"),
        ("MAIN 47/48 only: lines found, no addresses", LOG_NOZS, None, 0,
         "no ZS buffer (store)", "pvr-fwtrace-pr: OK"),
        ("addresses with no stall dump: unchecked", LOG_NO_DUMP, None, 0,
         "unchecked (no dumped Z/S words)", "pvr-fwtrace-pr: OK"),
        ("timestamp order is the log order", LOG_MATCH, None, 0,
         "[123451] T0-SPM> ZLoad address 0x00000000400e9e80", "[123455] T0-SPM> SStore"),
    ]
    failed = 0
    for name, text, sc, want_rc, *wants in cases:
        lines, bad = run(text, sc)
        out = "\n".join(lines)
        rc = 1 if bad else 0
        if rc == want_rc and all(w in out for w in wants):
            print("  ok    " + name)
        else:
            failed += 1
            print("  FAIL  %s: rc %d (want %d), want %r in:\n%s" % (name, rc, want_rc, wants, out))
    print("pvr-fwtrace-pr selftest: %s" % ("FAILED %d" % failed if failed else "all passed"))
    return 1 if failed else 0


def main(argv):
    args = argv[1:]
    if args == ["--selftest"]:
        return selftest()
    scratch = None
    if args and args[0] == "--spm-scratch":
        if len(args) < 2:
            print("pvr-fwtrace-pr: --spm-scratch needs an address", file=sys.stderr)
            return 2
        try:
            scratch = int(args[1], 0)
        except ValueError:
            print("pvr-fwtrace-pr: --spm-scratch %r is not an address" % args[1], file=sys.stderr)
            return 2
        args = args[2:]
    if len(args) > 1:
        print(__doc__.split("\n\n")[1], file=sys.stderr)
        return 2
    if args:
        try:
            with open(args[0], encoding="utf-8", errors="replace") as fh:
                text = fh.read()
        except OSError as e:
            print("pvr-fwtrace-pr: %s" % e, file=sys.stderr)
            return 2
    else:
        text = sys.stdin.read()
    lines, failed = run(text, scratch)
    for ln in lines:
        print(ln)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
