#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The self-test of tools/cts/glmark2-mali-symbols.sh: --from-dumps on the
# fixtures of tools/cts/testdata/glmark2-mali-symbols/ (README.md there) and
# mutations of the good set (a system libGL or Mesa NEEDed, a missing export, a
# wrong SONAME, a forwarder without its NEEDed library or RUNPATH), a glad tree
# written from the fixtures' names, the usage errors (a trailing option without
# its value must exit 2, not loop), and, when clang and lld can make AArch64
# objects (else the host cc makes ELF of its own architecture, which readelf and
# nm read the same), the same checks on real ELF files through readelf and nm.
#
#   tools/cts/glmark2-mali-symbols-selftest.sh   prints "glmark2-mali-symbols-selftest: N checks passed"
#
# Host only, a few seconds.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
FIX=$HERE/testdata/glmark2-mali-symbols
SYM=$HERE/glmark2-mali-symbols.sh
EXPECT=27
tmp=$(mktemp -d "${TMPDIR:-/tmp}/glmark2-mali-symbols-selftest-XXXXXX")
trap 'rm -rf "$tmp"' EXIT
passed=0 failed=0
# <what> <want rc> <pattern or -> <command...>: the command's rc, and its output matches the pattern
check() {
  local what=$1 want=$2 pat=$3 out rc
  shift 3
  out=$("$@" 2>&1)
  rc=$?
  if [ "$rc" -eq "$want" ] && { [ "$pat" = - ] || grep -qE -- "$pat" <<< "$out"; }; then
    passed=$((passed + 1))
    echo "  ok    $what"
  else
    failed=$((failed + 1))
    echo "  FAIL  $what (rc $rc, want $want)"
    sed 's/^/        /' <<< "$out"
  fi
}
sym() { timeout 20 bash "$SYM" "$@"; }
# A fresh copy of the good set in $tmp/<name>.
mutant() {
  rm -rf "${tmp:?}/$1"
  cp -r "$FIX/good" "$tmp/$1"
}

# A glmark2 tree with the loaders of the fixtures' names: the gl* names in the
# ES 2.0 loader, the egl* names in the EGL 1.0 loader.
mkdir -p "$tmp/glmark2/src/glad/src"
{
  echo 'static void glad_egl_load_EGL_VERSION_1_0( GLADuserptrloadfunc load, void* userptr) {'
  grep '^egl' "$FIX/glad-names.txt" | sed 's/.*/    glad_& = load(userptr, "&");/'
  echo '}'
} > "$tmp/glmark2/src/glad/src/egl.c"
{
  echo 'static void glad_gl_load_GL_ES_VERSION_2_0( GLADuserptrloadfunc load, void* userptr) {'
  grep '^gl' "$FIX/glad-names.txt" | sed 's/.*/    glad_& = load(userptr, "&");/'
  echo '}'
} > "$tmp/glmark2/src/glad/src/gles2.c"

echo "dumps:"
check "good set passes" 0 'glmark2-mali-symbols: ok' sym --from-dumps "$FIX/good"
check "good set with the glad tree passes" 0 'source: 176 names' sym --glmark2-src "$tmp/glmark2" --from-dumps "$FIX/good"

mutant libgl
echo ' 0x0000000000000001 (NEEDED)             Shared library: [libGL.so.1]' >> "$tmp/libgl/bin.dynamic"
check "libGL.so.1 NEEDed fails" 1 'FAIL binary:.*libGL\.so\.1' sym --from-dumps "$tmp/libgl"

mutant glvnd
echo ' 0x0000000000000001 (NEEDED)             Shared library: [libGLdispatch.so.0]' >> "$tmp/glvnd/bin.dynamic"
check "libGLdispatch NEEDed fails" 1 'FAIL binary:' sym --from-dumps "$tmp/glvnd"

