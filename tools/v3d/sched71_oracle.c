// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//
// sched71_oracle: Mesa 26.2.1's QPU scheduler (src/broadcom/compiler/
// qpu_schedule.c) at devinfo->ver = 71, run on a fixed set of instruction
// pairs; one TSV row per pair on stdout. Built and run by sched71_oracle.sh,
// whose header documents the columns, the pair set, the Mesa citations and
// how to regenerate tools/v3d/sched71_oracle.tsv.
//
// The scheduler's functions are static, so this file includes
// qpu_schedule.c (MIT, Broadcom/Intel) as it is; nothing of Mesa is copied
// here. Two things Mesa defines in files this harness does not link are
// stood in for below: vir_add_inst() (vir.c:375), which qpu_schedule.c's
// vir_nop() calls for its NOPs, and v3d_qpu_nop() (vir_to_qpu.c:49).

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "qpu/qpu_disasm.h"
#include "v3d_compiler.h"
#include "util/ralloc.h"
#include "util/dag.h"

// qpu_schedule.c frees the qinst of an instruction it merged into another
// (schedule_instructions, emit_thrsw). The harness finds each instruction of
// the pair in the result by its qinst pointer, so a freed qinst must not be
// handed out again by a later calloc: free() inside qpu_schedule.c only
// records the pointer. The headers above are include-guarded, so the macro
// reaches qpu_schedule.c's own code only.
static void *freed[8];
static int nfreed;
static void
oracle_free(void *p)
{
        if (nfreed < (int)ARRAY_SIZE(freed))
                freed[nfreed++] = p;
}
#define free(p) oracle_free(p)
#include "qpu_schedule.c"
#undef free

struct v3d_qpu_instr
v3d_qpu_nop(void)
{
        struct v3d_qpu_instr nop;

        memset(&nop, 0, sizeof(nop));
        nop.type = V3D_QPU_INSTR_TYPE_ALU;
        nop.alu.add.op = V3D_QPU_A_NOP;
        nop.alu.add.waddr = V3D_QPU_WADDR_NOP;
        nop.alu.add.magic_write = true;
        nop.alu.mul.op = V3D_QPU_M_NOP;
        nop.alu.mul.waddr = V3D_QPU_WADDR_NOP;
        nop.alu.mul.magic_write = true;
        return nop;
}

struct qinst *
vir_add_inst(enum v3d_qpu_add_op op, struct qreg dst, struct qreg src0,
             struct qreg src1)
{
        struct qinst *q = calloc(1, sizeof(*q));

        q->qpu = v3d_qpu_nop();
        q->qpu.alu.add.op = op;
        q->dst = dst;
        q->src[0] = src0;
        q->src[1] = src1;
        q->uniform = ~0;
        q->ip = -1;
        return q;
}

static struct v3d_device_info devinfo = {
        .ver = 71,
        .has_accumulators = false,
};

// The instruction forms of the pairs: the ops and signals the 7.1 back end
// emits (docs/notes/v3d71-backend.md). Each is built the way vir_to_qpu.c
// leaves it for qpu_schedule.c: ALU ops with register-file operands
// (raddr) and destination, small immediates packed into the operand's raddr
// and raddr_b with the matching small_imm_a..d signal bit (vir_to_qpu.c:90),
// the SFU as add-ALU ops (qpu_instr.c:788), magic writes as the mul-ALU MOV
// that vir_MOV_dest emits, signals on a NOP word.
enum form {
        F_FADD, F_ADD, F_SUB, F_AMOV, F_AFMOV,
        F_RECIP, F_RSQRT, F_EXP, F_LOG, F_SIN,
        F_FADD_SIA, F_FADD_SIB, F_ADD_SIB,
        F_FMUL, F_MMOV, F_MFMOV, F_FMUL_SIC, F_FMUL_SID, F_MMOV_SIC,
        F_LDUNIF, F_LDUNIFRF, F_LDVARY, F_LDTMU, F_THRSW, F_WRTMUC,
        F_TMUD, F_TMUT, F_TMUS, F_TMUA, F_TLB, F_TLBU,
        F_COUNT,
};

