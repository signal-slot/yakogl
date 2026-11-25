#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Build and check the rows.tsv of a Verdin iMX95 dEQP-GLES2 board session.

  python3 -I tools/cts/imx95-rows-tsv.py [OPTIONS] CAPTURE...   build rows.tsv
  python3 -I tools/cts/imx95-rows-tsv.py --check DIR...         check it

Build mode takes one console capture per '# image' row of
tools/cts/mali-gles2-tree.txt (the labels of one session, e.g. T6S4R01 to
T6S4R08): the serial console from that row's `go` to the start of the next
row. A CAPTURE is a file or a directory (every file of it is a capture); the
label comes from the file name (a T6S4R01-like token anywhere in it) or from
--map LABEL=FILE, which wins over the name. It writes <out> (default
rows.tsv) and the sidecar <out>.license, as the T6 slices keep their tsvs:

  row  label  cases  status  seconds  lifetime_s  bbm  exit_line

Parsed lines, all of them the ones tools/cts/README.md section 5 documents:
  `vi95_deqp: build <describe>` (or `vi95_gl_test: build ...`): the image ran
  `deqp: summary: N cases begun, N with a result, N without`: N -> cases
  `deqp: OK|FAILED (rc=N)` and the board layer's `deqp: PASS|FAIL`
  `yk-record: w0 w1 w2 w3`: the last record line, the four words
  `verdin_imx95: board: run over: PSCI SYSTEM_RESET at N ms`
  `verdin_imx95: WDOG3 feeding stopped at N ms after F feeds (lifetime over|no progress): ...`
  `verdin_imx95: lifetime over: PSCI SYSTEM_RESET at N ms`
  `verdin_imx95: WDOG3 armed: ... fed until N s of uptime`: N -> lifetime_s

status (the first rule that matches):
  done     a run-over line, or a `deqp: PASS|FAIL`/`deqp: OK|FAILED` line a
           cut capture left standing (the sample ended; no reset line)
  lifetime a `lifetime over: PSCI SYSTEM_RESET` line, or feeding stopped
           with `(lifetime over)`
  hang     feeding stopped with `(no progress)` (WDOG3 reset the board), or
           a build line with no end of run at all (the capture was cut)
  noboot   nothing of that in the capture (no console after `go`; at most
           the pre-sample boot lines, a refused WDOG3 arming)
Later occurrences win: a capture holding two runs is read for its last one.

seconds: the uptime of the run-over, lifetime-over or feeding-stopped line,
rounded to seconds; else --seconds LABEL=N (the host-timed value: the row
ended without a timestamp on the console); else '-' (check flags it).
lifetime_s: the console `fed until N s of uptime` when above 0, else
--lifetime-s N; never a constant here - driver/samples/zephyr/deqp/
verdin_imx95.conf says 600 today and the item says it may say 300 - and
without either source the tool refuses (exit 2).
bbm, from the read-back of tools/imx95_bbm_record.sh `read` (the four words
on one line, what the sessions keep per row in a --bbm-dir file named after
the label, or a --bbm LABEL=FILE):
  match     the words equal the capture's last `yk-record:` words
  none      all four words zero (cleared, nothing survived)
  differs   anything else, a capture with no yk-record line included
  not-read  no read-back for the row, or a file with no four words in it
cases is the summary's count, '-' when the row never got to a summary.

