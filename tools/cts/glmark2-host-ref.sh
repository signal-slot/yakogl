#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# glmark2-host-ref.sh -- reference pictures of glmark2-es2's 33 default scenes
# at 800x600, drawn by Mesa's llvmpipe on the host, for looking at a board's
# frame dump (driver/docs/glmark2.md, "The frame dump") by eye.
#
#   tools/cts/glmark2-host-ref.sh [--out DIR] [--scenes LIST] [--driver llvmpipe|softpipe]
#   tools/cts/glmark2-host-ref.sh --list
#   tools/cts/glmark2-host-ref.sh --compare DIR1 DIR2
#   tools/cts/glmark2-host-ref.sh --self-test
#
# Upstream glmark2 at the pinned commit (GLMARK2_SRC, tools/cts/deps.toml; it
# is cloned into the dependency layout when unset and absent) is copied into
# DIR/build, patched with glmark2-host-ref.patch beside this script and built
# as the null-glesv2 flavor (surfaceless EGL, --off-screen: no window system,
# no GPU). The patch prints, after each scene's last update() and before its
# last swap (the place the yakogl glmark2 image reads its picture), the image's
# 'glmark2-fb:' frame dump and 'glmark2-picture:' line. Each scene runs in a
# process of its own (-s 800x600 -b <scene>); its output is DIR/<slug>.log and
# tools/cts/glmark2-fb.py decodes it, unchanged and with the picture check, to
# DIR/<slug>.png. DIR/summary.txt has one line per scene (the decoder's
# verdict and the frame crc32), DIR/env.txt the commits, Mesa and renderer.
# Exit 0 when every scene decoded with every row good and its picture line
# agreeing, 1 otherwise.
#
# DIR defaults to records/host/<YYYYMMDD-HHMM>-gmref-<sha8 of HEAD>/ (records/
# is gitignored); nothing is written outside DIR and the dependency layout.
# --scenes takes a comma-separated list; an item selects the slugs equal to it
# or starting with it and '-' ("texture" is the three texture scenes,
# "terrain-default" or "terrain" the terrain). --list prints the 33 slugs and
# scene info strings. --compare prints, per slug present in both runs, whether
# the two frames' crc32 are equal. --self-test checks the scene list, the slugs
# and that the patch applies to GLMARK2_SRC, without a network or a GPU
# (GLMARK2_SRC must be present: tools/cts/deps-env.sh --build GLMARK2_SRC).
#
# The slug of a scene is its info string lower-cased, every run of characters
# outside [a-z0-9] replaced by one '-', and '-' trimmed from both ends
# ("[effect2d] kernel=0,1,0;1,-4,1;0,1,0;" -> effect2d-kernel-0-1-0-1-4-1-0-1-0).
# tools/cts/pi4-glmark2-dumps.sh names its board pictures by the same rule.
#
# Mesa: the system's when it has llvmpipe EGL and GLES2 (GMREF_MESA_PREFIX=/usr
# says so: a prefix whose lib/ or lib/<triplet>/ holds libEGL.so.1 and
# libGLESv2.so.2), else Mesa 26.2.1 (the release third_party/mesa-26.2.1 is
# from; the tarball's sha256 is in its UPSTREAM) built once with the gallium
# llvmpipe and softpipe drivers only, surfaceless EGL and GLES2, into
# <layout>/mesa-llvmpipe-26.2.1 (the layout is tools/cts/deps-env.sh --layout).
# It needs meson, ninja, bison, flex, python3 mako, pyyaml and packaging, zlib,
# expat, libdrm and LLVM (LLVM_DIR, default /usr/lib/llvm-21) with its
# llvm-config; the tarball is fetched into <layout>/downloads when absent.
#
# Environment: GLMARK2_SRC, GMREF_MESA_PREFIX, LLVM_DIR, GMREF_JOBS (build
# jobs, default nproc).
#
# Animated scenes. Every scene runs for its default duration (10 s of its own
# clock) and its last frame is wherever its animation stood then; the frame
# times follow the renderer's speed, so a board and llvmpipe (and two runs of
# llvmpipe) end on different poses. Two full runs on one host (2026-11-08,
# yakogl b585deb5, llvmpipe of Mesa 26.2.1 with LLVM 21.1.8, 256 bits)
# compared per scene, as the share of pixels with a channel more than 16
# levels apart:
#   animated, the pose moves between runs: compare the content only (the
#   model, its texture and shading, the effect), not the pose:
#     terrain-default (77%), desktop-blur-... (22%), desktop-effect-shadow-...
#     (22%), jellyfish-default (14%), refract-default (13%), the three buffer-*
#     (3 to 11%), the five conditionals-* and function-* (0.3 to 2.4%), the
#     three loop-* (0.2 to 1.8%), pulsar-light-false-quads-5-texture-false
#     (0.5%: its quads turn fast, so one frame's step shows)
#   animated, but the last pose is (almost) the same: the animation is a
#   function of the scene's clock, which stops at the duration, so the pose
#   moves by one frame's step at most (at most 0.18% of the pixels here; more
#   on a board whose frames are slower): compare the pose too, within the
#   picture value's tolerance, not pixel for pixel:
#     build-* (2), texture-* (3), shading-* (4), bump-* (3), shadow-default,
#     ideas-speed-duration
#   still, the same frame in both runs (crc32 equal):
#     effect2d-* (2)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
PATCH="$HERE/glmark2-host-ref.patch"
FB="$HERE/glmark2-fb.py"
PICTURES="$HERE/glmark2-pictures.txt"
COMMIT=22c527cb0556f3a1ac4445aaa52cc532760928d5 # tools/hostharness/glmark2/README.md
MESA_VERSION=26.2.1
MESA_UPSTREAM="$ROOT/third_party/mesa-$MESA_VERSION/UPSTREAM"

