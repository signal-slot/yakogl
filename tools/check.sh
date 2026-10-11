#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# What to run before pushing. There is no CI on this repository, so this
# script is the gate, and a red fmt or a single warning is a defect rather
# than a matter of taste: main arrived unformatted twice in one day, and a
# build that already prints a warning is one where the next real warning is
# invisible -- which is how three unused_unsafe of my own survived (283cc56)
# and how a function with no callers did (7715e3f).
#
#   tools/check.sh                   every row: what CI runs
#   tools/check.sh --changed <base>  the rows the diff from <base> can reach
#   tools/check.sh --local <base>    the same, but a diff that reaches everything
#                                    runs the local set only (what tools/land.sh
#                                    runs before a push: about ten minutes)
#   tools/check.sh --quick           the PowerVR build only, for a tight edit loop
#
# A test row builds its binaries and runs them concurrently, and skips a
# binary that passed on main with the same key (cargo_test below:
# YAKOGL_CHECK_TEST_JOBS, YAKOGL_TEST_OK, YAKOGL_CHECK_RECORD,
# YAKOGL_CHECK_VERBOSE). The summary counts those binaries apart from the
# rows --changed skipped.
#
# --changed maps each changed path to the parts it can break (scope_of_path
# below: a GPU back end, gl_test, the tools, the precompiled tables, the
# drivers crate, the samples, or everything) and skips the rows of the other
# parts, printing `skip` for each so the log says what was not run. A path
# the map does not know, and every shared layer (src/gl, src/shaderc, the
# OSALs, capi, Cargo.toml, this script), selects every row -- and --local then
# runs the rows marked `local` only: fmt, clippy and the suite in the three
# GPUs' compiler configurations, the gl_test.c arms and this script's own
# self-test. Every other row (the report-es2 and tier pairs, the archive
# symbol checks, the aarch64 arms, the Zephyr images, the table proofs) is
# CI's, and so is the whole run (every landing of a shared layer took 45 to
# 100 minutes here before --local; user, 2026-10-11). The full run is
# CI's (.github/workflows/gate.yml, a self-hosted runner with the gate's
# environment); locally a change is tested where it reaches (user,
# 2026-10-10: "普通はね、ローカルでは変更の影響範囲だけをテストするよ").
#
# In every configuration the library ships in:
#
#   cargo fmt --check                     formatting
#   cargo clippy --all-targets            ZERO warnings, tests included
#   cargo test                            the suite
#
# The configurations are not interchangeable. `gpu-vivante` compiles a
# different backend and a different shader table, `report-es2` changes which
# GL version's rules apply, `shaderc-online` compiles the in-process
# compiler, and each has caught
# defects the others could not: a zero-byte allocation only the GC7000
# reached, an ES 3.0-only attachment query, a helper compiled for nobody.
#
# Feature PAIRS are rows of their own because a pair can be broken while
# every single feature is green. A feature the gate never combines is a
# feature the gate does not cover.
#
# `shaderc-online` is the in-process compiler (src/shaderc/online/). It is
# here rather than folded into the default build because it is the only
# configuration that compiles no_std code with no allocator behind it: a
# `std` import or an accidental `alloc` dependency in the front end passes
# every other row and fails on Zephyr, where nobody would see it until the
# image did not link. It is also the only row that runs the USC round trip,
# which decodes all 2,476 blobs of the offline table and re-encodes them byte
# for byte -- the gate on the instruction-set tables. It appears twice,
# because each GPU has its own instruction set and its own table of blobs:
# `shaderc-online` alone runs the PowerVR round trip, and paired with
# `gpu-vivante` the GC7000 one. Neither covers the other -- and the same is
# true a second time over for `tests/online_reflection.rs`, which compares
# the front end's reflection against whichever offline table is compiled in.
# The two tables were built by two different generators, so agreeing with one
# says nothing about the other.
#
# Wall clock on a warm tree, measured: 17 s for the five that existed before
# `shaderc-online` was added, 5 s for --quick. A cold tree pays for a build
# of the crate and its dev-dependencies per configuration, which is minutes;
# run it once and it stays warm.
set -u

cd "$(dirname "$0")/.." || exit 2

CONFIGS=(
  "host-test,gpu-powervr"
  "host-test,gpu-powervr,report-es2"
  "host-test,gpu-vivante"
  "host-test,gpu-powervr,shaderc-online"
  "host-test,gpu-vivante,shaderc-online"
  # The pair every dEQP-GLES2 online image is built from: it refuses GLSL ES
  # 3.00, so tests that compile ES 3.00 are cfg'd out of it, and it was never
  # tested until 2026-09-24, when 80 tests failed in it.
  "host-test,gpu-powervr,shaderc-online,report-es2"
  # The i.MX8MP's dEQP-GLES2 image: the GC7000 with the in-process compiler,
  # reporting ES 2.0. Its GC7000 harness pins are per build.
  "host-test,gpu-vivante,shaderc-online,report-es2"
  # The precompiled tier in front of the compiler (`precompiled-shaders`),
  # with the table of the gl_test_es100 app that the step below generates
  # into build/precompiled/ for each GPU: the whole suite with the tier on
  # (every look-up of a source the table holds a hit), and its own tests --
  # a hit runs no compile and is what the compiler makes, the table's
  # programs replayed run no compiler at all.
  "host-test,gpu-powervr,shaderc-online,precompiled-shaders"
  "host-test,gpu-vivante,shaderc-online,precompiled-shaders"
)

