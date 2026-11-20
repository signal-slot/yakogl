#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The extract stage of tools/cts/deqp-es2-programs.sh.

usage: tools/cts/deqp-es2-programs.py OUT

Reads OUT/run (a tools/cts/host-deqp-run.sh run directory: all.txt and the
c_NNN.qpa / c_NNN.r<k>.qpa logs, written with --deqp-log-shader-sources=enable)
and writes, deterministically (no time, no path, no order of the file system):

  OUT/index.tsv     a header, then one row per program deduplicated by the
                    exact (vertex, fragment) text, sorted by id:
                    id, first case (all.txt order), cases, outcome, refusal
                    outcome is compiled or refused (the 4.2 library's: a
                    shader's CompileStatus or the program's LinkStatus not OK
                    in any case using it); refusal is the first line of the
                    first refusing log (vertex, fragment, then link), empty
                    when compiled
  OUT/programs/<id>.vert, <id>.frag   the sources, verbatim
  OUT/cases-without-program.txt   case <TAB> not-run | no-program, all.txt
                    order: the cases that began no process or logged no
                    program with both a vertex and a fragment source

id is the first 16 hex digits of sha256(vs + '\\0' + fs) over the UTF-8 text.
A program logged with only one stage (shader_api's lone compiles) is not a
program here; a case logging nothing else is no-program.
"""
import hashlib
import html
import os
import re
import sys

# dEQP logs a program in two forms (as tools/mali/deqp-host-measure.sh reads
# them). glu::ShaderProgram (shaders.*, most groups): <ShaderProgram
# LinkStatus=..> with the link log as its first <InfoLog>, then
# <VertexShader CompileStatus=..> <ShaderSource> <InfoLog> per shader. The
# attribute_location, shader_api and a few other groups log each shader as
# <Section Name="Vertex Shader Info"> with a <KernelSource> and an inner
# <Section Name="Info Log"> <Text>, and the link as <Section Name="Program Link
# Info"> after them, each closing with a "Compilation result: Ok|Failed" or
# "Link result: Ok|Fail" message (glsAttributeLocationTests.cpp logShaders,
# logProgram).
RE_UNIT = re.compile(
    r'<ShaderProgram\b(?P<pattr>[^>]*)>\s*(?:<InfoLog>(?P<plog>.*?)</InfoLog>)?(?P<p>)'
    r'|<(?P<a>Vertex|Fragment)Shader\b(?P<aattr>[^>]*)>(?P<ab>.*?)</(?P=a)Shader>'
    r'|(?P<e></ShaderProgram>)'
    r'|<Section Name="(?P<b>Vertex|Fragment) Shader Info"[^>]*>\s*<KernelSource>(?P<bsrc>.*?)</KernelSource>'
    r'(?P<brest>.*?)<Text>Compilation result: (?P<bres>\w+)</Text>'
    r'|<Section Name="Program Link Info"[^>]*>(?P<drest>.*?)<Text>Link result: (?P<dres>\w+)</Text>(?P<d>)',
    re.S)
RE_SOURCE = re.compile(r'<ShaderSource>(.*?)</ShaderSource>', re.S)
RE_INFO = re.compile(r'<InfoLog>(.*?)</InfoLog>', re.S)
RE_LINK = re.compile(r'LinkStatus="([^"]*)"')
RE_COMPILE = re.compile(r'CompileStatus="([^"]*)"')
RE_CASE = re.compile(r'^#beginTestCaseResult (\S+)', re.M)
RE_QPA = re.compile(r'^c_(\d+)(?:\.r(\d+))?\.qpa$')
RE_TEXT = re.compile(r'<Text>(.*?)</Text>', re.S)


def first_line(text):
    for line in text.splitlines():
        line = ' '.join(line.split())
        if line:
            return line
    return ''


def parse_case(block):
    """[(vs, fs, refusal or None)] of one case, in order of appearance.
    refusal is None when the program compiled and linked, else its first
    refusal line."""
    programs = []
    cur = {}

    def reset():
        cur.clear()
        cur.update(vs='', fs='', vlog='', flog='', link='', vok=True, fok=True, lok=True)

    def flush():
        if cur['vs'] or cur['fs'] or cur['link']:
            refusal = None
            for ok, log, what in ((cur['vok'], cur['vlog'], 'vertex shader'),
                                  (cur['fok'], cur['flog'], 'fragment shader'),
                                  (cur['lok'], cur['link'], 'link')):
                if ok is False:
                    refusal = first_line(log) or '%s failed, no info log' % what
                    break
            programs.append((cur['vs'], cur['fs'], refusal))
        reset()

    reset()
    for u in RE_UNIT.finditer(block):
        if u.group('p') is not None:
            flush()
            cur['link'] = html.unescape(u.group('plog') or '')
            m = RE_LINK.search(u.group('pattr'))
            cur['lok'] = (m.group(1) == 'OK') if m else None
        elif u.group('a'):
            st = 'vs' if u.group('a') == 'Vertex' else 'fs'
            if cur[st]:
                flush()
            ms = RE_SOURCE.search(u.group('ab'))
            ml = RE_INFO.search(u.group('ab'))
            cur[st] = html.unescape(ms.group(1)) if ms else ''
            cur['vlog' if st == 'vs' else 'flog'] = html.unescape(ml.group(1)) if ml else ''
            m = RE_COMPILE.search(u.group('aattr'))
            cur['vok' if st == 'vs' else 'fok'] = (m.group(1) == 'OK') if m else None
        elif u.group('e'):
            flush()
        elif u.group('b'):
            st = 'vs' if u.group('b') == 'Vertex' else 'fs'
            if cur[st]:
                flush()
            cur[st] = html.unescape(u.group('bsrc'))
            m = RE_TEXT.search(u.group('brest'))
            cur['vlog' if st == 'vs' else 'flog'] = html.unescape(m.group(1)) if m else ''
            cur['vok' if st == 'vs' else 'fok'] = u.group('bres') == 'Ok'
        else:
            m = RE_TEXT.search(u.group('drest'))
            cur['link'] = html.unescape(m.group(1)) if m else ''
            cur['lok'] = u.group('dres') == 'Ok'
            flush()
    flush()
    return programs


def main(argv):
    if len(argv) != 2:
        sys.exit(__doc__)
    out = argv[1]
    run = os.path.join(out, 'run')
    with open(os.path.join(run, 'all.txt')) as f:
        order = [line.strip() for line in f if line.strip()]

    qpas = []
    for name in os.listdir(run):
        m = RE_QPA.match(name)
        if m:
            qpas.append((int(m.group(1)), int(m.group(2) or -1), name))
    cases = {}
    for _, _, name in sorted(qpas):
        with open(os.path.join(run, name), encoding='utf-8', errors='replace') as f:
            parts = RE_CASE.split(f.read())
        for i in range(1, len(parts), 2):
            # a case begins in one process only (run_chunk resumes after it)
            cases.setdefault(parts[i], parse_case(parts[i + 1]))

    progs = {}       # id -> dict
    without = []
    for case in order:
        if case not in cases:
            without.append((case, 'not-run'))
            continue
        seen = False
        for vs, fs, refusal in cases[case]:
            if not vs or not fs:
                continue
            seen = True
            pid = hashlib.sha256((vs + '\0' + fs).encode('utf-8')).hexdigest()[:16]
            p = progs.get(pid)
            if p is None:
                p = progs[pid] = {'vs': vs, 'fs': fs, 'case': case, 'cases': set(), 'refusal': None}
            elif (p['vs'], p['fs']) != (vs, fs):
                sys.exit('deqp-es2-programs.py: id %s names two programs (%s, %s)' % (pid, p['case'], case))
            p['cases'].add(case)
            if refusal is not None and p['refusal'] is None:
                p['refusal'] = refusal
        if not seen:
            without.append((case, 'no-program'))

    pdir = os.path.join(out, 'programs')
    os.makedirs(pdir, exist_ok=True)
    for name in os.listdir(pdir):
        if name.endswith(('.vert', '.frag')) and name[:-5] not in progs:
            os.remove(os.path.join(pdir, name))
    rows = ['id\tcase\tcases\toutcome\trefusal\n']
    for pid in sorted(progs):
        p = progs[pid]
        for ext, src in (('.vert', p['vs']), ('.frag', p['fs'])):
            with open(os.path.join(pdir, pid + ext), 'w', encoding='utf-8', newline='') as f:
                f.write(src)
        rows.append('%s\t%s\t%d\t%s\t%s\n' % (pid, p['case'], len(p['cases']),
                                              'refused' if p['refusal'] is not None else 'compiled',
                                              p['refusal'] or ''))
    with open(os.path.join(out, 'index.tsv'), 'w', encoding='utf-8', newline='') as f:
        f.writelines(rows)
    with open(os.path.join(out, 'cases-without-program.txt'), 'w', encoding='utf-8', newline='') as f:
        f.writelines('%s\t%s\n' % w for w in without)

    refused = sum(1 for p in progs.values() if p['refusal'] is not None)
    covered = len(order) - len(without)
    notrun = sum(1 for w in without if w[1] == 'not-run')
    print('deqp-es2-programs: %d unique programs: %d compiled, %d refused at 4.2 (HOST-ONLY)'
          % (len(progs), len(progs) - refused, refused))
    print('deqp-es2-programs: %d of %d cases logged a program; %d not run, %d without a program'
          % (covered, len(order), notrun, len(without) - notrun))


if __name__ == '__main__':
    main(sys.argv)
