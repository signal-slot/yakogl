#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# glmark2-host-pose.sh -- glmark2-es2 scenes at 800x600 drawn by Mesa's
# llvmpipe on the host at a fixed frame time: the scene clock advances exactly
# d ms per frame, so each scene ends on the pose that a renderer taking d ms a
# frame (a board) reaches, whatever llvmpipe's own speed.
#
#   tools/cts/glmark2-host-pose.sh [--out DIR] --scenes LIST --dt LIST_MS [--repeat N]
#   tools/cts/glmark2-host-pose.sh --self-test
#
# Why. glmark2's animated scenes run 10 s of their own clock and the clock
# follows the renderer's speed (tools/cts/glmark2-host-ref.sh, "Animated
# scenes"), so llvmpipe and a board end on different poses and a probe that
# moves between two board runs cannot be told from a defect by value. With the
# frame time fixed at a board's measured ms per frame, llvmpipe draws that
# board's last pose.
#
# How. Upstream glmark2 at the pinned commit (GLMARK2_SRC, as
# glmark2-host-ref.sh finds it) is copied to DIR/src/glmark2-<commit>/ and
# patched with glmark2-host-pose.patch beside this script (src/scene.cpp only:
# with GMREF_FIXED_DT_MS=<d> the k-th Scene::update() reads k * d ms). That
# copy is handed to glmark2-host-ref.sh as its GLMARK2_SRC (the archive form
# it accepts), which applies glmark2-host-ref.patch on top (the frame dump and
# picture line), picks or builds Mesa, builds glmark2-es2-null into
# DIR/ref/build and draws the first selected scene at wall time
# (DIR/ref/, a baseline). Then every selected scene is drawn once per d (and
# per repeat) with GMREF_FIXED_DT_MS=d by that binary, the way
# glmark2-host-ref.sh runs it (surfaceless EGL, --off-screen, llvmpipe):
# DIR/<slug>/dt<d>[-r<n>].log, decoded by tools/cts/glmark2-fb.py to .png.
#
# Output. DIR/poses.txt has one line per scene and d, in the probe notation of
# tools/cts/glmark2-pictures.txt:
#   <scene info string> | dt <d> | <the text after "glmark2-picture: <scene>: "> | crc32 <frame crc32>
# (with --repeat N > 1, the first repeat's line). DIR/determinism.txt (only
# with --repeat > 1) says per scene and d whether every repeat gave the same
# frame crc32. DIR/env.txt names the commits, patches, Mesa and renderer.
# A run fails (exit 1) when a scene did not decode, glmark2's FrameTime is not
# d (the patch did not take), the renderer is not llvmpipe, or repeats differ.
#
# DIR defaults to records/host/<YYYYMMDD-HHMM>-gmpose-<sha8 of HEAD>/
# (records/ is gitignored). --scenes takes the slugs of glmark2-host-ref.sh
# --list, with its rule (an item selects the slugs equal to it or starting
# with it and '-'; "texture" is the three texture scenes). --dt takes a
# comma-separated list of ms per frame, each a decimal number in (0, 10000]
# ("16.95", "250"). --repeat N (default 1) draws each scene and d N times.
#
# The pose. The scene stops at the first update with k * d >= 10000 ms, and
# the frame it ends on was drawn at the update before, so the last frame shows
# the scene at t_last = (ceil(10000 / d) - 1) * d ms, short of 10 s by the gap
# g = 10000 - t_last, 0 < g <= d. Every scene but [buffer] draws a function
# of t_last alone (two d with the same t_last give the same crc32), so a d
# alone does not fix a board's pose: a board at d ms a frame ends with any g
# in (0, d], set by when its frames fall. To draw the scene at a given t_last
# T (10000 - d_board <= T < 10000), with n = ceil(10000 / d_board) frames:
#   d = T / (n - 1)      (keep d > 10000 / n; [buffer] also needs n right)
# e.g. the i.MX8MP's 59 FPS (d_board 16.95, n = 590) ending 15 ms short:
#   tools/cts/glmark2-host-pose.sh --scenes texture --dt 16.9525
# The board's ms per frame is 1000 / its integer FPS (the per-scene record),
# or 10000 / frames from page 0 (docs/notes/imx8mp-glmark2-gap.md); to bracket
# a board run whose g is unknown, sweep d over that FPS bin. The desktop,
# ideas and jellyfish scenes animate by a wall clock of their own; the patch
# does not fix them.
#
# --self-test checks, without a GPU or a network: the scene list and slugs of
# glmark2-host-ref.sh --list, --scenes and --dt parsing, the -b argument of an
# info string, the argument errors, and that glmark2-host-pose.patch applies
# to GLMARK2_SRC alone and on top of glmark2-host-ref.patch (GLMARK2_SRC must
# be present: tools/cts/deps-env.sh --build GLMARK2_SRC).
#
# Environment: GLMARK2_SRC, GMREF_MESA_PREFIX, LLVM_DIR, GMREF_JOBS (as
# glmark2-host-ref.sh reads them).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
REF="$HERE/glmark2-host-ref.sh"
REF_PATCH="$HERE/glmark2-host-ref.patch"
PATCH="$HERE/glmark2-host-pose.patch"
FB="$HERE/glmark2-fb.py"

