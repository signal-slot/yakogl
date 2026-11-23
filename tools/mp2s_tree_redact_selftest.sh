#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# mp2s_tree_redact_selftest.sh - no committed DK standalone file carries a
# board address, host name, UART path or serial (plan #362, item
# tree-redact-selftest).
#
#   tools/mp2s_tree_redact_selftest.sh [-v]
#
# Runs tools/mp2s_redact_check.py over every tracked file under the DK
# standalone paths (see PATHSPECS). Binary files are scanned as strings.
# A finding fails the run unless its path:line is in ALLOW below (fixtures
# whose job is to hold fake secrets, e.g. the redaction selftests' inputs).
# A real leak is never edited here: the run fails and prints path:line (the
# match itself is masked by the checker) for a new issue.
#
# Self-check: a fake CGNAT-range address put into a temp copy of a scanned
# file must be found.
#
# Summary: "mp2s_tree_redact_selftest: N files, F findings". Exit 0 clean,
# 1 findings, 2 setup error.
set -u
cd "$(dirname "$0")/.." || exit 2
CHK=tools/mp2s_redact_check.py
[ -f "$CHK" ] || { echo "mp2s_tree_redact_selftest: no $CHK" >&2; exit 2; }
command -v git >/dev/null || exit 2
WORK=$(mktemp -d "${TMPDIR:-/var/tmp}/mp2s_tree_redact.XXXXXX") || exit 2
trap 'rm -rf "$WORK"' EXIT

PATHSPECS=(
  docs/notes/stm32mp257f-dk-standalone.md
  'driver/tools/mp2-standalone/**'
  'tools/mp2s_*'
  tools/board-stm32mp2-standalone.sh
  tools/board_stm32mp2_standalone_selftest.sh
  'tools/testdata/mp2s-*/**'
  'tools/fwu_metadata*'
  'tools/efi_varfile*'
)

# Allow list: "path :: reason", one path per line. Every finding in an
# allowed path is accepted; any other path with a finding fails. All of
# these were read by hand: none is a real board identity.
ALLOW=$(cat <<'EOF'
driver/tools/mp2-standalone/st-env/stm32mp25-dk-initial-env.txt :: ST stock default serverip, a public default, not a lab address
driver/tools/mp2-standalone/qemu-product/README.md :: public upstream URL source.denx.de
driver/tools/mp2-standalone/qemu-card/README.md :: public upstream URL source.denx.de
driver/tools/mp2-standalone/qemu/README.md :: public upstream URL source.denx.de
driver/tools/mp2-standalone/qemu/build-uboot.sh :: public upstream URL source.denx.de
tools/mp2s_fakeboard_selftest.sh :: the literal ts.net as a search needle
tools/mp2s_tree_redact_selftest.sh :: this file: its allow list quotes the needles, its self-check a fake CGNAT address
tools/mp2s_redact_check.py :: the checker's own docstring names /dev/tty patterns
tools/mp2s_redact_check_selftest.sh :: deliberate fake secrets, the redaction selftest's inputs
tools/mp2s_report.py :: docstring names /dev/tty patterns
tools/mp2s_report_selftest.sh :: deliberate fake address, tty, serial and host inputs of the report redaction test
tools/mp2s_env_diff_selftest.sh :: fake documentation-range address, MAC and serial# inputs
tools/testdata/mp2s-env/both-bad.bin :: ST stock env serverip default
tools/testdata/mp2s-env/counting1.bin :: ST stock env serverip default
tools/testdata/mp2s-env/counting2.bin :: ST stock env serverip default
tools/testdata/mp2s-env/installed.bin :: ST stock env serverip default
tools/testdata/mp2s-env/left0.bin :: ST stock env serverip default
tools/testdata/mp2s-env/one-copy-bad.bin :: ST stock env serverip default
tools/testdata/mp2s-env/pristine.bin :: ST stock env serverip default
EOF
)

mapfile -t FILES < <(git ls-files -- "${PATHSPECS[@]}")
n=${#FILES[@]}
[ "$n" -gt 0 ] || { echo "mp2s_tree_redact_selftest: no files matched" >&2; exit 2; }

# Text view of every file: text as is, binary as `strings`.
SCAN="$WORK/scan"
mkdir -p "$SCAN"
i=0
for f in "${FILES[@]}"; do
  [ -f "$f" ] || continue
  i=$((i + 1))
  d="$SCAN/$i"; mkdir -p "$d"
  if grep -Iq . "$f" 2>/dev/null || [ ! -s "$f" ]; then
    cp "$f" "$d/f"
  else
    strings -n 4 "$f" > "$d/f"
  fi
  printf '%s\n' "$f" > "$d/name"
done

# scan <dir-of-numbered-copies> -> lines "path:line:kind:masked"
scan() {
  local d name
  for d in "$1"/*/; do
    name=$(cat "$d/name")
    python3 -I "$CHK" "$d/f" 2>/dev/null | sed "s|^[^:]*/f:|$name:|"
  done
}

findings=0
allowed=0
while IFS= read -r line; do
  [ -n "$line" ] || continue
  p=${line%%:*}
  if printf '%s\n' "$ALLOW" | sed 's/ :: .*//' | grep -qxF -- "$p"; then
    allowed=$((allowed + 1))
    continue
  fi
  findings=$((findings + 1))
  echo "FINDING $line"
done < <(scan "$SCAN")

# Self-check: a fake CGNAT address in a temp copy must be caught.
SELF="$WORK/self"; mkdir -p "$SELF/1"
cp "${FILES[0]}" "$SELF/1/f"
printf '\nhost %s here\n' "100.64.$((12)).34" >> "$SELF/1/f"
printf '%s\n' "${FILES[0]}" > "$SELF/1/name"
if scan "$SELF" | grep -q ':ipv4:'; then
  echo "self-check ok: fake CGNAT-range address is found"
else
  echo "self-check FAILED: fake CGNAT-range address not found"
  findings=$((findings + 1))
fi

[ "${1:-}" = -v ] && echo "allowed by list: $allowed"
echo "mp2s_tree_redact_selftest: $n files, $findings findings"
[ "$findings" -eq 0 ]
