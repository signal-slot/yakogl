// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014 Broadcom
// Ported from Mesa 26.2.1 src/broadcom/compiler/qpu_validate.c and
// qpu_schedule.c (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The rules between instructions for V3D 7.1 programs ([`Instr71`]): the
//! twin of [`super::verify`], which checks them for 4.2 and is unchanged.
//!
//! Mesa's `qpu_validate` (src/broadcom/compiler/qpu_validate.c, Mesa 26.2.1,
//! Copyright (c) 2014 Broadcom, MIT licence) is one function for every
//! version; this file keeps its branches that hold at `ver >= 71`, in Mesa's
//! order and with Mesa's messages as the rule names:
//!
//! * the branch and THRSW delay slots, the MULTOP/UMUL24 rtop hazard, LDUNIF
//!   after LDVARY (rf0, not r5, at 7.1), the THRSW pairing and the program
//!   end, as on 4.2;
//! * the 7.1 small-immediate rules (one per instruction, a/b only with an
//!   add operation, c/d only with a mul one; :204-218);
//! * the 7.1 THRSW-slot rules: LDVARY only out of the second delay slot
//!   (:302-305), and at the thread end no ALU register-file write in the
//!   THRSW instruction itself and no write of rf2/rf3 in it or its two slots
//!   (:380-419);
//! * the 7.1 reserved magic addresses (0..4, 10, 14..15, 19..31, 47..63;
//!   :104-115) and the one-peripheral count without LDTMU (:333-344).
//!
//! Left out because Mesa applies them only below 7.1: the accumulator rules
//! (an R4 read or write within two of a magic SFU write, :316-328; the 7.1
//! SFU is an add operation and its magic addresses are reserved, so "SFU
//! write too soon after SFU" counts nothing at 7.1), "LDVARY during THRSW
//! delay slots" (ver 42), "RF write after THREND" (ver 42; 7.1 has the rules
//! above instead), GFXH-1625 (no TMUWT in the last instruction, ver 42,
//! :422-426), GFXH-1633 (below 4.2, :229-248) and the "small imm a/c/d added
//! after V3D 7.1" refusal (below 7.1). Not transcribed, as in verify.rs:
//! the TLB-Z-write rules (:142-155, :187-197), which need Mesa's
//! `is_tlb_z_write` mark that the words do not carry.
//!
//! The rules marked "(ours)" are Mesa's too, but in its scheduler
//! (qpu_schedule.c), which never builds what they refuse, so its validator
//! does not check them; a back end of our own can, so they are checked here,
//! each citing the place. Every program Mesa compiles for 7.1
//! (tests/fixtures/v3d_mesa_programs_71.txt) passes all of them (the tests
//! below).
//!
//! A 7.1 SFU result is not a hazard to check: the register-file read of an
//! SFU result too soon stalls (qpu_schedule.c:778-786 is a priority, not a
//! refusal). The register file has no read-port limit at 7.1: each of the four
//! operands has its own address (qpu_merge_raddrs, qpu_schedule.c:1013-1032,
//! checks only the small immediates).

use super::isa71::*;
pub use super::verify::VerifyError;

/// The magic addresses the 7.1 rules name (`enum v3d_qpu_waddr`,
/// qpu_instr.h:93-138).
const WADDR_TLB: u8 = 7;
const WADDR_TLBU: u8 = 8;
const WADDR_UNIFA: u8 = 9;
const WADDR_TMUAU: u8 = 13;
const WADDR_VPMU: u8 = 15;
const WADDR_SYNCU: u8 = 17;
const WADDR_TMUC: u8 = 32;

fn is_sfu_waddr(waddr: u8) -> bool {
    /* v3d_qpu_magic_waddr_is_sfu (qpu_instr.c:617-631) */
    (19..=24).contains(&waddr)
}
fn is_tmu(waddr: u8) -> bool {
    /* ver >= 40: TMUD..TMUAU and TMUC..TMUHSLOD (qpu_instr.c:633-648) */
    (11..=13).contains(&waddr) || (32..=46).contains(&waddr)
}
fn is_tlb(waddr: u8) -> bool {
    waddr == WADDR_TLB || waddr == WADDR_TLBU
}
fn is_vpm(waddr: u8) -> bool {
    waddr == 14 || waddr == WADDR_VPMU
}
fn is_tsy(waddr: u8) -> bool {
    (16..=18).contains(&waddr)
}
/// `v3d71_magic_waddr_is_reserved` (qpu_validate.c:104-115).
fn is_reserved71(waddr: u8) -> bool {
    waddr <= 4
        || waddr == 10
        || (14..=15).contains(&waddr)
        || (19..=31).contains(&waddr)
        || (47..=54).contains(&waddr)
        || (55..=63).contains(&waddr)
}
/// `v3d_qpu_magic_waddr_loads_unif` (qpu_instr.c:680-692).
fn loads_unif(waddr: u8) -> bool {
    matches!(waddr, WADDR_VPMU | WADDR_TLBU | WADDR_TMUAU | WADDR_SYNCU)
}

