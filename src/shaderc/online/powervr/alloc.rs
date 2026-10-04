// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Register allocation for the USC: virtual registers onto the temporary
//! file, around whatever selection pinned.
//!
//! The GC7000's allocator (`vivante/alloc.rs`) is the sibling, and three of
//! its lessons are built in here rather than rediscovered:
//!
//! - **Registers the program never mentions still have to be reserved.**
//!   There, `gl_FragCoord` and the io registers were pinned by the hardware
//!   and the ABI, named by no instruction after copy propagation, and quietly
//!   handed to a temporary. Both were wrong pictures rather than errors. So
//!   here the pinned set is an explicit input and is never inferred.
//! - **Greedy colouring of live ranges is optimal only in start order.**
//!   Assigning in virtual-register order is *close* on a front end that
//!   numbers in creation order, and close is what leaves registers unused.
//! - **A bound must be the one the hardware is told, not the one an
//!   instruction can address.** There the encoding held 128 registers and the
//!   driver's field held 63, and the mismatch put 67 on the wire as 3. Here
//!   the file sizes are supplied by the caller for exactly that reason: this
//!   file does not know what the USC state declares, and a number invented
//!   here would be the same defect with a different constant.

use super::sel::{Bank, SelDst, SelGroup, SelSrc, VReg};
use super::select::Selected;
use crate::shaderc::online::arena::{Arena, ArenaError};

/// Which register file ran out. A refusal naming only a count would cost the
/// debugging cycle the GC7000's version did, and there is more than one file
/// here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegFile {
    Temp,
    Shared,
    VtxIn,
    Coeff,
}

