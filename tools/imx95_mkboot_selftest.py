#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""No-board selftest of tools/imx95-mkboot.py.

A synthetic i.MX 95 boot image in the layout of mkimage's flash_a55 for a B0
part (docs/notes/imx95-direct-boot.md, "The containers"), written here with
its own packing, not the tool's: container 0 an NXP-signed ELE container,
container 1 the DDR dummy, the OEI, the System Manager, SPL and the V2X dummy,
container 2 BL31, U-Boot and OP-TEE. No vendor binary: every image is filler
bytes. The tool's output is then read back here field by field (offsets,
alignment, hashes, the trampoline's instructions, the bytes before container
2), and every refusal is provoked once. Then a loader in the BL33 slot (its
AArch64 header's image_size against SPL's heap), the named limits (boot1,
boot0), and the boot count record (`count`, `count-info`): its bytes and the
CRC-32 the loader's own host test pins (driver/samples/zephyr/boot_stage2/
loader/src/emmc_tests.rs). Writes only to its own temporary directory."""
import hashlib
import os
import shutil
import struct
import subprocess
import sys
import tempfile

TOOL = os.path.join(os.path.dirname(os.path.abspath(__file__)), "imx95-mkboot.py")
d = tempfile.mkdtemp(prefix="imx95_mkboot_selftest.", dir=os.environ.get("TMPDIR"))
fails = 0
checks = 0


def ok(cond, what):
    global fails, checks
    checks += 1
    if not cond:
        fails += 1
        print("FAIL %s" % what)


def stop():
    """A build that should have worked did not: what follows reads its output."""
    shutil.rmtree(d)
    print("imx95_mkboot_selftest: %d checks, %d failed (stopped early)" % (checks, fails))
    sys.exit(1)


def filler(n, seed):
    out = bytearray()
    x = seed
    while len(out) < n:
        x = (x * 1103515245 + 12345) & 0xFFFFFFFF
        out += struct.pack("<I", x)
    return bytes(out[:n])


def sha384_padded(data, size):
    return hashlib.sha384(data.ljust(size, b"\0")).digest().ljust(64, b"\0")


def header(version, length, n, sig_off, flags=0):
    # version, length (le16), tag 0x87, flags, sw_version, fuse_version, images, sig offset, reserved
    return struct.pack("<BHBIHBBHH", version, length, 0x87, flags, 0, 0, n, sig_off, 0)


def entry(off, size, dst, ent, flags, meta, digest):
    return struct.pack("<IIQQII", off, size, dst, ent, flags, meta) + digest + b"\0" * 32


def sigblk(length=0x10, srk=0):
    return struct.pack("<BHBHHHHI", 0, length, 0x90, srk, 0, 0, 0, 0)


def put(buf, at, data):
    if len(buf) < at + len(data):
        buf.extend(b"\0" * (at + len(data) - len(buf)))
    buf[at:at + len(data)] = data


# flags: type | core << 4 | hash << 8 (1 = sha384); meta 2 = A55 core 0
EXEC_A55, EXEC_M33, OEI_M33 = 0x123, 0x113, 0x115
DDR_DUMMY, V2X_DUMMY, ELE = 0x10D, 0x11E, 0x166


def c2_bytes(images, version=2, start=0xC000, signed=False):
    """images: [(dst, entry, data)]; mkimage's -c -ap ... for container 2."""
    n = len(images)
    sig_off = 0x10 + 0x80 * n
    body = bytearray(start)
    ents = b""
    off = start
    for dst, ent, data in images:
        size = (len(data) + 0x3FF) // 0x400 * 0x400
        ents += entry(off, size, dst, ent, EXEC_A55, 2, sha384_padded(data, size))
        put(body, off, data.ljust(size, b"\0"))
        off += size
    head = header(version, sig_off + 0x10, n, sig_off, 0x10) + ents + (sigblk(0x40, 0x20) if signed else sigblk())
    put(body, 0, head)
    return bytes(body)


BL31 = filler(0x9A31, 1)
UBOOT = filler(0xC5555, 2)
TEE = filler(0x3E001, 3)


