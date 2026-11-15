#!/usr/bin/env python3
"""mboxdump: decode recorded VideoCore property-mailbox buffers (Pi 4 / Pi 5).

Usage: mboxdump.py [--strict] [FILE...]      (no FILE or "-": stdin)

Input format: one or more property buffers as 32-bit little-endian words,
whitespace separated, written 0x-prefixed or bare hex ("0x00000020" or
"20"; bare words are always hex). '#' starts a comment to end of line.
Buffers follow each other; a buffer's extent is its first word (total size
in bytes), so no separator is needed (blank lines are fine).

Buffer layout (raspberrypi/firmware wiki, "Mailbox property interface"):
  word 0   buffer size in bytes (including these 8 header bytes and the
           end tag)
  word 1   request/response code: 0 request, 0x80000000 success,
           0x80000001 error parsing the request
  tags     id, value buffer size in bytes, request/response word
           (bit 31 = the firmware answered this tag, bits 0-30 = length of
           the value it wrote, in bytes), then the value buffer
           (size bytes, a multiple of 4)
  end tag  one zero word

Tag ids and names follow driver/zephyr/lib/board/rpi4/rpi_mbox.c and
rpi_mbox.h (the framebuffer, clock and throttle tags used there; rpi_mbox.h
cites include/soc/bcm2835/raspberrypi-firmware.h for the numbers); the
layouts are those of the wiki page above.

Flags printed as "!!": a tag in a response buffer (code != 0) whose response
bit is clear (the firmware did not answer it), a tag answered with fewer
bytes than the tag defines (short), a response code of 0x80000001. With
--strict any flag makes the exit status 1. Exit status 2: malformed input
(bad hex, truncated or inconsistent buffer, tag running past the buffer,
missing end tag). Standard library only.
"""
import sys

# id: (name, response bytes the tag defines, [field names])
TAGS = {
    0x00000001: ("GET_FIRMWARE_REVISION", 4, ["revision"]),
    0x00010001: ("GET_BOARD_MODEL", 4, ["model"]),
    0x00010002: ("GET_BOARD_REVISION", 4, ["revision"]),
    0x00010003: ("GET_BOARD_MAC_ADDRESS", 6, ["mac_lo", "mac_hi"]),
    0x00010004: ("GET_BOARD_SERIAL", 8, ["serial_lo", "serial_hi"]),
    0x00010005: ("GET_ARM_MEMORY", 8, ["base", "size"]),
    0x00010006: ("GET_VC_MEMORY", 8, ["base", "size"]),
    0x00030001: ("GET_CLOCK_STATE", 8, ["clock_id", "state"]),
    0x00030002: ("GET_CLOCK_RATE", 8, ["clock_id", "hz"]),
    0x00030004: ("GET_MAX_CLOCK_RATE", 8, ["clock_id", "hz"]),
    0x00030006: ("GET_MIN_CLOCK_RATE", 8, ["clock_id", "hz"]),
    0x00030046: ("GET_THROTTLED", 4, ["throttled"]),
    0x00030047: ("GET_CLOCK_RATE_MEASURED", 8, ["clock_id", "hz"]),
    0x00038001: ("SET_CLOCK_STATE", 8, ["clock_id", "state"]),
    0x00038002: ("SET_CLOCK_RATE", 12, ["clock_id", "hz", "skip_turbo"]),
    0x00040001: ("FRAMEBUFFER_ALLOCATE", 8, ["bus_addr", "size"]),
    0x00040002: ("FRAMEBUFFER_BLANK", 4, ["state"]),
    0x00040003: ("FRAMEBUFFER_GET_PHYSICAL_WIDTH_HEIGHT", 8, ["width", "height"]),
    0x00040004: ("FRAMEBUFFER_GET_VIRTUAL_WIDTH_HEIGHT", 8, ["width", "height"]),
    0x00040005: ("FRAMEBUFFER_GET_DEPTH", 4, ["bpp"]),
    0x00040006: ("FRAMEBUFFER_GET_PIXEL_ORDER", 4, ["order"]),
    0x00040008: ("FRAMEBUFFER_GET_PITCH", 4, ["pitch"]),
    0x00040009: ("FRAMEBUFFER_GET_VIRTUAL_OFFSET", 8, ["x", "y"]),
    0x00048001: ("FRAMEBUFFER_RELEASE", 0, []),
    0x00048003: ("FRAMEBUFFER_SET_PHYSICAL_WIDTH_HEIGHT", 8, ["width", "height"]),
    0x00048004: ("FRAMEBUFFER_SET_VIRTUAL_WIDTH_HEIGHT", 8, ["width", "height"]),
    0x00048005: ("FRAMEBUFFER_SET_DEPTH", 4, ["bpp"]),
    0x00048006: ("FRAMEBUFFER_SET_PIXEL_ORDER", 4, ["order"]),
    0x00048009: ("FRAMEBUFFER_SET_VIRTUAL_OFFSET", 8, ["x", "y"]),
    0x0004800e: ("FRAMEBUFFER_SET_VSYNC", 4, ["value"]),
    # block number, status, then one 128-byte EDID block
    0x00030020: ("GET_EDID_BLOCK", 136, ["block", "status"]),
}
ORDER = {0: "BGR", 1: "RGB"}
THROTTLE_BITS = ((0, "undervoltage"), (1, "freq-capped"), (2, "throttled"),
                 (3, "soft-templimit"), (16, "undervoltage-seen"),
                 (17, "freq-cap-seen"), (18, "throttle-seen"),
                 (19, "soft-templimit-seen"))


