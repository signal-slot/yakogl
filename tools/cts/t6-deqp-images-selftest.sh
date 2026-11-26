#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Self-test of tools/cts/t6-deqp-images.sh --plan: every row of the
# repository's own mali-gles2-tree.txt resolves, the environment is checked
# by name (a set value must never be printed), unknown rows, missing names,
# a bad DEQP_MODULE and a bad table are refused, the commands and the output
# paths of the coming build are printed, and nothing is built; and of the
# SHA256SUMS and COMMIT a build leaves for driver/tools/uboot-t6/preflight.sh
# (a fake builder for the rows, preflight.sh on the result). Needs no
# toolchain, no west workspace and no board; the labels are read from the
# table, so a row moved or renamed here changes nothing.
# tools/check_selftests.py gives this file its row in the gate.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$HERE/mali-gles2-tree.txt
S=$HERE/t6-deqp-images.sh
W=$(mktemp -d "${TMPDIR:-/tmp}/t6plan.XXXXXX") || exit 1
trap 'rm -rf "$W"' EXIT
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }
unset DEQP_MODULE DEQP_STAGE1
# Values no test may ever see on stdout or stderr of the script under test.
EWS=planws7c4e ESDK=plansdk7c4e EWORK=planwork7c4e
export ZEPHYR_WS=$EWS ZEPHYR_SDK_INSTALL_DIR=$ESDK DEQP_WORK=$EWORK

# every "# image <LABEL>:" label, as the builder reads it
labels=$(awk '/^# image [A-Za-z0-9]+:/ { l = $3; sub(/:$/, "", l); print l }' "$T")
[ -n "$labels" ] || { echo "no image row in $T"; exit 1; }
first=${labels%%$'\n'*}

out=$("$S" --plan "$T" "$W/img" "$first" 2>"$W/err"); rc=$?
if [ $rc = 0 ]; then ok "a known row: exit 0"; else bad "known row: rc $rc: $(cat "$W/err")"; fi
if grep -q "^$first: filter " <<<"$out" && grep -q "cmake .*-B $W/img/$first -DBOARD=nanopc_t6 " <<<"$out" &&
	grep -q "ninja -C $W/img/$first\$" <<<"$out" &&
	grep -q "$W/img/$first/zephyr/zephyr.bin" <<<"$out" &&
	grep -q "$W/img/$first.built" <<<"$out" && grep -q "$W/img/$first.log" <<<"$out" &&
	grep -q -- "-DCONFIG_DEQP_CASE=" <<<"$out"; then
	ok "the cmake and ninja commands and the image, .built and log paths are printed"
else bad "plan output: $(head -4 <<<"$out")"; fi
if grep -q "$EWS\|$ESDK\|$EWORK" <<<"$out$(cat "$W/err")"; then
	bad "a value of the environment was printed"
else ok "the environment is named, never valued"; fi
if [ ! -e "$W/img" ]; then ok "nothing is built"; else bad "--plan created $W/img"; fi

all=$("$S" --plan "$T" "$W/img" 2>/dev/null); rc=$?
if [ $rc = 0 ]; then ok "no LABEL: exit 0 (all rows)"; else bad "no LABEL: rc $rc"; fi
rowsok=1
for l in $labels; do
	c=$(grep -c "^$l: filter " <<<"$all")
	[ "$c" = 1 ] || { bad "row $l: $c plan lines, want 1"; rowsok=0; }
done
[ $rowsok = 1 ] && ok "every label of $T names exactly one plan row ($(printf '%s\n' "$labels" | grep -c .) rows)"

"$S" --plan "$T" "$W/img" NOSUCHROW >"$W/o" 2>"$W/err"; rc=$?
if [ $rc != 0 ] && grep -q "unknown row NOSUCHROW" "$W/err"; then
	ok "an unknown row: refused, named"
else bad "unknown row: rc $rc: $(cat "$W/err")"; fi
"$S" --plan "$T" "$W/img" "$first" NOSUCHROW >"$W/o" 2>"$W/err"; rc=$?
if [ $rc != 0 ] && grep -q "unknown row NOSUCHROW" "$W/err"; then
	ok "an unknown row beside a known one: refused, named"
