#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Host harness: build one library under test with sanitizers, build the
# harness programs against it, and run every call stream in every OSAL mode.
# See README.md for what is recorded and compared.
#
# usage: run.sh build <out> <lib> <driver repo> [dev|release]
#        run.sh run   <out> [job-regex]
#        run.sh all   <out> <lib> <driver repo> [dev|release] [job-regex]
#
#   <lib>  rust:<crate dir>          the Rust crate (cargo; nightly + ASan)
#          csrc:<yakogl C tree>      a C yakogl tree (every .c compiled here)
#          prebuilt:<libyakogl.a>:<yakogl tree for the headers>
#   <driver repo>  powervr-bxs-am62p checkout (src/ = generators + headers,
#          tools/pvrlab/gl_test.c = the on-silicon test suite)
#
# environment:
#   HH_SAN=1        sanitizers (default): Rust -Zsanitizer=address (nightly),
#                   C -fsanitize=address,undefined; HH_SAN=0 builds without
#   HH_SHADERS      shader source tree for scen/hostile (default: <lib tree>/tools/shaders,
#                   else this tree's tools/shaders) - use the same one for both sides
#   HH_MODES        OSAL modes to run (default: all, see MODES below)
#   HH_JOBS         parallel runs (default: nproc)
#   HH_TIMEOUT      seconds per run (default 1200)
#   HH_SIZE=small   shorter sweeps (development)
#   CARGO_NIGHTLY   toolchain for the sanitizer build (default: nightly)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"

die() { echo "run.sh: $*" >&2; exit 1; }

# mode name -> recording-OSAL environment (rec.c)
declare -A MODES=(
  [l0]="HH_LAG=0"
  [l2]="HH_LAG=2"
  [l0nc]="HH_NOCPU=1"
  [l2nc]="HH_LAG=2 HH_NOCPU=1"
  [l0mp]="HH_MAPPHYS=1"
  [l0mpc]="HH_MAPPHYS=2"
  [z0]="HH_NHASH=1"
  [a0]="HH_NHASH=1 HH_FILL=0xa5"
  [z2]="HH_LAG=2 HH_NHASH=1"
  [a2]="HH_LAG=2 HH_NHASH=1 HH_FILL=0xa5"
  [base]="HH_BASE=1"
  [l3wf]="HH_LAG=3 HH_WAIT_FAIL=3"
  [ncall]="HH_NOCPU=2"
)
ALL_MODES="l0 l2 l0nc l2nc l0mp l0mpc z0 a0 z2 a2 base l3wf"

