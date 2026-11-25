#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Print the hand-typed staging and U-Boot lines of Verdin iMX95 dEQP rows.

  python3 -I tools/cts/imx95-uboot-row.py --storage <dev:part> --stage <path>
          [--conf <verdin_imx95.conf>] <img-dir> <LABEL | LABEL-LABEL>...

<img-dir> is what tools/cts/imx95-deqp-images.sh writes: <LABEL>/zephyr/
zephyr.bin (with zephyr/.config) and <LABEL>.built. --storage (U-Boot's
<dev:part> of the file system the module's Linux stages to) and --stage (the
file name for one label, or a directory ending in / which gets <LABEL>.bin;
several labels need the directory) have no defaults: they are the session's
decision. A range is two labels with the same prefix and a numeric tail
(T6S1R01-T6S1R12). Nothing is run: the ssh side prints with $VI95_SSH as the
placeholder for host and credentials. The run bound is the lifetime
(CONFIG_YAKOGL_BOARD_LIFETIME_S of the image's .config, else of the conf
file) plus the WDOG3 period (CONFIG_YAKOGL_BOARD_ALIVE_S).
A row whose AArch64 header image_size (offset 16) or file exceeds the 256 MiB
window, or that has no header, is refused (exit 1); other rows still print.
"""
import argparse
import hashlib
import os
import posixpath
import re
import struct
import sys

LOAD_ADDR = 0xD0000000
WINDOW = 256 * 1024 * 1024
DEFAULT_CONF = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                            "..", "..", "driver", "samples", "zephyr", "deqp",
                            "verdin_imx95.conf")


def kconfig(path, key):
    try:
        with open(path) as f:
            for line in f:
                m = re.match(r"\s*CONFIG_%s=(\S+)" % key, line)
                if m:
                    return int(m.group(1).strip('"'))
    except OSError:
        pass
    return None


def expand(args):
    out = []
    for a in args:
        m = re.fullmatch(r"([A-Za-z0-9]*?)(\d+)-([A-Za-z0-9]*?)(\d+)", a)
        if m and m.group(1) == m.group(3):
            p, lo, hi = m.group(1), m.group(2), m.group(4)
            if int(hi) < int(lo):
                raise SystemExit("range %s runs backwards" % a)
            out += ["%s%0*d" % (p, len(lo), n)
                    for n in range(int(lo), int(hi) + 1)]
        elif re.fullmatch(r"[A-Za-z0-9]+", a):
            out.append(a)
        else:
            raise SystemExit("bad label or range: %s" % a)
    return out


def row(img, label, storage, stage, conf):
    d = os.path.join(img, label)
    zb = os.path.join(d, "zephyr", "zephyr.bin")
    if not os.path.isfile(zb):
        return None, "%s: no %s" % (label, zb)
    with open(zb, "rb") as fh:
        data = fh.read()
    if len(data) < 64 or data[56:60] != b"ARM\x64":
        return None, "%s: %s has no AArch64 Image header" % (label, zb)
    isize = struct.unpack_from("<Q", data, 16)[0]
    if isize > WINDOW or len(data) > WINDOW:
        return None, ("%s: image_size %d (file %d) exceeds the 256 MiB window "
                      "(%d), refused" % (label, isize, len(data), WINDOW))
    cfg = os.path.join(d, "zephyr", ".config")
    life = kconfig(cfg, "YAKOGL_BOARD_LIFETIME_S")
    wd = kconfig(cfg, "YAKOGL_BOARD_ALIVE_S")
    src = cfg
    if life is None:
        life, src = kconfig(conf, "YAKOGL_BOARD_LIFETIME_S"), conf
    if wd is None:
        wd = kconfig(conf, "YAKOGL_BOARD_ALIVE_S")
    if life is None or wd is None:
        return None, "%s: no lifetime/WDOG3 period in %s or %s" % (label, cfg, conf)
    md5 = hashlib.md5(data).hexdigest()
    size = len(data)
    f = posixpath.join(stage, label + ".bin") if stage.endswith("/") else stage
    built = ""
    try:
        with open(os.path.join(img, label + ".built")) as fh:
            built = fh.read().strip()
    except OSError:
        pass
    o = ["== %s" % label,
         "# host: md5 %s  size 0x%x (%d)  image_size 0x%x  %s"
         % (md5, size, size, isize, zb)]
    if built:
        o.append("# built: %s" % built)
    o += ["# ssh side (module's Linux); $VI95_SSH is host and credentials, set by the session:",
          "$VI95_SSH 'rm -f %s'   # undo: removes the staged file" % f,
          "$VI95_SSH 'cat > %s' < %s" % (f, zb),
          "$VI95_SSH 'md5sum %s'   # expect %s" % (f, md5),
          "# U-Boot prompt:",
          "ext4load mmc %s 0x%x %s" % (storage, LOAD_ADDR, f),
          "md5sum 0x%x 0x%x   # expect %s" % (LOAD_ADDR, size, md5),
          "dcache off; icache flush; go 0x%x" % LOAD_ADDR,
          "# run bound: %d s lifetime + %d s WDOG3 = %d s (from %s)"
          % (life, wd, life + wd, src),
          "# after the row, undo again: $VI95_SSH 'rm -f %s'" % f]
    return "\n".join(o), None


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--storage", required=True, help="U-Boot <dev:part>")
    ap.add_argument("--stage", required=True,
                    help="staging path on the module (trailing / adds <LABEL>.bin)")
    ap.add_argument("--conf", default=DEFAULT_CONF)
    ap.add_argument("img_dir")
    ap.add_argument("labels", nargs="+")
    a = ap.parse_args()
    if not re.fullmatch(r"\d+:\d+", a.storage):
        sys.exit("--storage must be <dev:part>, e.g. 0:2")
    if not a.stage or any(c.isspace() or c == "'" for c in a.stage):
        sys.exit("--stage must be a non-empty path without whitespace or '")
    labels = expand(a.labels)
    if len(labels) > 1 and not a.stage.endswith("/"):
        sys.exit("more than one label needs a --stage directory (ending in /):"
                 " an explicit --stage file name would stage every row to"
                 " one file")
    bad = 0
    for l in labels:
        txt, err = row(a.img_dir, l, a.storage, a.stage, a.conf)
        if err:
            print("refused: " + err, file=sys.stderr)
            bad += 1
        else:
            print(txt + "\n")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
