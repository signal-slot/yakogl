#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# glmark2-es2's default benchmark set on the host, against YakoGL built for
# each GPU with the Linux OSAL (nothing is rendered; the GL core, the
# in-process compiler and the back end's command generation run), recording
# every shader text, compile/link failure, GL error, refused draw,
# incomplete-texture line and the EGL config (README.md).
#
# usage: run.sh <GLMARK2_SRC> [out] [gpu...]
#   GLMARK2_SRC  glmark2 at 22c527cb0556f3a1ac4445aaa52cc532760928d5 (a git
#                checkout, which is verified, or the GitHub archive of that
#                commit, which cannot be)
#   out          work directory (default: build/glmark2-host)
#   gpu          powervr vivante v3d (default: all three)
#
# environment:
#   GM2_FRAMES=3     frames per scene (glmark2's nframes option)
#   GM2_SIZE=800x600 window size (glmark2's default, and the boards')
#   GM2_WRITE=1      write FINDINGS.md (powervr, vivante) and FINDINGS-v3d.md
#                    (v3d) from the run (default); 0 leaves the tree alone.
#                    The shader corpus is written to <out>/shaders either
#                    way (powervr, vivante runs); it is GPL-3.0 text and is
#                    never committed (tools/shaders/glmark2/README.md)
#   GM2_JOBS         parallel scene runs (default: nproc)
#   GM2_KICKS=1      on the PowerVR, log after every draw the framebuffer it
#                    rendered into and the kick holding it ("gm2: draw",
#                    shim.c), to read a scene's pass order in logs/powervr/
#   YAKOGL_V3D_*     the V3D's settings (src/osal/linux.c, "the V3D"); each
#                    one not set is taken from the Pi 4's glmark2 image,
#                    driver/samples/zephyr/glmark2/boards/rpi_4b.conf (its
#                    CONFIG_YAKOGL_V3D_* lines: the 96 MiB carve-out)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
COMMIT=22c527cb0556f3a1ac4445aaa52cc532760928d5
VERSION=2023.01 # meson.build's project version at that commit

die() { echo "glmark2/run.sh: $*" >&2; exit 1; }

[ $# -ge 1 ] || { sed -n '3,32p' "$0"; exit 2; }
# The physical path: the tree is often reached through a symlink (ccfarm's
# dependency layout, ~/.local/share/ccfarm/deps/<project>/glmark2-src, is
# one), and git reports its top level resolved, so the comparison below
# must see the same spelling on both sides.
GM_ROOT="$(cd "$1" && pwd -P)" || die "no directory $1"
OUT="${2:-$ROOT/build/glmark2-host}"
shift $(( $# >= 2 ? 2 : 1 ))
GPUS=("$@")
[ ${#GPUS[@]} -gt 0 ] || GPUS=(powervr vivante v3d)
FRAMES="${GM2_FRAMES:-3}"
SIZE="${GM2_SIZE:-800x600}"

[ -f "$GM_ROOT/src/default-benchmarks.h" ] || die "$GM_ROOT is not a glmark2 tree (no src/default-benchmarks.h)"
# a git checkout of its own (not a directory inside another tree)
if [ "$(git -C "$GM_ROOT" rev-parse --show-toplevel 2>/dev/null)" = "$GM_ROOT" ]; then
  head=$(git -C "$GM_ROOT" rev-parse HEAD)
  dirty=$(git -C "$GM_ROOT" status --porcelain --untracked-files=no)
  [ "$head" = "$COMMIT" ] && [ -z "$dirty" ] ||
    die "$GM_ROOT is at $head (local changes: '${dirty}'), not the pinned $COMMIT without local changes"
  provenance="git checkout at $COMMIT"
else
  case "$(basename "$GM_ROOT")" in
    "glmark2-$COMMIT") provenance="GitHub archive glmark2-$COMMIT (not a git tree: the commit is the archive's name)" ;;
    *) die "$GM_ROOT is neither a git checkout nor the archive directory glmark2-$COMMIT" ;;
  esac
fi
for g in "${GPUS[@]}"; do
  case "$g" in
    powervr | vivante | v3d) ;;
    *) die "unknown gpu '$g' (powervr, vivante, v3d)" ;;
  esac
done

# The V3D's settings as the Pi 4's glmark2 image has them: each
# CONFIG_YAKOGL_V3D_<NAME> of its board file becomes YAKOGL_V3D_<NAME> for
# src/osal/linux.c (y = 1, n = 0), unless the environment sets it already.
# The values are listed in v3d-settings.txt.
PI4_CONF="$ROOT/driver/samples/zephyr/glmark2/boards/rpi_4b.conf"
[ -f "$PI4_CONF" ] || die "no $PI4_CONF"
while IFS='=' read -r name value; do
  name="${name#CONFIG_}"
  case "$value" in y) value=1 ;; n) value=0 ;; esac
  [ -n "${!name+x}" ] || export "$name=$value"
