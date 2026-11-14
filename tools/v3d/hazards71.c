/* SPDX-License-Identifier: MIT
 *
 * hazards71: Mesa 26.2.1's own verdicts on V3D 7.1 QPU programs, an oracle
 * for src/shaderc/online/v3d/verify71.rs.
 *
 * Mesa's qpu_validate (src/broadcom/compiler/qpu_validate.c) is compiled
 * into this binary unmodified (tools/v3d/hazards71.sh copies the file next to
 * tools/v3d/hazards71/v3d_compiler.h, which stands in for the part of Mesa's
 * v3d_compiler.h it uses), and run with a device of ver 71. Nothing here
 * decides a hazard: a program is built, packed with v3d_qpu_instr_pack,
 * unpacked again (so the validator sees what a word says, as verify71 does),
 * and handed to qpu_validate. The answer is Mesa's first failure message and
 * the instruction index it names, or "ok".
 *
 * Two modes:
 *   hazards71            the built-in cases: each is a small program, a
 *                        flag whether the compile has a last-THRSW to find
 *                        (c->last_thrsw), and the verdict its author read off
 *                        qpu_validate.c. The table is printed; the exit
 *                        status is 1 when Mesa's verdict is not the expected
 *                        one (so a misreading of the rule is caught here, not
 *                        copied into the Rust port).
 *   hazards71 --stdin    programs on stdin, one `prog NAME [last]` line and
 *                        then one 64-bit hex word per line; the table of
 *                        their verdicts (no expectation).
 *
 * The table, tab separated, one line per program:
 *   name  ver  flags  words  verdict  ip  message
 * flags: `-`, `last` (c->last_thrsw set) and/or `z=<ip>` (the instruction at
 * <ip> is a TLB Z write: qinst.is_tlb_z_write, which a word does not carry,
 * so a consumer that reads only words skips such a line). words: the 64-bit
 * words of the program, comma separated. verdict: ok or fail; ip and message
 * are `-` for ok.
 *
 * Build: tools/v3d/hazards71.sh.
 */
#include <inttypes.h>
#include <setjmp.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "v3d_compiler.h"

#define MAX_INSTR 16

static jmp_buf hz_jmp;
static bool hz_failed;
static int hz_ip;
static char hz_msg[160];

/* qpu_validate.c's fail_instr logs "v3d_qpu_validate at ip %d: %s:\n" first
 * and then the whole listing; only the first call is the verdict. */
void hz_loge(const char *fmt, ...)
{
	va_list ap;

	if (hz_failed)
		return;
	if (strncmp(fmt, "v3d_qpu_validate at ip", 22) != 0)
		return;
	va_start(ap, fmt);
	hz_ip = va_arg(ap, int);
	snprintf(hz_msg, sizeof(hz_msg), "%s", va_arg(ap, const char *));
	va_end(ap);
	hz_failed = true;
}

void hz_abort(void)
{
	longjmp(hz_jmp, 1);
}

/* ---- instruction builders -------------------------------------------- */

static struct v3d_qpu_instr nop(void)
{
	struct v3d_qpu_instr i;

	memset(&i, 0, sizeof(i));
	i.type = V3D_QPU_INSTR_TYPE_ALU;
	i.alu.add.op = V3D_QPU_A_NOP;
	i.alu.add.magic_write = true;
	i.alu.add.waddr = V3D_QPU_WADDR_NOP;
	i.alu.mul.op = V3D_QPU_M_NOP;
	i.alu.mul.magic_write = true;
	i.alu.mul.waddr = V3D_QPU_WADDR_NOP;
	return i;
}

static struct v3d_qpu_instr branch(void)
{
	struct v3d_qpu_instr i;

	memset(&i, 0, sizeof(i));
	i.type = V3D_QPU_INSTR_TYPE_BRANCH;
	i.branch.cond = V3D_QPU_BRANCH_COND_ALWAYS;
	i.branch.msfign = V3D_QPU_MSFIGN_NONE;
	i.branch.bdi = V3D_QPU_BRANCH_DEST_REL;
	i.branch.ub = false;
	return i;
}

static struct v3d_qpu_instr with_thrsw(struct v3d_qpu_instr i)
{
	i.sig.thrsw = true;
	return i;
}

/* add: dst = a + b (raddrs) */
static struct v3d_qpu_instr add_rf(uint8_t dst, uint8_t a, uint8_t b)
{
	struct v3d_qpu_instr i = nop();

