// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! How many instructions we emit, and how many of them are copies.
//!
//! The score says how many shaders compile; it says nothing about how well.
//! This is the quality gauge: total instructions and `mov` count over the
//! sources that compile, beside the reference compiler's numbers for the same
//! sources. A pass that is supposed to remove copies is measured here before
//! and after, so "it should help" is never the evidence.
//!
//! ```sh
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online \
//!     --test mov_survey -- --nocapture
//! ```
//!
//! Prints rather than asserts: these are quality numbers and they move with
//! every front-end change, so a threshold here would be a tripwire on someone
//! else's work.

#![cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]

use core::ffi::c_char;
use std::collections::BTreeMap;
use yakogl::shaderc::online::vivante::alloc;
use yakogl::shaderc::online::vivante::decode::decode_instr;
use yakogl::shaderc::table::{SHC_BLOB_TABLE, SHC_SOURCE_TABLE};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// Count real instructions and copies, ignoring the `nop` that an empty
/// program is.
fn count(w: &[u32], total: &mut usize, movs: &mut usize, mut hist: Option<&mut BTreeMap<&'static str, usize>>) {
    for q in w.as_chunks::<4>().0 {
        let Ok(i) = decode_instr(q) else { continue };
        if i.op().name == "nop" {
            continue;
        }
        *total += 1;
        if i.op().name == "mov" {
            *movs += 1;
        }
        if let Some(h) = hist.as_deref_mut() {
            *h.entry(i.op().name).or_insert(0) += 1;
        }
    }
}