die() { echo "glmark2-host-ref.sh: $*" >&2; exit 2; }

# The default benchmark list of upstream's src/default-benchmarks.h at
# $COMMIT, in its order: the -b argument, then the scene's info string (the
# options sorted, as glmark2 prints them; the pi4 column of
# tools/cts/glmark2-pictures.txt has the same strings).
SCENES=(
  "build:use-vbo=false|[build] use-vbo=false"
  "build:use-vbo=true|[build] use-vbo=true"
  "texture:texture-filter=nearest|[texture] texture-filter=nearest"
  "texture:texture-filter=linear|[texture] texture-filter=linear"
  "texture:texture-filter=mipmap|[texture] texture-filter=mipmap"
  "shading:shading=gouraud|[shading] shading=gouraud"
  "shading:shading=blinn-phong-inf|[shading] shading=blinn-phong-inf"
  "shading:shading=phong|[shading] shading=phong"
  "shading:shading=cel|[shading] shading=cel"
  "bump:bump-render=high-poly|[bump] bump-render=high-poly"
  "bump:bump-render=normals|[bump] bump-render=normals"
  "bump:bump-render=height|[bump] bump-render=height"
  "effect2d:kernel=0,1,0;1,-4,1;0,1,0;|[effect2d] kernel=0,1,0;1,-4,1;0,1,0;"
  "effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;|[effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;"
  "pulsar:quads=5:texture=false:light=false|[pulsar] light=false:quads=5:texture=false"
  "desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true|[desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4"
  "desktop:windows=4:effect=shadow|[desktop] effect=shadow:windows=4"
  "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false|[buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map"
  "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false|[buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata"
  "buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true|[buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map"
  "ideas:speed=duration|[ideas] speed=duration"
  "jellyfish|[jellyfish] <default>"
  "terrain|[terrain] <default>"
  "shadow|[shadow] <default>"
  "refract|[refract] <default>"
  "conditionals:vertex-steps=0:fragment-steps=0|[conditionals] fragment-steps=0:vertex-steps=0"
  "conditionals:vertex-steps=0:fragment-steps=5|[conditionals] fragment-steps=5:vertex-steps=0"
  "conditionals:vertex-steps=5:fragment-steps=0|[conditionals] fragment-steps=0:vertex-steps=5"
  "function:fragment-steps=5:fragment-complexity=low|[function] fragment-complexity=low:fragment-steps=5"
  "function:fragment-steps=5:fragment-complexity=medium|[function] fragment-complexity=medium:fragment-steps=5"
  "loop:vertex-steps=5:fragment-steps=5:fragment-loop=false|[loop] fragment-loop=false:fragment-steps=5:vertex-steps=5"
  "loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false|[loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5"
  "loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true|[loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5"
)

