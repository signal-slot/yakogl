#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# dEQP-GLES2 on the NanoPC-T6 (Mali-G610 over panthor, the board's own
# Armbian), one chunk per row of a tree table, into one slice directory.
#
#   BOARD_HOST=root@<t6> tools/cts/mali-deqp-run.sh --build DIR --out SLICEDIR \
#       --rows A-B [--tree tools/cts/mali-gles2-tree.txt] [--session S1..S4|smoke] [--dry-run]
#   tools/cts/mali-deqp-run.sh --dry-run --rows A-B [--tree T] [--session S]
#
# --dry-run prints the plan and touches nothing: no ssh or scp, no board lock,
# no BOARD_HOST needed. Only in a dry run are --build and --out optional (a
# real run needs both, exit 1 without). Without --build the plan says "build:
# not given" and the commit-on-main, library and size checks are "skipped (no
# build)"; without --out it names the slice directory it would use,
# build/mali-deqp-run/<session>-rows<A>-<B> (gitignored), and creates nothing.
# The default tree is tools/cts/mali-gles2-tree.txt, or $MALI_DEQP_TREE; a
# tree that does not exist is said so and exits 1, dry run or not.
#
# BOARD_HOST comes from the lease (target:nanopc-t6), never from a file;
# BOARD_SSH_OPTS (optional) is passed to ssh and scp, as run-on-board.sh does.
# Spell BOARD_HOST as run-on-board.sh and the kexec recipe do: the board lock
# (driver/tools/kexec-zephyr/board_lock.sh) is one file per BOARD_HOST string.
#
# The build directory is mali-deqp-build.sh's: deqp-gles2, data/ (the archive
# directory, holding gles2/), build.txt with a commit= line, and lib/ with
# libGLESv2.so.2 and libEGL.so.1. A real run refuses unless that commit is on
# origin/main (exit 6); --dry-run says whether it is and goes on.
#
# The run:
#   - takes the board lock (LOCK_WAIT 0: refuse, never queue; HOLD_MAX 21600 s,
#     a session of S1's size fits well inside) and refuses while a process holds
#     /dev/dri/renderD128 (exit 3), as run-on-board.sh does;
#   - stages the build under /root/yakogl-mali/<sha8>/ only (/tmp is tmpfs on
#     the board) after checking the free space there, and deletes it at the end;
#   - records the board's boot_id and uname -r;
#   - for each tree row A..B runs one chunk: one deqp-gles2 process over the
#     row's mustpass cases, LD_LIBRARY_PATH=<stage>/lib, under setsid and
#     timeout (CHUNK_LIMIT seconds, default 600 + 3 s a case), between
#     /dev/kmsg markers "yakogl-run-start/end mali-deqp <label> <stamp>" with a
#     dmesg -w capture around them. A process that stops before the row's end
#     (a crash, the timeout) is followed by a fresh one over the cases no
#     process began (dEQP's --deqp-caselist-file restart, as cts_resume.py and
#     host-deqp-run.sh do); a process that begins none of them loses the first
#     one (Lost chunk-crash or chunk-hang) and the rest go on. At most
#     MAX_ATTEMPTS (100) processes a chunk; what is left then is Lost not-run.
#   - after each chunk: does the board answer ssh (ssh_after), is boot_id the
#     same, how many kernel lines came between the markers and how many of
#     them are panthor job-timeout or reset lines. Every process's stderr must
#     have the line "yakogl-mali: loaded": the board's Mesa provides
#     libGLESv2.so.2 too, so a chunk without it is lost (wrong-lib) and the
#     run stops (exit 9). The last "yakogl-mali-exit:" line is kept.
#   - a board that stops answering ssh, or comes back with another boot_id,
#     ends the run: the rest is Lost board-lost and the board needs a person
#     (exit 8; nothing is retried). A deqp-gles2 that survives SIGKILL after a
#     hang (a wedged ioctl) ends it too (exit 10): the rest is Lost not-run.
#
# The slice (the format mali-deqp-tally.py reads), everything redacted (no
# host names, addresses, user names or serials):
#   meta.txt     commit, session, rows, tree, boot_id_before, boot_id_after,
#                kernel, lib, started, ended (key=value)
#   chunks.tsv   row label cases status seconds kmsg resets ssh_after
#                boot_same exit_line; status done|crash|hang|fault|lost,
#                "-" where a value is unknown (a row never run, a lost capture)
#   results.tsv  case status detail: dEQP's status and its Result text, or
#                Lost with the reason chunk-crash|chunk-hang|board-lost|
#                not-run|wrong-lib as the detail
#   kmsg/<label>.txt  the kernel lines between the chunk's markers
# The raw logs (every process's .qpa, stdout, stderr and the whole dmesg
# capture) go to RAW_DIR (default build/mali-deqp-run/<slice name>/, ignored by
# git); they are not redacted and are never committed.
#
# The runner never touches /boot, U-Boot, modules, Tailscale or sshd: it runs
# dmesg, fuser, df, uname, mkdir/rm under its stage, echo into /dev/kmsg, and
# deqp-gles2. Exit 0: every chunk ran (whatever dEQP said); 1 usage or a
# refusal, 3 render node busy, 4 the board or its tools, 6 not origin/main,
# 7 the boot_id changed, 8 ssh lost, 9 wrong library, 10 wedged process.
#
# Environment for the self-test (tools/cts/mali-deqp-selftest.sh) and odd
# hosts: MALI_DEQP_MAIN_REPO / MALI_DEQP_MAIN_REF (the repository and ref the
# commit must be on; default this checkout and origin/main), MALI_DEQP_MUSTPASS
# (default tools/cts/gles2-mustpass.txt), MALI_DEQP_TREE (the default tree
# table when --tree is not given), KMSG_SETTLE_S (1), STAGE_MARGIN_KB
# (free space wanted beyond the build's size, 1 GiB), DEQP_SURFACE (64).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
me=mali-deqp-run

