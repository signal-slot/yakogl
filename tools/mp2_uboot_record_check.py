#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Checks a STM32MP257F-DK U-Boot run record for the evidence of plan #357's c5.

    python3 -I tools/mp2_uboot_record_check.py RECORD_DIR [--verdict-line FILE]
    python3 -I tools/mp2_uboot_record_check.py --self-test

RECORD_DIR is what tools/board-stm32mp2-uboot.sh run wrote
(records/<board>/<YYYYMMDD-HHMM>-<name>-<sha8>/): meta.txt, console.raw,
console.ts, typed.log, env-before.txt, env-after.txt, boot-sha256.txt,
boot-sha256.after.txt, env.diff, boot.diff.  One line per check, then
"record-check: N ok, M failed" (", K skipped" when a check had no evidence
it may skip).  Exit 0 only when M is 0.

  1. verdict line: verdict=done, "row" (never DEV), boot=uboot-mmc|uboot-sd|uboot-tftp
  2. console scan: phases stopped, script, go, image reached; back in Linux
  3. fw_printenv before == after (the diff is printed if not)
  4. /boot stock files identical; no entry beyond yakogl/; nothing left staged
  5. u-boot-env dump sha256 before == after (skipped when neither was taken)
  6. redaction: no IP address, tty/serial path, USB serial or *.ts.net name

The runner does not store its verdict line: it is taken from --verdict-line,
from a line of a record file that has the shape, or derived from meta.txt by
the runner's own rules (the check says which).
"""
import difflib
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tempfile

BOOTS = ("uboot-mmc", "uboot-sd", "uboot-tftp")
NEED_PHASES = ("stopped", "script", "go", "image")
VERDICT_RX = re.compile(r"^\S+ \S+ \S+ rc=-?\d+ verdict=\S+ .*$")


def read(path):
    with open(path, "rb") as f:
        return f.read().decode("utf-8", "replace")


def meta_of(rec):
    m = {}
    p = os.path.join(rec, "meta.txt")
    if os.path.isfile(p):
        for ln in read(p).splitlines():
            if "=" in ln:
                k, v = ln.split("=", 1)
                m[k] = v   # the last one wins, as the runner's mget
    return m


def sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def kv(line):
    return dict(t.split("=", 1) for t in line.split() if "=" in t)


# ---- 1 -----------------------------------------------------------------------
def derive_verdict(m):
    """The runner's verdict_line rules over meta.txt."""
    bv = m.get("scan", "absent")
    ed, bd = m.get("env_diff", "absent"), m.get("boot_diff", "absent")
    if m.get("rebooted") == "0":
        v = "not_rebooted"
    elif m.get("back") != "1":
        v = "not_back"
    elif ed != "empty":
        v = "env_changed"
    elif bd != "empty":
        v = "boot_changed"
    elif m.get("cleanup") != "removed":
        v = "not_cleaned"
    elif bv == "done":
        v = "done"
    else:
        v = bv
    return "%s verdict=%s boot=%s %s" % (m.get("name", "?"), v, m.get("boot", "?"),
                                         "row" if m.get("kind") == "row" else m.get("kind", "?"))


def find_verdict(rec, explicit):
    if explicit:
        for ln in read(explicit).splitlines():
            if VERDICT_RX.match(ln):
                return ln, "from " + os.path.basename(explicit)
        return None, "no verdict line in %s" % explicit
    for fn in sorted(os.listdir(rec)):
        p = os.path.join(rec, fn)
        if fn.startswith("console") or not os.path.isfile(p):
            continue
        for ln in read(p).splitlines():
            if VERDICT_RX.match(ln) and " verdict=" in ln:
                return ln, "from " + fn
    return None, ""


def check_verdict(rec, m, explicit=None):
    line, src = find_verdict(rec, explicit)
    if line is None and src:
        return False, src
    if line is None:
        if not m:
            return False, "missing evidence: no meta.txt and no verdict line"
        line, src = derive_verdict(m), "derived from meta.txt (the runner stores no verdict line)"
    toks = line.split()
    d = kv(line)
    bad = []
    if d.get("verdict") != "done":
        bad.append("verdict=%s" % d.get("verdict"))
    if "DEV" in toks or "dev" in toks or m.get("kind", "row") != "row":
        bad.append("a DEV run")
    elif "row" not in toks:
        bad.append("not marked row")
    if d.get("boot") not in BOOTS:
        bad.append("boot=%s" % d.get("boot"))
    if bad:
        return False, "%s: %s (%s)" % ("; ".join(bad), line, src)
    return True, "verdict=done row boot=%s (%s)" % (d["boot"], src)


