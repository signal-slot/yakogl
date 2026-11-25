#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx95-g310-cause.py: each rule, the rule order, the header and
# comment lines, --summary, malformed lines (exit 2 with a message) and an
# empty input. No board, no network; POSIX sh (also run by bash in the gate).
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/imx95-g310-cause.py"
t=$(mktemp -d "${TMPDIR:-.}/g310-cause.XXXXXX") || exit 2
trap 'rm -rf "$t"' EXIT
fail=0
ok() { echo "ok: $1"; }
bad() { echo "FAIL: $1"; fail=1; }
TAB=$(printf '\t')

python3 -I "$tool" --help > "$t/help" 2>&1 && grep -q 'candidate' "$t/help" \
	&& ok "--help" || bad "--help"

# case, t6, imx95, the cause expected
cat > "$t/want" <<'EOF'
dEQP-GLES2.functional.color_clear.single_rgb	Pass	Pass	no-difference
dEQP-GLES2.functional.shaders.loops.for_constant_iterations.basic_lowp_int_vertex	Fail	Fail	compiler-refusal
dEQP-GLES2.functional.shaders.misc.x	NotSupported	NotSupported	compiler-refusal
dEQP-GLES2.info.renderer	Pass	Fail	renderer-name
dEQP-GLES2.info.vendor	Fail	Timeout	renderer-name
dEQP-GLES2.functional.multisample.common_edge_big_quad	Pass	Fail	tib-budget
dEQP-GLES2.functional.multisampled_render_to_texture.readpixels	Pass	QualityWarning	tib-budget
dEQP-GLES2.functional.fbo.render.color.mix_tex2d_rgba	Pass	Fail	tib-budget
dEQP-GLES2.functional.fbo.render.recreate_colorbuffer.x	Pass	Timeout	tib-budget
dEQP-GLES2.functional.texture.format.rgba32f_2d	Pass	Fail	tib-budget
dEQP-GLES2.functional.texture.format.rgb32f_cube	Fail	Pass	tib-budget
dEQP-GLES2.functional.fbo.render.resize.rbo_rgb565	Pass	Fail	tiler-levels
dEQP-GLES2.functional.texture.size.2d.2048x2048_rgba8888	Pass	Fail	tiler-levels
dEQP-GLES2.functional.texture.size.cube.1x4096_l8	Pass	Fail	tiler-levels
dEQP-GLES2.functional.fbo.completeness.size.4096	Pass	Fail	tiler-levels
dEQP-GLES2.functional.texture.size.2d.1024x1024_rgba8888	Pass	Fail	unexplained
dEQP-GLES2.performance.shaders.operator.add	Pass	Fail	shader-present
dEQP-GLES2.functional.draw.timing.x	Pass	Fail	shader-present
dEQP-GLES2.functional.attribute_location.bind_time.relink	Pass	Timeout	hang
dEQP-GLES2.functional.fragment_ops.blend.x	Pass	Crash	hang
dEQP-GLES2.functional.fragment_ops.depth.x	Fail	Missing	hang
dEQP-GLES2.functional.fragment_ops.stencil.x	Timeout	Missing	unexplained
dEQP-GLES2.functional.fbo.completeness.renderable.texture.color0.rgba4	Pass	Fail	unexplained
dEQP-GLES2.functional.fbo.api.x	NotSupported	Pass	unexplained
dEQP-GLES2.functional.texture.format.rgba16f_2d	CompatibilityWarning	InternalError	unexplained
EOF
cut -f1-3 "$t/want" > "$t/in"
if python3 -I "$tool" "$t/in" > "$t/got" 2> "$t/err" && cmp -s "$t/want" "$t/got"; then
	ok "every rule and the order (from a file)"
else
	bad "every rule and the order"; diff "$t/want" "$t/got"; cat "$t/err"
fi
if python3 -I "$tool" < "$t/in" > "$t/got2" 2>&1 && cmp -s "$t/want" "$t/got2"; then
	ok "stdin gives the same"
else
	bad "stdin"
fi
if python3 -I "$tool" - < "$t/in" > "$t/got3" 2>&1 && cmp -s "$t/want" "$t/got3"; then
	ok "'-' is stdin"
else
	bad "'-' is stdin"
fi