	i.alu.add.op = V3D_QPU_A_ADD;
	i.alu.add.a.raddr = a;
	i.alu.add.b.raddr = b;
	i.alu.add.waddr = dst;
	i.alu.add.magic_write = false;
	return i;
}

/* add MOV a -> magic waddr */
static struct v3d_qpu_instr add_magic(uint8_t waddr)
{
	struct v3d_qpu_instr i = nop();

	i.alu.add.op = V3D_QPU_A_MOV;
	i.alu.add.a.raddr = 1;
	i.alu.add.waddr = waddr;
	i.alu.add.magic_write = true;
	return i;
}

static struct v3d_qpu_instr mul_rf(uint8_t dst)
{
	struct v3d_qpu_instr i = nop();

	i.alu.mul.op = V3D_QPU_M_MOV;
	i.alu.mul.a.raddr = 1;
	i.alu.mul.waddr = dst;
	i.alu.mul.magic_write = false;
	return i;
}

static struct v3d_qpu_instr mul_magic(uint8_t waddr)
{
	struct v3d_qpu_instr i = mul_rf(0);

	i.alu.mul.waddr = waddr;
	i.alu.mul.magic_write = true;
	return i;
}

static struct v3d_qpu_instr mul_op(enum v3d_qpu_mul_op op)
{
	struct v3d_qpu_instr i = nop();

	i.alu.mul.op = op;
	i.alu.mul.a.raddr = 1;
	i.alu.mul.b.raddr = 2;
	i.alu.mul.waddr = 4;
	i.alu.mul.magic_write = false;
	return i;
}

/* a signal that writes a register-file address (ldvary, ldunifrf, ldtmu) */
static struct v3d_qpu_instr sig_rf(char which, uint8_t rf)
{
	struct v3d_qpu_instr i = nop();

	switch (which) {
	case 'v':
		i.sig.ldvary = true;
		break;
	case 'u':
		i.sig.ldunif = true;
		break;
	case 'r':
		i.sig.ldunifrf = true;
		break;
	case 't':
		i.sig.ldtmu = true;
		break;
	case 'b':
		i.sig.ldtlb = true;
		break;
	}
	if (which != 'u' && which != 'v') {
		i.sig_addr = rf;
		i.sig_magic = false;
	}
	return i;
}

static struct v3d_qpu_instr tmuwt(void)
{
	struct v3d_qpu_instr i = nop();

	i.alu.add.op = V3D_QPU_A_TMUWT;
	i.alu.add.waddr = V3D_QPU_WADDR_NOP;
	i.alu.add.magic_write = true;
	return i;
}

/* ---- the cases -------------------------------------------------------- */

struct hz_case {
	const char *name;
	bool last;
	int zip; /* instruction index of a TLB Z write, or -1 */
	int n;
	struct v3d_qpu_instr p[MAX_INSTR];
	const char *want; /* "ok" or the message Mesa's fail_instr gets */
	int want_ip;
};

static struct hz_case cases[128];
static int ncases;

static struct hz_case *begin(const char *name, bool last, const char *want, int want_ip)
{
	struct hz_case *c = &cases[ncases++];

	c->name = name;
	c->last = last;
	c->zip = -1;
	c->n = 0;
	c->want = want;
	c->want_ip = want_ip;
	return c;
}

static void put(struct hz_case *c, struct v3d_qpu_instr i)
{
	c->p[c->n++] = i;
}

static void nops(struct hz_case *c, int n)
{
	while (n--)
		put(c, nop());
}

/* the smallest program that ends: the THRSW, then its two delay slots */
static void tail(struct hz_case *c)
{
	put(c, with_thrsw(nop()));
	nops(c, 2);
}

