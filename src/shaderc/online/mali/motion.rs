// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2023-2025 Advanced Micro Devices, Inc.
// Derived from Mesa 26.2.1 src/compiler/nir/nir_opt_varyings.c (the
// backward inter-shader code motion) and nir_use_dominance.c (MIT). The
// upstream part stays under its licence (LICENSES/MIT.txt); our changes are
// Apache-2.0. See NOTICE.
//! `nir_opt_varyings`' backward inter-shader code motion
//! (nir_opt_varyings.c:4497-4807), which the GL linker runs on every
//! program (`gl_nir_lower_optimize_varyings`, gl_nir_linker.c:1477-1525):
//! the fragment shader's arithmetic on a varying that commutes with
//! interpolation moves into the vertex shader, and the varying carries the
//! result. femtovg's `ftcoord.x * 2.0 - 1.0` (`strokeMask`) is computed by
//! the vertex shader of every program that reads it (measured:
//! tools/mali/testdata/fvg_fill_color_nodiscard_noscissor.expect.json, the
//! varying program's `FMA.f32 r0, r0, 2.0, -1.0`; tests.rs
//! `femtovg_s_program_moves_fragment_code_into_the_vertex_shader`).
//!
//! The rule, per scalar input load (`backward_inter_shader_code_motion`):
//! walk the load's use post-dominators (`nir_use_dominance.c`: the
//! instruction every use chain passes through) while each is movable
//! (`update_movable_flags`, :3820-4004: every source movable, and an
//! operation on an interpolated value only if it commutes with
//! interpolation, `can_move_alu_across_interp` :3734-3793: `fadd`, `fneg`,
//! or `fmul`/`ffma` by a convergent value; constants are convergent);
//! the farthest movable one is cloned at the end of the producer
//! (`try_move_postdominator`, :4090-4494) and replaced in the consumer by
//! the load it started from, whose slot the result takes. After a move the
//! analysis starts over (:4791-4799).
//!
//! What the references measure beyond the rule's text:
//!
//! * a uniform is not convergent here: `paintMat * vec3(fpos, 1.0)` stays
//!   in the fragment shader (fvg_fill_gradient_nodiscard_noscissor.expect.json,
//!   its `FMA.f32 r1, u2.w0, r2, u2.w1`), so a chain with a uniform stops
//!   (Mesa's `can_move_uniforms`, :2006, is false for these programs);
//! * a post-dominator whose only users are the colour's stores is not moved
//!   (s4_fneg.expect.json: `-v.x` stored stays an `FADD.f32 ... .neg` in the
//!   fragment shader, and the varying program is the plain copy), where
//!   `ftcoord.x * 2.0 - 1.0` feeding `abs` is. The rule as read gives no
//!   such condition; it is pinned by the two references and named here;
//! * a dot product is not moved: the linker runs before the back end's
//!   `nir_lower_alu_width`, so it sees one `fdot2` over vector sources,
//!   which `can_move_alu_across_interp` refuses (its `default:`), where
//!   this graph already holds the lowered `fmul` and `ffma` chain
//!   (nir_fs.rs `alu32`). Measured: `dot(v.xy, vec2(2.0, 3.0)) * v.z`
//!   stays whole in the fragment shader and its vertex shader is the plain
//!   copy (tools/mali/testdata/pan_shc-out/s4_motion_dot.json), while the
//!   same arithmetic written out, `(v.x * 2.0 + v.y * 3.0) * v.z`, moves.
//!   So a node whose IR instruction is a dot is unmovable here;
//! * after a move, a load every use chain of which passed through the
//!   moved post-dominator is dead (`try_move_postdominator`, :4440-4494:
//!   its store is removed and its slot cleared; a load with another use
//!   keeps it, the dead use replaced by an undef). Here the users of a lane
//!   are only the nodes the colour still reaches, so `(v.x + v.y) * v.z`
//!   moves the sum once, into `v.x`'s lane, and `v.y` is not read
//!   (pan_shc-out/s4_motion_sum.json: the vertex shader stores x and z).
//!
//! The moved expression travels in the fragment record ([`Moved`], as bytes
//! in the program's vertex-shader key, backend.rs `FsReads`) to the vertex
//! shader built for the program (nir.rs `build`), which computes it where
//! Mesa's clone stands and stores it in the varying's lane.

use super::nir::{self, Alu, Kind, Nir, Val};
use super::nir_fs::{Fs, TexCoord, FRAG_Z, POINT_COORD};
use super::{Defect, MaliError};
use crate::shaderc::online::ir::Module;