# slug <scene info string>: the rule in the header.
slug() {
  printf '%s' "$1" | tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//'
}

layout() { "$HERE/deps-env.sh" --layout; }

# The glmark2 tree: GLMARK2_SRC, else the layout's glmark2-src (cloned at the
# pinned commit when absent and $1 is "fetch").
glmark2_src() {
  local src="${GLMARK2_SRC:-}"
  if [ -z "$src" ]; then
    src="$(layout)/glmark2-src"
    if [ ! -f "$src/src/default-benchmarks.h" ] && [ "${1-}" = fetch ]; then
      echo "glmark2-host-ref.sh: cloning glmark2 at $COMMIT into $src" >&2
      rm -rf "$src.part"
      git clone -q --no-checkout https://github.com/glmark2/glmark2 "$src.part"
      git -C "$src.part" checkout -q --detach "$COMMIT"
      mv "$src.part" "$src"
    fi
  fi
  [ -f "$src/src/default-benchmarks.h" ] ||
    die "no glmark2 tree at $src (set GLMARK2_SRC, or tools/cts/deps-env.sh --build GLMARK2_SRC)"
  src="$(cd "$src" && pwd -P)"
  # the pinned commit without local changes, or GitHub's archive of it
  if [ "$(git -C "$src" rev-parse --show-toplevel 2>/dev/null)" = "$src" ]; then
    local head dirty
    head=$(git -C "$src" rev-parse HEAD)
    dirty=$(git -C "$src" status --porcelain --untracked-files=no)
    [ "$head" = "$COMMIT" ] && [ -z "$dirty" ] ||
      die "$src is at $head (local changes: '$dirty'), not the pinned $COMMIT without local changes"
  else
    [ "$(basename "$src")" = "glmark2-$COMMIT" ] ||
      die "$src is neither a git checkout nor the archive directory glmark2-$COMMIT"
  fi
  printf '%s\n' "$src"
}

# The -b arguments of upstream's default list, one per line.
upstream_list() {
  sed -n 's/^ *benchmarks\.push_back("\(.*\)");$/\1/p' "$1/src/default-benchmarks.h"
}

# select_scenes <list>: the indices of SCENES the --scenes list selects.
select_scenes() {
  local want="$1" item i s found
  local -a items
  IFS=',' read -r -a items <<< "$want"
  local -A pick=()
  for item in "${items[@]}"; do
    [ -n "$item" ] || continue
    found=0
    for i in "${!SCENES[@]}"; do
      s=$(slug "${SCENES[$i]#*|}")
      if [ "$s" = "$item" ] || [ "${s#"$item"-}" != "$s" ]; then
        pick[$i]=1
        found=1
      fi
    done
    [ "$found" = 1 ] || die "--scenes: '$item' names no scene (see --list)"
  done
  for i in "${!SCENES[@]}"; do
    [ -z "${pick[$i]:-}" ] || echo "$i"
  done
}

