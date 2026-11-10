#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Host check (readelf and nm, no board): a glmark2-es2 built for the NanoPC-T6
# resolves GL and EGL only to YakoGL's Mali pair (tools/mali/panthor-gles), and
# the pair exports every entry point glmark2 imports.
#
#   tools/cts/glmark2-mali-symbols.sh [--glmark2-src DIR] [--libdir DIR] <glmark2-es2> <libGLESv2.so.2> <libEGL.so.1>
#   tools/cts/glmark2-mali-symbols.sh [--glmark2-src DIR] [--libdir DIR] --from-dumps DIR
#
# The three files are AArch64 ELF; readelf -d and nm -D read them on any host.
# --from-dumps DIR reads text instead (no tool is run): DIR/{bin,gles,egl}.dynamic
# (readelf -d), DIR/bin.undef (nm -D --undefined-only), DIR/{gles,egl}.defs
# (nm -D --defined-only).
#
# Checks (each failure is one line "FAIL <cause>: ..."):
#   pair      libGLESv2.so.2 has SONAME libGLESv2.so.2; libEGL.so.1 has SONAME
#             libEGL.so.1, NEEDs libGLESv2.so.2 and has RUNPATH or RPATH $ORIGIN
#   binary    the program NEEDs no GL or EGL library but the pair's two SONAMEs
#             (no libGL, libGLX, libOpenGL, libGLdispatch, libGLESv1_CM, no
#             Mesa library): the "sonama trap", the board's glvnd + Mesa 25.0.7
#             has the same two SONAMEs. A program that NEEDs the pair's names
#             without RUNPATH, and one that NEEDs none (glmark2 dlopen()s), get
#             the pair only from the run-time search path (LD_LIBRARY_PATH or
#             its RUNPATH): the "ok" line says which
#   imports   every gl* and egl* name the program imports (nm -D
#             --undefined-only) is defined by libGLESv2.so.2 or libEGL.so.1,
#             and so is the core glmark2 needs whatever it imports
#             (eglGetProcAddress, eglBindAPI, eglCreatePbufferSurface, ...)
#   source    with --glmark2-src DIR (the glmark2 tree GLMARK2_SRC names): every
#             name glad loads for EGL 1.0 to 1.4 and GL_ES_VERSION_2_0 is defined
#             by the pair (glmark2 dlopen()s and calls what glad loaded: a name
#             the pair lacks is a null call)
#
#   dlopen    with --glmark2-src DIR and the build tree's lib directory (--libdir
#             DIR, else <glmark2-es2's directory>/../lib, what RUNPATH
#             $ORIGIN/../lib names): glmark2's src/gl-state-egl.cpp dlopen()s
#             libEGL.so... and libGLESv2.so... in a list, the first name that
#             loads wins. One line per library: it fails unless the first name
#             of the list exists in that directory (an unversioned libEGL.so
#             missing there lets the system's glvnd libEGL.so load first). No
#             lib directory known (--from-dumps without --libdir): skipped
#
# Exit: 0 every check passed, 1 a check failed, 2 usage, an unreadable file or
# a missing tool.
set -uo pipefail

usage() {
  echo "usage: ${0##*/} [--glmark2-src DIR] [--libdir DIR] <glmark2-es2> <libGLESv2.so.2> <libEGL.so.1>" >&2
  echo "       ${0##*/} [--glmark2-src DIR] [--libdir DIR] --from-dumps DIR" >&2
  exit 2
}

SRC= LIBDIR= DUMPS= MODE=files
pos=()
while [ $# -gt 0 ]; do
  case $1 in
    --glmark2-src)
      [ $# -ge 2 ] && [ -n "$2" ] || usage
      SRC=$2
      shift 2
      ;;
    --libdir)
      [ $# -ge 2 ] && [ -n "$2" ] || usage
      LIBDIR=$2
      shift 2
      ;;
    --from-dumps)
      [ $# -ge 2 ] && [ -n "$2" ] || usage
      MODE=dumps DUMPS=$2
      shift 2
      ;;
    -*) usage ;;
    *)
      pos+=("$1")
      shift
      ;;
  esac
