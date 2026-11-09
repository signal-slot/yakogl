// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The link move's own tests (motion.rs): the record's numbering of the
//! operations case 199 appends, a node count case 199's lane y needs, and
//! the class [`flag`] gives each varying lane (docs/notes/mali-trunc.md
//! section 4.2, steps 1-3).

use super::*;
use crate::shaderc::online::codegen::lower_source;
use crate::shaderc::online::mali::link::VaryingConsts;
use crate::shaderc::online::mali::{nir_fs, FsPrecision};
use crate::shaderc::online::store;
use crate::shaderc::SHC_STAGE_FRAGMENT;

fn op(op: Alu, src: [u8; 3]) -> Node {
    Node::Op { op, src }
}

/// The key bytes of one node `k` of the first expression [`encode`] wrote.
fn node_bytes(bytes: &[u8], k: usize) -> &[u8] {
    &bytes[3 + NODE_BYTES * k..3 + NODE_BYTES * (k + 1)]
}

/// The numbers [`encode`] gives every operation of [`OPS`]: the five of
/// before case 199 keep theirs (fadd 0, fmul 1, fneg 2, ffma 3, u2f32 4,
/// the bytes of every record key so far), the integer ones case 199 moves
/// (fsarith_199_es3trunc) are appended as 5-11; each decodes back.
#[test]
fn the_appended_operations_keep_the_old_numbers_and_round_trip() {
    let numbered = [
        (Alu::Fadd, 0),
        (Alu::Fmul, 1),
        (Alu::Fneg, 2),
        (Alu::Ffma, 3),
        (Alu::U2f, 4),
        (Alu::F2i, 5),
        (Alu::F2u, 6),
        (Alu::I2f, 7),
        (Alu::Ishl, 8),
        (Alu::Ishr, 9),
        (Alu::Ushr, 10),
        (Alu::Isub, 11),
    ];
    assert_eq!(OPS.len(), numbered.len());
    for (a, number) in numbered {
        let mut m = Moved { location: 2, comp: 1, ..Moved::NONE };
        m.nodes[..3].copy_from_slice(&[Node::Lane { location: 2, comp: 1 }, Node::Const(3), op(a, [0, 1, 0])]);
        m.n = 3;
        let mut bytes = [0u8; MAX_BYTES];
        let n = encode(&[m], &mut bytes).expect("every operation of the table has a number");
        assert_eq!(n, 3 + 3 * NODE_BYTES);
        assert_eq!(node_bytes(&bytes, 2)[..5], [2, number, 0, 1, 0], "{a:?}'s number");
        let mut back = [Moved::NONE; MAX_MOVED];
        assert_eq!(decode(&bytes[..n], &mut back), Ok(1));
        assert_eq!(back[0], m, "{a:?}");
    }
    /* the byte past the table is no operation */
    let mut back = [Moved::NONE; MAX_MOVED];
    let defect = Err(MaliError::Defect(Defect::IrIndex));
    assert_eq!(decode(&[1, 0, 1, 2, OPS.len() as u8, 0, 0, 0, 0], &mut back), defect);
}

/// Case 199's lane y, `float((i >> 1) - (i << 3)) / 64.0` of `int(v.y)`
/// (docs/notes/mali-trunc.md section 2.1: ten distinct nodes), is one
/// expression the record holds, beside lane z's seven: both encode, decode
/// and fit [`MAX_BYTES`]; one node more than [`MAX_NODES`] does not decode.
#[test]
fn case_199_s_ten_node_lane_encodes_and_decodes() {
    let lane_y = [
        Node::Lane { location: 0, comp: 1 },
        op(Alu::F2i, [0, 0, 0]),
        Node::Const(1),
        op(Alu::Ishr, [1, 2, 0]),
        Node::Const(3),
        op(Alu::Ishl, [1, 4, 0]),
        op(Alu::Isub, [3, 5, 0]),
        op(Alu::I2f, [6, 0, 0]),
        Node::Const(0x3c80_0000),
        op(Alu::Fmul, [7, 8, 0]),
    ];
    let lane_z = [
        Node::Lane { location: 0, comp: 2 },
        op(Alu::F2u, [0, 0, 0]),
        Node::Const(2),
        op(Alu::Ushr, [1, 2, 0]),
        op(Alu::U2f, [3, 0, 0]),
        Node::Const(0x3c80_0000),
        op(Alu::Fmul, [4, 5, 0]),
    ];
    let mut y = Moved { location: 0, comp: 1, ..Moved::NONE };
    y.nodes[..lane_y.len()].copy_from_slice(&lane_y);
    y.n = lane_y.len() as u8;
    let mut z = Moved { location: 0, comp: 2, ..Moved::NONE };
    z.nodes[..lane_z.len()].copy_from_slice(&lane_z);
    z.n = lane_z.len() as u8;
    assert!(lane_y.len() <= MAX_NODES);
    let mut bytes = [0u8; MAX_BYTES];
    let n = encode(&[y, z], &mut bytes).expect("encodes");
    assert_eq!(n, 2 * 3 + (lane_y.len() + lane_z.len()) * NODE_BYTES);
    let mut back = [Moved::NONE; MAX_MOVED];
    assert_eq!(decode(&bytes[..n], &mut back), Ok(2));
    assert_eq!((back[0], back[1]), (y, z));
    /* every slot full still fits the record */
    let mut full = Moved { location: 1, comp: 3, ..Moved::NONE };
    full.nodes = [Node::Const(7); MAX_NODES];
    full.n = MAX_NODES as u8;
    assert_eq!(encode(&[full; MAX_MOVED], &mut bytes), Ok(MAX_BYTES));
    assert_eq!(decode(&bytes, &mut back), Ok(MAX_MOVED));
    /* a count past the nodes an expression has */
    let mut over = [0u8; 3 + (MAX_NODES + 1) * NODE_BYTES];
    over[2] = MAX_NODES as u8 + 1;
    for k in 0..=MAX_NODES {
        over[3 + k * NODE_BYTES] = 1;
    }
    assert_eq!(decode(&over, &mut back), Err(MaliError::Defect(Defect::IrIndex)));
}