impl RegFile {
    fn of(bank: Bank) -> Option<RegFile> {
        match bank {
            Bank::Temp => Some(RegFile::Temp),
            Bank::Shared => Some(RegFile::Shared),
            Bank::VtxIn => Some(RegFile::VtxIn),
            Bank::Coeff => Some(RegFile::Coeff),
            // The special file is the ISA's own constants and hardware
            // values. Nothing is allocated in it and nothing is counted.
            Bank::Special => None,
            // An index register, or a read through one: its index is a
            // selector, not a register number of any file this counts.
            Bank::Idx0 => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AllocError {
    /// More registers needed in one file than it holds. Carries **what the
    /// program required**, not the first index that missed: sizing anything
    /// from "where the walk stopped" invites a second raise, because that
    /// figure was never the requirement. On the GC7000 the true peak was 350
    /// where the reported number was 262.
    OutOfRegisters { file: RegFile, needed: usize },
    /// A virtual register outside the range this allocator tracks.
    TooManyValues { needed: usize },
    /// The arena could not hold the working set or the resolved program.
    ///
    /// Reported rather than trimmed: the whole point of allocating from the
    /// caller's scratch is that "this shader needs a bigger buffer" is an
    /// answer the caller can act on, where a short array is a wrong picture.
    Scratch(ArenaError),
    /// The buffer handed to [`boundary_of`] cannot hold the io VRegs.
    BoundaryBufferTooSmall { needed: usize },
    /// A program whose loop markers do not pair up, named.
    Unsupported(&'static str),
}

/// What [`allocate`] writes into `assignment` for a virtual register no group
/// and no boundary mentions. Reading a register out of one of these is the
/// bug it exists to make loud.
pub const UNASSIGNED: u16 = u16::MAX;

/// The values that live across the program's edges, where no group mentions
/// them.
///
/// **This is the whole reason the allocator can be right.** Selection does
/// not emit the io writes as groups, and says so: a vertex output is a
/// `uvsw_write` and the fragment colour is a move to `po0`, neither of which
/// is an `ir::Op`. They live in [`Selected::outputs`] and [`Selected::inputs`]
/// instead. Handed only the groups, the allocator sees a value whose one
/// consumer is an output write as dead at the instruction that defines it,
/// and recycles its register immediately.
///
/// That was not a hypothesis. On main at 97d9752, `texturefill.batchable.vert`
/// defined `VReg(152)`, `VReg(153)` and `VReg(154)` in three consecutive
/// groups, no later group read any of them, and all three came out as
/// `Temp2` -- three of the four components of the vertex position, in one
/// register. The colouring was correct for the picture it was given.
///
/// Named fields, not two `IntoIterator<Item = VReg>` arguments, on purpose:
/// those two would be swappable at a call site with no compile error and no
/// symptom short of silicon.
///
/// Build it with [`boundary_of`] rather than by hand.
#[derive(Clone, Copy, Default)]
pub struct Boundary<'a> {
    /// VRegs written **before the first group** -- a fragment varying's
    /// iterator, an attribute fetch. Their live ranges start at 0 however
    /// late the first group that reads them is.
    pub live_in: &'a [VReg],
    /// VRegs read **after the last group** by the builder's output writes.
    /// Their live ranges run to the end of the program.
    pub live_out: &'a [VReg],
}

impl Boundary<'_> {
    /// No values cross either edge.
    ///
    /// Honest for a hand-built group list in a test. For a real program it
    /// says the results are computed and discarded, so prefer
    /// [`boundary_of`], which cannot forget one.
    pub const NONE: Boundary<'static> = Boundary { live_in: &[], live_out: &[] };
}

/// The boundary of a selected program, into a caller-owned buffer.
///
/// One function rather than a documented rule, because "which VRegs cross the
/// edge" is exactly the kind of rule every caller would re-derive and one of
/// them would get subtly wrong, with no symptom until a register came back
/// holding another component.
///
/// `buf` needs `4 * (inputs.len() + outputs.len())` entries: four components
/// per io slot. Too small is a refusal carrying the requirement.
pub fn boundary_of<'b>(sel: &Selected<'_>, buf: &'b mut [VReg]) -> Result<Boundary<'b>, AllocError> {
    let needed = 4 * (sel.inputs.len() + sel.outputs.len())
        + usize::from(sel.discard.is_some())
        + 2 * usize::from(sel.alpha_to_one.is_some());
    if buf.len() < needed {
        return Err(AllocError::BoundaryBufferTooSmall { needed });
    }
    let mut n = 0usize;
    for (_, comps) in sel.inputs {
        for v in comps.iter().flatten() {
            buf[n] = *v;
            n += 1;
        }
    }
    let n_in = n;
    for (_, comps) in sel.outputs {
        for s in comps.iter().flatten() {
            /* Only a virtual source crosses the edge. An output written
             * straight from a uniform or an immediate names no value the
             * allocator places. */
            if let SelSrc::Virtual { v, .. } = s {
                buf[n] = *v;
                n += 1;
            }
        }
    }
    /* The discard flag, read by the builder's feedback `tst`, and a blend
     * variant's alpha-to-one pair, read by its blend tail. */
    let a2one = sel.alpha_to_one.map_or([None, None], |(x, f)| [Some(x), Some(f)]);
    for s in sel.discard.iter().chain(a2one.iter().flatten()) {
        if let SelSrc::Virtual { v, .. } = s {
            buf[n] = *v;
            n += 1;
        }
    }
    let buf: &'b [VReg] = buf;
    Ok(Boundary { live_in: &buf[..n_in], live_out: &buf[n_in..n] })
}

/// How large each file is **on the part being compiled for**.
///
/// Supplied rather than assumed. This file cannot see the USC state that
/// declares the temporary count, and the GC7000 taught the cost of guessing:
/// the bound that matters is the one the hardware is told, and a constant
/// here that disagreed with it would be silently truncated somewhere else.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// **252 on this part**, and the derivation matters more than the number.
    ///
    /// The count reaches the hardware through the PDS `DOUTU` `src0` field
    /// the driver builds for every kick (`pds_build_doutu_src0`,
    /// `driver/src/pvr_pds_render_gen.c`). That field is six bits and holds
    /// **blocks of four temporaries**, not temporaries, so the largest count
    /// that can be expressed is `0x3f * 4 = 252`. Measured by
    /// pvrgpu-platform from the encoding every draw on this part has used,
    /// not transcribed from a reference driver.
    ///
    /// See [`declared_temps`]: because the field counts blocks, a program
    /// using five temporaries is declared eight, and saving a register only
    /// pays when it crosses a multiple of four.
    pub temps: u16,
    pub shared: u16,
    pub vtx_in: u16,
    pub coeff: u16,
}

/// What the hardware is actually told, given a count of temporaries used.
///
/// The PDS `DOUTU` field counts **blocks of four**, so a count rounds up: 5
/// temporaries are declared as 8, and 8 as 8. An allocator that saves a
/// register from 8 to 7 has saved nothing the hardware can see.
///
/// This exists so the difference is stated once, in the file that produces
/// the count, rather than discovered by someone comparing `Allocated::temps`
/// against a register dump and finding they disagree by three.
pub const fn declared_temps(used: u16) -> u16 {
    used.div_ceil(4) * 4
}

/// The most virtual registers one program may name.
///
/// It was 1024, sized when the only evidence was the GC7000's peak of 350.
/// The PowerVR corpus then refused three sources needing 1064, 1096 and 1124
/// -- a bound found by a survey rather than by a board, which is the cheap
/// way to find one.
///
/// Raising it costs nothing now that the working arrays come from the arena
/// and are sized by the program: at 1024 they were fixed stack arrays
/// totalling 33,792 bytes whatever the shader. The bound survives only to
/// keep a runaway selection from asking the arena for something absurd.
///
/// It is a bound on the highest register NUMBER, not on how many are live:
/// selection numbers a value's lanes `instr * 4 + lane`, so a long
/// scalar-heavy shader names a sparse space. sceneeffects' bloom
/// (96bc0c44f5554256.frag, 696 lines, no loop) needs 26,622; 16,384 refused
/// it. At 65,536 the working arrays are at most about 1.2 MB of scratch, and
/// a program that large runs out of scratch (a refusal) before this.
pub const MAX_VREGS: usize = 65536;

/// The allocated program and what it cost, for the caller to put in the
/// shader's state.
///
/// Borrows the arena handed to [`allocate`]: the resolved groups and the
/// register map are written there rather than into caller slices, so there
/// is nothing to size in advance and nothing to get one entry short.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Allocated<'a> {
    /// The program with every virtual operand replaced by a temporary.
    pub groups: &'a [SelGroup],
    /// One above the highest temporary used: the number the USC state has to
    /// declare. **The field most likely to be silently wrong**, which is why
    /// it comes from here with a test rather than being derived by the caller
    /// from the highest register it happens to see.
    pub temps: u16,
    /// One above the highest register used in each of the other files, for
    /// the caller to declare or check. Zero when the file is untouched.
    pub shared: u16,
    pub vtx_in: u16,
    pub coeff: u16,
    /// Virtual register number -> the temporary it was given, or
    /// [`UNASSIGNED`] for a number no group and no boundary mentions.
    ///
    /// This is how the builder emits the io writes: `Selected::outputs` names
    /// a `SelSrc::Virtual(v)`, and nothing else can say which register that
    /// became.
    pub assignment: &'a [u16],
}

