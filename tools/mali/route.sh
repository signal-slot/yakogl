#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The Mali compiler measured on the route the product takes: each program
# through the back end's entry points in the GL core's order (compile of
# each stage, compile_vs_linked, link), compared with its Mesa 26.2.1
# drm-shim reference by the rule the integration tests use
# (tests/mali_ref/mod.rs `measure_list`, run by tests/mali_ref_selftest.rs
# `the_measure_reads_a_list`; tools/mali/corpus_check.py runs the same
# measure over the dEQP and glmark2 corpora). A uniform word is read as
# tests/mali_ref reads it (issue #8783); a reference is equal when it is
# equal under --uniform-values or with every uniform 0 (rule `either`: the
# directory holds references of both kinds).
#
#   tools/mali/route.sh measure <dir> [--jobs N] > <out>.tsv
#       every <name>.vert beside a <name>.frag in <dir>, against
#       <name>.expect.json when <dir> has it; one TSV row each, sorted:
#       <name> EQUAL|DIFFER|REFUSED|NOREF <digest> <detail>
#       (the digest: the compiled words and facts, `-` when refused; the
#       detail: the first difference, or the refusing step, its return code
#       and its log). N test processes at once (default 8).
#   tools/mali/route.sh compare <main>.tsv <tip>.tsv
#       the two landing rules, main -> tip (tools/mali/route_cells.py):
#       exit 1 when one is broken or a moved program has no reference.
#
# The two TSVs of a branch are measured from two trees (main's and the
# tip's), each with this script of its own tree, over the same directory.
set -euo pipefail
top=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
features=host-test,gpu-mali,shaderc-online,no-shader-table
test=the_measure_reads_a_list
usage() {
  echo "usage: $0 measure <dir> [--jobs N] | compare <main.tsv> <tip.tsv>" >&2
  exit 2
}
case "${1-}" in
  measure)
    [ $# -ge 2 ] || usage
    dir=$(cd "$2" && pwd) || usage
    jobs=8
    if [ $# -ge 3 ]; then
      [ "$3" = --jobs ] && [ $# -eq 4 ] || usage
      jobs=$4
    fi
    [ "$jobs" -ge 1 ] 2>/dev/null || usage
    pairs=$(find "$dir" -maxdepth 1 -name '*.vert' | while read -r v; do if [ -f "${v%.vert}.frag" ]; then echo; fi; done | wc -l)
    # an empty measure would read as "no rule broken" in compare
    [ "$pairs" -gt 0 ] || { echo "route.sh: no <name>.vert beside a <name>.frag in $dir" >&2; exit 1; }
    cd "$top"
    out=$(mktemp -d)
    trap 'rm -rf "$out"' EXIT
    # the list: <name> <vs> <fs> <expect.json or -> either, sorted by name
    find "$dir" -maxdepth 1 -name '*.vert' | LC_ALL=C sort | while read -r v; do
      n=${v%.vert}
      [ -f "$n.frag" ] || continue
      j=-
      [ -f "$n.expect.json" ] && j=$n.expect.json
      printf '%s\t%s\t%s\t%s\teither\n' "$(basename "$n")" "$v" "$n.frag" "$j"
    done > "$out/list"
    # one build, then the shares in parallel (one program at a time in a
    # process: one process per share is what runs them at once)
    cargo test -q --no-default-features --features "$features" --test mali_ref_selftest --no-run >&2
    # share k: its lines, through as many processes as it takes: a program
    # that kills the process (a compiler stack overflow, tests/
    # mali_deqp_defects.rs `lanecycle`) is REFUSED with the signal, and the
    # rest of the share goes on in a fresh process
    share() {
      local k=$1 list=$out/list.$k left rc dead
      awk -v k="$k" -v n="$jobs" '(NR - 1) % n == k' "$out/list" > "$list"
      : > "$out/$k"
      while [ -s "$list" ]; do
        rc=0
        YAKOGL_MALI_MEASURE_LIST=$list cargo test -q --no-default-features --features "$features" \
          --test mali_ref_selftest -- --exact "$test" --nocapture --test-threads 1 >"$out/$k.run" 2>"$out/$k.err" || rc=$?
        cat "$out/$k.run" >> "$out/$k"
        [ "$rc" -ne 0 ] || break
        dead=$(awk -F'\t' '$1 == "begin" {b = $2} $1 == "measure" {b = ""} END {print b}' "$out/$k.run")
        [ -n "$dead" ] || { tail -n 5 "$out/$k.err" >&2; return 1; }
        printf 'measure\t%s\tREFUSED\t-\tcrash: the measure process exited %s\n' "$dead" "$rc" >> "$out/$k"
        left=$(awk -F'\t' 'FNR == NR {if ($1 == "measure") done[$2] = 1; next} !($1 in done)' "$out/$k" "$list")
        printf '%s\n' "$left" | grep . > "$list" || : > "$list"
      done
    }
    pids=()
    for k in $(seq 0 $((jobs - 1))); do
      share "$k" &
      pids+=($!)
    done
    rc=0
    for k in "${!pids[@]}"; do
      wait "${pids[$k]}" || { rc=1; echo "route.sh: share $k of the measure failed" >&2; }
    done
    [ "$rc" -eq 0 ] || exit 1
    rows=$(for k in $(seq 0 $((jobs - 1))); do cat "$out/$k"; done | sed -n 's/^measure\t//p' | cut -f1-4 | LC_ALL=C sort)
    [ "$(printf '%s\n' "$rows" | grep -c .)" -eq "$pairs" ] ||
      { echo "route.sh: $pairs pairs in $dir, $(printf '%s\n' "$rows" | grep -c .) rows measured" >&2; exit 1; }
    printf '%s\n' "$rows"
    ;;
  compare)
    [ $# -eq 3 ] || usage
    exec python3 -I "$top/tools/mali/route_cells.py" "$2" "$3"
    ;;
  *) usage ;;
esac