usage() {
  sed -n '7,9p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 1
}

build= out= rows= tree=${MALI_DEQP_TREE:-$HERE/mali-gles2-tree.txt} session=adhoc dry=0
while [ $# -gt 0 ]; do
  case $1 in
    --build) build=${2:?}; shift 2 ;;
    --out) out=${2:?}; shift 2 ;;
    --rows) rows=${2:?}; shift 2 ;;
    --tree) tree=${2:?}; shift 2 ;;
    --session) session=${2:?}; shift 2 ;;
    --dry-run) dry=1; shift ;;
    -h | --help) usage ;;
    *) echo "$me: unknown argument '$1'" >&2; usage ;;
  esac
done
[ -n "$rows" ] || usage
[ "$dry" = 1 ] || { [ -n "$build" ] && [ -n "$out" ] || usage; }
[[ $rows =~ ^([0-9]+)-([0-9]+)$ ]] || { echo "$me: --rows wants A-B, not '$rows'" >&2; exit 1; }
row_a=${BASH_REMATCH[1]} row_b=${BASH_REMATCH[2]}
case $session in
  S1 | S2 | S3 | S4 | smoke | adhoc) ;;
  *) echo "$me: --session is S1..S4 or smoke, not '$session'" >&2; exit 1 ;;
esac
[ -f "$tree" ] || { echo "$me: no tree table $tree (the default is tools/cts/mali-gles2-tree.txt; \$MALI_DEQP_TREE or --tree name another)" >&2; exit 1; }
mustpass=${MALI_DEQP_MUSTPASS:-$HERE/gles2-mustpass.txt}
[ -f "$mustpass" ] || { echo "$me: no mustpass list $mustpass" >&2; exit 1; }

# The build directory (a dry run may go without one).
main_ref=${MALI_DEQP_MAIN_REF:-origin/main}
if [ -z "$build" ]; then
  commit='not given' on_main='skipped (no build)' lib_id='skipped (no build)' sha8='<sha8>'
  stage=/root/yakogl-mali/$sha8
else
  build=$(cd "$build" 2>/dev/null && pwd) || { echo "$me: no build directory" >&2; exit 1; }
  for f in deqp-gles2 build.txt lib/libGLESv2.so.2 lib/libEGL.so.1; do
    [ -e "$build/$f" ] || { echo "$me: the build directory lacks $f (mali-deqp-build.sh makes it)" >&2; exit 1; }
  done
  [ -d "$build/data" ] || { echo "$me: the build directory lacks data/" >&2; exit 1; }
  commit=$(sed -n 's/^commit=//p' "$build/build.txt" | head -1)
  [[ $commit =~ ^[0-9a-f]{7,40}$ ]] || { echo "$me: build.txt has no commit=<hex> line" >&2; exit 1; }
  main_repo=${MALI_DEQP_MAIN_REPO:-$REPO}
  full=$(git -C "$main_repo" rev-parse --verify -q "$commit^{commit}" 2>/dev/null) || full=
  on_main=no
  if [ -n "$full" ] && git -C "$main_repo" merge-base --is-ancestor "$full" "$main_ref" 2>/dev/null; then
    on_main=yes
    commit=$full
  fi
  sha8=${commit:0:8}
  stage=/root/yakogl-mali/$sha8
  lib_id=$(for l in libGLESv2.so.2 libEGL.so.1; do
    printf '%s:%s\n' "$l" "$(sha256sum < "$build/lib/$l" | cut -c1-12)"; done | paste -sd,)
  lib_id=$(sed -n 's/^lib=//p' "$build/build.txt" | head -1 | grep . || echo "$lib_id")
fi
tree_rel=$(realpath --relative-to="$REPO" "$tree")

