#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""x2-mkflash.py -- an Armadillo-X2 boot image whose BL33 is a Zephyr image.

The i.MX8M Plus boot image ("imx-boot", "flash.bin") that Atmark ships is

    0x00000  IVT + U-Boot SPL (+ DDR training firmware), read by the Boot ROM
    0x58000  the second-loader FIT that SPL reads through the ROM API
             (image offset + CONFIG_SYS_MMCSD_RAW_MODE_U_BOOT_SECTOR * 512
             - 0x8000 = image offset + 0x58000), with external data:
               uboot-1  U-Boot proper, load 0x40200000, no entry
               fdt-1    U-Boot's devicetree
               atf-1    TF-A BL31, load = entry = 0x970000
               tee-1    OP-TEE, load = entry = 0x56000000

SPL loads every image, then jumps to the first loadable with an entry point
(BL31); BL31 runs OP-TEE's init and enters BL33 at PLAT_NS_IMAGE_OFFSET =
0x40200000 in non-secure EL2. Replacing uboot-1's payload with a Zephyr image
linked at 0x40200000 makes Zephyr the BL33. Everything else (SPL, BL31,
OP-TEE, fdt-1) is copied unchanged; fdt-1 stays because SPL refuses a FIT
whose "firmware" has no devicetree to append. See docs/notes/x2-direct-boot.md.

  x2-mkflash.py info  IMAGE
  x2-mkflash.py build --base IMAGE --bl33 zephyr.bin -o OUT [--limit BYTES]
                      [--description TEXT]