self_test() {
  local ok=1 i s info src
  fail() { echo "FAIL $*"; ok=0; }
  [ "${#SCENES[@]}" = 33 ] || fail "the list has ${#SCENES[@]} scenes, not 33"
  # slugs: the rule on fixed cases, and one slug per scene
  local -A cases=(
    ["[build] use-vbo=false"]=build-use-vbo-false
    ["[effect2d] kernel=0,1,0;1,-4,1;0,1,0;"]=effect2d-kernel-0-1-0-1-4-1-0-1-0
    ["[jellyfish] <default>"]=jellyfish-default
    ["[buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map"]=buffer-columns-200-interleave-true-update-dispersion-0-9-update-fraction-0-5-update-method-map
    ["--Mixed  CASE__x--"]=mixed-case-x
  )
  for info in "${!cases[@]}"; do
    s=$(slug "$info")
    [ "$s" = "${cases[$info]}" ] || fail "slug '$info' is '$s', not '${cases[$info]}'"
  done
  local dup
  dup=$(for i in "${!SCENES[@]}"; do slug "${SCENES[$i]#*|}"; echo; done | sort | uniq -d)
  [ -z "$dup" ] || fail "slugs used twice: $dup"
  # the info strings: the pi4 column of the pictures table, in its order
  local want got
  want=$(sed -n 's/^pi4 | \(.*\) | .*$/\1/p' "$PICTURES")
  got=$(for i in "${!SCENES[@]}"; do echo "${SCENES[$i]#*|}"; done)
  [ "$want" = "$got" ] || fail "the info strings differ from the pi4 column of $PICTURES:
$(diff <(echo "$want") <(echo "$got") || true)"
  # the info string is the -b argument's options, sorted
  for i in "${!SCENES[@]}"; do
    local arg="${SCENES[$i]%%|*}" name opts
    name="${arg%%:*}"
    if [ "$name" = "$arg" ]; then opts="<default>"
    else opts=$(printf '%s' "${arg#*:}" | tr ':' '\n' | LC_ALL=C sort | paste -sd: -)
    fi
    [ "[$name] $opts" = "${SCENES[$i]#*|}" ] ||
      fail "entry $i: -b $arg gives '[$name] $opts', the list says '${SCENES[$i]#*|}'"
  done
  # --scenes
  [ "$(select_scenes texture | paste -sd' ' -)" = "2 3 4" ] || fail "--scenes texture"
  [ "$(select_scenes terrain,refract-default | paste -sd' ' -)" = "22 24" ] || fail "--scenes terrain,refract-default"
  [ "$(select_scenes loop-fragment-steps-5-fragment-uniform-true-vertex-steps-5 | paste -sd' ' -)" = 32 ] ||
    fail "--scenes <a full slug>"
  ( select_scenes nosuchscene ) > /dev/null 2>&1 && fail "--scenes nosuchscene was accepted"
  # upstream's list and the patch, against GLMARK2_SRC (no network)
  src=$(glmark2_src) || { echo "FAIL no GLMARK2_SRC"; return 1; }
  got=$(for i in "${!SCENES[@]}"; do echo "${SCENES[$i]%%|*}"; done)
  [ "$(upstream_list "$src")" = "$got" ] || fail "the -b arguments differ from $src/src/default-benchmarks.h"
  patch -p1 --dry-run -s -f -d "$src" < "$PATCH" > /dev/null || fail "$PATCH does not apply to $src"
  [ -f "$PATCH.license" ] || fail "no $PATCH.license"
  python3 "$FB" --self-test > /dev/null || fail "glmark2-fb.py --self-test"
  [ "$ok" = 1 ] && echo "self-test: ok" || echo "self-test: FAILED"
  [ "$ok" = 1 ]
}

# The Mesa prefix: GMREF_MESA_PREFIX, else the layout's build (made when absent).
mesa_prefix() {
  local prefix="${GMREF_MESA_PREFIX:-}"
  if [ -n "$prefix" ]; then
    printf '%s\n' "$prefix"
    return
  fi
  local deps
  deps="$(layout)"
  prefix="$deps/mesa-llvmpipe-$MESA_VERSION"
  if [ ! -f "$prefix/.complete" ]; then
    build_mesa "$deps" "$prefix" >&2
  fi
  printf '%s\n' "$prefix"
}

