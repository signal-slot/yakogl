/* SPDX-License-Identifier: MIT
 *
 * tri_validate71: Mesa 26.2.1's own V3D 7.1 qpu_validate() over one program of
 * a tools/v3d/tri-shaders71 json file (the "code" array of its "fs", "vs" or
 * "cs" object), at devinfo ver 71 rev 10 (7.1.10, the Pi 5's V3D).
 *
 *   tri_validate71 <json> <fs|vs|cs> <label>
 *
 * Prints one verdict line, "<label>: <n> instructions, ok" or
 * "<label>: <n> instructions, FAIL ip <i>: <validator message>", then the
 * program disassembled with Mesa's v3d_qpu_disasm (qpu_disasm.c, compiled
 * unchanged beside this file), the instruction the validator rejected marked
 * with "***". Exit status 0 ok, 1 rejected (or a word Mesa cannot unpack),
 * 2 usage or unreadable input.
 *
 * Like hazards71.c, qpu_validate.c is compiled into this binary through
 * #include with the compile's containers declared here, so the validator
 * runs unchanged; the program is multi-segment (c->last_thrsw set) when the
 * JSON says "single_seg": false. fail_instr() logs and abort()s: both are
 * caught. The ralloc calls of qpu_disasm.c and qpu_validate.c are served
 * below, so no Mesa util library is linked. Build: check71.d/tri_validate71.sh.
 */
#include <inttypes.h>
#include <limits.h>
#include <setjmp.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define V3D_COMPILER_H
#include "broadcom/common/v3d_device_info.h"
#include "broadcom/qpu/qpu_instr.h"
#include "util/list.h"
#include "util/ralloc.h"

struct qinst {
	struct list_head link;
	struct v3d_qpu_instr qpu;
	bool is_tlb_z_write;
};

struct qblock {
	struct list_head link;
	struct list_head instructions;
};

struct v3d_compile {
	const struct v3d_device_info *devinfo;
	struct list_head blocks;
	struct qinst *last_thrsw;
};

#define vir_for_each_block(block, c) list_for_each_entry(struct qblock, block, &c->blocks, link)
#define vir_for_each_inst(inst, block) \
	list_for_each_entry(struct qinst, inst, &block->instructions, link)
#define vir_for_each_inst_inorder(inst, c) \
	vir_for_each_block(_block, c) vir_for_each_inst(inst, _block)

static bool vir_is_add(struct qinst *inst)
{
	return inst->qpu.type == V3D_QPU_INSTR_TYPE_ALU && inst->qpu.alu.add.op != V3D_QPU_A_NOP;
}

static bool vir_is_mul(struct qinst *inst)
{
	return inst->qpu.type == V3D_QPU_INSTR_TYPE_ALU && inst->qpu.alu.mul.op != V3D_QPU_M_NOP;
}

/* the ralloc subset qpu_disasm.c and qpu_validate.c use, on malloc */
void *rzalloc_size(const void *ctx, size_t size)
{
	(void)ctx;
	return calloc(1, size);
}

void ralloc_free(void *ptr)
{
	free(ptr);
}

bool ralloc_vasprintf_rewrite_tail(char **str, size_t *start, const char *fmt, va_list args)
{
	va_list copy;
	va_copy(copy, args);
	int n = vsnprintf(NULL, 0, fmt, copy);
	va_end(copy);
	if (n < 0)
		return false;
	char *p = realloc(*str, *start + (size_t)n + 1);
	if (!p)
		return false;
	vsnprintf(p + *start, (size_t)n + 1, fmt, args);
	*str = p;
	*start += (size_t)n;
	return true;
}

static char first_msg[256];
static bool have_msg;
static int fail_ip;
static jmp_buf fail_jmp;

static void tv_loge(const char *fmt, ...)
{
	va_list ap;
	if (have_msg)
		return;
	va_start(ap, fmt);
	vsnprintf(first_msg, sizeof first_msg, fmt, ap);
	va_end(ap);
	have_msg = true;
}

static void tv_abort(void)
{
	longjmp(fail_jmp, 1);
}

#define mesa_loge(...) tv_loge(__VA_ARGS__)
#define abort() tv_abort()
#include "broadcom/qpu/qpu_disasm.h"

#include "compiler/qpu_validate.c"

#undef abort

#define MAXN 4096

static char *slurp(const char *path)
{
	FILE *f = fopen(path, "rb");
	if (!f)
		return NULL;
	fseek(f, 0, SEEK_END);
	long n = ftell(f);
	fseek(f, 0, SEEK_SET);
	char *b = malloc((size_t)n + 1);
	if (b && fread(b, 1, (size_t)n, f) != (size_t)n) {
		free(b);
		b = NULL;
	}
	if (b)
		b[n] = 0;
	fclose(f);
	return b;
}

