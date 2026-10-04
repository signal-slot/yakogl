// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Loops, before selection: expanded into their iterations, or kept as ONE
//! loop with a counted back edge.
//!
//! Every loop GLSL ES 1.00 admits in a shader of the table has a trip count
//! known when the shader is compiled: Appendix A limits a `for` to a loop
//! index initialised by a constant, compared against a constant and stepped
//! by one. A loop that leaves only by that test is expanded here into its
//! iterations, IR to IR, and selection never meets it.
//!
//! A loop that can leave on data (a second `break`, any `continue`) is KEPT,
//! if it sits in straight-line code: its body once, every exit a flag (as
//! below), and its trip count recorded ([`Trip`]). Selection gives it a
//! counter and a backward branch driven by that counter alone, the same in
//! every instance (`select::verify_loops`). Expanded, such a loop carried
//! its whole bound as copies: femtovg's filter_image was 4,094 dwords, 8
//! bytes under the image limit; kept, it is 636.
//!
//! The pass walks the IR in execution order and evaluates what it can: a
//! constant, a local holding one, an operation on those. The front end's
//! loop is `loop { c = <compare>; if (c) {} else { break; } <body>;
//! <step> }`, so at the head of each iteration the compare is a constant
//! and the `if` resolves: the arm it takes is inlined, and a `break` there
//! ends the expansion. An `if` whose condition is not known is copied with
//! both arms (selection converts it).
//!
//! A `break` or `continue` inside such an `if` depends on data. It becomes
//! a store of `true` to a flag local of its loop (a `continue`'s flag is
//! cleared at each iteration's start), and from then on every store,
//! output and discard in the loop is guarded by the flags being clear: an
//! `if` selection converts like any other. The loop still expands to its
//! constant bound; iterations after a break run and change nothing. The
//! expansion schedule (the loop index) is evaluated as if no flag were set,
//! which is sound because nothing a flagged iteration does is kept; a local
//! a guarded store wrote is not known after the loop.
//!
//! Every iteration's instructions are copies with fresh value ids, so each
//! value is still defined once; the locals carry what crosses iterations,
//! as they carry it across an `if`. The new module shares everything else
//! with the old one.
//!
//! What this computes is checked by the chain gate's IR interpreter, which
//! runs the ORIGINAL module with its loops as loops.

use crate::shaderc::online::arena::{AVec, Arena, ArenaError};
use crate::shaderc::online::ir::{self, AluOp, Node, Op, Src, ValueId};

/// Why a loop was not expanded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnrollError {
    /// A loop whose trip count depends on data (its head condition is not
    /// known when the expansion reaches it, so it would run to the bound).
    DataDependentExit,
    /// More iterations, or more copied instructions, than the bounds below.
    TooLong,
    /// A `return` inside a loop.
    ReturnInLoop,
    OutOfScratch,
}

impl From<ArenaError> for UnrollError {
    fn from(_: ArenaError) -> Self {
        UnrollError::OutOfScratch
    }
}

impl UnrollError {
    /// The refusal selection reports, one string per cause.
    pub fn message(self) -> &'static str {
        match self {
            UnrollError::DataDependentExit => "a loop whose trip count depends on data: it needs a real branch",
            UnrollError::TooLong => "a loop too long to unroll: it needs a real branch",
            UnrollError::ReturnInLoop => "a `return` inside a loop: it needs a real branch",
            UnrollError::OutOfScratch => "the scratch arena ran out unrolling a loop",
        }
    }
}

/// Iterations of one loop, and copied instructions in all, past which the
/// expansion stops: a loop that long is a program a branch should run.
const MAX_ITERATIONS: u32 = 256;

/// The most times a loop whose trip count is read at run time (a uniform
/// bound, ES 3.00: `for (int i = 0; float(i) < counts.y; i++)`, Qt's light
/// loops) runs. A DELIBERATE DEVIATION from GLSL, which has no bound: a
/// uniform past it stops the loop after this many iterations (the result is
/// then undefined), where GLSL would run on -- a +inf bound would never end,
/// and a program that does not end stops the GPU. It is the structural
/// termination guarantee (CLAUDE.md) for loops the compiler cannot count:
/// the back edge is taken only while the counter is below it. Qt caps its
/// lights at 15. The IR evaluator the tests compare against applies the
/// same cap, so host comparisons stay exact.
pub const UNIFORM_LOOP_CAP: u32 = 4096;
const MAX_INSTRS: usize = 16_384;

/// What one lane is known to hold, if anything.
type Known = [Option<u32>; 4];

/// How a block ended.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flow {
    Normal,
    Break,
    Continue,
    Return,
    /// A `break` or `continue` on data set its flag: the rest of this
    /// block is dead, the blocks around it go on under the guard.
    Flagged,
}

/// One loop being expanded.
#[derive(Clone, Copy)]
struct Frame {
    /// The flag a data-dependent `break` sets, once one has.
    brk: Option<ir::LocalId>,
    /// The flag a data-dependent `continue` sets, cleared per iteration.
    cnt: Option<ir::LocalId>,
    /// How many arms of `if`s on data the walk is inside, in this loop.
    on_data: u32,
    /// Index into `Unroll::guarded` where this loop's entries start.
    guarded_from: usize,
}

struct Unroll<'a> {
    m: &'a ir::Module<'a>,
    a: &'a Arena<'a>,
    instrs: AVec<'a, Op>,
    comps: AVec<'a, u8>,
    nodes: AVec<'a, Node>,
    /// By NEW value id: what each lane is known to hold.
    known: AVec<'a, Known>,
    /// By OLD value id: the new id of its latest copy.
    remap: &'a mut [Option<ValueId>],
    /// By local and lane: what it is known to hold.
    env: AVec<'a, Known>,
    /// The module's locals, and the flags the expansion adds after them.
    locals: AVec<'a, ir::VarDecl<'a>>,
    /// The loops being expanded, innermost last.
    frames: AVec<'a, Frame>,
    /// Locals a guarded store wrote, per loop (see `Frame::guarded_from`):
    /// not known once that loop ends.
    guarded: AVec<'a, ir::LocalId>,
    /// Bumped at every flag set; `covered` is the value the innermost
    /// guard in force was taken at. A block runs under the flags as they
    /// stand at `covered`, and wraps its remainder in a new guard once a
    /// flag has been set since.
    epoch: u32,
    covered: u32,
    depth: u32,
    /// How many loops that stay loops the walk is inside.
    rolled: u32,
    /// How many `if` arms the output being written is inside. A loop kept
    /// as a loop must sit in straight-line code: selection if-converts an
    /// arm, and a branch inside one is the divergence `cnd*` exists for.
    arms: u32,
    /// Every loop kept as a loop: its body's first node, and how many times
    /// the body runs.
    trips: AVec<'a, Trip>,
    /// Set by [`Unroll::sim`] when a pass meets a `break` under a condition
    /// it does not know: the loop can leave before its counted passes.
    data_break: core::cell::Cell<bool>,
}

/// A loop kept as a loop (see [`unroll`]): the body that starts at node
/// `first` runs `count` times, and every exit from it is a flag.
///
/// `exit`: for a loop whose trip count is read at run time, or whose count
/// is exact but which can leave earlier on a `break` the compiler cannot
/// decide, the break flag, every write of which is uniform (the same in
/// every instance): the back edge is then taken while the counter is below
/// `count` ([`UNIFORM_LOOP_CAP`], or the exact count) AND the flag is
/// clear, so the loop leaves as soon as its bound says, together in every
/// instance. `None`: the counter alone decides.
///
/// `divergent`: that flag may be set in some instances and not others (the
/// bound reads a varying, a texture, the fragment's position...). The loop
/// then runs under pco's execution mask (`powervr::cf`): an instance whose
/// flag is set leaves at the end of that pass and runs nothing more, and the
/// back edge is taken while any instance is still in -- up to `count`
/// passes, the same cap as a uniform bound.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Trip {
    pub first: u32,
    pub count: u32,
    pub exit: Option<ir::LocalId>,
    pub divergent: bool,
}

