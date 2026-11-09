#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# glmark2-es2 on the NanoPC-T6 (Mali-G610 over panthor, the board's own
# Armbian), one process a scene, over ssh, into one run directory.
#
#   BOARD_HOST=<user@t6> tools/cts/mali-glmark2-run.sh [--dry-run] [--dev] \
#       [--scenes <file|list>] [--timeout <s>] <builddir> <outdir>
#   tools/cts/mali-glmark2-run.sh --dry-run [--scenes <file|list>] [--timeout <s>]
#
# The build directory is mali-glmark2-build.sh's: bin/glmark2-es2-mali (an
# 800x600 pbuffer, the picture line on stderr), lib/libGLESv2.so.2 and
# libEGL.so.1 (the panthor-gles pair), share/glmark2/ and BUILD-INFO
# (commit=, dev=, glmark2=, flavour=pbuffer, ...).
#
# The scenes: glmark2's 33 default scenes (upstream src/default-benchmarks.h
# at the pin of mali-glmark2-build.sh, the scenes of glmark2-pictures.txt), in
# that order. --scenes takes a file (one item a line, '#' comments) or a list
# of items separated by blanks; an item is a scene as -b takes it
# ("build:use-vbo=false") or a 1-based index or range into the 33 ("1 22-25").
# A scene is letters, digits and _:=.,;- only.
#
# --dry-run prints the plan and touches nothing: no ssh or scp, no board
# lock, no BOARD_HOST needed, nothing written. Only in a dry run are
# <builddir> and <outdir> optional.
#
# BOARD_HOST comes from the lease (target:nanopc-t6), never from a file;
# BOARD_USER, when BOARD_HOST has no "user@", gives the login. BOARD_SSH_OPTS
# (optional) goes to ssh and scp. The board lock is
# driver/tools/kexec-zephyr/board_lock.sh's, one per BOARD_HOST spelling
# (LOCK_WAIT 0: refuse, never queue), as mali-deqp-run.sh takes it.
#
# The commit: BUILD-INFO's commit= (a git describe) must name a commit on
# origin/main and dev= must not be yes; else the run is refused (exit 3)
# before any ssh. --dev runs such a build anyway, for local tests only.
#
# The run:
#   - refuses while a process holds /dev/dri/renderD128 (exit 4), as
#     mali-deqp-run.sh does, and records the board's boot_id and uname -r;
#   - stages the build under the login's home, <home>/yakogl-mali-glmark2/
#     <sha8>-<stamp>/ (MALI_GLMARK2_STAGE_ROOT names another directory of the
#     board; never /boot or a system directory) after checking the free space,
#     and removes it at the end (and the yakogl-mali-glmark2 directory when it
#     is left empty);
#   - for each scene n runs one process, under setsid and
#     `timeout -k KILL_GRACE_S <timeout>` (default 180 s):
#       LD_LIBRARY_PATH=<stage>/lib <stage>/bin/glmark2-es2-mali --size 800x600 -b '<scene>'
#     and takes the kernel lines the scene added (the ring after its last
#     line before the scene: dmesg only reads, nothing is written to the
#     kernel log), classified by tools/cts/mali-kmsg.py as it is;
#   - a scene that outlives its timeout is a hang: `timeout` sends SIGTERM at
#     the limit and SIGKILL KILL_GRACE_S later; the runner then SIGKILLs the
#     process group over ssh and anything left of the program, and waits up
#     to 5 s for the kernel's teardown. One that survives (a wedged ioctl)
#     ends the run (exit 5, the board needs a person). The default limit,
#     180 s, is far above panthor 6.18's job timeout (5 s per job, then
#     queue_timedout_job logs "job timeout" and schedules a reset), so a
#     kernel-side timeout shows up as a job-timeout scene, not as a hang;
#   - after a scene whose kernel lines show a page fault, CS_FATAL/CS_FAULT, a
#     GPU fault, a timeout or a reset, or that hung or died by a signal, a probe
#     runs before the next scene: PROBE_SCENE (default build:use-vbo=false, an
#     unprivileged glmark2 scene) with a PROBE_TIMEOUT_S (60) bound. It passes
#     unless it hangs, crashes, faults or times out. A probe that fails is
#     asked once more; a second failure, a wedged process, or a board that
#     stops answering ssh ends the run (exit 5): the rest is not-run, the
#     runner prints "needs a person" and a final ssh-reachability line, and
#     nothing is retried. probes.tsv in the run directory lists the probes.
#     Panthor does not reset the device on an unhandled page fault (it marks
#     the address space faulty, kills its jobs and reports the fault), so a
#     faulting scene normally leaves the board usable; the probe proves it;
#   - after each scene: does the board answer ssh with the same boot_id? It
#     is asked again every 5 s for BOARD_BACK_S (120) seconds; a board that
#     does not answer, or comes back with another boot_id, ends the run: the
#     rest is not-run and the board needs a person (exit 5, nothing retried);
#   - every process's stderr must have "yakogl-mali: loaded ... lib=<path>"
#     with <path> in <stage>/lib: the board's glvnd + Mesa has the same two
#     SONAMEs (the "sonama trap"), so a scene without it, or naming another
#     library, ends the run (exit 3, wrong-lib).
#
# A scene's verdict, the first that holds:
#   hang               it outlived the timeout (killed)
#   wrong-lib          no "yakogl-mali: loaded" line, or one naming a library
#                      outside the stage
#   crash              ended by a signal, or with no "yakogl-mali-exit:" line
#   gpu-fault          mali-kmsg.py counts a reset, MMU, firmware, GPU or
#                      other panthor line, or the exit line gpu_faults > 0
#   job-timeout        mali-kmsg.py counts a timeout, or job_timeouts > 0
#   out-of-gpu-memory  an "out of GPU memory" line
#   refused-draw       the exit line's refused > 0, or a "draw refused" or
#                      "was not recorded" line
#   gl-error           a "GL error 0x.. in gl.." line
#   failed-scene       "Set up failed", "Unsupported", no FPS, or exit status
#                      other than 0
#   ok                 none of these
# and board-lost (the board went away or rebooted during it), not-run (the
# run had stopped before it).
#
# The run directory (<outdir>, absent or empty beforehand), everything
# redacted (no host names, addresses, user names or MACs):
#   scenes.tsv     scene commit verdict ms_frame picture kmsg exit_line, one
#                  row a scene in run order (row n is scene n): ms_frame is
#                  glmark2's FrameTime, or 1000 / FPS when the line has none;
#                  picture the text after "glmark2-picture: <info>: "; kmsg
#                  the panthor lines mali-kmsg.py counts; exit_line the last
#                  "yakogl-mali-exit:" line; "-" where there is none
#   pictures.txt   "mali | <scene info string> | <picture>", one line a scene
#                  with a picture line: the line form of glmark2-pictures.txt
#   kmsg/<n>.log   the kernel lines of scene n; kmsg/<n>.class mali-kmsg.py's
#                  line for them
#   log/<n>.out, log/<n>.err  scene n's stdout and stderr
#   meta.txt       commit, dev, glmark2, flavour, scenes, timeout,
#                  boot_id_before, boot_id_after, kernel, started, ended
#   summary.txt    the summary line, as printed:
#     mali-glmark2: <ok>/<n> scenes ok, failed <k> (<verdict> <count>, ...), commit <sha8>
# The raw files (fetched, not redacted) go to RAW_DIR (default
# build/mali-glmark2-run/<outdir's name>/, ignored by git), never committed.
#
# The runner never touches /boot, U-Boot, kernel modules, Tailscale or sshd
# and writes nothing outside its stage: it runs cat, dmesg, fuser, df, uname,
# mkdir/rm/rmdir under the stage, kill/pkill/pgrep of the staged program, and
# the program.
#
# Exit: 0 every scene ran (whatever its verdict); 1 usage, a bad build
# directory or a non-empty <outdir>; 3 refused (commit not on origin/main, a
# dev build, wrong-lib); 4 the board or its tools (render node busy, a tool
# missing, staging failed); 5 the board did not come back, rebooted, or kept
# a process through SIGKILL: it needs a person, nothing is retried.
#
# Environment for the self-test (tools/cts/mali-glmark2-selftest.sh) and odd
# hosts: MALI_GLMARK2_REPO / MALI_GLMARK2_MAIN_REF (the repository and ref the
# commit must be on; default this checkout and origin/main, as
# mali-glmark2-build.sh), MALI_GLMARK2_STAGE_ROOT, BOARD_BACK_S (120),
# KILL_GRACE_S (10), STAGE_MARGIN_KB (free space wanted beyond the build's
# size, 256 MiB), HOLD_MAX (default: the scenes' worst case), RAW_DIR,
# PROBE_SCENE, PROBE_TIMEOUT_S (60), PROBE_PAUSE_S (5, between two probes).
#
# Its gate row (the self-test, in tools/check.sh) is added by a separate item
# of plan #332, after #327's mali-gate-rows.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
me=mali-glmark2-run
PROGRAM=glmark2-es2-mali
SIZE=800x600