def base_image(c2_images, c2_signed=False, v2x=True, trailing=0x20000):
    """Toradex's imx-boot (7.8.0, mkimage flash_a55, OEI=YES, V2X=YES) in form:
    the ELE container at 0, the V2X firmware container at 0x4000 (version 0,
    tag 0x82, both NXP-signed), the boot container at 0x8000, container 2 at
    the next 1 KiB after the boot container's images. v2x=False leaves the
    V2X container out (V2X=NO): the boot container is then at 0x4000."""
    img = bytearray()
    ele = filler(0x1000, 4)
    # an odd header length, as the real ELE container's 0x2023: SPL aligns it to 0x4000
    head0 = header(2, 0x2023, 1, 0x90) + entry(0xC000, 0x1000, 0xE7FF8000, 0xE4000000, ELE, 0,
                                              sha384_padded(ele, 0x1000)) + sigblk(0x200, 0x40)
    put(img, 0, head0)
    put(img, 0xC000, ele)
    c1 = 0x4000
    if v2x:
        pri, sec = filler(0x800, 13), filler(0x400, 14)
        hv = struct.pack("<BHBIHBBHH", 0, 0x340, 0x82, 1, 0, 0, 2, 0x110, 0)
        hv += entry(0x5000, 0x800, 0x6000000, 0x6000000, 0x19B, 0, sha384_padded(pri, 0x800))
        hv += entry(0x6000, 0x400, 0x6000000, 0x6000000, 0x1AC, 0, sha384_padded(sec, 0x400)) + sigblk(0x100, 0x20)
        put(img, 0x4000, hv)
        put(img, 0x9000, pri)
        put(img, 0xA000, sec)
        c1 = 0x8000
    oei, sm, spl = filler(0x800, 5), filler(0x1400, 6), filler(0x1E00, 7)
    a_oei = c1 + 0x5000
    a_sm = a_oei + 0x800 + 0x10000          # -hold 65536 after the OEI
    a_spl = a_sm + 0x1400
    ents = [
        entry(a_oei + 0x800 - c1, 0, 0, 0, DDR_DUMMY, 0, sha384_padded(b"", 0)),
        entry(a_oei - c1, 0x800, 0x1FFC0000, 0x1FFC0001, OEI_M33, 0, sha384_padded(oei, 0x800)),
        entry(a_sm - c1, 0x1400, 0x1FFC0000, 0x1FFC0000, EXEC_M33, 0, sha384_padded(sm, 0x1400)),
        # SPL's size not a multiple of 1 KiB: SPL rounds the end up to 1 KiB
        entry(a_spl - c1, 0x1E00, 0x20480000, 0x20480000, EXEC_A55, 2, sha384_padded(spl, 0x1E00)),
        entry(a_spl + 0x1E00 - c1, 0, 0x8B000000, 0x8B000000, V2X_DUMMY, 0, sha384_padded(b"", 0)),
    ]
    put(img, c1, header(2, 0x10 + 0x80 * 5 + 0x10, 5, 0x10 + 0x80 * 5, 0x10) + b"".join(ents) + sigblk())
    put(img, a_oei, oei)
    put(img, a_sm, sm)
    put(img, a_spl, spl)
    c2_at = (a_spl + 0x1E00 + 0x3FF) // 0x400 * 0x400
    put(img, c2_at, c2_bytes(c2_images, signed=c2_signed))
    img.extend(b"\0" * trailing)
    return bytes(img), c2_at


THREE = [(0x8A200000, 0x8A200000, BL31), (0x90200000, 0x90200000, UBOOT), (0x8C000000, 0x8C000000, TEE)]
base, C2 = base_image(THREE)
ok(C2 == 0x20C00, "the synthetic image's container 2 offset (0x%x)" % C2)


def w(name, data):
    p = os.path.join(d, name)
    with open(p, "wb") as f:
        f.write(data)
    return p


def run(*args, env=None):
    e = dict(os.environ)
    e.pop("IMX95_BOOT_BASE", None)
    e.update(env or {})
    r = subprocess.run([sys.executable, "-I", TOOL] + list(args), capture_output=True, text=True, env=e)
    return r.returncode, r.stdout + r.stderr


def parse_c2(blob, at):
    v, ln, tag, flags, sw, fuse, n, sig_off, res = struct.unpack_from("<BHBIHBBHH", blob, at)
    ims = []
    for i in range(n):
        o, size, dst, ent, fl, meta = struct.unpack_from("<IIQQII", blob, at + 0x10 + 0x80 * i)
        dg = blob[at + 0x10 + 0x80 * i + 32: at + 0x10 + 0x80 * i + 96]
        ims.append(dict(off=o, size=size, dst=dst, entry=ent, flags=fl, meta=meta, hash=dg,
                        data=blob[at + o:at + o + size]))
    sig = struct.unpack_from("<BHBHHHHI", blob, at + sig_off)
    return dict(version=v, length=ln, tag=tag, flags=flags, n=n, sig_off=sig_off, sig=sig, images=ims)