/// `v3d_qpu_instr_is_sfu` (qpu_instr.c:787-811): the SFU operations of the
/// add ALU, which are V3D 7.x's only way to the SFU.
pub fn is_sfu_op(op: AddOp71) -> bool {
    use AddOp71::*;
    matches!(op, Recip | Rsqrt | Exp | Log | Sin | Rsqrt2 | Ballot | Bcastf | Alleq | Allfeq | Rotq | Rot | Shuffle)
}

/// The magic addresses an ALU instruction's add and mul units write.
fn magic_writes(alu: &Alu71) -> [Option<u8>; 2] {
    [
        (alu.add.op != AddOp71::Nop && alu.add.magic).then_some(alu.add.waddr),
        (alu.mul.op != MulOp71::Nop && alu.mul.magic).then_some(alu.mul.waddr),
    ]
}

/// The add unit writes a register-file address (`v3d_qpu_add_op_has_dst`).
fn add_rf_dest(alu: &Alu71) -> Option<u8> {
    (add_op_args(alu.add.op) & ARG_D != 0 && !alu.add.magic).then_some(alu.add.waddr)
}
fn mul_rf_dest(alu: &Alu71) -> Option<u8> {
    (mul_op_args(alu.mul.op) & ARG_D != 0 && !alu.mul.magic).then_some(alu.mul.waddr)
}
/// The signal writes a register-file address.
fn sig_rf_dest(alu: &Alu71) -> Option<u8> {
    (alu.sig.writes_address() && !alu.sig_magic).then_some(alu.sig_addr)
}

/// `v3d71_qpu_writes_waddr_explicitly` (qpu_instr.c:1066-1092).
fn writes_rf_explicitly(alu: &Alu71, rf: u8) -> bool {
    [add_rf_dest(alu), mul_rf_dest(alu), sig_rf_dest(alu)].contains(&Some(rf))
}

/// `v3d_qpu_writes_rf0_implicitly` (qpu_instr.c:1030-1040): LDVARY (its C
/// coefficient, one instruction later), LDUNIF and LDUNIFA write rf0.
fn writes_rf0_implicitly(alu: &Alu71) -> bool {
    alu.sig.ldvary || alu.sig.ldunif || alu.sig.ldunifa
}

/// `v3d71_qpu_reads_raddr` (qpu_instr.c:1054-1064).
fn reads_rf(alu: &Alu71, rf: u8) -> bool {
    alu.read_operands().any(|(o, imm)| !imm && o.raddr == rf)
}

/// `v3d_qpu_writes_unifa` (qpu_instr.c:874-900).
fn writes_unifa(alu: &Alu71) -> bool {
    magic_writes(alu).contains(&Some(WADDR_UNIFA))
        || (alu.sig.writes_address() && alu.sig_magic && alu.sig_addr == WADDR_UNIFA)
}

/// The instruction takes a word from the uniform stream (Mesa's
/// `qinst->uniform != ~0`): an LDUNIF/LDUNIFRF signal or a magic write that
/// loads one (TMUAU, TLBU, VPMU, SYNCU).
fn reads_uniform(alu: &Alu71) -> bool {
    alu.sig.ldunif || alu.sig.ldunifrf || magic_writes(alu).into_iter().flatten().any(loads_unif)
}

/// One instruction's peripheral accesses, as Mesa's scheduler counts them
/// (qpu_peripherals, qpu_schedule.c:840-893). A bit per kind.
const P_VPM_READ: u32 = 1 << 0;
const P_VPM_WRITE: u32 = 1 << 1;
const P_VPM_WAIT: u32 = 1 << 2;
const P_SFU: u32 = 1 << 3;
const P_TMU_WRITE: u32 = 1 << 4;
const P_TMU_READ: u32 = 1 << 5;
const P_TMU_WAIT: u32 = 1 << 6;
const P_WRTMUC: u32 = 1 << 7;
const P_TSY: u32 = 1 << 8;
const P_TLB_READ: u32 = 1 << 9;
const P_TLB_WRITE: u32 = 1 << 10;

fn peripherals71(alu: &Alu71) -> u32 {
    use AddOp71::*;
    let mut p = 0;
    let add = alu.add.op;
    /* v3d_qpu_add_op_reads_vpm / _writes_vpm (qpu_instr.c:694-724) */
    if matches!(add, Vpmsetup | LdvpmvIn | LdvpmvOut | LdvpmdIn | LdvpmdOut | Ldvpmp | LdvpmgIn | LdvpmgOut) {
        p |= P_VPM_READ;
    }
    if matches!(add, Vpmsetup | Stvpmv | Stvpmd | Stvpmp) {
        p |= P_VPM_WRITE;
    }
    if add == Vpmwt {
        p |= P_VPM_WAIT;
    }
    if add == Tmuwt {
        p |= P_TMU_WAIT;
    }
    /* v3d_qpu_uses_sfu: the SFU operations and the legacy magic writes */
    if is_sfu_op(add) {
        p |= P_SFU;
    }
    for w in magic_writes(alu).into_iter().flatten() {
        if is_vpm(w) {
            p |= P_VPM_WRITE;
        }
        if is_sfu_waddr(w) {
            p |= P_SFU;
        }
        if is_tmu(w) {
            p |= P_TMU_WRITE;
        }
        if is_tlb(w) {
            p |= P_TLB_WRITE;
        }
    }
    /* TSY counts only on the add unit (qpu_schedule.c:881-887) */
    if alu.add.op != Nop && alu.add.magic && is_tsy(alu.add.waddr) {
        p |= P_TSY;
    }
    if alu.sig.ldtmu {
        p |= P_TMU_READ;
    }
    if alu.sig.wrtmuc {
        p |= P_WRTMUC;
    }
    if alu.sig.ldtlb || alu.sig.ldtlbu {
        p |= P_TLB_READ;
    }
    p
}

