#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Read-only reader of U-Boot FWU metadata: is the booted bank accepted?

Input is an FWU metadata image as `dd` reads it from the metadata1 or
metadata2 partition (the partition may be larger than the structure; the
rest is ignored).  The files are opened O_RDONLY and never written.  Stdlib
only.  All integers are little endian (the structures are __packed and are
read with memcpy on the little-endian MP2 A35).

Layout implemented, after U-Boot's include/fwu_mdata.h.  The v1 layout is
the whole of v2023.10 upstream (include/fwu_mdata.h:59-66); the v2 layout
is in ST's U-Boot v2023.10 tree (branch v2023.10-stm32mp,
include/fwu_mdata.h:92-125, built when CONFIG_FWU_MDATA_V1 is off).  It
reached upstream with v2024.01.  Upstream v2023.10 has no v2.  Nothing is
copied from U-Boot; the offsets below are the facts the header states.

v1 (fwu_mdata.h:59-66, entries :22-26 and :38-42)
    0   u32 crc32                 CRC-32 (zlib) of bytes 4..size
    4   u32 version               1
    8   u32 active_index          bank U-Boot boots (fwu_mdata.h:62)
    12  u32 previous_active_index (:63)
    16  img_entry[CONFIG_FWU_NUM_IMAGES_PER_BANK], each:
          image_type_uuid[16], location_uuid[16],
          img_bank_info[CONFIG_FWU_NUM_BANKS], each:
            image_uuid[16], u32 accepted (1 = accepted), u32 reserved
    The size is 16 + n_images * (32 + n_banks * 24).  The header does not
    store it (it is a Kconfig), so the reader tries n_banks 2..4 and
    n_images 1..16 and takes the first size whose CRC matches; --v1-size
    BYTES pins it.  A v1 bank is accepted when every image's `accepted` of
    that bank is 1.

v2 (fwu_mdata.h:111-123)
    0   u32 crc32                 CRC-32 (zlib) of bytes 4..metadata_size (:113)
    4   u32 version               2
    8   u32 active_index
    12  u32 previous_active_index
    16  u32 metadata_size         whole structure with the image descriptors
    20  u16 desc_offset           0 if absent
    22  u16 reserved1
    24  u8  bank_state[4]         (:119)
    28  u32 reserved2
    The descriptors after byte 32 are not interpreted.
    bank_state values (ST/include/fwu.h:84-86, ST/lib/fwu_updates/fwu_v2.c:19-21):
    0xFC accepted, 0xFE valid but not accepted (the trial state, counted by
    CONFIG_FWU_TRIAL_STATE_CNT, ST/lib/fwu_updates/Kconfig:26-29), 0xFF
    invalid.

Exit codes
    0  fwu: bank <n> accepted
    3  fwu: bank <n> in trial (0xFE), refusing   (v1: "(accepted=0)")
    2  unreadable, truncated, bad CRC, copies disagree, unknown version,
       booted bank out of range or in a state other than accepted/trial
    1  usage