usage() {
  sed -n '8,10p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 1
}

# upstream src/default-benchmarks.h, in its order
DEFAULT_SCENES=(
  build:use-vbo=false
  build:use-vbo=true
  texture:texture-filter=nearest
  texture:texture-filter=linear
  texture:texture-filter=mipmap
  shading:shading=gouraud
  shading:shading=blinn-phong-inf
  shading:shading=phong
  shading:shading=cel
  bump:bump-render=high-poly
  bump:bump-render=normals
  bump:bump-render=height
  'effect2d:kernel=0,1,0;1,-4,1;0,1,0;'
  'effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;'
  pulsar:quads=5:texture=false:light=false
  desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true
  desktop:windows=4:effect=shadow
  buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false
  buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false
  buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true
  ideas:speed=duration
  jellyfish
  terrain
  shadow
  refract
  conditionals:vertex-steps=0:fragment-steps=0
  conditionals:vertex-steps=0:fragment-steps=5
  conditionals:vertex-steps=5:fragment-steps=0
  function:fragment-steps=5:fragment-complexity=low
  function:fragment-steps=5:fragment-complexity=medium
  loop:vertex-steps=5:fragment-steps=5:fragment-loop=false
  loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false
  loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true
)

dry=0 dev=0 scenes_arg= limit=180 pos=()
while [ $# -gt 0 ]; do
  case $1 in
    --dry-run) dry=1; shift ;;
    --dev) dev=1; shift ;;
    --scenes) scenes_arg=${2:?--scenes needs a file or a list}; shift 2 ;;
    --timeout) limit=${2:?--timeout needs seconds}; shift 2 ;;
    -h | --help) usage ;;
    --) shift; pos+=("$@"); break ;;
    -*) echo "$me: unknown option '$1'" >&2; usage ;;
    *) pos+=("$1"); shift ;;
  esac