/// Does one instruction make only the peripheral accesses V3D 7.x allows
/// together? At most one of the restricted ones (TMU write, WRTMUC, TSY, TLB
/// read, SFU, VPM read or write), except a WRTMUC signal with a TMU register
/// write other than TMUC, and at most one TLB access
/// (qpu_compatible_peripheral_access, qpu_schedule.c:936-975, ver >= 71; a
/// second TMU read cannot be encoded in one instruction).
pub fn peripherals_compatible71(alu: &Alu71) -> bool {
    let p = peripherals71(alu);
    if p.count_ones() <= 1 {
        return true;
    }
    const RESTRICTED: u32 = P_TMU_WRITE | P_WRTMUC | P_TSY | P_TLB_READ | P_SFU | P_VPM_READ | P_VPM_WRITE;
    let r = p & RESTRICTED;
    let tmu_not_tmuc = magic_writes(alu).into_iter().flatten().all(|w| w != WADDR_TMUC);
    if r.count_ones() > 1 && !(r == P_WRTMUC | P_TMU_WRITE && tmu_not_tmuc) {
        return false;
    }
    (p & (P_TLB_WRITE | P_TLB_READ)).count_ones() <= 1
}

/// Check the 7.1 program `prog`. `single_seg`: the program starts in its last
/// thread section (no last-thread-switch pair; Mesa starts with
/// `last_thrsw_found = !c->last_thrsw`, qpu_validate.c:461-472).
pub fn verify71(prog: &[Instr71], single_seg: bool) -> Result<(), VerifyError> {
    let mut last: Option<&Instr71> = None;
    let mut last_branch_ip: isize = -10;
    let mut last_thrsw_ip: isize = -10;
    let mut last_thrsw_found = single_seg;
    let mut thrend_found = false;
    let mut thrsw_count = 0;
    let mut rtop_hazard = false;
    let mut rtop_valid = false;
    /* Ours: the scheduler's scoreboard (qpu_schedule.c:575-599, set up at
     * :3036-3044) for the rules it keeps and the validator does not. */
    let mut last_ldvary_ip: isize = -10;
    let mut last_unifa_ip: isize = -10;
    let mut last_implicit_rf0_ip: isize = -10;
    let mut rf0_flops_conflict = false;

    for (i, instr) in prog.iter().enumerate() {
        let ip = i as isize;
        let fail = |rule| Err(VerifyError { ip: i, rule });
        /* in_branch_delay_slots / in_thrsw_delay_slots (qpu_validate.c:79-89) */
        let in_branch_delay = ip - last_branch_ip < 4;
        let in_thrsw_delay = ip - last_thrsw_ip < 3;

        let alu = match instr {
            Instr71::Branch(_) => {
                /* qpu_validate.c:157-164 */
                if in_branch_delay {
                    return fail("branch in a branch delay slot.");
                }
                if in_thrsw_delay {
                    return fail("branch in a THRSW delay slot.");
                }
                /* Ours: no branch in a UNIFA write's three delay slots
                 * (qpu_schedule.c:1483-1489, asserted at :2428). */
                if ip - last_unifa_ip <= 3 {
                    return fail("branch within three instructions of a UNIFA write (qpu_schedule.c, ours)");
                }
                last_branch_ip = ip;
                last = Some(instr);
                continue;
            }
            Instr71::Alu(alu) => alu,
        };

        /* qpu_validate.c:168-185 */
        if alu.mul.op == MulOp71::Multop && !in_branch_delay {
            rtop_valid = true;
        }
        if alu.mul.op == MulOp71::Umul24 {
            if rtop_hazard {
                return fail("UMUL24 reads rtop from MULTOP but it got cleared by a previous THRSW");
            }
            rtop_valid = false;
            rtop_hazard = false;
        }

        /* ver >= 71 (qpu_validate.c:204-218); vir_is_add / vir_is_mul are
         * "the unit's operation is not NOP" (vir.c:136-148). */
        let s = &alu.sig;
        if (s.small_imm_a || s.small_imm_b) && alu.add.op == AddOp71::Nop {
            return fail("small imm a/b used but no ADD inst");
        }
        if (s.small_imm_c || s.small_imm_d) && alu.mul.op == MulOp71::Nop {
            return fail("small imm c/d used but no MUL inst");
        }
        if s.small_imm_a as u32 + s.small_imm_b as u32 + s.small_imm_c as u32 + s.small_imm_d as u32 > 1 {
            return fail("only one small immediate can be enabled per instruction");
        }

        /* LDVARY writes rf0 one instruction later than LDUNIF does
         * (qpu_validate.c:220-227; the comment's r5 is rf0 at 7.1,
         * qpu_schedule.c:1462-1471). */
        if let Some(Instr71::Alu(prev)) = last {
            if prev.sig.ldvary && (alu.sig.ldunif || alu.sig.ldunifa) {
                return fail("LDUNIF after a LDVARY");
            }
        }

        /* Ours: nothing else writes rf0 right after an LDVARY, whose rf0
         * write lands then; another LDVARY's lands a cycle later
         * (writes_too_soon_after_write, qpu_schedule.c:716-726, asserted at
         * :2725-2727). */
        if ip == last_ldvary_ip + 1 && (writes_rf_explicitly(alu, 0) || (writes_rf0_implicitly(alu) && !alu.sig.ldvary))
        {
            return fail("rf0 written right after a LDVARY (qpu_schedule.c, ours)");
        }
        /* Not a rule: reading rf0 right after an LDVARY. The scheduler
         * avoids it when it picks (reads_too_soon, qpu_schedule.c:622-636),
         * but its own pipelining fixup (fixup_pipelined_ldvary, :2589-2730)
         * moves an LDVARY up in front of such a read, which then gets rf0's
         * value from before the LDVARY's write: Mesa's 7.1 code does it
         * (`fadd rf6, rf0, rf5` right after an `ldvary`, throughout
         * tests/fixtures/v3d_mesa_programs_71.txt). */

        /* Ours: an LDUNIFA reads what a UNIFA write set up only three
         * instructions later (qpu_schedule.c:1440-1442). */
        if (alu.sig.ldunifa || alu.sig.ldunifarf) && ip - last_unifa_ip <= 3 {
            return fail("LDUNIFA within three instructions of a UNIFA write (qpu_schedule.c, ours)");
        }

        /* qpu_validate.c:250-288 */
        let (mut tmu, mut sfu, mut vpm, mut tlb, mut tsy) = (0u32, 0u32, 0u32, 0u32, 0u32);
        for w in magic_writes(alu).into_iter().flatten() {
            tmu += is_tmu(w) as u32;
            sfu += is_sfu_waddr(w) as u32;
            vpm += is_vpm(w) as u32;
            tlb += is_tlb(w) as u32;
            tsy += is_tsy(w) as u32;
        }

        /* qpu_validate.c:290-307 */
        if in_thrsw_delay {
            if sfu > 0 {
                return fail("SFU write started during THRSW delay slots ");
            }
            if alu.sig.ldvary && ip - last_thrsw_ip == 2 {
                return fail("LDVARY in 2nd THRSW delay slot");
            }
        }
        /* Ours: the 7.1 SFU is an operation, and its result would land in
         * the other thread too (qpu_inst_before_thrsw_valid_in_delay_slot,
         * qpu_schedule.c:2075-2084: slots 1 and 2). */
        if (1..=2).contains(&(ip - last_thrsw_ip)) && is_sfu_op(alu.add.op) {
            return fail("SFU operation in a THRSW delay slot (qpu_schedule.c, ours)");
        }

        /* qpu_validate.c:309-314 */
        if magic_writes(alu).into_iter().flatten().any(is_reserved71) {
            return fail("write to a reserved magic waddr");
        }

        /* qpu_validate.c:330-344; LDTMU counts only at ver 42, and 7.1 has
         * no LDVPM signal. */
        let loads = alu.sig.ldtlb as u32 + alu.sig.ldtlbu as u32;
        if tmu + sfu + vpm + tlb + tsy + loads > 1 {
            return fail("Only one of [TMU, SFU, TSY, TLB read, VPM] allowed");
        }
        /* Ours: Mesa's scheduler rule, which its validator leaves out */
        if !peripherals_compatible71(alu) {
            return fail("more than one peripheral access (qpu_compatible_peripheral_access, ours)");
        }

        /* qpu_validate.c:349-371 */
        if alu.sig.thrsw {
            if in_branch_delay {
                return fail("THRSW in a branch delay slot.");
            }
            if last_thrsw_found {
                thrend_found = true;
            }
            if last_thrsw_ip == ip - 1 {
                if last_thrsw_found {
                    return fail("Two last-THRSW signals");
                }
                last_thrsw_found = true;
            } else {
                if in_thrsw_delay {
                    return fail("THRSW too close to another THRSW.");
                }
                thrsw_count += 1;
                last_thrsw_ip = ip;
            }
        }
        let slot = ip - last_thrsw_ip;

        /* Ours: a UNIFA write and the three instructions after it do not
         * overlap the thread switch, which happens after the THRSW's two
         * delay slots (qpu_schedule.c:2093-2108). */
        if writes_unifa(alu) && slot <= 2 {
            return fail("UNIFA write in a THRSW instruction or its delay slots (qpu_schedule.c, ours)");
        }

        /* ver >= 71 (qpu_validate.c:373-421). Mesa asks "not NOP and not
         * magic" of its VIR instruction, whose destination-less operations
         * write the magic NOP; in a decoded word an STVPM's waddr field is
         * its VPM index (opcode, codec71's canonical form), so the
         * question here is whether the operation has a destination
         * (`v3d_qpu_add_op_has_dst`), as qpu_instr.c:1074-1084 asks it. */
        if thrend_found && slot <= 2 {
            if let Some(w) = add_rf_dest(alu) {
                if slot == 0 {
                    return fail("ADD RF write at THREND");
                }
                if w == 2 || w == 3 {
                    return fail("RF2-3 write after THREND");
                }
            }
            if let Some(w) = mul_rf_dest(alu) {
                if slot == 0 {
                    return fail("MUL RF write at THREND");
                }
                if w == 2 || w == 3 {
                    return fail("RF2-3 write after THREND");
                }
            }
            if matches!(sig_rf_dest(alu), Some(2 | 3)) {
                return fail("RF2-3 write after THREND");
            }
            /* Ours: qpu_inst_valid_in_thrend_slot (qpu_schedule.c:1978-2060,
             * ver >= 71): no LDVARY, no uniform after the THRSW, and no read
             * of rf2/rf3, which fragment shader setup may overwrite. */
            if alu.sig.ldvary {
                return fail("LDVARY in a thread-end slot (qpu_schedule.c, ours)");
            }
            if slot > 0 && reads_uniform(alu) {
                return fail("uniform read in a thread-end delay slot (qpu_schedule.c, ours)");
            }
            if reads_rf(alu, 2) || reads_rf(alu, 3) {
                return fail("RF2-3 read after THREND (qpu_schedule.c, ours)");
            }
        }

        /* Ours: rf0 is undefined at a thread switch if its last write was
         * an implicit one landing with another signal's register-file write,
         * and the section's last instruction has a signal writing the
         * register file (update_scoreboard_for_rf0_flops and its use,
         * qpu_schedule.c:1691-1745 and :2110-2119). */
        if slot == 2 && sig_rf_dest(alu).is_some() && (rf0_flops_conflict || last_implicit_rf0_ip == ip) {
            return fail("a signal's RF write in the last THRSW delay slot with rf0 undefined (qpu_schedule.c, ours)");
        }
        if writes_rf_explicitly(alu, 0) || slot == 3 {
            last_implicit_rf0_ip = -10;
            rf0_flops_conflict = false;
        }
        if writes_rf0_implicitly(alu) {
            last_implicit_rf0_ip = if alu.sig.ldvary { ip + 1 } else { ip };
        }
        if last_implicit_rf0_ip == ip && sig_rf_dest(alu).is_some() {
            rf0_flops_conflict = true;
        }

        /* qpu_validate.c:429-432 */
        if rtop_valid && slot == 2 {
            rtop_hazard = true;
            rtop_valid = false;
        }

        if alu.sig.ldvary {
            last_ldvary_ip = ip;
        }
        if writes_unifa(alu) {
            last_unifa_ip = ip;
        }
        last = Some(instr);
    }

    /* qpu_validate.c:478-484 */
    let end = prog.len().saturating_sub(1);
    if thrsw_count > 1 && !last_thrsw_found {
        return Err(VerifyError { ip: end, rule: "thread switch found without last-THRSW in program" });
    }
    if !thrend_found {
        return Err(VerifyError { ip: end, rule: "No program-end THRSW found" });
    }
    Ok(())
}

