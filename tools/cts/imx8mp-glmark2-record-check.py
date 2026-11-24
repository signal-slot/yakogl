#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check that an i.MX8MP glmark2 by-eye record directory is complete and redacted.

  imx8mp-glmark2-record-check.py RECORD_DIR

RECORD_DIR is records/imx8mp/<YYYYMMDD-HHMM>-<name>-<sha8>/ (gitignored): the
layout of tools/board.sh (RD="$LOGDIR/$(date +%Y%m%d-%H%M)-$NAME-$sha8", meta.txt
with key=value lines) plus the camera pictures of the by-eye session.

Checks (one output line per problem, never the matched text of a redaction hit):
  name        <YYYYMMDD-HHMM>-<name>-<sha8>: sha8 is 8 lowercase hex digits
  commit      the recorded origin/main commit (file `commit`, or a meta.txt line
              commit=, origin_main= or build=) starts with the name's sha8
  console     a text log (*.log, *.txt, *.out, *.err, any depth) holds a
              `glmark2-picture: <scene info>:` line for each of the 33 scenes
  photograph  one PNG per scene, valid (signature, chunk CRCs, IHDR first, IEND
              last, IDAT inflates), see the naming scheme below
  sha256      the image's sha256: a file `sha256`, `image.sha256` or `*.sha256`
              holding a 64-hex digest, or a meta.txt line image_sha256=<64 hex>
  redaction   no text file holds an IPv4 address (dotted quads, version strings
              included, as imx8mp-gles3-redact.py), /home/, /Users/, /dev/tty,
              a user@host, or the host name in $HOSTNAME (if set, 3+ characters)

Photograph naming scheme (matches glmark2-fb.py --rundir's <NN>-<slug>.png):
  photos/<NN>-<slug>.png, directly in the record directory (or in photos/)
  NN    the scene's number 01..33 in the canonical order below (the host column
        of tools/cts/glmark2-pictures.txt; `imx8mp-glmark2-dumps.sh scenes`
        lists them as gm01..gm33), two digits
  slug  the scene's info string lowercased, every run of non-[a-z0-9] turned
        into '-', stripped of '-', cut at 48 characters (glmark2-fb.py slug())
  Only the NN prefix is matched (NN.png and NN-anything.png are accepted); the
  slug is for the reader. A second file for the same NN is a problem.

Last line: `record-check: <dir>: complete|incomplete (<n> problems)`.
Exit 0 complete, 1 incomplete, 2 bad argument.
"""
import os
import re
import socket
import struct
import sys
import zlib

SCENES = [
    "[build] use-vbo=false",
    "[build] use-vbo=true",
    "[texture] texture-filter=nearest",
    "[texture] texture-filter=linear",
    "[texture] texture-filter=mipmap",
    "[shading] shading=gouraud",
    "[shading] shading=blinn-phong-inf",
    "[shading] shading=phong",
    "[shading] shading=cel",
    "[bump] bump-render=high-poly",
    "[bump] bump-render=normals",
    "[bump] bump-render=height",
    "[effect2d] kernel=0,1,0;1,-4,1;0,1,0;",
    "[effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;",
    "[pulsar] light=false:quads=5:texture=false",
    "[desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4",
    "[desktop] effect=shadow:windows=4",
    "[buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map",
    "[buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata",
    "[buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map",
    "[ideas] speed=duration",
    "[jellyfish] <default>",
    "[terrain] <default>",
    "[shadow] <default>",
    "[refract] <default>",
    "[conditionals] fragment-steps=0:vertex-steps=0",
    "[conditionals] fragment-steps=5:vertex-steps=0",
    "[conditionals] fragment-steps=0:vertex-steps=5",
    "[function] fragment-complexity=low:fragment-steps=5",
    "[function] fragment-complexity=medium:fragment-steps=5",
    "[loop] fragment-loop=false:fragment-steps=5:vertex-steps=5",
    "[loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5",
    "[loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5",
]

NAME_RE = re.compile(r"^(\d{8}-\d{4})-([A-Za-z0-9][A-Za-z0-9._-]*)-([0-9a-f]{8})$")
TEXT_EXT = (".log", ".txt", ".out", ".err")
HEX64 = re.compile(r"\b[0-9a-f]{64}\b")
REDACT = [
    ("ipv4", re.compile(r"([0-9]{1,3}\.){3}[0-9]{1,3}")),
    ("home-path", re.compile(r"/home/")),
    ("users-path", re.compile(r"/Users/")),
    ("tty-path", re.compile(r"/dev/tty")),
    ("login", re.compile(r"[A-Za-z0-9._-]+@[A-Za-z0-9-]+")),
]


def slug(text):
    return re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")[:48]


def read_text(path):
    """The file's text, or None for a binary file (a NUL byte, as grep -I)."""
    with open(path, "rb") as f:
        data = f.read()
    if b"\0" in data:
        return None
    return data.decode("utf-8", "replace")


def all_files(root):
    for d, _, names in os.walk(root):
        for n in sorted(names):
            yield os.path.join(d, n)


