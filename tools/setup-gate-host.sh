#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# setup-gate-host.sh -- check and prepare the toolchain tools/check.sh needs,
# so the gate can run on any machine, not only the one that happens to have
# the tools.
#
#   tools/setup-gate-host.sh --check
#     Print one "ok" or "missing" line per tool. Exit 0 only when all are ok.
#
#   tools/setup-gate-host.sh --install
#     Add what rustup can add (clippy, rustfmt, the two aarch64-unknown-none
#     targets on every toolchain the gate runs under, see gate_toolchains):
#     user-space, idempotent, always safe. Install distro packages
#     only when passwordless sudo works (sudo -n true): on apt systems run
#     it, on Gentoo-like systems print the emerge equivalents instead. When
#     sudo is not available, print the exact owner command and fall back to
#     what is already on disk: a versioned or slotted clang
#     (/usr/lib/llvm-*/bin/clang, /usr/lib/llvm/*/bin/clang, clang-*) and an
#     existing libclang.so* for LIBCLANG_PATH. Values found are printed,
#     never applied: the script exports nothing into your shell and never
#     edits an rc file. It also prints, without applying it, the
#     CC_aarch64_unknown_none value for the C compiler build.rs would use for
#     the aarch64 rows.
#
# Idempotent and non-destructive: it only adds rustup components, rustup
# targets and distro packages; it removes nothing, overwrites no
# configuration, writes no file, and re-running it changes nothing.
#
# The tools are the ones tools/check.sh needs that rustup or a distro package
# can supply, and the distro packages that building the dependency entries of
# tools/cts/deps.toml needs (the same list: `tools/cts/deps-env.sh --packages
# apt`). The heavy per-machine workspaces themselves (ZEPHYR_WS with west and
# the Zephyr SDK, HAL_NXP_DIR, PCO_SHC with GLSL2USC_CACHE, MESA_PAN_TREE,
# MESA_V3D_TREE) are out of scope here: they are workspaces, not packages. `tools/cts/deps-env.sh --build-missing`
# makes them, after this script has installed the packages. --check still
# reports the MALI_CSFFW firmware file (check.sh's gpu::mali row gaps without
# it); --install only prints the command that fetches it, as that is a
# download, not a package. --check also reports whether
# driver/src/fw/pvr_fw_blob.h is there (the AM62P image rows gap without it;
# it is copied in by hand, never fetched, copied or committed by this script)
# and a CC_aarch64_unknown_none that names no executable. --check also
# reports GLMARK2_SRC (check.sh's glmark2 corpus row gaps unless it names a
# glmark2 tree at the commit tools/hostharness/glmark2/run.sh pins, tested
# the way run.sh tests it: a git checkout at the pin without local changes,
# or the GitHub archive directory glmark2-<pin>); --install only prints the
# commands that fetch it. --check also reports DEQP_GLES3_TEST_DIR (the dEQP
# shader tree es3_shaderc_corpus reads: unset skips it, a bad path fails it).
# --check also reports the rustup target aarch64-unknown-linux-gnu (check.sh's
# "clippy yakogl-mali-panthor (aarch64-unknown-linux-gnu)" row gaps without it
# on the toolchain the gate runs under; the same per-toolchain rule as
# aarch64-unknown-none applies). --check also reports PCO_SPM_LOAD_CAPTURE,
# the capture_spm_load of PCO_SHC's Mesa build (check.sh's "gen
# tools/pco_shc/spm_load == gen_spm_load.py capture" row gaps without it):
# looked for in check.sh's order, only when PCO_SHC names an executable (the
# row's own condition; PCO_SHC itself is one of the workspaces above, out of
# scope here), and --install only prints the build that makes it.
set -u

