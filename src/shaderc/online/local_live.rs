// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Which local lanes are live where: the backward liveness of the IR's
//! mutable slots, read from the IR alone.
//!
//! A local lane is live at a point when some path from there reads it
//! before writing it, and the read matters: a `LoadLocal` counts only when
//! its value reaches something that is kept (an output, a `discard`, a store
//! to a live lane, an `if`'s condition, or an implicit read the caller
//! names). That is strong liveness: a slot read only to compute a store to a
//! dead slot is dead too. Values are tracked whole (every lane of a value at
//! once); slots lane by lane, as the selectors hold them
//! (`local * LANES + lane`).
//!
//! Why it exists. A selector that keeps a loop as a loop gives each slot it
//! carries a home register live over the whole body, so that the next pass
//! starts from the value the last one left. Which slots need one is a
//! liveness question -- the slots live at the loop's head -- and answering it
//! with "every slot the body writes" made every temporary of an inlined
//! function a register for the whole loop: Qt Quick 3D's PrincipledMaterial,
//! whose light loop inlines a dozen functions, needed 497 temporaries on the
//! PowerVR against a file of 252 (2026-11-16). A temporary a pass writes and
//! then reads in the same pass is not live at the head and needs no home.
//!
//! The walk is structured, like the IR: an `if` is the union of its arms (the
//! selectors run both and select at the join, which reads no slot the
//! branching program would not), a loop is walked until the set at its head
//! stops growing (the sets only grow, so it ends), `return` ends every read.
//! `break` and `continue` take every lane: a kept loop has none (the
//! unroller turns them into flags), so that is only ever the safe answer for
//! a module no selector keeps a loop of.

use super::arena::{Arena, ArenaError};
use super::ir::{self, Node, Op};

/// Lanes per slot, as the selectors index them.
pub const LANES: usize = 4;

/// The result: per loop, the slot lanes it must carry from pass to pass.
pub struct LocalLive<'a> {
    /// Slot lanes the module declares (`locals.len() * LANES`).
    lanes: usize,
    /// The first node of each loop's body, in walk order.
    loops: &'a [u32],
    /// `loops.len() * lanes` flags: loop `i`'s set at `i * lanes`.
    heads: &'a [bool],
}

impl LocalLive<'_> {
    /// Must the loop whose body starts at node `body_first` carry slot lane
    /// `idx` (`local * LANES + lane`) -- is it live at the loop's head (a
    /// pass reads what the previous one left) or after the loop (the code
    /// that follows reads what the last pass left)? A lane past the module's
    /// own slots (one a selector added, such as the discard flag) or a loop
    /// the walk never met answers `true`, so a caller keeps what it would
    /// have kept without the analysis.
    ///
    /// A lane the body writes and that is live at neither place is a
    /// temporary of one pass: it needs no register beyond that pass.
    pub fn carried_by_loop(&self, body_first: u32, idx: usize) -> bool {
        if idx >= self.lanes {
            return true;
        }
        match self.loops.iter().position(|&f| f == body_first) {
            Some(i) => self.heads.get(i * self.lanes + idx).copied().unwrap_or(true),
            None => true,
        }
    }
}

/// The liveness of `m`'s slots. `exit_flag(body_first)` names the slot a
/// selector reads at a kept loop's back edge without a `LoadLocal` in the IR
/// (`unroll::Trip::exit`): that read is counted at the end of the body.
pub fn analyse<'a>(
    m: &ir::Module<'_>,
    a: &Arena<'a>,
    exit_flag: impl Fn(u32) -> Option<ir::LocalId>,
) -> Result<LocalLive<'a>, ArenaError> {
    let lanes = m.locals.len() * LANES;
    let (n_loops, depth) = shape(m, m.body, 0);
    let loops = a.alloc_filled(n_loops, u32::MAX)?;
    let heads = a.alloc_filled(n_loops * lanes, false)?;
    /* Three sets per nesting level -- an `if`'s other arm, or a loop's set
     * after it, its head and the pass being walked -- so nothing is
     * allocated inside the walk however many passes a loop takes. */
    let pool = a.alloc_filled((depth + 1) * SETS * lanes, false)?;
    let values = a.alloc_filled(m.instrs.len(), false)?;
    let live = a.alloc_filled(lanes, false)?;
    let mut w = Walk { m, lanes, values, loops, heads, seen: 0, exit_flag: &exit_flag };
    w.range(m.body, live, pool);
    Ok(LocalLive { lanes, loops: w.loops, heads: w.heads })
}

/// Sets per nesting level in the walk's pool.
const SETS: usize = 3;

