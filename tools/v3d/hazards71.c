/* SPDX-License-Identifier: MIT
 *
 * hazards71: Mesa's own verdicts for short V3D 7.1 instruction sequences, as an
 * oracle for the Rust verifier (src/shaderc/online/v3d/verify71.rs). Each
 * sequence is built here from instruction fields, packed with Mesa's
 * v3d_qpu_instr_pack at devinfo ver 71, unpacked again (so the validator sees
 * what a reader of the words would see) and run through Mesa 26.2.1's
 * qpu_validate() (src/broadcom/compiler/qpu_validate.c, compiled unchanged into
 * this binary through #include). One line per case on stdout:
 *
 *   <words> TAB <verdict> TAB <case name>
 *
 * <words>   the packed instructions, 16 hex digits each, separated by spaces.
 *           A leading token "multi" means the program was validated as a
 *           multi-segment one (c->last_thrsw set, qpu_validate.c:451-470);
 *           without it the compile is single-segment (c->last_thrsw NULL,
 *           last_thrsw_found starts true). A word followed by "/z" is a
 *           qinst with is_tlb_z_write set (qpu_validate.c:142-143). "-" means
 *           Mesa's packer refused the sequence (verdict "pack refused"), or,
 *           for a case named *_raw_*, that no word can carry it and the
 *           fields were validated as built.
 * <verdict> "ok", or "ip <n>: <message>" with the message of the first
 *           fail_instr() call (qpu_validate.c), n the instruction index
 *           (the instruction count for the two end-of-program rules).
 * <case>    group.name, the group naming the rule; a "# " comment above the
 *           case in this file cites the qpu_validate.c line.
 *
 * What the validator does not see at 7.1, and what the table therefore shows
 * as "ok": the rf0 implicit write of ldvary/ldunif/ldunifa and the rf0 flops
 * conflict (only qpu_schedule.c:716-724, 1692-1744, 2110-2118 know them), the
 * SFU ALU ops' latency (only magic writes to 19..24 count as SFU writes,
 * qpu_instr.c:618, and those are reserved at 7.1), and raddr read ports
 * (7.1 has one raddr per operand, so there is no port conflict to find;
 * qpu_pack.c packs each of the four independently). Those cases are in the
 * table to say so.
 *
 * The Mesa sources are compiled with -DNDEBUG (the packer asserts on some
 * words it accepts, see qpu_ref.c) and -DMESA_DEBUG=1 (qpu_validate() returns
 * at once otherwise). build: tools/v3d/hazards71.sh.
 */
#include <inttypes.h>
#include <limits.h>
#include <setjmp.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* qpu_validate.c includes "v3d_compiler.h", the compiler's whole header. Only
 * the containers it walks are needed, so take its include guard (v3d_compiler.h:24)
 * and declare those few types as Mesa does (v3d_compiler.h:145-168 qinst,
 * 512-519 qblock, 962 last_thrsw, 1648-1672 the vir_for_each_* macros;
 * vir.c:137-148 vir_is_add and vir_is_mul). */
#define V3D_COMPILER_H
#include "broadcom/common/v3d_device_info.h"
#include "broadcom/qpu/qpu_instr.h"
#include "util/list.h"

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

/* fail_instr() logs through mesa_loge() and then abort()s: catch both. The
 * dump it prints after the first line is dropped. */
static char first_msg[256];
static bool have_msg;
static jmp_buf fail_jmp;

static void hz_loge(const char *fmt, ...)
{
	va_list ap;
	if (have_msg)
		return;
	va_start(ap, fmt);
	vsnprintf(first_msg, sizeof first_msg, fmt, ap);
	va_end(ap);
	have_msg = true;
}

static void hz_abort(void)
{
	longjmp(fail_jmp, 1);
}

#define mesa_loge(...) hz_loge(__VA_ARGS__)
#define abort() hz_abort()
#include "broadcom/qpu/qpu_disasm.h"
#define ralloc_free(p)((void)(p))
#define v3d_qpu_decode(d, i) ((void)(d), (void)(i), "")

#include "compiler/qpu_validate.c"

#undef abort

/* ---- the sequences ---------------------------------------------------- */

typedef struct v3d_qpu_instr I;

static const struct v3d_device_info dev71 = { .ver = 71, .rev = 7 };