PROG=${0##*/}

usage() {
  cat <<EOF
usage: $PROG --check | --install

  --check    print one ok/missing line per tool; exit non-zero if any is missing
  --install  add rustup components and targets; install distro packages only
             through passwordless sudo (sudo -n); print owner commands and
             user-space fallback values otherwise. Never edits an rc file.
EOF
}

# ---------------------------------------------------------------------------
# checks: one line per tool, "ok <name> [detail]" or "missing <name> [detail]"
# ---------------------------------------------------------------------------

missing_count=0

line() {
  local status=$1 name=$2
  shift 2
  if [ "$status" = missing ]; then
    missing_count=$((missing_count + 1))
  fi
  if [ "$#" -gt 0 ]; then
    printf '%-7s %-38s %s\n' "$status" "$name" "$*"
  else
    printf '%-7s %s\n' "$status" "$name"
  fi
}

check_cargo() {
  local out ver
  out=$(cargo --version 2>/dev/null) || {
    line missing cargo "not on PATH (install rustup)"
    return
  }
  ver=${out#cargo }
  ver=${ver%% *}
  if [ "$(printf '%s\n1.88\n' "$ver" | sort -V | head -n 1)" = "1.88" ]; then
    line ok cargo "$out"
  else
    line missing cargo "$out is older than 1.88 (Cargo.toml rust-version)"
  fi
}

# A rustup target lives per toolchain. The gate runs under the toolchain in
# effect (RUSTUP_TOOLCHAIN, else the default), and on the farm that is the
# nightly ccfarm's settings name; check.sh's softfloat row also calls
# `cargo +nightly`. A target added only to the default toolchain is "missing"
# the moment the gate runs under nightly (runner A, 2026-10-10). So every target
# is checked and added on the toolchain in effect and on nightly, when one is
# installed.
gate_toolchains() {
  local active
  active=$(rustup show active-toolchain 2>/dev/null | head -1 | cut -d' ' -f1)
  [ -n "$active" ] && echo "$active"
  case $active in
    nightly*) ;;
    *) rustup toolchain list 2>/dev/null | grep -m1 -o '^nightly[^ ]*' ;;
  esac
}

target_installed() { # <target> <toolchain>
  rustup target list --installed --toolchain "$2" 2>/dev/null | grep -qx "$1"
}

check_rustup_target() {
  local t=$1 tc
  if ! command -v rustup >/dev/null 2>&1; then
    line missing "rustup-target-$t" "rustup is not installed"
    return
  fi
  for tc in $(gate_toolchains); do
    if target_installed "$t" "$tc"; then
      line ok "rustup-target-$t" "$tc"
    else
      line missing "rustup-target-$t" "rustup target add --toolchain $tc $t"
    fi
  done
}

# check.sh accepts a nightly with rust-src in place of the installed target
# (drivers_softfloat_check), so accept it here too.
check_softfloat_target() {
  local t=aarch64-unknown-none-softfloat tc sysroot
  if ! command -v rustup >/dev/null 2>&1; then
    line missing "rustup-target-$t" "rustup is not installed"
    return
  fi
  sysroot=$(rustc +nightly --print sysroot 2>/dev/null) || sysroot=
  for tc in $(gate_toolchains); do
    if target_installed "$t" "$tc"; then
      line ok "rustup-target-$t" "$tc"
    elif [ -n "$sysroot" ] && [ -d "$sysroot/lib/rustlib/src/rust/library/core" ]; then
      line ok "rustup-target-$t" "$tc: target not installed; a nightly with rust-src covers the row"
    else
      line missing "rustup-target-$t" "rustup target add --toolchain $tc $t (or a nightly with rust-src)"
    fi
  done
}

check_clippy() {
  local out
  out=$(cargo clippy --version 2>/dev/null) || {
    line missing clippy "rustup component add clippy"
    return
  }
  line ok clippy "${out%%$'\n'*}"
}

check_rustfmt() {
  local out
  out=$(cargo fmt --version 2>/dev/null) || {
    line missing rustfmt "rustup component add rustfmt"
    return
  }
  line ok rustfmt "${out%%$'\n'*}"
}

check_cc() {
  local out
  out=$(cc --version 2>/dev/null) || {
    line missing cc "no host C compiler named cc (distro gcc or clang)"
    return
  }
  line ok cc "${out%%$'\n'*}"
}

# The C compiler build.rs uses for the aarch64-unknown-none C wrappers:
# CC_<target>, CC_<target with underscores>, TARGET_CC, else plain clang
# (build.rs freestanding()). Prints the compiler on success.
env_cc() {
  local v p
  for v in CC_aarch64_unknown_none CC_aarch64-unknown-none TARGET_CC; do
    # printenv, not ${!v}: CC_aarch64-unknown-none has a hyphen, which bash
    # indirect expansion cannot name (build.rs and cmake can still set it).
    p=$(printenv "$v" 2>/dev/null) || continue
    [ -n "$p" ] || continue
    p=${p%% *}
    if command -v "$p" >/dev/null 2>&1; then
      printf '%s' "$p"
      return 0
    fi
  done
  return 1
}