done
[[ $limit =~ ^[1-9][0-9]*$ ]] || { echo "$me: --timeout wants whole seconds, not '$limit'" >&2; exit 1; }
[ "${#pos[@]}" -le 2 ] || usage
build=${pos[0]-} out=${pos[1]-}
if [ "$dry" = 0 ] && { [ -z "$build" ] || [ -z "$out" ]; }; then usage; fi

# ---- the scenes ----------------------------------------------------------------
SCENES=()
add_item() {
  local it=$1 a b i
  if [[ $it =~ ^([0-9]+)(-([0-9]+))?$ ]]; then
    a=${BASH_REMATCH[1]} b=${BASH_REMATCH[3]:-${BASH_REMATCH[1]}}
    if [ "$a" -lt 1 ] || [ "$b" -lt "$a" ] || [ "$b" -gt "${#DEFAULT_SCENES[@]}" ]; then
      echo "$me: --scenes item '$it' is outside 1-${#DEFAULT_SCENES[@]}" >&2
      exit 1
    fi
    for ((i = a; i <= b; i++)); do SCENES+=("${DEFAULT_SCENES[i - 1]}"); done
  elif [[ $it =~ ^[A-Za-z0-9_:=.,\;-]+$ ]]; then
    SCENES+=("$it")
  else
    echo "$me: scene '$it': letters, digits and _:=.,;- only" >&2
    exit 1
  fi
}
if [ -z "$scenes_arg" ]; then
  SCENES=("${DEFAULT_SCENES[@]}")
else
  if [ -f "$scenes_arg" ]; then
    items=$(sed 's/#.*//' "$scenes_arg")
  else
    items=$scenes_arg
  fi
  read -r -d '' -a list <<< "$items" || true # split on blanks, no globbing
  for it in ${list[@]+"${list[@]}"}; do add_item "$it"; done
  [ "${#SCENES[@]}" -gt 0 ] || { echo "$me: --scenes names no scene" >&2; exit 1; }
fi
nscenes=${#SCENES[@]}

# ---- the build -----------------------------------------------------------------
main_ref=${MALI_GLMARK2_MAIN_REF:-origin/main}
main_repo=${MALI_GLMARK2_REPO:-$REPO}
if [ -z "$build" ]; then
  commit='not given' on_main='skipped (no build)' dev_info=- sha8='<sha8>' gm_pin=- flavour=-
else
  build=$(cd "$build" 2>/dev/null && pwd) || { echo "$me: no build directory $build" >&2; exit 1; }
  for f in "bin/$PROGRAM" lib/libGLESv2.so.2 lib/libEGL.so.1 share/glmark2 BUILD-INFO; do
    [ -e "$build/$f" ] || { echo "$me: the build directory lacks $f (mali-glmark2-build.sh makes it)" >&2; exit 1; }
  done
  info() { sed -n "s/^$1=//p" "$build/BUILD-INFO" | head -1; }
  flavour=$(info flavour) gm_pin=$(info glmark2) dev_info=$(info dev)
  [ "$flavour" = pbuffer ] || { echo "$me: BUILD-INFO says flavour=${flavour:-<none>}, not pbuffer" >&2; exit 1; }
  described=$(info commit)
  [ -n "$described" ] || { echo "$me: BUILD-INFO has no commit= line" >&2; exit 1; }
  # git describe --always --abbrev=40 --dirty: <hex> or <tag>-<n>-g<hex>, then -dirty
  hex=${described%-dirty}
  if [[ $hex =~ -g([0-9a-f]{7,40})$ ]]; then hex=${BASH_REMATCH[1]}; fi
  commit=$described on_main=no
  if [[ $hex =~ ^[0-9a-f]{7,40}$ ]]; then
    full=$(git -C "$main_repo" rev-parse --verify -q "$hex^{commit}" 2>/dev/null) || full=
    if [ -n "$full" ]; then
      commit=$full
      if git -C "$main_repo" merge-base --is-ancestor "$full" "$main_ref" 2>/dev/null; then on_main=yes; fi
    fi
  fi
  [ "$described" = "${described%-dirty}" ] || on_main="no (a dirty tree)"
  case $dev_info in yes*) on_main="no (BUILD-INFO says dev=yes)" ;; esac
  sha8=${commit:0:8}
  [[ $sha8 =~ ^[0-9a-f]{8}$ ]] || sha8=unknown0
fi
refusal=
case $on_main in
  yes) ;;
  'skipped (no build)') ;;
  *) refusal="BUILD-INFO's commit $commit is not on $main_ref (on_main: $on_main): build from an origin/main commit, or --dev for a local test" ;;
esac

stamp=$(date -u +%Y%m%d-%H%M%S)
stage_root_given=${MALI_GLMARK2_STAGE_ROOT-}
gm_cmd() { # <stage> <id> <scene> [seconds]: the board's command for one process
  echo "cd $1 && setsid -w sh -c 'echo \$\$ > $1/run/$2.pgid; exec timeout -k ${KILL_GRACE_S:-10} ${4:-$limit}" \
    "env LD_LIBRARY_PATH=$1/lib $1/bin/$PROGRAM --size $SIZE -b \"\$1\"' glmark2 '$3'" \
    "> $1/run/$2.out 2> $1/run/$2.err < /dev/null"
}