else bad "mixed rows: rc $rc: $(cat "$W/err")"; fi

for v in ZEPHYR_WS ZEPHYR_SDK_INSTALL_DIR DEQP_WORK; do
	env -u "$v" "$S" --plan "$T" "$W/img" "$first" >"$W/o" 2>"$W/err"; rc=$?
	if [ $rc != 0 ] && grep -q "$v is not set" "$W/err" &&
		! grep -q "$EWS\|$ESDK\|$EWORK" "$W/err"; then
		ok "a missing $v: refused, named, unvalued"
	else bad "missing $v: rc $rc: $(cat "$W/err")"; fi
done
env -u DEQP_WORK DEQP_STAGE1=$W/stage1 "$S" --plan "$T" "$W/img" "$first" >"$W/o" 2>"$W/err"; rc=$?
if [ $rc != 0 ] && grep -q "DEQP_WORK is not set" "$W/err" && grep -q "DEQP_STAGE1 is set" "$W/err"; then
	ok "DEQP_STAGE1 alone: refused, and the message points at DEQP_WORK"
else bad "DEQP_STAGE1 alone: rc $rc: $(cat "$W/err")"; fi

out=$(DEQP_MODULE=gles3 "$S" --plan "$T" "$W/img" "$first" 2>"$W/err"); rc=$?
if [ $rc = 0 ] && grep -q -- "-DDEQP_MODULE=gles3" <<<"$out" && grep -q -- "--gles3" <<<"$out"; then
	ok "DEQP_MODULE=gles3: its cmake arguments and the --gles3 check are in the plan"
else bad "DEQP_MODULE=gles3: rc $rc: $(head -3 <<<"$out") $(cat "$W/err")"; fi
DEQP_MODULE=gl3 "$S" --plan "$T" "$W/img" "$first" >"$W/o" 2>"$W/err"; rc=$?
if [ $rc != 0 ] && grep -q "DEQP_MODULE must be gles2 or gles3" "$W/err"; then
	ok "a bad DEQP_MODULE: refused"
else bad "bad DEQP_MODULE: rc $rc: $(cat "$W/err")"; fi

# a texture-2048.conf row of one's own: the marker's argument joins that row's
# cmake line only
printf '# image A1: x\ndEQP-GLES2.a.*\n# image B7: y (with texture-2048.conf)\ndEQP-GLES2.b.*\n' > "$W/tex.txt"
out=$("$S" --plan "$W/tex.txt" "$W/img" 2>"$W/err"); rc=$?
if [ $rc = 0 ] && grep -q -- "-DEXTRA_CONF_FILE=texture-2048.conf" <<<"$out" &&
	[ "$(grep -c 'texture-2048' <<<"$out")" = 1 ] &&
	grep -q "^A1: filter dEQP-GLES2.a.\*$" <<<"$out"; then
	ok "a texture-2048.conf row: the argument on its own cmake line"
else bad "texture row: rc $rc: $(head -3 <<<"$out")"; fi

"$S" --plan "$W/none.txt" "$W/img" >"$W/o" 2>"$W/err"; rc=$?
[ $rc != 0 ] && ok "a missing table: refused" || bad "missing table: rc $rc"
printf '# image A1: x\ndEQP-GLES2.a.*\ndEQP-GLES2.b.*\n' > "$W/nolabel.txt"
"$S" --plan "$W/nolabel.txt" "$W/img" >"$W/o" 2>"$W/err"; rc=$?
if [ $rc != 0 ] && grep -q "filter line without a label" "$W/err"; then
	ok "a filter line without a label: refused, the line named"
else bad "no-label line: rc $rc: $(cat "$W/err")"; fi
"$S" --plan >"$W/o" 2>"$W/err"; rc=$?
[ $rc != 0 ] && ok "--plan without arguments: refused with the usage" || bad "--plan alone: rc $rc"

