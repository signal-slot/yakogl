// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/*
 * The C side of the V3D QPU FFI (qpu_ffi.rs): what the Rust wrapper needs
 * from Mesa 26.2.1's qpu_pack.c / qpu_instr.c (third_party/mesa-26.2.1,
 * compiled unmodified next to this file by build.rs) that is not a plain
 * function call.
 *
 * - The device the words are packed for: V3D 4.2, the Raspberry Pi 4.
 * - The layout of `struct v3d_qpu_instr` and of what it nests, as sizeof /
 *   offsetof values read from the real header, which the Rust mirror is
 *   checked against (qpu_ffi.rs, `the_mirror_is_mesas_layout`).
 * - The byte image of `struct v3d_qpu_sig` with each bit-field set alone
 *   (C leaves bit-field placement to the ABI, so it is measured, not
 *   assumed), and the values of the enums Mesa has no name function for.
 * - v3d_qpu_magic_waddr_name bounded to its table.
 */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include "broadcom/common/v3d_device_info.h"
#include "broadcom/qpu/qpu_instr.h"

/* The fields the QPU packer reads: the version (ver 42 selects the 4.2
 * tables and field layout) and has_accumulators (true before 7.1; Mesa's
 * v3d_get_device_info sets it from the version the same way). */
const struct v3d_device_info yakogl_v3d42_devinfo = {
        .ver = 42,
        .has_accumulators = true,
};

#define OFF(type, member) ((uint32_t)offsetof(type, member))

/* The order is the Rust mirror's `layout()`; one list, compared whole. */
const uint32_t yakogl_v3d_qpu_layout[] = {
        (uint32_t)sizeof(struct v3d_qpu_instr),
        (uint32_t)_Alignof(struct v3d_qpu_instr),
        OFF(struct v3d_qpu_instr, type),
        OFF(struct v3d_qpu_instr, sig),
        OFF(struct v3d_qpu_instr, sig_addr),
        OFF(struct v3d_qpu_instr, sig_magic),
        OFF(struct v3d_qpu_instr, raddr_a),
        OFF(struct v3d_qpu_instr, raddr_b),
        OFF(struct v3d_qpu_instr, flags),
        OFF(struct v3d_qpu_instr, alu),
        OFF(struct v3d_qpu_instr, branch),

        (uint32_t)sizeof(struct v3d_qpu_sig),
        (uint32_t)_Alignof(struct v3d_qpu_sig),

        (uint32_t)sizeof(struct v3d_qpu_flags),
        OFF(struct v3d_qpu_flags, ac),
        OFF(struct v3d_qpu_flags, mc),
        OFF(struct v3d_qpu_flags, apf),
        OFF(struct v3d_qpu_flags, mpf),
        OFF(struct v3d_qpu_flags, auf),
        OFF(struct v3d_qpu_flags, muf),

        (uint32_t)sizeof(struct v3d_qpu_input),
        OFF(struct v3d_qpu_input, mux),
        OFF(struct v3d_qpu_input, raddr),
        OFF(struct v3d_qpu_input, unpack),

        (uint32_t)sizeof(struct v3d_qpu_alu_instr),
        OFF(struct v3d_qpu_alu_instr, add),
        OFF(struct v3d_qpu_alu_instr, add.op),
        OFF(struct v3d_qpu_alu_instr, add.a),
        OFF(struct v3d_qpu_alu_instr, add.b),
        OFF(struct v3d_qpu_alu_instr, add.waddr),
        OFF(struct v3d_qpu_alu_instr, add.magic_write),
        OFF(struct v3d_qpu_alu_instr, add.output_pack),
        OFF(struct v3d_qpu_alu_instr, mul),
        OFF(struct v3d_qpu_alu_instr, mul.op),
        OFF(struct v3d_qpu_alu_instr, mul.a),
        OFF(struct v3d_qpu_alu_instr, mul.b),
        OFF(struct v3d_qpu_alu_instr, mul.waddr),
        OFF(struct v3d_qpu_alu_instr, mul.magic_write),
        OFF(struct v3d_qpu_alu_instr, mul.output_pack),

        (uint32_t)sizeof(struct v3d_qpu_branch_instr),
        OFF(struct v3d_qpu_branch_instr, cond),
        OFF(struct v3d_qpu_branch_instr, msfign),
        OFF(struct v3d_qpu_branch_instr, bdi),
        OFF(struct v3d_qpu_branch_instr, bdu),
        OFF(struct v3d_qpu_branch_instr, ub),
        OFF(struct v3d_qpu_branch_instr, raddr_a),
        OFF(struct v3d_qpu_branch_instr, offset),

        /* Every enum the struct holds is held as a 32-bit integer by the
         * mirror: the sizes say that is what the ABI gives them. */
        (uint32_t)sizeof(enum v3d_qpu_instr_type),
        (uint32_t)sizeof(enum v3d_qpu_cond),
        (uint32_t)sizeof(enum v3d_qpu_pf),
        (uint32_t)sizeof(enum v3d_qpu_uf),
        (uint32_t)sizeof(enum v3d_qpu_add_op),
        (uint32_t)sizeof(enum v3d_qpu_mul_op),
        (uint32_t)sizeof(enum v3d_qpu_mux),
        (uint32_t)sizeof(enum v3d_qpu_input_unpack),
        (uint32_t)sizeof(enum v3d_qpu_output_pack),
        (uint32_t)sizeof(enum v3d_qpu_branch_cond),
        (uint32_t)sizeof(enum v3d_qpu_msfign),
        (uint32_t)sizeof(enum v3d_qpu_branch_dest),
        (uint32_t)sizeof(bool),
};
const uint32_t yakogl_v3d_qpu_layout_len =
        sizeof(yakogl_v3d_qpu_layout) / sizeof(yakogl_v3d_qpu_layout[0]);