Standard library only. No signing: this is for boards whose HAB is open.
"""
import argparse
import hashlib
import struct
import sys

FIT_OFFSET = 0x58000          # sld_header_off of imx-mkimage's iMX8M flash_evk
IVT_TAG = 0x412000D1          # tag 0xD1, length 0x0020, version 0x41
FDT_MAGIC = 0xD00DFEED
BL33_LOAD = 0x40200000        # TF-A plat/imx/imx8m/imx8mp PLAT_NS_IMAGE_OFFSET
# The eMMC boot partition holds U-Boot's environment at 0x3FA000 (redundant)
# and 0x3FE000 (x2_defconfig CONFIG_ENV_OFFSET[_REDUND]); stay below both.
DEFAULT_LIMIT = 0x3FA000
DATA_START = 0x3000           # CONFIG_FIT_EXTERNAL_OFFSET of x2_defconfig
DATA_ALIGN = 0x1000

FDT_BEGIN_NODE, FDT_END_NODE, FDT_PROP, FDT_NOP, FDT_END = 1, 2, 3, 4, 9


# --- a minimal flattened-devicetree reader and writer -------------------------

class Node:
    def __init__(self, name):
        self.name = name
        self.props = []      # [(name, bytes)], in order
        self.children = []   # [Node], in order

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

    pos = off_struct
    stack, root = [], None
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
            stack[-1].props.append((string_at(nameoff), blob[pos:pos + length]))
            pos = (pos + length + 3) & ~3
        elif tok == FDT_NOP:
            continue
        elif tok == FDT_END:
            return root, total
        else:
            raise ValueError("bad FDT token %d at 0x%x" % (tok, pos - 4))


def fdt_build(root):
    strings, offsets = bytearray(), {}

    def name_off(name):
        if name not in offsets:
            offsets[name] = len(strings)
            strings.extend(name.encode() + b"\0")
        return offsets[name]

    st = bytearray()

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
    off_struct = off_rsv + 16                 # one empty reservation entry
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


# --- the image -----------------------------------------------------------------

def parse_image(img):
    """Return (ivt words, fit root, fit header size, {image name: (props, payload, position)})."""
    ivt = struct.unpack_from("<8I", img, 0)
    if ivt[0] != IVT_TAG:
        raise ValueError("no i.MX IVT at offset 0 (0x%08x); is this an imx-boot image?" % ivt[0])
    fit_blob = img[FIT_OFFSET:]
    root, fit_size = fdt_parse(fit_blob)
    images = root.child("images")
    if images is None or root.child("configurations") is None:
        raise ValueError("the FDT at 0x%x is not a FIT" % FIT_OFFSET)
    out = {}
    for node in images.children:
        size = get_u32(node.prop("data-size"))
        pos = get_u32(node.prop("data-position"))
        off = get_u32(node.prop("data-offset"))
        if pos is None and off is not None:
            pos = ((fit_size + 3) & ~3) + off
        if node.prop("data") is not None:
            payload, pos = node.prop("data"), None
        else:
            payload = fit_blob[pos:pos + size]
            if len(payload) != size:
                raise ValueError("%s: data past the end of the image" % node.name)
        out[node.name] = (node, payload, pos)
    return ivt, root, fit_size, out


def describe(img, label):
    ivt, root, fit_size, images = parse_image(img)
    print("%s: %d bytes (0x%x), sha256 %s" % (label, len(img), len(img), hashlib.sha256(img).hexdigest()))
    print("  IVT @0x0: entry 0x%x self 0x%x boot_data 0x%x csf 0x%x" % (ivt[1], ivt[5], ivt[4], ivt[6]))
    bd = struct.unpack_from("<3I", img, ivt[4] - ivt[5]) if 0 <= ivt[4] - ivt[5] < 0x100 else None
    if bd:
        print("  boot data: start 0x%x length 0x%x plugin %d (SPL ends at image offset 0x%x)" % (bd[0], bd[1], bd[2], bd[1]))
    print("  FIT @0x%x: header %d bytes, \"%s\"" % (FIT_OFFSET, fit_size, get_str(root.prop("description"))))
    for name, (node, payload, pos) in images.items():
        load, entry = get_u32(node.prop("load")), get_u32(node.prop("entry"))
        print("    %-7s %-24s type %-10s pos 0x%06x (image 0x%06x) size 0x%06x load %s entry %s sha256 %s" % (
            name, '"%s"' % get_str(node.prop("description")), get_str(node.prop("type")),
            pos if pos is not None else 0, FIT_OFFSET + (pos or 0), len(payload),
            "0x%x" % load if load is not None else "-", "0x%x" % entry if entry is not None else "-",
            hashlib.sha256(payload).hexdigest()[:16]))
    confs = root.child("configurations")
    print("  configurations: default \"%s\"" % get_str(confs.prop("default")))
    for c in confs.children:
        print("    %s: %s" % (c.name, ", ".join("%s=%s" % (n, get_str(v)) for n, v in c.props)))
    return ivt, root, fit_size, images


def firmware_node_name(root):
    confs = root.child("configurations")
    conf = confs.child(get_str(confs.prop("default")))
    return get_str(conf.prop("firmware")), get_str(conf.prop("loadables"))


def build(base, bl33, limit, description="Zephyr (BL33)"):
    ivt, root, fit_size, images = parse_image(base)
    fw_name, loadables = firmware_node_name(root)
    if fw_name not in images:
        raise ValueError("the default configuration's firmware %r is not an image" % fw_name)
    fw_node = images[fw_name][0]
    if get_u32(fw_node.prop("load")) != BL33_LOAD:
        raise ValueError("%s loads at 0x%x, not BL33's 0x%x" % (fw_name, get_u32(fw_node.prop("load")), BL33_LOAD))
    if fw_node.prop("entry") is not None:
        # SPL jumps to the firmware's entry if it has one; the chain needs
        # it to fall through to BL31's.
        raise ValueError("%s has an entry point: SPL would jump to it instead of BL31" % fw_name)
    if bl33[:4] == b"\x7fELF":
        raise ValueError("--bl33 is an ELF file; give zephyr.bin")
    if struct.unpack_from("<I", bl33, 0x38)[0] != 0x644D5241:
        print("note: no arm64 Image header (\"ARM\\x64\" at 0x38) in --bl33; entering at its first word anyway",
              file=sys.stderr)

    # Pad the payload to 16 bytes: SPL appends fdt-1 right after it, at
    # load + size, and an unaligned FDT is not what SPL's fdt code expects.
    payload = bl33 + bytes((-len(bl33)) % 16)

    # A fresh FIT with the same nodes and properties; only the firmware's
    # payload, description and the data positions change.
    fw_node.set("description", description.encode() + b"\0")
    order = [n.name for n in root.child("images").children]
    payloads = {name: images[name][1] for name in order}
    payloads[fw_name] = payload
    for name in order:
        node = images[name][0]
        node.props = [(n, v) for n, v in node.props if n not in ("data", "data-offset")]
        node.set("data-size", u32(len(payloads[name])))
        node.set("data-position", u32(0))
    hdr = fdt_build(root)
    if len(hdr) > DATA_START:
        raise ValueError("FIT header %d bytes, over 0x%x" % (len(hdr), DATA_START))
    pos = DATA_START
    for name in order:
        images[name][0].set("data-position", u32(pos))
        pos = (pos + len(payloads[name]) + DATA_ALIGN - 1) & ~(DATA_ALIGN - 1)
    hdr = fdt_build(root)           # same length: only u32 values changed
    fit = bytearray(hdr) + bytearray(DATA_START - len(hdr))
    for name in order:
        p = get_u32(images[name][0].prop("data-position"))
        fit.extend(bytes(p - len(fit)))
        fit.extend(payloads[name])
    out = bytes(base[:FIT_OFFSET]) + bytes(fit)
    if limit and len(out) > limit:
        raise ValueError("the image is %d bytes (0x%x), over the limit 0x%x (U-Boot's environment in the "
                         "eMMC boot partition); a smaller BL33, or --limit 0 for an SD card" % (len(out), len(out), limit))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("info", help="print the layout of an imx-boot image")
    p.add_argument("image")
    p = sub.add_parser("build", help="replace the FIT's U-Boot with a BL33 image")
    p.add_argument("--base", required=True, help="Atmark's imx-boot image (SPL, BL31, OP-TEE come from it)")
    p.add_argument("--bl33", required=True, help="zephyr.bin linked at 0x40200000")
    p.add_argument("-o", "--output", required=True)
    p.add_argument("--description", default="Zephyr (BL33)",
                   help="the firmware node's description (default: %(default)s)")
    p.add_argument("--limit", type=lambda s: int(s, 0), default=DEFAULT_LIMIT,
                   help="refuse an image larger than this (default 0x%x; 0 = no limit)" % DEFAULT_LIMIT)
    a = ap.parse_args()
    if a.cmd == "info":
        describe(open(a.image, "rb").read(), a.image)
        return 0
    base = open(a.base, "rb").read()
    bl33 = open(a.bl33, "rb").read()
    out = build(base, bl33, a.limit, a.description)
    # Read the result back the way it will be checked on the board.
    _, root, _, images = describe(out, a.output)
    fw_name, _ = firmware_node_name(root)
    assert images[fw_name][1][:len(bl33)] == bl33
    assert get_u32(images[fw_name][0].prop("load")) == BL33_LOAD
    assert out[:FIT_OFFSET] == base[:FIT_OFFSET]
    _, _, _, base_images = parse_image(base)
    for name, (_, payload, _) in base_images.items():
        if name != fw_name:
            assert images[name][1] == payload, name
    with open(a.output, "wb") as f:
        f.write(out)
    print("wrote %s: SPL and %s copied from %s, %s = %s (%d bytes)" % (
        a.output, ", ".join(n for n in images if n != fw_name), a.base, fw_name, a.bl33, len(bl33)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
