#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mesa-etna-build.sh - prepare the Mesa 26.2.1 etnaviv tree the Vivante host
# checks read as MESA_ETNA_TREE: etna_shc (tools/vivante/README.md, "Set up
# once") and the cs-golden drm-shim build ("The golden command stream",
# "Building it"). Issues #7, #1281.
#
#   tools/vivante/mesa-etna-build.sh [--tarball <mesa-26.2.1.tar.xz>] [<dir>]
#       make the tree under <dir> (default: the dependency layout's
#       mesa-etna-26.2.1, tools/cts/deps-env.sh --layout). A tree whose
#       marker is there and matches this checkout's inputs is left alone.
#   tools/vivante/mesa-etna-build.sh --dry-run [<dir>]
#       print the steps; change nothing.
#   tools/vivante/mesa-etna-build.sh --check [<dir>]
#       exit 0 (and print MESA_ETNA_TREE=<tree>) only on a complete tree;
#       else exit 2 with one `gap:` line on stderr.
#
# What <dir> holds:
#   downloads/            the Mesa tarball and five Python sdists, each
#                         checked against the sha256 pinned below
#   venv/                 a venv (no pip) with pycparser, mako, markupsafe,
#                         pyyaml and packaging copied in from those sdists:
#                         pure Python, no package index (pip is not usable on
#                         the farm's nodes)
#   mesa-26.2.1/          the tree, MESA_ETNA_TREE: etna_shc.c copied in,
#                         mesa-etna_shc.patch and mesa-cs-golden.patch applied
#     build-etnaviv/      etna_shc (gen_shader_table.py --st, glsl2vivante.py)
#     build/mesa-etnaviv-host/   libEGL, libGLESv2 and the etnaviv drm-shim
#                         (gate_st.sh golden's first build directory)
#   mesa-etna-build.done  the marker, written last: the sha256 of every input.
#                         An input that changed (a patch, etna_shc.c, this
#                         script's pins) makes the tree stale: --check says
#                         so, and a build starts it again from the tarball.
#
# Needs: python3 (>= 3.10, with venv), meson, ninja, a C/C++ compiler,
# pkg-config, bison, flex, patch, tar, xz, libdrm, expat, zlib and zstd
# development files; curl or python3 for the downloads (network once).
# The build directories hold absolute paths: the tree cannot be moved.
set -euo pipefail
export LC_ALL=C

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
SELF="$HERE/$(basename "${BASH_SOURCE[0]}")"
MESA_VER=26.2.1
MESA_URL="https://archive.mesa3d.org/mesa-$MESA_VER.tar.xz"
MESA_SHA=c47e81bddc4760360a41ac3c5acec38acb81f9d750ecef47e7f3adc7021a4442
# The Python build dependencies, sdists from PyPI (the digest is the one PyPI
# publishes for the file): <file> <sha256> <package directory in the sdist> <url>
PY_SDISTS=(
  "pycparser-2.23.tar.gz 78816d4f24add8f10a06d6f05b4d424ad9e96cfebf68a4ddc99c65c0720d00c2 pycparser https://files.pythonhosted.org/packages/fe/cf/d2d3b9f5699fb1e4615c8e32ff220203e43b248e1dfcc6736ad9057731ca/pycparser-2.23.tar.gz"
  "mako-1.3.10.tar.gz 99579a6f39583fa7e5630a28c3c1f440e4e97a414b80372649c0ce338da2ea28 mako https://files.pythonhosted.org/packages/9e/38/bd5b78a920a64d708fe6bc8e0a2c075e1389d53bef8413725c63ba041535/mako-1.3.10.tar.gz"
  "markupsafe-3.0.2.tar.gz ee55d3edf80167e48ea11a923c7386f4669df67d7994554387f84e7d8b0a2bf0 src/markupsafe https://files.pythonhosted.org/packages/b2/97/5d42485e71dfc078108a86d6de8fa46db44a1a9295e89c5d6d4a06e23a62/markupsafe-3.0.2.tar.gz"
  "pyyaml-6.0.2.tar.gz d584d9ec91ad65861cc08d42e834324ef890a082e591037abe114850ff7bbc3e lib/yaml https://files.pythonhosted.org/packages/54/ed/79a089b6be93607fa5cdaedf301d7dfb23af5f25c398d5ead2525b063e17/pyyaml-6.0.2.tar.gz"
  "packaging-25.0.tar.gz d443872c98d677bf60f6a1f2f8c1cb748e8fe762d2bf9d3148b5599295b0fc4f src/packaging https://files.pythonhosted.org/packages/a1/d4/1fc4078c65507b51b96ca8f8c3ba19e6a61c8253c72794544580a7b6c24d/packaging-25.0.tar.gz"
)
# The two configurations, as tools/vivante/README.md gives them.
OPTS_SHC=(-Dgallium-drivers=etnaviv -Dvulkan-drivers= -Dplatforms=
  -Dglx=disabled -Degl=disabled -Dgbm=disabled -Dllvm=disabled -Dtools=)
