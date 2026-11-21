#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Checks the DK's dEQP-GLES2 case list (P9):
#  - mp2-deqp-gles2.txt: no duplicate line, every line is a group of the
#    i.MX8MP's list (imx8mp-full-tree.txt) or a case under one, and every
#    i.MX8MP group is listed;
#  - mp2-deqp-gles2-exclude.txt: every line "<case|group.*><TAB><reason>" has
#    a non-empty reason, no duplicate, the name is under a listed group of the
#    i.MX8MP list and matches a case of gles2-mustpass.txt.
# Prints the count: mustpass cases of the list minus the exclusions.
# usage: sh tools/cts/mp2-deqp-list-check.sh
set -eu
d=$(cd "$(dirname "$0")" && pwd)
exec awk -v FS='\t' -v tree="$d/imx8mp-full-tree.txt" -v must="$d/gles2-mustpass.txt" \
  -v list="$d/mp2-deqp-gles2.txt" -v excl="$d/mp2-deqp-gles2-exclude.txt" '
function pre(p) { sub(/\*$/, "", p); return p }
# is name (a case or a group glob) the group g or under it
function under(name, g) {
  if (g == name) return 1
  return substr(g, length(g)) == "*" && index(name, pre(g)) == 1
}
function covered(name, arr, n,   i) {
  for (i = 1; i <= n; i++) if (under(name, arr[i])) return 1
  return 0
}
function bad(m) { print "mp2-deqp-list-check: " m; fail = 1 }
BEGIN {
  while ((getline l < tree) > 0) {
    if (l ~ /^#/ || l == "") continue
    k = split(l, a, ","); for (i = 1; i <= k; i++) { ng++; g[ng] = a[i] }
  }
  while ((getline l < must) > 0) if (l !~ /^#/ && l != "") { nm++; m[nm] = l }
  while ((getline l < list) > 0) {
    if (l ~ /^#/ || l == "") continue
    if (l in seen) bad("duplicate list line: " l)
    seen[l] = 1; nl++; ls[nl] = l
    if (!covered(l, g, ng)) bad("not in the i.MX8MP list: " l)
  }
  for (i = 1; i <= ng; i++) if (!(g[i] in seen)) bad("i.MX8MP group missing: " g[i])
  while ((getline l < excl) > 0) {
    if (l ~ /^#/ || l == "") continue
    n = split(l, f, "\t"); name = f[1]; reason = f[2]
    if (n < 2 || reason ~ /^ *$/) bad("no reason: " name)
    if (name in ex) bad("duplicate exclude: " name)
    ex[name] = 1; nx++; xs[nx] = name
    if (!covered(name, g, ng)) bad("excluded name not in the i.MX8MP list: " name)
    if (!covered(name, ls, nl)) bad("excluded name not under a listed group: " name)
    hit = 0
    for (i = 1; i <= nm; i++) if (under(m[i], name)) { hit = 1; break }
    if (!hit) bad("excluded name matches no mustpass case: " name)
  }
  cnt = 0
  for (i = 1; i <= nm; i++) {
    if (!covered(m[i], ls, nl)) continue
    if (covered(m[i], xs, nx)) continue
    cnt++
  }
  if (fail) exit 1
  print "mp2-deqp-list-check: ok, " nl " groups, " nx " excluded, " cnt " mustpass cases to run"
}'
