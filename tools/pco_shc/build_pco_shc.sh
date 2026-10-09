#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# build_pco_shc.sh - build the pco_shc that tools/glsl2usc.sh needs (Mesa
# 26.2.1's PowerVR compiler with tools/pco_shc/pco_shc.c and a freshly
# compiled usclib carrying usclib-tex.cl.patch), from the release tarball.
#
# Usage:
#   tools/pco_shc/build_pco_shc.sh <mesa-26.2.1.tar.xz> [<work dir>]
# The tarball is extracted into <work dir>/mesa-26.2.1 (default: the directory
# holding the tarball), which is deleted and recreated first; the build dir is
# <work dir>/mesa-26.2.1/build.
#
# Needs (generic name -> Gentoo package):
#   clang + LLVM (built against the same version) -> llvm-core/clang, llvm-core/llvm
#   SPIRV-LLVM-Translator     -> dev-util/spirv-llvm-translator
#   SPIRV-Tools (+ spirv-dis) -> dev-util/spirv-tools, dev-util/spirv-headers
#   glslang (glslangValidator, used by glsl2usc, not by this build) -> dev-util/glslang
#   meson, ninja              -> dev-build/meson, dev-build/ninja
#   python mako, pyyaml, packaging -> dev-python/mako, dev-python/pyyaml, dev-python/packaging
#   libdrm, expat, zlib, zstd -> x11-libs/libdrm, dev-libs/expat, sys-libs/zlib, app-arch/zstd
# Last built (2026-10-07, Gentoo, gcc 16.2.1_p20260926) with LLVM/clang 23.1.2,
# SPIRV-LLVM-Translator 23.1.1, SPIRV-Tools 1.4.357.0 (no libclc: this Mesa
# config builds mesa_clc without it): pco_usclib.spv md5
# 15eca99efcf1f20d9124abf5cdffd4ab -- the same as the 2026-10-05 build --,
# pco_shc id 01aa62a069bb on that host; the same versions on the landing host
# (2026-10-10, gcc 16.2.1_p20260926 p1) give f537fc86b41f, the id
# tests/reference/blobs.rs records, with the 2026-10-05 build's bytes. Every
# USC row of the table is byte-identical between the two ids: the id names
# the binary, and two hosts' builds of one source differ in it.
# LLVM_DIR (default /usr/lib/llvm/23) selects the LLVM.
#
# Meson options: -Dbuildtype=debug -Dvulkan-drivers=imagination
# -Dgallium-drivers= -Dgles1=disabled -Dgles2=disabled -Dglx=disabled
# -Degl=disabled -Dplatforms= and -ffile-prefix-map=<tree>=/mesa in c_args and
# cpp_args, with mesa-clc and precomp-compiler left at their default (built
# here): mesa_clc is what compiles the usclib, and a "system" mesa_clc would
# reuse whatever usclib that host built. The prefix map is all it takes for
# two work dirs to give the same pco_shc bytes (no build-id is emitted); the
# id can still depend on the compiler and library versions: a host with
# another gcc or LLVM may get a different id for the same output.
set -euo pipefail

[ $# -ge 1 ] && [ $# -le 2 ] || { sed -n '9,13p' "$0" >&2; exit 2; }
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
TARBALL="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
WORK="$(mkdir -p "${2:-$(dirname "$TARBALL")}" && cd "${2:-$(dirname "$TARBALL")}" && pwd)"
LLVM_DIR="${LLVM_DIR:-/usr/lib/llvm/23}"
export PATH="$LLVM_DIR/bin:$PATH"
export PKG_CONFIG_PATH="$LLVM_DIR/lib64/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"

UPSTREAM="$ROOT/third_party/mesa-26.2.1/UPSTREAM"
SHA=$(sed -n 's/^tarball-sha256: *//p' "$UPSTREAM")
[ -n "$SHA" ] || { echo "no tarball-sha256 in $UPSTREAM" >&2; exit 1; }
echo "$SHA  $TARBALL" | sha256sum -c -

SRC="$WORK/mesa-26.2.1"
rm -rf "$SRC"
tar -C "$WORK" -xf "$TARBALL"
cd "$SRC"
cp "$HERE"/{pco_shc.c,capture_usc.c,capture_smp_fs.c} src/imagination/vulkan/
patch -p1 < "$HERE/meson-vulkan.patch"
patch -p1 < "$HERE/usclib-tex.cl.patch"
# The tree's absolute path must not reach the binary (debug info, __FILE__),
# or the content id glsl2usc records would depend on where it was built.
PREFIX_MAP="-ffile-prefix-map=$SRC=/mesa"
meson setup build \
  -Dbuildtype=debug -Dvulkan-drivers=imagination -Dgallium-drivers= \
  -Dgles1=disabled -Dgles2=disabled -Dglx=disabled -Degl=disabled -Dplatforms= \
  -Dc_args="$PREFIX_MAP" -Dcpp_args="$PREFIX_MAP"
ninja -C build src/imagination/pco/usclib/pco_usclib.spv src/imagination/vulkan/pco_shc

# The usclib must be the patched one: usclib_tex_state_base_level compares
# textype against ROGUE_TEXSTATE_TEXTYPE_STRIDE (4) and branches on it. An
# unpatched (or stale) usclib has neither instruction in that function.
SPV=build/src/imagination/pco/usclib/pco_usclib.spv
spirv-dis "$SPV" -o build/pco_usclib.spvasm
FN=$(awk '/%usclib_tex_state_base_level = OpFunction /{f=1} f; f && /OpFunctionEnd/{f=0}' build/pco_usclib.spvasm)
if ! grep -q 'OpIEqual %bool .* %uint_4' <<<"$FN" || ! grep -q OpBranchConditional <<<"$FN"; then
  echo "error: $SPV is not the patched usclib (usclib_tex_state_base_level has no STRIDE test)" >&2
  exit 1
fi
echo "usclib: patched (usclib_tex_state_base_level tests textype == STRIDE)"

PCO_SHC="$SRC/build/src/imagination/vulkan/pco_shc"
echo "PCO_SHC=$PCO_SHC"
# glsl2usc's content id: the first 12 hex digits of the binary's SHA-256.
echo "toolchain id: pco_shc=$(sha256sum "$PCO_SHC" | cut -c1-12)"