LIMIT = "0x%x" % (0x400000 - 0x2200)
p_base = w("base.bin", base)

# info and check on Toradex's layout
rc, out = run("info", p_base)
ok(rc == 0, "info exits 0 (%d: %s)" % (rc, out))
ok("container 2 at 0x20c00" in out, "info names SPL's container 2 offset")
ok(out.count("sha384 ok") == 1 + 2 + 5 + 3, "info: every image hash with a size checks out (%s)" % out)
ok("container V2X @0x4000" in out and "container 1 (boot) @0x8000" in out, "info: the V2X container, then the boot container")
ok("(bl31)" in out and "(bl33)" in out and "(tee)" in out, "info names the three roles")
ok(out.count("\n") > 1 and "signed yes" in out.splitlines()[1], "info: container 0 is signed")
rc, out = run("check", p_base, "--limit", LIMIT)
ok(rc == 0 and "check: ok" in out, "check passes Toradex's layout (%s)" % out)

# without the V2X container (V2X=NO) the boot container is at 0x4000
nov2x, c2_nov2x = base_image(THREE, v2x=False)
rc, out = run("check", w("nov2x.bin", nov2x))
ok(c2_nov2x == 0x1CC00 and rc == 0 and "container 2 at 0x1cc00" in out, "check: no V2X container (%s)" % out)

# extract
rc, out = run("extract", p_base, os.path.join(d, "x"))
ok(rc == 0, "extract exits 0")
if rc != 0:
    stop()
for name, data in (("bl31.bin", BL31), ("bl33.bin", UBOOT), ("tee.bin", TEE)):
    got = open(os.path.join(d, "x", name), "rb").read()
    ok(got.rstrip(b"\0") == data.rstrip(b"\0") and len(got) % 0x400 == 0, "extract: %s" % name)

# build --image: the trampoline and the image at 0xd0000000
zimg = bytearray(filler(0x5123, 9))
zimg[0x08:0x10] = struct.pack("<Q", 0)
zimg[0x38:0x3C] = b"ARM\x64"
zimg = bytes(zimg)
p_z = w("zephyr.bin", zimg)
p_out = os.path.join(d, "direct.bin")
rc, out = run("build", "--base", p_base, "--image", p_z, "-o", p_out, "--limit", LIMIT)
ok(rc == 0, "build --image exits 0 (%s)" % out)
if rc != 0:
    stop()
new = open(p_out, "rb").read()
ok(new[:C2] == base[:C2], "build: containers 0 and 1 byte for byte")
c = parse_c2(new, C2)
ok((c["version"], c["tag"], c["n"], c["flags"]) == (2, 0x87, 4, 0x10),
   "build: container 2 version 2, tag 0x87, 4 images, the base's flags 0x10")
ok(c["length"] == 0x10 + 4 * 0x80 + 0x10 and c["sig_off"] == 0x10 + 4 * 0x80, "build: header length and signature offset")
ok(c["sig"] == (0, 0x10, 0x90, 0, 0, 0, 0, 0), "build: an empty signature block header")
ims = c["images"]
ok([i["dst"] for i in ims] == [0x8A200000, 0x90200000, 0x8C000000, 0xD0000000], "build: BL31, BL33, TEE, the image, in that order")
ok(ims[0]["off"] == 0xC000, "build: the first image where mkimage puts it (0xC000)")
ok(all(i["off"] % 0x400 == 0 and i["size"] % 0x400 == 0 for i in ims), "build: offsets and sizes in 1 KiB")
ok(all(a["off"] + a["size"] == b["off"] for a, b in zip(ims, ims[1:])), "build: the images back to back")
ok(all(i["hash"] == sha384_padded(i["data"], i["size"]) for i in ims), "build: every SHA-384 over the padded image")
ok(ims[0]["data"].rstrip(b"\0") == BL31.rstrip(b"\0") and ims[2]["data"].rstrip(b"\0") == TEE.rstrip(b"\0"),
   "build: BL31 and OP-TEE copied")