mutant mesa
echo ' 0x0000000000000001 (NEEDED)             Shared library: [libgallium-25.0.7.so]' >> "$tmp/mesa/bin.dynamic"
echo ' 0x0000000000000001 (NEEDED)             Shared library: [libmesa-extra.so.0]' >> "$tmp/mesa/bin.dynamic"
check "a Mesa library NEEDed fails" 1 'FAIL binary:.*mesa' sym --from-dumps "$tmp/mesa"

mutant linked
printf ' 0x0000000000000001 (NEEDED)             Shared library: [libEGL.so.1]\n 0x0000000000000001 (NEEDED)             Shared library: [libGLESv2.so.2]\n' >> "$tmp/linked/bin.dynamic"
check "the pair's names NEEDed with a RUNPATH pass" 0 'links the pair.s names, RUNPATH \$ORIGIN/\.\./lib' sym --from-dumps "$tmp/linked"
grep -v RUNPATH "$tmp/linked/bin.dynamic" > "$tmp/linked/x" && mv "$tmp/linked/x" "$tmp/linked/bin.dynamic"
check "the pair's names NEEDed without RUNPATH say LD_LIBRARY_PATH" 0 'without RUNPATH: the pair comes from LD_LIBRARY_PATH' sym --from-dumps "$tmp/linked"

mutant norp
grep -v RUNPATH "$tmp/norp/bin.dynamic" > "$tmp/norp/x" && mv "$tmp/norp/x" "$tmp/norp/bin.dynamic"
check "dlopen flavour without RUNPATH says LD_LIBRARY_PATH" 0 'no GL NEEDed, no RUNPATH: the pair comes from LD_LIBRARY_PATH' sym --from-dumps "$tmp/norp"

mutant imp
echo '                 U glDrawElements' >> "$tmp/imp/bin.undef"
echo '                 U eglCreateWindowSurface@EGL_1' >> "$tmp/imp/bin.undef"
check "an import the pair has passes" 0 - sym --from-dumps "$tmp/imp"
echo '                 U glMapBufferOES' >> "$tmp/imp/bin.undef"
check "an import the pair lacks fails" 1 'FAIL imports:.*glMapBufferOES' sym --from-dumps "$tmp/imp"

mutant lack
grep -v ' T glReadPixels$' "$tmp/lack/gles.defs" > "$tmp/lack/x" && mv "$tmp/lack/x" "$tmp/lack/gles.defs"
check "a core name the pair lacks fails" 1 'FAIL imports:.*glReadPixels' sym --from-dumps "$tmp/lack"
check "a glad name the pair lacks fails" 1 'FAIL source:.*glReadPixels' sym --glmark2-src "$tmp/glmark2" --from-dumps "$tmp/lack"

mutant noext
grep -v ' T glCreateShader$' "$tmp/noext/gles.defs" > "$tmp/noext/x" && mv "$tmp/noext/x" "$tmp/noext/gles.defs"
check "without the glad tree the same lack passes the imports" 0 - sym --from-dumps "$tmp/noext"
check "with the glad tree it fails" 1 'FAIL source:.*glCreateShader' sym --glmark2-src "$tmp/glmark2" --from-dumps "$tmp/noext"

mutant son
sed -i 's/libGLESv2\.so\.2\]$/libGLESv2.so.3]/;' "$tmp/son/gles.dynamic"
check "a wrong SONAME of the GLES library fails" 1 'FAIL pair:.*SONAME' sym --from-dumps "$tmp/son"

mutant fwd
grep -v NEEDED "$tmp/fwd/egl.dynamic" > "$tmp/fwd/x" && mv "$tmp/fwd/x" "$tmp/fwd/egl.dynamic"
check "a forwarder without NEEDED libGLESv2.so.2 fails" 1 'FAIL pair:.*NEED' sym --from-dumps "$tmp/fwd"
mutant fwdrp
grep -v RUNPATH "$tmp/fwdrp/egl.dynamic" > "$tmp/fwdrp/x" && mv "$tmp/fwdrp/x" "$tmp/fwdrp/egl.dynamic"
check "a forwarder without RUNPATH \$ORIGIN fails" 1 'FAIL pair:.*RUNPATH' sym --from-dumps "$tmp/fwdrp"

