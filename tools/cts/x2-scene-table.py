#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""x2-scene-table.py - the X2 slow-scene table: one row a scene run, ms a frame.

    tools/cts/x2-scene-table.py [--help] <dir>...

<dir>... is a run record directory (the <YYYYMMDD-HHMM>-<name>-<sha8>/ directory
tools/board.sh run leaves under the outdir tools/cts/x2-glmark2-scenes.sh is given)
or a container of them: that outdir, records/x2, a scene's directory. A container
is walked to any depth; every directory named <YYYYMMDD-HHMM>-<name>-<sha8> (or
...-probe) holding a meta.txt or an INDEX.txt is one run. Each run prints one row:

| commit | gc_tiled_rt | scene | ms a frame | FPS | verdict line |

commit      the record directory's <sha8> (the commit its image was built from).
gc_tiled_rt on    the console printed the sample's `glmark2: gc_tiled_rt=1` line, or a
                  parent directory carries the -tiled_rt suffix x2-glmark2-scenes.sh
                  gives an --conf CONFIG_GLMARK2_GC_TILED_RT=y run;
            off   a parent carries -tiled_rt_<val> with val not y (the knob built in,
                  set off);
            absent otherwise (the image had no knob at all: an older commit).
scene       the scene of the run: the bracketed scene of the console's glmark2 result
            line where the record has one (`[terrain] <default>: FPS: ...`), else the
            run name without its -nosel / -p<N> run suffix (x2-glmark2-scenes.sh
            writes ':' and '=' of a scene's options as '_').
ms a frame  glmark2's own FrameTime on the result line; else 1000 / FPS of its
            integer FPS (the derivation tools/cts/common-tally.py --gm-table uses);
            else the frame word v1 of INDEX.txt's decoded record page 0, us / 1000
            (what x2-glmark2-scenes.sh prints as ms/frame); '-' where the record
            carries none of the three.
FPS         the FPS integer on the result line; '-' where there is none.
verdict     INDEX.txt's `verdict: ` line verbatim: board.sh's line, with the run's
            name, verdict, boot, boot_verdict, result, resets, record words and
            back_after. A run whose INDEX.txt carries no verdict line prints a stop
            row: its verdict cell reads `stop: no verdict line in INDEX.txt`
            (`stop: no INDEX.txt` when the file is absent) and a note below the
            table names the stop rows: the run stopped before its verdict was
            written and its row is no measurement.

Everything read from a record is redacted before it prints: an IPv4 address becomes
192.0.2.n and a /dev/tty... path becomes <uart-path> (tools/board.sh already
redacts the files it copies the same way, and writes no host name at all; this is
a second pass, so no address-like string leaves this script).

Exit: 0 the table was printed (a stop row does not change the exit: the runner
already stopped at that run); 2 usage, an unknown option, a path that is not a
directory; 1 no record directory was found.

These are X2 development numbers (docs/notes/x2-glmark2-slow-scenes.md): they go
into notes, commit messages and issue comments, never into tools/cts/RESULTS.md,
STATUS.md or README.md.

Row to add to tools/check.sh later (not added here: another plan owns it):
  tools/cts/x2-scene-table-selftest.sh, in the host-script rows with the other
  tools/cts/*-selftest.sh.
"""
import os
import re
import sys

# the run directory names tools/board.sh writes (is_run_of): <YYYYMMDD-HHMM>-NAME-<sha8|probe>
RUN_RE = re.compile(r"^(\d{8})-(\d{4})-(.+)-([0-9a-f]{8}|probe)$")
# the run-name suffix x2-glmark2-scenes.sh picks per selector: <scene>-nosel, <scene>-p<N>
RUN_SUFFIX_RE = re.compile(r"-(nosel|p\d+)$")
# the directory suffix --conf CONFIG_GLMARK2_GC_TILED_RT=<val> adds: -tiled_rt (val y),
# -tiled_rt_<val> otherwise
TILED_DIR_RE = re.compile(r"^.*-tiled_rt(?:_([A-Za-z0-9_]+))?$")
KNOB_CONSOLE_RE = re.compile(r"\bgc_tiled_rt=1\b")

IPV4_RE = re.compile(r"\b\d{1,3}(?:\.\d{1,3}){3}\b")
TTY_RE = re.compile(r"/dev/tty[A-Za-z0-9._+-]*")

# glmark2's console lines, as tools/cts/common-tally.py reads them: the result line
# "[terrain] <default>: FPS: 6 FrameTime: 166.667 ms", and its bare tail
# "FPS: 6 FrameTime: 181.654 ms" when driver lines landed between the scene's start
# line "[terrain] <default>:" and its end.
GM_RESULT_RE = re.compile(r"(?:^|\s)(\[[\w-]+\] .*?): (?:FPS: |Set up failed|Unsupported)")
GM_BARE_FPS_RE = re.compile(r"(?:^|\s)FPS: \d+")
GM_FPS_RE = re.compile(r"FPS: (\d+)")
GM_FRAMETIME_RE = re.compile(r"FrameTime: (\d+(?:\.\d+)?) ms")
GM_HEADER_RE = re.compile(r"^(?:\d\d:\d\d:\d\d )? ?(\[[\w-]+\] .*)$")
ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")

INDEX_VERDICT_RE = re.compile(r"^verdict: (.*)$", re.M)
# INDEX.txt's decoded records (yakogl-tools record): "experiment 0 page 0: v0 v1 v2 v3 n/a n/a",
# v1 the last frame's t2 - t0 in us
RECORD_PAGE0_RE = re.compile(r"experiment \d+ page 0: (\d+) (\d+) (\d+) (\d+)")

KNOB_ORDER = {"absent": 0, "off": 1, "on": 2}


def redact(text):
    """an IPv4 address -> 192.0.2.n, a /dev/tty... path -> <uart-path> (board.sh's style)"""
    return TTY_RE.sub("<uart-path>", IPV4_RE.sub("192.0.2.n", text))


def cell(text):
    """one table cell: redacted, with '|' escaped so a verbatim line cannot break the table"""
    return redact(text).replace("|", "\\|")


def read_text(path):
    with open(path, "rb") as f:
        return f.read().decode("utf-8", errors="replace")


def plain_lines(text):
    """the record file's lines without NULs, CRs and ANSI escapes (a leading time stamp stays)"""
    text = text.replace("\0", "").replace("\r", "")
    return re.sub(ANSI_RE, "", text).split("\n")


def is_record(path):
    """path is a run record directory: its name is board.sh's and it holds meta.txt or INDEX.txt"""
    if not RUN_RE.match(os.path.basename(path)):
        return None
    if os.path.exists(os.path.join(path, "meta.txt")) or os.path.exists(os.path.join(path, "INDEX.txt")):
        return RUN_RE.match(os.path.basename(path))
    return None


def find_records(paths):
    """every run record directory at or under the arguments, in argument order, deduplicated"""
    out, seen = [], set()

    def add(path):
        real = os.path.realpath(path)
        if real not in seen:
            seen.add(real)
            out.append(path)

    for p in (os.path.normpath(a) for a in paths):
        if is_record(p):
            add(p)
            continue
        for root, dirs, _ in os.walk(p):
            keep = []
            for d in sorted(dirs):
                if is_record(os.path.join(root, d)):
                    add(os.path.join(root, d))  # a record holds no record: do not walk into it
                else:
                    keep.append(d)
            dirs[:] = keep
    return out


def load_meta(path):
    """meta.txt as key=value, the last of a key winning (board.sh's mget)"""
    meta = {}
    try:
        for line in read_text(path).splitlines():
            if "=" in line:
                k, v = line.split("=", 1)
                meta[k] = v.strip()
    except OSError:
        pass
    return meta


def knob_of(recdir, console_text):
    """on / off / absent, the knob's setting for this run (see the header)"""
    if KNOB_CONSOLE_RE.search(console_text):
        return "on"
    d = os.path.dirname(os.path.realpath(recdir))
    for _ in range(6):  # the -tiled_rt suffix sits on the scene directory, a parent or two up
        m = TILED_DIR_RE.match(os.path.basename(d))
        if m:
            return "on" if m.group(1) in (None, "y") else "off"
        parent = os.path.dirname(d)
        if parent == d:
            break
        d = parent
    return "absent"


def bracket_token(info):
    """the scene of a glmark2 info string "[terrain] <default>": terrain"""
    m = re.match(r"\[([\w-]+)\]", info or "")
    return m.group(1) if m else None


def scan_console(lines):
    """glmark2's scene result lines of a console, in order: [(info or None, fps, ft_ms), ...].
    A bare result line gets the scene of the header line that opened it (GM_HEADER_RE),
    as tools/cts/common-tally.py does; a scene run once is what an X2 scene image prints."""
    results, last_header = [], None
    for line in lines:
        m = GM_RESULT_RE.search(line)
        if m:
            f, t = GM_FPS_RE.search(line), GM_FRAMETIME_RE.search(line)
            results.append((m.group(1), int(f.group(1)) if f else None,
                            float(t.group(1)) if t else None))
            last_header = m.group(1)
            continue
        f = GM_FPS_RE.search(line)
        if f and GM_BARE_FPS_RE.search(line):
            t = GM_FRAMETIME_RE.search(line)
            results.append((last_header, int(f.group(1)), float(t.group(1)) if t else None))
            continue
        h = GM_HEADER_RE.match(line)
        if h:
            last_header = h.group(1)
    return results


def read_run(recdir):
    """one record directory into a row dict: sha8, knob, scene, ms, fps, verdict (None = stop)"""
    m = RUN_RE.match(os.path.basename(recdir))
    sha8 = m.group(4)
    meta = load_meta(os.path.join(recdir, "meta.txt"))
    name = meta.get("name") or m.group(3)
    dir_scene = RUN_SUFFIX_RE.sub("", name)

    console_text = ""
    for con in ("console.txt", "console.ts"):  # board.sh: a bridge log is console.ts
        try:
            console_text = read_text(os.path.join(recdir, con))
            if console_text:
                break
        except OSError:
            continue
    knob = knob_of(recdir, console_text)

    idx_text = None
    try:
        idx_text = read_text(os.path.join(recdir, "INDEX.txt"))
    except OSError:
        pass

    results = scan_console(plain_lines(console_text))
    chosen = None
    for r in results:
        token = bracket_token(r[0])
        if token == dir_scene or token == dir_scene.split("_")[0]:
            chosen = r
            break
    if chosen is None and results:
        chosen = results[0]

    scene = bracket_token(chosen[0]) if chosen and chosen[0] else None
    ms, fps = None, None
    if chosen and chosen[1] is not None:
        fps = chosen[1]
        ms = chosen[2] if chosen[2] is not None else 1000.0 / chosen[1]
    if ms is None and idx_text:
        p = RECORD_PAGE0_RE.search(idx_text)
        if p:
            ms = int(p.group(2)) / 1000.0
    if scene is None:
        scene = dir_scene

    if idx_text is None:
        verdict, stop = None, "stop: no INDEX.txt"
    else:
        v = INDEX_VERDICT_RE.search(idx_text)
        verdict, stop = (v.group(1) if v else None,
                         "stop: no verdict line in INDEX.txt")
    return {"dir": recdir, "sha8": sha8, "name": name, "knob": knob, "scene": scene,
            "ms": ms, "fps": fps, "verdict": verdict, "stop": stop}


def main(argv):
    args = []
    for a in argv:
        if a in ("-h", "--help"):
            sys.stdout.write(__doc__)
            return 0
        if a.startswith("-"):
            sys.stderr.write("x2-scene-table: unknown option %s\n" % a)
            return 2
        args.append(a)
    if not args:
        sys.stderr.write("usage: tools/cts/x2-scene-table.py <record dir or container>...\n")
        return 2
    missing = [a for a in args if not os.path.isdir(a)]
    if missing:
        sys.stderr.write("x2-scene-table: not a directory: %s\n" % ", ".join(missing))
        return 2

    records = find_records(args)
    if not records:
        sys.stderr.write("x2-scene-table: no record directories (<YYYYMMDD-HHMM>-<name>-<sha8> "
                         "with a meta.txt or an INDEX.txt) under: %s\n" % ", ".join(args))
        return 1

    runs = [read_run(r) for r in records]
    runs.sort(key=lambda r: (r["scene"].lower(), r["sha8"], KNOB_ORDER[r["knob"]], r["name"]))

    print("| commit | gc_tiled_rt | scene | ms a frame | FPS | verdict line |")
    print("|---|---|---|---|---|---|")
    for r in runs:
        ms = "%.3f" % r["ms"] if r["ms"] is not None else "-"
        fps = "%d" % r["fps"] if r["fps"] is not None else "-"
        verdict = r["verdict"] if r["verdict"] is not None else r["stop"]
        print("| %s | %s | %s | %s | %s | %s |"
              % (r["sha8"], r["knob"], cell(r["scene"]), ms, fps, cell(verdict)))

    stops = [r for r in runs if r["verdict"] is None]
    if stops:
        names = ", ".join(cell(r["name"]) for r in stops)
        print("\nx2-scene-table: stop: %d record(s) printed no verdict line: %s. "
              "Those rows are not measurements: the run stopped before board.sh wrote its verdict."
              % (len(stops), names))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