done
if [ "$MODE" = dumps ]; then
  [ ${#pos[@]} -eq 0 ] || usage
  [ -d "$DUMPS" ] || { echo "${0##*/}: $DUMPS is not a directory" >&2; exit 2; }
else
  [ ${#pos[@]} -eq 3 ] || usage
  BIN=${pos[0]} GLES=${pos[1]} EGL=${pos[2]}
  for f in "$BIN" "$GLES" "$EGL"; do
    [ -f "$f" ] || { echo "${0##*/}: $f is not a file" >&2; exit 2; }
  done
  for t in readelf nm; do
    command -v "$t" > /dev/null || { echo "${0##*/}: $t is missing" >&2; exit 2; }
  done
fi
if [ -n "$SRC" ]; then
  [ -f "$SRC/src/glad/src/egl.c" ] && [ -f "$SRC/src/glad/src/gles2.c" ] ||
    { echo "${0##*/}: $SRC is not a glmark2 tree (src/glad/src/egl.c, gles2.c)" >&2; exit 2; }
fi

# <role> <kind>: the text of readelf -d (dynamic), nm -D --undefined-only
# (undef) or nm -D --defined-only (defs) for bin, gles or egl.
text() {
  local role=$1 kind=$2 file
  if [ "$MODE" = dumps ]; then
    [ -r "$DUMPS/$role.$kind" ] || { echo "${0##*/}: $DUMPS/$role.$kind is missing" >&2; exit 2; }
    cat "$DUMPS/$role.$kind"
    return
  fi
  case $role in bin) file=$BIN ;; gles) file=$GLES ;; egl) file=$EGL ;; esac
  case $kind in
    dynamic) readelf -d "$file" ;;
    undef) nm -D --undefined-only "$file" ;;
    defs) nm -D --defined-only "$file" ;;
  esac
}
# The names of an nm listing: last field, without a @VERSION suffix.
names() { awk 'NF { n = $NF; sub(/@.*/, "", n); print n }' | sort -u; }
needed() { sed -n 's/.*(NEEDED).*Shared library: \[\(.*\)\]/\1/p'; }
soname() { sed -n 's/.*(SONAME).*Library soname: \[\(.*\)\]/\1/p'; }
runpath() { sed -n 's/.*(RU\{0,1\}N\{0,1\}PATH).*Library r[a-z]*path: \[\(.*\)\]/\1/p'; }

fails=0
fail() { # <cause> <text>
  fails=$((fails + 1))
  echo "FAIL $1: $2"
}
ok() { echo "ok   $1: $2"; }