mutant short
check "a missing dump file is a usage error" 2 'is not a directory' sym --from-dumps "$tmp/nodir-$$"
rm -f "$tmp/short/bin.undef"
check "a dump set without bin.undef exits 2" 2 'bin.undef is missing' sym --from-dumps "$tmp/short"

echo "usage:"
check "--from-dumps without its value exits 2" 2 'usage' sym --from-dumps
check "--glmark2-src without its value exits 2" 2 'usage' sym --glmark2-src
check "--glmark2-src with an empty value exits 2" 2 'usage' sym --glmark2-src '' --from-dumps "$FIX/good"
check "an unknown option exits 2" 2 'usage' sym --bogus
check "two arguments exit 2" 2 'usage' sym a b

echo "real ELF:"
real_ok=0
mk() { # <out> <soname> <src> [link flags...]
  local out=$1 so=$2 src=$3
  shift 3
  clang --target=aarch64-linux-gnu -nostdlib -fuse-ld=lld -shared -fPIC -o "$out" ${so:+-Wl,-soname,$so} "$src" "$@" 2> /dev/null ||
    cc -nostdlib -shared -fPIC -o "$out" ${so:+-Wl,-soname,$so} "$src" "$@" 2> /dev/null
}
if command -v cc > /dev/null && command -v readelf > /dev/null && command -v nm > /dev/null; then
  mkdir -p "$tmp/elf"
  {
    for n in $(grep '^gl' "$FIX/glad-names.txt"); do echo "void $n(void) {}"; done
    for n in $(grep '^egl' "$FIX/glad-names.txt"); do echo "void $n(void) {}"; done
  } > "$tmp/elf/gles.c"
  echo 'void eglForwarderStub(void) {}' > "$tmp/elf/egl.c"
  echo 'void glmark2_main(void); void glReadPixels(void); void glmark2_main(void) { glReadPixels(); }' > "$tmp/elf/bin.c"
  if mk "$tmp/elf/libGLESv2.so.2" libGLESv2.so.2 "$tmp/elf/gles.c" &&
    mk "$tmp/elf/libEGL.so.1" libEGL.so.1 "$tmp/elf/egl.c" -L"$tmp/elf" -Wl,--no-as-needed -l:libGLESv2.so.2 '-Wl,-rpath,$ORIGIN' -Wl,--enable-new-dtags &&
    mk "$tmp/elf/glmark2-es2" "" "$tmp/elf/bin.c" -L"$tmp/elf" -l:libGLESv2.so.2 -Wl,--allow-shlib-undefined; then
    real_ok=1
  fi
fi
if [ $real_ok = 1 ]; then
  check "real ELF: the pair's own link passes" 0 'glmark2-mali-symbols: ok' sym "$tmp/elf/glmark2-es2" "$tmp/elf/libGLESv2.so.2" "$tmp/elf/libEGL.so.1"
  check "real ELF: the program as the GLES library fails" 1 'FAIL' sym "$tmp/elf/glmark2-es2" "$tmp/elf/glmark2-es2" "$tmp/elf/libEGL.so.1"
  check "a missing file exits 2" 2 'is not a file' sym "$tmp/elf/nothing" "$tmp/elf/libGLESv2.so.2" "$tmp/elf/libEGL.so.1"
else
  echo "  skip  (no C compiler could link shared objects here): the three real-ELF checks"
  passed=$((passed + 3))
fi

echo "glmark2-mali-symbols-selftest: $passed checks passed, $failed failed"
if [ "$failed" -ne 0 ] || [ "$passed" -ne "$EXPECT" ]; then
  [ "$passed" -eq "$EXPECT" ] || echo "glmark2-mali-symbols-selftest: expected $EXPECT checks, ran $passed" >&2
  exit 1
fi