die() { echo "glmark2-host-pose.sh: $*" >&2; exit 2; }

# The pinned glmark2 commit, glmark2-host-ref.sh's.
ref_commit() { sed -n 's/^COMMIT=\([0-9a-f]\{40\}\).*$/\1/p' "$REF"; }

# The scene list: "<slug>\t<info string>" per scene, glmark2-host-ref.sh's.
scene_list() { bash "$REF" --list; }

# barg <info string>: the -b argument that gives it ("[texture]
# texture-filter=mipmap" -> texture:texture-filter=mipmap; glmark2 sorts the
# options, so the sorted list selects the same scene).
barg() {
  local name opts
  name="${1%%]*}"
  name="${name#[}"
  opts="${1#*] }"
  if [ "$opts" = "<default>" ]; then printf '%s\n' "$name"; else printf '%s:%s\n' "$name" "$opts"; fi
}

# select_scenes <list> <listing>: the listing's lines the --scenes list selects.
select_scenes() {
  local want="$1" listing="$2" item s info found
  local -a items
  IFS=',' read -r -a items <<< "$want"
  [ "${#items[@]}" -gt 0 ] || die "--scenes: an empty list"
  local -A pick=()
  for item in "${items[@]}"; do
    [ -n "$item" ] || continue
    found=0
    while IFS=$'\t' read -r s info; do
      if [ "$s" = "$item" ] || [ "${s#"$item"-}" != "$s" ]; then
        pick[$s]=1
        found=1
      fi
    done <<< "$listing"
    [ "$found" = 1 ] || die "--scenes: '$item' names no scene (glmark2-host-ref.sh --list)"
  done
  while IFS=$'\t' read -r s info; do
    [ -z "${pick[$s]:-}" ] || printf '%s\t%s\n' "$s" "$info"
  done <<< "$listing"
}

# parse_dts <list>: each d of the --dt list, one per line, as given.
parse_dts() {
  local d n=0
  local -a items
  IFS=',' read -r -a items <<< "$1"
  for d in "${items[@]}"; do
    [[ "$d" =~ ^[0-9]+(\.[0-9]+)?$ ]] || die "--dt: '$d' is not a decimal number of ms"
    awk -v d="$d" 'BEGIN { exit !(d > 0 && d <= 10000) }' || die "--dt: '$d' is not in (0, 10000]"
    printf '%s\n' "$d"
    n=$((n + 1))
  done
  [ "$n" -gt 0 ] || die "--dt: an empty list"
}

# The glmark2 tree to patch: GLMARK2_SRC, else the layout's glmark2-src.
glmark2_src() {
  local src="${GLMARK2_SRC:-}"
  [ -n "$src" ] || src="$("$HERE/deps-env.sh" --layout)/glmark2-src"
  [ -f "$src/src/default-benchmarks.h" ] ||
    die "no glmark2 tree at $src (set GLMARK2_SRC, or tools/cts/deps-env.sh --build GLMARK2_SRC)"
  (cd "$src" && pwd -P)
}

# redact <dir>: the run's text records (not its src/ and ref/build/ trees)
# name the repository, the dependency layout and the home directory by
# <repo>, <layout> and <home>, not by their paths.
redact() {
  local dir="$1" layout
  layout="$("$HERE/deps-env.sh" --layout 2>/dev/null || true)"
  find "$dir" \( -path "$dir/src" -o -path "$dir/ref/build" \) -prune -o \
    -type f \( -name '*.txt' -o -name '*.log' \) -print0 |
    while IFS= read -r -d '' f; do
      sed -i ${layout:+-e "s|$layout|<layout>|g"} -e "s|$ROOT|<repo>|g" \
        ${HOME:+-e "s|$HOME|<home>|g"} "$f"
    done
}

