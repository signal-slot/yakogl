#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Scan an i.MX8MP dEQP console capture for GPU recoveries, watchdog stops and row times.

    tools/cts/imx8mp-console-scan.py [--watch SPEC] [--one-based] [-q] console.txt|console.ts|console.raw ...

The capture is the one tools/board.sh imx8mp writes into a run directory when
BOARD_UART is set (docs/es3/board-recipe-imx8mp.md): console.txt ("HH:MM:SS
line"), console.ts ("<unix time> line") or console.raw (the bytes, no time
stamps: the times print as "-" and no elapsed time is known). A directory
names its console.txt, else console.ts, else console.raw.

It prints, with the time stamp of each line:

  ROW   start    "deqp: selector N" of a table image: the row (table entry N,
                 counting from 0 as the selector does) begins
  REC   recovery a GPU recovery of src/vivante (gc_render.c)
  WDOG  watchdog/reset  the WDOG1 arming, its failures and end, and every
                 boot banner (a reset shows as a second banner)
  FAULT fault    a Zephyr fatal error or exception, dEQP's fatal/exception
                 line, a case that ended Crash/Timeout/InternalError/ResourceError
  END   finish   "deqp: summary:" of the row (the "deqp: done:" before it is listed too)

and then one line per row with its elapsed board time (selector line to its
finish line). A row that ends WITHOUT a finish line is printed as

    !! NO FINISH row N ... (hang | watchdog end | reset)

in the table and again at the end: the log shows a new boot banner or a new
row before any "deqp: summary:" (a reset: the 600 s WDOG1 window ran out or
the SoC faulted), or the "gc_wdog_arm: feeding bound ... over" line (the
watchdog end), or simply stops (a hang with the console still open). The
last case that printed a result is named, which is where the hang is.

--watch SPEC (e.g. 13-15,57,95,122-127,58-60) marks those rows with WATCH
when they appear, and lists the watched rows absent from a log (the host
pre-triage expects hangs on entries 13-15, 57, 95 and 122-127 and watchdog
ends on 58-60; a slice's log holds only its own rows, so absence is not an
error). SPEC counts as the selector does; --one-based reads it as table
line numbers from 1.

Exit status: 0 every row seen finished; 1 a row ended without a finish line;
2 bad input. --self-test: see
tools/cts/imx8mp-console-scan-selftest.sh.