# ---- 2 -----------------------------------------------------------------------
def scan_line_of(rec, m):
    if m.get("scan_line", "").startswith("mp2-scan:"):
        return m["scan_line"], "meta.txt scan_line"
    ts = os.path.join(rec, "console.ts")
    tool = os.path.join(os.path.dirname(os.path.abspath(__file__)), "mp2_uboot_console.py")
    if os.path.isfile(ts) and os.path.getsize(ts) and os.path.isfile(tool):
        out = subprocess.run([sys.executable, "-I", tool, "scan", ts], capture_output=True, text=True).stdout
        if out.startswith("mp2-scan:"):
            return out.splitlines()[0], "rescan of console.ts"
    return None, ""


def check_scan(rec, m):
    line, src = scan_line_of(rec, m)
    if line is None:
        return False, "missing evidence: no scan_line in meta.txt and no console.ts to scan"
    d = kv(line)
    miss = [p for p in NEED_PHASES if d.get(p, "absent") == "absent"]
    bad = []
    if miss:
        bad.append("phases absent: " + ",".join(miss))
    if d.get("back") != "yes":
        bad.append("scan says the board was not back (back=%s)" % d.get("back"))
    if m.get("back") != "1":
        bad.append("meta back=%s (Linux did not answer ssh from a new boot)" % m.get("back", "absent"))
    if bad:
        return False, "; ".join(bad) + " (%s)" % src
    return True, "phases stopped script go image reached, back in Linux (%s)" % src


# ---- 3 -----------------------------------------------------------------------
def check_env(rec, m):
    a, b = os.path.join(rec, "env-before.txt"), os.path.join(rec, "env-after.txt")
    if not (os.path.isfile(a) and os.path.isfile(b)):
        return False, "missing evidence: env-before.txt / env-after.txt"
    ta, tb = read(a), read(b)
    if not ta.strip():
        return False, "missing evidence: env-before.txt is empty"
    if ta == tb:
        return True, "fw_printenv before == after (%d lines)" % len(ta.splitlines())
    diff = "".join(difflib.unified_diff(ta.splitlines(True), tb.splitlines(True), "env-before.txt", "env-after.txt"))
    return False, "fw_printenv differs:\n" + diff.rstrip("\n")


# ---- 4 -----------------------------------------------------------------------
def listing(path):
    out = {}
    for ln in read(path).splitlines():
        parts = ln.strip().split(None, 1)
        if len(parts) == 2:
            out[parts[1].lstrip("*")] = parts[0]
    return out


def under_yakogl(p):
    q = p[2:] if p.startswith("./") else p.lstrip("/")
    return q.startswith("yakogl/")


def check_boot(rec, m):
    a, b = os.path.join(rec, "boot-sha256.txt"), os.path.join(rec, "boot-sha256.after.txt")
    if not (os.path.isfile(a) and os.path.isfile(b)):
        return False, "missing evidence: boot-sha256.txt / boot-sha256.after.txt"
    la, lb = listing(a), listing(b)
    if not la:
        return False, "missing evidence: boot-sha256.txt is empty"
    stock = {p: h for p, h in la.items() if not under_yakogl(p)}
    bad = []
    for p, h in sorted(stock.items()):
        if p not in lb:
            bad.append("stock file gone: %s" % p)
        elif lb[p] != h:
            bad.append("stock file changed: %s" % p)
    for p in sorted(lb):
        if p in la:
            continue
        bad.append("left after cleanup: %s" % p if under_yakogl(p) else "extra entry outside yakogl/: %s" % p)
    for p in sorted(la):
        if under_yakogl(p) and p in lb:
            bad.append("left after cleanup: %s" % p)
    if m.get("cleanup") != "removed":
        bad.append("meta cleanup=%s (the runner did not read the staged files back gone)" % m.get("cleanup", "absent"))
    if bad:
        return False, "; ".join(bad)
    return True, "%d stock files identical, nothing beyond yakogl/, staged files removed" % len(stock)