/// The moved expressions one program may carry.
pub const MAX_MOVED: usize = 4;
/// The nodes of one moved expression.
pub const MAX_NODES: usize = 8;
/// One node's bytes in [`encode`].
const NODE_BYTES: usize = 6;
/// The bytes [`encode`] writes at most.
pub const MAX_BYTES: usize = MAX_MOVED * (3 + MAX_NODES * NODE_BYTES);

/// One node of a moved expression: a varying lane (the fragment shader's
/// location and component, which the vertex shader's output of the same
/// name has), a constant, or an operation on earlier nodes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Node {
    Lane { location: u8, comp: u8 },
    Const(u32),
    Op { op: Alu, src: [u8; 3] },
}

/// An expression moved into the vertex shader: its nodes in dependence
/// order, the last the result, stored in lane `comp` of varying
/// `location`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Moved {
    pub location: u8,
    pub comp: u8,
    pub nodes: [Node; MAX_NODES],
    pub n: u8,
}

impl Moved {
    pub const NONE: Moved = Moved { location: 0, comp: 0, nodes: [Node::Const(0); MAX_NODES], n: 0 };

    pub fn nodes(&self) -> &[Node] {
        &self.nodes[..self.n as usize]
    }
}

/// The operations a moved expression holds (the ones that commute with
/// interpolation, `can_move_alu_across_interp`), as [`encode`] numbers
/// them: the index is the byte in every record key, so an operation is only
/// ever appended. [`flag`] marks movable exactly these.
const OPS: [Alu; 4] = [Alu::Fadd, Alu::Fmul, Alu::Fneg, Alu::Ffma];

/// `moved` as the bytes of a record key, into `out`; the bytes written. An
/// operation outside [`OPS`] is a defect (it would otherwise travel as
/// another one): [`move_into_producer`] refuses it before it is recorded.
pub fn encode(moved: &[Moved], out: &mut [u8; MAX_BYTES]) -> Result<usize, MaliError> {
    let mut at = 0;
    for m in moved {
        out[at] = m.location;
        out[at + 1] = m.comp;
        out[at + 2] = m.n;
        at += 3;
        for nd in m.nodes() {
            let b = &mut out[at..at + NODE_BYTES];
            b.fill(0);
            match *nd {
                Node::Lane { location, comp } => {
                    b[0] = 0;
                    b[1] = location;
                    b[2] = comp;
                }
                Node::Const(c) => {
                    b[0] = 1;
                    b[1..5].copy_from_slice(&c.to_le_bytes());
                }
                Node::Op { op, src } => {
                    b[0] = 2;
                    b[1] = OPS.iter().position(|o| *o == op).ok_or(MaliError::Defect(Defect::MovedOp))? as u8;
                    b[2..5].copy_from_slice(&src);
                }
            }
            at += NODE_BYTES;
        }
    }
    Ok(at)
}

/// The expressions [`encode`] wrote, or a defect for bytes it did not (an
/// entry of no node among them: an expression has a result).
pub fn decode(bytes: &[u8], out: &mut [Moved; MAX_MOVED]) -> Result<usize, MaliError> {
    let defect = MaliError::Defect(Defect::IrIndex);
    let mut at = 0;
    let mut n = 0;
    while at < bytes.len() {
        if n >= MAX_MOVED || at + 3 > bytes.len() {
            return Err(defect);
        }
        let mut m = Moved { location: bytes[at], comp: bytes[at + 1], n: bytes[at + 2], ..Moved::NONE };
        at += 3;
        if m.n == 0 || m.n as usize > MAX_NODES || at + m.n as usize * NODE_BYTES > bytes.len() {
            return Err(defect);
        }
        for k in 0..m.n as usize {
            let b = &bytes[at..at + NODE_BYTES];
            m.nodes[k] = match b[0] {
                0 => Node::Lane { location: b[1], comp: b[2] },
                1 => Node::Const(u32::from_le_bytes([b[1], b[2], b[3], b[4]])),
                2 => Node::Op { op: *OPS.get(b[1] as usize).ok_or(defect)?, src: [b[2], b[3], b[4]] },
                _ => return Err(defect),
            };
            at += NODE_BYTES;
        }
        out[n] = m;
        n += 1;
    }
    Ok(n)
}