/// How many loops `r` holds, and how deeply its `if`s and loops nest.
fn shape(m: &ir::Module<'_>, r: ir::Range, d: usize) -> (usize, usize) {
    let mut loops = 0;
    let mut depth = d;
    for n in m.nodes_of(r) {
        match *n {
            Node::If { then_, else_, .. } => {
                let (l1, d1) = shape(m, then_, d + 1);
                let (l2, d2) = shape(m, else_, d + 1);
                loops += l1 + l2;
                depth = depth.max(d1).max(d2);
            }
            Node::Loop { body } => {
                let (l, d1) = shape(m, body, d + 1);
                loops += l + 1;
                depth = depth.max(d1);
            }
            _ => {}
        }
    }
    (loops, depth)
}

struct Walk<'m, 'a, 'f> {
    m: &'m ir::Module<'m>,
    lanes: usize,
    /// By value: something kept reads it.
    values: &'a mut [bool],
    loops: &'a mut [u32],
    heads: &'a mut [bool],
    /// Loops recorded so far (the next one's index in `loops`).
    seen: usize,
    exit_flag: &'f dyn Fn(u32) -> Option<ir::LocalId>,
}

impl Walk<'_, '_, '_> {
    fn use_value(&mut self, v: ir::ValueId) {
        if let Some(x) = self.values.get_mut(v.0 as usize) {
            *x = true;
        }
    }

    /// `live` holds the set after `r` on entry and the set before it on
    /// exit. `pool` is this level's sets and every deeper level's.
    fn range(&mut self, r: ir::Range, live: &mut [bool], pool: &mut [bool]) {
        let n = self.lanes;
        for node in self.m.nodes_of(r).iter().rev() {
            match *node {
                Node::Instr(id) => self.instr(id, live),
                Node::If { cond, then_, else_ } => {
                    let (mine, deeper) = pool.split_at_mut(SETS * n);
                    let arm = &mut mine[..n];
                    arm.copy_from_slice(live);
                    self.range(then_, arm, deeper);
                    self.range(else_, live, deeper);
                    for (l, t) in live.iter_mut().zip(arm.iter()) {
                        *l |= *t;
                    }
                    self.use_value(cond.val);
                }
                Node::Loop { body } => {
                    let (mine, deeper) = pool.split_at_mut(SETS * n);
                    self.loop_(body, live, mine, deeper);
                }
                Node::Break | Node::Continue => live.fill(true),
                Node::Return => live.fill(false),
            }
        }
    }

    fn loop_(&mut self, body: ir::Range, live: &mut [bool], mine: &mut [bool], deeper: &mut [bool]) {
        let n = self.lanes;
        let (after, rest) = mine.split_at_mut(n);
        let (head, pass) = rest.split_at_mut(n);
        after.copy_from_slice(live);
        head.fill(false);
        let flag = (self.exit_flag)(body.first);
        let slot = self.seen;
        self.seen += 1;
        loop {
            /* The set at the back edge: what follows the loop, what the next
             * pass reads at its head, and the exit flag the selector tests
             * there. */
            for i in 0..n {
                pass[i] = after[i] | head[i];
            }
            if let Some(x) = flag.and_then(|f| pass.get_mut(f.0 as usize * LANES)) {
                *x = true;
            }
            self.range(body, pass, deeper);
            let mut grew = false;
            for i in 0..n {
                if pass[i] && !head[i] {
                    head[i] = true;
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        if let Some(l) = self.loops.get_mut(slot) {
            *l = body.first;
        }
        if let Some(h) = self.heads.get_mut(slot * n..(slot + 1) * n) {
            for i in 0..n {
                h[i] = head[i] | after[i];
            }
        }
        /* Before the loop: its head (the body runs first), and what follows
         * it, for a selector that runs a loop of count 0 not at all. */
        for i in 0..n {
            live[i] = head[i] | after[i];
        }
    }

    fn instr(&mut self, id: ir::InstrId, live: &mut [bool]) {
        let Some(op) = self.m.instr(id) else { return };
        let used = self.values.get(id.0 as usize).copied().unwrap_or(false);
        match *op {
            Op::StoreLocal { local, index: None, src, write_mask } => {
                let base = local.0 as usize * LANES;
                let mut needed = false;
                for k in 0..LANES {
                    if write_mask & (1 << k) != 0 {
                        if let Some(l) = live.get_mut(base + k) {
                            needed |= *l;
                            *l = false;
                        }
                    }
                }
                if needed {
                    self.use_value(src.val);
                }
            }
            Op::StoreLocal { local, index: Some(i), src, .. } => {
                /* An element the walk cannot name: the store kills nothing,
                 * and the slot's other elements stay as they were. */
                let base = local.0 as usize * LANES;
                if let Some(l) = live.get_mut(base..base + LANES) {
                    l.fill(true);
                }
                self.use_value(i.val);
                self.use_value(src.val);
            }
            Op::LoadLocal { local, index, comps } => {
                if used {
                    let base = local.0 as usize * LANES;
                    let k = if index.is_some() { LANES } else { (comps as usize).clamp(1, LANES) };
                    if let Some(l) = live.get_mut(base..base + k) {
                        l.fill(true);
                    }
                    if let Some(i) = index {
                        self.use_value(i.val);
                    }
                }
            }
            Op::StoreOutput { .. } | Op::Discard => op.for_each_src(|s| self.use_value(s.val)),
            _ => {
                if used {
                    op.for_each_src(|s| self.use_value(s.val));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::powervr::unroll;
    use crate::shaderc::online::sema::Stage;
    use crate::shaderc::online::{lower, store};

    /// `src`'s fragment IR after the unroller, and for its one kept loop
    /// the names of the locals with a lane it carries (the unroller's
    /// nameless flags as `<flag>`), sorted.
    fn live_at_head(src: &str) -> std::vec::Vec<std::string::String> {
        let _l = store::test_lock();
        store::with_scratch(|a| {
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            let m0 = a.alloc_copy(core::slice::from_ref(&l.module)).unwrap();
            let u = unroll::unroll(&m0[0], a).unwrap().expect("a loop");
            assert_eq!(u.trips.len(), 1, "one kept loop");
            let t = u.trips[0];
            let live = analyse(u.module, a, |f| if f == t.first { t.exit } else { None }).unwrap();
            let mut out: std::vec::Vec<std::string::String> = (0..u.module.locals.len())
                .filter(|&k| (0..LANES).any(|lane| live.carried_by_loop(t.first, k * LANES + lane)))
                .map(|k| match u.module.locals[k].name {
                    "" => "<flag>".into(),
                    n => n.into(),
                })
                .collect();
            out.sort();
            out.dedup();
            out
        })
        .unwrap()
    }

    /// GLSL ES 1.00, so a report-es2 build runs these too (it refuses ES
    /// 3.00 sources): `o` is the fragment colour.
    const HEAD: &str = "#version 100\nprecision highp float;\nuniform int n;\nuniform vec4 u;\n\
                        #define o gl_FragColor\n";

    /// The accumulator crosses passes; a temporary written before it is read
    /// in the same pass does not; neither does the loop index's copy the
    /// front end makes, which the step writes before the next test reads it.
    /// The exit flag is read at the back edge, so it is live at the head.
    #[test]
    fn the_accumulator_and_the_exit_flag_are_live_a_pass_temporary_is_not() {
        let got = live_at_head(&std::format!(
            "{HEAD}void main() {{ vec4 acc = vec4(0.0); \
             for (int i = 0; i < n; i++) {{ vec4 t = u * float(i); acc += t; }} o = acc; }}"
        ));
        assert!(got.contains(&"acc".into()), "{got:?}");
        assert!(got.contains(&"<flag>".into()), "the exit flag: {got:?}");
        assert!(!got.contains(&"t".into()), "a temporary of one pass: {got:?}");
    }

    /// A local every pass writes and only the code after the loop reads IS
    /// live at the head: the unroller runs a kept loop's body under its
    /// flags (the head test becomes a flag too, so the pass that fails the
    /// test runs nothing), and on that pass the local must still hold what
    /// the previous pass left. The same with a uniform bound and with a
    /// constant one too long to expand.
    #[test]
    fn a_local_read_after_the_loop_is_carried_through_the_guarded_pass() {
        for bound in ["n", "300"] {
            let got = live_at_head(&std::format!(
                "{HEAD}void main() {{ float last = 0.0; vec4 acc = vec4(0.0); \
                 for (int i = 0; i < {bound}; i++) {{ last = float(i); acc += u; }} o = acc * last; }}"
            ));
            assert!(got.contains(&"last".into()) && got.contains(&"acc".into()), "bound {bound}: {got:?}");
        }
    }

    /// Written in one arm only, a local keeps its value from the previous
    /// pass on the other path: live at the head.
    #[test]
    fn a_local_written_in_one_arm_is_live_at_the_head() {
        let got = live_at_head(&std::format!(
            "{HEAD}void main() {{ float k = 1.0; vec4 acc = vec4(0.0); \
             for (int i = 0; i < n; i++) {{ if (u.x > float(i)) {{ k = float(i); }} acc += u * k; }} o = acc; }}"
        ));
        assert!(got.contains(&"k".into()), "{got:?}");
    }

    /// Strong liveness: `d` is read in the loop only to compute a store to
    /// `e`, which nothing reads, so neither is live at the head.
    #[test]
    fn a_local_read_only_for_a_dead_store_is_dead() {
        let got = live_at_head(&std::format!(
            "{HEAD}void main() {{ float d = 0.0; float e = 0.0; vec4 acc = vec4(0.0); \
             for (int i = 0; i < n; i++) {{ e = d * 2.0; d = d + 1.0; acc += u; }} o = acc; }}"
        ));
        assert!(!got.contains(&"d".into()) && !got.contains(&"e".into()), "{got:?}");
    }
}
