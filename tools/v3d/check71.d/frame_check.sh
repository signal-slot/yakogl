#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# frame_check: the driver's frame builder at V3D 7.1 (driver/src/v3d/v3d_frame.c
# built with -DV3D_CL_VER=71), its clear-only frame, four ways:
#  (a) tools/v3d/frame_check.c built at 7.1: Mesa's CLE decoder (clif_dump, a
#      version 71 device) reads the three lists in full, with the 7.1 census,
#      no packet outside it and no byte left over, at the sizes of the 4.2 run
#      (docs/notes/pi4-v3d-bringup.md: 1x1, 64x64, 200x100, 65x3000, 1920x1080,
#      4096x4096); the 64x64 frame is 19 / 96 / 27 bytes.
#  (b) the 64x64 clear to 0xffff0000 equals Mesa's own drm-shim dump of that
#      clear (tools/v3d/shim71/clear.clif) through tools/v3d/clif_cmp.py, with
#      the store's format fields of the target ignored (FORMAT_FIELDS below)
#      and nothing else: frame_check puts the buffers at the dump's addresses.
#  (c) the lists equal, byte for byte, the ones the Pi 5 probe writes by hand
#      (tools/probes/rpi_5/rpi5_clear/src/frame71.c, checked against Mesa's
#      packers by clear71.sh) for the same frames, and the same frames are
#      refused.
#  (d) each board clear dump of D3 (tools/v3d/pi5/*clear*.clif, Mesa's
#      V3D_DEBUG=clif of a GL clear under the Pi 5's Linux) equals our frame
#      for its size and clear colour, with the address fields
#      (ADDRESS_FIELDS) and FORMAT_FIELDS ignored; any other difference
#      fails. While there is no such dump, (d) is one line naming it and the
#      summary says so. That is not a gap: a gap names an input the host can
#      be given (MESA_V3D_TREE), and the gate is green only with none, so a
#      board record not made yet would hold every pull request red.
# Reference: tools/v3d/mesa_ref.sh (build/v3d-check/mesa), or MESA_V3D_TREE with
# build-v3d as the v3d rows of tools/check.sh. FRAME_CHECK_PI5_DIR replaces
# tools/v3d/pi5 (for trying (d) by hand). Convention:
# tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=frame_check
dir=build/v3d71/$name
mkdir -p "$dir/tmp" || exit 1

tree=${MESA_V3D_TREE:-}
mt=third_party/mesa-26.2.1
ref=build/v3d-check/mesa
probe=tools/probes/rpi_5/rpi5_clear/src
pi5=${FRAME_CHECK_PI5_DIR:-tools/v3d/pi5}
inc=(-D_GNU_SOURCE -DHAVE_ENDIAN_H)
if [ -z "$tree" ]; then
  tools/v3d/mesa_ref.sh >"$dir/mesa_ref.log" 2>&1 || {
    tail -20 "$dir/mesa_ref.log"
    exit 1
  }
  inc+=(-I"$mt/src" -I"$mt/include" -I"$mt/src/broadcom" -I"$ref/gen")
  libs=("$ref/libmesa_ref.a" -lexpat -lz -lm -lpthread)
elif [ -d "$tree/build-v3d" ]; then
  mb=$tree/build-v3d
  inc+=(-I"$tree/src" -I"$tree/include" -I"$tree/src/broadcom" -I"$mb/src/broadcom" -I"$mb/src")
  libs=("$mb/src/broadcom/libbroadcom_v3d.a" "$mb/src/broadcom/cle/libbroadcom_cle.a"
    "$mb/src/broadcom/libv3d_neon.a" "$mb/src/util/libmesa_util.a" -lexpat -lz -lzstd -lm -lpthread)
else
  echo "MESA_V3D_TREE='$tree' has no build-v3d (unset it for the vendored reference build)"
  echo "gap: MESA_V3D_TREE"
  exit 77
fi
inc+=(-Idriver/src/v3d -Idriver/src/v3d/generated)