self_test() {
  local ok=1 listing got rc src scratch commit
  fail() { echo "FAIL $*"; ok=0; }
  listing=$(scene_list) || { echo "FAIL glmark2-host-ref.sh --list"; return 1; }
  [ "$(printf '%s\n' "$listing" | wc -l)" = 33 ] || fail "glmark2-host-ref.sh --list has not 33 scenes"
  [ -z "$(printf '%s\n' "$listing" | cut -f1 | sort | uniq -d)" ] || fail "a slug is used twice"
  # --scenes
  got=$(select_scenes texture "$listing" | cut -f1 | paste -sd' ' -)
  [ "$got" = "texture-texture-filter-nearest texture-texture-filter-linear texture-texture-filter-mipmap" ] ||
    fail "--scenes texture gives '$got'"
  got=$(select_scenes texture,buffer-columns-200-interleave-false-update-dispersion-0-9-update-fraction-0-5-update-method-map,terrain,shadow,refract "$listing" | cut -f1 | wc -l)
  [ "$got" = 7 ] || fail "the sweep's seven scenes give $got"
  got=$(select_scenes shadow "$listing" | cut -f2)
  [ "$got" = "[shadow] <default>" ] || fail "--scenes shadow gives '$got', not the shadow scene alone"
  ( select_scenes nosuchscene "$listing" ) > /dev/null 2>&1 && fail "--scenes nosuchscene was accepted"
  ( select_scenes "" "$listing" ) > /dev/null 2>&1 && fail "--scenes '' was accepted"
  # -b
  [ "$(barg '[jellyfish] <default>')" = jellyfish ] || fail "barg <default>"
  [ "$(barg '[texture] texture-filter=mipmap')" = texture:texture-filter=mipmap ] || fail "barg texture"
  [ "$(barg '[effect2d] kernel=0,1,0;1,-4,1;0,1,0;')" = 'effect2d:kernel=0,1,0;1,-4,1;0,1,0;' ] || fail "barg effect2d"
  [ "$(barg '[buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map')" = \
    buffer:columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map ] || fail "barg buffer"
  # --dt
  [ "$(parse_dts 8,12,14,14.5,16.95,250 | paste -sd' ' -)" = "8 12 14 14.5 16.95 250" ] || fail "--dt good list"
  local bad
  for bad in 0 0.0 -1 abc 1e3 14. .5 10000.5 20000 "" "8,,12" "8;12"; do
    ( parse_dts "$bad" ) > /dev/null 2>&1 && fail "--dt '$bad' was accepted"
  done
  # argument errors exit 2 before anything is built
  for bad in "--scenes texture" "--dt 16" "--scenes texture --dt 0" "--scenes nosuchscene --dt 16" \
    "--scenes texture --dt 16 --repeat 0" "--bogus"; do
    rc=0
    # shellcheck disable=SC2086 # the words are the arguments
    bash "$0" $bad > /dev/null 2>&1 || rc=$?
    [ "$rc" = 2 ] || fail "'$bad' exits $rc, not 2"
  done
  # the patches, against GLMARK2_SRC (no network)
  commit=$(ref_commit)
  [ -n "$commit" ] || fail "no COMMIT= in $REF"
  [ "$(head -1 "$PATCH" | cut -d' ' -f2)" = "$commit" ] || fail "$PATCH names another commit than $REF"
  [ -f "$PATCH.license" ] || fail "no $PATCH.license"
  src=$(glmark2_src) || { echo "FAIL no GLMARK2_SRC"; return 1; }
  patch -p1 --dry-run -s -f -d "$src" < "$PATCH" > /dev/null || fail "$PATCH does not apply to $src"
  scratch=$(mktemp -d)
  mkdir -p "$scratch/src"
  cp "$src/meson.build" "$scratch/"
  cp "$src/src/main-loop.cpp" "$src/src/scene.cpp" "$scratch/src/"
  if patch -p1 -s -f -d "$scratch" < "$REF_PATCH" > /dev/null; then
    patch -p1 -s -f -d "$scratch" < "$PATCH" > /dev/null || fail "$PATCH does not apply on top of $REF_PATCH"
    grep -q GMREF_FIXED_DT_MS "$scratch/src/scene.cpp" || fail "the patched scene.cpp does not read GMREF_FIXED_DT_MS"
  else
    fail "$REF_PATCH does not apply to $src"
  fi
  # redact: the records name no path of the repository; src/ is left alone
  mkdir -p "$scratch/out/a" "$scratch/out/src"
  echo "png $ROOT/records/x.png" > "$scratch/out/a/x.fb.txt"
  echo "tree $ROOT" > "$scratch/out/src/keep.txt"
  redact "$scratch/out"
  [ "$(cat "$scratch/out/a/x.fb.txt")" = "png <repo>/records/x.png" ] || fail "redact left '$(cat "$scratch/out/a/x.fb.txt")'"
  [ "$(cat "$scratch/out/src/keep.txt")" = "tree $ROOT" ] || fail "redact changed src/"
  rm -rf "$scratch"
  [ "$ok" = 1 ] && echo "self-test: ok" || echo "self-test: FAILED"
  [ "$ok" = 1 ]
}

