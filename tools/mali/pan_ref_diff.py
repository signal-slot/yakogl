#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Compare pan_shc's programs with the programs a Mesa GL driver ran.

    tools/mali/pan_ref_diff.py list <trace>
    tools/mali/pan_ref_diff.py extract <trace> --draw > expect.json
    tools/mali/pan_ref_diff.py extract <trace> REGION=VA ... > expect.json
    tools/mali/pan_ref_diff.py diff <pan_shc stdout> <pan_shc stderr> <trace> --draw
    tools/mali/pan_ref_diff.py diff <pan_shc stdout> <pan_shc stderr> <trace> REGION=VA ...
    tools/mali/pan_ref_diff.py diff <pan_shc stdout> <pan_shc stderr> --expect expect.json
    tools/mali/pan_ref_diff.py diff-ref <A> <B>

<trace> is a pandecode dump (PAN_MESA_DEBUG=trace PANDECODE_DUMP_FILE=...):
its "Shader 0x... (GPU VA <va>) sz <n>" blocks hold each program's bytes,
eight per instruction, next to the disassembly. `list` prints every block's
VA and first instructions, so the application's programs can be told from
Mesa's internal ones (clear, blit). A REGION is one of pan_shc's regions
(vs.position, vs.position_no_psiz, vs.varying, fs.fragment); REGION=VA pairs
it with a trace block.

