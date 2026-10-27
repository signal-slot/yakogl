// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! How many times a constant `for` loop runs, read from the IR alone.
//!
//! One rule for every GPU: the GC7000's selection (select.rs `loop_convert`)
//! gives a recognized loop its exact count as the guard, and the shared
//! unroller (powervr/unroll.rs `trip_count`, used by the PowerVR and the V3D)
//! asks it once its own simulation has followed 256 passes (its MAX_ITERATIONS). Moved out of
//! select.rs by the (C') proposal (.claude/worktrees/proposal-loop-cap-constant-bounds.md).

use super::ir;

/// The longest constant `for` loop that keeps its exact count, in passes: one
/// boundary for every GPU (the GC7000's guard and the shared unroller both
/// read it through [`constant_loop_entries`]). A loop of at most this many
/// passes is exact -- `for (i = 0; i < 65536; i++)` runs 65,536 times --
/// and one longer gets the cap (4096), like a loop whose count cannot be read.
pub const EXACT_PASSES_MAX: u32 = 1 << 16;

/// How many times a constant `for` loop's head is entered, the entry whose
/// test fails included, or `None` when the loop is not that shape and its
/// trip count cannot be read from the program.
///
/// The shape is the one the front end builds for `for (i = A; i OP B; i += C)`
/// with A, B and C constants (ES 1.00 Appendix A's form):
/// - the body starts with instructions, then the head test
///   `If { cond, then: [], else: [Break] }` (lower.rs `break_unless`), `cond`
///   being `i < B` or `i <= B` (int or float, either operand order);
/// - every store to `i` in the body is the step `i = i + C` (one C). One
///   sits at the top level of the body; any other is followed at once by a
///   `Continue` (the front end emits the step before each `continue`), so a
///   pass runs exactly one step. None sits in a nested loop;
/// - `i`'s value when the loop is entered is known: `init_of(i, float)` gives
///   its lane-0 bits (the GC7000's selection reads the store before the loop,
///   [`init_from_preceding`]; the shared unroller its known values).
///
/// The count is simulated exactly: in i32 with every step checked for
/// overflow, or in f32 when A, B and C are whole numbers of at most 2^24
/// (where f32 arithmetic is exact). A loop that is still going after
/// [`EXACT_PASSES_MAX`] passes is not recognized. A shape missed here is not
/// wrong: the loop gets the cap (UNIFORM_LOOP_CAP) instead of its own count.
pub fn constant_loop_entries(
    m: &ir::Module<'_>,
    body: ir::Range,
    init_of: &dyn Fn(ir::LocalId, bool) -> Option<u32>,
) -> Option<u32> {
    let nodes = m.nodes_of(body);
    /* the head test: leading instructions, then If { cond, [], [Break] } */
    let head = nodes.iter().position(|n| !matches!(n, ir::Node::Instr(_)))?;
    let ir::Node::If { cond, then_, else_ } = nodes[head] else { return None };
    if then_.len != 0 || m.nodes_of(else_) != [ir::Node::Break] {
        return None;
    }
    let ir::Op::Alu(op, srcs) = *m.def(cond.val)? else { return None };
    let (float, le) = match op {
        ir::AluOp::Ilt => (false, false),
        ir::AluOp::Ile => (false, true),
        ir::AluOp::Flt => (true, false),
        ir::AluOp::Fle => (true, true),
        _ => return None,
    };
    let lane = |src: ir::Src| src.swizzle[cond.swizzle[0] as usize & 3] as usize & 3;
    let local_of = |src: ir::Src| match m.def(src.val) {
        Some(ir::Op::LoadLocal { local, index: None, .. }) => Some(*local),
        _ => None,
    };
    let const_of = |src: ir::Src, k: usize| match m.def(src.val) {
        Some(ir::Op::ConstI32(v)) if !float => Some(v[k] as u32),
        Some(ir::Op::ConstF32(v)) if float => Some(v[k]),
        _ => None,
    };
    /* i OP B, or B OP i (then the index is on the right) */
    let (i, bound, index_left) = match (local_of(srcs[0]), local_of(srcs[1])) {
        (Some(l), None) => (l, const_of(srcs[1], lane(srcs[1]))?, true),
        (None, Some(l)) => (l, const_of(srcs[0], lane(srcs[0]))?, false),
        _ => return None,
    };
    /* the steps */
    let mut step = None;
    let mut top_steps = 0;
    if !steps_ok(m, nodes, i, float, &mut step, true, &mut top_steps) || top_steps != 1 {
        return None;
    }
    let step = step?;
    let init = init_of(i, float)?;
    simulate_entries(float, le, index_left, init, bound, step)
}