build() {
  local out="$1" lib="$2" drv="$3" profile="${4:-dev}"
  local san="${HH_SAN:-1}" src kind csan="" rsan="" opt
  mkdir -p "$out"/{obj,lib,gen,bin,logs}
  out="$(cd "$out" && pwd)"
  drv="$(cd "$drv" && pwd)"
  [ -f "$drv/src/pvr_render.h" ] || die "$drv is not a driver repository (no src/pvr_render.h)"
  [ -f "$drv/tools/pvrlab/gl_test.c" ] || die "$drv has no tools/pvrlab/gl_test.c"
  kind="${lib%%:*}"
  case "$kind" in
    rust|csrc) src="$(cd "${lib#*:}" && pwd)" ;;
    prebuilt) local rest="${lib#prebuilt:}"; src="$(cd "${rest#*:}" && pwd)"; lib="${rest%%:*}" ;;
    *) die "unknown library kind '$kind'" ;;
  esac
  [ "$san" = 1 ] && csan="-fsanitize=address,undefined -fno-omit-frame-pointer"
  opt="-O1"; [ "$profile" = release ] && opt="-O2"
  local cinc=(-I"$src/include" -I"$src/src" -I"$src/src/osal" -I"$src/src/pvr" -I"$drv/src")
  # A driver whose generators call Mesa's PDS generators and csbgen packers
  # (driver/compat/pvr_mesa.h): their include directories and the file that
  # compiles the generators (driver/compat/pvr_mesa.cmake lists the same).
  local mesa_parts=()
  if [ -f "$drv/compat/pvr_mesa.h" ]; then
    local m="$drv/../third_party/mesa-26.2.1"
    cinc+=(-I"$drv/compat" -I"$drv/src/pvr/generated" -I"$m/include" -I"$m/src" -I"$m/src/imagination"
           -I"$m/src/imagination/include" -I"$m/src/imagination/common" -I"$m/src/imagination/vulkan"
           -I"$m/src/imagination/vulkan/pds" -I"$m/src/imagination/vulkan/pds/pvr_pds_programs")
    mesa_parts=("$drv/compat/pvr_mesa_pds.c")
  fi
  local cflags=(-g "$opt" -std=gnu11 $csan -DYAKOGL_OSAL_LINUX)

  {
    echo "lib=$kind:$src"; echo "driver=$drv"; echo "profile=$profile"; echo "san=$san"
    git -C "$src" log -1 --format='lib-commit=%H' 2>/dev/null || true
    git -C "$src" status --porcelain 2>/dev/null | sed 's/^/lib-dirty: /' | head -20 || true
    git -C "$drv" log -1 --format='driver-commit=%H' 2>/dev/null || true
  } > "$out/build-info.txt"

  rm -f "$out/lib/libyakogl.a"
  case "$kind" in
    rust)
      local tc="" rflags="" tgt=() pdir=debug
      [ "$profile" = release ] && pdir=release
      if [ "$san" = 1 ]; then
        tc="+${CARGO_NIGHTLY:-nightly}"
        rustc "$tc" -Z help >/dev/null 2>&1 || die "no nightly toolchain: -Zsanitizer=address needs one (HH_SAN=0 builds without)"
        rflags="-Zsanitizer=address"
        tgt=(--target x86_64-unknown-linux-gnu)
        pdir="x86_64-unknown-linux-gnu/$pdir"
      fi
      local pargs=() pkg=() staticlib=libyakogl.a features=osal-linux,panic-handler
      [ "$profile" = release ] && pargs=(--release)
      if [ -f "$src/capi/Cargo.toml" ]; then
        # workspace layout: the archive is the yakogl-capi package; its dev
        # build is the dev-lto profile (dev semantics, one object)
        pkg=(-p yakogl-capi)
        staticlib=libyakogl_capi.a
        features=osal-linux
        # a tree with the explicit PowerVR feature (the others build it implicitly)
        grep -q '^gpu-powervr' "$src/Cargo.toml" && features=osal-linux,gpu-powervr
        if [ "$profile" != release ]; then
          pargs=(--profile dev-lto)
          pdir="${pdir%debug}dev-lto"
        fi
      fi
      RUSTFLAGS="$rflags" cargo $tc build --manifest-path "$src/Cargo.toml" --target-dir "$out/cargo" "${pkg[@]}" \
        --no-default-features --features "$features" "${pargs[@]}" "${tgt[@]}" 2> "$out/cargo.log" \
        || { tail -20 "$out/cargo.log"; die "cargo build failed"; }
      cp "$out/cargo/$pdir/$staticlib" "$out/lib/libyakogl.a"
      local cparts=("$src/src/osal/linux.c" "$drv/src/pvr_vdm_ppp_gen.c" "$drv/src/pvr_pds_render_gen.c"
                    "$drv/src/pvr_pds_pixel_event_gen.c" "${mesa_parts[@]}")
      # the archive is not post-processed here (capi/assemble.cmake renames the
      # personality); the harness links no other Rust code
      [ -f "$src/capi/eh_personality.c" ] && cparts+=("$src/capi/eh_personality.c")
      cflags+=(-DYAKOGL_EH_PERSONALITY=rust_eh_personality)
      for f in "$src/src/abi_layout_check.c" "$src/src/pvr/ffi_layout_check.c"; do
        [ -f "$f" ] && cparts+=("$f")
      done
      local objs=()
      for f in "${cparts[@]}"; do
        local o="$out/obj/lib-$(basename "${f%.c}").o"
        gcc "${cflags[@]}" "${cinc[@]}" -c "$f" -o "$o" 2>> "$out/cc.log" \
          || { tail -30 "$out/cc.log"; die "compiling $f failed (a layout guard failing means the crate does not match this driver)"; }
        objs+=("$o")
      done
      ar qs "$out/lib/libyakogl.a" "${objs[@]}"
      ;;
    csrc)
      local objs=() files=("$src"/src/gl_*.c "$src"/src/pvr/*.c "$src"/src/shaderc/*.c "$src/src/osal/osal.c"
                           "$src/src/osal/linux.c" "$drv/src/pvr_vdm_ppp_gen.c" "$drv/src/pvr_pds_render_gen.c"
                           "$drv/src/pvr_pds_pixel_event_gen.c" "${mesa_parts[@]}")
      for f in "${files[@]}"; do
        [ -f "$f" ] || continue
        local o="$out/obj/lib-$(basename "${f%.c}").o"
        gcc "${cflags[@]}" "${cinc[@]}" -c "$f" -o "$o" 2>> "$out/cc.log" || { tail -30 "$out/cc.log"; die "compiling $f failed"; }
        objs+=("$o")
      done
      ar rcs "$out/lib/libyakogl.a" "${objs[@]}"
      ;;
    prebuilt)
      [ -f "$lib" ] || die "no archive $lib"
      cp "$lib" "$out/lib/libyakogl.a"
      ;;
  esac

  python3 "$HERE/gen_wrap.py" "$out/lib/libyakogl.a" "$src/include" "$out/gen" 2> "$out/gen/gen.log" \
    || { cat "$out/gen/gen.log"; die "gen_wrap failed"; }
  cat "$out/gen/gen.log"

  # harness objects
  local hflags=(-g -O1 -std=gnu11 $csan -DPVR_HEAP_GENERAL=GLES_HEAP_DATA -I"$HERE" -I"$HERE/shim"
                "${cinc[@]}" -I"$drv/tools/pvrlab")
  grep -q isp_scissor_base "$drv/src/pvr_render.h" && hflags+=(-DHH_KICK_SCISSOR)
  : > "$out/hcc.log"
  gcc "${hflags[@]}" -Wall -c "$HERE/rec.c" -o "$out/obj/rec.o" 2>> "$out/hcc.log"
  gcc "${hflags[@]}" -Wall -c "$HERE/wrap_rt.c" -o "$out/obj/wrap_rt.o" 2>> "$out/hcc.log"
  gcc "${hflags[@]}" -w -c "$out/gen/wrap_gen.c" -o "$out/obj/wrap_gen.o" 2>> "$out/hcc.log"
  gcc -c "$out/gen/wrap_asm.S" -o "$out/obj/wrap_asm.o"
  python3 - "$drv/tools/pvrlab/gl_test.c" "$out/gen/gl_test_host.c" <<'PY'
import sys
s = open(sys.argv[1]).read()
# no GPU: the pixel checks cannot pass; the harness compares the read-backs
# themselves (glReadPixels is interposed), so expect_px only reports
i = s.index("static bool expect_px(")
j = s.index("return ok;", i)
open(sys.argv[2], "w").write(s[:j] + "(void)ok; return true;" + s[j + len("return ok;"):])
PY
  gcc "${hflags[@]}" -w -c "$out/gen/gl_test_host.c" -o "$out/obj/gl_test_host.o" 2>> "$out/hcc.log" \
    || { tail -30 "$out/hcc.log"; die "gl_test.c does not build against this library's headers"; }
  local p
  for p in gltest scen hostile; do
    gcc "${hflags[@]}" -Wall -c "$HERE/$p.c" -o "$out/obj/$p.o" 2>> "$out/hcc.log" \
      || { tail -30 "$out/hcc.log"; die "$p.c failed"; }
  done
  # route the programs' gl*/egl* calls through the interposer
  for o in gltest gl_test_host scen hostile; do
    objcopy --redefine-syms="$out/gen/redefine.txt" "$out/obj/$o.o" "$out/obj/$o.w.o"
  done
  local common=("$out/obj/rec.o" "$out/obj/wrap_rt.o" "$out/obj/wrap_gen.o" "$out/obj/wrap_asm.o"
                "$out/lib/libyakogl.a" -lm -lpthread -ldl)
  gcc $csan -o "$out/bin/gltest" "$out/obj/gltest.w.o" "$out/obj/gl_test_host.w.o" "${common[@]}" 2>> "$out/hcc.log" \
    || { tail -30 "$out/hcc.log"; die "link gltest failed"; }
  gcc $csan -o "$out/bin/scen" "$out/obj/scen.w.o" "${common[@]}" 2>> "$out/hcc.log" \
    || { tail -30 "$out/hcc.log"; die "link scen failed"; }
  gcc $csan -o "$out/bin/hostile" "$out/obj/hostile.w.o" "${common[@]}" 2>> "$out/hcc.log" \
    || { tail -30 "$out/hcc.log"; die "link hostile failed"; }
  if grep -q 'warning' "$out/hcc.log"; then grep warning "$out/hcc.log" | head; fi

  # the job list: name|extra environment|program args
  local shaders="${HH_SHADERS:-}"
  if [ -z "$shaders" ]; then
    if [ -d "$src/tools/shaders" ]; then shaders="$src/tools/shaders"; else shaders="$ROOT/tools/shaders"; fi
  fi
  echo "shaders=$shaders" >> "$out/build-info.txt"
  local small=0; [ "${HH_SIZE:-}" = small ] && small=1
  {
    echo "gltest.all||gltest 7 0"
    echo "gltest.all-dsdump||gltest 7 2"
    echo "gltest.all-pretransform|HH_PRETRANSFORM=1 HH_NOCOALESCE=1|gltest 7 0"
    for c in $(sed -n '/^static int run_case(/,/^}/p' "$out/gen/gl_test_host.c" | grep -o 'case [0-9]*:' | grep -o '[0-9]*' | sort -n); do
      echo "gltest.$c||gltest $c 1"
    done
    local n1=400 n2=300 n3=3000 n4=400
    [ $small = 1 ] && { n1=60; n2=60; n3=300; n4=60; }
    echo "scen.state||scen state $n1 0"
    echo "scen.state-special||scen state $n1 1"
    echo "scen.state-special-inrange|HH_INRANGE=1|scen state $n1 1"
    echo "scen.state-pretransform|HH_PRETRANSFORM=1 HH_NOCOALESCE=1 HH_DEBUG=1|scen state 80 0"
    echo "scen.gets||scen gets"
    echo "scen.egl||scen egl"
    echo "scen.progs-top||scen progs . 1 0"
    echo "scen.progs-es100||scen progs es100 1 0"
    echo "scen.progs-femtovg||scen progs femtovg 1 0"
    echo "scen.progs-qt||scen progs qt 0 0"
    [ $small = 1 ] || echo "scen.progs-qt3d||scen progs qt3d 1 1"
    echo "scen.tex||scen tex $n2 1"
    echo "scen.tex-b||scen tex $n2 7"
    echo "scen.buf||scen buf $n1 1"
    echo "scen.fbo||scen fbo $n2 1"
    echo "scen.fbo-msaa|HH_SAMPLES=4|scen fbo $((n2 / 2)) 3"
    echo "scen.stress||scen stress $n3 0"
    echo "scen.stress-big||scen stress $((n3 / 6)) 50000"
    echo "scen.ctx2||scen ctx2"
    echo "scen.inst0||scen inst0"
    echo "scen.win||scen win"
    echo "scen.tex-fail|HH_FAIL_EVERY=5|scen tex $((n2 / 2)) 2"
    echo "scen.buf-fail|HH_FAIL_EVERY=3|scen buf $((n1 / 2)) 2"
    echo "scen.fbo-fail|HH_FAIL_EVERY=4|scen fbo $((n2 / 2)) 5"
    # 40 and up: past the library's start-up (the PowerVR backend alone takes
    # 37 allocations, four frame sets since upstream 11a728d), so each of
    # these fails a program's own allocation and not pvr_backend_init
    for k in 40 45 50 60 80 120; do echo "scen.progs-failat$k|HH_FAIL_AT=$k|scen progs . 0 0"; done
    for t in $("$out/bin/hostile" list); do echo "hostile.$t||hostile $t"; done
  } > "$out/jobs.txt"
  echo "shaders $shaders" > "$out/shaders.txt"
  echo "run.sh: built $out ($(wc -l < "$out/jobs.txt") jobs)"
}