# --dry-run is the builder's pass-through: rows, no environment, nothing built
out=$(env -u ZEPHYR_WS -u DEQP_WORK "$S" --dry-run "$T" "$W/img2" 2>"$W/err"); rc=$?
if [ $rc = 0 ] && [ "$(grep -c '^[A-Za-z0-9]* dEQP-GLES2' <<<"$out")" = "$(printf '%s\n' "$labels" | grep -c .)" ] &&
	[ ! -e "$W/img2" ]; then
	ok "--dry-run still names every row with its filter, no environment, nothing built"
else bad "--dry-run: rc $rc: $(cat "$W/err")"; fi

# SHA256SUMS and COMMIT, what driver/tools/uboot-t6/preflight.sh reads before
# a U-Boot session. A fake builder (T6_DEQP_BUILDER) makes the row
# directories as deqp-tree-images.sh does (the image, the <LABEL>.built
# line); FAILROW and EMPTYROW stand for a build that failed and an image
# that came out empty. The two files are written by the script under test.
cat > "$W/fake-builder.sh" <<'EOF'
#!/bin/bash
# fake deqp-tree-images.sh: nanopc_t6 [--dry-run] <table> <img-dir> [LABEL...]
shift
dry=; [ "${1:-}" = --dry-run ] && { dry=1; shift; }
tab=$1 out=$2; shift 2
[ -n "${FAKE_BUILDER_RC:-}" ] && { echo "fake builder: refused (rc $FAKE_BUILDER_RC)" >&2; exit "$FAKE_BUILDER_RC"; }
sel=$(awk '/^# image [A-Za-z0-9]+:/ { l = $3; sub(/:$/, "", l); print l }' "$tab")
[ $# -gt 0 ] && sel=$(printf '%s\n' "$@")
for l in $sel; do
	if [ -n "$dry" ]; then echo "$l dEQP-GLES2.x.*"; continue; fi
	mkdir -p "$out/$l/zephyr"
	case $l in
	FAILROW) echo "$l rc=1 sha256=none build=none" > "$out/$l.built"; continue ;;
	EMPTYROW) : > "$out/$l/zephyr/zephyr.bin"
		echo "$l rc=0 sha256=none build=none" > "$out/$l.built"; continue ;;
	esac
	echo "image of $l" > "$out/$l/zephyr/zephyr.bin"
	echo "$l rc=0 sha256=x build=y" > "$out/$l.built"
done
EOF
chmod +x "$W/fake-builder.sh"
printf '# image A1: x\ndEQP-GLES2.a.*\n# image B2: y\ndEQP-GLES2.b.*\n' > "$W/fake.txt"
export T6_DEQP_BUILDER=$W/fake-builder.sh
HEAD_SHA=$(git -C "$HERE/../.." rev-parse HEAD)

# --dry-run stays the builder's output byte for byte (images-selftest.sh 7f
# compares it to the table): no "would write" line, nothing on disk.
out=$("$S" --dry-run "$W/fake.txt" "$W/img3" A1 B2 2>&1); rc=$?
if [ $rc = 0 ] && [ "$out" = "$(printf 'A1 dEQP-GLES2.x.*\nB2 dEQP-GLES2.x.*')" ] &&
	[ ! -e "$W/img3" ]; then
	ok "--dry-run: the builder's lines exactly, no news of SHA256SUMS, nothing written"
else bad "--dry-run with a builder seam: rc $rc: $out"; fi

out=$("$S" "$W/fake.txt" "$W/img3" A1 B2 2>"$W/err"); rc=$?
want=$(cd "$W/img3" && sha256sum A1/zephyr/zephyr.bin B2/zephyr/zephyr.bin)
if [ $rc = 0 ] && [ "$(cat "$W/img3/SHA256SUMS")" = "$want" ] &&
	[ "$(cat "$W/img3/COMMIT")" = "$HEAD_SHA" ] && grep -qxE '[0-9a-f]{40}' "$W/img3/COMMIT"; then
	ok "a build writes SHA256SUMS (sha256sum format, both labels) and COMMIT (the full HEAD sha)"