ldr, br, target = struct.unpack_from("<IIQ", ims[1]["data"])
# LDR (literal), 64-bit: 0x58000000 | imm19 << 5 | Rt; BR: 0xD61F0000 | Rn << 5
ok(ldr & 0xFF00001F == 0x58000010 and (ldr >> 5) & 0x7FFFF == 2, "trampoline: ldr x16, [pc + 8]")
ok(br == 0xD61F0000 | 16 << 5, "trampoline: br x16")
ok(target == 0xD0000000 and ims[1]["size"] == 0x400, "trampoline: the target 0xd0000000, one sector")
ok(ims[3]["entry"] == 0xD0000000 and ims[3]["data"][:len(zimg)] == zimg, "build: zephyr.bin at its load address")
ok((ims[3]["flags"], ims[3]["meta"]) == (EXEC_A55, 2), "build: the image's flags and meta are the BL33 slot's")
rc, out = run("check", p_out, "--limit", LIMIT)
ok(rc == 0 and "4 images (bl31, bl33, tee, image@0xd0000000)" in out, "check passes the build (%s)" % out)

# build --bl33: a loader in U-Boot's slot, from IMX95_BOOT_BASE
loader = filler(0x18765, 11)
p_l = w("loader.bin", loader)
p_out2 = os.path.join(d, "loader-direct.bin")
rc, out = run("build", "--bl33", p_l, "-o", p_out2, "--limit", LIMIT, env={"IMX95_BOOT_BASE": p_base})
ok(rc == 0, "build --bl33 with IMX95_BOOT_BASE exits 0 (%s)" % out)
if rc != 0:
    stop()
c = parse_c2(open(p_out2, "rb").read(), C2)
ok(c["n"] == 3 and c["images"][1]["dst"] == 0x90200000 and c["images"][1]["data"][:len(loader)] == loader,
   "build --bl33: the loader in U-Boot's slot")

# a base without OP-TEE
base2, _ = base_image(THREE[:2])
rc, out = run("build", "--base", w("base2.bin", base2), "--image", p_z, "-o", os.path.join(d, "d2.bin"), "--limit", LIMIT)
ok(rc == 0, "build from a base without OP-TEE (%s)" % out)
if rc != 0:
    stop()
ok(parse_c2(open(os.path.join(d, "d2.bin"), "rb").read(), C2)["n"] == 3, "without OP-TEE: three images")


def refused(what, args, expect, env=None):
    rc, out = run(*args, env=env)
    ok(rc == 1 and expect in out, "%s: refused with '%s' (%d: %s)" % (what, expect, rc, out.strip()))


o = os.path.join(d, "no.bin")
refused("the limit", ["build", "--base", p_base, "--image", p_z, "-o", o, "--limit", "0x1f000"], "past the limit")
refused("no Image header", ["build", "--base", p_base, "--image", p_l, "-o", o, "--limit", LIMIT], "no AArch64 Image header")
refused("a load below BL33", ["build", "--base", p_base, "--image", p_z, "--load", "0x88000000", "-o", o, "--limit", LIMIT],
        "below 0x90200000")
big = bytearray(filler(0x200000, 12))
big[0x38:0x3C] = b"ARM\x64"
big[0x08:0x10] = b"\0" * 8
refused("SPL's heap", ["build", "--base", p_base, "--image", w("big.bin", bytes(big)), "--load", "0x93100000", "-o", o,
                       "--limit", "0x10000000"], "overlaps SPL's heap")
refused("--no-tee alone", ["build", "--base", p_base, "--image", p_z, "--no-tee", "-o", o, "--limit", LIMIT], "--no-tee needs --bl31")
refused("no base", ["build", "--image", p_z, "-o", o, "--limit", LIMIT], "no base image")
signed, _ = base_image(THREE, c2_signed=True)
refused("a signed container 2", ["build", "--base", w("signed.bin", signed), "--image", p_z, "-o", o, "--limit", LIMIT],
        "is signed")
ok(not os.path.exists(o), "no output file after a refusal")

# checks that fail
bad = bytearray(new)
bad[C2 + 0xC000 + 5] ^= 0xFF
rc, out = run("check", w("bad-hash.bin", bytes(bad)))
ok(rc == 1 and "bl31: sha384 of the image does not match" in out, "check: a changed BL31 byte (%s)" % out)
swapped, _ = base_image([THREE[1], THREE[0], THREE[2]])
rc, out = run("check", w("swapped.bin", swapped))
ok(rc == 1 and "image 0 is not BL31" in out, "check: BL31 not first (%s)" % out)
stray, _ = base_image([THREE[0], (0x90200000, 0x90200000, struct.pack("<IIQ", 0x58000050, 0xD61F0200, 0xD0000000))])
rc, out = run("check", w("stray.bin", stray))
ok(rc == 1 and "branches to 0xd0000000, where no image is loaded" in out, "check: a trampoline to nothing (%s)" % out)
notag = bytearray(base)
notag[C2 + 3] = 0x00
rc, out = run("info", w("notag.bin", bytes(notag)))
ok(rc == 1 and "no container header at 0x20c00" in out, "info: no container at SPL's offset (%s)" % out)
rc, out = run("check", p_out, "--limit", "0x20000")
ok(rc == 1 and "past the limit 0x20000" in out, "check: the limit")