check_clang() {
  local p out
  if p=$(env_cc); then
    line ok clang "build.rs uses $p (named by the environment)"
    return
  fi
  out=$(clang --version 2>/dev/null) || {
    line missing clang "no clang on PATH and no CC_aarch64_unknown_none / CC_aarch64-unknown-none / TARGET_CC"
    return
  }
  line ok clang "${out%%$'\n'*}"
}

# bindgen loads libclang at run time through clang-sys, and the probe is
# clang-sys's own search (1.9.1 build/common.rs search_libclang_directories):
# LIBCLANG_PATH alone when set, else `llvm-config --prefix` bin/lib/lib64,
# LD_LIBRARY_PATH, LIBRARY_PATH and the /usr/local and /usr lib directories,
# for libclang.so, libclang-*.so, libclang.so.* and libclang-*.so.* (Ubuntu
# ships only the versioned libclang-<N>.so; libclang-cpp is excluded).
# The libclang check of ccfarm's settings is the same code. find_library("clang")
# was not: it misses the versioned names and called runner B missing while
# bindgen ran there (2026-10-10).
probe_libclang() {
  python3 -c 'import glob,os,shutil,subprocess,sys; names=("libclang.so","libclang-*.so","libclang.so.*","libclang-*.so.*"); hit=lambda d: [p for n in names for p in sorted(glob.glob(os.path.join(d,n))) if "-cpp." not in os.path.basename(p)]; lp=os.environ.get("LIBCLANG_PATH"); f=hit(lp) + ([lp] if lp and os.path.isfile(lp) else []) if lp else []; pre=(lambda r: (r.stdout.splitlines() or [""])[0] if r.returncode==0 else "")(subprocess.run(["llvm-config","--prefix"],capture_output=True,text=True)) if not lp and shutil.which("llvm-config") else ""; dirs=[os.path.join(pre,s) for s in ("bin","lib","lib64")] if pre else []; dirs+=[d for v in ("LD_LIBRARY_PATH","LIBRARY_PATH") for d in os.environ.get(v,"").split(":") if d]; dirs+=[d for pat in ("/usr/local/llvm*/lib*","/usr/local/lib*/*/*","/usr/local/lib*/*","/usr/local/lib*","/usr/lib*/*/*","/usr/lib*/*","/usr/lib*") for d in sorted(glob.glob(pat))]; f=f or [p for d in dirs for p in hit(d)]; print(f[0] if f else ""); sys.exit(0 if f else 1)' 2>/dev/null
}

check_libclang() {
  local found
  if found=$(probe_libclang); then
    line ok libclang "$found"
  else
    line missing libclang "no libclang.so / libclang-*.so found (bindgen needs it for the csbgen-bindings --check row; distro clang/libclang, or LIBCLANG_PATH)"
  fi
}

check_yaml() {
  if python3 -c 'import yaml' 2>/dev/null; then
    line ok python3-yaml
  else
    line missing python3-yaml "python3 -c 'import yaml' failed (distro package python3-yaml / PyYAML)"
  fi
}

check_glslang() {
  local cmd=${GLSLANG:-glslangValidator} out
  out=$("$cmd" --version 2>/dev/null) || {
    line missing glslangValidator "$cmd not on PATH (distro package glslang; GLSLANG names it)"
    return
  }
  line ok glslangValidator "${out%%$'\n'*}"
}

check_pkgconfig() {
  local m out
  if ! command -v pkg-config >/dev/null 2>&1; then
    line missing pkg-config-expat "pkg-config is not installed"
    line missing pkg-config-zlib "pkg-config is not installed"
    return
  fi
  for m in expat zlib; do
    if pkg-config --exists "$m" 2>/dev/null; then
      out=$(pkg-config --modversion "$m" 2>/dev/null)
      line ok "pkg-config-$m" "$out"
    else
      line missing "pkg-config-$m" "pkg-config --exists $m failed (distro dev package for $m)"
    fi
  done
}

