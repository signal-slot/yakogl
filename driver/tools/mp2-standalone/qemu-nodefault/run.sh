#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# run.sh: nodefault partway through a count, in U-Boot v2023.10 under QEMU
# (README.md in this directory). Reuses ../qemu (build-uboot.sh redund,
# mkdisk.sh env, drive.py) and ../qemu-product (product.py's constants) without
# changing them. Prints "qemu-nodefault: ok" and exits 0 when every check
# passed; "qemu-nodefault: SKIP <reason>" and exit 0 when an input is missing;
# "qemu-nodefault: FAIL ..." and exit 1 otherwise.
#   QEMU_SYSTEM_AARCH64  the QEMU binary (required)
#   UBOOT_V2023_10_SRC   a U-Boot v2023.10 tree; default
#                        $CONNECTED_DEPS_DIR/u-boot-v2023.10 when it exists
#   CROSS_COMPILE, MP2S_QEMU_CACHE, MP2S_QEMU_KEEP=1: as ../qemu-product/run.sh
set -euo pipefail

pdir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$pdir/../../../.." && pwd)
skip() { echo "qemu-nodefault: SKIP $*"; exit 0; }

qemu=${QEMU_SYSTEM_AARCH64:-}
[ -n "$qemu" ] && [ -x "$qemu" ] || skip "QEMU_SYSTEM_AARCH64 (unset or not executable)"
if [ -z "${UBOOT_V2023_10_SRC:-}" ]; then
	if [ -n "${CONNECTED_DEPS_DIR:-}" ] && [ -f "$CONNECTED_DEPS_DIR/u-boot-v2023.10/Makefile" ]; then
		export UBOOT_V2023_10_SRC=$CONNECTED_DEPS_DIR/u-boot-v2023.10
	else
		skip "UBOOT_V2023_10_SRC (unset, and no u-boot-v2023.10 in CONNECTED_DEPS_DIR)"
	fi
fi
[ -f "$UBOOT_V2023_10_SRC/Makefile" ] || skip "UBOOT_V2023_10_SRC ($UBOOT_V2023_10_SRC has no Makefile)"
# shellcheck source=../qemu/build-uboot.sh
. "$pdir/../qemu/build-uboot.sh"
cross=$(mp2_qemu_cross)
[ -n "$cross" ] || skip "CROSS_COMPILE (no aarch64 cross compiler)"
for t in python3 mke2fs sfdisk truncate; do
	command -v "$t" > /dev/null || [ -x "/usr/sbin/$t" ] || [ -x "/sbin/$t" ] || skip "$t (not found)"
done
cache=$(mp2s_cache)

rc=0
uboot=$(bash "$pdir/../qemu/build-uboot.sh" redund) || rc=$?
[ "$rc" -ne 77 ] || skip "CROSS_COMPILE (no aarch64 cross compiler)"
[ "$rc" -eq 0 ] || { echo "qemu-nodefault: FAIL ../qemu/build-uboot.sh redund failed ($rc)"; exit 1; }
uboot=$(echo "$uboot" | tail -n 1)
echo "qemu-nodefault: U-Boot (redund) $uboot"
echo "qemu-nodefault: QEMU $("$qemu" --version | head -n 1)"

run=$(mktemp -d "$cache/nodefault.XXXXXX")
cleanup() { if [ "${MP2S_QEMU_KEEP:-}" != 1 ] && [ "$rc" -eq 0 ]; then rm -rf "$run"; fi; }
trap cleanup EXIT
mkdir -p "$run/logs" "$run/script"

# The product script, the way the runner builds it; the stand-in image.
bash "$pdir/../build.sh" "$run/script" | tail -n 1
"${cross}gcc" -nostdlib -c -DIMGSZ=0x2000 -o "$run/standin.o" "$pdir/../qemu/image.S"
"${cross}objcopy" -O binary -j .text "$run/standin.o" "$run/standin.bin"
truncate -s 4096 "$run/standin.bin"

python3 -I - "$qemu" "$uboot" "$run" "$root" "$pdir" <<'PY' || rc=$?
import os
import shutil
import subprocess
import sys