# The rows, their labels and case lists (the table's own reader,
# check-gles3-tree.py, so the runner and the check agree on what a row is).
PY=$(cat <<'PYEOF'
import glob, html, importlib.util, os, re, sys

HERE = os.environ['MALI_DEQP_HERE']


def tree_mod():
    spec = importlib.util.spec_from_file_location('cgt', os.path.join(HERE, 'check-gles3-tree.py'))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def plan(tree, mustpass, a, b, casedir):
    mod = tree_mod()
    rows = mod.read_table(open(tree).read())
    a, b = int(a), int(b)
    if not 1 <= a <= b <= len(rows):
        sys.exit('mali-deqp-run: --rows %d-%d is outside the table (%d rows)' % (a, b, len(rows)))
    names = [l.strip() for l in open(mustpass) if l.strip() and not l.startswith('#')]
    blob = '\n'.join(names)
    os.makedirs(casedir, exist_ok=True)
    for i in range(a, b + 1):
        lab, _, filt = rows[i - 1]
        if not mod.LABEL_OK.match(lab):
            sys.exit("mali-deqp-run: row %d's label '%s' is not [A-Za-z0-9]+" % (i, lab))
        cases = [m.group(0) for m in mod.row_regex(filt).finditer(blob)]
        with open(os.path.join(casedir, lab + '.txt'), 'w') as f:
            f.writelines(c + '\n' for c in cases)
        print('%d\t%s\t%d' % (i, lab, len(cases)))


def qpa_cases(path):
    """[(case, status, detail)] of one .qpa; status None for a case begun and
    never closed (the process stopped inside it)."""
    out, cur, buf = [], None, []
    try:
        lines = open(path, encoding='utf-8', errors='replace').read().splitlines()
    except OSError:
        return out
    for l in lines:
        if l.startswith('#beginTestCaseResult'):
            if cur is not None:
                out.append((cur, None, ''))
            parts = l.split()
            cur, buf = (parts[1] if len(parts) > 1 else '?'), []
        elif l.startswith('#endTestCaseResult') and cur is not None:
            m = re.search(r'<Result StatusCode="([^"]*)"[^>]*>(.*?)</Result>', '\n'.join(buf), re.S)
            out.append((cur, m.group(1), html.unescape(m.group(2))) if m else (cur, 'Unknown', 'no Result element'))
            cur = None
        elif l.startswith('#terminateTestCaseResult') and cur is not None:
            st = (l.split() + ['Terminated'])[1]
            out.append((cur, st, 'terminated: ' + st))
            cur = None
        elif cur is not None:
            buf.append(l)
    if cur is not None:
        out.append((cur, None, ''))
    return out


def rest(caselist, *qpas):
    begun = set()
    for q in qpas:
        begun |= {c for c, _, _ in qpa_cases(q)}
    for l in open(caselist):
        c = l.strip()
        if c and c not in begun:
            print(c)


def kmsg(log, start, end, outfile):
    """Lines between the markers into outfile; prints '<lines> <resets> <faults>',
    or '- - -' when the capture lacks a marker."""
    try:
        lines = open(log, encoding='utf-8', errors='replace').read().splitlines()
    except OSError:
        lines = []
    si = next((i for i, l in enumerate(lines) if start in l), None)
    ei = next((i for i, l in enumerate(lines) if end in l), None)
    if si is None or ei is None or ei < si:
        open(outfile, 'w').write('# kernel capture unavailable: no run markers\n')
        print('- - -')
        return
    during = [l for l in lines[si + 1:ei] if l.strip()]
    open(outfile, 'w').write(''.join(l + '\n' for l in during))
    resets = sum(1 for l in during if 'panthor' in l and re.search(r'job timeout|reset', l, re.I))
    faults = sum(1 for l in during if 'panthor' in l and re.search(r'fault', l, re.I))
    print('%d %d %d' % (len(during), resets, faults))


IPV4 = re.compile(r'(?<![\d.])(?:\d{1,3}\.){3}\d{1,3}(?![\d.])')
IPV6 = re.compile(r'\b(?:[0-9A-Fa-f]{1,4}:){7}[0-9A-Fa-f]{1,4}\b'
                  r'|\b[0-9A-Fa-f]{1,4}(?::[0-9A-Fa-f]{1,4})*::(?:[0-9A-Fa-f]{1,4}(?::[0-9A-Fa-f]{1,4})*)?\b')
MAC = re.compile(r'\b(?:[0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}\b')
SERIAL = re.compile(r'(serial(?:[ _-]?(?:no|number))?\s*[:=#]?\s*)[0-9A-Za-z-]{4,}', re.I)
HOME = re.compile(r'/(home|Users)/[^/\s]+')


def redactor():
    """Every word the slice must not keep: REDACT_WORDS (the runner's: the
    board's host and login, this host's names and user) and the values of
    YAKOGL_REDACTION_PRIVATE / _FILE (the release scan's private list)."""
    words = [w for w in os.environ.get('REDACT_WORDS', '').split('\n') if w]
    priv = os.environ.get('YAKOGL_REDACTION_PRIVATE', '').replace(';', '\n')
    pf = os.environ.get('YAKOGL_REDACTION_PRIVATE_FILE')
    if pf and os.path.isfile(pf):
        priv += '\n' + open(pf).read()
    for l in priv.splitlines():
        l = l.split('#', 1)[0]
        if ':' in l:
            v = l.split(':', 1)[1].strip().strip('"\'')
            if v:
                words.append(v)
    words = sorted({w for w in words if w and w != 'root'}, key=len, reverse=True)
    pats = [re.compile(r'(?<![\w.-])' + re.escape(w) + r'(?![\w-])', re.I) for w in words]

    def red(s):
        for p in pats:
            s = p.sub('<host>', s)
        s = MAC.sub('<mac>', s)
        s = IPV6.sub('<addr>', s)
        s = IPV4.sub('<addr>', s)
        s = SERIAL.sub(lambda m: m.group(1) + '<serial>', s)
        s = HOME.sub(r'/\1/<user>', s)
        return s
    return red


def field(s, n=240):
    s = re.sub(r'\s+', ' ', s).strip()
    return (s[:n] if s else '-')


def finalize(raw, out):
    red = redactor()
    plan_rows = [l.rstrip('\n').split('\t') for l in open(os.path.join(raw, 'plan.tsv'))]
    chunks = {}
    if os.path.exists(os.path.join(raw, 'chunks.tsv')):
        for l in open(os.path.join(raw, 'chunks.tsv')):
            p = l.rstrip('\n').split('\t')
            chunks[p[1]] = p
    attempts = {}
    if os.path.exists(os.path.join(raw, 'attempts.tsv')):
        for l in open(os.path.join(raw, 'attempts.tsv')):
            lab, k, reason = l.rstrip('\n').split('\t')[:3]
            attempts[(lab, k)] = reason
    lost = {}
    if os.path.exists(os.path.join(raw, 'lost.tsv')):
        for l in open(os.path.join(raw, 'lost.tsv')):
            c, r = l.rstrip('\n').split('\t')[:2]
            lost.setdefault(c, r)
    os.makedirs(os.path.join(out, 'kmsg'), exist_ok=True)
    meta = [l.rstrip('\n') for l in open(os.path.join(raw, 'meta.txt'))]
    with open(os.path.join(out, 'meta.txt'), 'w') as f:
        f.writelines(red(l) + '\n' for l in meta)
    res, chk = [], []
    for row, lab, n in plan_rows:
        c = chunks.get(lab)
        if c is None:
            c = [row, lab, n, 'lost', '0', '-', '-', '-', '-', '-', 'not-run']
        wrong = c[10] == 'wrong-lib'
        chk.append('\t'.join(c[:9] + [field(red(c[9]))]))
        kf = os.path.join(raw, 'kmsg', lab + '.txt')
        text = open(kf, encoding='utf-8', errors='replace').read() if os.path.exists(kf) \
            else '# kernel capture unavailable: the chunk did not run\n'
        open(os.path.join(out, 'kmsg', lab + '.txt'), 'w').write(red(text))
        got = {}
        for q in sorted(glob.glob(os.path.join(raw, 'qpa', lab + '.r*.qpa')),
                        key=lambda p: int(re.search(r'\.r(\d+)\.qpa$', p).group(1))):
            k = re.search(r'\.r(\d+)\.qpa$', q).group(1)
            for case, st, det in qpa_cases(q):
                if st is None:
                    got.setdefault(case, ('Lost', attempts.get((lab, k), 'chunk-crash')))
                else:
                    got[case] = (st, det)
        for l in open(os.path.join(raw, 'cases', lab + '.txt')):
            case = l.strip()
            if not case:
                continue
            if wrong:
                st, det = 'Lost', 'wrong-lib'
            elif case in lost:
                st, det = 'Lost', lost[case]
            else:
                st, det = got.get(case, ('Lost', 'not-run'))
            res.append('%s\t%s\t%s' % (case, st, field(red(det))))
    with open(os.path.join(out, 'chunks.tsv'), 'w') as f:
        f.write('row\tlabel\tcases\tstatus\tseconds\tkmsg\tresets\tssh_after\tboot_same\texit_line\n')
        f.writelines(l + '\n' for l in chk)
    with open(os.path.join(out, 'results.tsv'), 'w') as f:
        f.write('case\tstatus\tdetail\n')
        f.writelines(l + '\n' for l in res)
    tally = {}
    for l in res:
        st = l.split('\t')[1]
        tally[st] = tally.get(st, 0) + 1
    print('%d rows, %d cases: %s' % (len(plan_rows), len(res),
          ', '.join('%d %s' % (v, k) for k, v in sorted(tally.items(), key=lambda kv: -kv[1]))))


def lastline(path, prefix):
    try:
        lines = [l for l in open(path, encoding='utf-8', errors='replace').read().splitlines() if prefix in l]
    except OSError:
        lines = []
    print(lines[-1].strip().replace('\t', ' ') if lines else '-')


mode, args = sys.argv[1], sys.argv[2:]
{'plan': plan, 'rest': rest, 'kmsg': kmsg, 'finalize': finalize, 'lastline': lastline}[mode](*args)
PYEOF
)
export MALI_DEQP_HERE=$HERE
py() { python3 -I -c "$PY" "$@"; }