/// The vertex shader's side (`clone_ssa` at the end of the producer's
/// block, :4398-4420): each expression over the varyings' lanes as the
/// shader computes them, through the graph's own rules and fusion; the
/// result replaces the lane.
pub fn apply(vs: &mut Nir<'_>, varyings: &mut [Option<[Val; 4]>], moved: &[Moved]) -> Result<(), MaliError> {
    for m in moved {
        let mut vals = [Val::Undef; MAX_NODES];
        for (k, nd) in m.nodes().iter().enumerate() {
            vals[k] = match *nd {
                Node::Lane { location, comp } => varyings
                    .get(location as usize)
                    .copied()
                    .flatten()
                    .map(|l| l[comp as usize & 3])
                    .ok_or(MaliError::VaryingForm)?,
                Node::Const(c) => Val::Const(c),
                Node::Op { op, src } => {
                    let s = |i: usize| vals.get(src[i] as usize).copied().unwrap_or(Val::Undef);
                    vs.alu(op, s(0), s(1), s(2))?
                }
            };
        }
        let r = vals.get(m.n as usize - 1).copied().ok_or(MaliError::Defect(Defect::IrIndex))?;
        let lanes = varyings.get_mut(m.location as usize).and_then(|l| l.as_mut()).ok_or(MaliError::VaryingForm)?;
        lanes[m.comp as usize & 3] = r;
    }
    Ok(())
}

/// The root of the use post-dominator tree (`dom_nodes[0]`, which
/// post-dominates what no instruction does).
const ROOT: u16 = u16::MAX;
/// The colour's stores, as one node every colour lane is used by: a store
/// has no destination, so its post-dominator is the root.
const SINK: u16 = u16::MAX - 1;

/// Movability (`FLAG_MOVABLE`, `FLAG_UNMOVABLE`) and interpolation
/// (`FLAG_INTERP_CONVERGENT`, else the varyings' interpolation) of a value.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flag {
    Unmovable,
    Convergent,
    Interpolated,
}

struct Dom<'a> {
    nir: &'a Nir<'a>,
    colour: [Val; 4],
    /// Each node's position in the order, the key the intersection walks
    /// by (post-dominance: later is nearer the root).
    pos: [u16; nir::MAX_NODES],
    /// Each node's immediate use post-dominator.
    ipdom: [u16; nir::MAX_NODES],
    /// The nodes the colour still reaches: a node a move left behind
    /// (module text: Mesa removes the loads it post-dominated and undefs
    /// the uses it left) is no user.
    live: [bool; nir::MAX_NODES],
}

impl Dom<'_> {
    fn users(&self, n: u16, c: u8) -> impl Iterator<Item = u16> + '_ {
        let order = &self.nir.order[..self.nir.len];
        let alu = order.iter().copied().filter(move |&u| {
            let nd = self.nir.nodes[u as usize];
            let arity = match nd.kind {
                Kind::Alu(op) => op.arity(),
                Kind::Phi => 2,
                Kind::Discard => 1,
                _ => return false,
            };
            self.live[u as usize] && nd.src[..arity].contains(&Val::Lane(n, c))
        });
        let sink = self.colour.contains(&Val::Lane(n, c)).then_some(SINK);
        alu.chain(sink)
    }

    fn position(&self, n: u16) -> u32 {
        match n {
            ROOT => u32::MAX,
            SINK => u32::MAX - 1,
            _ => self.pos[n as usize] as u32,
        }
    }

    fn up(&self, n: u16) -> u16 {
        match n {
            ROOT => ROOT,
            SINK => ROOT,
            _ => self.ipdom[n as usize],
        }
    }

    /// `intersect` (nir_use_dominance.c:139-155).
    fn intersect(&self, mut a: u16, mut b: u16) -> u16 {
        while a != b {
            while self.position(a) < self.position(b) {
                a = self.up(a);
            }
            while self.position(b) < self.position(a) {
                b = self.up(b);
            }
        }
        a
    }

    /// The immediate post-dominator of the uses of lane `c` of node `n`
    /// (`calc_dominance`, :170-219: the intersection over the users; the
    /// root when there is none).
    fn of_uses(&self, n: u16, c: u8) -> u16 {
        let mut idom = None;
        for u in self.users(n, c) {
            idom = Some(match idom {
                None => u,
                Some(i) => self.intersect(u, i),
            });
        }
        idom.unwrap_or(ROOT)
    }
}