qemu, uboot, run, root, pdir = sys.argv[1:6]
sys.dont_write_bytecode = True
sys.path.insert(0, os.path.join(pdir, "..", "qemu-product"))
import product                      # constants; it imports ../qemu/drive.py as product.drive

drive = product.drive
us = drive.load(os.path.join(root, "tools", "uboot_script.py"), "uboot_script")

bad = []


def check(name, cond, detail=""):
    print("%s qemu-nodefault: %s%s" % ("ok  " if cond else "FAIL", name,
                                       "" if cond or not detail else ": " + detail), flush=True)
    if not cond:
        bad.append(name)


# The disk: GPT, bootfs with the product script, the stand-in image and the
# stock stand-in; partition 3 u-boot-env (the redund U-Boot's two 8 KiB copies).
w = os.path.join(run, "work")
stage = os.path.join(w, "bootfs")
os.makedirs(os.path.join(stage, "yakogl", "standalone"))
shutil.copy(os.path.join(run, "script", "boot.scr.uimg"), os.path.join(stage, drive.YAKOGL))
shutil.copy(os.path.join(run, "standin.bin"),
            os.path.join(stage, "yakogl", "standalone", "default.bin"))
with open(os.path.join(stage, drive.STOCK), "wb") as g:
    g.write(us.build(product.STOCK_CMD.encode(), name="stock-standin", arch="arm64", time=0))
disk, flash = os.path.join(w, "sd.img"), os.path.join(w, "env.img")
subprocess.run(["bash", os.path.join(pdir, "..", "qemu", "mkdisk.sh"), disk, stage, "env"], check=True)
with open(flash, "wb") as f:
    f.truncate(64 << 20)
argv = [qemu, "-machine", "virt", "-cpu", "cortex-a57", "-m", "2G", "-display", "none",
        "-serial", "stdio", "-monitor", "none", "-no-reboot", "-bios", uboot,
        "-drive", "if=pflash,unit=1,format=raw,file=" + flash, "-device", "sdhci-pci",
        "-drive", "if=none,id=sd0,format=raw,file=" + disk, "-device", "sd-card,drive=sd0"]

# The u-boot-env partition as the DK's runner reads it (dd), and its layout.
PART = 0x1e00000                    # mkdisk.sh: start=61440 sectors
PSZ = 512 * 1024
LAY = ["--redundant", "--size", "0x2000", "--offset", "0x7e000", "--offset2", "0x7c000"]
envtool = os.path.join(root, "tools", "uboot_env.py")
plan = os.path.join(root, "tools", "mp2s_env_plan.py")
part = os.path.join(w, "part.bin")


def read_part(path):
    with open(disk, "rb") as f:
        f.seek(PART)
        data = f.read(PSZ)
    with open(path, "wb") as g:
        g.write(data)
    return data


def write_part(data):
    with open(disk, "r+b") as g:
        g.seek(PART)
        g.write(data)


def val(path, name):
    """The value of NAME in the partition image PATH (b"" when absent)."""
    r = subprocess.run([sys.executable, "-I", envtool] + LAY + ["print", "-n", path, name],
                       capture_output=True)
    return r.stdout if r.returncode == 0 else b""


def py(*a):
    return subprocess.run([sys.executable, "-I"] + list(a), capture_output=True, text=True)


def session(label, body):
    q = drive.Qemu(argv, os.path.join(run, "logs", "%s.log" % label))
    try:
        return body(q)
    except (TimeoutError, EOFError) as e:
        check("%s: console" % label, False, "%s (log %s)" % (e, q.log.name))
        return None
    finally:
        q.close()


fixture = product.read_fixture(os.path.join(root, "driver", "tools", "mp2-standalone",
                                            "st-env", "stm32mp25-dk-initial-env.txt"))
st = ["setenv %s '%s'" % (k, fixture[k]) for k in product.ST_VARS]


def setup(q):                       # the board as ST ships it; two saves: both copies valid
    q.stop_autoboot()
    for p in st + product.QEMU_VARS:
        q.cmd(p)
    q.cmd("saveenv")
    q.cmd("saveenv")
    return True


def full(q):
    q.wait_prompt(180)
    return q.lines()


def mp2(lines):
    return [ln for ln in lines if ln.startswith(("mp2s:", "mp2q:"))]