done < <(grep -E '^CONFIG_YAKOGL_V3D_[A-Z0-9_]+=' "$PI4_CONF")
[ -n "${YAKOGL_V3D_CARVEOUT_BYTES:-}" ] || die "$PI4_CONF sets no CONFIG_YAKOGL_V3D_CARVEOUT_BYTES"

mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
GM="$GM_ROOT/src"
echo "glmark2: $provenance" | tee "$OUT/provenance.txt"
env | grep -E '^YAKOGL_V3D_' | sort > "$OUT/v3d-settings.txt" || true

# ---------------------------------------------------------------------------
# glmark2: upstream's sources, unchanged (src/meson.build's common_sources,
# the GLESv2 glad loader, the EGL state, the bundled image libraries), as
# driver/samples/zephyr/glmark2/CMakeLists.txt lists them, with upstream's
# own libmatrix/util.cc (the host reads the data files from GLMARK2_SRC/data).
# GPU-independent: built once.
# ---------------------------------------------------------------------------
GMO="$OUT/glmark2-obj"
mkdir -p "$GMO"
cxx_src=(benchmark-collection.cpp benchmark.cpp canvas-generic.cpp gl-headers.cpp
  gl-visual-config.cpp image-reader.cpp
  libmatrix/log.cc libmatrix/mat.cc libmatrix/program.cc libmatrix/shader-source.cc libmatrix/util.cc
  main-loop.cpp mesh.cpp model.cpp options.cpp results-file.cpp
  scene-buffer.cpp scene-build.cpp scene-bump.cpp scene-clear.cpp scene-conditionals.cpp
  scene.cpp scene-default-options.cpp scene-desktop.cpp scene-effect-2d.cpp
  scene-function.cpp scene-grid.cpp
  scene-ideas/a.cc scene-ideas.cpp scene-ideas/d.cc scene-ideas/e.cc scene-ideas/i.cc
  scene-ideas/lamp.cc scene-ideas/logo.cc scene-ideas/m.cc scene-ideas/n.cc
  scene-ideas/o.cc scene-ideas/s.cc scene-ideas/splines.cc scene-ideas/table.cc
  scene-ideas/t.cc
  scene-jellyfish.cpp scene-loop.cpp scene-pulsar.cpp scene-refract.cpp
  scene-shading.cpp scene-shadow.cpp
  scene-terrain/base-renderer.cpp scene-terrain/blur-renderer.cpp
  scene-terrain/copy-renderer.cpp scene-terrain.cpp scene-terrain/luminance-renderer.cpp
  scene-terrain/normal-from-height-renderer.cpp scene-terrain/overlay-renderer.cpp
  scene-terrain/renderer-chain.cpp scene-terrain/simplex-noise-renderer.cpp
  scene-terrain/terrain-renderer.cpp scene-terrain/texture-renderer.cpp
  scene-texture.cpp shared-library.cpp text-renderer.cpp texture.cpp
  gl-state-egl.cpp)
c_src=(glad/src/gles2.c glad/src/egl.c)
for f in adler32 compress crc32 deflate gzclose gzlib gzread gzwrite infback inffast inflate \
  inftrees trees uncompr zutil; do c_src+=("zlib/$f.c"); done
for f in png pngerror pnggccrd pngget pngmem pngpread pngread pngrio pngrtran pngrutil pngset \
  pngtrans pngvcrd pngwio pngwrite pngwtran pngwutil; do c_src+=("libpng/$f.c"); done