# The image configuration: the in-process compiler and no offline table at all.
# It is the one a product ships, and the only one where `pub mod offline` and
# the generated tables are absent, so nothing else catches a new use of them.
#
# BUILT AND LINTED, NOT TESTED, and the reason is worth stating rather than
# leaving as an odd exception: most of the GL suite compiles its shaders
# through the table, and this configuration has none. The online compiler
# serves 98 of the 142 ES 1.00 sources today, so running those tests here
# would fail 297 of them for a reason that is the milestone rather than a
# regression -- a red row nobody could act on. These rows move into CONFIGS
# above when the compiler covers what the suite asks for; until then they
# prove the thing they can prove, which is that the library builds, links and
# lints with no table in it.
BUILD_ONLY_CONFIGS=(
  "host-test,gpu-vivante,shaderc-online,no-shader-table"
  "host-test,gpu-powervr,shaderc-online,no-shader-table"
)
# The parts of the tree a changed path can break. `all` is every row; the
# others name the rows below that carry them in a `scope` line. The map is
# conservative on purpose: a shared layer, a generated file's generator, or
# a path it has never seen selects everything, and a wrong `skip` here is
# what the full run in CI exists to catch.
scope_of_path() {
  case "$1" in
  docs/* | */*.md | *.md | website/* | NOTICE | LICENSE* | .github/* | records/* | .gitignore | .ccfarm/*) echo docs ;;
  tools/precompile/* | tools/shaders/* | tests/fixtures/precompile*) echo precompile ;;
  src/shaderc/online/v3d/* | src/shaderc/online/v3d_abi.rs | src/v3d/* | driver/src/v3d/* | tools/v3d/* | tests/v3d_*) echo v3d ;;
  src/shaderc/online/vivante/* | src/vivante/* | driver/src/vivante/* | tools/vivante/* | tools/gc/* | tools/gc7000-isa/* | tools/hal-nxp/* | driver/rust/src/soc/imx8mp/*) echo vivante ;;
  src/shaderc/online/powervr/* | src/pvr/* | driver/src/pvr/* | driver/src/pvr_* | driver/src/fw/* | driver/compat/* | tools/pvr/* | tools/pco_shc/* | tools/pco-isa/* | tools/isa-extract/* | tools/glsl2usc.sh | driver/tools/offsetof/* | tests/reference/* | driver/rust/src/soc/am62p/* | tools/ti-zephyr/* | tools/ti-trm/*) echo pvr ;;
  driver/tools/pvrlab/*) echo gltest ;;
  tools/yakogl-tools/*) echo toolscrate ;;
  tools/osal/*) echo osal ;;
  driver/samples/*) echo samples ;;
  driver/rust/* | driver/*) echo drivers ;;
  tools/cts/* | tools/testdata/* | tools/*.py | tools/board.sh | tools/board_selftest.sh | tools/setup-gate-host.sh | tools/uboot-netboot-lib.sh | tools/website-check.sh | tools/hostharness/* | tools/README.md) echo tools ;;
  *) echo all ;;
  esac
}
QUICK=0
[ "${1-}" = "--quick" ] && { CONFIGS=("host-test,gpu-powervr"); QUICK=1; }
# --scope-of <path>...: print each path's scope and exit (tools/check_scope_selftest.sh).
if [ "${1-}" = "--scope-of" ]; then
  shift
  for sp in "$@"; do echo "$sp $(scope_of_path "$sp")"; done
  exit 0
fi
CHANGED_BASE=
LOCAL=0
[ "${1-}" = "--local" ] && { LOCAL=1; set -- --changed "${2-}"; }
if [ "${1-}" = "--changed" ]; then
  CHANGED_BASE=${2:?usage: tools/check.sh --changed <base-commit>}
  git rev-parse --verify "$CHANGED_BASE^{commit}" >/dev/null 2>&1 ||
    { echo "check.sh: --changed $CHANGED_BASE is not a commit"; exit 2; }
fi
# YAKOGL_CHECK_CONFIGS="host-test,gpu-powervr host-test,gpu-vivante" (space-separated feature
# sets) tests only those, for a change whose reach is known (user, 2026-09-26);
# the image builds, the aarch64 checks and the tool tests still run. Default: all.
if [ -n "${YAKOGL_CHECK_CONFIGS-}" ]; then read -r -a CONFIGS <<< "$YAKOGL_CHECK_CONFIGS"; fi

rc=0
red() { printf '\033[31m%s\033[0m\n' "$*"; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }

# A row that could not run is its own outcome, never folded into ok or failed:
# `gap <what was not checked>` prints it, counts it, and makes the gate red,
# because a skipped step that looks green is how a blind spot is born.
oks=0
fails=0
gaps=0
gap() {
  if ! wanted; then skip "$*"; return; fi
  printf '\033[33mgap   %s\033[0m\n' "$*"
  gaps=$((gaps + 1))
  rc=1
}

# The scopes a cargo feature set reaches: its GPU, and the precompiled tier.
config_scopes() {
  local out=""
  case "$1" in *gpu-powervr*) out="pvr" ;; *gpu-vivante*) out="vivante" ;; *gpu-v3d*) out="v3d" ;; esac
  case "$1" in *precompiled-shaders*) out="$out precompile" ;; esac
  echo "$out"
}
SCOPES=all
if [ -n "$CHANGED_BASE" ]; then
  # YAKOGL_CHECK_PATHS="<path> ..." stands in for the diff (the self-test).
  if [ -n "${YAKOGL_CHECK_PATHS-}" ]; then
    changed_paths=$(printf '%s\n' $YAKOGL_CHECK_PATHS)
  else
    changed_paths=$( { git diff --name-only "$CHANGED_BASE" HEAD
      git status --porcelain | cut -c4- | sed 's/^.* -> //'; } | sort -u | grep . )
  fi
  SCOPES=$(for cp in $changed_paths; do scope_of_path "$cp"; done | sort -u | tr '\n' ' ')
  case " $SCOPES " in *" all "*) SCOPES=all ;; esac
  echo "check.sh: --changed $CHANGED_BASE: $(printf '%s\n' "$changed_paths" | grep -c .) path(s), scopes: ${SCOPES:-none}"
  if [ "$SCOPES" = all ]; then
    for cp in $changed_paths; do [ "$(scope_of_path "$cp")" = all ] && echo "      every row: $cp"; done
  fi
fi
ROW_SCOPES=all
scope() { ROW_SCOPES="$*"; }
# 0 when the rows under the current `scope` line are selected.
wanted() {
  local s
  if [ "$SCOPES" = all ]; then
    [ "$LOCAL" -eq 0 ] && return 0
    for s in $ROW_SCOPES; do [ "$s" = local ] && return 0; done
    return 1
  fi
  for s in $ROW_SCOPES; do
    [ "$s" = all ] && return 0
    case " $SCOPES " in *" $s "*) return 0 ;; esac
  done
  return 1
}
skips=0
skip() {
  printf '\033[2mskip  %s\033[0m\n' "$*"
  skips=$((skips + 1))
}

# Every row ends with the seconds it took, so a gate log says where the time
# went: measured 2026-10-10, a full gate is about 45 min alone and 90 min
# beside four cargo builds on this 8-core host, and no row carried a time, so
# nothing said which rows to shorten or run in parallel.
gate_t0=$SECONDS
row_t0=$SECONDS
row_time() { printf '[%ds]' "$((SECONDS - row_t0))"; row_t0=$SECONDS; }

# step <label> <command...>
step() {
  local label="$1"
  shift
  local out warns
  if ! wanted; then skip "$label"; return 0; fi
  if [ -n "${YAKOGL_CHECK_PLAN-}" ]; then echo "plan  $label"; oks=$((oks + 1)); return 0; fi
  row_t0=$SECONDS
  if out=$("$@" 2>&1); then
    # clippy exits 0 even with warnings, so the warning count is what decides.
    #
    # Count EVERY `warning:` line and drop only cargo's per-crate summaries
    # ("warning: `yakogl` (lib) generated 3 warnings"). The previous pattern was
    # `^warning: [a-z]`, which silently missed every lint whose message begins
    # with anything else -- and clippy opens most of its messages with the
    # offending expression in backticks. `unnecessary_min_or_max` on
    # select.rs:428 sat on main reporting "ok" because its text starts with a
    # backtick: 2026-09-22, found while chasing something else.
    #
    # A gate that cannot see a whole class of its own tool's output is worse
    # than no gate, because it is believed. Anything but a summary counts.
    warns=$(printf '%s\n' "$out" | grep -E '^warning: ' | grep -cvE 'generated [0-9]+ warning')
    if [ "$warns" -gt 0 ]; then
      red "FAIL  $label: $warns warning(s) $(row_time)"
      printf '%s\n' "$out" | grep -E '^warning: |^ +--> ' | grep -vE 'generated [0-9]+ warning' | head -40
      rc=1
      fails=$((fails + 1))
      return
    fi
    green "ok    $label $(row_time)"
    oks=$((oks + 1))
  else
    red "FAIL  $label $(row_time)"
    # The whole output is kept: the last 40 lines alone cannot name a failed
    # test when a later binary ran after it (`--no-fail-fast` runs them all,
    # and the integration binaries' passes push the `--lib` failure out of the
    # tail). On 2026-10-11 a red precompiled-shaders row left the log with
    # "1 target failed: --lib" and no test name, and the rerun passed.
    mkdir -p build/check-fail
    printf '%s\n' "$out" > "build/check-fail/$((fails + 1)).log"
    printf '%s\n' "$out" | grep -E '^test .* FAILED$|panicked at|^error(\[E[0-9]+\])?: ' | head -40
    printf '%s\n' "$out" | tail -40
    echo "(whole output: build/check-fail/$((fails + 1)).log)"
    rc=1
    fails=$((fails + 1))
  fi
}

# A test row: cargo_test <cargo test arguments> [-- <harness arguments>]
#
# `cargo test` runs a row's test binaries one after another, so a row took
# the sum of its binaries' times on one core (docs/ci.md, section 1: 405 s
# for 53 binaries, the slowest 78 s). cargo_test builds the binaries
# (`--no-run`), then runs them itself, YAKOGL_CHECK_TEST_JOBS at a time
# (default: every core). Each binary's harness still runs its own tests in
# parallel. Every binary runs, whatever the others do (cargo's
# --no-fail-fast). Each binary's output goes to its own file under
# build/check/tests-<pid>/logs/; the row prints all of them in cargo's order, so a
# row that greps the output reads what `cargo test` printed. A failing
# binary is named at the end with its log.
#
# A binary that already passed on main is not run again. Its key is the
# sha256 of its bytes, the arguments it gets, its directory, the
# environment (test_env_key) and the tracked files the tests read at run
# time (test_tree_key). An entry for the key in YAKOGL_TEST_OK holds the
# main commit that recorded it and the binary's log, which the row prints
# instead. Entries are written only when YAKOGL_CHECK_RECORD=<commit> is
# set: the gate's run on main sets it, a pull request's run never does.
# A row prints `skip   <binary> (passed on main <commit>)` for each binary
# it did not run, outside the row's captured output, so the log says it.
#
# Leading `env NAME=value...` arguments are exported first (the tier rows'
# table), because `env` cannot run a shell function.
exec 3>&1
TEST_OK_DIR=$(realpath -m "${YAKOGL_TEST_OK:-${CARGO_TARGET_DIR:-target}/../test-ok}")
TEST_JOBS=${YAKOGL_CHECK_TEST_JOBS:-$(nproc)}
# cargo-nextest when the host has it (YAKOGL_CHECK_NEXTEST=0 turns it off):
# it runs each test as its own process across all binaries of the row, so
# a binary's tests spread over the cores too. Without it, the binaries run
# concurrently as below. A row that reads libtest's output (`--libtest`) or
# passes harness arguments always runs the binaries itself. The runner is
# part of the cache key: a pass under one runner says nothing about the
# other (nextest runs every test in a fresh process, libtest shares one).
NEXTEST_VERSION=
[ "${YAKOGL_CHECK_NEXTEST-}" = 0 ] || NEXTEST_VERSION=$(cargo nextest --version 2>/dev/null | head -1)
# One directory per run, because a run can hold another (the self-tests run
# check.sh inside the gate); the directories of runs that ended go.
TEST_RUN=build/check/tests-$$
for d in build/check/tests-*; do
  [ -d "$d" ] && ! kill -0 "${d##*-}" 2>/dev/null && rm -rf "$d"
done
TEST_STATE=$TEST_RUN/state
mkdir -p "$TEST_STATE"
: > "$TEST_STATE/counts"

# The environment a test binary runs in, as text: the toolchain, the host C
# compiler (tests/v3d_glmark2_mem.rs compiles C at run time), every line of
# `tools/cts/deps-env.sh --check` with the sha256 of the line's marker when
# it is a file (pco_shc, glslangValidator, the firmware header: a rebuilt
# tool at the same path is a new key), YAKOGL_TEST_ENV_EXTRA, and the value
# of every environment variable a test reads by name (test_env_names).
# Made once per run.
test_env_names() {
  grep -rhoE --include='*.rs' '(env::var(_os)?|env_set)\("[A-Za-z0-9_]+"\)' src tests tools driver |
    sed -E 's/.*\("([A-Za-z0-9_]+)"\)$/\1/' | sort -u
}
test_env_key() {
  local f="$TEST_STATE/env-key" st name marker v
  if [ ! -s "$f" ]; then
    {
      rustc -vV
      echo "cc: $(cc --version 2>/dev/null | head -1)"
      tools/cts/deps-env.sh --check 2>&1 | while read -r st name marker; do
        if [ -f "${marker-}" ]; then
          echo "$st $name $marker sha256=$(sha256sum < "$marker" | cut -c1-64)"
        else
          echo "$st $name ${marker-}"
        fi
      done
      echo "extra: ${YAKOGL_TEST_ENV_EXTRA-}"
      for v in $(test_env_names); do
        if [ -n "${!v+x}" ]; then echo "env $v=${!v}"; fi
      done
    } > "$f.tmp" || return 1
    mv "$f.tmp" "$f" || return 1
  fi
  sha256sum < "$f" | cut -c1-64
}

# The files a test may read at run time, as one hash: every tracked file
# (its blob id; a file changed or added in the working tree by its content)
# except the paths no test reads. The tests read the tree at run time
# (tools/shaders, tests/fixtures, tests/reference, headers, and
# src/test_rules.rs reads src/ and tests/ themselves), so the binary's bytes
# alone are not its inputs. The exceptions: the documents, the CI files,
# this script and its self-tests; and for every binary but the library's
# own unit tests (src/lib.rs, which holds test_rules), the integration test
# sources tests/*.rs, each compiled into its own binary. So a change to one
# integration test reruns that binary and the library's unit tests.
# $1: lib or other.
test_tree_key() {
  local f="$TEST_STATE/tree-key-$1" ex p ok_rel
  if [ ! -s "$f" ]; then
    ex=(':(exclude)docs' ':(exclude)website' ':(exclude)records' ':(exclude).github' ':(exclude).ccfarm'
      ':(exclude,glob)*.md' ':(exclude)tools/check.sh' ':(exclude)tools/check_scope_selftest.sh'
      ':(exclude)tools/check_testcache_selftest.sh')
    [ "$1" = lib ] || ex+=(':(exclude,glob)tests/*.rs')
    ok_rel=$(realpath -m --relative-base="$PWD" "$TEST_OK_DIR")
    case "$ok_rel" in /*) ;; *) ex+=(":(exclude)$ok_rel") ;; esac
    {
      git ls-files -s -- . "${ex[@]}"
      { git ls-files -m -- . "${ex[@]}"; git ls-files -o --exclude-standard -- . "${ex[@]}"; } | sort -u |
        while IFS= read -r p; do
          if [ -f "$p" ]; then echo "worktree $(git hash-object -- "$p") $p"; else echo "worktree deleted $p"; fi
        done
    } | sha256sum | cut -c1-64 > "$f.tmp" || return 1
    mv "$f.tmp" "$f" || return 1
  fi
  cat "$f"
}

cargo_test() {
  local cargo_args=() filter=() harness=() a after_dd=0 value=0 libtest_only=0
  if [ "${1-}" = --libtest ]; then libtest_only=1; shift; fi
  if [ "${1-}" = env ]; then
    shift
    while [ $# -gt 0 ] && [[ $1 == [A-Za-z_]*=* ]]; do export "${1?}"; shift; done
  fi
  while [ $# -gt 0 ]; do
    a=$1
    shift
    if [ "$after_dd" -eq 1 ]; then harness+=("$a"); continue; fi
    if [ "$value" -eq 1 ]; then cargo_args+=("$a"); value=0; continue; fi
    case "$a" in
    --) after_dd=1 ;;
    # every binary runs below, whatever the others do
    --no-fail-fast) ;;
    --features | -F | -p | --package | --test | --bin | --example | --bench | --profile | --target | \
      -j | --jobs | --manifest-path | --exclude | --color | --config | -Z) cargo_args+=("$a"); value=1 ;;
    -*) cargo_args+=("$a") ;;
    # the test name filter cargo passes to every binary
    *) filter+=("$a") ;;
    esac
  done
  local runner=libtest
  [ "$libtest_only" -eq 0 ] && [ "${#harness[@]}" -eq 0 ] && [ -n "$NEXTEST_VERSION" ] && runner=nextest
  local row
  row=$(($(wc -l < "$TEST_STATE/counts") + 1))
  local logs="$TEST_RUN/logs/$row" alogs="$PWD/$TEST_RUN/logs/$row"
  mkdir -p "$logs" || return 1
  local listing
  listing=$(cargo test --no-run --message-format=json-render-diagnostics "${cargo_args[@]}") || {
    echo "cargo_test: the build failed (cargo test --no-run ${cargo_args[*]})"
    echo "0 0" >> "$TEST_STATE/counts"
    return 1
  }
  # executable, package directory, target name, kind, source path; cargo's order
  local bins
  bins=$(printf '%s\n' "$listing" | python3 -c '
import json, os, sys
seen = set()
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if m.get("reason") != "compiler-artifact" or not m.get("executable") or not m["profile"]["test"]:
        continue
    if m["executable"] in seen:
        continue
    seen.add(m["executable"])
    t = m["target"]
    print("\t".join([m["executable"], os.path.dirname(m["manifest_path"]), t["name"], "/".join(t["kind"]), t["src_path"]]))
') || return 1
  if [ -z "$bins" ]; then
    echo "cargo_test: cargo listed no test binary for: cargo test ${cargo_args[*]}"
    echo "0 0" >> "$TEST_STATE/counts"
    return 1
  fi
  local envk treek_lib treek_other
  # A key that could not be made is a red row, never an empty key.
  envk=$(test_env_key) && treek_lib=$(test_tree_key lib) && treek_other=$(test_tree_key other) &&
    [ ${#envk} -eq 64 ] && [ ${#treek_lib} -eq 64 ] && [ ${#treek_other} -eq 64 ] || {
    echo "cargo_test: the cache key could not be made (env '${envk-}', tree '${treek_other-}', lib tree '${treek_lib-}')"
    echo "0 0" >> "$TEST_STATE/counts"
    return 1
  }
  [ "${YAKOGL_CHECK_VERBOSE-}" = 1 ] &&
    echo "key    row $row: env $envk ($TEST_STATE/env-key), tree $treek_other, lib tree $treek_lib" >&3
  local i=0 ran=0 cached=0 exe dir name kind src label h treek entry
  local -a labels=() hashes=() states=() exes=()
  while IFS=$'\t' read -r exe dir name kind src; do
    i=$((i + 1))
    label="$name ($kind)"
    treek=$treek_other
    [ "$src" = "$PWD/src/lib.rs" ] && treek=$treek_lib
    h=$( {
      echo "binary $(sha256sum < "$exe" | cut -c1-64)"
      echo "dir ${dir#"$PWD"}"
      echo "args ${filter[*]} -- ${harness[*]}"
      if [ "$runner" = nextest ]; then echo "runner $NEXTEST_VERSION"; else echo "runner libtest"; fi
      echo "env $envk"
      echo "tree $treek"
    } | sha256sum | cut -c1-64)
    labels+=("$label")
    hashes+=("$h")
    exes+=("$exe")
    entry="$TEST_OK_DIR/$h"
    if [ -s "$entry" ] && [ -f "$entry.log" ]; then
      echo "skip   $label (passed on main $(head -1 "$entry"))" >&3
      [ "${YAKOGL_CHECK_VERBOSE-}" = 1 ] && echo "       key $h" >&3
      cp "$entry.log" "$logs/$i.log" && echo 0 > "$logs/$i.rc"
      states+=(cached)
      cached=$((cached + 1))
      continue
    fi
    [ "${YAKOGL_CHECK_VERBOSE-}" = 1 ] && echo "run    $label key $h" >&3
    states+=(run)
    ran=$((ran + 1))
    [ "$runner" = nextest ] && continue
    while [ "$(jobs -rp | wc -l)" -ge "$TEST_JOBS" ]; do wait -n; done
    (
      cd "$dir" || exit 2
      CARGO_MANIFEST_DIR=$dir "$exe" "${filter[@]}" "${harness[@]}" > "$alogs/$i.log" 2>&1
      echo $? > "$alogs/$i.rc"
    ) &
  done <<< "$bins"
  wait
  echo "$ran $cached" >> "$TEST_STATE/counts"
  if [ "$runner" = nextest ] && [ "$ran" -gt 0 ]; then
    nextest_run "$logs" || return 1
  fi
  local failed=() rc_i
  for ((i = 1; i <= ${#labels[@]}; i++)); do
    if [ "${states[i - 1]}" = cached ]; then
      echo "     Running ${labels[i - 1]}: not run, its log from main"
      cat "$logs/$i.log" 2>/dev/null
    elif [ "$runner" = libtest ]; then
      echo "     Running ${labels[i - 1]}"
      cat "$logs/$i.log" 2>/dev/null
    fi
    rc_i=$(cat "$logs/$i.rc" 2>/dev/null || echo none)
    if [ "$rc_i" != 0 ]; then
      failed+=("${labels[i - 1]}: exit $rc_i, log $logs/$i.log")
    elif [ "${states[i - 1]}" = run ] && [ -n "${YAKOGL_CHECK_RECORD-}" ] && [ ! -e "$logs/norecord" ]; then
      mkdir -p "$TEST_OK_DIR" &&
        cp "$logs/$i.log" "$TEST_OK_DIR/${hashes[i - 1]}.log.tmp" &&
        mv "$TEST_OK_DIR/${hashes[i - 1]}.log.tmp" "$TEST_OK_DIR/${hashes[i - 1]}.log" &&
        echo "$YAKOGL_CHECK_RECORD" > "$TEST_OK_DIR/${hashes[i - 1]}.tmp" &&
        mv "$TEST_OK_DIR/${hashes[i - 1]}.tmp" "$TEST_OK_DIR/${hashes[i - 1]}"
    fi
  done
  [ "$runner" = nextest ] && [ "$ran" -gt 0 ] && cat "$logs/nextest.log"
  if [ "${#failed[@]}" -gt 0 ]; then
    printf 'cargo_test: %d of %d test binaries failed:\n' "${#failed[@]}" "${#labels[@]}"
    printf '  FAILED binary %s\n' "${failed[@]}"
    return 1
  fi
}

# cargo_test's nextest arm: one `cargo nextest run` over the binaries that
# are not cached, chosen by their nextest binary ids, which
# `cargo nextest list` maps from their paths (a path without an id is a red
# row, never a binary left out). Reads cargo_test's locals; $1 is the log
# directory. Each binary gets the nextest lines that name it as its log.
# A binary named by a line other than PASS, SLOW or SKIP failed.
nextest_run() {
  local logs=$1 ids j id expr="" st bad
  ids=$(cargo nextest list "${cargo_args[@]}" --message-format json 2>/dev/null | python3 -c '
import json, sys
for k, v in json.load(sys.stdin)["rust-suites"].items():
    print(v["binary-path"] + "\t" + k)
') || { echo "cargo_test: cargo nextest list failed"; return 1; }
  local -A id_of=()
  while IFS=$'\t' read -r a id; do id_of[$a]=$id; done <<< "$ids"
  local -a run_ids=()
  for ((j = 1; j <= ${#exes[@]}; j++)); do
    [ "${states[j - 1]}" = run ] || continue
    id=${id_of[${exes[j - 1]}]-}
    [ -n "$id" ] || { echo "cargo_test: nextest has no binary id for ${exes[j - 1]}"; return 1; }
    run_ids[j]=$id
    expr="${expr:+$expr | }binary_id(=$id)"
  done
  cargo nextest run "${cargo_args[@]}" --no-fail-fast --test-threads "$TEST_JOBS" "${filter[@]}" \
    -E "$expr" > "$logs/nextest.log" 2>&1
  st=$?
  local named=0
  for j in "${!run_ids[@]}"; do
    grep -F " ${run_ids[j]} " "$logs/nextest.log" > "$logs/$j.log"
    bad=$(grep -cvE '^ *(PASS|SLOW|SKIP) ' "$logs/$j.log")
    if [ "$bad" -gt 0 ]; then echo 1 > "$logs/$j.rc"; named=$((named + 1)); else echo 0 > "$logs/$j.rc"; fi
  done
  if [ "$st" -ne 0 ]; then
    # Nothing from a failed run is recorded. A failed run that names no
    # binary fails them all.
    : > "$logs/norecord"
    if [ "$named" -eq 0 ]; then
      for j in "${!run_ids[@]}"; do echo "$st" > "$logs/$j.rc"; done
    fi
  fi
  return 0
}

# The summary line and the verdict; the end of every run.
summary() {
  echo
  mode="every row"
  [ -n "$CHANGED_BASE" ] && mode="--changed $CHANGED_BASE, scopes: $SCOPES"
  [ "$LOCAL" -eq 1 ] && mode="--local $CHANGED_BASE, scopes: $SCOPES"
  # The test binaries the rows ran and the ones they did not run because the
  # same binary passed on main (cargo_test), apart from the rows `--changed`
  # skipped.
  bins_run=0
  bins_cached=0
  runner_name="the binaries concurrently (no cargo-nextest)"
  [ -n "$NEXTEST_VERSION" ] && runner_name="$NEXTEST_VERSION (the rows that read libtest's output run the binaries concurrently)"
  while read -r r c; do
    bins_run=$((bins_run + r))
    bins_cached=$((bins_cached + c))
  done < "$TEST_STATE/counts"
  echo "check.sh: $oks ok, $fails failed, $gaps gap(s) (a gap was not run: it is not a pass), $skips skipped ($mode) in $((SECONDS - gate_t0))s; test binaries: $bins_run run, $bins_cached skipped (passed on main); runner: $runner_name"
  if [ "$rc" -eq 0 ]; then
    green "check.sh: clean"
  else
    red "check.sh: fix the above before pushing"
  fi
  exit "$rc"
}

# --cargo-test <cargo test arguments>: one test row through cargo_test and
# the summary, nothing else (tools/check_testcache_selftest.sh).
# --test-keys: print cargo_test's keys and the variable names the
# environment key reads, and exit (the self-test).
if [ "${1-}" = "--test-keys" ]; then
  echo "env $(test_env_key)"
  echo "tree $(test_tree_key other)"
  echo "lib-tree $(test_tree_key lib)"
  echo "names $(test_env_names | tr '\n' ' ')"
  exit 0
fi
if [ "${1-}" = "--cargo-test" ]; then
  shift
  step "test    $*" cargo_test "$@"
  summary
fi

scope all local
step "cargo fmt --check" cargo fmt --check

# The no_std archive crate, which every product build and every board pair go
# through (capi/, `cargo build -p yakogl-capi`). None of the rows below cover
# it: they all carry `host-test`, which links std, so anything the library
# reaches can call into std and pass all nine of them. `f32::floor` in
# src/gl/framebuffer.rs did exactly that on 2026-09-22 -- the crate has no
# libm -- and the break only showed when a board pair was built by hand.
# Twelve seconds on a warm tree, and it is the only row that says "this
# compiles for a target with no operating system". Built as the offline
# images take it (the Mesa reference as the table, `mesa-reference-table`),
# the configuration with the most exported entry points, which
# tools/api_coverage.py reads from this archive.
scope pvr tools
step "build   yakogl-capi (no_std)" cargo build -p yakogl-capi --features gpu-powervr,mesa-reference-table

# The tables of the tier configurations: yakogl-precompile on the gl_test_es100
# app for each GPU (about 5 s each warm). A configuration with the tier reads
# its GPU's through YAKOGL_PRECOMPILED_TABLE (build.rs).
tier_tables() {
  local gpu feature
  for gpu in powervr gc7000; do
    feature=gpu-powervr
    [ "$gpu" = gc7000 ] && feature=gpu-vivante
    cargo run -q -p yakogl-precompile --features "$feature" -- --app gl_test_es100 \
      -o "build/precompiled/check-$gpu.rs" || return 1
  done
}
tier=()
scope pvr vivante precompile
for f in "${CONFIGS[@]}"; do
  case "$f" in *precompiled-shaders*) step "gen     precompiled tables for the tier configurations" tier_tables; break ;; esac
done
for f in "${CONFIGS[@]}"; do
  tier=()
  scope "$(config_scopes "$f")"
  case "$f" in host-test,gpu-powervr,shaderc-online | host-test,gpu-vivante,shaderc-online) scope "$ROW_SCOPES local" ;; esac
  case "$f" in
  *precompiled-shaders*gpu-vivante* | *gpu-vivante*precompiled-shaders*)
    tier=(env "YAKOGL_PRECOMPILED_TABLE=$PWD/build/precompiled/check-gc7000.rs") ;;
  *precompiled-shaders*) tier=(env "YAKOGL_PRECOMPILED_TABLE=$PWD/build/precompiled/check-powervr.rs") ;;
  esac
  step "clippy  $f" "${tier[@]}" cargo clippy --no-default-features --features "$f" --all-targets
  # --no-fail-fast: cargo otherwise stops at the first test BINARY that fails,
  # and every binary after it goes unrun. A red row then names one failure
  # and hides the rest: on 2026-09-23 a gate red on deqp_repro_shaders never
  # ran encoding_vocabulary, whose own failure surfaced a push later.
  step "test    $f" cargo_test "${tier[@]}" --no-fail-fast --no-default-features --features "$f"
done

# An image without the compiler: the precompiled shaders the whole back end
# (`precompiled-shaders` without `shaderc-online`). Its host configuration
# carries no compiler, so the GL suite, which compiles what it likes, is not
# its test; linted with its unit tests, and run on the back end's own
# (shaderc::precompiled: a source is its row or a refusal by name, a variant
# or a program the table lacks is refused by name, what a GL client sees).
if [ "$QUICK" -eq 0 ]; then
  for gpu in powervr gc7000; do
    feature=gpu-powervr
    [ "$gpu" = gc7000 ] && feature=gpu-vivante
    c="host-test,$feature,precompiled-shaders,no-shader-table"
    alone=(env "YAKOGL_PRECOMPILED_TABLE=$PWD/build/precompiled/check-$gpu.rs")
    scope "$(config_scopes "$c")"
    step "clippy  $c (no compiler; lib and its tests)" "${alone[@]}" \
      cargo clippy --no-default-features --features "$c" --lib --profile test
    step "test    $c (no compiler: shaderc::precompiled)" \
      cargo_test "${alone[@]}" --no-default-features --features "$c" --lib shaderc::precompiled
  done
fi

for f in "${BUILD_ONLY_CONFIGS[@]}"; do
  scope "$(config_scopes "$f")"
  step "clippy  $f (no tests: see BUILD_ONLY_CONFIGS)" \
    cargo clippy --no-default-features --features "$f" --lib
done

# The test knobs (`test-knobs`, Cargo.toml): the C entry points the gl_test,
# gc_gl_test and deqp images set, and the switches behind them. No row above
# has the feature, so they test each knob's default, which is what a product
# runs; this one lints the GC7000 with the knobs in and runs the library's
# tests, the knob-driven ones among them. Those are named here, and each
# must have run and passed: a knob test that stops being compiled would
# otherwise leave the row green with one test fewer.
KNOBS_CONFIG="host-test,gpu-vivante,shaderc-online,test-knobs"
KNOBS_TESTS=(
  vivante::backend::backend_tests::the_flip_study_draws_a_top_down_target_bottom_up_only_when_asked
  vivante::backend::backend_tests::the_flip_study_ends_a_frame_with_flipped_resolve_copies_only_when_asked
  vivante::backend::backend_tests::msaa_bit_one_adds_etnaviv_s_small_msaa_settings_and_the_probe_reads_the_scratch
  vivante::backend::backend_tests::msaa_bit_two_writes_the_sample_mask_and_coverage_as_etnaviv_does
  vivante::backend::backend_tests::a_multisampled_draw_is_supertiled_and_resolved_like_etnaviv_only_when_on
  vivante::backend::backend_tests::msaa_bit_three_scales_the_viewport_and_clip_to_the_scratch_samples
  gl::egl::tests::the_gc7000_offers_the_multisampled_config_while_msaa_is_on
  gl::texture::tests::the_gc7000_narrow_la_knob_selects_the_rgba8_store
)
knobs_lib_tests() {
  local out t
  out=$(cargo_test --libtest --no-fail-fast --no-default-features --features "$KNOBS_CONFIG" --lib 2>&1) ||
    { printf '%s\n' "$out" | grep -E '^test result|FAILED|panicked' | head -20; return 1; }
  printf '%s\n' "$out" | grep -E '^test result'
  for t in "${KNOBS_TESTS[@]}"; do
    printf '%s\n' "$out" | grep -qxF "test $t ... ok" || { echo "knob test did not run and pass: $t"; return 1; }
  done
}
scope vivante
step "clippy  $KNOBS_CONFIG" cargo clippy --no-default-features --features "$KNOBS_CONFIG" --all-targets
step "test    $KNOBS_CONFIG (--lib, ${#KNOBS_TESTS[@]} knob tests by name)" knobs_lib_tests

# The Raspberry Pi 4's V3D (`gpu-v3d`). Its only shader back end is the
# in-process compiler and it has no offline table, so its one host
# configuration is the image's feature set plus `host-test` (lib.rs refuses
# every other). Linted with every target. Tested --lib, the GL suite
# (`gl::`, `testutil::`) included: TestGl::shader hands each named shader's
# GLSL ES 1.00 text (testutil::shader_source) to glShaderSource and
# glCompileShader, so the online compiler compiles it as it would an
# application's, and a shader it refuses fails the test with its log.
#
# Two things keep the row honest. The test count: at least V3D_MIN_TESTS must
# pass (2460 when the GL suite joined the row, 2026-09-29), so a filter or a
# cfg that drops a population turns the row red. And the ignored tests: each
# is printed with its reason, one line per test, and kept in
# build/check/v3d-ignored.txt (`ignored<TAB><test><TAB><reason>`), because
# an ignore nobody reads is a skip. Tests gated at compile time by GPU
# (`cfg(feature = "gpu-powervr")`, the
# PowerVR table's own values) are not in this binary and are not listed.
V3D_CONFIG="host-test,gpu-v3d,shaderc-online,no-shader-table"
V3D_MIN_TESTS=2460
V3D_IGNORED="build/check/v3d-ignored.txt"
v3d_lib_tests() {
  local out passed
  out=$(cargo_test --libtest --no-fail-fast --no-default-features --features "$V3D_CONFIG" --lib 2>&1)
  local status=$?
  printf '%s\n' "$out" | grep -E '^test result|FAILED|panicked' | head -20
  mkdir -p "$(dirname "$V3D_IGNORED")" || return 1
  # "test <path> ... ignored, <reason>" (or no reason) -> one TSV row each
  printf '%s\n' "$out" | sed -nE \
    -e 's/^test ([^ ]+) \.\.\. ignored, (.*)$/ignored\t\1\t\2/p' \
    -e 's/^test ([^ ]+) \.\.\. ignored$/ignored\t\1\t(no reason given)/p' |
    sort > "$V3D_IGNORED" || return 1
  [ "$status" -eq 0 ] || return 1
  passed=$(printf '%s\n' "$out" | sed -nE 's/^test result: ok\. ([0-9]+) passed.*/\1/p' | head -1)
  if [ -z "$passed" ] || [ "$passed" -lt "$V3D_MIN_TESTS" ]; then
    echo "only ${passed:-no} test(s) passed; the row expects at least $V3D_MIN_TESTS"
    return 1
  fi
  # The summary's own ignored count must equal the rows written; if not, a
  # line format changed and the list is incomplete.
  local want got
  want=$(printf '%s\n' "$out" | sed -nE 's/^test result: ok\. .* ([0-9]+) ignored;.*/\1/p' | head -1)
  got=$(wc -l < "$V3D_IGNORED")
  if [ "$want" != "$got" ]; then
    echo "the summary says ${want:-?} ignored, $V3D_IGNORED lists $got"
    return 1
  fi
}
scope v3d local
step "clippy  $V3D_CONFIG" cargo clippy --no-default-features --features "$V3D_CONFIG" --all-targets
step "test    $V3D_CONFIG (--lib, GL suite on the online compiler)" v3d_lib_tests
scope v3d
# The V3D compiler (src/shaderc/online/v3d) and the Mesa QPU packer build.rs
# compiles for it exist only in `gpu-v3d` builds (docs/source-form.md, F1),
# so the rows that ran its tests without the feature -- `host-test,shaderc-online`
# and its report-es2 twin, until 2026-10-01 -- no longer can. They run here:
# the QPU interpreter validated on Mesa's code (tests/v3d_sim_mesa.rs), and the
# compiler's own tests (roundtrip, mesa_ref, golden, qpu_ffi, chain, ...) and
# v3d_sim_mesa again in a report-es2 build, whose pins differ (GLSL ES 3.00
# refused). The ES 2.0 build's GL suite is not in that row: it never ran with
# gpu-v3d before either. tests/v3d_cube_noseamless.rs (the Pi 4's
# non-seamless cube defect, docs/v3d-noseamless-cube.md) runs in both builds
# too: the keyed (cube_noseam) programs against a non-seamless reference
# computed in the test, and the unkeyed ones as golden hashes.
step "test    $V3D_CONFIG (tests/v3d_sim_mesa.rs)" \
  cargo_test --no-fail-fast --no-default-features --features "$V3D_CONFIG" --test v3d_sim_mesa
