#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx8mp-env.sh (refuses with the variable's name, succeeds on stub directories), of
# imx8mp-deqp-stage1.sh (DEQP_MODULE/gles3 hand-off to cts-build-zephyr.sh; gles2 stays the default) and of
# imx8mp-gles3-artefacts.sh (one line per artefact with size and sha256, non-zero when one is missing, the
# .config keys a GLES3 image sets). The scripts run from copies under a scratch ROOT whose cts-build-zephyr.sh,
# zephyr-modules.sh and west are stubs. Host only; no west build, no board. Writes under ${TMPDIR:-$PWD}.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
case "${TMPDIR:-$PWD}" in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "refusing a scratch dir under /tmp; set TMPDIR" >&2; exit 2 ;; esac
tmp=$(mktemp -d "${TMPDIR:-$PWD}/gles3-env-selftest.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
nfail=0 nok=0
ok() { echo "ok: $*"; nok=$((nok + 1)); }
bad() { echo "FAIL: $*" >&2; nfail=$((nfail + 1)); }

root=$tmp/root
mkdir -p "$root/tools/cts" "$tmp/ws/zephyr" "$tmp/venv/bin" "$tmp/sdk"
cp "$here/imx8mp-env.sh" "$here/imx8mp-deqp-stage1.sh" "$here/imx8mp-gles3-artefacts.sh" "$root/tools/cts/"
echo ': # stub activate' > "$tmp/venv/bin/activate"
cat > "$root/tools/cts/zephyr-modules.sh" <<'STUB'
#!/bin/bash
echo "stub-modules:$1"
STUB
cat > "$root/tools/cts/cts-build-zephyr.sh" <<'STUB'
echo "MODULE=${MODULE-<unset>}"
echo "WORK=${WORK-<unset>}"
echo "BOARD=${BOARD-<unset>}"
echo "ZEPHYR_SDK=${ZEPHYR_SDK-<unset>}"
exit "${STUB_RC:-0}"
STUB
chmod +x "$root/tools/cts/"*.sh
# the env file is sourced by a throwaway script, since it exits on failure
cat > "$root/tools/cts/src-env.sh" <<'STUB'
. "$(dirname "$0")/imx8mp-env.sh"
imx8mp_zephyr_env
echo "WS=$ZEPHYR_WS BASE=$ZEPHYR_BASE MODULES=$ZEPHYR_MODULES"
STUB

genv() { # run "$@" with the good environment
	env -u ZEPHYR_WORKSPACE -u ZEPHYR_BASE -u DEQP_MODULE -u DEQP_STAGE1 -u DEQP_STAGE1_GLES3 -u CLUSTER_DIR \
		ZEPHYR_WS="$tmp/ws" ZEPHYR_VENV="$tmp/venv" ZEPHYR_SDK_INSTALL_DIR="$tmp/sdk" IMX8MP_OUT="$tmp/out" "$@"
}

# ---- (a) imx8mp-env.sh
e=$root/tools/cts/src-env.sh
genv bash "$e" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -q "WS=$tmp/ws BASE=$tmp/ws/zephyr MODULES=stub-modules:$tmp/ws" "$tmp/o" && [ -d "$tmp/out" ]; then
	ok "env: stub directories succeed, ZEPHYR_BASE follows the workspace, IMX8MP_OUT created"
else bad "env: stub directories: exit $rc, out: $(cat "$tmp/o") err: $(cat "$tmp/e")"; fi
for v in ZEPHYR_WS ZEPHYR_VENV ZEPHYR_SDK_INSTALL_DIR IMX8MP_OUT; do
	genv env -u "$v" bash "$e" > "$tmp/o" 2> "$tmp/e"; rc=$?
	if [ $rc -eq 2 ] && grep -q "$v is not set" "$tmp/e"; then ok "env: $v unset: exit 2 naming it"
	else bad "env: $v unset: exit $rc, stderr: $(cat "$tmp/e")"; fi
done
genv env ZEPHYR_WS="" bash "$e" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 2 ] && grep -q "ZEPHYR_WS is not set" "$tmp/e"; then ok "env: ZEPHYR_WS empty: exit 2"
else bad "env: ZEPHYR_WS empty: exit $rc, stderr: $(cat "$tmp/e")"; fi
genv env -u ZEPHYR_WS ZEPHYR_WORKSPACE="$tmp/ws" bash "$e" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ]; then ok "env: old name ZEPHYR_WORKSPACE still read"; else bad "env: ZEPHYR_WORKSPACE: exit $rc, stderr: $(cat "$tmp/e")"; fi
genv env ZEPHYR_VENV="$tmp/novenv" bash "$e" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 2 ] && grep -q "novenv/bin/activate not found" "$tmp/e"; then ok "env: missing venv: exit 2 naming its activate"
else bad "env: missing venv: exit $rc, stderr: $(cat "$tmp/e")"; fi
genv env ZEPHYR_WS="$tmp/nows" bash "$e" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 2 ] && grep -q "nows/zephyr not found" "$tmp/e" && grep -q ZEPHYR_WS "$tmp/e"; then ok "env: workspace without zephyr/: exit 2 naming ZEPHYR_WS"
else bad "env: workspace without zephyr/: exit $rc, stderr: $(cat "$tmp/e")"; fi
genv env ZEPHYR_BASE="$tmp/other" bash "$e" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -q "BASE=$tmp/ws/zephyr" "$tmp/o" && grep -q 'preset ZEPHYR_BASE' "$tmp/e"; then ok "env: foreign ZEPHYR_BASE replaced with a line"
else bad "env: foreign ZEPHYR_BASE: exit $rc, out: $(cat "$tmp/o") err: $(cat "$tmp/e")"; fi