[ -n "$out" ] || out=$REPO/build/mali-deqp-run/$session-rows$row_a-$row_b
name=$(basename "$out")
raw=${RAW_DIR:-$REPO/build/mali-deqp-run/$name}
work=$raw
[ "$dry" = 0 ] || work=$(mktemp -d "${TMPDIR:-/tmp}/mali-deqp-dry-XXXXXX")
rm -rf "$work/cases"
mkdir -p "$work/cases"
py plan "$tree" "$mustpass" "$row_a" "$row_b" "$work/cases" > "$work/plan.tsv"
total=$(awk -F'\t' '{ s += $3 } END { print s + 0 }' "$work/plan.tsv")
nrows=$(wc -l < "$work/plan.tsv")
limit_for() { echo "${CHUNK_LIMIT:-$((600 + 3 * $1))}"; }
surface=${DEQP_SURFACE:-64}
deqp_cmd() { # <list> <qpa> <limit>
  echo "cd $stage && setsid -w timeout -k 10 $3 env LD_LIBRARY_PATH=$stage/lib $stage/deqp-gles2" \
    "--deqp-caselist-file=$stage/$1 --deqp-log-filename=$stage/$2 --deqp-archive-dir=$stage/data" \
    "--deqp-surface-type=pbuffer --deqp-surface-width=$surface --deqp-surface-height=$surface" \
    "--deqp-log-images=disable --deqp-log-shader-sources=enable"
}

