#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""mp2s_stage_selftest.py - tools/mp2s_stage.py on fake roots (plan #362,
item stage-files). No board; fake roots under a temp directory.

    python3 -I tools/mp2s_stage_selftest.py

The fake DK: /boot mounted from mmcblk1p4 (PARTNAME bootfs) of an SD card,
ST's stock files under boot/ (Image.gz, boot.scr.uimg, mmc0_extlinux/, a
symlink) and the one-shot's boot/yakogl/<token>.scr. Each case runs the CLI
(python3 -I) and checks its exit code, a substring of its output and the
files left on the fake bootfs. The read-back failure runs in process with
the module's readback() replaced. This is a .py so that the scope map of
tools/check.sh keeps it in `tools`.
"""
import hashlib
import importlib.util
import io
import contextlib
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
STAGE = os.path.join(HERE, "mp2s_stage.py")
UIMG = b"\x27\x05\x19\x56" + bytes(60) + b"echo mp2s\n"
IMAGE = bytes(56) + b"ARMd" + bytes(64) + b"mp2_wdtr: product refresh armed\n"
ENV = b"\x00" * 0x4000


def w(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb" if isinstance(data, bytes) else "w") as f:
        f.write(data)


def rdb(path):
    with open(path, "rb") as f:
        return f.read()


def mkroot(r, inputs):
    w(f"{r}/proc/self/mountinfo",
      "28 1 179:4 / /boot rw,relatime shared:3 - ext4 /dev/mmcblk1p4 rw\n")
    w(f"{r}/sys/block/mmcblk1/device/type", "SD\n")
    w(f"{r}/sys/block/mmcblk0/device/type", "MMC\n")
    w(f"{r}/sys/class/block/mmcblk1p4/uevent", "MAJOR=179\nPARTNAME=bootfs\n")
    w(f"{r}/sys/class/block/mmcblk0p4/uevent", "MAJOR=179\nPARTNAME=bootfs\n")
    w(f"{r}/sys/class/block/mmcblk1p5/uevent", "MAJOR=179\nPARTNAME=rootfs\n")
    w(f"{r}/boot/Image.gz", b"kernel")
    w(f"{r}/boot/boot.scr.uimg", b"st script")
    w(f"{r}/boot/mmc0_extlinux/extlinux.conf", "label st\n")
    w(f"{r}/boot/yakogl/tok.scr", b"one-shot")
    os.symlink("mmc0_extlinux", f"{r}/boot/extlinux")
    w(f"{inputs}/boot.scr.uimg", UIMG)
    w(f"{inputs}/u-boot-env.bin", ENV)
    w(f"{inputs}/image.bin", IMAGE)


def sd(r):
    return f"{r}/boot/yakogl/standalone"


def run(r, i, *args):
    argv = [a.replace("@", i) for a in args]
    p = subprocess.run([sys.executable, "-I", STAGE, "--root", r, *argv],
                       capture_output=True, text=True)
    return p.returncode, (p.stdout + p.stderr).strip()


INSTALL = ("install", "--script", "@/boot.scr.uimg", "--env-image", "@/u-boot-env.bin")
DEFAULT = ("default", "--image", "@/image.bin", "--free-bytes", "1000000")


def installed(r, i):
    rc, out = run(r, i, *INSTALL)
    assert rc == 0, out


def defaulted(r, i):
    installed(r, i)
    rc, out = run(r, i, *DEFAULT)
    assert rc == 0, out


def ls(r):
    return sorted(os.listdir(sd(r))) if os.path.isdir(sd(r)) else None


INSTALL_SET = ["boot-sha256.txt", "boot.scr.uimg", "boot.scr.uimg.sha256", "u-boot-env.bin"]


def chk_install(r, i):
    assert ls(r) == INSTALL_SET, ls(r)
    assert rdb(f"{sd(r)}/boot.scr.uimg") == UIMG
    assert rdb(f"{sd(r)}/u-boot-env.bin") == ENV
    h = hashlib.sha256(UIMG).hexdigest()
    assert rdb(f"{sd(r)}/boot.scr.uimg.sha256") == f"{h}  boot.scr.uimg\n".encode()
    rec = rdb(f"{sd(r)}/boot-sha256.txt").decode()
    assert "yakogl/standalone" not in rec, rec
    assert f"{hashlib.sha256(b'kernel').hexdigest()}  Image.gz\n" in rec, rec
    assert "symlink  extlinux -> mmc0_extlinux\n" in rec, rec
    assert "  yakogl/tok.scr\n" in rec, rec
    assert rdb(f"{r}/boot/boot.scr.uimg") == b"st script"


def chk_default(r, i):
    assert ls(r) == sorted(INSTALL_SET + ["default.bin"]), ls(r)
    assert rdb(f"{sd(r)}/default.bin") == IMAGE


def chk_uninstalled(r, i):
    assert ls(r) is None, ls(r)
    assert rdb(f"{r}/boot/yakogl/tok.scr") == b"one-shot"


def chk_untouched(r, i):
    assert ls(r) is None, ls(r)


def chk_installed_only(r, i):
    assert ls(r) == INSTALL_SET, ls(r)


def setup_stray(r, i):
    w(f"{sd(r)}/notes.txt", "x")


def setup_link(r, i):
    os.makedirs(f"{r}/elsewhere")
    os.makedirs(f"{r}/boot/yakogl", exist_ok=True)
    os.symlink("../../elsewhere", sd(r))


def setup_file_link(r, i):
    os.makedirs(sd(r))
    os.symlink("/etc/passwd", f"{sd(r)}/default.bin")


def setup_emmc(r, i):
    w(f"{r}/proc/self/mountinfo", "28 1 179:4 / /boot rw - ext4 /dev/mmcblk0p4 rw\n")


def setup_emmc_says_mmc(r, i):
    w(f"{r}/sys/block/mmcblk1/device/type", "MMC\n")


def setup_rootfs(r, i):
    w(f"{r}/proc/self/mountinfo", "28 1 179:5 / /boot rw - ext4 /dev/mmcblk1p5 rw\n")


def setup_unmounted(r, i):
    w(f"{r}/proc/self/mountinfo", "")


def setup_bad_script(r, i):
    w(f"{i}/boot.scr.uimg", b"echo not a uimage\n" + bytes(64))


def setup_installed_stock_changed(r, i):
    installed(r, i)
    w(f"{r}/boot/Image.gz", b"other kernel")


def setup_installed_tampered(r, i):
    installed(r, i)
    w(f"{sd(r)}/boot.scr.uimg", UIMG + b"x")


def setup_installed_off(r, i):
    installed(r, i)
    os.rename(f"{sd(r)}/boot.scr.uimg", f"{sd(r)}/boot.scr.uimg.off")


def setup_part_left(r, i):
    installed(r, i)
    w(f"{sd(r)}/default.bin.part", b"cut off")


def setup_defaulted_stock_changed(r, i):
    defaulted(r, i)
    w(f"{r}/boot/mmc0_extlinux/extlinux.conf", "label changed\n")
    rc, out = run(r, i, "remove-default")
    assert rc == 0, out


def setup_tiny(r, i):
    w(f"{i}/image.bin", b"x" * 0x3f)


H_OTHER = "0" * 64

# name, setup, argv, exit, substring, check of the files left
CASES = [
    ("install", None, INSTALL, 0, "installed boot.scr.uimg", chk_install),
    ("install_again", installed, INSTALL, 0, "matches the install record", chk_install),
    ("install_stray_file", setup_stray, INSTALL, 3, "'notes.txt', which is not ours", None),
    ("install_stage_symlink", setup_link, INSTALL, 3, "not a plain directory", None),
    ("install_symlink_entry", setup_file_link, INSTALL, 3, "not a regular file", None),
    ("install_boot_on_emmc", setup_emmc, INSTALL, 3, "mmcblk0", chk_untouched),
    ("install_disk_not_sd", setup_emmc_says_mmc, INSTALL, 3, "not SD", chk_untouched),
    ("install_not_bootfs", setup_rootfs, INSTALL, 3, "'rootfs' of the card, not bootfs", chk_untouched),
    ("install_unmounted", setup_unmounted, INSTALL, 3, "nothing is mounted on /boot", chk_untouched),
    ("install_not_uimage", setup_bad_script, INSTALL, 3, "is not a uImage", chk_untouched),
    ("install_stock_changed", setup_installed_stock_changed, INSTALL, 4, "changed  Image.gz", None),
    ("install_with_default", defaulted, INSTALL, 3, "nodefault first", chk_default),
    ("install_after_card_undo", setup_installed_off, INSTALL, 3, "uninstall first", None),
    ("install_missing_input", None, ("install", "--script", "@/nope", "--env-image", "@/u-boot-env.bin"), 2, "no such file", chk_untouched),
    ("default", installed, DEFAULT, 0, f"staged default.bin: {len(IMAGE)} bytes", chk_default),
    ("default_not_installed", None, DEFAULT, 3, "not installed", chk_untouched),
    ("default_twice", defaulted, DEFAULT, 3, "already staged", chk_default),
    ("default_script_tampered", setup_installed_tampered, DEFAULT, 3, "its .sha256 says", None),
    ("default_script_not_expected", installed, DEFAULT + ("--script-sha256", H_OTHER), 3, "not the expected", chk_installed_only),
    ("default_script_expected", installed, DEFAULT + ("--script-sha256", hashlib.sha256(UIMG).hexdigest()), 0, "read back", chk_default),
    ("default_half_free", installed, ("default", "--image", "@/image.bin", "--free-bytes", "200"), 3, "more than half", chk_installed_only),
    ("default_too_small", setup_tiny, DEFAULT, 3, "the window takes", chk_untouched),
    ("default_part_left", setup_part_left, DEFAULT, 0, "staged default.bin", chk_default),
    ("default_after_card_undo", setup_installed_off, DEFAULT, 3, "card undo", None),
    ("remove_default", defaulted, ("remove-default",), 0, "removed default.bin", chk_installed_only),
    ("remove_default_none", installed, ("remove-default",), 0, "no default.bin staged", chk_installed_only),
    ("remove_default_nothing", None, ("remove-default",), 0, "nothing staged", chk_untouched),
    ("status", defaulted, ("status",), 0, "matches the install record", chk_default),
    ("status_stock_changed", setup_installed_stock_changed, ("status",), 4, "NEEDS A PERSON", chk_installed_only),
    ("status_not_installed", None, ("status",), 0, "not installed", chk_untouched),
    ("uninstall", installed, ("uninstall",), 0, "matches the install record", chk_uninstalled),
    ("uninstall_with_default", defaulted, ("uninstall",), 3, "nodefault first", chk_default),
    ("uninstall_after_card_undo", setup_installed_off, ("uninstall",), 0, "boot.scr.uimg.off", chk_uninstalled),
    ("uninstall_stock_changed", setup_defaulted_stock_changed, ("uninstall",), 4, "changed  mmc0_extlinux/extlinux.conf", chk_uninstalled),
    ("uninstall_stray_file", setup_stray, ("uninstall",), 3, "not ours", None),
    ("uninstall_nothing", None, ("uninstall",), 0, "nothing installed", chk_untouched),
    ("usage", None, ("frobnicate",), 2, "invalid choice", None),
]


def readback_case(t):
    """A read-back that differs removes the file and changes nothing else."""
    spec = importlib.util.spec_from_file_location("mp2s_stage", STAGE)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    r, i = os.path.join(t, "readback", "root"), os.path.join(t, "readback", "in")
    mkroot(r, i)
    installed(r, i)
    mod.readback = lambda path: "0" * 64
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        rc = mod.main(["--root", r, "default", "--image", f"{i}/image.bin",
                       "--free-bytes", "1000000"])
    out = buf.getvalue().strip()
    ok = rc == 1 and "read-back" in out and ls(r) == INSTALL_SET
    return ok, "exit %d, '%s', files %s" % (rc, out, ls(r))


def main():
    fail = 0
    with tempfile.TemporaryDirectory(prefix="mp2s_stage.") as t:
        for name, setup, argv, want, sub, after in CASES:
            r, i = os.path.join(t, name, "root"), os.path.join(t, name, "in")
            mkroot(r, i)
            try:
                if setup:
                    setup(r, i)
                rc, out = run(r, i, *argv)
                good = rc == want and sub in out
                if good and after:
                    after(r, i)
            except AssertionError as e:
                good, rc, out = False, -1, "assertion: %s" % e
            if good:
                print(f"ok   {name}: {out.splitlines()[-1] if out else ''}")
            else:
                print(f"FAIL {name}: exit {rc} (want {want}), '{out}' (want '*{sub}*')")
                fail = 1
        ok, msg = readback_case(t)
        print(("ok   " if ok else "FAIL ") + "readback_mismatch: " + msg)
        fail |= not ok
    print("mp2s_stage_selftest: %d cases, %s" % (len(CASES) + 1, "all ok" if not fail else "FAILED"))
    return fail


if __name__ == "__main__":
    sys.exit(main())