# ---- (b) imx8mp-deqp-stage1.sh
s1=$root/tools/cts/imx8mp-deqp-stage1.sh
lg=$tmp/out/deqp-stage1.log
genv env DEQP_STAGE1="$tmp/s1" "$s1" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -q '^rc=0' "$tmp/o" && grep -q "^MODULE=<unset>" "$lg" \
	&& grep -q "^WORK=$tmp/s1\$" "$lg" && grep -q '^BOARD=verdin_imx8mp/mimx8ml8/a53$' "$lg" \
	&& grep -q "^ZEPHYR_SDK=$tmp/sdk\$" "$lg"; then
	ok "stage1: default stays gles2 (no MODULE, WORK=DEQP_STAGE1, board and SDK handed over)"
else bad "stage1: default: exit $rc, log: $(cat "$lg" 2>&1) err: $(cat "$tmp/e")"; fi
genv env DEQP_MODULE=gles3 DEQP_STAGE1="$tmp/s1g" "$s1" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -q '^MODULE=gles3$' "$lg" && grep -q "^WORK=$tmp/s1g\$" "$lg"; then
	ok "stage1: DEQP_MODULE=gles3 passes MODULE=gles3 and WORK=DEQP_STAGE1"
else bad "stage1: DEQP_MODULE=gles3: exit $rc, log: $(cat "$lg")"; fi
genv "$s1" gles3 > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 0 ] && grep -q '^MODULE=gles3$' "$lg" && grep -q "^WORK=$tmp/out/deqp-stage1-gles3\$" "$lg"; then
	ok "stage1: 'gles3' argument: MODULE=gles3, WORK=\$IMX8MP_OUT/deqp-stage1-gles3 (the recipe's directory)"
else bad "stage1: gles3 argument: exit $rc, log: $(cat "$lg") err: $(cat "$tmp/e")"; fi
genv env DEQP_STAGE1_GLES3="$tmp/alt" "$s1" gles3 > "$tmp/o" 2> "$tmp/e"
if grep -q "^WORK=$tmp/alt\$" "$lg"; then ok "stage1: gles3 argument honours DEQP_STAGE1_GLES3"; else bad "stage1: DEQP_STAGE1_GLES3 not honoured"; fi
genv env DEQP_MODULE=gles4 DEQP_STAGE1="$tmp/s1" "$s1" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 2 ] && grep -q 'DEQP_MODULE must be gles2 or gles3' "$tmp/e"; then ok "stage1: unknown DEQP_MODULE: exit 2"
else bad "stage1: unknown module: exit $rc, stderr: $(cat "$tmp/e")"; fi
genv "$s1" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 2 ] && grep -q 'DEQP_STAGE1 is not set' "$tmp/e"; then ok "stage1: no DEQP_STAGE1 (gles2): exit 2 naming it"
else bad "stage1: no DEQP_STAGE1: exit $rc, stderr: $(cat "$tmp/e")"; fi
genv env STUB_RC=3 DEQP_STAGE1="$tmp/s1" "$s1" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 3 ] && grep -q '^rc=3' "$tmp/o"; then ok "stage1: build failure rc passed through"
else bad "stage1: failing build: exit $rc, out: $(cat "$tmp/o")"; fi
env -u IMX8MP_OUT -u ZEPHYR_BASE ZEPHYR_WS="$tmp/ws" ZEPHYR_VENV="$tmp/venv" ZEPHYR_SDK_INSTALL_DIR="$tmp/sdk" DEQP_STAGE1="$tmp/s1" "$s1" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 2 ] && grep -q 'IMX8MP_OUT is not set' "$tmp/e"; then ok "stage1: no IMX8MP_OUT: exit 2 naming it"
else bad "stage1: no IMX8MP_OUT: exit $rc, stderr: $(cat "$tmp/e")"; fi