# The fields left out of the comparisons, by name (clif_cmp.py --ignore).
# FORMAT_FIELDS: our builder stores the target as a raster buffer with a byte
# stride, Mesa's GL clear as a UIF texture with a height in UIF blocks
# (docs/notes/v3d71-clear-frame.md 1.3, #6); the dummy stores of the render
# list have 0 in both either way.
FORMAT_FIELDS=(--ignore 'STORE.Memory Format' --ignore 'STORE.Height in UB or Stride')
# ADDRESS_FIELDS: where the buffers sit (the board's are the kernel's); the
# submit's lines are arg0.. of ADD_BIN (BCL start, end, tile allocation, its
# size, tile state) and ADD_RENDER (RCL start, end, tile allocation). The
# tile allocation's size (ADD_BIN.arg3) is compared.
ADDRESS_FIELDS=(--ignore 'OCCLUSION_QUERY_COUNTER_ENABLE.address'
  --ignore 'MULTICORE_RENDERING_TILE_LIST_BASE.address' --ignore 'STORE.Address'
  --ignore 'GENERIC_TILE_LIST.start' --ignore 'GENERIC_TILE_LIST.end'
  --ignore 'ADD_BIN.arg0' --ignore 'ADD_BIN.arg1' --ignore 'ADD_BIN.arg2' --ignore 'ADD_BIN.arg4'
  --ignore 'ADD_RENDER.arg0' --ignore 'ADD_RENDER.arg1' --ignore 'ADD_RENDER.arg2')

fails=0
fail() {
  echo "FAIL: $*"
  fails=$((fails + 1))
}

# ---- build: the builder at 7.1 with our warnings, frame_check against Mesa ----
{
  cc -O1 -std=c11 -Wall -Wextra -Werror -DV3D_CL_VER=71 "${inc[@]}" -c driver/src/v3d/v3d_frame.c \
    -o "$dir/v3d_frame71.o" &&
    cc -O1 -std=c11 -Wall -Werror -DV3D_CL_VER=71 "${inc[@]}" tools/v3d/frame_check.c \
      "$dir/v3d_frame71.o" "${libs[@]}" -o "$dir/frame_check71"
} >"$dir/build.log" 2>&1 || {
  tail -30 "$dir/build.log"
  exit 1
}

# ---- (a) Mesa's decoder at the 4.2 run's sizes ----
na=0
for wh in "1 1" "64 64" "200 100" "65 3000" "1920 1080" "4096 4096"; do
  read -r w h <<<"$wh"
  na=$((na + 1))
  out=$(TMPDIR=$dir/tmp "$dir/frame_check71" "$w" "$h" 2>&1)
  rc=$?
  last=$(printf '%s\n' "$out" | tail -n 1)
  if [ "$rc" -ne 0 ] || ! printf '%s\n' "$last" | grep -q ": ok$"; then
    fail "(a) ${w}x$h: exit $rc"
    printf '%s\n' "$out" | sed 's/^/  /'
  fi
done

# ---- (b) the 64x64 clear against Mesa's drm-shim dump ----
out=$(TMPDIR=$dir/tmp FRAME_CHECK_CLEAR=0xffff0000 FRAME_CHECK_CLIF=$dir/clear64.clif \
  "$dir/frame_check71" 64 64 2>&1)
rc=$?
case $out in
*"BCL 19 RCL 96 list 27 bytes"*) ;;
*) fail "(b) 64x64: not 19 / 96 / 27 bytes: $out" ;;
esac
if [ "$rc" -ne 0 ]; then
  fail "(b) 64x64: frame_check exit $rc"
fi
python3 -I tools/v3d/clif_cmp.py "$dir/clear64.clif" tools/v3d/shim71/clear.clif "${FORMAT_FIELDS[@]}" \
  >"$dir/shim71.cmp" 2>&1 || {
  fail "(b) differs from tools/v3d/shim71/clear.clif:"
  sed 's/^/  /' "$dir/shim71.cmp"
}
shim_line=$(tail -n 1 "$dir/shim71.cmp")

