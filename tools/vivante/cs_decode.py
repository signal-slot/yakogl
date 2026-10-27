#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Decode a Vivante command stream into the register writes it performs.

Reads the file the patched drm-shim writes (ETNA_SHIM_CS_DUMP): a sequence of
submits, each a 32-bit byte length followed by that many bytes of stream.
With --raw it reads one bare stream instead, which is how this backend's own
buffer is compared against Mesa's.

The point is the diff. Two streams printed this way line up register by
register, so "what does a working driver tell this part" stops being a
reading exercise in someone else's source.

A stream is only half of what the GPU reads, though. Texture descriptors,
shader binaries and vertex buffers are blocks in memory that the stream merely
points at, so a stream diff is blind to them: a wrong 256-byte texture
descriptor hides behind a set of registers that match perfectly. --mem reads
the memory dump the shim and this backend's OSAL now write beside the stream
(ETNA_SHIM_MEM_DUMP / YAKOGL_MEM_DUMP), and --tex-desc follows each
NTE_DESCRIPTOR_ADDR into it and prints the descriptor field by field.
"""
import argparse
import os
import re
import struct
import sys

DEFAULT_REGS = os.path.expanduser(
    "~/pvr-lab/mesa-build/mesa-26.2.1/src/etnaviv/hw/state_3d.xml.h")
# The 3D header does not carry the GL block -- the semaphore, the stall token
# and GL_FLUSH_CACHE live in state.xml.h. That is the bank a hang is read in,
# and leaving it unnamed meant every diff printed bare addresses there (worse,
# 0x03800 picked up a TS name from a neighbouring bank). Named from both.
DEFAULT_REGS2 = os.path.expanduser(
    "~/pvr-lab/mesa-build/mesa-26.2.1/src/etnaviv/hw/state.xml.h")
DEFAULT_TEXDESC = os.path.expanduser(
    "~/pvr-lab/mesa-build/mesa-26.2.1/src/etnaviv/hw/texdesc_3d.xml.h")

# A texture descriptor is 256 bytes, and its first 0x40 are the mip chain's
# addresses, so a plain define below 0x40 is a field value and not an offset.
TEXDESC_BYTES = 256
TEXDESC_FIRST_FIELD = 0x40

# The front end's opcodes, in the top five bits of a command word.
OPCODES = {
    0x01: "LOAD_STATE",
    0x02: "END",
    0x03: "NOP",
    0x04: "DRAW_2D",
    0x05: "DRAW_PRIMITIVES",
    0x06: "DRAW_INDEXED_PRIMITIVES",
    0x07: "WAIT",
    0x08: "LINK",
    0x09: "STALL",
    0x0a: "CALL",
    0x0b: "RETURN",
    0x0d: "CHIP_SELECT",
    0x0e: "DRAW_INSTANCED",
}

def load_regs(path, prefix="VIVS_"):
    """Name registers from Mesa's own register header, so the names in a diff
    are the ones its source uses and nothing is named by hand (or wrongly)."""
    plain = re.compile(r"^#define\s+%s([A-Za-z0-9_]+)\s+0x([0-9a-fA-F]+)\s*$" % prefix)
    array = re.compile(r"^#define\s+%s([A-Za-z0-9_]+)\(i0\)\s+"
                       r"\s*\(0x([0-9a-fA-F]+)\s*\+\s*0x([0-9a-fA-F]+)\*\(i0\)\)" % prefix)
    size = re.compile(r"^#define\s+%s([A-Za-z0-9_]+)__LEN\s+0x([0-9a-fA-F]+)" % prefix)
    names, arrays, lens = {}, {}, {}

    for line in open(path):
        m = plain.match(line)
        if m and not m.group(1).endswith(("__LEN", "__ESIZE")):
            names.setdefault(int(m.group(2), 16), m.group(1))
            continue
        m = array.match(line)
        if m:
            arrays[m.group(1)] = (int(m.group(2), 16), int(m.group(3), 16))
            continue
        m = size.match(line)
        if m:
            lens[m.group(1)] = int(m.group(2), 16)
    return names, arrays, lens


class Regs:
    """Registers by address; an unknown one prints as its address.

    Banks overlap, so which array an address belongs to has to be decided, not
    guessed at. An array whose header gives no __LEN used to be assumed 0x400
    entries long, which runs it straight through every bank above it: that is
    how NTE_DESCRIPTOR_SAMP_CTRL0 came to be printed for an address that is
    really SH_HALTI5_UNIFORMS, and a diff read under those names says the
    wrong thing with complete confidence. An array with no __LEN now ends
    where the next register begins, and of the arrays that still contain an
    address the one starting closest below it wins.
    """

    def __init__(self, header=None, header2=None):
        self.names, self.arrays, self.lens = ({}, {}, {})
        if header:
            self.names, self.arrays, self.lens = load_regs(header)
        if header2:
            # The 3D header wins where both define an address: it is the one
            # this GPU's 3D pipe is described by, and setdefault keeps it.
            names2, arrays2, lens2 = load_regs(header2)
            for addr, nm in names2.items():
                self.names.setdefault(addr, nm)
            for nm, v in arrays2.items():
                self.arrays.setdefault(nm, v)
            for nm, v in lens2.items():
                self.lens.setdefault(nm, v)
        self.banks = self._banks()

    def _banks(self):
        """(base, name, stride, count) per array, sorted by base."""
        starts = sorted(set(list(self.names) +
                            [base for base, _ in self.arrays.values()]))
        banks = []
        for nm, (base, stride) in self.arrays.items():
            count = self.lens.get(nm)
            if count is None:
                nxt = next((b for b in starts if b > base), None)
                count = max(1, (nxt - base) // stride) if nxt else 0x400
            banks.append((base, nm, stride, count))
        banks.sort()
        return banks

    def name(self, addr):
        if addr in self.names:
            return self.names[addr]
        best = None
        for base, nm, stride, count in self.banks:
            if base > addr:
                break
            if (addr - base) % stride == 0 and (addr - base) // stride < count:
                best = (base, nm, stride)
        if best:
            base, nm, stride = best
            return "%s[%u]" % (nm, (addr - base) // stride)
        return "0x%05x" % addr

    def is_uniform(self, addr):
        nm = self.name(addr)
        return "UNIFORM" in nm and "[" in nm


class Mem:
    """The blocks a stream points at, read from a memory dump.

    The dump is what the patched drm-shim writes for ETNA_SHIM_MEM_DUMP and
    what this backend's Linux OSAL writes for YAKOGL_MEM_DUMP: per allocation
    a device address, a size, a zero, the bytes, padded to 8. Both sides use
    the same format so one reader resolves a pointer on either.
    """

    def __init__(self, paths):
        self.blocks = []
        for path in paths or []:
            data = open(path, "rb").read()
            off = 0
            while off + 16 <= len(data):
                addr, size, _ = struct.unpack_from("<QII", data, off)
                off += 16
                if off + size > len(data):
                    break
                self.blocks.append((addr, data[off:off + size]))
                off += size + ((8 - size % 8) % 8)

    def read(self, addr, length):
        """The last record wins: a buffer written, submitted, rewritten and
        submitted again is the contents it was last submitted with."""
        for base, body in reversed(self.blocks):
            if base <= addr and addr + length <= base + len(body):
                return body[addr - base:addr - base + length]
        return None


class TexDesc:
    """Names the dwords of a 256-byte texture descriptor.

    The offsets come from Mesa's texdesc_3d.xml.h, the same header its
    etnaviv_texture_desc.c fills the block through, so the two sides are named
    identically. Only defines at 0x40 and above are offsets: the first 0x40
    bytes are the mip chain's addresses, and a plain define below that is a
    field value that happens to fit.
    """

    def __init__(self, header):
        names, self.arrays, self.lens = load_regs(header, prefix="TEXDESC_")
        self.names = {a: n for a, n in names.items()
                      if TEXDESC_FIRST_FIELD <= a < TEXDESC_BYTES and a % 4 == 0}

    def name(self, off):
        if off in self.names:
            return self.names[off]
        for nm, (base, stride) in self.arrays.items():
            count = self.lens.get(nm, 0)
            if base <= off < base + count * stride and (off - base) % stride == 0:
                return "%s[%u]" % (nm, (off - base) // stride)
        return "0x%02x" % off


def print_tex_descs(state, regs, mem, texdesc):
    """Follow every NTE_DESCRIPTOR_ADDR into memory and print what it points at.

    This is the half of the picture a stream diff cannot show. The address
    itself differs between the two drivers (they allocate in different heaps),
    so it is deliberately not printed as a value to be compared -- the slot
    number and the block's contents are.
    """
    for addr in sorted(state):
        nm = regs.name(addr)
        if not nm.startswith("NTE_DESCRIPTOR_ADDR["):
            continue
        gpu = state[addr]
        if not gpu:
            continue
        slot = nm[len("NTE_DESCRIPTOR_ADDR["):-1]
        body = mem.read(gpu, TEXDESC_BYTES)
        print("==== texture descriptor, slot %s (at 0x%08x) ====" % (slot, gpu))
        if body is None:
            print("  not in the memory dump")
            continue
        words = struct.unpack("<%uI" % (TEXDESC_BYTES // 4), body)
        for i, w in enumerate(words):
            if w:
                print("  +0x%02x %-24s 0x%08x" % (i * 4, texdesc.name(i * 4), w))


def final_state(words, regs, state):
    """The value each register is left holding, one register per entry.

    A LOAD_STATE of n words sets n consecutive registers, so it is expanded
    here: that is what makes two streams comparable line by line even when one
    driver batches a bank into one command and the other writes it singly.
    """
    i, n = 0, len(words)
    while i < n:
        w = words[i]
        op = w >> 27
        if op == 0x01:
            count = (w >> 16) & 0x3ff
            if count == 0:
                count = 0x400
            addr = (w & 0xffff) * 4
            for j in range(count):
                if i + 1 + j < n:
                    state[addr + j * 4] = words[i + 1 + j]
            i += 1 + count
            if (1 + count) % 2:
                i += 1
            continue
        i += 1
        if op == 0x09:
            i += 1
    return state


def decode(words, out, regs, uniforms_only=False):
    i = 0
    n = len(words)
    while i < n:
        w = words[i]
        op = w >> 27
        name = OPCODES.get(op)
        if op == 0x01:
            count = (w >> 16) & 0x3ff
            if count == 0:
                count = 0x400
            fixp = (w >> 26) & 1
            addr = (w & 0xffff) * 4
            vals = words[i + 1:i + 1 + count]
            if not uniforms_only or regs.is_uniform(addr):
                out.append("LOAD_STATE 0x%05x %-30s count=%-3u%s %s" % (
                    addr, regs.name(addr), count, " FIXP" if fixp else "",
                    " ".join("0x%08x" % v for v in vals[:16]) +
                    (" ..." if len(vals) > 16 else "")))
            i += 1 + count
            if (1 + count) % 2:
                i += 1  # the stream is padded to an even number of words
            continue
        if not uniforms_only:
            out.append("%-12s 0x%08x" % (name or "OP_0x%02x" % op, w))
        i += 1
        if op == 0x09:
            i += 1  # STALL carries the source/destination word after it
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("path")
    ap.add_argument("--raw", action="store_true",
                    help="one bare stream, not the shim's length-prefixed dump")
    ap.add_argument("--uniforms-only", action="store_true",
                    help="only writes into a constant-file window")
    ap.add_argument("--submit", type=int, default=None,
                    help="decode only this submit (0-based)")
    ap.add_argument("--final-state", action="store_true",
                    help="print the value each register is left holding, "
                         "one per line, for diffing two streams")
    ap.add_argument("--regs", default=DEFAULT_REGS,
                    help="Mesa's state_3d.xml.h, which the names come from")
    ap.add_argument("--regs2", default=DEFAULT_REGS2,
                    help="Mesa's state.xml.h, which names the GL block: the "
                         "semaphore, the stall token and GL_FLUSH_CACHE")
    ap.add_argument("--mem", action="append", default=[],
                    help="a memory dump (ETNA_SHIM_MEM_DUMP / YAKOGL_MEM_DUMP): "
                         "the blocks the stream only points at")
    ap.add_argument("--tex-desc", action="store_true",
                    help="follow each NTE_DESCRIPTOR_ADDR into --mem and print "
                         "the 256-byte texture descriptor it names")
    ap.add_argument("--texdesc-regs", default=DEFAULT_TEXDESC,
                    help="Mesa's texdesc_3d.xml.h, which the field names come from")
    args = ap.parse_args()

    if args.tex_desc:
        args.final_state = True
        if not args.mem:
            sys.exit("--tex-desc needs a --mem dump to read the descriptor from")

    regs = Regs(args.regs if args.regs and os.path.exists(args.regs) else None,
                args.regs2 if args.regs2 and os.path.exists(args.regs2) else None)
    # No dump means the driver submitted nothing. That is an answer -- it is
    # how the missing depth-only clear was found -- so say it rather than
    # failing with a traceback that reads like a broken tool.
    if not os.path.exists(args.path):
        sys.exit(f"{args.path}: no command stream was submitted")
    data = open(args.path, "rb").read()
    submits = []
    if args.raw:
        submits.append(data)
    else:
        off = 0
        while off + 4 <= len(data):
            (length,) = struct.unpack_from("<I", data, off)
            off += 4
            submits.append(data[off:off + length])
            off += length

    state = {}
    for idx, s in enumerate(submits):
        if args.submit is not None and idx != args.submit:
            continue
        words = struct.unpack("<%uI" % (len(s) // 4), s[:len(s) // 4 * 4])
        if args.final_state:
            final_state(words, regs, state)
            continue
        print("==== submit %u: %u words ====" % (idx, len(words)))
        out = []
        decode(words, out, regs, args.uniforms_only)
        for line in out:
            print(line)

    if args.tex_desc:
        texdesc = TexDesc(args.texdesc_regs) if os.path.exists(args.texdesc_regs) \
            else None
        if texdesc is None:
            sys.exit("no texdesc_3d.xml.h at %s" % args.texdesc_regs)
        print_tex_descs(state, regs, Mem(args.mem), texdesc)
    elif args.final_state:
        for addr in sorted(state):
            print("0x%05x %-40s 0x%08x" % (addr, regs.name(addr), state[addr]))


if __name__ == "__main__":
    sys.exit(main())
