#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# crosscheck.py - compare our two independent readings of Mesa's pco encoding.
#
# Two programs in this repository read the same source: this directory's
# extract.py (which feeds the Rust codec tables) and tools/isa-extract/
# pco_isa_extract.py (which feeds docs/online-shaderc/isa.md and its
# machine-readable table).  Two transcriptions of one source are a hazard
# unless they are compared, so this compares them, field for field, over
# everything both record.  Anything it prints is a disagreement to explain
# before either table is trusted.
#
#   tools/pco-isa/extract.py <mesa pco dir> build/isa.json
#   tools/pco-isa/crosscheck.py build/isa.json docs/online-shaderc/isa/usc-isa.json

import json
import sys


def diff_enums(a, b, out):
    # Only enumerations both record are compared. Ours collects the ones a bit
    # set's fields actually reach, theirs every one declared, so an enum on one
    # side only is a difference of scope and not of encoding; it is reported as
    # a note rather than counted as a disagreement.
    for name in sorted(set(a) | set(b)):
        if name not in a or name not in b:
            side = "theirs" if name not in a else "ours"
            out.append(f"NOTE enum {name}: recorded by {side} only (not reached by any field on the other side)")
            continue
        ea = {e["name"]: e["value"] for e in a[name]["elems"]}
        eb = {e["name"]: e["value"] for e in b[name]["elems"]}
        if ea != eb:
            for k in sorted(set(ea) | set(eb)):
                if ea.get(k) != eb.get(k):
                    out.append(f"enum {name}.{k}: ours {ea.get(k)} theirs {eb.get(k)}")


def diff_bit_sets(a, b, their_types, out):
    for name in sorted(set(a) | set(b)):
        if name not in a or name not in b:
            out.append(f"bit set {name}: only in {'theirs' if name not in a else 'ours'}")
            continue
        fa, fb = a[name]["fields"], b[name]["fields"]
        pieces_b = b[name]["pieces"]
        for f in sorted(set(fa) | set(fb)):
            if f not in fa or f not in fb:
                out.append(f"{name}.{f}: field only in {'theirs' if f not in fa else 'ours'}")
                continue
            ours = [(p["byte"], p["hi"], p["lo"]) for p in fa[f]["pieces"]]
            theirs = [
                (pieces_b[p]["byte"], pieces_b[p]["hi"], pieces_b[p]["lo"])
                for p in fb[f]["pieces"]
            ]
            if ours != theirs:
                out.append(f"{name}.{f}: pieces ours {ours} theirs {theirs}")
            if fa[f]["reserved"] != fb[f]["reserved"]:
                out.append(
                    f"{name}.{f}: reserved ours {fa[f]['reserved']} theirs {fb[f]['reserved']}"
                )
            ot, tt = fa[f]["type"], their_types[fb[f]["type"]]
            if ot["num_bits"] != tt["num_bits"] or ot["encode"] != tt["encode"]:
                out.append(
                    f"{name}.{f}: type ours {ot['num_bits']}b/{ot['encode']!r} "
                    f"theirs {tt['num_bits']}b/{tt['encode']!r}"
                )

        sa = {s["name"]: s for s in a[name]["structs"]}
        sb = b[name]["structs"]
        for s in sorted(set(sa) | set(sb)):
            if s not in sa or s not in sb:
                out.append(f"{name}/{s}: form only in {'theirs' if s not in sa else 'ours'}")
                continue
            if sa[s]["bytes"] != sb[s]["num_bytes"]:
                out.append(f"{name}/{s}: bytes ours {sa[s]['bytes']} theirs {sb[s]['num_bytes']}")
            ours_fixed = {e["field"]: e["value"] for e in sa[s]["encode"] if e["kind"] == "fixed"}
            theirs_fixed = {k: v["value"] for k, v in sb[s]["pinned"].items()}
            # A reserved field pinned by the form is in ours and not in theirs;
            # report that separately rather than as a mismatch, because it is
            # the one schema gap and not a disagreement about the encoding.
            reserved = {f for f, v in a[name]["fields"].items() if v["reserved"] is not None}
            for f in sorted(set(ours_fixed) | set(theirs_fixed)):
                if ours_fixed.get(f) == theirs_fixed.get(f):
                    continue
                if f in reserved and f not in theirs_fixed:
                    out.append(f"NOTE {name}/{s}: reserved field {f} pinned in ours, absent in theirs")
                else:
                    out.append(
                        f"{name}/{s}.{f}: fixed ours {ours_fixed.get(f)} theirs {theirs_fixed.get(f)}"
                    )
            ours_set = {
                e["struct_field"]: e["field"] for e in sa[s]["encode"] if e["kind"] == "settable"
            }
            theirs_set = {k: v["field"] for k, v in sb[s]["settable"].items()}
            if ours_set != theirs_set:
                for k in sorted(set(ours_set) | set(theirs_set)):
                    if ours_set.get(k) != theirs_set.get(k):
                        out.append(
                            f"{name}/{s}.{k}: settable ours {ours_set.get(k)} theirs {theirs_set.get(k)}"
                        )


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: crosscheck.py <ours.json> <theirs.json>")
    a = json.load(open(sys.argv[1]))
    b = json.load(open(sys.argv[2]))
    out = []
    diff_enums(a["enums"], b["enums"], out)
    diff_bit_sets(a["bit_sets"], b["bit_sets"], b["field_types"], out)

    notes = [m for m in out if m.startswith("NOTE ")]
    diffs = [m for m in out if not m.startswith("NOTE ")]
    for m in diffs:
        print(m)
    print(f"{len(diffs)} disagreements about the encoding, {len(notes)} notes")
    for m in notes:
        print("  " + m)
    sys.exit(1 if diffs else 0)


if __name__ == "__main__":
    main()
