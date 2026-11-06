#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# shim71: the control lists Mesa's v3d driver submitted on the drm-shim at V3D
# 7.1 (tools/v3d/shim71/*.clif, from tools/v3d/drm_shim_ref.sh) are read with
# the packet spec of the Mesa reference build at version 71 (v3d_spec_load, the
# spec Mesa's CLE decoder and clif_dump use): every packet a ctrllist names is
# a command of that spec, the lists are not empty, and the .stdout beside each
# says the shim reported a V3D 7.1. A packet of another generation (4.2 only)
# would not be found. Mesa has no CLIF text parser, so the tool below reads the
# names that its own dump (v3d_print_group) wrote.
# Reference: tools/v3d/mesa_ref.sh (build/v3d-check/mesa), or MESA_V3D_TREE
# with build-v3d as the v3d rows of tools/check.sh. Convention:
# tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=shim71
dir=build/v3d71/$name
mkdir -p "$dir" || exit 1

tree=${MESA_V3D_TREE:-}
mt=third_party/mesa-26.2.1
ref=build/v3d-check/mesa
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

cat >"$dir/shim71_check.c" <<'EOF'
/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
#include <ctype.h>
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include "common/v3d_device_info.h"
#include "cle/v3d_decoder.h"

/* the spec's command names, as the CLIF dump spells them (upper case, '_') */
static char names[512][64];
static int nnames;

static void norm(const char *x, char *out, size_t n)
{
	size_t i = 0;
	/* "Tile Rendering Mode Cfg (Common)" is dumped TILE_RENDERING_MODE_CFG_COMMON */
	for (; *x && i + 1 < n; x++) {
		if (isalnum((unsigned char)*x))
			out[i++] = toupper((unsigned char)*x);
		else if (i && out[i - 1] != '_')
			out[i++] = '_';
	}
	while (i && out[i - 1] == '_')
		i--;
	out[i] = 0;
}

int main(int argc, char **argv)
{
	struct v3d_device_info devinfo = { .ver = 71, .has_accumulators = false,
					   .cle_readahead = 256 };
	struct v3d_spec *spec = v3d_spec_load(&devinfo);
	if (!spec) {
		printf("FAIL: no V3D 7.1 packet spec\n");
		return 1;
	}
	/* several commands share an opcode (121, Tile Rendering Mode Cfg) and are
	 * told apart by the next byte: try them all, keep each name once */
	for (int op = 0; op < 256; op++) {
		for (int sub = 0; sub < 256; sub++) {
			uint8_t p[16] = { (uint8_t)op, (uint8_t)sub };
			struct v3d_group *g = v3d_spec_find_instruction(spec, p);
			char n[64];
			int dup = 0;
			if (!g)
				continue;
			norm(v3d_group_get_name(g), n, sizeof(n));
			for (int i = 0; i < nnames; i++)
				dup |= !strcmp(names[i], n);
			if (!dup && nnames < 512)
				strcpy(names[nnames++], n);
		}
	}
	if (nnames < 50) {
		printf("FAIL: the 7.1 spec has only %d commands\n", nnames);
		return 1;
	}
	int files = 0, packets = 0, failed = 0;
	for (int a = 1; a < argc; a++) {
		FILE *f = fopen(argv[a], "r");
		char line[512];
		int here = 0, ctrl = 0, inlist = 0;
		if (!f) {
			printf("FAIL: %s: cannot open\n", argv[a]);
			failed++;
			continue;
		}
		while (fgets(line, sizeof line, f)) {
			char *s = strstr(line, "MESA: info: ");
			if (!s)
				continue;
			s += 12;
			s[strcspn(s, "\n")] = 0;
			if (!strncmp(s, "@format ", 8)) {
				inlist = !strncmp(s, "@format ctrllist", 16);
				ctrl += inlist;
				continue;
			}
			if (!inlist || !*s || s[0] == ' ' || s[0] == '@')
				continue;
			int found = 0;
			for (int i = 0; i < nnames && !found; i++)
				found = !strcmp(names[i], s);
			if (!found) {
				printf("FAIL: %s: packet %s is not a V3D 7.1 command\n", argv[a], s);
				failed++;
			}
			here++;
		}
		fclose(f);
		if (!ctrl || !here) {
			printf("FAIL: %s: no ctrllist packets\n", argv[a]);
			failed++;
		}
		files++;
		packets += here;
	}
	if (!files) {
		printf("FAIL: no CLIF file given\n");
		return 1;
	}
	printf("shim71: %d CLIF files, %d packets in %d commands of the 7.1 spec, %d failed\n",
	       files, packets, nnames, failed);
	return failed != 0;
}
EOF
cc -O1 -std=gnu11 "${inc[@]}" "$dir/shim71_check.c" "${libs[@]}" -o "$dir/shim71_check" \
  >"$dir/build.log" 2>&1 || {
  tail -20 "$dir/build.log"
  exit 1
}

for c in tools/v3d/shim71/*.clif; do
  grep -q 'renderer V3D 7\.1' "${c%.clif}.stdout" || {
    echo "FAIL: ${c%.clif}.stdout does not say the shim reported a V3D 7.1"
    exit 1
  }
done
"$dir/shim71_check" tools/v3d/shim71/*.clif
