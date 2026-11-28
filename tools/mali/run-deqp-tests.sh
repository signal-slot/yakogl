#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Run every tests/mali_deqp_*.rs with the feature set its header names.
# See docs/notes/mali-deqp-tests-runner.md.
#
#   tools/mali/run-deqp-tests.sh [--list] [--include-ignored] [--only <glob>]...
#
# The header line is the first comment line `//! cargo test <flags> --test <name>`.
# Files with the same <flags> run as one `cargo test <flags> --test a --test b`.
set -u

usage() {
  sed -n '5,9p' "$0" | sed 's/^# \{0,1\}//'
  exit 2
}

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
list=0
ignored=0
globs=()
while [ $# -gt 0 ]; do
  case $1 in
    --list) list=1 ;;
    --include-ignored) ignored=1 ;;
    --only)
      [ $# -ge 2 ] || usage
      globs+=("$2")
      shift
      ;;
    -h | --help) usage ;;
    *) echo "run-deqp-tests.sh: unknown option $1" >&2; usage ;;
  esac
  shift
done

cd "$root" || exit 2

files=()
if [ ${#globs[@]} -eq 0 ]; then
  globs=('tests/mali_deqp_*.rs')
fi
for g in "${globs[@]}"; do
  matched=0
  # shellcheck disable=SC2231  # the glob is meant to expand here
  for f in $g; do
    [ -f "$f" ] || continue
    case $f in tests/mali_deqp_*.rs) ;; *) continue ;; esac
    files+=("$f")
    matched=1
  done
  if [ $matched -eq 0 ]; then
    echo "run-deqp-tests.sh: no tests/mali_deqp_*.rs matches '$g'" >&2
    exit 2
  fi
done
mapfile -t files < <(printf '%s\n' "${files[@]}" | sort -u)

keys=()   # distinct feature-flag strings, in first-seen order
names=()  # parallel: space-separated test names per key
bad=0
for f in "${files[@]}"; do
  name=$(basename "$f" .rs)
  line=$(grep -m1 -E '^//[/!]? *cargo test ' "$f" || true)
  flags=""
  if [ -n "$line" ]; then
    line=${line#*cargo test }
    set -f
    # shellcheck disable=SC2086
    set -- $line
    set +f
    while [ $# -gt 0 ]; do
      if [ "$1" = "--test" ]; then shift; else flags="$flags $1"; fi
      shift
    done
    flags=${flags# }
  fi
  case " $flags " in
    *" --features "* | *" --features="*) ;;
    *)
      echo "run-deqp-tests.sh: $f: no feature-set header (a 'cargo test ... --features ...' comment line)" >&2
      bad=$((bad + 1))
      continue
      ;;
  esac
  idx=-1
  for i in "${!keys[@]}"; do
    if [ "${keys[$i]}" = "$flags" ]; then idx=$i; break; fi
  done
  if [ $idx -lt 0 ]; then
    keys+=("$flags")
    names+=("$name")
  else
    names[idx]="${names[$idx]} $name"
  fi
done

failed=$bad
ntests=0
for i in "${!keys[@]}"; do
  args=()
  for n in ${names[$i]}; do
    args+=(--test "$n")
    ntests=$((ntests + 1))
  done
  # shellcheck disable=SC2206
  cmd=(cargo test ${keys[$i]} "${args[@]}")
  if [ $ignored -eq 1 ]; then cmd+=(-- --include-ignored); fi
  if [ $list -eq 1 ]; then
    echo "${cmd[*]}"
    continue
  fi
  echo "== group $((i + 1)): ${cmd[*]}"
  if ! "${cmd[@]}"; then
    echo "== group $((i + 1)) FAILED" >&2
    failed=$((failed + 1))
  fi
done

echo "${#keys[@]} groups, $((ntests + bad)) tests files, $failed failed"
[ "$failed" -eq 0 ]