if [ "$dry" = 1 ]; then
  echo "$me: dry run: no ssh connection, nothing staged"
  if [ -z "$build" ]; then
    echo "$me: build: not given; commit-on-$main_ref, library and size checks: $on_main"
  else
    echo "$me: build $build, commit $commit, on $main_ref: $on_main$([ "$on_main" = yes ] || echo ' (a real run refuses it)')"
  fi
  echo "$me: library $lib_id"
  echo "$me: session $session, rows $row_a-$row_b of $tree_rel: $nrows row(s), $total case(s)"
  while IFS=$'\t' read -r row lab n; do
    echo "$me:   row $row $lab: $n case(s), limit $(limit_for "$n") s a process"
  done < "$work/plan.tsv"
  echo "$me: stage \$BOARD_HOST:$stage/ (deqp-gles2, data/, lib/), removed at the end"
  echo "$me: slice $out, raw logs $raw"
  echo "$me: per chunk <label>, process <k> (over ssh to \$BOARD_HOST):"
  echo "  nohup dmesg -w > $stage/dmesg-<label>.log 2>&1 < /dev/null &"
  echo "  echo 'yakogl-run-start mali-deqp <label> <stamp>' >> /dev/kmsg"
  echo "  $(deqp_cmd '<label>.r<k>.txt' '<label>.r<k>.qpa' '<limit>') > $stage/<label>.r<k>.out 2> $stage/<label>.r<k>.err"
  echo "  echo 'yakogl-run-end mali-deqp <label> <stamp>' >> /dev/kmsg; kill <dmesg pid>"
  echo "  cat /proc/sys/kernel/random/boot_id"
  rm -rf "$work"
  exit 0
fi

[ "$on_main" = yes ] || {
  echo "$me: build.txt's commit $commit is not on $main_ref (fetch it, or build from an origin/main commit): not running" >&2
  exit 6
}
host=${BOARD_HOST:?set BOARD_HOST=<user@host> (the target:nanopc-t6 lease)}
read -r -a opts <<< "${BOARD_SSH_OPTS-}"
if [ -e "$out" ] && [ -n "$(ls -A "$out" 2>/dev/null)" ]; then
  echo "$me: $out exists and is not empty: not overwriting a slice" >&2
  exit 1
fi

# The names the slice must not keep (redaction, below): the board's host and
# login as BOARD_HOST spells them, this host's names and its user.
# shellcheck disable=SC2086  # REDACT_EXTRA is a list of words
REDACT_WORDS=$(printf '%s\n' "$host" "${host#*@}" "${host%@*}" "$(hostname 2>/dev/null)" \
  "$(hostname -f 2>/dev/null)" "$(id -un)" ${REDACT_EXTRA-} | awk 'length($0) > 0 && !seen[$0]++')
