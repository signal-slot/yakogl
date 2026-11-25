#!/bin/sh
# Selftest of imx95-g310-cause.py: no board, no network.
set -u
here=$(dirname "$0")
tool="$here/imx95-g310-cause.py"
tab=$(printf '\t')
fail=0
run() { python3 -I "$tool" "$@"; }
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }
# expect <case> <t6> <imx> <cause>
expect() {
	got=$(printf '%s\t%s\t%s\n' "$1" "$2" "$3" | run | cut -f4)
	if [ "$got" = "$4" ]; then ok "$4: $1 $2 $3"; else bad "$1 $2 $3: want $4 got $got"; fi
}
expect dEQP-GLES2.functional.shaders.x Pass Pass none
expect dEQP-GLES2.functional.shaders.x Fail Fail compiler-refusal
expect dEQP-GLES2.functional.shaders.x NotSupported NotSupported compiler-refusal
expect dEQP-GLES2.functional.fbo.multisample.a Fail Fail compiler-refusal
expect dEQP-GLES2.info.renderer Pass Fail renderer-name
expect dEQP-GLES2.functional.fbo.multisample.a Pass Fail tib-budget
expect dEQP-GLES2.functional.fbo.completeness.rgba32f Pass Fail tib-budget
expect dEQP-GLES2.functional.fbo.render.resize.rbo_rgba Pass Fail tiler-levels
expect dEQP-GLES2.functional.size.2048x2048 Pass Fail tiler-levels
expect dEQP-GLES2.functional.size.12048 Pass Fail unexplained
expect dEQP-GLES2.performance.draw Pass Fail shader-present
expect dEQP-GLES2.functional.other Pass Timeout hang
expect dEQP-GLES2.functional.other Pass Crash hang
expect dEQP-GLES2.functional.other Pass Missing hang
expect dEQP-GLES2.functional.other Timeout Timeout unexplained
expect dEQP-GLES2.functional.other Pass Fail unexplained
expect dEQP-GLES2.functional.other Fail Pass unexplained
# comments and blank lines pass through, summary counts
out=$(printf '# note\ndEQP-GLES2.a\tPass\tPass\n\ndEQP-GLES2.b\tFail\tFail\ndEQP-GLES2.info.c\tPass\tFail\n' | run --summary)
echo "$out" | grep -q '^# note$' && ok "comment kept" || bad "comment"
echo "$out" | grep -q "^# compiler-refusal${tab}1\$" && ok "summary refusal" || bad "summary refusal"
echo "$out" | grep -q "^# renderer-name${tab}1\$" && ok "summary renderer" || bad "summary renderer"
echo "$out" | grep -q "^# total${tab}3\$" && ok "summary total" || bad "summary total"
# file argument
f=$(mktemp) || exit 1
printf 'dEQP-GLES2.b\tFail\tFail\n' >"$f"
[ "$(run "$f" | cut -f4)" = compiler-refusal ] && ok "file input" || bad "file input"
rm -f "$f"
# malformed: exit 2 with a message
for l in 'onlyone' "a${tab}Pass" "a${tab}Pass${tab}Pass${tab}x" "a${tab}Pass${tab}Bogus"; do
	msg=$(printf '%s\n' "$l" | run 2>&1 >/dev/null)
	rc=$?
	if [ $rc -eq 2 ] && [ -n "$msg" ]; then ok "malformed rc=2"; else bad "malformed '$l' rc=$rc"; fi
done
# empty input
out=$(printf '' | run --summary)
rc=$?
if [ $rc -eq 0 ] && echo "$out" | grep -q "^# total${tab}0\$"; then ok "empty summary"; else bad "empty summary"; fi
out=$(printf '' | run)
rc=$?
if [ $rc -eq 0 ] && [ -z "$out" ]; then ok "empty plain"; else bad "empty plain"; fi
run --help >/dev/null && ok "help" || bad "help"
if [ $fail -eq 0 ]; then echo "imx95-g310-cause selftest: pass"; else echo "imx95-g310-cause selftest: FAIL"; fi
exit $fail