The patterns are the strings the code prints (quoted where they are used).
"""
import argparse
import os
import re
import sys

# ---- what the code prints ----------------------------------------------------

# main.cpp:96 (the table image's selector): printk("deqp: selector %d\n", want)
ROW_START = re.compile(r'deqp: selector (\d+)\b')
# main.cpp:141: printk("deqp: case filter '%s'\n", ...), follows the selector
CASE_FILTER = re.compile(r"deqp: case filter '(.*)'")
# deqp_log.c:80: printk("deqp: %s %s\n", status, cur_case)
CASE_RESULT = re.compile(r'deqp: (\w+) (dEQP-\S+)')
BAD_STATUS = {'Crash', 'Timeout', 'InternalError', 'ResourceError'}
# deqp_log.c:121: "deqp: summary: %u cases begun, %u with a result, %u without"
# main.cpp:177:   "deqp: done: %d passed, %d failed, ..., complete=%d"
ROW_END = re.compile(r'deqp: (summary|done): (.*)')

# driver/src/vivante/gc_render.c (LOG_ERR / LOG_WRN, Zephyr prefixes them with
# "<err> gc_render: " or similar, so the match is not anchored):
#   :794 "recovering the GPU (%s): fences %u..%u abandoned, %u already done"
#   :818 "recover: GPU reset and re-initialised (reset %u); fences through %u report a lost context"
#   :962 "fence %u: wait abandoned after %u ms at the ..."
#   :978 "fence %u: nothing retired for %u ms, waited %u ms (completed %u)"
#   :981 "fence %u did not complete in %u ms (completed %u)"
#   :1002 "=== GC7000 stall dump: %s ==="
#   :701 "%s: the reset did not take (%d); the GPU is left stopped"
#   :732 "%s: re-init failed (%d) after a reset that took; ..."
RECOVERY = re.compile(
    r'recovering the GPU \(|recover: GPU reset and re-initialised|'
    r'fence \d+: wait abandoned|fence \d+: nothing retired|fence \d+ did not complete|'
    r'GC7000 stall dump|the reset did not take|re-init failed')

# One recovery prints two lines (:794 starts it, :818 ends it); the count is
# the starts.
RECOVERY_START = re.compile(r'recovering the GPU \(')

# driver/zephyr/lib/board/imx8mp/gc_wdog_arm.c:
#   :121 "gc_wdog_arm: armed WCR ... expiry %u ms, fed for %u s"   (the image's boot, WDOG1 running)
#   :112 "gc_wdog_arm: FAILED (rc %d, refusal %u): ...: resetting the SoC now"
#   :173 "gc_wdog_arm: feeding bound of %u s over; WDOG1 resets the SoC within %u ms"  (the watchdog end)
#   :144 "gc_wdog_arm: proof FAILED: no WDOG1 reset within %u s of arming: software reset"
#   :158 "gc_wdog_arm: proof: sample finished; waiting for WDOG1 ..."
WDOG_ARMED = re.compile(r'gc_wdog_arm: armed')
WDOG_END = re.compile(r'gc_wdog_arm: (FAILED|feeding bound|proof FAILED|proof: )')
# The RTI feeder of the AM62P/Pi 4 images ("RTI feeder lifetime over",
# "rti_arm: feeding bound of", tools/board.sh END_RE_DEFAULT): not this board,
# but a capture of the wrong image should still stand out.
WDOG_OTHER = re.compile(r'RTI feeder lifetime over|rti_arm: feeding bound of')
# A boot banner: Zephyr's "*** Booting Zephyr OS build ..." (the image
# starts), U-Boot's "U-Boot SPL 20xx" / "U-Boot 20xx" (the SoC reset and
# the board went through the boot loader again)
BOOT = re.compile(r'Booting Zephyr OS|U-Boot SPL \d|^U-Boot \d{4}\.|\bU-Boot \d{4}\.\d\d')

# Zephyr's fatal handler: ">>> ZEPHYR FATAL ERROR 0: CPU exception on CPU 0"
# (tools/board.sh finds it in the image's console), the AArch64 dump lines
# "ESR_ELn: 0x..." / "ELR_ELn: 0x...", dEQP's main.cpp:69 "deqp: fatal: dEQP ended
# the run (die()); later cases did not run" and :183 "deqp: exception: %s".
FAULT = re.compile(r'ZEPHYR FATAL ERROR|\bESR_EL\d|Fatal exception|deqp: fatal:|deqp: exception:|'
                   r'kernel panic|Kernel panic')

STAMP_HMS = re.compile(r'^(\d\d):(\d\d):(\d\d) (.*)$')
STAMP_UNIX = re.compile(r'^(\d{9,}(?:\.\d+)?) (.*)$')


def clean(b):
    return b.replace(b'\r', b'').replace(b'\0', b'').decode('utf-8', 'replace')


def resolve(path):
    """A run directory names its console.txt, else console.ts, else console.raw."""
    if os.path.isdir(path):
        for n in ('console.txt', 'console.ts', 'console.raw'):
            if os.path.isfile(os.path.join(path, n)):
                return os.path.join(path, n)
        raise OSError('%s: no console.txt, console.ts or console.raw' % path)
    return path


def read_log(path):
    """[(seconds or None, text)] for a capture; HH:MM:SS wraps at midnight are unrolled."""
    out, day, last = [], 0, None
    with open(path, 'rb') as f:
        for raw in f:
            line = clean(raw.rstrip(b'\n'))
            t = None
            m = STAMP_HMS.match(line)
            if m:
                t = int(m.group(1)) * 3600 + int(m.group(2)) * 60 + int(m.group(3))
                if last is not None and t + day * 86400 < last - 3600:
                    day += 1
                t += day * 86400
                last = t
                line = m.group(4)
            else:
                m = STAMP_UNIX.match(line)
                if m:
                    t = float(m.group(1))
                    line = m.group(2)
            out.append((t, line))
    return out


def fmt_t(t):
    if t is None:
        return '--:--:--'
    if t >= 86400 * 365:   # a unix time
        import time
        return time.strftime('%H:%M:%S', time.localtime(int(t)))
    t = int(t) % 86400
    return '%02d:%02d:%02d' % (t // 3600, t // 60 % 60, t % 60)


def fmt_el(a, b):
    if a is None or b is None:
        return '?'
    return '%ds' % round(b - a)


def parse_spec(spec, base):
    rows = set()
    for part in filter(None, (spec or '').split(',')):
        lo, _, hi = part.partition('-')
        for i in range(int(lo), int(hi or lo) + 1):
            rows.add(i - base)
    return rows


class Row:
    def __init__(self, n, t, lineno):
        self.n, self.t0, self.line0 = n, t, lineno
        self.filter = ''
        self.results = 0
        self.bad = 0
        self.last_case = ''
        self.recoveries = 0
        self.t_end = None      # the finish line's time
        self.t_stop = None     # where an unfinished row stopped
        self.summary = ''
        self.how = None        # None while open; 'finish', 'reset', 'watchdog end', 'hang', 'next row'
        self.last_t = t        # the last time stamp seen inside the row
        self.wdog_end = False
        self.faults = 0


def scan(lines, watch, out):
    rows, cur = [], None
    boots = 0

    def close(how, t):
        nonlocal cur
        if cur is not None and cur.how is None:
            cur.how = how
            if how != 'finish':
                cur.t_stop = t
        cur = None

    for i, (t, text) in enumerate(lines, 1):
        tag = None
        m = ROW_START.search(text)
        if m:
            close('next row' if cur is not None else '', t)
            cur = Row(int(m.group(1)), t, i)
            rows.append(cur)
            tag = 'ROW  '
            text = text.strip() + ('   [WATCH]' if cur.n in watch else '')
        elif BOOT.search(text):
            boots += 1
            if cur is not None and cur.how is None:
                close('watchdog end' if cur.wdog_end else 'reset', t)
            tag = 'WDOG '
        elif WDOG_ARMED.search(text) or WDOG_END.search(text) or WDOG_OTHER.search(text):
            tag = 'WDOG '
            if cur is not None and WDOG_END.search(text) and 'proof: sample' not in text:
                cur.wdog_end = True
        elif RECOVERY.search(text):
            tag = 'REC  '
            if cur is not None and RECOVERY_START.search(text):
                cur.recoveries += 1
        elif ROW_END.search(text):
            tag = 'END  '
            # "done:" comes with the summary; the summary is the finish line
            if cur is not None and ROW_END.search(text).group(1) == 'summary':
                cur.t_end, cur.summary = t, ROW_END.search(text).group(0)
                cur.how = 'finish'
                if t is not None:
                    cur.last_t = t
                cur = None
            elif ROW_END.search(text).group(1) == 'summary':
                text += '   (no row open)'
        elif FAULT.search(text):
            tag = 'FAULT'
            if cur is not None:
                cur.faults += 1
        else:
            m = CASE_RESULT.search(text)
            if m and cur is not None:
                cur.results += 1
                cur.last_case = m.group(2)
                if m.group(1) in BAD_STATUS:
                    cur.bad += 1
                    tag = 'FAULT'
            elif cur is not None:
                m = CASE_FILTER.search(text)
                if m:
                    cur.filter = m.group(1)
        if cur is not None and t is not None:
            cur.last_t = t
        if tag:
            out.append('%s %s %s' % (fmt_t(t), tag, text.strip()))
    if cur is not None and cur.how is None:
        cur.how = 'hang'   # the log stops with the row open
        cur.t_stop = lines[-1][0] if lines else None
    return rows, boots


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('log', nargs='+')
    ap.add_argument('--watch', help='rows to mark, e.g. 13-15,57,95,122-127')
    ap.add_argument('--one-based', action='store_true', help='--watch counts table lines from 1')
    ap.add_argument('-q', '--quiet', action='store_true', help='only the per-row table')
    a = ap.parse_args(argv)
    try:
        watch = parse_spec(a.watch, 1 if a.one_based else 0)
    except ValueError:
        print('imx8mp-console-scan: bad --watch spec %r' % a.watch, file=sys.stderr)
        return 2
    bad = 0
    for path in a.log:
        try:
            path = resolve(path)
            lines = read_log(path)
        except OSError as e:
            print('imx8mp-console-scan: %s' % e, file=sys.stderr)
            return 2
        events = []
        rows, boots = scan(lines, watch, events)
        print('== %s (%d lines, %d boot banner(s), %d row(s))' % (path, len(lines), boots, len(rows)))
        if not a.quiet:
            print('\n'.join(events))
        print('-- rows: selector, start, elapsed board time, results, recoveries, end')
        unfinished = []
        for r in rows:
            if r.how == 'finish':
                el = fmt_el(r.t0, r.t_end)
                end = 'finish   ' + r.summary
            else:
                stop = r.t_stop or r.last_t
                el = fmt_el(r.t0, stop) + '+'
                end = '!! NO FINISH (%s)' % ('hang' if r.how == 'hang' else r.how)
                unfinished.append(r)
            w = '  [WATCH]' if r.n in watch else ''
            print('row %3d  %s  %6s  results %4d  recoveries %2d  %s%s' % (
                r.n, fmt_t(r.t0), el, r.results, r.recoveries, end, w))
            if r.how != 'finish':
                print('         last result: %s' % (r.last_case or '(none)'))
        seen = {r.n for r in rows}
        missing = sorted(watch - seen)
        for r in unfinished:
            print('!! NO FINISH row %d (%s) at %s; last result %s; %d result(s), %d recovery(ies)%s' % (
                r.n, r.how, fmt_t(r.t0), r.last_case or 'none', r.results, r.recoveries,
                '  [WATCH: expected]' if r.n in watch else ''))
        if missing:
            print('-- watched rows not in this log: %s' % ','.join(str(n) for n in missing))
        bad += len(unfinished)
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