"""

import argparse
import os
import struct
import sys
import zlib

MAX_READ = 1 << 20
V2_HDR = 32
V2_TRIAL, V2_ACCEPTED, V2_INVALID = 0xFE, 0xFC, 0xFF


class Bad(Exception):
    pass


def read_ro(path):
    try:
        fd = os.open(path, os.O_RDONLY)
    except OSError as e:
        raise Bad("cannot open %s: %s" % (path, e.strerror))
    try:
        data = b""
        while len(data) < MAX_READ:
            chunk = os.read(fd, MAX_READ - len(data))
            if not chunk:
                break
            data += chunk
    except OSError as e:
        raise Bad("cannot read %s: %s" % (path, e.strerror))
    finally:
        os.close(fd)
    return data


def crc(data, size):
    return zlib.crc32(data[4:size]) & 0xFFFFFFFF


def v1_sizes():
    for banks in (2, 3, 4):
        for imgs in range(1, 17):
            yield 16 + imgs * (32 + banks * 24), banks


def parse(path, v1_size=None):
    """Return a dict: version, active, previous, size, banks (index ->
    'accepted' | 'trial' | 'invalid' | raw), raw bank states, label."""
    data = read_ro(path)
    if len(data) < 16:
        raise Bad("%s: truncated (%d bytes)" % (path, len(data)))
    stored, version, active, prev = struct.unpack_from("<IIII", data, 0)
    if version == 2:
        if len(data) < V2_HDR:
            raise Bad("%s: truncated (%d bytes)" % (path, len(data)))
        msize = struct.unpack_from("<I", data, 16)[0]
        if msize < V2_HDR or msize > MAX_READ:
            raise Bad("%s: metadata_size %d out of range" % (path, msize))
        if len(data) < msize:
            raise Bad("%s: truncated (%d of %d bytes)" % (path, len(data), msize))
        if crc(data, msize) != stored:
            raise Bad("%s: bad CRC" % path)
        raw = list(data[24:28])
        return dict(version=2, active=active, previous=prev, size=msize,
                    raw=raw, data=data[:msize], path=path)
    if version == 1:
        cands = [(v1_size, 2)] if v1_size else list(v1_sizes())
        for size, banks in cands:
            if size < 16 or len(data) < size:
                continue
            if crc(data, size) != stored:
                continue
            n = (size - 16) // (32 + banks * 24)
            accepted = []
            for b in range(banks):
                ok = True
                for i in range(n):
                    off = 16 + i * (32 + banks * 24) + 32 + b * 24 + 16
                    ok = ok and struct.unpack_from("<I", data, off)[0] == 1
                accepted.append(ok)
            return dict(version=1, active=active, previous=prev, size=size,
                        raw=accepted, data=data[:size], path=path)
        if len(data) < 16 + 80:
            raise Bad("%s: truncated (%d bytes)" % (path, len(data)))
        raise Bad("%s: bad CRC" % path)
    raise Bad("%s: unknown version %d" % (path, version))


def bank_status(m, bank):
    """'accepted', 'trial' or raise Bad; returns (status, label)."""
    if bank < 0 or bank >= len(m["raw"]):
        raise Bad("bank %d out of range (%d banks)" % (bank, len(m["raw"])))
    r = m["raw"][bank]
    if m["version"] == 1:
        return ("accepted", "") if r else ("trial", "accepted=0")
    if r == V2_ACCEPTED:
        return "accepted", ""
    if r == V2_TRIAL:
        return "trial", "0xFE"
    raise Bad("bank %d in state 0x%02X (not accepted/trial)" % (bank, r))


def cmd_show(a):
    m = parse(a.image, a.v1_size)
    print("version: %d" % m["version"])
    print("active_index: %d" % m["active"])
    print("previous_active_index: %d" % m["previous"])
    for b, r in enumerate(m["raw"]):
        if m["version"] == 2:
            print("bank %d: 0x%02X" % (b, r))
        else:
            print("bank %d: %s" % (b, "accepted" if r else "not accepted"))
    return 0


def cmd_check(a):
    ms = [parse(p, a.v1_size) for p in [a.image] + ([a.image2] if a.image2 else [])]
    if len(ms) == 2:
        x, y = ms
        if x["data"] != y["data"]:
            raise Bad("copies disagree (%s, %s)" % (x["path"], y["path"]))
    m = ms[0]
    bank = m["active"] if a.booted_bank is None else a.booted_bank
    st, label = bank_status(m, bank)
    if st == "accepted":
        print("fwu: bank %d accepted" % bank)
        return 0
    print("fwu: bank %d in trial (%s), refusing" % (bank, label))
    return 3


def main(argv):
    p = argparse.ArgumentParser(prog="fwu_metadata.py", description=__doc__.split("\n")[0])
    p.add_argument("--v1-size", type=int, default=None)
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("show")
    s.add_argument("image")
    c = sub.add_parser("check")
    c.add_argument("image")
    c.add_argument("image2", nargs="?")
    c.add_argument("--booted-bank", type=int, default=None)
    try:
        a = p.parse_args(argv)
    except SystemExit as e:
        return 1 if e.code else 0
    try:
        return cmd_show(a) if a.cmd == "show" else cmd_check(a)
    except Bad as e:
        print("fwu: %s" % e, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
