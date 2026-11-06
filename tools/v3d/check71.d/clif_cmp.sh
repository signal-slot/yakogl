#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# clif_cmp: self-test of tools/v3d/clif_cmp.py on tools/v3d/shim71/clear.clif:
# against itself it is equal; a copy with one field changed is found, and is
# equal again with that field ignored; a copy with a packet removed is found;
# a copy with only a buffer address changed is found and is equal with
# --ignore-addresses. Host only, no Mesa input. Copies are made in
# build/v3d71/clif_cmp/. Convention: tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=clif_cmp
dir=build/v3d71/$name
mkdir -p "$dir" || exit 1
cmp_py=tools/v3d/clif_cmp.py
ref=tools/v3d/shim71/clear.clif
tri=tools/v3d/shim71/triangle.clif
fails=0
n=0

# expect <want status> <label> <args...>; the output is $dir/<n>.out
expect() {
  local want=$1 label=$2 rc
  shift 2
  n=$((n + 1))
  python3 -I "$cmp_py" "$@" >"$dir/$n.out" 2>&1
  rc=$?
  if [ "$rc" -ne "$want" ] || ! tail -n 1 "$dir/$n.out" | grep -Eq '^clif_cmp: [0-9]+ packets, [0-9]+ differences$'; then
    echo "FAIL: $label: exit $rc, want $want"
    sed 's/^/  /' "$dir/$n.out"
    fails=$((fails + 1))
  fi
}

# names <label> <text>: the last case's output has a line containing <text>
names() {
  grep -qF -- "$2" "$dir/$n.out" || {
    echo "FAIL: $1: output lacks '$2'"
    sed 's/^/  /' "$dir/$n.out"
    fails=$((fails + 1))
  }
}

sed '/Width (in pixels)/s/ 64$/ 65/' "$ref" >"$dir/field.clif"
sed '0,/^MESA: info: FLUSH$/{/^MESA: info: FLUSH$/d}' "$ref" >"$dir/missing.clif"
sed '0,/\[CL_0xe000+0x00000000\]/s//[CL_0xe000+0x00000040]/' "$ref" >"$dir/addr.clif"
for f in field missing addr; do
  if cmp -s "$ref" "$dir/$f.clif"; then
    echo "FAIL: $f.clif was not changed"
    fails=$((fails + 1))
  fi
done

expect 0 "clear against itself" "$ref" "$ref"
expect 1 "changed field found" "$ref" "$dir/field.clif"
names "changed field" 'TILE_BINNING_MODE_CFG.Width (in pixels): 64 != 65'
expect 0 "changed field ignored" "$ref" "$dir/field.clif" --ignore '*.Width (in pixels)'
expect 1 "other field ignored" "$ref" "$dir/field.clif" --ignore 'STORE.*'
expect 1 "packet missing found" "$ref" "$dir/missing.clif"
names "missing packet" 'FLUSH: present != absent'
expect 1 "address change found" "$ref" "$dir/addr.clif"
names "address change" 'GENERIC_TILE_LIST.start'
expect 0 "address change ignored" "$ref" "$dir/addr.clif" --ignore-addresses
expect 1 "addresses ignored, field changed" "$ref" "$dir/field.clif" --ignore-addresses
expect 0 "triangle against itself" "$tri" "$tri"
expect 1 "clear against triangle" "$ref" "$tri"

if [ "$fails" -ne 0 ]; then
  echo "$name: $fails of $n cases did not behave"
  exit 1
fi
echo "$name: $n cases, 0 failed"
