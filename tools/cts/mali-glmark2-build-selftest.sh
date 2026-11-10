#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The self-test of tools/cts/mali-glmark2-build.sh, the NanoPC-T6 glmark2-es2
# build, without network, cross toolchain or board: --dry-run in a scratch
# repository (main has c1, a side branch c2) with fakes on PATH that fail if
# called (cargo, meson, ninja, curl, python3's fetch is never reached), and
# --check on a fixture tree whose readelf is a fake printing the fixtures of
# tools/cts/testdata/mali-glmark2-build/ (README.md there): a good tree, a
# too-new GLIBC need, a system libEGL NEEDED, and mutations of the good one
# (among them lib/libEGL.so and lib/libGLESv2.so missing, copied or linked
# elsewhere: tree.links makes them in the copy of the good tree).
#
#   tools/cts/mali-glmark2-build-selftest.sh   prints "mali-glmark2-build-selftest: N checks passed"
#
# Host only, a few seconds.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
FIX=$HERE/testdata/mali-glmark2-build
BUILD_SH=$HERE/mali-glmark2-build.sh
EXPECT=41
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mali-glmark2-build-selftest-XXXXXX")
cleanup() {
  if [ "${MALI_GLMARK2_SELFTEST_KEEP-}" = 1 ]; then echo "mali-glmark2-build-selftest: kept $tmp"; else rm -rf "$tmp"; fi
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

# Fakes that log and fail: a dry run and --check must call none of them.
mkdir -p "$tmp/nobin" "$tmp/fakebin"
for t in cargo meson ninja curl rustc clang; do
  printf '#!/bin/bash\necho "%s $*" >> "%s/called.log"\nexit 99\n' "$t" "$tmp" > "$tmp/nobin/$t"
done
cp "$FIX/bin/readelf" "$tmp/fakebin/readelf"
chmod +x "$tmp"/nobin/* "$tmp"/fakebin/*

# A repository whose main has commit c1 and whose side branch has c2.
g() { git -C "$tmp/repo" -c user.name=selftest -c user.email=selftest -c commit.gpgsign=false "$@"; }
mkdir -p "$tmp/repo"
git -C "$tmp/repo" init -q
echo one > "$tmp/repo/file.txt"
g add file.txt
g commit -q -m c1
g branch -f main HEAD
c1=$(g rev-parse HEAD)
g checkout -q -b side
g commit -q --allow-empty -m c2
c2=$(g rev-parse HEAD)
g checkout -q "$c1"

# dry <name> [VAR=value...] -- <args...>: one dry run of the build in a scratch
# cwd, every input path pointing at nothing; status to $tmp/<name>.rc, output
# to $tmp/<name>.txt.
dry() {
  local n=$1
  shift
  local vars=()
  while [ "$1" != -- ]; do vars+=("$1"); shift; done
  shift
  mkdir -p "$tmp/cwd-$n"
  (cd "$tmp/cwd-$n" && env PATH="$tmp/nobin:$PATH" MALI_GLMARK2_REPO="$tmp/repo" MALI_GLMARK2_MAIN_REF=main \
    TOOLCHAIN="$tmp/none/toolchain" GLMARK2_SRC="$tmp/none/glmark2" IMG_SYSROOT="$tmp/none/img" \
    BUILD="$tmp/none/build" "${vars[@]}" bash "$BUILD_SH" "$@" > "$tmp/$n.txt" 2>&1; echo $? > "$tmp/$n.rc")
}
rc_is() { [ "$(cat "$tmp/$1.rc")" = "$2" ]; }
says() { grep -qF -- "$2" "$tmp/$1.txt"; }
nothing_written() { [ ! -e "$tmp/none" ] && [ ! -e "$tmp/out" ] && [ ! -e "$tmp/out.part" ] && [ ! -e "$tmp/called.log" ] &&
  [ -z "$(ls -A "$tmp/cwd-$1")" ]; }

echo "mali-glmark2-build-selftest: --dry-run of main's c1"
dry main -- --dry-run "$tmp/out"
check "a dry run on main exits 0 and says it runs nothing" \
  bash -c "[ \"\$(cat '$tmp/main.rc')\" = 0 ] && grep -q -- '--dry-run: the steps below are printed, none is run' '$tmp/main.txt'"
check "it prints the commit, dev=no, the glmark2 pin and flavour pbuffer" \
  says main "commit=$c1 dev=no glmark2=22c527cb0556f3a1ac4445aaa52cc532760928d5 flavour=pbuffer"
check "it prints the six steps in order" \
  bash -c "[ \"\$(grep -o '^== [0-9]\.' '$tmp/main.txt' | paste -sd' ')\" = '== 1. == 2. == 3. == 4. == 5. == 6.' ]"
check "the toolchain is MALI_DEQP_TOOLCHAIN, not built when TOOLCHAIN is given" \
  bash -c "grep -q 'MALI_DEQP_TOOLCHAIN' '$tmp/main.txt' && ! grep -q '^+ .*deps-env.sh --build' '$tmp/main.txt'"
check "the three image -dev packages are fetched from the snapshot, sha256 named" \
  bash -c "[ \$(grep -c '^+ fetch https://snapshot.debian.org/archive/debian/20261101T000000Z/pool/main/.*_arm64.deb (sha256 [0-9a-f]\{64\})' '$tmp/main.txt') = 3 ]"
check "the pair is built by tools/mali/panthor-gles/build.sh and checked by its check.sh" \
  bash -c "grep -q '^+ .*/tools/mali/panthor-gles/build.sh $tmp/none/build/gles\$' '$tmp/main.txt' &&
    grep -q '^+ .*/tools/mali/panthor-gles/check.sh $tmp/none/build/gles\$' '$tmp/main.txt'"
check "glmark2 is a shared clone of the pin with mali-glmark2-pbuffer.patch applied" \
  bash -c "grep -q '^+ git clone -q --shared --no-checkout $tmp/none/glmark2 $tmp/none/build/glmark2\$' '$tmp/main.txt' &&
    grep -q '^+ git -C $tmp/none/build/glmark2 checkout -q --detach 22c527cb0556f3a1ac4445aaa52cc532760928d5\$' '$tmp/main.txt' &&
    grep -q '^+ git -C $tmp/none/build/glmark2 apply .*/tools/cts/mali-glmark2-pbuffer.patch\$' '$tmp/main.txt'"
check "a missing GLMARK2_SRC is said in a dry run, with how to make it" \
  says main "tools/cts/deps-env.sh --build GLMARK2_SRC"
check "meson builds only the pbuffer-glesv2 flavour, static, from the cross file" \
  bash -c "grep -q '^+ meson setup .*--cross-file $tmp/none/build/cross-trixie-aarch64.ini --buildtype=release -Dprefer_static=true -Dflavors=pbuffer-glesv2\$' '$tmp/main.txt' &&
    grep -q '^+ ninja -C $tmp/none/build/meson -j[0-9]* src/glmark2-es2-pbuffer\$' '$tmp/main.txt'"
check "the layout: bin/glmark2-es2-mali, lib/ the pair and its unversioned links, share/glmark2 the data, BUILD-INFO" \
  bash -c "grep -q '^+ cp .*/meson/src/glmark2-es2-pbuffer $tmp/out.part/bin/glmark2-es2-mali\$' '$tmp/main.txt' &&
    grep -q '^+ cp $tmp/none/build/gles/libGLESv2.so.2 $tmp/none/build/gles/libEGL.so.1 $tmp/out.part/lib/\$' '$tmp/main.txt' &&
    grep -q '^+ ln -s libGLESv2.so.2 $tmp/out.part/lib/libGLESv2.so\$' '$tmp/main.txt' &&
    grep -q '^+ ln -s libEGL.so.1 $tmp/out.part/lib/libEGL.so\$' '$tmp/main.txt' &&
    grep -q '^+ cp -r .*/data/shaders .*/data/models .*/data/textures $tmp/out.part/share/glmark2/\$' '$tmp/main.txt' &&
    grep -q '^+ write $tmp/out.part/BUILD-INFO (commit=$c1 dev=no glmark2=22c527cb0556f3a1ac4445aaa52cc532760928d5 flavour=pbuffer glibc_max= glibcxx_max= cxxabi_max=)' '$tmp/main.txt' &&
    grep -q '^+ mv $tmp/out.part $tmp/out\$' '$tmp/main.txt'"
check "the RUNPATH and the floors are named" \
  bash -c "grep -q 'RUNPATH \$ORIGIN/../lib' '$tmp/main.txt' && grep -q 'GLIBC_2.41 GLIBCXX_3.4.33 CXXABI_1.3.15' '$tmp/main.txt'"
check "a dry run writes nothing and calls no build tool" nothing_written main

echo "mali-glmark2-build-selftest: --dry-run --gles-lib"
mkdir -p "$tmp/pair"
: > "$tmp/pair/libGLESv2.so.2"
: > "$tmp/pair/libEGL.so.1"
dry gl -- --dry-run --gles-lib "$tmp/pair" "$tmp/out"
check "--gles-lib: the pair is copied from it, build.sh is not run" \
  bash -c "[ \"\$(cat '$tmp/gl.rc')\" = 0 ] && grep -q 'the pair: libGLESv2.so.2 and libEGL.so.1 from --gles-lib $tmp/pair' '$tmp/gl.txt' &&
    ! grep -q 'panthor-gles/build.sh' '$tmp/gl.txt' && grep -q '^+ cp $tmp/pair/libGLESv2.so.2 $tmp/pair/libEGL.so.1 $tmp/out.part/lib/\$' '$tmp/gl.txt'"
rm "$tmp/pair/libEGL.so.1"
dry gl2 -- --dry-run --gles-lib "$tmp/pair" "$tmp/out"
check "--gles-lib without libEGL.so.1 is refused (exit 1)" \
  bash -c "[ \"\$(cat '$tmp/gl2.rc')\" = 1 ] && grep -q 'has no libEGL.so.1' '$tmp/gl2.txt'"

echo "mali-glmark2-build-selftest: refusals"
g checkout -q "$c2"
dry side -- --dry-run "$tmp/out"
check "HEAD not on the main ref: refused with exit 3, nothing written" \
  bash -c "[ \"\$(cat '$tmp/side.rc')\" = 3 ] && grep -q 'refused: HEAD $c2 is not on main' '$tmp/side.txt' && [ ! -e '$tmp/none' ]"
dry sidedev -- --dev --dry-run "$tmp/out"
check "--dev builds it anyway: exit 0, dev=yes" \
  bash -c "[ \"\$(cat '$tmp/sidedev.rc')\" = 0 ] && grep -q 'commit=$c2 dev=yes' '$tmp/sidedev.txt' && grep -q 'not a record' '$tmp/sidedev.txt'"
g checkout -q "$c1"
echo two > "$tmp/repo/file.txt"
dry dirty -- --dry-run "$tmp/out"
check "a tracked change: refused with exit 3" \
  bash -c "[ \"\$(cat '$tmp/dirty.rc')\" = 3 ] && grep -q 'changes to tracked files' '$tmp/dirty.txt'"
dry dirtydev -- --dev --dry-run "$tmp/out"
check "--dev with a tracked change: exit 0, commit ends in -dirty" \
  bash -c "[ \"\$(cat '$tmp/dirtydev.rc')\" = 0 ] && grep -q 'commit=$c1-dirty dev=yes' '$tmp/dirtydev.txt'"
g checkout -q -- file.txt
dry noref MALI_GLMARK2_MAIN_REF=no-such-ref -- --dry-run "$tmp/out"
check "a main ref that does not resolve: refused with exit 3" \
  bash -c "[ \"\$(cat '$tmp/noref.rc')\" = 3 ] && grep -q 'main ref no-such-ref does not resolve' '$tmp/noref.txt'"
mkdir -p "$tmp/full"
: > "$tmp/full/x"
dry full -- --dry-run "$tmp/full"
check "an outdir that is not empty: exit 1" \
  bash -c "[ \"\$(cat '$tmp/full.rc')\" = 1 ] && grep -q 'is never overwritten' '$tmp/full.txt'"
dry none --
dry bad -- --bogus "$tmp/out"
check "no outdir, or an unknown option: usage, exit 2" \
  bash -c "[ \"\$(cat '$tmp/none.rc')\" = 2 ] && [ \"\$(cat '$tmp/bad.rc')\" = 2 ] && grep -q 'unknown argument' '$tmp/bad.txt'"
check "no refusal wrote anything or called a build tool" \
  bash -c "[ ! -e '$tmp/none' ] && [ ! -e '$tmp/out' ] && [ ! -e '$tmp/called.log' ]"

# --check: the fixture tree, with the fake readelf reading $tmp/re/<case>/.
# tree <case> <readelf fixture dir...>: a copy of the fixture tree with the
# links of tree.links, and a readelf directory of the good fixtures overlaid
# with the given ones.
tree() {
  local c=$1 d l to
  shift
  rm -rf "$tmp/t/$c" "$tmp/re/$c"
  mkdir -p "$tmp/t" "$tmp/re/$c"
  cp -r "$FIX/tree" "$tmp/t/$c"
  while read -r l to; do
    case $l in '' | '#'*) continue ;; esac
    ln -s "$to" "$tmp/t/$c/$l"
  done < "$FIX/tree.links"
  cp "$FIX/readelf/good"/*.txt "$tmp/re/$c/"
  for d in "$@"; do cp "$FIX/readelf/$d"/*.txt "$tmp/re/$c/"; done
}
chk() { # <case>: --check on it; status to $tmp/chk-<case>.rc, output to .txt
  local c=$1
  env PATH="$tmp/nobin:$PATH" READELF="$tmp/fakebin/readelf" FAKE_READELF_DIR="$tmp/re/$c" \
    bash "$BUILD_SH" --check "$tmp/t/$c" > "$tmp/chk-$c.txt" 2>&1
  echo $? > "$tmp/chk-$c.rc"
}
check_fails() { # <case> <text>: --check exits 1 and says <text>
  [ "$(cat "$tmp/chk-$1.rc")" = 1 ] && grep -qF -- "$2" "$tmp/chk-$1.txt" && grep -q 'does not fit the board' "$tmp/chk-$1.txt"
}
edit() { sed -i "$2" "$tmp/re/$1/$3.txt"; } # <case> <sed> <file>: change one fixture

echo "mali-glmark2-build-selftest: --check on the fixture trees"
tree good
chk good
check "a good tree: exit 0" rc_is chk-good 0
check "it prints the floors readelf found and the commit" \
  says chk-good "GLIBC_2.38 GLIBCXX_3.4.32 CXXABI_1.3.9 (floors GLIBC_2.41 GLIBCXX_3.4.33 CXXABI_1.3.15); bin/glmark2-es2-mali RUNPATH \$ORIGIN/../lib; lib/ is the pair; commit 78c917aabbdf6b85d96d99be29e5ce39a41d83c1"
tree glibc glibc-too-new
chk glibc
check "a GLIBC_2.42 need: exit 1, named against the floor" check_fails glibc "GLIBC_2.42 needed > the board's 2.41"
tree egl system-libegl
chk egl
check "a system libEGL.so.1 NEEDED by the program: exit 1" check_fails egl "NEEDs libEGL.so.1: a system GL/EGL library"
tree cxx
edit cxx 's/GLIBCXX_3.4.32/GLIBCXX_3.4.34/' glmark2-es2-mali
chk cxx
check "a GLIBCXX_3.4.34 need: exit 1" check_fails cxx "GLIBCXX_3.4.34 needed > the board's 3.4.33"
tree png
edit png 's/Shared library: \[libm.so.6\]/&\n 0x0000000000000001 (NEEDED)             Shared library: [libpng16.so.16]/' glmark2-es2-mali
chk png
check "a NEEDED libpng16.so.16: exit 1 (it must be static)" check_fails png "NEEDs libpng16.so.16, which a minimal Armbian trixie may lack"
tree rp
edit rp "s/\[\\\$ORIGIN\/..\/lib\]/[\/usr\/lib\/aarch64-linux-gnu]/" glmark2-es2-mali
chk rp
check "a RUNPATH other than \$ORIGIN/../lib: exit 1" check_fails rp "has RUNPATH '/usr/lib/aarch64-linux-gnu', not \$ORIGIN/../lib"
tree arch
edit arch 's/AArch64/Advanced Micro Devices X86-64/' glmark2-es2-mali
chk arch
check "an x86-64 program: exit 1" check_fails arch "bin/glmark2-es2-mali is not AArch64"
tree mesa
printf 'Mesa 25.0.7 libGLESv2 stand-in\n' > "$tmp/t/mesa/lib/libGLESv2.so.2"
chk mesa
check "a libGLESv2.so.2 without the pair's contract text (the board's Mesa): exit 1" \
  check_fails mesa "has no 'yakogl-mali: loaded' text: not the panthor-gles pair"
tree stray
cp "$tmp/t/stray/lib/libEGL.so.1" "$tmp/t/stray/bin/libEGL.so.1"
chk stray
check "another libEGL.so.1 in the tree: exit 1" check_fails stray "bin/libEGL.so.1: a libEGL/libGLESv2 other than the pair's"
tree nolinks
rm "$tmp/t/nolinks/lib/libEGL.so" "$tmp/t/nolinks/lib/libGLESv2.so"
chk nolinks
check "no lib/libEGL.so and lib/libGLESv2.so (glmark2 dlopens them first): exit 1, both named" \
  bash -c "grep -qF 'no lib/libEGL.so (glmark2 dlopens it before libEGL.so.1' '$tmp/chk-nolinks.txt' &&
    grep -qF 'no lib/libGLESv2.so (glmark2 dlopens it before libGLESv2.so.2' '$tmp/chk-nolinks.txt' &&
    [ \"\$(cat '$tmp/chk-nolinks.rc')\" = 1 ]"
tree nogles
rm "$tmp/t/nogles/lib/libGLESv2.so"
chk nogles
check "lib/libEGL.so alone, no lib/libGLESv2.so: exit 1" check_fails nogles "no lib/libGLESv2.so (glmark2 dlopens it"
tree copy
rm "$tmp/t/copy/lib/libGLESv2.so"
cp "$tmp/t/copy/lib/libGLESv2.so.2" "$tmp/t/copy/lib/libGLESv2.so"
chk copy
check "a lib/libGLESv2.so that is a copy, not a link: exit 1" \
  check_fails copy "lib/libGLESv2.so is not a link to libGLESv2.so.2 (a copy loads as a second library)"
tree elsewhere
ln -sfn /usr/lib/aarch64-linux-gnu/libEGL.so.1 "$tmp/t/elsewhere/lib/libEGL.so"
chk elsewhere
check "a lib/libEGL.so linking to the board's libEGL.so.1: exit 1" \
  check_fails elsewhere "lib/libEGL.so links to '/usr/lib/aarch64-linux-gnu/libEGL.so.1', not libEGL.so.1"
tree crossed
ln -sfn libEGL.so.1 "$tmp/t/crossed/lib/libGLESv2.so"
chk crossed
check "a lib/libGLESv2.so linking to libEGL.so.1: exit 1" \
  check_fails crossed "lib/libGLESv2.so links to 'libEGL.so.1', not libGLESv2.so.2"
tree fwd
edit fwd '/Library runpath/d' libEGL.so.1
chk fwd
check "a libEGL.so.1 without RUNPATH \$ORIGIN (would find the board's libGLESv2): exit 1" \
  check_fails fwd "lib/libEGL.so.1 has RUNPATH '', not \$ORIGIN"
tree info
sed -i 's/^glibc_max=.*/glibc_max=2.41/; s/^flavour=.*/flavour=drm/' "$tmp/t/info/BUILD-INFO"
chk info
check "a BUILD-INFO whose maxima or flavour do not match: exit 1" \
  bash -c "grep -qF 'BUILD-INFO glibc_max=2.41, but readelf finds 2.38' '$tmp/chk-info.txt' && grep -qF 'BUILD-INFO flavour=drm, not pbuffer' '$tmp/chk-info.txt' && [ \"\$(cat '$tmp/chk-info.rc')\" = 1 ]"
tree data
rm -rf "$tmp/t/data/share/glmark2/textures"
chk data
check "no share/glmark2/textures: exit 1" check_fails data "share/glmark2/textures is missing or empty"
check "--check called no build tool" bash -c "[ ! -e '$tmp/called.log' ]"

total=$((passed + failed))
if [ "$failed" -ne 0 ] || [ "$total" -ne "$EXPECT" ]; then
  echo "mali-glmark2-build-selftest: $failed of $total checks failed (the self-test pins $EXPECT checks)" >&2
  for f in "$tmp"/*.txt; do echo "--- $f"; tail -15 "$f"; done >&2
  exit 1
fi
echo "mali-glmark2-build-selftest: $passed checks passed"