else bad "sums: rc $rc: $(cat "$W/img3/SHA256SUMS" 2>&1) $(cat "$W/err")"; fi

# rewritten, never merged: a selective build lists only its own labels
echo "deadbeef  OLD/zephyr/zephyr.bin" > "$W/img3/SHA256SUMS"
echo 0123456789abcdef0123456789abcdef01234567 > "$W/img3/COMMIT"
out=$("$S" "$W/fake.txt" "$W/img3" B2 2>"$W/err"); rc=$?
if [ $rc = 0 ] && [ "$(cat "$W/img3/SHA256SUMS")" = "$(cd "$W/img3" && sha256sum B2/zephyr/zephyr.bin)" ] &&
	[ "$(cat "$W/img3/COMMIT")" = "$HEAD_SHA" ]; then
	ok "SHA256SUMS and COMMIT are rewritten, not merged: a selective build lists only its own labels"
else bad "rewrite: rc $rc: $(cat "$W/img3/SHA256SUMS") $(cat "$W/err")"; fi

# a row whose build failed (FAILROW) or whose image is empty (EMPTYROW) is
# left out and named; a real re-run would also find its .built rc=0 gone
out=$("$S" "$W/fake.txt" "$W/img4" A1 FAILROW EMPTYROW 2>"$W/err"); rc=$?
if [ $rc = 0 ] && [ "$(cat "$W/img4/SHA256SUMS")" = "$(cd "$W/img4" && sha256sum A1/zephyr/zephyr.bin)" ] &&
	grep -q "FAILROW is not listed" "$W/err" && grep -q "EMPTYROW is not listed" "$W/err"; then
	ok "a failed build and an empty image are left out of SHA256SUMS and named"
else bad "failed rows: rc $rc: $(cat "$W/img4/SHA256SUMS" 2>&1) $(cat "$W/err")"; fi

# a builder that refuses: its exit code, nothing written
out=$(FAKE_BUILDER_RC=3 "$S" "$W/fake.txt" "$W/img5" A1 2>"$W/err"); rc=$?
if [ $rc = 3 ] && [ ! -e "$W/img5" ]; then
	ok "a refused build (rc 3): the code stands, no SHA256SUMS, no COMMIT"
else bad "refused build: rc $rc: $(cat "$W/err")"; fi

# preflight.sh reads exactly this layout: preflight-selftest.sh proves the
# checker; this is the contract between the two scripts. The build commit
# must be reachable from origin/main of the repository preflight runs
# against, so a throw-away repository lends its HEAD as the COMMIT (the
# script's own COMMIT line was checked against the real HEAD above).
R=$W/prepo
git init -q "$R"
echo 'records/*' > "$R/.gitignore"
git -C "$R" -c user.name=t -c user.email=t@t -c commit.gpgsign=false commit -q --allow-empty -m one
git -C "$R" update-ref refs/remotes/origin/main HEAD
"$S" "$W/fake.txt" "$W/img6" A1 B2 > /dev/null 2>&1
git -C "$R" rev-parse HEAD > "$W/img6/COMMIT"
pf=$(env -i PATH="$PATH" HOME="$W" PREFLIGHT_REPO="$R" T6_UART=x T6_HAND=x \
	bash "$HERE/../../driver/tools/uboot-t6/preflight.sh" "$W/img6" 2>&1)
if grep -q "^ok   c: A1/zephyr/zephyr.bin exists, sha256 matches" <<<"$pf" &&
	grep -q "^ok   c: B2/zephyr/zephyr.bin exists, sha256 matches" <<<"$pf" &&
	grep -q "^ok   c: build commit" <<<"$pf" && ! grep -q '^FAIL c:' <<<"$pf"; then
	ok "preflight.sh accepts the SHA256SUMS and (a reachable) COMMIT of a build"
else bad "preflight: $pf"; fi
unset T6_DEQP_BUILDER

[ $fail = 0 ] && echo "t6-deqp-images-selftest: all ok"
exit $fail