# ---- (c) imx8mp-gles3-artefacts.sh
a=$root/tools/cts/imx8mp-gles3-artefacts.sh
ao=$tmp/aout img=es3-deadbeef
s=$ao/deqp-stage1-gles3/build/modules/gles3
mkdir -p "$ao/$img/zephyr" "$s/functional"
printf 'bin-bytes' > "$ao/$img/zephyr/zephyr.bin"
printf 'elf-bytes-longer' > "$ao/$img/zephyr/zephyr.elf"
printf 'table' > "$ao/$img.table.conf"
printf 'package' > "$s/libdeqp-gles3-package.a"
printf 'functional-lib' > "$s/functional/libdeqp-gles3-functional.a"
printf '# CONFIG_DEQP_REPORT_ES2 is not set\n# CONFIG_YAKOGL_REPORT_ES2 is not set\n' > "$ao/$img/zephyr/.config"
printf 'Memory region Used\nRAM: 1 B\nnoise\n' > "$ao/$img.log"
files=("$ao/$img/zephyr/zephyr.bin" "$ao/$img/zephyr/zephyr.elf" "$ao/$img.table.conf" "$s/libdeqp-gles3-package.a" "$s/functional/libdeqp-gles3-functional.a")
env -u DEQP_STAGE1_GLES3 IMX8MP_OUT="$ao" "$a" $img > "$tmp/o" 2> "$tmp/e"; rc=$?
good=1
for f in "${files[@]}"; do
	want="$(stat -c %s "$f") $(sha256sum "$f" | cut -d' ' -f1) $f"
	grep -qxF "$want" "$tmp/o" || { good=0; bad "artefacts: no line '$want'"; }
done
if [ $rc -eq 0 ] && [ "$(grep -cE '^[0-9]+ [0-9a-f]{64} ' "$tmp/o")" -eq 5 ] && [ $good -eq 1 ]; then
	ok "artefacts: five lines with the right size, sha256 and path"
else bad "artefacts: exit $rc, out: $(cat "$tmp/o")"; fi
if grep -qx '# CONFIG_DEQP_REPORT_ES2 is not set' "$tmp/o" && grep -qx '# CONFIG_YAKOGL_REPORT_ES2 is not set' "$tmp/o"; then
	ok "artefacts: .config keys of a GLES3 image (both REPORT_ES2 not set) printed"
else bad "artefacts: .config keys: $(cat "$tmp/o")"; fi
if grep -qx 'RAM: 1 B' "$tmp/o" && grep -qx 'Memory region Used' "$tmp/o" && ! grep -q noise "$tmp/o"; then
	ok "artefacts: memory lines of the build log printed"
else bad "artefacts: memory lines"; fi
printf 'CONFIG_DEQP_REPORT_ES2=y\nCONFIG_YAKOGL_REPORT_ES2=y\n' > "$ao/$img/zephyr/.config"
IMX8MP_OUT="$ao" "$a" $img > "$tmp/o" 2>&1
if grep -qx 'CONFIG_DEQP_REPORT_ES2=y' "$tmp/o"; then ok "artefacts: an ES 2.0 image shows =y"; else bad "artefacts: =y not shown"; fi
: > "$ao/$img/zephyr/.config"
IMX8MP_OUT="$ao" "$a" $img > "$tmp/o" 2>&1
if grep -q 'CONFIG_YAKOGL_REPORT_ES2: not in .config' "$tmp/o"; then ok "artefacts: absent key reported"; else bad "artefacts: absent key not reported"; fi
for f in "${files[@]}"; do
	mv "$f" "$f.hide"
	IMX8MP_OUT="$ao" "$a" $img > "$tmp/o" 2> "$tmp/e"; rc=$?
	if [ $rc -ne 0 ] && grep -qF "$f" "$tmp/e"; then ok "artefacts: missing ${f#"$ao"/}: exit $rc naming it"
	else bad "artefacts: missing ${f#"$ao"/}: exit $rc, stderr: $(cat "$tmp/e")"; fi
	mv "$f.hide" "$f"
done
IMX8MP_OUT="$ao" "$a" > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -eq 2 ]; then ok "artefacts: no argument: exit 2"; else bad "artefacts: no argument: exit $rc"; fi
env -u IMX8MP_OUT "$a" $img > "$tmp/o" 2> "$tmp/e"; rc=$?
if [ $rc -ne 0 ] && grep -q IMX8MP_OUT "$tmp/e"; then ok "artefacts: no IMX8MP_OUT: exit $rc naming it"; else bad "artefacts: no IMX8MP_OUT: exit $rc"; fi

echo "$nok ok, $nfail failed"
[ $nfail -eq 0 ]
