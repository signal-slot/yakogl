#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""README.md of a committed NanoPC-T6 dEQP-GLES2 slice.

Usage:
  tools/cts/mali-slice-readme.py SLICEDIR            print the README
  tools/cts/mali-slice-readme.py --write SLICEDIR    write SLICEDIR/README.md
  tools/cts/mali-slice-readme.py --check SLICEDIR    exit 1 unless SLICEDIR/README.md
                                                     equals what would be generated

SLICEDIR is tools/cts/log/<name>-t6-gles2-<...>/ as tools/cts/mali-deqp-run.sh
writes it: meta.txt, chunks.tsv, results.tsv, kmsg/<label>.txt and, since
2026-11-15, refusals.tsv. The README gives the session, rows, commit, kernel
and library, start and end, the dEQP status counts, the count of cases with
their own refusal (when the slice has refusals.tsv), the Lost counts by
reason, the chunks that were not done or
had kernel lines or resets (or whose ssh_after or boot_same is not yes), and
the runner command with BOARD_HOST left as a placeholder. The output depends
on the slice alone (no clock, no host). Input holding an IP address or
user@host is refused (exit 2), so is a slice missing a file.
"""
import argparse
import collections
import os
import re
import sys

PRIVATE = re.compile(
    r"(?<![\w.])(?:\d{1,3}\.){3}\d{1,3}(?![\w.])|[\w.+-]+@[\w-]+(?:\.[\w-]+)*|"
    r"/home/|/Users/|\.ts\.net|/dev/tty")
CHUNK_COLS = "row label cases status seconds kmsg resets ssh_after boot_same exit_line".split()
# The board profiles of tools/mali/panthor/board-profile.sh: name, and what runs.
BOARDS = {
    "nanopc-t6": ("NanoPC-T6", "Mali-G610 over panthor, the board's own Armbian"),
    "verdin-imx95": ("Verdin iMX95", "Mali-G310 over panthor, the board's own Linux"),
    # A Zephyr image per row, loaded and started by hand from the module's
    # U-Boot; no kernel, no panthor, no kmsg, no ssh during a row.
    "verdin-imx95-zephyr": ("Verdin iMX95", "Mali-G310 under Zephyr, started from U-Boot"),
}
ZEPHYR = "verdin-imx95-zephyr"
ROW_COLS = "seconds lifetime_s bbm".split()


class Refuse(Exception):
    pass


def read(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            text = f.read()
    except OSError as e:
        raise Refuse("%s: %s" % (path, e.strerror))
    m = PRIVATE.search(text)
    if m:
        raise Refuse("%s: holds %r, which looks like an address, user or home "
                     "path; the slice must be redacted" % (path, m.group(0)))
    return text


def tsv(text, header):
    lines = text.splitlines()
    if not lines or lines[0].split("\t") != header:
        raise Refuse("unexpected TSV header %r, want %r" % (lines[:1], header))
    out = []
    for line in lines[1:]:
        if line:
            c = line.split("\t")
            c += [""] * (len(header) - len(c))
            out.append(dict(zip(header, c)))
    return out


def rows_tsv(text):
    """rows.tsv of a Zephyr slice: a header holding seconds, lifetime_s and bbm
    (other columns, such as row and label, are ignored)."""
    lines = text.splitlines()
    head = lines[0].split("\t") if lines else []
    if any(c not in head for c in ROW_COLS):
        raise Refuse("rows.tsv header %r lacks %s" % (head, ", ".join(ROW_COLS)))
    out = []
    for line in lines[1:]:
        if line:
            c = dict(zip(head, line.split("\t")))
            try:
                out.append((int(c["seconds"]), int(c["lifetime_s"]), c.get("bbm") or "-"))
            except (ValueError, KeyError):
                raise Refuse("rows.tsv: bad row %r" % line)
    return out


def cell(s):
    return (s or "-").replace("|", "\\|")


def table(head, rows):
    out = ["| " + " | ".join(head) + " |", "|" + "---|" * len(head)]
    out += ["| " + " | ".join(cell(x) for x in r) + " |" for r in rows]
    return out


def counts(items):
    return sorted(items.items(), key=lambda kv: (-kv[1], kv[0]))


def odd(c):
    return (c["status"] != "done" or c["ssh_after"] not in ("yes", "-")
            or c["boot_same"] not in ("yes", "-")
            or c["kmsg"] not in ("0", "-", "") or c["resets"] not in ("0", "-", ""))


def build(d):
    meta = {}
    for line in read(os.path.join(d, "meta.txt")).splitlines():
        if "=" in line:
            k, v = line.split("=", 1)
            meta[k.strip()] = v.strip()
    chunks = tsv(read(os.path.join(d, "chunks.tsv")), CHUNK_COLS)
    results = tsv(read(os.path.join(d, "results.tsv")), ["case", "status", "detail"])
    refusals = None
    if os.path.exists(os.path.join(d, "refusals.tsv")):
        refusals = tsv(read(os.path.join(d, "refusals.tsv")), ["case", "refusal"])
    zephyr = meta.get("board") == ZEPHYR
    rows = None
    if zephyr and os.path.exists(os.path.join(d, "rows.tsv")):
        rows = rows_tsv(read(os.path.join(d, "rows.tsv")))
    kdir = os.path.join(d, "kmsg")
    if os.path.isdir(kdir):
        for n in sorted(os.listdir(kdir)):
            read(os.path.join(kdir, n))
    name = os.path.basename(os.path.normpath(d))

    def g(k):
        return meta.get(k) or "-"

    commit = g("commit")
    # The board: a slice of a board other than the NanoPC-T6 names it in a
    # board= line (mali-deqp-run.sh, the board profile's MALI_BOARD).
    board = BOARDS.get(meta.get("board", "nanopc-t6"), (meta.get("board"), "Mali over panthor, the board's own Linux"))
    st = collections.Counter(r["status"] for r in results)
    lost = collections.Counter(r["detail"] or "-" for r in results if r["status"] == "Lost")
    ch = collections.Counter(c["status"] for c in chunks)
    b0, b1 = meta.get("boot_id_before"), meta.get("boot_id_after")
    boot = ("same before and after" if b0 and b0 == b1
            else "before %s, after %s" % (g("boot_id_before"), g("boot_id_after")))

    out = [
        "<!-- SPDX-License-Identifier: Apache-2.0 -->",
        "<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->",
        "<!-- Generated by tools/cts/mali-slice-readme.py --write; do not edit. -->",
        "# %s dEQP-GLES2 %s, rows %s, at %s" % (board[0], g("session"), g("rows"), commit[:8]),
        "",
        "commit %s" % commit,
        "",
        "Slice `%s` of the %s (%s), written by %s." % (name, board[0], board[1],
            "`tools/cts/imx95-uart-slice.py` from the console capture" if zephyr
            else "`tools/cts/mali-deqp-run.sh`"),
        "",
        "| | |", "|---|---|",
        "| session | %s |" % g("session"),
        "| rows | %s of `%s` |" % (g("rows"), g("tree")),
        "| commit | `%s` |" % commit,
    ] + ([
        "| image | `vi95_deqp: build %s` |" % g("build"),
        "| firmware | %s |" % g("fw"),
    ] if zephyr else [
        "| kernel | %s |" % g("kernel"),
        "| library | %s |" % g("lib"),
    ]) + [
        "| started | %s |" % g("started"),
        "| ended | %s |" % g("ended"),
    ] + ([] if zephyr else ["| boot_id | %s |" % boot]) + [
        "",
        "## Result",
        "",
        "%d cases in %d chunks." % (len(results), len(chunks)),
        "",
        "dEQP status counts (`results.tsv`):",
        "",
    ]
    out += table(["status", "cases"], [(k, str(v)) for k, v in counts(st)] or [("-", "0")])
    if refusals is not None:
        not_pass = {r["case"] for r in results if r["status"] != "Pass"}
        out += ["", "Cases with the library's own refusal of a shader or a program "
                "(`refusals.tsv`): %d, of which %d are not Pass."
                % (len(refusals), sum(1 for r in refusals if r["case"] in not_pass))]
    out += ["", "Lost cases by reason (`results.tsv` detail):", ""]
    out += table(["reason", "cases"], [(k, str(v)) for k, v in counts(lost)] or [("none", "0")])
    out += ["", "Chunks by status (`chunks.tsv`): %s." %
            (", ".join("%s %d" % kv for kv in counts(ch)) or "none"), ""]

    if rows is not None:
        over = sum(1 for sec, life, _ in rows if sec >= life)
        bbm = collections.Counter(b for _, _, b in rows)
        out += ["## Rows (`rows.tsv`)", "",
                "%d rows, %d s in all; %d reached their lifetime (seconds >= lifetime_s); "
                "longest %d s." % (len(rows), sum(r[0] for r in rows), over,
                                   max((r[0] for r in rows), default=0)), "",
                "BBM record read back (`bbm`):", ""]
        out += table(["bbm", "rows"], [(k, str(v)) for k, v in counts(bbm)] or [("-", "0")])
        out.append("")
    flagged = [c for c in chunks if odd(c)]
    out += ["## Chunks to look at", ""]
    if flagged and zephyr:
        out += ["Chunks that were not done (a Zephyr row has no kernel lines, "
                "no ssh and no boot_id).", ""]
        out += table(CHUNK_COLS, [[c[k] for k in CHUNK_COLS] for c in flagged])
    elif flagged:
        out += ["Chunks that were not done, or had kernel lines or panthor resets, "
                "or whose ssh_after or boot_same is not yes. The kernel lines are "
                "in `kmsg/<label>.txt`.", ""]
        out += table(CHUNK_COLS, [[c[k] for k in CHUNK_COLS] for c in flagged])
    elif zephyr:
        out.append("None: every chunk is done.")
    else:
        out.append("None: every chunk is done, with no kernel line, no reset, ssh "
                   "answering and the same boot afterwards.")
    if zephyr:
        out += [
            "",
            "## How the slice was produced",
            "",
            "From the images of commit `%s`, one per `# image` row of `%s`, built with "
            "`tools/cts/imx95-deqp-images.sh`. Each image was staged on the module's "
            "root file system, loaded at the module's U-Boot prompt over the serial "
            "console and entered there; the boot partitions were not written. "
            "The slice holds no host name, address, user name, MAC address or serial; "
            "the raw console captures stay out of git." % (commit[:8], g("tree")),
        ]
    else:
        out += [
            "",
            "## How the slice was produced",
            "",
            "From a build of commit `%s` made with `tools/cts/mali-deqp-build.sh`:" % commit[:8],
            "",
            "```",
            ("" if "board" not in meta else "MALI_BOARD=%s " % meta["board"])
            + "BOARD_HOST=<board> tools/cts/mali-deqp-run.sh --build <build-dir> "
            "--out tools/cts/log/%s --rows %s --tree %s --session %s"
            % (name, g("rows"), g("tree"), g("session")),
            "```",
            "",
            "`BOARD_HOST` comes from the lease on `target:%s`. The slice holds " % meta.get("board", "nanopc-t6") +
            "no host name, address, user name or serial; the raw dEQP logs stay out "
            "of git.",
        ]
    text = "\n".join(out) + "\n"
    m = PRIVATE.search(text)
    if m:
        raise Refuse("output would hold %r" % m.group(0))
    return text


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    g = ap.add_mutually_exclusive_group()
    g.add_argument("--write", action="store_true", help="write SLICEDIR/README.md")
    g.add_argument("--check", action="store_true",
                   help="exit 1 unless SLICEDIR/README.md equals the generated text")
    ap.add_argument("dir")
    a = ap.parse_args()
    try:
        text = build(a.dir)
    except Refuse as e:
        print("mali-slice-readme: %s" % e, file=sys.stderr)
        return 2
    path = os.path.join(a.dir, "README.md")
    if a.write:
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
    elif a.check:
        try:
            with open(path, encoding="utf-8") as f:
                have = f.read()
        except OSError:
            have = None
        if have != text:
            print("mali-slice-readme: %s is %s; run tools/cts/mali-slice-readme.py "
                  "--write %s" % (path, "missing" if have is None else "stale", a.dir),
                  file=sys.stderr)
            return 1
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