enum unit { U_ADD, U_MUL, U_SIG, U_MAGIC };

struct form_desc {
        const char *name;
        enum unit unit;
        int op;          // add or mul opcode, U_ADD/U_MUL/U_MAGIC
        int nrf;         // register-file sources (excluding small immediates)
        int imm_slot;    // -1, or 0..3: small_imm_a..d
        uint32_t imm;    // the immediate's value
        bool dst;        // writes a register-file destination
        bool rf0;        // writes rf0 implicitly (qpu_instr.c:1031)
        uint32_t waddr;  // magic waddr, U_MAGIC
};

#define F1 0x3f800000u

static const struct form_desc forms[F_COUNT] = {
        [F_FADD]     = { "fadd", U_ADD, V3D_QPU_A_FADD, 2, -1, 0, true },
        [F_ADD]      = { "add", U_ADD, V3D_QPU_A_ADD, 2, -1, 0, true },
        [F_SUB]      = { "sub", U_ADD, V3D_QPU_A_SUB, 2, -1, 0, true },
        [F_AMOV]     = { "mov.a", U_ADD, V3D_QPU_A_MOV, 1, -1, 0, true },
        [F_AFMOV]    = { "fmov.a", U_ADD, V3D_QPU_A_FMOV, 1, -1, 0, true },
        [F_RECIP]    = { "recip", U_ADD, V3D_QPU_A_RECIP, 1, -1, 0, true },
        [F_RSQRT]    = { "rsqrt", U_ADD, V3D_QPU_A_RSQRT, 1, -1, 0, true },
        [F_EXP]      = { "exp", U_ADD, V3D_QPU_A_EXP, 1, -1, 0, true },
        [F_LOG]      = { "log", U_ADD, V3D_QPU_A_LOG, 1, -1, 0, true },
        [F_SIN]      = { "sin", U_ADD, V3D_QPU_A_SIN, 1, -1, 0, true },
        [F_FADD_SIA] = { "fadd.si_a", U_ADD, V3D_QPU_A_FADD, 1, 0, F1, true },
        [F_FADD_SIB] = { "fadd.si_b", U_ADD, V3D_QPU_A_FADD, 1, 1, F1, true },
        [F_ADD_SIB]  = { "add.si_b", U_ADD, V3D_QPU_A_ADD, 1, 1, 1, true },
        [F_FMUL]     = { "fmul", U_MUL, V3D_QPU_M_FMUL, 2, -1, 0, true },
        [F_MMOV]     = { "mov.m", U_MUL, V3D_QPU_M_MOV, 1, -1, 0, true },
        [F_MFMOV]    = { "fmov.m", U_MUL, V3D_QPU_M_FMOV, 1, -1, 0, true },
        [F_FMUL_SIC] = { "fmul.si_c", U_MUL, V3D_QPU_M_FMUL, 1, 2, F1, true },
        [F_FMUL_SID] = { "fmul.si_d", U_MUL, V3D_QPU_M_FMUL, 1, 3, F1, true },
        [F_MMOV_SIC] = { "mov.m.si_c", U_MUL, V3D_QPU_M_MOV, 0, 2, 1, true },
        [F_LDUNIF]   = { "ldunif", U_SIG, 0, 0, -1, 0, false, true },
        [F_LDUNIFRF] = { "ldunifrf", U_SIG, 0, 0, -1, 0, true },
        [F_LDVARY]   = { "ldvary", U_SIG, 0, 0, -1, 0, true, true },
        [F_LDTMU]    = { "ldtmu", U_SIG, 0, 0, -1, 0, true },
        [F_THRSW]    = { "thrsw", U_SIG, 0, 0, -1, 0, false },
        [F_WRTMUC]   = { "wrtmuc", U_SIG, 0, 0, -1, 0, false },
        [F_TMUD]     = { "tmud", U_MAGIC, V3D_QPU_M_MOV, 1, -1, 0, false, false, V3D_QPU_WADDR_TMUD },
        [F_TMUT]     = { "tmut", U_MAGIC, V3D_QPU_M_MOV, 1, -1, 0, false, false, V3D_QPU_WADDR_TMUT },
        [F_TMUS]     = { "tmus", U_MAGIC, V3D_QPU_M_MOV, 1, -1, 0, false, false, V3D_QPU_WADDR_TMUS },
        [F_TMUA]     = { "tmua", U_MAGIC, V3D_QPU_M_MOV, 1, -1, 0, false, false, V3D_QPU_WADDR_TMUA },
        [F_TLB]      = { "tlb", U_MAGIC, V3D_QPU_M_MOV, 1, -1, 0, false, false, V3D_QPU_WADDR_TLB },
        [F_TLBU]     = { "tlbu", U_MAGIC, V3D_QPU_M_MOV, 1, -1, 0, false, false, V3D_QPU_WADDR_TLBU },
};

