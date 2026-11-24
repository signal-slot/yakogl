#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of mp2_uboot_session_preflight.sh against a fake tree built under
# $TMPDIR (MP2_PREFLIGHT_ROOT): the passing run, then one failure at a time.
# No board, no network, nothing written outside the temp directory.
set -u
here=$(cd "$(dirname "$0")" && pwd)
p="$here/mp2_uboot_session_preflight.sh"
base=${TMPDIR:-/var/tmp}
w=$(mktemp -d "$base/mp2pre-selftest.XXXXXX") || exit 1
trap 'rm -rf "$w"' EXIT
nok=0
nbad=0
pass() { nok=$((nok + 1)); echo "ok: $1"; }
fail() { nbad=$((nbad + 1)); echo "FAIL: $1"; }

# a fake tree: stubs answer unless FAKE_FAIL names them
t=$w/tree
mkdir -p "$t/tools" "$t/driver/tools/uboot-mp2"
stub() { # path
  mkdir -p "$(dirname "$1")"
  printf '#!/usr/bin/env bash\ncase " ${FAKE_FAIL:-} " in *" %s "*) exit 1 ;; esac\nexit 0\n' "$(basename "$1")" >"$1"
}
for f in driver/tools/uboot-mp2/selftest.sh driver/tools/uboot-mp2/oneshot_selftest.sh \
  tools/board_stm32mp2_uboot_selftest.sh tools/mp2_tftp_preflight.sh; do stub "$t/$f"; done
for f in tools/mp2_uboot_console.py tools/uboot_env_mp2_selftest.py; do
  mkdir -p "$(dirname "$t/$f")"
  printf 'import os, sys\nsys.exit(1 if "%s" in os.environ.get("FAKE_FAIL", "").split() else 0)\n' "$(basename "$f")" >"$t/$f"