OUT=""
WANT=""
DTS=""
REPEAT=1
while [ $# -gt 0 ]; do
  case "$1" in
    --self-test) self_test; exit $? ;;
    --out) OUT="${2:?--out DIR}"; shift 2 ;;
    --scenes) WANT="${2:?--scenes LIST}"; shift 2 ;;
    --dt) DTS="${2:?--dt LIST_MS}"; shift 2 ;;
    --repeat) REPEAT="${2:?--repeat N}"; shift 2 ;;
    -h | --help) awk 'NR > 4 && /^#/ { print; next } NR > 4 { exit }' "$0"; exit 0 ;;
    *) die "unknown argument '$1' (--help)" ;;
  esac
done
[ -n "$WANT" ] || die "--scenes LIST is needed"
[ -n "$DTS" ] || die "--dt LIST_MS is needed"
[[ "$REPEAT" =~ ^[1-9][0-9]*$ ]] || die "--repeat: '$REPEAT' is not a count of 1 or more"
mapfile -t DT < <(parse_dts "$DTS")
[ "${#DT[@]}" -gt 0 ] || die "--dt: no d"
LISTING=$(scene_list) || die "glmark2-host-ref.sh --list failed"
SELECTED=$(select_scenes "$WANT" "$LISTING")
[ -n "$SELECTED" ] || die "--scenes: no scene"

if [ -z "$OUT" ]; then
  OUT="$ROOT/records/host/$(date +%Y%m%d-%H%M)-gmpose-$(git -C "$ROOT" rev-parse --short=8 HEAD)"
fi
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"

# The patched tree, in the archive form glmark2-host-ref.sh accepts.
COMMIT=$(ref_commit)
[ -n "$COMMIT" ] || die "no COMMIT= in $REF"
SRC=$(glmark2_src)
STAGE="$OUT/src/glmark2-$COMMIT"
rm -rf "$OUT/src"
mkdir -p "$STAGE"
tar -C "$SRC" --exclude=./.git -cf - . | tar -C "$STAGE" -xf -
patch -p1 -s -f -d "$STAGE" < "$PATCH"

# glmark2-host-ref.sh builds Mesa (when needed) and glmark2-es2-null, and draws
# the first scene at wall time as a baseline.
FIRST=$(printf '%s\n' "$SELECTED" | head -1 | cut -f1)
echo "== building through glmark2-host-ref.sh (baseline: $FIRST at wall time)"
rc=0
env -u GMREF_FIXED_DT_MS GLMARK2_SRC="$STAGE" bash "$REF" --out "$OUT/ref" --scenes "$FIRST" \
  > "$OUT/ref.log" 2>&1 || rc=$?
GM="$OUT/ref/build/glmark2-build/src/glmark2-es2-null"
[ -x "$GM" ] || { tail -20 "$OUT/ref.log" >&2; die "glmark2-host-ref.sh built no $GM (rc $rc, $OUT/ref.log)"; }
[ "$rc" = 0 ] || echo "glmark2-host-pose.sh: the wall-time baseline failed (rc $rc, $OUT/ref.log); going on" >&2
LIBDIR=$(sed -n 's/^mesa: .* (\(.*\))$/\1/p' "$OUT/ref/env.txt")
[ -n "$LIBDIR" ] && [ -e "$LIBDIR/libEGL.so.1" ] || die "no Mesa library directory in $OUT/ref/env.txt"