static void define_cases(void)
{
	struct hz_case *c;
	int w;

	c = begin("ok_minimal", false, "ok", -1);
	tail(c);

	c = begin("ok_two_adds_then_end", false, "ok", -1);
	put(c, add_rf(3, 1, 2));
	put(c, add_rf(4, 3, 3));
	tail(c);

	c = begin("no_program_end", false, "No program-end THRSW found", 2);
	nops(c, 2);

	/* branch and THRSW delay slots */
	c = begin("branch_in_branch_slot", false, "branch in a branch delay slot.", 1);
	put(c, branch());
	put(c, branch());
	c = begin("branch_in_thrsw_slot", false, "branch in a THRSW delay slot.", 2);
	put(c, with_thrsw(nop()));
	nops(c, 1);
	put(c, branch());
	c = begin("thrsw_in_branch_slot", false, "THRSW in a branch delay slot.", 1);
	put(c, branch());
	put(c, with_thrsw(nop()));
	c = begin("thrsw_too_close", false, "THRSW too close to another THRSW.", 2);
	put(c, with_thrsw(nop()));
	nops(c, 1);
	put(c, with_thrsw(nop()));

	/* the THRSW pairing */
	c = begin("ok_last_thrsw_pair", true, "ok", -1);
	put(c, with_thrsw(nop()));
	put(c, with_thrsw(nop()));
	nops(c, 3);
	put(c, with_thrsw(nop()));
	nops(c, 2);
	c = begin("two_last_thrsw_signals", false, "Two last-THRSW signals", 1);
	put(c, with_thrsw(nop()));
	put(c, with_thrsw(nop()));
	c = begin("thrsw_without_last_thrsw", true,
		  "thread switch found without last-THRSW in program", 7);
	put(c, with_thrsw(nop()));
	nops(c, 3);
	put(c, with_thrsw(nop()));
	nops(c, 2);

	/* LDUNIF after LDVARY (rf0 at 7.1 is ldunifrf, which the rule skips) */
	c = begin("ldunif_after_ldvary", false, "LDUNIF after a LDVARY", 1);
	put(c, sig_rf('v', 0));
	put(c, sig_rf('u', 0));
	c = begin("ok_ldunifrf_after_ldvary", false, "ok", -1);
	put(c, sig_rf('v', 0));
	put(c, sig_rf('r', 5));
	tail(c);

	/* LDVARY in THRSW delay slots: only the second is refused at 7.1 */
	c = begin("ok_ldvary_in_thrsw_slot_1", false, "ok", -1);
	put(c, with_thrsw(nop()));
	put(c, sig_rf('v', 0));
	nops(c, 1);
	c = begin("ldvary_in_thrsw_slot_2", false, "LDVARY in 2nd THRSW delay slot", 2);
	put(c, with_thrsw(nop()));
	nops(c, 1);
	put(c, sig_rf('v', 0));
	c = begin("ok_ldvary_in_thrsw_instruction", false, "ok", -1);
	put(c, with_thrsw(sig_rf('v', 0)));
	nops(c, 2);

	/* reserved magic write addresses at 7.1: 0..4, 10, 14..15, 19..31, 47..63 */
	{
		static const struct {
			const char *n;
			int waddr;
			bool ok;
		} add_w[] = {
			{ "add_waddr_0_r0", 0, false },
			{ "add_waddr_4", 4, false },
			{ "add_waddr_10", 10, false },
			{ "add_waddr_14_vpm", 14, false },
			{ "add_waddr_15_vpmu", 15, false },
			{ "add_waddr_19_recip", 19, false },
			{ "add_waddr_31", 31, false },
			{ "add_waddr_47", 47, false },
			{ "add_waddr_55_r5rep", 55, false },
			{ "add_waddr_5_r5", 5, true },
			{ "add_waddr_6_nop", 6, true },
			{ "add_waddr_7_tlb", 7, true },
			{ "add_waddr_8_tlbu", 8, true },
			{ "add_waddr_9_unifa", 9, true },
			{ "add_waddr_11_tmud", 11, true },
			{ "add_waddr_13_tmuau", 13, true },
			{ "add_waddr_16_tsy", 16, true },
			{ "add_waddr_32_tmuc", 32, true },
			{ "add_waddr_46_tmuhsl", 46, true },
		};

		for (w = 0; w < (int)(sizeof(add_w) / sizeof(add_w[0])); w++) {
			c = begin(add_w[w].n, false,
				  add_w[w].ok ? "ok" : "write to a reserved magic waddr",
				  add_w[w].ok ? -1 : 0);
			put(c, add_magic(add_w[w].waddr));
			if (add_w[w].ok)
				tail(c);
		}
	}
	c = begin("mul_waddr_3_reserved", false, "write to a reserved magic waddr", 0);
	put(c, mul_magic(3));
	c = begin("mul_waddr_22_reserved", false, "write to a reserved magic waddr", 0);
	put(c, mul_magic(22));
	c = begin("mul_waddr_7_tlb_ok", false, "ok", -1);
	put(c, mul_magic(7));
	tail(c);

	/* the SFU is an add operation at 7.1; its magic addresses are reserved */
	c = begin("sfu_magic_write_in_thrsw_slot", false,
		  "SFU write started during THRSW delay slots ", 1);
	put(c, with_thrsw(nop()));
	put(c, add_magic(20));
	c = begin("ok_recip_op_in_thrsw_slot", false, "ok", -1);
	put(c, with_thrsw(nop()));
	{
		struct v3d_qpu_instr i = nop();

		i.alu.add.op = V3D_QPU_A_RECIP;
		i.alu.add.a.raddr = 1;
		i.alu.add.waddr = 4;
		i.alu.add.magic_write = false;
		put(c, i);
	}
	nops(c, 1);
	c = begin("ok_sfu_ops_back_to_back", false, "ok", -1);
	{
		struct v3d_qpu_instr i = nop();

		i.alu.add.op = V3D_QPU_A_RECIP;
		i.alu.add.a.raddr = 1;
		i.alu.add.waddr = 4;
		i.alu.add.magic_write = false;
		put(c, i);
		put(c, i);
	}
	tail(c);

	/* one peripheral per instruction; LDTMU is not counted at 7.1 */
	c = begin("tmu_and_tlb_write", false,
		  "Only one of [TMU, SFU, TSY, TLB read, VPM] allowed", 0);
	{
		struct v3d_qpu_instr i = add_magic(11);

		i.alu.mul.op = V3D_QPU_M_MOV;
		i.alu.mul.a.raddr = 2;
		i.alu.mul.waddr = 7;
		i.alu.mul.magic_write = true;
		put(c, i);
	}
	c = begin("tsy_and_tmu_write", false,
		  "Only one of [TMU, SFU, TSY, TLB read, VPM] allowed", 0);
	{
		struct v3d_qpu_instr i = add_magic(16);

		i.alu.mul.op = V3D_QPU_M_MOV;
		i.alu.mul.a.raddr = 2;
		i.alu.mul.waddr = 13;
		i.alu.mul.magic_write = true;
		put(c, i);
	}
	c = begin("ldtlb_and_tmu_write", false,
		  "Only one of [TMU, SFU, TSY, TLB read, VPM] allowed", 0);
	{
		struct v3d_qpu_instr i = add_magic(11);

		i.sig.ldtlb = true;
		i.sig_addr = 6;
		put(c, i);
	}
	c = begin("ok_ldtmu_and_tlb_write", false, "ok", -1);
	{
		struct v3d_qpu_instr i = add_magic(7);

		i.sig.ldtmu = true;
		i.sig_addr = 6;
		put(c, i);
	}
	tail(c);

	/* small immediates: a/b with an add op, c/d with a mul one ("only one per instruction" cannot
	 * be a case: Mesa's packer refuses two small immediates, so no word has them) */
	c = begin("small_imm_a_without_add", false, "small imm a/b used but no ADD inst", 0);
	{
		struct v3d_qpu_instr i = mul_rf(4);

		i.sig.small_imm_a = true;
		put(c, i);
	}
	c = begin("small_imm_b_without_add", false, "small imm a/b used but no ADD inst", 0);
	{
		struct v3d_qpu_instr i = mul_rf(4);

		i.sig.small_imm_b = true;
		put(c, i);
	}
	c = begin("small_imm_c_without_mul", false, "small imm c/d used but no MUL inst", 0);
	{
		struct v3d_qpu_instr i = add_rf(3, 1, 2);

		i.sig.small_imm_c = true;
		put(c, i);
	}
	c = begin("small_imm_d_without_mul", false, "small imm c/d used but no MUL inst", 0);
	{
		struct v3d_qpu_instr i = add_rf(3, 1, 2);

		i.sig.small_imm_d = true;
		put(c, i);
	}
	c = begin("ok_small_imm_b_with_add", false, "ok", -1);
	{
		struct v3d_qpu_instr i = add_rf(3, 1, 0);

		i.sig.small_imm_b = true;
		put(c, i);
	}
	tail(c);
	c = begin("ok_small_imm_d_with_mul", false, "ok", -1);
	{
		struct v3d_qpu_instr i = mul_rf(4);

		i.alu.mul.b.raddr = 0;
		i.sig.small_imm_d = true;
		put(c, i);
	}
	tail(c);

	/* register-file writes at the thread end (7.1: rf2/rf3 and the THRSW
	 * instruction itself) */
	c = begin("add_rf_write_in_thrend_instr", false, "ADD RF write at THREND", 0);
	put(c, with_thrsw(add_rf(10, 1, 2)));
	c = begin("mul_rf_write_in_thrend_instr", false, "MUL RF write at THREND", 0);
	put(c, with_thrsw(mul_rf(10)));
	c = begin("add_rf2_write_after_thrend", false, "RF2-3 write after THREND", 1);
	put(c, with_thrsw(nop()));
	put(c, add_rf(2, 1, 1));
	c = begin("mul_rf3_write_after_thrend", false, "RF2-3 write after THREND", 2);
	put(c, with_thrsw(nop()));
	nops(c, 1);
	put(c, mul_rf(3));
	c = begin("ldunifrf_rf2_after_thrend", false, "RF2-3 write after THREND", 1);
	put(c, with_thrsw(nop()));
	put(c, sig_rf('r', 2));
	c = begin("ok_rf5_writes_after_thrend", false, "ok", -1);
	put(c, with_thrsw(nop()));
	put(c, add_rf(5, 1, 1));
	put(c, mul_rf(6));
	c = begin("ok_rf2_write_third_slot_after_thrend", false, "ok", -1);
	put(c, with_thrsw(nop()));
	nops(c, 2);
	put(c, add_rf(2, 1, 1));
	c = begin("ok_ldunifrf_rf5_in_thrend_instr", false, "ok", -1);
	put(c, with_thrsw(sig_rf('r', 5)));
	nops(c, 2);

	/* GFXH-1625 (TMUWT in the last instruction) is a 4.2 rule only */
	c = begin("ok_tmuwt_in_last_instruction", false, "ok", -1);
	put(c, with_thrsw(nop()));
	nops(c, 1);
	put(c, tmuwt());

	/* the MULTOP / UMUL24 rtop hazard */
	c = begin("multop_thrsw_umul24", false,
		  "UMUL24 reads rtop from MULTOP but it got cleared by a previous THRSW", 4);
	put(c, mul_op(V3D_QPU_M_MULTOP));
	put(c, with_thrsw(nop()));
	nops(c, 2);
	put(c, mul_op(V3D_QPU_M_UMUL24));
	c = begin("ok_multop_umul24_adjacent", false, "ok", -1);
	put(c, mul_op(V3D_QPU_M_MULTOP));
	put(c, mul_op(V3D_QPU_M_UMUL24));
	tail(c);
	c = begin("ok_multop_in_branch_slot_umul24", false, "ok", -1);
	put(c, branch());
	put(c, mul_op(V3D_QPU_M_MULTOP));
	nops(c, 2);
	put(c, with_thrsw(nop()));
	nops(c, 2);
	put(c, mul_op(V3D_QPU_M_UMUL24));

	/* the Z-write rules: need qinst.is_tlb_z_write, which a word lacks */
	c = begin("setmsf_after_tlb_z_write", false, "SETMSF after TLB Z write", 1);
	c->zip = 0;
	put(c, add_magic(7));
	{
		struct v3d_qpu_instr i = nop();

		i.alu.add.op = V3D_QPU_A_SETMSF;
		i.alu.add.a.raddr = 1;
		put(c, i);
	}
	c = begin("msf_after_tlb_z_write", false, "MSF read after TLB Z write", 1);
	c->zip = 0;
	put(c, add_magic(7));
	{
		struct v3d_qpu_instr i = nop();

		i.alu.add.op = V3D_QPU_A_MSF;
		i.alu.add.waddr = 4;
		i.alu.add.magic_write = false;
		put(c, i);
	}
}

