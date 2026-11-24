#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# One host check that a STM32MP257F-DK U-Boot board session can start, run
# before the board is leased (tools/board-stm32mp2-uboot.sh, docs/notes/).
#
#   tools/mp2_uboot_session_preflight.sh [--sha256 <hex>] [--tftp] <image>
#
# Checks: tools/board.sh --dry-run stm32mp257f-dk uboot-status, disarm and run
# reach the runner; the host self-tests of the runner, the script builder, the
# console, the env code (and the env_read probe, if there is one) pass; the
# image exists, matches --sha256, is below the runner's MAX_IMG, has an
# image_size (byte 16) that fits LOAD..LOAD_END, and starts with an AArch64 B
# instruction; BOARD_HOST and BOARD_UART are set (and NETBOOT_SERVER and
# NETBOOT_TFTP_ROOT with --tftp or BOARD_BOOT=uboot-tftp). With --tftp,
# tools/mp2_tftp_preflight.sh passes too.
#
# Never contacts the board or the network (every board step is --dry-run).
# Output: one "preflight: ok|FAIL <what>" line per check, then
# "preflight: N ok, F failed". Exit 0 only when F is 0, 2 on a usage error.
# A variable's value is never printed (only set or not set), nor is the output
# of a check: a check says only whether it passed.
# Test override (selftest): MP2_PREFLIGHT_ROOT=<tree laid out like the repo>.
set -u
usage() { echo "usage: $0 [--sha256 <hex>] [--tftp] <image>" >&2; exit 2; }
want_sha=
tftp=0
img=
while [ $# -gt 0 ]; do
  case $1 in
  --sha256) [ $# -ge 2 ] || usage; want_sha=$2; shift 2 ;;
  --tftp) tftp=1; shift ;;
  -*) usage ;;
  *) [ -z "$img" ] || usage; img=$1; shift ;;
  esac