step "test    $V3D_CONFIG (tests/v3d_cube_noseamless.rs)" \
  cargo_test --no-fail-fast --no-default-features --features "$V3D_CONFIG" --test v3d_cube_noseamless
# glmark2's V3D allocations replayed through the OSAL's carve-out allocator
# (src/osal/v3d_granules.h) and Mesa's layout (driver/src/v3d/v3d_tex.c), both
# built from C by the test with `cc`: every large request fits the Pi 4's
# 96 MiB, and 48 MiB reproduces the refusals RESULTS.md records.
step "test    $V3D_CONFIG (tests/v3d_glmark2_mem.rs)" \
  cargo_test --no-fail-fast --no-default-features --features "$V3D_CONFIG" --test v3d_glmark2_mem
# ES 3.0's integer colour render targets through the GL API (the stand-in
# back end of the test draws nothing); the ES 2.0 report below must refuse
# every one of the formats.
step "test    $V3D_CONFIG (tests/es3_int_rt.rs)" \
  cargo_test --no-fail-fast --no-default-features --features "$V3D_CONFIG" --test es3_int_rt
# ES 3.0 transform feedback objects, varyings, queries and rasterizer
# discard through the GL API (the capture itself is item tf-capture); the ES
# 2.0 report below pins the old answers.
step "test    $V3D_CONFIG (tests/es3_tf_api.rs)" \
  cargo_test --no-fail-fast --no-default-features --features "$V3D_CONFIG" --test es3_tf_api
# ES 3.0's sRGB colour buffers and textures through the GL API (the stand-in
# back end stores a clear as one that encodes must); the V3D samples the
# class and refuses a draw into it by name. The ES 2.0 report below must
# have no sRGB format.
step "test    $V3D_CONFIG (tests/es3_srgb.rs)" \
  cargo_test --no-fail-fast --no-default-features --features "$V3D_CONFIG" --test es3_srgb
V3D_ES2_CONFIG="$V3D_CONFIG,report-es2"
step "clippy  $V3D_ES2_CONFIG" cargo clippy --no-default-features --features "$V3D_ES2_CONFIG" --all-targets
step "test    $V3D_ES2_CONFIG (--lib shaderc::online::v3d)" \
  cargo_test --no-fail-fast --no-default-features --features "$V3D_ES2_CONFIG" --lib shaderc::online::v3d
step "test    $V3D_ES2_CONFIG (tests/v3d_sim_mesa.rs)" \
  cargo_test --no-fail-fast --no-default-features --features "$V3D_ES2_CONFIG" --test v3d_sim_mesa
step "test    $V3D_ES2_CONFIG (tests/v3d_cube_noseamless.rs)" \
  cargo_test --no-fail-fast --no-default-features --features "$V3D_ES2_CONFIG" --test v3d_cube_noseamless
step "test    $V3D_ES2_CONFIG (tests/es3_int_rt.rs)" \
  cargo_test --no-fail-fast --no-default-features --features "$V3D_ES2_CONFIG" --test es3_int_rt
