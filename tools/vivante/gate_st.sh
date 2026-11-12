#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# gate_st.sh - the two Vivante host checks for the STM32MP257F's GPU row
# (model 0x8000, revision 0x6205, product 0x80003, eco 0; issues #1281 Q1, #7)
# against a Mesa 26.2.1 tree. Host only, no board.
#
#   tools/vivante/gate_st.sh hwdb     the hwdb comparison equals the pinned
#                                     tools/vivante/testdata/hwdb-compare-st-6205.txt
#   tools/vivante/gate_st.sh golden   cs_golden against the GC7000UL and the
#                                     GCNANOULTRA31 shim rows: the signature of
#                                     modes 1, 8 and 15 equals GOLDEN_WANT
#                                     (README.md, "Another part: the ST row")
#
# Environment:
#   MESA_ETNA_TREE   Mesa 26.2.1 source tree (required)
#   MESA_ETNA_BUILD  build directory for `golden` (default
#                    $MESA_ETNA_TREE/build/mesa-etnaviv-host, then
#                    $MESA_ETNA_TREE/build-host, then
#                    $MESA_ETNA_TREE/build); the tree must carry
#                    mesa-cs-golden.patch and be built as the README says
#   CC               C compiler for cs_golden.c (default cc)
#   GATE_ST_WANT_FILE  a file replacing GOLDEN_WANT (for testing)
#
# Exit codes: 0 pass; 1 mismatch (the diff is on stdout); 2 gap (exactly one
# `gap:` line on stderr; the check was NOT run, and a gap is not a pass);
# 64 usage.

set -u
export LC_ALL=C

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

# Per mode: the GCNANOULTRA31 run's renderer line, whether PE_MEM_CONFIG
# moved against the GC7000UL run, whether TS_MEM_CONFIG moved, and how many
# other decoded lines moved. Modes 1 and 8 draw with colour (and depth) tile
# status; mode 15 is a depth clear with nothing drawn, whose stream is the
# same on both rows.
GOLDEN_WANT=$(
	cat <<'EOF'
mode 1 renderer: Vivante GC8000 rev 6205 pe_mem_config=moved ts_mem_config=moved other=0
mode 8 renderer: Vivante GC8000 rev 6205 pe_mem_config=moved ts_mem_config=moved other=0
mode 15 renderer: Vivante GC8000 rev 6205 pe_mem_config=same ts_mem_config=same other=0
EOF
)

usage() {
	echo "usage: $0 hwdb|golden" >&2
	exit 64
}

gap() {
	# $1: subcommand, $2: why
	echo "gap: MESA_ETNA_TREE $2; the gate_st $1 check was NOT run" >&2
	exit 2
}

[ $# -eq 1 ] || usage
case $1 in
hwdb | golden) SUB=$1 ;;
*) usage ;;
esac

TREE=${MESA_ETNA_TREE:-}
[ -n "$TREE" ] || gap "$SUB" "is not set"
[ -d "$TREE" ] || gap "$SUB" "is not a directory ($TREE)"

run_hwdb() {
	local h
	for h in st nxp; do
		[ -f "$TREE/src/etnaviv/hwdb/$h/gc_feature_database.h" ] ||
			gap hwdb "lacks src/etnaviv/hwdb/$h/gc_feature_database.h"
	done
	python3 -I "$HERE/hwdb_compare.py" "$TREE" --check "$HERE/testdata/hwdb-compare-st-6205.txt"
}

# One signature line per mode, from the two decoded streams and the ST run.
signature() {
	# $1 mode, $2 file with the GC7000UL decode, $3 file with the ST decode,
	# $4 file with the ST run's stderr
	local mode=$1 a=$2 b=$3 err=$4 renderer pe ts other d
	renderer=$(grep -m1 '^renderer:' "$err" || true)
	[ -n "$renderer" ] || renderer="renderer: (none)"
	d=$(diff "$a" "$b" | grep '^[<>]' || true)
	if printf '%s\n' "$d" | grep -q 'PE_MEM_CONFIG'; then pe=moved; else pe=same; fi
	if printf '%s\n' "$d" | grep -q 'TS_MEM_CONFIG'; then ts=moved; else ts=same; fi
	other=$(printf '%s\n' "$d" | grep -v 'PE_MEM_CONFIG\|TS_MEM_CONFIG' | grep -c '^[<>]' || true)
	echo "mode $mode $renderer pe_mem_config=$pe ts_mem_config=$ts other=$other"
}

