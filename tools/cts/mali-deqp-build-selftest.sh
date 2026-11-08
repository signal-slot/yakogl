#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The self-test of tools/cts/mali-deqp-build.sh, the NanoPC-T6 deqp-gles2
# cross build, without network, cross toolchain or board: the script is copied
# into a scratch git repository (it finds its repository from its own
# location) and dry-run there with fakes on PATH that fail if called (cargo,
# cmake, ninja, curl, git clone/fetch, the cross compiler), plus its symbol
# floor helpers on version lists. Fixtures: tools/cts/testdata/mali-deqp-build/
# (README.md there).
#
#   tools/cts/mali-deqp-build-selftest.sh   prints "mali-deqp-build-selftest: N checks passed"
#
# Host only, a few seconds.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
FIX=$HERE/testdata/mali-deqp-build
BUILD_SH=$HERE/mali-deqp-build.sh
EXPECT=29
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mali-deqp-build-selftest-XXXXXX")
cleanup() {
  if [ "${MALI_DEQP_SELFTEST_KEEP-}" = 1 ]; then echo "mali-deqp-build-selftest: kept $tmp"; else rm -rf "$tmp"; fi
}
trap cleanup EXIT
passed=0 failed=0
check() { # <what> <command...>
  local what=$1
  shift
  if "$@" > /dev/null 2>&1; then
    passed=$((passed + 1))
    echo "  ok    $what"
  else
    failed=$((failed + 1))
    echo "  FAIL  $what"
  fi
}

# Fakes that log and fail: a dry run must call none of them. git is a wrapper
# that fails on clone and fetch and passes everything else (rev-parse, status).
mkdir -p "$tmp/nobin"
for t in cargo cmake ninja curl wget rustc aarch64-linux-gnu-g++ aarch64-linux-gnu-gcc aarch64-linux-gnu-objdump aarch64-linux-gnu-readelf; do
  printf '#!/bin/bash\necho "%s $*" >> "%s/called.log"\nexit 99\n' "$t" "$tmp" > "$tmp/nobin/$t"
done
real_git=$(command -v git)
cat > "$tmp/nobin/git" <<EOF
#!/bin/bash
a=("\$@")
while [ "\${a[0]-}" = -C ] || [ "\${a[0]-}" = -c ]; do a=("\${a[@]:2}"); done
case \${a[0]-} in
clone | fetch | pull | submodule) echo "git \$*" >> "$tmp/called.log"; exit 99 ;;
esac
exec "$real_git" "\$@"
EOF
chmod +x "$tmp"/nobin/*

# A scratch repository with the script, its patch and the stand-in deps-env.sh.
repo=$tmp/repo
g() { git -C "$repo" -c user.name=selftest -c user.email=selftest -c commit.gpgsign=false "$@"; }
mkdir -p "$repo/tools/cts"
git -C "$repo" init -q
cp "$BUILD_SH" "$HERE/cts-yakogl-target.patch" "$repo/tools/cts/"
cp "$FIX/deps-env.sh" "$repo/tools/cts/deps-env.sh"
cp "$FIX/mustpass.txt" "$repo/tools/cts/gles2-mustpass.txt"
chmod +x "$repo/tools/cts/deps-env.sh"
g add -A
g commit -q -m c1
c1=$(g rev-parse HEAD)
SCRIPT=$repo/tools/cts/mali-deqp-build.sh
cts_commit=$(sed -n 's/^CTS_COMMIT=//p' "$BUILD_SH")

# dry <name> -- <args...>: one run of the copy in a scratch cwd, every input
# path pointing at nothing; status to $tmp/<name>.rc, output to $tmp/<name>.txt.
dry() {
  local n=$1
  shift 2
  mkdir -p "$tmp/cwd-$n"
  (cd "$tmp/cwd-$n" && env PATH="$tmp/nobin:$PATH" FAKE_DEPS="$tmp/none/deps" FAKE_CALLED="$tmp/called.log" \
    CTS="$tmp/none/cts" BUILD="$tmp/none/build" JOBS=3 bash "$SCRIPT" "$@" > "$tmp/$n.txt" 2>&1; echo $? > "$tmp/$n.rc")
}
rc_is() { [ "$(cat "$tmp/$1.rc")" = "$2" ]; }
says() { grep -qF -- "$2" "$tmp/$1.txt"; }
untouched() { [ ! -e "$tmp/none" ] && [ ! -e "$tmp/out" ] && [ ! -e "$tmp/out.part" ] && [ ! -e "$tmp/called.log" ]; }

echo "mali-deqp-build-selftest: usage and --help"
dry noout -- --dry-run
check "no --out: usage, exit 2" rc_is noout 2
dry bad -- --bogus --out "$tmp/out"
check "an unknown argument: exit 2, named" \
  bash -c "[ \"\$(cat '$tmp/bad.rc')\" = 2 ] && grep -q \"unknown argument '--bogus'\" '$tmp/bad.txt'"
dry help -- --help
check "--help: exit 0, prints the header's usage and the floors" \
  bash -c "[ \"\$(cat '$tmp/help.rc')\" = 0 ] && grep -q 'mali-deqp-build.sh --out DIR' '$tmp/help.txt' && grep -q 'GLIBC 2.41' '$tmp/help.txt'"
check "--help stops at the header (no 'set -euo')" bash -c "! grep -q 'set -euo' '$tmp/help.txt'"
check "none of those wrote anything or called a fake" untouched

echo "mali-deqp-build-selftest: --dry-run on a clean tree"
dry clean -- --dry-run --out "$tmp/out"
check "exit 0, and it says it runs nothing" \
  bash -c "[ \"\$(cat '$tmp/clean.rc')\" = 0 ] && grep -q -- '--dry-run: the steps below are printed, none is run' '$tmp/clean.txt'"
check "it prints the commit, dirty=no and the CTS pin" says clean "commit=$c1 dirty=no cts=$cts_commit"
check "it prints the five steps in order" \
  bash -c "[ \"\$(grep -o '^== [0-9]\.' '$tmp/clean.txt' | paste -sd' ')\" = '== 1. == 2. == 3. == 4. == 5.' ]"
check "the absent default toolchain is made by deps-env.sh --build MALI_DEQP_TOOLCHAIN (printed, not run)" \
  bash -c "grep -q '^+ .*deps-env.sh --build --board nanopc_t6 MALI_DEQP_TOOLCHAIN\$' '$tmp/clean.txt'"
check "the CTS copy is a shared clone at the pin with the patch applied" \
  bash -c "grep -q '^+ git clone -q --shared --no-checkout $tmp/none/cts $tmp/none/build/cts\$' '$tmp/clean.txt' &&
    grep -q '^+ git -C $tmp/none/build/cts checkout -q --detach $cts_commit\$' '$tmp/clean.txt' &&
    grep -q '^+ git -C $tmp/none/build/cts apply .*/tools/cts/cts-yakogl-target.patch\$' '$tmp/clean.txt'"
