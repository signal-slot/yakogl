// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `select71::select71` (the five rules of `docs/notes/v3d71-backend.md`,
//! "select71.rs") over the ES 1.00 corpus the 4.2 back end is gated on
//! (`tests/v3d_online_corpus.rs`): every `X.vert` + `X.frag` pair of
//! `tools/shaders/` whose two sources the 4.2 compiler accepts.
//!
//! Each pair's fragment, vertex and coordinate shader is taken through the
//! front half the way `chain::compile_program_keyed` does (unroll, prepare,
//! select at `Pressure::None`, which does not depend on the register file),
//! then lowered. The invariants of every `Selected71` that lowers are checked
//! below. A pair lowers when its three stages do. The count of pairs that
//! lower, the front-half refusals and the 7.1 refusals (by stage and pair)
//! are pinned: a change in either is a decision, not a drift.
//!
//! The shader-record facts (`fragment`, `vattr_sizes`, `vpm_output_rows`,
//! `uses_vid`, `uses_iid`, `uses_center_w`, `writes_z`, `disable_ez`) stay in
//! `Selected`; `Selected71` carries only `fragment` and `vregs`. What is
//! checked is that `select71` took `Selected` by reference and left every one
//! of them as it was (a copy before and after), and that `fragment` and the
//! register count follow it.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_select_corpus -- --nocapture
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::ir::IoKind;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::v3d::isa::AddOp;
use yakogl::shaderc::online::v3d::prepare::{fragment_inputs, prepare_fragment_with, prepare_vertex_with, SOp};
use yakogl::shaderc::online::v3d::select::{select_fragment, select_vertex, Operand, Pressure, Selected, VOp};
use yakogl::shaderc::online::v3d::select71::{select71, Select71Error};
use yakogl::shaderc::online::v3d::vop71::{AddOp71, Op71, Selected71, Src71};
use yakogl::shaderc::online::{lower, store};

/// Pairs both of whose sources the 4.2 compiler takes (`tests/v3d_online_corpus.rs`
/// links the same population).
const PAIRS: usize = 33;
/// Pairs whose fragment, vertex and coordinate shader all lower.
const LOWERED_PAIRS: usize = 31;
/// Front-half failures (not select71's): `<pair> <stage>: <error>`.
const FRONT_REFUSALS: &[&str] =
    &["es100/loop_nested fs: VaryingWithoutVertexOutput", "es100/ret_nested fs: VaryingWithoutVertexOutput"];
/// select71's refusals: `<pair> <stage>: <error>`.
const REFUSALS: &[&str] = &[];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("tools/shaders is readable") {
        let p = e.expect("a directory entry").path();
        if p.is_dir() {
            sources(&p, out);
        } else if matches!(p.extension().and_then(|x| x.to_str()), Some("vert" | "frag")) {
            out.push(p);
        }
    }
}

