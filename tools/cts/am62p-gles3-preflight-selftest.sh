#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# am62p-gles3-preflight-selftest.sh - selftest of am62p-gles3-preflight.sh on a fake layout (a four-row table,
# stub image records) under $TMPDIR (removed at the end): the all-ok path and every FAIL: generator, table,
# missing/rc=1/other-commit/dirty/unknown images, a wrong directory name or row directory, RUN_BOUND and
# RETURN_BOUND, each env variable unset, rows past the end, the gl_test directory; the native-3D arm from the
# directory name; usage errors (exit 2); that no env value appears in the output and that nothing is written.
# The worker's own env is replaced (env -i). The generator and table checks are injected (PREFLIGHT_GEN,
# PREFLIGHT_CHECK), so no mustpass list is read.
#   TMPDIR=<dir> bash tools/cts/am62p-gles3-preflight-selftest.sh
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
: "${TMPDIR:?set TMPDIR to a directory}"
case $(cd "$TMPDIR" && pwd -P) in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "TMPDIR must not be under /tmp" >&2; exit 2 ;; esac
W=$(mktemp -d "$TMPDIR/am62p-preflight.XXXXXX") || exit 2
trap 'rm -rf "$W"' EXIT
fails=0
SHA=0123456789abcdef0123456789abcdef01234567
S8=01234567
SUM=0123456789abcdef
UARTVAL=selftest-uart-value-7
WSVAL=selftest-ws-value-8
FWVAL=selftest-fw-value-9
rc=0

# a table: rows 1 and 3 plain, row 2 labelled (its record is <LABEL>.built), row 4 plain
cat > "$W/table.txt" <<'EOF'
# fake table
dEQP-GLES3.a.*
# image Q2:
dEQP-GLES3.b.*
dEQP-GLES3.c.*
dEQP-GLES3.d.*
EOF
cat > "$W/gen-ok" <<'EOF'
#!/bin/bash
exit 0
EOF
cat > "$W/gen-bad" <<'EOF'
#!/bin/bash
exit 1
EOF
cp "$W/gen-ok" "$W/chk-ok"; cp "$W/gen-bad" "$W/chk-bad"
chmod +x "$W"/gen-* "$W"/chk-*

mk() { # mk <name>: build/deqp/<name>/ with four good records; echoes the directory
	local d=$W/$1
	mkdir -p "$d/img1-$S8" "$d/Q2" "$d/img3-$S8" "$d/img4-$S8"
	{
		echo "img1 rc=0 bin=$SUM"
		echo "img3 rc=0 bin=$SUM"
		echo "img4 rc=0 bin=$SUM"
		echo BUILD-EXIT
	} > "$d/build.out"
	echo "Q2 rc=0 sha256=$SUM" > "$d/Q2.built"
}
run() { # run <env assignments...> -- args...: output in $W/log, status in $rc
	local e=(PATH="$PATH" HOME="$W" ZEPHYR_WS="$WSVAL" DEQP_WORK="$WSVAL" PVR_FW_BLOB="$FWVAL" BOARD_UART="$UARTVAL"
		RUN_BOUND=900 RETURN_BOUND=240 PREFLIGHT_GEN="$W/gen-ok" PREFLIGHT_CHECK="$W/chk-ok")
	while [ $# -gt 0 ] && [ "$1" != -- ]; do e+=("$1"); shift; done
	shift
	env -i "${e[@]}" bash "$HERE/am62p-gles3-preflight.sh" --table "$W/table.txt" "$@" > "$W/log" 2>&1
	rc=$?
}
unsetv() { # unsetv <VAR>: run with <VAR> removed from the good env
	local v=$1 e=(PATH="$PATH" HOME="$W" ZEPHYR_WS="$WSVAL" DEQP_WORK="$WSVAL" PVR_FW_BLOB="$FWVAL" BOARD_UART="$UARTVAL"
		RUN_BOUND=900 RETURN_BOUND=240 PREFLIGHT_GEN="$W/gen-ok" PREFLIGHT_CHECK="$W/chk-ok") x out=()
	shift
	for x in "${e[@]}"; do case $x in "$v="*) ;; *) out+=("$x") ;; esac; done
	env -i "${out[@]}" bash "$HERE/am62p-gles3-preflight.sh" --table "$W/table.txt" "$@" > "$W/log" 2>&1
	rc=$?
}
expect() { # expect <name> <want rc> <pattern>
	local good=1
	[ "$rc" -eq "$2" ] || good=0
	grep -q -- "$3" "$W/log" || good=0
	for v in "$UARTVAL" "$WSVAL" "$FWVAL" "$W"; do
		if grep -q -- "$v" "$W/log"; then good=0; echo "  FAIL a value or path of the layout in the output"; fi
	done
	if [ $good -eq 1 ]; then echo "  ok   $1"
	else echo "  FAIL $1 (rc=$rc)"; sed 's/^/       /' "$W/log"; fails=$((fails + 1)); fi
}

