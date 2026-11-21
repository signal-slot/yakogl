#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host self-test of ref71.sh's refusals: a missing knob exits 3 and names it.
# Needs no Mesa tree and no glmark2 checkout.
set -u
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/ref71_selftest.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
fail=0
ok() { echo "ok: $1"; }
bad() { echo "FAIL: $1"; fail=1; }

printf '#!/bin/sh\nexit 0\n' > "$tmp/shc"
chmod +x "$tmp/shc"
mkdir "$tmp/src" "$tmp/corpus"

check() { # <name> <want rc> <want text> <output> <rc>
  if [ "$5" -eq "$2" ] && grep -q -- "$3" <<<"$4"; then ok "$1"; else bad "$1 (rc $5: $4)"; fi
}

out=$(env -u V3D_SHC -u GLMARK2_SRC bash "$here/ref71.sh" "$tmp/corpus" "$tmp/o.tsv" 2>&1)
check "no V3D_SHC: exit 3 naming it" 3 "gap: V3D_SHC" "$out" $?

out=$(env -u GLMARK2_SRC V3D_SHC="$tmp/shc" bash "$here/ref71.sh" "$tmp/corpus" "$tmp/o.tsv" 2>&1)
check "no GLMARK2_SRC: exit 3 naming it" 3 "gap: GLMARK2_SRC" "$out" $?

out=$(V3D_SHC="$tmp/shc" GLMARK2_SRC="$tmp/src" bash "$here/ref71.sh" "$tmp/corpus" "$tmp/o.tsv" 2>&1)
check "no index.txt: exit 3" 3 "index.txt" "$out" $?

out=$(bash "$here/ref71.sh" 2>&1)
check "no arguments: usage, exit 2" 2 "Usage" "$out" $?
exit $fail