/// Ours: the register-file registers `prog` reads or writes all exist at
/// `threads` threads. The rule is 4.2's ([`super::verify::registers_fit_threads`]):
/// Mesa's allocator has no version branch here, rf0..rf31 at 4 threads and
/// rf0..rf63 at 1 or 2 (vir_register_allocate.c:1046-1070, :1433-1435). At
/// 7.1 every operand the operations read is an address, unless its small
/// immediate flag is set.
pub fn registers_fit_threads71(prog: &[Instr71], threads: u32) -> Result<(), VerifyError> {
    const RULE: &str = "a register-file register past what a thread has at this thread count";
    let end = match threads {
        4 => 32,
        1 | 2 => 64,
        _ => return Err(VerifyError { ip: 0, rule: "a thread count the V3D does not run (not 1, 2 or 4)" }),
    };
    for (ip, instr) in prog.iter().enumerate() {
        let past = match instr {
            Instr71::Alu(alu) => {
                alu.read_operands().any(|(o, imm)| !imm && o.raddr >= end)
                    || [add_rf_dest(alu), mul_rf_dest(alu), sig_rf_dest(alu)].into_iter().flatten().any(|r| r >= end)
            }
            Instr71::Branch(br) => br.reads_raddr_a() && br.raddr_a >= end,
        };
        if past {
            return Err(VerifyError { ip, rule: RULE });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::codec71::decode71;
    use super::super::isa::num_src;
    use super::*;
    use std::vec::Vec;

    static FIXTURE_71: &str = include_str!("../../../../tests/fixtures/v3d_mesa_programs_71.txt");

    /// The stage programs of the 7.1 fixture: (pair, stage, threads, words).
    fn fixture_programs() -> Vec<(String, String, u32, Vec<u64>)> {
        let mut out = Vec::new();
        let mut pair = String::new();
        let mut threads: std::collections::BTreeMap<String, u32> = Default::default();
        for line in FIXTURE_71.lines() {
            let w: Vec<&str> = line.split(' ').filter(|s| !s.is_empty()).collect();
            match w.first().copied() {
                Some("pair") => {
                    pair = format!("{} + {}", w[1], w[2]);
                    threads.clear();
                }
                Some(st @ ("fs" | "vs" | "cs")) if w.get(1) == Some(&"meta") => {
                    let t = w[2..].iter().find_map(|kv| kv.strip_prefix("threads=")).expect("threads=");
                    threads.insert(st.into(), t.parse().unwrap());
                }
                Some(st @ ("fs" | "vs" | "cs")) if w.get(1) == Some(&"code") => {
                    let words: Vec<u64> =
                        w[2..].iter().map(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()).collect();
                    if !words.is_empty() {
                        let t = *threads.get(st).unwrap_or_else(|| panic!("{pair} {st}: code before meta"));
                        out.push((pair.clone(), st.to_string(), t, words));
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// A program has a last-thread-switch pair (two THRSW in a row) unless
    /// it starts in its last section (Mesa's `single_seg = !c->last_thrsw`).
    fn single_seg(prog: &[Instr71]) -> bool {
        let thrsw = |i: &Instr71| matches!(i, Instr71::Alu(a) if a.sig.thrsw);
        !prog.windows(2).any(|w| thrsw(&w[0]) && thrsw(&w[1]))
    }

    /// Every program Mesa 26.2.1 compiles for V3D 7.1 (v3d_shc --device 71,
    /// the fixture of tests/v3d_sim_mesa.rs) passes every rule here, Mesa's
    /// and ours, and fits its thread count's registers. Pinned: the 242
    /// compiled pairs' fragment, vertex and coordinate shaders.
    #[test]
    fn every_mesa_7_1_program_passes() {
        let progs = fixture_programs();
        let mut fails = Vec::new();
        for (pair, st, threads, words) in &progs {
            let prog: Vec<Instr71> =
                words.iter().map(|&w| decode71(w).unwrap_or_else(|e| panic!("{pair} {st}: {w:#x}: {e:?}"))).collect();
            if let Err(e) = verify71(&prog, single_seg(&prog)) {
                let mut at = String::new();
                super::super::listing71::write_instr71(&mut at, &prog[e.ip]).unwrap();
                fails.push(format!("{pair} {st}: {e:?}: {at}"));
            }
            if let Err(e) = registers_fit_threads71(&prog, *threads) {
                fails.push(format!("{pair} {st} at {threads} threads: {e:?}"));
            }
        }
        assert!(fails.is_empty(), "{} of {} refused:\n{}", fails.len(), progs.len(), fails.join("\n"));
        assert_eq!(progs.len(), 3 * 242, "the fixture's stage programs");
    }

    fn nop() -> Alu71 {
        Alu71::nop()
    }
    fn thrsw() -> Alu71 {
        let mut a = nop();
        a.sig.thrsw = true;
        a
    }
    /// An add `mov` (or another add operation) to register `waddr`.
    fn add(op: AddOp71, waddr: u8, magic: bool) -> Alu71 {
        let mut a = nop();
        a.add.op = op;
        a.add.waddr = waddr;
        a.add.magic = magic;
        a.add.a.raddr = 10;
        if num_src(add_op_args(op)) > 1 {
            a.add.b.raddr = 11;
        }
        a
    }
    fn mul(op: MulOp71, waddr: u8, magic: bool) -> Alu71 {
        let mut a = nop();
        a.mul.op = op;
        a.mul.waddr = waddr;
        a.mul.magic = magic;
        a.mul.a.raddr = 10;
        a.mul.b.raddr = 11;
        a
    }
    fn ldvary(rf: u8) -> Alu71 {
        let mut a = nop();
        a.sig.ldvary = true;
        a.sig_addr = rf;
        a
    }
    fn ldtmu(rf: u8) -> Alu71 {
        let mut a = nop();
        a.sig.ldtmu = true;
        a.sig_addr = rf;
        a
    }
    fn branch() -> Instr71 {
        Instr71::Branch(Branch {
            cond: BranchCond::Always,
            msfign: Msfign::None,
            bdi: BranchDest::Rel,
            ub: false,
            bdu: BranchDest::Rel,
            raddr_a: 0,
            offset: 0,
        })
    }

    /// `body`, then a thread end (THRSW and its two delay slots), with the
    /// last-thread-switch pair between them unless `single_seg`.
    fn with_end(body: &[Instr71], single_seg: bool) -> Vec<Instr71> {
        let mut p = body.to_vec();
        if !single_seg {
            p.extend([Instr71::Alu(thrsw()), Instr71::Alu(thrsw()), Instr71::Alu(nop()), Instr71::Alu(nop())]);
        }
        p.extend([Instr71::Alu(thrsw()), Instr71::Alu(nop()), Instr71::Alu(nop())]);
        p
    }

    fn rule(prog: &[Instr71], single_seg: bool) -> Result<(), &'static str> {
        verify71(prog, single_seg).map_err(|e| e.rule)
    }
    fn body_rule(body: &[Alu71]) -> Result<(), &'static str> {
        let body: Vec<Instr71> = body.iter().map(|a| Instr71::Alu(*a)).collect();
        rule(&with_end(&body, true), true)
    }
    /// `tail` replaces the thread end's THRSW and its two slots.
    fn end_rule(tail: [Alu71; 3]) -> Result<(), &'static str> {
        let mut p: Vec<Instr71> = (0..4).map(|_| Instr71::Alu(nop())).collect();
        p.extend(tail.map(Instr71::Alu));
        rule(&p, true)
    }

    /// The known positives the bad cases below differ from.
    #[test]
    fn the_good_shapes_pass() {
        assert_eq!(body_rule(&[nop()]), Ok(()));
        assert_eq!(rule(&with_end(&[Instr71::Alu(nop())], false), false), Ok(()));
        assert_eq!(body_rule(&[add(AddOp71::Recip, 10, false), nop()]), Ok(()), "an SFU operation");
        assert_eq!(body_rule(&[ldvary(10), ldvary(11), nop()]), Ok(()), "LDVARY after LDVARY");
        let mut wrtmuc = mul(MulOp71::Mov, 11, true);
        wrtmuc.sig.wrtmuc = true;
        assert_eq!(body_rule(&[wrtmuc]), Ok(()), "WRTMUC with a TMU write other than TMUC");
        let mut si = add(AddOp71::Add, 10, false);
        si.sig.small_imm_b = true;
        assert_eq!(body_rule(&[si]), Ok(()), "one small immediate with an add");
        assert_eq!(end_rule([thrsw(), add(AddOp71::Mov, 10, false), nop()]), Ok(()), "an RF write in a THREND slot");
    }

    /// One bad program per rule, each refused with that rule's name.
    #[test]
    fn each_rule_refuses_its_bad_program() {
        use AddOp71 as A;
        use MulOp71 as M;
        let i = Instr71::Alu;

        /* the branch and THRSW delay slots */
        let p = with_end(&[branch(), i(nop()), branch(), i(nop()), i(nop()), i(nop())], true);
        assert_eq!(rule(&p, true), Err("branch in a branch delay slot."));
        let p = with_end(&[i(thrsw()), i(nop()), branch(), i(nop()), i(nop()), i(nop())], false);
        assert_eq!(rule(&p, false), Err("branch in a THRSW delay slot."));
        let p = with_end(&[i(add(A::Mov, 9, true)), i(nop()), branch(), i(nop()), i(nop()), i(nop())], true);
        assert_eq!(rule(&p, true), Err("branch within three instructions of a UNIFA write (qpu_schedule.c, ours)"));
        let p = with_end(&[branch(), i(thrsw()), i(nop()), i(nop())], false);
        assert_eq!(rule(&p, false), Err("THRSW in a branch delay slot."));

        /* rtop: MULTOP, a THRSW's second slot, then UMUL24 */
        let p = with_end(&[i(thrsw()), i(mul(M::Multop, 6, true)), i(nop()), i(mul(M::Umul24, 10, false))], false);
        assert_eq!(rule(&p, false), Err("UMUL24 reads rtop from MULTOP but it got cleared by a previous THRSW"));

        /* small immediates */
        let mut a = nop();
        a.sig.small_imm_a = true;
        assert_eq!(body_rule(&[a]), Err("small imm a/b used but no ADD inst"));
        let mut a = nop();
        a.sig.small_imm_d = true;
        assert_eq!(body_rule(&[a]), Err("small imm c/d used but no MUL inst"));
        let mut a = add(A::Add, 10, false);
        a.mul = mul(M::Add, 11, false).mul;
        a.sig.small_imm_b = true;
        a.sig.small_imm_c = true;
        assert_eq!(body_rule(&[a]), Err("only one small immediate can be enabled per instruction"));

        /* LDVARY and rf0 */
        let mut ldunif = nop();
        ldunif.sig.ldunif = true;
        assert_eq!(body_rule(&[ldvary(10), ldunif]), Err("LDUNIF after a LDVARY"));
        assert_eq!(
            body_rule(&[ldvary(10), add(A::Mov, 0, false)]),
            Err("rf0 written right after a LDVARY (qpu_schedule.c, ours)")
        );
        let mut ldunifa = nop();
        ldunifa.sig.ldunifa = true;
        assert_eq!(
            body_rule(&[add(A::Mov, 9, true), nop(), ldunifa]),
            Err("LDUNIFA within three instructions of a UNIFA write (qpu_schedule.c, ours)")
        );

        /* the THRSW delay slots */
        let p = with_end(&[i(thrsw()), i(add(A::Mov, 19, true)), i(nop())], false);
        assert_eq!(rule(&p, false), Err("SFU write started during THRSW delay slots "));
        let p = with_end(&[i(thrsw()), i(nop()), i(ldvary(10))], false);
        assert_eq!(rule(&p, false), Err("LDVARY in 2nd THRSW delay slot"));
        let p = with_end(&[i(thrsw()), i(add(A::Recip, 10, false)), i(nop())], false);
        assert_eq!(rule(&p, false), Err("SFU operation in a THRSW delay slot (qpu_schedule.c, ours)"));
        let p = with_end(&[i(thrsw()), i(add(A::Mov, 9, true)), i(nop())], false);
        assert_eq!(
            rule(&p, false),
            Err("UNIFA write in a THRSW instruction or its delay slots (qpu_schedule.c, ours)")
        );
        /* LDVARY in the first slot lands rf0 in the second, where LDTMU
         * writes the register file */
        let p = with_end(&[i(thrsw()), i(ldvary(10)), i(ldtmu(11)), i(nop())], false);
        assert_eq!(
            rule(&p, false),
            Err("a signal's RF write in the last THRSW delay slot with rf0 undefined (qpu_schedule.c, ours)")
        );

        /* magic addresses and peripherals */
        assert_eq!(body_rule(&[add(A::Mov, 0, true)]), Err("write to a reserved magic waddr"));
        assert_eq!(body_rule(&[add(A::Mov, 55, true)]), Err("write to a reserved magic waddr"));
        let mut a = add(A::Mov, 11, true);
        a.mul = mul(M::Mov, 7, true).mul;
        assert_eq!(body_rule(&[a]), Err("Only one of [TMU, SFU, TSY, TLB read, VPM] allowed"));
        let mut a = add(A::Recip, 10, false);
        a.mul = mul(M::Mov, 11, true).mul;
        assert_eq!(body_rule(&[a]), Err("more than one peripheral access (qpu_compatible_peripheral_access, ours)"));

        /* the THRSW pairing and the program end */
        let p = with_end(&[i(thrsw()), i(thrsw()), i(nop()), i(nop())], false);
        assert_eq!(rule(&p, false), Err("Two last-THRSW signals"));
        let p = with_end(&[i(thrsw()), i(nop()), i(thrsw()), i(nop()), i(nop())], false);
        assert_eq!(rule(&p, false), Err("THRSW too close to another THRSW."));
        let p: Vec<Instr71> =
            [thrsw(), nop(), nop(), nop(), thrsw(), nop(), nop(), nop(), thrsw(), nop(), nop()].map(i).to_vec();
        assert_eq!(rule(&p, false), Err("thread switch found without last-THRSW in program"));
        assert_eq!(rule(&[i(nop()), i(nop())], true), Err("No program-end THRSW found"));

        /* the thread end */
        let mut t = thrsw();
        t.add = add(A::Mov, 10, false).add;
        assert_eq!(end_rule([t, nop(), nop()]), Err("ADD RF write at THREND"));
        let mut t = thrsw();
        t.mul = mul(M::Mov, 10, false).mul;
        assert_eq!(end_rule([t, nop(), nop()]), Err("MUL RF write at THREND"));
        assert_eq!(end_rule([thrsw(), add(A::Mov, 3, false), nop()]), Err("RF2-3 write after THREND"));
        assert_eq!(end_rule([thrsw(), nop(), ldtmu(2)]), Err("RF2-3 write after THREND"));
        assert_eq!(end_rule([thrsw(), ldvary(10), nop()]), Err("LDVARY in a thread-end slot (qpu_schedule.c, ours)"));
        assert_eq!(
            end_rule([thrsw(), nop(), ldunif]),
            Err("uniform read in a thread-end delay slot (qpu_schedule.c, ours)")
        );
        let mut r = add(A::Mov, 10, false);
        r.add.a.raddr = 2;
        assert_eq!(end_rule([thrsw(), r, nop()]), Err("RF2-3 read after THREND (qpu_schedule.c, ours)"));
    }

    /// rf32 is another thread's at 4 threads and this one's at 2.
    #[test]
    fn registers_past_the_threads_share_are_refused() {
        let p = with_end(&[Instr71::Alu(add(AddOp71::Mov, 32, false))], true);
        assert_eq!(registers_fit_threads71(&p, 2), Ok(()));
        assert_eq!(
            registers_fit_threads71(&p, 4).map_err(|e| e.rule),
            Err("a register-file register past what a thread has at this thread count")
        );
        let mut a = add(AddOp71::Mov, 10, false);
        a.add.a.raddr = 40;
        let p = with_end(&[Instr71::Alu(a)], true);
        assert!(registers_fit_threads71(&p, 4).is_err(), "a read");
        a.sig.small_imm_a = true;
        let p = with_end(&[Instr71::Alu(a)], true);
        assert_eq!(registers_fit_threads71(&p, 4), Ok(()), "a small immediate is no register");
    }
}