/// A virtual register number no group in `groups` uses.
///
/// Whoever inserts groups after selection -- the builder's legalisation pass,
/// a copy the back end needs -- takes its numbers from here rather than from
/// the convention "anything above the highest". Two passes each choosing
/// "above the highest" choose the *same* number, and the result is two values
/// sharing a register: a wrong picture, found late. A counter costs nothing.
pub fn fresh_above(groups: &[SelGroup]) -> FreshVRegs {
    let mut next = 0u32;
    let mut note = |v: VReg| next = next.max(v.0 + 1);
    for g in groups {
        if let Some(SelDst::Virtual(v)) = g.dst {
            note(v);
        }
        for s in g.srcs.iter().flatten() {
            if let SelSrc::Virtual { v, .. } = s {
                note(*v);
            }
        }
    }
    FreshVRegs { next }
}

/// Hands out virtual registers nothing else is using. See [`fresh_above`].
#[derive(Clone, Copy, Debug)]
pub struct FreshVRegs {
    next: u32,
}

impl FreshVRegs {
    pub fn take(&mut self) -> VReg {
        let v = VReg(self.next);
        self.next += 1;
        v
    }

    /// How many have been handed out past the program's own highest.
    pub fn issued_above(&self, groups: &[SelGroup]) -> u32 {
        self.next - fresh_above(groups).next
    }
}

#[derive(Clone, Copy)]
struct Range {
    first: u32,
    last: u32,
    used: bool,
}