# ---- 5 -----------------------------------------------------------------------
def env_dumps(rec, m):
    """('before'|'after') -> sha256 of a u-boot-env dump file, or meta
    uboot_env_sha256_before/_after. The runner takes none yet."""
    got = {}
    for fn in sorted(os.listdir(rec)):
        low = fn.lower()
        if "u-boot-env" in low or "uboot-env" in low or "uboot_env" in low:
            for side in ("before", "after"):
                if side in low and os.path.isfile(os.path.join(rec, fn)):
                    got[side] = sha(os.path.join(rec, fn))
    for side in ("before", "after"):
        v = m.get("uboot_env_sha256_" + side)
        if v:
            got.setdefault(side, v.strip().lower())
    return got


def check_uenv(rec, m):
    g = env_dumps(rec, m)
    if not g:
        return None, "no u-boot-env dump in the record (not taken)"
    if len(g) == 1:
        return False, "only the %s u-boot-env dump is in the record" % next(iter(g))
    if g["before"] != g["after"]:
        return False, "u-boot-env sha256 differs: before %s after %s" % (g["before"], g["after"])
    return True, "u-boot-env sha256 before == after (%s)" % g["before"][:16]


# ---- 6 -----------------------------------------------------------------------
H4 = r"[0-9A-Fa-f]{1,4}"
REDACT = [
    ("IPv4", re.compile(r"(?<![0-9.])(?:(?:25[0-5]|2[0-4][0-9]|1?[0-9]{1,2})\.){3}(?:25[0-5]|2[0-4][0-9]|1?[0-9]{1,2})(?![0-9]|\.[0-9])")),
    ("IPv6", re.compile(r"(?<![0-9A-Za-z:])(?:(?:%s:){7}%s|(?:%s(?::%s){0,6})?::(?:%s(?::%s){0,6})?)(?![0-9A-Za-z:])" % ((H4,) * 6))),
    ("serial device path", re.compile(r"/dev/(?:tty|cu\.|serial)[A-Za-z0-9_./-]*")),
    ("USB serial", re.compile(r"\busb-[A-Za-z0-9]+_[A-Za-z0-9_.-]*-if[0-9]{2}|(?i:\b(?:iserial|serialnumber|serial number|id_serial(?:_short)?|serial#))\s*[:=]\s*(?!<redacted>)[A-Za-z0-9]{4,}")),
    ("tailnet host", re.compile(r"[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.ts\.net\b", re.I)),
]
TEST_NET_1 = re.compile(r"^192\.0\.2\.[0-9]{1,3}$")   # the runner's placeholders


def check_redaction(rec, m):
    hits = []
    nfiles = 0
    for root, _, files in os.walk(rec):
        for fn in sorted(files):
            p = os.path.join(root, fn)
            nfiles += 1
            for n, ln in enumerate(read(p).replace("\0", "").splitlines(), 1):
                for kind, rx in REDACT:
                    for mt in rx.finditer(ln):
                        if kind == "IPv4" and TEST_NET_1.match(mt.group(0)):
                            continue
                        if kind == "IPv6" and "::" == mt.group(0):
                            continue
                        hits.append("%s:%d: %s" % (os.path.relpath(p, rec), n, kind))
    if hits:
        return False, "%d unredacted (value not printed): %s" % (len(hits), "; ".join(hits[:8]))
    return True, "%d files scanned: no address, serial path, USB serial or tailnet name" % nfiles


CHECKS = [
    ("verdict", check_verdict),
    ("console scan", check_scan),
    ("fw_printenv", check_env),
    ("/boot sha256", check_boot),
    ("u-boot-env", check_uenv),
    ("redaction", check_redaction),
]


def run_checks(rec, explicit=None, out=print):
    if not os.path.isdir(rec):
        out("record-check: %s is not a directory" % rec)
        return 0, 1, 0
    m = meta_of(rec)
    ok = bad = skip = 0
    for name, fn in CHECKS:
        try:
            r, msg = fn(rec, m, explicit) if fn is check_verdict else fn(rec, m)
        except Exception as e:   # a malformed record is a failure, not a crash
            r, msg = False, "missing evidence: %s" % e
        if r is None:
            skip += 1
            out("skip   %s: %s" % (name, msg))
        elif r:
            ok += 1
            out("ok     %s: %s" % (name, msg))
        else:
            bad += 1
            out("FAILED %s: %s" % (name, msg))
    out("record-check: %d ok, %d failed%s" % (ok, bad, ", %d skipped" % skip if skip else ""))
    return ok, bad, skip


