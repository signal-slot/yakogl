#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# check71.sh - the host checks of the V3D 7.1 (Raspberry Pi 5) work: runs every
# executable tools/v3d/check71.d/*.sh in sorted order, from the repository
# root, and prints one line per script (status, name, its last line). The
# convention a script follows is tools/v3d/check71.d/README.md.
#
# Exit status: 0 when every script passed; 77 and "gap: <names>" when some
# script exited 77 (an input is missing) and none failed; 1 when any failed
# (a gap does not hide a failure).
set -u
cd "$(dirname "$0")/../.." || exit 1

shopt -s nullglob
pass=0
failed=()
gaps=()
for s in tools/v3d/check71.d/*.sh; do
  [ -x "$s" ] || continue
  name=$(basename "$s" .sh)
  out=$("$s" 2>&1)
  status=$?
  last=$(printf '%s\n' "$out" | tail -n 1)
  case $status in
  0)
    pass=$((pass + 1))
    echo "ok      $name: $last"
    ;;
  77)
    gaps+=("$name")
    echo "gap     $name: $last"
    ;;
  *)
    failed+=("$name")
    echo "FAILED  $name (exit $status): $last"
    printf '%s\n' "$out" | tail -n 20 | sed 's/^/    /'
    ;;
  esac
done

echo "check71: $pass ok, ${#failed[@]} failed, ${#gaps[@]} gap(s)"
if [ "${#failed[@]}" -gt 0 ]; then
  echo "failed: ${failed[*]}"
  exit 1
fi
if [ "${#gaps[@]}" -gt 0 ]; then
  echo "gap: ${gaps[*]}"
  exit 77
fi
exit 0