static void
die(const char *what, const char *detail)
{
        fprintf(stderr, "sched71_oracle: %s%s%s\n", what, detail ? ": " : "",
                detail ? detail : "");
        exit(1);
}

// One instruction of form f writing rf[dst] (when it has a destination) and
// reading rf[src[0]], rf[src[1]] in its register-file operands, in order.
// uniform is the instruction's index into the uniform stream (ldunif,
// ldunifrf, wrtmuc), as qinst->uniform holds it.
static struct qinst *
build(enum form f, int dst, const int src[2], int uniform)
{
        const struct form_desc *d = &forms[f];
        struct qinst *q = vir_add_inst(V3D_QPU_A_NOP, vir_nop_reg(),
                                       vir_nop_reg(), vir_nop_reg());
        struct v3d_qpu_instr *i = &q->qpu;
        struct v3d_qpu_input *in[2];
        int nsrc = d->nrf + (d->imm_slot >= 0);

        switch (d->unit) {
        case U_ADD:
                i->alu.add.op = d->op;
                i->alu.add.waddr = dst;
                i->alu.add.magic_write = false;
                in[0] = &i->alu.add.a;
                in[1] = &i->alu.add.b;
                break;
        case U_MUL:
        case U_MAGIC:
                i->alu.mul.op = d->op;
                i->alu.mul.waddr = d->unit == U_MAGIC ? d->waddr : dst;
                i->alu.mul.magic_write = d->unit == U_MAGIC;
                in[0] = &i->alu.mul.a;
                in[1] = &i->alu.mul.b;
                break;
        case U_SIG:
                in[0] = in[1] = NULL;
                break;
        }

        if (d->unit != U_SIG) {
                int next = 0;
                for (int s = 0; s < nsrc; s++) {
                        // Operand s of the add ALU is small_imm_a/b, of the
                        // mul ALU small_imm_c/d.
                        int slot = (d->unit == U_ADD ? 0 : 2) + s;
                        if (slot == d->imm_slot) {
                                uint32_t packed;
                                if (!v3d_qpu_small_imm_pack(&devinfo, d->imm,
                                                            &packed))
                                        die("small immediate", d->name);
                                in[s]->raddr = packed;
                                i->raddr_b = packed;
                                i->sig.small_imm_a |= slot == 0;
                                i->sig.small_imm_b |= slot == 1;
                                i->sig.small_imm_c |= slot == 2;
                                i->sig.small_imm_d |= slot == 3;
                        } else {
                                in[s]->raddr = src[next++];
                        }
                }
        }

        switch (f) {
        case F_LDUNIF:
                i->sig.ldunif = true;
                q->uniform = uniform;
                break;
        case F_LDUNIFRF:
                i->sig.ldunifrf = true;
                i->sig_addr = dst;
                q->uniform = uniform;
                break;
        case F_LDVARY:
                i->sig.ldvary = true;
                i->sig_addr = dst;
                break;
        case F_LDTMU:
                i->sig.ldtmu = true;
                i->sig_addr = dst;
                break;
        case F_THRSW:
                i->sig.thrsw = true;
                break;
        case F_WRTMUC:
                i->sig.wrtmuc = true;
                q->uniform = uniform;
                break;
        case F_TMUS:
        case F_TMUA:
                // The lookup's terminator: one ldtmu reads its result.
                q->ldtmu_count = 1;
                break;
        default:
                break;
        }

        uint64_t packed;
        if (!v3d_qpu_instr_pack(&devinfo, i, &packed))
                die("form does not pack at 7.1", d->name);
        return q;
}