run_env=(env LD_LIBRARY_PATH="$LIBDIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  LIBGL_DRIVERS_PATH="$LIBDIR/dri" EGL_PLATFORM=surfaceless LIBGL_ALWAYS_SOFTWARE=1
  GALLIUM_DRIVER=llvmpipe MESA_SHADER_CACHE_DISABLE=true)
{
  echo "yakogl: $(git -C "$ROOT" rev-parse HEAD)"
  echo "glmark2: $COMMIT + tools/cts/glmark2-host-pose.patch sha256 $(sha256sum "$PATCH" | cut -d' ' -f1)" \
    "+ tools/cts/glmark2-host-ref.patch sha256 $(sha256sum "$REF_PATCH" | cut -d' ' -f1)"
  grep -a -E '^(mesa|host|GL_)|GL_(VENDOR|RENDERER|VERSION)' "$OUT/ref/env.txt" || true
  echo "dt: ${DT[*]} ms; repeat $REPEAT"
} > "$OUT/env.txt"

: > "$OUT/poses.txt"
[ "$REPEAT" = 1 ] || : > "$OUT/determinism.txt"
fails=0
while IFS=$'\t' read -r s info; do
  arg=$(barg "$info")
  mkdir -p "$OUT/$s"
  for d in "${DT[@]}"; do
    crcs=()
    for r in $(seq 1 "$REPEAT"); do
      base="$OUT/$s/dt$d"
      [ "$REPEAT" = 1 ] || base="$base-r$r"
      if [ "$REPEAT" = 1 ]; then echo "== $s dt $d"; else echo "== $s dt $d (repeat $r of $REPEAT)"; fi
      grc=0
      "${run_env[@]}" GMREF_FIXED_DT_MS="$d" "$GM" --off-screen -s 800x600 -b "$arg" > "$base.log" 2>&1 || grc=$?
      why=""
      [ "$grc" = 0 ] || why="glmark2 exited $grc"
      grep -aq 'GL_RENDERER: *llvmpipe' "$base.log" || why="${why:+$why; }the renderer is not llvmpipe"
      ft=$(grep -ao 'FrameTime: [0-9.]*' "$base.log" | tail -1 | awk '{print $2}')
      awk -v f="${ft:-x}" -v d="$d" 'BEGIN { exit !(f != "x" && f - d < 0.01 && d - f < 0.01) }' ||
        why="${why:+$why; }FrameTime ${ft:-none}, not $d (the clock is not fixed)"
      frc=0
      python3 -I "$FB" "$base.log" "$base.png" --scene "$info" > "$base.fb.txt" 2>&1 || frc=$?
      [ "$frc" = 0 ] || why="${why:+$why; }glmark2-fb.py rc $frc"
      crc=$(grep -ao 'glmark2-fb: end crc32 [0-9a-f]*' "$base.log" | tail -1 | awk '{print $4}')
      pic=$(grep -a "^glmark2-picture: " "$base.log" | tail -1 | sed 's/^glmark2-picture: .*: \([0-9]*x[0-9]*;.*\)$/\1/')
      crcs+=("${crc:-none}")
      if [ "$r" = 1 ]; then
        printf '%s | dt %s | %s | crc32 %s\n' "$info" "$d" "${pic:-absent}" "${crc:-none}" >> "$OUT/poses.txt"
      fi
      if [ -n "$why" ]; then
        echo "   FAILED: $why"
        fails=$((fails + 1))
      else
        echo "   ${pic} crc32 $crc"
      fi
    done
    if [ "$REPEAT" != 1 ]; then
      if [ "$(printf '%s\n' "${crcs[@]}" | sort -u | wc -l)" = 1 ] && [ "${crcs[0]}" != none ]; then
        echo "same     $s dt $d (${crcs[0]} x $REPEAT)" >> "$OUT/determinism.txt"
      else
        echo "differs  $s dt $d (${crcs[*]})" >> "$OUT/determinism.txt"
        fails=$((fails + 1))
      fi
    fi
  done
done <<< "$SELECTED"
redact "$OUT"
echo "$(printf '%s\n' "$SELECTED" | wc -l) scene(s) x ${#DT[@]} d x $REPEAT, $fails failed: $OUT/poses.txt"
[ "$fails" = 0 ]