run_golden() {
	local build="" b piece sub shim egl gles regs inc libdirs d
	if [ -n "${MESA_ETNA_BUILD:-}" ]; then
		build=$MESA_ETNA_BUILD
		[ -d "$build" ] || gap golden "has a MESA_ETNA_BUILD that is not a directory ($build)"
	else
		for b in "$TREE/build/mesa-etnaviv-host" "$TREE/build-host" "$TREE/build"; do
			if [ -d "$b" ]; then
				build=$b
				break
			fi
		done
		[ -n "$build" ] || gap golden "has no build directory (build/mesa-etnaviv-host, build-host or build)"
	fi

	shim=$(find "$build" -name libetnaviv_noop_drm_shim.so -print -quit 2>/dev/null)
	egl=$(find "$build" -name 'libEGL.so' -print -quit 2>/dev/null)
	gles=$(find "$build" -name 'libGLESv2.so' -print -quit 2>/dev/null)
	[ -n "$shim" ] || gap golden "has no libetnaviv_noop_drm_shim.so under $build"
	[ -n "$egl" ] || gap golden "has no libEGL.so under $build"
	[ -n "$gles" ] || gap golden "has no libGLESv2.so under $build"
	regs=$TREE/src/etnaviv/hw/state_3d.xml.h
	inc=$TREE/include
	[ -f "$regs" ] || gap golden "lacks src/etnaviv/hw/state_3d.xml.h"
	[ -f "$inc/GLES2/gl2.h" ] || gap golden "lacks include/GLES2/gl2.h"
	grep -q ETNA_SHIM_CS_DUMP "$TREE/src/etnaviv/drm-shim/etnaviv_noop.c" 2>/dev/null ||
		gap golden "is not patched with mesa-cs-golden.patch (no ETNA_SHIM_CS_DUMP in etnaviv_noop.c)"

	# Every directory under the build with a shared library, so the shim,
	# libEGL and the driver find each other.
	libdirs=
	while IFS= read -r d; do
		libdirs=$libdirs:$d
	done < <(find "$build" -name 'lib*.so*' -printf '%h\n' 2>/dev/null | sort -u)
	libdirs=${libdirs#:}

	local tmp cc
	tmp=$(mktemp -d "${TMPDIR:-/tmp}/gate_st.XXXXXX") || {
		echo "gate_st: cannot create a temporary directory" >&2
		exit 1
	}
	trap 'rm -rf "$tmp"' EXIT
	read -r -a cc <<<"${CC:-cc}"
	if ! "${cc[@]}" -O1 -I"$inc" -I"$build/include" -o "$tmp/cs_golden" "$HERE/cs_golden.c" \
		-L"$(dirname "$egl")" -L"$(dirname "$gles")" -lEGL -lGLESv2 2>"$tmp/cc.err"; then
		echo "gate_st: building cs_golden.c failed:"
		cat "$tmp/cc.err"
		exit 1
	fi

	local mode gpu status=0 got="" regs2args=()
	[ -f "$TREE/src/etnaviv/hw/state.xml.h" ] && regs2args=(--regs2 "$TREE/src/etnaviv/hw/state.xml.h")
	for mode in 1 8 15; do
		for gpu in GC7000UL GCNANOULTRA31; do
			# MESA_LOADER_DRIVER_OVERRIDE: on a host with a GPU of its own the EGL
			# loader names that GPU's driver ("virtio_gpu: driver missing") and
			# eglInitialize fails before the shim is asked anything.
			env LD_PRELOAD="$shim" LD_LIBRARY_PATH="$libdirs" EGL_PLATFORM=surfaceless \
				MESA_LOADER_DRIVER_OVERRIDE=etnaviv \
				ETNA_SHIM_GPU="$gpu" \
				ETNA_SHIM_CS_DUMP="$tmp/$mode.$gpu.cs" ETNA_SHIM_MEM_DUMP="$tmp/$mode.$gpu.mem" \
				"$tmp/cs_golden" "$mode" >"$tmp/$mode.$gpu.out" 2>"$tmp/$mode.$gpu.err" || {
				echo "mismatch: cs_golden mode $mode on $gpu exited nonzero"
				cat "$tmp/$mode.$gpu.err"
				status=1
				continue 2
			}
			python3 -I "$HERE/cs_decode.py" "$tmp/$mode.$gpu.cs" --regs "$regs" "${regs2args[@]}" \
				>"$tmp/$mode.$gpu.txt" 2>"$tmp/$mode.$gpu.derr" || {
				echo "mismatch: cs_decode.py failed on mode $mode, $gpu"
				cat "$tmp/$mode.$gpu.derr"
				status=1
				continue 2
			}
		done
		got=$got$(signature "$mode" "$tmp/$mode.GC7000UL.txt" "$tmp/$mode.GCNANOULTRA31.txt" \
			"$tmp/$mode.GCNANOULTRA31.err")$'\n'
	done

	local want=$GOLDEN_WANT
	if [ -n "${GATE_ST_WANT_FILE:-}" ]; then
		want=$(cat "$GATE_ST_WANT_FILE") || {
			echo "gate_st: cannot read GATE_ST_WANT_FILE" >&2
			exit 1
		}
	fi
	got=${got%$'\n'}
	if [ "$status" -eq 0 ] && [ "$got" = "$want" ]; then
		echo "gate_st golden: signature matches (3 modes)"
		return 0
	fi
	echo "gate_st golden: signature differs (want -, got +)"
	diff <(printf '%s\n' "$want") <(printf '%s\n' "$got") || true
	return 1
}

case $SUB in
hwdb) run_hwdb ;;
golden) run_golden ;;
esac
exit $?