# --bl33 with a loader's AArch64 Image header (driver/samples/zephyr/boot_stage2
# for verdin_imx95: RAM 0x90200000 + 32 MiB): its image_size must end below
# SPL's heap at 0x93200000; the named limits.
def loader_image(image_size, n=0x1F000):
    b = bytearray(filler(n, 15))
    b[0x08:0x10] = b"\0" * 8
    b[0x10:0x18] = struct.pack("<Q", image_size)
    b[0x38:0x3C] = b"ARM\x64"
    return bytes(b)


p_ld = w("bs2.bin", loader_image(0x89000))
rc, out = run("build", "--base", p_base, "--bl33", p_ld, "-o", os.path.join(d, "bs2-direct.bin"), "--limit", "boot1")
ok(rc == 0 and "of the limit 0x1f7dc00" in out, "build --bl33 a loader, --limit boot1 = 0x1f7dc00 (%s)" % out)
rc, out = run("check", os.path.join(d, "bs2-direct.bin"), "--limit", "boot0")
ok(rc == 0 and "limit 0x1f7de00" in out, "check --limit boot0 = 0x1f7de00 (%s)" % out)
refused("a loader whose RAM reaches SPL's heap",
        ["build", "--base", p_base, "--bl33", w("bs2-big.bin", loader_image(0x3000001)), "-o", o, "--limit", "boot1"],
        "past 0x93200000")

# count: the record the loader reads at boot1 sector 64494 (loader/src/emmc.rs)
p_c = os.path.join(d, "count.bin")
rc, out = run("count", "--remaining", "3", "-o", p_c)
ok(rc == 0 and "seek=64494" in out and "offset 0x1f7dc00" in out, "count exits 0 and names the sector (%s)" % out)
rec = open(p_c, "rb").read() if rc == 0 else b""
ok(len(rec) == 512 and rec[:8] == b"YKBOOTCT", "count: one sector, the magic")
ok(rec[8:24] == struct.pack("<IIII", 1, 3, 3, 64494) and not any(rec[24:508]), "count: version, remaining, initial, sector, zeros")
# The same CRC the loader's host test pins (emmc_tests.rs record_layout_and_crc).
ok(rec[508:] == struct.pack("<I", 0xB8414914), "count: the CRC-32 the loader's test expects")
rc, out = run("count-info", p_c)
ok(rc == 0 and "3 of 3 starts left" in out, "count-info reads it back (%s)" % out)
rc, out = run("count-info", w("zero.bin", b"\0" * 512))
ok(rc == 1 and "no record (magic)" in out and "way back" in out, "count-info: boot1 as delivered (%s)" % out)
dump = bytearray(64512 * 512)
dump[64494 * 512:64495 * 512] = rec
rc, out = run("count-info", w("boot1.bin", bytes(dump)))
ok(rc == 0 and "boot1 dump at 0x1f7dc00" in out and "3 of 3" in out, "count-info: a whole boot1 dump (%s)" % out)
bad = bytearray(rec)
bad[12] = 2
rc, out = run("count-info", w("count-bad.bin", bytes(bad)))
ok(rc == 1 and "CRC" in out, "count-info: a changed count without its CRC (%s)" % out)
rc, out = run("count", "--remaining", "0", "--initial", "2", "-o", os.path.join(d, "count0.bin"))
rc2, out2 = run("count-info", os.path.join(d, "count0.bin"))
ok(rc == 0 and rc2 == 0 and "0 of 2" in out2 and "takes the way back" in out2, "count 0: the next start goes back (%s)" % out2)
refused("remaining above initial", ["count", "--remaining", "4", "--initial", "3", "-o", os.path.join(d, "c4.bin")],
        "--remaining <= --initial")
rc, out = run("count-info", w("short.bin", b"\0" * 100))
ok(rc == 1 and "one 512-byte sector" in out, "count-info: neither a sector nor a dump")

shutil.rmtree(d)
print("imx95_mkboot_selftest: %d checks, %d failed" % (checks, fails))
sys.exit(1 if fails else 0)
