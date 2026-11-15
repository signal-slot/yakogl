/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The part of Mesa 26.2.1's src/broadcom/compiler/v3d_compiler.h that
 * qpu_validate.c uses, so that Mesa's own qpu_validate.c (copied byte for
 * byte, never edited) compiles without the NIR and VIR machinery around it.
 * tools/v3d/hazards71.sh puts this header next to that copy, where the
 * file's `#include "v3d_compiler.h"` finds it before Mesa's.
 *
 * Mesa's definitions it stands in for (MIT):
 *   struct qinst, struct qblock, struct v3d_compile  v3d_compiler.h
 *   vir_for_each_block / _inst / _inst_inorder       v3d_compiler.h:1648-1672
 *   vir_is_add, vir_is_mul                           vir.c:137-148
 * Two replacements, both on the failure path only: mesa_loge records the
 * first message (fail_instr's "v3d_qpu_validate at ip %d: %s:") instead of
 * printing it, and abort() leaves qpu_validate through longjmp instead of
 * ending the process. Neither changes which instruction is refused or why.
 */
#ifndef HAZARDS71_V3D_COMPILER_H
#define HAZARDS71_V3D_COMPILER_H

#include <limits.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

#include "common/v3d_device_info.h"
#include "qpu/qpu_instr.h"
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
	struct qinst *last_thrsw;
	struct list_head blocks;
};

#define vir_for_each_block(block, c) \
	list_for_each_entry(struct qblock, block, &c->blocks, link)
#define vir_for_each_inst(inst, block) \
	list_for_each_entry(struct qinst, inst, &block->instructions, link)
#define vir_for_each_inst_inorder(inst, c) \
	vir_for_each_block(_block, c) \
		vir_for_each_inst(inst, _block)

static inline bool vir_is_add(struct qinst *inst)
{
	return inst->qpu.type == V3D_QPU_INSTR_TYPE_ALU &&
	       inst->qpu.alu.add.op != V3D_QPU_A_NOP;
}

static inline bool vir_is_mul(struct qinst *inst)
{
	return inst->qpu.type == V3D_QPU_INSTR_TYPE_ALU &&
	       inst->qpu.alu.mul.op != V3D_QPU_M_NOP;
}

void hz_loge(const char *fmt, ...);
void hz_abort(void) __attribute__((noreturn));
#define mesa_loge hz_loge
#define abort hz_abort

void qpu_validate(struct v3d_compile *c);

#endif
