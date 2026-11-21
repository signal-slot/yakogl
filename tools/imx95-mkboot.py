#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""imx95-mkboot.py -- the Verdin iMX95's boot image with our image as BL33.

The i.MX 95 boot image ("imx-boot", mkimage's flash_a55 "flash.bin") that
Toradex writes at offset 0 of the eMMC's boot partition is three AHAB
containers (docs/notes/imx95-direct-boot.md, "The chain"):

    0x0000   container 0   ELE firmware (NXP-signed, appended as it is)
    0x4000   container 1   OEI + DDR firmware, System Manager, A55 U-Boot SPL
                           (the Boot ROM and the ELE load and start these)
    1K page  container 2   TF-A BL31 -> 0x8A200000 (entry), U-Boot proper
    after                  -> 0x90200000, OP-TEE -> 0x8C000000 (if present)

U-Boot SPL finds container 2 itself: the end of container 1's images,
rounded up to 1 KiB (u-boot arch/arm/mach-imx/image-container.c,
get_imageset_end). It copies every image of container 2 to its destination,
does not check their hashes (Toradex's SPL has no CONFIG_AHAB_BOOT) and jumps
to image 0's entry, BL31. BL31 enters BL33 at the fixed 0x90200000
(PLAT_NS_IMAGE_OFFSET) in non-secure EL2. This tool rebuilds container 2 only;
containers 0 and 1 are copied byte for byte.

  imx95-mkboot.py info    IMAGE
  imx95-mkboot.py check   IMAGE [--limit BYTES]
  imx95-mkboot.py extract IMAGE DIR
  imx95-mkboot.py build   [--base IMAGE] (--bl33 FILE | --image zephyr.bin
                          [--load ADDR]) -o OUT --limit BYTES
                          [--bl31 FILE] [--tee FILE | --no-tee]

`--bl33 FILE`: FILE takes U-Boot's slot at 0x90200000 (a loader linked
there, such as driver/samples/zephyr/boot_stage2 with its payload).
`--image zephyr.bin`: the image goes into container 2 as a fourth image at its
link address (default 0xd0000000, the Verdin iMX95 board's), and U-Boot's slot
holds a 16-byte trampoline (`ldr x16, #8; br x16; .quad ADDR`): SPL copies
the image there, BL31 enters the trampoline, the trampoline branches to the
image with the registers, the EL and the MMU state BL31 left.
`--base` defaults to $IMX95_BOOT_BASE: the board's own boot image (a dump of
the boot partition, or Toradex's imx-boot), never committed. `--limit` is
the boot partition's size less U-Boot's environment (0x2200 bytes at its
end, CONFIG_ENV_OFFSET=0xFFFFDE00).

Standard library only. The containers are written unsigned (signature block
header only), with each image's hash: for an OEM-open part.
Exit 0: done; 1: a check failed or a build was refused; 2: usage.
"""
import argparse
import hashlib
import os
import struct
import sys

TAGS = (0x87, 0x82)          # container header tag (0x82: an appended V2X/ELE form)
SIG_TAG = 0x90               # signature block header tag
HDR = struct.Struct("<BHBIHBBHH")          # 16 bytes
IMG = struct.Struct("<IIQQII64s32s")       # 128 bytes
SIG = struct.Struct("<BHBHHHHI")           # 16 bytes
HDR_IMG_OFFSET = 0x10
IMG_SIZE = 128
SECTOR = 0x400               # mkimage -soc IMX9 sector size: image offsets and sizes
PAGE = 0x400                 # SPL rounds the image set's end up to 1 KiB

# TF-A (imx-atf lf_v2.10_6.6.52_2.2.x, plat/imx/imx95/include/platform_def.h)
BL31_ADDR = 0x8A200000
BL31_LIMIT = 0x8A240000
BL33_ADDR = 0x90200000       # PLAT_NS_IMAGE_OFFSET: BL33's fixed entry
TEE_ADDR = 0x8C000000        # BL32_BASE, BL32_SIZE 0x02000000
TEE_SIZE = 0x02000000
# U-Boot SPL's heap while it loads container 2 (verdin-imx95_defconfig
# CONFIG_SPL_CUSTOM_SYS_MALLOC_ADDR/_SIZE): the container header is read into
# it. BL31 (SPD=opteed) writes 0 at 0x93200000 and OP-TEE puts its devicetree
# overlay there (BL32_FDT_OVERLAY_ADDR = PLAT_NS_IMAGE_OFFSET + 0x3000000).
SPL_HEAP = (0x93200000, 0x93280000)
DDR = (0x80000000, 0x880000000)
DEFAULT_LOAD = 0xD0000000    # driver/boards/toradex/verdin_imx95 zephyr,sram (PR #9043)
ARM64_MAGIC = 0x644D5241     # "ARM\x64" at 0x38 of an AArch64 Image header
TRAMPOLINE_WORDS = (0x58000050, 0xD61F0200)   # ldr x16, #8 ; br x16

TYPES = {0x3: "exec", 0x4: "data", 0x5: "oei/ddr", 0x6: "ele", 0x7: "prov", 0x8: "dek/fcb",
         0xB: "v2x-pri", 0xC: "v2x-sec", 0xD: "ddr-dummy", 0xE: "v2x-dummy"}
CORES = {0x1: "m33", 0x2: "a55", 0x4: "upower", 0x6: "ele", 0xB: "m7", 0xD: "m33s"}
HASHES = {0: ("sha256", 32), 1: ("sha384", 48), 2: ("sha512", 64)}


class Fail(Exception):
    pass


def align(x, a):
    return (x + a - 1) // a * a


def u(s):
    return int(s, 0)


class Container:
    def __init__(self, blob, off):
        if off + HDR.size > len(blob):
            raise Fail("no container header at 0x%x: the file ends at 0x%x" % (off, len(blob)))
        (self.version, self.length, self.tag, self.flags, self.sw_version, self.fuse_version,
         self.num_images, self.sig_blk_offset, self.reserved) = HDR.unpack_from(blob, off)
        self.off = off
        if self.tag not in TAGS:
            raise Fail("no container header at 0x%x (tag 0x%02x, not 0x87)" % (off, self.tag))
        if self.version not in (0, 2):
            raise Fail("container at 0x%x: version %d, not 0 or 2" % (off, self.version))
        if self.num_images == 0 or self.num_images > 16:
            raise Fail("container at 0x%x: %d images" % (off, self.num_images))
        self.images = []
        for i in range(self.num_images):
            p = off + HDR_IMG_OFFSET + i * IMG_SIZE
            if p + IMG_SIZE > len(blob):
                raise Fail("container at 0x%x: image entry %d runs past the file" % (off, i))
            o, size, dst, entry, flags, meta, digest, iv = IMG.unpack_from(blob, p)
            self.images.append(dict(index=i, offset=o, size=size, dst=dst, entry=entry,
                                    flags=flags, meta=meta, hash=digest, iv=iv))
        self.sig = None
        if self.sig_blk_offset:
            p = off + self.sig_blk_offset
            if p + SIG.size > len(blob):
                raise Fail("container at 0x%x: signature block past the file" % off)
            v, ln, tag, srk, cert, blob_o, sig_o, res = SIG.unpack_from(blob, p)
            self.sig = dict(version=v, length=ln, tag=tag, srk=srk, cert=cert, blob=blob_o,
                            signature=sig_o)

    def end(self):
        """SPL's get_container_size: the furthest byte, from the header."""
        m = self.length
        for im in self.images:
            m = max(m, im["offset"] + im["size"])
        if self.sig:
            m = max(m, self.sig_blk_offset + self.sig["length"])
        return m

    def signed(self):
        s = self.sig
        return bool(s and (s["length"] > SIG.size or s["srk"] or s["cert"] or s["blob"] or s["signature"]))


def img_type(im):
    return im["flags"] & 0xF


def img_core(im):
    return (im["flags"] >> 4) & 0xF


def hash_kind(im):
    return HASHES.get((im["flags"] >> 8) & 0x7)


def digest(data, size, kind):
    """mkimage set_image_hash: the image padded with zeros to its size field."""
    name, n = kind
    h = hashlib.new(name)
    h.update(data[:size].ljust(size, b"\0"))
    return h.digest().ljust(64, b"\0")


def image_bytes(blob, c, im):
    a = c.off + im["offset"]
    return blob[a:a + im["size"]]


def is_v2x(c):
    """SPL's is_v2x_fw_container: version 0, images 0xB on core 9 then 0xC on core 10."""
    if c.version != 0 or not 2 <= c.num_images <= 4:
        return False
    a, b = c.images[0], c.images[1]
    return (img_type(a), img_core(a), img_type(b), img_core(b)) == (0xB, 9, 0xC, 10)


class Layout:
    """The containers of an image set at offset 0, found as U-Boot SPL finds them."""

    def __init__(self, blob):
        self.blob = blob
        c0 = Container(blob, 0)
        self.align = 0x4000 if c0.version == 2 else 0x400
        self.c0 = c0
        o1 = align(c0.length, self.align)
        c1 = Container(blob, o1)
        self.v2x = None
        if is_v2x(c1):
            self.v2x = c1
            c1 = Container(blob, o1 + align(c1.length, self.align))
        self.c1 = c1
        self.c2_off = align(c1.off + c1.end(), PAGE)
        self.c2 = Container(blob, self.c2_off)
        self.end = self.c2_off + self.c2.end()

    def containers(self):
        out = [("0 (ELE)", self.c0)]
        if self.v2x:
            out.append(("V2X", self.v2x))
        out += [("1 (boot)", self.c1), ("2 (SPL loads)", self.c2)]
        return out


def role(im):
    if im["dst"] == BL31_ADDR and im["entry"] == BL31_ADDR:
        return "bl31"
    if im["dst"] == BL33_ADDR:
        return "bl33"
    if im["dst"] == TEE_ADDR:
        return "tee"
    return "image@0x%x" % im["dst"]


def describe(im, blob, c):
    kind = hash_kind(im)
    ok = "?"
    if kind:
        ok = "ok" if digest(image_bytes(blob, c, im), im["size"], kind) == im["hash"] else "MISMATCH"
    return ("  [%d] file 0x%07x size 0x%07x dst 0x%09x entry 0x%09x %-9s core %-6s meta 0x%x %s %s"
            % (im["index"], c.off + im["offset"], im["size"], im["dst"], im["entry"],
               TYPES.get(img_type(im), "0x%x" % img_type(im)),
               CORES.get(img_core(im), "0x%x" % img_core(im)), im["meta"],
               kind[0] if kind else "hash?", ok))


def cmd_info(args):
    blob = read(args.image)
    lay = Layout(blob)
    print("file %s: %d bytes; header alignment 0x%x" % (args.image, len(blob), lay.align))
    for name, c in lay.containers():
        print("container %s @0x%x: version %d tag 0x%02x length 0x%x flags 0x%x sw %d fuse %d "
              "images %d signed %s" % (name, c.off, c.version, c.tag, c.length, c.flags, c.sw_version,
                                       c.fuse_version, c.num_images, "yes" if c.signed() else "no"))
        for im in c.images:
            print(describe(im, blob, c) + ("  (%s)" % role(im) if c is lay.c2 else ""))
    print("container 2 at 0x%x (SPL: end of container 1's images, rounded to 1 KiB); "
          "image set ends at 0x%x" % (lay.c2_off, lay.end))
    return 0


def problems(blob, limit=None):
    """Every rule a direct boot image must keep; [] when it keeps them all."""
    lay = Layout(blob)
    c2 = lay.c2
    out = []
    ims = c2.images
    if ims[0]["dst"] != BL31_ADDR or ims[0]["entry"] != BL31_ADDR:
        out.append("container 2 image 0 is not BL31 at 0x%x (SPL jumps to image 0's entry)" % BL31_ADDR)
    n33 = len([im for im in ims if role(im) == "bl33"])
    if n33 != 1:
        out.append("%d images at BL33's entry 0x%x, not 1 (TF-A enters it there)" % (n33, BL33_ADDR))
    spans = []
    for im in ims:
        name = role(im)
        lo, hi = im["dst"], im["dst"] + im["size"]
        if im["offset"] % SECTOR or im["size"] % SECTOR:
            out.append("%s: offset 0x%x or size 0x%x not a multiple of 0x%x" % (name, im["offset"], im["size"], SECTOR))
        if im["offset"] < c2.length:
            out.append("%s: data at 0x%x inside the header (length 0x%x)" % (name, im["offset"], c2.length))
        if not (DDR[0] <= lo and hi <= DDR[1]):
            out.append("%s: 0x%x..0x%x is not in DDR" % (name, lo, hi))
        if lo < SPL_HEAP[1] and SPL_HEAP[0] < hi:
            out.append("%s: 0x%x..0x%x overlaps SPL's heap and OP-TEE's overlay at 0x%x" % (name, lo, hi, SPL_HEAP[0]))
        if name == "bl31" and hi > BL31_LIMIT:
            out.append("bl31: ends at 0x%x, past BL31_LIMIT 0x%x" % (hi, BL31_LIMIT))
        if name == "tee" and hi > TEE_ADDR + TEE_SIZE:
            out.append("tee: ends at 0x%x, past BL32_BASE + BL32_SIZE" % hi)
        if name.startswith("image@") and lo < BL33_ADDR:
            out.append("%s: below 0x%x, where TF-A refuses a non-secure entry" % (name, BL33_ADDR))
        kind = hash_kind(im)
        if not kind:
            out.append("%s: unknown hash type in flags 0x%x" % (name, im["flags"]))
        elif digest(image_bytes(blob, c2, im), im["size"], kind) != im["hash"]:
            out.append("%s: %s of the image does not match the header" % (name, kind[0]))
        if c2.off + im["offset"] + im["size"] > len(blob):
            out.append("%s: runs past the end of the file" % name)
        spans.append((lo, hi, name))
    spans.sort()
    for (a_lo, a_hi, a), (b_lo, b_hi, b) in zip(spans, spans[1:]):
        if b_lo < a_hi:
            out.append("%s and %s overlap in memory" % (a, b))
    files = sorted((im["offset"], im["offset"] + im["size"], role(im)) for im in ims)
    for (a_lo, a_hi, a), (b_lo, b_hi, b) in zip(files, files[1:]):
        if b_lo < a_hi:
            out.append("%s and %s overlap in the file" % (a, b))
    for im in ims:
        if role(im) == "bl33" and im["size"] >= 16:
            w0, w1, target = struct.unpack_from("<IIQ", image_bytes(blob, c2, im))
            if (w0, w1) == TRAMPOLINE_WORDS:
                hit = [x for x in ims if x["dst"] == target]
                if not hit:
                    out.append("bl33 trampoline branches to 0x%x, where no image is loaded" % target)
    if c2.signed():
        out.append("container 2 has a signature: this tool's containers are unsigned")
    if limit is not None and lay.end > limit:
        out.append("the image set ends at 0x%x, past the limit 0x%x" % (lay.end, limit))
    return out


def cmd_check(args):
    blob = read(args.image)
    found = problems(blob, args.limit)
    for p in found:
        print("check: FAIL %s" % p)
    if found:
        return 1
    lay = Layout(blob)
    print("check: ok: container 2 at 0x%x, %d images (%s), ends at 0x%x%s"
          % (lay.c2_off, lay.c2.num_images, ", ".join(role(im) for im in lay.c2.images), lay.end,
             "" if args.limit is None else ", limit 0x%x" % args.limit))
    return 0


def cmd_extract(args):
    blob = read(args.image)
    lay = Layout(blob)
    os.makedirs(args.dir, exist_ok=True)
    for im in lay.c2.images:
        name = role(im).replace("@", "-") + ".bin"
        with open(os.path.join(args.dir, name), "wb") as f:
            f.write(image_bytes(blob, lay.c2, im))
        print("%s: 0x%x bytes (padded to 0x%x), dst 0x%x" % (name, im["size"], SECTOR, im["dst"]))
    return 0


def container_bytes(c2, images):
    """A container 2 in mkimage's form: the header, the image entries, a
    signature block header with no signature, the images from c2's first data
    offset, each at a multiple of 0x400 and padded to it."""
    n = len(images)
    sig_off = HDR_IMG_OFFSET + n * IMG_SIZE
    length = sig_off + SIG.size
    start = min(im["offset"] for im in c2.images)
    start = max(start, align(length, SECTOR))
    out = bytearray(start)
    entries = []
    off = start
    for im in images:
        data = im["data"]
        size = align(len(data), SECTOR)
        kind = HASHES[(im["flags"] >> 8) & 0x7]
        entries.append(IMG.pack(off, size, im["dst"], im["entry"], im["flags"], im["meta"],
                                digest(data, size, kind), b"\0" * 32))
        out += data.ljust(size, b"\0")
        off += size
    head = HDR.pack(c2.version, length, 0x87, c2.flags, c2.sw_version, c2.fuse_version, n, sig_off, 0)
    head += b"".join(entries) + SIG.pack(0, SIG.size, SIG_TAG, 0, 0, 0, 0, 0)
    out[0:len(head)] = head
    return bytes(out)


def aarch64_image(data, load):
    if len(data) < 64 or struct.unpack_from("<I", data, 0x38)[0] != ARM64_MAGIC:
        raise Fail("--image: no AArch64 Image header (magic 'ARM\\x64' at 0x38)")
    text_offset = struct.unpack_from("<Q", data, 8)[0]
    if text_offset:
        raise Fail("--image: text_offset 0x%x, not 0: the image is linked at its load address" % text_offset)
    if load < BL33_ADDR or load % 0x1000:
        raise Fail("--load 0x%x: below 0x%x or not 4 KiB aligned" % (load, BL33_ADDR))


def cmd_build(args):
    base_path = args.base or os.environ.get("IMX95_BOOT_BASE")
    if not base_path:
        raise Fail("no base image: --base or IMX95_BOOT_BASE (the board's boot partition dump)")
    if args.no_tee and not args.bl31:
        raise Fail("--no-tee needs --bl31: a BL31 built with SPD=opteed enters OP-TEE before BL33")
    base = read(base_path)
    lay = Layout(base)
    c2 = lay.c2
    if c2.signed():
        raise Fail("the base's container 2 is signed; this tool writes unsigned containers (OEM-open parts)")
    bad = problems(base)
    if bad:
        raise Fail("the base does not pass the checks: " + "; ".join(bad))
    old = {role(im): im for im in c2.images}
    images = []
    for im in c2.images:
        r = role(im)
        new = dict(im, data=bytes(image_bytes(base, c2, im)))
        if r == "bl31" and args.bl31:
            new["data"] = read(args.bl31)
        elif r == "tee":
            if args.no_tee:
                continue
            if args.tee:
                new["data"] = read(args.tee)
        elif r == "bl33":
            if args.bl33:
                new["data"] = read(args.bl33)
            else:
                new["data"] = struct.pack("<IIQ", TRAMPOLINE_WORDS[0], TRAMPOLINE_WORDS[1], args.load)
        elif r.startswith("image@"):
            raise Fail("the base's container 2 already holds %s: start from Toradex's image" % r)
        images.append(new)
    if args.tee and "tee" not in old:
        raise Fail("--tee: the base has no OP-TEE image; its BL31 was built without one")
    if args.image:
        data = read(args.image)
        aarch64_image(data, args.load)
        slot = old["bl33"]
        images.append(dict(dst=args.load, entry=args.load, flags=slot["flags"], meta=slot["meta"], data=data))
    out = base[:lay.c2_off] + container_bytes(c2, images)
    found = problems(out, args.limit)
    if found:
        raise Fail("the new image does not pass the checks: " + "; ".join(found))
    if hashlib.sha256(out[:lay.c2_off]).digest() != hashlib.sha256(base[:lay.c2_off]).digest():
        raise Fail("containers 0 and 1 changed")
    with open(args.output, "wb") as f:
        f.write(out)
    new = Layout(out)
    print("build: %s: %d bytes, sha256 %s" % (args.output, len(out), hashlib.sha256(out).hexdigest()))
    print("build: containers 0 and 1 copied (0x%x bytes, sha256 %s)"
          % (lay.c2_off, hashlib.sha256(out[:lay.c2_off]).hexdigest()))
    for im in new.c2.images:
        print("build: %-16s dst 0x%09x size 0x%07x" % (role(im), im["dst"], im["size"]))
    print("build: the image set ends at 0x%x of the limit 0x%x" % (new.end, args.limit))
    return 0


def read(path):
    with open(path, "rb") as f:
        return f.read()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("info", help="print the containers of a boot image")
    p.add_argument("image")
    p = sub.add_parser("check", help="check a boot image field by field")
    p.add_argument("image")
    p.add_argument("--limit", type=u)
    p = sub.add_parser("extract", help="write container 2's images to a directory")
    p.add_argument("image")
    p.add_argument("dir")
    p = sub.add_parser("build", help="rebuild container 2 with our image as BL33")
    p.add_argument("--base")
    g = p.add_mutually_exclusive_group(required=True)
    g.add_argument("--bl33", help="a binary for U-Boot's slot at 0x90200000")
    g.add_argument("--image", help="zephyr.bin, loaded at --load behind a trampoline")
    p.add_argument("--load", type=u, default=DEFAULT_LOAD)
    p.add_argument("--bl31")
    t = p.add_mutually_exclusive_group()
    t.add_argument("--tee")
    t.add_argument("--no-tee", action="store_true")
    p.add_argument("-o", "--output", required=True)
    p.add_argument("--limit", type=u, required=True)
    args = ap.parse_args()
    try:
        return {"info": cmd_info, "check": cmd_check, "extract": cmd_extract, "build": cmd_build}[args.cmd](args)
    except Fail as e:
        print("imx95-mkboot.py: %s" % e, file=sys.stderr)
        return 1
    except OSError as e:
        print("imx95-mkboot.py: %s" % e, file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
