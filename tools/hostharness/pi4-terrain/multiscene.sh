#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# TFU mip chains a frame of glmark2 terrain after FBO scenes, in ONE host
# process, on the V3D stand-in (docs/notes/v3d-terrain-copies.md, "Host run":
# run.sh there runs one process per scene, so the Pi 4's stale render-into
# marks, which an FBO scene's deleted textures leave in the GL core's
# TARGET_USE for terrain's new textures, cannot appear).
#
# It builds or reuses the glmark2 host harness for the V3D
# (tools/hostharness/glmark2/run.sh, same <out>/lib-v3d), then links its own
# binary <out>/glmark2-host-v3d-ms: the harness objects plus
# multiscene_hook.c, which prints the back end's cumulative counters after
# every swap. It runs the scene list in one process and then the last scene
# alone, and prints for the last scene a table of its frames: TFU chains,
# TFU refusals (unknown) and tiled copies (plain, first-sample) a frame.
# one-process.sh in this directory also runs list entries in one process
# (it prints the back end's own arena lines); this is the per-frame counter
# table.
#
# The library is NOT taken from an earlier run: every start rebuilds
# <out>/lib-v3d/libyakogl.a from the current tree (incremental) and relinks
# when the archive, the hook or a harness object is newer than the binary,
# so a run before and after a change to src/v3d measures each. An <out>
# whose lib-v3d was configured from another source tree is refused.
#
# usage: multiscene.sh <GLMARK2_SRC> [out]   (out: build/pi4-multiscene)
#   MS_SCENES  space-separated list entries, in order, exactly as
#              glmark2-host-v3d --gm2-list prints them; the last one is the
#              scene the table is for
#              (default: "desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true
#              desktop:windows=4:effect=shadow terrain", the FBO scenes the
#              Pi 4's list runs before terrain, then terrain)
#   MS_FRAMES  frames per scene (default 20)
set -euo pipefail

src=${1:?usage: multiscene.sh <GLMARK2_SRC> [out]}
out=${2:-build/pi4-multiscene}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
frames=${MS_FRAMES:-20}
scenes=${MS_SCENES:-"desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true desktop:windows=4:effect=shadow terrain"}
die() { echo "multiscene.sh: $*" >&2; exit 1; }

mkdir -p "$out"
out=$(cd "$out" && pwd)
lib="$out/lib-v3d"

# the harness: built by run.sh (one terrain frame, nothing written to the tree)
if [ ! -f "$lib/CMakeCache.txt" ] || [ ! -d "$out/glmark2-obj" ] || [ ! -f "$out/v3d-settings.txt" ]; then
  echo "multiscene: building the glmark2 host harness in $out" >&2
  env GM2_WRITE=0 GM2_FRAMES=1 GM2_ENTRIES='^terrain$' \
    "$root/tools/hostharness/glmark2/run.sh" "$src" "$out" v3d >"$out/build.log" 2>&1 ||
    { tail -20 "$out/build.log" >&2; die "run.sh failed, see $out/build.log"; }
fi
home=$(sed -n 's/^CMAKE_HOME_DIRECTORY:INTERNAL=//p' "$lib/CMakeCache.txt")
[ "$(cd "$home" && pwd -P)" = "$(cd "$root" && pwd -P)" ] ||
  die "$lib was configured from $home, not $root: use another out"

# the library of the current tree
cmake --build "$lib" --target yakogl_archive -j "$(nproc)" >"$out/lib-build.log" 2>&1 ||
  { tail -30 "$out/lib-build.log" >&2; die "building libyakogl.a failed"; }

bin="$out/glmark2-host-v3d-ms"
objs=()
for o in "$out"/glmark2-obj/*.o; do
  [ "$(basename "$o")" = shared-library.o ] || objs+=("$o") # run.sh links the dlsym-renamed copy
done
stale=0
for dep in "$lib/libyakogl.a" "$here/multiscene_hook.c" "${objs[@]}"; do
  [ "$dep" -nt "$bin" ] && stale=1
done
if [ ! -x "$bin" ] || [ $stale = 1 ]; then
  gcc -O1 -g -Wall -std=gnu11 -I"$root/include" -c "$here/multiscene_hook.c" -o "$out/multiscene_hook.o"
  g++ -o "$bin" "${objs[@]}" "$out/multiscene_hook.o" "$lib/libyakogl.a" \
    -Wl,--wrap=eglGetProcAddress -lm -lpthread -ldl || die "linking $bin failed"
fi

# the Pi 4 image's V3D settings, as run.sh used them (unless set)
while IFS== read -r k v; do
  [ -n "$k" ] && [ -z "${!k:-}" ] && export "$k=$v"
done <"$out/v3d-settings.txt"

mapfile -t list < <("$bin" --gm2-list)
read -r -a want <<<"$scenes"
args=(--size 800x600 -b ":nframes=$frames")
for s in "${want[@]}"; do
  printf '%s\n' "${list[@]}" | grep -qxF -- "$s" || die "no list entry '$s' (glmark2-host-v3d --gm2-list)"
  args+=(-b "$s")
done
last=${want[${#want[@]} - 1]}

run() { # run LOG ARGS...: one process; stderr and stdout of the run to LOG
  local log=$1
  shift
  ( cd "$out" && YAKOGL_ERROR_LOG=1000000 timeout "${GM2_TIMEOUT:-1200}" "$bin" "$@" ) >"$log" 2>&1 ||
    { tail -20 "$log" >&2; die "run failed, see $log"; }
}

# report LOG: the frames of the scene LOG's last scene-begin line names from the cumulative counters of LOG.
# A swap line is "gm2ms: swap N chains C queued Q full F refused R0..R13
# copies C0..C15" (multiscene_hook.c): $5 chains, $14 refused 'unknown'
# (MIP_REFUSAL_NAMES[3]), $26 'plain', $28 'first-sample', $39 'image-store'
# (COPY_REASON_NAMES[0, 2, 13]).
report() {
  local scene
  scene=$(grep -o '^gm2: scene-begin [0-9]*' "$1" | tail -1 | cut -d' ' -f3)
  awk -v scene="$scene" '
    /^gm2: scene-begin / { cur = $3; n = 0; next }
    /^gm2ms: swap / {
      if (cur == scene) {
        n++
        printf "%5d %7d %8d %6d %12d %11d\n", n, $5 - p5, $14 - p14, $26 - p26, $28 - p28, $39 - p39
        tc += $5 - p5; if (n > 1) { st += $5 - p5; sf++ }
        c[n] = $5 - p5
      }
      p5 = $5; p14 = $14; p26 = $26; p28 = $28; p39 = $39
      next
    }
    END {
      printf "chains: %d over %d frames", tc, n
      if (sf) printf ", %.2f a frame after frame 1", st / sf
      printf "; per frame:"
      for (i = 1; i <= n && i <= 12; i++) printf " %d", c[i]
      printf "\n"
    }' "$1"
}
hdr() { printf 'frame  chains  unknown  plain  first-sample  image-store\n'; }

run "$out/multiscene.log" "${args[@]}"
solo=(--size 800x600 -b ":nframes=$frames" -b "$last")
run "$out/multiscene-solo.log" "${solo[@]}"
echo "== $last after: ${want[*]:0:${#want[@]}-1} (one process, $frames frames a scene), per frame"
hdr
report "$out/multiscene.log"
echo "== $last alone (its own process), per frame"
hdr
report "$out/multiscene-solo.log"