if [ "$dry" = 1 ]; then
  echo "$me: dry run: no ssh connection, no board lock, nothing staged or written"
  if [ -z "$build" ]; then
    echo "$me: build: not given; commit-on-$main_ref check: $on_main"
  else
    note=
    if [ -n "$refusal" ]; then
      if [ "$dev" = 1 ]; then note=' (--dev: run anyway, not a record)'; else note=' (a real run refuses it, exit 3)'; fi
    fi
    echo "$me: build $build: commit $commit, on $main_ref: $on_main$note"
    echo "$me: glmark2 $gm_pin, flavour $flavour, dev ${dev_info:-no}"
  fi
  echo "$me: $nscenes scene(s), each its own process, timeout $limit s (then SIGKILL after ${KILL_GRACE_S:-10} s):"
  for ((i = 0; i < nscenes; i++)); do echo "$me:   $((i + 1)) ${SCENES[i]}"; done
  echo "$me: stage \$BOARD_HOST:${stage_root_given:-<home>/yakogl-mali-glmark2}/$sha8-<stamp>/ (bin/, lib/, share/, BUILD-INFO), removed at the end"
  echo "$me: out ${out:-<outdir>} (scenes.tsv, pictures.txt, kmsg/<n>.log, log/, meta.txt, summary.txt), raw files ${RAW_DIR:-build/mali-glmark2-run/<outdir name>}"
  echo "$me: per scene <n> (over ssh to \$BOARD_HOST):"
  echo "  dmesg | tail -n 1"
  echo "  $(gm_cmd '<stage>' '<n>' '<scene>')"
  echo "  dmesg; cat /proc/sys/kernel/random/boot_id   (every 5 s for ${BOARD_BACK_S:-120} s until it answers)"
  echo "  after a fault, hang or crash: the same command for ${PROBE_SCENE:-build:use-vbo=false} (bound ${PROBE_TIMEOUT_S:-60} s), twice at most; then 'needs a person'"
  echo "  on a hang: kill -KILL -- -<pgid>; pkill -KILL -f <stage>/bin/$PROGRAM; pgrep -f <stage>/bin/$PROGRAM"
  exit 0
fi

if [ -n "$refusal" ]; then
  if [ "$dev" = 1 ]; then
    echo "$me: --dev: $refusal; running it as a local test, not a record" >&2
  else
    echo "$me: refused: $refusal" >&2
    exit 3
  fi
fi
host=${BOARD_HOST:?set BOARD_HOST=<user@host> (the target:nanopc-t6 lease)}
[[ $host == *@* ]] || [ -z "${BOARD_USER-}" ] || host=$BOARD_USER@$host
read -r -a opts <<< "${BOARD_SSH_OPTS-}"
if [ -e "$out" ] && [ -n "$(ls -A "$out" 2>/dev/null)" ]; then
  echo "$me: $out exists and is not empty: not overwriting a run" >&2
  exit 1
fi
mkdir -p "$out"
out=$(cd "$out" && pwd)
raw=${RAW_DIR:-$REPO/build/mali-glmark2-run/$(basename "$out")}
rm -rf "$raw/run" "$raw/kmsg" "$raw/pictures.txt" "$raw/meta.txt" "$raw/meta.full"
mkdir -p "$raw/run" "$raw/kmsg" "$out/kmsg" "$out/log"

# The names the run directory must not keep (redaction): the board's host and
# login as BOARD_HOST spells them, this host's names and its user.
# shellcheck disable=SC2086  # REDACT_EXTRA is a list of words
REDACT_WORDS=$(printf '%s\n' "$host" "${host#*@}" "${host%@*}" "$(hostname 2>/dev/null)" \
  "$(hostname -f 2>/dev/null)" "$(id -un)" ${REDACT_EXTRA-} | awk 'length($0) > 0 && !seen[$0]++')
export REDACT_WORDS