class Malformed(Exception):
    pass


def parse_words(text):
    out = []
    for ln, line in enumerate(text.splitlines(), 1):
        line = line.split("#", 1)[0]
        for tok in line.split():
            try:
                v = int(tok, 16)
            except ValueError:
                raise Malformed("line %d: bad hex word %r" % (ln, tok))
            if not 0 <= v <= 0xFFFFFFFF:
                raise Malformed("line %d: word %r over 32 bits" % (ln, tok))
            out.append(v)
    return out


def describe(tid, vals):
    fields = TAGS[tid][2]
    parts = []
    for i, v in enumerate(vals):
        f = fields[i] if i < len(fields) else "v%d" % i
        s = "%s=0x%x" % (f, v)
        if f == "order" and v in ORDER:
            s += "(%s)" % ORDER[v]
        if tid == 0x00030046 and i == 0:
            bits = [n for b, n in THROTTLE_BITS if v >> b & 1]
            s += "(%s)" % (",".join(bits) or "clean")
        parts.append(s)
    return " ".join(parts)


def decode_buffer(w, pos, idx, out):
    """Decode the buffer at w[pos:]; returns (next pos, flag count)."""
    flags = 0
    if len(w) - pos < 3:
        raise Malformed("buffer %d: fewer than 3 words left" % idx)
    size, code = w[pos], w[pos + 1]
    if size % 4 or size < 12:
        raise Malformed("buffer %d: bad size 0x%x" % (idx, size))
    n = size // 4
    if pos + n > len(w):
        raise Malformed("buffer %d: size 0x%x but only %d words given"
                        % (idx, size, len(w) - pos))
    b = w[pos:pos + n]
    cname = {0: "request", 0x80000000: "success",
             0x80000001: "error"}.get(code, "unknown")
    out.append("buffer %d: size=%d code=0x%08x (%s)" % (idx, size, code, cname))
    if code == 0x80000001:
        out.append("  !! firmware reported an error parsing the request")
        flags += 1
    i = 2
    while True:
        if i >= n:
            raise Malformed("buffer %d: no end tag" % idx)
        tid = b[i]
        if tid == 0:
            if i != n - 1:
                out.append("  note: %d word(s) of padding after end tag"
                           % (n - 1 - i))
            break
        if i + 3 > n:
            raise Malformed("buffer %d: tag 0x%x header past buffer end"
                            % (idx, tid))
        bsz, rr = b[i + 1], b[i + 2]
        if bsz % 4 or i + 3 + bsz // 4 > n:
            raise Malformed("buffer %d: tag 0x%08x value size %d runs past "
                            "the buffer" % (idx, tid, bsz))
        resp = bool(rr >> 31)
        rlen = rr & 0x7FFFFFFF
        vals = b[i + 3:i + 3 + bsz // 4]
        name = TAGS[tid][0] if tid in TAGS else "UNKNOWN"
        line = "  tag 0x%08x %s: bufsize=%d %s len=%d" % (
            tid, name, bsz, "response" if resp else "request", rlen)
        shown = vals[:(rlen + 3) // 4] if resp and rlen < bsz else vals
        if tid == 0x00030020:
            line += " " + describe(tid, shown[:2]) + \
                " edid_bytes=%d" % max(0, rlen - 8)
        elif tid in TAGS:
            line += " " + describe(tid, shown)
        else:
            line += " values=" + " ".join("0x%x" % v for v in shown)
        out.append(line)
        if code != 0 and not resp:
            out.append("  !! tag 0x%08x not answered (response bit clear)"
                       % tid)
            flags += 1
        elif resp and tid in TAGS and rlen < TAGS[tid][1] <= bsz:
            out.append("  !! tag 0x%08x answered short: %d of %d bytes"
                       % (tid, rlen, TAGS[tid][1]))
            flags += 1
        i += 3 + bsz // 4
    return pos + n, flags


def decode(text, out):
    w = parse_words(text)
    if not w:
        raise Malformed("no words in input")
    pos, idx, flags = 0, 0, 0
    while pos < len(w):
        pos, f = decode_buffer(w, pos, idx, out)
        flags += f
        idx += 1
    return flags


def main(argv):
    strict = False
    files = []
    for a in argv:
        if a == "--strict":
            strict = True
        elif a in ("-h", "--help"):
            sys.stdout.write(__doc__)
            return 0
        else:
            files.append(a)
    out = []
    try:
        texts = []
        for f in files or ["-"]:
            if f == "-":
                texts.append(sys.stdin.read())
            else:
                with open(f) as fh:
                    texts.append(fh.read())
        flags = decode("\n".join(texts), out)
    except Malformed as e:
        if out:
            print("\n".join(out))
        sys.stderr.write("mboxdump: malformed: %s\n" % e)
        return 2
    except OSError as e:
        sys.stderr.write("mboxdump: %s\n" % e)
        return 2
    print("\n".join(out))
    print("%d flag(s)" % flags)
    return 1 if strict and flags else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