# The Mali-G610's CSF firmware, which is never committed: check.sh's
# "yakogl-drivers gpu::mali on the board's mali_csffw.bin" row reads
# $MALI_CSFFW, else deps-env.sh's place for it, and is a gap when the file is
# missing. Resolved the same way as check.sh (mali_csffw=...).
mali_csffw_path() {
  local f=${MALI_CSFFW-}
  if [ -z "$f" ]; then
    f=$("$(dirname "$0")/cts/deps-env.sh" --layout 2>/dev/null)/mali-csffw/mali_csffw.bin
  fi
  printf '%s' "$f"
}

check_mali_csffw() {
  local f
  f=$(mali_csffw_path)
  if [ -f "$f" ]; then
    line ok MALI_CSFFW "$f"
  else
    line missing MALI_CSFFW "$f not found (the gpu::mali row would gap): tools/cts/deps-env.sh --build MALI_CSFFW, or set MALI_CSFFW to the board's mali_csffw.bin"
  fi
}

# The glmark2 sources the "glmark2 corpus generated from GLMARK2_SRC" row
# reads (check.sh). Accepted exactly as tools/hostharness/glmark2/run.sh
# accepts the tree it is handed: it holds src/default-benchmarks.h, and it is
# either a git checkout of its own at the pinned commit without local changes
# (an archive carries no git metadata, so it cannot pass that test), or the
# GitHub archive of that commit, whose directory name is its provenance. The
# commit belongs to run.sh; nothing here names a host path.

# The pinned commit, read from run.sh's own COMMIT= line so this check and
# the row cannot drift apart. Empty when run.sh has none.
glmark2_pin() {
  sed -n 's/^COMMIT=\([0-9a-f]\{40\}\)\([^0-9a-f].*\)\?$/\1/p' \
    "$(dirname "$0")/hostharness/glmark2/run.sh" 2>/dev/null | head -n 1
}

# Print how $GLMARK2_SRC is such a tree, and exit 0, exactly when run.sh
# would accept it; exit 1 silently otherwise.
glmark2_provenance() {
  local commit src head dirty
  commit=$(glmark2_pin)
  [ -n "$commit" ] || return 1
  [ -n "${GLMARK2_SRC-}" ] || return 1
  # the physical path: run.sh compares it with git's resolved top level, so
  # a symlink to the tree (the dependency layout's glmark2-src) passes too.
  src=$(cd "$GLMARK2_SRC" 2>/dev/null && pwd -P) || return 1
  [ -f "$src/src/default-benchmarks.h" ] || return 1
  if [ "$(git -C "$src" rev-parse --show-toplevel 2>/dev/null)" = "$src" ]; then
    head=$(git -C "$src" rev-parse HEAD 2>/dev/null)
    dirty=$(git -C "$src" status --porcelain --untracked-files=no 2>/dev/null)
    [ "$head" = "$commit" ] && [ -z "$dirty" ] || return 1
    printf 'git checkout at %s' "$commit"
  elif [ "$(basename "$src")" = "glmark2-$commit" ]; then
    printf "GitHub archive glmark2-%s (not a git tree: the commit is the archive's name)" "$commit"
  else
    return 1
  fi
}

check_glmark2_src() {
  local commit provenance hint src
  if provenance=$(glmark2_provenance); then
    line ok GLMARK2_SRC "$provenance ($GLMARK2_SRC)"
    return
  fi
  commit=$(glmark2_pin)
  if [ -z "$commit" ]; then
    line missing GLMARK2_SRC "the pinned commit is not readable from tools/hostharness/glmark2/run.sh (its COMMIT= line)"
    return
  fi
  hint="git clone https://github.com/glmark2/glmark2 <dir> && git -C <dir> checkout --detach $commit, or unpack the GitHub archive of $commit so that the directory is named glmark2-$commit, then set GLMARK2_SRC=<dir>; tools/cts/deps-env.sh --build GLMARK2_SRC does the first into its layout"
  if [ -n "${GLMARK2_SRC-}" ] && src=$(cd "$GLMARK2_SRC" 2>/dev/null && pwd -P) &&
    [ -f "$src/src/default-benchmarks.h" ] &&
    [ "$(git -C "$src" rev-parse --show-toplevel 2>/dev/null)" = "$src" ]; then
    line missing GLMARK2_SRC "$src is a git checkout other than the pinned $commit without local changes (the glmark2 corpus row would gap): git -C \"$src\" checkout --detach $commit"
  else
    line missing GLMARK2_SRC "GLMARK2_SRC='${GLMARK2_SRC-}' names no glmark2 tree at the pinned $commit: neither a git checkout of it without local changes nor the archive directory glmark2-$commit: $hint"
  fi
}