D=am62p-gles3-$S8
mk "$D"
find "$W/$D" | sort > "$W/before"
run -- --commit "$SHA" --img-dir "$W/$D"
expect "all ok" 0 "preflight: all ok"
grep -q "ok   all 4 built image(s) are commit $S8" "$W/log" || { echo "  FAIL commit line"; fails=$((fails + 1)); }
grep -q "ok   arm: DEQP_PVR_NATIVE_3D unset" "$W/log" || { echo "  FAIL default arm line"; fails=$((fails + 1)); }
grep -q "ok   BOARD_UART is set" "$W/log" || { echo "  FAIL BOARD_UART line"; fails=$((fails + 1)); }
run -- --commit "$S8" --img-dir "$W/$D" --rows 0,2-3
expect "8-digit commit, rows 0,2-3" 0 "ok   rows 0,2-3: 3 image(s) of 4"
run GLES3_WORK=x DEQP_WORK= -- --commit "$SHA" --img-dir "$W/$D"
expect "GLES3_WORK alone is enough" 0 "ok   GLES3_WORK is set"
run -- --commit "$SHA" --img-dir "$W/$D" --skip-table
expect "--skip-table" 0 "skip generator and table"

# the arm, from the directory name
mk "$D-n3d1"
run -- --commit "$SHA" --img-dir "$W/$D-n3d1"
expect "arm n3d1" 0 "ok   arm: DEQP_PVR_NATIVE_3D=1"
mk "$D-n3d0-idiv1"
run -- --commit "$SHA" --img-dir "$W/$D-n3d0-idiv1"
expect "arm n3d0 idiv1" 0 "ok   arm: DEQP_PVR_INSTANCE_DIVISOR=1"
mk "$D-bogus"
run -- --commit "$SHA" --img-dir "$W/$D-bogus"
expect "unknown arm suffix" 1 "FAIL image directory name has an unknown suffix"

# the table
run PREFLIGHT_GEN="$W/gen-bad" -- --commit "$SHA" --img-dir "$W/$D"
expect "generator --check fails" 1 "FAIL gen-am62p-gles3-tree.py --check fails"
run PREFLIGHT_CHECK="$W/chk-bad" -- --commit "$SHA" --img-dir "$W/$D"
expect "check-gles3-tree fails" 1 "FAIL table fails check-gles3-tree.py"
run -- --commit "$SHA" --img-dir "$W/$D" --table "$W/nonexistent"
expect "missing table" 1 "FAIL table file not found"

# the images
mk "$D-m"; mv "$W/$D-m" "$W/x"; mkdir "$W/m" ; mv "$W/x" "$W/m/$D"
sed -i '/^img3 /d' "$W/m/$D/build.out"
run -- --commit "$SHA" --img-dir "$W/m/$D"
expect "build.out line missing" 1 "FAIL image(s) without a build record .*img3"
rm "$W/m/$D/Q2.built"
run -- --commit "$SHA" --img-dir "$W/m/$D"
expect "labelled record missing" 1 "FAIL image(s) without a build record .*Q2"
mkdir "$W/r"; mk "$D-r"; mv "$W/$D-r" "$W/r/$D"
sed -i 's/^img4 rc=0 bin=.*/img4 rc=1 bin=none/' "$W/r/$D/build.out"
run -- --commit "$SHA" --img-dir "$W/r/$D"
expect "row with rc=1" 1 "FAIL image(s) not built .*img4(rc=1"
echo "Q2 rc=not-conformance sha256=none" > "$W/r/$D/Q2.built"
run -- --commit "$SHA" --img-dir "$W/r/$D"
expect "labelled row not conformance" 1 "FAIL image(s) not built .*Q2(rc=not-conformance"
run -- --commit "$SHA" --img-dir "$W/r/$D" --rows 0-2
expect "rows narrow the check (img4 left out, Q2 still bad)" 1 "FAIL image(s) not built .*Q2"
mkdir "$W/b"; mk "$D-b"; mv "$W/$D-b" "$W/b/$D"
echo "Q2 rc=0 sha256=$SUM build=fedcba98" > "$W/b/$D/Q2.built"
run -- --commit "$SHA" --img-dir "$W/b/$D"
expect "build string of another commit" 1 "FAIL image(s) not built from commit $S8: Q2(build fedcba98)"
echo "Q2 rc=0 sha256=$SUM build=$S8-dirty" > "$W/b/$D/Q2.built"
run -- --commit "$SHA" --img-dir "$W/b/$D"
expect "dirty build string" 1 "FAIL image(s) not built from commit $S8: Q2(build .*-dirty"
echo "Q2 rc=0 sha256=$SUM build=unknown" > "$W/b/$D/Q2.built"
run -- --commit "$SHA" --img-dir "$W/b/$D"
expect "unknown build string" 1 "FAIL commit not recorded for: Q2"
echo "Q2 rc=0 sha256=$SUM build=$S8" > "$W/b/$D/Q2.built"
run -- --commit "$SHA" --img-dir "$W/b/$D"
expect "clean build string agrees" 0 "preflight: all ok"
mkdir "$W/b/$D/img3-aaaaaaaa"
run -- --commit "$SHA" --img-dir "$W/b/$D"
expect "row directory of another tag" 1 "FAIL image(s) not built from commit $S8: img3(directory img3-aaaaaaaa)"

