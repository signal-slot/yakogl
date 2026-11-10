#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The tally of one NanoPC-T6 glmark2-es2 run (Mali-G610, panthor, back end gpu-mali).

  python3 -I tools/cts/glmark2-mali-tally.py [--column COL] [--table FILE] <rundir>
  python3 -I tools/cts/glmark2-mali-tally.py --picture-lines [--column COL] <rundir>
  python3 -I tools/cts/glmark2-mali-tally.py --results-row [--column COL] [--table FILE] <rundir>
  python3 -I tools/cts/glmark2-mali-tally.py --status-row [--column COL] [--table FILE] <rundir>
  python3 -I tools/cts/glmark2-mali-tally.py --causes <rundir>
  python3 -I tools/cts/glmark2-mali-tally.py [--allow-verdict-mismatch] ... <runner-dir>

It plays for glmark2 the part tools/cts/mali-deqp-tally.py plays for dEQP-GLES2.
The run is one glmark2-es2 process per scene (tools/cts/mali-glmark2-build.sh's
pbuffer-glesv2 flavour, 800x600, on the tools/mali/panthor-gles pair), and
<rundir> holds:

  env.txt            key=value lines: commit (7 to 40 hex, required), board,
                     kernel, pair_sha256 (the libGLESv2.so.2 of the pair), and
                     optionally date (YYYY-MM-DD). Other keys are kept, unread.
  <NN>-<slug>.log    the scene's glmark2-es2 stdout and stderr: glmark2's result
                     line "[scene] opts: FPS: N FrameTime: X ms" (or " Set up
                     failed", " Unsupported"), the patch's "glmark2-picture:"
                     line, the pair's "yakogl-mali: loaded" and
                     "yakogl-mali-exit:" lines (tools/mali/panthor-gles/README.md,
                     "The two lines") and whatever the library logged
  <NN>-<slug>.kmsg   tools/cts/mali-kmsg.py's output for the scene: its line
                     "kmsg: resets=N timeouts=N mmu_faults=N fw_faults=N
                     gpu_faults=N other_panthor=N first=..." (or its --json
                     form); other lines (the redacted kernel lines) are skipped
  <NN>-<slug>.status exit=<code> and timeout=<0|1> (timeout=1: the runner's
                     time limit killed the process)

<slug> and the 33 scenes are those of `tools/cts/glmark2-host-ref.sh --list`,
which must agree with the host column of tools/cts/glmark2-pictures.txt. <NN>
is two digits, the runner's order; it is not read (a scene is its slug). A
scene with none of the three files is not-run; one with a .log but no .status
or .kmsg, an unknown slug, a slug given twice, a log whose result or picture
line names another scene, or a log with two result lines is malformed.