/// The steps of the index `i` in `nodes` (see [`constant_loop_entries`]):
/// false when a store to `i` is anything else. `top` is the loop body's own
/// node list, where the one ordinary step sits.
fn steps_ok(
    m: &ir::Module<'_>,
    nodes: &[ir::Node],
    i: ir::LocalId,
    float: bool,
    step: &mut Option<u32>,
    top: bool,
    top_steps: &mut u32,
) -> bool {
    for (k, n) in nodes.iter().enumerate() {
        match *n {
            ir::Node::Instr(id) => {
                let Some(ir::Op::StoreLocal { local, index, src, write_mask }) = m.instr(id) else { continue };
                if *local != i {
                    continue;
                }
                if index.is_some() || write_mask & 1 == 0 {
                    return false;
                }
                let Some(c) = step_constant(m, *src, i, float) else { return false };
                if step.is_some_and(|s| s != c) {
                    return false;
                }
                *step = Some(c);
                if matches!(nodes.get(k + 1), Some(ir::Node::Continue)) {
                    continue;
                }
                if !top {
                    return false;
                }
                *top_steps += 1;
            }
            ir::Node::If { then_, else_, .. } => {
                if !steps_ok(m, m.nodes_of(then_), i, float, step, false, top_steps)
                    || !steps_ok(m, m.nodes_of(else_), i, float, step, false, top_steps)
                {
                    return false;
                }
            }
            ir::Node::Loop { body } if writes_local(m, body, i) => return false,
            _ => {}
        }
    }
    true
}

/// The C of a store `i = i + C` (either operand order), or `None`.
fn step_constant(m: &ir::Module<'_>, src: ir::Src, i: ir::LocalId, float: bool) -> Option<u32> {
    let ir::Op::Alu(op, srcs) = *m.def(src.val)? else { return None };
    if op != if float { ir::AluOp::Fadd } else { ir::AluOp::Iadd } {
        return None;
    }
    let lane = src.swizzle[0] as usize & 3;
    let is_i = |s: ir::Src| matches!(m.def(s.val), Some(ir::Op::LoadLocal { local, index: None, .. }) if *local == i);
    let c = |s: ir::Src| match m.def(s.val) {
        Some(ir::Op::ConstI32(v)) if !float => Some(v[s.swizzle[lane] as usize & 3] as u32),
        Some(ir::Op::ConstF32(v)) if float => Some(v[s.swizzle[lane] as usize & 3]),
        _ => None,
    };
    if is_i(srcs[0]) {
        c(srcs[1])
    } else if is_i(srcs[1]) {
        c(srcs[0])
    } else {
        None
    }
}

/// Does anything in `r`, at any depth, store to `local`?
fn writes_local(m: &ir::Module<'_>, r: ir::Range, local: ir::LocalId) -> bool {
    m.nodes_of(r).iter().any(|n| match *n {
        ir::Node::Instr(id) => matches!(m.instr(id), Some(ir::Op::StoreLocal { local: l, .. }) if *l == local),
        ir::Node::If { then_, else_, .. } => writes_local(m, then_, local) || writes_local(m, else_, local),
        ir::Node::Loop { body } => writes_local(m, body, local),
        _ => false,
    })
}

