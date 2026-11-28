#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Add up a whole tree's per-case statuses across the slice directories of one commit.
#
#   tools/cts/tree-summary.sh <tree-file> <sha8>
#
# Reads the tree table as tree-verdicts-check.sh does (one filter list per line;
# a "# image LABEL:" line labels the row after it, otherwise img<N>), takes every
# tools/cts/log/*<set>-<sha8>* directory (<set> = the tree file's stem minus
# "-tree", e.g. am62p-gles3), and from them:
#   - the verdict lines "board.sh <board> <label> rc=N verdict=<v> ... resets=<n|absent>"
#     found in any file (the last line of a row counts), and
#   - the per-case statuses of each directory's cases.tsv (<case> TAB <status>
#     [TAB message]; '#' lines skipped; see log/README.md).
# A case is attributed to the first row whose filter (comma list of globs, '*'
# only) selects it. A case listed twice is counted once (first status) and
# reported as a warning, a conflicting status as well. CompatibilityWarning
# counts as QualityWarning (as am62p-gles3-merge.py does); any other status
# word is "other"; the status NoResult counts as no result.
#
# Cases without a result: the mustpass cases (MUSTPASS, default
# tools/cts/gles3-mustpass.txt next to this script) selected by a row that has a
# verdict and that no cases.tsv lists, plus the NoResult lines. Without a
# readable mustpass file only the NoResult lines are counted (said in the block).
#
# LOGDIR (default tools/cts/log) and MUSTPASS override the two inputs, for tests.
# Prints the block, then the per-row table. Exit 0 when every row has a verdict
# line, 1 when a row is missing (or no directory/cases were found), 2 on usage.
set -u

if [ $# -ne 2 ]; then
  echo "usage: ${0##*/} <tree-file> <sha8>" >&2
  exit 2
fi
tree=$1
sha=$2
if [ ! -r "$tree" ]; then
  echo "${0##*/}: cannot read $tree" >&2
  exit 2
fi
case $sha in
  [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]) ;;
  *) echo "${0##*/}: sha8 must be 8 lowercase hex digits: $sha" >&2; exit 2 ;;
esac

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
logdir=${LOGDIR:-$here/log}
stem=$(basename "$tree" .txt)
set_name=${stem%-tree}
mustpass=${MUSTPASS:-$here/gles3-mustpass.txt}
[ -r "$mustpass" ] || mustpass=/dev/null

dirs=()
for d in "$logdir"/*"$set_name"-"$sha"*; do
  [ -d "$d" ] && dirs+=("$d")
done

tmp=$(mktemp -d "${TMPDIR:-/tmp}/tsum.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT
: >"$tmp/verdicts"
: >"$tmp/cases"
for d in "${dirs[@]}"; do
  for f in "$d"/*; do
    [ -f "$f" ] || continue
    grep -aE '^board\.sh [^ ]+ [^ ]+ rc=[0-9-]+ verdict=' "$f" >>"$tmp/verdicts" 2>/dev/null
  done
  if [ -r "$d/cases.tsv" ]; then
    grep -av '^#' "$d/cases.tsv" | awk -F'\t' -v d="${d##*/}" 'NF >= 2 { print $1 "\t" $2 "\t" d }' >>"$tmp/cases"
  fi
done