step "test    $V3D_ES2_CONFIG (tests/es3_tf_api.rs)" \
  cargo_test --no-fail-fast --no-default-features --features "$V3D_ES2_CONFIG" --test es3_tf_api
step "test    $V3D_ES2_CONFIG (tests/es3_srgb.rs)" \
  cargo_test --no-fail-fast --no-default-features --features "$V3D_ES2_CONFIG" --test es3_srgb
# The ignored population of the row above, by name: never silent.
if [ -s "$V3D_IGNORED" ]; then
  echo "      $V3D_CONFIG: $(wc -l < "$V3D_IGNORED") ignored test(s), listed in $V3D_IGNORED:"
  sed 's/^/      /' "$V3D_IGNORED"
fi
# The FFI pins of src/v3d/ffi.rs, asserted against the real headers, and the
# job builder the image links, both with the host C compiler: a header that
# moved, or a warning in the builder, fails here rather than in an image.
# Their byte-for-byte checks against Mesa (tools/v3d/cl_golden.c,
# job_check.c) and the textures' (tex_check) run below against the Mesa
# sources vendored in third_party/mesa-26.2.1 (built by tools/v3d/mesa_ref.sh);
# MESA_V3D_TREE optionally names a built Mesa tree to compare against instead.
#
# driver/src/v3d includes Mesa's generated packers and Mesa's texture tiling
# (driver/src/v3d/generated/, what Mesa's generators make), which include the
# Mesa headers vendored unmodified in third_party/.
v3d_inc=(-Idriver/src/v3d -Idriver/src/v3d/generated -Ithird_party/mesa-26.2.1/src/broadcom
  -Ithird_party/mesa-26.2.1/src -Ithird_party/mesa-26.2.1/include)