Check mode (exit 1 on any violation, exit 0 with one OK line per directory
otherwise): every DIR/rows.tsv has this exact header, one line per '# image'
row of --tree named by the session range its labels span (none twice, none
missing, in tree order, row = the label's number within its session), and
integer seconds and lifetime_s, a status and a bbm from the vocabularies.
"""
import argparse
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_TREE = os.path.join(HERE, "mali-gles2-tree.txt")

HEADER = "row\tlabel\tcases\tstatus\tseconds\tlifetime_s\tbbm\texit_line"
COLUMNS = HEADER.split("\t")
STATUSES = ("done", "hang", "lifetime", "noboot")
BBMS = ("match", "differs", "none", "not-read")
LICENSE = (
    "SPDX-License-Identifier: Apache-2.0\n"
    "SPDX-FileCopyrightText: 2026 Signal Slot, Inc.\n"
    "Rows table of a Verdin iMX95 dEQP-GLES2 session, built by\n"
    "tools/cts/imx95-rows-tsv.py from one console capture per row.\n"
)
EXIT_LINE_MAX = 200

LABEL_IN_NAME = re.compile(r"T6S\d{1,2}R\d{1,2}", re.IGNORECASE)
LABEL_CANON = re.compile(r"^T6S(\d{1,2})R(\d{1,2})$")
RE_BUILD = re.compile(r"vi95_\w+: build ")
RE_SUMMARY = re.compile(r"deqp: summary: (\d+) cases begun, (\d+) with a result, (\d+) without")
RE_END = re.compile(r"deqp: (?:OK|FAILED) \(rc=-?\d+\)")
RE_VERDICT = re.compile(r"(?:deqp|gl_test): (?:PASS|FAIL)\b")
RE_RECORD = re.compile(r"yk-record: ((?:[0-9a-fA-F]{8} ){3}[0-9a-fA-F]{8})")
RE_RUNOVER = re.compile(r"verdin_imx95: board: run over: PSCI SYSTEM_RESET at (\d+) ms")
RE_LIFETIME = re.compile(r"verdin_imx95: lifetime over: PSCI SYSTEM_RESET at (\d+) ms")
RE_STOPPED = re.compile(
    r"verdin_imx95: WDOG3 feeding stopped at (\d+) ms after \d+ feeds \((lifetime over|no progress)\)"
)
RE_FED = re.compile(r"fed until (\d+) s of uptime")
WORD = re.compile(r"^[0-9a-fA-F]{8}$")


def die(msg):
    sys.exit("imx95-rows-tsv: %s" % msg)


def label_key(label):
    m = LABEL_CANON.match(label)
    if not m:
        return None
    return (int(m.group(1)), int(m.group(2)))


def session_ordinals(tree):
    """label -> its number within its session, in tree order."""
    ordinal = {}
    seen = {}
    for l in tree:
        s = label_key(l)[0]
        seen[s] = seen.get(s, 0) + 1
        ordinal[l] = seen[s]
    return ordinal


def load_tree(path):
    """The '# image' labels, in file order."""
    labels = []
    try:
        with open(path, encoding="utf-8") as f:
            for line in f:
                m = re.match(r"# image (\S+):", line)
                if m:
                    labels.append(m.group(1))
    except OSError as e:
        die("tree %s: %s" % (path, e))
    if not labels:
        die("tree %s names no '# image' row" % path)
    for l in labels:
        if label_key(l) is None:
            die("tree %s: label %s is not T6S<session>R<row>" % (path, l))
    return labels


def clean_exit_line(line):
    line = line.replace("\t", " ").strip()
    return line[:EXIT_LINE_MAX]


def parse_capture(path):
    """Everything the rows.tsv needs from one capture, as a dict."""
    try:
        with open(path, "rb") as f:
            text = f.read().decode("utf-8", "replace").replace("\r", "")
    except OSError as e:
        die("capture %s: %s" % (path, e))
    out = {
        "path": path,
        "build": None,
        "cases": None,
        "end": None,
        "verdict": None,
        "record": None,
        "runover": None,
        "lifetime": None,
        "stopped": None,  # (ms, reason)
        "fed": None,
        "last_line": None,
    }
    for raw in text.splitlines():
        line = raw.strip()
        if not line:
            continue
        out["last_line"] = line
        if RE_BUILD.search(line):
            out["build"] = line
        m = RE_SUMMARY.search(line)
        if m:
            out["cases"] = int(m.group(1))
        if RE_END.search(line):
            out["end"] = line
        m = RE_VERDICT.search(line)
        if m:
            out["verdict"] = line
        m = RE_RECORD.search(line)
        if m:
            out["record"] = [w.lower() for w in m.group(1).split()]
        m = RE_RUNOVER.search(line)
        if m:
            out["runover"] = int(m.group(1))
        m = RE_LIFETIME.search(line)
        if m:
            out["lifetime"] = int(m.group(1))
        m = RE_STOPPED.search(line)
        if m:
            out["stopped"] = (int(m.group(1)), m.group(2))
        m = RE_FED.search(line)
        if m:
            out["fed"] = int(m.group(1))
    return out


def status_and_exit_line(p):
    """(status, exit_line) by the documented rule order."""
    if p["runover"] is not None:
        return "done", "verdin_imx95: board: run over: PSCI SYSTEM_RESET at %d ms" % p["runover"]
    if p["lifetime"] is not None:
        return "lifetime", "verdin_imx95: lifetime over: PSCI SYSTEM_RESET at %d ms" % p["lifetime"]
    if p["stopped"] is not None:
        ms, reason = p["stopped"]
        line = (
            "verdin_imx95: WDOG3 feeding stopped at %d ms after ? feeds (%s)" % (ms, reason)
        )
        if reason == "lifetime over":
            return "lifetime", line
        return "hang", line
    for line in (p["verdict"], p["end"]):
        if line:
            return "done", line  # the sample ended; the capture lost the reset
    if p["build"]:
        return "hang", p["build"]  # it booted and never ended on the console
    return "noboot", "-"


def read_bbm_words(path):
    """The four words of an imx95_bbm_record.sh read, or None (not-read)."""
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            tokens = f.read().split()
    except OSError as e:
        die("bbm %s: %s" % (path, e))
    words = [t.lower() for t in tokens if WORD.match(t)]
    if len(words) < 4:
        if tokens:
            sys.stderr.write(
                "imx95-rows-tsv: %s: no four words of eight hex digits, counted not-read\n" % path
            )
        return None
    return words[-4:]


def bbm_value(readback, record):
    if readback is None:
        return "not-read"
    if all(w == "00000000" for w in readback):
        return "none"
    if record is not None and readback == record:
        return "match"
    return "differs"


def ms_to_seconds(ms):
    return (ms + 500) // 1000


def expand_captures(pos, tree_set):
    """[(label, path)] from files, directories and --map entries."""
    files = []
    for p in pos:
        if os.path.isdir(p):
            for name in sorted(os.listdir(p)):
                full = os.path.join(p, name)
                if os.path.isfile(full):
                    files.append((name, full))
        elif os.path.isfile(p):
            files.append((os.path.basename(p), p))
        else:
            die("capture %s: no such file or directory" % p)
    by_label = {}
    for name, path in files:
        m = LABEL_IN_NAME.search(name)
        if not m:
            continue  # named by --map, or not a capture of a row
        label = m.group(0).upper()
        if label not in tree_set:
            die("capture %s: label %s is no '# image' row of the tree" % (path, label))
        if label in by_label:
            die("label %s: two captures (%s, %s)" % (label, by_label[label], path))
        by_label[label] = path
    return by_label


def build(args, tree):
    tree_set = set(tree)
    by_label = expand_captures(args.captures, tree_set)
    for spec in args.map:
        label, _, path = spec.partition("=")
        label = label.upper()
        if "=" not in spec or not path:
            die("--map wants LABEL=FILE, got %r" % spec)
        if label not in tree_set:
            die("--map %s: no such '# image' row in the tree" % label)
        if not os.path.isfile(path):
            die("--map %s: %s: not a file" % (spec, path))
        if label in by_label and by_label[label] != path:
            die("label %s: two captures (%s, %s)" % (label, by_label[label], path))
        by_label[label] = path
    if not by_label:
        die("no capture named a row (name files T6S4R01... or use --map)")

    seconds = {}
    for spec in args.seconds:
        label, _, value = spec.partition("=")
        label = label.upper()
        if "=" not in spec or not value.isdigit() or label not in tree_set:
            die("--seconds wants LABEL=N, got %r" % spec)
        seconds[label] = int(value)

    def bbm_file(label):
        if label in bbm_map:
            return bbm_map[label]
        if args.bbm_dir:
            for suffix in ("", ".txt", ".bbm"):
                p = os.path.join(args.bbm_dir, label + suffix)
                if os.path.isfile(p):
                    return p
        return None

    bbm_map = {}
    for spec in args.bbm:
        label, _, path = spec.partition("=")
        label = label.upper()
        if "=" not in spec or not path or label not in tree_set:
            die("--bbm wants LABEL=FILE, got %r" % spec)
        if not os.path.isfile(path):
            die("--bbm %s: %s: not a file" % (spec, path))
        bbm_map[label] = path

    labels = [l for l in tree if l in by_label]
    ordinals = session_ordinals(tree)
    rows = []
    missing_lifetime = []
    for label in labels:
        p = parse_capture(by_label[label])
        status, exit_line = status_and_exit_line(p)
        if p["fed"] and p["fed"] > 0:
            lifetime = p["fed"]
        elif args.lifetime_s is not None:
            lifetime = args.lifetime_s
        else:
            lifetime = None
            missing_lifetime.append(label)
        ms = p["runover"] if p["runover"] is not None else p["lifetime"]
        if ms is None and p["stopped"] is not None:
            ms = p["stopped"][0]
        secs = ms_to_seconds(ms) if ms is not None else seconds.get(label)
        path = bbm_file(label)
        rows.append(
            {
                "row": str(ordinals[label]),
                "label": label,
                "cases": "-" if p["cases"] is None else str(p["cases"]),
                "status": status,
                "seconds": "-" if secs is None else str(secs),
                "lifetime_s": "-" if lifetime is None else str(lifetime),
                "bbm": bbm_value(read_bbm_words(path) if path else None, p["record"]),
                "exit_line": exit_line,
            }
        )
    if missing_lifetime:
        die(
            "no lifetime for %s: no 'fed until N s of uptime' line in its capture and no "
            "--lifetime-s (the conf may say 300 or 600; it is never guessed here)"
            % ", ".join(missing_lifetime)
        )

    out = "\n".join([HEADER] + ["\t".join(r[c] for c in COLUMNS) for r in rows]) + "\n"
    with open(args.out, "w", encoding="utf-8") as f:
        f.write(out)
    with open(args.out + ".license", "w", encoding="utf-8") as f:
        f.write(LICENSE)
    counts = {}
    for r in rows:
        counts[r["status"]] = counts.get(r["status"], 0) + 1
    print(
        "%s: %d rows (%s)"
        % (
            args.out,
            len(rows),
            ", ".join("%s %d" % (s, counts[s]) for s in STATUSES if s in counts),
        )
    )
    return 0


def check_dirs(args, tree):
    keys = {l: label_key(l) for l in tree}
    ordinal = session_ordinals(tree)
    bad = 0
    for d in args.check:
        errs = []
        path = os.path.join(d, "rows.tsv")
        try:
            with open(path, encoding="utf-8", errors="replace") as f:
                content = f.read()
        except OSError as e:
            print("%s: %s" % (d, e), file=sys.stderr)
            bad += 1
            continue
        lines = content.split("\n")
        while lines and lines[-1] == "":
            lines.pop()
        if not lines or lines[0] != HEADER:
            errs.append("missing or wrong header line")
            body = []
        else:
            body = lines[1:]
        labels = []
        for i, line in enumerate(body, start=2):
            fields = line.split("\t")
            if len(fields) != len(COLUMNS):
                errs.append("line %d: %d tab fields, want %d" % (i, len(fields), len(COLUMNS)))
                continue
            f = dict(zip(COLUMNS, fields))
            if f["label"] not in keys:
                errs.append("line %d: label %s is no '# image' row of the tree" % (i, f["label"]))
            else:
                labels.append(f["label"])
                if f["row"] != str(ordinal[f["label"]]):
                    errs.append(
                        "line %d: %s has row %s, want %d"
                        % (i, f["label"], f["row"], ordinal[f["label"]])
                    )
            if f["cases"] != "-" and not f["cases"].isdigit():
                errs.append("line %d: cases %r is not an integer or '-'" % (i, f["cases"]))
            if f["status"] not in STATUSES:
                errs.append("line %d: status %r is not one of %s" % (i, f["status"], "|".join(STATUSES)))
            if f["bbm"] not in BBMS:
                errs.append("line %d: bbm %r is not one of %s" % (i, f["bbm"], "|".join(BBMS)))
            if not f["seconds"].isdigit():
                errs.append("line %d: seconds %r is not an integer" % (i, f["seconds"]))
            if not f["lifetime_s"].isdigit():
                errs.append("line %d: lifetime_s %r is not an integer" % (i, f["lifetime_s"]))
        dupes = sorted({l for l in labels if labels.count(l) > 1})
        for l in dupes:
            errs.append("label %s names %d rows, want one" % (l, labels.count(l)))
        labels = [l for l in labels if l in keys]
        uniq = sorted(dict.fromkeys(labels), key=lambda l: keys[l])
        if uniq:
            lo, hi = keys[uniq[0]], keys[uniq[-1]]
            want = [l for l in tree if lo <= keys[l] <= hi]
            if uniq != want:
                missing = [l for l in want if l not in uniq]
                extra = [l for l in uniq if l not in want]
                errs.append(
                    "the labels span %s..%s but the rows are not exactly those of the tree%s"
                    % (
                        uniq[0],
                        uniq[-1],
                        (" (missing %s)" % ", ".join(missing) if missing else "")
                        + (" (unexpected %s)" % ", ".join(extra) if extra else ""),
                    )
                )
        else:
            errs.append("no rows")
        if errs:
            bad += 1
            for e in errs:
                print("%s: %s" % (d, e), file=sys.stderr)
        else:
            print("OK: %s: %d rows" % (d, len(uniq)))
    return 1 if bad else 0


def main(argv):
    ap = argparse.ArgumentParser(prog="imx95-rows-tsv.py", description=__doc__.splitlines()[0])
    ap.add_argument("--check", metavar="DIR", nargs="+",
                    help="check DIR/rows.tsv (exit 1 on any violation) instead of building")
    ap.add_argument("--tree", default=DEFAULT_TREE,
                    help="the '# image' tree (default: mali-gles2-tree.txt next to this tool)")
    ap.add_argument("--out", default="rows.tsv", help="build: the file to write (rows.tsv)")
    ap.add_argument("--map", action="append", default=[], metavar="LABEL=FILE",
                    help="the capture of LABEL, whatever the file name")
    ap.add_argument("--seconds", action="append", default=[], metavar="LABEL=N",
                    help="host-timed seconds for LABEL, when the console has no end line")
    ap.add_argument("--lifetime-s", type=int, default=None, metavar="N",
                    help="lifetime in seconds, when the console has no 'fed until' line")
    ap.add_argument("--bbm", action="append", default=[], metavar="LABEL=FILE",
                    help="the imx95_bbm_record.sh read of LABEL")
    ap.add_argument("--bbm-dir", metavar="DIR",
                    help="a directory of read-backs named LABEL[.txt|.bbm]")
    ap.add_argument("captures", nargs="*", metavar="CAPTURE",
                    help="build: one console capture per row (file or directory)")
    args = ap.parse_args(argv)
    tree = load_tree(args.tree)
    if args.check:
        if args.captures:
            die("--check takes directories, not captures")
        return check_dirs(args, tree)
    if not args.captures:
        die("build wants one capture per row (or use --check)")
    return build(args, tree)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
