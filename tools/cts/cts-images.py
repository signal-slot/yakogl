#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Pull the images out of a dEQP .qpa, so a "Got faulty pixels" failure can be
looked at instead of guessed about.

  cts-images.py --out DIR result.qpa [more.qpa ...]

dEQP embeds each image as base64 inside <Image ...>...</Image>, grouped in an
<ImageSet>; a rendering case that fails normally carries the rendered result,
the reference the suite expected, and often an error mask. They are written as
  <out>/<case name>/<image set>.<image name>.png
which keeps a case's set together and sorts sensibly.

Only PNG-compressed images are written. dEQP can also emit raw pixel data for
some formats; those are reported and skipped rather than written wrong, because
a mislabelled image is worse than a missing one.
"""
import argparse
import base64
import os
import re
import sys

CASE_RE = re.compile(rb"#beginTestCaseResult\s+(\S+)")
SET_RE = re.compile(rb'<ImageSet\b[^>]*\bName="([^"]*)"', re.I)
IMG_RE = re.compile(
    rb'<Image\b([^>]*)>(.*?)</Image>', re.I | re.DOTALL)
ATTR_RE = re.compile(rb'(\w+)\s*=\s*"([^"]*)"')


def safe(name):
    return re.sub(r"[^A-Za-z0-9._-]", "_", name)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("qpa", nargs="+")
    ap.add_argument("--out", required=True)
    ap.add_argument("--only", help="only cases whose name contains this")
    args = ap.parse_args()

    written = skipped = 0
    for path in args.qpa:
        data = open(path, "rb").read()
        # Split the log into per-case blobs, keeping the case name.
        marks = [(m.start(), m.group(1).decode()) for m in CASE_RE.finditer(data)]
        for i, (pos, case) in enumerate(marks):
            end = marks[i + 1][0] if i + 1 < len(marks) else len(data)
            if args.only and args.only not in case:
                continue
            blob = data[pos:end]
            setname = "images"
            ms = SET_RE.search(blob)
            if ms:
                setname = ms.group(1).decode("utf-8", "replace")
            for attrs, body in IMG_RE.findall(blob):
                a = {k.decode().lower(): v.decode("utf-8", "replace")
                     for k, v in ATTR_RE.findall(attrs)}
                if a.get("compressionmode", "").upper() != "PNG":
                    skipped += 1
                    continue
                d = os.path.join(args.out, safe(case))
                os.makedirs(d, exist_ok=True)
                fn = "%s.%s.png" % (safe(setname), safe(a.get("name", "image")))
                with open(os.path.join(d, fn), "wb") as f:
                    f.write(base64.b64decode(b"".join(body.split())))
                written += 1

    print("wrote %d image(s) under %s" % (written, args.out))
    if skipped:
        print("skipped %d non-PNG image(s) (raw pixel data, not written)" % skipped)
    return 0 if written else 1


if __name__ == "__main__":
    sys.exit(main())