// Mesa's disassembly of one word (pack, then v3d_qpu_disasm), with its
// column padding (runs of spaces or tabs) printed as one space and none at
// the end, so that it stays one compact TSV field.
static void
print_disasm(const struct v3d_qpu_instr *i)
{
        uint64_t packed;
        if (!v3d_qpu_instr_pack(&devinfo, i, &packed))
                die("a scheduled word does not pack", NULL);
        const char *s = v3d_qpu_disasm(&devinfo, packed);
        bool space = false, started = false;
        for (const char *p = s; *p; p++) {
                if (*p == ' ' || *p == '\t') {
                        space = started;
                        continue;
                }
                if (space)
                        putchar(' ');
                space = false;
                started = true;
                putchar(*p);
        }
        ralloc_free((void *)s);
}

enum kind { K_IND, K_RAW, K_RAW0, K_WAR, K_WAW };
static const char *const kind_name[] = { "ind", "raw", "raw0", "war", "waw" };

// Registers: the producer writes rf10 (raw, war, waw) and the consumer
// reads it; every other operand is a register nothing else in the pair
// touches. rf0 is the implicit destination of ldunif and ldvary.
#define R_X 10

static int run_count;

// The word of the scheduled block that holds an instruction of the pair, -1
// when it is not found. A thrsw may be merged back into an earlier word
// (emit_thrsw), an ldvary moved into the previous one
// (fixup_pipelined_ldvary): those are found by their signal (an ldvary also
// by its destination), the first such word for the pair's first
// instruction, the last for its second. Anything else is found by its
// qinst; a qinst that qpu_schedule.c freed was merged into the other
// instruction's word, which the caller resolves.
static int
word_of(struct qblock *block, struct qinst *q, enum form f,
        const struct v3d_qpu_instr *orig, bool first)
{
        int n = 0, found = -1;

        list_for_each_entry(struct qinst, w, &block->instructions, link) {
                const struct v3d_qpu_instr *i = &w->qpu;
                bool hit;
                if (f == F_THRSW)
                        hit = i->type == V3D_QPU_INSTR_TYPE_ALU && i->sig.thrsw;
                else if (f == F_LDVARY)
                        hit = i->type == V3D_QPU_INSTR_TYPE_ALU &&
                              i->sig.ldvary && i->sig_addr == orig->sig_addr;
                else
                        hit = w == q;
                if (hit && (found < 0 || !first))
                        found = n;
                n++;
        }
        return found;
}

static bool
was_freed(struct qinst *q)
{
        for (int k = 0; k < nfreed; k++) {
                if (freed[k] == q)
                        return true;
        }
        return false;
}