/* ---- running one program --------------------------------------------- */

struct result {
	bool ok;
	int ip;
	char msg[160];
};

static void run(const struct v3d_device_info *devinfo, const struct v3d_qpu_instr *p,
		int n, bool last, int zip, struct result *r)
{
	void *ctx = ralloc_context(NULL);
	struct v3d_compile c;
	struct qblock block;
	struct qinst thrsw;
	int i;

	memset(&c, 0, sizeof(c));
	c.devinfo = devinfo;
	c.last_thrsw = last ? &thrsw : NULL;
	list_inithead(&c.blocks);
	list_inithead(&block.instructions);
	list_addtail(&block.link, &c.blocks);
	for (i = 0; i < n; i++) {
		struct qinst *q = rzalloc(ctx, struct qinst);

		q->qpu = p[i];
		q->is_tlb_z_write = (i == zip);
		list_addtail(&q->link, &block.instructions);
	}
	hz_failed = false;
	if (setjmp(hz_jmp) == 0)
		qpu_validate(&c);
	r->ok = !hz_failed;
	r->ip = hz_ip;
	snprintf(r->msg, sizeof(r->msg), "%s", hz_msg);
	ralloc_free(ctx);
}

static void print_line(const char *name, int ver, bool last, int zip, const uint64_t *w,
		       int n, const struct result *r)
{
	int i;

	printf("%s\t%d\t", name, ver);
	if (!last && zip < 0)
		printf("-");
	if (last)
		printf("last%s", zip >= 0 ? "," : "");
	if (zip >= 0)
		printf("z=%d", zip);
	printf("\t");
	for (i = 0; i < n; i++)
		printf("%s0x%016" PRIx64, i ? "," : "", w[i]);
	if (r->ok)
		printf("\tok\t-\t-\n");
	else
		printf("\tfail\t%d\t%s\n", r->ip, r->msg);
}