# ---- self-test ------------------------------------------------------------------
SCAN_OK = ("mp2-scan: verdict=done back=yes lines=240 cycles=2 bl2=yes uboot=+4.1s stopped=+5.0s "
           "script=+5.2s go=+5.9s image=+6.0s image_end=+7.1s watchdog=absent linux=+12.0s")
ENV = "bootcmd=run distro_bootcmd\nbootdelay=1\nscriptaddr=0x84000000\nethaddr=<redacted>\n"
SHA = ("%s  ./extlinux/extlinux.conf\n%s  ./stm32mp257f-dk.dtb\n%s  ./Image\n"
       % ("a" * 64, "b" * 64, "c" * 64))


def make_record(root, name="good", **over):
    d = os.path.join(root, name)
    os.makedirs(d)
    meta = {"board": "stm32mp257f-dk", "boot": "uboot-mmc", "name": "mp2_hello", "kind": "row",
            "scan": "done", "scan_line": SCAN_OK, "back": "1", "cleanup": "removed",
            "env_diff": "empty", "boot_diff": "empty"}
    files = {"console.ts": "1.0 U-Boot 2023.10\n2.0 mp2_hello: PASS: ok from 192.0.2.1\n",
             "typed.log": "typed 1 line\n", "env-before.txt": ENV, "env-after.txt": ENV,
             "boot-sha256.txt": SHA, "boot-sha256.after.txt": SHA, "env.diff": "", "boot.diff": ""}
    for k, v in over.items():
        if k in meta or k.startswith("m_"):
            k = k[2:] if k.startswith("m_") else k
            if v is None:
                meta.pop(k, None)
            else:
                meta[k] = v
        elif v is None:
            files.pop(k, None)
        else:
            files[k] = v
    with open(os.path.join(d, "meta.txt"), "w") as f:
        f.write("".join("%s=%s\n" % kv_ for kv_ in meta.items()))
    for fn, txt in files.items():
        with open(os.path.join(d, fn), "w") as f:
            f.write(txt)
    return d