/// A varying's name, a component and its [`flag`].
type LaneFlag = (String, u8, Flag);

/// [`flag`] of every lane of the fragment shader `src`'s varyings, after
/// the front end and the fragment graph (nir_fs.rs `walk`), with the link's
/// uniform-only lanes `consts`; and of the first node of each listed
/// operation.
fn flags_of(src: &str, consts: VaryingConsts, ops: &[Alu]) -> (Vec<LaneFlag>, Vec<(Alu, Flag)>) {
    store::with_scratch(|s| {
        let l = lower_source(SHC_STAGE_FRAGMENT, src.as_bytes(), &[], s).expect("lowers");
        let text = s.alloc_str(src).expect("scratch");
        let prec = FsPrecision { consts, ..FsPrecision::of(&l.module, text, s).expect("precision") };
        let f = nir_fs::walk(&l.module, s, prec).expect("the fragment graph");
        let mut lanes = Vec::new();
        let mut alus = Vec::new();
        for &id in &f.nir.order[..f.nir.len] {
            match f.nir.nodes[id as usize].kind {
                Kind::Input { loc, comps } => {
                    let name = l.module.inputs.iter().find(|d| d.location == loc).map_or("?", |d| d.name);
                    for c in 0..comps {
                        lanes.push((name.to_string(), c, flag(&f, &l.module, Val::Lane(id, c), 0)));
                    }
                }
                Kind::Alu(a) if ops.contains(&a) && !alus.iter().any(|(b, _)| *b == a) => {
                    alus.push((a, flag(&f, &l.module, Val::Lane(id, 0), 0)));
                }
                _ => {}
            }
        }
        (lanes, alus)
    })
    .expect("the scratch arena is free")
}

const LANES_FS: &str = "#version 300 es
precision highp float;
in vec4 v;
flat in vec4 w;
out vec4 o;
void main() {
    o = vec4(v.x, float(int(v.y) >> 1), w.x, v.z);
}
";

fn lane(lanes: &[LaneFlag], name: &str, c: u8) -> Flag {
    lanes.iter().find(|l| l.0 == name && l.1 == c).unwrap_or_else(|| panic!("{name}.{c} is not read")).2
}

/// A smooth-declared lane the vertex shader computes from uniforms alone
/// (the link's [`VaryingConsts::flat`], case 199's `v`) is convergent, and
/// so is an operation over it, any [`OPS`] has (`f2i`, `ishr`, `i2f32`:
/// nir_opt_varyings.c:3888 asks `can_move_alu_across_interp` only of an
/// interpolated value); a `flat`-declared lane stays flat, an
/// attribute-fed one interpolated, and the same integer operations over it
/// unmovable.
#[test]
fn input_lanes_are_flat_convergent_or_interpolated() {
    let mut consts = VaryingConsts::NONE;
    consts.push_flat(location_of(LANES_FS, "v"), 1);
    let ints = [Alu::F2i, Alu::Ishr, Alu::I2f];
    let (lanes, alus) = flags_of(LANES_FS, consts, &ints);
    assert_eq!(lane(&lanes, "v", 0), Flag::Interpolated);
    assert_eq!(lane(&lanes, "v", 1), Flag::Convergent);
    assert_eq!(lane(&lanes, "w", 0), Flag::Flat);
    for a in ints {
        assert!(alus.contains(&(a, Flag::Convergent)), "{a:?} over a convergent lane: {alus:?}");
    }
    /* no uniform-only lane: v.y interpolated, its integer chain unmovable */
    let (lanes, alus) = flags_of(LANES_FS, VaryingConsts::NONE, &ints);
    assert_eq!(lane(&lanes, "v", 1), Flag::Interpolated);
    assert_eq!(lane(&lanes, "w", 0), Flag::Flat);
    for a in ints {
        assert!(alus.contains(&(a, Flag::Unmovable)), "{a:?} over an interpolated lane: {alus:?}");
    }
    /* a flat-declared lane marked uniform-only is still the flat one */
    let mut both = VaryingConsts::NONE;
    both.push_flat(location_of(LANES_FS, "w"), 0);
    let (lanes, _) = flags_of(LANES_FS, both, &[]);
    assert_eq!(lane(&lanes, "w", 0), Flag::Flat);
}

/// The location the front end gives the fragment input `name` of `src`.
fn location_of(src: &str, name: &str) -> u8 {
    store::with_scratch(|s| {
        let l = lower_source(SHC_STAGE_FRAGMENT, src.as_bytes(), &[], s).expect("lowers");
        l.module.inputs.iter().find(|d| d.name == name).expect("declared").location
    })
    .expect("the scratch arena is free")
}
