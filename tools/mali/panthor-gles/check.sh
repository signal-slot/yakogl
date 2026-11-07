#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Host check of the pair build.sh wrote: tools/mali/panthor-gles/check.sh <dir>
#
#   - both are AArch64 shared objects with their SONAMEs (readelf -d), and
#     libEGL.so.1 needs libGLESv2.so.2 and finds it beside itself ($ORIGIN);
#   - every entry point dEQP's yakogl target looks up by name
#     (src/deqp-entry-points.txt) is a defined function there: the ES 2.0
#     ones in libGLESv2.so.2, the EGL ones in libEGL.so.1 or the library it
#     needs (what dlsym on the libEGL.so.1 handle searches);
#   - nothing needs a GLIBC above 2.41 or a GLIBCXX above 3.4.33, the board's
#     floors (objdump -T when one that reads AArch64 is here, else
#     readelf -V: the same version needs);
#   - libGLESv2.so.2 carries the contract lines' text and a constructor.
#
# Exit 0: every check passed. 1: a check failed (each failure is a line).
# 2: usage, or a tool is missing.
set -eu

GLIBC_MAX=2.41
GLIBCXX_MAX=3.4.33

here=$(cd "$(dirname "$0")" && pwd)
list=$here/src/deqp-entry-points.txt
[ $# -eq 1 ] || {
	echo "usage: check.sh <dir with libGLESv2.so.2 and libEGL.so.1>" >&2
	exit 2
}
dir=$1
gles=$dir/libGLESv2.so.2
egl=$dir/libEGL.so.1
command -v readelf >/dev/null 2>&1 || {
	echo "check.sh: needs readelf (binutils)" >&2
	exit 2
}

fails=0
ok() { echo "ok    $*"; }
bad() {
	echo "FAIL  $*"
	fails=$((fails + 1))
}

for f in "$gles" "$egl"; do
	[ -f "$f" ] || {
		echo "check.sh: $f is missing (tools/mali/panthor-gles/build.sh $dir writes it)" >&2
		exit 2
	}
done

# readelf -d's value in brackets for a tag: soname, needed, runpath
dyn() { readelf -d "$1" | sed -n "s/.*($2).*\[\(.*\)\]/\1/p"; }

for f in "$gles" "$egl"; do
	h=$(readelf -h "$f")
	case "$h" in
	*"Type:"*"DYN"*) ;;
	*) bad "${f##*/}: not a shared object" ;;
	esac
	case "$h" in
	*"Machine:"*"AArch64"*) ok "${f##*/}: AArch64 shared object" ;;
	*) bad "${f##*/}: not AArch64 ($(printf '%s\n' "$h" | sed -n 's/^ *Machine: *//p'))" ;;
	esac
done

s=$(dyn "$gles" SONAME)
[ "$s" = libGLESv2.so.2 ] && ok "libGLESv2.so.2: SONAME $s" || bad "libGLESv2.so.2: SONAME '${s}', want libGLESv2.so.2"
s=$(dyn "$egl" SONAME)
[ "$s" = libEGL.so.1 ] && ok "libEGL.so.1: SONAME $s" || bad "libEGL.so.1: SONAME '${s}', want libEGL.so.1"
if dyn "$egl" NEEDED | grep -qx libGLESv2.so.2; then
	ok "libEGL.so.1: NEEDED libGLESv2.so.2"
else
	bad "libEGL.so.1: does not need libGLESv2.so.2 (the EGL entry points are there)"
fi
# shellcheck disable=SC2016 # the loader's $ORIGIN
if dyn "$egl" RUNPATH | tr ':' '\n' | grep -qx '$ORIGIN'; then
	ok "libEGL.so.1: RUNPATH \$ORIGIN (finds the libGLESv2.so.2 beside it, not the board's)"
else
	bad "libEGL.so.1: no RUNPATH \$ORIGIN"
fi
if readelf -d "$gles" | grep -q '(INIT_ARRAY)'; then
	ok "libGLESv2.so.2: has a constructor (INIT_ARRAY)"
else
	bad "libGLESv2.so.2: no INIT_ARRAY: nothing installs the OSAL at load"
fi

# defined, global or weak, default-visibility functions of an object
exports() {
	readelf -W --dyn-syms "$1" |
		awk '$4 == "FUNC" && ($5 == "GLOBAL" || $5 == "WEAK") && $6 == "DEFAULT" && $7 != "UND" { sub(/@.*/, "", $8); print $8 }' |
		sort -u
}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/panthor-gles-check.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
exports "$gles" >"$tmp/gles"
{
	cat "$tmp/gles"
	exports "$egl"
} | sort -u >"$tmp/egl"
n_egl=0
n_gles=0
missing=
while read -r api name; do
	case "$api" in
	'' | '#'*) continue ;;
	egl) set -- "$tmp/egl" && n_egl=$((n_egl + 1)) ;;
	gles2) set -- "$tmp/gles" && n_gles=$((n_gles + 1)) ;;
	*)
		bad "$list: unknown api '$api'"
		continue
		;;
	esac
	grep -qx "$name" "$1" || missing="$missing $api:$name"
done <"$list"
if [ -z "$missing" ]; then
	ok "entry points: all $n_egl EGL and $n_gles ES 2.0 names dEQP looks up are exported"
else
	bad "entry points missing:$missing"
fi
[ "$n_egl" -gt 0 ] && [ "$n_gles" -gt 0 ] || bad "$list: no names read"

# The version needs. An objdump that reads AArch64 (the cross binutils' or
# LLVM's) is used when there is one; the host's GNU objdump reads only its
# own architecture as a rule, and readelf -V gives the same needs.
objdump=
for o in aarch64-linux-gnu-objdump llvm-objdump objdump; do
	command -v "$o" >/dev/null 2>&1 || continue
	if "$o" -T "$gles" >/dev/null 2>&1; then
		objdump=$o
		break
	fi
done
for f in "$gles" "$egl"; do
	if [ -n "$objdump" ]; then
		"$objdump" -T "$f"
	else
		readelf -W -V "$f"
	fi | grep -o 'GLIBC\(XX\)\{0,1\}_[0-9][0-9.]*' | sort -u >"$tmp/vers"
	worst=
	for lib in GLIBC GLIBCXX; do
		max=$GLIBC_MAX
		[ "$lib" = GLIBCXX ] && max=$GLIBCXX_MAX
		top=$(sed -n "s/^${lib}_//p" "$tmp/vers" | sort -V | tail -n 1)
		[ -n "$top" ] || continue
		if [ "$(printf '%s\n%s\n' "$top" "$max" | sort -V | tail -n 1)" = "$max" ]; then
			worst="$worst ${lib}_$top"
		else
			bad "${f##*/}: needs ${lib}_$top, above the board's ${lib}_$max"
			worst="$worst ${lib}_$top(!)"
		fi
	done
	case "$worst" in
	*'(!)'*) ;;
	*) ok "${f##*/}: highest version needs:${worst:- none} (ceiling GLIBC_$GLIBC_MAX, GLIBCXX_$GLIBCXX_MAX; ${objdump:-readelf -V})" ;;
	esac
done

for text in "yakogl-mali: loaded" "yakogl-mali-exit:" "yakogl-build="; do
	if grep -aq "$text" "$gles"; then
		ok "libGLESv2.so.2: carries '$text'"
	else
		bad "libGLESv2.so.2: no '$text' in the object"
	fi
done

if [ "$fails" -eq 0 ]; then
	echo "check.sh: $dir: every check passed"
	exit 0
fi
echo "check.sh: $dir: $fails check(s) failed"
exit 1