/// The post-dominators of every ALU node (`nir_calc_use_dominance_impl`
/// with post-dominance: the instructions in reverse order, each user
/// settled before its definition, so one pass converges).
fn dominance<'a>(nir: &'a Nir<'a>, colour: [Val; 4]) -> Dom<'a> {
    let mut d =
        Dom { nir, colour, pos: [0; nir::MAX_NODES], ipdom: [ROOT; nir::MAX_NODES], live: [false; nir::MAX_NODES] };
    for (k, &id) in nir.order[..nir.len].iter().enumerate() {
        d.pos[id as usize] = k as u16;
    }
    /* what the colour, the `if`s' conditions and the discards' reach
     * (`nir_opt_dce`'s mark, a source before its user in the order, so one
     * pass from the end; a discard is a side effect, so the compare it
     * tests is live, and Mesa's link moves what feeds it: s6_discloop's
     * `v_uv.x + v_uv.y`, the sum its varying program stores) */
    let conds = nir.live_ifs().map(|r| r.cond);
    let discards = nir.discards().map(|id| nir.nodes[id as usize].src[0]);
    for l in colour.into_iter().chain(conds).chain(discards) {
        if let Val::Lane(n, _) = l {
            d.live[n as usize] = true;
        }
    }
    for k in (0..nir.len).rev() {
        let id = nir.order[k];
        if !d.live[id as usize] {
            continue;
        }
        for s in nir.nodes[id as usize].src {
            if let Val::Lane(n, _) = s {
                d.live[n as usize] = true;
            }
        }
    }
    for k in (0..nir.len).rev() {
        let id = nir.order[k];
        if matches!(nir.nodes[id as usize].kind, Kind::Alu(_) | Kind::Phi) {
            d.ipdom[id as usize] = d.of_uses(id, 0);
        }
    }
    d
}

/// `update_movable_flags` for a value: a constant is movable and
/// convergent; a varying's lane movable and interpolated; gl_PointCoord,
/// gl_FragCoord.z, a uniform (measured, above) and a look-up are not; an
/// operation is movable when its sources are and, on an interpolated
/// value, when it commutes with interpolation; a dot product's lowered
/// chain is not (module text: the linker sees the `fdot`).
fn flag(f: &Fs<'_>, m: &Module<'_>, v: Val, depth: u8) -> Flag {
    let nir = &f.nir;
    let Val::Lane(n, _) = v else {
        return if matches!(v, Val::Const(_)) { Flag::Convergent } else { Flag::Unmovable };
    };
    if depth > MAX_NODES as u8 {
        return Flag::Unmovable;
    }
    let nd = nir.nodes[n as usize];
    match nd.kind {
        Kind::Input { loc, .. } => {
            /* a special varying is no output of the vertex shader */
            if loc == POINT_COORD || loc == FRAG_Z {
                Flag::Unmovable
            } else {
                Flag::Interpolated
            }
        }
        Kind::Alu(_) if f.is_dot_node(m, n) => Flag::Unmovable,
        /* an operation inside an `if`: no reference moves one (every
         * femtovg chain the link moves stands before the scissor `if`) */
        Kind::Alu(_) if nir.block[n as usize] != 0 => Flag::Unmovable,
        Kind::Alu(op) => {
            let mut flags = [Flag::Convergent; 3];
            for (k, fl) in flags.iter_mut().enumerate().take(op.arity()) {
                *fl = flag(f, m, nd.src[k], depth + 1);
                if *fl == Flag::Unmovable {
                    return Flag::Unmovable;
                }
            }
            if flags[..op.arity()].iter().all(|f| *f == Flag::Convergent) {
                return Flag::Convergent;
            }
            /* `can_move_alu_across_interp` (every arm an [`OPS`] member) */
            let ok = match op {
                Alu::Fadd | Alu::Fneg => true,
                Alu::Fmul | Alu::Ffma => flags[0] == Flag::Convergent || flags[1] == Flag::Convergent,
                _ => false,
            };
            if ok {
                Flag::Interpolated
            } else {
                Flag::Unmovable
            }
        }
        _ => Flag::Unmovable,
    }
}

/// The nodes under `root` in dependence order, as a [`Moved`] (the clone
/// `clone_ssa` makes); None when it does not fit.
fn expression(nir: &Nir<'_>, root: u16, location: u8, comp: u8) -> Option<Moved> {
    let mut m = Moved { location, comp, ..Moved::NONE };
    fn visit(nir: &Nir<'_>, v: Val, m: &mut Moved) -> Option<u8> {
        let nd = match v {
            Val::Const(c) => Node::Const(c),
            Val::Lane(n, c) => match nir.nodes[n as usize].kind {
                Kind::Input { loc, .. } => Node::Lane { location: loc, comp: c },
                Kind::Alu(op) => {
                    let mut src = [0u8; 3];
                    for (k, s) in src.iter_mut().enumerate().take(op.arity()) {
                        *s = visit(nir, nir.nodes[n as usize].src[k], m)?;
                    }
                    Node::Op { op, src }
                }
                _ => return None,
            },
            _ => return None,
        };
        if let Some(k) = m.nodes().iter().position(|x| *x == nd) {
            return Some(k as u8);
        }
        if m.n as usize >= MAX_NODES {
            return None;
        }
        m.nodes[m.n as usize] = nd;
        m.n += 1;
        Some(m.n - 1)
    }
    visit(nir, Val::Lane(root, 0), &mut m).map(|_| m)
}

