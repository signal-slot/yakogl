#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Is a whole-tree board record complete?
#
#   tools/cts/tree-verdicts-check.sh <tree-file> <sha8>
#
# Reads the tree table (one filter per line; a "# image LABEL:" line labels
# the filter after it, otherwise the row is img<N>, as check-gles3-tree.py and
# board.sh name them), takes every tools/cts/log/*-<set>-<sha8>* directory
# (<set> = the tree file's stem minus the board, e.g. am62p-gles3-tree.txt ->
# am62p-gles3), and for each row finds its verdict line, as board.sh prints
# it: "board.sh <board> <label> rc=N verdict=<v> ...". board.sh labels an
# unlabelled row img<N>-<TREE_TAG> (tag may be empty) when TREE_IMAGE_DIR is
# set, N counted over the whole table; when no line has the exact label
# img<N>, the lines labelled img<N>-<tag> count (img2 never matches img20-x). When a row has several
# lines (a rerun) the last one in directory-name, then file, order counts.
# Prints one line per row: "<label> <verdict|missing>", then the leaks
# (IPv4 addresses, /dev/tty*, /dev/serial* paths in any file of the
# directories) and a summary. Exit 0 only when every row is verdict=done and
# nothing leaks; 1 otherwise; 2 on usage errors.
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
logdir=$here/log
stem=$(basename "$tree" .txt)
set_name=${stem%-tree}

dirs=()
for d in "$logdir"/*"$set_name"-"$sha"*; do
  [ -d "$d" ] && dirs+=("$d")
done
echo "tree: $tree  set: $set_name  sha8: $sha  directories: ${#dirs[@]}"
for d in "${dirs[@]}"; do echo "  dir: ${d##*/}"; done

# Verdict lines of every directory, in order.
lines=$(mktemp "${TMPDIR:-/tmp}/tvc.XXXXXX") || exit 2
trap 'rm -f "$lines"' EXIT
for d in "${dirs[@]}"; do
  for f in "$d"/*; do
    [ -f "$f" ] || continue
    grep -aE '^board\.sh [^ ]+ [^ ]+ rc=[0-9-]+ verdict=' "$f" >>"$lines" 2>/dev/null
  done
done

status=$(awk '
  FILENAME == ARGV[1] {
    if ($1 == "board.sh" && !seen[$0]++) { last[$3] = $5; n[$3]++; seq[$3] = ++c }
    next
  }
  /^# image / { l = $3; sub(/:.*/, "", l); pending = l; next }
  /^#/ || /^[ \t]*$/ { next }
  {
    idx++
    explicit = (pending != "")
    lab = explicit ? pending : "img" idx
    pending = ""
    rows++
    v = last[lab]
    cnt = n[lab]
    if (v == "" && !explicit) {
      # board.sh labels an unlabelled row img<N>-<TREE_TAG> (tag may be empty)
      pre = lab "-"; best = 0
      for (k in last) {
        if (substr(k, 1, length(pre)) == pre) {
          cnt += n[k]
          if (seq[k] > best) { best = seq[k]; v = last[k] }
        }
      }
    }
    if (v == "") { v = "missing"; miss++ }
    else { sub(/^verdict=/, "", v); if (v != "done") bad++ }
    extra = (cnt > 1) ? "  (" cnt " lines, last counts)" : ""
    printf "%s %s%s\n", lab, v, extra
    if (v == "done") ok++
  }
  END { printf "rows: %d  done: %d  missing: %d  not done: %d\n", rows, ok, miss, bad }
' "$lines" "$tree")
printf '%s\n' "$status"
summary_ok=0
case $status in
  *"rows: "*"  done: "*) ;;
esac
tail1=$(printf '%s\n' "$status" | tail -n 1)
rows_n=$(printf '%s\n' "$tail1" | awk '{print $2}')
done_n=$(printf '%s\n' "$tail1" | awk '{print $4}')
[ "${rows_n:-0}" -gt 0 ] && [ "$rows_n" = "$done_n" ] && summary_ok=1

leaks=0
if [ ${#dirs[@]} -gt 0 ]; then
  out=$(grep -rIEn '([0-9]{1,3}\.){3}[0-9]{1,3}|/dev/(tty|serial|cu\.)[A-Za-z0-9_./-]*' "${dirs[@]}" 2>/dev/null | cut -c1-160)
  if [ -n "$out" ]; then
    leaks=$(printf '%s\n' "$out" | wc -l)
    echo "leaks: $leaks line(s) with an IPv4 address or a tty/serial path:"
    printf '%s\n' "$out" | head -n 20 | sed 's/^/  /'
  else
    echo "leaks: none"
  fi
fi

if [ "$summary_ok" = 1 ] && [ "$leaks" = 0 ]; then
  echo "tree-verdicts-check: OK"
  exit 0
fi
echo "tree-verdicts-check: FAIL"
exit 1