export REDACT_WORDS

LOCK_WAIT=${LOCK_WAIT:-0}
HOLD_MAX=${HOLD_MAX:-21600}
finalized=0 cpid= staged=0 board_ok=1
rm -f "$raw/meta.txt"   # a stale one must not become a slice on an early exit
on_exit() {
  rc=$?
  set +e
  if [ -n "$cpid" ] && [ "$board_ok" = 1 ]; then bssh "${opts[@]}" "$host" "kill $cpid" 2>/dev/null; fi
  if [ "$staged" = 1 ] && [ "$board_ok" = 1 ]; then
    bssh -t 60 "${opts[@]}" "$host" "rm -rf -- $stage" ||
      echo "$me: could not remove $stage on the board" >&2
  fi
  if [ "$finalized" = 0 ] && [ -f "$raw/meta.txt" ]; then
    echo "ended=$(date -u +%FT%TZ)" >> "$raw/meta.txt"
    echo "boot_id_after=unknown" >> "$raw/meta.txt"
    fix_meta
    py finalize "$raw" "$out" > /dev/null &&
      echo "$me: stopped early (exit $rc): slice $out written with what ran" >&2
  fi
}
trap on_exit EXIT
# shellcheck source=../../driver/tools/kexec-zephyr/board_lock.sh
. "$REPO/driver/tools/kexec-zephyr/board_lock.sh"
echo "$me: holding the board lock $BOARD_LOCK (pid $$, HOLD_MAX $HOLD_MAX s)"

rssh() { bssh "${opts[@]}" "$host" "$@"; }
# meta.txt in the order of the format, from raw/meta.kv (last value wins).
fix_meta() {
  local k
  [ -f "$raw/meta.txt" ] || return 0
  cp "$raw/meta.txt" "$raw/meta.kv"
  for k in commit session rows tree boot_id_before boot_id_after kernel lib started ended; do
    printf '%s=%s\n' "$k" "$(sed -n "s/^$k=//p" "$raw/meta.kv" | tail -1)"
  done > "$raw/meta.txt"
}

rm -rf "$raw/qpa" "$raw/kmsg" "$raw/log"
mkdir -p "$raw/qpa" "$raw/kmsg" "$raw/log" "$out"
: > "$raw/chunks.tsv"
: > "$raw/attempts.tsv"
: > "$raw/lost.tsv"
{
  echo "commit=$commit"
  echo "session=$session"
  echo "rows=$row_a-$row_b"
  echo "tree=$tree_rel"
  echo "lib=$lib_id"
  echo "started=$(date -u +%FT%TZ)"
} > "$raw/meta.txt"

rssh 'command -v fuser >/dev/null && command -v dmesg >/dev/null && command -v setsid >/dev/null && command -v timeout >/dev/null' || {
  echo "$me: fuser, dmesg, setsid or timeout is missing on the board (or ssh failed): not starting" >&2
  board_ok=0
  exit 4
}
rc=0
users=$(rssh 'fuser -v /dev/dri/renderD128 2>&1') || rc=$?
case $rc in
  0) echo "$me: /dev/dri/renderD128 is in use, not starting:" >&2; echo "$users" >&2; exit 3 ;;
  1) ;;
  *) echo "$me: fuser failed with $rc, not starting" >&2; exit 4 ;;
esac
boot0=$(rssh 'cat /proc/sys/kernel/random/boot_id') || { echo "$me: cannot read the board's boot_id" >&2; exit 4; }
kernel=$(rssh 'uname -r') || kernel=unknown
echo "boot_id_before=$boot0" >> "$raw/meta.txt"
echo "kernel=$kernel" >> "$raw/meta.txt"
echo "$me: boot_id before $boot0, kernel $kernel"

need_kb=$(( $(du -skL "$build/deqp-gles2" "$build/data" "$build/lib" | awk '{ s += $1 } END { print s }') + ${STAGE_MARGIN_KB:-1048576} ))
rssh "mkdir -p /root/yakogl-mali && rm -rf -- $stage && mkdir -p $stage" || { echo "$me: cannot make $stage" >&2; exit 4; }
staged=1
free_kb=$(rssh "df -Pk $stage" | awk 'NR == 2 { print $4 }')
[ -n "$free_kb" ] && [ "$free_kb" -ge "$need_kb" ] || {
  echo "$me: ${free_kb:-?} KiB free under /root/yakogl-mali, the stage needs $need_kb: not starting" >&2
  exit 4
}
bscp -t 1800 -q -r "${opts[@]}" "$build/deqp-gles2" "$build/data" "$build/lib" "$host:$stage/" ||
  { echo "$me: staging failed" >&2; exit 4; }
echo "$me: staged under $stage ($need_kb KiB with the margin, $free_kb free)"

