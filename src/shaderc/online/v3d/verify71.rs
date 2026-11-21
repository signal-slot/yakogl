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
//! Left out because Mesa applies them only below 7.1: "LDVARY during THRSW
//! delay slots" (ver 42), "RF write after THREND" (ver 42; 7.1 has the rules
//! above instead), GFXH-1625 (no TMUWT in the last instruction, ver 42,
//! :422-426), GFXH-1633 (below 4.2, :229-248) and the "small imm a/c/d added
//! after V3D 7.1" refusal (below 7.1). Left out because they count nothing at
//! 7.1: the accumulator rules (an R4 read or write within two of a magic SFU
//! write, :316-328) have no version branch, but 7.1 has no accumulators
//! (`v3d_qpu_writes_r4` is false without them, qpu_instr.c:962-966, and an
//! operand is a register address, not a mux), and the magic SFU addresses
//! are reserved at 7.1, so "SFU write too soon after SFU" never counts; the
//! LDVPM signal of the peripheral count does not exist at 7.1. Not
//! transcribed, as in verify.rs: the TLB-Z-write rules (:142-155, :187-197),
//! which need Mesa's `is_tlb_z_write` mark that the words do not carry.
//!
//! The rules marked "(ours)" are Mesa's too, but in its scheduler
//! (qpu_schedule.c), which never builds what they refuse, so its validator
//! does not check them; a back end of our own can, so they are checked here,
//! each citing the place. Every program Mesa compiles for 7.1
//! (tests/fixtures/v3d_mesa_programs_71.txt) passes all of them (the tests
//! below). They are, in the order they are checked:
//!
//! * no branch within three instructions of a UNIFA write (:1488), no rf0
//!   write right after an LDVARY (:699-728), no LDUNIFA within three of a
//!   UNIFA write (:1440-1442);
//! * one uniform-stream word per instruction, and none with an LDUNIFA
//!   (:1513-1531);
//! * in a branch's delay slots no uniform and no UNIFA write (:2396-2412),
//!   and no flag write behind a conditional branch (:2482-2485, :2992);
//! * no SFU operation in a THRSW's delay slots (:2080-2083), the 7.1
//!   peripheral combinations (qpu_compatible_peripheral_access, :895-975),
//!   no UNIFA write in a THRSW instruction or its slots (:2093-2108);
//! * in the thread-end slots no LDVARY, no uniform after the THRSW and no
//!   rf2/rf3 read (qpu_inst_valid_in_thrend_slot, :1978-2059);
//! * rf0's flops at a thread switch (:1704-1745, :2110-2119).
//!
//! [`tlb_after_scoreboard_wait71`] is the scheduler's pixel-scoreboard rule
//! (:731-752): it needs the shader state's lock bit, so it is a function of
//! its own and [`verify71`] does not call it.
//!
//! Left out of qpu_schedule.c, each for its reason:
//!
//! * the read of rf0 right after an LDVARY (reads_too_soon, :623-636; Mesa's
//!   own output does it, see below) and the read of an SFU result in the next
//!   instruction (read_stalls, :779-787): priorities, not refusals;
//! * the accumulator timing (mux_reads_too_soon, :602-620, and the r4 part
//!   of writes_too_soon_after_write, :699-711): no accumulators at 7.1;
//! * no branch with MSFIGN and a condition other than ALWAYS/A0/NA0 right
//!   after a SETMSF: choose_instruction_to_schedule keeps it at every
//!   version (:1491-1500), but emit_branch names it a V3D 4.x restriction
//!   and drops it at `ver >= 71` (`is_safe_msf_branch`, :2430-2438);
//! * the TMU output FIFO bound when a lookup pairs with an LDTMU
//!   (:1563-1584): it needs each lookup's result count (`ldtmu_count`),
//!   which the TMU configuration decides and the words do not show;
//! * what qpu_inst_after_thrsw_valid_in_delay_slot keeps out of a THRSW's
//!   slots (:2131-2218: TLB, TMU writes, WRTMUC, TMU waits, accumulator
//!   writes, MULTOP, flag writes, BARRIERID): it judges only instructions
//!   scheduled after the THRSW, and one scheduled before it may sit in the
//!   same slot; the words do not say which (its branch and THRSW parts are
//!   the validator's rules above);
//! * emit_branch's LDVARY guard (:2471-2477): it limits only moving a branch
//!   up; an unmoved branch right after an LDVARY is legal, so the words
//!   cannot tell;
//! * the thread end's TLB Z write in slot 2 (:1984-1985): Mesa's
//!   `is_tlb_z_write` mark again;
//! * "at least one outstanding lookup or TSY wait" at a thread switch (the
//!   comment at :2168-2169): Mesa does not enforce it (a FS's last THRSW has
//!   none when the shader samples nothing);
//! * the dependencies (calculate_deps, :310-545, such as flags, rtop, TLB and
//!   TMU operations ordered against a THRSW): the program's own data flow,
//!   which a check of the words cannot know (rtop is the validator's rule
//!   above);
//! * qpu_merge_inst's limits (:1235-1386: two address-writing signals, a
//!   signal address in the condition field, the packer): codec71's encoding
//!   (the small immediates are the validator's rule above); and
//!   fixup_pipelined_ldvary (:2612-2731), whose asserts are the LDVARY-slot
//!   and rf0 rules above.
//!
//! [`peripherals_compatible71`] counts restricted accesses per word, where
//! Mesa compares the two instructions it merges: a lone VPMSETUP (a VPM read
//! and write from one operation) passes Mesa and is refused here. select71
//! never emits VPMSETUP, and sched71 packs with this function, so loosening
//! it would move sched71's words; it is left as it is.
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
    uniform_reads(alu) > 0
}