tmp=$(mktemp -d "${TMPDIR:-/tmp}/glmark2-mali-symbols-XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT
for r in bin gles egl; do text $r dynamic > "$tmp/$r.dynamic" || exit 2; done
text bin undef | names > "$tmp/bin.undef" || exit 2
{ text gles defs; text egl defs; } | names > "$tmp/pair.defs" || exit 2

# pair
before=$fails
sg=$(soname < "$tmp/gles.dynamic")
se=$(soname < "$tmp/egl.dynamic")
[ "$sg" = libGLESv2.so.2 ] || fail pair "the GLES library has SONAME '${sg:-none}', not libGLESv2.so.2"
[ "$se" = libEGL.so.1 ] || fail pair "the EGL library has SONAME '${se:-none}', not libEGL.so.1"
needed < "$tmp/egl.dynamic" | grep -qx libGLESv2.so.2 ||
  fail pair "libEGL.so.1 does not NEED libGLESv2.so.2 (it would not find the pair's, or load none)"
runpath < "$tmp/egl.dynamic" | tr ':' '\n' | grep -qxF '$ORIGIN' ||
  fail pair 'libEGL.so.1 has no RUNPATH $ORIGIN (its NEEDed libGLESv2.so.2 may be the system'"'"'s)'
[ $fails -eq $before ] && ok pair 'libGLESv2.so.2 and libEGL.so.1 (forwarder, NEEDs libGLESv2.so.2, RUNPATH $ORIGIN)'

# binary
before=$fails
needed < "$tmp/bin.dynamic" > "$tmp/bin.needed"
bad=$({
  grep -E '^(libGL|libEGL|libOpenGL|libGLX|libGLdispatch|libGLES|libglapi|libgbm)' "$tmp/bin.needed" |
    grep -vxF -e libEGL.so.1 -e libGLESv2.so.2
  grep -i mesa "$tmp/bin.needed"
} | sort -u | tr '\n' ' ')
[ -z "$bad" ] || fail binary "the program NEEDs ${bad}(not the pair): it links the system glvnd or Mesa"
if [ $fails -eq $before ]; then
  rp=$(runpath < "$tmp/bin.dynamic")
  if grep -qx -e libEGL.so.1 -e libGLESv2.so.2 "$tmp/bin.needed"; then
    if [ -n "$rp" ]; then
      ok binary "links the pair's names, RUNPATH $rp (that directory must hold the pair)"
    else
      ok binary "links the pair's names without RUNPATH: the pair comes from LD_LIBRARY_PATH only, else the system glvnd is loaded"
    fi
  elif [ -n "$rp" ]; then
    ok binary "dlopen flavour, no GL NEEDed, RUNPATH $rp (that directory must hold the pair)"
  else
    ok binary "dlopen flavour, no GL NEEDed, no RUNPATH: the pair comes from LD_LIBRARY_PATH only"
  fi
fi

# imports
before=$fails
core="eglGetDisplay eglInitialize eglBindAPI eglChooseConfig eglCreateContext eglCreatePbufferSurface eglMakeCurrent eglSwapBuffers eglGetProcAddress glGetString glReadPixels glFinish"
{ grep -E '^(gl|egl)[A-Z]' "$tmp/bin.undef"; printf '%s\n' $core; } | sort -u > "$tmp/imports"
missing=$(comm -23 "$tmp/imports" "$tmp/pair.defs" | tr '\n' ' ')
[ -z "$missing" ] || fail imports "the pair does not export: ${missing}"
[ $fails -eq $before ] && ok imports "$(wc -l < "$tmp/imports") gl/egl names the program imports or needs, all exported by the pair"

# source
if [ -n "$SRC" ]; then
  before=$fails
  {
    awk '/^static void glad_egl_load_EGL_VERSION_1_[0-4]\(/ { on = 1; next } /^}/ { on = 0 } on' "$SRC/src/glad/src/egl.c"
    awk '/^static void glad_gl_load_GL_ES_VERSION_2_0\(/ { on = 1; next } /^}/ { on = 0 } on' "$SRC/src/glad/src/gles2.c"
  } | sed -n 's/.*load(userptr, "\([A-Za-z0-9_]*\)").*/\1/p' | sort -u > "$tmp/glad"
  n=$(wc -l < "$tmp/glad")
  if [ "$n" -lt 100 ]; then
    fail source "only $n names found in glad's EGL 1.0-1.4 and GL_ES_VERSION_2_0 loaders (the tree is not glmark2's)"
  else
    missing=$(comm -23 "$tmp/glad" "$tmp/pair.defs" | tr '\n' ' ')
    [ -z "$missing" ] || fail source "glad loads, the pair does not export: ${missing}"
    [ $fails -eq $before ] && ok source "$n names glad loads (EGL 1.0-1.4, GL_ES_VERSION_2_0), all exported by the pair"
  fi
fi

# dlopen
if [ -n "$SRC" ]; then
  if [ -z "$LIBDIR" ] && [ "$MODE" = files ]; then
    LIBDIR=$(dirname "$BIN")/../lib
  fi
  if [ -z "$LIBDIR" ]; then
    echo "skip dlopen: no build-tree lib directory (give --libdir)"
  elif [ ! -f "$SRC/src/gl-state-egl.cpp" ]; then
    fail dlopen "$SRC/src/gl-state-egl.cpp is missing"
  else
    for lib in libEGL libGLESv2; do
      first=$(grep -o "\"$lib\\.so[0-9.]*\"" "$SRC/src/gl-state-egl.cpp" | head -n 1 | tr -d '"')
      if [ -z "$first" ]; then
        fail dlopen "no \"$lib.so...\" name in gl-state-egl.cpp"
      elif [ -e "$LIBDIR/$first" ]; then
        ok dlopen "$lib: first name $first exists in $LIBDIR"
      else
        fail dlopen "$lib: glmark2 dlopen()s $first first and $LIBDIR has none: the system $lib loads"
      fi
    done
  fi
fi

if [ $fails -eq 0 ]; then
  echo "glmark2-mali-symbols: ok"
  exit 0
fi
echo "glmark2-mali-symbols: $fails check(s) failed"
exit 1
