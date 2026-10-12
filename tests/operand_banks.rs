// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! How many operands of each register group one instruction may hold.
//!
//! Folding a uniform read or a literal into its consumer removes an
//! instruction, and both are worth about a hundred instructions over the
//! corpus. Whether the machine ALLOWS the fold is a different question, and it
//! is not one to guess at: an instruction with more operands of a bank than
//! the encoding has room for is a shader that assembles and draws something
//! else.
//!
//! # How a source slot is read, which is where the first version went wrong
//!
//! **A source slot is present when the operation HAS a `SRC{n}_RGROUP` field
//! for it.** The generated tables expose the field only for slots the
//! encoding uses, so presence is the question; `SRC{n}_USE` is absent from
//! 13,830 of the 13,908 instructions -- including every `mad`, `mul`, `mov`,
//! `add`, `select` and `sqrt` -- and gating on it counted 53 immediates where
//! there are 1,808.
//!
//! The disassembly of the control blob shows the same thing from the other
//! side: `mov t1.xy__, void, void, t0.xyxx` prints three slots and carries
//! one group field, because `void` is an absent slot rather than a zero one.
//!
//! An immediate has no second encoding, contrary to what the redundant
//! `p.is_immediate() || p.group == RGROUP_IMMED` in `alloc.rs` suggests:
//! `is_immediate()` IS `group == RGROUP_IMMED`, so the disjunction tests one
//! thing twice. I read a second encoding into it and was wrong.
//!
//! The reference programs hold up to two immediates per instruction (two in
//! 328, three never). This asks the same question of every group,
//! because **`src()` already folds uniform reads into consumers today** -- so
//! if this machine limits constant-file operands per instruction, the limit is
//! one we may already be exceeding, and nothing in the pipeline would say so.
//! The board results we have came from shaders where the case may simply never
//! arise.
//!
//! The reference compiler is the oracle: what it never emits, we refuse.
//!
//! # The answer, and it closes the hazard rather than confirming it
//!
//! ```text
//!   group          0 ops     1      2      3    max
//!   temp            1019   6200   5537   1152    3
//!   uniform_0       9544   3499    823     42    3
//!   immediate      12100   1480    328      0    2
//!   internal       13904      4      0      0    1
//!   uniform_1      13908      0      0      0    0
//!   thread         13908      0      0      0    0
//! ```
//!
//! **This counts OPERANDS per group, not registers.** The reference puts two
//! uniform operands in 823 instructions and three in 42 -- and in every one
//! of them they are lanes of ONE register (`u0.x`, `u0.y`). Two DIFFERENT
//! constant registers in one instruction it never emits (0 of 13,908), and
//! this core cannot take them: etnaviv refuses such an instruction unless
//! the core has SH_NO_ONECONST_LIMIT (etnaviv_asm.c:38-71; lower_alu's "insert
//! movs for multiple constants", etnaviv_compiler_nir.c:836), and this
//! GC7000's minor_features8 (0x00e10ef3) does not. This file once concluded
//! from the operand count that "the hazard does not exist"; it did, and it
//! drew sin/cos wrong on silicon (gl_test 77). tests/constant_registers.rs is
//! the check for that rule; this one says only how many operands of a group
//! an instruction may hold.
//!
//! The immediate column reproduces those counts exactly, which is what says
//! the reader here is right.
//!
//! ```sh
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online \
//!     --test operand_banks -- --nocapture
//! ```

#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online", not(feature = "no-shader-table")))]

use std::collections::BTreeMap;
use yakogl::shaderc::mesa_reference::SHC_BLOB_TABLE;
use yakogl::shaderc::online::vivante::decode::decode_instr;

/// The groups `vivante/build.rs` names, by their encoded value.
fn group_name(g: u32) -> &'static str {
    match g {
        0 => "temp",
        1 => "internal",
        2 => "uniform_0",
        3 => "uniform_1",
        4 => "thread",
        7 => "immediate",
        _ => "other",
    }
}

