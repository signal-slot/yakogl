#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Scan Pi 4 dEQP consoles for GPU recoveries, hung jobs, resets and run times per image.

    tools/cts/pi4-console-scan.py [-q] <LABEL>.console.txt|<session dir> ...
    tools/cts/pi4-console-scan.py --self-test

The input is the console of one v3d_deqp image on the Raspberry Pi 4
(docs/es3/board-recipe-pi4.md section 4): the <LABEL>.console.txt files of a
tools/cts/log/<date>-pi4-gles3-*/ session, either without time stamps (the
copies of the leased serial console, 2026-10-31-pi4-gles3-a) or with the
"HH:MM:SS " stamps tools/board.sh pi4 writes, or a console.ts ("<unix time>
line"). A directory names every *.console.txt in it, by name. The label is
the file name before ".console.txt". Paths print as given.

It prints, with the time stamp of each line ("--:--:--" without stamps):

  BOOT  boot     the firmware's "Read start4.elf" (a boot of the SoC: one
                 before the image, one more after a reset), Zephyr's banner,
                 a Linux boot after the image
  START start    "deqp: case filter '...'": the image's dEQP run begins
  REC   recovery "osal(v3d): job N: GPU reset (recovery N)" (src/osal/zephyr_v3d.c
                 recover(): a job did not end and the GPU was reset)
  HANG  hang     "osal(v3d): job N bin|render did not end", "TFU job did not
                 end", "job N: the GPU reset failed" (the GPU is taken as hung,
                 nothing more runs), the OSAL's "FIRST FAILURE" registers
  SLOW  slow     "osal(v3d): job N bin|render took N ms (slow; ...)": ended late
  WDOG  watchdog "board: watchdog armed", "board: PM watchdog resets the SoC ~N s
                 after boot", "board: feeding stopped", "v3d_deqp: the record
                 at N s after boot at the latest"
  FAULT fault    Zephyr's fatal error, dEQP's "FATAL ERROR"/"deqp: fatal:"/
                 "deqp: exception:", an MMU fault line, a case that ended
                 Crash/Timeout/InternalError/ResourceError
  END   finish   "deqp: done:", "deqp: summary:" (the finish line), dEQP's
                 "Test run was ABORTED!", the record's heading
                 ("v3d_deqp <ending>: build ...") and its end line
                 ("v3d_deqp: build <sha> <ending>: N cases, ...")

then one line per image: start, finish, the board time between them (the
stamps), the run time the record states ("N cases in N ms"), the cases,
recoveries, job timeouts (jobs that did not end), hung (the GPU reset
failed), MMU faults, slow jobs, resets (a firmware boot after the run began),
fatal faults and how it ended. Without stamps, start and finish are seconds
after boot from the record ("record at N s after boot": "@N s").

The record (driver/samples/zephyr/deqp/src/v3d_board.c print_record) is
printed after the run, on the UART too, and repeats what the run printed:
"slow jobs: ...; MMU faults: N; recoveries: N; hung: ...", "FIRST FAILURE",
the first hung job's line, "driver: <line>" copies of driver lines, the
fault lines. Its lines are read for its own counts only, never counted as
events: a count is the larger of the live lines and the record's figure.

An image that ends without "deqp: summary:" is printed as

    !! NO FINISH <LABEL> (<how>) ...

in the table and again at the end, how being: fatal (the record's "FATAL"
heading, a Zephyr fatal error), photo deadline (the record's "INCOMPLETE",
then the PM watchdog), watchdog reset (a firmware boot after the run began,
the PM watchdog announced), reset (the same, no watchdog announced) or hang
(the console stops with the run open). The last case with a result is named.

Exit status: 0 every image finished; 1 an image ended without a finish line;
2 bad input. --self-test scans tools/cts/testdata/pi4-console-scan/.

