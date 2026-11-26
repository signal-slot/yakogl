#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Acceptance c3 of plan #405 (the DK's P9) as one command: the committed
# dEQP-GLES2 shard logs against the summary line quoted in the P9 note.
#
#   tools/cts/mp2-p9-c3-check.sh [--notes FILE] [--diff-opt OPT]... [LOG...]
#
# Runs `python3 -I tools/cts/mp2-p9-diff.py --report-only [OPT...] LOG...`
# (LOG default: tools/cts/log/*-mp2-gles2-*/s*/shard-*.log, from the
# repository root), takes its last line ("mp2-p9-diff: shared S, new N,
# fixed F, missing M[, excluded E]") and compares it with the last such line
# quoted in the notes file (default docs/notes/stm32mp257f-dk-p9.md; each
# slice replaces the quoted line, so the last one is the current one). When M
# is not 0, every name the diff lists under "mustpass names missing from the
# DK run" must be on a line of the notes file that also says "cause" (any
# case).
#
# Prints the diff's line, the quoted line, then one verdict line, last:
#   mp2-p9-c3: ok
#   mp2-p9-c3: failed: <what>
# Exit 0 on ok, 1 on failed (no log, no quoted line, the lines differ, a
# missing case without its cause, or the diff did not run), 2 on a usage
# error. The same check is c3 of tools/cts/mp2-p9-notes-check.py (the whole
# acceptance set); this is c3 alone, with the lines shown. Self-test:
# tools/cts/mp2_p9_c3_check_selftest.sh. No board.
set -u

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
diff_tool=$here/mp2-p9-diff.py
notes=$root/docs/notes/stm32mp257f-dk-p9.md
opts=()

usage() {
  echo "usage: $0 [--notes FILE] [--diff-opt OPT]... [LOG...]" >&2
  exit 2
}
failed() {
  echo "mp2-p9-c3: failed: $1"
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
  --notes) [ $# -ge 2 ] || usage; notes=$2; shift 2 ;;
  --diff-opt) [ $# -ge 2 ] || usage; opts+=("$2"); shift 2 ;;
  -h | --help) usage ;;
  --) shift; break ;;
  -*) echo "mp2-p9-c3: unknown option: $1" >&2; usage ;;
  *) break ;;
  esac
done

if [ $# -gt 0 ]; then
  logs=("$@")
else
  shopt -s nullglob
  logs=("$root"/tools/cts/log/*-mp2-gles2-*/s*/shard-*.log)
  shopt -u nullglob
fi
[ -r "$notes" ] || { echo "mp2-p9-c3: cannot read the notes file: $notes" >&2; exit 2; }
[ ${#logs[@]} -gt 0 ] || failed "no dEQP shard log (tools/cts/log/*-mp2-gles2-*/s*/shard-*.log)"
for l in "${logs[@]}"; do
  [ -r "$l" ] || failed "cannot read the shard log: $l"
done

summary='mp2-p9-diff: shared [0-9]+, new [0-9]+, fixed [0-9]+, missing [0-9]+(, excluded [0-9]+)?'
out=$(python3 -I "$diff_tool" --report-only "${opts[@]+"${opts[@]}"}" "${logs[@]}" 2>&1)
rc=$?
last=$(printf '%s\n' "$out" | tail -n 1)
[ "$rc" -eq 0 ] || failed "mp2-p9-diff.py exited $rc: $last"
printf '%s\n' "$last" | grep -Eqx "$summary" || failed "unexpected last line of mp2-p9-diff.py: $last"
# The longest match on each line, so a quoted ", excluded E" is kept.
quoted=$(grep -Eo "$summary" "$notes" | tail -n 1)
echo "diff:   $last"
echo "quoted: ${quoted:-(none)}"
[ -n "$quoted" ] || failed "no 'mp2-p9-diff: shared ...' line quoted in $notes"
[ "$quoted" = "$last" ] || failed "$notes quotes '$quoted' but the shard logs give '$last'"

missing=$(printf '%s\n' "$last" | sed -E 's/.*missing ([0-9]+).*/\1/')
if [ "$missing" -ne 0 ]; then
  bare=$(printf '%s\n' "$out" | awk -v notes="$notes" '
    BEGIN { while ((getline l < notes) > 0) if (tolower(l) ~ /cause/) c[++n] = l }
    /^== / { inside = index($0, "missing from the DK run") > 0; next }
    /^mp2-p9-diff:/ { next }
    inside && NF {
      for (i = 1; i <= n; i++) if (index(c[i], $0)) next
      print
    }')
  if [ -n "$bare" ]; then
    k=$(printf '%s\n' "$bare" | grep -c .)
    failed "missing $missing: $k case(s) without a notes line naming it and its cause, first $(printf '%s\n' "$bare" | head -n 1)"
  fi
fi
echo "mp2-p9-c3: ok"