if command -v cc >/dev/null 2>&1; then
  # driver/src/v3d/generated/ is what Mesa's own generators make from the
  # vendored inputs, byte for byte, with the commands of Mesa's meson files
  # (driver/src/v3d/v3d_cl.h, v3d_tiling_mesa.c); u_format_table.py needs
  # PyYAML, as Mesa's build does. And every vendored file is the upstream
  # file (third_party/*/UPSTREAM).
  v3d_generated_regenerate() {
    local m=third_party/mesa-26.2.1/src g=driver/src/v3d/generated
    PYTHONDONTWRITEBYTECODE=1 python3 "$m/broadcom/cle/gen_pack_header.py" \
      "$m/broadcom/cle/v3d_packet.xml" 42 | cmp - "$g/v3d_packet_v42_pack.h" || return 1
    PYTHONDONTWRITEBYTECODE=1 python3 "$m/util/format/u_format_table.py" \
      "$m/util/format/u_format.yaml" --enums | cmp - "$g/util/format/u_format_gen.h"
  }
  scope v3d
  step "gen     driver/src/v3d/generated == Mesa's generators on third_party" v3d_generated_regenerate
  # driver/src/pvr/generated/csbgen/rogue/ is what Mesa's csbgen makes from
  # the vendored XML, byte for byte, with the command of Mesa's
  # src/imagination/csbgen/rogue/meson.build; every header there has its XML.
  pvr_generated_regenerate() {
    local m=third_party/mesa-26.2.1/src/imagination/csbgen g=driver/src/pvr/generated/csbgen/rogue h n
    for h in "$g"/*.h; do
      n=$(basename "$h" .h)
      PYTHONDONTWRITEBYTECODE=1 python3 "$m/gen_pack_header.py" "$m/rogue/$n.xml" | cmp - "$h" || return 1
    done
  }
  scope pvr
  step "gen     driver/src/pvr/generated == Mesa's csbgen on third_party" pvr_generated_regenerate
  upstream_files_unmodified() {
    local u
    for u in third_party/*/UPSTREAM; do
      (cd "${u%/UPSTREAM}" && awk 'NF==2 && $1 ~ /^[0-9a-f]{64}$/ {print $1 "  " $2}' UPSTREAM | sha256sum --quiet -c -) || return 1
    done
  }
  scope all
  step "sha256  third_party files are the upstream files (UPSTREAM)" upstream_files_unmodified
  scope v3d
  step "cc      v3d FFI layout pins" cc -std=c11 -fsyntax-only -Wall -Wextra -Werror \
    "${v3d_inc[@]}" src/v3d/ffi_layout_check.c
  step "cc      v3d job builder" cc -std=c11 -fsyntax-only -Wall -Wextra -Werror \
    "${v3d_inc[@]}" driver/src/v3d/v3d_job.c
  step "cc      v3d textures" cc -std=c11 -fsyntax-only -Wall -Wextra -Werror \
    "${v3d_inc[@]}" driver/src/v3d/v3d_tex.c
  # The CPU reference the LINEAR board test compares with (dEQP's filters,
  # tools/v3d/tex_filter_ref.h): hand-computed pixels and the test's controls.
  # No Mesa needed. The row requires its "run (planned N), 0 failed" line.
  v3d_filter_ref_check() {
    local out
    mkdir -p build/v3d-check || return 1
    cc -std=c11 -O1 -Wall -Wextra -Werror -Itools/v3d tools/v3d/tex_filter_ref_check.c \
      -o build/v3d-check/tex_filter_ref_check || return 1
    out=$(build/v3d-check/tex_filter_ref_check) || { printf '%s\n' "$out" | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE '^tex_filter_ref_check: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "tex_filter_ref_check exited 0 without its result line"
      return 1
    }
  }
  step "run     v3d tex_filter_ref_check (dEQP's filters, the LINEAR plan)" v3d_filter_ref_check
  # The board harness that times v3d_tex_store_level (tools/v3d/tex_tiling_bench.h):
  # every size runs and every tiled texel is checked; its refusals; its check
  # can fail. The host's timings are printed but judge nothing.
  v3d_tiling_bench_check() {
    local out
    mkdir -p build/v3d-check || return 1
    cc -std=c11 -O1 -Wall -Wextra -Werror "${v3d_inc[@]}" -Itools/v3d \
      tools/v3d/tex_tiling_bench_check.c driver/src/v3d/v3d_tex.c driver/src/v3d/v3d_tiling_mesa.c \
      -lm -o build/v3d-check/tex_tiling_bench_check || return 1
    out=$(build/v3d-check/tex_tiling_bench_check) || { printf '%s\n' "$out" | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE '^tex_tiling_bench_check: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "tex_tiling_bench_check exited 0 without its result line"
      return 1
    }
  }
  step "run     v3d tex_tiling_bench_check (the tiling-time harness)" v3d_tiling_bench_check
  # The TFU probe of v3d_gl_test case 140 (tools/v3d/tfu_case.h): its sizes
  # reach every tiling, every packed field against its source, one job as
  # Mesa's literals, the refusals, and a compare that can fail.
  v3d_tfu_case_check() {
    local out
    mkdir -p build/v3d-check || return 1
    cc -std=c11 -O1 -Wall -Wextra -Werror "${v3d_inc[@]}" -Itools/v3d \
      tools/v3d/tfu_case_check.c driver/src/v3d/v3d_tex.c driver/src/v3d/v3d_tiling_mesa.c \
      -lm -o build/v3d-check/tfu_case_check || return 1
    out=$(build/v3d-check/tfu_case_check) || { printf '%s\n' "$out" | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE '^tfu_case_check: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "tfu_case_check exited 0 without its result line"
      return 1
    }
  }
  step "run     v3d tfu_case_check (the TFU probe, case 140)" v3d_tfu_case_check
  # The PowerVR VDM state emitters and the PPP state block
  # (driver/src/pvr_vdm_ppp_gen.c): every field (sizes, enums, indices, clip
  # tiles, address alignment) at its bound is accepted and one past it
  # refused, by name, with nothing written; the vertex PDS
  # program's temps give VDM_STATE5.vs_pds_temp_size 1
  # (tools/pvr/vdm_field_check.c). No Mesa needed.
  # driver/compat/pvr_mesa.h: Mesa's PDS generators and csbgen packers, which
  # the generators call (driver/compat/pvr_mesa.cmake lists the same).
  pvr_mesa_inc=(-Idriver/compat -Idriver/src/pvr/generated -Ithird_party/mesa-26.2.1/include
    -Ithird_party/mesa-26.2.1/src -Ithird_party/mesa-26.2.1/src/imagination
    -Ithird_party/mesa-26.2.1/src/imagination/include -Ithird_party/mesa-26.2.1/src/imagination/common
    -Ithird_party/mesa-26.2.1/src/imagination/vulkan -Ithird_party/mesa-26.2.1/src/imagination/vulkan/pds
    -Ithird_party/mesa-26.2.1/src/imagination/vulkan/pds/pvr_pds_programs)
  pvr_vdm_field_check() {
    local out
    mkdir -p build/pvr-check || return 1
    cc -std=c11 -O1 -Wall -Wextra -Werror -Idriver/src "${pvr_mesa_inc[@]}" \
      tools/pvr/vdm_field_check.c driver/src/pvr_vdm_ppp_gen.c \
      driver/src/pvr_pds_render_gen.c driver/compat/pvr_mesa_pds.c \
      -o build/pvr-check/vdm_field_check || return 1
    out=$(build/pvr-check/vdm_field_check) || { printf '%s\n' "$out" | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE '^vdm_field_check: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "vdm_field_check exited 0 without its result line"
      return 1
    }
  }
  scope pvr
  step "run     pvr vdm_field_check (VDM/PPP fields refuse, never mask)" pvr_vdm_field_check
  # The render register words the C side packs with Mesa's csbgen
  # (driver/src/pvr_render_regs.c) against the literals the driver carried
  # before (tools/pvr/render_regs_check.c). The row requires its line.
  pvr_render_regs_check() {
    local out
    mkdir -p build/pvr-check || return 1
    cc -std=c11 -O1 -Wall -Wextra -Werror -Idriver/src "${pvr_mesa_inc[@]}" \
      tools/pvr/render_regs_check.c driver/src/pvr_render_regs.c driver/compat/pvr_mesa_pds.c \
      -o build/pvr-check/render_regs_check || return 1
    out=$(build/pvr-check/render_regs_check) || { printf '%s\n' "$out" | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE '^render_regs_check: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "render_regs_check exited 0 without its result line"
      return 1
    }
  }
  step "run     pvr render_regs_check (csbgen register words = the former literals)" pvr_render_regs_check
  # Every firmware-interface number the PowerVR driver uses (sizes, offsets,
  # command numbers, flags, registers, MMU bits, heaps, the MH tags, the boot
  # alignment-check table), pinned to the board's value and proved against
  # the vendored TI/Img DDK 25.2 headers with the AM62P build configuration
  # (driver/tools/offsetof/ddk_abi_pins.h, one _Static_assert each, through
  # the driver's own include, driver/compat/pvr_ddk.h). The board's build
  # asserts the same list (driver/compat/pvr_ddk_check.c). No DDK checkout
  # needed: third_party/ holds it.
  pvr_ddk_abi_check() {
    local out
    mkdir -p build/pvr-check || return 1
    # shellcheck disable=SC2046  # the flags are one word each
    cc -std=gnu11 -Wall -Wextra -Werror $(driver/tools/offsetof/ddk-cflags.sh) \
      driver/tools/offsetof/ddk_abi_check.c -o build/pvr-check/ddk_abi_check || return 1
    out=$(build/pvr-check/ddk_abi_check) || { printf '%s\n' "$out" | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE '^ddk_abi_check: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "ddk_abi_check exited 0 without its result line"
      return 1
    }
  }
  step "run     pvr ddk_abi_check (FWIF numbers = the vendored DDK 25.2 headers)" pvr_ddk_abi_check
  # driver/src/v3d/v3d_hw.c against the register model (tools/v3d/hw_mock_test.c):
  # the power, MMU, job, overflow, reset and TFU sequences in their Linux order.
  # It was never a gate row before the TFU was added to it.
  v3d_hw_mock_test() {
    local out
    mkdir -p build/v3d-check || return 1
    cc -std=c11 -Wall -Wextra -Werror -Itools/v3d/hostshim -Idriver/src/v3d \
      tools/v3d/hw_mock_test.c driver/src/v3d/v3d_hw.c \
      -o build/v3d-check/hw_mock_test || return 1
    out=$(build/v3d-check/hw_mock_test) || { printf '%s\n' "$out" | grep -v '^  ' | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE '^hw_mock_test: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "hw_mock_test exited 0 without its result line"
      return 1
    }
  }
  scope v3d
  step "run     v3d hw_mock_test (v3d_hw.c against the register model)" v3d_hw_mock_test
  # The TI/Img DDK's RGXStart / RGXStop and firmware image loader
  # (rgxstartstop.c, rgxfwimageutils.c, compiled through driver/compat exactly
  # as the board's build does) on our rgxlayer (driver/rust, feature pvr, built
  # for the host) with a recording register bus (tools/pvr/rgxlayer_check.c):
  # every feature/erratum/value answer = the configuration headers, the image
  # placement = the former loader's, and the start sequence's difference from
  # the one the driver used before = tools/pvr/rgxstart/rgxstart.diff, the stop
  # sequence = rgxstop.trace (tools/pvr/rgxstart/README.md explains each).
  pvr_rgxlayer_check() {
    local out d=build/pvr-check/rgxlayer
    mkdir -p "$d" || return 1
    cargo build -q -p yakogl-drivers --features pvr --target-dir build/pvr-check/rgx-target || return 1
    # shellcheck disable=SC2046  # the flags are one word each
    cc -std=gnu11 -O1 -Wall -Wextra -Werror -DTEE_DDK $(driver/tools/offsetof/ddk-cflags.sh) \
      -Idriver/rust/include tools/pvr/rgxlayer_check.c driver/compat/pvr_rgxstartstop.c \
      driver/compat/pvr_rgxfwimageutils.c driver/compat/pvr_rgxlayer_log.c \
      driver/compat/pvr_rgxlayer_check.c build/pvr-check/rgx-target/debug/libyakogl_drivers.a \
      -o build/pvr-check/rgxlayer_check || return 1
    out=$(build/pvr-check/rgxlayer_check "$d") || { printf '%s\n' "$out" | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE '^rgxlayer_check: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "rgxlayer_check exited 0 without its result line"
      return 1
    }
    diff -u --label main-38438f4d.trace --label rgxstart.trace \
      tools/pvr/rgxstart/main-38438f4d.trace "$d/rgxstart.trace" > "$d/rgxstart.diff"
    cmp -s tools/pvr/rgxstart/rgxstart.diff "$d/rgxstart.diff" || {
      echo "RGXStart's sequence moved: its diff from the former one is not tools/pvr/rgxstart/rgxstart.diff"
      diff tools/pvr/rgxstart/rgxstart.diff "$d/rgxstart.diff" | head -20
      return 1
    }
    cmp -s tools/pvr/rgxstart/rgxstop.trace "$d/rgxstop.trace" || {
      echo "RGXStop's sequence moved"
      diff tools/pvr/rgxstart/rgxstop.trace "$d/rgxstop.trace" | head -20
      return 1
    }
  }
  scope pvr drivers
  step "run     pvr rgxlayer_check (DDK RGXStart/RGXStop/image loader on the Rust rgxlayer)" pvr_rgxlayer_check
  # The PowerVR kernel side in Rust (driver/rust/src/gpu/pvr: mem, mmu, ccb,
  # freelist, fwinit, render; docs/source-form.md step 7): every layout
  # number it uses against the vendored DDK headers (km_check pins ->
  # pins_check, one _Static_assert each, plus the alignment-check table and
  # the MH tag names), and its scenarios on a recording platform
  # (tools/pvr/km/scenario.h) whose records must hash to tools/pvr/km/
  # records.sha256: the records of the C it replaced (record_c.sh, cut out of
  # origin/main 674ea1cf), and fwinit-step7 for the DDK sizes
  # (fwinit-step7.diff). tools/pvr/km/README.md explains each.
  pvr_km_check() {
    local out d=build/pvr-check/km s
    mkdir -p "$d" || return 1
    cargo build -q -p yakogl-drivers --features pvr --target-dir build/pvr-check/rgx-target || return 1
    cc -std=gnu11 -O1 -Wall -Wextra -Werror -Idriver/rust/include -Itools/pvr/km -Idriver/src \
      tools/pvr/km/km_check.c build/pvr-check/rgx-target/debug/libyakogl_drivers.a \
      -o "$d/km_check" || return 1
    "$d/km_check" pins > "$d/rust_pins.h" || return 1
    # shellcheck disable=SC2046  # the flags are one word each
    cc -std=gnu11 -Wall -Wextra -Werror $(driver/tools/offsetof/ddk-cflags.sh) -I"$d" \
      tools/pvr/km/pins_check.c -o "$d/pins_check" || return 1
    out=$("$d/pins_check") || { printf '%s\n' "$out" | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE '^pins_check: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "pins_check exited 0 without its result line"
      return 1
    }
    for s in mem ccb; do
      "$d/km_check" "$s" > "$d/$s-674ea1cf.rec" || return 1
    done
    "$d/km_check" fwinit-former > "$d/fwinit-674ea1cf.rec" || return 1
    "$d/km_check" fwinit > "$d/fwinit-step7.rec" || return 1
    for s in render render-verbose; do
      "$d/km_check" "$s" > "$d/$s.rec" || return 1
      python3 tools/pvr/km/rec_normalize.py < "$d/$s.rec" > "$d/$s-674ea1cf.norm" || return 1
    done
    cp tools/pvr/km/records.sha256 "$d/" || return 1
    (cd "$d" && sha256sum --quiet -c records.sha256) || {
      echo "a kernel-side record moved: tools/pvr/km/README.md says how to diff it against the C"
      return 1
    }
  }
  step "run     pvr km_check (the Rust kernel side = the C it replaced; layouts = the DDK headers)" pvr_km_check
  # Tile at upload: the copy made from the caller's rows (v3d_tex_store_box at
  # the unpack stride) and the copy made at a draw from the GL core's storage
  # (v3d_tex_store_level at tex_pitch) must be the same bytes, for every
  # unpack state (tools/v3d/tex_upload_check.c). No Mesa needed. The row
  # requires its "run (planned N), 0 failed" line.
  v3d_upload_check() {
    local out
    mkdir -p build/v3d-check || return 1
    cc -std=c11 -O1 -Wall -Wextra -Werror "${v3d_inc[@]}" \
      tools/v3d/tex_upload_check.c driver/src/v3d/v3d_tex.c driver/src/v3d/v3d_tiling_mesa.c \
      -lm -o build/v3d-check/tex_upload_check || return 1
    out=$(build/v3d-check/tex_upload_check) || { printf '%s\n' "$out" | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE '^tex_upload_check: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "tex_upload_check exited 0 without its result line"
      return 1
    }
  }
  scope v3d
  step "run     v3d tex_upload_check (eager copy = lazy copy)" v3d_upload_check
  # The V3D OSAL's rule for giving freed GPU memory back
  # (src/osal/v3d_frees.h, which zephyr_v3d.c uses and no host build
  # reaches): tools/v3d/osal_frees_check.c. The row requires its line.
  v3d_frees_check() {
    local out
    mkdir -p build/v3d-check || return 1
    cc -std=c11 -O1 -Wall -Wextra -Werror -Isrc/osal tools/v3d/osal_frees_check.c \
      -o build/v3d-check/osal_frees_check || return 1
    out=$(build/v3d-check/osal_frees_check) || { printf '%s\n' "$out" | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE '^osal_frees_check: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "osal_frees_check exited 0 without its result line"
      return 1
    }
  }
  step "run     v3d osal_frees_check (frees back at once with no job open)" v3d_frees_check
  # The V3D OSAL's carve-out allocator (src/osal/v3d_granules.h, which
  # zephyr_v3d.c uses and no host build reaches): tools/v3d/osal_granules_check.c
  # (first fit and last fit against one-granule-at-a-time models, and the
  # STRESS1D replay, tools/v3d/stress1d_replay.h). The row requires its line.
  v3d_granules_check() {
    local out
    mkdir -p build/v3d-check || return 1
    cc -std=c11 -O1 -Wall -Wextra -Werror -Isrc/osal tools/v3d/osal_granules_check.c \
      -o build/v3d-check/osal_granules_check || return 1
    out=$(build/v3d-check/osal_granules_check) || { printf '%s\n' "$out" | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE '^osal_granules_check: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "osal_granules_check exited 0 without its result line"
      return 1
    }
  }
  step "run     v3d osal_granules_check (the carve-out's first fit, small blocks from the top, the STRESS1D replay)" v3d_granules_check
  # The pending-free queue every deferring OSAL shares (src/osal/pending_frees.h:
  # zephyr.c and zephyr_gc.c / zephyr_v3d.c through
  # gc_frees.h / v3d_frees.h): tools/osal/pending_frees_check.c. A full queue
  # waits for its oldest fence, a failed wait refuses and counts, never a
  # silent leak (a code review finding). The row requires its line.
  osal_pending_frees_check() {
    local out
    mkdir -p build/osal-check || return 1
    cc -std=gnu11 -O1 -Wall -Wextra -Werror -Isrc/osal tools/osal/pending_frees_check.c \
      -o build/osal-check/pending_frees_check || return 1
    out=$(build/osal-check/pending_frees_check) || { printf '%s\n' "$out" | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE '^pending_frees_check: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "pending_frees_check exited 0 without its result line"
      return 1
    }
  }
  scope osal
  step "run     osal pending_frees_check (a full queue waits, never leaks silently)" osal_pending_frees_check
  # The GC7000 Zephyr OSAL's rule for the same (src/osal/gc_frees.h, which
  # zephyr_gc.c uses and no host build reaches): tools/gc/osal_frees_check.c.
  gc_frees_check() {
    local out
    mkdir -p build/gc-check || return 1
    cc -std=c11 -O1 -Wall -Wextra -Werror -Isrc/osal tools/gc/osal_frees_check.c \
      -o build/gc-check/osal_frees_check || return 1
    out=$(build/gc-check/osal_frees_check) || { printf '%s\n' "$out" | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE '^osal_frees_check: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "gc osal_frees_check exited 0 without its result line"
      return 1
    }
  }
  scope vivante
  step "run     gc7000 osal_frees_check (frees back at once with no frame open)" gc_frees_check
  # GPU-neutral OSAL checks (tools/osal/): each program is built from its own
  # .c plus the cc arguments given, and the row requires its
  # "<name>: N checks run (planned N), 0 failed" line.
  # osal_host_check <name> [extra cc arguments]
  osal_host_check() {
    local name=$1 out
    shift
    mkdir -p build/osal-check || return 1
    cc -std=c11 -O1 -Wall -Wextra -Werror -Iinclude -Isrc/osal "$@" "tools/osal/$name.c" \
      -o "build/osal-check/$name" || return 1
    out=$("build/osal-check/$name") || { printf '%s\n' "$out" | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE "^$name: ([0-9]+) checks run \\(planned \\1\\), 0 failed\$" || {
      echo "$name exited 0 without its result line"
      return 1
    }
  }
  # The one gpu_write / gpu_read argument check all seven OSALs make
  # (src/osal/osal_bounds.h): off + n wrapping, the edges, NULL buffers.
  scope osal
  step "run     osal_bounds_check (gpu_write/gpu_read refuse a wrapped range)" \
    osal_host_check osal_bounds_check
  # The line buffer of the Zephyr OSALs' log() (src/osal/osal_log.h): a line
  # of OSAL_LOG_LINE_MAX - 1 bytes, glog!'s longest, comes out whole.
  step "run     osal_log_check (a 255-byte log line survives the Zephyr OSAL buffer)" \
    osal_host_check osal_log_check
  # The bare-metal OSAL fails closed (src/osal/baremetal.c, linked unchanged;
  # no board build compiles it on this host): init refuses a missing or
  # incomplete board registration and touches no address.
  step "run     baremetal_config_check (init refuses without a complete board)" \
    osal_host_check baremetal_config_check -DYAKOGL_OSAL_BAREMETAL src/osal/baremetal.c
  # The Linux host OSAL's fake device-VA allocator (src/osal/linux_heap.h,
  # which src/osal/linux.c uses and no cargo test reaches -- the Rust host
  # tests run against src/testutil.rs's mock OSAL; linux.c itself is linked
  # by tools/hostharness and a `cmake -DYAKOGL_OSAL=linux` build, neither run
  # by this gate): tools/osal/linux_heap_check.c (a finite range, exhaustion,
  # reuse, coalescing incl. into the bump, and a random sequence against a
  # brute-force first-fit model), from a code review finding.
  step "run     linux osal linux_heap_check (finite range, reuse, no overlap)" \
    osal_host_check linux_heap_check
  # The textures are RUN against Mesa 26.2.1, not only compiled: layout,
  # tiling, glTexSubImage2D boxes, the format table, every GL ES 2.0 sampler
  # and the P0/P1 words (tools/v3d/tex_check.c; tools/v3d/tex_check.sh builds
  # it into build/v3d-check/). Nothing but the repo is needed: the Mesa
  # sources vendored in third_party/mesa-26.2.1 are built into
  # build/v3d-check/mesa/ by tools/v3d/mesa_ref.sh. MESA_V3D_TREE, a Mesa tree
  # with build-v3d (tools/v3d/README.md), compares against that tree instead;
  # set and not a Mesa tree, it is an error, not a gap. The row also requires
  # the result line itself, so a run that printed no count cannot pass.
  v3d_mesa_tree=${MESA_V3D_TREE:-}
  v3d_tex_check() {
    local out status
    out=$(tools/v3d/tex_check.sh 2>&1)
    status=$?
    printf '%s\n' "$out" | grep -E 'FAIL|checks run|boxes stored|no Mesa tree|error' | head -20
    [ "$status" -eq 0 ] || return 1
    printf '%s\n' "$out" |
      grep -qE '^tex_check: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "tex_check exited 0 without its result line"
      return 1
    }
  }
  # cl_golden (v3d_cl.h's checked packers against the packers of Mesa's own
  # build, which cl_golden_mesa.c compiles apart) and job_check
  # (the job builder through Mesa's CLE decoder) against the same tree, each
  # built with the command in its own header comment, into build/v3d-check/.
  # job_check writes a Mesa log file into TMPDIR, so TMPDIR is under build/.
  # The row requires the program's own "N <what> run (planned N), 0 failed".
  # v3d_mesa_run <name> <what> <cc arguments, without -o>
  v3d_mesa_run() {
    local name=$1 what=$2 out status
    shift 2
    local dir=build/v3d-check
    mkdir -p "$dir/tmp" || return 1
    if ! "$@" -o "$dir/$name" >"$dir/$name.build.log" 2>&1; then
      tail -20 "$dir/$name.build.log"
      return 1
    fi
    out=$(TMPDIR="$PWD/$dir/tmp" "$dir/$name" 2>&1)
    status=$?
    printf '%s\n' "$out" | grep -E "FAIL|$what run" | head -20
    [ "$status" -eq 0 ] || return 1
    printf '%s\n' "$out" |
      grep -qE "^$name: ([0-9]+) $what run \(planned \1\), 0 failed\$" || {
      echo "$name exited 0 without its result line"
      return 1
    }
  }
  scope v3d
  if [ -z "$v3d_mesa_tree" ]; then
    v3d_ref=build/v3d-check/mesa
    step "build   v3d mesa reference (tools/v3d/mesa_ref.sh)" tools/v3d/mesa_ref.sh
    step "run     v3d tex_check (against Mesa 26.2.1)" v3d_tex_check
    v3d_mt=third_party/mesa-26.2.1
    step "run     v3d cl_golden (against Mesa 26.2.1)" v3d_mesa_run cl_golden cases \
      cc -O1 -std=c11 -D_GNU_SOURCE -DHAVE_ENDIAN_H -I"$v3d_mt/src" -I"$v3d_mt/include" \
      -I"$v3d_mt/src/broadcom" -I"$v3d_ref/gen" "${v3d_inc[@]}" \
      tools/v3d/cl_golden.c tools/v3d/cl_golden_mesa.c -lm
    step "run     v3d job_check (against Mesa 26.2.1)" v3d_mesa_run job_check checks \
      cc -std=c11 -D_GNU_SOURCE -DHAVE_ENDIAN_H -I"$v3d_mt/src" -I"$v3d_mt/include" \
      -I"$v3d_mt/src/broadcom" -I"$v3d_ref/gen" "${v3d_inc[@]}" \
      tools/v3d/job_check.c \
      driver/src/v3d/v3d_job.c driver/src/v3d/v3d_frame.c driver/src/v3d/v3d_tex.c driver/src/v3d/v3d_tiling_mesa.c \
      "$v3d_ref/libmesa_ref.a" -lexpat -lz -lm -lpthread
  elif [ -d "$v3d_mesa_tree/build-v3d" ]; then
    step "run     v3d tex_check (against Mesa 26.2.1)" v3d_tex_check
    v3d_mb=$v3d_mesa_tree/build-v3d
    step "run     v3d cl_golden (against Mesa 26.2.1)" v3d_mesa_run cl_golden cases \
      cc -O1 -std=c11 -D_GNU_SOURCE -DHAVE_ENDIAN_H -I"$v3d_mesa_tree/src" -I"$v3d_mesa_tree/include" \
      -I"$v3d_mesa_tree/src/broadcom" -I"$v3d_mb/src/broadcom" -I"$v3d_mb/src" "${v3d_inc[@]}" \
      tools/v3d/cl_golden.c tools/v3d/cl_golden_mesa.c -lm
    step "run     v3d job_check (against Mesa 26.2.1)" v3d_mesa_run job_check checks \
      cc -std=c11 -D_GNU_SOURCE -DHAVE_ENDIAN_H -I"$v3d_mesa_tree/src" -I"$v3d_mesa_tree/include" \
      -I"$v3d_mesa_tree/src/broadcom" -I"$v3d_mb/src/broadcom" -I"$v3d_mb/src" "${v3d_inc[@]}" \
      tools/v3d/job_check.c \
      driver/src/v3d/v3d_job.c driver/src/v3d/v3d_frame.c driver/src/v3d/v3d_tex.c driver/src/v3d/v3d_tiling_mesa.c \
      "$v3d_mb/src/broadcom/libbroadcom_v3d.a" "$v3d_mb/src/broadcom/cle/libbroadcom_cle.a" \
      "$v3d_mb/src/broadcom/libv3d_neon.a" "$v3d_mb/src/util/libmesa_util.a" \
      -lexpat -lz -lzstd -lm -lpthread
  else
    echo "MESA_V3D_TREE=$v3d_mesa_tree has no build-v3d (tools/v3d/README.md)"
    step "run     v3d tex_check, cl_golden, job_check (MESA_V3D_TREE is not a Mesa tree)" false
  fi
  # The GL suite every board runs, compiled on the host for the PowerVR arm
  # (GL_TEST_HOST) and the GC7000 arm (GL_TEST_USER); the V3D arm is
  # Zephyr-only. gl_internal.h's assertion that the Khronos tokens keep their
  # values is checked here, where a header that shadowed GL_MAX_VERTEX_ATTRIBS
  # once went unseen (the query asked for pname 16 on every board).
  # The GC7000 arm twice: without the test knobs, as a product library has
  # none, and with them (YAKOGL_TEST_KNOBS, the gc_gl_test image's arm).
  scope gltest local
  for arm in "-DGL_TEST_HOST=1" "-DGL_TEST_USER=1 -DYAKOGL_GPU_VIVANTE=1" \
    "-DGL_TEST_USER=1 -DYAKOGL_GPU_VIVANTE=1 -DYAKOGL_TEST_KNOBS=1"; do
    step "cc      gl_test.c $arm" cc -std=gnu11 -fsyntax-only -Wall -Wextra -Werror \
      $arm -Iinclude -Isrc -Isrc/osal -Idriver/tools/pvrlab -Idriver/src \
      -Idriver/src/vivante driver/tools/pvrlab/gl_test.c
  done
  # The set walker's count, RUN (tests/gltest_set_tally.c includes gl_test.c):
  # a by-name skip is skipped, not failed, and the first two failures are
  # named -- the counts the i.MX8MP's CASE_SET record carries, where the
  # skip of case 16 once read as the set's first failure.
  gltest_set_tally() {
    local out
    mkdir -p build || return 1
    cc -std=gnu11 -Wall -Wextra -Werror -DGL_TEST_HOST=1 -Iinclude -Isrc -Isrc/osal \
      -Idriver/tools/pvrlab -Idriver/src tests/gltest_set_tally.c -o build/gltest_set_tally \
      -no-pie -Wl,--unresolved-symbols=ignore-all || return 1
    out=$(build/gltest_set_tally) || { printf '%s\n' "$out" | grep -v ' ok$' | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE '^gltest_set_tally: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "gltest_set_tally exited 0 without its result line"
      return 1
    }
  }
  step "run     gl_test set walker count (a skip is not a failure)" gltest_set_tally
  # Case 44's GC7000 layout probe, RUN (tests/gltest_hdr_probe.c includes
  # gl_test.c as the GC7000 arm): the slot codes the i.MX8MP's CASE record
  # carries, worked on the layouts the board's one datum allows.
  gltest_hdr_probe() {
    local out
    mkdir -p build || return 1
    cc -std=gnu11 -Wall -Wextra -Werror -DGL_TEST_USER=1 -DYAKOGL_GPU_VIVANTE=1 -Iinclude -Isrc -Isrc/osal \
      -Idriver/tools/pvrlab -Idriver/src -Idriver/src/vivante tests/gltest_hdr_probe.c -o build/gltest_hdr_probe \
      -no-pie -Wl,--unresolved-symbols=ignore-all || return 1
    out=$(build/gltest_hdr_probe) || { printf '%s\n' "$out" | grep -v ' ok$' | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE '^gltest_hdr_probe: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "gltest_hdr_probe exited 0 without its result line"
      return 1
    }
  }
  step "run     gl_test case 44 probe decode" gltest_hdr_probe
  # The two headers gl_test.c includes are generated from tools/shaders/, never
  # edited: regenerate them and refuse a drift. They hold the only copy of the
  # GLSL the suite compiles, while the source list, the offline table and the
  # host tests name the files -- the 61 texts gl_test.c wrote as C string
  # literals became tools/shaders/gltest/ (88246148), so a header that is not
  # that text is a suite drawing one program while everything else checks another.
  scope gltest precompile
  step "gen     gl_test_shaders{,_es100}.h == gen_gl_test_shaders.py on tools/shaders" \
    python3 driver/tools/pvrlab/gen_gl_test_shaders.py --check
  # The PowerVR render pool's free lists (driver/rust/src/gpu/pvr/pool.rs, the
  # Rust port of the former driver/src/pvr_render_pool.h, which pvr_render.c
  # calls in the Zephyr build): its host tests, the
  # golden trace of the C pool replayed to equality among them. They also run
  # in the "test    yakogl-drivers" row; this row asserts their population (the
  # count and the golden test by name), so a test that stops being compiled
  # or matched fails here instead of shrinking the row. A full table puts the
  # free on the overflow chain, never drops it (a code review finding). The
  # top region's tests and the glmark2/Slint/cluster replays on the AM62P's
  # layout (pool/tests/replay.rs) are among them.
  pvr_pool_tests() {
    local out want=28
    out=$(cargo_test --libtest --no-fail-fast -p yakogl-drivers --all-features gpu::pvr::pool:: 2>&1) ||
      { printf '%s\n' "$out" | tail -20; return 1; }
    printf '%s\n' "$out" | grep -qE "^test result: ok\. $want passed; 0 failed; 0 ignored;" || {
      echo "pvr pool tests: not exactly $want passed:"
      printf '%s\n' "$out" | grep -E '^test result' | head -3
      return 1
    }
    printf '%s\n' "$out" | grep -qE 'pool::tests::the_rust_pool_replays_the_c_pools_golden_trace \.\.\. ok$' || {
      echo "pvr pool tests: the golden-trace replay did not run"
      return 1
    }
  }
  scope pvr drivers
  step "test    pvr render pool (Rust; the C pool's golden trace replayed, 28 tests)" pvr_pool_tests
  # The AM62P RTI arming arithmetic (driver/zephyr/lib/board/am62p/rti_arm.h):
  # the preload for 60 s at each clock rate the board reported, and the
  # refusal of the MHz parents. The row requires its line.
  pvr_rti_arm_check() {
    local out
    mkdir -p build/pvr-check || return 1
    cc -std=c11 -O1 -Wall -Wextra -Werror tools/pvr/rti_arm_check.c \
      -o build/pvr-check/rti_arm_check || return 1
    out=$(build/pvr-check/rti_arm_check) || { printf '%s\n' "$out" | tail -20; return 1; }
    printf '%s\n' "$out" |
      grep -qE '^rti_arm_check: ([0-9]+) checks run \(planned \1\), 0 failed$' || {
      echo "rti_arm_check exited 0 without its result line"
      return 1
    }
  }
  scope pvr tools
  step "run     am62p rti_arm_check (60 s preload per reported RTI clock rate)" pvr_rti_arm_check