done
[ -n "$img" ] || usage
if [ -n "$want_sha" ]; then
  case $want_sha in *[!0-9a-fA-F]*) usage ;; esac
  [ ${#want_sha} = 64 ] || usage
fi

root=${MP2_PREFLIGHT_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}
ok=0
bad=0
res() { # status what
  if [ "$1" = ok ]; then ok=$((ok + 1)); else bad=$((bad + 1)); fi
  echo "preflight: $1 $2"
}
# quiet NAME CMD...: pass iff CMD exits 0; its output is dropped
quiet() {
  local name=$1
  shift
  if "$@" >/dev/null 2>&1 </dev/null; then res ok "$name"; else res FAIL "$name"; fi
}
setcheck() { # NAME: set or not set, never the value
  if [ -n "${!1:-}" ]; then res ok "$1 is set"; else res FAIL "$1 is not set"; fi
}

cd "$root" 2>/dev/null || { echo "preflight: FAIL tree root is not enterable"; echo "preflight: 0 ok, 1 failed"; exit 1; }
case $img in /*) ;; *) img=$OLDPWD/$img ;; esac

# 1. board.sh dry-runs reach the runner (a dry-run touches neither board nor net)
runner_name=board-stm32mp2-uboot.sh
dry() { # verb args...: the dry-run succeeds and the runner answered
  local out
  out=$(bash tools/board.sh --dry-run stm32mp257f-dk "$@" 2>&1 </dev/null) || return 1
  case $out in *"$runner_name"*) return 0 ;; esac
  return 1
}
if [ -f tools/board.sh ]; then
  logd=$(mktemp -d "${TMPDIR:-/var/tmp}/mp2pre.XXXXXX") || logd=
  quiet "board.sh --dry-run uboot-status reaches the runner" dry uboot-status
  quiet "board.sh --dry-run disarm reaches the runner" dry disarm
  quiet "board.sh --dry-run run reaches the runner" dry run "$img" "${logd:-/nonexistent}"
  [ -z "$logd" ] || rm -rf "$logd"
else
  res FAIL "tools/board.sh is missing"
fi

# 2. host self-tests
st() { # name path cmd...: run the self-test (a missing file fails)
  local name=$1 path=$2
  shift 2
  [ -f "$path" ] || { res FAIL "$name: $path is missing"; return; }
  quiet "$name passes" "$@"
}
st "driver/tools/uboot-mp2/selftest.sh" driver/tools/uboot-mp2/selftest.sh bash driver/tools/uboot-mp2/selftest.sh
st "driver/tools/uboot-mp2/oneshot_selftest.sh" driver/tools/uboot-mp2/oneshot_selftest.sh bash driver/tools/uboot-mp2/oneshot_selftest.sh
st "tools/mp2_uboot_console.py --self-test" tools/mp2_uboot_console.py python3 -I tools/mp2_uboot_console.py --self-test
st "tools/board_stm32mp2_uboot_selftest.sh" tools/board_stm32mp2_uboot_selftest.sh bash tools/board_stm32mp2_uboot_selftest.sh
st "tools/uboot_env_mp2_selftest.py" tools/uboot_env_mp2_selftest.py python3 -I tools/uboot_env_mp2_selftest.py
for f in tools/*env_read*selftest*; do
  [ -f "$f" ] || continue
  case $f in *.py) st "$f" "$f" python3 -I "$f" ;; *) st "$f" "$f" bash "$f" ;; esac
done

# 3. the image against the runner's own constants (read from its source, never sourced)
src=tools/board-stm32mp2-uboot.sh
const() { # NAME: the value of a top-level NAME=... line, arithmetic whitelisted
  local line v
  line=$(grep -m1 -E "^$1=" "$src" 2>/dev/null) || return 1
  v=$(printf '%s' "${line#"$1"=}" | sed -E -e 's/[[:space:]]*#.*$//' -e 's/^\$\(\((.*)\)\)$/\1/' -e 's/^[[:space:]]+//' -e 's/[[:space:]]+$//')
  # whitelist: numbers (decimal or 0x hex) joined by * + - only
  printf '%s' "$v" | grep -Eq '^(0x[0-9a-fA-F]+|[0-9]+)([[:space:]]*[-*+][[:space:]]*(0x[0-9a-fA-F]+|[0-9]+))*$' || return 1
  echo $((v))
}
MAX_IMG=$(const MAX_IMG) || MAX_IMG=
LOAD=$(const LOAD) || LOAD=
LOAD_END=$(const LOAD_END) || LOAD_END=
if [ -n "$MAX_IMG" ] && [ -n "$LOAD" ] && [ -n "$LOAD_END" ]; then
  res ok "runner constants MAX_IMG, LOAD, LOAD_END read"
else
  res FAIL "runner constants MAX_IMG, LOAD, LOAD_END not readable from $src"
fi

if [ -f "$img" ] && [ -r "$img" ]; then
  res ok "image exists"
  size=$(stat -c %s -- "$img" 2>/dev/null || echo -1)
  if [ -n "$want_sha" ]; then
    got=$(sha256sum -- "$img" 2>/dev/null | cut -d' ' -f1)
    if [ "$got" = "$(printf '%s' "$want_sha" | tr 'A-F' 'a-f')" ]; then res ok "image sha256 matches"; else res FAIL "image sha256 differs"; fi
  fi
  if [ -n "$MAX_IMG" ] && [ "$size" -ge 0 ] && [ "$size" -lt "$MAX_IMG" ]; then
    res ok "image is below MAX_IMG"
  else
    res FAIL "image is not below MAX_IMG"
  fi
  if [ "$size" -ge 64 ]; then
    isz=$(od -An -v -tu8 --endian=little -N8 -j16 -- "$img" 2>/dev/null | tr -d ' ')
    if [ -n "$LOAD" ] && [ -n "$LOAD_END" ] && [ -n "$isz" ] && [ "$isz" -gt 0 ] 2>/dev/null &&
       [ "$isz" -le $((LOAD_END - LOAD)) ]; then
      res ok "image_size fits LOAD..LOAD_END"
    else
      res FAIL "image_size does not fit LOAD..LOAD_END"
    fi
    w=$(od -An -v -tu4 --endian=little -N4 -j0 -- "$img" 2>/dev/null | tr -d ' ')
    if [ -n "$w" ] && [ $((w >> 26)) = 5 ]; then
      res ok "image starts with an AArch64 B instruction"
    else
      res FAIL "image does not start with an AArch64 B instruction"
    fi
  else
    res FAIL "image is too short for a header"
  fi
else
  res FAIL "image does not exist"
fi

# 4. environment (names only)
setcheck BOARD_HOST
setcheck BOARD_UART
if [ "$tftp" = 1 ] || [ "${BOARD_BOOT:-}" = uboot-tftp ]; then
  setcheck NETBOOT_SERVER
  setcheck NETBOOT_TFTP_ROOT
fi
if [ "$tftp" = 1 ]; then
  if [ -f tools/mp2_tftp_preflight.sh ]; then
    quiet "tools/mp2_tftp_preflight.sh passes" bash tools/mp2_tftp_preflight.sh
  else
    res FAIL "tools/mp2_tftp_preflight.sh is missing"
  fi
fi

echo "preflight: $ok ok, $bad failed"
[ "$bad" = 0 ]