# The dEQP shader tree the shaderc corpus part of tests/es3_shaderc_corpus.rs
# reads (the full-suite rows of the host-test,gpu-powervr,shaderc-online and
# host-test,gpu-vivante,shaderc-online configurations run it). Unset, the test
# prints that it skipped and passes; set, it expect()s the directory to be
# readable and measures every .test file in it, so a value naming a missing
# path fails those rows. The repository pins no VK-GL-CTS revision (it only
# names the checkout's default place, ~/com/github/KhronosGroup/VK-GL-CTS in
# tools/cts/cts-build.sh), so none is named here either.
check_deqp_gles3_dir() {
  local d=${DEQP_GLES3_TEST_DIR-} n
  if [ -z "$d" ]; then
    line ok DEQP_GLES3_TEST_DIR "unset: the es3_shaderc_corpus corpus part is skipped and passes; set it to a VK-GL-CTS data/gles3/shaders directory to run it"
    return
  fi
  if [ ! -d "$d" ]; then
    line missing DEQP_GLES3_TEST_DIR "$d is not a directory (the shaderc-online suite rows would fail in es3_shaderc_corpus): unset it, or point it at data/gles3/shaders of a VK-GL-CTS checkout (this repository pins no revision; tools/cts/cts-build.sh uses ~/com/github/KhronosGroup/VK-GL-CTS by default)"
    return
  fi
  n=$(find "$d" -maxdepth 1 -type f -name '*.test' 2>/dev/null | wc -l)
  if [ "$n" -gt 0 ]; then
    line ok DEQP_GLES3_TEST_DIR "$d ($n .test files)"
  else
    line missing DEQP_GLES3_TEST_DIR "$d holds no .test file (not a data/gles3/shaders directory of a VK-GL-CTS checkout): unset it or point it at that directory"
  fi
}

# The PowerVR firmware header the AM62P image rows embed (check.sh,
# needs_pvr_fw_blob). It is gitignored and never committed; this only reports.
check_pvr_fw_blob() {
  local f
  f=$(dirname "$0")/../driver/src/fw/pvr_fw_blob.h
  if [ -f "$f" ]; then
    line ok pvr_fw_blob.h "$f"
  else
    line missing pvr_fw_blob.h "$f not found (the AM62P image rows would gap): copy it in by hand, e.g. from the path in PVR_FW_BLOB (never fetched, never committed)"
  fi
}

# The capture_spm_load of the Mesa build PCO_SHC came from, which check.sh's
# "gen tools/pco_shc/spm_load == gen_spm_load.py capture" row regenerates the
# partial render's load shader with and compares byte for byte; without a
# program the row is a gap naming the build. Printed the way check.sh resolves
# one (spm_load_capture): the PCO_SPM_LOAD_CAPTURE variable, then beside the
# resolved PCO_SHC (the tools/README.md form: PCO_SHC in the build directory,
# or a link into it), then the deps layout's Mesa tree at <PCO_SHC's
# directory>.src. A host prepared before tools/cts/deps.toml's entry has
# pco_shc and not this program. Without PCO_SHC the row is a gap naming
# PCO_SHC itself, so nothing is reported in that case: PCO_SHC is a workspace
# (the header), not a tool this script judges.
pco_spm_load_capture_path() {
  local p
  for p in "${PCO_SPM_LOAD_CAPTURE-}" \
           "$(dirname "$(readlink -f "${PCO_SHC-}")")/capture_spm_load" \
           "$(dirname "${PCO_SHC-}").src/mesa-26.2.1/build/src/imagination/vulkan/capture_spm_load"; do
    if [ -n "$p" ] && [ -x "$p" ]; then printf '%s' "$p"; return 0; fi
  done
  return 1
}