#[cfg(test)]
thread_local! {
    /// A test's switch: a counted loop's uniform `break` does not end it
    /// early (the rule before 2026-09-26), for the A/B over the corpus.
    static EARLY_EXIT_OFF: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// Whether a loop with an exact count also leaves on its uniform break
/// flag: always, except in a test that switched it off.
fn early_exit_on() -> bool {
    #[cfg(test)]
    {
        !EARLY_EXIT_OFF.with(|h| h.get())
    }
    #[cfg(not(test))]
    {
        true
    }
}

/// What [`unroll`] made: the module, and the loops it kept.
pub struct Unrolled<'a> {
    pub module: &'a ir::Module<'a>,
    pub trips: &'a [Trip],
}

/// Expand every loop in `m`. `None` when it has none, so the caller keeps
/// the module it has.
pub fn unroll<'a>(m: &'a ir::Module<'a>, a: &'a Arena<'a>) -> Result<Option<Unrolled<'a>>, UnrollError> {
    if !m.nodes.iter().any(|n| matches!(n, Node::Loop { .. })) {
        return Ok(None);
    }
    let mut u = Unroll {
        m,
        a,
        instrs: AVec::new(),
        comps: AVec::new(),
        nodes: AVec::new(),
        known: AVec::new(),
        remap: a.alloc_filled(m.instrs.len(), None)?,
        env: AVec::new(),
        locals: AVec::new(),
        frames: AVec::new(),
        guarded: AVec::new(),
        epoch: 0,
        covered: 0,
        depth: 0,
        rolled: 0,
        arms: 0,
        trips: AVec::new(),
        data_break: core::cell::Cell::new(false),
    };
    /* The expanded program is at least as long as the one it expands: room
     * for that up front. Three lists grown by doubling in turn cannot grow in
     * place, and each move left its old buffer behind -- 1.37 MB of a 2 MiB
     * arena on a dEQP tmp_array shader of 6,205 instructions around a
     * four-trip loop, which ran out before the loop was reached. */
    u.instrs.reserve(a, m.instrs.len())?;
    u.comps.reserve(a, m.comps.len())?;
    u.known.reserve(a, m.instrs.len())?;
    for _ in m.locals {
        u.env.push(a, [None; 4])?;
    }
    u.locals.extend_from_slice(a, m.locals)?;
    let (body, flow) = u.block(m.body)?;
    if matches!(flow, Flow::Break | Flow::Continue) {
        return Err(UnrollError::DataDependentExit);
    }
    let out = ir::Module {
        instrs: u.instrs.into_slice(),
        comps: u.comps.into_slice(),
        nodes: u.nodes.into_slice(),
        locals: u.locals.into_slice(),
        body,
        ..*m
    };
    let module = a.alloc_copy(core::slice::from_ref(&out))?.first().ok_or(UnrollError::OutOfScratch)?;
    Ok(Some(Unrolled { module, trips: u.trips.into_slice() }))
}

impl<'a> Unroll<'a> {
    /// Copy `r` into a block of its own and return where it landed.
    fn block(&mut self, r: ir::Range) -> Result<(ir::Range, Flow), UnrollError> {
        /* The list is given back once it is copied into `nodes`. */
        let mut here: AVec<'a, Node> = AVec::new();
        let flow = self.into(r, &mut here)?;
        let first = self.nodes.len() as u32;
        let len = here.len() as u32;
        self.nodes.extend_from_slice(self.a, here.as_slice())?;
        here.release(self.a);
        Ok((ir::Range { first, len }, flow))
    }

    /// Copy `r` onto the end of `out`, stopping at a `break`, `continue` or
    /// `return` the walk reaches.
    fn into(&mut self, r: ir::Range, out: &mut AVec<'a, Node>) -> Result<Flow, UnrollError> {
        for (i, n) in self.m.nodes_of(r).iter().enumerate() {
            /* A flag was set since the guard this block runs under: the rest
             * of it goes under `if (flags) {} else { rest }`. */
            if self.epoch > self.covered {
                let mark = self.epoch;
                if let Some(g) = self.guard(out)? {
                    let saved = self.covered;
                    self.covered = mark;
                    let rest = ir::Range { first: r.first + i as u32, len: r.len - i as u32 };
                    self.arms += 1;
                    let (body, flow) = self.block(rest)?;
                    self.arms -= 1;
                    self.covered = saved;
                    out.push(self.a, Node::If { cond: g, then_: ir::Range::EMPTY, else_: body })?;
                    return Ok(flow);
                }
                self.covered = mark;
            }
            match *n {
                Node::Instr(id) => {
                    let new = self.instr(id)?;
                    if self.covered > 0 {
                        if let Some(Op::StoreLocal { local, .. }) = self.instrs.as_slice().get(new.0 as usize) {
                            self.guarded.push(self.a, *local)?;
                        }
                    }
                    out.push(self.a, Node::Instr(new))?;
                }
                Node::If { cond, then_, else_ } => {
                    let c = self.src(cond);
                    match self.lane(c, 0) {
                        /* Known: the arm it takes, inline. */
                        Some(x) => {
                            let flow = self.into(if x != 0 { then_ } else { else_ }, out)?;
                            if flow != Flow::Normal {
                                return Ok(flow);
                            }
                        }
                        /* Not known, and an arm leaves the loop: the arms
                         * set flags instead (see the module comment). */
                        None if self.depth > 0 && (self.leaves(then_) || self.leaves(else_)) => {
                            self.on_data(1);
                            let before = self.env_copy()?;
                            self.arms += 1;
                            let (t, tf) = self.block(then_)?;
                            let after_then = self.env_copy()?;
                            self.restore(before.as_slice());
                            let (e, ef) = self.block(else_)?;
                            self.arms -= 1;
                            /* The schedule follows the path that goes on: an
                             * arm that ended by setting a flag contributes
                             * nothing (a `continue` arm steps the index
                             * itself, and meeting it with the other arm would
                             * lose the index and the bound with it). */
                            match (tf == Flow::Flagged, ef == Flow::Flagged) {
                                (true, false) => {}
                                (false, true) => self.restore(after_then.as_slice()),
                                _ => self.meet(after_then.as_slice()),
                            }
                            after_then.release(self.a);
                            before.release(self.a);
                            self.on_data(-1);
                            if matches!(tf, Flow::Return) || matches!(ef, Flow::Return) {
                                return Err(UnrollError::ReturnInLoop);
                            }
                            out.push(self.a, Node::If { cond: c, then_: t, else_: e })?;
                        }
                        /* Not known: both arms, for selection to convert.
                         * Neither may leave the loop, and what each stores
                         * is known afterwards only where they agree. */
                        None => {
                            let before = self.env_copy()?;
                            self.arms += 1;
                            let (t, tf) = self.block(then_)?;
                            let after_then = self.env_copy()?;
                            self.restore(before.as_slice());
                            let (e, ef) = self.block(else_)?;
                            self.arms -= 1;
                            self.meet(after_then.as_slice());
                            after_then.release(self.a);
                            before.release(self.a);
                            if tf == Flow::Return || ef == Flow::Return {
                                /* A `return` in an arm of `main` outside any
                                 * loop is selection's to handle. */
                                if self.depth > 0 {
                                    return Err(UnrollError::ReturnInLoop);
                                }
                            }
                            out.push(self.a, Node::If { cond: c, then_: t, else_: e })?;
                        }
                    }
                }
                /* A loop counted at run time is kept as a loop wherever it
                 * sits -- inside an `if` arm too (selection runs both arms
                 * and joins with selects, so the loop runs and its results
                 * are kept or dropped; it stores no output and ends at the
                 * cap). One that leaves on data under a constant count is
                 * kept only outside the arms, as it always was. */
                Node::Loop { body }
                    if (self.arms == 0 && self.rolls(body))
                        || self.counted_at_run_time(body)
                        || self.too_long_to_expand(body) =>
                {
                    self.roll(body, out)?
                }
                Node::Loop { body } => {
                    self.depth += 1;
                    let frame = Frame { brk: None, cnt: None, on_data: 0, guarded_from: self.guarded.len() };
                    self.frames.push(self.a, frame)?;
                    /* Where the loop's flags are cleared, once it is known
                     * which it made (see the `Break | Continue` arm). */
                    let entry = out.len();
                    let mut n = 0u32;
                    loop {
                        n += 1;
                        if n > MAX_ITERATIONS {
                            return Err(UnrollError::TooLong);
                        }
                        /* A `continue` on data skips the rest of one
                         * iteration only. */
                        if let Some(f) = self.frame().and_then(|f| f.cnt) {
                            self.set(f, false, out)?;
                        }
                        match self.into(body, out)? {
                            Flow::Break => break,
                            Flow::Return => return Err(UnrollError::ReturnInLoop),
                            Flow::Normal | Flow::Continue | Flow::Flagged => {}
                        }
                    }
                    /* What a guarded store wrote depends on data. */
                    let from = self.frame().map_or(0, |f| f.guarded_from);
                    for i in from..self.guarded.len() {
                        let l = self.guarded.as_slice()[i];
                        if let Some(e) = self.env.as_mut_slice().get_mut(l.0 as usize) {
                            *e = [None; 4];
                        }
                    }
                    while self.guarded.len() > from {
                        self.guarded.pop();
                    }
                    /* The flags the iterations made, cleared before the
                     * first one: this point is on every path to every read
                     * of them. */
                    if let Some(f) = self.frame() {
                        for flag in [f.brk, f.cnt].into_iter().flatten() {
                            let at = out.len();
                            self.set(flag, false, out)?;
                            let n = out.len() - at;
                            out.as_mut_slice()[entry..].rotate_right(n);
                        }
                    }
                    self.frames.pop();
                    self.depth -= 1;
                }
                Node::Break | Node::Continue => {
                    let brk = matches!(n, Node::Break);
                    let Some(f) = self.frame() else { return Err(UnrollError::DataDependentExit) };
                    if f.on_data == 0 {
                        return Ok(if brk { Flow::Break } else { Flow::Continue });
                    }
                    /* On data: set this loop's flag, made on first use. The
                     * first use can be inside an `if` arm, whose nodes do
                     * not run on every path to the flag's later reads (the
                     * guards after the arm, the next iterations), so the
                     * flag is not cleared HERE: the loop's expansion clears
                     * it at its entry, before the first iteration, once it
                     * is done (the `Node::Loop` arm of `block`). Cleared in
                     * the arm, dEQP's return_in_nested_loop read it unset
                     * on every path that skipped the arm. */
                    let flag = match if brk { f.brk } else { f.cnt } {
                        Some(x) => x,
                        None => {
                            let x = self.new_flag(false, out)?;
                            if let Some(top) = self.frames.as_mut_slice().last_mut() {
                                if brk {
                                    top.brk = Some(x);
                                } else {
                                    top.cnt = Some(x);
                                }
                            }
                            x
                        }
                    };
                    self.set(flag, true, out)?;
                    self.epoch += 1;
                    return Ok(Flow::Flagged);
                }
                Node::Return => {
                    if self.depth > 0 {
                        return Err(UnrollError::ReturnInLoop);
                    }
                    out.push(self.a, Node::Return)?;
                    return Ok(Flow::Return);
                }
            }
        }
        Ok(Flow::Normal)
    }

