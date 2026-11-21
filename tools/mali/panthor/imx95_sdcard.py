#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The pure parts of tools/mali/panthor/imx95-sdcard.sh (the Verdin iMX95's
microSD card for the panthor Linux route), so that its self-test can run them
on the host without a kernel, a rootfs or a card:

  imx95_sdcard.py config-check FRAGMENT DOTCONFIG
      every CONFIG_X=v and "# CONFIG_X is not set" line of FRAGMENT holds in
      DOTCONFIG; one "config: CONFIG_X: want v, have w" line a miss, exit 1
  imx95_sdcard.py bootcmd PARTUUID_PREFIX
      the U-Boot script source of the card (boot.cmd), its root
      PARTUUID=<prefix>-02
  imx95_sdcard.py mkscr IN OUT
      IN wrapped as a legacy U-Boot script image (boot.scr, what `mkimage -A
      arm64 -T script -C none` makes); the header's time is SOURCE_DATE_EPOCH
      (0 without it)
  imx95_sdcard.py scr-dump IN
      the script of a boot.scr on stdout after its magic, type and both CRCs
      are checked; exit 1 on a bad image
  imx95_sdcard.py layout DISK_ID BOOT_MIB TOTAL_MIB
      the sfdisk script of the card: MBR with that disk id, partition 1 FAT32
      (0x0c, bootable) of BOOT_MIB from 1 MiB, partition 2 Linux to the end
  imx95_sdcard.py pin TOML NAME
      the sha256 pinned for the first input of the entry NAME in a deps.toml
"""
import struct
import os
import sys
import zlib

IH_MAGIC = 0x27051956
IH_OS_LINUX, IH_ARCH_ARM64, IH_TYPE_SCRIPT, IH_COMP_NONE = 5, 22, 6, 0
HEADER = ">IIIIIIIBBBB32s"  # 64 bytes, big-endian (include/image.h)


def die(msg, code=1):
    print(f"imx95_sdcard: {msg}", file=sys.stderr)
    sys.exit(code)


def parse_config(text):
    """{CONFIG_X: value} with "n" for "# CONFIG_X is not set"."""
    out = {}
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("# CONFIG_") and line.endswith(" is not set"):
            out[line[2:-len(" is not set")]] = "n"
        elif line.startswith("CONFIG_") and "=" in line:
            k, v = line.split("=", 1)
            out[k] = v
    return out


def config_check(fragment, dotconfig):
    want = parse_config(fragment)
    have = parse_config(dotconfig)
    return [f"config: {k}: want {v}, have {have.get(k, 'n')}"
            for k, v in want.items() if have.get(k, "n") != v]


def bootcmd(prefix):
    if len(prefix) != 8 or any(c not in "0123456789abcdef" for c in prefix):
        die(f"PARTUUID prefix must be 8 lower-case hex digits: {prefix!r}", 2)
    return f"""\