done
cat >"$t/tools/board.sh" <<'EOF'
#!/usr/bin/env bash
# fake: the dry-run verbs are answered by "the runner" unless FAKE_FAIL names them
verb=
for a in "$@"; do case $a in uboot-status | disarm | run) verb=$a ;; esac; done
case " ${FAKE_FAIL:-} " in *" board-$verb "*) exit 1 ;; esac
case " ${FAKE_FAIL:-} " in *" board-silent "*) echo "somebody else"; exit 0 ;; esac
echo "board-stm32mp2-uboot.sh stm32mp257f-dk: $verb"
EOF
cat >"$t/tools/board-stm32mp2-uboot.sh" <<'EOF'
#!/usr/bin/env bash
LOAD=0xa0000000                   # link address
LOAD_END=0xb0000000               # 256 MiB after it
MAX_IMG=$((255 * 1024 * 1024))    # refused at this size and above
EOF
chmod +x "$t"/tools/*.sh

# image: <instr-le-hex-bytes> <image_size> -> file; 64-byte header
mkimg() { # file instr_word_hex image_size
  local f=$1 iw=$2 isz=$3 i
  {
    for i in 6 4 2 0; do printf "\\x${iw:$i:2}"; done
    head -c 12 /dev/zero
    for i in 0 1 2 3 4 5 6 7; do printf "\\x$(printf %02x $(((isz >> (8 * i)) & 255)))"; done
    head -c 40 /dev/zero
  } >"$f"
}
mkimg "$w/good.bin" 14000010 4096
mkimg "$w/big.bin" 14000010 $((0x10000001))
mkimg "$w/nob.bin" d503201f 4096
head -c 8 /dev/zero >"$w/short.bin"
sha=$(sha256sum "$w/good.bin" | cut -d' ' -f1)
badsha=$(printf '%064d' 0)

# run NAME WANT_RC WANT_FAIL_PATTERN(empty: none) [env...] -- args...
run() {
  local name=$1 want=$2 pat=$3
  shift 3
  local envs=()
  while [ "$1" != -- ]; do envs+=("$1"); shift; done
  shift
  out=$(env MP2_PREFLIGHT_ROOT="$t" BOARD_HOST=h BOARD_UART=u "${envs[@]}" bash "$p" "$@" 2>&1)
  rc=$?
  last=$(printf '%s\n' "$out" | tail -n 1)
  n=$(printf '%s\n' "$last" | sed -n 's/^preflight: \([0-9]*\) ok, \([0-9]*\) failed$/\1/p')
  f=$(printf '%s\n' "$last" | sed -n 's/^preflight: \([0-9]*\) ok, \([0-9]*\) failed$/\2/p')
  lines=$(printf '%s\n' "$out" | grep -c '^preflight: \(ok\|FAIL\) ')
  good=1
  [ "$rc" = "$want" ] || good=0
  if [ "$want" != 2 ]; then
    [ -n "$n" ] && [ "$lines" = $((n + f)) ] || good=0
    if [ -z "$pat" ]; then [ "$f" = 0 ] || good=0
    else printf '%s\n' "$out" | grep -q "^preflight: FAIL .*$pat" || good=0; fi
  fi
  if [ "$good" = 1 ]; then pass "$name"; else fail "$name (rc=$rc)"; printf '%s\n' "$out"; fi
}

run all-pass 0 '' -- "$w/good.bin"
run sha-match 0 '' -- --sha256 "$sha" "$w/good.bin"
run sha-differs 1 'sha256 differs' -- --sha256 "$badsha" "$w/good.bin"
run no-image 1 'image does not exist' -- "$w/none.bin"
run too-big-size 1 'image_size does not fit' -- "$w/big.bin"
run not-branch 1 'AArch64 B instruction' -- "$w/nob.bin"
run too-short 1 'too short' -- "$w/short.bin"
run board-status 1 'uboot-status' FAKE_FAIL=board-uboot-status -- "$w/good.bin"
run board-disarm 1 'disarm' FAKE_FAIL=board-disarm -- "$w/good.bin"
run board-run 1 'run reaches' FAKE_FAIL=board-run -- "$w/good.bin"
run board-not-runner 1 'reaches the runner' FAKE_FAIL=board-silent -- "$w/good.bin"
run selftest-sh 1 'uboot-mp2/selftest.sh' FAKE_FAIL=selftest.sh -- "$w/good.bin"
run selftest-oneshot 1 'oneshot_selftest.sh' FAKE_FAIL=oneshot_selftest.sh -- "$w/good.bin"
run selftest-console 1 'mp2_uboot_console' FAKE_FAIL=mp2_uboot_console.py -- "$w/good.bin"
run selftest-runner 1 'board_stm32mp2_uboot_selftest' FAKE_FAIL=board_stm32mp2_uboot_selftest.sh -- "$w/good.bin"
run selftest-env 1 'uboot_env_mp2_selftest' FAKE_FAIL=uboot_env_mp2_selftest.py -- "$w/good.bin"
run no-host 1 'BOARD_HOST is not set' BOARD_HOST= -- "$w/good.bin"
run no-uart 1 'BOARD_UART is not set' BOARD_UART= -- "$w/good.bin"
run tftp-needs-env 1 'NETBOOT_SERVER is not set' -- --tftp "$w/good.bin"
run tftp-boot-env 1 'NETBOOT_TFTP_ROOT is not set' BOARD_BOOT=uboot-tftp NETBOOT_SERVER=s -- "$w/good.bin"
run tftp-ok 0 '' NETBOOT_SERVER=s NETBOOT_TFTP_ROOT=r -- --tftp "$w/good.bin"
run tftp-preflight-fails 1 'mp2_tftp_preflight' NETBOOT_SERVER=s NETBOOT_TFTP_ROOT=r FAKE_FAIL=mp2_tftp_preflight.sh -- --tftp "$w/good.bin"
run usage-none 2 '' --
run usage-flag 2 '' -- --bogus "$w/good.bin"
run usage-sha 2 '' -- --sha256 xyz "$w/good.bin"

# a runner whose constants are not plain arithmetic is refused, not evaluated
printf 'LOAD=0xa0000000\nLOAD_END=0xb0000000\nMAX_IMG=$(touch %s/pwned)\n' "$w" >"$t/tools/board-stm32mp2-uboot.sh"
run constants-whitelist 1 'constants' -- "$w/good.bin"
[ ! -e "$w/pwned" ] && pass constants-not-evaluated || fail constants-not-evaluated

# secrets: a value is never printed
out=$(env MP2_PREFLIGHT_ROOT="$t" BOARD_HOST=sekrit-host BOARD_UART=sekrit-uart bash "$p" "$w/good.bin" 2>&1)
printf '%s\n' "$out" | grep -q sekrit && fail "values-not-printed" || pass "values-not-printed"

echo "mp2_uboot_session_preflight_selftest: $nok ok, $nbad failed"
[ "$nbad" = 0 ]