check "cmake configures Ninja, Release, DEQP_TARGET=yakogl, deqp-gles2 only; the build uses -j3" \
  bash -c "grep -q '^+ cmake -S $tmp/none/build/cts -B $tmp/none/build/build -G Ninja .*-DCMAKE_BUILD_TYPE=Release .*-DDEQP_TARGET=yakogl .*-DSELECTED_BUILD_TARGETS=deqp-gles2\$' '$tmp/clean.txt' &&
    grep -q '^+ cmake --build $tmp/none/build/build --target deqp-gles2 -- -j3\$' '$tmp/clean.txt'"
check "step 4 names the floors" says clean "GLIBC 2.41, GLIBCXX 3.4.33, CXXABI 1.3.15"
check "the layout: deqp-gles2, data/gles2, mustpass, build.txt, moved from out.part" \
  bash -c "grep -q '^+ cp .*/modules/gles2/deqp-gles2 $tmp/out.part/deqp-gles2\$' '$tmp/clean.txt' &&
    grep -q '^+ cp -r .*/modules/gles2/gles2 $tmp/out.part/data/gles2\$' '$tmp/clean.txt' &&
    grep -q '^+ cp .*/tools/cts/gles2-mustpass.txt $tmp/out.part/mustpass\$' '$tmp/clean.txt' &&
    grep -q '^+ write $tmp/out.part/build.txt (commit=$c1 dirty=no cts=$cts_commit' '$tmp/clean.txt' &&
    grep -q '^+ mv $tmp/out.part $tmp/out\$' '$tmp/clean.txt'"
check "without --gles-lib, no lib/ step" bash -c "! grep -q 'part/lib' '$tmp/clean.txt'"
check "a dry run wrote nothing and called no fake" untouched

echo "mali-deqp-build-selftest: --gles-lib"
mkdir -p "$tmp/pair"
dry nolib -- --dry-run --out "$tmp/out" --gles-lib "$tmp/nolib"
check "--gles-lib with a missing directory: exit 1, the missing file named" \
  bash -c "[ \"\$(cat '$tmp/nolib.rc')\" = 1 ] && grep -q 'has no libGLESv2.so.2' '$tmp/nolib.txt'"