fn how_many_instructions_and_how_many_are_copies() {
    /* coalescable, live-across, seed, io -- the allocator's partition. */
    let mut part = (0usize, 0usize, 0usize, 0usize);
    /* Of the coalescable copies: takeable by a rename, blocked by a swizzle,
     * blocked by a modifier or a condition, and not a join at all. */
    let mut ready = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut mov_transformed = 0usize;
    let (mut ready_straight, mut ready_branchy) = (0usize, 0usize);
    let (mut branchy_sources, mut branchy_instrs) = (0usize, 0usize);
    let (mut branches, mut branches_back) = (0usize, 0usize);
    let (mut straight_sources, mut sum_overlap, mut sum_split) = (0usize, 0usize, 0usize);
    let mut read_only = 0usize;
    let mut worst: Vec<(usize, &str)> = Vec::new();
    /* store_local, load_local, if_seed, store_output, uniform_idx. */
    let mut sites = (0usize, 0usize, 0usize, 0usize, 0usize);
    /* uniform, immediate, other, read-once, uniform read-once, imm read-once. */
    let mut seeds = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    /* of the read-once immediates: consumer is a mov, an ALU, or nothing. */
    let mut cons = (0usize, 0usize, 0usize);
    /* folded, over cap, wrong operation, immediate modifier, unsound. */
    let mut fold = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut dce = 0usize;
    /* swizzled, not a value, read twice, a slot, not the last instruction. */
    let mut rt = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut legal = 0usize;
    let mut remap = 0usize;
    /* slot -> slot stores: destination under control flow, or not. */
    let mut s2s = (0usize, 0usize);
    /* of the load copies: slot under control flow, slot with a masked store. */
    let mut ll = (0usize, 0usize, 0usize);
    /* copies a per-component slot rename could take: loads, stores. */
    let mut one_value = (0usize, 0usize);
    let (mut ours_total, mut ours_mov, mut n) = (0usize, 0usize, 0usize);
    let (mut ref_total, mut ref_mov) = (0usize, 0usize);
    let mut ops: BTreeMap<&'static str, usize> = BTreeMap::new();

    for e in SHC_SOURCE_TABLE.iter() {
        let txt = e.text.trim_end_matches('\0');
        let key = yakogl::shaderc::sha256::sha256(txt.as_bytes());
        let Some(b) = SHC_BLOB_TABLE.iter().find(|b| b.sha256 == key && b.blend_key == 0) else { continue };
        let stage = if b.stage == SHC_STAGE_FRAGMENT { SHC_STAGE_FRAGMENT } else { SHC_STAGE_VERTEX };
        yakogl::shaderc::online::backend::forget_compiled();
        let mut out = ShcShader::ZERO;
        let mut log = [0 as c_char; 512];
        let mut s = String::from(txt);
        s.push('\0');
        let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
        // SAFETY: a NUL-terminated source, a real output, a log of the size announced.
        let rc = unsafe { compile(stage, s.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
        if rc != 0 {
            continue;
        }
        n += 1;
        count(out.usc_words(), &mut ours_total, &mut ours_mov, Some(&mut ops));
        count(b.shader.usc_words(), &mut ref_total, &mut ref_mov, None);

        /* Valid for the program most recently allocated, which is the one
         * just compiled: per source, so the item has a size AND a place to
         * start rather than only a total. */
        let (_m, co, ov, sd, io) = alloc::last_mov_breakdown();
        let (rd, sw, tr, cn, nj) = alloc::last_coalesce_ready();
        mov_transformed += alloc::last_mov_transformed();
        if alloc::last_had_branch() {
            ready_branchy += rd;
            branchy_sources += 1;
            branchy_instrs += out.usc_words().len() / 4;
            let (br, back) = alloc::last_branches();
            branches += br;
            branches_back += back;
        } else {
            ready_straight += rd;
            straight_sources += 1;
            sum_overlap += alloc::last_overlap();
            sum_split += alloc::last_split_overlap();
            read_only += alloc::last_read_only_vregs();
        }
        ready.0 += rd;
        ready.1 += sw;
        ready.2 += tr;
        ready.3 += cn;
        ready.4 += nj;
        part.0 += co;
        part.1 += ov;
        part.2 += sd;
        part.3 += io;
        /* WHAT each seed materialised, from the operand side: a uniform read
         * and an immediate fold into their consumers, anything else does not.
         * Summed here so the fold in `select.rs` is measured before and after
         * in the same run as the instruction count it moves. */
        let (su, si, so, once, uonce, ionce) = alloc::last_seed_breakdown();
        seeds.0 += su;
        seeds.1 += si;
        seeds.2 += so;
        seeds.3 += once;
        seeds.4 += uonce;
        seeds.5 += ionce;
        let (to_mov, to_alu, imm_dead) = alloc::last_imm_consumers();
        cons.0 += to_mov;
        cons.1 += to_alu;
        cons.2 += imm_dead;
        if ov > 0 {
            worst.push((ov, e.name));
        }
        /* Which construct wrote each copy -- the other half of the same
         * question, and visible only in selection. */
        let k = yakogl::shaderc::online::select::last_mov_sites();
        sites.0 += k.store_local;
        sites.1 += k.load_local;
        sites.2 += k.if_seed;
        sites.3 += k.store_output;
        sites.4 += k.uniform_idx;
        /* The constant fold, and the size of what it declined. */
        fold.0 += k.folded;
        fold.1 += k.fold_over_cap;
        fold.2 += k.fold_wrong_op;
        fold.3 += k.fold_modifier;
        fold.4 += k.fold_unsound;
        dce += k.dead_deleted;
        rt.0 += k.retarget_swizzled;
        rt.1 += k.retarget_not_a_value;
        rt.2 += k.retarget_read_twice;
        rt.3 += k.retarget_is_a_slot;
        rt.4 += k.retarget_not_last;
        legal += k.legalised;
        remap += k.retarget_lane_remap;
        s2s.0 += k.slot_to_slot_under_cf;
        s2s.1 += k.slot_to_slot_plain;
        ll.0 += k.load_local_cf;
        ll.1 += k.load_local_masked;
        ll.2 += k.load_local_joinable;
        one_value.0 += k.load_one_value;
        one_value.1 += k.store_one_value;
    }

    let pct = |a: usize, b: usize| a * 100 / b.max(1);
    println!("\n=== instructions across the {n} sources that compile ===\n");
    println!("  ours       {ours_total:>6} instructions, {ours_mov:>5} movs ({}%)", pct(ours_mov, ours_total));
    println!("  reference  {ref_total:>6} instructions, {ref_mov:>5} movs ({}%)", pct(ref_mov, ref_total));
    println!("  ours / reference = {:.2}x", ours_total as f64 / ref_total.max(1) as f64);

    /* WHAT THE COPIES ARE, which is what decides whose work each one is:
     * coalescable and io are the allocator's, a seed is a value materialised
     * that an operand could have named, and LIVE-ACROSS is neither -- both
     * values are live past the copy, so it is a value needed in two places,
     * which is a shape selection chose. The allocator's partition (d89db01),
     * summed here so a change to selection is measured in the same run as the
     * instruction count it moves. */
    println!(
        "  of our {ours_mov} movs, {mov_transformed} carry neg/abs/sat: fneg, fabs and fsat, which are not copies"
    );
    println!(
        "\n  our {} movs: {} coalescable + {} live-across + {} seed + {} io",
        ours_mov, part.0, part.1, part.2, part.3
    );
    println!(
        "  by the construct that wrote them: {} store_local + {} load_local + {} if_seed + {} store_output + {} uniform_idx",
        sites.0, sites.1, sites.2, sites.3, sites.4
    );
    /* And of the coalescable ones, how many a rename could actually take.
     * "The ranges do not overlap" is the textbook condition; this machine
     * adds that the copy must not have swizzled, negated, saturated or been
     * conditional, because deleting it leaves every later reader reading the
     * source in place. Sizing the pass from the 463 alone would be sizing it
     * from a number that includes copies it cannot touch. */
    /* The rename is only sound where the live ranges mean what they say. A
     * copy inside a conditional arm happens only when the arm runs, and the
     * range model here is linear -- it does not know that. So the takeable
     * ones are split by whether their program has a branch at all, and only
     * the straight-line half is what a pass built on these ranges may have. */
    println!("  of those takeable, {ready_straight} are in straight-line programs and {ready_branchy} in programs with a branch");
    /* How big the branch-aware-liveness item is, before anyone writes it: the
     * linear ranges this allocator uses are exact on a straight-line program
     * and only an upper bound on one with control flow, so everything that
     * bound costs -- registers and the 73 copies above -- is confined to
     * these sources. Sized here rather than guessed, which is the lesson the
     * coalescing item taught three times over. */
    println!(
        "  programs with a branch: {branchy_sources} of {n} sources, {branchy_instrs} of {ours_total} instructions"
    );
    println!("  {branches} branches in all, {branches_back} of them backward (a backward edge is a loop)");
    /* What non-SSA selection output costs the allocator. A value written
     * twice has ONE linear range, from its first write to its last read, so
     * it is counted live across everything in between even where it holds
     * nothing. Splitting the range at each redefinition is the fix; this is
     * its size, over the straight-line programs where splitting is sound. */
    println!(
        "  over {straight_sources} straight-line sources: peak live {sum_overlap} as ranged, {sum_split} if split at each redefinition, {read_only} read but never written"
    );
    println!(
        "  of the {} coalescable: {} a rename can take, {} swizzled, {} not copies (neg/abs/sat MOV), {} conditional, {} dead stores",
        part.0, ready.0, ready.1, ready.2, ready.3, ready.4
    );
    assert_eq!(
        ready.0 + ready.1 + ready.2 + ready.3 + ready.4,
        part.0,
        "the four reasons must partition the coalescable copies, or one of them is being counted twice"
    );
    println!(
        "  the {} seeds: {} uniform + {} immediate + {} other; {} read at most once ({} uniform, {} immediate)",
        part.2, seeds.0, seeds.1, seeds.2, seeds.3, seeds.4, seeds.5
    );
    println!(
        "  of the read-once immediates: {} feed a mov, {} feed an ALU operand, {} feed nothing",
        cons.0, cons.1, cons.2
    );
    println!(
        "  the constant fold: {} seeds deleted; declined {} over a group's cap + {} operation never takes the group + {} an immediate under a modifier + {} not safe to move",
        fold.0, fold.1, fold.2, fold.3, fold.4
    );
    println!("  dead-code elimination deleted {dce}; legalisation added {legal}");
    println!(
        "  why a store could not be retargeted: {} swizzled + {} not a value register + {} value read twice + {} a slot's register + {} not the last instruction",
        rt.0, rt.1, rt.2, rt.3, rt.4
    );
    println!("    and {remap} more had their producer's lanes remapped instead of a copy");
    println!(
        "    of the slot-register ones, {} store into a slot an arm touches and {} into one no arm touches",
        s2s.0, s2s.1
    );
    println!(
        "  of the {} load copies: {} the slot is under control flow, {} a store is masked, {} neither",
        sites.1,
        ll.0,
        ll.1,
        sites.1 - ll.0 - ll.1
    );
    println!("    of those {} under control flow, a select at the join could carry {}", ll.0, ll.2);
    println!(
        "  per-component slot tracking would take {} load copies and {} store copies\n    \
         (only the slots whose masked stores all come from ONE value: the rest cannot be read as one operand)",
        one_value.0, one_value.1
    );
    worst.sort_by_key(|(n, _)| core::cmp::Reverse(*n));
    println!("  the sources with the most live-across copies:");
    for (k, name) in worst.iter().take(8) {
        println!("    {k:>4}  {name}");
    }
    println!();
    println!("  our op mix: {ops:?}");
}

/// Why a slot could not be renamed, per source.
///
/// The rename in `select.rs` removes both the store's copy and the load's,
/// but only for a slot no arm touches and whose every stored value has a
/// register. This says how many slots each shader has of each kind, so the
/// item is sized from the corpus rather than from one shader.
fn how_many_slots_could_be_renamed() {
    use yakogl::shaderc::online::arena::Arena;
    use yakogl::shaderc::online::{lower, sema};

    let (mut ok, mut cf, mut shape, mut n) = (0usize, 0usize, 0usize, 0usize);
    /* never stored, indexed, constructor-like, other partial. */
    let mut shapes = (0usize, 0usize, 0usize, 0usize, 0usize);
    let (mut join_ok, mut join_br, mut join_no) = (0usize, 0usize, 0usize);
    for e in SHC_SOURCE_TABLE.iter() {
        let stage = if e.name.contains(".vert") { sema::Stage::Vertex } else { sema::Stage::Fragment };
        let txt = e.text.trim_end_matches('\0');
        let mut mem = vec![0u8; 4 << 20];
        let a = Arena::new(&mut mem);
        let Ok(src) = a.alloc_str(txt) else { continue };
        let Ok(l) = lower::compile_to_ir(&a, stage, src) else { continue };
        n += 1;
        for i in 0..l.module.locals.len() as u32 {
            let id = yakogl::shaderc::online::ir::LocalId(i);
            let under = touched_under_cf(&l.module, id);
            let whole = every_store_is_whole(&l.module, id);
            if under {
                cf += 1;
                match join_reach(&l.module, id) {
                    JoinReach::Ok => join_ok += 1,
                    JoinReach::Branches => join_br += 1,
                    JoinReach::No => join_no += 1,
                }
            } else if !whole {
                shape += 1;
                match store_shape(&l.module, id) {
                    StoreShape::NeverStored => shapes.0 += 1,
                    StoreShape::Indexed => shapes.1 += 1,
                    StoreShape::ConstructorLike => {
                        shapes.2 += 1;
                        if masked_stores_share_one_value(&l.module, id) {
                            shapes.4 += 1;
                        }
                    }
                    StoreShape::PartialOther => shapes.3 += 1,
                    StoreShape::Whole => unreachable!("`whole` and `store_shape` disagree about this slot"),
                }
            } else {
                ok += 1;
            }
        }
    }
    println!("\n=== local slots over {n} sources ===");
    println!("  renamable (no arm touches it, every store whole)      {ok}");
    println!("  an arm of an `if` or loop touches it                  {cf}");
    println!("  a store is masked or indexed, or there is no store     {shape}");
    println!("\n  and what those {shape} really are:");
    println!("    never stored at all (costs no copy either)            {}", shapes.0);
    println!("    a dynamically indexed store                           {}", shapes.1);
    println!("    disjoint masked stores covering the slot's type       {}", shapes.2);
    println!("    masked stores that overlap or leave a component unset  {}", shapes.3);
    println!(
        "    ...of the {} covering ones, {} take every component from ONE value\n         \
         (the only ones per-component tracking can turn into a single operand)",
        shapes.2, shapes.4
    );
    println!("\n  of the arm-touched ones, how many could the JOIN reach:");
    println!("    every store whole-register, and every touching `if` speculable   {join_ok}");
    println!("    a touching arm holds a texture or a discard (branches; one arm runs) {join_br}");
    println!("    a touching arm is a loop, or a store is masked/indexed            {join_no}");
}

fn touched_under_cf(m: &yakogl::shaderc::online::ir::Module<'_>, local: yakogl::shaderc::online::ir::LocalId) -> bool {
    use yakogl::shaderc::online::ir::{Node, Op, Range};
    fn go(
        m: &yakogl::shaderc::online::ir::Module<'_>,
        r: Range,
        local: yakogl::shaderc::online::ir::LocalId,
        nested: bool,
    ) -> bool {
        m.nodes_of(r).iter().any(|nd| match *nd {
            Node::Instr(id) => {
                nested
                    && matches!(m.instr(id), Some(Op::LoadLocal { local: l, .. } | Op::StoreLocal { local: l, .. }) if *l == local)
            }
            Node::If { then_, else_, .. } => go(m, then_, local, true) || go(m, else_, local, true),
            Node::Loop { body } => go(m, body, local, true),
            _ => false,
        })
    }
    go(m, m.body, local, false)
}

/// Every store to this slot writes the whole register, and there is one.
///
/// What blocks the OPERAND rename. This used to ALSO require the stored value
/// to have a register of its own -- what blocked the REGISTER rename -- two
/// different questions in one predicate, which stopped being right the moment
/// the operand form landed. Leaving them merged would print a number that
/// means neither.
/// What a slot's stores look like, for the slots `every_store_is_whole`
/// refuses. Its `false` covers four different situations and they have four
/// different fixes, so counting them as one is how an item gets the wrong
/// size -- which has happened to me twice on this file.
#[derive(PartialEq, Eq, Debug)]
enum StoreShape {
    /// Every store writes the whole register: renamable as far as shape goes.
    Whole,
    /// Never stored at all. Not a rename candidate and not a cost either --
    /// no copy is emitted for a slot nothing writes.
    NeverStored,
    /// A dynamically indexed store: the register is chosen at run time.
    Indexed,
    /// Masked stores whose masks are disjoint and together cover **the
    /// components the slot's type has**: a constructor, `vec4(a, b, c, d)`
    /// written lane by lane, or a `float` written in its x lane. The value
    /// exists in pieces and the slot is where they are gathered.
    ///
    /// Against the TYPE and not against `0b1111`, which is the correction
    /// this enum was born wrong with: a `float` slot writes mask `0001` and
    /// its other three lanes are not a hole, they do not exist. Counting
    /// them as one put 487 slots in the bucket below and made the item look
    /// like something it is not.
    ConstructorLike,
    /// Masked stores that overlap, or that leave a component of the type
    /// unwritten.
    PartialOther,
}

/// Of a masked-store slot, do ALL its stores take their value from ONE value?
///
/// The question that sizes per-component slot tracking, and the one that says
/// why the item is smaller than the copies it is blamed for. Tracking a
/// slot's components separately only removes a copy if a later READ of the
/// slot can still be ONE operand: this is a vector machine, and an operand is
/// four lanes of one register through one swizzle. Four components living in
/// four registers cannot be read as one operand at all, so the gather that
/// the masked stores perform today is not waste, it is the work.
///
/// `vec4 c; c.xy = a.xy; c.zw = a.zw;` is the case that pays: every component
/// comes from `a`, so a read of `c` is `a` with a swizzle and both stores and
/// the load go. `vec4 c; c.x = f; c.y = g;` is not, whatever the tracking.
fn masked_stores_share_one_value(
    m: &yakogl::shaderc::online::ir::Module<'_>,
    local: yakogl::shaderc::online::ir::LocalId,
) -> bool {
    use yakogl::shaderc::online::ir::Op;
    let mut from: Option<u32> = None;
    let mut any = false;
    for op in m.instrs.iter() {
        let Op::StoreLocal { local: l, index, src, .. } = op else { continue };
        if *l != local {
            continue;
        }
        if index.is_some() {
            return false;
        }
        any = true;
        match from {
            None => from = Some(src.val.0),
            Some(v) if v == src.val.0 => {}
            Some(_) => return false,
        }
    }
    any
}

fn store_shape(m: &yakogl::shaderc::online::ir::Module<'_>, local: yakogl::shaderc::online::ir::LocalId) -> StoreShape {
    use yakogl::shaderc::online::ir::Op;
    let (mut n, mut whole, mut indexed) = (0usize, 0usize, 0usize);
    let (mut union, mut overlap) = (0u8, false);
    for op in m.instrs.iter() {
        let Op::StoreLocal { local: l, index, write_mask, .. } = op else { continue };
        if *l != local {
            continue;
        }
        n += 1;
        if index.is_some() {
            indexed += 1;
            continue;
        }
        if *write_mask == 0b1111 {
            whole += 1;
        }
        if union & *write_mask != 0 {
            overlap = true;
        }
        union |= *write_mask;
    }
    if n == 0 {
        return StoreShape::NeverStored;
    }
    if indexed > 0 {
        return StoreShape::Indexed;
    }
    if whole == n {
        return StoreShape::Whole;
    }
    /* The components the slot's type actually has: a `float` slot is one
     * lane wide and its mask is `0001`, which covers it. */
    let comps = m.locals.get(local.0 as usize).map_or(4, |d| d.ty.comps).clamp(1, 4);
    let all = ((1u16 << comps) - 1) as u8;
    if !overlap && union == all {
        return StoreShape::ConstructorLike;
    }
    StoreShape::PartialOther
}

/// A `float` slot written in its x lane is COVERED, not full of holes.
///
/// The assertion the lesson belongs in, rather than the comment it was. The
/// first version of `store_shape` compared each slot's mask union against
/// `0b1111`, so a one-component slot -- whose other three lanes do not exist
/// -- was classed as a partial write, and the survey reported 487 slots in a
/// bucket that really holds 80. Nothing failed; the table was simply wrong,
/// and an item was sized from it.
///
/// A test that builds both slots and asserts both answers fails if anyone
/// compares against a constant width again.
fn a_float_slots_single_lane_covers_it_and_a_vec4s_does_not() {
    use yakogl::shaderc::online::ir;

    let locals = [
        ir::VarDecl { name: "f", ty: ir::Type::new(ir::ScalarType::F32, 1), array_len: 1 },
        ir::VarDecl { name: "v", ty: ir::Type::F32X4, array_len: 1 },
    ];
    let instrs = [
        ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
        /* `float f = <x>;` -- one lane, and that is the whole of it. */
        ir::Op::StoreLocal { local: ir::LocalId(0), index: None, src: ir::Src::id(ir::ValueId(0)), write_mask: 0b0001 },
        /* `vec4 v; v.x = <x>;` -- one lane of four, which leaves three. */
        ir::Op::StoreLocal { local: ir::LocalId(1), index: None, src: ir::Src::id(ir::ValueId(0)), write_mask: 0b0001 },
    ];
    let nodes: [ir::Node; 3] = core::array::from_fn(|i| ir::Node::Instr(ir::InstrId(i as u32)));
    let m = ir::Module {
        instrs: &instrs,
        nodes: &nodes,
        locals: &locals,
        body: ir::Range { first: 0, len: 3 },
        ..ir::Module::empty(yakogl::shaderc::SHC_STAGE_VERTEX)
    };
    assert_eq!(
        store_shape(&m, ir::LocalId(0)),
        StoreShape::ConstructorLike,
        "a float slot's x lane covers its type; the other three are not holes, they do not exist"
    );
    assert_eq!(
        store_shape(&m, ir::LocalId(1)),
        StoreShape::PartialOther,
        "a vec4 slot with only x written really does leave three components unset"
    );
}

fn every_store_is_whole(
    m: &yakogl::shaderc::online::ir::Module<'_>,
    local: yakogl::shaderc::online::ir::LocalId,
) -> bool {
    use yakogl::shaderc::online::ir::Op;
    let mut stored = false;
    for op in m.instrs.iter() {
        let Op::StoreLocal { local: l, index, write_mask, .. } = op else { continue };
        if *l != local {
            continue;
        }
        if index.is_some() || *write_mask != 0b1111 {
            return false;
        }
        stored = true;
    }
    stored
}

/// Could renaming through an `if`'s join reach this arm-touched slot?
///
/// Three answers, because they are three different pieces of work. A join
/// rename only works where BOTH arms run -- if-conversion -- since the operand
/// an arm records is defined only on its own path. An arm holding a texture
/// fetch or a `discard` gets a real branch instead (`must_not_speculate` in
/// `select.rs`), so those slots are out of reach until the branch path grows
/// its own join.
enum JoinReach {
    Ok,
    Branches,
    No,
}

fn join_reach(m: &yakogl::shaderc::online::ir::Module<'_>, local: yakogl::shaderc::online::ir::LocalId) -> JoinReach {
    use yakogl::shaderc::online::ir::{Node, Op, Range};

    fn speculable(m: &yakogl::shaderc::online::ir::Module<'_>, r: Range) -> bool {
        m.nodes_of(r).iter().all(|n| match *n {
            Node::Instr(id) => !matches!(m.instr(id), Some(Op::Tex { .. } | Op::Discard)),
            Node::If { then_, else_, .. } => speculable(m, then_) && speculable(m, else_),
            _ => false,
        })
    }

    /* Every store to the slot must write the whole register, wherever it is. */
    for op in m.instrs.iter() {
        let Op::StoreLocal { local: l, index, write_mask, .. } = op else { continue };
        if *l == local && (index.is_some() || *write_mask != 0b1111) {
            return JoinReach::No;
        }
    }

    fn walk(
        m: &yakogl::shaderc::online::ir::Module<'_>,
        r: Range,
        local: yakogl::shaderc::online::ir::LocalId,
        out: &mut JoinReach,
    ) {
        for n in m.nodes_of(r) {
            match *n {
                Node::If { then_, else_, .. } => {
                    let touches = |rr| {
                        m.nodes_of(rr).iter().any(|x| match *x {
                            Node::Instr(id) => matches!(
                                m.instr(id),
                                Some(Op::LoadLocal { local: l, .. } | Op::StoreLocal { local: l, .. }) if *l == local
                            ),
                            _ => true,
                        })
                    };
                    if (touches(then_) || touches(else_)) && !(speculable(m, then_) && speculable(m, else_)) {
                        *out = JoinReach::Branches;
                    }
                    walk(m, then_, local, out);
                    walk(m, else_, local, out);
                }
                Node::Loop { body } => {
                    let touches = m.nodes_of(body).iter().any(|x| match *x {
                        Node::Instr(id) => matches!(
                            m.instr(id),
                            Some(Op::LoadLocal { local: l, .. } | Op::StoreLocal { local: l, .. }) if *l == local
                        ),
                        _ => true,
                    });
                    if touches {
                        *out = JoinReach::No;
                    }
                }
                _ => {}
            }
        }
    }

    let mut out = JoinReach::Ok;
    walk(m, m.body, local, &mut out);
    out
}

/// Both surveys, in ONE test.
///
/// The first compiles through `SHC_ONLINE`, which resets the compiler's keep
/// arena; the second walks the IR in an arena of its own. They do not collide
/// today -- measured stable at 6,121 instructions over four runs -- and they
/// are one test anyway, because `temps_survey` had exactly this shape, its
/// two tests DID collide, and its corpus count swung from 10 to 108 to 10 on
/// identical code while passing. A binary that resets the compiler's global
/// state runs one test; that is the rule `test_rules.rs` now checks, and an
/// exception nobody can see the reason for is how the rule gets worn away.
#[test]
fn the_mov_surveys() {
    how_many_instructions_and_how_many_are_copies();
    how_many_slots_could_be_renamed();
    /* Not a survey but an assertion, and it lives here because it is about
     * `store_shape`, which the second survey uses. One test per binary is the
     * rule whatever the test is for. */
    a_float_slots_single_lane_covers_it_and_a_vec4s_does_not();
}