/// Place every [`SelSrc::Virtual`] and [`SelDst::Virtual`] on a temporary,
/// writing the resolved groups into `out`.
///
/// Nothing virtual survives: the builder downstream sees physical operands
/// only. Refuses rather than spills -- a spill needs memory operations this
/// back end has not measured, and a refusal is a shader someone can read
/// about.
///
/// `pinned` is the set of registers selection chose and the allocator must
/// not move, as `(bank, reg)` pairs. It is an argument and not inferred from
/// the groups, because the registers that matter are exactly the ones no
/// instruction mentions.
pub fn allocate<'a>(
    groups: &[SelGroup],
    limits: Limits,
    pinned: impl IntoIterator<Item = (Bank, u16)>,
    boundary: Boundary<'_>,
    a: &Arena<'a>,
) -> Result<Allocated<'a>, AllocError> {
    // What the program needs, counted over every operand, so the answer does
    // not depend on where a walk happened to stop.
    //
    // The boundary is counted too: a live-in VReg an iterator writes and no
    // group ever reads is still a register that has to exist.
    let mut need = fresh_above(groups).next as usize;
    for v in boundary.live_in.iter().chain(boundary.live_out) {
        need = need.max(v.0 as usize + 1);
    }
    if need > MAX_VREGS {
        return Err(AllocError::TooManyValues { needed: need });
    }

    /* Every working array is sized by what this program needs and comes from
     * the caller's arena. They were fixed `[_; MAX_VREGS]` on the stack,
     * which measured 33,792 bytes for a 20-group shader -- a figure this
     * compiler cannot spend on Zephyr, and one that grew with the bound
     * rather than with the program.
     *
     * Note the two dimensions. `ranges`, `order` and `phys` are per VALUE.
     * `reserved` and `busy` are per TEMPORARY REGISTER, and were also
     * `MAX_VREGS` long -- four times larger than the file they describe. */
    let temps = limits.temps as usize;
    let mem = |e: ArenaError| AllocError::Scratch(e);
    let ranges = a.alloc_filled(need, Range { first: u32::MAX, last: 0, used: false }).map_err(mem)?;
    let order = a.alloc_filled(need, 0u32).map_err(mem)?;
    let phys = a.alloc_filled(need, UNASSIGNED).map_err(mem)?;
    let reserved = a.alloc_filled(temps, false).map_err(mem)?;
    // Per temporary, the group index it is busy to; -1 for free.
    let busy = a.alloc_filled(temps, -1i64).map_err(mem)?;

    let mut high = Counts::default();

    // The pinned registers, first: they are reservations inside the very
    // files the allocator allocates from.
    for (bank, reg) in pinned {
        note_physical(reserved, &mut high, limits, bank, reg)?;
    }

    // Live ranges over group positions.
    for (i, g) in groups.iter().enumerate() {
        if let Some(SelDst::Virtual(v)) = g.dst {
            mark(ranges, v, i);
        }
        if let Some(SelDst::Fixed { bank, reg }) = g.dst {
            note_physical(reserved, &mut high, limits, bank, reg)?;
        }
        for s in g.srcs.iter().flatten() {
            match s {
                SelSrc::Virtual { v, .. } => mark(ranges, *v, i),
                SelSrc::Fixed { bank, reg, .. } => note_physical(reserved, &mut high, limits, *bank, *reg)?,
                SelSrc::Imm(_) => {}
            }
        }
    }

    /* The two edges, after the groups so that a value mentioned by both ends
     * up with the union of the ranges rather than whichever was written last.
     *
     * A live-in VReg's range starts at 0 however late its first reader is:
     * the iterator wrote it before group 0, so every register live over the
     * groups in between is live over it too. A live-out VReg's range runs to
     * `groups.len()`, one past the last group, which is where the builder's
     * output write reads it. */
    for v in boundary.live_in {
        mark(ranges, *v, 0);
    }
    for v in boundary.live_out {
        mark(ranges, *v, groups.len());
    }

    /* A loop's body runs again from its top (`SelOp::LoopBegin` ..
     * `SelOp::LoopEnd`, the back edge): a value live INTO the loop is read
     * by every iteration, so it is live to the back edge wherever its last
     * reader sits. A register placed after that reader and before the back
     * edge would overwrite it for the next iteration. A value defined in the
     * body needs nothing: each iteration defines it again before reading it. */
    let mut open: [usize; 4] = [0; 4];
    let mut depth = 0usize;
    for (i, g) in groups.iter().enumerate() {
        match g.op {
            super::sel::SelOp::LoopBegin => {
                *open.get_mut(depth).ok_or(AllocError::Unsupported("loops nested too deeply"))? = i;
                depth += 1;
            }
            super::sel::SelOp::LoopEnd => {
                depth = depth.checked_sub(1).ok_or(AllocError::Unsupported("a back edge with no loop"))?;
                let top = open[depth] as u32;
                for r in ranges.iter_mut().filter(|r| r.used && r.first < top && r.last >= top) {
                    r.last = r.last.max(i as u32);
                }
            }
            _ => {}
        }
    }

    /* In order of increasing start point. Live ranges are intervals, and
     * greedy colouring of an interval graph uses exactly the maximum overlap
     * -- the minimum -- only in start order. Any other order lets a
     * long-lived value assigned late find the low registers marked busy past
     * its own start and take a fresh one for good. */
    let mut n_used = 0usize;
    for (v, r) in ranges.iter().enumerate() {
        if r.used {
            order[n_used] = v as u32;
            n_used += 1;
        }
    }
    order[..n_used].sort_unstable_by_key(|&v| ranges[v as usize].first);

    for &vi in order[..n_used].iter() {
        let v = vi as usize;
        let r = ranges[v];
        let mut placed = None;
        for reg in 0..temps {
            if reserved[reg] || busy[reg] >= r.first as i64 {
                continue;
            }
            placed = Some(reg);
            break;
        }
        let Some(reg) = placed else {
            /* The requirement, not the first miss: how many temporaries are
             * live at the busiest point, plus the pinned ones. */
            return Err(AllocError::OutOfRegisters {
                file: RegFile::Temp,
                needed: peak_demand(ranges, groups.len(), reserved),
            });
        };
        busy[reg] = r.last as i64;
        phys[v] = reg as u16;
        high.temps = high.temps.max(reg as u16 + 1);
    }

    let out = match groups.first() {
        Some(first) => {
            let buf = a.alloc_filled(groups.len(), *first).map_err(mem)?;
            for (slot, g) in buf.iter_mut().zip(groups) {
                *slot = resolve(g, phys);
            }
            &*buf
        }
        None => &[][..],
    };
    /* The map, so the builder can find the register an output write reads.
     * Without it the colouring above is right and unusable: `Selected::outputs`
     * names a `VReg` and nothing else could translate it. */
    Ok(Allocated {
        groups: out,
        temps: high.temps,
        shared: high.shared,
        vtx_in: high.vtx_in,
        coeff: high.coeff,
        assignment: phys,
    })
}

/// The high-water mark per file while allocating. Not [`Allocated`], which
/// carries the program as well and has a lifetime.
#[derive(Clone, Copy, Default)]
struct Counts {
    temps: u16,
    shared: u16,
    vtx_in: u16,
    coeff: u16,
}