/* The values of the enums qpu_instr.c has no name function for; the others
 * (ops, conditions, flags, packs, unpacks, branch conditions, msfign) are
 * checked through Mesa's own name tables. */
const uint32_t yakogl_v3d_qpu_enum_values[] = {
        V3D_QPU_INSTR_TYPE_ALU, V3D_QPU_INSTR_TYPE_BRANCH,
        V3D_QPU_MUX_R0, V3D_QPU_MUX_R1, V3D_QPU_MUX_R2, V3D_QPU_MUX_R3,
        V3D_QPU_MUX_R4, V3D_QPU_MUX_R5, V3D_QPU_MUX_A, V3D_QPU_MUX_B,
        V3D_QPU_BRANCH_DEST_ABS, V3D_QPU_BRANCH_DEST_REL,
        V3D_QPU_BRANCH_DEST_LINK_REG, V3D_QPU_BRANCH_DEST_REGFILE,
        V3D_QPU_WADDR_NOP,
};
const uint32_t yakogl_v3d_qpu_enum_values_len =
        sizeof(yakogl_v3d_qpu_enum_values) / sizeof(yakogl_v3d_qpu_enum_values[0]);

#define SIG_FIELDS 17

/* out[i] = the bytes of a v3d_qpu_sig with only field i set, fields in
 * declaration order (thrsw .. small_imm_d). Returns sizeof(struct
 * v3d_qpu_sig), or 0 when `size` is not it. */
uint32_t
yakogl_v3d_qpu_sig_images(uint8_t *out, uint32_t size)
{
        if (size != sizeof(struct v3d_qpu_sig))
                return 0;
        for (int i = 0; i < SIG_FIELDS; i++) {
                struct v3d_qpu_sig s;
                memset(&s, 0, sizeof(s));
                switch (i) {
                case 0: s.thrsw = true; break;
                case 1: s.ldunif = true; break;
                case 2: s.ldunifa = true; break;
                case 3: s.ldunifrf = true; break;
                case 4: s.ldunifarf = true; break;
                case 5: s.ldtmu = true; break;
                case 6: s.ldvary = true; break;
                case 7: s.ldvpm = true; break;
                case 8: s.ldtlb = true; break;
                case 9: s.ldtlbu = true; break;
                case 10: s.ucb = true; break;
                case 11: s.rotate = true; break;
                case 12: s.wrtmuc = true; break;
                case 13: s.small_imm_a = true; break;
                case 14: s.small_imm_b = true; break;
                case 15: s.small_imm_c = true; break;
                case 16: s.small_imm_d = true; break;
                }
                memcpy(out + (size_t)i * sizeof(s), &s, sizeof(s));
        }
        return (uint32_t)sizeof(struct v3d_qpu_sig);
}

/* v3d_qpu_magic_waddr_name for 4.2, NULL past the end of its table: the
 * table is indexed by enum v3d_qpu_waddr up to V3D_QPU_WADDR_R5REP (55) and
 * Mesa reads out of bounds for 56..63 (qpu_instr.c). NULL inside it too,
 * where the table has no entry (25..31, 47..54). */
const char *
yakogl_v3d42_magic_waddr_name(uint32_t waddr)
{
        if (waddr > V3D_QPU_WADDR_R5REP)
                return NULL;
        return v3d_qpu_magic_waddr_name(&yakogl_v3d42_devinfo, (enum v3d_qpu_waddr)waddr);
}