for f in jcapimin jcapistd jccoefct jccolor jcdctmgr jchuff jcinit jcmainct jcmarker jcmaster \
  jcomapi jcparam jcphuff jcprepct jcsample jdapimin jdapistd jdatadst jdatasrc jdcoefct jdcolor \
  jddctmgr jdhuff jdinput jdmainct jdmarker jdmaster jdmerge jdphuff jdpostct jdsample jerror \
  jfdctflt jfdctfst jfdctint jidctflt jidctfst jidctint jidctred jmemmgr jmemnobs jquant1 jquant2 \
  jsimd_none jutils; do c_src+=("libjpeg-turbo/$f.c"); done
# YakoGL's include/ first: glad's EGL header then takes <EGL/eglplatform.h>
# from there (EGLNativeWindowType is struct gles_native_window *).
gm_inc=(-I"$ROOT/include" -I"$GM/glad/include" -I"$GM" -I"$GM/libmatrix" -I"$GM/scene-ideas"
  -I"$GM/scene-terrain" -I"$GM/libpng" -I"$GM/zlib" -I"$GM/libjpeg-turbo")
gm_def=(-D_GNU_SOURCE -DGLMARK2_USE_GLESv2 -DGLMARK2_USE_EGL -DUSE_EXCEPTIONS
  "-DGLMARK_VERSION=\"$VERSION\"" "-DGLMARK_DATA_PATH=\"$GM_ROOT/data\""
  '-DGLMARK2_EXECUTABLE="glmark2-es2"')
# one object per source, rebuilt when the source is newer
export GM GMO
export GM_INC="${gm_inc[*]}" GM_DEF_FILE="$GMO/defs"
printf '%s\n' "${gm_def[@]}" > "$GM_DEF_FILE"
echo "glmark2: compiling upstream (${#cxx_src[@]} C++, ${#c_src[@]} C sources)"
: > "$GMO/cc.log"
printf '%s\n' "${cxx_src[@]}" "${c_src[@]}" |
  xargs -P "${GM2_JOBS:-$(nproc)}" -I{} bash -c '
    f="{}"; o="$GMO/$(echo "${f%.*}" | tr / _).o"
    [ "$o" -nt "$GM/$f" ] && exit 0
    mapfile -t defs < "$GM_DEF_FILE"
    case "$f" in
      *.c) gcc -O1 -g -w -DZ_HAVE_UNISTD_H $GM_INC "${defs[@]}" -c "$GM/$f" -o "$o" ;;
      *) g++ -O1 -g -w -std=c++17 $GM_INC "${defs[@]}" -c "$GM/$f" -o "$o" ;;
    esac' >> "$GMO/cc.log" 2>&1 || { tail -30 "$GMO/cc.log"; die "compiling glmark2 failed"; }
# shared-library.o's dlopen/dlsym are shim.c's (the library is linked in)
objcopy --redefine-sym dlopen=gm2_dlopen --redefine-sym dlsym=gm2_dlsym \
  --redefine-sym dlclose=gm2_dlclose --redefine-sym dlerror=gm2_dlerror \
  "$GMO/shared-library.o" "$GMO/shared-library.gm2.o"
gm_objs=()
for f in "${cxx_src[@]}" "${c_src[@]}"; do
  o="$GMO/$(echo "${f%.*}" | tr / _).o"
  [ "$f" = shared-library.cpp ] && o="$GMO/shared-library.gm2.o"
  gm_objs+=("$o")
done
g++ -O1 -g -Wall -std=c++17 "${gm_inc[@]}" "${gm_def[@]}" -I"$HERE" -c "$HERE/host_main.cpp" -o "$GMO/host_main.o"
gcc -O1 -g -Wall -std=gnu11 -I"$ROOT/include" -I"$HERE" -c "$HERE/shim.c" -o "$GMO/shim.o"

# ---------------------------------------------------------------------------
# Per GPU: the library (the top-level CMake host build: Linux OSAL, the
# in-process compiler, OpenGL ES 2.0 reported, release, as the board images
# take it), the program, and one process per benchmark-list entry (so each
# scene's once-per-process lines, e.g. the incomplete-texture warning, are
# its own).
# ---------------------------------------------------------------------------
# capi/yakogl_capi.cmake takes the rustc next to cargo: the first cargo on
# PATH that has one (a wrapper such as a build cache's shim may not)
CARGO_BIN=""
IFS=: read -r -a path_dirs <<< "$PATH"
for d in "${path_dirs[@]}"; do
  if [ -x "$d/cargo" ] && [ -x "$d/rustc" ]; then CARGO_BIN="$d/cargo"; break; fi