else
  red "check.sh: no C compiler: the v3d FFI pins were NOT checked; this is a gap, not a pass"
  rc=1
fi

# The arms that only a PRODUCT build compiles.
#
# Every configuration above carries `host-test`, and that feature is not just
# extra tests: the crate switches implementations on it, so each
# `#[cfg(not(feature = "host-test"))]` block -- the code that actually ships --
# is invisible to all of them. On 2026-09-22 that let a call to a method that
# does not exist (`Global::get_mut`; `Global::get` already returns `&mut T`)
# sit on main in src/shaderc/online/select.rs, with a green gate, until a
# Zephyr image was built from it and the cargo step failed. The gate said
# "clean" about code it had never compiled.
#
# So these build the library as an image takes it: no `host-test`, and for the
# bare-metal target, which is also the only configuration where a std-only
# dependency creeping into no_std code shows up. `cargo check` rather than
# clippy, because the point is "does it compile at all"; both GPUs, because
# the backends have their own cfg arms.
TARGET_CONFIGS=(
  "osal-zephyr,gpu-vivante,shaderc-online,no-shader-table"
  "osal-zephyr,gpu-powervr,shaderc-online,no-shader-table"
  "osal-zephyr,gpu-v3d,shaderc-online,no-shader-table"
)
# The compiler-less image (`precompiled-shaders` alone), with the PowerVR
# gl_test_es100 table the tier rows use.
target_precompiled() {
  [ -f build/precompiled/check-powervr.rs ] || tier_tables || return 1
  env "YAKOGL_PRECOMPILED_TABLE=$PWD/build/precompiled/check-powervr.rs" \
    cargo check -p yakogl --no-default-features --features osal-zephyr,gpu-powervr,precompiled-shaders \
    --target aarch64-unknown-none
}
if rustup target list --installed 2>/dev/null | grep -qx aarch64-unknown-none; then
  for f in "${TARGET_CONFIGS[@]}"; do
    scope "$(config_scopes "$f")"
    step "check   $f (aarch64-unknown-none, no host-test)" \
      cargo check -p yakogl --no-default-features --features "$f" \
        --target aarch64-unknown-none
  done
  scope pvr precompile
  step "check   osal-zephyr,gpu-powervr,precompiled-shaders (aarch64-unknown-none, no compiler)" target_precompiled
else
  red "check.sh: aarch64-unknown-none is NOT installed -- the product arms were NOT compiled"
  red "          (rustup target add aarch64-unknown-none); this is a gap, not a pass"
  rc=1
fi

# The fault-record decoder repeats constants and word offsets that live in
# driver/zephyr/lib/board/imx8mp/gc_sample.c. A decoder that has drifted from
# the writer reports a confident wrong reason, which is worse than no channel
# at all, so its self-test runs here (six pages, well under a second).
if command -v python3 >/dev/null 2>&1; then
  scope tools
  step "tools   fault-record decoder self-test" python3 tools/cts/fault-record.py --self-test
  # The self-test only proves the decoder agrees with ITSELF. This one reads
  # gc_sample.c and checks every word it writes against what the decoder
  # believes, which is what catches a field moving or appearing. It found one
  # the first time it ran: word 14, the map-time uptime, was being written and
  # silently ignored.
  step "tools   fault-record matches its writer" python3 tools/cts/fault-record.py --check-writer
  # The shader-table generator's two guards, both of which exist because the
  # table it writes SHIPS. One: a directory under tools/shaders/ it was never
  # told about is a failure rather than rows nobody reviewed -- cts-runner's
  # 13 GPU-hanging wedge probes lived there, and a regeneration would have
  # shipped them. Two: the row comparison the rebuild refusal rests on,
  # exercised against the committed table so that a parser which has drifted
  # from the emitted shape fails here rather than reporting "nothing changed".
  scope vivante tools
  step "tools   shader-table generator guards" python3 tools/vivante/gen_shader_table.py --self-test
  # The source list's gl_test programs (tools/shaders/precompiled.list, apps
  # gl_test and gl_test_es100) are what gl_test.c links, read out of its text
  # by glsl2v3d.py's reader: a case that links a new program and a list that
  # was not told fail here, not as a miss on a board.
  scope gltest precompile
  step "tools   precompiled.list gl_test pairs == what gl_test.c links" python3 tools/precompile/gl_test_pairs.py --check
  # The PowerVR driver's USC blobs (driver/src/pvr_usc_*_blob.h) are the
  # rendering of tools/pco_shc/usc_blobs.txt, the list of what Mesa's pco
  # made for each program, and the clear-attach words are the listed ones
  # in both copies (Mesa's, tests/reference/pvr_internal.rs; the backend's,
  # our encoder's, src/pvr/internal.rs). No Mesa build needed; that the list is what pco
  # makes is `gen_usc_blobs.py capture <capture_usc> --check` against a
  # build (tools/pco_shc/README.md), which this row does not run.
  scope pvr
  step "gen     driver/src/pvr_usc_*_blob.h == tools/pco_shc/usc_blobs.txt" \
    python3 tools/pco_shc/gen_usc_blobs.py render --check
  # The Zephyr dEQP tally's two verdicts on a log: MISSING (no summary) and
  # INCOMPLETE (fewer case lines than the summary's "with a result": the
  # serial capture lost lines, which on 2026-09-25 hid 12 mustpass cases
  # behind an exit status of 0).
  scope tools
  step "tools   deqp-uart-tally log verdicts" python3 tools/cts/deqp-uart-tally.py --self-test
  # The common set's tally: board.sh's verdict per row and what it does not
  # say (glmark2 score and scenes, slint probes, gl_test passed of run and the
  # failing cases), on synthetic runs through board.sh's own tally.
  step "tools   common-tally row verdicts" python3 tools/cts/common-tally.py --self-test
  # The .qpa rebuilt from a CONFIG_DEQP_ECHO_QPA console: a case whose end
  # or an image the bridge lost is named, never read as a picture.
  step "tools   qpa-from-uart rebuild verdicts" python3 tools/cts/qpa-from-uart.py --self-test
  # The AM62P netboot (driver/docs/am62p-netboot-plan.md): the one-shot TFTP
  # server on loopback (a 16 MiB transfer with windows, lost packets, block
  # number wrap, one-shot rename, paths outside the root), about 5 s; and the
  # UART phase scan, one verdict per missing phase, "empty" never a pass.
  step "tools   am62p netboot TFTP server" python3 tools/tftp_oneshot.py --self-test
  step "tools   am62p netboot UART phases" python3 tools/am62p_netboot_uart.py --self-test
  # The i.MX8MP netboot's verdicts (driver/docs/imx8mp-netboot-plan.md) are
  # tools/board.sh's now, replayed on the 2026-10-03 records by
  # tools/board_selftest.sh ("tools   board.sh dry-run matrix and replays"
  # below); the record's classes and stage table are
  # driver/rust/src/board/record.rs, decoded by yakogl-tools record ("test
  # yakogl-tools" below).
  # The GLES3 mustpass list (tools/cts/gen-gles3-mustpass.sh at VK-GL-CTS
  # 93bca01): its count is pinned, so a list regenerated from another
  # checkout fails here until the pin is changed on purpose, and a tally that
  # read a shorter file would never learn of the missing cases.
  step "tools   gles3-mustpass: 42,810 cases, sorted, unique, all GLES3" python3 -c '
import sys
l = open("tools/cts/gles3-mustpass.txt").read().splitlines()
ok = len(l) == 42810 and l == sorted(set(l)) and all(x.startswith("dEQP-GLES3.") for x in l)
print(len(l), "cases"); sys.exit(0 if ok else 1)'
  # The AM62P GLES3 tree (tools/cts/am62p-gles3-tree.txt) is generated from
  # that list; a hand edit, a case listed twice or in no image, or an image
  # over 2,000 cases fails here, and a case the board's 600 s watchdog cannot
  # fit is split in the generator, not in the table.
  step "tools   am62p-gles3-tree: generator self-test and table check" sh -c \
    'python3 tools/cts/gen-am62p-gles3-tree.py --self-test && python3 tools/cts/gen-am62p-gles3-tree.py --check'
  # R-23: nothing checked the public headers against what the library
  # exports and what eglGetProcAddress answers, so a header could promise an
  # entry point the archive never carried, or eglGetProcAddress could answer
  # for a name that is not actually exported (and vice versa), and nobody
  # would notice until an application's link or a conformance run hit it.
  # tools/api_coverage.py reads gl2.h + egl.h (declared), `nm`s the
  # yakogl-capi archive `cargo build -p yakogl-capi` above already built
  # (exported), and parses the PROCS table in src/gl/egl.rs (procaddress).
  scope tools pvr
  step "tools   api_coverage.py self-test" python3 tools/api_coverage.py --self-test
  step "tools   api coverage: gl2.h + egl.h vs exported vs eglGetProcAddress" python3 tools/api_coverage.py
  # The population, not only its zero: a declared-but-not-exported name does
  # not fail api_coverage.py itself (that is a documented gap, not an
  # inconsistency of ours -- see the tool's docstring), so its count is
  # pinned here instead. Both are 0 today: the ES 2.0 core (gl2.h) and the
  # EGL 1.x core (egl.h) are both fully exported. A regression (an entry
  # point lost) and a silent improvement (one gained without updating this
  # pin) are both a red row, never a quiet pass.
  step "tools   api coverage: 0 ES 2.0 core + 0 EGL 1.x core entry points missing from the exports" python3 -c '
import re, subprocess, sys
out = subprocess.run([sys.executable, "tools/api_coverage.py"], capture_output=True, text=True).stdout
line = next((l for l in out.splitlines() if l.startswith("PINNED ")), None)
if line is None:
    print("no PINNED line in api_coverage.py output"); sys.exit(1)