fn mark(ranges: &mut [Range], v: VReg, i: usize) {
    if let Some(r) = ranges.get_mut(v.0 as usize) {
        r.used = true;
        r.first = r.first.min(i as u32);
        r.last = r.last.max(i as u32);
    }
}

/// Record a register selection pinned, and refuse if it is outside its file.
fn note_physical(
    reserved: &mut [bool],
    high: &mut Counts,
    limits: Limits,
    bank: Bank,
    reg: u16,
) -> Result<(), AllocError> {
    let Some(file) = RegFile::of(bank) else { return Ok(()) };
    let (size, seen) = match file {
        RegFile::Temp => (limits.temps, &mut high.temps),
        RegFile::Shared => (limits.shared, &mut high.shared),
        RegFile::VtxIn => (limits.vtx_in, &mut high.vtx_in),
        RegFile::Coeff => (limits.coeff, &mut high.coeff),
    };
    if reg >= size {
        return Err(AllocError::OutOfRegisters { file, needed: reg as usize + 1 });
    }
    *seen = (*seen).max(reg + 1);
    if file == RegFile::Temp {
        if let Some(slot) = reserved.get_mut(reg as usize) {
            *slot = true;
        }
    }
    Ok(())
}

/// The most temporaries live at any one group, plus the pinned ones: the
/// number no assignment can go below, and therefore the honest requirement to
/// report when the file is too small.
///
/// Positions run to `n` inclusive, not `n - 1`: position `n` is where the
/// builder's output writes read, and on a program whose last groups each
/// define one component of the position that is exactly where the peak is.
fn peak_demand(ranges: &[Range], n: usize, reserved: &[bool]) -> usize {
    let pinned = reserved.iter().filter(|x| **x).count();
    let mut peak = 0usize;
    for i in 0..=n {
        let live = ranges.iter().filter(|r| r.used && r.first <= i as u32 && i as u32 <= r.last).count();
        peak = peak.max(live);
    }
    peak + pinned
}