#define MAXN 16
static I prog[MAXN];
static bool zflag[MAXN];
static int nprog;
static bool multi;
static const char *case_name;
static int ncase;

static void begin(const char *name, bool is_multi)
{
	case_name = name;
	multi = is_multi;
	nprog = 0;
	memset(prog, 0, sizeof prog);
	memset(zflag, 0, sizeof zflag);
}

static I nop(void)
{
	I i;
	memset(&i, 0, sizeof i);
	i.type = V3D_QPU_INSTR_TYPE_ALU;
	i.alu.add.op = V3D_QPU_A_NOP;
	i.alu.add.waddr = V3D_QPU_WADDR_NOP;
	i.alu.add.magic_write = true;
	i.alu.mul.op = V3D_QPU_M_NOP;
	i.alu.mul.waddr = V3D_QPU_WADDR_NOP;
	i.alu.mul.magic_write = true;
	return i;
}

/* add: dst = op(rfa, rfb). magic selects a magic waddr instead of an rf. */
static I add(I i, enum v3d_qpu_add_op op, bool magic, int waddr, int a, int b)
{
	i.alu.add.op = op;
	i.alu.add.magic_write = magic;
	i.alu.add.waddr = waddr;
	i.alu.add.a.raddr = a;
	i.alu.add.b.raddr = b;
	return i;
}

static I mul(I i, enum v3d_qpu_mul_op op, bool magic, int waddr, int a, int b)
{
	i.alu.mul.op = op;
	i.alu.mul.magic_write = magic;
	i.alu.mul.waddr = waddr;
	i.alu.mul.a.raddr = a;
	i.alu.mul.b.raddr = b;
	return i;
}

static I thrsw(I i)
{
	i.sig.thrsw = true;
	return i;
}

static I ldvary(I i)
{
	i.sig.ldvary = true;
	return i;
}

static I ldunif(I i)
{
	i.sig.ldunif = true;
	return i;
}

static I ldunifa(I i)
{
	i.sig.ldunifa = true;
	return i;
}

/* ldunifrf and ldtmu write an rf (or a magic register) named by sig_addr. */
static I ldunifrf(I i, int rf)
{
	i.sig.ldunifrf = true;
	i.sig_addr = rf;
	return i;
}

static I ldtmu(I i, int rf)
{
	i.sig.ldtmu = true;
	i.sig_addr = rf;
	return i;
}

static I ldtlb(I i, int rf)
{
	i.sig.ldtlb = true;
	i.sig_addr = rf;
	return i;
}

static I branch(bool msf)
{
	I i;
	memset(&i, 0, sizeof i);
	i.type = V3D_QPU_INSTR_TYPE_BRANCH;
	i.branch.cond = msf ? V3D_QPU_BRANCH_COND_ALLA : V3D_QPU_BRANCH_COND_ALWAYS;
	i.branch.msfign = msf ? V3D_QPU_MSFIGN_P : V3D_QPU_MSFIGN_NONE;
	i.branch.bdi = V3D_QPU_BRANCH_DEST_REL;
	i.branch.bdu = V3D_QPU_BRANCH_DEST_REL;
	i.branch.offset = 0;
	return i;
}

static void emit(I i)
{
	prog[nprog++] = i;
}

static void emit_z(I i)
{
	zflag[nprog] = true;
	emit(i);
}

static void nops(int n)
{
	while (n-- > 0)
		emit(nop());
}

/* raw: validate the fields as built, without Mesa's packer (for a rule whose
 * input no 7.1 word can carry); the words column is then "-". */