m = re.match(r"PINNED missing_gl2=(\d+) missing_egl=(\d+)", line)
missing_gl2, missing_egl = int(m.group(1)), int(m.group(2))
print(line)
sys.exit(0 if (missing_gl2, missing_egl) == (0, 0) else 1)'

fi

# RUN the board setup script, do not read it.
#
# board-sudo-setup.sh shipped unrunnable: the fault-record wrapper's heredoc
# expanded $FAULT_RECORD_ADDR at line 132 while the default that sets it sat at
# line 207, so under `set -u` it died with "unbound variable" the first time
# anyone ran it from a clean environment -- which was the user, installing the
# wrapper this work is blocked on. It had been reviewed by reading.
#
# --dry-run prints what it would install and touches no machine, so the gate can
# execute it. `env -u` is the point: inheriting the variable from the developer's
# shell is exactly how the defect stayed invisible here.
scope tools
step "tools   board-sudo-setup.sh --dry-run runs from a clean environment" \
  env -u FAULT_RECORD_ADDR -u KO_DIR -u KEXEC_DIR BOARD_HOST=torizon@gate.invalid \
    driver/tools/kexec-zephyr/board-sudo-setup.sh --dry-run

# The table is GONE from the artefact, not merely absent from the source.
#
# A `#[cfg]` that compiles is not evidence: the point of `no-shader-table` is
# bytes that are not in the image, so the check reads the built archive. The
# pattern is the table MODULE (`mesa_reference`), not a row's name: the
# driver's own programs (`pvr::internal`, `vivante::internal`, our
# compiler's) are rows of the same shape and belong in every image, so a
# looser pattern reports their symbols and invites someone to decide they
# are close enough to zero.
#
# Both arms are built and counted, because "0 symbols" proves nothing unless
# the same command finds them when the table IS there.
#
# The archive is read where cargo put it, asked of cargo: a fixed `.target/`
# held only in the shells that export CARGO_TARGET_DIR=.target, and anywhere
# else it read a stale archive from an earlier build, the control's 0 failing
# a tree whose freshly built archive held 1,200 table symbols (2026-09-23).
table_symbols() {
  local dir
  dir=$(cargo metadata --no-deps --format-version 1 |
    python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])') || return 1
  cargo build --release --no-default-features --features "$1" >/dev/null 2>&1 || return 1
  local rlib="$dir/release/libyakogl.rlib" n
  n=$(nm --defined-only "$rlib" 2>/dev/null | grep -cE '14mesa_reference|SHC_BLOB_TABLE' || true)
  # The release profile is `lto = true`, and cargo may then build the rlib's
  # object as LLVM bitcode alone, which an nm without the LLVM plugin (GNU
  # binutils 2.46 here, 2026-10-08) reads as "no symbols" -- not one symbol
  # in the whole archive. The table's names are still in its bytes, so they
  # are counted there instead; the control arm keeps this honest (3 with the
  # table, 0 without, measured).
  if [ "$n" -eq 0 ] && ! nm --defined-only "$rlib" 2>/dev/null | grep -qE ' [TtDdRrBb] '; then
    n=$(grep -c -a -E '14mesa_reference|SHC_BLOB_TABLE' "$rlib" || true)
  fi
  echo "$n"
}

check_table_absent() {
  local with without
  with=$(table_symbols "host-test,gpu-vivante,shaderc-online") || return 1
  without=$(table_symbols "host-test,gpu-vivante,shaderc-online,no-shader-table") || return 1
  if [ "$with" -eq 0 ]; then
    echo "the control found no table symbols with the table compiled in ($with):"
    echo "the pattern no longer matches, so the 'without' count means nothing"
    return 1
  fi
  if [ "$without" -ne 0 ]; then
    echo "no-shader-table still links $without table symbol(s)"
    return 1
  fi
  echo "table symbols: $with with the table, $without without"
}

scope vivante precompile
step "no table in the artefact" check_table_absent

# No Mesa reference in a product artefact. The Mesa-built tables
# (tests/reference/, `shaderc::mesa_reference`) are host-test material; the
# one product build that may link them is a measurement image that asks for
# them as its table (`mesa-reference-table`: Kconfig YAKOGL_SHADERC_OFFLINE,
# the C builds' SHADERC offline). This builds the archive an image links
# (yakogl-capi, release, as capi/yakogl_capi.cmake does) for the product
# configurations and counts the reference's symbols in it: the modules
# (`14mesa_reference`; the internal programs' `13mesa_internal` and
# `17mesa_clear_shader`; v0 mangling) and the two tables. The control is the
# same command on a measurement build, which must find them. Every OSAL,
# every GPU, the in-process compiler with and without `no-shader-table`, on
# the image target; the host target once (osal-linux). --quick builds the
# control and the AM62P image only; the full matrix (20 archives and two controls) took 863 s here.
mesa_reference_symbols() {
  local dir target="$1" features="$2" lib table=()
  case "$features" in
  *precompiled-shaders*gpu-vivante* | *gpu-vivante*precompiled-shaders*)
    table=(env "YAKOGL_PRECOMPILED_TABLE=$PWD/build/precompiled/check-gc7000.rs") ;;
  *precompiled-shaders*) table=(env "YAKOGL_PRECOMPILED_TABLE=$PWD/build/precompiled/check-powervr.rs") ;;
  esac
  dir=$(cargo metadata --no-deps --format-version 1 |
    python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])') || return 1
  if [ "$target" = host ]; then
    "${table[@]}" cargo build -q -p yakogl-capi --release --no-default-features --features "$features" \
      >/dev/null 2>&1 || return 1
    lib="$dir/release/libyakogl_capi.a"
  else
    "${table[@]}" cargo build -q -p yakogl-capi --release --target "$target" --no-default-features \
      --features "$features" >/dev/null 2>&1 || return 1
    lib="$dir/$target/release/libyakogl_capi.a"
  fi
  nm "$lib" 2>/dev/null |
    grep -cE '14mesa_reference|13mesa_internal|17mesa_clear_shader|SHC_BLOB_TABLE|SHC_SOURCE_TABLE' || true
}

check_no_mesa_reference() {
  local t=aarch64-unknown-none n f configs=()
  if ! rustup target list --installed 2>/dev/null | grep -qx "$t"; then
    echo "$t is not installed (rustup target add $t): the image archives were NOT built"
    return 1
  fi
  for f in "osal-zephyr,gpu-powervr,mesa-reference-table" "osal-zephyr,gpu-vivante,mesa-reference-table"; do
    n=$(mesa_reference_symbols "$t" "$f") || { echo "the control build [$f] failed"; return 1; }
    if [ "$n" -eq 0 ]; then
      echo "the control [$f] links no reference symbol: the pattern no longer matches, so 0 below means nothing"
      return 1
    fi
    echo "control $f: $n reference symbols"
    [ "$QUICK" -eq 1 ] && break
  done
  [ -f build/precompiled/check-powervr.rs ] && [ -f build/precompiled/check-gc7000.rs ] || tier_tables || return 1
  if [ "$QUICK" -eq 1 ]; then
    configs=("$t osal-zephyr,gpu-powervr,shaderc-online,no-shader-table"
      "$t osal-zephyr,gpu-powervr,precompiled-shaders")
  else
    for osal in zephyr baremetal linux; do
      for gpu in powervr vivante v3d; do
        configs+=("$t osal-$osal,gpu-$gpu,shaderc-online,no-shader-table" "$t osal-$osal,gpu-$gpu,shaderc-online")
      done
    done
    configs+=("host osal-linux,gpu-powervr,shaderc-online" "host osal-linux,gpu-vivante,shaderc-online")
    # the precompiled shaders, alone and in front of the compiler
    for gpu in powervr vivante; do
      configs+=("$t osal-zephyr,gpu-$gpu,precompiled-shaders"
        "$t osal-zephyr,gpu-$gpu,shaderc-online,no-shader-table,precompiled-shaders")
    done
  fi
  for c in "${configs[@]}"; do
    n=$(mesa_reference_symbols "${c%% *}" "${c#* }") || { echo "the product build [$c] failed"; return 1; }
    if [ "$n" -ne 0 ]; then
      echo "the product archive [$c] links $n Mesa reference symbol(s)"
      return 1
    fi
  done
  echo "${#configs[@]} product archive(s), 0 reference symbols each"
}

scope pvr vivante v3d precompile
step "no Mesa reference in the artefact" check_no_mesa_reference

# No test knob in the artefact either, read the same way: the C archive a
# GC7000 product image links (yakogl-capi with the Zephyr image's features,
# the in-process compiler and no table) must define no gc_debug_* or
# gc_perf_* symbol at all. The same archive with `test-knobs`, what
# gc_gl_test links, must define exactly the knobs, which is the control: a
# pattern that stopped matching would find neither. Exact sets, so a new
# `#[no_mangle]` switch fails here until it is moved behind the feature and
# listed.
KNOB_TEST_SYMBOLS="gc_debug_flip_experiment gc_debug_msaa gc_debug_msaa_scratch gc_debug_msaa_scratch_map
gc_debug_msaa_scratch_zero gc_debug_narrow_la gc_debug_set_flip_experiment gc_debug_set_msaa
gc_debug_set_narrow_la"
knob_symbols() {
  local dir
  dir=$(cargo metadata --no-deps --format-version 1 |
    python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])') || return 1
  cargo build -p yakogl-capi --release --no-default-features --features "$1" >/dev/null 2>&1 || return 1
  nm --defined-only -g "$dir/release/libyakogl_capi.a" 2>/dev/null |
    awk 'NF == 3 && $3 ~ /^gc_(debug|perf)_/ { print $3 }' | sort -u
}

check_knobs_absent() {
  local product knobs want_knobs
  product=$(knob_symbols "osal-zephyr,gpu-vivante,shaderc-online,no-shader-table") || return 1
  knobs=$(knob_symbols "osal-zephyr,gpu-vivante,shaderc-online,no-shader-table,test-knobs") || return 1
  want_knobs=$(printf '%s\n' $KNOB_TEST_SYMBOLS | sort -u)
  if [ "$knobs" != "$want_knobs" ]; then
    echo "the test-knobs archive's gc_debug_*/gc_perf_* symbols are not the expected set:"
    diff <(printf '%s\n' "$want_knobs") <(printf '%s\n' "$knobs")
    return 1
  fi
  if [ -n "$product" ]; then
    echo "the product archive defines gc_debug_*/gc_perf_* symbols:"
    printf '%s\n' "$product"
    return 1
  fi
  echo "gc_debug_* symbols: 0 in the product archive, $(printf '%s\n' "$knobs" | wc -l) with test-knobs"
}

scope vivante
step "no test knob in the artefact" check_knobs_absent

# The rows above name THIS package's features, so they never build the
# workspace's other crates. `yakogl-tools` links the library and calls into it,
# so a change to a public signature breaks it and every row still passes:
# `sema::check` gained a return value and `online-check` went on compiling
# `Ok(())` until someone ran the binary by hand. One row closes that.
scope toolscrate
step "clippy  yakogl-tools" cargo clippy -p yakogl-tools --all-targets
# Clippy alone does not run its tests: the cube helper's level-aware clamp
# (6874c8ee) changed the GLSL prelude and three glsl::tests stayed red on main
# under a clean gate.
step "test    yakogl-tools" cargo_test -p yakogl-tools
# yakogl-precompile (tools/precompile), the build-time precompiler of the
# source list: it links the library as an image does -- the in-process
# compiler, no table, no `host-test` -- so no row above builds that
# configuration with std around it. Lint it for every build it makes a table
# for, and run its tests for both GPUs (the GC7000 one has the link-time
# attribute layout; about 15 s and 1 s).
scope precompile pvr vivante
for f in gpu-powervr gpu-vivante gpu-powervr,report-es2 gpu-vivante,report-es2; do
  step "clippy  yakogl-precompile $f" cargo clippy -p yakogl-precompile --features "$f" --all-targets
done
step "test    yakogl-precompile gpu-powervr" cargo_test -p yakogl-precompile --features gpu-powervr
step "test    yakogl-precompile gpu-vivante" cargo_test -p yakogl-precompile --features gpu-vivante
# The back ends' own programs (the frame and clear quads' shaders; the list's
# `internal` lines), committed as src/pvr/internal.rs and
# src/vivante/internal.rs because every image binds them, compiler or not:
# made again by our compiler and required to be the committed bytes, so a
# compiler change that moves one of them moves the file in the same commit
# (and tests/internal_programs.rs runs the new programs against Mesa's).
step "gen     src/pvr/internal.rs == yakogl-precompile --internal (our compiler)" \
  cargo run -q -p yakogl-precompile --features gpu-powervr -- --internal --verify src/pvr/internal.rs
step "gen     src/vivante/internal.rs == yakogl-precompile --internal (our compiler)" \
  cargo run -q -p yakogl-precompile --features gpu-vivante -- --internal --verify src/vivante/internal.rs
# tools/board.sh, the one runner for the three boards (driver/docs/one-method.md
# section 4), without a board: every verb on every board and backend under
# --dry-run with ssh/scp/curl/ping/flock/stty/sudo stubbed (none may run), the
# refusals, and the 2026-10-03 AM62P captures, i.MX8MP records, Pi 4 records
# and the i.MX8MP full-tree results replayed through its verdict, tally and
# diff, each against what the old runner prints for the same input. The check
# count is pinned (about 35 s).
scope tools
step "tools   board.sh dry-run matrix and replays" tools/board_selftest.sh
# The map --changed selects rows by, pinned path by path, and the plans of
# a docs, a V3D, a gl_test and a shared-layer change.
scope tools local
step "tools   check.sh --changed: the path-to-scope map and four plans" tools/check_scope_selftest.sh
# cargo_test's cache: a recorded binary is skipped, a new key runs it again,
# a failing binary is named and never recorded.
step "tools   check.sh test cache: record, skip, run again on a new key" tools/check_testcache_selftest.sh
# The PowerVR backend's view of Mesa's csbgen packers (src/pvr/csbgen.rs and
# the C wrappers build.rs compiles, driver/src/pvr/generated/csbgen_wrap.c)
# is bindgen's output over driver/compat/pvr_csbgen_rs.h, committed so that no
# build needs libclang: regenerate it here and require the committed bytes.
# bindgen loads libclang at run time (clang-sys: LIBCLANG_PATH, else the
# usual places), which this machine has; a gate without it is a red row.
scope pvr toolscrate
step "gen     src/pvr/csbgen.rs + csbgen_wrap.c == bindgen" \
  cargo run -q -p yakogl-tools -- csbgen-bindings --check