# ---- (c) the probe's hand-packed lists ----
cat >"$dir/probe_eq.c" <<'EOF'
/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
#include <stdio.h>
#include <string.h>

#include "frame71.h" /* includes v3d_frame.h, built at V3D_CL_VER 71 */

#define CAP 65536
static uint8_t ours[3][CAP], probe[3][CAP];
static const char *const names[3] = { "binning", "render", "tile list" };
static const uint32_t gpu[3] = { 0x00101000u, 0x00111000u, 0x00121000u };
static int total, bad;

static void result(int ok, const char *what)
{
	total++;
	if (!ok && ++bad < 20)
		printf("FAIL: %s\n", what);
}

/* Both builders on one frame: both refuse it, or both write the same bytes. */
static void one(uint32_t w, uint32_t h, uint32_t clear, uint32_t target, uint32_t stride,
		uint32_t tile_alloc, uint32_t cap)
{
	struct v3d_clear_frame f = { w, h, clear, target, stride, tile_alloc };
	struct frame71 p = { w, h, clear, target, stride, tile_alloc };
	struct v3d_cl cl[3];
	struct cl71 pl[3];
	char what[160];

	memset(ours, 0xa5, sizeof ours);
	memset(probe, 0xa5, sizeof probe);
	for (int i = 0; i < 3; i++) {
		v3d_cl_init(&cl[i], ours[i], gpu[i], cap);
		cl71_init(&pl[i], probe[i], gpu[i], cap);
	}
	bool a = v3d_frame_build_clear(&f, &cl[0], &cl[1], &cl[2]);
	bool b = frame71_build_clear(&p, &pl[0], &pl[1], &pl[2]);
	if (a != b) {
		snprintf(what, sizeof what, "%ux%u (lists of %u bytes): the driver %s, the probe %s", w,
			 h, cap, a ? "builds" : "refuses", b ? "builds" : "refuses");
		result(0, what);
		return;
	}
	if (!a) {
		result(1, "");
		return;
	}
	for (int i = 0; i < 3; i++) {
		uint32_t at = 0;
		while (at < cl[i].used && at < pl[i].len && ours[i][at] == probe[i][at])
			at++;
		snprintf(what, sizeof what,
			 "%ux%u clear %08x: %s list, driver %u bytes, probe %u, first difference at byte %u",
			 w, h, clear, names[i], cl[i].used, pl[i].len, at);
		result(cl[i].used == pl[i].len && memcmp(ours[i], probe[i], pl[i].len) == 0, what);
	}
}