/// Run the index from `init` by `step` while `i OP bound` holds (OP `<`, or
/// `<=` with `le`; the index on the right with `!index_left`) and count the
/// head entries, the failing one included; `None` past [`EXACT_PASSES_MAX`] passes, on
/// an overflow, or for a float value arithmetic would round.
fn simulate_entries(float: bool, le: bool, index_left: bool, init: u32, bound: u32, step: u32) -> Option<u32> {
    let test = |a: f64, b: f64| {
        let (x, y) = if index_left { (a, b) } else { (b, a) };
        if le {
            x <= y
        } else {
            x < y
        }
    };
    let (mut i, b, c) = if float {
        let exact = |bits: u32| {
            let v = f32::from_bits(bits);
            (v.is_finite() && v == (v as i64) as f32 && v.abs() <= (1u32 << 24) as f32).then_some(v as f64)
        };
        (exact(init)?, exact(bound)?, exact(step)?)
    } else {
        (init as i32 as f64, bound as i32 as f64, step as i32 as f64)
    };
    let limit = if float { (1u32 << 24) as f64 } else { i32::MAX as f64 };
    /* EXACT_PASSES_MAX passes, and the entry whose test fails */
    for entries in 1..=EXACT_PASSES_MAX + 1 {
        if !test(i, b) {
            return Some(entries);
        }
        i += c;
        if i.abs() > limit || (!float && i < i32::MIN as f64) {
            return None;
        }
    }
    None
}

/// The index's value before the loop from the node list around it: the last
/// store to `i` before the loop is a constant (read as float or int), and
/// nothing between them writes `i`. For [`constant_loop_entries`]'s `init_of`
/// when no known values are at hand (the GC7000's selection).
pub fn init_from_preceding(m: &ir::Module<'_>, before: &[ir::Node], i: ir::LocalId, float: bool) -> Option<u32> {
    for n in before.iter().rev() {
        match *n {
            ir::Node::Instr(id) => match m.instr(id) {
                Some(ir::Op::StoreLocal { local, index: None, src, write_mask }) if *local == i => {
                    if write_mask & 1 == 0 {
                        continue;
                    }
                    return match m.def(src.val) {
                        Some(ir::Op::ConstI32(v)) if !float => Some(v[src.swizzle[0] as usize & 3] as u32),
                        Some(ir::Op::ConstF32(v)) if float => Some(v[src.swizzle[0] as usize & 3]),
                        _ => None,
                    };
                }
                Some(ir::Op::StoreLocal { local, .. }) if *local == i => return None,
                _ => {}
            },
            ir::Node::If { then_, else_, .. } => {
                if writes_local(m, then_, i) || writes_local(m, else_, i) {
                    return None;
                }
            }
            ir::Node::Loop { body } if writes_local(m, body, i) => return None,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one boundary: a constant loop of EXACT_PASSES_MAX passes is
    /// exact (that many passes and the failing entry), one pass more is not
    /// recognized and gets the cap. The GC7000 (select.rs) and the shared
    /// unroller (powervr/unroll.rs) pin the same numbers from GLSL.
    #[test]
    fn a_loop_of_exact_passes_max_is_exact_and_one_more_is_not() {
        let n = EXACT_PASSES_MAX;
        assert_eq!(n, 65_536);
        assert_eq!(simulate_entries(false, false, true, 0, n, 1), Some(n + 1), "{n} passes: exact");
        assert_eq!(simulate_entries(false, false, true, 0, n + 1, 1), None, "{} passes: capped", n + 1);
        assert_eq!(simulate_entries(false, true, true, 1, n, 1), Some(n + 1), "i <= {n} from 1: {n} passes");
        let f = |v: f32| v.to_bits();
        assert_eq!(simulate_entries(true, false, true, f(0.0), f(n as f32), f(1.0)), Some(n + 1), "float, {n} passes");
    }
}