/// Source paths (relative to `tools/shaders`, sorted) the 4.2 compiler accepts.
fn compiling_sources(root: &Path) -> BTreeMap<String, String> {
    use core::ffi::c_char;
    use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
    let mut files = Vec::new();
    sources(root, &mut files);
    files.sort();
    assert_eq!(files.len(), 324, "tools/shaders holds 324 .vert/.frag sources");
    let mut ok = BTreeMap::new();
    for p in files {
        let rel = p.strip_prefix(root).expect("under the root").to_string_lossy().into_owned();
        let stage = if rel.ends_with(".vert") { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        let src = std::fs::read_to_string(&p).expect("a UTF-8 source");
        yakogl::shaderc::online::backend::forget_compiled();
        let mut out = ShcShader::ZERO;
        let mut log = [0 as c_char; 1024];
        let z = format!("{src}\0");
        let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
        // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
        let rc = unsafe { f(stage, z.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
        if rc == 0 {
            ok.insert(rel, src);
        }
    }
    ok
}

/// What select71 did to one stage.
enum Outcome {
    Lowered,
    Front(String),
    Refused(Select71Error),
}

fn count(l: &Selected71<'_>, f: impl Fn(&Op71) -> bool) -> usize {
    l.ops.iter().filter(|o| f(o)).count()
}

/// The invariants of one lowered stage; `sel` is the 4.2 selection and
/// `before` the copy taken before the call.
fn check(sel: &Selected<'_>, before: &Selected<'_>, l: &Selected71<'_>, what: &str) {
    /* the shader-record facts are as selected */
    assert_eq!(l.fragment, sel.fragment, "{what}");
    assert_eq!(sel.fragment, before.fragment, "{what}");
    assert_eq!(sel.vregs, before.vregs, "{what}");
    assert_eq!(sel.vattr_sizes, before.vattr_sizes, "{what}");
    assert_eq!(sel.vpm_output_rows, before.vpm_output_rows, "{what}");
    assert_eq!(
        (sel.uses_vid, sel.uses_iid, sel.uses_center_w, sel.writes_z, sel.disable_ez),
        (before.uses_vid, before.uses_iid, before.uses_center_w, before.writes_z, before.disable_ez),
        "{what}"
    );
    assert!(sel.ops == before.ops, "{what}: select71 changed the 4.2 operations");

    /* register count: the extra registers are rule 5's, one ldunif each */
    let ldunif42 = sel.ops.iter().filter(|o| matches!(o, VOp::Ldunif { .. })).count();
    let rule5 = count(l, |o| matches!(o, Op71::Ldunif { .. })) - ldunif42;
    assert_eq!(l.vregs as usize, sel.vregs as usize + rule5, "{what}: vregs");

    /* rule 1: r5 became LdvaryC, one for one, after an ldvary */
    let r5 = sel.ops.iter().flat_map(|o| o.uses()).filter(|o| matches!(o, Operand::R5)).count();
    let mut seen_ldvary = false;
    let mut vary_c = 0;
    let mut payload = [0usize; 4];
    for op in l.ops {
        if matches!(op, Op71::Ldvary { .. }) {
            seen_ldvary = true;
        }
        let mut smalls = 0;
        for s in op.uses() {
            match s {
                Src71::LdvaryC => {
                    vary_c += 1;
                    assert!(seen_ldvary, "{what}: LdvaryC before any ldvary");
                }
                /* rule 2: W is rf3, centroid W rf1, Z rf2 */
                Src71::Payload(n) => {
                    assert!((1..=3).contains(&n), "{what}: payload {n}");
                    payload[n as usize] += 1;
                }
                Src71::SmallImm(_) => smalls += 1,
                _ => {}
            }
        }
        /* rule 5 */
        assert!(smalls <= 1, "{what}: {op:?} reads {smalls} small immediates");
    }
    assert_eq!(vary_c, r5, "{what}: LdvaryC uses vs r5 uses");
    let mut want = [0usize; 4];
    for o in sel.ops.iter().flat_map(|o| o.uses()) {
        if let Operand::Payload(n) = o {
            want[[3, 1, 2][n as usize]] += 1;
        }
    }
    assert_eq!(payload, want, "{what}: payload uses");

    let ftoin = count(l, |o| matches!(o, Op71::Add { op: AddOp71::Ftoin, .. }));
    /* a shader's own round-to-nearest conversions stay */
    let ftoin42 = sel.ops.iter().filter(|o| matches!(o, VOp::Add { op: AddOp::Ftoin, .. })).count();
    if sel.fragment {
        assert_eq!(ftoin, ftoin42, "{what}");
    } else {
        /* rule 3: the two viewport pairs are two ftoin; no scaled chain left */
        assert_eq!(ftoin, ftoin42 + 2, "{what}: ftoin");
        let def = |v| l.ops.iter().find(|o| o.def() == Some(v));
        for op in l.ops {
            if let Op71::Add { op: AddOp71::Ftoiz, a: Src71::V(v), .. } = op {
                assert!(
                    !matches!(def(*v), Some(Op71::Add { op: AddOp71::Ffloor, .. })),
                    "{what}: an ffloor -> ftoiz pair survives"
                );
            }
        }
        /* rule 4: the final vpmwt only (a shader's own are kept) */
        let vpmwt42 = sel.ops.iter().filter(|o| matches!(o, VOp::Add { op: AddOp::Vpmwt, .. })).count();
        assert_eq!(count(l, |o| matches!(o, Op71::Add { op: AddOp71::Vpmwt, .. })), vpmwt42 - 1, "{what}: vpmwt");
        assert!(!matches!(l.ops.last(), Some(Op71::Add { op: AddOp71::Vpmwt, .. })), "{what}: the closing vpmwt");
    }
}

fn lower_stage(sel: &Selected<'_>, a: &Arena<'_>, what: &str) -> Outcome {
    let before = *sel;
    match select71(sel, a) {
        Ok(l) => {
            check(sel, &before, &l, what);
            Outcome::Lowered
        }
        Err(e) => Outcome::Refused(e),
    }
}

/// Fragment, vertex and coordinate shader of one pair: `(stage, outcome)`.
fn pair(vsrc: &str, fsrc: &str, key: &str) -> Vec<(&'static str, Outcome)> {
    store::with_scratch(|a| {
        let mut out = Vec::new();
        macro_rules! front {
            ($e:expr, $what:expr) => {
                match $e {
                    Ok(v) => v,
                    Err(e) => {
                        out.push(($what, Outcome::Front(format!("{e:?}"))));
                        return out;
                    }
                }
            };
        }
        let v = front!(lower::compile_to_ir(a, Stage::Vertex, a.alloc_str(vsrc).unwrap()).map_err(|e| e.msg), "vs");
        let f = front!(lower::compile_to_ir(a, Stage::Fragment, a.alloc_str(fsrc).unwrap()).map_err(|e| e.msg), "fs");
        let (vm, vtrips) = front!(
            match yakogl::shaderc::online::powervr::unroll::unroll(&v.module, a) {
                Ok(Some(u)) => Ok((u.module, u.trips)),
                Ok(None) => Ok((&v.module, &[][..])),
                Err(e) => Err(e.message()),
            },
            "vs"
        );
        let (fm, ftrips) = front!(
            match yakogl::shaderc::online::powervr::unroll::unroll(&f.module, a) {
                Ok(Some(u)) => Ok((u.module, u.trips)),
                Ok(None) => Ok((&f.module, &[][..])),
                Err(e) => Err(e.message()),
            },
            "fs"
        );
        let fsp = front!(prepare_fragment_with(fm, ftrips, 0, a), "fs");
        let inputs = front!(fragment_inputs(&fsp, a), "fs");
        let mut flat = 0u64;
        let mut centroid = 0u64;
        let mut wanted = Vec::new();
        for (i, &(loc, comp)) in inputs.iter().enumerate() {
            let Some(d) = fm.inputs.iter().find(|d| d.kind == IoKind::Varying && d.location == loc) else {
                out.push(("fs", Outcome::Front("VaryingWithoutVertexOutput".into())));
                return out;
            };
            if d.interp == yakogl::shaderc::online::ir::INTERP_FLAT {
                flat |= 1 << i;
            }
            if d.interp == yakogl::shaderc::online::ir::INTERP_CENTROID {
                centroid |= 1 << i;
            }
            let Some(o) = vm.outputs.iter().find(|o| o.kind == IoKind::Varying && o.name == d.name) else {
                out.push(("fs", Outcome::Front("VaryingWithoutVertexOutput".into())));
                return out;
            };
            wanted.push((o.location, comp));
        }
        let sel = front!(select_fragment(&fsp, inputs, flat, centroid, Pressure::None, a), "fs");
        out.push(("fs", lower_stage(&sel, a, &format!("{key} fs"))));
        let vsp = front!(prepare_vertex_with(vm, &wanted, vtrips, 0, a), "vs");
        let csp = front!(prepare_vertex_with(vm, &[], vtrips, 0, a), "cs");
        let mut records: Vec<u8> = Vec::new();
        for op in vsp.ops {
            if let SOp::Attr { loc, .. } = *op {
                if !records.contains(&loc) {
                    records.push(loc);
                }
            }
        }
        records.sort_unstable();
        let sel = front!(select_vertex(&vsp, &records, false, 0, Pressure::None, a), "vs");
        out.push(("vs", lower_stage(&sel, a, &format!("{key} vs"))));
        let sel = front!(select_vertex(&csp, &records, true, 0, Pressure::None, a), "cs");
        out.push(("cs", lower_stage(&sel, a, &format!("{key} cs"))));
        out
    })
    .expect("scratch")
}

#[test]
fn select71_lowers_the_es100_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders");
    let ok = compiling_sources(&root);
    let (mut pairs, mut lowered) = (0usize, 0usize);
    let (mut front, mut refused) = (Vec::new(), Vec::new());
    for (v, vsrc) in ok.iter().filter(|(k, _)| k.ends_with(".vert")) {
        let f = format!("{}.frag", v.trim_end_matches(".vert"));
        let Some(fsrc) = ok.get(&f) else { continue };
        pairs += 1;
        let key = v.trim_end_matches(".vert").to_string();
        let r = pair(vsrc, fsrc, &key);
        let mut all = r.len() == 3;
        for (stage, o) in r {
            match o {
                Outcome::Lowered => {}
                Outcome::Front(e) => {
                    all = false;
                    front.push(format!("{key} {stage}: {e}"));
                }
                Outcome::Refused(e) => {
                    all = false;
                    refused.push(format!("{key} {stage}: {e:?}"));
                }
            }
        }
        lowered += all as usize;
    }
    println!("select71: {pairs} pairs, {lowered} lower at 7.1");
    for s in &front {
        println!("  front half: {s}");
    }
    for s in &refused {
        println!("  refused at 7.1: {s}");
    }
    let mut names: BTreeMap<&str, usize> = BTreeMap::new();
    for s in &refused {
        *names.entry(s.split_once(": ").unwrap().1).or_default() += 1;
    }
    for (n, c) in &names {
        println!("  {c} x {n}");
    }
    assert_eq!(pairs, PAIRS, "pairs");
    assert_eq!(lowered, LOWERED_PAIRS, "pairs that lower");
    assert_eq!(front, FRONT_REFUSALS, "front-half refusals");
    assert_eq!(refused, REFUSALS, "select71 refusals");
}