# The Verdin iMX95 microSD card of the panthor Linux route
# (tools/mali/panthor/imx95-sdcard.sh). U-Boot's bootstd finds this script
# on partition 1 of mmc1 (boot_targets=mmc1 mmc0 dhcp) and sets devtype,
# devnum and distro_bootpart; run by hand (`load mmc 1:1 ${{scriptaddr}}
# boot.scr; source ${{scriptaddr}}`) it defaults to mmc 1:1. The device
# tree follows U-Boot's ${{variant}} (wifi | nonwifi, from the module's
# config block) on the Mallow, not ${{fdtfile}} (whose default board is
# "dev"); ${{sd_fdtfile}} names another, ${{sd_extra_args}} adds kernel
# arguments (for example crashkernel=256M), both only when set.
if test -z "${{devtype}}"; then setenv devtype mmc; fi
if test -z "${{devnum}}"; then setenv devnum 1; fi
if test -z "${{distro_bootpart}}"; then setenv distro_bootpart 1; fi
if test "${{variant}}" = "wifi"; then setenv sd_fdt imx95-verdin-wifi-mallow.dtb; else setenv sd_fdt imx95-verdin-nonwifi-mallow.dtb; fi
if test -n "${{sd_fdtfile}}"; then setenv sd_fdt "${{sd_fdtfile}}"; fi
setenv bootargs "root=PARTUUID={prefix}-02 rootfstype=ext4 rootwait rw console=ttyLP2,115200 ${{sd_extra_args}}"
echo "imx95-sdcard: ${{devtype}} ${{devnum}}:${{distro_bootpart}} Image dtbs/${{sd_fdt}}"
if load ${{devtype}} ${{devnum}}:${{distro_bootpart}} ${{kernel_addr_r}} Image && load ${{devtype}} ${{devnum}}:${{distro_bootpart}} ${{fdt_addr_r}} dtbs/${{sd_fdt}}; then booti ${{kernel_addr_r}} - ${{fdt_addr_r}}; fi
echo "imx95-sdcard: the card's kernel did not boot"
"""


def mkscr(script, stamp):
    data = struct.pack(">II", len(script), 0) + script
    dcrc = zlib.crc32(data) & 0xFFFFFFFF
    name = b"imx95-sdcard boot script"
    fields = [IH_MAGIC, 0, stamp, len(data), 0, 0, dcrc,
              IH_OS_LINUX, IH_ARCH_ARM64, IH_TYPE_SCRIPT, IH_COMP_NONE, name]
    hcrc = zlib.crc32(struct.pack(HEADER, *fields)) & 0xFFFFFFFF
    fields[1] = hcrc
    return struct.pack(HEADER, *fields) + data


def scr_dump(image):
    size = struct.calcsize(HEADER)
    if len(image) < size + 8:
        raise ValueError("shorter than a header and a length word")
    f = list(struct.unpack(HEADER, image[:size]))
    if f[0] != IH_MAGIC:
        raise ValueError(f"magic {f[0]:#x}")
    hcrc, f[1] = f[1], 0
    if zlib.crc32(struct.pack(HEADER, *f)) & 0xFFFFFFFF != hcrc:
        raise ValueError("header crc")
    data = image[size:size + f[3]]
    if len(data) != f[3] or zlib.crc32(data) & 0xFFFFFFFF != f[6]:
        raise ValueError("data crc or size")
    if (f[8], f[9]) != (IH_ARCH_ARM64, IH_TYPE_SCRIPT):
        raise ValueError(f"arch {f[8]} type {f[9]}: not an arm64 script")
    n, end = struct.unpack(">II", data[:8])
    if end != 0 or 8 + n > len(data):
        raise ValueError("not a single-script image")
    return data[8:8 + n]


def layout(disk_id, boot_mib, total_mib):
    if not 0 < disk_id < 1 << 32:
        die("the disk id is a non-zero 32-bit number", 2)
    if boot_mib < 32 or total_mib < 1 + boot_mib + 512:
        die("a boot partition of 32 MiB or more and 512 MiB or more of root", 2)
    sect = 2048  # 512-byte sectors in a MiB
    return (f"label: dos\nlabel-id: {disk_id:#010x}\nunit: sectors\n\n"
            f"start={sect}, size={boot_mib * sect}, type=c, bootable\n"
            f"start={(1 + boot_mib) * sect}, size={(total_mib - 1 - boot_mib) * sect}, type=83\n")


def pin(toml_path, name):
    import tomllib
    with open(toml_path, "rb") as f:
        deps = tomllib.load(f).get("dep", [])
    for d in deps:
        if d.get("name") == name:
            inputs = d.get("inputs") or die(f"{name} has no inputs")
            return inputs[0].get("sha256") or die(f"{name}: its first input pins no sha256")
    die(f"no entry {name} in {toml_path}")


def main(argv):
    if not argv:
        die(__doc__.strip(), 2)
    cmd, args = argv[0], argv[1:]
    if cmd == "config-check" and len(args) == 2:
        with open(args[0]) as a, open(args[1]) as b:
            bad = config_check(a.read(), b.read())
        for line in bad:
            print(line)
        return 1 if bad else 0
    if cmd == "bootcmd" and len(args) == 1:
        sys.stdout.write(bootcmd(args[0]))
        return 0
    if cmd == "mkscr" and len(args) == 2:
        stamp = int(os.environ.get("SOURCE_DATE_EPOCH") or 0)
        with open(args[0], "rb") as f:
            out = mkscr(f.read(), stamp)
        with open(args[1], "wb") as f:
            f.write(out)
        return 0
    if cmd == "scr-dump" and len(args) == 1:
        with open(args[0], "rb") as f:
            try:
                sys.stdout.buffer.write(scr_dump(f.read()))
            except ValueError as e:
                die(f"{args[0]}: not a script image: {e}")
        return 0
    if cmd == "layout" and len(args) == 3:
        sys.stdout.write(layout(int(args[0], 0), int(args[1]), int(args[2])))
        return 0
    if cmd == "pin" and len(args) == 2:
        print(pin(args[0], args[1]))
        return 0
    die(f"usage: see {sys.argv[0]} with no argument", 2)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