/* the message with the trailing space or period kept: the table is Mesa's text */
static int run_builtin(const struct v3d_device_info *devinfo)
{
	int i, bad = 0;

	for (i = 0; i < ncases; i++) {
		struct hz_case *c = &cases[i];
		struct v3d_qpu_instr back[MAX_INSTR];
		uint64_t w[MAX_INSTR];
		struct result r;
		int k;

		for (k = 0; k < c->n; k++) {
			if (!v3d_qpu_instr_pack(devinfo, &c->p[k], &w[k]) ||
			    !v3d_qpu_instr_unpack(devinfo, w[k], &back[k])) {
				fprintf(stderr, "hazards71: case %s: instruction %d does not pack at ver %d\n",
					c->name, k, devinfo->ver);
				return 2;
			}
		}
		run(devinfo, back, c->n, c->last, c->zip, &r);
		print_line(c->name, devinfo->ver, c->last, c->zip, w, c->n, &r);
		{
			bool ok = strcmp(c->want, "ok") == 0;

			if (r.ok != ok || (!ok && (strcmp(r.msg, c->want) != 0 || r.ip != c->want_ip))) {
				fprintf(stderr, "hazards71: case %s: Mesa says %s ip %d '%s', expected %s ip %d\n",
					c->name, r.ok ? "ok" : "fail", r.ip, r.msg, c->want,
					c->want_ip);
				bad++;
			}
		}
	}
	return bad ? 1 : 0;
}

