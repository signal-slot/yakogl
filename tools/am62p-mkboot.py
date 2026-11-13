#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""am62p-mkboot.py -- a Verdin AM62P u-boot.img whose payload is a Zephyr image.

The TI K3 boot chain on the Verdin AM62P (Toradex's prebuilt Torizon
bootloader, docs/direct-boot.md):

    Boot ROM -> tiboot3.bin  R5 SPL + TIFS + board config (TI-signed), on the
                             WKUP R5F: DDR, then loads tispl.bin
             -> tispl.bin    a FIT: TF-A BL31 (0x80000000), OP-TEE (0x9e800000),
                             the DM firmware (the R5 becomes the DM) and the
                             A53 SPL (0x80080000 = TF-A's PRELOADED_BL33_BASE)
             -> u-boot.img   a FIT the A53 SPL loads (spl_load_simple_fit):
                               uboot  U-Boot proper, load 0x80800000, os u-boot
                               fdt-0  U-Boot's devicetree
                             conf-0: firmware = loadables = uboot, fdt = fdt-0

This tool writes a new u-boot.img with the same nodes, where "uboot" carries
zephyr.bin with load = entry = the Zephyr link address (0x82000000 by
default). The A53 SPL then loads Zephyr instead of U-Boot proper and jumps
to it in EL2 with the MMU and data cache off. tiboot3.bin and tispl.bin are
not touched. fdt-0 is copied byte for byte (with its TI certificate): the SPL
appends it after the firmware, as it does for U-Boot.

The new FIT keeps its data outside the FDT structure (data-position), so the
SPL reads only a small header and then each payload straight to its load
address. Toradex's prebuilt embeds the data instead; with a 16 MB payload the
SPL would read the whole FIT into one buffer first.

  am62p-mkboot.py info  FIT
  am62p-mkboot.py build --base u-boot.img --zephyr zephyr.bin -o OUT
                        [--load ADDR] [--description TEXT]
  am62p-mkboot.py check --base u-boot.img --zephyr zephyr.bin OUT [--load ADDR]
  am62p-mkboot.py bootpart --tiboot3 tiboot3.bin --tispl tispl.bin
                        --uboot u-boot-zephyr.img --size BYTES -o OUT
  am62p-mkboot.py stage2img --zephyr zephyr.bin -o OUT [--load ADDR]
                        [--compress deflate|none] [--name TEXT]
                        [--segment FILE@ADDR ...] [--staging ADDR]
                        [--split-zeros MIN] [--clear-after BYTES]
                        [--sector N --end-sector N]
  am62p-mkboot.py stage2check --zephyr zephyr.bin [--segment FILE@ADDR ...] IMG
  am62p-mkboot.py stage2-vectors DIR

`bootpart` lays out a whole eMMC boot partition (tiboot3 at 0, tispl at
0x80000, u-boot.img at 0x280000: Toradex's dd seek=0/1024/5120 sectors) and
refuses a u-boot.img that runs into U-Boot's environment (the last 0x2200
bytes, CONFIG_ENV_OFFSET = -0x2200) or past the partition.

An image too large for the boot partition's u-boot.img slot (1.49 MiB in a
4 MiB boot partition) goes in two parts (docs/direct-boot.md, "The second
stage"): `build --load 0x80800000` with the second-stage loader
(driver/samples/zephyr/boot_stage2) as the u-boot.img payload, and
`stage2img`, which writes the real image for the eMMC USER area: one
512-byte YKSTAGE2 header sector (layout in
driver/samples/zephyr/boot_stage2/loader/src/lib.rs) and the payload,
raw-deflate compressed by default, for sector STAGE2_SECTOR (64) onwards,
ending before STAGE2_END_SECTOR (8192, the first partition). `stage2check`
re-reads one the way the loader will; `stage2-vectors` writes the images the
loader crate's host test decodes (BS2_VECTORS), so the Python writer and the
Rust reader are checked against each other.

With `--segment FILE@ADDR` (repeatable) `stage2img` writes a version-2
header instead: the zephyr.bin is segment 0 (at --load, holding the entry)
and each FILE is placed at its ADDR, the way kexec_zephyr's FILE@ADDR
segments are (the AM62P showcase: the Supervisor, r5_ddr.bin@0x90000000 and
r5_tcm.bin@0x90c00000). Every segment must lie in the loader's region
(0x82000000 .. the GPU carveout), clear of the others (segment 0 up to its
arm64 image_size, .bss included) and clear of the loader's staging buffer
(--staging, CONFIG_BOOT_STAGE2_STAGING, for as many bytes as the sector
range holds). Without --segment (and without the two options below) the
version-1 header is written, which loaders built before version 2 also read.

`--split-zeros MIN` and `--clear-after BYTES` keep zeros out of the payload
(both write the version-2 header; a loader from before 2026-11-13 refuses
the result). The loader clears a segment's `zero bytes`, the RAM right after
its image bytes, with the MMU and caches on while the eMMC transfers run.
With --split-zeros the zephyr.bin is cut at every run of at least MIN zero
bytes: the run, shrunk inward to 4 KiB address boundaries, is not stored,
and what follows it becomes another segment, a continuation part of the
image inside segment 0's footprint (the header's `image parts` counts
segment 0 and these). Each part's zero bytes are the run after it. When the
runs give more parts than the 8 segments hold, the longest are cut and the
tool says how many were left in. --clear-after is the zero bytes after the
image's last byte, at `load + len(zephyr.bin)` exactly: give the distance
from there to the end of the image's .bss, and the image may skip its own
.bss clear (Zephyr's CONFIG_SKIP_BSS_CLEAR) as long as it is only ever
started by this loader from such a payload. It must fit segment 0's
footprint (the arm64 image_size). `stage2check` reads the parts from the
header, puts the image together again (parts and zero runs) and compares it
with --zephyr.

Standard library only. No signing: the Zephyr payload has no TI x509
certificate, so the A53 SPL on an HS-FS AM62P prints "Did not detect image
signing certificate. Skipping authentication" and loads it. An HS-SE device
would refuse it.
"""
import argparse
import hashlib
import struct
import sys
import zlib

FDT_MAGIC = 0xD00DFEED
FDT_BEGIN_NODE, FDT_END_NODE, FDT_PROP, FDT_NOP, FDT_END = 1, 2, 3, 4, 9

ZEPHYR_LOAD = 0x82000000      # the a53 board's zephyr,sram (U-Boot "go" and kexec use it too)
GPU_CARVEOUT = 0x97800000     # the first byte this image must not reach
SPL_STACK_TOP = 0x82000000    # CONFIG_SPL_STACK_R_ADDR (verdin-am62p_a53_defconfig): stack and
                              # the simple malloc pool grow down from here, so a load at or
                              # above it does not overwrite the running SPL
DATA_ALIGN = 0x1000
UBOOT_PROPER_LOAD = 0x80800000  # U-Boot proper's load address in Toradex's u-boot.img: the
                                # second-stage loader goes there (driver/samples/zephyr/boot_stage2)
SPL_BSS = 0x80c80000          # CONFIG_SPL_BSS_START_ADDR (verdin-am62p_a53_defconfig): a payload
                              # below the stack top, and the devicetree the SPL appends, end here
FDT_ROOM = 0x40000            # room left for that devicetree (fdt-0 with its certificate: 85 KB)

# The second stage's image in the eMMC user area (boot_stage2 Kconfig defaults)
STAGE2_MAGIC = b"YKSTAGE2"
STAGE2_VERSION = 1            # one image
STAGE2_VERSION_SEGMENTS = 2   # the segment table (loader/src/lib.rs)
STAGE2_MAX_SEGMENTS = 8
STAGE2_TABLE = 96             # the segment table's offset in the header, 40 bytes an entry
STAGE2_ENTRY_BYTES = 40
STAGE2_SECTOR = 64            # header sector: clear of the DOS table (0) and a GPT's sectors 1..33
STAGE2_END_SECTOR = 8192      # Torizon's mmcblk0p1 starts here (sfdisk -d, 2026-10-02)
STAGE2_STAGING = 0x90000000   # where the loader reads a compressed payload; bounds the image
COMP_NONE, COMP_DEFLATE = 0, 1

# eMMC boot partition layout (Toradex image.json dd seek= in 512-byte sectors;
# CONFIG_SYS_MMCSD_RAW_MODE_U_BOOT_SECTOR 0x400 in the R5 SPL, 0x1400 in the A53 SPL)
TIBOOT3_OFF = 0x0
TISPL_OFF = 0x400 * 512       # 0x80000
UBOOT_OFF = 0x1400 * 512      # 0x280000
ENV_FROM_END = 0x2200         # CONFIG_ENV_OFFSET=0xFFFFDE00, ENV_SIZE 0x2000, in boot0


# --- a minimal flattened-devicetree reader and writer (as tools/x2-mkflash.py) --

class Node:
    def __init__(self, name):
        self.name = name
        self.props = []
        self.children = []

    def prop(self, name):
        for n, v in self.props:
            if n == name:
                return v
        return None

    def set(self, name, value):
        for i, (n, _) in enumerate(self.props):
            if n == name:
                self.props[i] = (name, value)
                return
        self.props.append((name, value))

    def drop(self, *names):
        self.props = [(n, v) for n, v in self.props if n not in names]

    def child(self, name):
        for c in self.children:
            if c.name == name:
                return c
        return None


def fdt_parse(blob):
    magic, total, off_struct, off_strings = struct.unpack_from(">IIII", blob, 0)
    if magic != FDT_MAGIC:
        raise ValueError("not a flattened devicetree (magic 0x%08x)" % magic)

    def string_at(off):
        end = blob.index(b"\0", off_strings + off)
        return blob[off_strings + off:end].decode()

    pos, stack, root = off_struct, [], None
    while True:
        (tok,) = struct.unpack_from(">I", blob, pos)
        pos += 4
        if tok == FDT_BEGIN_NODE:
            end = blob.index(b"\0", pos)
            node = Node(blob[pos:end].decode())
            pos = (end + 4) & ~3
            if stack:
                stack[-1].children.append(node)
            else:
                root = node
            stack.append(node)
        elif tok == FDT_END_NODE:
            stack.pop()
        elif tok == FDT_PROP:
            length, nameoff = struct.unpack_from(">II", blob, pos)
            pos += 8
            stack[-1].props.append((string_at(nameoff), bytes(blob[pos:pos + length])))
            pos = (pos + length + 3) & ~3
        elif tok == FDT_NOP:
            continue
        elif tok == FDT_END:
            return root, total
        else:
            raise ValueError("bad FDT token %d at 0x%x" % (tok, pos - 4))


def fdt_build(root):
    strings, offsets, st = bytearray(), {}, bytearray()

    def name_off(name):
        if name not in offsets:
            offsets[name] = len(strings)
            strings.extend(name.encode() + b"\0")
        return offsets[name]

    def pad4():
        while len(st) % 4:
            st.append(0)

    def emit(node):
        st.extend(struct.pack(">I", FDT_BEGIN_NODE))
        st.extend(node.name.encode() + b"\0")
        pad4()
        for n, v in node.props:
            st.extend(struct.pack(">III", FDT_PROP, len(v), name_off(n)))
            st.extend(v)
            pad4()
        for c in node.children:
            emit(c)
        st.extend(struct.pack(">I", FDT_END_NODE))

    emit(root)
    st.extend(struct.pack(">I", FDT_END))
    off_rsv = 40
    off_struct = off_rsv + 16
    off_strings = off_struct + len(st)
    total = off_strings + len(strings)
    hdr = struct.pack(">10I", FDT_MAGIC, total, off_struct, off_strings,
                      off_rsv, 17, 16, 0, len(strings), len(st))
    return hdr + bytes(16) + bytes(st) + bytes(strings)


def u32(v):
    return struct.pack(">I", v)


def get_u32(v):
    return struct.unpack(">I", v)[0] if v is not None and len(v) == 4 else None


def get_str(v):
    return v.rstrip(b"\0").decode(errors="replace").replace("\0", ", ") if v is not None else None


def strlist(v):
    return [s.decode() for s in v.rstrip(b"\0").split(b"\0")] if v else []


# --- the FIT ---------------------------------------------------------------------

def ti_cert_len(data):
    """Length of a TI x509 certificate at the start of a payload, or 0 (security.c's
    "primitive" detection: two DER SEQUENCEs with 2-byte lengths)."""
    if len(data) >= 6 and data[0] == 0x30 and data[1] == 0x82 and data[4] == 0x30 and data[5] == 0x82:
        return struct.unpack_from(">H", data, 2)[0] + 4
    return 0


def parse_fit(blob):
    """Return (root, header size, {image name: (node, payload, data-position or None)})."""
    root, total = fdt_parse(blob)
    images = root.child("images")
    if images is None or root.child("configurations") is None:
        raise ValueError("not a FIT (no /images or /configurations)")
    out = {}
    for node in images.children:
        if node.prop("data") is not None:
            out[node.name] = (node, node.prop("data"), None)
            continue
        size = get_u32(node.prop("data-size"))
        pos = get_u32(node.prop("data-position"))
        off = get_u32(node.prop("data-offset"))
        if pos is None and off is not None:
            pos = ((total + 3) & ~3) + off
        if pos is None or size is None:
            raise ValueError("%s: no data, data-position or data-offset" % node.name)
        payload = bytes(blob[pos:pos + size])
        if len(payload) != size:
            raise ValueError("%s: data runs past the end of the file" % node.name)
        out[node.name] = (node, payload, pos)
    return root, total, out


def default_conf(root):
    confs = root.child("configurations")
    name = get_str(confs.prop("default"))
    conf = confs.child(name)
    if conf is None:
        raise ValueError("default configuration %r missing" % name)
    return conf


def describe(blob, label):
    root, total, images = parse_fit(blob)
    print("%s: %d bytes (0x%x), sha256 %s" % (label, len(blob), len(blob), hashlib.sha256(blob).hexdigest()))
    print("  FIT header %d bytes, \"%s\", data %s" % (
        total, get_str(root.prop("description")),
        "embedded" if all(p is None for _, _, p in images.values()) else "external (data-position)"))
    for name, (node, payload, pos) in images.items():
        load, entry = get_u32(node.prop("load")), get_u32(node.prop("entry"))
        cert = ti_cert_len(payload)
        h = node.child("hash")
        hv = ""
        if h is not None and get_str(h.prop("algo")) == "crc32":
            ok = get_u32(h.prop("value")) == zlib.crc32(payload) & 0xFFFFFFFF
            hv = " crc32 %s" % ("ok" if ok else "MISMATCH")
        print("    %-12s %-34s type %-10s os %-12s %s size 0x%07x load %-10s entry %-10s cert %s%s" % (
            name, '"%s"' % get_str(node.prop("description")), get_str(node.prop("type")),
            get_str(node.prop("os")) or "-",
            "pos 0x%06x" % pos if pos is not None else "embedded  ", len(payload),
            "0x%x" % load if load is not None else "-", "0x%x" % entry if entry is not None else "-",
            "%d bytes" % cert if cert else "none", hv))
        print("    %-12s sha256 %s" % ("", hashlib.sha256(payload).hexdigest()))
    confs = root.child("configurations")
    print("  configurations: default \"%s\"" % get_str(confs.prop("default")))
    for c in confs.children:
        print("    %s: %s" % (c.name, "; ".join("%s=%s" % (n, get_str(v)) for n, v in c.props)))
    return root, images


def check_zephyr(zephyr, load):
    if zephyr[:4] == b"\x7fELF":
        raise ValueError("--zephyr is an ELF file; give zephyr.bin")
    if len(zephyr) < 64 or struct.unpack_from("<I", zephyr, 0x38)[0] != 0x644D5241:
        raise ValueError("no arm64 Image header (\"ARM\\x64\" at 0x38) in --zephyr: "
                         "build with CONFIG_AARCH64_IMAGE_HEADER=y")
    image_size = struct.unpack_from("<Q", zephyr, 16)[0]
    footprint = max(image_size, len(zephyr))
    if load < 0x80000000:
        # Not the AM62P's DDR (it starts at 0x80000000): the i.MX8MP's map,
        # whose loader's Kconfig and BUILD_ASSERTs check the addresses.
        return image_size
    if load < SPL_STACK_TOP:
        # Only U-Boot proper's own slot, for a small image (the second-stage
        # loader): the file and the devicetree the SPL appends end below the
        # SPL's BSS, and the image's RAM ends below 0x82000000, where the
        # image it loads goes.
        if load != UBOOT_PROPER_LOAD:
            raise ValueError("load 0x%x is below 0x%x, where the A53 SPL's stack and malloc pool are "
                             "(only 0x%x, U-Boot proper's slot, is allowed below it)"
                             % (load, SPL_STACK_TOP, UBOOT_PROPER_LOAD))
        if load + len(zephyr) + FDT_ROOM > SPL_BSS:
            raise ValueError("a %d-byte image at 0x%x and the SPL's devicetree run into the SPL's BSS at "
                             "0x%x" % (len(zephyr), load, SPL_BSS))
        if load + footprint > SPL_STACK_TOP:
            raise ValueError("0x%x + 0x%x (the header's image_size) runs past 0x%x" % (load, footprint, SPL_STACK_TOP))
        return image_size
    if load + footprint > GPU_CARVEOUT:
        raise ValueError("0x%x + 0x%x (the header's image_size, .bss included) runs into the GPU "
                         "carveout at 0x%x" % (load, footprint, GPU_CARVEOUT))
    return image_size


def build(base, zephyr, load, description):
    """zephyr None: the control image, U-Boot proper repacked unchanged with
    external data (if it boots Torizon, the packing is right)."""
    root, _, images = parse_fit(base)
    conf = default_conf(root)
    fw = get_str(conf.prop("firmware"))
    if fw not in images:
        raise ValueError("the default configuration's firmware %r is not an image" % fw)
    if get_str(images[fw][0].prop("os")) != "u-boot":
        raise ValueError("%s is os %r, not u-boot: not the u-boot.img this tool knows"
                         % (fw, get_str(images[fw][0].prop("os"))))
    order = [n.name for n in root.child("images").children]
    payloads = {name: images[name][1] for name in order}

    if zephyr is not None:
        check_zephyr(zephyr, load)
        # SPL appends the devicetree at ALIGN(load + size, 8); pad to 16 so it
        # starts right after the payload.
        payload = zephyr + bytes((-len(zephyr)) % 16)
        fw_node = images[fw][0]
        fw_node.set("description", description.encode() + b"\0")
        fw_node.set("load", u32(load))
        fw_node.set("entry", u32(load))
        h = fw_node.child("hash")
        if h is not None:
            if get_str(h.prop("algo")) != "crc32":
                raise ValueError("%s/hash is %s; only crc32 is recomputed here" % (fw, get_str(h.prop("algo"))))
            h.set("value", u32(zlib.crc32(payload) & 0xFFFFFFFF))
        payloads[fw] = payload
    for name in order:
        node = images[name][0]
        node.drop("data", "data-offset")
        node.set("data-size", u32(len(payloads[name])))
        node.set("data-position", u32(0))
    hdr = fdt_build(root)
    start = (len(hdr) + DATA_ALIGN - 1) & ~(DATA_ALIGN - 1)
    pos = start
    for name in order:
        images[name][0].set("data-position", u32(pos))
        pos = (pos + len(payloads[name]) + DATA_ALIGN - 1) & ~(DATA_ALIGN - 1)
    hdr2 = fdt_build(root)
    assert len(hdr2) == len(hdr)            # only u32 values changed
    out = bytearray(hdr2) + bytearray(start - len(hdr2))
    for name in order:
        p = get_u32(images[name][0].prop("data-position"))
        out.extend(bytes(p - len(out)))
        out.extend(payloads[name])
    return bytes(out)


def check(base, zephyr, out, load):
    """Re-parse OUT the way the A53 SPL will and compare it with its inputs."""
    broot, _, bimages = parse_fit(base)
    root, total, images = parse_fit(out)
    conf, bconf = default_conf(root), default_conf(broot)
    fw = get_str(conf.prop("firmware"))
    errors = []
    if [(n, v) for n, v in conf.props] != [(n, v) for n, v in bconf.props]:
        errors.append("the default configuration differs from the base's")
    if sorted(images) != sorted(bimages):
        errors.append("image nodes %s, base has %s" % (sorted(images), sorted(bimages)))
    node, payload, pos = images[fw]
    if payload[:len(zephyr)] != zephyr or any(payload[len(zephyr):]):
        errors.append("%s payload is not zephyr.bin (+ zero padding)" % fw)
    if get_u32(node.prop("load")) != load or get_u32(node.prop("entry")) != load:
        errors.append("%s load/entry are not 0x%x" % (fw, load))
    if ti_cert_len(payload):
        errors.append("%s payload starts like a TI certificate" % fw)
    if fw not in strlist(conf.prop("loadables")):
        errors.append("%s is not among the loadables" % fw)
    h = node.child("hash")
    if h is not None and get_u32(h.prop("value")) != zlib.crc32(payload) & 0xFFFFFFFF:
        errors.append("%s crc32 does not match its payload" % fw)
    for name, (n, p, ps) in images.items():
        if ps is None:
            errors.append("%s has embedded data" % name)
        elif ps % 512:
            errors.append("%s data-position 0x%x is not sector aligned" % (name, ps))
        elif ps < total:
            errors.append("%s data overlaps the FIT header" % name)
        if name != fw:
            if p != bimages[name][1]:
                errors.append("%s payload differs from the base's" % name)
            bn = bimages[name][0]
            keep = lambda nd: [(k, v) for k, v in nd.props if k not in ("data", "data-size", "data-position", "data-offset")]
            if keep(n) != keep(bn):
                errors.append("%s properties differ from the base's" % name)
    fdt_name = get_str(conf.prop("fdt"))
    if fdt_name and ti_cert_len(images[fdt_name][1]) == 0:
        errors.append("%s lost its TI certificate" % fdt_name)
    image_size = check_zephyr(zephyr, load)
    return errors, image_size


def bootpart(tiboot3, tispl, uboot, size):
    for name, blob, off, nxt in (("tiboot3.bin", tiboot3, TIBOOT3_OFF, TISPL_OFF),
                                 ("tispl.bin", tispl, TISPL_OFF, UBOOT_OFF)):
        if off + len(blob) > nxt:
            raise ValueError("%s is %d bytes, over its slot (0x%x..0x%x)" % (name, len(blob), off, nxt))
    limit = size - ENV_FROM_END
    end = UBOOT_OFF + len(uboot)
    if end > limit:
        raise ValueError("u-boot.img ends at 0x%x, past 0x%x (a %d-byte boot partition less U-Boot's "
                         "environment, 0x%x bytes from its end): %d bytes too many; a boot partition "
                         "of at least %d bytes (0x%x) would hold it"
                         % (end, limit, size, ENV_FROM_END, end - limit, end + ENV_FROM_END, end + ENV_FROM_END))
    # whole 512-byte sectors, so "mmc write" from U-Boot writes exactly this
    img = bytearray((end + 511) & ~511)
    img[TIBOOT3_OFF:TIBOOT3_OFF + len(tiboot3)] = tiboot3
    img[TISPL_OFF:TISPL_OFF + len(tispl)] = tispl
    img[UBOOT_OFF:end] = uboot
    return bytes(img)


# --- the second stage's image in the eMMC user area --------------------------------

def stage2_header(load, entry, stored, image, compression, name):
    """The 512-byte YKSTAGE2 header (boot_stage2/loader/src/lib.rs)."""
    nm = name.encode()[:32]
    h = bytearray(512)
    struct.pack_into("<8sIIII", h, 0, STAGE2_MAGIC, STAGE2_VERSION, 512, compression, 0)
    struct.pack_into("<QQQQII", h, 24, load, entry, len(stored), len(image),
                     zlib.crc32(stored) & 0xFFFFFFFF, zlib.crc32(image) & 0xFFFFFFFF)
    h[64:64 + len(nm)] = nm
    struct.pack_into("<I", h, 508, zlib.crc32(bytes(h[:508])) & 0xFFFFFFFF)
    return bytes(h)


def stage2_capacity(sector=STAGE2_SECTOR, end_sector=STAGE2_END_SECTOR):
    """Bytes of payload the user-area range holds after the header sector."""
    return (end_sector - sector - 1) * 512


def deflate_raw(data):
    c = zlib.compressobj(9, zlib.DEFLATED, -15, 9)
    return c.compress(data) + c.flush()


def stage2img(zephyr, load, compression, name, sector=STAGE2_SECTOR, end_sector=STAGE2_END_SECTOR):
    """Header + payload, padded to whole sectors, for user-area sector `sector`."""
    # The AM62P's DDR starts at 0x80000000; an image there must clear the A53
    # SPL's stack and the loader (SPL_STACK_TOP) and stay below the loader's
    # staging buffer. Another SoC's DDR (the i.MX8MP's at 0x40000000, loader
    # at 0x40200000, image at 0x48000000) has its own map, checked by the
    # loader's Kconfig and BUILD_ASSERTs.
    if 0x80000000 <= load < SPL_STACK_TOP:
        raise ValueError("the second stage loads the image at 0x%x or above (got 0x%x)" % (SPL_STACK_TOP, load))
    check_zephyr(zephyr, load)
    if load >= 0x80000000 and len(zephyr) > STAGE2_STAGING - load:
        raise ValueError("a %d-byte image at 0x%x reaches the loader's staging buffer at 0x%x"
                         % (len(zephyr), load, STAGE2_STAGING))
    stored = deflate_raw(zephyr) if compression == COMP_DEFLATE else zephyr
    cap = stage2_capacity(sector, end_sector)
    if len(stored) > cap:
        raise ValueError("the payload is %d bytes (%s), over the %d bytes of user-area sectors %d..%d after "
                         "the header: %d bytes too many"
                         % (len(stored), "deflate" if compression == COMP_DEFLATE else "uncompressed", cap,
                            sector + 1, end_sector - 1, len(stored) - cap))
    out = stage2_header(load, load, stored, zephyr, compression, name) + stored
    return out + bytes((-len(out)) % 512)


def stage2_v2_raw(segs, entry, name, total=None, parts=0):
    """A version-2 header + stored bytes, no checks (the vectors use it for
    malformed payloads). segs: dicts with load, image (bytes), stored (bytes),
    comp, and optionally footprint, offset, scrc, icrc overriding the
    computed values, and zero (the zero bytes, 0 by default). parts is the
    header's image parts field as written (0: the field before parts, one)."""
    body = bytearray()
    table = []
    for s in segs:
        off = s.get("offset", len(body))
        if off > len(body):
            body.extend(bytes(off - len(body)))
        stored = s["stored"]
        end = off + len(stored)
        if end > len(body):
            body.extend(bytes(end - len(body)))
        body[off:end] = stored
        body.extend(bytes((-len(body)) % 512))
        table.append((s["load"], len(s["image"]), s.get("footprint", len(s["image"])), off, len(stored), s["comp"],
                      s.get("scrc", zlib.crc32(stored) & 0xFFFFFFFF), s.get("icrc", zlib.crc32(s["image"]) & 0xFFFFFFFF),
                      s.get("zero", 0)))
    h = bytearray(512)
    struct.pack_into("<8sIIIIQQ", h, 0, STAGE2_MAGIC, STAGE2_VERSION_SEGMENTS, 512, len(segs), parts, entry,
                     len(body) if total is None else total)
    nm = name.encode()[:32]
    h[64:64 + len(nm)] = nm
    for i, t in enumerate(table[:STAGE2_MAX_SEGMENTS]):
        struct.pack_into("<QIIIIIIII", h, STAGE2_TABLE + i * STAGE2_ENTRY_BYTES, *t)
    struct.pack_into("<I", h, 508, zlib.crc32(bytes(h[:508])) & 0xFFFFFFFF)
    return bytes(h) + bytes(body)


def parse_segment_arg(text):
    """FILE@ADDR, as kexec_zephyr takes it."""
    path, sep, addr = text.rpartition("@")
    if not sep or not path:
        raise ValueError("--segment %r: give FILE@ADDR" % text)
    return path, int(addr, 0)


STAGE2_PART_ALIGN = 4096      # a continuation part starts on such an address


def stage2_split(zephyr, load, min_run, max_parts):
    """Where --split-zeros cuts: ([(start, end)] the extents of the file that
    are stored, in order; the number of runs left in for want of segments).
    A run of at least min_run zero bytes is cut out after shrinking it inward
    to 4 KiB address boundaries, so that the part after it starts on one (a
    run up to the file's end keeps its end: no part follows it). The first
    extent keeps the arm64 header."""
    import re
    if min_run < 1:
        raise ValueError("--split-zeros %d: give a run length of at least 1 byte" % min_run)
    cuts, tail = [], None
    for m in re.finditer(b"\\x00{%d,}" % min_run, zephyr):
        a = ((load + m.start() + STAGE2_PART_ALIGN - 1) & -STAGE2_PART_ALIGN) - load
        if a < 64:
            a += STAGE2_PART_ALIGN
        if m.end() == len(zephyr):
            if a < len(zephyr):
                tail = a
            continue
        b = ((load + m.end()) & -STAGE2_PART_ALIGN) - load
        if b > a:
            cuts.append((a, b))
    left_in = max(0, len(cuts) - (max_parts - 1))
    if left_in:
        cuts = sorted(sorted(cuts, key=lambda c: c[0] - c[1])[:max_parts - 1])
    extents, start = [], 0
    for a, b in cuts:
        extents.append((start, a))
        start = b
    extents.append((start, len(zephyr) if tail is None else tail))
    return extents, left_in


def stage2_parts_check(segs, parts):
    """The loader's rules for zero bytes and image parts, for segments as
    dicts (load, image_bytes, zero_bytes, footprint): errors as strings."""
    errors = []
    for i, s in enumerate(segs):
        if s["image_bytes"] + s["zero_bytes"] > s["footprint"]:
            errors.append("segment %d: %d image bytes + %d zero bytes are over its footprint %d"
                          % (i, s["image_bytes"], s["zero_bytes"], s["footprint"]))
    if not 1 <= parts <= len(segs):
        errors.append("%d image parts in a table of %d segments" % (parts, len(segs)))
        return errors
    s0 = segs[0]
    occupied_end = s0["load"] + s0["image_bytes"] + s0["zero_bytes"]
    for i in range(1, parts):
        p = segs[i]
        if p["load"] < occupied_end:
            errors.append("part %d at 0x%x is on segment 0's image and zero bytes (up to 0x%x)"
                          % (i, p["load"], occupied_end))
        if p["load"] + p["footprint"] > s0["load"] + s0["footprint"]:
            errors.append("part %d (0x%x..0x%x) is not inside segment 0's footprint (up to 0x%x)"
                          % (i, p["load"], p["load"] + p["footprint"], s0["load"] + s0["footprint"]))
        if i > 1 and p["load"] <= segs[i - 1]["load"]:
            errors.append("part %d at 0x%x is not above part %d at 0x%x" % (i, p["load"], i - 1, segs[i - 1]["load"]))
        if p["footprint"] != p["image_bytes"] + p["zero_bytes"]:
            errors.append("part %d: footprint %d is not its %d image bytes + %d zero bytes"
                          % (i, p["footprint"], p["image_bytes"], p["zero_bytes"]))
    return errors


def stage2_segments_check(segs, sector, end_sector, staging, parts=1, occupied=None):
    """The loader's version-2 rules for segments [(load, footprint, label)]:
    errors as strings. The AM62P map (load >= 0x80000000): every segment in
    0x82000000 .. GPU_CARVEOUT, none on the staging buffer; any map: no two
    overlap, at most 8, sizes below 4 GiB. The first `parts` segments are the
    image's parts: segment 0 is taken against the others of them with
    `occupied` (its image bytes + zero bytes) in place of its footprint
    (stage2_parts_check has the rest of the parts' rules)."""
    errors = []
    if not 1 <= len(segs) <= STAGE2_MAX_SEGMENTS:
        errors.append("%d segments: the loader takes 1..%d" % (len(segs), STAGE2_MAX_SEGMENTS))
    cap = stage2_capacity(sector, end_sector)
    am62p = segs and segs[0][0] >= 0x80000000
    if am62p and staging + cap > GPU_CARVEOUT:
        errors.append("the staging buffer 0x%x + %d bytes (the sector range) runs into the GPU carveout at 0x%x"
                      % (staging, cap, GPU_CARVEOUT))
    for i, (load, fp, label) in enumerate(segs):
        if fp >= 1 << 32:
            errors.append("%s: %d bytes, over the header's 32-bit sizes" % (label, fp))
        if am62p and (load < SPL_STACK_TOP or load + fp > GPU_CARVEOUT):
            errors.append("%s: 0x%x..0x%x is outside the loader's region 0x%x..0x%x"
                          % (label, load, load + fp, SPL_STACK_TOP, GPU_CARVEOUT))
        if am62p and load < staging + cap and staging < load + fp:
            errors.append("%s: 0x%x..0x%x overlaps the loader's staging buffer 0x%x..0x%x (--staging, "
                          "CONFIG_BOOT_STAGE2_STAGING)" % (label, load, load + fp, staging, staging + cap))
        for j in range(i + 1, len(segs)):
            load2, fp2, label2 = segs[j]
            n = occupied if i == 0 and j < parts and occupied is not None else fp
            if load < load2 + fp2 and load2 < load + n:
                errors.append("%s (0x%x..0x%x) overlaps %s (0x%x..0x%x)"
                              % (label, load, load + n, label2, load2, load2 + fp2))
    return errors


def stage2img_segments(zephyr, load, extra, compression, name, sector=STAGE2_SECTOR, end_sector=STAGE2_END_SECTOR,
                       staging=STAGE2_STAGING, split_zeros=0, clear_after=0):
    """A version-2 payload: zephyr.bin at `load` (segment 0, the entry), then
    each (data, addr, label) of `extra`, every stored range sector aligned.
    split_zeros (a run length, 0: off) cuts zephyr.bin at its zero runs into
    segment 0 and continuation parts (stage2_split); clear_after is the zero
    bytes after the image's last byte."""
    if 0x80000000 <= load < SPL_STACK_TOP:
        raise ValueError("the second stage loads the image at 0x%x or above (got 0x%x)" % (SPL_STACK_TOP, load))
    image_size = check_zephyr(zephyr, load)
    fp0 = max(image_size, len(zephyr))
    if clear_after < 0 or len(zephyr) + clear_after > fp0:
        raise ValueError("--clear-after %d: the %d image bytes and that run past segment 0's footprint 0x%x "
                         "(the arm64 image_size)" % (clear_after, len(zephyr), fp0))
    extents = [(0, len(zephyr))]
    if split_zeros:
        if len(extra) >= STAGE2_MAX_SEGMENTS:
            raise ValueError("%d --segment: no table entry left for the image" % len(extra))
        extents, left_in = stage2_split(zephyr, load, split_zeros, STAGE2_MAX_SEGMENTS - len(extra))
        if left_in:
            print("am62p-mkboot.py: --split-zeros %d: %d more zero run(s) of that length left in the stored bytes: "
                  "%d parts and %d --segment fill the table's %d entries (the longest runs are cut)"
                  % (split_zeros, left_in, len(extents), len(extra), STAGE2_MAX_SEGMENTS), file=sys.stderr)
    # Each part's zero bytes: up to the next part, and after the last one up
    # to the image's last byte and --clear-after past it.
    ends = [st for st, _ in extents[1:]] + [len(zephyr) + clear_after]
    table = [dict(load=load + st, image_bytes=en - st, zero_bytes=nxt - en, footprint=nxt - st)
             for (st, en), nxt in zip(extents, ends)]
    table[0]["footprint"] = fp0
    segs = [(load, fp0, "segment 0 (zephyr.bin, .bss included)")]
    segs += [(t["load"], t["footprint"], "part %d of zephyr.bin" % i) for i, t in enumerate(table) if i]
    for data, addr, label in extra:
        if not data:
            raise ValueError("%s is empty" % label)
        segs.append((addr, len(data), label))
    errors = stage2_parts_check(table, len(table))
    errors += stage2_segments_check(segs, sector, end_sector, staging, len(table),
                                    table[0]["image_bytes"] + table[0]["zero_bytes"])
    if errors:
        raise ValueError("; ".join(errors))
    pack = (lambda d: deflate_raw(d)) if compression == COMP_DEFLATE else (lambda d: d)
    raw = []
    for (st, en), t in zip(extents, table):
        raw.append(dict(load=t["load"], image=zephyr[st:en], stored=pack(zephyr[st:en]), comp=compression,
                        footprint=t["footprint"], zero=t["zero_bytes"]))
    raw += [dict(load=addr, image=data, stored=pack(data), comp=compression) for data, addr, _ in extra]
    # One part is written as 0, the field's value before parts: a payload
    # without zero bytes is then byte for byte what it was.
    out = stage2_v2_raw(raw, load, name, parts=len(table) if len(table) > 1 else 0)
    cap = stage2_capacity(sector, end_sector)
    if len(out) - 512 > cap:
        raise ValueError("the payload is %d bytes (%s), over the %d bytes of user-area sectors %d..%d after "
                         "the header: %d bytes too many"
                         % (len(out) - 512, "deflate" if compression == COMP_DEFLATE else "uncompressed", cap,
                            sector + 1, end_sector - 1, len(out) - 512 - cap))
    return out


def stage2_parse(img):
    """(fields dict, stored bytes) of a YKSTAGE2 image, or ValueError. A
    version-2 header's fields carry "segments" (a list of dicts) and the
    stored bytes are the whole payload after the header; "parts" is the
    image parts as the loader reads them (a 0 in the header is 1)."""
    if len(img) < 512 or img[:8] != STAGE2_MAGIC:
        raise ValueError("no YKSTAGE2 header")
    if struct.unpack_from("<I", img, 508)[0] != zlib.crc32(img[:508]) & 0xFFFFFFFF:
        raise ValueError("header CRC-32 mismatch")
    if struct.unpack_from("<I", img, 8)[0] == STAGE2_VERSION_SEGMENTS:
        _, ver, hb, count, parts, entry, total = struct.unpack_from("<8sIIIIQQ", img, 0)
        segs = []
        for i in range(min(count, STAGE2_MAX_SEGMENTS)):
            load, nimage, fp, off, nstored, comp, scrc, icrc, nzero = struct.unpack_from(
                "<QIIIIIIII", img, STAGE2_TABLE + i * STAGE2_ENTRY_BYTES)
            segs.append(dict(load=load, image_bytes=nimage, footprint=fp, stored_offset=off, stored_bytes=nstored,
                             compression=comp, stored_crc32=scrc, image_crc32=icrc, zero_bytes=nzero))
        f = dict(version=ver, header_bytes=hb, count=count, parts=max(parts, 1), entry=entry,
                 load=segs[0]["load"] if segs else None,
                 stored_bytes=total, segments=segs, name=img[64:96].rstrip(b"\0").decode(errors="replace"),
                 compression=segs[0]["compression"] if segs else None)
        stored = bytes(img[512:512 + total])
        if len(stored) != total:
            raise ValueError("the file holds %d payload bytes, the header says %d" % (len(stored), total))
        return f, stored
    _, ver, hb, comp, _ = struct.unpack_from("<8sIIII", img, 0)
    load, entry, nstored, nimage, scrc, icrc = struct.unpack_from("<QQQQII", img, 24)
    f = dict(version=ver, header_bytes=hb, compression=comp, load=load, entry=entry, stored_bytes=nstored,
             image_bytes=nimage, stored_crc32=scrc, image_crc32=icrc,
             name=img[64:96].rstrip(b"\0").decode(errors="replace"))
    stored = bytes(img[512:512 + nstored])
    if len(stored) != nstored:
        raise ValueError("the file holds %d payload bytes, the header says %d" % (len(stored), nstored))
    return f, stored


def stage2_unpack_one(comp, stored, nimage, icrc):
    """The image of one stored range, or (None, error)."""
    if comp == COMP_DEFLATE:
        try:
            d = zlib.decompressobj(-15)
            image = d.decompress(stored) + d.flush()
        except zlib.error as e:
            return None, "raw deflate: %s" % e
    elif comp == COMP_NONE:
        image = stored
    else:
        return None, "compression %d" % comp
    if len(image) != nimage or zlib.crc32(image) & 0xFFFFFFFF != icrc:
        return None, "the image's length or CRC-32 is not the header's"
    return image, None


def stage2check_segments(img, f, stored, zephyr, load, extra, sector, end_sector, staging):
    """A version-2 payload: what the loader checks, plus: the image parts
    (segment 0 and its continuation parts) with their zero bytes are this
    zephyr.bin, and the segments after them are the `extra` (data, addr,
    label), in order. Sets f["clear_after"], the zero bytes past the image's
    last byte."""
    errors = []
    segs = f["segments"]
    parts = f["parts"]
    if f["header_bytes"] != 512:
        errors.append("header size %d" % f["header_bytes"])
    if f["count"] != len(segs) or not 1 <= parts <= len(segs):
        errors.append("%d segments and %d image parts in the header: the loader takes 1..%d segments and "
                      "1..that many parts" % (f["count"], parts, STAGE2_MAX_SEGMENTS))
        return errors, f
    if len(segs) != parts + len(extra):
        errors.append("%d segments in the header, %d of them the image: %d given with --segment"
                      % (f["count"], parts, len(extra)))
        return errors, f
    tail = img[STAGE2_TABLE + len(segs) * STAGE2_ENTRY_BYTES:508]
    if any(img[40:64]) or any(tail):
        errors.append("a reserved header field or an unused table entry is not zero")
    if f["entry"] != load or segs[0]["load"] != load:
        errors.append("entry/segment 0 load 0x%x/0x%x, not 0x%x" % (f["entry"], segs[0]["load"], load))
    if f["stored_bytes"] % 512 or f["stored_bytes"] > stage2_capacity(sector, end_sector):
        errors.append("the payload (%d bytes) is not whole sectors or runs past user-area sector %d"
                      % (f["stored_bytes"], end_sector - 1))
    if len(img) != 512 + f["stored_bytes"]:
        errors.append("the file is %d bytes, the header says 512 + %d" % (len(img), f["stored_bytes"]))
    labels = ["segment 0 (zephyr.bin)"] + ["part %d of zephyr.bin" % i for i in range(1, parts)]
    labels += [label for _, _, label in extra]
    prev_end = 0
    images = []
    for s, label in zip(segs, labels):
        off, n = s["stored_offset"], s["stored_bytes"]
        images.append(None)
        if off % 512 or off < prev_end or off + n > f["stored_bytes"]:
            errors.append("%s: stored range +0x%x (%d bytes) not aligned, out of order or past the payload"
                          % (label, off, n))
            continue
        prev_end = (off + n + 511) & ~511
        part = stored[off:off + n]
        if zlib.crc32(part) & 0xFFFFFFFF != s["stored_crc32"]:
            errors.append("%s: CRC-32 of the stored bytes mismatch" % label)
        if s["compression"] == COMP_NONE and s["stored_crc32"] != s["image_crc32"]:
            errors.append("%s: uncompressed, but the two CRC-32s differ" % label)
        images[-1], err = stage2_unpack_one(s["compression"], part, s["image_bytes"], s["image_crc32"])
        if err:
            errors.append("%s: %s" % (label, err))
        if s["footprint"] < s["image_bytes"]:
            errors.append("%s: footprint %d below the image's %d bytes" % (label, s["footprint"], s["image_bytes"]))
    # The image as the loader leaves it: each part at its address, its zero
    # bytes after it, nothing between those and the next part.
    whole, at = bytearray(), 0
    for s, image, label in zip(segs[:parts], images, labels):
        if image is None:
            break
        if s["load"] - load != at:
            errors.append("%s: at +0x%x, but the image bytes and zero bytes before it end at +0x%x"
                          % (label, s["load"] - load, at))
            break
        if at + len(image) > len(zephyr):
            errors.append("%s: its image bytes run past the given file's %d bytes" % (label, len(zephyr)))
            break
        whole += image
        whole += bytes(min(s["zero_bytes"], len(zephyr) - len(whole)))
        at += len(image) + s["zero_bytes"]
    else:
        if at < len(zephyr):
            errors.append("the image and its zero bytes end at +0x%x, short of the given file's %d bytes"
                          % (at, len(zephyr)))
        elif bytes(whole) != zephyr:
            errors.append("segment 0 (zephyr.bin): the image%s is not the given file"
                          % (" (the parts and their zero runs put together)" if parts > 1 else ""))
        f["clear_after"] = at - len(zephyr)
    for s, image, (data, addr, label) in zip(segs[parts:], images[parts:], extra):
        if image is not None and image != data:
            errors.append("%s: the image is not the given file" % label)
        if s["load"] != addr:
            errors.append("%s: load 0x%x, not 0x%x" % (label, s["load"], addr))
    if not errors:
        image_size = check_zephyr(zephyr, load)
        if image_size > segs[0]["footprint"]:
            errors.append("segment 0's arm64 image_size 0x%x is over its footprint 0x%x"
                          % (image_size, segs[0]["footprint"]))
        errors += stage2_parts_check(segs, parts)
        errors += stage2_segments_check([(s["load"], s["footprint"], label) for s, label in zip(segs, labels)],
                                        sector, end_sector, staging, parts,
                                        segs[0]["image_bytes"] + segs[0]["zero_bytes"])
    return errors, f


def stage2check(img, zephyr, load=ZEPHYR_LOAD, sector=STAGE2_SECTOR, end_sector=STAGE2_END_SECTOR, extra=(),
                staging=STAGE2_STAGING):
    """What the loader checks, plus: the payload is this zephyr.bin (and, in
    a version-2 payload, the `extra` segments)."""
    errors = []
    try:
        f, stored = stage2_parse(img)
    except ValueError as e:
        return ["%s" % e], None
    if f["version"] == STAGE2_VERSION_SEGMENTS:
        return stage2check_segments(img, f, stored, zephyr, load, list(extra), sector, end_sector, staging)
    if extra:
        return ["a version-1 (single image) payload, but --segment was given"], f
    if f["version"] != STAGE2_VERSION or f["header_bytes"] != 512:
        errors.append("header version %d / size %d" % (f["version"], f["header_bytes"]))
    if f["load"] != load or f["entry"] != load:
        errors.append("load/entry 0x%x/0x%x, not 0x%x" % (f["load"], f["entry"], load))
    if zlib.crc32(stored) & 0xFFFFFFFF != f["stored_crc32"]:
        errors.append("CRC-32 of the stored payload mismatch")
    if len(stored) > stage2_capacity(sector, end_sector):
        errors.append("the payload runs past user-area sector %d" % (end_sector - 1))
    if len(img) % 512 or len(img) > (end_sector - sector) * 512:
        errors.append("the file is %d bytes: not whole sectors, or past sector %d" % (len(img), end_sector - 1))
    if f["compression"] == COMP_DEFLATE:
        try:
            d = zlib.decompressobj(-15)
            image = d.decompress(stored) + d.flush()
        except zlib.error as e:
            errors.append("raw deflate: %s" % e)
            image = b""
    elif f["compression"] == COMP_NONE:
        image = stored
    else:
        errors.append("compression %d" % f["compression"])
        image = b""
    if len(image) != f["image_bytes"] or zlib.crc32(image) & 0xFFFFFFFF != f["image_crc32"]:
        errors.append("the image's length or CRC-32 is not the header's")
    if image != zephyr:
        errors.append("the image is not this zephyr.bin")
    return errors, f


def stage2_vectors(outdir):
    """The loader crate's host-test inputs: ok-*.img must load, bad-*.img must
    be refused (loader/src/lib.rs, mkboot_vectors)."""
    import os
    os.makedirs(outdir, exist_ok=True)

    def zimage(size, seed):
        # an arm64 Image header, then a mix the encoder treats differently:
        # text (dynamic Huffman, long matches), noise (stored blocks), zeros
        z = bytearray(size)
        z[0:4] = struct.pack("<I", 0x14000010)
        z[16:24] = struct.pack("<Q", size + 0x10000)
        z[0x38:0x3C] = b"ARM\x64"
        pos, x, n = 0x40, seed, 0
        while pos < size:
            kind = n % 3
            ln = min(size - pos, 20000 + (n * 7919) % 40000)
            if kind == 0:
                chunk = b"".join(b"symbol_%d = 0x%08x;\n" % (i, (i * 2654435761) & 0xFFFFFFFF)
                                 for i in range(pos, pos + ln // 20 + 1))[:ln]
            elif kind == 1:
                buf = bytearray(ln)
                for i in range(ln):
                    x = (x * 1103515245 + 12345) & 0x7FFFFFFF
                    buf[i] = x >> 16 & 0xFF
                chunk = bytes(buf)
            else:
                chunk = bytes(ln)
            z[pos:pos + ln] = chunk
            pos += ln
            n += 1
        return bytes(z)

    big, small = zimage(700000, 1), zimage(3000, 2)
    files = {}
    files["ok-deflate-mixed.img"] = stage2img(big, ZEPHYR_LOAD, COMP_DEFLATE, "mixed")
    files["ok-deflate-small.img"] = stage2img(small, ZEPHYR_LOAD, COMP_DEFLATE, "small")
    files["ok-none.img"] = stage2img(big[:200000], ZEPHYR_LOAD, COMP_NONE, "none")
    c1 = zlib.compressobj(1, zlib.DEFLATED, -15)
    lvl1 = c1.compress(big) + c1.flush()
    files["ok-deflate-level1.img"] = stage2_header(ZEPHYR_LOAD, ZEPHYR_LOAD, lvl1, big, COMP_DEFLATE, "level1") + lvl1
    good = bytearray(files["ok-deflate-mixed.img"])
    b = bytearray(good)
    b[512 + 1000] ^= 0x20
    files["bad-stored-crc.img"] = bytes(b)
    b = bytearray(good)
    b[100] ^= 1
    files["bad-header-crc.img"] = bytes(b)
    # a broken stream with a matching stored CRC: refused by inflate or by
    # the image CRC
    f, stored = stage2_parse(bytes(good))
    st = bytearray(stored)
    st[len(st) // 2] ^= 0x55
    files["bad-deflate.img"] = stage2_header(ZEPHYR_LOAD, ZEPHYR_LOAD, bytes(st), big, COMP_DEFLATE, "bad") + bytes(st)
    files["bad-wrong-load.img"] = stage2_header(0x83000000, 0x83000000, stored, big, COMP_DEFLATE, "load") + stored
    plain = bytes(4096)
    files["bad-not-arm64.img"] = stage2_header(ZEPHYR_LOAD, ZEPHYR_LOAD, plain, plain, COMP_NONE, "plain") + plain
    files["bad-entry-outside.img"] = stage2_header(ZEPHYR_LOAD, ZEPHYR_LOAD + len(big), stored, big, COMP_DEFLATE,
                                                   "entry") + stored

    # version 2: the showcase's shape (an image whose .bss runs past its
    # file, a large blob and a small one above it), and its refusals
    ddr = zimage(120000, 3)[0x40:]
    tcm = bytes((i * 5) & 0xFF for i in range(288))
    blob_at, tcm_at = 0x88000000, 0x88c00000
    for comp, tag in ((COMP_DEFLATE, "deflate"), (COMP_NONE, "none")):
        files["ok-v2-three-%s.img" % tag] = stage2img_segments(
            big, ZEPHYR_LOAD, [(ddr, blob_at, "ddr"), (tcm, tcm_at, "tcm")], comp, "three-" + tag)
    files["ok-v2-one.img"] = stage2img_segments(small, ZEPHYR_LOAD, [], COMP_DEFLATE, "one")
    fp = len(big) + 0x10000                          # zimage's image_size

    def v2(*segs, **kw):
        return stage2_v2_raw([dict(load=a, image=d, stored=deflate_raw(d), comp=COMP_DEFLATE, **o)
                              for d, a, o in segs], ZEPHYR_LOAD, "bad", **kw)
    s0 = (big, ZEPHYR_LOAD, dict(footprint=fp))
    files["bad-v2-overlap-bss.img"] = v2(s0, (ddr, ZEPHYR_LOAD + len(big), {}))
    files["bad-v2-overlap-blobs.img"] = v2(s0, (ddr, blob_at, {}), (tcm, blob_at + len(ddr) - 1, {}))
    files["bad-v2-on-staging.img"] = v2(s0, (tcm, STAGE2_STAGING, {}))
    files["bad-v2-outside-ram.img"] = v2(s0, (tcm, GPU_CARVEOUT, {}))
    files["bad-v2-below-ram.img"] = v2(s0, (tcm, 0x80000000, {}))
    files["bad-v2-unaligned.img"] = v2(s0, (ddr, blob_at, dict(offset=len(deflate_raw(big)) + 4)))
    files["bad-v2-no-segments.img"] = stage2_v2_raw([], ZEPHYR_LOAD, "bad")
    files["bad-v2-footprint.img"] = v2((big, ZEPHYR_LOAD, dict(footprint=len(big))), (ddr, blob_at, {}))
    files["bad-v2-past-payload.img"] = v2(s0, (ddr, blob_at, {}), total=512)
    good3 = bytearray(files["ok-v2-three-deflate.img"])
    f3, _ = stage2_parse(bytes(good3))
    good3[512 + f3["segments"][1]["stored_offset"] + 50] ^= 0x08
    files["bad-v2-segment-crc.img"] = bytes(good3)

    # zero bytes and image parts: an image cut at two zero runs (three parts,
    # the RAM after its last byte cleared too), the same with the two blobs
    # and deflate, the zero bytes alone, and the refusals
    split = bytearray(big[:300000])
    split[40000:140000] = bytes(100000)
    split[200000:260000] = bytes(60000)
    split = bytes(split)
    fp0 = struct.unpack_from("<Q", split, 16)[0]     # as zimage wrote it for 700000 bytes
    files["ok-v2-split-none.img"] = stage2img_segments(split, ZEPHYR_LOAD, [], COMP_NONE, "split-none",
                                                       split_zeros=50000, clear_after=0x8000)
    files["ok-v2-split-deflate.img"] = stage2img_segments(
        split, ZEPHYR_LOAD, [(ddr, blob_at, "ddr"), (tcm, tcm_at, "tcm")], COMP_DEFLATE, "split-deflate",
        split_zeros=50000, clear_after=0x8000)
    files["ok-v2-clear-after.img"] = stage2img_segments(small, ZEPHYR_LOAD, [], COMP_NONE, "clear-after",
                                                        clear_after=0x10000)
    fs, _ = stage2_parse(files["ok-v2-split-none.img"])
    if (fs["parts"], fs["count"]) != (3, 3):
        raise ValueError("the split vector has %d parts in %d segments, not 3 in 3" % (fs["parts"], fs["count"]))
    ext = [(s["load"] - ZEPHYR_LOAD, s["image_bytes"], s["zero_bytes"]) for s in fs["segments"]]

    def cut(nparts, *changes, more=()):
        # the split vector's three parts with (index, key, value) changes, then more segments
        segs = [dict(load=ZEPHYR_LOAD + o, image=split[o:o + n], stored=split[o:o + n], comp=COMP_NONE,
                     footprint=n + z, zero=z) for o, n, z in ext]
        segs[0]["footprint"] = fp0
        for i, key, value in changes:
            segs[i][key] = value
        segs += [dict(load=a, image=d, stored=d, comp=COMP_NONE) for d, a in more]
        return stage2_v2_raw(segs, ZEPHYR_LOAD, "bad", parts=nparts)
    files["bad-v2-zero-over-footprint.img"] = cut(3, (2, "zero", ext[2][2] + 1))
    files["bad-v2-parts-over-count.img"] = cut(4)
    files["bad-v2-part-outside.img"] = cut(3, (2, "load", ZEPHYR_LOAD + fp0 - ext[2][1]))
    files["bad-v2-part-on-occupied.img"] = cut(3, (1, "load", ZEPHYR_LOAD + ext[0][1] + ext[0][2] - 4096))
    files["bad-v2-parts-descending.img"] = cut(3, (1, "load", ZEPHYR_LOAD + ext[2][0] + 0x40000),
                                               (2, "load", ZEPHYR_LOAD + ext[1][0] + 0x40000))
    # not a part, so segment 0's whole footprint counts
    files["bad-v2-blob-in-footprint.img"] = cut(3, more=[(tcm, ZEPHYR_LOAD + fp0 - 4096)])
    for name, data in files.items():
        with open(os.path.join(outdir, name), "wb") as fh:
            fh.write(data)
    return sorted(files)


def self_test():
    """Synthetic inputs in the prebuilt's form (embedded data, TI-like
    certificates, crc32 hashes); no Toradex file needed."""
    fails = []

    def expect(name, cond):
        print("  %-60s %s" % (name, "ok" if cond else "FAIL"))
        if not cond:
            fails.append(name)

    def refused(fn):
        try:
            fn()
        except ValueError:
            return True
        return False

    def fake_cert(n):                         # 30 82 LL LL 30 82 ...: what security.c detects
        body = bytes([0x30, 0x82, 0, 0]) + bytes((i * 7) & 0xFF for i in range(n - 8))
        return bytes([0x30, 0x82]) + struct.pack(">H", len(body)) + body

    def node(name, props, children=()):
        n = Node(name)
        n.props = list(props)
        n.children = list(children)
        return n

    def hashed(data):
        return node("hash", [("value", u32(zlib.crc32(data) & 0xFFFFFFFF)), ("algo", b"crc32\0")])

    ub = fake_cert(1700) + bytes(range(256)) * 300
    dt = fake_cert(1700) + b"\xd0\x0d\xfe\xed" + bytes(5000)
    root = node("", [("timestamp", u32(0)), ("description", b"FIT image with multiple configurations\0")], [
        node("images", [], [
            node("uboot", [("load", u32(0x80800000)), ("compression", b"none\0"), ("arch", b"arm\0"),
                           ("os", b"u-boot\0"), ("type", b"firmware\0"), ("description", b"U-Boot\0"),
                           ("data", ub)], [hashed(ub)]),
            node("fdt-0", [("description", b"board\0"), ("type", b"flat_dt\0"), ("arch", b"arm\0"),
                           ("compression", b"none\0"), ("data", dt)], [hashed(dt)])]),
        node("configurations", [("default", b"conf-0\0")], [
            node("conf-0", [("description", b"board\0"), ("firmware", b"uboot\0"), ("loadables", b"uboot\0"),
                            ("fdt", b"fdt-0\0")])])])
    base = fdt_build(root)

    def zimage(size, image_size):
        z = bytearray(size)
        z[0:4] = struct.pack("<I", 0x14000010)            # b +0x40
        z[16:24] = struct.pack("<Q", image_size)
        z[0x38:0x3C] = b"ARM\x64"
        for i in range(0x40, size):
            z[i] = (i * 13) & 0xFF
        return bytes(z)

    z = zimage(100003, 0x200000)
    out = build(base, z, ZEPHYR_LOAD, "Zephyr")
    errors, _ = check(base, z, out, ZEPHYR_LOAD)
    expect("build + check: no errors", errors == [])
    root2, total, images = parse_fit(out)
    fwn, payload, pos = images["uboot"]
    expect("payload = zephyr.bin + zero padding to 16", payload[:len(z)] == z and len(payload) % 16 == 0
           and not any(payload[len(z):]))
    expect("load = entry = 0x82000000", get_u32(fwn.prop("load")) == ZEPHYR_LOAD
           and get_u32(fwn.prop("entry")) == ZEPHYR_LOAD)
    expect("crc32 recomputed", get_u32(fwn.child("hash").prop("value")) == zlib.crc32(payload) & 0xFFFFFFFF)
    expect("fdt-0 byte for byte, certificate kept", images["fdt-0"][1] == dt and ti_cert_len(images["fdt-0"][1]))
    expect("external data, small header, sector aligned",
           total < DATA_ALIGN and all(p is not None and p % 512 == 0 for _, _, p in images.values()))
    expect("deterministic", build(base, z, ZEPHYR_LOAD, "Zephyr") == out)
    z2 = bytearray(z)
    z2[5000] ^= 1
    expect("check: another zephyr.bin is a FAIL", check(base, bytes(z2), out, ZEPHYR_LOAD)[0] != [])
    bad = bytearray(out)
    bad[pos + 10] ^= 1
    expect("check: a flipped payload byte is a FAIL", check(base, z, bytes(bad), ZEPHYR_LOAD)[0] != [])
    expect("refused: no arm64 header", refused(lambda: build(base, bytes(4096), ZEPHYR_LOAD, "Z")))
    expect("refused: an ELF file", refused(lambda: build(base, b"\x7fELF" + z[4:], ZEPHYR_LOAD, "Z")))
    expect("refused: load below the SPL's stack top (not U-Boot proper's slot)",
           refused(lambda: build(base, z, 0x80000000, "Z")))
    s2 = zimage(90000, 0x35000)
    s2out = build(base, s2, UBOOT_PROPER_LOAD, "boot_stage2")
    expect("second stage at 0x80800000: build + check", check(base, s2, s2out, UBOOT_PROPER_LOAD)[0] == [])
    expect("refused: a second stage whose file reaches the SPL's BSS",
           refused(lambda: build(base, zimage(SPL_BSS - UBOOT_PROPER_LOAD - FDT_ROOM + 1, 0x500000),
                                 UBOOT_PROPER_LOAD, "Z")))
    expect("refused: a second stage whose RAM reaches 0x82000000",
           refused(lambda: build(base, zimage(4096, SPL_STACK_TOP - UBOOT_PROPER_LOAD + 1), UBOOT_PROPER_LOAD, "Z")))

    # the second stage's user-area image
    zz = zimage(300000, 0x400000)
    for comp in (COMP_DEFLATE, COMP_NONE):
        im = stage2img(zz, ZEPHYR_LOAD, comp, "test")
        errs, f = stage2check(im, zz)
        expect("stage2img %s: stage2check clean, whole sectors" % ("deflate" if comp else "none"),
               errs == [] and len(im) % 512 == 0 and f["compression"] == comp and f["name"] == "test")
    im = stage2img(zz, ZEPHYR_LOAD, COMP_DEFLATE, "test")
    hdr = im[:512]
    expect("stage2 header: magic, version 1, load = entry, CRC over 0..508",
           hdr[:8] == STAGE2_MAGIC and struct.unpack_from("<I", hdr, 8)[0] == 1
           and struct.unpack_from("<QQ", hdr, 24) == (ZEPHYR_LOAD, ZEPHYR_LOAD)
           and struct.unpack_from("<I", hdr, 508)[0] == zlib.crc32(hdr[:508]) & 0xFFFFFFFF)
    bad = bytearray(im)
    bad[600] ^= 1
    expect("stage2check: a flipped payload byte is a FAIL", stage2check(bytes(bad), zz)[0] != [])
    bad = bytearray(im)
    bad[40] ^= 1
    expect("stage2check: a flipped header byte is a FAIL", stage2check(bytes(bad), zz)[0] != [])
    expect("stage2check: another zephyr.bin is a FAIL", stage2check(im, z)[0] != [])
    noise = bytearray(zimage(stage2_capacity() + 1, stage2_capacity() + 1))
    x = 7
    for i in range(0x40, len(noise)):
        x = (x * 1103515245 + 12345) & 0x7FFFFFFF
        noise[i] = x >> 16 & 0xFF
    expect("refused: a payload one byte over the user-area range", refused(
        lambda: stage2img(bytes(noise), ZEPHYR_LOAD, COMP_NONE, "big")))
    fit = stage2img(bytes(noise[:stage2_capacity()]), ZEPHYR_LOAD, COMP_NONE, "fit")
    expect("accepted: a payload that fills the range exactly",
           len(fit) == (STAGE2_END_SECTOR - STAGE2_SECTOR) * 512)
    expect("refused: a user-area image for a load below 0x82000000",
           refused(lambda: stage2img(zz, UBOOT_PROPER_LOAD, COMP_DEFLATE, "low")))
    # version 2: several segments
    blob, tcm = bytes(range(256)) * 400, bytes(288)
    extra = [(blob, 0x90000000, "blob"), (tcm, 0x90c00000, "tcm")]
    slot = dict(sector=21073376, end_sector=21204448, staging=0x91000000)    # a 64 MiB slot file
    for comp in (COMP_DEFLATE, COMP_NONE):
        im = stage2img_segments(zz, ZEPHYR_LOAD, extra, comp, "multi", **slot)
        errs, f = stage2check(im, zz, ZEPHYR_LOAD, slot["sector"], slot["end_sector"], extra, slot["staging"])
        expect("stage2img --segment %s: stage2check clean, three segments, whole sectors"
               % ("deflate" if comp else "none"),
               errs == [] and f["version"] == 2 and f["count"] == 3 and len(im) % 512 == 0
               and all(s["stored_offset"] % 512 == 0 for s in f["segments"]))
    f = stage2_parse(im)[0]
    expect("stage2 v2: segment 0's footprint is the arm64 image_size, the others their length",
           [s["footprint"] for s in f["segments"]] == [0x400000, len(blob), len(tcm)])
    expect("stage2check: a segment file that differs is a FAIL",
           stage2check(im, zz, ZEPHYR_LOAD, slot["sector"], slot["end_sector"],
                       [(blob, 0x90000000, "blob"), (bytes(287) + b"\1", 0x90c00000, "tcm")], slot["staging"])[0] != [])
    expect("stage2check: a missing --segment is a FAIL",
           stage2check(im, zz, ZEPHYR_LOAD, slot["sector"], slot["end_sector"], extra[:1], slot["staging"])[0] != [])
    bad = bytearray(im)
    bad[512 + f["segments"][2]["stored_offset"]] ^= 1
    expect("stage2check: a flipped byte in the last segment is a FAIL",
           stage2check(bytes(bad), zz, ZEPHYR_LOAD, slot["sector"], slot["end_sector"], extra, slot["staging"])[0] != [])
    expect("refused: segments on the default staging buffer (0x90000000)",
           refused(lambda: stage2img_segments(zz, ZEPHYR_LOAD, extra, COMP_DEFLATE, "s",
                                              slot["sector"], slot["end_sector"])))
    expect("refused: a segment inside segment 0's .bss",
           refused(lambda: stage2img_segments(zz, ZEPHYR_LOAD, [(tcm, ZEPHYR_LOAD + 0x3ff000, "t")], COMP_NONE, "s")))
    expect("refused: two segments that share a byte",
           refused(lambda: stage2img_segments(zz, ZEPHYR_LOAD, [(blob, 0x88000000, "a"),
                                                                (tcm, 0x88000000 + len(blob) - 1, "b")], COMP_NONE, "s")))
    expect("refused: a segment in the GPU carveout",
           refused(lambda: stage2img_segments(zz, ZEPHYR_LOAD, [(tcm, GPU_CARVEOUT - 100, "t")], COMP_NONE, "s")))
    expect("refused: nine segments",
           refused(lambda: stage2img_segments(zz, ZEPHYR_LOAD, [(tcm, 0x88000000 + i * 0x1000, "t%d" % i)
                                                                for i in range(8)], COMP_NONE, "s")))
    expect("refused: a staging buffer that runs into the GPU carveout",
           refused(lambda: stage2img_segments(zz, ZEPHYR_LOAD, [], COMP_NONE, "s", slot["sector"],
                                              slot["end_sector"], 0x95000000)))
    expect("stage2 v2 without zero options: image parts field 0, no zero bytes (as before them)",
           im[20:24] == bytes(4) and f["parts"] == 1 and not any(s["zero_bytes"] for s in f["segments"]))
    # zero bytes and image parts
    zs = bytearray(zz)
    zs[50000:120000] = bytes(70000)
    zs[180000:200000] = bytes(20000)
    zs = bytes(zs)
    s2check = lambda i, z, x=(): stage2check(i, z, ZEPHYR_LOAD, slot["sector"], slot["end_sector"], x, slot["staging"])
    for comp in (COMP_DEFLATE, COMP_NONE):
        im = stage2img_segments(zs, ZEPHYR_LOAD, extra, comp, "split", split_zeros=16384, clear_after=0x100000, **slot)
        errs, f = s2check(im, zs, extra)
        expect("stage2img --split-zeros --clear-after %s: stage2check clean, 3 parts + 2 segments"
               % ("deflate" if comp else "none"),
               errs == [] and f["parts"] == 3 and f["count"] == 5 and f["clear_after"] == 0x100000)
    sg = f["segments"]
    expect("split: parts on 4 KiB addresses, the runs shrunk inward, the last zero bytes from the file's end",
           [(s["load"] - ZEPHYR_LOAD, s["image_bytes"], s["zero_bytes"]) for s in sg[:3]]
           == [(0, 53248, 65536), (118784, 61440, 16384), (196608, 103392, 0x100000)])
    expect("split: segment 0 keeps the arm64 footprint, a part's is its image and zero bytes",
           [s["footprint"] for s in sg[:3]] == [0x400000, 61440 + 16384, 103392 + 0x100000])
    expect("split: the zero runs are not stored", len(im) == 512 + 53248 + 61440 + 103424 + len(blob) + 512)
    expect("stage2check: a split image against the file without the zero runs is a FAIL", s2check(im, zz, extra)[0] != [])

    def resealed(image, changes):
        b = bytearray(image)
        for fmt, off, value in changes:
            struct.pack_into(fmt, b, off, value)
        struct.pack_into("<I", b, 508, zlib.crc32(bytes(b[:508])) & 0xFFFFFFFF)
        return bytes(b)
    ent = lambda i: STAGE2_TABLE + i * STAGE2_ENTRY_BYTES
    expect("stage2check: zero bytes over a part's footprint is a FAIL",
           s2check(resealed(im, [("<I", ent(1) + 36, 16385)]), zs, extra)[0] != [])
    expect("stage2check: zero bytes short of the next part is a FAIL",
           s2check(resealed(im, [("<I", ent(1) + 36, 16383), ("<I", ent(1) + 12, 61440 + 16383)]), zs, extra)[0] != [])
    expect("stage2check: more image parts than the image has is a FAIL",
           s2check(resealed(im, [("<I", 20, 4)]), zs, extra)[0] != [])
    expect("stage2check: a part counted as a segment is a FAIL",
           s2check(resealed(im, [("<I", 20, 2)]), zs, [(zs[196608:], ZEPHYR_LOAD + 196608, "p")] + extra)[0] != [])
    expect("stage2check: a part moved off its address is a FAIL",
           s2check(resealed(im, [("<Q", ent(2), ZEPHYR_LOAD + 196608 + 4096)]), zs, extra)[0] != [])
    errs, f = s2check(resealed(im, [("<I", ent(2) + 36, 4096), ("<I", ent(2) + 12, 103392 + 4096)]), zs, extra)
    expect("stage2check: a shorter clear-after is clean, and is reported", errs == [] and f["clear_after"] == 4096)
    im = stage2img_segments(zz, ZEPHYR_LOAD, [], COMP_NONE, "bss", clear_after=0x400000 - len(zz))
    errs, f = stage2check(im, zz)
    expect("stage2img --clear-after alone: version 2, one segment, zero bytes up to the footprint's end",
           errs == [] and f["version"] == 2 and (f["count"], f["parts"]) == (1, 1) and im[20:24] == bytes(4)
           and f["segments"][0]["zero_bytes"] == 0x400000 - len(zz) and f["clear_after"] == 0x400000 - len(zz))
    expect("refused: --clear-after one byte past segment 0's footprint",
           refused(lambda: stage2img_segments(zz, ZEPHYR_LOAD, [], COMP_NONE, "s", clear_after=0x400001 - len(zz))))
    ze = zs[:250000] + bytes(50000)
    im = stage2img_segments(ze, ZEPHYR_LOAD, [], COMP_NONE, "tail", split_zeros=16384, clear_after=4096)
    errs, f = stage2check(im, ze)
    expect("split: a zero run up to the file's end joins the last part's zero bytes",
           errs == [] and f["parts"] == 3 and f["segments"][2]["zero_bytes"] == 300000 - 253952 + 4096
           and f["clear_after"] == 4096)
    many = bytearray(zz)
    for i, n in enumerate([8192, 16384] + [20480] * 5):    # seven runs, the first two the shortest
        many[16384 + i * 28672:16384 + i * 28672 + n] = bytes(n)
    many = bytes(many)
    im = stage2img_segments(many, ZEPHYR_LOAD, extra, COMP_NONE, "many", split_zeros=8192, **slot)
    errs, f = s2check(im, many, extra)
    expect("split: more runs than segments: 6 parts beside 2 segments, the two shortest runs left in",
           errs == [] and (f["parts"], f["count"]) == (6, 8)
           and f["segments"][0]["image_bytes"] == 16384 + 2 * 28672)
    expect("refused: .bss into the GPU carveout",
           refused(lambda: build(base, zimage(4096, GPU_CARVEOUT - ZEPHYR_LOAD + 1), ZEPHYR_LOAD, "Z")))
    rp = build(base, None, None, None)
    _, _, rimages = parse_fit(rp)
    expect("repack: payloads and load addresses unchanged",
           all(rimages[n][1] == parse_fit(base)[2][n][1] for n in rimages)
           and get_u32(rimages["uboot"][0].prop("load")) == 0x80800000
           and rimages["uboot"][0].prop("entry") is None)
    tb, ts = b"\x01" * 1000, b"\x02" * 3000
    bp = bootpart(tb, ts, out, 4 << 20)
    expect("bootpart: offsets 0 / 0x80000 / 0x280000, 512-byte padded",
           bp[:1000] == tb and bp[TISPL_OFF:TISPL_OFF + 3000] == ts and bp[UBOOT_OFF:UBOOT_OFF + len(out)] == out
           and len(bp) % 512 == 0 and len(bp) - (UBOOT_OFF + len(out)) < 512)
    big = out + bytes((4 << 20) - ENV_FROM_END - UBOOT_OFF - len(out) + 1)
    expect("bootpart: refused one byte into U-Boot's environment", refused(lambda: bootpart(tb, ts, big, 4 << 20)))
    expect("bootpart: accepted up to the environment", len(bootpart(tb, ts, big[:-1], 4 << 20)) <= (4 << 20))
    expect("bootpart: refused an oversized tispl", refused(lambda: bootpart(tb, bytes(0x200001), out, 4 << 20)))
    print("self-test: %s" % ("PASS" if not fails else "FAIL (%d)" % len(fails)))
    return 1 if fails else 0


def main():
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("info", help="print the structure of a FIT (u-boot.img, tispl.bin)")
    p.add_argument("fit")
    for name in ("build", "check"):
        p = sub.add_parser(name, help="write u-boot.img with Zephyr in place of U-Boot proper"
                           if name == "build" else "re-parse a built image against its inputs")
        p.add_argument("--base", required=True, help="Toradex's prebuilt u-boot.img")
        p.add_argument("--zephyr", required=True, help="zephyr.bin, linked at --load")
        p.add_argument("--load", type=lambda s: int(s, 0), default=ZEPHYR_LOAD,
                       help="load and entry address (default 0x%x)" % ZEPHYR_LOAD)
        if name == "build":
            p.add_argument("-o", "--output", required=True)
            p.add_argument("--description", default="Zephyr (in place of U-Boot proper)")
        else:
            p.add_argument("image")
    p = sub.add_parser("repack", help="the control: U-Boot proper unchanged, data moved out of the FDT")
    p.add_argument("--base", required=True, help="Toradex's prebuilt u-boot.img")
    p.add_argument("-o", "--output", required=True)
    p = sub.add_parser("stage2img", help="the second stage's image for the eMMC user area")
    p.add_argument("--zephyr", required=True, help="zephyr.bin of a direct image, linked at --load")
    p.add_argument("--load", type=lambda s: int(s, 0), default=ZEPHYR_LOAD)
    p.add_argument("--compress", choices=("deflate", "none"), default="deflate")
    p.add_argument("--name", default=None, help="name in the header (default: the zephyr.bin's directory)")
    p.add_argument("--sector", type=lambda s: int(s, 0), default=STAGE2_SECTOR,
                   help="user-area sector of the header (CONFIG_BOOT_STAGE2_SECTOR)")
    p.add_argument("--end-sector", type=lambda s: int(s, 0), default=STAGE2_END_SECTOR,
                   help="first sector the image must not reach (CONFIG_BOOT_STAGE2_END_SECTOR)")
    p.add_argument("--split-zeros", type=lambda s: int(s, 0), default=0, metavar="MIN",
                   help="cut zephyr.bin at every run of at least MIN zero bytes (e.g. 65536): the runs are not "
                   "stored, the loader clears them (version-2 header, image parts)")
    p.add_argument("--clear-after", type=lambda s: int(s, 0), default=0, metavar="BYTES",
                   help="zero bytes the loader clears after the image's last byte (up to the end of its .bss)")
    p.add_argument("-o", "--output", required=True)
    p = sub.add_parser("stage2check", help="re-read a second-stage image the way the loader will")
    p.add_argument("--zephyr", required=True)
    p.add_argument("--load", type=lambda s: int(s, 0), default=ZEPHYR_LOAD)
    p.add_argument("--sector", type=lambda s: int(s, 0), default=STAGE2_SECTOR)
    p.add_argument("--end-sector", type=lambda s: int(s, 0), default=STAGE2_END_SECTOR)
    p.add_argument("image")
    for q in (sub.choices["stage2img"], p):
        q.add_argument("--segment", action="append", default=[], metavar="FILE@ADDR",
                       help="another segment at ADDR (repeatable): a version-2 payload, zephyr.bin is segment 0")
        q.add_argument("--staging", type=lambda s: int(s, 0), default=STAGE2_STAGING,
                       help="the loader's CONFIG_BOOT_STAGE2_STAGING (default 0x%x): no segment may be on it"
                       % STAGE2_STAGING)
    p = sub.add_parser("stage2-vectors", help="write the loader crate's host-test images (BS2_VECTORS)")
    p.add_argument("dir")
    p = sub.add_parser("bootpart", help="lay out a whole eMMC boot partition image")
    p.add_argument("--tiboot3", required=True)
    p.add_argument("--tispl", required=True)
    p.add_argument("--uboot", required=True, help="the u-boot.img to place at 0x280000")
    p.add_argument("--size", required=True, type=lambda s: int(s, 0),
                   help="the boot partition's size in bytes (/sys/block/mmcblk0boot1/size x 512)")
    p.add_argument("-o", "--output", required=True)
    a = ap.parse_args()

    if a.cmd == "info":
        describe(open(a.fit, "rb").read(), a.fit)
        return 0
    if a.cmd == "bootpart":
        tb, ts, ub = (open(f, "rb").read() for f in (a.tiboot3, a.tispl, a.uboot))
        img = bootpart(tb, ts, ub, a.size)
        with open(a.output, "wb") as f:
            f.write(img)
        print("wrote %s: %d bytes (0x%x, %d sectors = 0x%x): tiboot3 @0x0 (%d), tispl @0x%x (%d), "
              "u-boot.img @0x%x (%d); sha256 %s" % (
                  a.output, len(img), len(img), len(img) // 512, len(img) // 512, len(tb), TISPL_OFF, len(ts),
                  UBOOT_OFF, len(ub), hashlib.sha256(img).hexdigest()))
        return 0

    if a.cmd == "stage2-vectors":
        names = stage2_vectors(a.dir)
        print("wrote %d images to %s: %s" % (len(names), a.dir, " ".join(names)))
        return 0
    if a.cmd in ("stage2img", "stage2check"):
        import os
        zephyr = open(a.zephyr, "rb").read()
        extra = []
        for spec in a.segment:
            path, addr = parse_segment_arg(spec)
            extra.append((open(path, "rb").read(), addr, "%s@0x%x" % (os.path.basename(path), addr)))
        comp = COMP_DEFLATE if getattr(a, "compress", "deflate") == "deflate" else COMP_NONE
        if a.cmd == "stage2img":
            name = a.name or os.path.basename(os.path.dirname(os.path.dirname(os.path.abspath(a.zephyr))))
            if extra or a.split_zeros or a.clear_after:
                img = stage2img_segments(zephyr, a.load, extra, comp, name, a.sector, a.end_sector, a.staging,
                                         a.split_zeros, a.clear_after)
            else:
                img = stage2img(zephyr, a.load, comp, name, a.sector, a.end_sector)
        else:
            img = open(a.image, "rb").read()
        sector, end_sector = a.sector, a.end_sector
        errors, f = stage2check(img, zephyr, a.load, sector, end_sector, extra, a.staging)
        if errors:
            for e in errors:
                print("stage2check: FAIL: %s" % e, file=sys.stderr)
            return 1
        nsec = len(img) // 512
        if f["version"] == STAGE2_VERSION_SEGMENTS:
            print("%s: %d bytes = %d sectors, user-area sectors %d..%d (of %d..%d); \"%s\", header version 2, "
                  "%d segments%s, stored %d bytes, entry 0x%x, staging 0x%x; sha256 %s" % (
                      a.output if a.cmd == "stage2img" else a.image, len(img), nsec, sector, sector + nsec - 1,
                      sector, end_sector - 1, f["name"], f["count"],
                      " (the first %d are the image's parts)" % f["parts"] if f["parts"] > 1 else "",
                      f["stored_bytes"], f["entry"], a.staging, hashlib.sha256(img).hexdigest()))
            labels = ["zephyr.bin"] + ["zephyr.bin part %d" % i for i in range(1, f["parts"])] + [w[2] for w in extra]
            for i, (s, label) in enumerate(zip(f["segments"], labels)):
                end = s["load"] + s["image_bytes"]
                print("  [%d] %-24s 0x%08x..0x%08x image %9d bytes (CRC-32 %08x), footprint 0x%x, %s %d bytes "
                      "at +0x%x%s" % (i, label, s["load"], end, s["image_bytes"], s["image_crc32"], s["footprint"],
                                      "deflate" if s["compression"] == COMP_DEFLATE else "stored",
                                      s["stored_bytes"], s["stored_offset"],
                                      ", then %d zero bytes 0x%08x..0x%08x" % (s["zero_bytes"], end,
                                                                               end + s["zero_bytes"])
                                      if s["zero_bytes"] else ""))
            nzero = sum(s["zero_bytes"] for s in f["segments"])
            if nzero:
                top = a.load + len(zephyr)
                print("  cleared by the loader: %d bytes in %d range(s), %d of them after the image's last byte "
                      "(0x%08x..0x%08x); of the image's %d bytes %d are stored"
                      % (nzero, sum(1 for s in f["segments"] if s["zero_bytes"]), f["clear_after"], top,
                         top + f["clear_after"], len(zephyr),
                         sum(s["image_bytes"] for s in f["segments"][:f["parts"]])))
        else:
            print("%s: %d bytes = %d sectors, user-area sectors %d..%d (of %d..%d); \"%s\", %s, stored %d bytes "
                  "(CRC-32 %08x), image %d bytes (CRC-32 %08x), load = entry = 0x%x; sha256 %s" % (
                      a.output if a.cmd == "stage2img" else a.image, len(img), nsec, sector,
                      sector + nsec - 1, sector, end_sector - 1, f["name"],
                      "deflate" if f["compression"] == COMP_DEFLATE else "uncompressed", f["stored_bytes"],
                      f["stored_crc32"], f["image_bytes"], f["image_crc32"], f["load"],
                      hashlib.sha256(img).hexdigest()))
        print("  write: dd if=IMG of=/dev/mmcblk0 bs=512 seek=%d conv=fsync   (%d sectors, %d left free)"
              % (sector, nsec, end_sector - sector - nsec))
        print("stage2check: ok")
        if a.cmd == "stage2img":
            with open(a.output, "wb") as fh:
                fh.write(img)
            print("wrote %s" % a.output)
        return 0

    base = open(a.base, "rb").read()
    if a.cmd == "repack":
        out = build(base, None, None, None)
        broot, _, bimages = parse_fit(base)
        root, total, images = parse_fit(out)
        bad = [n for n in bimages if images[n][1] != bimages[n][1] or images[n][2] is None
               or images[n][2] % 512 or images[n][0].prop("load") != bimages[n][0].prop("load")]
        if [n.name for n in root.child("images").children] != [n.name for n in broot.child("images").children]:
            bad.append("node order")
        if default_conf(root).props != default_conf(broot).props:
            bad.append("configuration")
        describe(out, a.output)
        if bad:
            print("check: FAIL: %s" % ", ".join(bad), file=sys.stderr)
            return 1
        with open(a.output, "wb") as f:
            f.write(out)
        print("check: ok (every payload, load address and the configuration as in the base); wrote %s"
              % a.output)
        return 0
    zephyr = open(a.zephyr, "rb").read()
    if a.cmd == "build":
        out = build(base, zephyr, a.load, a.description)
    else:
        out = open(a.image, "rb").read()
    errors, image_size = check(base, zephyr, out, a.load)
    describe(out, a.output if a.cmd == "build" else a.image)
    print("  zephyr.bin %d bytes, header image_size 0x%x: 0x%x..0x%x (GPU carveout at 0x%x)" % (
        len(zephyr), image_size, a.load, a.load + max(image_size, len(zephyr)), GPU_CARVEOUT))
    print("  in an eMMC boot partition u-boot.img ends at 0x%x: the partition needs >= %d bytes (0x%x)" % (
        UBOOT_OFF + len(out), UBOOT_OFF + len(out) + ENV_FROM_END, UBOOT_OFF + len(out) + ENV_FROM_END))
    if errors:
        for e in errors:
            print("check: FAIL: %s" % e, file=sys.stderr)
        return 1
    print("check: ok (configuration, fdt-0 and its certificate as in the base; payload = zephyr.bin; "
          "load = entry = 0x%x; external data, sector aligned)" % a.load)
    if a.cmd == "build":
        with open(a.output, "wb") as f:
            f.write(out)
        print("wrote %s" % a.output)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except ValueError as e:
        print("am62p-mkboot.py: %s" % e, file=sys.stderr)
        sys.exit(2)