run_one() {
  # run_one <out> <job name> <mode> <job env> <prog> <args...>
  local out="$1" name="$2" mode="$3" jenv="$4" prog="$5"; shift 5
  local base="$out/logs/$name.$mode" shaders rc t0=$SECONDS
  shaders="$(sed -n 's/^shaders=//p' "$out/build-info.txt")"
  (
    cd "$out/logs"
    env ${MODES[$mode]} $jenv HH_SHADERS="$shaders" HH_CALLS="$base.calls" \
      ASAN_OPTIONS="detect_leaks=0:print_summary=1:handle_abort=0" UBSAN_OPTIONS="print_stacktrace=1" \
      timeout "${HH_TIMEOUT:-1200}" "$out/bin/$prog" "$@" > "$base.log" 2> "$base.err"
  ) && rc=0 || rc=$?
  echo "$rc" > "$base.rc"
  echo "$((SECONDS - t0))" > "$base.time"
}

run() {
  local out filter="${2:-.}"
  out="$(cd "$1" && pwd)"
  [ -f "$out/jobs.txt" ] || die "$out is not built"
  local modes="${HH_MODES:-$ALL_MODES}"
  : > "$out/runlist.txt"
  while IFS='|' read -r name jenv cmd; do
    [[ "$name" =~ $filter ]] || continue
    for m in $modes; do
      [ -n "${MODES[$m]+x}" ] || die "unknown mode $m"
      printf '%s|%s|%s|%s\n' "$name" "$m" "$jenv" "$cmd" >> "$out/runlist.txt"
    done
  done < "$out/jobs.txt"
  # longest first, so the slow sweeps start early
  awk -F'|' '{ k = ($1 ~ /qt3d|gets|stress|egl|progs/) ? 0 : 1; print k "\t" $0 }' "$out/runlist.txt" |
    sort -s -k1,1n | cut -f2- |
    xargs -d '\n' -P "${HH_JOBS:-$(nproc)}" -I{} "$HERE/run.sh" _one "$out" {}
  echo "run.sh: $(wc -l < "$out/runlist.txt") runs done in $out/logs"
}

cmd="${1:-}"; shift || true
case "$cmd" in
  build) [ $# -ge 3 ] || die "usage: run.sh build <out> <lib> <driver> [profile]"; build "$@" ;;
  _one) IFS='|' read -r name mode jenv jcmd <<< "$2"
        # shellcheck disable=SC2086
        run_one "$1" "$name" "$mode" "$jenv" $jcmd ;;
  run) [ $# -ge 1 ] || die "usage: run.sh run <out> [job-regex]"; run "$@" ;;
  all) [ $# -ge 3 ] || die "usage: run.sh all <out> <lib> <driver> [profile] [job-regex]"
       build "$1" "$2" "$3" "${4:-dev}"; run "$1" "${5:-.}" ;;
  *) sed -n '3,30p' "$0"; exit 2 ;;
esac
