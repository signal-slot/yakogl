#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""m7_image.py - strip a Cortex-M7 ELF to what the A53 needs to load it.

    m7_image.py <in.elf> <out.elf>

Route 3 of the direct boot (docs/notes/imx8mp-direct-boot.md, "Route 3"):
the A53 image carries the M7 firmware and loads it itself (board_m7.c). It
needs the ELF header, the program headers and the bytes of each PT_LOAD,
nothing else. The output keeps the file layout up to the end of the last
loaded byte (offsets stay valid, so `readelf -l` reads it as it reads the
input), zeroes every byte there that no program header covers, and drops
the section headers. Debug info and the symbol table follow the segments in
a Zephyr ELF, so they are cut off.

Refuses anything but a little-endian ELF32 for ARM (EM_ARM) of type EXEC
with at least one PT_LOAD.
"""
import struct
import sys

PT_LOAD = 1
EM_ARM = 40


def main(argv):
    if len(argv) != 3:
        sys.stderr.write(__doc__)
        return 2
    data = open(argv[1], "rb").read()
    if data[:4] != b"\x7fELF" or data[4] != 1 or data[5] != 1:
        sys.exit(f"m7_image: {argv[1]}: not a little-endian ELF32")
    (e_type, e_machine, _ver, e_entry, e_phoff, _shoff, _flags, e_ehsize,
     e_phentsize, e_phnum, _shentsize, _shnum, _shstrndx) = struct.unpack_from(
        "<HHIIIIIHHHHHH", data, 16)
    if e_type != 2 or e_machine != EM_ARM:
        sys.exit(f"m7_image: {argv[1]}: type {e_type} machine {e_machine}, want EXEC for ARM")
    if e_phentsize != 32 or e_phnum == 0:
        sys.exit(f"m7_image: {argv[1]}: no usable program headers")

    keep = [(0, e_ehsize), (e_phoff, e_phoff + e_phnum * e_phentsize)]
    loads = 0
    for i in range(e_phnum):
        p_type, p_offset, _vaddr, p_paddr, p_filesz, p_memsz, _f, _a = struct.unpack_from(
            "<IIIIIIII", data, e_phoff + i * e_phentsize)
        if p_type != PT_LOAD:
            continue
        loads += 1
        if p_offset + p_filesz > len(data):
            sys.exit(f"m7_image: segment {i} runs past the end of the file")
        if p_filesz:
            keep.append((p_offset, p_offset + p_filesz))
        print(f"m7_image: LOAD paddr 0x{p_paddr:08x} filesz 0x{p_filesz:x} memsz 0x{p_memsz:x}")
    if loads == 0:
        sys.exit(f"m7_image: {argv[1]}: no PT_LOAD")

    end = max(b for _, b in keep)
    out = bytearray(end)
    for a, b in keep:
        out[a:b] = data[a:b]
    # e_shoff, e_shnum, e_shstrndx: no section headers in the output
    struct.pack_into("<I", out, 32, 0)
    struct.pack_into("<HH", out, 48, 0, 0)
    open(argv[2], "wb").write(out)
    print(f"m7_image: entry 0x{e_entry:08x}, {loads} PT_LOADs, {len(data)} -> {end} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