: > "$tmp/pair/libGLESv2.so.2"
dry halflib -- --dry-run --out "$tmp/out" --gles-lib "$tmp/pair"
check "--gles-lib without libEGL.so.1: exit 1" \
  bash -c "[ \"\$(cat '$tmp/halflib.rc')\" = 1 ] && grep -q 'has no libEGL.so.1' '$tmp/halflib.txt'"
: > "$tmp/pair/libEGL.so.1"
dry lib -- --dry-run --out "$tmp/out" --gles-lib "$tmp/pair"
check "--gles-lib with the pair: exit 0, copied to out.part/lib" \
  bash -c "[ \"\$(cat '$tmp/lib.rc')\" = 0 ] && grep -q '^+ cp $tmp/pair/libGLESv2.so.2 $tmp/pair/libEGL.so.1 $tmp/out.part/lib/\$' '$tmp/lib.txt'"

echo "mali-deqp-build-selftest: a tree with changes, and an outdir in the way"
echo changed >> "$repo/tools/cts/gles2-mustpass.txt"
dry dirty -- --dry-run --out "$tmp/out"
check "a tracked change: refused without --allow-dirty (exit 1)" \
  bash -c "[ \"\$(cat '$tmp/dirty.rc')\" = 1 ] && grep -q 'working tree has changes to tracked files' '$tmp/dirty.txt' && ! grep -q '^== ' '$tmp/dirty.txt'"
dry dirtyok -- --dry-run --allow-dirty --out "$tmp/out"
check "--allow-dirty: exit 0, dirty=yes, 'not a record'" \
  bash -c "[ \"\$(cat '$tmp/dirtyok.rc')\" = 0 ] && grep -q 'commit=$c1 dirty=yes' '$tmp/dirtyok.txt' && grep -q 'not a record' '$tmp/dirtyok.txt'"
g checkout -q -- tools/cts/gles2-mustpass.txt
: > "$repo/untracked.txt"
dry untr -- --dry-run --out "$tmp/out"
check "an untracked file alone is not a change" bash -c "[ \"\$(cat '$tmp/untr.rc')\" = 0 ] && grep -q 'dirty=no' '$tmp/untr.txt'"
mkdir -p "$tmp/full"
: > "$tmp/full/x"
dry full -- --dry-run --out "$tmp/full"
check "an outdir that is not empty: exit 1, never overwritten" \
  bash -c "[ \"\$(cat '$tmp/full.rc')\" = 1 ] && grep -q 'not empty' '$tmp/full.txt'"
check "no run wrote anything or called a fake" untouched

echo "mali-deqp-build-selftest: the symbol floor helpers (step 4)"
# max_of and ver_le are cut out of the script and run on version lists.
grep -E '^(max_of|ver_le)\(\)' "$BUILD_SH" > "$tmp/helpers.sh"
check "both helpers are in the script" bash -c "[ \"\$(wc -l < '$tmp/helpers.sh')\" = 2 ]"
check "max_of picks the largest GLIBCXX by version order, not text order" \
  bash -c ". '$tmp/helpers.sh'; [ \"\$(printf 'GLIBCXX_3.4.9\nGLIBCXX_3.4.33\nGLIBCXX_3.4\nGLIBC_2.41\n' | max_of GLIBCXX)\" = 3.4.33 ]"
check "max_of GLIBC does not take GLIBCXX lines" \
  bash -c ". '$tmp/helpers.sh'; [ \"\$(printf 'GLIBC_2.17\nGLIBC_2.38\nGLIBCXX_3.4.34\n' | max_of GLIBC)\" = 2.38 ]"
check "ver_le: at the floor and below pass, one above fails" \
  bash -c ". '$tmp/helpers.sh'; ver_le 2.41 2.41 && ver_le 2.38 2.41 && ver_le 3.4.9 3.4.33 && ! ver_le 2.42 2.41 && ! ver_le 3.4.34 3.4.33 && ! ver_le 1.3.16 1.3.15"
check "the floor constants are the board's" \
  bash -c "grep -q '^GLIBC_FLOOR=2.41\$' '$BUILD_SH' && grep -q '^GLIBCXX_FLOOR=3.4.33\$' '$BUILD_SH' && grep -q '^CXXABI_FLOOR=1.3.15\$' '$BUILD_SH'"
check "the fakes were never called by any check" bash -c "[ ! -e '$tmp/called.log' ]"

total=$((passed + failed))
if [ "$failed" -ne 0 ] || [ "$total" -ne "$EXPECT" ]; then
  echo "mali-deqp-build-selftest: $failed of $total checks failed (the self-test pins $EXPECT checks)" >&2
  for f in "$tmp"/*.txt; do echo "--- $f"; tail -15 "$f"; done >&2
  exit 1
fi
echo "mali-deqp-build-selftest: $passed checks passed"