awk -v sha="$sha" -v ndirs="${#dirs[@]}" -v haveMust="$([ "$mustpass" = /dev/null ] && echo 0 || echo 1)" '
  function re_of(g,   r, i, c) {
    r = "^"
    for (i = 1; i <= length(g); i++) {
      c = substr(g, i, 1)
      if (c == "*") r = r ".*"
      else if (index(".[]\\^$(){}+?|", c)) r = r "\\" c
      else r = r c
    }
    return r "$"
  }
  function row_of(cs,   k, L, best, p) {
    if (cs in memo) return memo[cs]
    best = 0
    if ((cs in exact) && (!best || exact[cs] < best)) best = exact[cs]
    for (L in plens) {
      p = substr(cs, 1, L)
      if ((p in prefix) && (!best || prefix[p] < best)) best = prefix[p]
    }
    for (k = 1; k <= nre; k++) if ((!best || prow[k] < best) && cs ~ pre[k]) best = prow[k]
    memo[cs] = best
    return best
  }
  function add_pat(g, r,   body) {
    if (index(g, "*") == 0) { if (!(g in exact) || exact[g] > r) exact[g] = r }
    else if (substr(g, length(g)) == "*" && index(substr(g, 1, length(g) - 1), "*") == 0) {
      body = substr(g, 1, length(g) - 1)
      if (!(body in prefix) || prefix[body] > r) prefix[body] = r
      plens[length(body)] = 1
    } else { nre++; pre[nre] = re_of(g); prow[nre] = r }
  }
  FILENAME == ARGV[1] {                      # tree
    if ($0 ~ /^# image /) { l = $3; sub(/:.*/, "", l); pending = l; next }
    if ($0 ~ /^#/ || $0 ~ /^[ \t]*$/) next
    rows++
    lab[rows] = (pending != "") ? pending : "img" rows
    pending = ""
    n = split($0, g, ",")
    for (i = 1; i <= n; i++) if (g[i] != "") add_pat(g[i], rows)
    next
  }
  FILENAME == ARGV[2] {                      # verdict lines
    key = $3
    if ($0 in seenv) next
    seenv[$0] = 1
    vline[key] = $0; hasv[key] = 1
    next
  }
  FILENAME == ARGV[3] {                      # mustpass
    if ($0 == "" || $0 ~ /^#/) next
    if (mseen[$1]++) next
    mrow[$1] = row_of($1)
    next
  }
  {                                          # cases: case TAB status TAB dir
    c = $1; s = $2
    if (c in st) {
      dup++
      if (st[c] != s) { conflict++; if (nconf++ < 5) cmsg = cmsg "  conflict: " c " " st[c] " vs " s "\n" }
      else if (ndup++ < 5) cmsg = cmsg "  twice: " c "\n"
      next
    }
    st[c] = s
    r = row_of(c)
    if (s == "Pass") k = "p"
    else if (s == "Fail") k = "f"
    else if (s == "NotSupported") k = "n"
    else if (s == "QualityWarning" || s == "CompatibilityWarning") k = "q"
    else if (s == "NoResult") k = "z"
    else k = "o"
    tot[k]++
    if (r == 0) stray++
    else cnt[r, k]++
  }
  END {
    ok = 0; resets = 0
    for (r = 1; r <= rows; r++) {
      v = "missing"; rs = ""
      if (lab[r] in hasv) {
        ok++
        line = vline[lab[r]]
        v = line; sub(/.* verdict=/, "", v); sub(/ .*/, "", v)
        rs = line; if (rs ~ / resets=/) { sub(/.* resets=/, "", rs); sub(/ .*/, "", rs) } else rs = ""
        if (rs ~ /^[0-9]+$/) resets += rs
        img = line; sub(/^board\.sh [^ ]+ [^ ]+ /, "", img)
      }
      rv[r] = v; rr[r] = (rs == "" ? "-" : rs)
    }
    # cases without a result: mustpass cases of rows with a verdict, absent from every cases.tsv
    for (c in mseen) {
      if (c in st) continue
      r = mrow[c]
      if (r > 0 && (lab[r] in hasv)) { cnt[r, "z"]++; tot["z"]++; derived++ }
    }
    printf "commit: %s\n", sha
    printf "directories: %d\n", ndirs
    printf "rows with a verdict: %d of %d\n", ok, rows
    printf "Pass: %d\nFail: %d\nNotSupported: %d\nQualityWarning: %d\nother: %d\n", tot["p"], tot["f"], tot["n"], tot["q"], tot["o"]
    printf "no result: %d%s\n", tot["z"], (haveMust ? "" : "  (no mustpass list: NoResult lines only)")
    printf "cases listed: %d  (outside every row filter: %d)\n", tot["p"]+tot["f"]+tot["n"]+tot["q"]+tot["o"]+tot["z"]-derived, stray
    printf "resets: %d\n", resets
    if (dup) printf "warning: %d case(s) listed twice, counted once (%d with a conflicting status)\n%s", dup, conflict, cmsg
    printf "\n%-10s %-6s %7s %7s %7s %7s %7s  %s\n", "row", "image", "pass", "fail", "NS", "QW", "none", "verdict"
    for (r = 1; r <= rows; r++)
      printf "%-10s %-6s %7d %7d %7d %7d %7d  %s%s\n", lab[r], r, cnt[r,"p"], cnt[r,"f"], cnt[r,"n"], cnt[r,"q"], cnt[r,"z"], rv[r], (rv[r] != "missing" && rr[r] != "-" && rr[r] != "absent" && rr[r] != "0" ? " (resets=" rr[r] ")" : "")
    miss = rows - ok
    if (miss > 0) printf "\ntree-summary: FAIL: %d row(s) without a verdict\n", miss
    exit (miss > 0 || rows == 0) ? 1 : 0
  }
' "$tree" "$tmp/verdicts" "$mustpass" "$tmp/cases"