OPTS_GOLDEN=(-Dgallium-drivers=etnaviv -Dvulkan-drivers= -Dtools=drm-shim
  -Degl=enabled -Dgles1=disabled -Dgles2=enabled -Dglx=disabled
  -Dplatforms= -Dgbm=disabled -Dllvm=disabled -Dvideo-codecs=
  -Dshared-glapi=enabled -Dglvnd=disabled -Dbuildtype=debugoptimized)
SHC_REL=build-etnaviv/src/gallium/drivers/etnaviv/etna_shc
GOLDEN_REL=build/mesa-etnaviv-host

usage() {
  sed -n '10,18p' "$SELF" >&2
  exit 64
}

MODE=build TARBALL= DIR=
while [ $# -gt 0 ]; do
  case $1 in
  --dry-run) MODE=dry-run ;;
  --check) MODE=check ;;
  --tarball)
    [ $# -ge 2 ] || usage
    TARBALL=$2
    shift
    ;;
  -h | --help) usage ;;
  -*) usage ;;
  *)
    [ -z "$DIR" ] || usage
    DIR=$1
    ;;
  esac
  shift
done
if [ -z "$DIR" ]; then
  DIR="$("$ROOT/tools/cts/deps-env.sh" --layout)/mesa-etna-$MESA_VER"