# Header, comment and empty lines pass through; CRLF is accepted.
printf 'case\tt6_result\timx95_result\n# a comment\n\ndEQP-GLES2.info.vendor\tPass\tFail\r\n' > "$t/hdr"
printf 'case\tt6_result\timx95_result\tcause\n# a comment\n\ndEQP-GLES2.info.vendor\tPass\tFail\trenderer-name\n' > "$t/hdr.want"
if python3 -I "$tool" "$t/hdr" > "$t/hdr.got" 2>&1 && cmp -s "$t/hdr.want" "$t/hdr.got"; then
	ok "header, comments, empty line, CRLF"
else
	bad "header, comments, empty line, CRLF"; diff "$t/hdr.want" "$t/hdr.got"
fi

# --summary: the lines, then one '#' row per cause in rule order, then the total.
cat > "$t/sum.want" <<EOF
# cause${TAB}count
# no-difference${TAB}1
# compiler-refusal${TAB}2
# renderer-name${TAB}2
# tib-budget${TAB}6
# tiler-levels${TAB}4
# shader-present${TAB}2
# hang${TAB}3
# unexplained${TAB}5
# total${TAB}25
EOF
if python3 -I "$tool" --summary "$t/in" > "$t/sum.got" 2>&1 \
	&& head -n 25 "$t/sum.got" | cmp -s "$t/want" - \
	&& tail -n +26 "$t/sum.got" | cmp -s "$t/sum.want" -; then
	ok "--summary"
else
	bad "--summary"; cat "$t/sum.got"
fi
# The summarised output reads again as input (the table is '#' lines) only
# after the cause column is cut; a fourth column is malformed.
if python3 -I "$tool" "$t/sum.got" > /dev/null 2>&1; then
	bad "a four-column line is accepted"
else
	ok "a four-column line is refused"
fi

# Malformed lines: exit 2, a message naming the line, nothing on stdout.
malformed() { # name, content, message fragment
	printf '%b' "$2" > "$t/bad"
	python3 -I "$tool" "$t/bad" > "$t/bad.out" 2> "$t/bad.err"
	rc=$?
	if [ "$rc" -eq 2 ] && [ ! -s "$t/bad.out" ] && grep -q -- "$3" "$t/bad.err"; then
		ok "malformed: $1"
	else
		bad "malformed: $1 (rc $rc)"; cat "$t/bad.err"
	fi
}
malformed "two fields" 'dEQP-GLES2.info.vendor\tPass\n' 'line 1: 2 tab-separated fields'
malformed "four fields" '# c\ndEQP-GLES2.info.vendor\tPass\tFail\tx\n' 'line 2: 4 tab-separated fields'
malformed "space-separated" 'dEQP-GLES2.info.vendor Pass Fail\n' 'line 1: 1 tab-separated fields'
malformed "unknown t6 result" 'a.b\tPassed\tFail\n' "line 1: t6_result 'Passed'"
malformed "unknown imx95 result" 'a.b\tPass\tpass\n' "line 1: imx95_result 'pass'"
malformed "empty case" '\tPass\tFail\n' 'line 1: empty case name'
malformed "bad line after good ones" 'a.b\tPass\tFail\na.c\tPass\n' 'line 2:'

# A missing file and an unknown option: exit 2.
python3 -I "$tool" "$t/no-such-file" > /dev/null 2>&1
[ $? -eq 2 ] && ok "missing file exits 2" || bad "missing file"
python3 -I "$tool" --no-such-option < /dev/null > /dev/null 2>&1
[ $? -eq 2 ] && ok "unknown option exits 2" || bad "unknown option"

# Empty input: exit 0, no output; --summary gives the zero table.
if python3 -I "$tool" < /dev/null > "$t/empty.got" 2>&1 && [ ! -s "$t/empty.got" ]; then
	ok "empty input"
else
	bad "empty input"
fi
if python3 -I "$tool" --summary < /dev/null > "$t/empty.sum" 2>&1 \
	&& [ "$(grep -c "${TAB}0\$" "$t/empty.sum")" -eq 9 ] \
	&& [ "$(wc -l < "$t/empty.sum")" -eq 10 ]; then
	ok "empty input --summary"
else
	bad "empty input --summary"; cat "$t/empty.sum"
fi

[ "$fail" -eq 0 ] && echo "imx95-g310-cause-selftest: all ok" || echo "imx95-g310-cause-selftest: FAILED"
exit "$fail"
