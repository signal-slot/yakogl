#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Read-only reader of U-Boot's EFI variable file ubootefi.var.

Never writes: the file is opened 'rb' only.

Usage: efi_varfile.py [--fwu] [--list] FILE
  (default / --list)  one line per variable:
                      <guid> attr=0x<hex> len=<n> <name> = <hex data>
  --fwu               one line: 'fwu-trial: counter <n>' or 'fwu-trial: absent'
  exit 0 ok, 2 bad file ('efivar: bad file: <why>' on stderr), 64 usage.

Sources (ST U-Boot v2023.10-stm32mp-r1, same as upstream v2023.10):
  File format, include/efi_variable.h:
    :95   EFI_VAR_FILE_NAME  "ubootefi.var" (on the EFI system partition,
          lib/efi_loader/efi_var_file.c:127-146)
    :103  EFI_VAR_FILE_MAGIC 0x0161566966456255 (u64, little endian:
          bytes 55 62 45 66 69 56 61 01 "UbEfiVa", version 1)
    :114-120 struct efi_var_entry {u32 length; u32 attr; u64 time;
          efi_guid_t guid; u16 name[];}  length = DATA length only
    :131-137 struct efi_var_file {u64 reserved; u64 magic; u32 length;
          u32 crc32; entries[]}  header 24 bytes, little endian
  Writer, lib/efi_loader/efi_var_file.c:56-118 (efi_var_collect): the data
    follows the NUL-terminated UTF-16 name; the next entry starts at
    ALIGN(data end, 8) (:104); reserved = 0 (:108); length = whole file
    including header (:110,113); crc32 = crc32(0, entries, length - 24)
    (:111-112), the standard zlib CRC-32.
  Checker, efi_var_file.c:158-169 (efi_var_restore): reserved must be 0,
    magic and crc32 must match, else "Invalid EFI variables file".
  FWU trial counter, lib/fwu_updates/fwu.c:
    :42-45  efi_set_variable_int(u"TrialStateCtr", &efi_global_variable_guid,
            NV|BOOTSERVICE_ACCESS, sizeof(u16), ...): a u16, deleted
            (:742 trial_counter_update(NULL)) when the platform leaves
            trial state, so present means a trial is running.
    :55-69  trial_counter_read; :672-680 fwu_trial_state_ctr_start sets 0;
    :74-95  fwu_trial_count_update increments it per boot, compares with
            CONFIG_FWU_TRIAL_STATE_CNT (lib/fwu_updates/Kconfig:26).
  Name TrialStateCtr, GUID 8be4df61-93ca-11d2-aa0d-00e098032b8c
    (EFI_GLOBAL_VARIABLE_GUID, include/efi_api.h:413-415).
"""
import struct
import sys
import zlib

MAGIC = 0x0161566966456255
HDR = 24
ENT_HDR = 32
FWU_NAME = "TrialStateCtr"
FWU_GUID = "8be4df61-93ca-11d2-aa0d-00e098032b8c"


class BadFile(Exception):
    pass


def fmt_guid(b):
    a, b2, c = struct.unpack_from("<IHH", b, 0)
    return "%08x-%04x-%04x-%s-%s" % (a, b2, c, b[8:10].hex(), b[10:16].hex())


def parse(buf):
    if len(buf) < HDR:
        raise BadFile("truncated: %d bytes, header needs %d" % (len(buf), HDR))
    reserved, magic, length, crc = struct.unpack_from("<QQII", buf, 0)
    if reserved:
        raise BadFile("reserved field not 0")
    if magic != MAGIC:
        raise BadFile("bad magic 0x%016x" % magic)
    if length < HDR:
        raise BadFile("length %d below header size" % length)
    if length > len(buf):
        raise BadFile("truncated: header length %d, file %d bytes"
                      % (length, len(buf)))
    got = zlib.crc32(buf[HDR:length]) & 0xFFFFFFFF
    if got != crc:
        raise BadFile("crc32 mismatch: header 0x%08x, computed 0x%08x"
                      % (crc, got))
    out = []
    off = HDR
    while off < length:
        if off + ENT_HDR > length:
            raise BadFile("entry header at %d runs past length" % off)
        dlen, attr, _time = struct.unpack_from("<IIQ", buf, off)
        guid = fmt_guid(buf[off + 8 + 8:off + 32])
        p = off + ENT_HDR
        end = p
        while True:
            if end + 2 > length:
                raise BadFile("unterminated name at %d" % off)
            if buf[end:end + 2] == b"\0\0":
                break
            end += 2
        try:
            name = buf[p:end].decode("utf-16-le")
        except UnicodeDecodeError:
            raise BadFile("bad UTF-16 name at %d" % off)
        dstart = end + 2
        if dstart + dlen > length:
            raise BadFile("entry at %d: data runs past length" % off)
        out.append({"guid": guid, "attr": attr, "name": name,
                    "data": bytes(buf[dstart:dstart + dlen])})
        off = (dstart + dlen + 7) & ~7
    return out


def main(argv):
    fwu = False
    path = None
    for a in argv:
        if a == "--fwu":
            fwu = True
        elif a == "--list":
            pass
        elif a.startswith("-") or path is not None:
            sys.stderr.write("usage: efi_varfile.py [--fwu] [--list] FILE\n")
            return 64
        else:
            path = a
    if path is None:
        sys.stderr.write("usage: efi_varfile.py [--fwu] [--list] FILE\n")
        return 64
    try:
        with open(path, "rb") as f:
            buf = f.read()
        vs = parse(buf)
    except BadFile as e:
        sys.stderr.write("efivar: bad file: %s\n" % e)
        return 2
    except OSError as e:
        sys.stderr.write("efivar: bad file: %s\n" % e)
        return 2
    if fwu:
        for v in vs:
            if v["name"] == FWU_NAME and v["guid"] == FWU_GUID:
                d = v["data"]
                if len(d) != 2:
                    sys.stderr.write("efivar: bad file: %s is %d bytes, "
                                     "expected 2\n" % (FWU_NAME, len(d)))
                    return 2
                print("fwu-trial: counter %d" % struct.unpack("<H", d)[0])
                return 0
        print("fwu-trial: absent")
        return 0
    for v in vs:
        print("%s attr=0x%x len=%d %s = %s" % (
            v["guid"], v["attr"], len(v["data"]), v["name"],
            v["data"][:32].hex()))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