fn resolve(g: &SelGroup, phys: &[u16]) -> SelGroup {
    let mut out = *g;
    if let Some(SelDst::Virtual(v)) = g.dst {
        out.dst = Some(SelDst::Fixed { bank: Bank::Temp, reg: phys[v.0 as usize] });
    }
    for (slot, s) in out.srcs.iter_mut().zip(g.srcs.iter()) {
        if let Some(SelSrc::Virtual { v, mods }) = s {
            *slot = Some(SelSrc::Fixed { bank: Bank::Temp, reg: phys[v.0 as usize], mods: *mods });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::sel::SelOp;
    use super::*;
    use crate::shaderc::online::ir::AluOp;
    use crate::shaderc::online::powervr::sel::{Mods, OpType};

    const LIMITS: Limits = Limits { temps: 8, shared: 16, vtx_in: 8, coeff: 8 };

    fn v(n: u32) -> SelSrc {
        SelSrc::Virtual { v: VReg(n), mods: Mods::default() }
    }
    fn imm() -> SelSrc {
        SelSrc::Imm(0x3f80_0000)
    }
    /// A group defining `d` from two immediates.
    fn def(d: u32) -> SelGroup {
        SelGroup {
            op: SelOp::Alu(AluOp::Fadd),
            ty: OpType::F32,
            dst: Some(SelDst::Virtual(VReg(d))),
            srcs: [Some(imm()), Some(imm()), None],
        }
    }
    /// A group reading `a` and `b` into `d`.
    fn use2(d: u32, a: u32, b: u32) -> SelGroup {
        SelGroup {
            op: SelOp::Alu(AluOp::Fadd),
            ty: OpType::F32,
            dst: Some(SelDst::Virtual(VReg(d))),
            srcs: [Some(v(a)), Some(v(b)), None],
        }
    }
    fn blank() -> SelGroup {
        SelGroup { op: SelOp::Alu(AluOp::Fadd), ty: OpType::F32, dst: None, srcs: [None; 3] }
    }

    /// Scratch for one test. 64 KiB is `MIN_SCRATCH_BYTES`, the smallest
    /// buffer the product ever lends the compiler, so a test that fits here
    /// fits on the board.
    const TEST_SCRATCH: usize = 64 << 10;

    fn run<'a>(g: &[SelGroup], pinned: &[(Bank, u16)], a: &Arena<'a>) -> Result<Allocated<'a>, AllocError> {
        run_b(g, pinned, Boundary::NONE, a)
    }

    fn run_b<'a>(
        g: &[SelGroup],
        pinned: &[(Bank, u16)],
        boundary: Boundary<'_>,
        a: &Arena<'a>,
    ) -> Result<Allocated<'a>, AllocError> {
        allocate(g, LIMITS, pinned.iter().copied(), boundary, a)
    }

    /// A value live into a loop keeps its register to the back edge: the
    /// next iteration reads it again. Without the loop the same program
    /// reuses the register at once.
    #[test]
    fn a_value_live_into_a_loop_keeps_its_register_to_the_back_edge() {
        let marker = |op| SelGroup { op, ty: OpType::U32, dst: None, srcs: [None; 3] };
        let reg = |a: &Allocated<'_>, i: usize| match a.groups[i].dst {
            Some(SelDst::Fixed { reg, .. }) => reg,
            _ => panic!("no destination"),
        };
        /* v0 before the loop, read first thing in it; v2 defined after that
         * read, still inside. */
        let looped =
            [def(0), marker(SelOp::LoopBegin), use2(1, 0, 0), def(2), use2(3, 2, 1), marker(SelOp::LoopEnd), blank()];
        let straight = [def(0), use2(1, 0, 0), def(2), use2(3, 2, 1)];
        let mut buf = [0u8; TEST_SCRATCH];
        let arena = Arena::new(&mut buf);
        let a = run(&looped, &[], &arena).expect("allocates");
        assert_ne!(reg(&a, 3), reg(&a, 0), "v2 took v0's register inside the loop that reads v0 again");
        let b = run(&straight, &[], &arena).expect("allocates");
        assert_eq!(reg(&b, 2), reg(&b, 0), "the control: without the loop v0's register is free by then");
    }

    /// Nothing virtual reaches the builder: that is the contract the two
    /// halves were split on.
    #[test]
    fn every_virtual_operand_becomes_a_temporary() {
        let sel = [def(0), def(1), use2(2, 0, 1)];
        let mut buf = [0u8; TEST_SCRATCH];
        let arena = Arena::new(&mut buf);
        let a = run(&sel, &[], &arena).expect("allocates");
        assert_eq!(a.groups.len(), 3);
        for g in a.groups {
            assert!(!matches!(g.dst, Some(SelDst::Virtual(_))), "a virtual destination survived");
            for s in g.srcs.iter().flatten() {
                assert!(!matches!(s, SelSrc::Virtual { .. }), "a virtual source survived");
            }
        }
        assert!(a.temps >= 1, "temps was not reported");
    }

    /// Greedy colouring reaches the minimum only in start order. The GC7000
    /// allocator assigned in virtual-register order and lost a register on
    /// every program where numbering and start order disagreed.
    #[test]
    fn a_value_numbered_late_but_starting_early_costs_nothing_extra() {
        // v1 spans the program, v0 lives only at the end.
        let sel = [def(1), use2(2, 1, 1), def(0), use2(3, 0, 0)];
        let mut buf = [0u8; TEST_SCRATCH];
        let arena = Arena::new(&mut buf);
        let a = run(&sel, &[], &arena).expect("allocates");
        /* EXACTLY two. Virtual-register order needs three on this program and
         * start order needs two, so a bound of "at most three" passes under
         * both and measures nothing -- which is what my first version of this
         * assertion did, and reverting the sort did not fail it. A threshold
         * has to be tight enough to exclude the defect, and the only way to
         * know it is to revert the fix and watch. */
        assert_eq!(a.temps, 2, "start order should need two temporaries here; virtual-register order needs three");
    }

    /// Pinned registers are reservations inside the file being allocated
    /// from, and they are given rather than inferred: on the GC7000 the
    /// registers that mattered were exactly the ones no instruction named.
    #[test]
    fn allocation_goes_around_a_pinned_temporary() {
        let sel = [def(0)];
        let mut buf = [0u8; TEST_SCRATCH];
        let arena = Arena::new(&mut buf);
        let a = run(&sel, &[(Bank::Temp, 0)], &arena).expect("allocates");
        let Some(SelDst::Fixed { reg, .. }) = a.groups[0].dst else { panic!("no destination") };
        assert_ne!(reg, 0, "a temporary was placed on the pinned register");

        // The control: with nothing pinned it takes r0, so the test above is
        // measuring the reservation and not an allocator that avoids r0.
        let a2 = run(&sel, &[], &arena).expect("allocates");
        let Some(SelDst::Fixed { reg: reg2, .. }) = a2.groups[0].dst else { panic!("no destination") };
        assert_eq!(reg2, 0, "with nothing pinned the first temporary should be r0");
    }

    /// The refusal names WHICH file and what the program REQUIRED -- not the
    /// first index that missed. Sizing anything from "where the walk stopped"
    /// invites a second raise, because that figure was never the requirement.
    #[test]
    fn running_out_names_the_file_and_the_requirement() {
        // Nine values all live at once against a file of eight.
        let mut sel = [blank(); 16];
        for (i, s) in sel.iter_mut().enumerate().take(9) {
            *s = def(i as u32);
        }
        sel[9] = use2(9, 0, 1);
        for (i, s) in sel.iter_mut().enumerate().take(16).skip(10) {
            *s = use2(10 + i as u32, (i - 10) as u32 + 2, 0);
        }
        let mut buf = [0u8; TEST_SCRATCH];
        let arena = Arena::new(&mut buf);
        match allocate(&sel, LIMITS, [], Boundary::NONE, &arena) {
            Err(AllocError::OutOfRegisters { file, needed }) => {
                assert_eq!(file, RegFile::Temp, "the wrong file was named");
                assert!(
                    needed > LIMITS.temps as usize,
                    "the requirement {needed} is not past the file's {}",
                    LIMITS.temps
                );
            }
            other => panic!("nine live values in a file of eight were not refused: {other:?}"),
        }
    }

    /// A pinned register outside its own file is refused, with that file
    /// named -- the shared file is not the temporary file and a count that
    /// conflated them would be the GC7000's wrong-bound defect again.
    #[test]
    fn a_pinned_register_past_its_file_is_refused_by_file() {
        let sel = [def(0)];
        let mut buf = [0u8; TEST_SCRATCH];
        let arena = Arena::new(&mut buf);
        match allocate(&sel, LIMITS, [(Bank::Shared, LIMITS.shared)], Boundary::NONE, &arena) {
            Err(AllocError::OutOfRegisters { file, needed }) => {
                assert_eq!(file, RegFile::Shared);
                assert_eq!(needed, LIMITS.shared as usize + 1);
            }
            other => panic!("a shared register past the file was not refused: {other:?}"),
        }
        // The special file is not allocated from and is not counted.
        allocate(&sel, LIMITS, [(Bank::Special, 9999)], Boundary::NONE, &arena)
            .expect("the special file has no size to exceed");
    }

    /// Fresh virtual registers come from a counter, not from the convention
    /// "anything above the highest". Two passes each choosing "above the
    /// highest" choose the SAME number, and two values then share a register.
    #[test]
    fn two_passes_asking_for_fresh_registers_get_different_ones() {
        let sel = [def(0), def(1), use2(2, 0, 1)];
        let mut fresh = fresh_above(&sel);
        let a = fresh.take();
        let b = fresh.take();
        assert_ne!(a, b, "two fresh registers collided");
        assert!(a.0 > 2 && b.0 > 2, "a fresh register reused one the program already names");

        // What the convention would have done: both passes read the same
        // program and both pick the same number.
        let convention_pass_one = fresh_above(&sel).next;
        let convention_pass_two = fresh_above(&sel).next;
        assert_eq!(convention_pass_one, convention_pass_two, "the trap this API exists to avoid");
        assert_eq!(fresh.issued_above(&sel), 2);
    }

    /// A scratch buffer too small for the working set is REPORTED, not
    /// trimmed. This replaced two caller-slice refusals (`OutputTooSmall`,
    /// `AssignmentTooSmall`) that the arena made impossible to reach: with
    /// the program and the map allocated by size, there is no slice left for
    /// a caller to get one entry short.
    #[test]
    fn a_scratch_buffer_too_small_is_reported_rather_than_trimmed() {
        let sel = [def(0), def(1), use2(2, 0, 1)];
        let mut tiny = [0u8; 8];
        let arena = Arena::new(&mut tiny);
        assert_eq!(
            allocate(&sel, LIMITS, [], Boundary::NONE, &arena),
            Err(AllocError::Scratch(ArenaError::OutOfScratch))
        );
    }

    /// And the working set is sized by the PROGRAM, not by `MAX_VREGS`.
    ///
    /// It used to be five `[_; MAX_VREGS]` arrays on the stack: 33,792 bytes
    /// for a three-group shader, and 540,672 at today's bound. The figure
    /// asserted here is loose on purpose -- it is a ceiling that catches a
    /// return to fixed sizing, not a budget to tune.
    #[test]
    fn the_working_set_is_sized_by_the_program() {
        let sel = [def(0), def(1), use2(2, 0, 1)];
        let mut buf = [0u8; TEST_SCRATCH];
        let arena = Arena::new(&mut buf);
        run(&sel, &[], &arena).expect("allocates");
        let used = arena.used();
        assert!(used < 8 << 10, "a three-group shader took {used} bytes of scratch");
        assert!(used > 0, "nothing was allocated; this test would pass on a stub");
    }

    /* ---- the two edges ---------------------------------------------- */

    /// The defect this file's `Boundary` exists for, in four groups.
    ///
    /// Three values are defined and never read by a later group, because
    /// their only consumer is the vertex output write -- which is not a
    /// group. Without the boundary all three get one register, and the
    /// program computes a position out of one component written three times.
    ///
    /// Reverting the `live_out` marking in `allocate` turns this into
    /// `left: 1, right: 3`.
    #[test]
    fn values_read_only_by_an_output_write_keep_their_own_registers() {
        let sel = [def(0), def(1), def(2), def(3)];
        let live_out = [VReg(0), VReg(1), VReg(2), VReg(3)];
        let b = Boundary { live_in: &[], live_out: &live_out };

        let mut buf = [0u8; TEST_SCRATCH];
        let arena = Arena::new(&mut buf);
        let alloc = run_b(&sel, &[], b, &arena).expect("allocates");
        let map = alloc.assignment;
        let regs = [map[0], map[1], map[2], map[3]];
        let mut shared = 0usize;
        for (i, a) in regs.iter().enumerate() {
            for b2 in regs.iter().skip(i + 1) {
                if a == b2 {
                    shared += 1;
                }
            }
        }
        assert_eq!(shared, 0, "live-out values share a register: {regs:?}");

        // And the control: told nothing, the allocator recycles, which is the
        // right answer to the wrong question. This is what the corpus saw.
        let map2 = run_b(&sel, &[], Boundary::NONE, &arena).expect("allocates").assignment;
        assert_eq!(
            [map2[0], map2[1], map2[2], map2[3]],
            [0, 0, 0, 0],
            "without the boundary the four single-point ranges should all take temp 0"
        );
    }

    /// The mirror: a VReg an iterator wrote before group 0 is live from 0,
    /// however late the group that reads it is.
    ///
    /// **Not exercised by any corpus source**, and that is a fact about
    /// selection rather than about this: every fragment varying is still
    /// refused there ("the coefficient file may need an iterator the builder
    /// emits"), so `Selected::inputs` is empty everywhere today. Hand-built
    /// on purpose, and labelled so nobody reads it as measured.
    #[test]
    fn a_value_written_before_the_first_group_is_live_from_the_first_group() {
        /* VReg(9) is written by an iterator, read only at the last group.
         * VReg(0) and VReg(1) live across the middle. Without the live-in
         * mark, VReg(9)'s range is the single last position and it lands on
         * the register VReg(0) vacated -- the one the iterator's write
         * already clobbered. */
        let sel = [def(0), def(1), use2(2, 0, 1), use2(3, 9, 2)];
        let live_in = [VReg(9)];
        let b = Boundary { live_in: &live_in, live_out: &[] };

        let mut buf = [0u8; TEST_SCRATCH];
        let arena = Arena::new(&mut buf);
        let map = run_b(&sel, &[], b, &arena).expect("allocates").assignment;
        assert_ne!(map[9], map[0], "the iterator's register was reused for VReg(0)");
        assert_ne!(map[9], map[1], "the iterator's register was reused for VReg(1)");

        // The control, again: without it the collision is what happens.
        let map2 = run_b(&sel, &[], Boundary::NONE, &arena).expect("allocates").assignment;
        assert!(
            map2[9] == map2[0] || map2[9] == map2[1],
            "the point of the mark is that VReg(9) otherwise shares; it did not: {map2:?}"
        );
    }

    /// The map names every value the program has, and is exactly that long.
    ///
    /// It is no longer possible for a caller to hand in a map one entry
    /// short: the arena sizes it. What IS still possible is an off-by-one in
    /// `need`, which this catches.
    #[test]
    fn the_map_covers_the_values_and_nothing_more() {
        let sel = [def(0), def(1), use2(2, 0, 1)];
        let mut buf = [0u8; TEST_SCRATCH];
        let arena = Arena::new(&mut buf);
        let map = run_b(&sel, &[], Boundary::NONE, &arena).expect("allocates").assignment;
        assert_eq!(map.len(), 3, "the map should have one entry per value this program names");
        for (v, m) in map.iter().enumerate() {
            assert_ne!(*m, UNASSIGNED, "VReg({v}) has no register");
        }
    }
}

