#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""ramlog_decode.py - the console text of a RAM-log ring dump.

    python3 -I ramlog_decode.py DUMPFILE > console.txt

The dump is the ring region as U-Boot saved it (see README.md). Little
endian, a 32-byte header of u32 magic 0x474C4B59 ("YKLG" as bytes), version
(1), size, head (total bytes written, mod 2^32), seq, flags (bit0 wrapped),
two zero words, then `size` data bytes. Stream byte i is data[i % size].

  head <= size : the text is data[:head]
  head >  size : the last `size` bytes in stream order, and the line
                 "[ramlog: N bytes lost]" (N = head - size) on stderr

Exit 0 ok; 3 bad magic or version ("ramlog: no ring" on stderr); 4 size
inconsistent with the file length (the file is shorter than 32 + size).
Bytes after the ring in the file are ignored. Undecodable bytes become
U+FFFD, the output is UTF-8.
"""
import struct
import sys

MAGIC = 0x474C4B59
HDR = 32


def decode(blob):
    """-> (exit code, text bytes, stderr line or None)"""
    if len(blob) < HDR:
        return 3, b"", "ramlog: no ring"
    magic, version, size, head, _seq, _flags, _r0, _r1 = struct.unpack_from("<8I", blob)
    if magic != MAGIC or version != 1:
        return 3, b"", "ramlog: no ring"
    if size == 0 or len(blob) < HDR + size:
        return 4, b"", "ramlog: size %d does not fit the %d-byte file" % (size, len(blob))
    data = blob[HDR:HDR + size]
    if head <= size:
        return 0, data[:head], None
    start = head % size
    return 0, data[start:] + data[:start], "[ramlog: %d bytes lost]" % (head - size)


def main(argv):
    if len(argv) != 2 or argv[1] in ("-h", "--help"):
        sys.stderr.write(__doc__)
        return 2
    try:
        with open(argv[1], "rb") as f:
            blob = f.read()
    except OSError as e:
        sys.stderr.write("ramlog: %s\n" % e)
        return 2
    rc, text, err = decode(blob)
    if err:
        sys.stderr.write(err + "\n")
    if rc == 0:
        sys.stdout.buffer.write(text.decode("utf-8", "replace").encode("utf-8"))
        sys.stdout.buffer.flush()
    return rc


if __name__ == "__main__":
    sys.exit(main(sys.argv))