# The display/SoC drivers (driver/rust, the yakogl-drivers crate): its own
# lint and its host tests against the register model (src/model.rs), with both
# SoC features, since no row above names the package. `cargo fmt --check` at the
# top already covers it (a workspace member).
# The one Slint sample's frame check (driver/samples/zephyr/slint/check): the
# AM62P reference table, the tolerance and the record words, on synthetic
# frames. The Zephyr images themselves are not built here.
scope samples
step "clippy  yakogl-slint-check" cargo clippy -p yakogl-slint-check --all-targets
step "test    yakogl-slint-check" cargo_test -p yakogl-slint-check
# The cluster sample's check and records (driver/samples/zephyr/cluster/check):
# the host reference's tables per panel size, the tolerance, the present
# accounting and the record words, on synthetic frames.
step "clippy  yakogl-cluster-check" cargo clippy -p yakogl-cluster-check --all-targets
step "test    yakogl-cluster-check" cargo_test -p yakogl-cluster-check
# The one triangle sample's scene and check (driver/samples/zephyr/triangle/check):
# the whole-window classification, the tolerance, the PICTURE packing and the
# shader texts against both offline tables, on synthetic frames.
step "clippy  yakogl-triangle-check" cargo clippy -p yakogl-triangle-check --all-targets
step "test    yakogl-triangle-check" cargo_test -p yakogl-triangle-check
scope drivers pvr vivante
step "clippy  yakogl-drivers" cargo clippy -p yakogl-drivers --all-features --all-targets
# --all-features is not every arm that ships: a cfg arm can lint clean with all
# features on and fail with one of them off (78c4c208's fence-wait loop was
# the literal empty range 1..1 without pvr-fine-wait, which clippy refuses,
# and this row did not see it). So clippy each feature set an image or a
# knob builds, alone, with -D warnings: none, each SoC, the PowerVR rgxlayer
# alone and as the AM62P builds it, with each opt-in knob and both, and the
# i.MX 8M Plus GC7000 side. The V3D has no feature in this crate (its
# driver is C, driver/src/v3d). A new feature or knob joins this list.
drivers_clippy_arms() {
  local f
  for f in "" imx8mp am62p pvr am62p,pvr am62p,pvr,pvr-fine-wait am62p,pvr,pvr-kick-times \
    am62p,pvr,pvr-fine-wait,pvr-kick-times am62p,pvr,pvr-grow-scene imx8mp,vivante; do
    echo "== yakogl-drivers features [$f]"
    cargo clippy -p yakogl-drivers --no-default-features --features "$f" --all-targets -- -D warnings ||
      { echo "clippy refused yakogl-drivers features [$f]"; return 1; }
  done
}
step "clippy  yakogl-drivers, each shipped feature arm (-D warnings)" drivers_clippy_arms
step "test    yakogl-drivers (register model)" cargo_test --no-fail-fast -p yakogl-drivers --all-features
# The target it ships for: aarch64-unknown-none-softfloat (no FP/SIMD registers:
# the code runs at PRE_KERNEL_1 and in ISRs). With the target installed for the
# default toolchain that is a plain check; otherwise a nightly with rust-src
# builds core for it; with neither the row is a gap. The feature sets an image
# builds: all SoC drivers; the AM62P's (the GPU's rgxlayer and the display
# path); the i.MX 8M Plus GC7000 kernel side (gpu::vivante); and
# none (only the SoC-independent I2C driver, ip::sn65dsi8x).
drivers_softfloat_check() {
  local t=aarch64-unknown-none-softfloat f
  for f in imx8mp,am62p,pvr,vivante am62p,pvr imx8mp,vivante ""; do
    if rustup target list --installed 2>/dev/null | grep -qx "$t"; then
      cargo check -p yakogl-drivers --no-default-features --features "$f" --target "$t" || return 1
    else
      CARGO_FUTURE_INCOMPAT_REPORT_FREQUENCY=never cargo +nightly check -p yakogl-drivers \
        --no-default-features --features "$f" --target "$t" -Z build-std=core || return 1
    fi
  done
}
if rustup target list --installed 2>/dev/null | grep -qx aarch64-unknown-none-softfloat ||
  [ -d "$(rustc +nightly --print sysroot 2>/dev/null)/lib/rustlib/src/rust/library/core" ]; then
  step "check   yakogl-drivers (aarch64-unknown-none-softfloat)" drivers_softfloat_check
else
  gap "yakogl-drivers for aarch64-unknown-none-softfloat: neither the target (rustup target add) nor a nightly with rust-src"
fi
if command -v cc >/dev/null 2>&1; then
  # The C view of the crate's structs, with the layout pins the Rust side
  # asserts too (driver/rust/include/yakogl_drivers.h).
  step "cc      yakogl_drivers.h layout pins" cc -std=c11 -fsyntax-only -Wall -Wextra -Werror \
    -x c driver/rust/include/yakogl_drivers.h
fi
# One Zephyr image per GPU, built with -DYAKOGL_WERROR=ON: our C files (the
# gl_test suite, the samples' main.c, the OSAL and the layout pins) compile
# without a warning. The cargo rows never compile them, so the i.MX8MP and
# AM62P gl_test images carried three unused-function warnings (cases 142,
# 145, 148, dispatched only on the V3D) that nothing reported. Zephyr's own
# files stay outside -Werror (tisci.c warns). ZEPHYR_WS is the west
# workspace; without one the rows are gaps. About 1 min an image on a warm
# host (56, 58 and 46 s measured), so not in --quick.
zephyr_werror_image() {
  local board="$1" sample="$2" dir="build/zephyr-werror/$3"
  shift 3
  local mods log="$dir.log" root="$PWD"
  mods=$(PATH="$ZEPHYR_WS/.venv/bin:$PATH" tools/cts/zephyr-modules.sh "$ZEPHYR_WS") || return 1
  mkdir -p build/zephyr-werror
  if ! (cd "$ZEPHYR_WS" && PATH="$ZEPHYR_WS/.venv/bin:$PATH" ZEPHYR_BASE="$ZEPHYR_WS/zephyr" \
    west build -p always -b "$board" -d "$root/$dir" "$root/driver/samples/zephyr/$sample" -- \
    "-DZEPHYR_MODULES=$mods" "-DYAKOGL_DIR=$root" -DYAKOGL_WERROR=ON "$@") >"$log" 2>&1 </dev/null; then
    grep -E 'error:|Error ' "$log" | head -20
    echo "(the whole log: $log)"
    return 1
  fi
}
# The image without a compiler (the precompiled shaders of gl_test's ES 1.00
# set its whole back end): it builds, and neither the compiler nor the
# scratch region the OSAL lends it (zephyr_shaderc_scratch.c) is in it.
zephyr_precompiled_only_image() {
  local elf=build/zephyr-werror/am62p-precompiled-only/zephyr/zephyr.elf syms
  zephyr_werror_image verdin_am62p/am62p54/a53 gl_test am62p-precompiled-only \
    -DCONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y -DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n \
    -DCONFIG_YAKOGL_SHADERC_PRECOMPILED_ONLY=y '-DCONFIG_YAKOGL_PRECOMPILED_APPS="gl_test_es100"' || return 1
  syms=$(nm "$elf") || return 1
  grep -q 'shaderc11precompiled5alone15SHC_PRECOMPILED' <<<"$syms" || {
    echo "the image has no SHC_PRECOMPILED back end"
    return 1
  }
  if grep -E 'yakogl_shaderc_scratch|shc_online_|7shaderc6online' <<<"$syms" | head -5 | grep .; then
    echo "^ the compiler or its scratch region is in the image without a compiler"
    return 1
  fi
}
if [ "$QUICK" -eq 0 ]; then
  scope pvr vivante v3d gltest drivers samples precompile
  if [ -n "${ZEPHYR_WS-}" ] && [ -d "${ZEPHYR_WS-}/zephyr" ]; then
    scope pvr gltest drivers samples precompile
    step "zephyr  am62p gl_test image, -Werror on our C files" zephyr_werror_image \
      verdin_am62p/am62p54/a53 gl_test am62p \
      -DCONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y -DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n
    step "zephyr  am62p gl_test image without a compiler (precompiled shaders only), -Werror" \
      zephyr_precompiled_only_image
    scope vivante gltest drivers samples
    step "zephyr  imx8mp gc_gl_test image, -Werror on our C files" zephyr_werror_image \
      verdin_imx8mp/mimx8ml8/a53 gc_gl_test imx8mp -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y
    scope v3d gltest drivers samples
    step "zephyr  rpi4 v3d_gl_test image, -Werror on our C files" zephyr_werror_image rpi_4b v3d_gl_test rpi4
  else
    gap "zephyr images: no west workspace at ZEPHYR_WS='${ZEPHYR_WS-}' (set ZEPHYR_WS): our C files were NOT compiled with -Werror"
  fi
fi

# The precompiled tables (yakogl-precompile, tools/precompile/prove.sh): for
# each build, the whole list's table made twice is the same bytes, each of
# its rows is what the compiler makes for that row alone in a fresh cache,
# its row and code-byte counts and those of two applications' sublists
# (femtovg; gl_test's ES 1.00 build) are tests/fixtures/precompile_counts.txt's,
# and it compiles against the library. Measured on a warm tree, the tool's
# build for each feature set included: 51 s (PowerVR), 164 s (GC7000), 35 s
# and 5 s (their ES 2.0 builds), so not in --quick.
#
# Then the Mesa table from the same list: tools/glsl2usc.sh, which reads its
# inputs and variants from tools/shaders/precompiled.list, regenerates
# tests/reference/blobs.rs byte for byte with the toolchain the table records
# (PCO_SHC; GLSLANG; GLSL2USC_CACHE keeps a rerun at about 3.5 minutes).
# Without PCO_SHC the row is a gap: the list's PowerVR half was not shown to
# be the committed table's.
mesa_table_repro() {
  local out=build/mesa-repro/blobs.rs log=build/mesa-repro/glsl2usc.log
  mkdir -p build/mesa-repro && rm -f "$out"
  if ! env -u GLSL2USC_NEW_REFERENCE GLSL2USC_OUT="$PWD/$out" tools/glsl2usc.sh >"$log" 2>&1; then
    tail -20 "$log"
    return 1
  fi
  # The rows are the reference. The header's `// toolchain:` line names the
  # pco_shc and glslang BINARIES (the first 12 hex digits of their SHA-256),
  # and two hosts' builds of one Mesa source differ in that hash while every
  # USC row is the same (tools/pco_shc/build_pco_shc.sh, 2026-10-07 vs
  # 2026-10-10). So the ids are printed as provenance and the rows are
  # compared byte for byte; a pco_shc whose rows differ still fails here.
  local recorded here
  recorded=$(grep -m1 '^// toolchain: ' tests/reference/blobs.rs | cut -c15-)
  here=$(grep -m1 '^// toolchain: ' "$out" | cut -c15-)
  cmp <(grep -v '^// toolchain: ' "$out") <(grep -v '^// toolchain: ' tests/reference/blobs.rs) || {
    echo "the table glsl2usc.sh makes from precompiled.list is not tests/reference/blobs.rs (log: $log)"
    echo "  recorded toolchain: $recorded"
    echo "  this host:          $here"
    return 1
  }
  echo "$(grep -c '^    ShcBlob {$' "$out") rows, byte for byte (recorded $recorded; this host $here)"
}
if [ "$QUICK" -eq 0 ]; then
  for f in gpu-powervr gpu-vivante gpu-powervr,report-es2 gpu-vivante,report-es2; do
    scope "$(config_scopes "$f")" precompile
    step "gen     precompiled table $f: deterministic, rows == fresh compiles, counts pinned, compiles" \
      tools/precompile/prove.sh "$f"
  done
  scope pvr precompile
  if [ -n "${PCO_SHC-}" ] && [ -x "${PCO_SHC-}" ]; then
    step "gen     tests/reference/blobs.rs == tools/glsl2usc.sh on precompiled.list (pco_shc)" mesa_table_repro
  else
    gap "tests/reference/blobs.rs from precompiled.list: no pco_shc at PCO_SHC='${PCO_SHC-}' (set PCO_SHC, GLSLANG, GLSL2USC_CACHE): the list was NOT shown to make the committed table"
  fi
fi

# glmark2's shaders through the online compiler of each back end
# (tests/glmark2_corpus.rs). The corpus is GPL-3.0 text and never committed:
# tools/hostharness/glmark2/run.sh generates it from GLMARK2_SRC (glmark2 at
# the pinned commit) into build/glmark2-host/shaders. Without GLMARK2_SRC the
# row is a gap. The same run's logs must hold no depth texture sampled stale
# (shadow and refract on the PowerVR, pvr-depth-resolve: "the backend did not
# resolve its depth").
glmark2_corpus_tests() {
  local c stale
  GM2_WRITE=0 tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-host powervr vivante \
    >build/glmark2-host.log 2>&1 || { tail -20 build/glmark2-host.log; return 1; }
  stale=$(grep -l 'did not resolve its depth' build/glmark2-host/logs/*/*.log || true)
  [ -z "$stale" ] || { echo "a depth texture sampled stale in: $stale"; return 1; }
  # Plain `cargo test`, not cargo_test: the corpus is generated under
  # build/, outside git, so no key of cargo_test covers it.
  for c in host-test,gpu-powervr,shaderc-online host-test,gpu-vivante,shaderc-online \
    host-test,gpu-v3d,shaderc-online,no-shader-table; do
    GLMARK2_CORPUS="$PWD/build/glmark2-host/shaders" cargo test --no-default-features --features "$c" \
      --test glmark2_corpus -- --nocapture 2>&1 | grep -E '^(SKIP|test result|(powervr|gc7000|v3d):)' || return 1
  done
}
if [ "$QUICK" -eq 0 ]; then
  # tools: the row runs tools/hostharness/glmark2/run.sh, so a change there
  # reaches it (the scope map sends tools/hostharness/* to `tools`).
  scope pvr vivante v3d tools
  if [ -n "${GLMARK2_SRC-}" ] && [ -d "${GLMARK2_SRC-}/src" ]; then
    mkdir -p build
    step "test    glmark2 corpus generated from GLMARK2_SRC compiles and links (tests/glmark2_corpus.rs)" glmark2_corpus_tests
  else
    gap "glmark2 corpus: no glmark2 tree at GLMARK2_SRC='${GLMARK2_SRC-}' (set GLMARK2_SRC): the glmark2 shaders were NOT compiled"
  fi
fi

# driver/rust/src/soc/imx8mp/regs.rs is generated from NXP's BSD-3 hal_nxp
# headers (tools/hal-nxp/gen_regs.py), never edited: regenerate and compare.
# The generator itself refuses inputs whose sha256 is not the pinned one (hal_nxp
# 1e91007b), so a different hal_nxp fails here rather than drifting silently.
# HAL_NXP_DIR is the hal_nxp checkout (Zephyr's modules/hal/nxp); without one
# the row is a gap.
hal_nxp_dir=${HAL_NXP_DIR:-$HOME/com/github/zephyrproject-rtos/modules/hal/nxp}
if command -v python3 >/dev/null 2>&1; then
  scope vivante tools
  step "tools   hal-nxp gen_regs self-test" python3 tools/hal-nxp/gen_regs.py --self-test
  scope vivante
  if [ -d "$hal_nxp_dir/mcux/mcux-sdk-ng/devices/i.MX/i.MX8MP/periph" ]; then
    hal_nxp_regs_regenerate() {
      python3 tools/hal-nxp/gen_regs.py "$hal_nxp_dir" | cmp - driver/rust/src/soc/imx8mp/regs.rs
    }
    scope vivante
    step "gen     driver/rust imx8mp regs.rs == gen_regs.py on hal_nxp 1e91007b" hal_nxp_regs_regenerate
  else
    gap "imx8mp regs.rs: no hal_nxp at $hal_nxp_dir (set HAL_NXP_DIR): the generated registers were NOT checked"
  fi
  # driver/rust/src/soc/am62p/ti_pr114866.rs: the four D-PHY / DSI facts the
  # AM62Px TRM does not state, generated from TI's Zephyr driver vendored
  # unmodified in third_party/zephyr-pr114866-2026-07-29-2473da6b (the
  # generator refuses a file whose sha256 is not the pinned one).
  scope pvr drivers
  step "gen     driver/rust am62p ti_pr114866.rs == gen_dphy_data.py on third_party" \
    python3 tools/ti-zephyr/gen_dphy_data.py --check
  # driver/compat/pvr_rgx_core.h spells out every feature, erratum and value of
  # the vendored DDK's rgx_bvnc_defs_km.h for the rgxlayer pins; generated.
  scope pvr
  step "gen     driver/compat/pvr_rgx_core.h == gen_rgx_core.py" python3 tools/pvr/gen_rgx_core.py --check
fi

summary