int main(int argc, char **argv)
{
	if (argc != 4) {
		fprintf(stderr, "usage: tri_validate71 <json> <fs|vs|cs> <label>\n");
		return 2;
	}
	const char *label = argv[3];
	char *txt = slurp(argv[1]);
	if (!txt) {
		fprintf(stderr, "tri_validate71: cannot read %s\n", argv[1]);
		return 2;
	}
	char key[16];
	snprintf(key, sizeof key, "\"%s\": {", argv[2]);
	char *obj = strstr(txt, key);
	if (!obj) {
		fprintf(stderr, "tri_validate71: no %s object in %s\n", argv[2], argv[1]);
		return 2;
	}
	/* the object ends where the next stage object begins */
	char *end = NULL;
	for (const char *o = "fsvscs"; *o; o += 2) {
		char k2[16];
		snprintf(k2, sizeof k2, "\"%.2s\": {", o);
		char *e = strstr(obj + 1, k2);
		if (e && (!end || e < end))
			end = e;
	}
	char *seg = strstr(obj, "\"single_seg\": ");
	bool multi = seg && (!end || seg < end) && !strncmp(seg + 14, "false", 5);
	char *code = strstr(obj, "\"code\": [");
	if (!code || (end && code > end)) {
		fprintf(stderr, "tri_validate71: no code array in %s of %s\n", argv[2], argv[1]);
		return 2;
	}
	code += 9;
	static uint64_t words[MAXN];
	int n = 0;
	for (;;) {
		while (*code == ' ' || *code == ',' || *code == '\n')
			code++;
		if (*code != '"')
			break;
		if (n == MAXN)
			return 2;
		words[n++] = strtoull(code + 1, &code, 16);
		if (*code != '"')
			return 2;
		code++;
	}
	if (n == 0)
		return 2;

	static const struct v3d_device_info dev71 = { .ver = 71, .rev = 10 };
	static struct qinst qi[MAXN];
	static struct qblock blk;
	static struct v3d_compile c;
	list_inithead(&c.blocks);
	list_inithead(&blk.instructions);
	list_addtail(&blk.link, &c.blocks);
	c.devinfo = &dev71;
	for (int i = 0; i < n; i++) {
		if (!v3d_qpu_instr_unpack(&dev71, words[i], &qi[i].qpu)) {
			printf("%s: %d instructions, FAIL ip %d: Mesa cannot unpack 0x%016" PRIx64 "\n",
			       label, n, i, words[i]);
			return 1;
		}
		/* The packed word has no magic-write bit for an op without a
		 * destination (qpu_pack.c no_magic_write: stvpmv, ...), so an
		 * unpacked one reads "write rf<n>"; the compiler hands the
		 * validator such an op with its nop destination (a magic write
		 * to NOP), and qpu_validate's THREND rule (qpu_validate.c
		 * "ADD RF write at THREND") would otherwise reject every stvpmv
		 * in a thread-end slot. Restore that form. */
		struct v3d_qpu_instr *q = &qi[i].qpu;
		if (q->type == V3D_QPU_INSTR_TYPE_ALU) {
			if (q->alu.add.op != V3D_QPU_A_NOP && !v3d_qpu_add_op_has_dst(q->alu.add.op)) {
				q->alu.add.magic_write = true;
				q->alu.add.waddr = V3D_QPU_WADDR_NOP;
			}
			if (q->alu.mul.op != V3D_QPU_M_NOP && !v3d_qpu_mul_op_has_dst(q->alu.mul.op)) {
				q->alu.mul.magic_write = true;
				q->alu.mul.waddr = V3D_QPU_WADDR_NOP;
			}
		}
		list_addtail(&qi[i].link, &blk.instructions);
	}
	if (multi)
		c.last_thrsw = &qi[n - 1];

	bool ok = true;
	if (setjmp(fail_jmp)) {
		ok = false;
		/* "v3d_qpu_validate at ip N: msg:" -> "ip N: msg" */
		char *m = strstr(first_msg, "ip ");
		sscanf(m ? m + 3 : "-1", "%d", &fail_ip);
		size_t l = strlen(first_msg);
		while (l && (first_msg[l - 1] == '\n' || first_msg[l - 1] == ':'))
			first_msg[--l] = 0;
		printf("%s: %d instructions, FAIL %s\n", label, n, m ? m : first_msg);
	} else {
		qpu_validate(&c);
		printf("%s: %d instructions, ok (%s)\n", label, n,
		       multi ? "multi-segment" : "single-segment");
	}
	for (int i = 0; i < n; i++) {
		const char *s = v3d_qpu_decode(&dev71, &qi[i].qpu);
		printf("  %2d %016" PRIx64 "  %s%s\n", i, words[i], s,
		       !ok && i == fail_ip ? "   *** rejected ***" : "");
		ralloc_free((void *)s);
	}
	return ok ? 0 : 1;
}