static void
run_pair(enum kind k, enum form fa, enum form fb)
{
        int a_dst = 0, b_dst = 0;
        int a_src[2] = { 1, 2 }, b_src[2] = { 3, 4 };

        switch (k) {
        case K_IND:
                a_dst = 11;
                b_dst = 12;
                break;
        case K_RAW:
                a_dst = R_X;
                b_dst = 12;
                b_src[0] = R_X;
                break;
        case K_RAW0:
                a_dst = 11;
                b_dst = 12;
                b_src[0] = 0;
                break;
        case K_WAR:
                a_dst = 11;
                a_src[0] = R_X;
                b_dst = R_X;
                break;
        case K_WAW:
                a_dst = R_X;
                b_dst = R_X;
                break;
        }
        nfreed = 0;
        void *mem_ctx = ralloc_context(NULL);
        struct v3d_compile *c = rzalloc(mem_ctx, struct v3d_compile);
        nir_shader *s = rzalloc(mem_ctx, nir_shader);
        struct qblock *block = rzalloc(mem_ctx, struct qblock);
        enum quniform_contents contents[4] = { QUNIFORM_CONSTANT,
                QUNIFORM_CONSTANT, QUNIFORM_CONSTANT, QUNIFORM_CONSTANT };
        uint32_t data[4] = { 0, 0, 0, 0 };
        uint32_t next_uniform = 0;

        // The compile the scheduler sees: a fragment shader at 4 threads,
        // no ldvary pipelining (num_inputs 0), the scoreboard locked on the
        // last thrsw.
        s->info.stage = MESA_SHADER_FRAGMENT;
        c->s = s;
        c->devinfo = &devinfo;
        c->threads = 4;
        c->num_inputs = 0;
        c->lock_scoreboard_on_first_thrsw = false;
        c->uniform_contents = rzalloc_array(mem_ctx, enum quniform_contents, 4);
        c->uniform_data = rzalloc_array(mem_ctx, uint32_t, 4);
        c->uniform_array_size = 4;
        list_inithead(&block->instructions);

        // v3d_qpu_schedule_instructions()'s initial scoreboard
        // (qpu_schedule.c:3033-3044), then the pair's context: after the
        // program's last thrsw (TLB access allowed, qpu_schedule.c:740) and
        // with one TMU lookup whose two results are not read yet (so that an
        // ldtmu has a result to read, qpu_schedule.c:1685).
        struct choose_scoreboard sb;
        memset(&sb, 0, sizeof(sb));
        sb.last_ldvary_tick = -10;
        sb.last_unifa_write_tick = -10;
        sb.last_magic_sfu_write_tick = -10;
        sb.last_uniforms_reset_tick = -10;
        sb.last_thrsw_tick = -10;
        sb.last_branch_tick = -10;
        sb.last_setmsf_tick = -10;
        sb.last_stallable_sfu_tick = -10;
        sb.first_ldtmu_after_thrsw = true;
        sb.last_implicit_rf0_write_tick = -10;
        sb.first_thrsw_emitted = true;
        sb.last_thrsw_emitted = true;
        sb.pending_ldtmu_count = 2;

        struct qinst *qa = build(fa, a_dst, a_src, 0);
        struct qinst *qb = build(fb, b_dst, b_src, 1);
        struct v3d_qpu_instr ia = qa->qpu, ib = qb->qpu;

        // qpu_schedule_instructions_block() (qpu_schedule.c:2873), with the
        // DAG kept so that the A->B edge and its latency can be read.
        sb.dag = dag_create(mem_ctx);
        struct list_head setup_list;
        list_inithead(&setup_list);
        struct schedule_node *na = rzalloc(mem_ctx, struct schedule_node);
        struct schedule_node *nb = rzalloc(mem_ctx, struct schedule_node);
        dag_init_node(sb.dag, &na->dag);
        dag_init_node(sb.dag, &nb->dag);
        na->inst = qa;
        nb->inst = qb;
        list_addtail(&na->link, &setup_list);
        list_addtail(&nb->link, &setup_list);
        calculate_forward_deps(c, sb.dag, &setup_list);
        calculate_reverse_deps(c, sb.dag, &setup_list);

        bool order = false, war = false;
        util_dynarray_foreach(&na->dag.edges, struct dag_edge, e) {
                if (e->child != &nb->dag)
                        continue;
                if (e->data)
                        war = true;
                else
                        order = true;
        }
        const char *dep = order ? "order" : war ? "war" : "none";
        uint32_t latency = instruction_latency(&devinfo, na, nb);

        struct v3d_qpu_instr merged;
        bool merge_inst = qpu_merge_inst(&devinfo, &merged, &ia, &ib);

        dag_traverse_bottom_up(sb.dag, compute_delay, c);

        // One NOP already scheduled ahead of the pair, so that
        // fixup_pipelined_ldvary() (qpu_schedule.c:2651) has a previous
        // word; left out when the pair has a thrsw, which emit_thrsw()
        // would otherwise move into it (qpu_schedule.c:2283).
        bool prefix = fa != F_THRSW && fb != F_THRSW;
        if (prefix)
                insert_scheduled_instruction(c, block, &sb, vir_nop());

        schedule_instructions(c, &sb, block, contents, data, &next_uniform);

        int pa = word_of(block, qa, fa, &ia, true);
        int pb = word_of(block, qb, fb, &ib, false);
        if (pa < 0 && was_freed(qa))
                pa = pb;
        else if (pb < 0 && was_freed(qb))
                pb = pa;
        if (pa < 0 || pb < 0)
                die("could not place the pair in the schedule",
                    forms[fa].name);

        printf("%s.%s.%s\t%s\t", kind_name[k], forms[fa].name, forms[fb].name,
               kind_name[k]);
        print_disasm(&ia);
        putchar('\t');
        print_disasm(&ib);
        printf("\t%s\t%u\t%s\t%s\t%d\t", dep, latency,
               merge_inst ? "yes" : "no", pa == pb ? "yes" : "no", pb - pa);
        int n = 0;
        list_for_each_entry(struct qinst, w, &block->instructions, link) {
                if (n++)
                        printf(" | ");
                print_disasm(&w->qpu);
        }
        putchar('\n');
        run_count++;

        ralloc_free(mem_ctx);
}