fi
case $DIR in /*) ;; *) DIR="$PWD/$DIR" ;; esac
TREE="$DIR/mesa-$MESA_VER"
VENV="$DIR/venv"
DL="$DIR/downloads"
MARKER="$DIR/mesa-etna-build.done"

sha256() { sha256sum "$1" | cut -c1-64; }

# The marker's text: every input the tree is made from.
expected_marker() {
  echo "mesa-etna-build 1"
  echo "mesa $MESA_VER $MESA_SHA"
  local s f sha
  for s in "${PY_SDISTS[@]}"; do
    read -r f sha _ <<<"$s"
    echo "sdist $f $sha"
  done
  for f in etna_shc.c mesa-etna_shc.patch mesa-cs-golden.patch; do
    echo "input $f $(sha256 "$HERE/$f")"
  done
  echo "options build-etnaviv ${OPTS_SHC[*]}"
  echo "options $GOLDEN_REL ${OPTS_GOLDEN[*]}"
}

# Why the tree under $DIR is not complete, or nothing.
why_not_complete() {
  [ -d "$DIR" ] || { echo "no directory $DIR"; return; }
  [ -f "$MARKER" ] || { echo "no marker $MARKER (not built, or the build did not finish)"; return; }
  if [ "$(cat "$MARKER")" != "$(expected_marker)" ]; then
    echo "the tree under $DIR is stale: $MARKER does not match this checkout's inputs (rebuild with $0 $DIR)"
    return
  fi
  local f
  for f in src/etnaviv/hwdb/st/gc_feature_database.h src/etnaviv/hwdb/nxp/gc_feature_database.h \
    src/etnaviv/hw/state_3d.xml.h include/GLES2/gl2.h src/gallium/drivers/etnaviv/etna_shc.c; do
    [ -f "$TREE/$f" ] || { echo "the tree lacks $f"; return; }
  done
  [ -x "$TREE/$SHC_REL" ] || { echo "no etna_shc at $TREE/$SHC_REL"; return; }
  grep -q ETNA_SHIM_CS_DUMP "$TREE/src/etnaviv/drm-shim/etnaviv_noop.c" ||
    { echo "the tree is not patched with mesa-cs-golden.patch"; return; }
  for f in libetnaviv_noop_drm_shim.so libEGL.so libGLESv2.so; do
    [ -n "$(find "$TREE/$GOLDEN_REL" -name "$f" -print -quit 2>/dev/null)" ] ||
      { echo "no $f under $TREE/$GOLDEN_REL"; return; }
  done
}

if [ "$MODE" = check ]; then
  why=$(why_not_complete)
  if [ -n "$why" ]; then
    echo "gap: MESA_ETNA_TREE: $why" >&2
    exit 2
  fi
  echo "MESA_ETNA_TREE=$TREE"
  exit 0
fi

# step <text>: print a step; in a dry run that is all.
step() { echo "== $*"; }
run() {
  if [ "$MODE" = dry-run ]; then
    printf '   '
    printf '%q ' "$@"
    echo
  else
    "$@"
  fi
}

# fetch <file> <sha256> <url>: into $DL, checked.
fetch() {
  local f=$1 want=$2 url=$3 dest="$DL/$1"
  if [ -f "$dest" ] && [ "$(sha256 "$dest")" = "$want" ]; then
    step "have $dest (sha256 ok)"
    return
  fi
  step "fetch $url -> $dest (sha256 $want)"
  [ "$MODE" = dry-run ] && return
  mkdir -p "$DL"
  rm -f "$dest.part"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 -o "$dest.part" "$url"
  else
    python3 -I -c 'import shutil, sys, urllib.request
with urllib.request.urlopen(sys.argv[1]) as r, open(sys.argv[2], "wb") as f:
    shutil.copyfileobj(r, f)' "$url" "$dest.part"
  fi
  local got
  got=$(sha256 "$dest.part")
  if [ "$got" != "$want" ]; then
    rm -f "$dest.part"
    echo "mesa-etna-build: $url: sha256 $got, expected $want" >&2
    exit 1
  fi
  mv "$dest.part" "$dest"
}

echo "mesa-etna-build: $MODE, directory $DIR"
why=$(why_not_complete)
if [ -z "$why" ]; then
  step "complete: $MARKER matches this checkout's inputs; nothing to do"
  echo "MESA_ETNA_TREE=$TREE"
  exit 0
fi
step "to do: $why"

if [ "$MODE" = build ]; then
  for c in python3 meson ninja cc c++ pkg-config bison flex patch tar xz sha256sum; do
    command -v "$c" >/dev/null 2>&1 || { echo "mesa-etna-build: $c is not on PATH" >&2; exit 1; }
  done
  mkdir -p "$DIR"
  # One build at a time per directory.
  if command -v flock >/dev/null 2>&1; then
    exec 9>"$DIR/.lock"
    flock -n 9 || { echo "mesa-etna-build: another build holds $DIR/.lock" >&2; exit 1; }
  fi
  # The marker goes before anything else does: a build that stops halfway
  # leaves no marker, and the next one starts again.
  rm -f "$MARKER"
fi

# 1. The inputs.
if [ -n "$TARBALL" ]; then
  [ -f "$TARBALL" ] || { echo "mesa-etna-build: no tarball $TARBALL" >&2; exit 1; }
  step "use $TARBALL (sha256 must be $MESA_SHA)"
  if [ "$MODE" = build ] && [ "$(sha256 "$TARBALL")" != "$MESA_SHA" ]; then
    echo "mesa-etna-build: $TARBALL: sha256 $(sha256 "$TARBALL"), expected $MESA_SHA" >&2
    exit 1
  fi
  MESA_TARBALL=$TARBALL
else
  fetch "mesa-$MESA_VER.tar.xz" "$MESA_SHA" "$MESA_URL"
  MESA_TARBALL="$DL/mesa-$MESA_VER.tar.xz"
fi
for s in "${PY_SDISTS[@]}"; do
  read -r f sha _ url <<<"$s"
  fetch "$f" "$sha" "$url"
done

# 2. The venv: the packages' pure-Python directories copied in, no pip.
step "venv $VENV: pycparser, mako, markupsafe, pyyaml, packaging from the sdists"
run rm -rf "$VENV.part" "$DIR/sdists"
run python3 -m venv --without-pip "$VENV.part"
run mkdir -p "$DIR/sdists"
if [ "$MODE" = build ]; then
  site=$("$VENV.part/bin/python3" -I -c 'import sysconfig; print(sysconfig.get_paths()["purelib"])')
fi
for s in "${PY_SDISTS[@]}"; do
  read -r f _ pkg _ <<<"$s"
  run tar -C "$DIR/sdists" -xzf "$DL/$f"
  if [ "$MODE" = build ]; then
    cp -R "$DIR/sdists/${f%.tar.gz}/$pkg" "$site/"
  else
    echo "   cp -R $DIR/sdists/${f%.tar.gz}/$pkg <venv site-packages>/"
  fi
done
run "$VENV.part/bin/python3" -I -c 'import pycparser, mako.template, markupsafe, yaml, packaging.version
assert packaging.version.Version(pycparser.__version__) >= packaging.version.Version("2.20")
print("venv: pycparser", pycparser.__version__, "mako", mako.__version__, "yaml", yaml.__version__)'
run rm -rf "$VENV" "$DIR/sdists"
run mv "$VENV.part" "$VENV"

# 3. The tree.
step "tree $TREE: extract, copy etna_shc.c, apply the two patches"
run rm -rf "$TREE"
run tar -C "$DIR" -xJf "$MESA_TARBALL"
run cp "$HERE/etna_shc.c" "$TREE/src/gallium/drivers/etnaviv/etna_shc.c"
run patch -d "$TREE" -p1 --batch --forward -i "$HERE/mesa-etna_shc.patch"
run patch -d "$TREE" -p1 --batch --forward -i "$HERE/mesa-cs-golden.patch"

# 4. The builds. The venv's python3 comes first on PATH, so Mesa's meson
# (find_program python3.x / python3) runs its generators with it.
export PATH="$VENV/bin:$PATH"
step "build-etnaviv: etna_shc (README.md, \"Set up once\")"
run meson setup "$TREE/build-etnaviv" "$TREE" "${OPTS_SHC[@]}"
run ninja -C "$TREE/build-etnaviv" src/gallium/drivers/etnaviv/etna_shc
step "$GOLDEN_REL: libEGL, libGLESv2 and the drm-shim (README.md, \"Building it\")"
run meson setup "$TREE/$GOLDEN_REL" "$TREE" "${OPTS_GOLDEN[@]}"
run ninja -C "$TREE/$GOLDEN_REL"

# 5. The marker, last.
step "marker $MARKER"
if [ "$MODE" = dry-run ]; then
  echo "   (the sha256 of the tarball, the sdists, etna_shc.c and the two patches, and the options)"
  exit 0
fi
expected_marker >"$MARKER.part"
mv "$MARKER.part" "$MARKER"
why=$(why_not_complete)
if [ -n "$why" ]; then
  echo "mesa-etna-build: the build finished but the tree is not complete: $why" >&2
  rm -f "$MARKER"
  exit 1
fi
echo "MESA_ETNA_TREE=$TREE"