check_pco_spm_load_capture() {
  local p
  if [ -z "${PCO_SHC-}" ] || [ ! -x "${PCO_SHC-}" ]; then
    return 0
  fi
  if p=$(pco_spm_load_capture_path); then
    line ok PCO_SPM_LOAD_CAPTURE "$p"
  else
    line missing PCO_SPM_LOAD_CAPTURE "no capture_spm_load at PCO_SPM_LOAD_CAPTURE='${PCO_SPM_LOAD_CAPTURE-}' or beside PCO_SHC='${PCO_SHC-}' (the spm_load gen row would gap): tools/cts/deps-env.sh --build PCO_SPM_LOAD_CAPTURE"
  fi
}

# A CC_aarch64_unknown_none that names no executable breaks the aarch64 rows
# (build.rs uses it instead of clang on PATH). Silent when it is unset.
check_cc_aarch64() {
  local cc=${CC_aarch64_unknown_none-}
  [ -n "$cc" ] || return 0
  if command -v "$cc" >/dev/null 2>&1; then
    line ok CC_aarch64_unknown_none "$cc"
  else
    line missing CC_aarch64_unknown_none "$cc is not an executable: unset it (build.rs then falls back to clang on PATH) or point it at a working compiler, e.g. /usr/bin/clang"
  fi
}

run_checks() {
  check_cargo
  check_rustup_target aarch64-unknown-none
  # the board target of yakogl-mali-panthor (check.sh's mali gltest rows)
  check_rustup_target aarch64-unknown-linux-gnu
  check_softfloat_target
  check_clippy
  check_rustfmt
  check_cc
  check_clang
  check_libclang
  check_yaml
  check_glslang
  check_pkgconfig
  check_mali_csffw
  check_glmark2_src
  check_deqp_gles3_dir
  check_pvr_fw_blob
  check_pco_spm_load_capture
  check_cc_aarch64
}

# ---------------------------------------------------------------------------
# install
# ---------------------------------------------------------------------------

run() {
  printf '+ %s\n' "$*"
  "$@"
}

# apt: the packages tools/check.sh needs, plus (below) the packages of every
# entry of tools/cts/deps.toml, so an entry's packages reach this script
# without a second edit.
apt_packages=(clang libclang-dev python3-yaml glslang-tools pkg-config libexpat1-dev zlib1g-dev)
# Gentoo-like: the same tools as portage categories.
emerge_packages=(sys-devel/clang sys-libs/clang dev-python/pyyaml virtual/pkgconfig dev-libs/expat sys-libs/zlib media-libs/glslang)

# add_dep_packages <array name> <family>: append deps.toml's packages of the
# family to the array, each once.
add_dep_packages() {
  local -n arr=$1
  local p q have
  while IFS= read -r p; do
    have=0
    for q in "${arr[@]}"; do [ "$q" = "$p" ] && have=1; done
    [ "$have" -eq 1 ] || arr+=("$p")
  done < <("$(dirname "$0")/cts/deps-env.sh" --packages "$2" 2>/dev/null)
}
add_dep_packages apt_packages apt
add_dep_packages emerge_packages portage

install_rustup() {
  if ! command -v rustup >/dev/null 2>&1; then
    echo "rustup is not installed: install it first (https://rustup.rs); the cargo rows cannot be prepared without it."
    return
  fi
  run rustup component add clippy rustfmt
  local tc
  for tc in $(gate_toolchains); do
    run rustup target add --toolchain "$tc" aarch64-unknown-none aarch64-unknown-none-softfloat
  done
}

install_distro() {
  if command -v apt-get >/dev/null 2>&1; then
    if sudo -n true 2>/dev/null; then
      run sudo -n apt-get install -y "${apt_packages[@]}"
    else
      echo "no passwordless sudo here; owner command:"
      echo "  sudo apt-get install -y ${apt_packages[*]}"
    fi
    return
  fi
  if command -v emerge >/dev/null 2>&1 || [ -d /var/db/portage ]; then
    echo "Gentoo-like system detected; the emerge equivalents are:"
    echo "  emerge -1 --noreplace ${emerge_packages[*]}"
    if sudo -n true 2>/dev/null; then
      echo "with passwordless sudo: sudo -n emerge -1 --noreplace ${emerge_packages[*]}"
    fi
    return
  fi
  echo "no apt-get or emerge found: install clang, libclang, PyYAML, glslang,"
  echo "pkg-config, expat and zlib with this distribution's package manager."
}