static int run_stdin(const struct v3d_device_info *devinfo)
{
	char line[256], name[128] = "";
	struct v3d_qpu_instr p[MAX_INSTR];
	uint64_t w[MAX_INSTR];
	int n = 0;
	bool last = false, have = false;
	int rc = 0;

	for (;;) {
		bool eof = !fgets(line, sizeof(line), stdin);

		if (eof || strncmp(line, "prog ", 5) == 0) {
			if (have) {
				struct result r;

				run(devinfo, p, n, last, -1, &r);
				print_line(name, devinfo->ver, last, -1, w, n, &r);
			}
			if (eof)
				break;
			{
				char flag[32] = "";

				if (sscanf(line + 5, "%127s %31s", name, flag) < 1) {
					fprintf(stderr, "hazards71: bad prog line: %s", line);
					return 2;
				}
				last = strcmp(flag, "last") == 0;
				have = true;
				n = 0;
			}
			continue;
		}
		if (line[0] == '\n' || line[0] == '#')
			continue;
		if (!have || n >= MAX_INSTR) {
			fprintf(stderr, "hazards71: word outside a program or too many: %s", line);
			return 2;
		}
		{
			char *end;

			w[n] = strtoull(line, &end, 16);
			if (end == line) {
				fprintf(stderr, "hazards71: not a word: %s", line);
				return 2;
			}
			memset(&p[n], 0, sizeof(p[n]));
			if (!v3d_qpu_instr_unpack(devinfo, w[n], &p[n])) {
				fprintf(stderr, "hazards71: %s: word %d does not unpack at ver %d\n",
					name, n, devinfo->ver);
				rc = 2;
				have = false;
				continue;
			}
			n++;
		}
	}
	return rc;
}

int main(int argc, char **argv)
{
	struct v3d_device_info devinfo;
	bool use_stdin = false;
	int ver = 71, i;

	for (i = 1; i < argc; i++) {
		if (strcmp(argv[i], "--stdin") == 0) {
			use_stdin = true;
		} else {
			fprintf(stderr, "usage: hazards71 [--stdin]\n");
			return 2;
		}
	}
	memset(&devinfo, 0, sizeof(devinfo));
	devinfo.ver = ver;
	if (use_stdin)
		return run_stdin(&devinfo);
	define_cases();
	return run_builtin(&devinfo);
}