mkdir "$W/o"; mk "am62p-gles3-fedcba98"; mv "$W/am62p-gles3-fedcba98" "$W/o/"
run -- --commit "$SHA" --img-dir "$W/o/am62p-gles3-fedcba98"
expect "directory of another commit" 1 "FAIL image directory is commit fedcba98, not $S8"
mkdir "$W/p"; mk "$D-dirty"; mv "$W/$D-dirty" "$W/p/"
run -- --commit "$SHA" --img-dir "$W/p/$D-dirty"
expect "dirty directory name" 1 "FAIL image directory name says a modified tree"
mkdir "$W/q"; mk "mybuild"; mv "$W/mybuild" "$W/q/"
run -- --commit "$SHA" --img-dir "$W/q/mybuild"
expect "directory name without identity" 1 "FAIL image directory name carries no build identity"
run -- --commit "$SHA" --img-dir "$W/nonexistent"
expect "image directory missing" 1 "FAIL image directory does not exist"
run -- --commit "$SHA" --img-dir "$W/$D" --rows 3-4
expect "rows past the table end" 1 "FAIL rows 3-4 reach past the table end (rows 0-3)"
run -- --commit zz --img-dir "$W/$D"
expect "commit not hex" 1 "FAIL commit is not hex"
run -- --commit abc123 --img-dir "$W/$D"
expect "commit too short" 1 "FAIL commit is shorter than 8"

# the bounds
run RUN_BOUND=899 -- --commit "$SHA" --img-dir "$W/$D"
expect "RUN_BOUND 899" 1 "FAIL RUN_BOUND is 899, below 900"
run RUN_BOUND=1500 -- --commit "$SHA" --img-dir "$W/$D"
expect "RUN_BOUND 1500" 0 "ok   RUN_BOUND is 1500"
run RUN_BOUND=abc -- --commit "$SHA" --img-dir "$W/$D"
expect "RUN_BOUND not a number" 1 "FAIL RUN_BOUND is not a number"
unsetv RUN_BOUND --commit "$SHA" --img-dir "$W/$D"
expect "RUN_BOUND unset" 1 "FAIL RUN_BOUND is not set"
run RETURN_BOUND=180 -- --commit "$SHA" --img-dir "$W/$D"
expect "RETURN_BOUND 180" 1 "FAIL RETURN_BOUND is 180, not 240"
unsetv RETURN_BOUND --commit "$SHA" --img-dir "$W/$D"
expect "RETURN_BOUND unset" 1 "FAIL RETURN_BOUND is not set"

# the env
for v in ZEPHYR_WS PVR_FW_BLOB BOARD_UART; do
	unsetv "$v" --commit "$SHA" --img-dir "$W/$D"
	expect "$v unset" 1 "FAIL $v is unset"
done
unsetv DEQP_WORK --commit "$SHA" --img-dir "$W/$D"
expect "no stage 1 work directory" 1 "FAIL neither DEQP_WORK nor GLES3_WORK is set"

# the gl_test directory
mkdir "$W/gl" "$W/glempty"
echo stub > "$W/gl/gl_test.bin"
run -- --commit "$SHA" --img-dir "$W/$D" --gltest-img "$W/gl"
expect "gl_test directory ok" 0 "ok   gl_test image directory exists"
run -- --commit "$SHA" --img-dir "$W/$D" --gltest-img "$W/glempty"
expect "gl_test directory empty" 1 "FAIL gl_test image directory is empty"
run -- --commit "$SHA" --img-dir "$W/$D" --gltest-img "$W/nonexistent"
expect "gl_test directory missing" 1 "FAIL gl_test image directory does not exist"

# nothing in the environment
env -i PATH="$PATH" PREFLIGHT_GEN="$W/gen-ok" PREFLIGHT_CHECK="$W/chk-ok" bash "$HERE/am62p-gles3-preflight.sh" \
	--table "$W/table.txt" --commit "$SHA" --img-dir "$W/$D" > "$W/log" 2>&1; rc=$?
expect "empty env fails with FAIL lines" 1 "FAIL RUN_BOUND is not set"
if grep -qi 'traceback' "$W/log"; then echo "  FAIL traceback with no environment"; fails=$((fails + 1)); fi

# usage errors
for bad in "--bogus" "--rows 5-1" "--rows a" "--rows 1-2-3" "--commit" "--img-dir" "--table" "--gltest-img"; do
	# shellcheck disable=SC2086
	env -i PATH="$PATH" bash "$HERE/am62p-gles3-preflight.sh" $bad > "$W/log" 2>&1; rc=$?
	expect "usage error: $bad" 2 "usage:"
done

find "$W/$D" | sort > "$W/after"
if cmp -s "$W/before" "$W/after"; then echo "  ok   read-only (file listing unchanged)"
else echo "  FAIL read-only"; diff "$W/before" "$W/after"; fails=$((fails + 1)); fi

if [ $fails -eq 0 ]; then echo "preflight selftest: all ok"; exit 0; fi
echo "preflight selftest: $fails FAILED"
exit 1
