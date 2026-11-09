// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The run-time index trees of [`super::ilt_trees`] by the shape of the IR
//! (docs/notes/mali-indexed-locals.md sections 3.1-3.3 and 5).

use super::super::MaliError;
use crate::shaderc::online::codegen;
use crate::shaderc::online::ir::{AluOp, Op, Src, ValueId};
use crate::shaderc::online::store;
use crate::shaderc::{SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The IR of `src` after [`super::passes`] (`Err` its refusal), and after
/// [`super::ilt_trees`] alone (the front end's where it finds no ladder).
fn rewrite(stage: u32, src: &str) -> (Result<Vec<Op>, MaliError>, Vec<Op>) {
    let _k = store::test_keep();
    store::with_scratch(|s| {
        let l = codegen::lower_source(stage, src.as_bytes(), &[], s).expect("the front end takes it");
        let passes = super::passes(&l.module, s).map(|r| match r {
            Some((instrs, _)) => instrs.to_vec(),
            None => l.module.instrs.to_vec(),
        });
        let trees = match super::ilt_trees(&l.module, s).expect("the trees") {
            Some(t) => t.instrs.to_vec(),
            None => l.module.instrs.to_vec(),
        };
        (passes, trees)
    })
    .expect("the scratch arena is free")
}

fn op(ir: &[Op], s: Src) -> Op {
    ir[s.val.0 as usize]
}

/// `s` is `ilt(i, mid)` of the index `i` (any value but a constant) and a
/// constant `mid`, in that order (`nir_ilt_imm`), read splat: `i`.
fn ilt(ir: &[Op], s: Src, mid: i32) -> ValueId {
    assert_eq!(s.swizzle, [0; 4], "a compare is read splat");
    let Op::Alu(AluOp::Ilt, [i, k, _]) = op(ir, s) else { panic!("not an ilt: {:?}", op(ir, s)) };
    assert_eq!(op(ir, k), Op::ConstI32([mid; 4]));
    assert!(!matches!(op(ir, i), Op::ConstI32(_)));
    i.val
}

/// The value at `s` is `_nir_select_from_array_helper(x, i, start, end)`:
/// `bcsel(ilt(i, mid), lower, upper)`, `leaf(k, x)` at each element `k`.
fn search(ir: &[Op], s: Src, i: Option<ValueId>, start: i32, end: i32, leaf: &dyn Fn(i32, Src)) -> ValueId {
    if end - start == 1 {
        leaf(start, s);
        return i.expect("a tree of one element");
    }
    let mid = start + (end - start) / 2;
    let Op::Alu(AluOp::Bcsel, [c, lo, hi]) = op(ir, s) else { panic!("not a select: {:?}", op(ir, s)) };
    let at = ilt(ir, c, mid);
    assert!(i.is_none_or(|i| i == at), "one index");
    search(ir, lo, Some(at), start, mid, leaf);
    search(ir, hi, Some(at), mid, end, leaf)
}

/// Element `k` is a load of local `k` (the run's slot).
fn slot_load(ir: &[Op], k: i32, x: Src) {
    assert!(matches!(op(ir, x), Op::LoadLocal { local, index: None, .. } if local.0 == k as u32), "{:?}", op(ir, x));
}

fn no_ieq(ir: &[Op]) -> bool {
    !ir.iter().any(|o| matches!(o, Op::Alu(AluOp::Ieq, _)))
}

/// The front end's read ladders become Mesa's binary search: a run of three
/// (`arr[i]`: `ilt(i, 1)` over `ilt(i, 2)`), a matrix's two columns (`p[i]`,
/// `ilt(i, 1)`), four (`t[i]`: `ilt(i, 2)` over `ilt(i, 1)` and `ilt(i,
/// 3)`), a vector's four lanes (`v[j]`, the same tree; [`super::passes`]
/// still refuses it, `IndexedLocal`, until fs-indexlocal) -- no `ieq`
/// left. A `?:` chain of the source's own compares is no ladder and stays.
#[test]
fn glsl_rewrites_a_read_ladder_into_the_ilt_search() {
    let read = |src: &str, n: i32, leaf: &dyn Fn(&[Op], i32, Src)| {
        let (passes, _) = rewrite(SHC_STAGE_FRAGMENT, src);
        let ir = passes.expect("the passes take it");
        assert!(no_ieq(&ir), "{ir:?}");
        let Some(Op::StoreOutput { src: colour, .. }) = ir.iter().rev().find(|o| matches!(o, Op::StoreOutput { .. }))
        else {
            panic!("no colour")
        };
        let root = match op(&ir, *colour) {
            Op::Alu(AluOp::Bcsel, _) => *colour,
            Op::Alu(_, [x, ..]) => x,
            o => panic!("{o:?}"),
        };
        let i = search(&ir, Src::id(root.val), None, 0, n, &|k, x| leaf(&ir, k, x));
        assert!(matches!(ir[i.0 as usize], Op::LoadUniform { .. }));
    };
    read(
        "precision highp float; uniform int u_i; void main() { float arr[3]; arr[0] = 0.125; arr[1] = 0.25; \
         arr[2] = 0.5; gl_FragColor = vec4(arr[u_i]); }",
        3,
        &slot_load,
    );
    read(
        "precision highp float; uniform int u_i; uniform vec4 u_k; void main() { \
         mat2 p = mat2(u_k.x, u_k.y, u_k.z, u_k.w); gl_FragColor = p[u_i].xyxy; }",
        2,
        &slot_load,
    );
    read(
        "precision highp float; uniform int u_i; uniform vec4 a; void main() { vec4 t[4]; t[0] = a; \
         t[1] = a.yzwx; t[2] = a.zwxy; t[3] = a.wxyz; gl_FragColor = vec4(t[u_i].x); }",
        4,
        &slot_load,
    );

    /* a vector's lane: the tree over the lanes of one value */
    let lanes = "precision highp float; uniform int u_j; uniform vec4 u_k; void main() { vec4 v = u_k * u_k; \
                 gl_FragColor = vec4(v[u_j]); }";
    let (passes, ir) = rewrite(SHC_STAGE_FRAGMENT, lanes);
    assert_eq!(passes, Err(MaliError::IndexedLocal));
    assert!(no_ieq(&ir), "{ir:?}");
    let root = (0..ir.len())
        .rev()
        .find(|&v| matches!(ir[v], Op::Alu(AluOp::Bcsel, _)))
        .expect("the tree's root, its last select");
    let vec = core::cell::Cell::new(None);
    search(&ir, Src::id(ValueId(root as u32)), None, 0, 4, &|k, x| {
        assert_eq!(x.swizzle[0], k as u8);
        assert!(vec.get().is_none_or(|v| v == x.val));
        vec.set(Some(x.val));
    });

    /* a source's own `?:` chain: each compare its own load of `u_i`, made
     * before its arms; the vertex stage keeps its ladders (its walk takes
     * no `ilt`) */
    let own = "precision highp float; uniform int u_i; uniform vec4 a; void main() { \
               gl_FragColor = u_i == 0 ? a : (u_i == 1 ? a.yzwx : a.zwxy); }";
    let (_, ir) = rewrite(SHC_STAGE_FRAGMENT, own);
    assert_eq!(ir.iter().filter(|o| matches!(o, Op::Alu(AluOp::Ieq, _))).count(), 2);
    let vs = "uniform int u_i; attribute vec4 a_p; void main() { vec4 t[3]; t[0] = a_p; t[1] = a_p.yxzw; \
              t[2] = a_p.zyxw; gl_Position = t[u_i]; }";
    let (_, ir) = rewrite(SHC_STAGE_VERTEX, vs);
    assert!(!no_ieq(&ir));
}

/// The front end's write ladders, `bcsel(ieq(i, k), v, old)` into each
/// slot, take the condition of the tree's path to the slot after
/// nir_opt_algebraic.py:2486-2489 (section 3.2): for three slots
/// `ilt(i, 1)`; `iand(ilt(i, 2), inot(ilt(i, 1)))`; `ior(ilt(i, 1),
/// ilt(i, 2))` with the old value first (other_80_matdyn's words 7-15); for
/// a matrix's two columns `ilt(i, 1)` both, the second the old value
/// first. A component written at a run-time index stays an `ieq` lane mask
/// (section 3.3).
#[test]
fn glsl_rewrites_a_write_ladder_into_the_ilt_tree() {
    /* the select each slot of local run 0.. stores, by slot */
    let stores = |ir: &[Op], n: usize| -> Vec<[Src; 3]> {
        (0..n as u32)
            .map(|l| {
                let s = ir
                    .iter()
                    .filter_map(|o| match *o {
                        Op::StoreLocal { local, index: None, src, .. } if local.0 == l => Some(src),
                        _ => None,
                    })
                    .next_back()
                    .expect("the slot's last store");
                let Op::Alu(AluOp::Bcsel, s) = op(ir, s) else { panic!("{:?}", op(ir, s)) };
                s
            })
            .collect()
    };
    let old = |ir: &[Op], x: Src, l: u32| matches!(op(ir, x), Op::LoadLocal { local, index: None, .. } if local.0 == l);
    let stored = |ir: &[Op], x: Src| matches!(op(ir, x), Op::Alu(AluOp::Fmul, _));

    let src = "precision highp float; uniform int u_i; uniform float u_x; void main() { float arr[3]; \
               arr[0] = 0.125; arr[1] = 0.25; arr[2] = 0.5; arr[u_i + 1] = u_x * u_x; \
               gl_FragColor = vec4(arr[0], arr[1], arr[2], 1.0); }";
    let (passes, _) = rewrite(SHC_STAGE_FRAGMENT, src);
    let ir = passes.expect("the passes take it");
    assert!(no_ieq(&ir), "{ir:?}");
    let [s0, s1, s2] = stores(&ir, 3)[..] else { unreachable!() };
    /* arr0' = bcsel(d1, X, a0) */
    let j = ilt(&ir, s0[0], 1);
    assert!(matches!(ir[j.0 as usize], Op::Alu(AluOp::Iadd, _)), "the index `u_i + 1`");
    assert!(stored(&ir, s0[1]) && old(&ir, s0[2], 0));
    let d1 = s0[0].val;
    /* arr1' = bcsel(iand(d2, inot(d1)), X, a1) */
    let Op::Alu(AluOp::Iand, [d2, nd1, _]) = op(&ir, s1[0]) else { panic!("{:?}", op(&ir, s1[0])) };
    assert_eq!(ilt(&ir, d2, 2), j);
    assert_eq!(op(&ir, nd1), Op::Alu(AluOp::Inot, [Src { val: d1, swizzle: [0; 4] }; 3]));
    assert!(stored(&ir, s1[1]) && old(&ir, s1[2], 1));
    /* arr2' = bcsel(ior(d1, d2), a2, X): the old value first */
    let Op::Alu(AluOp::Ior, [a, b, _]) = op(&ir, s2[0]) else { panic!("{:?}", op(&ir, s2[0])) };
    assert_eq!((a.val, b.val), (d1, d2.val));
    assert!(old(&ir, s2[1], 2) && stored(&ir, s2[2]));

    /* a matrix's columns: ilt(i, 1) for both */
    let src = "precision highp float; uniform int u_i; uniform vec4 u_k; void main() { \
               mat2 p = mat2(u_k.x, u_k.y, u_k.z, u_k.w); p[u_i] = u_k.zw * u_k.xy; \
               gl_FragColor = vec4(p[0], p[1]); }";
    let (passes, _) = rewrite(SHC_STAGE_FRAGMENT, src);
    let ir = passes.expect("the passes take it");
    assert!(no_ieq(&ir), "{ir:?}");
    let [c0, c1] = stores(&ir, 2)[..] else { unreachable!() };
    assert_eq!(ilt(&ir, c0[0], 1), ilt(&ir, c1[0], 1));
    assert!(stored(&ir, c0[1]) && old(&ir, c0[2], 0));
    assert!(old(&ir, c1[1], 1) && stored(&ir, c1[2]));

    /* a component at a run-time index: the ieq lane mask stays */
    let src = "precision highp float; uniform int u_i; uniform vec4 u_k; void main() { vec4 v = u_k; \
               v[u_i] = u_k.x * u_k.y; gl_FragColor = v; }";
    let (passes, _) = rewrite(SHC_STAGE_FRAGMENT, src);
    let ir = passes.expect("the passes take it");
    assert!(ir
        .iter()
        .any(|o| matches!(o, Op::Alu(AluOp::Ieq, [_, k, _]) if op(&ir, *k) == Op::ConstI32([0, 1, 2, 3]))));
    assert!(!ir.iter().any(|o| matches!(o, Op::Alu(AluOp::Ilt, _))));
}