def in_order(lines, pats):
    return None not in drive.seq(lines, pats)


PROT = ("bootcmd", "bootcount", "altbootcmd")


def main():
    if not session("setup", setup):
        return check("setup boot", False)
    read_part(part)
    before = {k: val(part, k) for k in PROT}
    check("setup: bootcmd (ST's, from the fixture), bootcount, altbootcmd are set",
          all(before.values()) and before["bootcmd"].decode() == fixture["bootcmd"], repr(before))

    # (1) a counted default of 3, computed by the runner's tool, written back as
    # fw_setenv would (the inactive copy, flags + 1; the other copy untouched)
    rec = os.path.join(w, "default-prefixes.txt")
    out = os.path.join(w, "part-default.bin")
    r = py(plan, "default", part, out, "--count", "3", "--record", rec, *LAY)
    check("env_plan default --count 3 accepted", r.returncode == 0, r.stderr.strip())
    if r.returncode != 0:
        return
    with open(out, "rb") as f:
        write_part(f.read())
    lines1 = session("boot1", full)
    if lines1 is None:
        return
    print("     boot 1: " + " / ".join(mp2(lines1)), flush=True)
    check("boot 1: boots left 2, then the image",
          in_order(lines1, [product.FOUND_OURS, product.LEFT % 2, product.GO, product.IMAGE]))
    check("boot 1: no bad CRC", not any("bad CRC" in ln for ln in lines1))

    # (2) power is off (QEMU has exited): the host edits the partition
    mid_path = os.path.join(w, "part-mid.bin")
    mid = read_part(mid_path)
    check("after boot 1: the saved mp2s_left is 2", val(mid_path, "mp2s_left") == b"2")
    out2 = os.path.join(w, "part-nodefault.bin")
    r = py(plan, "nodefault", mid_path, out2, "--record", rec, *LAY)
    check("env_plan nodefault accepted", r.returncode == 0, r.stderr.strip())
    if r.returncode != 0:
        return
    with open(out2, "rb") as f:
        new = f.read()
    changed = [n for n, o in (("copy 1", 0x7e000), ("copy 2", 0x7c000))
               if new[o:o + 0x2000] != mid[o:o + 0x2000]]
    check("nodefault changes exactly one environment copy, as fw_setenv does",
          len(changed) == 1 and new[:0x7c000] == mid[:0x7c000] and new[0x80000:] == mid[0x80000:],
          "changed %r" % changed)
    write_part(new)
    r = py(plan, "verify", mid_path, out2, "--expect", "nodefault", *LAY)
    check("env_plan verify --expect nodefault", r.returncode == 0, r.stderr.strip())

    # (3) boot: the stock path
    lines2 = session("boot2", full)
    if lines2 is None:
        return
    print("     boot 2: " + " / ".join(mp2(lines2)), flush=True)
    check("boot 2: no bad CRC", not any("bad CRC" in ln for ln in lines2))
    check("boot 2: the stock path is reached", product.STOCK in lines2)
    check("boot 2: no 'mp2s:' line, no image, our script not found",
          not any(ln.startswith("mp2s:") for ln in lines2)
          and product.IMAGE not in lines2 and product.FOUND_OURS not in lines2, repr(mp2(lines2)))

    # (4) the protected variables, byte for byte, against before step 1
    after_path = os.path.join(w, "part-after.bin")
    read_part(after_path)
    for k in PROT:
        a = val(after_path, k)
        check("%s byte-identical to before step 1" % k, a == before[k], "%r -> %r" % (before[k], a))
        print("     %s: %r (before) / %r (after boot 2)" % (k, before[k], a), flush=True)
    for k in ("mp2s_ver", "mp2s_left", "mp2s_prefixes"):
        check("%s is gone from the saved environment" % k, val(after_path, k) == b"")
    check("boot_prefixes is back to the stock value",
          val(after_path, "boot_prefixes") == val(part, "boot_prefixes"))


main()
sys.exit(1 if bad else 0)
PY
if [ "$rc" -ne 0 ]; then
	echo "qemu-nodefault: FAIL (run directory and console logs kept in $run)"
	exit 1
fi
echo "qemu-nodefault: ok"