static void finish(bool raw)
{
	uint64_t words[MAXN];
	struct qblock block;
	struct v3d_compile c;
	struct qinst q[MAXN];
	char verdict[320];
	int i;

	for (i = 0; i < nprog; i++) {
		if (raw) {
			words[i] = 0;
			memset(&q[i], 0, sizeof q[i]);
			q[i].qpu = prog[i];
			q[i].is_tlb_z_write = zflag[i];
			continue;
		}
		if (!v3d_qpu_instr_pack(&dev71, &prog[i], &words[i])) {
			printf("-\tpack refused\t%s\n", case_name);
			ncase++;
			return;
		}
		/* the validator reads what the words say */
		memset(&q[i], 0, sizeof q[i]);
		if (!v3d_qpu_instr_unpack(&dev71, words[i], &q[i].qpu)) {
			printf("-\tunpack refused\t%s\n", case_name);
			ncase++;
			return;
		}
		q[i].is_tlb_z_write = zflag[i];
	}

	memset(&c, 0, sizeof c);
	c.devinfo = &dev71;
	list_inithead(&c.blocks);
	list_inithead(&block.instructions);
	list_addtail(&block.link, &c.blocks);
	for (i = 0; i < nprog; i++)
		list_addtail(&q[i].link, &block.instructions);
	if (multi)
		c.last_thrsw = &q[0];

	have_msg = false;
	if (setjmp(fail_jmp) == 0) {
		qpu_validate(&c);
		strcpy(verdict, "ok");
	} else {
		int ip = -1, used = 0;
		char *p, *e;
		sscanf(first_msg, "v3d_qpu_validate at ip %d: %n", &ip, &used);
		p = first_msg + used;
		e = p + strlen(p);
		while (e > p && (e[-1] == '\n' || e[-1] == ':' || e[-1] == ' '))
			*--e = 0;
		snprintf(verdict, sizeof verdict, "ip %d: %s", ip, p);
	}

	if (multi)
		printf("multi ");
	if (raw)
		printf("-");
	for (i = 0; !raw && i < nprog; i++)
		printf("%s%016" PRIx64 "%s", i ? " " : "", words[i], zflag[i] ? "/z" : "");
	printf("\t%s\t%s\n", verdict, case_name);
	ncase++;
}

static void end(void)
{
	finish(false);
}

static void end_raw(void)
{
	finish(true);
}