The patterns are the strings the code prints (quoted where they are used).
"""
import argparse
import os
import re
import sys

# ---- what the code prints ----------------------------------------------------

# The firmware's boot (start4.elf loaded: once per SoC boot), Zephyr's banner,
# Linux after the image (the Pi's normal boot after the watchdog reset).
FW_BOOT = re.compile(r'Read start4\w*\.elf ')
ZEPHYR_BOOT = re.compile(r'\*\*\* Booting Zephyr OS')
LINUX_BOOT = re.compile(r'Booting Linux on|Linux version \d')

# deqp main.cpp: printk("deqp: case filter '%s'\n", ...)
RUN_START = re.compile(r"^deqp: case filter '(.*)'")
# deqp_log.c: printk("deqp: %s %s\n", status, cur_case)
CASE_RESULT = re.compile(r'^deqp: (\w+) (dEQP-\S+)')
BAD_STATUS = {'Crash', 'Timeout', 'InternalError', 'ResourceError'}
# deqp_log.c "deqp: summary: %u cases begun, ...", main.cpp "deqp: done: ..."
RUN_END = re.compile(r'^deqp: (summary|done): ')
# dEQP's tcuApp.cpp when a session stops early (a ResourceError)
ABORTED = re.compile(r'^Test run was ABORTED!')
# v3d_board.c record_case(): "v3d_deqp: case #%u %c %ums k%u/%u ..."
CASE_LINE = re.compile(r'^v3d_deqp: case #(\d+) (\S) (\d+)ms ')

# src/osal/zephyr_v3d.c (printk, unprefixed):
#   recover():    "osal(v3d): job %u: GPU reset (recovery %u)"
#                 "osal(v3d): job %u: the GPU reset failed (%d): the GPU is taken as hung; ..."
#   bin wait:     "osal(v3d): job %u bin did not end (%d, after %u ms)"
#   render wait:  "osal(v3d): job %u render did not end (%d, after %u ms, %u ms of it in this wait)"
#   TFU:          "osal(v3d): TFU job did not end (%d after %u us): ..."
#   first_failure() -> diag_regs(): "osal(v3d): FIRST FAILURE: %s, rc %d, ..." and
#                 diag_line()'s "... job %u MMU fault: mmu_ctl ..." when the hub saw one
#   slow:         "osal(v3d): job %u bin|render took %u ms (slow; past %u ms)"
# A did-not-end rc of -14 (-EFAULT) is an MMU fault (diag_mmu_faults++).
RECOVERY = re.compile(r'^osal\(v3d\): job \d+: GPU reset \(recovery (\d+)\)')
TIMEOUT = re.compile(r'^osal\(v3d\): (?:job \d+ (?:bin|render)|TFU job) did not end \((-?\d+)')
RESET_FAILED = re.compile(r'^osal\(v3d\): job (\d+): the GPU reset failed')
FIRST_FAILURE = re.compile(r'^osal\(v3d\): FIRST FAILURE')
MMU_FAULT = re.compile(r' MMU fault: ')
SLOW = re.compile(r'^osal\(v3d\): job \d+ (?:bin|render) took \d+ ms \(slow')
EFAULT = -14

# driver/zephyr/lib/board/rpi4/rpi_wdog.c, board_rpi4.c (the "armed" line can
# follow the firmware's last MESS line on the same row), v3d_board.c init
WDOG = re.compile(r'board: watchdog armed|^board: PM watchdog resets the SoC|'
                  r'^board: feeding stopped|^board: NO watchdog|^v3d_deqp: the record at \d+ s')
WDOG_ANNOUNCED = re.compile(r'board: PM watchdog resets the SoC|board: watchdog armed')

# Zephyr's fatal handler, dEQP's own fatal lines (stdout "FATAL ERROR: ...",
# main.cpp "deqp: fatal:" / "deqp: exception:"), board/common/board.c's stop
FAULT = re.compile(r'ZEPHYR FATAL ERROR|\bESR_EL\d|^FATAL ERROR|^deqp: fatal:|^deqp: exception:|'
                   r'^board: FAIL |Kernel panic')

# v3d_board.c print_record(): the heading, then the record's lines, then the
# end line. Between them every line belongs to the record.
REC_HEAD = re.compile(r'^v3d_deqp ([A-Z][^:]*): build (\S+), filter ')
REC_TIMES = re.compile(r'^  (\d+) cases in (\d+) ms, record at (\d+) s after boot, reset at ~(\d+) s')
REC_JOBS = re.compile(r'^  jobs: slow in (\d+) cases.*?(?:; the first hung or timed-out job in #(\d+))?$')
# osal_v3d_diag_print(): "slow jobs: %u (...); MMU faults: %u; recoveries: %u; hung: none" or
# "hung: job %u (the GPU reset failed), ..."
REC_DIAG = re.compile(r'^slow jobs: (\d+) .*; MMU faults: (\d+); recoveries: (\d+); hung: (none|job (\d+))')
REC_IN_PROGRESS = re.compile(r'^  in progress: (.*)')
REC_END = re.compile(r'^v3d_deqp: build (\S+) (.+?): (\d+) cases, ')

STAMP_HMS = re.compile(r'^(\d\d):(\d\d):(\d\d) (.*)$')
STAMP_UNIX = re.compile(r'^(\d{9,}(?:\.\d+)?) (.*)$')


def clean(b):
    return b.replace(b'\r', b'').replace(b'\0', b'').decode('utf-8', 'replace')


def inputs(path):
    """A session directory names its *.console.txt files, by name; paths stay as given."""
    if os.path.isdir(path):
        names = sorted(n for n in os.listdir(path) if n.endswith('.console.txt'))
        if not names:
            raise OSError('%s: no *.console.txt' % path)
        return [os.path.join(path, n) for n in names]
    return [path]


def label_of(path):
    base = os.path.basename(path)
    for suffix in ('.console.txt', '.console.ts', '.txt', '.ts'):
        if base.endswith(suffix):
            return base[:-len(suffix)] or base
    return base


def read_lines(raw_lines):
    """[(seconds or None, text)]; HH:MM:SS wraps at midnight are unrolled."""
    out, day, last = [], 0, None
    for raw in raw_lines:
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


def read_log(path):
    with open(path, 'rb') as f:
        return read_lines(f.readlines())


def fmt_t(t):
    if t is None:
        return '--:--:--'
    if t >= 86400 * 365:   # a unix time
        import time
        return time.strftime('%H:%M:%S', time.localtime(int(t)))
    t = int(t) % 86400
    return '%02d:%02d:%02d' % (t // 3600, t // 60 % 60, t % 60)


class Run:
    """One dEQP run: from its case filter line to its summary (or where it stopped)."""

    def __init__(self, label, t, booted, wdog):
        self.label = label
        self.t0 = t
        self.t_boot = booted
        self.t_end = None
        self.t_stop = None
        self.last_t = t
        self.how = None          # None while open, 'finish', or how it stopped
        self.summary = ''
        self.results = 0
        self.bad = 0
        self.last_case = ''
        self.cases_ms = 0        # the per-case times of "v3d_deqp: case #N"
        self.case_lines = 0
        # live events
        self.recoveries = 0
        self.timeouts = 0
        self.mmu = 0
        self.slow = 0
        self.hung = None         # the job whose reset failed
        self.faults = 0
        self.resets = 0
        self.wdog = wdog         # the PM watchdog was announced
        self.aborted = False
        # the record (v3d_board.c print_record)
        self.ending = ''         # its heading's ending: DONE..., STOPPED..., INCOMPLETE..., FATAL...
        self.rec_cases = None
        self.rec_ms = None
        self.rec_at = None
        self.rec_reset_at = None
        self.rec_recoveries = 0
        self.rec_mmu = 0
        self.rec_slow = 0
        self.rec_hung = None
        self.rec_first_hung = None
        self.in_progress = ''
        self.rec_end = ''

    # a count is the larger of the live lines and the record's figure: the
    # record repeats the run's events, it does not add to them
    def n_recoveries(self):
        return max(self.recoveries, self.rec_recoveries)

    def n_timeouts(self):
        return max(self.timeouts, 1 if self.rec_first_hung is not None else 0)

    def n_mmu(self):
        return max(self.mmu, self.rec_mmu)

    def n_slow(self):
        return max(self.slow, self.rec_slow)

    def hung_job(self):
        return self.hung if self.hung is not None else self.rec_hung


def scan(label, lines, out):
    """Scan one console. Returns (runs, firmware boots, Zephyr boots)."""
    runs, cur = [], None
    fw_boots = zephyr_boots = 0
    t_boot = None
    wdog = False
    in_record = False

    def stop(how, t):
        nonlocal cur
        if cur is not None and cur.how is None:
            cur.how = how
            cur.t_stop = t if t is not None else cur.last_t
        cur = None

    last = None   # the last run, open or not: the record and a later reset belong to it
    for t, text in lines:
        tag = None
        if FW_BOOT.search(text):
            fw_boots += 1
            in_record = False
            tag = 'BOOT '
            if last is not None and fw_boots > 1:
                last.resets += 1
                if last.how is None:
                    if last.ending.startswith('FATAL') or last.faults:
                        how = 'fatal, then reset'
                    elif last.ending.startswith('INCOMPLETE'):
                        how = 'photo deadline'
                    else:
                        how = 'watchdog reset' if last.wdog else 'reset'
                    cur = last
                    stop(how, t)
        elif ZEPHYR_BOOT.search(text):
            zephyr_boots += 1
            in_record = False
            t_boot = t
            tag = 'BOOT '
        elif LINUX_BOOT.search(text):
            tag = 'BOOT '
        elif in_record:
            # the record repeats the run's lines: its own figures only
            r = last
            m = REC_END.match(text)
            if m:
                in_record = False
                tag = 'END  '
                if r is not None:
                    r.rec_end = text.strip()
            elif r is not None:
                m = REC_TIMES.match(text)
                if m:
                    r.rec_cases, r.rec_ms = int(m.group(1)), int(m.group(2))
                    r.rec_at, r.rec_reset_at = int(m.group(3)), int(m.group(4))
                m = REC_JOBS.match(text)
                if m:
                    if m.group(2) is not None:
                        r.rec_first_hung = int(m.group(2))
                m = REC_DIAG.match(text)
                if m:
                    r.rec_slow, r.rec_mmu, r.rec_recoveries = int(m.group(1)), int(m.group(2)), int(m.group(3))
                    if m.group(5) is not None:
                        r.rec_hung = int(m.group(5))
                m = REC_IN_PROGRESS.match(text)
                if m:
                    r.in_progress = m.group(1)
        else:
            m = RUN_START.match(text)
            if m:
                stop('next run', t)
                n = sum(1 for r in runs if r.label.split('#')[0] == label)
                cur = Run(label if n == 0 else '%s#%d' % (label, n + 1), t, t_boot, wdog)
                runs.append(cur)
                last = cur
                tag = 'START'
            elif REC_HEAD.match(text):
                in_record = True
                tag = 'END  '
                if last is not None:
                    last.ending = REC_HEAD.match(text).group(1)
                    if last.ending.startswith('FATAL'):
                        # the record of a fault: one more only when the
                        # fault printed nothing of its own before it
                        tag = 'FAULT'
                        if last.how is None and not last.faults:
                            last.faults += 1
            elif RUN_END.match(text):
                tag = 'END  '
                if cur is not None and RUN_END.match(text).group(1) == 'summary':
                    cur.t_end, cur.summary = t, text.strip()
                    cur.how = 'finish'
                    if t is not None:
                        cur.last_t = t
                    cur = None
                elif cur is None and RUN_END.match(text).group(1) == 'summary':
                    text += '   (no run open)'
            elif ABORTED.match(text):
                tag = 'END  '
                if cur is not None:
                    cur.aborted = True
            elif RECOVERY.match(text):
                tag = 'REC  '
                if cur is not None:
                    cur.recoveries += 1
            elif TIMEOUT.match(text):
                tag = 'HANG '
                if cur is not None:
                    cur.timeouts += 1
                    if int(TIMEOUT.match(text).group(1)) == EFAULT:
                        cur.mmu += 1
            elif RESET_FAILED.match(text):
                tag = 'HANG '
                if cur is not None:
                    cur.hung = int(RESET_FAILED.match(text).group(1))
            elif FIRST_FAILURE.match(text):
                tag = 'FAULT' if MMU_FAULT.search(text) else 'HANG '
            elif SLOW.match(text):
                tag = 'SLOW '
                if cur is not None:
                    cur.slow += 1
            elif WDOG.search(text):
                tag = 'WDOG '
                if WDOG_ANNOUNCED.search(text):
                    wdog = True
                    if cur is not None:
                        cur.wdog = True
            elif FAULT.search(text):
                tag = 'FAULT'
                if cur is not None:
                    cur.faults += 1
            elif cur is not None:
                m = CASE_RESULT.match(text)
                if m:
                    cur.results += 1
                    cur.last_case = m.group(2)
                    if m.group(1) in BAD_STATUS:
                        cur.bad += 1
                        tag = 'FAULT'
                else:
                    m = CASE_LINE.match(text)
                    if m:
                        cur.case_lines += 1
                        cur.cases_ms += int(m.group(3))
        if cur is not None and t is not None:
            cur.last_t = t
        if tag:
            out.append('%s %s %s' % (fmt_t(t), tag, text.strip()))
    if cur is not None and cur.how is None:
        if cur.ending.startswith('FATAL') or cur.faults:
            how = 'fatal'
        elif cur.ending.startswith('INCOMPLETE'):
            how = 'photo deadline'
        else:
            how = 'hang'   # the console stops with the run open
        stop(how, lines[-1][0] if lines else None)
    return runs, fw_boots, zephyr_boots


def fmt_s(t):
    return '?' if t is None else '%ds' % round(t)


def row(r):
    """The image's line in the table."""
    done = r.how == 'finish'
    if r.t0 is not None:
        start = fmt_t(r.t0)
        finish = fmt_t(r.t_end) if done else 'none'
        until = r.t_end if done else r.t_stop   # an unfinished run: up to where it stopped
        board = fmt_s(until - r.t0 if until is not None else None)
        if not done and board != '?':
            board += '+'
    else:
        # the record's own clock: it starts the run's time at boot (SYS_INIT)
        if r.rec_at is not None and r.rec_ms is not None:
            start = '@%ds' % max(0, round(r.rec_at - r.rec_ms / 1000.0))
        else:
            start = '?'
        finish = '@%ds' % r.rec_at if done and r.rec_at is not None else ('?' if done else 'none')
        board = '?'
    if r.rec_ms is not None:
        run = '%.1f s' % (r.rec_ms / 1000.0)
    elif r.case_lines:
        run = '%.1f s+' % (r.cases_ms / 1000.0)   # the cases that printed a time
    else:
        run = '?'
    cases = r.rec_cases if r.rec_cases is not None else r.results
    hung = r.hung_job()
    end = ('finish   ' + r.summary) if r.how == 'finish' else '!! NO FINISH (%s)' % r.how
    if r.ending:
        end += '; record: %s' % r.ending
    return ('%-6s start %-8s  finish %-8s  board %-6s  run %-9s  cases %5d  recoveries %d  '
            'job timeouts %d  hung %s  MMU faults %d  slow %d  resets %d  faults %d  %s' % (
                r.label, start, finish, board, run, cases, r.n_recoveries(), r.n_timeouts(),
                '-' if hung is None else 'job %d' % hung, r.n_mmu(), r.n_slow(), r.resets,
                r.faults + r.bad, end))