build_mesa() {
  local deps="$1" prefix="$2" sha tarball llvm="${LLVM_DIR:-/usr/lib/llvm-21}"
  sha=$(sed -n 's/^tarball-sha256: *//p' "$MESA_UPSTREAM")
  [ -n "$sha" ] || die "no tarball-sha256 in $MESA_UPSTREAM"
  tarball="$deps/downloads/mesa-$MESA_VERSION.tar.xz"
  if [ ! -f "$tarball" ]; then
    mkdir -p "$deps/downloads"
    curl -fsSL -o "$tarball.part" "https://archive.mesa3d.org/mesa-$MESA_VERSION.tar.xz"
    mv "$tarball.part" "$tarball"
  fi
  echo "$sha  $tarball" | sha256sum -c - > /dev/null || die "$tarball does not have the sha256 of $MESA_UPSTREAM"
  echo "glmark2-host-ref.sh: building Mesa $MESA_VERSION (llvmpipe, softpipe) into $prefix"
  rm -rf "$prefix"
  mkdir -p "$prefix"
  tar -C "$prefix" -xf "$tarball"
  [ -x "$llvm/bin/llvm-config" ] || die "no LLVM at $llvm (set LLVM_DIR)"
  PATH="$llvm/bin:$PATH" meson setup "$prefix/build" "$prefix/mesa-$MESA_VERSION" \
    --prefix "$prefix" --libdir lib -Dbuildtype=release \
    -Dgallium-drivers=llvmpipe,softpipe -Dvulkan-drivers= -Dplatforms= \
    -Degl=enabled -Dgles2=enabled -Dgles1=disabled -Dopengl=true -Dglx=disabled \
    -Dgbm=disabled -Dglvnd=disabled -Dllvm=enabled -Dshared-llvm=enabled \
    -Dvideo-codecs= -Dtools= -Dvalgrind=disabled -Dlibunwind=disabled \
    -Dlmsensors=disabled -Dzstd=disabled -Dbuild-tests=false
  PATH="$llvm/bin:$PATH" ninja -C "$prefix/build" -j "${GMREF_JOBS:-$(nproc)}" install
  touch "$prefix/.complete"
}

# The Mesa library directory under a prefix.
mesa_libdir() {
  local d
  for d in "$1/lib" "$1/lib/$(gcc -dumpmachine 2>/dev/null || echo x86_64-linux-gnu)" "$1/lib64"; do
    if [ -e "$d/libEGL.so.1" ] && [ -e "$d/libGLESv2.so.2" ]; then
      printf '%s\n' "$d"
      return
    fi
  done
  die "no libEGL.so.1 and libGLESv2.so.2 under $1"
}

# build_glmark2 <src> <build dir>: a patched copy of <src>, built.
build_glmark2() {
  local src="$1" b="$2"
  rm -rf "$b/glmark2" "$b/glmark2-build"
  mkdir -p "$b/glmark2"
  tar -C "$src" --exclude=./.git -cf - . | tar -C "$b/glmark2" -xf -
  patch -p1 -s -f -d "$b/glmark2" < "$PATCH"
  meson setup "$b/glmark2-build" "$b/glmark2" -Dbuildtype=release \
    -Dflavors=null-glesv2 -Ddata-path="$b/glmark2/data" > "$b/glmark2-meson.log" 2>&1 ||
    { tail -30 "$b/glmark2-meson.log" >&2; die "glmark2's meson setup failed ($b/glmark2-meson.log)"; }
  ninja -C "$b/glmark2-build" -j "${GMREF_JOBS:-$(nproc)}" src/glmark2-es2-null > "$b/glmark2-ninja.log" 2>&1 ||
    { tail -30 "$b/glmark2-ninja.log" >&2; die "glmark2's build failed ($b/glmark2-ninja.log)"; }
}

compare() {
  local a="$1" b="$2" i s ca cb same=() moved=()
  for i in "${!SCENES[@]}"; do
    s=$(slug "${SCENES[$i]#*|}")
    [ -f "$a/$s.log" ] && [ -f "$b/$s.log" ] || continue
    ca=$(grep -ao 'glmark2-fb: end crc32 [0-9a-f]*' "$a/$s.log" | tail -1 | awk '{print $4}')
    cb=$(grep -ao 'glmark2-fb: end crc32 [0-9a-f]*' "$b/$s.log" | tail -1 | awk '{print $4}')
    if [ -n "$ca" ] && [ "$ca" = "$cb" ]; then
      echo "same     $s ($ca)"
    else
      echo "differs  $s (${ca:-none} ${cb:-none})"
    fi
  done
}

OUT=""
WANT=""
DRIVER=llvmpipe
while [ $# -gt 0 ]; do
  case "$1" in
    --self-test) self_test; exit $? ;;
    --list)
      for i in "${!SCENES[@]}"; do printf '%s\t%s\n' "$(slug "${SCENES[$i]#*|}")" "${SCENES[$i]#*|}"; done
      exit 0 ;;
    --compare) [ $# -eq 3 ] || die "--compare DIR1 DIR2"; compare "$2" "$3"; exit 0 ;;
    --out) OUT="${2:?--out DIR}"; shift 2 ;;
    --scenes) WANT="${2:?--scenes LIST}"; shift 2 ;;
    --driver) DRIVER="${2:?--driver llvmpipe|softpipe}"; shift 2 ;;
    -h | --help) awk 'NR > 4 && /^#/ { print; next } NR > 4 { exit }' "$0"; exit 0 ;;
    *) die "unknown argument '$1' (--help)" ;;
  esac
