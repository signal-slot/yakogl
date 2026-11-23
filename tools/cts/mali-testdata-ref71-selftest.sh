#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host self-test of mali-testdata-ref71.sh's refusals: a missing or unusable
# v3d_shc exits 3 and names V3D_SHC; an unknown option is usage, exit 2.
# Needs no Mesa tree.
set -u
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mali_ref71_selftest.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
fail=0
ok() { echo "ok: $1"; }
bad() { echo "FAIL: $1"; fail=1; }
check() { # <name> <want rc> <want text> <output> <rc>
  if [ "$5" -eq "$2" ] && grep -q -- "$3" <<<"$4"; then ok "$1"; else bad "$1 (rc $5: $4)"; fi
}

out=$(env -u V3D_SHC bash "$here/mali-testdata-ref71.sh" --out "$tmp/o.tsv" 2>&1)
check "no v3d_shc: exit 3 naming V3D_SHC" 3 "gap: V3D_SHC" "$out" $?

out=$(env -u V3D_SHC bash "$here/mali-testdata-ref71.sh" --shc "$tmp/nope" --out "$tmp/o.tsv" 2>&1)
check "--shc not executable: exit 3" 3 "gap: V3D_SHC" "$out" $?

out=$(bash "$here/mali-testdata-ref71.sh" --bogus 2>&1)
check "unknown option: usage, exit 2" 2 "Usage" "$out" $?
exit $fail