def report(paths_lines, quiet, print_fn=print):
    """Scan [(path as given, lines)] and print. Returns (runs, unfinished)."""
    all_runs, unfinished = [], []
    for path, lines in paths_lines:
        events = []
        runs, fw, zb = scan(label_of(path), lines, events)
        print_fn('== %s (%d lines, %d firmware boot(s), %d Zephyr boot(s), %d run(s))' % (
            path, len(lines), fw, zb, len(runs)))
        if not quiet:
            for e in events:
                print_fn(e)
        if not runs:
            print_fn('!! NO RUN in %s: no "deqp: case filter" line' % path)
        for r in runs:
            print_fn(row(r))
            if r.how != 'finish':
                print_fn('         last result: %s%s' % (
                    r.last_case or '(none)',
                    '; in progress: %s' % r.in_progress if r.in_progress else ''))
                unfinished.append(r)
        all_runs += runs
    if len(paths_lines) > 1 or unfinished:
        print_fn('-- %d image run(s): %d finished, %d without a finish line; recoveries %d, job timeouts %d, '
                 'hung %d, MMU faults %d, resets %d, faults %d' % (
                     len(all_runs), len(all_runs) - len(unfinished), len(unfinished),
                     sum(r.n_recoveries() for r in all_runs), sum(r.n_timeouts() for r in all_runs),
                     sum(1 for r in all_runs if r.hung_job() is not None),
                     sum(r.n_mmu() for r in all_runs), sum(r.resets for r in all_runs),
                     sum(r.faults + r.bad for r in all_runs)))
    for r in unfinished:
        print_fn('!! NO FINISH %s (%s); last result %s; %d result(s), %d recovery(ies), %d job timeout(s)' % (
            r.label, r.how, r.last_case or 'none', r.results, r.n_recoveries(), r.n_timeouts()))
    return all_runs, unfinished