/// How many words the instruction takes from the uniform stream: one per
/// LDUNIF/LDUNIFRF signal and per magic write that loads one.
fn uniform_reads(alu: &Alu71) -> u32 {
    (alu.sig.ldunif || alu.sig.ldunifrf) as u32
        + magic_writes(alu).into_iter().flatten().filter(|&w| loads_unif(w)).count() as u32
}

/// `v3d_qpu_writes_flags` (qpu_instr.c:1140-1150).
fn writes_flags(alu: &Alu71) -> bool {
    let f = &alu.flags;
    f.apf != Pf::None || f.mpf != Pf::None || f.auf != Uf::None || f.muf != Uf::None
}

/// `qpu_inst_is_tlb` (qpu_schedule.c:132-154): a TLB read signal or a magic
/// TLB/TLBU write.
fn is_tlb_access(alu: &Alu71) -> bool {
    alu.sig.ldtlb || alu.sig.ldtlbu || magic_writes(alu).into_iter().flatten().any(is_tlb)
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
    let mut last_branch_cond = BranchCond::Always;

    for (i, instr) in prog.iter().enumerate() {
        let ip = i as isize;
        let fail = |rule| Err(VerifyError { ip: i, rule });
        macro_rules! ours {
            ($bad:expr, $rule:expr) => {
                if $bad {
                    return fail($rule);
                }
            };
        }
        /* in_branch_delay_slots / in_thrsw_delay_slots (qpu_validate.c:79-89) */
        let in_branch_delay = ip - last_branch_ip < 4;
        let in_thrsw_delay = ip - last_thrsw_ip < 3;

        let alu = match instr {
            Instr71::Branch(br) => {
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
                last_branch_cond = br.cond;
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

        /* Ours: one uniform-stream word per instruction, and none with an
         * LDUNIFA ("Simulator complains if we have two uniforms loaded in
         * the same instruction", choose_instruction_to_schedule,
         * qpu_schedule.c:1513-1531). */
        ours!(uniform_reads(alu) > 1, "two uniforms in one instruction (qpu_schedule.c, ours)");
        ours!(
            (alu.sig.ldunifa || alu.sig.ldunifarf) && reads_uniform(alu),
            "LDUNIFA with a uniform in one instruction (qpu_schedule.c, ours)"
        );

        /* Ours: what a branch delay slot may hold
         * (qpu_inst_valid_in_branch_delay_slot, qpu_schedule.c:2396-2412,
         * used by emit_branch at :2487 and by qpu_set_branch_targets at
         * :3002): no uniform (the branch moves the uniform stream, :2973-2983)
         * and no UNIFA write (a UNIFA write and its three followers do not
         * straddle a branch, the twin of the check at :1488); a branch or
         * THRSW there is Mesa's validator's rule. And no flag write in a
         * conditional branch's slots: emit_branch never moves the branch
         * above one (:2482-2485) and fills the rest with NOPs (:2513-2514);
         * qpu_set_branch_targets copies successor instructions only behind
         * an unconditional branch (:2992). */
        if in_branch_delay {
            ours!(reads_uniform(alu), "uniform read in a branch delay slot (qpu_schedule.c, ours)");
            ours!(writes_unifa(alu), "UNIFA write in a branch delay slot (qpu_schedule.c, ours)");
            ours!(
                last_branch_cond != BranchCond::Always && writes_flags(alu),
                "flags written in a conditional branch's delay slot (qpu_schedule.c, ours)"
            );
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

/// Ours: every TLB access of the fragment shader `prog` comes after the
/// scoreboard wait, which the thread switch does when it happens, two delay
/// slots after its THRSW: the first THRSW's when `lock_on_first` (the shader
/// state's `lock_scoreboard_on_first_thrsw`), else the last-thread-switch
/// pair's first. Mesa's scheduler keeps it (scoreboard_is_locked and
/// pixel_scoreboard_too_soon, qpu_schedule.c:731-752, checked at :1459 and
/// :1536; the TLB operations depend on every THRSW, :494-497; a FS always
/// has a last THRSW, nir_to_vir.c:5016-5017) and its validator does not.
/// A TLB access may sit in a later THRSW's slots, as the thread end's
/// (qpu_inst_valid_in_thrend_slot, :1978-2059, refuses only a TLB Z write
/// in slot 2, a mark the words do not carry). Not part of [`verify71`]:
/// its callers do not pass the shader state's bit, and Mesa's validator
/// accepts the single-segment TLB rows of tools/v3d/hazards71.tsv
/// (`tlbz.*`) that this refuses.
pub fn tlb_after_scoreboard_wait71(prog: &[Instr71], lock_on_first: bool) -> Result<(), VerifyError> {
    let thrsw = |i: &Instr71| matches!(i, Instr71::Alu(a) if a.sig.thrsw);
    let mut lock: Option<usize> = None;
    for (ip, instr) in prog.iter().enumerate() {
        if lock.is_none() && thrsw(instr) {
            if lock_on_first {
                lock = Some(ip);
            } else if ip > 0 && thrsw(&prog[ip - 1]) {
                lock = Some(ip - 1);
            }
        }
        if let Instr71::Alu(alu) = instr {
            if is_tlb_access(alu) && !lock.is_some_and(|l| ip >= l + 3) {
                return Err(VerifyError { ip, rule: "TLB access before the scoreboard wait (qpu_schedule.c, ours)" });
            }
        }
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

    fn ldunif() -> Alu71 {
        let mut a = nop();
        a.sig.ldunif = true;
        a
    }
    fn ldunifa() -> Alu71 {
        let mut a = nop();
        a.sig.ldunifa = true;
        a
    }
    /// An add `mov` to rf10 that pushes the Z flag.
    fn flag_write() -> Alu71 {
        let mut a = add(AddOp71::Mov, 10, false);
        a.flags.apf = Pf::PushZ;
        a
    }
    fn branch_if(cond: BranchCond) -> Instr71 {
        let Instr71::Branch(mut b) = branch() else { unreachable!() };
        b.cond = cond;
        Instr71::Branch(b)
    }
    /// `slot` (1..=3) or later after a branch, then a NOP up to past the
    /// slots, then a single-segment thread end.
    fn after_branch(br: Instr71, at: usize, a: Alu71) -> Result<(), &'static str> {
        let mut body = std::vec![br, Instr71::Alu(nop()), Instr71::Alu(nop()), Instr71::Alu(nop())];
        body.insert(at, Instr71::Alu(a));
        body.push(Instr71::Alu(nop()));
        rule(&with_end(&body, true), true)
    }

    /// One uniform-stream word per instruction, none with an LDUNIFA
    /// (qpu_schedule.c:1513-1531).
    #[test]
    fn one_uniform_per_instruction() {
        let tmu = |waddr| {
            let mut a = ldunif();
            a.add = add(AddOp71::Mov, waddr, true).add;
            a
        };
        /* TMUAU (13) loads a uniform, TMUA (12) does not */
        assert_eq!(body_rule(&[tmu(13)]), Err("two uniforms in one instruction (qpu_schedule.c, ours)"));
        assert_eq!(body_rule(&[tmu(12)]), Ok(()), "LDUNIF with a TMUA write");
        let mut a = ldunifa();
        a.add = add(AddOp71::Mov, 13, true).add;
        assert_eq!(body_rule(&[a]), Err("LDUNIFA with a uniform in one instruction (qpu_schedule.c, ours)"));
        let mut a = ldunifa();
        a.sig.ldunif = true;
        assert_eq!(body_rule(&[a]), Err("LDUNIFA with a uniform in one instruction (qpu_schedule.c, ours)"));
        assert_eq!(body_rule(&[ldunifa()]), Ok(()), "LDUNIFA alone");
        assert_eq!(body_rule(&[ldunif(), ldunifa()]), Ok(()), "LDUNIF, then LDUNIFA");
    }

    /// No uniform and no UNIFA write in a branch delay slot
    /// (qpu_schedule.c:2396-2412), no flag write behind a conditional branch
    /// (:2482-2485).
    #[test]
    fn branch_delay_slot_contents() {
        let cond = branch_if(BranchCond::A0);
        for slot in 1..=3 {
            assert_eq!(
                after_branch(branch(), slot, ldunif()),
                Err("uniform read in a branch delay slot (qpu_schedule.c, ours)"),
                "slot {slot}"
            );
            assert_eq!(
                after_branch(branch(), slot, add(AddOp71::Mov, 9, true)),
                Err("UNIFA write in a branch delay slot (qpu_schedule.c, ours)"),
                "slot {slot}"
            );
            assert_eq!(
                after_branch(cond, slot, flag_write()),
                Err("flags written in a conditional branch's delay slot (qpu_schedule.c, ours)"),
                "slot {slot}"
            );
            /* an unconditional branch's slots may write flags (Mesa copies
             * its target's first instructions there, :2992-3008) */
            assert_eq!(after_branch(branch(), slot, flag_write()), Ok(()), "slot {slot}");
        }
        /* the instruction after the third slot */
        assert_eq!(after_branch(branch(), 4, ldunif()), Ok(()));
        assert_eq!(after_branch(branch(), 4, add(AddOp71::Mov, 9, true)), Ok(()));
        assert_eq!(after_branch(cond, 4, flag_write()), Ok(()));
        /* and the flag write before the branch */
        let p = with_end(
            &[Instr71::Alu(flag_write()), cond, Instr71::Alu(nop()), Instr71::Alu(nop()), Instr71::Alu(nop())],
            true,
        );
        assert_eq!(rule(&p, true), Ok(()));
    }

    /// A TLB write (TLB, magic 7) `at` the index into a two-section FS:
    /// nop, THRSW (ip 1), nop, nop, the last-THRSW pair (ip 4 and 5), nop,
    /// nop, nop, nop, the thread end (ip 10).
    fn tlb_at(at: usize) -> Vec<Instr71> {
        let mut p: Vec<Instr71> =
            [nop(), thrsw(), nop(), nop(), thrsw(), thrsw(), nop(), nop(), nop(), nop(), thrsw(), nop(), nop()]
                .map(Instr71::Alu)
                .to_vec();
        let Instr71::Alu(a) = &mut p[at] else { unreachable!() };
        a.mul = mul(MulOp71::Mov, WADDR_TLB, true).mul;
        p
    }

    /// The scoreboard wait comes with the lock THRSW's switch, two slots
    /// after it (qpu_schedule.c:731-752).
    #[test]
    fn tlb_waits_for_the_scoreboard() {
        const RULE: &str = "TLB access before the scoreboard wait (qpu_schedule.c, ours)";
        let tlb = |at, first| tlb_after_scoreboard_wait71(&tlb_at(at), first).map_err(|e| (e.ip, e.rule));
        for at in [0, 3, 4, 5, 6] {
            assert_eq!(rule(&tlb_at(at), false), Ok(()), "verify71 does not check it ({at})");
            assert_eq!(tlb(at, false), Err((at, RULE)), "locked on the last THRSW pair, {at}");
        }
        for at in [7, 8, 9, 10, 11, 12] {
            assert_eq!(tlb(at, false), Ok(()), "after the pair's switch, {at}");
        }
        /* locked on the first THRSW (ip 1): from ip 4 on, its pair's slots too */
        assert_eq!(tlb(0, true), Err((0, RULE)));
        assert_eq!(tlb(3, true), Err((3, RULE)));
        for at in [4, 5, 6, 7] {
            assert_eq!(tlb(at, true), Ok(()), "after the first switch, {at}");
        }
        /* a TLB read signal is an access too */
        let mut p = tlb_at(7);
        let Instr71::Alu(a) = &mut p[3] else { unreachable!() };
        a.sig.ldtlb = true;
        a.sig_addr = 10;
        assert_eq!(tlb_after_scoreboard_wait71(&p, true).map_err(|e| e.ip), Err(3));
        /* a program with no TLB access has nothing to wait for */
        assert_eq!(tlb_after_scoreboard_wait71(&with_end(&[Instr71::Alu(nop())], true), false), Ok(()));
    }

    /// Every fragment shader Mesa 26.2.1 compiles for 7.1 makes its TLB
    /// accesses after the scoreboard wait, even counted from the last
    /// THRSW pair (the fixture does not record the lock bit; the first
    /// THRSW is never later than the pair).
    #[test]
    fn every_mesa_7_1_fragment_shader_waits_for_the_scoreboard() {
        let mut n = 0;
        for (pair, st, _, words) in fixture_programs().iter().filter(|p| p.1 == "fs") {
            let prog: Vec<Instr71> = words.iter().map(|&w| decode71(w).unwrap()).collect();
            if let Err(e) = tlb_after_scoreboard_wait71(&prog, false) {
                panic!("{pair} {st}: {e:?}");
            }
            n += prog.iter().any(|i| matches!(i, Instr71::Alu(a) if is_tlb_access(a))) as usize;
        }
        assert_eq!(n, 242, "fragment shaders with a TLB access");
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
