#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Console side of run.sh (README.md in this directory).

    drive.py --qemu QEMU --uboot U-BOOT.BIN --disk DISK --env ENV --tftp DIR
             --mem SIZE --logs DIR [--big-tftp]

Boots QEMU's virt machine with U-Boot once per case, stops autoboot, reads
"printenv", types the case's line (the DK console runner's typed line, the
same text), reads "printenv" again and powers off. Before the cases one boot
stores the environment (the only saveenv of the test); after them one boot
reads it back. Prints one line per check and the summary
"run.sh: N ok, M failed" last; exit 0 only when nothing failed.
"""
import argparse
import hashlib
import os
import re
import select
import subprocess
import sys
import time

PROMPT = re.compile(rb"\n=> ")
STOP = re.compile(rb"Hit any key to stop autoboot")
LOAD = "0xa0000000"
WINDOW = "0x10000000"
UNDO = "mp2z: undo: nothing persistent; run bootcmd boots ST Linux"
STOCK_BOOTCMD = "echo mp2q: stock bootcmd ran"   # build-uboot.sh's BOOTCOMMAND
BACK = ["mp2z: back to bootcmd", "mp2q: stock bootcmd ran"]
RAN = [
    "mp2z: go " + LOAD,
    "## Starting application at 0xA0000000 ...",
    "mp2q: image ran at its link address 0xa0000000",
    "## Application terminated, rc = 0x0",
    "mp2z: the image returned",
]
# U-Boot's own variables that a load, dhcp or tftp sets in RAM; the scripts'
# mp2z_* are RAM-only by contract. Nothing else may differ after a case.
SIDE_EFFECT = {"filesize", "fileaddr"}
NET_SIDE_EFFECT = {"ethact", "ipaddr", "netmask", "gatewayip", "serverip", "dnsip",
                   "dnsip2", "domain", "hostname", "bootfile", "ntpserverip"}

MMC = ("load mmc 0:2 ${scriptaddr} /yakogl/mp2-mmc.scr && source ${scriptaddr}; "
       "echo mp2z: back to bootcmd; run bootcmd")
NET = ("setenv autoload no; dhcp; tftpboot ${scriptaddr} 10.0.2.2:mp2-net.scr && "
       "source ${scriptaddr}; echo mp2z: back to bootcmd; run bootcmd")


def mmc_case(name, prefix, expect):
    return dict(name=name, route="mmc", line=prefix + MMC, expect=expect)


def net_case(name, prefix, expect):
    return dict(name=name, route="net", line=prefix + NET, expect=expect)


def cases(big_tftp):
    mmc_head = [UNDO]
    p = "setenv mp2z_dev 0; setenv mp2z_part 2; "

    def mmc_at(f):
        return mmc_head + ["mp2z: mmc 0:2 %s -> %s" % (f, LOAD)]

    out = [
        mmc_case("mmc-ok", p, mmc_at("/yakogl/mp2z.bin") + [
            "mp2z: loaded 1000 bytes at %s, RAM footprint 0x2000 of the %s-byte window" % (LOAD, WINDOW),
        ] + RAN + BACK),
        mmc_case("mmc-missing", p + "setenv mp2z_file /yakogl/missing.bin; ",
                 mmc_at("/yakogl/missing.bin") + [
                     "mp2z: refused: /yakogl/missing.bin not found on mmc 0:2"] + BACK),
        mmc_case("mmc-file-above-window", p + "setenv mp2z_file /yakogl/big.bin; ",
                 mmc_at("/yakogl/big.bin") + [
                     "mp2z: refused: file 10000001 bytes, above the %s-byte window" % WINDOW] + BACK),
        mmc_case("mmc-image-size-above-window", p + "setenv mp2z_file /yakogl/bigimg.bin; ",
                 mmc_at("/yakogl/bigimg.bin") + [
                     "mp2z: refused: the image takes 0x10000001 bytes of RAM (header image_size), "
                     "above the %s-byte window" % WINDOW] + BACK),
        mmc_case("mmc-no-magic", p + "setenv mp2z_file /yakogl/nomagic.bin; ",
                 mmc_at("/yakogl/nomagic.bin") + [
                     "mp2z: refused: no ARM64 Image magic at byte 56 of the loaded file"] + BACK),
        mmc_case("mmc-no-part", "setenv mp2z_dev 0; ", mmc_head + [
            "mp2z: refused: mp2z_part (the bootfs partition number) is not set"] + BACK),
    ]
    q = "setenv mp2z_server 10.0.2.2; "

    def net_at(f):
        return [UNDO, "mp2z: tftp 10.0.2.2:%s -> %s" % (f, LOAD)]

    out += [
        net_case("net-ok", q, net_at("mp2z.bin") + [
            "mp2z: loaded 1000 bytes at %s, RAM footprint 0x2000 of the %s-byte window" % (LOAD, WINDOW),
        ] + RAN + BACK),
        net_case("net-missing", q + "setenv mp2z_file missing.bin; ", net_at("missing.bin") + [
            "mp2z: refused: tftpboot failed"] + BACK),
        net_case("net-image-size-above-window", q + "setenv mp2z_file bigimg.bin; ",
                 net_at("bigimg.bin") + [
                     "mp2z: refused: the image takes 0x10000001 bytes of RAM (header image_size), "
                     "above the %s-byte window" % WINDOW] + BACK),
        net_case("net-no-server", "", [UNDO, "mp2z: refused: mp2z_server is not set"] + BACK),
    ]
    if big_tftp:
        out.append(net_case("net-file-above-window", q + "setenv mp2z_file big.bin; ",
                            net_at("big.bin") + [
                                "mp2z: refused: file 10000001 bytes, above the %s-byte window" % WINDOW]
                            + BACK))
    return out


class Qemu:
    def __init__(self, argv, log_path):
        self.log = open(log_path, "wb")
        self.p = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.STDOUT)
        self.buf = b""
        self.pos = 0

    def expect(self, pat, timeout):
        end = time.monotonic() + timeout
        fd = self.p.stdout.fileno()
        while True:
            m = pat.search(self.buf, self.pos)
            if m:
                self.pos = m.end()
                return m
            left = end - time.monotonic()
            if left <= 0:
                raise TimeoutError("no %r within %ds" % (pat.pattern, timeout))
            r, _, _ = select.select([fd], [], [], min(left, 1.0))
            if r:
                chunk = os.read(fd, 65536)
                if not chunk:
                    raise EOFError("QEMU exited (rc %r)" % self.p.wait())
                self.buf += chunk
                self.log.write(chunk)
                self.log.flush()

    def send(self, text):
        self.p.stdin.write(text.encode())
        self.p.stdin.flush()

    def cmd(self, line, timeout=60):
        """Types LINE, returns its output lines (the echo dropped)."""
        start = len(self.buf)
        self.pos = max(self.pos, start)
        self.send(line + "\r")
        m = self.expect(PROMPT, timeout)
        text = self.buf[start:m.start()].decode(errors="replace").replace("\r", "")
        return text.split("\n")[1:]

    def boot(self):
        self.expect(STOP, 120)
        self.send(" ")
        self.expect(PROMPT, 20)

    def cut(self):
        """Power cut: SIGKILL, no poweroff, no flush. Keeps what was printed."""
        self.p.kill()
        self.p.wait()
        rest = self.p.stdout.read()
        self.buf += rest
        self.log.write(rest)

    def close(self):
        if self.p.poll() is None:
            try:
                self.send("poweroff\r")
                self.p.wait(10)
            except (OSError, subprocess.TimeoutExpired):
                self.p.kill()
                self.p.wait()
        self.log.close()


def typed(q, line):
    """Types LINE without waiting for the prompt."""
    q.pos = max(q.pos, len(q.buf))
    q.send(line + "\r")


def cutoff_cases(big_tftp):
    """Power cuts: (name, typed line, regex to cut on, seconds to wait after it)."""
    mmc = "setenv mp2z_dev 0; setenv mp2z_part 2; "
    net = "setenv mp2z_server 10.0.2.2; "
    out = [
        ("cut-mmc-after-undo", mmc + MMC, re.escape(UNDO), 0),
        ("cut-net-after-undo", net + NET, re.escape(UNDO), 0),
        # Kill while ext4load reads the 192 MiB image.
        ("cut-mmc-during-load", mmc + "setenv mp2z_file /yakogl/long.bin; " + MMC,
         r"mp2z: mmc 0:2 /yakogl/long\.bin", 0.1),
        # Kill while tftpboot is receiving (its progress hashes).
        ("cut-net-during-load", net + "setenv mp2z_file long.bin; " + NET, r"#####", 0.2),
    ]
    return out


def printenv(q):
    env = {}
    for ln in q.cmd("printenv"):
        if ln.startswith("Environment size:") or "=" not in ln:
            continue
        k, v = ln.split("=", 1)
        env[k] = v
    return env


def sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def main():
    ap = argparse.ArgumentParser()
    for a in ("qemu", "uboot", "disk", "env", "tftp", "mem", "logs"):
        ap.add_argument("--" + a, required=True)
    ap.add_argument("--big-tftp", action="store_true")
    a = ap.parse_args()
    argv = [a.qemu, "-machine", "virt", "-cpu", "cortex-a57", "-m", a.mem,
            "-display", "none", "-serial", "stdio", "-monitor", "none", "-no-reboot",
            "-bios", a.uboot,
            "-drive", "if=pflash,unit=1,format=raw,file=" + a.env,
            "-device", "sdhci-pci",
            "-drive", "if=none,id=sd0,format=raw,file=" + a.disk,
            "-device", "sd-card,drive=sd0",
            "-netdev", "user,id=n0,tftp=" + a.tftp,
            "-device", "e1000,netdev=n0"]
    res = {"ok": 0, "failed": 0}

    def check(name, cond, detail=""):
        res["ok" if cond else "failed"] += 1
        print("%s %s%s" % ("ok  " if cond else "FAIL", name,
                           "" if cond or not detail else ": " + detail), flush=True)

    def session(name, body):
        q = Qemu(argv, os.path.join(a.logs, name + ".log"))
        try:
            q.boot()
            return body(q)
        except (TimeoutError, EOFError) as e:
            check(name + ": console", False, "%s (log %s)" % (e, q.log.name))
            return None
        finally:
            q.close()

    # The stored environment: one saveenv, here and nowhere else.
    def setup(q):
        q.cmd("saveenv")
        return printenv(q)
    setup_env = session("setup", setup) or {}
    env_sha, disk_sha = sha(a.env), sha(a.disk)
    first = {}

    for c in cases(a.big_tftp):
        def body(q, c=c):
            before = printenv(q)
            out = q.cmd(c["line"], timeout=120)
            after = printenv(q)
            return before, out, after, q.buf
        got = session(c["name"], body)
        if got is None:
            continue
        before, out, after, raw = got
        first = first or before
        n = c["name"]
        check(n + ": stored environment read (no bad CRC)", b"bad CRC" not in raw)
        seq = [ln for ln in out if ln.startswith(("mp2z: ", "mp2q: ", "## Starting", "## Application"))]
        check(n + ": console lines in order", seq == c["expect"],
              "\n  got:    %r\n  expect: %r" % (seq, c["expect"]))
        went = any(ln.startswith("## Starting application") for ln in out)
        check(n + (": go reached" if "mp2z: go " + LOAD in c["expect"] else ": go not reached"),
              went == ("mp2z: go " + LOAD in c["expect"]))
        allowed = SIDE_EFFECT | (NET_SIDE_EFFECT if c["route"] == "net" else set())
        keys = set(before) | set(after)
        diff = sorted(k for k in keys if before.get(k) != after.get(k))
        bad = [k for k in diff if not k.startswith("mp2z_") and k not in allowed]
        # The typed net line's own "setenv autoload no" (not the script's).
        if c["route"] == "net" and "autoload" in bad and after.get("autoload") == "no" \
                and "autoload" not in before:
            bad.remove("autoload")
        check(n + ": printenv after = before (but mp2z_* and U-Boot's load/net variables)",
              not bad, "changed: " + ", ".join("%s %r -> %r" % (k, before.get(k), after.get(k))
                                               for k in bad))
        check(n + ": no saveenv", b"Saving Environment" not in raw)
        check(n + ": stored environment unchanged", sha(a.env) == env_sha)
        check(n + ": disk unchanged", sha(a.disk) == disk_sha)

    # A fresh boot reads back the same environment the first case started from.
    final = session("final", printenv)
    if final is not None:
        check("final: printenv after a reset = printenv of the first case",
              final == first and bool(first),
              "differs in " + ", ".join(sorted(k for k in set(final) | set(first)
                                               if final.get(k) != first.get(k))))
    # Power cuts: the stored environment must come out of each unchanged.
    for name, line, pat, wait in cutoff_cases(a.big_tftp):
        q = Qemu(argv, os.path.join(a.logs, name + ".log"))
        try:
            q.boot()
            typed(q, line)
            q.expect(re.compile(pat.encode()), 120)
            time.sleep(wait)
            q.cut()
            raw = q.buf.decode(errors="replace")
            check(name + ": undo line printed before the cut", UNDO in raw)
            check(name + ": cut before go", "mp2z: go " not in raw and "## Starting" not in raw
                  and "mp2z: loaded" not in raw)
            check(name + ": stored environment unchanged after the cut", sha(a.env) == env_sha)
            check(name + ": disk unchanged after the cut", sha(a.disk) == disk_sha)
        except (TimeoutError, EOFError) as e:
            check(name + ": console", False, "%s (log %s)" % (e, q.log.name))
            q.close()
            continue
        q.log.close()
        back = session(name + "-reboot", lambda q: (printenv(q), q.buf))
        if back is None:
            continue
        env, rawb = back
        check(name + ": reboot read the stored environment (no bad CRC)", b"bad CRC" not in rawb)
        diff = sorted(k for k in set(env) | set(setup_env)
                      if env.get(k) != setup_env.get(k)
                      and not k.startswith("mp2z_") and k not in SIDE_EFFECT)
        check(name + ": printenv after the reboot = setup printenv", not diff,
              "differs in " + ", ".join(diff))
        check(name + ": bootcmd is the stock value", env.get("bootcmd") == STOCK_BOOTCMD
              and env.get("bootcmd") == setup_env.get("bootcmd"), repr(env.get("bootcmd")))
        check(name + ": stored environment unchanged after the reboot", sha(a.env) == env_sha)

    print("run.sh: %d ok, %d failed" % (res["ok"], res["failed"]))
    return 0 if res["failed"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
