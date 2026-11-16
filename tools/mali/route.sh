#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The Mali compiler measured on the route the product takes: each program
# through the back end's entry points in the GL core's order (compile of
# each stage, compile_vs_linked, link), compared with its Mesa 26.2.1
# drm-shim reference (src/shaderc/online/mali/tests.rs
# `the_product_route_is_measured_against_the_references`).
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
test=shaderc::online::mali::tests::the_product_route_is_measured_against_the_references
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
    # one build, then the shares in parallel (the test serialises its own
    # compiles: one process per share is what runs them at once)
    cargo test -q --no-default-features --features "$features" --lib --no-run >&2
    out=$(mktemp -d)
    trap 'rm -rf "$out"' EXIT
    pids=()
    for k in $(seq 0 $((jobs - 1))); do
      # cargo's -q is the test binary's --quiet too
      YAKOGL_MALI_ROUTE_DIR=$dir YAKOGL_MALI_ROUTE_SHARD=$k/$jobs cargo test -q --no-default-features \
        --features "$features" --lib -- --exact "$test" --nocapture --test-threads 1 >"$out/$k" 2>"$out/$k.err" &
      pids+=($!)
    done
    rc=0
    for k in "${!pids[@]}"; do
      wait "${pids[$k]}" || { rc=1; echo "route.sh: share $k of the measure failed:" >&2; tail -n 5 "$out/$k.err" >&2; }
    done
    [ "$rc" -eq 0 ] || exit 1
    rows=$(cat "$out"/*[0-9] | sed -n 's/^route\t//p' | LC_ALL=C sort)
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