    /// Back to `before` for the other arm. A flag the first arm made is
    /// past its end, and not known.
    /// A copy of what is known about the locals, for an `if` whose arms must
    /// each start from it; given back with `release` when the `if` is done
    /// (kept, the copies ran a 2 MiB arena out on dEQP's tmp_array shapes:
    /// an `arr[i++] = x` is an `if` per element, two copies each).
    fn env_copy(&self) -> Result<AVec<'a, Known>, UnrollError> {
        let mut v = AVec::with_capacity(self.a, self.env.len())?;
        v.extend_from_slice(self.a, self.env.as_slice())?;
        Ok(v)
    }

    fn restore(&mut self, before: &[Known]) {
        let env = self.env.as_mut_slice();
        let n = before.len().min(env.len());
        env[..n].copy_from_slice(&before[..n]);
        for e in env[n..].iter_mut() {
            *e = [None; 4];
        }
    }

    /// Does this loop stay a loop? One that can leave other than by its
    /// head test (a second `break`, or any `continue`) does: expanded, it
    /// would carry its whole bound as copies (femtovg's filter_image, 24 of
    /// them, fitted its image with 8 bytes to spare). One with a loop inside
    /// it is expanded as before.
    fn rolls(&self, body: ir::Range) -> bool {
        fn walk(m: &ir::Module<'_>, r: ir::Range, exits: &mut u32, nested: &mut bool) {
            for n in m.nodes_of(r) {
                match n {
                    Node::Break => *exits += 1,
                    Node::Continue => *exits += 2,
                    Node::If { then_, else_, .. } => {
                        walk(m, *then_, exits, nested);
                        walk(m, *else_, exits, nested);
                    }
                    Node::Loop { .. } => *nested = true,
                    _ => {}
                }
            }
        }
        let (mut exits, mut nested) = (0, false);
        walk(self.m, body, &mut exits, &mut nested);
        exits > 1 && !nested
    }

    /// Keep a loop as a loop: its body once, every exit a flag, and how many
    /// times it runs in [`Unroll::trips`]. Selection gives it a counter and a
    /// backward branch on that counter alone.
    ///
    /// The count is the static schedule's (`trip_count`), which is sound for
    /// the reason the expansion's is: the entries after a flag was set run
    /// under it and keep nothing. Every entry runs under the flags from its
    /// first node, so the one that breaks at its head computes the compare
    /// and nothing else.
    fn roll(&mut self, body: ir::Range, out: &mut AVec<'a, Node>) -> Result<(), UnrollError> {
        /* A kept loop may sit inside another: `break` and `continue` in
         * GLSL leave the innermost loop only, so each loop's flags are its
         * own (made at its head, inside the outer body, afresh on every
         * outer pass), and its counter starts from zero each time selection
         * reaches it. */
        /* A loop whose count is read at run time runs up to the cap, and
         * leaves on its (uniform) break flag. */
        /* ... and one whose bound may differ between instances runs under
         * the execution mask, each instance leaving on its own flag. */
        /* A loop whose count is exact (a constant head) but which can also
         * leave early on a uniform `break` (femtovg's filter_image: 24
         * passes, left at the blur's tap count) keeps its count and leaves
         * on the flag too, as Mesa leaves at the break: before, the counter
         * alone decided and every pass after the break ran under the flag,
         * doing nothing (2341 executed instructions against Mesa's 65).
         * A break that may differ between instances keeps the counter
         * alone: every pass runs, the ones after it under the flag. */
        self.data_break.set(false);
        let counted = self.trip_count(body);
        let data_break = self.data_break.get();
        let (count, flagged, divergent) = match counted {
            Ok(c) if data_break && early_exit_on() && self.uniform_exits(body) => (c, true, false),
            Ok(c) => (c, false, false),
            Err(UnrollError::DataDependentExit) if self.uniform_exits(body) => (UNIFORM_LOOP_CAP, true, false),
            Err(UnrollError::DataDependentExit) => (UNIFORM_LOOP_CAP, true, true),
            Err(e) => return Err(e),
        };
        /* What the body stores is not known inside it, nor after it. */
        self.forget_stores(body);
        /* The flags are made before the loop: a `break` holds across
         * entries, and one made inside would be cleared by the next. */
        let brk = self.new_flag(true, out)?;
        let cnt = if self.has_continue(body) { Some(self.new_flag(true, out)?) } else { None };
        if count == 0 {
            return Ok(());
        }
        self.depth += 1;
        self.rolled += 1;
        let frame = Frame { brk: Some(brk), cnt, on_data: 1, guarded_from: self.guarded.len() };
        self.frames.push(self.a, frame)?;
        let mut here: AVec<'a, Node> = AVec::new();
        if let Some(c) = cnt {
            self.set(c, false, &mut here)?;
        }
        let saved = self.covered;
        self.epoch += 1;
        let flow = self.into(body, &mut here)?;
        self.covered = saved;
        let first = self.nodes.len() as u32;
        let len = here.len() as u32;
        self.nodes.extend_from_slice(self.a, here.as_slice())?;
        let from = self.frame().map_or(0, |f| f.guarded_from);
        while self.guarded.len() > from {
            self.guarded.pop();
        }
        self.frames.pop();
        self.rolled -= 1;
        self.depth -= 1;
        self.forget_stores(body);
        if flow == Flow::Return {
            return Err(UnrollError::ReturnInLoop);
        }
        self.trips.push(self.a, Trip { first, count, exit: flagged.then_some(brk), divergent })?;
        out.push(self.a, Node::Loop { body: ir::Range { first, len } })?;
        Ok(())
    }

    /// How many times a loop's body is entered, the entry that leaves by a
    /// known `break` included, following the schedule a flagged exit does
    /// not change (as the expansion does). Evaluated over the ORIGINAL
    /// module's values; nothing is emitted.
    fn trip_count(&self, body: ir::Range) -> Result<u32, UnrollError> {
        /* The whole module's value table, given back before returning (the
         * arena reuses it): it was allocated and kept on every call, twice
         * per loop, which on a long Qt shader with several loops ran the
         * scratch out. */
        let mut vals: AVec<'a, Known> = AVec::with_capacity(self.a, self.m.instrs.len())?;
        for _ in 0..self.m.instrs.len() {
            vals.push(self.a, [None; 4])?;
        }
        let mut env: AVec<'a, Known> = AVec::new();
        env.extend_from_slice(self.a, self.env.as_slice())?;
        let mut count = 0;
        let result = loop {
            if count >= MAX_ITERATIONS {
                /* Past what the simulation follows: a constant `for` loop
                 * still has its own count, read from its index alone
                 * (online/loop_count.rs, the GC7000's rule too) -- cheap,
                 * a few operations a pass, where simulating the body 65,536
                 * times made tests/stack_survey run for over ten minutes. It
                 * used to be cut at UNIFORM_LOOP_CAP here: a constant 5000
                 * loop ran 4096 passes on the PowerVR and the V3D. */
                let known = |i: ir::LocalId, _float: bool| self.env.as_slice().get(i.0 as usize).and_then(|k| k[0]);
                break crate::shaderc::online::loop_count::constant_loop_entries(self.m, body, &known)
                    .ok_or(UnrollError::DataDependentExit);
            }
            count += 1;
            match self.sim(body, vals.as_mut_slice(), env.as_mut_slice()) {
                Ok(Flow::Break) => break Ok(count),
                Ok(_) => {}
                Err(e) => break Err(e),
            }
        };
        env.release(self.a);
        vals.release(self.a);
        result
    }

    /// One pass of `r` for [`Unroll::trip_count`]: what `vals` (by original
    /// value) and `env` (by local) are known to hold afterwards, and how it
    /// ended. A `break` under a condition not known is a flag, and the walk
    /// follows the arm that goes on.
    fn sim(&self, r: ir::Range, vals: &mut [Known], env: &mut [Known]) -> Result<Flow, UnrollError> {
        for n in self.m.nodes_of(r) {
            match *n {
                Node::Instr(id) => {
                    let op = *self.m.instr(id).ok_or(UnrollError::OutOfScratch)?;
                    let lane = |s: Src, k: usize| vals.get(s.val.0 as usize).and_then(|l| l[s.swizzle[k] as usize & 3]);
                    let known = eval_with(&op, lane, |l| env.get(l.0 as usize).copied().unwrap_or([None; 4]));
                    if let Op::StoreLocal { local, index, src, write_mask } = op {
                        let from = vals.get(src.val.0 as usize).copied().unwrap_or([None; 4]);
                        if let Some(slot) = env.get_mut(local.0 as usize) {
                            for k in 0..4 {
                                if index.is_some() {
                                    slot[k] = None;
                                } else if write_mask & (1 << k) != 0 {
                                    slot[k] = from[src.swizzle[k] as usize & 3];
                                }
                            }
                        }
                    }
                    if let Some(v) = vals.get_mut(id.0 as usize) {
                        *v = known;
                    }
                }
                Node::If { cond, then_, else_ } => {
                    match vals.get(cond.val.0 as usize).and_then(|l| l[cond.swizzle[0] as usize & 3]) {
                        Some(x) => {
                            let f = self.sim(if x != 0 { then_ } else { else_ }, vals, env)?;
                            if f != Flow::Normal {
                                return Ok(f);
                            }
                        }
                        None => {
                            /* Both arms' environments, given back at the end
                             * of this `if` (the arena reuses them): kept, they
                             * cost two copies of every local per unknown `if`
                             * per simulated pass -- 4.8 MB on one Qt light
                             * loop before a single iteration was emitted. */
                            let mut before: AVec<'a, Known> = AVec::new();
                            before.extend_from_slice(self.a, env)?;
                            let tf = self.sim(then_, vals, env)?;
                            let mut after_then: AVec<'a, Known> = AVec::new();
                            after_then.extend_from_slice(self.a, env)?;
                            env.copy_from_slice(before.as_slice());
                            before.release(self.a);
                            let ef = self.sim(else_, vals, env)?;
                            if tf == Flow::Break || ef == Flow::Break {
                                self.data_break.set(true);
                            }
                            let leave = match (tf != Flow::Normal, ef != Flow::Normal) {
                                (true, false) => false,
                                (false, true) => {
                                    env.copy_from_slice(after_then.as_slice());
                                    false
                                }
                                (false, false) => {
                                    for (x, y) in env.iter_mut().zip(after_then.as_slice().iter()) {
                                        for (l, r) in x.iter_mut().zip(y) {
                                            if *l != *r {
                                                *l = None;
                                            }
                                        }
                                    }
                                    false
                                }
                                /* Both arms leave on data: nothing after
                                 * this in the entry runs unflagged. */
                                (true, true) => true,
                            };
                            after_then.release(self.a);
                            if leave {
                                return Ok(Flow::Flagged);
                            }
                        }
                    }
                }
                Node::Break => return Ok(Flow::Break),
                Node::Continue => return Ok(Flow::Continue),
                Node::Return => return Err(UnrollError::ReturnInLoop),
                /* An inner loop: what it stores is not known after it, and
                 * the outer pass goes on (its `break` and `continue` are
                 * its own). A `return` inside is still refused. */
                Node::Loop { body } => {
                    if self.has_return(body) {
                        return Err(UnrollError::ReturnInLoop);
                    }
                    self.forget_in(body, env);
                }
            }
        }
        Ok(Flow::Normal)
    }

    /// Does `r` hold a `return`, anywhere inside it?
    fn has_return(&self, r: ir::Range) -> bool {
        self.m.nodes_of(r).iter().any(|n| match *n {
            Node::Return => true,
            Node::If { then_, else_, .. } => self.has_return(then_) || self.has_return(else_),
            Node::Loop { body } => self.has_return(body),
            _ => false,
        })
    }

    /// For [`Unroll::sim`]: every local `r` stores to, anywhere inside it,
    /// becomes not known in `env`.
    fn forget_in(&self, r: ir::Range, env: &mut [Known]) {
        for n in self.m.nodes_of(r) {
            match *n {
                Node::Instr(id) => {
                    if let Some(Op::StoreLocal { local, .. }) = self.m.instr(id) {
                        if let Some(slot) = env.get_mut(local.0 as usize) {
                            *slot = [None; 4];
                        }
                    }
                }
                Node::If { then_, else_, .. } => {
                    self.forget_in(then_, env);
                    self.forget_in(else_, env);
                }
                Node::Loop { body } => self.forget_in(body, env),
                _ => {}
            }
        }
    }

    /// Every local `r` stores to, anywhere inside it, becomes not known.
    fn forget_stores(&mut self, r: ir::Range) {
        for n in self.m.nodes_of(r) {
            match *n {
                Node::Instr(id) => {
                    if let Some(Op::StoreLocal { local, .. }) = self.m.instr(id) {
                        if let Some(e) = self.env.as_mut_slice().get_mut(local.0 as usize) {
                            *e = [None; 4];
                        }
                    }
                }
                Node::If { then_, else_, .. } => {
                    self.forget_stores(then_);
                    self.forget_stores(else_);
                }
                Node::Loop { body } => self.forget_stores(body),
                _ => {}
            }
        }
    }

    /// Does `r` hold a `continue` of this loop (not of one inside it)?
    /// Does this loop's trip count depend on data (its schedule cannot be
    /// followed to an exit)? Such a loop is kept as a loop, counted at run
    /// time, when its exits are uniform ([`Unroll::uniform_exits`]).
    fn counted_at_run_time(&self, body: ir::Range) -> bool {
        matches!(self.trip_count(body), Err(UnrollError::DataDependentExit))
    }

    /// Is this loop's count known but past what the expansion copies
    /// ([`MAX_ITERATIONS`])? It is then kept as a loop with its exact count,
    /// wherever it sits, as a loop counted at run time is.
    fn too_long_to_expand(&self, body: ir::Range) -> bool {
        matches!(self.trip_count(body), Ok(c) if c > MAX_ITERATIONS)
    }

    /// Is every exit of this loop uniform -- the condition of every `if` a
    /// `break` sits in traced, through every operation, to uniforms,
    /// constants and locals only such values are stored to? Conservative:
    /// an input of any kind (a varying, an attribute, gl_VertexID, the
    /// fragment's position), a texture look-up or size, a derivative, and
    /// anything else it does not know make it answer no. A `return` or a
    /// nested loop's exits count as exits too.
    fn uniform_exits(&self, body: ir::Range) -> bool {
        let Ok(bad) = self.a.alloc_filled(self.m.locals.len(), false) else { return false };
        /* The locals that may hold a non-uniform value: the greatest
         * fixpoint -- every local starts uniform, and one is marked the
         * moment any value stored to it is not uniform under the current
         * marking, until nothing changes. A counter (`i = i + 1`) stays
         * uniform; a local anything non-uniform reaches, however
         * indirectly, does not. */
        loop {
            let mut changed = false;
            for op in self.m.instrs {
                if let Op::StoreLocal { local, index, src, .. } = *op {
                    let Some(b) = bad.get(local.0 as usize).copied() else { return false };
                    if b {
                        continue;
                    }
                    let ok =
                        index.is_none_or(|i| self.uniform_value(i.val, bad, 0)) && self.uniform_value(src.val, bad, 0);
                    if !ok {
                        bad[local.0 as usize] = true;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        fn walk(u: &Unroll<'_>, r: ir::Range, conds: &mut [Option<Src>; 64], n: &mut usize, bad: &[bool]) -> bool {
            for node in u.m.nodes_of(r) {
                match *node {
                    Node::Break | Node::Return => {
                        if conds[..*n].iter().flatten().any(|c| !u.uniform_value(c.val, bad, 0)) {
                            return false;
                        }
                    }
                    Node::If { cond, then_, else_ } => {
                        if *n >= conds.len() {
                            return false;
                        }
                        conds[*n] = Some(cond);
                        *n += 1;
                        let ok = walk(u, then_, conds, n, bad) && walk(u, else_, conds, n, bad);
                        *n -= 1;
                        if !ok {
                            return false;
                        }
                    }
                    Node::Loop { body } if !walk(u, body, conds, n, bad) => return false,
                    _ => {}
                }
            }
            true
        }
        let mut conds = [None; 64];
        let mut n = 0usize;
        walk(self, body, &mut conds, &mut n, bad)
    }

    /// Is value `v` of the original module uniform (see
    /// [`Unroll::uniform_exits`]), given the locals `bad` marks as possibly
    /// not? Through every operation to its leaves: a constant, a uniform (a
    /// dynamically indexed one by a uniform index), a local not marked. An
    /// input, a texture look-up or size, a derivative, or anything else is
    /// not.
    fn uniform_value(&self, v: ValueId, bad: &[bool], depth: u32) -> bool {
        if depth > 256 {
            return false;
        }
        let Some(op) = self.m.instr(ir::InstrId(v.0)) else { return false };
        match *op {
            Op::ConstF32(_) | Op::ConstI32(_) | Op::ConstBool(_) | Op::LoadUniform { .. } => true,
            Op::LoadUniformIdx { index, .. } => self.uniform_value(index.val, bad, depth + 1),
            Op::Alu(o, a) => {
                if matches!(o, AluOp::Fddx | AluOp::Fddy) {
                    return false;
                }
                (0..o.arity()).all(|k| self.uniform_value(a[k].val, bad, depth + 1))
            }
            Op::LoadLocal { local, index, .. } => {
                !bad.get(local.0 as usize).copied().unwrap_or(true)
                    && index.is_none_or(|i| self.uniform_value(i.val, bad, depth + 1))
            }
            _ => false,
        }
    }

    fn has_continue(&self, r: ir::Range) -> bool {
        self.m.nodes_of(r).iter().any(|n| match n {
            Node::Continue => true,
            Node::If { then_, else_, .. } => self.has_continue(*then_) || self.has_continue(*else_),
            _ => false,
        })
    }

    fn frame(&self) -> Option<Frame> {
        self.frames.as_slice().last().copied()
    }

    fn on_data(&mut self, by: i32) {
        if let Some(f) = self.frames.as_mut_slice().last_mut() {
            f.on_data = f.on_data.wrapping_add_signed(by);
        }
    }

    /// After an `if` on data: what each arm stored is known only where
    /// the two agree.
    fn meet(&mut self, after_then: &[Known]) {
        for (x, y) in self.env.as_mut_slice().iter_mut().zip(after_then.iter()) {
            for (l, r) in x.iter_mut().zip(y) {
                if *l != *r {
                    *l = None;
                }
            }
        }
    }

    /// An instruction the expansion makes, with a fresh id.
    fn make(&mut self, op: Op, comps: u8, out: &mut AVec<'a, Node>) -> Result<ValueId, UnrollError> {
        if self.instrs.len() >= MAX_INSTRS {
            return Err(UnrollError::TooLong);
        }
        let v = ValueId(self.instrs.len() as u32);
        let known = self.eval(&op);
        self.instrs.push(self.a, op)?;
        self.comps.push(self.a, comps)?;
        self.known.push(self.a, known)?;
        out.push(self.a, Node::Instr(ir::InstrId(v.0)))?;
        Ok(v)
    }

    /// A new flag local. `clear`: cleared where it is made, which must be
    /// a point every read of the flag comes after on every path (a kept
    /// loop's flags, made before the loop). Otherwise the caller clears it
    /// at such a point itself (an expanded loop's, at its entry).
    fn new_flag(&mut self, clear: bool, out: &mut AVec<'a, Node>) -> Result<ir::LocalId, UnrollError> {
        let id = ir::LocalId(self.locals.len() as u32);
        let ty = ir::Type::new(ir::ScalarType::Bool, 1);
        self.locals.push(self.a, ir::VarDecl { name: "", ty, array_len: 1 })?;
        self.env.push(self.a, [None; 4])?;
        if clear {
            self.set(id, false, out)?;
        }
        Ok(id)
    }

    /// `flag = value`, unguarded.
    fn set(&mut self, flag: ir::LocalId, value: bool, out: &mut AVec<'a, Node>) -> Result<(), UnrollError> {
        let c = self.make(Op::ConstBool([value; 4]), 1, out)?;
        let st = Op::StoreLocal { local: flag, index: None, src: Src::id(c), write_mask: 1 };
        self.make(st, 1, out)?;
        if let Some(e) = self.env.as_mut_slice().get_mut(flag.0 as usize) {
            /* Unknown after a set on data: the flags are the one thing the
             * schedule must not assume. */
            *e = [None; 4];
        }
        Ok(())
    }

    /// The condition every effect in the loops being expanded is guarded by:
    /// any of their flags set. `None` while no flag exists.
    fn guard(&mut self, out: &mut AVec<'a, Node>) -> Result<Option<Src>, UnrollError> {
        let mut acc: Option<ValueId> = None;
        for i in 0..self.frames.len() {
            let f = self.frames.as_slice()[i];
            for flag in [f.brk, f.cnt].into_iter().flatten() {
                let l = self.make(Op::LoadLocal { local: flag, index: None, comps: 1 }, 1, out)?;
                acc = Some(match acc {
                    None => l,
                    Some(p) => self.make(Op::Alu(AluOp::Ior, [Src::id(p), Src::id(l), Src::id(l)]), 1, out)?,
                });
            }
        }
        Ok(acc.map(Src::splat))
    }

    /// Does this range hold a `break` or `continue` that leaves it -- one not
    /// inside a loop of its own?
    fn leaves(&self, r: ir::Range) -> bool {
        self.m.nodes_of(r).iter().any(|n| match n {
            Node::Break | Node::Continue => true,
            Node::If { then_, else_, .. } => self.leaves(*then_) || self.leaves(*else_),
            _ => false,
        })
    }

    fn src(&self, s: Src) -> Src {
        match self.remap.get(s.val.0 as usize).copied().flatten() {
            Some(v) => Src { val: v, swizzle: s.swizzle },
            None => s,
        }
    }

    /// Lane `k` of an already remapped source, if known.
    fn lane(&self, s: Src, k: usize) -> Option<u32> {
        self.known.as_slice().get(s.val.0 as usize).and_then(|l| l[s.swizzle[k] as usize & 3])
    }

    /// Copy one instruction with its sources remapped, give it a fresh id,
    /// and note what its value is known to be.
    fn instr(&mut self, id: ir::InstrId) -> Result<ir::InstrId, UnrollError> {
        let op = *self.m.instr(id).ok_or(UnrollError::OutOfScratch)?;
        let s = |u: &Self, x: Src| u.src(x);
        let op = match op {
            Op::Alu(o, a) => Op::Alu(o, [s(self, a[0]), s(self, a[1]), s(self, a[2])]),
            Op::LoadUniformIdx { base_dw, stride_dw, len, index, comps } => {
                Op::LoadUniformIdx { base_dw, stride_dw, len, index: s(self, index), comps }
            }
            Op::StoreOutput { slot, src } => Op::StoreOutput { slot, src: s(self, src) },
            Op::LoadLocal { local, index, comps } => Op::LoadLocal { local, index: index.map(|i| s(self, i)), comps },
            Op::StoreLocal { local, index, src, write_mask } => {
                Op::StoreLocal { local, index: index.map(|i| s(self, i)), src: s(self, src), write_mask }
            }
            Op::Tex { dim, shadow, proj, ret, sampler, coord, lod, offset } => {
                let id = |u: &Self, v: ValueId| u.src(Src::id(v)).val;
                let lod = match lod {
                    ir::TexLod::Implicit => ir::TexLod::Implicit,
                    ir::TexLod::Bias(x) => ir::TexLod::Bias(s(self, x)),
                    ir::TexLod::Lod(x) => ir::TexLod::Lod(s(self, x)),
                    ir::TexLod::Fetch(x) => ir::TexLod::Fetch(s(self, x)),
                    ir::TexLod::Grad { ddx, ddy } => ir::TexLod::Grad { ddx: id(self, ddx), ddy: id(self, ddy) },
                };
                Op::Tex { dim, shadow, proj, ret, sampler, coord: s(self, coord), lod, offset }
            }
            Op::TexSize { sampler, dim, lod } => Op::TexSize { sampler, dim, lod: s(self, lod) },
            other => other,
        };
        if self.instrs.len() >= MAX_INSTRS {
            return Err(UnrollError::TooLong);
        }
        let new = ValueId(self.instrs.len() as u32);
        let known = self.eval(&op);
        if let Op::StoreLocal { local, index, src, write_mask } = op {
            let known = self.known.as_slice();
            if let Some(slot) = self.env.as_mut_slice().get_mut(local.0 as usize) {
                if index.is_some() {
                    *slot = [None; 4];
                } else {
                    for k in 0..4 {
                        if write_mask & (1 << k) != 0 {
                            slot[k] = known.get(src.val.0 as usize).and_then(|l| l[src.swizzle[k] as usize & 3]);
                        }
                    }
                }
            }
        }
        self.instrs.push(self.a, op)?;
        self.comps.push(self.a, self.m.comps_of(ValueId(id.0)))?;
        self.known.push(self.a, known)?;
        if let Some(r) = self.remap.get_mut(id.0 as usize) {
            *r = Some(new);
        }
        Ok(ir::InstrId(new.0))
    }

    /// What `op`'s lanes are known to be, over the expansion's own values.
    fn eval(&self, op: &Op) -> Known {
        eval_with(op, |s, k| self.lane(s, k), |l| self.env.as_slice().get(l.0 as usize).copied().unwrap_or([None; 4]))
    }
}

/// What `op`'s lanes are known to be, given what each source lane and each
/// local is known to hold. Booleans are all ones or zero, as the back ends
/// hold them; only whether one is zero is ever asked.
fn eval_with(op: &Op, lane: impl Fn(Src, usize) -> Option<u32>, local: impl Fn(ir::LocalId) -> Known) -> Known {
    {
        let b = |x: bool| if x { u32::MAX } else { 0 };
        match *op {
            Op::ConstF32(c) => c.map(Some),
            Op::ConstI32(c) => c.map(|x| Some(x as u32)),
            Op::ConstBool(c) => c.map(|x| Some(b(x))),
            Op::LoadLocal { local: l, index: None, .. } => local(l),
            Op::Alu(o, a) => {
                use AluOp::*;
                let n = o.arity();
                let mut out = [None; 4];
                for (k, slot) in out.iter_mut().enumerate() {
                    let x = lane(a[0], k);
                    let y = if n > 1 { lane(a[1], k) } else { Some(0) };
                    let z = if n > 2 { lane(a[2], k) } else { Some(0) };
                    let (Some(p), Some(q), Some(r)) = (x, y, z) else { continue };
                    let f = f32::from_bits;
                    let fb = |v: f32| v.to_bits();
                    *slot = Some(match o {
                        Fadd => fb(f(p) + f(q)),
                        Fmul => fb(f(p) * f(q)),
                        Ffma => fb(f(p) * f(q) + f(r)),
                        Fneg => p ^ 0x8000_0000,
                        Fabs => p & 0x7fff_ffff,
                        Iadd => p.wrapping_add(q),
                        Imul => p.wrapping_mul(q),
                        Ineg => p.wrapping_neg(),
                        Flt => b(f(p) < f(q)),
                        Fle => b(f(p) <= f(q)),
                        Feq => b(f(p) == f(q)),
                        Fne => b(f(p) != f(q)),
                        Ilt => b((p as i32) < (q as i32)),
                        Ile => b((p as i32) <= (q as i32)),
                        Ieq => b(p == q),
                        Ine => b(p != q),
                        Iand => p & q,
                        Ior => p | q,
                        Ixor => p ^ q,
                        Bnot => b(p == 0),
                        I2f => fb(p as i32 as f32),
                        F2i => f(p) as i32 as u32,
                        B2i => u32::from(p != 0),
                        B2f => fb(if p != 0 { 1.0 } else { 0.0 }),
                        F2b => b(f(p) != 0.0),
                        I2b => b(p != 0),
                        Bcsel => {
                            if p != 0 {
                                q
                            } else {
                                r
                            }
                        }
                        _ => continue,
                    });
                }
                out
            }
            _ => [None; 4],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::sema::Stage;
    use crate::shaderc::online::{lower, store};

    /// Unroll `src`'s fragment IR and hand back what `f` makes of it.
    fn with_unrolled<T>(
        src: &str,
        f: impl FnOnce(&ir::Module<'_>, Result<Option<Unrolled<'_>>, UnrollError>) -> T,
    ) -> T {
        let _l = store::test_lock();
        store::with_scratch(|a| {
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            let m = a.alloc_copy(core::slice::from_ref(&l.module)).unwrap();
            let m: &ir::Module<'_> = &m[0];
            f(m, unroll(m, a))
        })
        .unwrap()
    }

    /// The first local the unroller made that `m` reads before a store to
    /// it on some path, or `None`. Flags are the only locals it makes, so
    /// this is "a flag read before it is cleared". A loop's body runs at
    /// least once (an expanded loop's copies are straight-line code; a kept
    /// loop's trip count is at least 1).
    fn flag_read_unset(orig_locals: usize, m: &ir::Module<'_>) -> Option<(u32, u32)> {
        fn walk(m: &ir::Module<'_>, r: ir::Range, set: &mut Vec<bool>, from: usize) -> Option<(u32, u32)> {
            for n in m.nodes_of(r) {
                match n {
                    Node::Instr(id) => match m.instr(*id) {
                        Some(Op::LoadLocal { local, .. }) if local.0 as usize >= from && !set[local.0 as usize] => {
                            return Some((local.0, id.0));
                        }
                        Some(Op::StoreLocal { local, .. }) => set[local.0 as usize] = true,
                        _ => {}
                    },
                    Node::If { then_, else_, .. } => {
                        let mut t = set.clone();
                        let mut e = set.clone();
                        if let Some(x) = walk(m, *then_, &mut t, from).or_else(|| walk(m, *else_, &mut e, from)) {
                            return Some(x);
                        }
                        for (s, (a, b)) in set.iter_mut().zip(t.iter().zip(&e)) {
                            *s = *a && *b;
                        }
                    }
                    Node::Loop { body } => {
                        if let Some(x) = walk(m, *body, set, from) {
                            return Some(x);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        let mut set = vec![false; m.locals.len()];
        walk(m, m.body, &mut set, orig_locals)
    }

    /// dEQP functions.control_flow.return_in_nested_loop: a `return` from
    /// inside a `for` inside a `for`. At i == 0 the inner loop is kept (it
    /// can leave on data), so the return flag is not known afterwards, and
    /// the outer loop's `if (returned) break` makes the outer break flag on
    /// first use, inside that `if`'s arm. The flag was cleared there, in the
    /// arm, so every path that skipped the arm read it unset -- the IR
    /// evaluator's UNSET, which is true -- and every later iteration and the
    /// return itself were skipped: func returned its final 1.0 whatever `a`
    /// was (the board drew it wrong on the AM62P batch 16 and the Pi 4).
    /// Now an expanded loop clears the flags it made at its entry.
    ///
    /// Revert check: make the lazily made flag cleared where it is made
    /// (`new_flag(true, ..)` in the `Break | Continue` arm, and no clear at
    /// the loop's entry) and the unrolled module gives 1.0 with its flag
    /// read undefined.
    #[test]
    fn a_return_inside_a_nested_loop_is_kept() {
        use crate::shaderc::online::eval::{f32x4, run, Inputs};
        use crate::shaderc::online::ir::IoSlot;
        let src = "precision mediump float; varying vec4 v; \
            float func(float a) { for (int i = 0; i < 6; i++) { a = -a; \
              for (int j = 0; j < 4; j++) { a = -a; if (i == 1) return a; } \
              if (i == 4) return 1.0; } return 1.0; } \
            void main() { gl_FragColor = vec4(func(v.x), func(v.y), 0.0, 1.0); }";
        with_unrolled(src, |m, u| {
            /* dEQP's own values, on the original: func(a) is -a after i ==
             * 0's five negations and i == 1's two */
            for (x, y) in [(-0.5f32, 1.5f32), (0.3, -2.25)] {
                let io = [(IoSlot::Varying(0), f32x4([x, y, 0.0, 0.0]))];
                let got = run(m, &Inputs { io: &io, uniforms: &[] }).expect("the original runs");
                assert_eq!(got.output_f32(IoSlot::FragColor(0)), Some([-x, -y, 0.0, 1.0]), "the original");
            }
            /* The evaluator cannot run the unrolled module (a kept loop's
             * count is outside the IR), so the property that failed is
             * checked: no flag is read before it is cleared. The values
             * end to end are the V3D three-way pair es100/ret_nested. */
            let u = u.expect("unrolls").expect("has loops");
            assert!(!u.trips.is_empty(), "the inner loop at i == 0 is kept: the case this pins");
            assert!(u.module.locals.len() > m.locals.len(), "the unroller made flags");
            assert_eq!(flag_read_unset(m.locals.len(), u.module), None, "(flag local, instruction) read unset");
        });
    }

    fn count(m: &ir::Module<'_>, want: impl Fn(&Op) -> bool) -> usize {
        fn walk(m: &ir::Module<'_>, r: ir::Range, want: &dyn Fn(&Op) -> bool) -> usize {
            m.nodes_of(r)
                .iter()
                .map(|n| match n {
                    Node::Instr(id) => usize::from(m.instr(*id).is_some_and(want)),
                    Node::If { then_, else_, .. } => walk(m, *then_, want) + walk(m, *else_, want),
                    Node::Loop { body } => walk(m, *body, want),
                    _ => 0,
                })
                .sum()
        }
        walk(m, m.body, &want)
    }

    /// `for (int i = 0; i < 3; i++)` becomes three copies of its body and no
    /// loop; the reachable nodes hold one `Fadd` per iteration.
    #[test]
    fn a_counted_loop_becomes_its_iterations() {
        with_unrolled(
            "precision mediump float; uniform vec4 u; void main(){ float s = 0.0; \
             for (int i = 0; i < 3; i++) { s += u[i]; } gl_FragColor = vec4(s); }",
            |before, after| {
                let after = after.expect("unrolls").expect("the shader has a loop").module;
                assert!(!after.nodes.iter().any(|n| matches!(n, Node::Loop { .. } | Node::Break | Node::Continue)));
                let adds = |m: &ir::Module<'_>| count(m, |op| matches!(op, Op::Alu(AluOp::Fadd, _)));
                assert_eq!(adds(before), 1);
                assert_eq!(adds(after), 3, "one sum per iteration, and no fourth");
            },
        );
    }

    /// Nested loops multiply: 3 x 3 bodies.
    #[test]
    fn nested_loops_multiply() {
        with_unrolled(
            "precision mediump float; uniform vec4 u; void main(){ float s = 0.0; \
             for (int x = -1; x <= 1; x++) { for (int y = -1; y <= 1; y++) { s += u.x * float(x * y); } } \
             gl_FragColor = vec4(s); }",
            |_, after| {
                let after = after.expect("unrolls").expect("loops").module;
                assert_eq!(count(after, |op| matches!(op, Op::Alu(AluOp::Fadd, _))), 9);
            },
        );
    }

    /// A loop that can leave on data stays a loop: its body once, the
    /// `break` a flag made before it, and its trip count recorded -- 25,
    /// the 24 iterations and the entry whose head test breaks. No `break`
    /// is left for selection. A loop longer than the expansion bound (1000
    /// iterations of a constant bound) was refused; its exit is uniform, so
    /// it is now kept as a loop counted at run time: UNIFORM_LOOP_CAP
    /// entries at most, leaving on its break flag after its 1000.
    #[test]
    fn a_break_on_data_keeps_the_loop_with_its_trip_count_and_an_overlong_loop_runs_counted() {
        with_unrolled(
            "precision mediump float; uniform vec4 u; void main(){ float s = 0.0; \
             for (float i = 1.0; i <= 24.0; i += 1.0) { if (i >= u.x) { break; } s += i; } gl_FragColor = vec4(s); }",
            |before, after| {
                let u = after.expect("unrolls").expect("the shader has a loop");
                let after = u.module;
                assert_eq!(u.trips.len(), 1, "one loop kept");
                assert_eq!(u.trips[0].count, 25, "24 iterations and the entry that breaks at its head");
                assert!(
                    u.trips[0].exit.is_some(),
                    "the uniform break ends the loop early, not only the counter (femtovg filter_image)"
                );
                let loops: Vec<_> =
                    after.nodes_of(after.body).iter().filter(|n| matches!(n, Node::Loop { .. })).collect();
                assert_eq!(loops.len(), 1);
                assert!(!after.nodes.iter().any(|n| matches!(n, Node::Break | Node::Continue)));
                assert_eq!(
                    count(after, |op| matches!(op, Op::Alu(AluOp::Fadd, _))),
                    2,
                    "the sum and the index step, once"
                );
                assert_eq!(after.locals.len(), before.locals.len() + 1, "one flag");
            },
        );
        with_unrolled(
            "precision mediump float; uniform vec4 u; void main(){ float s = 0.0; \
             for (int i = 0; i < 1000; i++) { s += u.x; } gl_FragColor = vec4(s); }",
            |_, after| {
                let u = after.expect("unrolls").expect("the shader has a loop");
                assert_eq!(u.trips.len(), 1, "one loop kept");
                assert_eq!(u.trips[0].count, 1001, "kept with its own count: 1000 passes and the entry that fails");
                assert!(u.trips[0].exit.is_none(), "the counter alone decides");
            },
        );
    }

    /// A counted loop whose break reads a varying (it may differ between
    /// instances) keeps the counter alone: every instance runs every pass,
    /// the ones after its break under the flag. Revert check: taking the
    /// flag without the uniform test gives this loop an exit.
    #[test]
    fn a_counted_loop_with_a_divergent_break_keeps_the_counter_alone() {
        with_unrolled(
            "precision mediump float; varying vec4 v; void main(){ float s = 0.0; \
             for (float i = 1.0; i <= 24.0; i += 1.0) { if (i >= v.x) { break; } s += i; } gl_FragColor = vec4(s); }",
            |_, after| {
                let u = after.expect("unrolls").expect("the shader has a loop");
                assert_eq!(u.trips.len(), 1, "one loop kept");
                assert_eq!(u.trips[0].count, 25);
                assert!(u.trips[0].exit.is_none() && !u.trips[0].divergent, "{:?}", u.trips[0]);
            },
        );
    }

    /// Every shader in tools/shaders through the unroller with the early
    /// exit on and off (EARLY_EXIT_OFF): the loops it gives an exit, per
    /// file. The list is what a board run of the change must cover (gl_test
    /// cases whose shaders are here); femtovg's filter_image is the known
    /// member. Pinned, so a new member is a deliberate re-pin. Every file is
    /// accounted for: compiled, or refused by the front end, counted.
    #[test]
    fn the_corpus_loops_the_early_exit_reaches_are_pinned() {
        use std::string::String;
        fn files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let mut es: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
            es.sort();
            for p in es {
                if p.is_dir() {
                    files(&p, out);
                } else if matches!(p.extension().and_then(|e| e.to_str()), Some("frag" | "vert")) {
                    out.push(p);
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders");
        let mut all = Vec::new();
        files(&root, &mut all);
        let (mut compiled, mut refused, mut with_loops) = (0usize, 0usize, 0usize);
        let mut reached: Vec<String> = Vec::new();
        let _l = store::test_lock();
        for p in &all {
            let src = std::fs::read_to_string(p).unwrap();
            let stage = if p.extension().unwrap() == "vert" { Stage::Vertex } else { Stage::Fragment };
            let name = p.strip_prefix(&root).unwrap().to_string_lossy().into_owned();
            let exits = |off: bool| -> Option<Option<Vec<bool>>> {
                EARLY_EXIT_OFF.with(|h| h.set(off));
                let r = store::with_scratch(|a| {
                    let text = a.alloc_str(&src).unwrap();
                    let Ok(l) = lower::compile_to_ir(a, stage, text) else { return None };
                    let m = a.alloc_copy(core::slice::from_ref(&l.module)).unwrap();
                    Some(match unroll(&m[0], a) {
                        Ok(Some(u)) => Some(u.trips.iter().map(|t| t.exit.is_some()).collect()),
                        _ => None,
                    })
                })
                .unwrap();
                EARLY_EXIT_OFF.with(|h| h.set(false));
                r
            };
            let (Some(on), Some(off)) = (exits(false), exits(true)) else {
                refused += 1;
                continue;
            };
            compiled += 1;
            if on.is_some() {
                with_loops += 1;
            }
            if on != off {
                reached.push(name);
            }
        }
        std::println!("{} files: {compiled} compiled ({with_loops} with a kept loop), {refused} refused", all.len());
        for r in &reached {
            std::println!("  early exit: {r}");
        }
        assert_eq!(compiled + refused, all.len(), "every file accounted for");
        assert!(reached.iter().any(|r| r == "femtovg/filter_image.frag"), "the known member: {reached:?}");
        assert_eq!(reached, PINNED_EARLY_EXIT, "the loops the early exit reaches moved");
    }

    /// Measured 2026-09-26: 708 files, 652 compiled (82 with a kept loop),
    /// 56 refused by the front end. gl_test 70 loopcarry and 71 deadloop,
    /// and femtovg's image blur. 709 / 653 / 83 with gl_test 148 texbranch,
    /// whose loop is the blur's shape.
    const PINNED_EARLY_EXIT: &[&str] = &[
        "es100/deadloop.frag",
        "es100/loopcarry.frag",
        "es100/texbranch.frag",
        "femtovg/filter_image.frag",
        "femtovg/filter_image.nodiscard.frag",
    ];

    /// A constant loop past what the expansion copies is kept with its own
    /// count, not cut at UNIFORM_LOOP_CAP (online/loop_count.rs): 5000 passes
    /// and the entry that fails, the counter alone deciding. A loop that
    /// leaves only on data still gets the cap.
    #[test]
    fn a_constant_loop_of_5000_is_kept_with_its_exact_count() {
        with_unrolled(
            "precision mediump float; uniform vec4 u; void main(){ float s = 0.0; \
             for (int i = 0; i < 5000; i++) { s += u.x; } gl_FragColor = vec4(s); }",
            |_, after| {
                let u = after.expect("unrolls").expect("the shader has a loop");
                assert_eq!(u.trips.len(), 1, "one loop kept");
                assert_eq!(u.trips[0].count, 5001, "5000 passes and the entry that fails, not the cap");
                assert!(u.trips[0].exit.is_none(), "the counter alone decides");
            },
        );
        with_unrolled(
            "precision mediump float; uniform vec4 u; void main(){ float s = 0.0; \
             while (true) { if (s > u.x) break; s += 1.0; } gl_FragColor = vec4(s); }",
            |_, after| {
                let u = after.expect("unrolls").expect("the shader has a loop");
                assert_eq!(u.trips[0].count, UNIFORM_LOOP_CAP, "a data-only exit keeps the cap");
            },
        );
    }

    /// The boundary shared with the GC7000 (loop_count.rs EXACT_PASSES_MAX):
    /// 65,536 passes are kept with their exact count, 65,537 get the cap.
    #[test]
    fn the_exact_boundary_is_65536_passes() {
        let src = |n: u32| {
            std::format!("precision mediump float; uniform vec4 u; void main(){{ float s = 0.0; for (int i = 0; i < {n}; i++) {{ s += u.x; }} gl_FragColor = vec4(s); }}")
        };
        with_unrolled(&src(65_536), |_, after| {
            let u = after.expect("unrolls").expect("a loop");
            assert_eq!((u.trips[0].count, u.trips[0].exit.is_none()), (65_537, true), "65,536 passes: exact");
        });
        with_unrolled(&src(65_537), |_, after| {
            let u = after.expect("unrolls").expect("a loop");
            assert_eq!(u.trips[0].count, UNIFORM_LOOP_CAP, "65,537 passes: capped");
        });
    }
}