def png_problem(path):
    """None if path is a valid PNG, else a short reason."""
    with open(path, "rb") as f:
        data = f.read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        return "not a PNG (bad signature)"
    pos, first, idat, last = 8, True, b"", None
    while pos < len(data):
        if pos + 8 > len(data):
            return "truncated chunk header"
        n, typ = struct.unpack(">I4s", data[pos:pos + 8])
        end = pos + 8 + n + 4
        if end > len(data):
            return "truncated chunk " + typ.decode("latin1")
        body = data[pos + 8:pos + 8 + n]
        crc = struct.unpack(">I", data[end - 4:end])[0]
        if zlib.crc32(typ + body) & 0xFFFFFFFF != crc:
            return "bad CRC in " + typ.decode("latin1")
        if first:
            if typ != b"IHDR" or n != 13:
                return "IHDR not first"
            w, h = struct.unpack(">II", body[:8])
            if w == 0 or h == 0:
                return "zero size"
            first = False
        if typ == b"IDAT":
            idat += body
        last = typ
        pos = end
    if first:
        return "no chunks"
    if last != b"IEND":
        return "IEND missing"
    try:
        zlib.decompress(idat)
    except zlib.error:
        return "IDAT does not inflate"
    return None


def check(rd):
    probs = []
    base = os.path.basename(os.path.normpath(rd))
    m = NAME_RE.match(base)
    sha8 = None
    if not m:
        probs.append("name: '%s' is not <YYYYMMDD-HHMM>-<name>-<sha8>" % base)
    else:
        sha8 = m.group(3)

    files = list(all_files(rd))
    rel = lambda p: os.path.relpath(p, rd)
    texts = {}
    for p in files:
        if p.lower().endswith(".png"):
            continue
        t = read_text(p)
        if t is not None:
            texts[p] = t

    # commit
    commits = []
    cf = os.path.join(rd, "commit")
    if cf in texts and texts[cf].strip():
        commits.append(texts[cf].split()[0])
    mt = os.path.join(rd, "meta.txt")
    if mt in texts:
        for line in texts[mt].splitlines():
            k, _, v = line.partition("=")
            if k in ("commit", "origin_main", "build"):
                commits.append(v.strip())
    hexc = [c for c in commits if re.fullmatch(r"[0-9a-f]{8,40}", c)]
    if not hexc:
        probs.append("commit: no recorded origin/main commit (file commit, or commit=/origin_main=/build= in meta.txt)")
    elif sha8 and not any(c.startswith(sha8) for c in hexc):
        probs.append("commit: recorded commit does not start with the name's sha8")

    # console
    logs = [p for p in texts if p.lower().endswith(TEXT_EXT)]
    blob = "\n".join(l for p in logs for l in texts[p].splitlines() if "glmark2-picture:" in l)
    for i, info in enumerate(SCENES, 1):
        if ("glmark2-picture: " + info + ": ") not in blob:
            probs.append("console: no glmark2-picture line for scene %02d %s" % (i, info))

    # photographs
    found = {}
    for p in files:
        d = os.path.relpath(os.path.dirname(p), rd)
        mm = re.fullmatch(r"(\d\d)(-[A-Za-z0-9._-]*)?\.png", os.path.basename(p))
        if mm and d in (".", "photos"):
            found.setdefault(int(mm.group(1)), []).append(p)
    for n in sorted(found):
        if not 1 <= n <= len(SCENES):
            probs.append("photograph: %s has no scene (NN is 01..%d)" % (rel(found[n][0]), len(SCENES)))
    for i, info in enumerate(SCENES, 1):
        ps = found.get(i, [])
        if not ps:
            probs.append("photograph: missing %02d-%s.png (scene %s)" % (i, slug(info), info))
        elif len(ps) > 1:
            probs.append("photograph: scene %02d has %d files" % (i, len(ps)))
        else:
            why = png_problem(ps[0])
            if why:
                probs.append("photograph: %s: %s" % (rel(ps[0]), why))

    # sha256
    ok = False
    for p, t in texts.items():
        b = os.path.basename(p)
        if (b in ("sha256", "image.sha256") or b.endswith(".sha256")) and HEX64.search(t):
            ok = True
    if mt in texts and re.search(r"^image_sha256=[0-9a-f]{64}\s*$", texts[mt], re.M):
        ok = True
    if not ok:
        probs.append("sha256: no image sha256 (file sha256 / *.sha256 with a 64-hex digest, or image_sha256= in meta.txt)")

    # redaction (file, line and pattern name only)
    host = os.environ.get("HOSTNAME") or ""
    if not host:
        try:
            host = socket.gethostname()
        except OSError:
            host = ""
    pats = list(REDACT)
    if len(host) >= 3:
        pats.append(("host-name", re.compile(r"(?<![A-Za-z0-9-])" + re.escape(host) + r"(?![A-Za-z0-9-])", re.I)))
    for p, t in texts.items():
        for ln, line in enumerate(t.splitlines(), 1):
            for name, rx in pats:
                if rx.search(line):
                    probs.append("redaction: %s:%d: %s" % (rel(p), ln, name))
        for name, rx in pats:  # a hit in the file name itself
            if rx.search(rel(p)):
                probs.append("redaction: %s: %s in the file name" % (rel(p), name))
    return probs


def main(argv):
    if len(argv) != 2 or argv[1] in ("-h", "--help"):
        print("usage: imx8mp-glmark2-record-check.py RECORD_DIR", file=sys.stderr)
        return 2
    rd = argv[1]
    if not os.path.isdir(rd):
        print("record-check: not a directory: " + rd, file=sys.stderr)
        return 2
    probs = check(rd)
    for p in probs:
        print(p)
    print("record-check: %s: %s (%d problems)" % (rd, "incomplete" if probs else "complete", len(probs)))
    return 1 if probs else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