PY=$(cat <<'PYEOF'
import json, os, re, subprocess, sys

HERE = os.environ['MALI_GLMARK2_HERE']
IPV4 = re.compile(r'(?<![\d.])(?:\d{1,3}\.){3}\d{1,3}(?![\d.])')
IPV6 = re.compile(r'\b(?:[0-9A-Fa-f]{1,4}:){7}[0-9A-Fa-f]{1,4}\b'
                  r'|\b[0-9A-Fa-f]{1,4}(?::[0-9A-Fa-f]{1,4})*::(?:[0-9A-Fa-f]{1,4}(?::[0-9A-Fa-f]{1,4})*)?\b')
MAC = re.compile(r'\b(?:[0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}\b')
HOME = re.compile(r'/(home|Users)/[^/\s]+')


def redactor():
    words = [w for w in os.environ.get('REDACT_WORDS', '').split('\n') if w and w != 'root']
    words = sorted(set(words), key=len, reverse=True)
    pats = [re.compile(r'(?<![\w.-])' + re.escape(w) + r'(?![\w-])', re.I) for w in words]

    def red(s):
        for p in pats:
            s = p.sub('<host>', s)
        s = MAC.sub('<mac>', s)
        s = IPV6.sub('<addr>', s)
        s = IPV4.sub('<addr>', s)
        return HOME.sub(r'/\1/<user>', s)
    return red


def read(path):
    try:
        return open(path, encoding='utf-8', errors='replace').read()
    except OSError:
        return ''


def redact(src, dst):
    open(dst, 'w').write(redactor()(read(src)))


def window(before, after, dst):
    """The lines of the dmesg dump <after> past the last line <before> held
    (the ring's last line before the scene) into dst."""
    last = read(before).strip('\n').split('\n')[-1].strip()
    lines = read(after).splitlines()
    idx = -1
    if last:
        idx = next((i for i in range(len(lines) - 1, -1, -1) if lines[i].strip() == last), None)
        if idx is None:
            sys.stderr.write('the last kernel line before the scene left the ring: the whole ring kept\n')
            idx = -1
    open(dst, 'w').write(''.join(l + '\n' for l in lines[idx + 1:] if l.strip()))


def exit_fields(line):
    f = {}
    for k in ('draws', 'refused', 'gpu_faults', 'job_timeouts'):
        m = re.search(r'\b' + k + r'=(\d+)', line)
        f[k] = int(m.group(1)) if m else 0
    return f


def judge(rc, elapsed, limit, out, err, klog, libdir, kclass):
    """One scene: prints verdict, ms_frame, picture, kmsg count, exit line,
    info string and the picture line form, tab-separated."""
    rc, elapsed, limit = int(rc), int(elapsed), int(limit)
    text = re.sub(r'[\x00-\x08\x0b-\x1f\x7f]', '', read(out) + '\n' + read(err))
    lines = text.splitlines()
    loaded = [l for l in lines if 'yakogl-mali: loaded' in l]
    lib = 'missing'
    if loaded:
        m = re.search(r'\blib=(\S+)', loaded[-1])
        lib = 'ok' if m and os.path.dirname(m.group(1)) == libdir else 'foreign'
    exits = [l.strip() for l in lines if 'yakogl-mali-exit:' in l]
    exit_line = exits[-1].replace('\t', ' ') if exits else '-'
    ef = exit_fields(exit_line)
    if os.path.exists(klog):
        r = json.loads(subprocess.run([sys.executable, '-I', os.path.join(HERE, 'mali-kmsg.py'), '--json', klog],
                                      capture_output=True, text=True, check=True).stdout)
        line = subprocess.run([sys.executable, '-I', os.path.join(HERE, 'mali-kmsg.py'), klog],
                              capture_output=True, text=True, check=True).stdout.strip()
        open(kclass, 'w').write(line + '\n')
        # cs_faults (CS_FATAL, CS_FAULT, tiler heap) counts as a GPU fault too;
        # .get: a copy of mali-kmsg.py without that key still works
        keys = ('resets', 'timeouts', 'mmu_faults', 'fw_faults', 'gpu_faults', 'cs_faults', 'other_panthor')
        kcount = sum(r.get(k, 0) for k in keys)
        faults = sum(r.get(k, 0) for k in keys if k != 'timeouts')
        timeouts = r['timeouts']
        kstr = str(kcount)
    else:
        open(kclass, 'w').write('kmsg: unavailable\n')
        faults = timeouts = 0
        kstr = '-'
    pic, info = '-', '-'
    for l in lines:
        m = re.search(r'glmark2-picture: (\[[\w-]+\] .*?): ((?:\d+x\d+;|absent).*)$', l)
        if m:
            info, pic = m.group(1), m.group(2).strip()
    res = [l for l in lines if re.search(r'(\[[\w-]+\] .*?): (FPS: |FrameTime: |Set up failed|Unsupported)', l)]
    if info == '-' and res:
        info = re.search(r'(\[[\w-]+\] .*?): ', res[-1]).group(1)
    ms = '-'
    fps_seen = False
    for l in res:
        t = re.search(r'FrameTime: (\d+(?:\.\d+)?) ms', l)
        f = re.search(r'FPS: (\d+)', l)
        if t:
            ms = '%.3f' % float(t.group(1))
        elif f and int(f.group(1)) > 0:
            ms = '%.3f' % (1000.0 / int(f.group(1)))
        fps_seen = fps_seen or bool(f or t)
    setup_failed = any(re.search(r'Set up failed|Unsupported', l) for l in res)
    if rc in (124, 137) and (rc == 124 or elapsed >= limit):
        v = 'hang'
    elif lib != 'ok':
        v = 'wrong-lib'
    elif rc >= 128 or exit_line == '-':
        v = 'crash'
    elif faults or ef['gpu_faults']:
        v = 'gpu-fault'
    elif timeouts or ef['job_timeouts']:
        v = 'job-timeout'
    elif re.search(r'out of GPU memory', text):
        v = 'out-of-gpu-memory'
    elif ef['refused'] or re.search(r'draw refused|was not recorded \(-?\d+\)', text):
        v = 'refused-draw'
    elif re.search(r'GL error 0x[0-9a-fA-F]+ in ', text):
        v = 'gl-error'
    elif setup_failed or not fps_seen or rc != 0:
        v = 'failed-scene'
    else:
        v = 'ok'
    red = redactor()
    print('\t'.join([v, ms, red(pic), kstr, red(exit_line), info, lib]))


mode, args = sys.argv[1], sys.argv[2:]
{'redact': redact, 'window': window, 'judge': judge}[mode](*args)
PYEOF
)
export MALI_GLMARK2_HERE=$HERE
py() { python3 -I -c "$PY" "$@"; }