int main(void)
{
	/* ---- small immediates (qpu_validate.c:199-218): one per operand, own
	 * unit, at most one per instruction. 4-bit raddr value 3 stands for the
	 * packed immediate. ---- */
	/* qpu_validate.c:204-208 */
	begin("smallimm.add_a_ok", false);
	{
		I i = add(nop(), V3D_QPU_A_ADD, false, 5, 3, 2);
		i.sig.small_imm_a = true;
		emit(i);
	}
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("smallimm.add_b_ok", false);
	{
		I i = add(nop(), V3D_QPU_A_ADD, false, 5, 1, 3);
		i.sig.small_imm_b = true;
		emit(i);
	}
	emit(thrsw(nop()));
	nops(2);
	end();
	/* qpu_validate.c:209-212 */
	begin("smallimm.mul_c_ok", false);
	{
		I i = mul(nop(), V3D_QPU_M_MOV, false, 5, 3, 0);
		i.sig.small_imm_c = true;
		emit(i);
	}
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("smallimm.mul_d_ok", false);
	{
		I i = mul(nop(), V3D_QPU_M_FMUL, false, 5, 1, 3);
		i.sig.small_imm_d = true;
		emit(i);
	}
	emit(thrsw(nop()));
	nops(2);
	end();
	/* qpu_validate.c:204-208: small imm a with no ADD */
	begin("smallimm.a_without_add_rejected", false);
	{
		I i = mul(nop(), V3D_QPU_M_MOV, false, 5, 1, 0);
		i.sig.small_imm_a = true;
		emit(i);
	}
	emit(thrsw(nop()));
	nops(2);
	end();
	/* qpu_validate.c:204-208: small imm b with no ADD */
	begin("smallimm.b_without_add_rejected", false);
	{
		I i = mul(nop(), V3D_QPU_M_FMUL, false, 5, 1, 2);
		i.sig.small_imm_b = true;
		emit(i);
	}
	emit(thrsw(nop()));
	nops(2);
	end();
	/* qpu_validate.c:209-212: small imm c with no MUL */
	begin("smallimm.c_without_mul_rejected", false);
	{
		I i = add(nop(), V3D_QPU_A_ADD, false, 5, 1, 2);
		i.sig.small_imm_c = true;
		emit(i);
	}
	emit(thrsw(nop()));
	nops(2);
	end();
	/* qpu_validate.c:209-212: small imm d with no MUL */
	begin("smallimm.d_without_mul_rejected", false);
	{
		I i = add(nop(), V3D_QPU_A_ADD, false, 5, 1, 2);
		i.sig.small_imm_d = true;
		emit(i);
	}
	emit(thrsw(nop()));
	nops(2);
	end();
	/* qpu_validate.c:213-217: two small immediates. The packer refuses the
	 * signal combination (the two _unpackable rows), so no word carries it and
	 * the fields are validated as built (words "-"). */
	begin("smallimm.a_and_d_raw_rejected", false);
	{
		I i = add(nop(), V3D_QPU_A_ADD, false, 5, 3, 2);
		i = mul(i, V3D_QPU_M_FMUL, false, 6, 1, 3);
		i.sig.small_imm_a = true;
		i.sig.small_imm_d = true;
		emit(i);
	}
	emit(thrsw(nop()));
	nops(2);
	end_raw();
	begin("smallimm.a_and_b_raw_rejected", false);
	{
		I i = add(nop(), V3D_QPU_A_ADD, false, 5, 3, 3);
		i.sig.small_imm_a = true;
		i.sig.small_imm_b = true;
		emit(i);
	}
	emit(thrsw(nop()));
	nops(2);
	end_raw();
	begin("smallimm.a_and_d_unpackable", false);
	{
		I i = add(nop(), V3D_QPU_A_ADD, false, 5, 3, 2);
		i = mul(i, V3D_QPU_M_FMUL, false, 6, 1, 3);
		i.sig.small_imm_a = true;
		i.sig.small_imm_d = true;
		emit(i);
	}
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("smallimm.a_and_b_unpackable", false);
	{
		I i = add(nop(), V3D_QPU_A_ADD, false, 5, 3, 3);
		i.sig.small_imm_a = true;
		i.sig.small_imm_b = true;
		emit(i);
	}
	emit(thrsw(nop()));
	nops(2);
	end();

	/* ---- ldvary / ldunif placement (qpu_validate.c:220-227), the rf0
	 * implicit writes, and the V3D 4.2-only GFXH-1633 rule (:229-250) ---- */
	/* qpu_validate.c:224-227 */
	begin("ldvary.then_ldunif_rejected", false);
	emit(ldvary(nop()));
	emit(ldunif(nop()));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("ldvary.then_ldunifa_rejected", false);
	emit(ldvary(nop()));
	emit(ldunifa(nop()));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("ldvary.nop_ldunif_ok", false);
	emit(ldvary(nop()));
	emit(nop());
	emit(ldunif(nop()));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("ldunif.then_ldvary_ok", false);
	emit(ldunif(nop()));
	emit(ldvary(nop()));
	emit(thrsw(nop()));
	nops(2);
	end();
	/* the test names ldunif and ldunifa only (:224-225): ldunifrf is not seen */
	begin("ldvary.then_ldunifrf_ok", false);
	emit(ldvary(nop()));
	emit(ldunifrf(nop(), 5));
	emit(thrsw(nop()));
	nops(2);
	end();
	/* qpu_validate.c:236: GFXH-1633 does not apply at 7.1 */
	begin("ldunif.then_ldunifa_ok_71", false);
	emit(ldunif(nop()));
	emit(ldunifa(nop()));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("ldunifa.then_ldunifrf_ok_71", false);
	emit(ldunifa(nop()));
	emit(ldunifrf(nop(), 5));
	emit(thrsw(nop()));
	nops(2);
	end();

	/* ---- rf0: only the scheduler knows (qpu_schedule.c:716-724) ---- */
	begin("rf0.ldvary_then_read_rf0_ok", false);
	emit(ldvary(nop()));
	emit(add(nop(), V3D_QPU_A_ADD, false, 5, 0, 1));
	emit(add(nop(), V3D_QPU_A_ADD, false, 6, 0, 1));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("rf0.ldunif_then_read_rf0_ok", false);
	emit(ldunif(nop()));
	emit(add(nop(), V3D_QPU_A_ADD, false, 5, 0, 1));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("rf0.ldvary_then_explicit_rf0_write_ok", false);
	emit(ldvary(nop()));
	emit(add(nop(), V3D_QPU_A_ADD, false, 0, 1, 2));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("rf0.ldunif_same_instruction_rf0_read_ok", false);
	emit(add(ldunif(nop()), V3D_QPU_A_ADD, false, 5, 0, 1));
	emit(thrsw(nop()));
	nops(2);
	end();

	/* ---- raddr: one port per operand at 7.1, nothing to conflict ---- */
	begin("raddr.four_distinct_ok", false);
	emit(mul(add(nop(), V3D_QPU_A_ADD, false, 5, 1, 2), V3D_QPU_M_FMUL, false, 6, 3, 4));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("raddr.same_ok", false);
	emit(mul(add(nop(), V3D_QPU_A_ADD, false, 5, 7, 7), V3D_QPU_M_FMUL, false, 6, 7, 7));
	emit(thrsw(nop()));
	nops(2);
	end();
	/* v3d_qpu_uses_mux (qpu_instr.c:1043) compares the union mux/raddr: a read
	 * of rf4 looks like a read of R4 after an SFU write, but there is no SFU
	 * write at 7.1 that qpu_validate sees (below). */
	begin("raddr.read_rf4_ok", false);
	emit(add(nop(), V3D_QPU_A_ADD, false, 5, 4, 4));
	emit(add(nop(), V3D_QPU_A_ADD, false, 5, 4, 4));
	emit(thrsw(nop()));
	nops(2);
	end();

	/* ---- SFU (qpu_validate.c:315-328, 290-296) ---- */
	/* 7.1's SFU is an ALU op with an rf/magic destination; qpu_validate counts
	 * only magic writes 19..24 as SFU writes (qpu_instr.c:618) */
	begin("sfu.alu_recip_then_read_ok", false);
	emit(add(nop(), V3D_QPU_A_RECIP, false, 5, 1, 0));
	emit(add(nop(), V3D_QPU_A_ADD, false, 6, 5, 1));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("sfu.alu_recip_then_recip_ok", false);
	emit(add(nop(), V3D_QPU_A_RECIP, false, 5, 1, 0));
	emit(add(nop(), V3D_QPU_A_RSQRT, false, 6, 2, 0));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("sfu.alu_recip_in_thrsw_slot_ok", false);
	emit(thrsw(nop()));
	emit(add(nop(), V3D_QPU_A_RECIP, false, 5, 1, 0));
	nops(2);
	end();
	/* qpu_validate.c:308-313: the 4.2 SFU waddrs are reserved at 7.1 */
	begin("sfu.magic_recip_rejected", false);
	emit(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_RECIP, 1, 0));
	emit(thrsw(nop()));
	nops(2);
	end();

	/* ---- reserved magic waddrs (qpu_validate.c:105-116 v3d71 table, 308-313) ---- */
	begin("waddr.r0_rejected", false);
	emit(add(nop(), V3D_QPU_A_ADD, true, V3D_QPU_WADDR_R0, 1, 2));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("waddr.r4_rejected", false);
	emit(mul(nop(), V3D_QPU_M_FMUL, true, V3D_QPU_WADDR_R4, 1, 2));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("waddr.tmul_rejected", false);
	emit(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_TMUL, 1, 0));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("waddr.vpm_rejected", false);
	emit(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_VPM, 1, 0));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("waddr.rep_rejected", false);
	emit(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_REP, 1, 0));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("waddr.quad_ok", false);
	emit(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_QUAD, 1, 0));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("waddr.tmud_ok", false);
	emit(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_TMUD, 1, 0));
	emit(thrsw(nop()));
	nops(2);
	end();
	/* qpu_validate.c:118-134: a NOP op's waddr is not looked at */
	begin("waddr.nop_op_reserved_waddr_ok", false);
	{
		I i = nop();
		i.alu.add.waddr = V3D_QPU_WADDR_R0;
		emit(i);
	}
	emit(thrsw(nop()));
	nops(2);
	end();

	/* ---- one of TMU/SFU/TSY/TLB/VPM per instruction, TMU write/read
	 * ordering (qpu_validate.c:330-344). ldtmu counts only at 4.2 (:338). ---- */
	begin("tmu.tmua_with_rf_ok", false);
	emit(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_TMUA, 1, 0));
	emit(nop());
	emit(ldtmu(nop(), 5));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("tmu.tmud_and_tmua_rejected", false);
	emit(mul(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_TMUD, 1, 0), V3D_QPU_M_MOV, true,
		 V3D_QPU_WADDR_TMUA, 2, 0));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("tmu.tmua_and_tlb_rejected", false);
	emit(mul(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_TMUA, 1, 0), V3D_QPU_M_MOV, true,
		 V3D_QPU_WADDR_TLB, 2, 0));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("tmu.tmua_and_tsy_rejected", false);
	emit(mul(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_TMUA, 1, 0), V3D_QPU_M_MOV, true,
		 V3D_QPU_WADDR_SYNC, 2, 0));
	emit(thrsw(nop()));
	nops(2);
	end();
	/* qpu_validate.c:338: ldtmu in the same instruction as a TMU write is fine at 7.1 */
	begin("tmu.ldtmu_with_tmua_ok_71", false);
	emit(ldtmu(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_TMUA, 1, 0), 5));
	emit(thrsw(nop()));
	nops(2);
	end();
	/* qpu_validate.c:339: ldtlb is counted */
	begin("tmu.ldtlb_with_tmua_rejected", false);
	emit(ldtlb(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_TMUA, 1, 0), 5));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("tmu.tmuwt_ok", false);
	emit(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_TMUA, 1, 0));
	emit(add(nop(), V3D_QPU_A_TMUWT, true, V3D_QPU_WADDR_NOP, 0, 0));
	emit(thrsw(nop()));
	nops(2);
	end();

	/* ---- thread switch and its delay slots (qpu_validate.c:290-306, 346-371) ---- */
	begin("thrsw.single_ok", false);
	nops(2);
	emit(thrsw(nop()));
	nops(2);
	end();
	/* qpu_validate.c:484 */
	begin("thrsw.missing_rejected", false);
	nops(5);
	end();
	/* qpu_validate.c:361: single segment, consecutive thrsw */
	begin("thrsw.two_in_a_row_single_rejected", false);
	emit(thrsw(nop()));
	emit(thrsw(nop()));
	nops(2);
	end();
	/* qpu_validate.c:364-367 */
	begin("thrsw.too_close_rejected", false);
	emit(thrsw(nop()));
	emit(nop());
	emit(thrsw(nop()));
	nops(2);
	end();
	/* qpu_validate.c:290-296: the SFU check cannot fire at 7.1; its rule is
	 * the one before it, the reserved waddr (above) */
	/* qpu_validate.c:299-306 (ver >= 71): ldvary in the first delay slot is fine,
	 * in the second it is not, in the third it is */
	begin("thrsw.ldvary_slot1_ok", false);
	nops(1);
	emit(thrsw(nop()));
	emit(ldvary(nop()));
	nops(2);
	end();
	begin("thrsw.ldvary_slot2_rejected", false);
	nops(1);
	emit(thrsw(nop()));
	emit(nop());
	emit(ldvary(nop()));
	nops(2);
	end();
	begin("thrsw.ldvary_slot3_ok", false);
	nops(1);
	emit(thrsw(nop()));
	nops(2);
	emit(ldvary(nop()));
	nops(1);
	end();
	begin("thrsw.ldvary_in_thrsw_instruction_ok", false);
	nops(1);
	emit(ldvary(thrsw(nop())));
	nops(3);
	end();
	/* multi-segment programs (qpu_validate.c:451-470, 474-484): a switch, the
	 * last-thrsw pair, and the program-end switch */
	begin("multi.pair_then_end_ok", true);
	nops(2);
	emit(thrsw(nop()));
	nops(3);
	emit(thrsw(nop()));
	emit(thrsw(nop()));
	nops(3);
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("multi.no_last_thrsw_rejected", true);
	nops(2);
	emit(thrsw(nop()));
	nops(3);
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("multi.no_end_thrsw_rejected", true);
	nops(2);
	emit(thrsw(nop()));
	nops(3);
	emit(thrsw(nop()));
	emit(thrsw(nop()));
	nops(3);
	end();
	/* qpu_validate.c:364: a third in a row is too close to the first */
	begin("multi.three_in_a_row_rejected", true);
	emit(thrsw(nop()));
	emit(thrsw(nop()));
	emit(thrsw(nop()));
	nops(3);
	end();
	/* a lone switch is not a program-end one in a multi-segment program */
	begin("multi.single_without_end_rejected", true);
	nops(2);
	emit(thrsw(nop()));
	nops(2);
	end();

	/* ---- the writes at and after THREND (qpu_validate.c:373-427) ---- */
	/* qpu_validate.c:380-384 */
	begin("thrend.add_rf_write_in_thrsw_rejected", false);
	emit(thrsw(add(nop(), V3D_QPU_A_ADD, false, 5, 1, 2)));
	nops(2);
	end();
	/* qpu_validate.c:397-401 */
	begin("thrend.mul_rf_write_in_thrsw_rejected", false);
	emit(thrsw(mul(nop(), V3D_QPU_M_FMUL, false, 5, 1, 2)));
	nops(2);
	end();
	begin("thrend.magic_write_in_thrsw_ok", false);
	emit(thrsw(add(nop(), V3D_QPU_A_ADD, true, V3D_QPU_WADDR_TMUA, 1, 2)));
	nops(2);
	end();
	/* qpu_validate.c:385-389 */
	begin("thrend.add_rf2_slot1_rejected", false);
	emit(thrsw(nop()));
	emit(add(nop(), V3D_QPU_A_ADD, false, 2, 1, 2));
	nops(1);
	end();
	begin("thrend.add_rf3_slot2_rejected", false);
	emit(thrsw(nop()));
	emit(nop());
	emit(add(nop(), V3D_QPU_A_ADD, false, 3, 1, 2));
	end();
	/* qpu_validate.c:403-407 */
	begin("thrend.mul_rf2_slot1_rejected", false);
	emit(thrsw(nop()));
	emit(mul(nop(), V3D_QPU_M_FMUL, false, 2, 1, 2));
	nops(1);
	end();
	begin("thrend.mul_rf3_slot2_rejected", false);
	emit(thrsw(nop()));
	emit(nop());
	emit(mul(nop(), V3D_QPU_M_FMUL, false, 3, 1, 2));
	end();
	/* qpu_validate.c:411-418: the signal's rf write */
	begin("thrend.sig_rf2_slot1_rejected", false);
	emit(thrsw(nop()));
	emit(ldtmu(nop(), 2));
	nops(1);
	end();
	begin("thrend.sig_rf3_slot2_rejected", false);
	emit(thrsw(nop()));
	emit(nop());
	emit(ldunifrf(nop(), 3));
	end();
	/* other rfs are fine at 7.1 (4.2: "RF write after THREND", :379, :396, :414) */
	begin("thrend.add_rf5_slot1_ok_71", false);
	emit(thrsw(nop()));
	emit(add(nop(), V3D_QPU_A_ADD, false, 5, 1, 2));
	nops(1);
	end();
	begin("thrend.mul_rf5_slot2_ok_71", false);
	emit(thrsw(nop()));
	emit(nop());
	emit(mul(nop(), V3D_QPU_M_FMUL, false, 5, 1, 2));
	end();
	begin("thrend.sig_rf5_slot1_ok_71", false);
	emit(thrsw(nop()));
	emit(ldtmu(nop(), 5));
	nops(1);
	end();
	/* the window is the thrsw and its two delay slots: rf2 in the third is fine */
	begin("thrend.add_rf2_slot3_ok", false);
	emit(thrsw(nop()));
	nops(2);
	emit(add(nop(), V3D_QPU_A_ADD, false, 2, 1, 2));
	end();
	begin("thrend.add_rf2_before_thrsw_ok", false);
	emit(add(nop(), V3D_QPU_A_ADD, false, 2, 1, 2));
	emit(thrsw(nop()));
	nops(2);
	end();
	/* qpu_validate.c:422-426: GFXH-1625 (TMUWT in the last instruction) is 4.2 only */
	begin("thrend.tmuwt_last_slot_ok_71", false);
	emit(thrsw(nop()));
	emit(nop());
	emit(add(nop(), V3D_QPU_A_TMUWT, true, V3D_QPU_WADDR_NOP, 0, 0));
	end();
	/* the "at THREND" case in a multi-segment program: only the thrsw after
	 * the last-thrsw pair is THREND (:355-357 and :373) */
	begin("multi.rf_write_in_mid_thrsw_ok", true);
	emit(thrsw(add(nop(), V3D_QPU_A_ADD, false, 2, 1, 2)));
	nops(3);
	emit(thrsw(nop()));
	emit(thrsw(nop()));
	nops(3);
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("multi.rf_write_in_end_thrsw_rejected", true);
	emit(thrsw(nop()));
	nops(3);
	emit(thrsw(nop()));
	emit(thrsw(nop()));
	nops(3);
	emit(thrsw(add(nop(), V3D_QPU_A_ADD, false, 5, 1, 2)));
	nops(2);
	end();

	/* ---- rtop: MULTOP/UMUL24 (qpu_validate.c:168-185, 429-432) ---- */
	begin("rtop.multop_umul24_ok", false);
	emit(mul(nop(), V3D_QPU_M_MULTOP, false, 5, 1, 2));
	emit(mul(nop(), V3D_QPU_M_UMUL24, false, 6, 1, 2));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("rtop.cleared_by_thrsw_rejected", false);
	emit(mul(nop(), V3D_QPU_M_MULTOP, false, 5, 1, 2));
	emit(thrsw(nop()));
	nops(2);
	emit(mul(nop(), V3D_QPU_M_UMUL24, false, 6, 1, 2));
	end();
	/* the UMUL24 one slot earlier is still fine: the hazard is set after the
	 * second delay slot */
	begin("rtop.umul24_in_slot2_ok", false);
	emit(mul(nop(), V3D_QPU_M_MULTOP, false, 5, 1, 2));
	emit(thrsw(nop()));
	emit(nop());
	emit(mul(nop(), V3D_QPU_M_UMUL24, false, 6, 1, 2));
	end();
	begin("rtop.no_multop_ok", false);
	emit(thrsw(nop()));
	nops(2);
	emit(mul(nop(), V3D_QPU_M_UMUL24, false, 6, 1, 2));
	end();

	/* ---- branches and delay slots (qpu_validate.c:156-163, 349-352) ---- */
	begin("branch.then_thrsw_after_slots_ok", false);
	emit(branch(false));
	nops(3);
	emit(thrsw(nop()));
	nops(2);
	end();
	/* qpu_validate.c:349-352 */
	begin("branch.thrsw_in_delay_slot_rejected", false);
	emit(branch(false));
	nops(1);
	emit(thrsw(nop()));
	nops(2);
	end();
	/* qpu_validate.c:158-159 */
	begin("branch.in_branch_delay_slot_rejected", false);
	emit(branch(false));
	nops(2);
	emit(branch(false));
	nops(1);
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("branch.after_branch_slots_ok", false);
	emit(branch(false));
	nops(3);
	emit(branch(false));
	nops(3);
	emit(thrsw(nop()));
	nops(2);
	end();
	/* qpu_validate.c:160-161 */
	begin("branch.in_thrsw_delay_slot_rejected", false);
	emit(thrsw(nop()));
	emit(branch(false));
	nops(2);
	end();
	begin("branch.after_thrsw_slots_ok", false);
	emit(thrsw(nop()));
	nops(2);
	emit(branch(false));
	nops(3);
	end();

	/* ---- MSF after a TLB Z write (qpu_validate.c:142-154, 188-197) ---- */
	begin("tlbz.msf_before_ok", false);
	emit(add(nop(), V3D_QPU_A_MSF, false, 5, 0, 0));
	emit_z(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_TLB, 1, 0));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("tlbz.msf_after_rejected", false);
	emit_z(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_TLB, 1, 0));
	emit(add(nop(), V3D_QPU_A_MSF, false, 5, 0, 0));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("tlbz.setmsf_after_rejected", false);
	emit_z(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_TLB, 1, 0));
	emit(add(nop(), V3D_QPU_A_SETMSF, false, 5, 1, 0));
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("tlbz.msf_branch_after_rejected", false);
	emit_z(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_TLB, 1, 0));
	emit(branch(true));
	nops(3);
	emit(thrsw(nop()));
	nops(2);
	end();
	begin("tlbz.plain_branch_after_ok", false);
	emit_z(add(nop(), V3D_QPU_A_MOV, true, V3D_QPU_WADDR_TLB, 1, 0));
	emit(branch(false));
	nops(3);
	emit(thrsw(nop()));
	nops(2);
	end();

	fprintf(stderr, "hazards71: %d cases\n", ncase);
	return 0;
}