/// Print a candidate for the positive control: the smallest blob, its
/// disassembly and its operand mix, so the control can be written from what
/// the machine says rather than from what this file computes.
fn show_a_small_blob() {
    let mut best: Option<(usize, usize)> = None;
    for (i, b) in SHC_BLOB_TABLE.iter().enumerate() {
        let n = b.shader.usc_words().len();
        if n >= 8 && best.is_none_or(|(_, m)| n < m) {
            best = Some((i, n));
        }
    }
    let (i, n) = best.expect("a blob");
    let b = &SHC_BLOB_TABLE[i];
    println!("blob {i}: {n} words, {} instructions", n / 4);
    for (k, q) in b.shader.usc_words().as_chunks::<4>().0.iter().enumerate() {
        if let Ok(ins) = decode_instr(&[q[0], q[1], q[2], q[3]]) {
            let mut t = String::new();
            let _ = yakogl::shaderc::online::vivante::listing::instr_text(&mut t, k, &ins);
            let groups: Vec<u32> = (0..3).filter_map(|j| ins.get(&format!("SRC{j}_RGROUP"))).collect();
            println!("  {t}   <- groups {groups:?}");
        }
    }
}

/// A blob whose operand mix is read off the disassembly, asserted exactly.
///
/// The lead's requirement, and the reason is the first version of this file:
/// it counted nothing and printed a healthy-looking table of near-zeros,
/// passing its only guard (`instrs > 0`). A control whose answer is known
/// independently of the counter fails in that case instead.
///
/// Blob 13 is two instructions, and its disassembly says what they read:
///
/// ```text
///   mov t1.xy__, void, void, t0.xyxx     <- t0, a TEMP
///   mov t1.__zw, void, void, u0.xxxy     <- u0, a UNIFORM
/// ```
///
/// Read from the printed text, not from the field reader under test. (It is
/// also `cube.vert`'s `u0.xxxy`, the instruction `ir.md` uses to settle which
/// lane a masked store takes -- so its shape is documented elsewhere too.)
fn the_control_blob_reads_one_temp_and_one_uniform() {
    let b = &SHC_BLOB_TABLE[13];
    let w = b.shader.usc_words();
    assert_eq!(w.len(), 8, "the control blob is two instructions; the table moved under it");

    let mut per: BTreeMap<u32, usize> = BTreeMap::new();
    for q in w.as_chunks::<4>().0 {
        let ins = decode_instr(&[q[0], q[1], q[2], q[3]]).expect("the control blob decodes");
        assert_eq!(ins.op().name, "mov");
        for n in 0..3 {
            if let Some(g) = ins.get(&format!("SRC{n}_RGROUP")) {
                *per.entry(g).or_insert(0) += 1;
            }
        }
    }
    assert_eq!(per.get(&0).copied().unwrap_or(0), 1, "one temp operand, from `t0.xyxx`");
    assert_eq!(per.get(&2).copied().unwrap_or(0), 1, "one uniform operand, from `u0.xxxy`");
    assert_eq!(per.get(&7).copied().unwrap_or(0), 0, "and no immediate");
    assert_eq!(per.values().sum::<usize>(), 2, "two operands in all: the `void` slots are absent, not zero");
}