# ---- the board -----------------------------------------------------------------
LOCK_WAIT=${LOCK_WAIT:-0}
back_s=${BOARD_BACK_S:-120}
probe_s=${PROBE_TIMEOUT_S:-60}
PROBE_SCENE=${PROBE_SCENE:-build:use-vbo=false}
[[ $probe_s =~ ^[1-9][0-9]*$ ]] || { echo "$me: PROBE_TIMEOUT_S wants whole seconds, not '$probe_s'" >&2; exit 1; }
[[ $PROBE_SCENE =~ ^[A-Za-z0-9_:=.,\;-]+$ ]] || { echo "$me: PROBE_SCENE: letters, digits and _:=.,;- only" >&2; exit 1; }
HOLD_MAX=${HOLD_MAX:-$((nscenes * (limit + ${KILL_GRACE_S:-10} + back_s + 120 + 2 * (probe_s + ${KILL_GRACE_S:-10} + back_s + 120)) + 1800))}
staged=0 board_ok=1 stage= stage_root= made_root=0 finished=0
rows=() probes=()
on_exit() {
  set +e
  if [ "$finished" = 0 ] && [ -f "$raw/meta.txt" ]; then
    write_tables
    echo "$me: stopped early: $out holds what ran; $summary" >&2
  fi
  if [ "$staged" = 1 ] && [ "$board_ok" = 1 ]; then
    rssh "pkill -KILL -f '[${stage:0:1}]${stage:1}/bin/$PROGRAM'; rm -rf -- '$stage'" ||
      echo "$me: could not remove $stage on the board" >&2
    [ "$made_root" = 0 ] || rssh "rmdir -- '$stage_root' 2>/dev/null; true"
  elif [ "$staged" = 1 ]; then
    echo "$me: the stage $stage is left on the board (the board does not answer)" >&2
  fi
}
trap on_exit EXIT
# shellcheck source=../../driver/tools/kexec-zephyr/board_lock.sh
. "$REPO/driver/tools/kexec-zephyr/board_lock.sh"
echo "$me: holding the board lock (pid $$, HOLD_MAX $HOLD_MAX s)"
rssh() { bssh "${opts[@]}" "$host" "$@"; }

rssh "for t in cat dmesg fuser df uname setsid timeout pkill pgrep; do command -v \$t > /dev/null || { echo \$t; exit 1; }; done; dmesg > /dev/null" || {
  echo "$me: a tool the run needs is missing on the board, dmesg is not readable for this login, or ssh failed: not starting" >&2
  board_ok=0
  exit 4
}
rc=0
users=$(rssh 'fuser -v /dev/dri/renderD128 2>&1') || rc=$?
case $rc in
  0) echo "$me: /dev/dri/renderD128 is in use, not starting:" >&2; echo "$users" >&2; exit 4 ;;
  1) ;;
  *) echo "$me: fuser failed with $rc, not starting" >&2; exit 4 ;;
esac
boot0=$(rssh 'cat /proc/sys/kernel/random/boot_id') || boot0=
if [ -z "$boot0" ]; then echo "$me: cannot read the board's boot_id" >&2; exit 4; fi
kernel=$(rssh 'uname -r') || kernel=unknown
echo "$me: boot_id before $boot0, kernel $kernel"

if [ -n "$stage_root_given" ]; then
  stage_root=$stage_root_given
else
  stage_root=$(rssh 'cd && pwd -P')/yakogl-mali-glmark2 || stage_root=