`--draw` pairs the regions with the programs of the trace's FIRST draw only: a
draw is the run of descriptors from one "Position FAU" line to the next. Its
shader program descriptors ("Position shader", "Varying shader", "Fragment
shader") name each program ("Binary:") and say its register allocation and
preload, and the "<kind> FAU" tables above them print the words of the push
uniforms. The position program is paired with pan_shc's draw_position_region
(the no_psiz copy for a draw that is not of points, pan_cmdstream.h:238-249),
the varying program with vs.varying, the fragment program with fs.fragment.
The regions of the two sides must be the same set; the one position variant
the draw does not run is named and left out.

`extract` writes the paired programs as JSON (tools/mali/testdata/*.expect.json,
`diff --expect` reads it): per region the words and disassembly, the register
allocation (`work_reg_count`), the preload mask and the FAU words the trace
prints, and for a VS with a varying program `secondary_offset`, the varying
program's VA less the position program's. The trace prints these values;
pan_shc names them (the uniform or sysval each FAU word is).

What `diff` compares, per region:

- the instruction words: equal, or each differing index with both words and
  both disassembly lines. The reference's line comes from the reference
  Mesa's disassembler, whose syntax can differ from 26.2.1's (25.0.7 prints
  `^r0` and `u1` where 26.2.1 prints `r0^` and `u0.w1`), so the words decide
  and the text is only a reading aid. A trace block's program ends at the
  first all-zero word (disasm.py disassemble_valhall); pan_shc's region ends at
  the next region and its zero padding is left out;
- `work_reg_count` and the preload mask;
- the FAU words. A word the compiler promoted (pan_shc: `constant`) must equal
  the trace's word. A word the driver fills at draw time (a sysval or a
  uniform) is state, not code: it is not compared. The trace prints a table
  of whole 64-bit slots and sometimes fewer than pan_shc's words (the line
  says "N of M words printed"); a printed word beyond pan_shc's last must be
  the zero pad;
- for a VS with a varying program, `secondary_offset`.

`diff-ref A B` compares two references of the same kind (each a trace, taken
at its first draw, or an expect.json) on every one of these, with no word
exempt: the same driver for the same source, or two driver versions.

`diff` and `diff-ref` exit 0 only when every region was found on both sides and
nothing differs; they print the count of regions compared.
"""
import hashlib
import json
import re
import sys

SHADER = re.compile(r"^Shader 0x[0-9a-f]+ \(GPU VA ([0-9a-f]+)\) sz \d+")
INSTR = re.compile(r"^((?:[0-9a-f]{2} ){8})\s*(.*)$")

KINDS = ("Position", "Varying", "Fragment")
KIND_REGION = {"Position": "vs.position", "Varying": "vs.varying", "Fragment": "fs.fragment"}
POSITIONS = ("vs.position", "vs.position_no_psiz")


def words_of(hexbytes):
    return int.from_bytes(bytes(int(x, 16) for x in hexbytes.split()), "little")


def read_trace(path):
    """{va: [(word, text)]} for every shader block of a pandecode dump."""
    blocks = {}
    cur = None
    for line in open(path, errors="replace"):
        line = line.rstrip("\n")
        m = SHADER.match(line)
        if m:
            cur = int(m.group(1), 16)
            blocks[cur] = []
            continue
        if cur is None:
            continue
        m = INSTR.match(line)
        if m:
            blocks[cur].append((words_of(m.group(1)), m.group(2).strip()))
        elif line.strip() == "":
            continue  # pandecode leaves a blank line after a branch
        elif blocks[cur]:
            cur = None
    return blocks


DESC = re.compile(r"^\s+(Position|Varying|Fragment) shader Shader @[0-9a-f]+:")
FAU_HEAD = re.compile(r"^(Position|Varying|Fragment) FAU @[0-9a-f]+:")
FAU_ROW = re.compile(r"^\s*(?:u\d+\s+)?([0-9A-Fa-f]{8})\s+([0-9A-Fa-f]{8})\s*$")
REGS = re.compile(r"^\s+Register allocation: (\d+) Per Thread")
PRELOAD = re.compile(r"^\s+R(\d+)-R\d+: 0x([0-9a-f]+)")
BINARY = re.compile(r"^\s+Binary: 0x([0-9a-f]+)")


def read_draw(path):
    """{kind: {"va", "work_reg_count", "preload", "fau"}} of the FIRST draw.

    A draw is the run of lines from one "Position FAU" table to the next (the
    tables come before the descriptors). A kind the draw has no descriptor for
    is not in the result: a trace of tri has no "Varying shader" in its first
    draw, and the draw after it (Mesa's own blit) has one.
    """
    draw, fau = {}, {}
    desc = None  # the descriptor being read
    table = None  # the FAU table being read
    for line in open(path, errors="replace"):
        line = line.rstrip("\n")
        m = FAU_HEAD.match(line)
        if m:
            if m.group(1) == "Position" and "Position" in draw:
                break  # the next draw
            table = fau[m.group(1)] = []
            continue
        if table is not None:
            m = FAU_ROW.match(line)
            if m:
                table.extend(int(m.group(i), 16) for i in (1, 2))
                continue
            table = None
        m = DESC.match(line)
        if m:
            if m.group(1) == "Position" and "Position" in draw:
                break
            desc = draw[m.group(1)] = {"va": None, "work_reg_count": None, "preload": None}
            continue
        if desc is None:
            continue
        m = REGS.match(line)
        if m:
            desc["work_reg_count"] = int(m.group(1))
            continue
        m = PRELOAD.match(line)
        if m:
            desc["preload"] = (desc["preload"] or 0) | int(m.group(2), 16) << int(m.group(1))
            continue
        m = BINARY.match(line)
        if m:
            desc["va"] = int(m.group(1), 16)
            desc = None
    for kind, d in draw.items():
        d["fau"] = fau.get(kind)
    return {k: d for k, d in draw.items() if d["va"] is not None}


def region_of(blocks, va, facts):
    """One reference region: the block at va and the descriptor's facts."""
    ins = blocks.get(va, [])
    out = {"va": "%x" % va, "found": va in blocks,
           "words": ["%016x" % w for w, _ in ins], "text": [t for _, t in ins],
           "work_reg_count": None, "preload": None, "fau": None}
    if facts:
        out["work_reg_count"] = facts["work_reg_count"]
        out["preload"] = None if facts["preload"] is None else "0x%016x" % facts["preload"]
        out["fau"] = None if facts["fau"] is None else ["%08x" % w for w in facts["fau"]]
    return out


def secondary_offset(regions):
    """The varying program's VA less the position program's, or None."""
    pos = [r for n, r in regions.items() if n in POSITIONS and r["found"]]
    var = regions.get("vs.varying")
    if not pos or not var or not var["found"]:
        return None
    return int(var["va"], 16) - int(pos[0]["va"], 16)


def reference_from_trace(path, pairs=None):
    """The reference of a trace: pairs is {region: va}, or None for the first draw."""
    blocks = read_trace(path)
    draw = read_draw(path)
    by_va = {d["va"]: d for d in draw.values()}
    if pairs is None:
        pairs = {KIND_REGION[k]: d["va"] for k, d in draw.items()}
    regions = {name: region_of(blocks, va, by_va.get(va)) for name, va in pairs.items()}
    return {"trace": path.rsplit("/", 1)[-1], "secondary_offset": secondary_offset(regions),
            "regions": regions}


def normalize_reference(ref):
    """{name: {words, text, work_reg_count, preload, fau}} with ints, from a reference."""
    out = {}
    for name, r in ref["regions"].items():
        out[name] = {
            "found": r["found"], "va": r["va"],
            "words": [int(w, 16) for w in r["words"]], "text": r["text"],
            "work_reg_count": r.get("work_reg_count"),
            "preload": None if r.get("preload") is None else int(r["preload"], 16),
            "fau": None if r.get("fau") is None else [int(w, 16) for w in r["fau"]]}
    return out


def load_reference(path):
    if path.endswith(".json"):
        return json.load(open(path))
    return reference_from_trace(path)


def read_pan_shc(path):
    """(build line, {region: region dict as normalize_reference's}, draw position, secondary_offset)."""
    text = open(path).read()
    first, _, rest = text.partition("\n")
    if not first.startswith("pan_shc: build "):
        raise SystemExit("%s: no pan_shc build line first" % path)
    js = json.loads(rest)
    regions = {}
    position = "vs." + js["vs"]["draw_position_region"]
    for stage in ("vs", "fs"):
        s = js[stage]
        words = [int(w, 16) for w in s["code"]]
        # a constant is compared, a word the driver fills is not (None)
        fau = [int(w["constant"], 16) if "constant" in w else None for w in s["fau"]["words"]]
        for r in s["regions"]:
            w = words[r["offset"] // 8:(r["offset"] + r["size"]) // 8]
            while w and w[-1] == 0:
                w.pop()
            secondary = r["name"] == "varying"
            regions["%s.%s" % (stage, r["name"])] = {
                "found": True, "va": "(pan_shc)", "words": w, "text": [],
                "work_reg_count": s["secondary_work_reg_count"] if secondary else s["work_reg_count"],
                "preload": int(s["secondary_preload"] if secondary else s["preload"], 16),
                "fau": fau}
    sec = js["vs"]["secondary_offset"] if "vs.varying" in regions else None
    return first, regions, position, sec


def read_disasm(path):
    text = {}
    for line in open(path, errors="replace"):
        m = INSTR.match(line.strip())
        if m:
            text[words_of(m.group(1))] = m.group(2).strip()
    return text


def word_digest(rows):
    h = hashlib.sha256()
    for i, ow, tw in rows:
        h.update(("%d %s %s\n" % (i, ow, tw)).encode())
    return h.hexdigest()[:12]


def compare_words(name, a, b, la, lb, atext):
    """Print the word comparison of one region; True when it is byte-equal."""
    diffs = [i for i in range(max(len(a["words"]), len(b["words"])))
             if i >= len(a["words"]) or i >= len(b["words"]) or a["words"][i] != b["words"][i]]
    if not diffs:
        print("%s vs %s: byte-equal, %d instructions" % (name, b["va"], len(a["words"])))
        return True
    rows, lines = [], []
    for i in diffs:
        ow = "%016x" % a["words"][i] if i < len(a["words"]) else "(none)"
        tw = "%016x" % b["words"][i] if i < len(b["words"]) else "(none)"
        rows.append((i, ow, tw))
        od = (atext.get(a["words"][i], "?") if atext is not None else a["text"][i]) if i < len(a["words"]) else ""
        td = b["text"][i] if i < len(b["text"]) else ""
        lines.append("    [%2d] %-8s %s  %s" % (i, la, ow, od))
        lines.append("         %-8s %s  %s" % (lb, tw, td))
    print("%s vs %s: %d of %d/%d instructions differ (words sha256 %s)"
          % (name, b["va"], len(diffs), len(a["words"]), len(b["words"]), word_digest(rows)))
    print("\n".join(lines))
    return False


def compare_facts(name, a, b, la, lb):
    """Print the facts line of one region; True when they agree."""
    bad = []
    for key in ("work_reg_count", "preload"):
        x, y = a[key], b[key]
        if x is None or y is None:
            bad.append("%s absent in %s" % (key, la if x is None else lb))
        elif x != y:
            fmt = "%d" if key == "work_reg_count" else "%#x"
            bad.append(("%s %s " + fmt + ", %s " + fmt) % (key, la, x, lb, y))
    fa, fb = a["fau"], b["fau"]
    note = ""
    if fa is None and fb is None:
        note = " (fau table printed by neither side, not compared)"
    elif fa is None or fb is None:
        bad.append("fau absent in %s" % (la if fa is None else lb))
    else:
        # a word that is None on one side (a driver-filled word) is not compared
        n = min(len(fa), len(fb))
        compared = 0
        for i in range(n):
            if fa[i] is None or fb[i] is None:
                continue
            compared += 1
            if fa[i] != fb[i]:
                bad.append("fau word %d %s %08x, %s %08x" % (i, la, fa[i], lb, fb[i]))
        # the longer side's extra words: a printed table is whole 64-bit slots
        if len(fb) > len(fa):
            for i in range(len(fa), len(fb)):
                if fb[i] != 0:
                    bad.append("fau word %d: %s has none, %s %08x (not the zero pad)" % (i, la, lb, fb[i]))
        if len(fb) < len(fa):
            note = "; %s prints %d of %d words" % (lb, len(fb), len(fa))
        elif len(fa) < len(fb) and len(fb) - len(fa) > 1:
            bad.append("fau length %s %d, %s %d" % (la, len(fa), lb, len(fb)))
        note = " (%d fau words compared%s)" % (compared, note)
    if bad:
        print("%s facts: differ: %s" % (name, "; ".join(bad)))
        return False
    print("%s facts: equal, work_reg_count %d, preload %#x%s"
          % (name, a["work_reg_count"], a["preload"], note))
    return True


def compare(ours, theirs, la, lb, atext=None, skip=(), only=None, sec=(None, None)):
    """Compare two {region: ...} dicts. Returns (regions compared, problems).

    skip: regions neither side's draw runs. only: compare just these (named pairs).
    """
    names = sorted(set(only) if only is not None else set(ours) | set(theirs))
    bad = 0
    for name in names:
        if name in skip:
            print("%s: not run by the draw, not compared" % name)
            continue
        if name not in ours:
            print("%s: %s has no such region (has %s)" % (name, la, ", ".join(sorted(ours))))
            bad += 1
            continue
        if name not in theirs or not theirs[name]["found"]:
            print("%s: %s has no shader at VA %s"
                  % (name, "the reference" if lb == "reference" else lb,
                     theirs[name]["va"] if name in theirs else "(none)"))
            bad += 1
            continue
        ok = compare_words(name, ours[name], theirs[name], la, lb, atext)
        ok &= compare_facts(name, ours[name], theirs[name], la, lb)
        bad += not ok
    if "vs.varying" in ours and "vs.varying" in theirs and "vs.varying" not in skip:
        if sec[0] is None or sec[1] is None or sec[0] != sec[1]:
            print("secondary_offset differs: %s %s, %s %s" % (la, sec[0], lb, sec[1]))
            bad += 1
        else:
            print("secondary_offset: equal, %d" % sec[0])
    return len([n for n in names if n not in skip]), bad


def finish(count, bad):
    print("pan_ref_diff: %d region(s) compared, %d differ or missing" % (count, bad))
    return 1 if bad or not count else 0


def main(argv):
    if len(argv) == 2 and argv[0] == "list":
        for va, ins in sorted(read_trace(argv[1]).items()):
            print("%x: %d instructions" % (va, len(ins)))
            for w, d in ins[:3]:
                print("    %016x  %s" % (w, d))
        return 0
    if len(argv) >= 3 and argv[0] == "extract":
        if argv[2:] == ["--draw"]:
            ref = reference_from_trace(argv[1])
        else:
            pairs = {n: int(v, 16) for n, _, v in (p.partition("=") for p in argv[2:])}
            ref = reference_from_trace(argv[1], pairs)
        missing = [n for n, r in ref["regions"].items() if not r["found"]]
        if missing or not ref["regions"]:
            print("pan_ref_diff: not in the trace: %s" % (", ".join(missing) or "(no region)"),
                  file=sys.stderr)
            return 1
        json.dump(ref, sys.stdout, indent=1)
        print()
        return 0
    if len(argv) == 3 and argv[0] == "diff-ref":
        a, b = load_reference(argv[1]), load_reference(argv[2])
        print("diff-ref: A %s, B %s" % (argv[1].rsplit("/", 1)[-1], argv[2].rsplit("/", 1)[-1]))
        count, bad = compare(normalize_reference(a), normalize_reference(b), "A", "B",
                             sec=(a.get("secondary_offset"), b.get("secondary_offset")))
        return finish(count, bad)
    if len(argv) >= 5 and argv[0] == "diff":
        build, ours, position, ours_sec = read_pan_shc(argv[1])
        ours_text = read_disasm(argv[2])
        if argv[3] == "--expect":
            if len(argv) != 5:
                print(__doc__, file=sys.stderr)
                return 2
            ref = json.load(open(argv[4]))
        elif argv[4:] == ["--draw"]:
            draw = read_draw(argv[3])
            pairs = {position: draw["Position"]["va"]} if "Position" in draw else {}
            for kind in ("Varying", "Fragment"):
                if kind in draw:
                    pairs[KIND_REGION[kind]] = draw[kind]["va"]
            ref = reference_from_trace(argv[3], pairs)
        else:
            pairs = {n: int(v, 16) for n, _, v in (p.partition("=") for p in argv[4:])}
            ref = reference_from_trace(argv[3], pairs)
        theirs = normalize_reference(ref)
        # The position variant the draw does not run is pan_shc's, not the draw's.
        skip = {p for p in POSITIONS if p != position and p in ours and p not in theirs}
        named = argv[3] != "--expect" and argv[4:] != ["--draw"]
        print(build)
        count, bad = compare(ours, theirs, "pan_shc", "reference", atext=ours_text,
                             skip=set() if named else skip, only=set(theirs) if named else None,
                             sec=(ours_sec, ref.get("secondary_offset")))
        return finish(count, bad)
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