# ---- self-test ---------------------------------------------------------------

FIXTURES = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'testdata', 'pi4-console-scan')
SHOWN = 'tools/cts/testdata/pi4-console-scan'


def self_test():
    """The fixtures, scanned as a session directory: the expected output byte for
    byte, and the counts that matter checked on their own."""
    fails = []

    def check(what, ok):
        if not ok:
            fails.append(what)

    names = sorted(n for n in os.listdir(FIXTURES) if n.endswith('.console.txt'))
    check('four fixtures', names == ['clean.console.txt', 'fatal.console.txt',
                                     'recovery.console.txt', 'watchdog.console.txt'])
    got = []
    pl = [('%s/%s' % (SHOWN, n), read_log(os.path.join(FIXTURES, n))) for n in names]
    runs, unfinished = report(pl, False, got.append)
    by = {r.label: r for r in runs}

    c = by.get('clean')
    check('clean: finished, run 22.0 s, 475 cases, nothing counted',
          c is not None and c.how == 'finish' and c.rec_ms == 21965 and c.rec_cases == 475 and
          c.n_recoveries() == 0 and c.n_timeouts() == 0 and c.resets == 0 and c.faults == 0)
    check('clean: @0s to @22s after boot', any(l.startswith('clean  start @0s') and 'finish @22s' in l
                                               for l in got))
    v = by.get('recovery')
    # two live recoveries, repeated by the record ("recoveries: 2", "driver:"
    # copies, the first hung job's line): 2, not 4 or 6
    check('recovery: 2 recoveries, 2 job timeouts, 1 MMU fault, 1 slow, finished',
          v is not None and v.how == 'finish' and v.n_recoveries() == 2 and v.n_timeouts() == 2 and
          v.n_mmu() == 1 and v.n_slow() == 1 and v.hung_job() is None)
    check('recovery: board time 31s from stamps', any(l.startswith('recovery start 10:00:05') and
                                                       'board 31s' in l for l in got))
    w = by.get('watchdog')
    check('watchdog: no finish, watchdog reset, 1 reset, last case named',
          w is not None and w.how == 'watchdog reset' and w.resets == 1 and
          w.last_case == 'dEQP-GLES3.functional.texture.filtering.2d.formats.rgba8_linear')
    check('watchdog: a NO FINISH line', any(l.startswith('!! NO FINISH watchdog (watchdog reset)') for l in got))
    f = by.get('fatal')
    check('fatal: no finish, fatal then reset',
          f is not None and f.how == 'fatal, then reset' and f.faults == 1)
    check('two unfinished', sorted(r.label for r in unfinished) == ['fatal', 'watchdog'])
    check('no absolute path printed', not any(os.path.dirname(os.path.abspath(__file__)) in l for l in got))
    with open(os.path.join(FIXTURES, 'expected.txt'), encoding='utf-8') as fh:
        want = fh.read().splitlines()
    if got != want:
        fails.append('output differs from expected.txt')
        import difflib
        for d in difflib.unified_diff(want, got, 'expected.txt', 'scan', lineterm='', n=1):
            print(d, file=sys.stderr)
    # a stamped HH:MM:SS wrap at midnight must not make a negative time
    wl = read_lines([b"23:59:50 deqp: case filter 'x'\n",
                     b'00:00:20 deqp: summary: 1 cases begun, 1 with a result, 0 without\n'])
    rr, _, _ = scan('wrap', wl, [])
    check('midnight wrap: 30s', rr and rr[0].t_end - rr[0].t0 == 30)
    # the watchdog fixture cut before the firmware's second boot: the console
    # stops with the run open
    wd = read_log(os.path.join(FIXTURES, 'watchdog.console.txt'))
    cut = [i for i, (_, s) in enumerate(wd) if FW_BOOT.search(s)][1]
    rr, _, _ = scan('cut', wd[:cut], [])
    check('a console that stops open: hang', rr and rr[0].how == 'hang' and rr[0].resets == 0)
    # a failed reset, then the record's "hung: job N (the GPU reset failed)":
    # one hung job, one timeout
    hl = read_lines([b"deqp: case filter 'x'\n",
                     b'osal(v3d): job 5 render did not end (-110, after 4000 ms, 4000 ms of it in this wait)\n',
                     b'osal(v3d): job 5: the GPU reset failed (-5): the GPU is taken as hung; no later job is '
                     b'submitted\n',
                     b"v3d_deqp INCOMPLETE (the photo deadline came first): build 0ff58329, filter 'x'\n",
                     b'  jobs: slow in 0 cases; the first hung or timed-out job in #0\n',
                     b'slow jobs: 0 (past 1000 ms, ended; the slowest 0 ms); MMU faults: 0; recoveries: 0; '
                     b'hung: job 5 (the GPU reset failed), 3 later jobs not submitted, 12 KiB not given back\n',
                     b'driver: osal(v3d): job 5: the GPU reset failed (-5): the GPU is taken as hung\n',
                     b'v3d_deqp: build 0ff58329 INCOMPLETE (the photo deadline came first): 1 cases, P 0 F 0\n'])
    rr, _, _ = scan('hung', hl, [])
    check('a failed reset: hung job 5, 1 timeout, photo deadline',
          rr and rr[0].hung_job() == 5 and rr[0].n_timeouts() == 1 and rr[0].n_recoveries() == 0 and
          rr[0].how == 'photo deadline')
    for f_ in fails:
        print('self-test: FAIL %s' % f_, file=sys.stderr)
    if not fails:
        print('self-test: ok')
    return 1 if fails else 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('log', nargs='*')
    ap.add_argument('-q', '--quiet', action='store_true', help='only the per-image table')
    ap.add_argument('--self-test', action='store_true', help='scan the fixtures and compare')
    ap.add_argument('--write-expected', action='store_true', help=argparse.SUPPRESS)
    a = ap.parse_args(argv)
    if a.self_test:
        return self_test()
    if a.write_expected:
        names = sorted(n for n in os.listdir(FIXTURES) if n.endswith('.console.txt'))
        got = []
        report([('%s/%s' % (SHOWN, n), read_log(os.path.join(FIXTURES, n))) for n in names], False, got.append)
        with open(os.path.join(FIXTURES, 'expected.txt'), 'w', encoding='utf-8') as fh:
            fh.write('\n'.join(got) + '\n')
        return 0
    if not a.log:
        ap.print_usage(sys.stderr)
        return 2
    pl = []
    for p in a.log:
        try:
            for path in inputs(p):
                pl.append((path, read_log(path)))
        except OSError as e:
            print('pi4-console-scan: %s' % e, file=sys.stderr)
            return 2
    _, unfinished = report(pl, a.quiet)
    return 1 if unfinished else 0


if __name__ == '__main__':
    sys.exit(main())