# A clang already on disk but not on PATH (a versioned or slotted install):
# Debian keeps clang under /usr/lib/llvm-<N>/bin, Gentoo under
# /usr/lib/llvm/<N>/bin. Prints the highest-versioned candidate.
find_clang() {
  local d c ver best="" bestver=""
  for d in /usr/lib/llvm-*/bin /usr/lib/llvm/*/bin /usr/bin /usr/local/bin "$HOME/.local/bin"; do
    [ -d "$d" ] || continue
    # clang and clang-<version> only: clangd, clang-format and clang-tidy
    # match clang* but are not compilers.
    for c in "$d"/clang "$d"/clang-[0-9]*; do
      [ -x "$c" ] || continue
      ver=$("$c" --version 2>/dev/null | sed -n '1s/.*version \([0-9.]*\).*/\1/p')
      [ -n "$ver" ] || continue
      if [ -z "$bestver" ] ||
        [ "$(printf '%s\n%s\n' "$bestver" "$ver" | sort -V | tail -n 1)" = "$ver" ]; then
        best=$c
        bestver=$ver
      fi
    done
  done
  [ -n "$best" ] || return 1
  printf '%s' "$best"
}

# A directory holding libclang.so* (Debian: /usr/lib/llvm-<N>/lib and
# /usr/lib/x86_64-linux-gnu; Gentoo: /usr/lib/llvm/<N>/lib).
find_libclang_dir() {
  local d f
  for d in /usr/lib/llvm-*/lib /usr/lib/llvm/*/lib \
    /usr/lib/x86_64-linux-gnu /usr/lib /usr/lib64 /usr/local/lib /usr/local/lib64; do
    [ -d "$d" ] || continue
    for f in "$d"/libclang.so "$d"/libclang-*.so "$d"/libclang.so.* "$d"/libclang-*.so.*; do
      [ -e "$f" ] || continue
      printf '%s' "$d"
      return 0
    done
  done
  return 1
}

# Print the values a machine would need exported or registered, without
# applying anything.
report_values() {
  local cc_path="" libdir
  if cc_path=$(env_cc); then
    :
  elif cc_path=$(command -v clang 2>/dev/null) && [ -n "$cc_path" ]; then
    :
  elif cc_path=$(find_clang); then
    echo "clang is installed but not on PATH: $cc_path"
    echo "to name it for build.rs (print only, not applied):"
    echo "  export CC_aarch64_unknown_none=$cc_path"
  fi
  if [ -n "$cc_path" ]; then
    echo "C compiler for the aarch64 rows (print only, not applied):"
    echo "  export CC_aarch64_unknown_none=$cc_path"
  fi
  if [ ! -f "$(mali_csffw_path)" ]; then
    echo "mali_csffw.bin is missing (print only, not run):"
    echo "  tools/cts/deps-env.sh --build MALI_CSFFW"
  fi
  if [ -z "$(glmark2_provenance)" ]; then
    echo "glmark2 at the pinned commit is missing (print only, not run):"
    echo "  tools/cts/deps-env.sh --build GLMARK2_SRC"
    echo "  git clone https://github.com/glmark2/glmark2 <dir> && git -C <dir> checkout --detach $(glmark2_pin), then set GLMARK2_SRC=<dir>"
  fi
  if [ -n "${PCO_SHC-}" ] && [ -x "${PCO_SHC-}" ] && ! pco_spm_load_capture_path >/dev/null; then
    echo "capture_spm_load (the Mesa build PCO_SHC came from) is missing (print only, not run):"
    echo "  tools/cts/deps-env.sh --build PCO_SPM_LOAD_CAPTURE"
  fi
  if ! probe_libclang >/dev/null 2>&1 && libdir=$(find_libclang_dir); then
    echo "libclang is on disk at $libdir (print only, not applied):"
    echo "  export LIBCLANG_PATH=$libdir"
  fi
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------

case ${1-} in
  --check)
    run_checks
    if [ "$missing_count" -gt 0 ]; then
      echo "missing: $missing_count tool(s); run $PROG --install"
      exit 1
    fi
    exit 0
    ;;
  --install)
    install_rustup
    install_distro
    echo
    echo "after install:"
    run_checks
    report_values
    if [ "$missing_count" -gt 0 ]; then
      echo "not complete: $missing_count tool(s) still missing; the owner commands above are what is left"
      exit 1
    fi
    exit 0
    ;;
  *)
    usage
    exit 2
    ;;
esac