settle=${KMSG_SETTLE_S:-1}
max_attempts=${MAX_ATTEMPTS:-100}
stop=          # why the run stopped: board-lost | wrong-lib | wedged
exit_rc=0
alive() { # sets board_ok and boot_same; 0 when the board answers with the same boot
  local b
  if b=$(rssh 'cat /proc/sys/kernel/random/boot_id' 2>/dev/null) && [ -n "$b" ]; then
    boot_now=$b
    [ "$b" = "$boot0" ] && boot_same=yes || boot_same=no
  else
    board_ok=0 boot_same=- boot_now=unreachable
  fi
  [ "$board_ok" = 1 ] && [ "$boot_same" = yes ]
}
mark_rest() { # <file of cases> <reason>
  awk -v r="$2" 'length($0) { print $0 "\t" r }' "$1" >> "$raw/lost.tsv"
}

while IFS=$'\t' read -r row lab n; do
  if [ -n "$stop" ]; then
    case $stop in board-lost) mark_rest "$raw/cases/$lab.txt" board-lost ;; *) mark_rest "$raw/cases/$lab.txt" not-run ;; esac
    printf '%s\t%s\t%s\tlost\t0\t-\t-\t%s\t-\t-\tnot-run\n' "$row" "$lab" "$n" \
      "$([ "$stop" = board-lost ] && echo no || echo yes)" >> "$raw/chunks.tsv"
    continue
  fi
  t0=$(date +%s)
  stamp=$(date +%Y%m%d-%H%M%S)-$row
  start="yakogl-run-start mali-deqp $lab $stamp" end="yakogl-run-end mali-deqp $lab $stamp"
  echo "$me: row $row $lab: $n case(s)"
  cpid=$(rssh "nohup dmesg -w > $stage/dmesg-$lab.log 2>&1 < /dev/null & echo \$!") || cpid=
  [ -n "$cpid" ] || { echo "$me: could not start dmesg -w on the board" >&2; exit 4; }
  rssh "echo '$start' >> /dev/kmsg"
  cp "$raw/cases/$lab.txt" "$raw/log/$lab.left"
  k=0 hung=0 crashed=0 why= boot_same=yes
  while [ -s "$raw/log/$lab.left" ]; do
    if [ "$k" -ge "$max_attempts" ]; then
      echo "$me: row $row $lab: $max_attempts processes, the rest is not run" >&2
      mark_rest "$raw/log/$lab.left" not-run
      break
    fi
    left=$(wc -l < "$raw/log/$lab.left")
    lim=$(limit_for "$left")
    bscp -q "${opts[@]}" "$raw/log/$lab.left" "$host:$stage/$lab.r$k.txt" || { board_ok=0; }
    rrc=
    if [ "$board_ok" = 1 ]; then
      o=$(bssh -t $((lim + 60)) "${opts[@]}" "$host" \
        "$(deqp_cmd "$lab.r$k.txt" "$lab.r$k.qpa" "$lim") > $stage/$lab.r$k.out 2> $stage/$lab.r$k.err < /dev/null; echo yakogl-deqp-rc=\$?") || true
      rrc=$(printf '%s\n' "$o" | sed -n 's/^yakogl-deqp-rc=//p' | tail -1)
    fi
    if [ -z "$rrc" ] && ! alive; then
      [ "$board_ok" = 1 ] && echo "$me: row $row $lab: the board came back with another boot_id ($boot0 -> $boot_now)" >&2
      why=board-lost
      printf '%s\t%s\tboard-lost\n' "$lab" "$k" >> "$raw/attempts.tsv"
      break
    fi
    # A process the host-side timeout cut off: the board answers, the process did not.
    [ -n "$rrc" ] || rrc=124
    fetched=1
    for ext in qpa out err; do
      bscp -q "${opts[@]}" "$host:$stage/$lab.r$k.$ext" "$raw/qpa/$lab.r$k.$ext" 2>/dev/null ||
        { : > "$raw/qpa/$lab.r$k.$ext"; fetched=0; }
    done
    mv "$raw/qpa/$lab.r$k.out" "$raw/qpa/$lab.r$k.err" "$raw/log/"
    if [ "$fetched" = 0 ] && ! alive; then
      why=board-lost
      rm -f "$raw/qpa/$lab.r$k.qpa"
      printf '%s\t%s\tboard-lost\n' "$lab" "$k" >> "$raw/attempts.tsv"
      break
    fi
    case $rrc in
      0 | 1) reason=not-run ;;
      124 | 137) reason=chunk-hang hung=1 ;;
      *) reason=chunk-crash crashed=1 ;;
    esac
    printf '%s\t%s\t%s\t%s\n' "$lab" "$k" "$reason" "$rrc" >> "$raw/attempts.tsv"
    echo "$me: row $row $lab: process $k over $left case(s) exited $rrc"
    if ! grep -q 'yakogl-mali: loaded' "$raw/log/$lab.r$k.err"; then
      echo "$me: row $row $lab: process $k's stderr lacks 'yakogl-mali: loaded': the board's own libGLESv2.so.2 ran, not yakogl's; stopping" >&2
      why=wrong-lib
      break
    fi
    if [ "$reason" = chunk-hang ]; then
      wedged=$(rssh "pkill -KILL -f '[/]root/yakogl-mali/$sha8/deqp-gles2'; sleep 1; pgrep -f '[/]root/yakogl-mali/$sha8/deqp-gles2'" || true)
      if [ -n "$wedged" ]; then
        echo "$me: row $row $lab: deqp-gles2 survives SIGKILL on the board (a wedged ioctl?): the board needs a person" >&2
        why=wedged
        mark_rest "$raw/log/$lab.left" not-run
        break
      fi
    fi
    qpas=()
    for ((j = 0; j <= k; j++)); do qpas+=("$raw/qpa/$lab.r$j.qpa"); done
    py rest "$raw/cases/$lab.txt" "${qpas[@]}" > "$raw/log/$lab.rest"
    grep -vxF -f <(cut -f1 "$raw/lost.tsv") "$raw/log/$lab.rest" > "$raw/log/$lab.next" || true
    rest_n=$(wc -l < "$raw/log/$lab.next")
    if [ "$rest_n" -gt 0 ] && [ "$rest_n" -eq "$left" ]; then
      if [ "$reason" = not-run ]; then
        # dEQP ended normally and began none of them: it does not know them.
        mark_rest "$raw/log/$lab.next" not-run
        : > "$raw/log/$lab.next"
      else
        head -1 "$raw/log/$lab.next" | mark_rest /dev/stdin "$reason"
        sed -i 1d "$raw/log/$lab.next"
      fi
    fi
    mv "$raw/log/$lab.next" "$raw/log/$lab.left"
    k=$((k + 1))
  done
  if [ "$why" = board-lost ]; then
    mark_rest "$raw/log/$lab.left" board-lost
  else
    rssh "echo '$end' >> /dev/kmsg; sleep $settle; kill $cpid" || true
  fi
  cpid=
  counts='- - -'
  if [ "$why" != board-lost ] && alive && bscp -q "${opts[@]}" "$host:$stage/dmesg-$lab.log" "$raw/log/dmesg-$lab.log"; then
    counts=$(py kmsg "$raw/log/dmesg-$lab.log" "$start" "$end" "$raw/kmsg/$lab.txt")
  else
    echo '# kernel capture unavailable: the board was lost' > "$raw/kmsg/$lab.txt"
  fi
  [ "$board_ok" = 1 ] && [ "$boot_same" = yes ] || why=board-lost
  read -r kl resets faults <<< "$counts"
  if [ -n "$why" ] && [ "$why" != wedged ]; then status=lost
  elif [ "$hung" = 1 ] || [ "$why" = wedged ]; then status=hang
  elif [ "$crashed" = 1 ]; then status=crash
  elif [ "$resets" != - ] && [ $((resets + faults)) -gt 0 ]; then status=fault
  else status=done
  fi
  exit_line=-
  last=$(ls "$raw/log/$lab".r*.err 2>/dev/null | sort -V | tail -1) || true
  [ -z "$last" ] || exit_line=$(py lastline "$last" 'yakogl-mali-exit:')
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$row" "$lab" "$n" "$status" "$(($(date +%s) - t0))" \
    "$kl" "$resets" "$([ "$board_ok" = 1 ] && echo yes || echo no)" "$boot_same" "$exit_line" "${why:--}" >> "$raw/chunks.tsv"
  echo "$me: row $row $lab: $status, $kl kernel line(s), $resets reset line(s), ssh_after $([ "$board_ok" = 1 ] && echo yes || echo no), boot_same $boot_same"
  case $why in
    board-lost) stop=board-lost
      if [ "$board_ok" = 1 ]; then exit_rc=7; else exit_rc=8; fi
      echo "$me: the board is lost (ssh or boot_id): the rest is Lost board-lost; the board needs a person, not retried" >&2 ;;
    wrong-lib) stop=wrong-lib exit_rc=9 ;;
    wedged) stop=wedged exit_rc=10 ;;
  esac
done < "$raw/plan.tsv"

if [ "$board_ok" = 1 ]; then alive || true; else boot_now=unreachable; fi
echo "boot_id_after=${boot_now:-unknown}" >> "$raw/meta.txt"
echo "ended=$(date -u +%FT%TZ)" >> "$raw/meta.txt"
fix_meta
summary=$(py finalize "$raw" "$out")
finalized=1
echo "$me: boot_id after ${boot_now:-unknown}"
echo "$me: slice $out: $summary (raw logs $raw, never committed)"
exit "$exit_rc"