fn how_many_operands_of_one_group_an_instruction_holds() {
    /* group -> (count per instruction -> how many instructions) */
    let mut hist: BTreeMap<u32, BTreeMap<usize, usize>> = BTreeMap::new();
    let mut instrs = 0usize;
    let mut blobs = 0usize;

    for b in SHC_BLOB_TABLE.iter() {
        let w = b.shader.usc_words();
        if w.len() < 4 {
            continue;
        }
        blobs += 1;
        for q in w.as_chunks::<4>().0 {
            let Ok(ins) = decode_instr(&[q[0], q[1], q[2], q[3]]) else { continue };
            instrs += 1;
            let mut per: BTreeMap<u32, usize> = BTreeMap::new();
            /* A source slot is present when the operation HAS a group field
             * for it, not when `SRC{n}_USE` says so.
             *
             * `SRC0_USE` is ABSENT from
             * 13,830 of the 13,908 instructions, including every `mad`,
             * `mul`, `mov`, `add`, `select` and `sqrt` -- the ordinary ALU
             * operations, which is to say almost all of them. `SRC0_RGROUP`
             * is present on 10,110. Gating on a field that is not there
             * counted nothing and printed a healthy-looking table of zeros. */
            for n in 0..3 {
                let Some(g) = ins.get(&format!("SRC{n}_RGROUP")) else { continue };
                *per.entry(g).or_insert(0) += 1;
            }
            /* Every group gets a row for this instruction, including zero, so
             * the columns sum to the instruction count and a missing bar is
             * visible as a zero rather than as an absence. */
            for g in [0u32, 1, 2, 3, 4, 7] {
                let k = per.get(&g).copied().unwrap_or(0);
                *hist.entry(g).or_default().entry(k).or_insert(0) += 1;
            }
        }
    }

    println!("\n=== the reference's operands per instruction, by register group ===");
    println!("  {blobs} blobs, {instrs} instructions\n");
    println!("  {:<12} {:>9} {:>9} {:>9} {:>9}   max", "group", "0", "1", "2", "3");
    for (g, counts) in &hist {
        let at = |k: usize| counts.get(&k).copied().unwrap_or(0);
        let max = counts.iter().filter(|(_, n)| **n > 0).map(|(k, _)| *k).max().unwrap_or(0);
        println!("  {:<12} {:>9} {:>9} {:>9} {:>9}   {max}", group_name(*g), at(0), at(1), at(2), at(3));
    }

    println!("\n  what a fold may therefore produce, per instruction:");
    for (g, counts) in &hist {
        let max = counts.iter().filter(|(_, n)| **n > 0).map(|(k, _)| *k).max().unwrap_or(0);
        println!("    {:<12} at most {max}", group_name(*g));
    }

    /* THE GUARD THAT WOULD HAVE CAUGHT THE FIRST VERSION OF THIS FILE.
     *
     * The reference programs' immediate column: 12,100 instructions with
     * none, 1,480 with one, 328 with two, never three. Pinning it here means a reader that silently
     * stops counting -- which is exactly what gating on `SRC{n}_USE` did --
     * fails instead of printing a healthy-looking table of zeros.
     *
     * `instrs > 0` was the only guard the first version had, and it passed
     * while the table was wrong by a factor of 34. */
    /* +152 / +27 / +18 (2026-10-02): the six FemtoVG NO_SCISSOR rows appended
     * to the table (gen_shader_table.py --append), 197 instructions. */
    /* -3 / +2 / 0, and one instruction fewer (2026-10-04): es300/dynidx.frag
     * and qt3d/e0880dcd5582d4de.frag regenerated with a uniform array read at
     * a run-time index lowered to a UBO load (gen_shader_table.py --replace).
     * Each `movar a.x, idx` + `mov u[a.x]` pair (no immediate) became
     * `imullo0 idx, 16` (one immediate: the vec4 stride in bytes) + `load`
     * (none); e0880's second indexed read reuses that byte offset and is one
     * `load` where it was another movar + mov. dynidx: 8/3 -> 7/4 over 11
     * instructions; e0880: 25/8 -> 23/9, 33 -> 32 instructions. */
    /* 2026-10-12 (issue #22): 220 rows appended to the table
     * (tools/vivante/README.md, "The table against its sources"). */
    assert_eq!(instrs, 19364, "instructions in the table");
    let imm = hist.get(&7).expect("the immediate group has a row");
    assert_eq!(imm.get(&0).copied().unwrap_or(0), 16330, "immediates: none");
    assert_eq!(imm.get(&1).copied().unwrap_or(0), 2541, "immediates: one");
    assert_eq!(imm.get(&2).copied().unwrap_or(0), 493, "immediates: two");
    assert_eq!(imm.get(&3).copied().unwrap_or(0), 0, "immediates: three, which the reference never emits");

    /* Three source slots is the encoding's own shape; a row above that means
     * the decoder and this test disagree about what an operand is. */
    for (g, counts) in &hist {
        for (k, n) in counts {
            assert!(*k <= 3, "group {} shows {k} operands in one instruction ({n} times)", group_name(*g));
        }
    }
    assert!(instrs > 0, "no instructions decoded");
}

