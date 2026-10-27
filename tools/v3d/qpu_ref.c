/* SPDX-License-Identifier: MIT
 *
 * qpu_ref: Mesa's own answer for V3D 4.2 QPU words, as the reference for
 * YakoGL's online compiler's decoder, encoder and listing
 * (src/shaderc/online/v3d). For each 64-bit word on stdin (one hex word per
 * line) it prints what Mesa 26.2.1 does with it, and nothing it concludes:
 *
 *   0x<word> U=0                            v3d_qpu_instr_unpack refused it
 *   0x<word> U=1 K=<k> P=0x<repack> <text>  unpacked; K is whether
 *                                           v3d_qpu_instr_pack succeeded, P
 *                                           its word, <text> v3d_qpu_decode
 *
 * The Rust test v3d::mesa_ref::decoder_agrees_with_mesa_on_every_field_combination
 * feeds it an exhaustive enumeration of every field group and compares.
 *
 * Mesa's qpu sources are compiled into this binary with -DNDEBUG, because
 * Mesa asserts on some words it accepts (qpu_pack.c:1660, LDVPMP with a
 * magic write; qpu_disasm.c:77, a small immediate past the table); with the
 * assertions on, one such word would abort the whole run. The Rust side
 * names those words from their bits instead. build-v3d itself is
 * debugoptimized with assertions on, so its libbroadcom_qpu.a is not used.
 *
 * Build (M = a Mesa 26.2.1 tree with build-v3d, see tools/v3d/README.md):
 *   cc -O2 -std=gnu11 -DNDEBUG -DHAVE_ENDIAN_H -DBUILD_ID="\"$(git describe --always --dirty)\"" \
 *      -I$M/include -I$M/src -I$M/src/broadcom -I$M/build-v3d/src \
 *      tools/v3d/qpu_ref.c $M/src/broadcom/qpu/qpu_pack.c \
 *      $M/src/broadcom/qpu/qpu_instr.c $M/src/broadcom/qpu/qpu_disasm.c \
 *      $M/build-v3d/src/util/libmesa_util.a -lm -lpthread -o qpu_ref
 *
 * Exit status 0 when every line read was a word and was answered.
 */
#include <inttypes.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "broadcom/common/v3d_device_info.h"
#include "broadcom/qpu/qpu_disasm.h"
#include "broadcom/qpu/qpu_instr.h"
#include "util/ralloc.h"

#ifndef BUILD_ID
#define BUILD_ID "unknown"
#endif

/* v3d_qpu_magic_waddr_name (qpu_instr.c:30-99) indexes a table whose last
 * entry is V3D_QPU_WADDR_R5REP (55) without a bound check, so its text for a
 * magic address 56..63 reads past the table: garbage or a crash. Such a word
 * is answered "OOB" instead of with text. */
static bool magic_name_out_of_bounds(const struct v3d_device_info *devinfo,
				     const struct v3d_qpu_instr *instr)
{
	if (instr->type != V3D_QPU_INSTR_TYPE_ALU)
		return false;
	return (instr->alu.add.magic_write && instr->alu.add.waddr > V3D_QPU_WADDR_R5REP) ||
	       (instr->alu.mul.magic_write && instr->alu.mul.waddr > V3D_QPU_WADDR_R5REP) ||
	       (v3d_qpu_sig_writes_address(devinfo, &instr->sig) && instr->sig_magic &&
		instr->sig_addr > V3D_QPU_WADDR_R5REP);
}

int main(void)
{
	struct v3d_device_info devinfo;
	char line[128];
	unsigned long read = 0, answered = 0;

	memset(&devinfo, 0, sizeof(devinfo));
	devinfo.ver = 42;
	fprintf(stderr, "qpu_ref: build %s, Mesa qpu_pack/qpu_disasm ver %d\n", BUILD_ID, devinfo.ver);

	while (fgets(line, sizeof(line), stdin)) {
		char *end;
		uint64_t w;
		struct v3d_qpu_instr instr;

		read++;
		w = strtoull(line, &end, 16);
		if (end == line || (*end != '\n' && *end != '\0')) {
			fprintf(stderr, "qpu_ref: line %lu is not a word: %s", read, line);
			return 1;
		}
		memset(&instr, 0, sizeof(instr));
		if (!v3d_qpu_instr_unpack(&devinfo, w, &instr)) {
			printf("0x%016" PRIx64 " U=0\n", w);
		} else {
			uint64_t p = 0;
			bool k = v3d_qpu_instr_pack(&devinfo, &instr, &p);

			if (magic_name_out_of_bounds(&devinfo, &instr)) {
				printf("0x%016" PRIx64 " U=1 K=%d P=0x%016" PRIx64 " OOB\n", w, k, p);
			} else {
				const char *s = v3d_qpu_decode(&devinfo, &instr);

				printf("0x%016" PRIx64 " U=1 K=%d P=0x%016" PRIx64 " %s\n", w, k, p, s);
				ralloc_free((void *)s);
			}
		}
		answered++;
	}
	fprintf(stderr, "qpu_ref: %lu words read, %lu answered\n", read, answered);
	return read == answered ? 0 : 1;
}