#[cfg(test)]
mod declared_temps_tests {
    use super::*;

    /// The hardware is told blocks of four, so the number this allocator
    /// reports and the number the GPU receives are not the same number.
    #[test]
    fn a_count_is_declared_rounded_up_to_a_block_of_four() {
        assert_eq!(declared_temps(0), 0);
        assert_eq!(declared_temps(1), 4);
        assert_eq!(declared_temps(5), 8, "five temporaries are declared as eight");
        assert_eq!(declared_temps(8), 8, "eight is already a whole block");
        assert_eq!(declared_temps(13), 16);
        // The largest the six-bit block field can express: 0x3f blocks of 4.
        assert_eq!(declared_temps(252), 252);
        assert_eq!(0x3f * 4, 252);
    }

    /// And therefore the corpus numbers, read honestly.
    ///
    /// The figures moved when `Boundary` landed, because the old ones came
    /// from a picture with the output writes missing: `texturefill.batchable.vert`
    /// reads 7 where it read 5, and the corpus peak is 9
    /// (`distancefieldshiftedtext.vert`). Both are worth stating because the
    /// DECLARED count did not move for the first: 5 and 7 are both declared
    /// as 8. A register saved inside a block buys nothing, and a register
    /// *found* inside a block costs nothing either.
    #[test]
    fn the_corpus_counts_cost_what_they_are_declared_not_what_they_use() {
        assert_eq!(declared_temps(5), 8);
        assert_eq!(declared_temps(7), 8, "texturefill.batchable.vert, with the output writes counted");
        assert_eq!(declared_temps(9), 12, "distancefieldshiftedtext.vert, the corpus peak");
        assert!(declared_temps(9) < 252, "the temp file is nowhere near binding on this corpus");
    }
}