/// Which OPERATIONS the reference gives a non-temporary operand, and the same
/// question asked of what we emit.
///
/// The caps above say how MANY operands of a group an instruction holds. They
/// do not say which operations hold them, and that is a separate fact with a
/// separate answer:
///
/// ```text
///   texld     568 temp operands, 0 uniform, 0 immediate
///   branch*   208 temp,          0 uniform, 0 immediate
///   mad      6962 temp,       2912 uniform, 221 immediate
/// ```
///
/// `OpId::takes_group` holds the lists this derives, and this test fails if
/// they drift apart -- the rule in the compiler and its oracle, side by side.
/// It is what `fold_constant_reads` asks before folding a constant into a
/// consumer.
///
/// The second half turns the question on us: every operand WE emit, against
/// the same rule. A disagreement is not proof of a defect -- the reference's
/// silence is not the machine's refusal -- but it is a place where we do
/// something no shipped shader does, and those are worth knowing by name.
fn which_operations_the_reference_gives_a_non_temporary_operand() {
    /* Pairs the table shows only since 220 rows were appended to it
     * (2026-10-12, issue #22): Mesa does give these operations a uniform or
     * an immediate operand. `OpId::takes_group` still refuses them, which
     * costs an instruction each (the move into a temporary) and changes no
     * output; widening it would change what the online compiler emits for
     * every shader that uses them, and nothing has run that on a board. The
     * other direction stays strict: an operand we allow that the reference
     * never shows still fails, and so does an entry here that the reference
     * no longer shows or the rule now allows. A follow-up can widen the rule
     * with a board run. */
    const STILL_REFUSED: &[(&str, u32)] = &[
        ("exp", 2),
        ("frc", 2),
        ("log", 2),
        ("or", 2),
        ("rsq", 2),
        ("set", 7),
        ("texld", 2),
        ("texldb", 2),
        ("xor", 7),
    ];
    use yakogl::shaderc::online::vivante::build::op_id;

    let mut seen: BTreeMap<(&'static str, u32), usize> = BTreeMap::new();
    for b in SHC_BLOB_TABLE.iter() {
        for q in b.shader.usc_words().as_chunks::<4>().0 {
            let Ok(ins) = decode_instr(&[q[0], q[1], q[2], q[3]]) else { continue };
            for n in 0..3 {
                if let Some(g) = ins.get(&format!("SRC{n}_RGROUP")) {
                    *seen.entry((ins.op().name, g)).or_insert(0) += 1;
                }
            }
        }
    }
    assert!(seen.len() > 20, "the reference corpus decoded into nothing; the reader or the table moved");

    /* Every operation that appears at all, against the compiler's rule. An
     * operation the reference never shows with a group must be refused for
     * it, and one it shows in quantity must be allowed. */
    let ops: std::collections::BTreeSet<&str> = seen.keys().map(|(o, _)| *o).collect();
    let mut drift: Vec<String> = Vec::new();
    for (op, g) in STILL_REFUSED {
        let n = seen.get(&(*op, *g)).copied().unwrap_or(0);
        let allowed = op_id(op).is_some_and(|id| id.takes_group(*g as u8));
        if n == 0 || allowed {
            drift.push(format!("STILL_REFUSED {op} group {g} is stale: reference {n}, takes_group {allowed}"));
        }
    }
    for op in &ops {
        let Some(id) = op_id(op) else { continue };
        for g in [1u32, 2, 3, 4, 7] {
            let n = seen.get(&(op, g)).copied().unwrap_or(0);
            let allowed = id.takes_group(g as u8);
            if n > 0 && !allowed && STILL_REFUSED.contains(&(*op, g)) {
                continue;
            }
            if (n > 0) != allowed {
                drift.push(format!("{op} group {g}: reference {n}, takes_group {allowed}"));
            }
        }
    }
    assert!(drift.is_empty(), "OpId::takes_group and the reference disagree: {drift:?}");
    println!("\n=== {} operations, {} (operation, group) pairs in the reference ===", ops.len(), seen.len());
}

/// Where WE give an operand the reference never gives that operation.
///
/// Printed, not asserted: the reference's silence is evidence about what the
/// machine is known to accept, not proof of what it refuses, and a shader
/// that draws on the board is not made wrong by being unusual. But each row
/// here is a construct no shipped shader uses, so each is a candidate the
/// day a picture is wrong for no visible reason.
fn where_we_give_an_operand_the_reference_never_gives_that_operation() {
    use core::ffi::c_char;
    use yakogl::shaderc::mesa_reference::SHC_SOURCE_TABLE;
    use yakogl::shaderc::online::vivante::build::op_id;
    use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

    let mut odd: BTreeMap<(String, u32), usize> = BTreeMap::new();
    let (mut n, mut instrs) = (0usize, 0usize);
    for e in SHC_SOURCE_TABLE.iter() {
        let txt = e.text.trim_end_matches('\0');
        let key = yakogl::shaderc::sha256::sha256(txt.as_bytes());
        let Some(b) = SHC_BLOB_TABLE.iter().find(|b| b.sha256 == key && b.blend_key == 0) else { continue };
        let stage = if b.stage == SHC_STAGE_FRAGMENT { SHC_STAGE_FRAGMENT } else { SHC_STAGE_VERTEX };
        yakogl::shaderc::online::backend::forget_compiled();
        let mut out = ShcShader::ZERO;
        let mut log = [0 as c_char; 512];
        let mut src = String::from(txt);
        src.push('\0');
        let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
        // SAFETY: a NUL-terminated source, a real output, a log of the size announced.
        let rc = unsafe { compile(stage, src.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
        if rc != 0 {
            continue;
        }
        n += 1;
        for q in out.usc_words().as_chunks::<4>().0 {
            let Ok(ins) = decode_instr(&[q[0], q[1], q[2], q[3]]) else { continue };
            instrs += 1;
            let Some(id) = op_id(ins.op().name) else { continue };
            for k in 0..3 {
                let Some(g) = ins.get(&format!("SRC{k}_RGROUP")) else { continue };
                if !id.takes_group(g as u8) {
                    *odd.entry((ins.op().name.to_string(), g)).or_insert(0) += 1;
                }
            }
        }
    }
    assert!(n > 100 && instrs > 1000, "the corpus did not compile; {n} sources, {instrs} instructions");
    println!("\n=== our {instrs} instructions over {n} sources, against the reference's per-operation groups ===");
    if odd.is_empty() {
        println!("  every operand is one the reference gives that operation");
    }
    for ((op, g), k) in &odd {
        println!("  {k:>5}  {op} takes a {} operand, which the reference never gives it", group_name(*g));
    }
}

/// All of it, in ONE test.
///
/// `where_we_give_an_operand...` compiles the corpus, which resets the
/// compiler's keep arena, and `cargo` runs a binary's tests on parallel
/// threads: the other four would then be reading a table pulled out from
/// under them. That is `test_rules.rs`'s rule, and it caught this file --
/// five tests -- at the gate, which is what a structural check is for.
///
/// `show_a_small_blob` is the helper that chose the control blob rather than
/// a check; it runs only when asked, and it is kept because the NEXT control
/// has to be chosen the same way.
#[test]
fn the_operand_bank_surveys() {
    the_control_blob_reads_one_temp_and_one_uniform();
    how_many_operands_of_one_group_an_instruction_holds();
    which_operations_the_reference_gives_a_non_temporary_operand();
    where_we_give_an_operand_the_reference_never_gives_that_operation();
    if std::env::var_os("YAKOGL_SHOW_SMALL_BLOB").is_some() {
        show_a_small_blob();
    }
}
