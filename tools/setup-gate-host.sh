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
# can supply. The heavy per-machine workspaces (ZEPHYR_WS with
# west and the Zephyr SDK, HAL_NXP_DIR, PCO_SHC with GLSL2USC_CACHE,
# MESA_V3D_TREE, driver/src/fw/pvr_fw_blob.h) are out of scope here: they are
# workspaces and firmware, not packages.
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
# nightly .ccfarm/project.toml names; check.sh's softfloat row also calls
# `cargo +nightly`. A target added only to the default toolchain is "missing"
# the moment the gate runs under nightly (nuc, 2026-10-10). So every target
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

# bindgen loads libclang at run time (clang-sys: LIBCLANG_PATH first, then
# the usual places; check.sh's own probe is find_library).
probe_libclang() {
  local lp=${LIBCLANG_PATH:-} found
  if [ -n "$lp" ] && ls "$lp"/libclang.so* >/dev/null 2>&1; then
    printf 'LIBCLANG_PATH=%s' "$lp"
    return 0
  fi
  found=$(python3 -c 'import ctypes.util; v = ctypes.util.find_library("clang"); print(v or "")' 2>/dev/null)
  [ -n "$found" ] || return 1
  printf 'find_library("clang") -> %s' "$found"
}

check_libclang() {
  local found
  if found=$(probe_libclang); then
    line ok libclang "$found"
  else
    line missing libclang "no libclang.so* found (bindgen needs it for the csbgen-bindings --check row; distro clang/libclang, or LIBCLANG_PATH)"
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

run_checks() {
  check_cargo
  check_rustup_target aarch64-unknown-none
  check_softfloat_target
  check_clippy
  check_rustfmt
  check_cc
  check_clang
  check_libclang
  check_yaml
  check_glslang
  check_pkgconfig
}

# ---------------------------------------------------------------------------
# install
# ---------------------------------------------------------------------------

run() {
  printf '+ %s\n' "$*"
  "$@"
}

# apt: the packages tools/check.sh needs.
apt_packages=(clang libclang-dev python3-yaml glslang-tools pkg-config libexpat1-dev zlib1g-dev)
# Gentoo-like: the same tools as portage categories.
emerge_packages=(sys-devel/clang sys-libs/clang dev-python/pyyaml virtual/pkgconfig dev-libs/expat sys-libs/zlib media-libs/glslang)

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
    for f in "$d"/libclang.so*; do
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