def self_test():
    base = os.environ.get("TMPDIR", "")
    if not base or os.path.realpath(base).startswith("/tmp") or not os.path.isdir(base):
        print("record-check self-test: set TMPDIR to a directory outside /tmp", file=sys.stderr)
        return 2
    root = tempfile.mkdtemp(prefix="mp2-record-check.", dir=base)
    fails = []
    n = [0]

    def expect(label, rec, want_fail, substr=None, explicit=None):
        lines = []
        _, bad, _ = run_checks(rec, explicit, out=lines.append)
        text = "\n".join(lines)
        good = ((bad == 0) if not want_fail else (bad > 0 and (substr is None or any(
            l.startswith("FAILED") and substr in l for l in text.splitlines() + text.split("\nFAILED")))))
        if want_fail and substr:
            good = good and substr in text
        n[0] += 1
        print("%s %s" % ("ok  " if good else "FAIL", label))
        if not good:
            fails.append(label)
            print(text)

    def only(label, rec, check, substr, also=()):
        # exactly the named check fails, and says substr
        lines = []
        run_checks(rec, None, out=lines.append)
        f = [l for l in lines if l.startswith("FAILED")]
        mine = [l for l in f if l.startswith("FAILED " + check + ":")]
        rest = [l for l in f if l not in mine and not any(l.startswith("FAILED " + a + ":") for a in also)]
        good = len(mine) == 1 and substr in mine[0] and not rest
        n[0] += 1
        print("%s %s" % ("ok  " if good else "FAIL", label))
        if not good:
            fails.append(label)
            print("\n".join(lines))

    try:
        g = make_record(root, "good")
        expect("good record passes (u-boot-env skipped, counted)", g, False)
        lines = []
        run_checks(g, None, out=lines.append)
        n[0] += 1
        t = lines[-1] == "record-check: 5 ok, 0 failed, 1 skipped" and any(l.startswith("skip") for l in lines)
        print("%s summary line and the skip" % ("ok  " if t else "FAIL"))
        if not t:
            fails.append("summary")

        # 1 verdict
        only("verdict: scan not done", make_record(root, "v1", scan="hang", scan_line=SCAN_OK), "verdict", "verdict=hang")
        only("verdict: DEV run", make_record(root, "v2", kind="dev"), "verdict", "DEV")
        vf = os.path.join(root, "vl.txt")
        with open(vf, "w") as f:
            f.write("board-stm32mp2-uboot.sh stm32mp257f-dk mp2_hello rc=0 verdict=done boot=uboot-mmc DEV boot_verdict=done\n")
        lines = []
        run_checks(g, vf, out=lines.append)
        n[0] += 1
        t = any(l.startswith("FAILED verdict") and "DEV" in l for l in lines)
        print("%s verdict: DEV in an explicit verdict line" % ("ok  " if t else "FAIL"))
        if not t:
            fails.append("explicit DEV")
        with open(vf, "w") as f:
            f.write("board-stm32mp2-uboot.sh stm32mp257f-dk mp2_hello rc=0 verdict=done boot=uboot-tftp row boot_verdict=done\n")
        lines = []
        run_checks(g, vf, out=lines.append)
        n[0] += 1
        t = any(l.startswith("ok     verdict") and "uboot-tftp" in l for l in lines)
        print("%s verdict: explicit good line, boot=uboot-tftp" % ("ok  " if t else "FAIL"))
        if not t:
            fails.append("explicit good")
        only("verdict: unknown boot", make_record(root, "v3", boot="jtag"), "verdict", "boot=jtag")
        only("verdict: env changed in the stored verdict line",
             make_record(root, "v4", **{"verdict.txt": "t s n rc=6 verdict=env_changed boot=uboot-mmc row\n"}), "verdict", "env_changed")

        # 2 scan
        only("scan: image phase absent",
             make_record(root, "s1", scan_line=SCAN_OK.replace("image=+6.0s", "image=absent")), "console scan", "image")
        only("scan: board not back", make_record(root, "s2", back="0", cleanup="removed", scan_line=SCAN_OK.replace("back=yes", "back=no")),
             "console scan", "not", also=("verdict",))
        r = make_record(root, "s3", scan_line=None, **{"console.ts": None})
        lines = []
        run_checks(r, None, out=lines.append)
        n[0] += 1
        t = any(l.startswith("FAILED console scan") and "missing evidence" in l for l in lines)
        print("%s scan: no scan evidence is missing evidence" % ("ok  " if t else "FAIL"))
        if not t:
            fails.append("scan missing")

        # 3 env
        e = make_record(root, "e1", **{"env-after.txt": ENV.replace("bootdelay=1", "bootdelay=0") + "mp2z_file=left\n"})
        lines = []
        run_checks(e, None, out=lines.append)
        n[0] += 1
        t = any(l.startswith("FAILED fw_printenv") for l in lines) and "+mp2z_file=left" in "\n".join(lines) \
            and "-bootdelay=1" in "\n".join(lines)
        print("%s fw_printenv: nonempty diff fails and is printed" % ("ok  " if t else "FAIL"))
        if not t:
            fails.append("env diff")
        only("fw_printenv: after missing", make_record(root, "e2", **{"env-after.txt": None}), "fw_printenv", "missing evidence")

        # 4 boot
        only("boot: changed stock file",
             make_record(root, "b1", **{"boot-sha256.after.txt": SHA.replace("b" * 64, "d" * 64)}), "/boot sha256", "stock file changed: ./stm32mp257f-dk.dtb")
        only("boot: extra entry outside yakogl/",
             make_record(root, "b2", **{"boot-sha256.after.txt": SHA + "e" * 64 + "  ./extra.bin\n"}), "/boot sha256", "outside yakogl/")
        only("boot: staged file left under yakogl/",
             make_record(root, "b3", **{"boot-sha256.after.txt": SHA + "e" * 64 + "  ./yakogl/mp2z-x.bin\n"}), "/boot sha256", "left after cleanup")
        only("boot: stock file gone", make_record(root, "b4", **{"boot-sha256.after.txt": SHA.split("\n", 1)[1]}), "/boot sha256", "gone")
        expect("boot: yakogl/ entries in both listings are not stock",
               make_record(root, "b5", **{"boot-sha256.txt": SHA + "e" * 64 + "  ./yakogl/old.bin\n",
                                          "boot-sha256.after.txt": SHA + "e" * 64 + "  ./yakogl/old.bin\n"}), True, "left after cleanup")
        only("boot: cleanup not removed", make_record(root, "b6", cleanup="NOT removed"), "/boot sha256", "cleanup", also=("verdict",))

        # 5 u-boot-env
        ue = make_record(root, "u1")
        for side, data in (("before", b"\x01\x02"), ("after", b"\x01\x02")):
            with open(os.path.join(ue, "u-boot-env-%s.bin" % side), "wb") as f:
                f.write(data)
        lines = []
        run_checks(ue, None, out=lines.append)
        n[0] += 1
        t = lines[-1] == "record-check: 6 ok, 0 failed"
        print("%s u-boot-env: equal dumps pass" % ("ok  " if t else "FAIL"))
        if not t:
            fails.append("uenv equal")
        ud = make_record(root, "u2")
        for side, data in (("before", b"\x01\x02"), ("after", b"\x01\x03")):
            with open(os.path.join(ud, "u-boot-env-%s.bin" % side), "wb") as f:
                f.write(data)
        only("u-boot-env: dumps differ", ud, "u-boot-env", "differs")
        uo = make_record(root, "u3")
        with open(os.path.join(uo, "u-boot-env-before.bin"), "wb") as f:
            f.write(b"x")
        only("u-boot-env: only one dump", uo, "u-boot-env", "only the before")

        # 6 redaction
        for label, text, kind in (
                ("IPv4", "tftp from 10.1.2.3 ok\n", "IPv4"),
                ("IPv4 near a placeholder", "from 192.0.2.1 then 203.0.113.7\n", "IPv4"),
                ("IPv6", "link fe80::1ff:fe23:4567:890a up\n", "IPv6"),
                ("IPv6 full", "a 2001:0db8:85a3:0000:0000:8a2e:0370:7334 b\n", "IPv6"),
                ("tty path", "console on /dev/ttyUSB3\n", "serial device path"),
                ("serial path", "/dev/serial/by-id/x\n", "serial device path"),
                ("USB serial", "usb-FTDI_FT232R_USB_UART_A50285BI-if00-port0\n", "USB serial"),
                ("USB serial number", "iSerial: 0123456789AB\n", "USB serial"),
                ("tailnet name", "ssh dk1.tail1234.ts.net\n", "tailnet host")):
            r = make_record(root, "r-" + label.replace(" ", "_"), **{"typed.log": text})
            only("redaction: " + label, r, "redaction", kind)
        r = make_record(root, "r-nul", **{"console.raw": "ok\0 10.9.8.7\n"})
        only("redaction: address in console.raw behind a NUL", r, "redaction", "console.raw:1")
        expect("redaction: version, MAC, time, <redacted> and TEST-NET placeholders pass",
               make_record(root, "r-ok", **{"typed.log": "U-Boot 2023.10.0 ethaddr=<redacted> serial#=<redacted> "
                                            "12:34:56 aa:bb:cc:dd:ee:ff 192.0.2.7 Linux 6.1.82 foo::bar console [ttySTM0]\n"}), False)
        r = make_record(root, "r-leak", **{"typed.log": "ip 10.1.2.3\n"})
        lines = []
        run_checks(r, None, out=lines.append)
        n[0] += 1
        t = "10.1.2.3" not in "\n".join(lines)
        print("%s redaction: the leaked value is not printed" % ("ok  " if t else "FAIL"))
        if not t:
            fails.append("echo")

        # exit codes through the CLI
        for rec, want in ((g, 0), (e, 1)):
            rc = subprocess.run([sys.executable, "-I", os.path.abspath(__file__), rec], capture_output=True).returncode
            n[0] += 1
            t = (rc == 0) == (want == 0)
            print("%s cli exit %d for %s" % ("ok  " if t else "FAIL", rc, os.path.basename(rec)))
            if not t:
                fails.append("cli")
        n[0] += 1
        t = subprocess.run([sys.executable, "-I", os.path.abspath(__file__), os.path.join(root, "nothing")],
                           capture_output=True).returncode != 0
        print("%s cli: a missing directory fails" % ("ok  " if t else "FAIL"))
        if not t:
            fails.append("cli missing")
    finally:
        shutil.rmtree(root, ignore_errors=True)
    print("record-check self-test: %d checks, %d failed" % (n[0], len(fails)))
    return 1 if fails else 0


def main(argv):
    args = argv[1:]
    if args == ["--self-test"]:
        return self_test()
    explicit = None
    if "--verdict-line" in args:
        i = args.index("--verdict-line")
        if i + 1 >= len(args):
            print(__doc__.split("\n\n")[1], file=sys.stderr)
            return 2
        explicit = args[i + 1]
        del args[i:i + 2]
    if len(args) != 1 or args[0].startswith("-"):
        print(__doc__.split("\n\n")[1], file=sys.stderr)
        return 2
    _, bad, _ = run_checks(args[0], explicit)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
