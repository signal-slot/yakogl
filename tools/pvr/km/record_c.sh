#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# record_c.sh <workdir>: rebuild the records of the C the PowerVR kernel side
# had before docs/source-form.md step 7 (tools/pvr/km/*-674ea1cf.rec):
# cuts the functions out of driver/src/pvr_fw_boot.c at origin/main 674ea1cf
# and runs record_c.c on them. Needs the commit in the repository's history;
# the records themselves are committed, so the check (tools/check.sh,
# km_check.c) does not.
set -eu
REPO=$(cd "$(dirname "$0")/../../.." && pwd)
OUT=$1
mkdir -p "$OUT"
git -C "$REPO" show 674ea1cf:driver/src/pvr_fw_boot.c > "$OUT/pvr_fw_boot.c"
# The heap layout and ring sizes.
awk '/^\/\* ---- FW address regions/ {on = 1} /^\/\* ---- carveout \+ bump allocators/ {on = 0} on' \
	"$OUT/pvr_fw_boot.c" > "$OUT/old_layout.inc"
# The carveout allocators and the page tables: from the allocator banner to
# the banner of the DDK's start/stop.
awk '/^\/\* ---- carveout \+ bump allocators/ {on = 1} /^\/\* ---- the DDK.s start\/stop/ {on = 0} on' \
	"$OUT/pvr_fw_boot.c" > "$OUT/old_mem_mmu.inc"
# The render pool: from its cursor to the KCCB code.
awk '/^static uint32_t g_render_next/ {on = 1} /^\/\* Post a prebuilt KCCB command/ {on = 0} on' \
	"$OUT/pvr_fw_boot.c" > "$OUT/old_bo.inc"
grep '^uint64_t pvr_bo_phys' "$OUT/pvr_fw_boot.c" > "$OUT/old_bo_phys.inc"
# The FWIF globals (the CCB offsets, the freelist tracking).
awk '/^\/\* ---- FWIF control structures/ {on = 1} /^\/\* ---- exported accessors/ {on = 0} on' \
	"$OUT/pvr_fw_boot.c" > "$OUT/old_fwif_vars.inc"
# The KCCB, the freelist answers and the FWCCB service.
awk '/^\/\* Post a prebuilt KCCB command/ {on = 1} /^\/\* The TQ_3D fill test/ {on = 0} on' \
	"$OUT/pvr_fw_boot.c" > "$OUT/old_ccb.inc"
# The grow pool, the counters and the freelist builder.
awk '/^void pvr_fl_pool_init/ {on = 1} /^#ifdef PVR_LINUX_MODULE/ {on = 0} on' \
	"$OUT/pvr_fw_boot.c" > "$OUT/old_fl.inc"
# The FWIF structures and the boot wait.
awk '/^\/\* Allocate a heap object, return its heap offset/ {on = 1} /^\/\* PM\/clock\/power snapshot/ {on = 0} on' \
	"$OUT/pvr_fw_boot.c" > "$OUT/old_fwinit.inc"
awk '/^static int pvr_wait_for_fw_boot/ {on = 1} /^\/\* ---- M5: KCCB command round-trip/ {on = 0} on' \
	"$OUT/pvr_fw_boot.c" > "$OUT/old_wait.inc"
# The accessors and the whole render service.
awk '/^\/\* ---- exported accessors/ {on = 1} /^\/\* Allocate a heap object, return its heap offset/ {on = 0} on' \
	"$OUT/pvr_fw_boot.c" > "$OUT/old_accessors.inc"
git -C "$REPO" show 674ea1cf:driver/src/pvr_render.c > "$OUT/old_render.inc"
# shellcheck disable=SC2046
cc -std=gnu11 -O1 -Wall -Wno-unused-function -DTEE_DDK $("$REPO/driver/tools/offsetof/ddk-cflags.sh") \
	-I"$REPO/tools/pvr/km/hostshim" -I"$REPO/tools/pvr/rgxstart/hostshim" -I"$REPO/driver/src" \
	-I"$REPO/tools/pvr/km" -I"$OUT" "$REPO/tools/pvr/km/record_c.c" -o "$OUT/record_c"
# shellcheck disable=SC2046
cc -std=gnu11 -O1 -Wall -Wno-unused-function -DTEE_DDK -DPVR_RENDER_VERBOSE \
	$("$REPO/driver/tools/offsetof/ddk-cflags.sh") \
	-I"$REPO/tools/pvr/km/hostshim" -I"$REPO/tools/pvr/rgxstart/hostshim" -I"$REPO/driver/src" \
	-I"$REPO/tools/pvr/km" -I"$OUT" "$REPO/tools/pvr/km/record_c.c" -o "$OUT/record_c_verbose"
for s in mem ccb fwinit render; do
	"$OUT/record_c" "$s" > "$OUT/$s-674ea1cf.rec"
	echo "$OUT/$s-674ea1cf.rec"
done
"$OUT/record_c_verbose" render > "$OUT/render-verbose-674ea1cf.rec"
echo "$OUT/render-verbose-674ea1cf.rec"