int main(void)
{
	static const uint32_t sz[] = { 1, 7, 8, 9, 63, 64, 65, 127, 128, 129, 255, 256, 257,
				       720, 1080, 1280, 1919, 1920, 1921, 3000, 4096 };
	static const uint32_t colours[] = { 0xff00ff00u, 0xffff0000u, 0x00000000u, 0xffffffffu,
					    0x80402010u };
	const int n = sizeof sz / sizeof sz[0];

	for (int i = 0; i < n; i++)
		for (int j = 0; j < n; j++)
			one(sz[i], sz[j], colours[(i + j) % 5], 0x00200200u + 64u * (uint32_t)i,
			    sz[i] * 4 + 64u * (uint32_t)j, 0x00400000u + 64u * (uint32_t)j, CAP);
	/* the shim's frame and rpi5_clear's own (128x64, two tiles) */
	one(64, 64, 0xffff0000u, 0x00003000u, 256, 0x00009000u, CAP);
	one(128, 64, 0xff00ff00u, 0x00100000u + 0x1000u + 512u, 512, 0x00110000u, CAP);
	/* refused by both */
	one(0, 64, 0xff00ff00u, 0x00200000u, 0, 0x00400000u, CAP);
	one(64, 0, 0xff00ff00u, 0x00200000u, 256, 0x00400000u, CAP);
	one(65536, 64, 0xff00ff00u, 0x00200000u, 262144, 0x00400000u, CAP);
	one(64, 65536, 0xff00ff00u, 0x00200000u, 256, 0x00400000u, CAP);
	one(64, 64, 0xff00ff00u, 0x00200000u, 256, 0x00400020u, CAP); /* tile alloc not 64-aligned */
	one(64, 64, 0xff00ff00u, 0x00200000u, 256, 0x00400000u, 20);  /* lists of 20 bytes */

	printf("probe_eq: %d of %d equal, %d failed\n", total - bad, total, bad);
	return bad != 0;
}
EOF
{
  cc -O1 -std=c11 -Wall -Wextra -Werror -DV3D_CL_VER=71 "${inc[@]}" -I"$probe" "$dir/probe_eq.c" \
    "$probe/frame71.c" "$dir/v3d_frame71.o" -lm -o "$dir/probe_eq"
} >"$dir/probe_eq.log" 2>&1 || {
  tail -30 "$dir/probe_eq.log"
  exit 1
}
probe_out=$("$dir/probe_eq" 2>&1) || {
  fail "(c) the driver's lists differ from frame71.c's:"
  printf '%s\n' "$probe_out" | sed 's/^/  /'
}
probe_line=$(printf '%s\n' "$probe_out" | tail -n 1)

# ---- (d) D3's board dumps ----
shopt -s nullglob
d3=("$pi5"/*clear*.clif)
shopt -u nullglob
nd=0
for ref_clif in "${d3[@]}"; do
  nd=$((nd + 1))
  # the frame of the dump: its size and its clear colour (the first of each)
  dims=$(sed 's/^MESA: info: \{0,1\}//' "$ref_clif" | awk '
    /^TILE_BINNING_MODE_CFG$/ { bin = 1; next }
    /^[A-Z@]/ { bin = 0 }
    bin && /Width \(in pixels\):/ && !w { w = $NF }
    bin && /Height \(in pixels\):/ && !h { h = $NF }
    /Clear Color low bits:/ && c == "" { c = $NF }
    END { if (w && h && c != "") printf "%s %s %s\n", w, h, c }')
  if [ -z "$dims" ]; then
    fail "(d) $ref_clif: no TILE_BINNING_MODE_CFG size or RENDER_TARGET_PART1 clear colour in it"
    continue
  fi
  read -r w h c <<<"$dims"
  base=$(basename "$ref_clif" .clif)
  out=$(TMPDIR=$dir/tmp FRAME_CHECK_CLEAR=$c FRAME_CHECK_CLIF=$dir/d3_$base.clif \
    "$dir/frame_check71" "$w" "$h" 2>&1) || {
    fail "(d) $ref_clif: frame_check ${w}x$h: $out"
    continue
  }
  python3 -I tools/v3d/clif_cmp.py "$dir/d3_$base.clif" "$ref_clif" "${FORMAT_FIELDS[@]}" \
    "${ADDRESS_FIELDS[@]}" >"$dir/d3_$base.cmp" 2>&1 || {
    fail "(d) ${w}x$h clear $c differs from $ref_clif:"
    sed 's/^/  /' "$dir/d3_$base.cmp"
  }
  echo "(d) $ref_clif: ${w}x$h clear $c, $(tail -n 1 "$dir/d3_$base.cmp")"
done

summary="(a) $na sizes, (b) shim71 ${shim_line#clif_cmp: }, (c) $probe_line, (d) $nd D3 dump(s)"
if [ "$fails" -ne 0 ]; then
  echo "$name: $summary, $fails failed"
  exit 1
fi
if [ "$nd" -eq 0 ]; then
  echo "(d) $pi5/*clear*.clif (the Pi 5's V3D_DEBUG=clif dump of a clear, item d3-linux): none recorded yet, nothing to compare"
  summary+=" (none recorded yet)"
fi
echo "$name: $summary, 0 failed"
exit 0