A runner directory instead (tools/cts/mali-glmark2-run.sh's <outdir>, recognised
by its scenes.tsv and meta.txt) is read directly:

  scenes.tsv         row n is scene n: scene ("name:opts", glmark2's -b form) and
                     the runner's verdict; the scene is matched to the --list
                     scene of the same name and options (sorted: the -b form
                     has them in upstream's order, the info string in glmark2's)
  log/<n>.out+.err   the scene's log (the .out, then the .err): as <NN>-<slug>.log
  kmsg/<n>.class     mali-kmsg.py's line (with its cs_faults, counted); the file
                     kmsg/<n>.log supplies the kernel's memory-failure lines
  meta.txt           commit (required), kernel and started (its date) as env.txt;
                     there is no board address or pair sha256 in the runner's meta

The runner records no exit status and no timeout flag, so the .status is made
from the runner's verdict: hang gives timeout=1 and exit 137; crash gives exit
139 when the log has a yakogl-mali-exit line (the process died by a signal after
it; which signal is not recorded, the cause says so) and exit 0 when it has not
(the tally finds that crash itself); failed-scene whose log has an FPS and no
Set up failed or Unsupported gives exit 1; any other verdict gives exit 0. A
scene the runner marks not-run or board-lost has no files here, so the tally
says not-run. The tally then derives its own verdicts as above and compares
them with the runner's:

  runner               tally
  ok                   ok
  gl-error             gl-error
  hang                 hang
  crash                crash
  gpu-fault            gpu-fault
  job-timeout          gpu-fault   (the tally counts kmsg timeouts as gpu-fault)
  out-of-gpu-memory    oom
  refused-draw         fail        (plan #309's refusals)
  failed-scene         fail
  wrong-lib            fail        (no "yakogl-mali: loaded" line)
  not-run, board-lost  not-run

A scene whose two verdicts differ (the tally also counts GL_OUT_OF_MEMORY and the
kernel's out of memory as oom, the runner does not yet) is printed on stderr with
both verdicts, after the output, and the exit status is 3; with
--allow-verdict-mismatch the same lines are printed as "allowed" and the exit
status is 0. The tally's verdict is the one in every output.

Verdict per scene, the first that holds:
  not-run    no file of the scene
  hang       timeout=1
  gpu-fault  a counted panthor line in .kmsg, or gpu_faults / job_timeouts of
             the pair's exit line above 0
  crash      exit >= 128 (a signal: exit - 128), or a loaded line and no
             yakogl-mali-exit line (the process died before atexit)
  oom        "out of GPU memory" or a refused tiled copy (common-tally.py's
             GLMARK2_BAD_LINES out-of-memory, tiled-copy), the back end's
             "mali: the OSAL refused a buffer" and "mali: no sample store"
             (MALI_OOM_LINES), "GL error 0x505" and the kernel's "Out of
             memory:" and "page allocation failure" in the .log or .kmsg
  gl-error   "GL error 0x... in ..." (GLMARK2_BAD_LINES gl-error) but 0x505,
             and not when the scene also has a refused program or draw: the
             refusal is the cause and the GL errors after it (0x501 at
             glGetAttribLocation, 0x502 at the draw, what ES 2.0 asks for
             when a client goes on with a program that did not link) its
             consequence, so that scene is fail (as the runner's refused-draw)
             with the GL errors under "also"
  fail       anything else wrong: Set up failed, Unsupported, no result line,
             exit not 0, no "yakogl-mali: loaded" line (the board's Mesa ran:
             the sonama trap), refused draws or programs (the exit line's
             refused, first_refused), an incomplete texture, a draw refused or
             not recorded (GLMARK2_BAD_LINES)
  ok         a result line with an FPS and none of the above

The picture is judged apart from the verdict, with common-tally.py (imported,
unchanged): its picture parser and judge, against column COL (default mali)
of --table (default tools/cts/glmark2-pictures.txt). Frame time and FPS come
from common-tally.py's glmark2_speed (glmark2's FrameTime, else 1000 / FPS).

Outputs:
  (default)        the per-scene table: scene, verdict, ms/frame, FPS, picture
                   value (crc16, mean, black), picture judgement, panthor
                   lines counted in .kmsg; then a summary and the causes
  --picture-lines  "COL | <scene> | <picture value>" for every scene with a
                   picture value, for glmark2-pictures.txt (common-tally.py
                   --gm-picture-lines' form); the others are named on stderr
  --results-row    a draft of the NanoPC-T6 glmark2-es2 row of
                   tools/cts/RESULTS.md, with the per-scene list under it
  --status-row     a draft of the Japanese summary line of tools/cts/STATUS.md
  --causes         the failed scenes grouped by cause, each with a draft issue
                   title and text (a refusal is marked as plan #309's); the
                   scenes not run are listed, with no issue

Nothing printed carries a board address: IPv4 addresses, user@host logins and
UART device paths read from the run are replaced by <addr>, <login>, <uart>.

Exit status: 0 the tally was produced, 1 malformed input, 2 usage, 3 a runner
directory whose verdicts disagree with the tally's (without the flag).
"""

import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
HOST_REF = os.path.join(HERE, "glmark2-host-ref.sh")
PICTURES = os.path.join(HERE, "glmark2-pictures.txt")
SCENE_COUNT = 33
VERDICTS = ("not-run", "hang", "gpu-fault", "crash", "oom", "gl-error", "fail", "ok")
KMSG_KEYS = ("resets", "timeouts", "mmu_faults", "fw_faults", "gpu_faults", "cs_faults", "other_panthor")
# the mali-kmsg.py line of today and of the first fixtures (no cs_faults)
RUNNER_VERDICTS = {"ok": "ok", "gl-error": "gl-error", "hang": "hang", "crash": "crash",
                   "gpu-fault": "gpu-fault", "job-timeout": "gpu-fault", "out-of-gpu-memory": "oom",
                   "refused-draw": "fail", "failed-scene": "fail", "wrong-lib": "fail",
                   "not-run": "not-run", "board-lost": "not-run"}
FILE_RE = re.compile(r"^(\d\d)-([a-z0-9-]+)\.(log|kmsg|status)$")
KMSG_RE = re.compile(r"^kmsg: resets=(\d+) timeouts=(\d+) mmu_faults=(\d+) fw_faults=(\d+) "
                     r"gpu_faults=(\d+) (?:cs_faults=(\d+) )?other_panthor=(\d+) first=(.*)$")
LOADED_RE = re.compile(r"yakogl-mali: loaded ")
EXIT_RE = re.compile(r"yakogl-mali-exit: draws=(\d+) refused=(\d+) first_refused=(.*) "
                     r"gpu_faults=(\d+) job_timeouts=(\d+)\s*$")
SETUP_RE = re.compile(r"(?:^|\s)(Set up failed|Unsupported)\s*$")
GL_ERROR_RE = re.compile(r"GL error (0x[0-9a-fA-F]+) in (\w+)")
# the scenes the results row names in bold, as the other boards' rows do
SLOW_SCENES = ("[terrain] <default>", "[shadow] <default>", "[refract] <default>")
OWNER_309 = ("refused", "draw-refused", "draw-not-recorded")
# the memory failures that make a scene oom besides common-tally.py's out of GPU memory and tiled
# copy: the Mali back end's own lines (src/mali/backend.rs alloc(), "no sample store": the MSAA
# frame is degraded, so the scene did not run as asked), GL_OUT_OF_MEMORY (0x505) and the
# kernel's (in the scene's .log or .kmsg)
MALI_OOM_LINES = (
    ("osal-refused", re.compile(r"mali: the OSAL refused a buffer of \d+ bytes")),
    ("no-sample-store", re.compile(r"mali: no sample store of \d+ bytes")),
)
GL_OOM_RE = re.compile(r"GL error 0x0*505 in ", re.I)
KERNEL_OOM_RE = re.compile(r"Out of memory:|page allocation failure")
OOM_RE = re.compile(r"out of GPU memory|no GPU memory for|%s|%s|%s" % (
    "|".join(rx.pattern for _, rx in MALI_OOM_LINES), GL_OOM_RE.pattern, KERNEL_OOM_RE.pattern), re.I)
STAMP_RE = re.compile(r"^\s*(?:<\d+>)?\[\s*\d+\.\d+\]\s*")


class Malformed(Exception):
    pass


def load_common():
    """tools/cts/common-tally.py as a module (its name has a '-')"""
    spec = importlib.util.spec_from_file_location("common_tally", os.path.join(HERE, "common-tally.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


CT = load_common()


def describe():
    try:
        return subprocess.run(["git", "-C", REPO, "describe", "--always", "--dirty"],
                              capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def redact(text):
    """no board address in anything printed: IPv4 (with a port), user@host, UART paths"""
    text = re.sub(r"\b[\w.-]+@[\w.-]+\b", "<login>", text)
    text = re.sub(r"\b(?:\d{1,3}\.){3}\d{1,3}(?::\d+)?\b", "<addr>", text)
    return re.sub(r"/dev/tty\w+", "<uart>", text)


def normalize(text):
    """a cause key: numbers and hex out, as mali-deqp-tally.py's normalize"""
    t = re.sub(r"0x[0-9a-fA-F]+", "<hex>", text)
    t = re.sub(r"(?<![\w<])-?\d+(?:\.\d+)?", "#", t)
    return re.sub(r"\s+", " ", t).strip() or "-"


def scene_list():
    """[(slug, info)] of glmark2-host-ref.sh --list, checked against the host column"""
    p = subprocess.run(["bash", HOST_REF, "--list"], capture_output=True, text=True)
    scenes = [tuple(l.split("\t", 1)) for l in p.stdout.splitlines() if "\t" in l]
    if p.returncode != 0 or len(scenes) != SCENE_COUNT:
        raise Malformed("glmark2-host-ref.sh --list gave %d scenes (exit %d), not %d"
                        % (len(scenes), p.returncode, SCENE_COUNT))
    host = list(CT.read_picture_table(PICTURES)["columns"].get("host", {}))
    if host != [info for _, info in scenes]:
        raise Malformed("the host column of glmark2-pictures.txt is not the --list scenes")
    return scenes


def read_kv(path, what):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            lines = f.read().splitlines()
    except OSError as e:
        raise Malformed("%s: %s" % (path, e.strerror))
    out = {}
    for n, line in enumerate(lines, 1):
        if not line.strip() or line.startswith("#"):
            continue
        k, sep, v = line.partition("=")
        if not sep or not k.strip():
            raise Malformed("%s:%d is not key=value (%s)" % (path, n, what))
        out[k.strip()] = v.strip()
    return out


def read_env(rundir):
    env = read_kv(os.path.join(rundir, "env.txt"), "env.txt")
    if not re.fullmatch(r"[0-9a-f]{7,40}", env.get("commit", "")):
        raise Malformed("%s: commit %r is not 7 to 40 hex" % (os.path.join(rundir, "env.txt"),
                                                             env.get("commit", "")))
    return env


def read_status(path):
    st = read_kv(path, "exit=<code>, timeout=<0|1>")
    if not re.fullmatch(r"-?\d+", st.get("exit", "")):
        raise Malformed("%s: exit %r is not a number" % (path, st.get("exit", "")))
    if st.get("timeout") not in ("0", "1"):
        raise Malformed("%s: timeout %r is not 0 or 1" % (path, st.get("timeout", "")))
    return {"exit": int(st["exit"]), "timeout": st["timeout"] == "1",
            "signal_unknown": st.get("signal_unknown") == "1"}


def read_kmsg(path):
    text = CT.read_text(path)
    if text is None:
        raise Malformed("%s: unreadable" % path)
    for line in text.splitlines():
        m = KMSG_RE.match(line.strip())
        if m:
            g = m.groups()
            k = dict(zip(KMSG_KEYS, (int(x or 0) for x in g[:7])))
            k["first"] = g[7]
            return k
        if line.startswith("{"):
            try:
                j = json.loads(line)
                k = {key: int(j.get(key, 0) if key == "cs_faults" else j[key]) for key in KMSG_KEYS}
                k["first"] = str(j["first"])
                return k
            except (ValueError, KeyError, TypeError):
                pass
    raise Malformed("%s: no mali-kmsg.py line ('kmsg: resets=N ...')" % path)


def run_files(rundir, scenes):
    """{slug: {"log"|"kmsg"|"status": path}}; Malformed on an unknown or repeated file"""
    known = {s for s, _ in scenes}
    files, nn = {}, {}
    try:
        names = sorted(os.listdir(rundir))
    except OSError as e:
        raise Malformed("%s: %s" % (rundir, e.strerror))
    for name in names:
        m = FILE_RE.match(name)
        if not m:
            continue
        num, slug, kind = m.groups()
        if slug not in known:
            raise Malformed("%s: %s is no scene of glmark2-host-ref.sh --list" % (name, slug))
        if nn.setdefault(slug, num) != num:
            raise Malformed("%s: the scene %s is also numbered %s" % (name, slug, nn[slug]))
        files.setdefault(slug, {})[kind] = os.path.join(rundir, name)
    return files


def kmsg_memory_lines(path):
    """the kernel's memory-failure lines of a .kmsg, time stamps off (not mali-kmsg.py's own line)"""
    out = []
    for line in CT.plain_lines(CT.read_text(path) or ""):
        if line.startswith("{") or KMSG_RE.match(line.strip()) or not KERNEL_OOM_RE.search(line):
            continue
        out.append(STAMP_RE.sub("", line.strip()))
    return out


def first_line(text, rx):
    for line in CT.plain_lines(text):
        if rx.search(line):
            return line.strip()
    return None


def tally_scene(index, slug, info, paths, refs, tol):
    """one scene's row: verdict, cause, ms, fps, picture, judgement, kmsg count"""
    sc = {"index": index, "slug": slug, "info": info, "verdict": "ok", "cause": "", "also": [],
          "ms": None, "fps": None, "ms_src": None, "picture": None, "picture_raw": None,
          "judgement": ("absent", "no picture line"), "kmsg": 0, "kmsg_first": "-",
          "resets": 0, "exit": None, "timeout": False, "gl_errors": 0, "oom_lines": 0,
          "excerpt": []}
    if not paths:
        sc.update(verdict="not-run", cause="not run: no log, kmsg or status file",
                  judgement=("not-run", "the scene did not run"))
        return sc
    for kind in ("log", "status", "kmsg"):
        if kind not in paths:
            raise Malformed("%s: the %s has no .%s" % (slug, "scene", kind))
    log = paths["log"]
    text = CT.read_text(log)
    if text is None:
        raise Malformed("%s: unreadable" % log)
    st = read_status(paths["status"])
    km = read_kmsg(paths["kmsg"])
    sc.update(exit=st["exit"], timeout=st["timeout"], kmsg=sum(km[k] for k in KMSG_KEYS),
              kmsg_first=km["first"], resets=km["resets"])

    speed = CT.glmark2_speed(text)["scenes"]
    if len(speed) > 1:
        raise Malformed("%s: %d result lines, one scene a log" % (log, len(speed)))
    if speed:
        got = speed[0]["info"]
        if got and got != info:
            raise Malformed("%s: the result line names %s, the file is %s" % (log, got, info))
        sc.update(ms=speed[0]["ms"], fps=speed[0]["fps"], ms_src=speed[0]["ms_src"])
    pics = CT.glmark2_pictures(text)
    for other in pics["order"]:
        if other != info:
            raise Malformed("%s: the picture line names %s, the file is %s" % (log, other, info))
    if info in pics["scenes"]:
        sc["picture"] = pics["scenes"][info]
        sc["picture_raw"] = pics["raw"][info]
        sc["judgement"] = CT.judge_one_picture(pics, info, refs.get(info), tol)

    bad = {}
    for line in CT.plain_lines(text):
        for name, rx in CT.GLMARK2_BAD_LINES:
            if rx.search(line):
                bad[name] = bad.get(name, 0) + 1
    all_lines = CT.plain_lines(text)
    gl_oom = sum(1 for l in all_lines if GL_OOM_RE.search(l))
    kmem = kmsg_memory_lines(paths["kmsg"])
    sc["gl_errors"] = max(bad.get("gl-error", 0) - gl_oom, 0)
    sc["oom_lines"] = (bad.get("out-of-memory", 0) + bad.get("tiled-copy", 0) + gl_oom + len(kmem)
                       + sum(1 for l in all_lines if KERNEL_OOM_RE.search(l)
                             or any(rx.search(l) for _, rx in MALI_OOM_LINES)))
    ex = None
    for line in CT.plain_lines(text):
        m = EXIT_RE.search(line)
        if m:
            ex = m
    setup = first_line(text, SETUP_RE)

    # every reason, the verdict is the first in VERDICTS order
    found = []
    if st["timeout"]:
        found.append(("hang", "hang: no exit within the runner's time limit"
                      + ("" if km["first"] == "-" else "; kernel: " + normalize(km["first"]))))
    gf = ex and (int(ex.group(4)) or int(ex.group(5)))
    if sc["kmsg"] or gf:
        why = ", ".join("%s %d" % (k, km[k]) for k in KMSG_KEYS if km[k])
        if gf:
            why = (why + "; " if why else "") + "pair gpu_faults %s job_timeouts %s" % (ex.group(4), ex.group(5))
        found.append(("gpu-fault", "gpu-fault: %s" % (normalize(km["first"]) if km["first"] != "-"
                                                       else "the pair's exit line counts a lost job"),
                      why))
    if st["exit"] >= 128 and not st["timeout"]:
        found.append(("crash", "crash: killed by a signal (number not recorded)" if st.get("signal_unknown")
                      else "crash: signal %d" % (st["exit"] - 128)))
    elif ex is None and not st["timeout"] and LOADED_RE.search(text):
        # without the loaded line the board's Mesa ran (a fail below): no exit line is expected
        found.append(("crash", "crash: no yakogl-mali-exit line (the process died before atexit)"))
    if sc["oom_lines"]:
        l = first_line(text, OOM_RE) or kmem[0]
        found.append(("oom", "oom: " + normalize(STAMP_RE.sub("", l))))
    if sc["gl_errors"]:
        codes = sorted({"GL error %s in %s" % (m.group(1).lower(), m.group(2))
                        for m in GL_ERROR_RE.finditer(text) if int(m.group(1), 16) != 0x505})
        found.append(("gl-error", "gl-error: " + ", ".join(codes)))
    if not LOADED_RE.search(text):
        found.append(("fail", "fail: no 'yakogl-mali: loaded' line (not the panthor-gles pair)"))
    if ex and int(ex.group(2)):
        # before set up failed: a refused program is why a scene does not set up
        found.append(("fail", "fail: refused: %s" % normalize(ex.group(3).strip())))
    if setup:
        found.append(("fail", "fail: %s" % ("set up failed" if "Set up" in setup else "unsupported")))
    elif not speed or sc["fps"] is None:
        if not st["timeout"] and st["exit"] < 128:
            found.append(("fail", "fail: no result line"))
    if 0 < st["exit"] < 128 and not st["timeout"]:
        found.append(("fail", "fail: exit %d" % st["exit"]))
    for name in ("incomplete-texture", "draw-refused", "draw-not-recorded"):
        if bad.get(name):
            found.append(("fail", "fail: %s (%d lines)" % (name, bad[name])))
    if found:
        # A refused program or draw is the cause, and the GL errors after it (0x501
        # at glGetAttribLocation, 0x502 at the draw: what ES 2.0 asks for when a
        # client goes on with a program that did not link) are its consequence: its
        # fail outranks gl-error, as the runner's refused-draw does (the NanoPC-T6's
        # desktop blur and buffer scenes). Without a refusal gl-error keeps its rank.
        refusal = bool(ex and int(ex.group(2))) or any(bad.get(n) for n in ("draw-refused", "draw-not-recorded"))
        found.sort(key=lambda f: VERDICTS.index("fail") + 0.5 if refusal and f[0] == "gl-error"
                   else VERDICTS.index(f[0]))
        sc["verdict"], sc["cause"] = found[0][0], redact(found[0][1])
        sc["also"] = [redact(f[1]) for f in found[1:]]
    # the lines an issue quotes: the failure lines of the log, then the kernel's first
    pat = re.compile(r"GL error|out of GPU memory|no GPU memory|refused|incomplete|Set up failed|"
                     r"Unsupported|Error:|yakogl-mali-exit|Segmentation|Aborted|panic|no sample store|"
                     r"Out of memory:|page allocation failure")
    lines, seen = [], {}
    for l in CT.plain_lines(text):
        if pat.search(l):
            l = redact(l.strip())
            if l not in seen:
                lines.append(l)
            seen[l] = seen.get(l, 0) + 1
    sc["excerpt"] = [l if seen[l] == 1 else "%s (%d times)" % (l, seen[l]) for l in lines][:4]
    for l in list(dict.fromkeys(kmem))[:2]:
        sc["excerpt"].append("kmsg: " + redact(l))
    if km["first"] != "-" and "kmsg: " + redact(km["first"]) not in sc["excerpt"]:
        sc["excerpt"].append("kmsg: " + redact(km["first"]))
    return sc


def is_runner_dir(rundir):
    return (os.path.isfile(os.path.join(rundir, "scenes.tsv"))
            and os.path.isfile(os.path.join(rundir, "meta.txt")))


def runner_info(scene):
    """the runner's 'name:opts' (glmark2's -b form, the options in the order the
    default list gives them) as glmark2's '[name] opts' info string: the options
    sorted, as glmark2 prints them (upstream's pulsar:quads=5:texture=false:light=false
    is '[pulsar] light=false:quads=5:texture=false')"""
    name, sep, opts = scene.partition(":")
    opts = ":".join(sorted(o for o in opts.split(":") if o)) if sep else ""
    return "[%s] %s" % (name, opts or "<default>")


def read_runner(rundir, scenes):
    """a mali-glmark2-run.sh directory as the tally's own layout, in a new temp directory:
    returns (tempdir, {slug: runner verdict}); the caller removes it"""
    meta = read_kv(os.path.join(rundir, "meta.txt"), "meta.txt")
    try:
        with open(os.path.join(rundir, "scenes.tsv"), encoding="utf-8", errors="replace") as f:
            rows = [l.rstrip("\n").split("\t") for l in f if l.strip()]
    except OSError as e:
        raise Malformed("%s: %s" % (rundir, e.strerror))
    if not rows or rows[0][:3] != ["scene", "commit", "verdict"]:
        raise Malformed("%s: scenes.tsv has no 'scene commit verdict' header" % rundir)
    by_info = {info: slug for slug, info in scenes}
    tmp = tempfile.mkdtemp(prefix="glmark2-mali-tally.")
    try:
        started = meta.get("started", "")
        env = ["commit=" + meta.get("commit", ""), "kernel=" + meta.get("kernel", "absent")]
        if re.match(r"\d{4}-\d\d-\d\d", started):
            env.append("date=" + started[:10])
        with open(os.path.join(tmp, "env.txt"), "w") as f:
            f.write("\n".join(env) + "\n")
        runner, seen = {}, set()
        for n, row in enumerate(rows[1:], 1):
            if len(row) < 7:
                raise Malformed("%s: scenes.tsv row %d has %d fields, not 7" % (rundir, n, len(row)))
            info = runner_info(row[0])
            slug = by_info.get(info)
            if slug is None:
                raise Malformed("%s: scenes.tsv row %d: %s is no scene of glmark2-host-ref.sh --list"
                                % (rundir, n, row[0]))
            if slug in seen:
                raise Malformed("%s: scenes.tsv names %s twice" % (rundir, row[0]))
            seen.add(slug)
            verdict = row[2]
            if verdict not in RUNNER_VERDICTS:
                raise Malformed("%s: scenes.tsv row %d: unknown verdict %r" % (rundir, n, verdict))
            runner[slug] = verdict
            if RUNNER_VERDICTS[verdict] == "not-run":
                continue
            text = ""
            for ext in ("out", "err"):
                t = CT.read_text(os.path.join(rundir, "log", "%d.%s" % (n, ext)))
                if t is None:
                    raise Malformed("%s: log/%d.%s is unreadable" % (rundir, n, ext))
                text += t if t.endswith("\n") or not t else t + "\n"
            kl = CT.read_text(os.path.join(rundir, "kmsg", "%d.log" % n)) or ""
            cls = CT.read_text(os.path.join(rundir, "kmsg", "%d.class" % n)) or ""
            cline = next((l for l in cls.splitlines() if KMSG_RE.match(l.strip())), None)
            if cline is None:
                # 'kmsg: unavailable' or no file: the runner counted nothing
                cline = "kmsg: resets=0 timeouts=0 mmu_faults=0 fw_faults=0 gpu_faults=0 other_panthor=0 first=-"
            has_exit = any(EXIT_RE.search(l) for l in text.splitlines())
            speed = CT.glmark2_speed(text)["scenes"]
            code, timeout, unknown = 0, 0, 0
            if verdict == "hang":
                code, timeout = 137, 1
            elif verdict == "crash" and has_exit:
                code, unknown = 139, 1
            elif (verdict == "failed-scene" and speed and speed[0]["fps"] is not None
                  and not first_line(text, SETUP_RE)):
                code = 1
            base = os.path.join(tmp, "%02d-%s" % (n, slug))
            with open(base + ".log", "w") as f:
                f.write(text)
            with open(base + ".kmsg", "w") as f:
                f.write(cline.strip() + "\n" + "".join(
                    l + "\n" for l in kl.splitlines() if not l.startswith("#")))
            with open(base + ".status", "w") as f:
                f.write("exit=%d\ntimeout=%d\n%s" % (code, timeout, "signal_unknown=1\n" if unknown else ""))
        return tmp, runner
    except BaseException:
        shutil.rmtree(tmp, ignore_errors=True)
        raise


def tally(rundir, column="mali", table_path=PICTURES):
    scenes = scene_list()
    table = CT.read_picture_table(table_path)
    refs = table["columns"].get(column, {})
    tmp, runner = None, None
    if is_runner_dir(rundir):
        tmp, runner = read_runner(rundir, scenes)
    try:
        src = tmp or rundir
        env = read_env(src)
        files = run_files(src, scenes)
        rows = [tally_scene(i, slug, info, files.get(slug, {}), refs, table["tolerance"])
                for i, (slug, info) in enumerate(scenes, 1)]
    finally:
        if tmp:
            shutil.rmtree(tmp, ignore_errors=True)
    # a scene the runner list lacks (a --scenes subset) is not-run in the tally and not compared
    mismatch = [(r, runner[r["slug"]]) for r in rows
                if runner and r["slug"] in runner and RUNNER_VERDICTS[runner[r["slug"]]] != r["verdict"]]
    return {"rundir": rundir, "env": env, "rows": rows, "column": column,
            "column_known": column in table["columns"], "table": table_path,
            "runner": runner, "mismatch": mismatch}


# ---- summaries ---------------------------------------------------------------------

def counts(t):
    c = {v: 0 for v in VERDICTS}
    for r in t["rows"]:
        c[r["verdict"]] += 1
    return c


def ran(t):
    return [r for r in t["rows"] if r["verdict"] != "not-run"]


def score(t):
    """glmark2's rule (main-loop.cpp: score_ += average_fps() of each scene set up, / the
    count): None unless all 33 have an FPS"""
    fps = [r["fps"] for r in t["rows"] if r["fps"] is not None]
    return sum(fps) // len(fps) if len(fps) == SCENE_COUNT else None


def picture_counts(t):
    c = {}
    for r in t["rows"]:
        if r["verdict"] == "not-run":
            continue
        c[r["judgement"][0]] = c.get(r["judgement"][0], 0) + 1
    return c


def ms_text(v):
    return "-" if v is None else "%.2f" % v


def picture_short(r):
    p = r["picture"]
    if p is None:
        return "-" if r["picture_raw"] is None else r["picture_raw"].split(" (")[0]
    return "crc16 %s mean %d %d %d%s" % ((p["crc16"],) + p["mean"] + (" black" if p["black"] else "",))


def groups(t):
    """{cause: [row, ...]} of the scenes that are not ok, in first-seen order"""
    out = {}
    for r in t["rows"]:
        if r["verdict"] != "ok":
            out.setdefault(r["cause"], []).append(r)
    return out


def header(t):
    e = t["env"]
    return ("run %s: commit %s, board %s, kernel %s, pair sha256 %s"
            % (os.path.basename(os.path.normpath(t["rundir"])), e["commit"],
               redact(e.get("board", "absent")), redact(e.get("kernel", "absent")),
               e.get("pair_sha256", "absent")))


def summary_line(t):
    c = counts(t)
    ms = [r["ms"] for r in t["rows"] if r["ms"] is not None]
    s = score(t)
    return ("scenes run %d of %d; %s; GL error lines %d, out of GPU memory lines %d, panthor kernel "
            "lines %d (resets %d); %.2f ms/frame summed over %d scenes; score %s"
            % (len(ran(t)), SCENE_COUNT, ", ".join("%s %d" % (v, c[v]) for v in VERDICTS if c[v]),
               sum(r["gl_errors"] for r in t["rows"]), sum(r["oom_lines"] for r in t["rows"]),
               sum(r["kmsg"] for r in t["rows"]), sum(r["resets"] for r in t["rows"]), sum(ms), len(ms),
               "absent (not every scene has an FPS)" if s is None else
               "%d (glmark2's rule over the per-scene processes)" % s))


def picture_summary(t):
    pc = picture_counts(t)
    col = t["column"] + ("" if t["column_known"] else " (not in the table: every scene no-reference)")
    return ("pictures against column %s: %d scenes, %s"
            % (col, sum(pc.values()), ", ".join("%s %d" % (k, pc[k]) for k in sorted(pc)) or "none"))


def render_table(t):
    w = max(len(r["info"]) for r in t["rows"])
    out = ["glmark2-mali-tally: build %s" % describe(), header(t),
           "%-2s %-*s  %-9s %9s %5s  %-28s %-12s %s" % ("#", w, "scene", "verdict", "ms/frame", "FPS",
                                                        "picture", "judgement", "kmsg")]
    for r in t["rows"]:
        out.append("%02d %-*s  %-9s %9s %5s  %-28s %-12s %d" % (
            r["index"], w, r["info"], r["verdict"], ms_text(r["ms"]),
            "-" if r["fps"] is None else r["fps"], picture_short(r), r["judgement"][0], r["kmsg"]))
    out.append(summary_line(t))
    out.append(picture_summary(t))
    for r in t["rows"]:
        if r["verdict"] != "ok":
            out.append("scene %s: %s (%s)%s" % (r["info"], r["verdict"], r["cause"],
                                                "".join("; also %s" % a for a in r["also"])))
    for r in t["rows"]:
        st, why = r["judgement"]
        if r["verdict"] != "not-run" and (st != "match" or why):
            out.append("scene %s: picture %s (%s)" % (r["info"], st.replace("-", " "), why))
    return out


def render_picture_lines(t):
    out, notes = [], []
    for r in t["rows"]:
        if r["picture"] is not None:
            out.append("%s | %s | %s" % (t["column"], r["info"], r["picture_raw"]))
        else:
            notes.append("glmark2-mali-tally: %s has no picture value (%s), no line for it"
                         % (r["info"], r["judgement"][0]))
    return out, notes


def slow_text(t):
    parts = []
    for info in SLOW_SCENES:
        r = next(x for x in t["rows"] if x["info"] == info)
        name = info.split("]")[0][1:]
        if r["ms"] is None:
            parts.append("%s %s" % (name, r["verdict"]))
        else:
            parts.append("%s %.2f ms a frame (%d FPS)%s" % (name, r["ms"], r["fps"],
                                                           "" if r["verdict"] == "ok" else ", " + r["verdict"]))
    return ", ".join(parts)


def short(info):
    """'[build] use-vbo=false' -> 'build use-vbo=false'; '[terrain] <default>' -> 'terrain'"""
    name, _, opts = info[1:].partition("] ")
    return name if opts == "<default>" else "%s %s" % (name, opts)


def date_of(t):
    return t["env"].get("date") or "<date>"


def render_results_row(t):
    e, c = t["env"], counts(t)
    sha = e["commit"][:8]
    bad = [r for r in t["rows"] if r["verdict"] not in ("ok", "not-run")]
    s = score(t)
    gle = sum(r["gl_errors"] for r in t["rows"])
    oom = sum(1 for r in t["rows"] if r["oom_lines"])
    cell = ("%s, %d of %d scenes ran, %d failed, %d GL errors, %d out of GPU memory; main `%s`, %s "
            "(`%s`, kernel %s, pair libGLESv2.so.2 sha256 `%s`), one glmark2-es2 process per scene, "
            "800x600 on an EGL pbuffer; **%s**; %s; per-scene ms/frame and verdict below; %s"
            % ("no score (not every scene has an FPS)" if s is None else
               "%d (glmark2's rule over the per-scene processes)" % s,
               len(ran(t)), SCENE_COUNT, len(bad), gle, oom, sha, date_of(t),
               os.path.basename(os.path.normpath(t["rundir"])), redact(e.get("kernel", "absent")),
               e.get("pair_sha256", "absent")[:16], slow_text(t),
               "failed: " + "; ".join("%s: %s" % (short(r["info"]), r["cause"])
                                      for r in bad) if bad else "no failed scene",
               picture_summary(t)))
    if c["not-run"]:
        cell += "; not run: " + ", ".join(short(r["info"]) for r in t["rows"] if r["verdict"] == "not-run")
    resets = sum(r["resets"] for r in t["rows"])
    out = ["| NanoPC-T6 | %s | see \"Mali: NanoPC-T6 (RK3588, Valhall v10, panthor), gl_test sweep\" "
           "| %d (panthor resets in the per-scene kernel lines) |" % (cell.replace("|", "\\|"), resets), "",
           "- NanoPC-T6 glmark2-es2 per scene (main `%s`, %s; ms a frame is glmark2's FrameTime; "
           "picture against column `%s`):" % (sha, date_of(t), t["column"]), "",
           "  | # | Scene | ms/frame | FPS | Verdict | Picture | panthor lines |",
           "  | ---: | --- | ---: | ---: | --- | --- | ---: |"]
    for r in t["rows"]:
        out.append("  | %d | `%s` | %s | %s | %s | %s | %d |" % (
            r["index"], r["info"], ms_text(r["ms"]), "-" if r["fps"] is None else r["fps"],
            r["verdict"] if r["verdict"] == "ok" else r["cause"].replace("|", "\\|"),
            r["judgement"][0], r["kmsg"]))
    return out


JA_VERDICT = {"not-run": "未実行", "hang": "ハング", "gpu-fault": "GPU フォルト", "crash": "クラッシュ",
              "oom": "GPU メモリ不足", "gl-error": "GL エラー", "fail": "失敗", "ok": "正常"}
JA_PICTURE = {"match": "一致", "mismatch": "不一致", "black": "黒", "absent": "読み出し無し",
              "unreadable": "解読不能", "no-reference": "基準無し"}


def render_status_row(t):
    e = t["env"]
    c = counts(t)
    bad = [r for r in t["rows"] if r["verdict"] not in ("ok", "not-run")]
    ms = [r["ms"] for r in t["rows"] if r["ms"] is not None]
    slow = []
    for info in SLOW_SCENES:
        r = next(x for x in t["rows"] if x["info"] == info)
        name = info.split("]")[0][1:]
        slow.append("%s %s" % (name, "%.2f ms" % r["ms"] if r["ms"] is not None else JA_VERDICT[r["verdict"]]))
    pc = picture_counts(t)
    if t["column_known"]:
        pic = "絵は %s 列と %d 中 %d 一致（%s）。" % (
            t["column"], sum(pc.values()), pc.get("match", 0),
            "、".join("%s %d" % (JA_PICTURE.get(k, k), pc[k]) for k in sorted(pc) if k != "match") or "他は無し")
    else:
        pic = "絵は %s 列がまだ無いので判定していない（読み出せた絵 %d シーン、目では未確認）。" % (
            t["column"], sum(1 for r in t["rows"] if r["picture"] is not None))
    s = score(t)
    line = ("- **NanoPC-T6（RK3588、Mali-G610、panthor）の glmark2-es2。** main %s から 1 シーン 1 プロセス、"
            "800x600 の EGL pbuffer で 33 シーンを実行した（%s、%s）。実行 %d/%d、正常 %d、失敗 %d%s、"
            "GL エラー %d 件、GPU メモリ不足 %d シーン、panthor のカーネルメッセージ %d 件（リセット %d）。"
            "%s、%d シーン合計 %.2f ms/フレーム%s。%s"
            % (e["commit"][:8], date_of(t), os.path.basename(os.path.normpath(t["rundir"])),
               len(ran(t)), SCENE_COUNT, c["ok"], len(bad),
               "（%s）" % "、".join("%s: %s" % (short(r["info"]), JA_VERDICT[r["verdict"]]) for r in bad)
               if bad else "",
               sum(r["gl_errors"] for r in t["rows"]), sum(1 for r in t["rows"] if r["oom_lines"]),
               sum(r["kmsg"] for r in t["rows"]), sum(r["resets"] for r in t["rows"]),
               "、".join(slow), len(ms), sum(ms),
               "、スコア %d（シーンごとのプロセスに glmark2 の式を当てた値）" % s if s is not None else "",
               pic))
    if c["not-run"]:
        line += "未実行 %d シーン（%s）。" % (c["not-run"], "、".join(
            short(r["info"]) for r in t["rows"] if r["verdict"] == "not-run"))
    return [line]


def owned_309(cause):
    """a cause plan #309 owns (OWNER_309: a refused program, a refused or unrecorded draw); an
    oom cause is not, even when its line says the OSAL refused a buffer"""
    return any(cause.startswith("fail: " + name) for name in OWNER_309)


def render_causes(t):
    e = t["env"]
    run = os.path.basename(os.path.normpath(t["rundir"]))
    g = {c: rows for c, rows in groups(t).items() if rows[0]["verdict"] != "not-run"}
    out = ["glmark2-mali-tally: causes of %s (main `%s`): %d" % (run, e["commit"][:8], len(g))]
    for n, (cause, rows) in enumerate(g.items(), 1):
        verdict = rows[0]["verdict"]
        names = ", ".join("`%s`" % r["info"] for r in rows)
        title = "gpu-mali: glmark2-es2 %s on the NanoPC-T6: %s" % (
            short(rows[0]["info"]) if len(rows) == 1 else "%d scenes" % len(rows), cause)
        body = ["Cause %d: %s" % (n, cause), "",
                "The first glmark2-es2 run on the NanoPC-T6 (Mali-G610, panthor, back end gpu-mali, #20) "
                "from main `%s` (kernel %s, the tools/mali/panthor-gles pair, libGLESv2.so.2 sha256 `%s`), "
                "one glmark2-es2 process per scene at 800x600 on an EGL pbuffer, gives verdict %s in %d "
                "scene(s): %s." % (e["commit"][:8], redact(e.get("kernel", "absent")),
                                   e.get("pair_sha256", "absent")[:16], verdict, len(rows), names), ""]
        for r in rows:
            body.append("- `%s`: exit %s, timeout %d, panthor kernel lines %d%s%s" % (
                r["info"], r["exit"], int(r["timeout"]), r["kmsg"],
                "".join("; also %s" % a for a in r["also"]),
                "" if r["ms"] is None else "; %.2f ms a frame" % r["ms"]))
            for l in r["excerpt"]:
                body.append("  > %s" % l)
        body += ["", "Run directory `%s`, tallied by `tools/cts/glmark2-mali-tally.py`. Rerun the scene "
                 "alone with tools/cts/mali-glmark2-run.sh to reproduce. ROADMAP.md stage 2 asks for all 33 "
                 "scenes with no failed scene, no GL error and no scene out of GPU memory." % run]
        if any(owned_309(x) for r in rows for x in [r["cause"]] + r["also"]):
            body += ["", "A refusal of the Mali back end or its compiler: plan #309 owns these; "
                     "cross-reference its issue rather than opening a new one."]
        out += ["", "## Draft issue %d" % n, "", "Title: %s" % title, ""] + body
    if not g:
        out.append("no failed scene: no issue to open")
    nr = [r["info"] for r in t["rows"] if r["verdict"] == "not-run"]
    if nr:
        out += ["", "not run, no issue (the run is incomplete: rerun them): " + ", ".join("`%s`" % i for i in nr)]
    return out


def main(argv):
    mode, column, table, args, allow = None, "mali", PICTURES, [], False
    i = 0
    while i < len(argv):
        a = argv[i]
        if a in ("--picture-lines", "--results-row", "--status-row", "--causes"):
            mode = a
        elif a == "--allow-verdict-mismatch":
            allow = True
        elif a in ("--column", "--table"):
            i += 1
            if i >= len(argv):
                print("glmark2-mali-tally: %s needs a value" % a, file=sys.stderr)
                return 2
            if a == "--column":
                column = argv[i]
            else:
                table = argv[i]
        elif a.startswith("--"):
            print("glmark2-mali-tally: unknown option %s" % a, file=sys.stderr)
            return 2
        else:
            args.append(a)
        i += 1
    if len(args) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        t = tally(args[0], column, table)
    except (Malformed, SystemExit) as e:
        # SystemExit: common-tally's read_picture_table on a malformed table
        print("glmark2-mali-tally: malformed: %s" % e, file=sys.stderr)
        return 1
    if mode == "--picture-lines":
        out, notes = render_picture_lines(t)
        for n in notes:
            print(n, file=sys.stderr)
    elif mode == "--results-row":
        out = render_results_row(t)
    elif mode == "--status-row":
        out = render_status_row(t)
    elif mode == "--causes":
        out = render_causes(t)
    else:
        out = render_table(t)
    print("\n".join(out))
    sys.stdout.flush()
    for r, rv in t["mismatch"]:
        print("glmark2-mali-tally: %sscene %s: runner verdict %s, tally verdict %s (%s)"
              % ("allowed: " if allow else "", r["info"], rv, r["verdict"], r["cause"] or "-"),
              file=sys.stderr)
    return 3 if t["mismatch"] and not allow else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
