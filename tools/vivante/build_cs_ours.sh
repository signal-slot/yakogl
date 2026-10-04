#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# build_cs_ours.sh - build tools/vivante/cs_ours.c against this library.
#
# In the C tree cs_ours.c was built by compiling the library's .c files (the
# driver repository's build/run_cs_ours.sh does it through CMake). Here the
# library is Rust: the archive comes out of cargo as the `yakogl-capi`
# package's staticlib, and the C parts that are still C -- the Linux OSAL, the
# layout guard, the panic personality shim -- are compiled and appended with
# `ar`. That is exactly what tools/hostharness/run.sh does for its own C
# harnesses (see its README, "Library specifications"), so this follows it
# rather than inventing a second way.
#
#   tools/vivante/build_cs_ours.sh [driver tree] [out dir]
#
#     [driver tree]  default: this repository's driver/ (the driver tree was
#                    merged in as driver/ on 2026-09-22; before that it was a
#                    separate powervr-bxs-am62p checkout). Two things come from it:
#                    tools/pvrlab/gl_test_shaders.h (cs_ours.c draws with
#                    gl_test's own shader sources, so the offline table's
#                    SHA-256 lookup finds the shaders the board runs) and
#                    src/pvr_render.h (src/osal/linux.c includes it).
#     [out dir]      default build/cs-ours
#
# Then run it as run_cs_ours.sh does -- nothing here needs a GPU:
#
#     env YAKOGL_CS_DUMP=$OUT/case4.cs YAKOGL_MEM_DUMP=$OUT/case4.mem \
#         $OUT/cs_ours 4
#     python3 tools/vivante/cs_decode.py $OUT/case4.cs --mem $OUT/case4.mem
#
# Environment: CARGO_PROFILE=release|dev (default release, as the board build),
# CC (default cc), SHADERC_ONLINE=1 to build the in-process compiler in (the
# offline table has no row for a shader outside it, e.g. mode 25's gl_test 108
# D; left off, every mode keeps compiling through the table as the board's
# offline image does). The archive has no `std`, so YAKOGL_SHADERC cannot pick
# the table back at run time: an online build runs mode 25 (and the ES 1.00
# shapes), a default build every other mode.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
CC="${CC:-cc}"

die() { echo "build_cs_ours.sh: $*" >&2; exit 1; }

DRV="$(cd "${1:-$ROOT/driver}" && pwd)" || die "no such driver tree: ${1:-$ROOT/driver}"
OUT="${2:-$ROOT/build/cs-ours}"
PROFILE="${CARGO_PROFILE:-release}"

[ -f "$DRV/tools/pvrlab/gl_test_shaders.h" ] || \
  die "$DRV has no tools/pvrlab/gl_test_shaders.h (cs_ours.c uses gl_test's shader sources)"
[ -f "$DRV/src/pvr_render.h" ] || die "$DRV has no src/pvr_render.h (src/osal/linux.c includes it)"
[ -f "$ROOT/src/vivante/gc_kick.h" ] || \
  die "src/vivante/gc_kick.h is missing: the OSAL needs the C view of the kick descriptor
     (src/vivante/kick.rs says so). It comes with the Vivante backend."

mkdir -p "$OUT/obj"

# 1. the Rust library. `gpu-vivante` is the archive crate's own feature, which
#    forwards to the library crate's (capi/Cargo.toml).
pargs=(--release); pdir=release
[ "$PROFILE" = release ] || { pargs=(--profile dev-lto); pdir=dev-lto; }
echo "build_cs_ours.sh: cargo ($PROFILE)"
cargo build --manifest-path "$ROOT/Cargo.toml" --target-dir "$OUT/cargo" \
  -p yakogl-capi --no-default-features \
  --features "osal-linux,gpu-vivante${SHADERC_ONLINE:+,shaderc-online}" "${pargs[@]}"
cp "$OUT/cargo/$pdir/libyakogl_capi.a" "$OUT/libyakogl.a"

# 2. the C parts of the library, appended to the archive. src/pvr/ffi_layout_check.c
#    is deliberately absent: it pins src/pvr/ffi.rs, and the PowerVR backend is
#    not in a gpu-vivante build. The archive is not post-processed here
#    (capi/assemble.cmake renames the personality), so the shim is told the
#    name the crate actually exports -- as run.sh does.
cflags=(-g -O1 -std=gnu11 -Wall -DYAKOGL_OSAL_LINUX -DYAKOGL_GPU_VIVANTE
        -DYAKOGL_EH_PERSONALITY=rust_eh_personality)
cinc=(-I"$ROOT/include" -I"$ROOT/src" -I"$ROOT/src/osal" -I"$ROOT/src/pvr"
      -I"$ROOT/src/vivante" -I"$DRV/src")
objs=()
for f in "$ROOT/src/osal/linux.c" "$ROOT/src/abi_layout_check.c" "$ROOT/capi/eh_personality.c"; do
  [ -f "$f" ] || continue
  o="$OUT/obj/$(basename "${f%.c}").o"
  "$CC" "${cflags[@]}" "${cinc[@]}" -c "$f" -o "$o" \
    || die "compiling $f failed (a layout guard failing means the crate and the headers have drifted)"
  objs+=("$o")
done
ar qs "$OUT/libyakogl.a" "${objs[@]}"

# 3. cs_ours itself
"$CC" "${cflags[@]}" "${cinc[@]}" -I"$DRV/tools/pvrlab" \
  -o "$OUT/cs_ours" "$HERE/cs_ours.c" "$OUT/libyakogl.a" -lm \
  || die "building cs_ours.c failed"

echo "build_cs_ours.sh: $OUT/cs_ours"