fi
[[ $stage_root =~ ^/[A-Za-z0-9._/-]+$ ]] || { echo "$me: the stage directory '$stage_root' is not a plain absolute path" >&2; exit 4; }
case $stage_root/ in
  /boot/* | /bin/* | /sbin/* | /lib/* | /lib64/* | /usr/* | /etc/* | /proc/* | /sys/* | /dev/* | /run/* | /var/lib/* | /opt/*)
    echo "$me: refusing to stage under $stage_root (a system directory)" >&2; exit 4 ;;
esac
rssh "[ -d '$stage_root' ]" || made_root=1
need_kb=$(($(du -skL "$build/bin" "$build/lib" "$build/share" | awk '{ s += $1 } END { print s }') + ${STAGE_MARGIN_KB:-262144}))
stage=$stage_root/$sha8-$stamp
rssh "mkdir -p '$stage/run'" || { echo "$me: cannot make the stage $stage" >&2; exit 4; }
staged=1
stage=$(rssh "cd '$stage' && pwd -P") || stage=
if ! [[ $stage =~ ^/[A-Za-z0-9._/-]+$ ]]; then echo "$me: cannot resolve the stage" >&2; exit 4; fi
free_kb=$(rssh "df -Pk '$stage'" | awk 'NR == 2 { print $4 }')
if [ -z "$free_kb" ] || [ "$free_kb" -lt "$need_kb" ]; then
  echo "$me: ${free_kb:-?} KiB free under $stage_root, the stage needs $need_kb: not starting" >&2
  exit 4
fi
bscp -t 1800 -q -r "${opts[@]}" "$build/bin" "$build/lib" "$build/share" "$build/BUILD-INFO" "$host:$stage/" ||
  { echo "$me: staging failed" >&2; exit 4; }
echo "$me: staged ($need_kb KiB with the margin, $free_kb free)"

dev_meta=no
[ -z "$refusal" ] || dev_meta="yes (--dev; on $main_ref: $on_main)"
{
  echo "commit=$commit"
  echo "dev=$dev_meta"
  echo "glmark2=$gm_pin"
  echo "flavour=$flavour"
  echo "scenes=$nscenes"
  echo "timeout=$limit"
  echo "boot_id_before=$boot0"
  echo "kernel=$kernel"
  echo "started=$(date -u +%FT%TZ)"
} > "$raw/meta.txt"

alive() { # 0 when the board answers with boot0; sets board_ok, boot_now
  local b t0
  t0=$(date +%s)
  while :; do
    if b=$(rssh 'cat /proc/sys/kernel/random/boot_id' 2>/dev/null) && [ -n "$b" ]; then
      boot_now=$b
      [ "$b" = "$boot0" ] && return 0
      return 1
    fi
    if [ $(($(date +%s) - t0)) -ge "$back_s" ]; then
      board_ok=0 boot_now=unreachable
      return 1
    fi
    sleep "${BOARD_POLL_S:-5}"
  done
}

# The run directory's tables from the rows so far (an early stop writes what ran).
write_tables() {
  local j nok breakdown
  {
    cat "$raw/meta.txt"
    echo "boot_id_after=${boot_now:-unknown}"
    echo "ended=$(date -u +%FT%TZ)"
  } > "$raw/meta.full"
  py redact "$raw/meta.full" "$out/meta.txt"
  for ((j = ${#rows[@]}; j < nscenes; j++)); do rows+=("${SCENES[j]}	$commit	not-run	-	-	-	-"); done
  {
    printf 'scene\tcommit\tverdict\tms_frame\tpicture\tkmsg\texit_line\n'
    printf '%s\n' "${rows[@]}"
  } > "$out/scenes.tsv"
  {
    echo "# mali column, commit $commit, tools/cts/mali-glmark2-run.sh"
    [ ! -f "$raw/pictures.txt" ] || cat "$raw/pictures.txt"
  } > "$out/pictures.txt"
  nok=$(awk -F'\t' 'NR > 1 && $3 == "ok"' "$out/scenes.tsv" | wc -l)
  breakdown=$(awk -F'\t' 'NR > 1 && $3 != "ok" { c[$3]++; if (!($3 in o)) o[$3] = NR }
    END { for (v in c) print o[v] "\t" v " " c[v] }' "$out/scenes.tsv" | sort -n | cut -f2 | paste -sd, | sed 's/,/, /g')
  summary="mali-glmark2: $nok/$nscenes scenes ok, failed $((nscenes - nok))${breakdown:+ ($breakdown)}, commit $sha8"
  echo "$summary" > "$out/summary.txt"
  if [ "${#probes[@]}" -gt 0 ]; then
    { printf 'after_scene\tattempt\tresult\tverdict\tkmsg\n'; printf '%s\n' "${probes[@]}"; } > "$out/probes.tsv"
  fi
}

stop= exit_rc=0

# run_one <id> <scene> <seconds>: one glmark2 process on the board, its raw
# files, kernel window and verdict. Sets rrc lost wedged klog verdict ms pic
# kcount exit_line info lib (globals).
run_one() {
  local id=$1 scene=$2 lim=$3 o t0 elapsed ext
  wedged=
  rssh 'dmesg | tail -n 1' > "$raw/kmsg/$id.before" 2>/dev/null || : > "$raw/kmsg/$id.before"
  t0=$(date +%s)
  o=$(bssh -t $((lim + ${KILL_GRACE_S:-10} + 60)) "${opts[@]}" "$host" "$(gm_cmd "$stage" "$id" "$scene" "$lim"); echo yakogl-glmark2-rc=\$?") || true
  elapsed=$(($(date +%s) - t0))
  rrc=$(printf '%s\n' "$o" | sed -n 's/^yakogl-glmark2-rc=//p' | tail -1)
  lost=0
  if ! alive; then
    lost=1
  fi
  if [ "$lost" = 0 ] && { [ -z "$rrc" ] || [ "$rrc" = 124 ] || [ "$rrc" = 137 ]; }; then
    # A hang (timeout sent SIGTERM at the limit, SIGKILL KILL_GRACE_S later), or
    # an ssh cut off by the host-side bound: SIGKILL the process group and what
    # is left of the program, then wait up to 5 s for the kernel to finish the
    # teardown (panthor destroys the process's groups and VM in the file
    # release). A process still there after that is stuck in an ioctl.
    [ -n "$rrc" ] || rrc=124 elapsed=$lim
    wedged=$(rssh "pg=\$(cat '$stage/run/$id.pgid' 2>/dev/null); [ -z \"\$pg\" ] || kill -KILL -- -\$pg 2>/dev/null;
      pkill -KILL -f '[${stage:0:1}]${stage:1}/bin/$PROGRAM'; p=; for k in 1 2 3 4 5; do sleep 1; p=\$(pgrep -f '[${stage:0:1}]${stage:1}/bin/$PROGRAM') || break; done; echo \$p" || true)
    if [ -n "$wedged" ]; then
      echo "$me: $id: $PROGRAM survives SIGKILL on the board (a wedged ioctl?): the board needs a person" >&2
    fi
  fi
  for ext in out err; do
    if [ "$lost" = 1 ] || ! bscp -q "${opts[@]}" "$host:$stage/run/$id.$ext" "$raw/run/$id.$ext" 2>/dev/null; then
      : > "$raw/run/$id.$ext"
    fi
  done
  klog=$raw/kmsg/$id.log
  if [ "$lost" = 0 ] && rssh dmesg > "$raw/kmsg/$id.after" 2>/dev/null; then
    py window "$raw/kmsg/$id.before" "$raw/kmsg/$id.after" "$klog" 2>> "$raw/kmsg/$id.note"
  else
    klog=$raw/kmsg/$id.none
  fi
  IFS=$'\t' read -r verdict ms pic kcount exit_line info lib < <(py judge "${rrc:-255}" "$elapsed" "$lim" \
    "$raw/run/$id.out" "$raw/run/$id.err" "$klog" "$stage/lib" "$raw/kmsg/$id.class")
  [ -z "$wedged" ] || verdict=hang
}

# save_one <id>: the redacted log and kernel files of one process.
save_one() {
  local id=$1 ext
  for ext in out err; do py redact "$raw/run/$id.$ext" "$out/log/$id.$ext"; done
  if [ -f "$klog" ]; then
    py redact "$klog" "$out/kmsg/$id.log"
    cp "$raw/kmsg/$id.class" "$out/kmsg/$id.class"
  else
    echo '# kernel capture unavailable: the board was lost' > "$out/kmsg/$id.log"
  fi
}

board_gone_msg() { # <what>
  if [ "$board_ok" = 1 ]; then
    echo "$me: $1: the board came back with another boot_id ($boot0 -> $boot_now): it rebooted; the board needs a person, not retried" >&2
  else
    echo "$me: $1: the board does not answer ssh after ${back_s} s: the board needs a person, not retried" >&2
  fi
}

# After a fault, a hang or a crash: is the device still usable? An
# unprivileged short scene (PROBE_SCENE) runs as the next process; it passes
# when it neither hangs, crashes, faults nor times out. A failed probe is
# asked once more; a second failure, a wedged process, or a board that stops
# answering ssh ends the run (needs a person). Nothing is retried after the
# board is gone.
probe_device() { # <n>
  local n=$1 k good
  for k in 1 2; do
    echo "$me: scene $n: probing the device with $PROBE_SCENE ($k/2, bound $probe_s s)"
    run_one "probe$n-$k" "$PROBE_SCENE" "$probe_s"
    save_one "probe$n-$k"
    if [ "$lost" = 1 ]; then
      probes+=("$n	$k	board-lost	-	-")
      board_gone_msg "scene $n: probe"
      stop=board-lost exit_rc=5
      return
    fi
    case $verdict in
      hang | crash | gpu-fault | job-timeout | wrong-lib) good=fail ;;
      *) good=pass ;;
    esac
    probes+=("$n	$k	$good	$verdict	$kcount")
    echo "$me: scene $n: probe $k: $good ($verdict, kmsg $kcount)"
    [ "$good" = fail ] || return 0
    if [ -n "$wedged" ]; then
      stop=wedged exit_rc=5
      return
    fi
    [ "$k" = 2 ] || sleep "${PROBE_PAUSE_S:-5}"
  done
  echo "$me: scene $n: the device failed its probe twice: the board needs a person, not retried" >&2
  stop=probe-failed exit_rc=5
}

# One try at ssh for the closing line of a stop that needs a person.
final_reach() {
  local b
  if b=$(bssh -t 30 "${opts[@]}" "$host" 'cat /proc/sys/kernel/random/boot_id' 2> /dev/null) && [ -n "$b" ]; then
    if [ "$b" = "$boot0" ]; then
      echo "$me: final ssh reachability: answers, boot_id unchanged"
    else
      echo "$me: final ssh reachability: answers, boot_id changed (rebooted)"
    fi
  else
    echo "$me: final ssh reachability: does not answer"
  fi
}

for ((i = 0; i < nscenes; i++)); do
  n=$((i + 1)) scene=${SCENES[i]}
  if [ -n "$stop" ]; then
    rows+=("$scene	$commit	not-run	-	-	-	-")
    printf '# not run: the run stopped (%s)\n' "$stop" > "$out/kmsg/$n.log"
    continue
  fi
  echo "$me: scene $n/$nscenes $scene"
  run_one "$n" "$scene" "$limit"
  if [ "$lost" = 1 ]; then
    verdict=board-lost stop=board-lost exit_rc=5
    board_gone_msg "scene $n"
  elif [ "$verdict" = wrong-lib ]; then
    stop=wrong-lib exit_rc=3
    why="a 'yakogl-mali: loaded' line naming a library outside $stage/lib"
    [ "$lib" != missing ] || why="no 'yakogl-mali: loaded' line"
    echo "$me: scene $n: refused: stderr has $why: the board's own libGLESv2.so.2 ran, not the staged pair; stopping" >&2
  fi
  [ -z "$wedged" ] || stop=wedged exit_rc=5
  rows+=("$scene	$commit	$verdict	$ms	$pic	$kcount	$exit_line")
  [ "$pic" = - ] || [ "$info" = - ] || printf 'mali | %s | %s\n' "$info" "$pic" >> "$raw/pictures.txt"
  save_one "$n"
  echo "$me: scene $n: $verdict, ms/frame $ms, kmsg $kcount, rc ${rrc:--}"
  if [ -z "$stop" ]; then
    probe=0
    case $verdict in hang | crash | gpu-fault | job-timeout) probe=1 ;; esac
    [ "$kcount" = - ] || [ "$kcount" = 0 ] || probe=1
    [ "${rrc:-0}" -lt 128 ] 2> /dev/null || probe=1
    [ "$probe" = 0 ] || probe_device "$n"
  fi
done

if [ "$board_ok" = 1 ] && [ "$stop" != board-lost ]; then alive || true; fi
write_tables
finished=1
echo "$me: boot_id after ${boot_now:-unknown}; run directory $out (raw files $raw, never committed)"
echo "$summary"
if [ "$exit_rc" = 5 ]; then
  echo "$me: needs a person: the run stopped ($stop); the scenes after it are not-run and nothing is retried"
  final_reach
fi
exit "$exit_rc"
