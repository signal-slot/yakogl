#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host selftest of tools/mp2_dt_overlap_check.py; fixtures are written at run
# time. No board, python3 standard library only.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
chk="$here/mp2_dt_overlap_check.py"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/mp2dt.XXXXXX")" || exit 2
trap 'rm -rf "$tmp"' EXIT
ok=0 failed=0

run() { out="$(python3 -I "$chk" "$@" 2>"$tmp/err")"; rc=$?; }
check() { # name, condition result
    if [ "$2" = 1 ]; then ok=$((ok + 1)); echo "ok: $1"
    else failed=$((failed + 1)); echo "FAIL: $1 (rc=$rc out=[$out] err=[$(cat "$tmp/err")])"; fi
}

cat >"$tmp/base.dts" <<'EOF'
/dts-v1/;
/ {
	ddr0: memory@84000000 {
		device_type = "memory";
		reg = <0x84000000 0x76800000>;
		#address-cells = <1>;
		#size-cells = <1>;
		sec: memory@a0000000 { reg = <0xa0000000 DT_SIZE_M(256)>; };
	};
	reserved-memory {
		#address-cells = <1>;
		#size-cells = <1>;
		ranges;
		gpu_reserved: gpu@fa800000 { reg = <0xfa800000 DT_SIZE_M(64)>; no-map; };
	};
};
EOF
cat >"$tmp/fb.overlay" <<'EOF'
/ { reserved-memory { ltdc_fb: framebuffer@b0000000 {
	reg = <0xb0000000 DT_SIZE_M(32)>; no-map; }; }; };
EOF
cat >"$tmp/carve_dup.overlay" <<'EOF'
/ { reserved-memory { gpu_carveout_glmark2: gpu-carveout@b0000000 {
	reg = <0xb0000000 0x0f000000>; no-map; }; }; };
EOF
cat >"$tmp/carve_adj.overlay" <<'EOF'
/ { reserved-memory { gpu_carveout_glmark2: gpu-carveout@b2000000 {
	reg = <0xb2000000 0x0e000000>; no-map; }; }; };
EOF
cat >"$tmp/cells2.dts" <<'EOF'
/ {
	#address-cells = <2>;
	#size-cells = <2>;
	reserved-memory {
		#address-cells = <2>;
		#size-cells = <2>;
		a: a@100000000 { reg = <0x1 0x00000000 0x0 0x20000000>; };
		b: b@110000000 { reg = <0x1 0x10000000 0x0 0x20000000>; };
		c: c@120000000 { reg = <0x1 0x20000000 0x0 0x10000000>; };
	};
};
EOF

run "$tmp/base.dts" "$tmp/carve_adj.overlay"
check "no overlap: exit 0, nothing printed" $([ $rc = 0 ] && [ -z "$out" ] && echo 1 || echo 0)

run "$tmp/base.dts" "$tmp/fb.overlay" "$tmp/carve_dup.overlay"
check "0xb0000000 double claim: exit 1" $([ $rc = 1 ] && echo 1 || echo 0)
check "double claim line names both and the range" \
    $([ "$out" = "overlap: ltdc_fb gpu_carveout_glmark2 0xb0000000-0xb1ffffff" ] && echo 1 || echo 0)

run "$tmp/base.dts" "$tmp/fb.overlay" "$tmp/carve_adj.overlay"
check "adjacent ranges (end 0xb2000000, next starts there): exit 0" \
    $([ $rc = 0 ] && [ -z "$out" ] && echo 1 || echo 0)

run "$tmp/cells2.dts"
check "cells 2/2: a-b and b-c overlap, a-c do not" \
    $([ $rc = 1 ] && [ "$out" = "$(printf 'overlap: a b 0x110000000-0x11fffffff\noverlap: b c 0x120000000-0x12fffffff')" ] && echo 1 || echo 0)

run --range go=0xa0000000:0x10000000 "$tmp/base.dts" "$tmp/carve_adj.overlay"
check "go window clear of reserved memory: exit 0" $([ $rc = 0 ] && echo 1 || echo 0)

run --range go=0xa0000000:0x10000001 "$tmp/base.dts" "$tmp/fb.overlay"
check "go window one byte into ltdc_fb: exit 1, one byte" \
    $([ $rc = 1 ] && [ "$out" = "overlap: ltdc_fb go 0xb0000000-0xb0000000" ] && echo 1 || echo 0)

run --range sec=0xaf000000:0x1000 --range go=0xa0000000:0x10000000 "$tmp/base.dts"
check "two extra ranges overlap each other: exit 1" $([ $rc = 1 ] && echo 1 || echo 0)

run --range bad "$tmp/base.dts"
check "bad --range: exit 2" $([ $rc = 2 ] && echo 1 || echo 0)

run "$tmp/none.dts"
check "missing file: exit 2" $([ $rc = 2 ] && echo 1 || echo 0)

echo "$ok ok, $failed failed"
[ "$failed" = 0 ]