done
[ -n "$CARGO_BIN" ] || die "no cargo on PATH with a rustc next to it (capi/yakogl_capi.cmake needs one)"
for g in "${GPUS[@]}"; do
  B="$OUT/lib-$g"
  echo "glmark2: libyakogl.a for $g ($CARGO_BIN)"
  cmake -S "$ROOT" -B "$B" -DYAKOGL_OSAL=linux -DYAKOGL_GPU="$g" -DYAKOGL_SHADERC=online \
    -DYAKOGL_REPORT_ES2=ON -DYAKOGL_CARGO_PROFILE=release -DYAKOGL_CARGO="$CARGO_BIN" > "$B.cmake.log" 2>&1 ||
    { tail -20 "$B.cmake.log"; die "cmake ($g) failed"; }
  cmake --build "$B" --target yakogl_archive -j "$(nproc)" >> "$B.cmake.log" 2>&1 ||
    { tail -30 "$B.cmake.log"; die "building libyakogl.a ($g) failed"; }
  g++ -o "$OUT/glmark2-host-$g" "$GMO/host_main.o" "$GMO/shim.o" "${gm_objs[@]}" "$B/libyakogl.a" \
    -lm -lpthread -ldl || die "linking glmark2-host-$g failed"
done

mapfile -t ENTRIES < <("$OUT/glmark2-host-${GPUS[0]}" --gm2-list)
echo "glmark2: ${#ENTRIES[@]} default benchmarks, $FRAMES frames each, ${GPUS[*]}"
printf '%s\n' "${ENTRIES[@]}" > "$OUT/entries.txt"
jobs=()
for g in "${GPUS[@]}"; do
  rm -rf "$OUT/logs/$g" "$OUT/capture/$g"
  mkdir -p "$OUT/logs/$g" "$OUT/capture/$g"
  for i in "${!ENTRIES[@]}"; do jobs+=("$g $i"); done
done
run_entry() {
  local g="$1" i="$2" entry rc=0
  entry=$(sed -n "$((i + 1))p" "$OUT/entries.txt")
  ( cd "$OUT/logs/$g"
    YAKOGL_ERROR_LOG=1000000 GM2_CAPTURE="$OUT/capture/$g" timeout "${GM2_TIMEOUT:-600}" \
      "$OUT/glmark2-host-$g" --size "$SIZE" -b ":nframes=$FRAMES" -b "$entry" \
      > "$OUT/logs/$g/$i.log" 2>&1 ) || rc=$?
  echo "$rc" > "$OUT/logs/$g/$i.rc"
}
export -f run_entry
export OUT SIZE FRAMES
printf '%s\n' "${jobs[@]}" | xargs -P "${GM2_JOBS:-$(nproc)}" -L1 bash -c 'run_entry $0 $1'
echo "glmark2: runs done in $OUT/logs"

# Two reports: the PowerVR and the GC7000 in FINDINGS.md (with the shader
# corpus), the V3D in FINDINGS-v3d.md (its run models the Pi 4's carve-out,
# which the other two do not model for their boards).
pvr_gc=() v3d=()
for g in "${GPUS[@]}"; do
  if [ "$g" = v3d ]; then v3d=(v3d); else pvr_gc+=("$g"); fi
done
report() {
  python3 -I "$HERE/report.py" "$OUT" --commit "$COMMIT" --size "$SIZE" --frames "$FRAMES" "$@"
}
if [ ${#pvr_gc[@]} -gt 0 ]; then
  if [ "${GM2_WRITE:-1}" = 1 ]; then
    report --shaders "$OUT/shaders" --findings "$ROOT/tools/cts/log/hostharness/glmark2/FINDINGS.md" "${pvr_gc[@]}"
  else
    report --shaders "$OUT/shaders" "${pvr_gc[@]}"
  fi
fi
if [ ${#v3d[@]} -gt 0 ]; then
  if [ "${GM2_WRITE:-1}" = 1 ]; then
    report --findings "$ROOT/tools/cts/log/hostharness/glmark2/FINDINGS-v3d.md" --settings "$OUT/v3d-settings.txt" v3d
  else
    report --settings "$OUT/v3d-settings.txt" v3d
  fi
fi