static bool
reads_rf(enum form f)
{
        return forms[f].nrf > 0;
}

static bool
writes_rf(enum form f)
{
        return forms[f].dst;
}

int
main(void)
{
        printf("# tools/v3d/sched71_oracle.tsv: Mesa 26.2.1 qpu_schedule.c at "
               "ver 71 on instruction pairs.\n"
               "# Generated by tools/v3d/sched71_oracle.sh (its header "
               "documents the columns); do not edit.\n");
        printf("id\tkind\ta\tb\tdep\tlatency\tmerge_inst\tmerge\tdist\t"
               "schedule\n");

        // ind: every form against every form, independent registers: the
        // pairing verdicts (one thrsw per pair).
        for (int a = 0; a < F_COUNT; a++) {
                for (int b = 0; b < F_COUNT; b++) {
                        if (a == F_THRSW && b == F_THRSW)
                                continue;
                        run_pair(K_IND, a, b);
                }
        }
        // raw: every form writing a register-file destination, then every
        // form reading one, the second reading the first's destination.
        for (int a = 0; a < F_COUNT; a++) {
                for (int b = 0; b < F_COUNT; b++) {
                        if (writes_rf(a) && reads_rf(b))
                                run_pair(K_RAW, a, b);
                }
        }
        // raw0: the implicit rf0 writers (ldunif, ldvary), then every form
        // reading rf0.
        for (int a = 0; a < F_COUNT; a++) {
                for (int b = 0; b < F_COUNT; b++) {
                        if (forms[a].rf0 && reads_rf(b))
                                run_pair(K_RAW0, a, b);
                }
        }
        // war, waw: every reader (writer), then every writer.
        for (int a = 0; a < F_COUNT; a++) {
                for (int b = 0; b < F_COUNT; b++) {
                        if (reads_rf(a) && writes_rf(b))
                                run_pair(K_WAR, a, b);
                }
        }
        for (int a = 0; a < F_COUNT; a++) {
                for (int b = 0; b < F_COUNT; b++) {
                        if (writes_rf(a) && writes_rf(b))
                                run_pair(K_WAW, a, b);
                }
        }
        printf("# %d pairs\n", run_count);
        return 0;
}