done
case "$DRIVER" in llvmpipe | softpipe) ;; *) die "--driver llvmpipe or softpipe, not '$DRIVER'" ;; esac
if [ -z "$OUT" ]; then
  OUT="$ROOT/records/host/$(date +%Y%m%d-%H%M)-gmref-$(git -C "$ROOT" rev-parse --short=8 HEAD)"
fi
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
if [ -n "$WANT" ]; then
  mapfile -t PICK < <(select_scenes "$WANT")
else
  PICK=("${!SCENES[@]}")
fi

SRC=$(glmark2_src fetch)
[ "$(upstream_list "$SRC")" = "$(for i in "${!SCENES[@]}"; do echo "${SCENES[$i]%%|*}"; done)" ] ||
  die "the default list of $SRC differs from this script's"
PREFIX=$(mesa_prefix)
LIBDIR=$(mesa_libdir "$PREFIX")
build_glmark2 "$SRC" "$OUT/build"
GM="$OUT/build/glmark2-build/src/glmark2-es2-null"

run_env=(env LD_LIBRARY_PATH="$LIBDIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  LIBGL_DRIVERS_PATH="$LIBDIR/dri" EGL_PLATFORM=surfaceless LIBGL_ALWAYS_SOFTWARE=1
  GALLIUM_DRIVER="$DRIVER" MESA_SHADER_CACHE_DISABLE=true)
{
  echo "yakogl: $(git -C "$ROOT" rev-parse HEAD)"
  echo "glmark2: $COMMIT ($SRC) + tools/cts/glmark2-host-ref.patch sha256 $(sha256sum "$PATCH" | cut -d' ' -f1)"
  echo "mesa: $PREFIX ($LIBDIR)"
  echo "driver: $DRIVER"
  echo "host: $(uname -srm)"
} > "$OUT/env.txt"

: > "$OUT/summary.txt"
fails=0
for i in "${PICK[@]}"; do
  arg="${SCENES[$i]%%|*}"
  info="${SCENES[$i]#*|}"
  s=$(slug "$info")
  echo "== $s"
  rc=0
  "${run_env[@]}" "$GM" --off-screen -s 800x600 -b "$arg" > "$OUT/$s.log" 2>&1 || rc=$?
  [ "$rc" = 0 ] || echo "glmark2 exited $rc" >> "$OUT/$s.log"
  # a reference is the software renderer's, never a GPU's
  if ! grep -aq "GL_RENDERER: *$DRIVER" "$OUT/$s.log"; then
    echo "the renderer is not $DRIVER" >> "$OUT/$s.log"
    [ "$rc" != 0 ] || rc=99
  fi
  if [ ! -s "$OUT/env.txt.renderer" ]; then
    grep -a -m3 -E 'GL_(VENDOR|RENDERER|VERSION)' "$OUT/$s.log" > "$OUT/env.txt.renderer" || true
  fi
  frc=0
  verdict=$(python3 -I "$FB" "$OUT/$s.log" "$OUT/$s.png" --scene "$info" 2>&1) || frc=$?
  printf '%s\n' "$verdict" | sed 's/^/   /'
  crc=$(grep -ao 'glmark2-fb: end crc32 [0-9a-f]*' "$OUT/$s.log" | tail -1 | awk '{print $4}')
  if [ "$rc" = 0 ] && [ "$frc" = 0 ]; then
    echo "ok      $s crc32 ${crc:-none}" >> "$OUT/summary.txt"
  else
    echo "FAILED  $s (glmark2 rc $rc, glmark2-fb.py rc $frc) crc32 ${crc:-none}" >> "$OUT/summary.txt"
    fails=$((fails + 1))
  fi
done
cat "$OUT/env.txt.renderer" >> "$OUT/env.txt" 2>/dev/null || true
rm -f "$OUT/env.txt.renderer"
echo "${#PICK[@]} scene(s), $fails failed: $OUT/summary.txt"
[ "$fails" = 0 ]
