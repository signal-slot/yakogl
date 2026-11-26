#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Host self-test of tools/mp2_uboot_redact_check.py on DK U-Boot run records
generated here (plan #357, item uboot-redact-check).

    python3 -I tools/mp2_uboot_redact_selftest.py

No board, no network, python3 standard library only. One redacted record
(the shape tools/board-stm32mp2-uboot.sh's redact_run leaves: <redacted> env
values, addresses as 192.0.2.N) must pass; then one dirty copy per pattern
must be refused, naming the file and line, with the value masked. Fixture
values come from RFC 5737 (198.51.100.0/24, 203.0.113.0/24), locally
administered MACs and made-up names; the private-range, tailnet, MAC, tty and
login@host values are assembled at run time, so this file holds none of them
literally. The temporary directory is made under $TMPDIR. Last line:
"mp2_uboot_redact_selftest: N ok, M failed"; exit 0 only when M is 0.
"""
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
CHECK = os.path.join(HERE, "mp2_uboot_redact_check.py")
BOARD_VARS = ("BOARD_HOST", "BOARD_UART", "BOARD_USER", "NETBOOT_SERVER")

ok = failed = 0


def report(good, name, detail=""):
    global ok, failed
    if good:
        ok += 1
        print("ok   " + name)
    else:
        failed += 1
        print("FAIL " + name)
        for line in detail.splitlines():
            print("     " + line)


def run(args, stdin=None, extra_env=None):
    env = {k: v for k, v in os.environ.items() if k not in BOARD_VARS}
    env.update(extra_env or {})
    p = subprocess.run([sys.executable, "-I", CHECK] + args, input=stdin,
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
    return p.returncode, p.stdout.decode("utf-8", "replace"), \
        p.stderr.decode("utf-8", "replace")


def detail(rc, out, err):
    return "rc=%d\n%s%s" % (rc, out, err)


# ---- values, built from parts -------------------------------------------------
DEV = "/" + "dev"
COLON = ":"


def mac(sep, last):  # locally administered
    return sep.join(["02", "00", "5e", "10", "20", last])


V4_DOC1 = "198.51.100.23"
V4_DOC2 = "203.0.113.5"
V4_PRIV = ".".join(str(n) for n in (10, 20, 30, 40))
V4_TAIL = ".".join(str(n) for n in (100, 64 + 37, 7, 9))
TSNAME = "mp2bench.example-tailnet.ts" + "." + "net"
LOGIN = "ro" + "ot"
V6 = "fd12" + ":" + "3456" + "::" + "9"
SHA = "3f9a0c4e5b6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9a0b1c2d3e4f5a6b"

META = """board=stm32mp257f-dk
boot=uboot-mmc
image=mp2_hello
image_sha256=%s
env_dev_before=mmcblk1p3
env_dev_after=mmcblk1p3
env_before_bin_sha256=0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9
env_diff=empty
board_host=<board-host>
uart=<BOARD_UART>
""" % SHA

CONSOLE = (
    b"NOTICE:  BL2: v2.10-stm32mp2-r1.0\r\n"
    b"U-Boot 2023.10-stm32mp-r1 (Oct 01 2026 - 00:00:00 +0000)\r\n"
    b"\x1b[0mModel: STMicroelectronics STM32MP257F-DK\r\n"
    b"In:    serial@400e0000\r\nOut:   serial@400e0000\r\n"
    b"Net:   eth0: ethernet@482c0000\r\n\0\0"
    b"ethact=ethernet@8000000port@1\r\n"
    b"Hit any key to stop autoboot:  0 \r\nSTM32MP> "
    b"setenv mp2z_server 192.0.2.1; tftpboot ${scriptaddr} 192.0.2.1:yakogl/mp2_hello.scr\r\n"
    b"DHCP client bound to address 192.0.2.2 (12 ms)\r\n"
    b"mp2z: go 0xa0000000 (memory@a0000000, 0x00000000a0000000)\r\n"
    b"*** Booting Zephyr OS build v4.1.0 ***\r\nmp2_hello: PASS: hello\r\n"
    b"listening on 0.0.0.0, loopback 127.0.0.1, Linux 6.6.48, 1.2.3.4.5\r\n"
    b"[    1.2] 400e0000.serial: ttySTM0 at MMIO 0x400e0000\r\n"
    b"eth0 link 00:00:00:00:00:00\r\n")

ENV_TXT = """board_id=<redacted>
bootcmd=run bootcmd_stm32mp
ethaddr=<redacted>
ipaddr=<redacted>
scriptaddr=0x90100000
serial#=<redacted>
serverip=<redacted>
"""


def write(path, data, mode="w"):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, mode + ("b" if isinstance(data, bytes) else "")) as f:
        f.write(data)


def mkrec(r):
    write(os.path.join(r, "meta.txt"), META)
    write(os.path.join(r, "console.raw"), CONSOLE)
    write(os.path.join(r, "console.ts"),
          "0.001 U-Boot 2023.10-stm32mp-r1\n12.500 mp2_hello: PASS: hello\n")
    write(os.path.join(r, "typed.log"),
          "typed ok: 1 line on <BOARD_UART>, server <NETBOOT_SERVER>, ssh <board-host>\n")
    write(os.path.join(r, "env-before.txt"), ENV_TXT)
    write(os.path.join(r, "env-after.txt"), ENV_TXT)
    write(os.path.join(r, "env.diff"), "")
    write(os.path.join(r, "boot-sha256.txt"), "%s  /boot/yakogl/mp2_hello.bin\n" % SHA)
    write(os.path.join(r, "verdict.txt"),
          "verdict: PASS (mp2_hello: PASS: hello), env unchanged\n")
    write(os.path.join(r, "INDEX.txt"),
          "meta.txt console.raw console.ts typed.log env-before.txt "
          "env-after.txt verdict.txt\n")


def lines_of(path):
    with open(path, "rb") as f:
        return f.read().count(b"\n")


def main():
    base = os.environ.get("TMPDIR") or HERE
    with tempfile.TemporaryDirectory(prefix="mp2_uboot_redact_selftest.",
                                     dir=base) as t:
        body(t)
    print("mp2_uboot_redact_selftest: %d ok, %d failed" % (ok, failed))
    return 0 if failed == 0 else 1


def body(t):
    rec = os.path.join(t, "rec", "20261125-1200-mp2_hello-0123abcd")
    mkrec(rec)

    rc, out, err = run([rec])
    report(rc == 0 and not out, "redacted record passes", detail(rc, out, err))
    rc, out, err = run([os.path.join(rec, "console.raw"),
                        os.path.join(rec, "env-before.txt")])
    report(rc == 0, "redacted files given one by one pass", detail(rc, out, err))
    rc, out, err = run(["-"], stdin=CONSOLE)
    report(rc == 0, "stdin (-) of the redacted console passes", detail(rc, out, err))

    # ---- one dirty record per pattern -------------------------------------
    # A copy of the record with TEXT appended to FILE must be refused (exit 1)
    # with "FILE:<its line>: KIND:", and VALUE must not appear in the output.
    cases = [
        ("IPv4, RFC 5737 198.51.100/24 (not redact_run's 192.0.2/24)", "console.raw", "ipv4",
         "Using ethernet@482c0000 device, TFTP from server " + V4_DOC1, V4_DOC1),
        ("IPv4, RFC 5737 203.0.113/24", "console.ts", "ipv4",
         "13.000 DHCP client bound to address " + V4_DOC2, V4_DOC2),
        ("IPv4, private range", "meta.txt", "ipv4", "server=" + V4_PRIV, V4_PRIV),
        ("tailnet address (100.64/10)", "typed.log", "tailnet", "ssh to " + V4_TAIL, V4_TAIL),
        ("tailnet name (*.ts.net)", "verdict.txt", "tailnet", "host " + TSNAME, TSNAME),
        ("MAC, ':'", "console.raw", "mac", "eth0: link up, " + mac(COLON, "01"), mac(COLON, "01")),
        ("MAC, '-'", "INDEX.txt", "mac", "adapter " + mac("-", "02"), mac("-", "02")),
        ("IPv6 (ULA)", "console.raw", "ipv6", "inet6 %s/64" % V6, V6),
        ("/dev/ttyUSB*", "typed.log", "tty", "opened %s/tty%s at 115200" % (DEV, "USB0"), "tty" + "USB0"),
        ("/dev/tty* (ACM)", "meta.txt", "tty", "uart=%s/tty%s" % (DEV, "ACM3"), "tty" + "ACM3"),
        ("/dev/serial/by-id link", "typed.log", "tty",
         "uart %s/serial/by-id/usb-STMicroelectronics_STLINK-V3_004C1A2B3C4D-if02" % DEV, "1A2B3C4D"),
        ("USB serial (iSerial)", "typed.log", "serial", "  iSerial                 3 004C1A2B3C4D",
         "004C1A2B3C4D"),
        ("USB serial (SerialNumber)", "meta.txt", "serial", "SerialNumber: 0671FF3833", "0671FF3833"),
        ("USB serial (ID_SERIAL_SHORT)", "meta.txt", "serial", "ID_SERIAL_SHORT=A9F3K2Q7", "A9F3K2Q7"),
        ("ethaddr= value", "env-before.txt", "env", "ethaddr=" + mac(COLON, "03"), mac(COLON, "03")),
        ("serverip= value", "env-after.txt", "env", "serverip=" + V4_DOC1, V4_DOC1),
        ("ipaddr= value", "env-before.txt", "env", "ipaddr=" + V4_DOC2, V4_DOC2),
        ("serial#= value", "env-before.txt", "env", "serial#=002B00253232511639353236",
         "002B00253232511639353236"),
        ("board_id= value", "env-after.txt", "env", "board_id=0x00000AB3", "0x00000AB3"),
        ("hostname= value (env.diff '>' line)", "env.diff", "env", "> hostname=mp2bench", "mp2bench"),
        ("ssh login@host", "typed.log", "login", "ssh %s@mp2bench 'dd if=x'" % LOGIN,
         LOGIN + "@mp2bench"),
        ("ssh:// login@host", "meta.txt", "login", "board_host=ssh://%s@mp2bench" % LOGIN,
         LOGIN + "@mp2bench"),
        ("shell prompt login@host", "console.raw", "login", "%s@mp2bench:~# reboot" % LOGIN,
         LOGIN + "@mp2bench"),
        ("LAN host name (.local)", "typed.log", "lanhost", "server mp2bench.local", "mp2bench.local"),
    ]
    for n, (name, fname, kind, text, value) in enumerate(cases, 1):
        d = os.path.join(t, "dirty%d" % n)
        mkrec(d)
        path = os.path.join(d, fname)
        write(path, (text + "\n").encode(), "a")
        line = lines_of(path)
        rc, out, err = run([d])
        want = "%s:%d: %s: " % (path, line, kind)
        good = rc == 1 and want in out and value not in out and value not in err
        report(good, "refused: %s (%s:%d, %s, masked)" % (name, fname, line, kind),
               detail(rc, out, err) + "want 1 and '%s:%d: %s:'" % (fname, line, kind))

    # the values of the board variables, when set
    d = os.path.join(t, "envval")
    mkrec(d)
    write(os.path.join(d, "typed.log"), b"console on bridgebox9:4711\n", "a")
    rc, out, err = run([d], extra_env={"BOARD_UART": "bridgebox9:4711"})
    report(rc == 1 and "typed.log:2: envval: " in out and "bridgebox9" not in out + err,
           "refused: BOARD_UART's value (envval, masked)", detail(rc, out, err))
    rc, out, err = run([d])
    report(rc == 0, "the same line passes with BOARD_UART unset", detail(rc, out, err))

    # line numbers count '\n' lines of the raw capture (NULs, '\r', escapes removed)
    d = os.path.join(t, "lines")
    raw = os.path.join(d, "console.raw")
    write(raw, b"a\r\n\0b\r\n\x1b[1mc\x1b[0m\r\nfrom " + V4_DOC1.encode() + b"\r\n")
    rc, out, err = run([d])
    report(rc == 1 and ("%s:4: ipv4: 1***[13]" % raw) in out,
           "raw capture: line 4 of a NUL/CR/ANSI file, masked as 1***[13]",
           detail(rc, out, err))

    # ---- the allowlist ------------------------------------------------------
    d = os.path.join(t, "allow")
    mkrec(d)
    write(os.path.join(d, "console.raw"),
          ("getty on %s/tty%s\n" % (DEV, "STM0")).encode(), "a")
    rules = {
        "allow.ok": "# reviewed: the DK's own Linux console device, not a host UART path\n"
                    "tty | */console.raw | /dev/ttySTM0 | the board's own console, ST's image\n",
        "allow.glob": "tty | */typed.log | /dev/ttySTM0 | wrong file\n",
        "allow.part": "tty | */console.raw | /dev/tty | not the whole match\n",
        "allow.bad": "tty | */console.raw | /dev/ttySTM0\n",
        "allow.kind": "nosuchkind | * | x | y\n",
    }
    for name, text in rules.items():
        write(os.path.join(t, name), text)
    allow = lambda name: ["--allow", os.path.join(t, name), d]  # noqa: E731

    rc, out, err = run([d])
    report(rc == 1 and ": tty: /***" in out,
           "without --allow the board console path is refused", detail(rc, out, err))
    rc, out, err = run(allow("allow.ok"))
    report(rc == 0 and "1 finding(s) dropped" in err,
           "--allow: a reviewed rule drops it, counted on stderr", detail(rc, out, err))
    rc, out, err = run(allow("allow.glob"))
    report(rc == 1, "--allow: a rule for another file does not drop it", detail(rc, out, err))
    rc, out, err = run(allow("allow.part"))
    report(rc == 1, "--allow: the regex must match the whole hit", detail(rc, out, err))
    rc, out, err = run(allow("allow.bad"))
    report(rc == 2 and "allow.bad:1:" in err,
           "--allow: a rule without a reason stops with exit 2 and its line",
           detail(rc, out, err))
    rc, out, err = run(allow("allow.kind"))
    report(rc == 2, "--allow: an unknown kind stops with exit 2", detail(rc, out, err))
    rc, out, err = run(allow("no-such-allow"))
    report(rc == 2, "--allow: a missing allowlist stops with exit 2", detail(rc, out, err))

    # ---- usage and the tree ---------------------------------------------------
    rc, out, err = run([])
    report(rc == 2, "no path: exit 2", detail(rc, out, err))
    rc, out, err = run([os.path.join(t, "no-such-record")])
    report(rc == 2, "a missing path: exit 2", detail(rc, out, err))
    d = os.path.join(t, "link")
    mkrec(d)
    outside = os.path.join(t, "outside.txt")
    write(outside, "from %s\n" % V4_DOC1)
    os.symlink(outside, os.path.join(d, "outside.txt"))
    rc, out, err = run([d])
    report(rc == 0 and "not followed (symlink)" in err,
           "a symlink in the record is not followed, and said so", detail(rc, out, err))
    write(os.path.join(d, "sub", "extra.txt"), "from %s\n" % V4_DOC2)
    rc, out, err = run([d])
    report(rc == 1 and "sub/extra.txt:1: ipv4:" in out,
           "a subdirectory of the record is read", detail(rc, out, err))


if __name__ == "__main__":
    sys.exit(main())