/// The fragment shader's side: the moves, each applied to the graph (the
/// post-dominator's users read the varying's lane) and recorded in
/// `f.moved`, until none is left. `m` is the shader's IR (which nodes are a
/// dot product's).
pub fn move_into_producer(f: &mut Fs<'_>, m: &Module<'_>) -> Result<(), MaliError> {
    let colour = f.colour.ok_or(MaliError::NoColour)?;
    while let Some((n, c, root)) = find(f, m, colour)? {
        let loc = match f.nir.nodes[n as usize].kind {
            Kind::Input { loc, .. } => loc,
            _ => return Err(MaliError::Defect(Defect::IrIndex)),
        };
        let mv = expression(&f.nir, root, loc, c).ok_or(MaliError::MovedTooLarge)?;
        if f.nmoved >= MAX_MOVED {
            return Err(MaliError::MovedTooLarge);
        }
        /* what [`flag`] let through is what [`encode`] numbers: anything
         * else is this module's defect, refused here, never recorded */
        if mv.nodes().iter().any(|nd| matches!(nd, Node::Op { op, .. } if !OPS.contains(op))) {
            return Err(MaliError::Defect(Defect::MovedOp));
        }
        f.moved[f.nmoved] = mv;
        f.nmoved += 1;
        /* `nir_def_rewrite_uses(postdom_def, new_input)` */
        for k in 0..f.nir.len {
            let id = f.nir.order[k] as usize;
            for s in f.nir.nodes[id].src.iter_mut() {
                if *s == Val::Lane(root, 0) {
                    *s = Val::Lane(n, c);
                }
            }
        }
        let mut col = colour;
        for l in col.iter_mut() {
            if *l == Val::Lane(root, 0) {
                *l = Val::Lane(n, c);
            }
        }
        f.colour = Some(col);
    }
    Ok(())
}

/// One move: the first input lane (in slot order) with a movable farthest
/// post-dominator, and that post-dominator.
fn find(f: &Fs<'_>, m: &Module<'_>, colour: [Val; 4]) -> Result<Option<(u16, u8, u16)>, MaliError> {
    let colour = f.colour.unwrap_or(colour);
    let d = dominance(&f.nir, colour);
    let mut inputs: [Option<u16>; super::select::MAX_VARYINGS] = [None; super::select::MAX_VARYINGS];
    for &id in &f.nir.order[..f.nir.len] {
        if let Kind::Input { loc, .. } = f.nir.nodes[id as usize].kind {
            if let Some(slot) = inputs.get_mut(loc as usize) {
                *slot = Some(id);
            }
        }
    }
    /* the look-up's coordinate lanes have the look-up for a user, which
     * no post-dominator of the lane's other users can move past (the
     * farthest movable one is none): fill_color.glyph's stroke term,
     * `ftcoord.x * 2.0 - 1.0`, stays in the fragment shader (its
     * reference's varying program has no FMA) where fill_color's moves */
    let coord = f.look_up().and_then(|t| match t.coord {
        TexCoord::Varying(loc) => Some(loc),
        TexCoord::Const(..) | TexCoord::Computed => None,
    });
    for n in inputs.into_iter().flatten() {
        let loc = match f.nir.nodes[n as usize].kind {
            Kind::Input { loc, .. } => loc,
            _ => continue,
        };
        for c in 0..4u8 {
            if d.users(n, c).next().is_none() || (coord == Some(loc) && c < 2) {
                continue;
            }
            let mut iter = d.of_uses(n, c);
            let mut postdom = None;
            while iter != ROOT && iter != SINK {
                if flag(f, m, Val::Lane(iter, 0), 0) == Flag::Unmovable {
                    break;
                }
                postdom = Some(iter);
                iter = d.up(iter);
            }
            let Some(pd) = postdom else { continue };
            /* measured (module text): not when the colour's stores are
             * its only users */
            if d.up(pd) == SINK {
                continue;
            }
            return Ok(Some((n, c, pd)));
        }
    }
    Ok(None)
}
