// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Mali-G610 (Valhall) link, moved verbatim from `shaderc/link.rs`.
//!
//! The Mali pairs the stages through the varying buffer the IDVS varying
//! program writes and the fragment shader reads (`LD_VAR_BUF_IMM` at a byte
//! offset): each stage's Valhall record lays its varyings out
//! (`MaliShader::varyings`, Mesa's `pan_varying_layout`), and a program links
//! only when every fragment input, paired with the vertex output of its name,
//! sits at the same byte offset in both layouts.
//!
//! A fragment shader that declares its varyings in another order than the
//! vertex shader lays them out otherwise; Mesa's link gives the fragment
//! shader the vertex shader's layout (`fs->vs_varying_layout =
//! vs->vs_varying_layout`, pan_shader.c:681-682, docs/notes/
//! mali-frame-refusals.md section 6), and so does this one: the fragment
//! shader is compiled again for the vertex shader's order
//! ([`VaryingOrder`]) and replaces the program's.

use core::ffi::c_char;
use core::fmt::Write;

use crate::shaderc::link::{duplicate_name, reject_duplicate, reject_truncated_names};
use crate::shaderc::ShcShader;
use crate::util::{name_bytes, Bytes, CStrWriter, EINVAL};

/// The Mali's link of two layouts that place one varying at different
/// offsets even with the fragment shader compiled for the vertex shader's
/// order ([`VaryingOrder`]), or where that compile is not made: a fragment
/// shader whose vertex shader is built for it (its reads compacted or code
/// moved, `compile_vs_for_reads`), a fragment location past
/// [`ORDER_LOCATIONS`], or a vertex layout with an output the fragment
/// shader does not read before one it does (Mesa's link removes it, the
/// vertex shader would have to be built for the program).
pub const MALI_LAYOUTS_DIFFER: &str =
    "a Mali link whose stages place a varying at different offsets is not lowered yet";
/// A fragment shader that reads part of a varying: Mesa's link keeps only
/// the components read (`compact_varyings`), so the vertex shader's layout
/// is the program's, built from the fragment shader's reads at its own
/// locations (`compile_vs_linked`, mod.rs `reads_at_vertex_locations`). Left
/// for a vertex shader that was not built for the program's fragment shader.
pub const MALI_PARTIAL_READ: &str =
    "a Mali link whose fragment shader reads part of a varying (the link keeps only those components) is not lowered yet";
/// A fragment input the fragment record loads has no slot in the vertex
/// record's layout. The program's vertex shader is built for what the
/// fragment record loads, so this is the back end's defect, not the
/// shader's.
pub const MALI_NO_VERTEX_SLOT: &str =
    "Mali link: a vertex output the record lists has no slot in the record's layout (a back-end defect)";
/// The fragment shader has code Mesa's link moves into the vertex shader
/// (motion.rs) and the vertex shader was not built for it (the program's
/// vertex shader is built for the fragment shader's record at
/// glLinkProgram, `compile_vs_for_reads`): the pair would compute the
/// varying in neither stage.
pub const MALI_NOT_BUILT_FOR_MOTION: &str =
    "a Mali link whose vertex shader was not built for the fragment shader's moved code is not lowered yet";

/// The fragment locations a [`VaryingOrder`] key holds: one nibble each
/// below the tag.
pub const ORDER_LOCATIONS: usize = 7;
/// The tag of a [`VaryingOrder`] key: bit 31, with bits 28-30 clear.
const ORDER_TAG: u32 = 1 << 31;
/// A nibble that names no vertex rank (sorts last).
const NO_RANK: u8 = 0xf;

/// A fragment shader's varyings in its vertex shader's order: for each
/// fragment location 0 to [`ORDER_LOCATIONS`] - 1, the rank of the vertex
/// output of its name in the vertex shader's generic layout (its slots by
/// offset). The fragment layout (nir_fs.rs `layout`) takes its reads in
/// this order, so Mesa's packing-class sort, stable in the producer's
/// order, places them where the vertex shader did.
///
/// The compile carries it as the variant key (`blend_variant` with
/// [`VaryingOrder::key`]): the Mali never asks for a blend variant (it
/// blends through the blend descriptor, `gl::draw`'s
/// `fixed_function_blend`), so the key space is free, and a key is what
/// the record cache keys a variant by, so the program's fragment shader is
/// a record of its own beside the shader object's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VaryingOrder(pub [u8; ORDER_LOCATIONS]);

impl VaryingOrder {
    /// The variant key: [`ORDER_TAG`] and nibble `l` the rank of fragment
    /// location `l`.
    pub fn key(self) -> u32 {
        self.0.iter().enumerate().fold(ORDER_TAG, |k, (l, &r)| k | u32::from(r & 0xf) << (4 * l))
    }

    /// The order a variant key carries; None for any other key (a blend
    /// key, which the Mali refuses).
    pub fn of_key(key: u32) -> Option<VaryingOrder> {
        if key & 0xf000_0000 != ORDER_TAG {
            return None;
        }
        let mut o = [NO_RANK; ORDER_LOCATIONS];
        for (l, r) in o.iter_mut().enumerate() {
            *r = (key >> (4 * l)) as u8 & 0xf;
        }
        Some(VaryingOrder(o))
    }

    /// The rank of fragment location `loc` ([`NO_RANK`] past the key).
    pub fn rank(&self, loc: u8) -> u8 {
        self.0.get(loc as usize).copied().unwrap_or(NO_RANK)
    }
}

/// The constant lanes one program's vertex shader may propagate.
pub const MAX_CONSTS: usize = 16;

/// The component tag of a flat lane in [`VaryingConsts`]
/// ([`VaryingConsts::push_flat`]).
const FLAT_LANE: u8 = 0x80;

/// The component tag of a lane equal to another in [`VaryingConsts`]
/// ([`VaryingConsts::push_equal`]).
const EQUAL_LANE: u8 = 0x40;

/// A vertex shader's varying lanes that are constants: Mesa's link
/// propagates each into the fragment shader's read of it and removes the
/// lane from the layout (nir_opt_varyings.c:2880-2900,
/// `propagate_uniform_expressions`; case114_f.expect.json: `v_dir =
/// vec3(1.0, ..)`, the fragment shader's `CUBEFACE1.discard r2,
/// 0x3F800000, r0, r1` and its `.v2` load of the two lanes left). The
/// fragment shader is compiled with them (mod.rs `compile_fs_for_vs`),
/// then the vertex shader for its reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VaryingConsts {
    /// (location, component, fp32 bits).
    at: [(u8, u8, u32); MAX_CONSTS],
    n: usize,
    full: bool,
}

impl Default for VaryingConsts {
    fn default() -> Self {
        VaryingConsts::NONE
    }
}

impl VaryingConsts {
    pub const NONE: VaryingConsts = VaryingConsts { at: [(0, 0, 0); MAX_CONSTS], n: 0, full: false };

    /// Lane `comp` of varying `location` is the constant `bits`; past
    /// [`MAX_CONSTS`] the set is [`VaryingConsts::full`].
    pub fn push(&mut self, location: u8, comp: u8, bits: u32) {
        match self.at.get_mut(self.n) {
            Some(e) => {
                *e = (location, comp, bits);
                self.n += 1;
            }
            None => self.full = true,
        }
    }

    /// More constant lanes than [`MAX_CONSTS`]: a program the link step
    /// does not build (mod.rs `vs_constants` refuses it).
    pub fn full(&self) -> bool {
        self.full
    }

    /// The constant of lane `comp` of varying `location`, if it is one.
    pub fn get(&self, location: u8, comp: u8) -> Option<u32> {
        self.as_slice().iter().find(|e| e.0 == location && e.1 == comp).map(|e| e.2)
    }

    /// Lane `comp` of varying `location` is computed from uniforms alone
    /// (no attribute, no look-up): Mesa's link keeps it a varying but
    /// promotes it to flat (nir_opt_varyings.c:4964-5002), so the fragment
    /// shader loads it `.src_flat32` (varyconst_117_uniidxf.expect.json).
    /// Kept beside the constants, its component tagged [`FLAT_LANE`] and
    /// no bits, so that [`VaryingConsts::get`] never finds it and the
    /// record key (backend.rs `LinkConsts::key`) holds it.
    pub fn push_flat(&mut self, location: u8, comp: u8) {
        self.push(location, comp | FLAT_LANE, 0);
    }

    /// Lane `comp` of varying `location` is a flat lane ([`push_flat`]).
    ///
    /// [`push_flat`]: VaryingConsts::push_flat
    pub fn flat(&self, location: u8, comp: u8) -> bool {
        self.as_slice().iter().any(|e| e.0 == location && e.1 == comp | FLAT_LANE)
    }

    /// Lane `comp` of varying `location` stores the value lane `of_comp`
    /// of varying `of_location` stores, the first such lane of the vertex
    /// shader: Mesa's link keeps one of equal lanes and reads the others
    /// from it (`deduplicate_outputs`, nir_opt_varyings.c:3253-3420), so
    /// the fragment shader lays them out as one ([`VaryingConsts::class`];
    /// nir_fs.rs `layout`). Kept beside the constants, its component tagged
    /// [`EQUAL_LANE`] and the other lane in the bits; past [`MAX_CONSTS`] it
    /// is dropped, not refused: the lanes are then laid out apart and the
    /// program's vertex shader refuses them by name (`VaryingDuplicate`).
    pub fn push_equal(&mut self, location: u8, comp: u8, of_location: u8, of_comp: u8) {
        if self.n < MAX_CONSTS {
            self.push(location, comp | EQUAL_LANE, u32::from(of_location) << 8 | u32::from(of_comp));
        }
    }

    /// The first lane of the vertex shader that stores the value lane `comp`
    /// of varying `location` stores ([`push_equal`]): itself, unless an
    /// earlier lane stores it too.
    ///
    /// [`push_equal`]: VaryingConsts::push_equal
    pub fn class(&self, location: u8, comp: u8) -> (u8, u8) {
        let e = self.as_slice().iter().find(|e| e.0 == location && e.1 == comp | EQUAL_LANE);
        e.map_or((location, comp), |e| ((e.2 >> 8) as u8, e.2 as u8))
    }

    pub fn as_slice(&self) -> &[(u8, u8, u32)] {
        &self.at[..self.n]
    }

    /// Lane `comp` of varying `location` is no constant after all
    /// ([`VaryingConsts::push`] undone; its flat or equal tags stay).
    pub fn remove(&mut self, location: u8, comp: u8) {
        if let Some(i) = self.as_slice().iter().position(|e| e.0 == location && e.1 == comp) {
            self.at.copy_within(i + 1..self.n, i);
            self.n -= 1;
        }
    }

    /// These lanes at the varying locations of the program's fragment
    /// shader: `at` gives the fragment location of the varying at a vertex
    /// location, paired by name as Mesa's link pairs the stages, None when
    /// the fragment shader does not declare it. A location is each stage's
    /// own declaration order, so the two stages number one varying
    /// differently as soon as they declare their varyings in another order
    /// or the fragment shader declares fewer (varyconst_names_swap: `a, b`
    /// in the vertex shader, `b, a` in the fragment one).
    ///
    /// A lane of a varying the fragment shader does not declare is left
    /// out: no one reads it, the link removes it before it propagates or
    /// merges anything (`remove_dead_varyings`, nir_opt_varyings.c:2113,
    /// called at :5973, the two other passes at :6037-6041). So a lane
    /// equal to such a lane ([`VaryingConsts::push_equal`]) has lost the
    /// lane it was read from: the first lane of that value the fragment
    /// shader declares takes its place, and the later ones are read from
    /// that one.
    pub fn at_fragment(&self, at: impl Fn(u8) -> Option<u8>) -> FsConsts {
        let mut out = VaryingConsts::NONE;
        /* (a vertex lane the fragment shader does not declare, the first
         * declared lane of its value, at its fragment location) */
        let mut heads = [((0u8, 0u8), (0u8, 0u8)); MAX_CONSTS];
        let mut nheads = 0;
        for &(location, comp, bits) in self.as_slice() {
            let Some(l) = at(location) else { continue };
            if comp & EQUAL_LANE == 0 {
                /* a constant or a flat lane: `comp` holds the flat tag */
                out.push(l, comp, bits);
                continue;
            }
            let (c, of) = (comp & !EQUAL_LANE, ((bits >> 8) as u8, bits as u8));
            let head = match at(of.0) {
                Some(of_l) => Some((of_l, of.1)),
                None => heads[..nheads].iter().find(|h| h.0 == of).map(|h| h.1),
            };
            match (head, heads.get_mut(nheads)) {
                (Some((of_l, of_c)), _) => out.push_equal(l, c, of_l, of_c),
                (None, Some(h)) => {
                    *h = (of, (l, c));
                    nheads += 1;
                }
                /* never: each head is a lane of this table, MAX_CONSTS at most */
                (None, None) => {}
            }
        }
        FsConsts(out)
    }
}

/// [`VaryingConsts`] at the varying locations of the program's fragment
/// shader: what that shader is compiled with (mod.rs `compile_fs_for_vs`)
/// and what its record key holds (backend.rs `LinkConsts::key`). Made by
/// [`VaryingConsts::at_fragment`] alone, so that no vertex location reaches
/// a fragment shader.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FsConsts(VaryingConsts);

impl FsConsts {
    pub const NONE: FsConsts = FsConsts(VaryingConsts::NONE);

    /// No lane of the vertex shader's reaches this fragment shader.
    pub fn is_empty(&self) -> bool {
        self.0.as_slice().is_empty()
    }

    /// [`VaryingConsts::get`], at the fragment shader's `location`.
    pub fn get(&self, location: u8, comp: u8) -> Option<u32> {
        self.0.get(location, comp)
    }

    /// [`VaryingConsts::flat`], at the fragment shader's `location`.
    pub fn flat(&self, location: u8, comp: u8) -> bool {
        self.0.flat(location, comp)
    }

    /// [`VaryingConsts::class`], both lanes at the fragment shader's
    /// locations.
    pub fn class(&self, location: u8, comp: u8) -> (u8, u8) {
        self.0.class(location, comp)
    }

    /// Each lane as (fragment location, tagged component, bits): the record
    /// key's bytes.
    pub fn as_slice(&self) -> &[(u8, u8, u32)] {
        self.0.as_slice()
    }
}

/// The Mali's `struct shc_backend.link` table rule: the null and name checks
/// of [`link_shaders`](crate::shaderc::link::link_shaders), every fragment input fed by the vertex output of its
/// name and at least as wide, both stages Valhall records, and each input
/// the fragment code reads at the vertex shader's offset (an input the
/// record lists and no code reads has no slot: nothing to place); where
/// only the offsets differ, the
/// fragment shader compiled for the vertex shader's order
/// ([`VaryingOrder`]) replaces `*fs` (the old copy released). 0, -EINVAL (a
/// link error) or -ENOTSUP ([`MALI_NO_VERTEX_SLOT`],
/// [`MALI_LAYOUTS_DIFFER`], each with its own line), or the compile's own
/// code and line.
///
/// # Safety
/// As [`link_shaders`](crate::shaderc::link::link_shaders).
pub unsafe fn link_shaders_mali(
    vs: *const ShcShader,
    fs: *mut ShcShader,
    log: *mut c_char,
    log_sz: usize,
    who: &str,
) -> i32 {
    let raw_log = log;
    // SAFETY: the caller passes a null log or one valid for log_sz bytes.
    let mut log = unsafe { CStrWriter::new(log, log_sz) };
    if vs.is_null() || fs.is_null() {
        let _ = write!(log, "{who}: NULL VS/FS in link");
        return -EINVAL;
    }
    // SAFETY: both are non-null and point at valid shc_shader objects.
    let (vs, fs_mut) = unsafe { (&*vs, &mut *fs) };
    for (sh, stage) in [(vs, "vertex"), (&*fs_mut, "fragment")] {
        let rc = reject_truncated_names(sh, &mut log, who, stage);
        if rc != 0 {
            return rc;
        }
        let varyings = &sh.varyings[..(sh.num_varyings as usize).min(sh.varyings.len())];
        if let Some(d) = duplicate_name(varyings, |v| name_bytes(&v.name)) {
            let what = if stage == "vertex" { "vertex outputs" } else { "fragment inputs" };
            return reject_duplicate(&mut log, who, what, d);
        }
    }
    let (at, order) = match pair(vs, fs_mut, &mut log, who) {
        Pairing::Linked => return 0,
        Pairing::Refused(rc) => return rc,
        Pairing::Differ { at, order } => (at, order),
    };
    let differ = |log: &mut CStrWriter, fs: &ShcShader| {
        let _ = write!(log, "{who}: {MALI_LAYOUTS_DIFFER} ('{}')", Bytes(name_bytes(&fs.varyings[at].name)));
        -crate::util::ENOTSUP
    };
    /* the compile goes through the record cache, which holds only shaders
     * this back end published (a code generator's record in a test is not) */
    let backend = &crate::shaderc::online::backend::SHC_ONLINE;
    let (Some(order), Some(variant)) = (order, backend.blend_variant) else {
        return differ(&mut log, fs_mut);
    };
    if !crate::shaderc::online::backend::is_ours(fs_mut) {
        return differ(&mut log, fs_mut);
    }
    let mut built = ShcShader::ZERO;
    // SAFETY: `fs_mut` is a valid shader, `built` a real output, the log as
    // the caller passed it.
    let rc = unsafe { variant(fs_mut, order.key(), &mut built, raw_log, log_sz) };
    if rc != 0 {
        return rc;
    }
    // SAFETY: as above (the log was the compile's; this line replaces it).
    let mut log = unsafe { CStrWriter::new(raw_log, log_sz) };
    match pair(vs, &built, &mut log, who) {
        Pairing::Linked => {
            let old = core::mem::replace(fs_mut, built);
            crate::shaderc::online::backend::release(&old);
            0
        }
        other => {
            crate::shaderc::online::backend::release(&built);
            match other {
                Pairing::Refused(rc) => rc,
                _ => differ(&mut log, fs_mut),
            }
        }
    }
}

/// What [`pair`] found.
enum Pairing {
    /// Every fragment input at its vertex output's offset.
    Linked,
    /// Refused, its line written.
    Refused(i32),
    /// Only offsets differ, fragment varying `at` (its index in the
    /// shader's list) the first; the order that would place the reads as
    /// the vertex shader does, when one can be made.
    Differ { at: usize, order: Option<VaryingOrder> },
}

/// Each fragment input paired with the vertex output of its name.
fn pair(vs: &ShcShader, fs: &ShcShader, log: &mut CStrWriter, who: &str) -> Pairing {
    use crate::shaderc::online::mali::{MaliShader, Section};
    let (Some(vr), Some(fr)) = (MaliShader::of(vs), MaliShader::of(fs)) else {
        let _ = write!(log, "{who}: a shader of this program was not compiled by the in-process compiler");
        return Pairing::Refused(-EINVAL);
    };
    if vr.moved != fr.moved {
        let _ = write!(log, "{who}: {MALI_NOT_BUILT_FOR_MOTION}");
        return Pairing::Refused(-crate::util::ENOTSUP);
    }
    /* a vertex shader built for this fragment shader's reads laid them out
     * from the fragment shader's locations: its layout is not its own */
    let mut orderable = !fr.compacted && fr.moved.is_empty();
    let mut order = [NO_RANK; ORDER_LOCATIONS];
    let mut differ = None;
    let outputs = &vs.varyings[..(vs.num_varyings as usize).min(vs.varyings.len())];
    for (i, v) in fs.varyings[..(fs.num_varyings as usize).min(fs.varyings.len())].iter().enumerate() {
        let o = outputs.iter().find(|o| name_bytes(&o.name) == name_bytes(&v.name));
        let Some(o) = o.filter(|o| o.components >= v.components) else {
            let _ = write!(
                log,
                "{who}: fragment varying '{}{}",
                Bytes(name_bytes(&v.name)),
                if o.is_some() {
                    "' is narrower in the vertex shader"
                } else {
                    "' is not written by the vertex shader"
                }
            );
            return Pairing::Refused(-EINVAL);
        };
        /* a slot of no component holds nothing (a fragment shader compiled
         * with the vertex shader's lanes lists a varying it reads from an
         * equal lane, or folded, with mask 0) */
        let slot = |r: &MaliShader, loc: u8| {
            r.varyings
                .iter()
                .find(|s| s.section == Section::Generic && s.varying == Some(loc) && s.mask != 0)
                .map(|s| (s.offset, s.mask))
        };
        /* the record lists what the shader statically uses; an input no
         * code reads (under `if (false)`, or a constant the link folded)
         * has no slot and nothing to place */
        let Some(b) = slot(fr, v.location) else { continue };
        let a = slot(vr, o.location);
        let why = match a {
            Some(a) if a == b => None,
            None => Some(MALI_NO_VERTEX_SLOT),
            Some((_, a)) if a != b.1 => Some(MALI_PARTIAL_READ),
            Some(_) => {
                differ.get_or_insert(i);
                None
            }
        };
        if let Some(why) = why {
            let _ = write!(log, "{who}: {why} ('{}')", Bytes(name_bytes(&v.name)));
            return Pairing::Refused(-crate::util::ENOTSUP);
        }
        /* the vertex slot's rank: the generic slots before it */
        let rank =
            a.map(|(at, _)| vr.varyings.iter().filter(|s| s.section == Section::Generic && s.offset < at).count());
        match (rank, order.get_mut(v.location as usize)) {
            (Some(r), Some(o)) if r < NO_RANK as usize => *o = r as u8,
            _ => orderable = false,
        }
    }
    match differ {
        None => Pairing::Linked,
        Some(at) => Pairing::Differ { at, order: orderable.then_some(VaryingOrder(order)) },
    }
}

/// [`VaryingConsts::at_fragment`] on tables built by hand: the vertex
/// shader's lanes at the fragment shader's locations.
#[cfg(test)]
mod at_fragment_tests {
    use super::*;

    /// The vertex locations 0..3 are the varyings `a, b, c, d`; the fragment
    /// shader declares `d, b, a`: `c` is not declared.
    fn at(location: u8) -> Option<u8> {
        [Some(2), Some(1), None, Some(0)].get(usize::from(location)).copied().flatten()
    }

    /// A constant and a flat lane move to the fragment location of their
    /// varying's name; a lane of a varying the fragment shader does not
    /// declare is left out.
    #[test]
    fn a_constant_or_flat_lane_moves_to_its_names_location() {
        let mut v = VaryingConsts::NONE;
        v.push(0, 1, 0x3f00_0000);
        v.push_flat(3, 2);
        v.push(2, 0, 0x3f80_0000);
        v.push_flat(2, 1);
        let f = v.at_fragment(at);
        assert_eq!(f.get(2, 1), Some(0x3f00_0000), "`a`: the vertex location 0, the fragment location 2");
        assert!(f.flat(0, 2), "`d`: the vertex location 3, the fragment location 0");
        assert_eq!(f.as_slice().len(), 2, "no lane of `c`: {f:?}");
        assert_eq!((f.get(0, 1), f.flat(2, 2)), (None, false), "nothing is left at the vertex locations");
    }

    /// The same table when both stages number the varyings alike.
    #[test]
    fn equal_locations_keep_the_table() {
        let mut v = VaryingConsts::NONE;
        v.push(0, 1, 7);
        v.push_flat(1, 0);
        v.push_equal(3, 2, 1, 3);
        assert_eq!(v.at_fragment(Some).as_slice(), v.as_slice());
        assert!(VaryingConsts::NONE.at_fragment(Some).is_empty());
    }

    /// A lane equal to a lane of a declared varying is read from that lane
    /// at its fragment location.
    #[test]
    fn an_equal_lane_follows_the_lane_it_is_read_from() {
        let mut v = VaryingConsts::NONE;
        v.push_equal(3, 2, 0, 1);
        let f = v.at_fragment(at);
        assert_eq!(f.class(0, 2), (2, 1), "`d.z` is read from `a.y`");
        assert_eq!(f.class(2, 1), (2, 1), "`a.y` is its own lane");
    }

    /// Lanes equal to a lane of the varying that is not declared: the first
    /// declared one is the lane the later ones are read from, and is no
    /// equal lane itself. An equal lane of the undeclared varying is left
    /// out with it.
    #[test]
    fn equal_lanes_of_an_undeclared_lane_are_read_from_the_first_declared_one() {
        let mut v = VaryingConsts::NONE;
        v.push_equal(2, 3, 2, 0);
        v.push_equal(3, 0, 2, 0);
        v.push_equal(3, 1, 2, 0);
        v.push_equal(1, 2, 2, 1);
        let f = v.at_fragment(at);
        assert_eq!(f.class(0, 0), (0, 0), "`d.x` is the first declared lane of `c.x`'s value");
        assert_eq!(f.class(0, 1), (0, 0), "`d.y` is read from `d.x`");
        assert_eq!(f.class(1, 2), (1, 2), "`b.z` is the only declared lane of `c.y`'s value");
        assert_eq!(f.as_slice().len(), 1, "one equal lane: {f:?}");
    }
}

/// The Mali's link on the Valhall back end's own records (the code
/// generator's, `codegen::compile_linked_blocks`): every outcome of
/// [`link_shaders_mali`] by its line.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::backend::codegen_priv;
    use crate::shaderc::online::mali::{MaliShader, Section, VaryingSlot};
    use crate::shaderc::online::{codegen, store};
    use crate::shaderc::{SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
    use crate::util::cstr_bytes;

    const TRI_VS: &str = include_str!("../../../../tools/mali/testdata/tri.vert");
    const TV_VS: &str = include_str!("../../../../tools/mali/testdata/tri_varying.vert");
    const TV_FS: &str = include_str!("../../../../tools/mali/testdata/tri_varying.frag");
    /// tri_varying.frag with `vcol` declared and not read.
    const UNUSED_FS: &str =
        "precision mediump float; varying vec3 vcol; void main(){ gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0); }";
    /// tri_varying.frag reading `vcol.y` only, at highp: Mesa's link keeps
    /// that one component (at offset 0, nir_fs.rs `layout`; a mediump one
    /// is loaded whole, fsload_v3_one.expect.json).
    const PART_FS: &str =
        "precision highp float; varying vec3 vcol; void main(){ gl_FragColor = vec4(vcol.y, 0.5, 0.25, 1.0); }";
    /// tri.vert with `vcol` declared and not written.
    const UNWRITTEN_VS: &str =
        "attribute vec2 pos; varying vec3 vcol; void main(){ gl_Position = vec4(pos, 0.0, 1.0); }";

    /// One stage into keep (the caller holds `store::test_keep`).
    fn stage(stage: u32, src: &str) -> ShcShader {
        store::with_scratch(|s| {
            codegen::compile_linked_blocks(stage, src.as_bytes(), 0, &[], s, store::keep()).map(|(sh, _)| sh)
        })
        .expect("the scratch arena is free")
        .unwrap_or_else(|e| panic!("{src}: {e:?}"))
    }

    /// `sh` with its Mali record's varying layout replaced (a record in
    /// keep, tagged as a code generator's).
    fn with_layout(sh: &ShcShader, varyings: &'static [VaryingSlot]) -> ShcShader {
        let o = MaliShader::of(sh).expect("a Mali record");
        /* the link reads the layout only */
        let r: MaliShader<'static> =
            MaliShader { fau: &[], attributes: &[], varyings, moved: &[], ubo: &[], samplers: &[], ..*o };
        let kept = store::keep().alloc_copy(core::slice::from_ref(&r)).expect("keep has room");
        ShcShader { priv_: codegen_priv(kept.as_ptr().cast()), ..*sh }
    }

    /// The link's result and its log.
    fn link(vs: &ShcShader, fs: &ShcShader) -> (i32, String) {
        let mut fs = *fs;
        let mut log = [0 as c_char; 512];
        // SAFETY: two valid shaders and a log of the announced size.
        let rc = unsafe { link_shaders_mali(vs, &mut fs, log.as_mut_ptr(), log.len(), "t") };
        // SAFETY: the writer NUL-terminated the log.
        (rc, String::from_utf8_lossy(unsafe { cstr_bytes(log.as_ptr()) }).into_owned())
    }

    #[test]
    fn each_mali_link_outcome_is_its_own_line() {
        let _k = store::test_keep();
        let (tri_vs, tv_vs, tv_fs) =
            (stage(SHC_STAGE_VERTEX, TRI_VS), stage(SHC_STAGE_VERTEX, TV_VS), stage(SHC_STAGE_FRAGMENT, TV_FS));
        let unused_fs = stage(SHC_STAGE_FRAGMENT, UNUSED_FS);
        let part_fs = stage(SHC_STAGE_FRAGMENT, PART_FS);
        /* a vertex shader that declares a varying and does not write it
         * is refused at compile time: no real record lists an output
         * without a slot, so the slotless vertex layout is made by hand;
         * a fragment record lists an input no code reads (statically used
         * under `if (false)`) and has no slot for it: that layout too */
        assert!(store::with_scratch(|s| {
            codegen::compile_linked_blocks(SHC_STAGE_VERTEX, UNWRITTEN_VS.as_bytes(), 0, &[], s, store::keep()).is_err()
        })
        .expect("the scratch arena is free"));
        static AT_16: [VaryingSlot; 1] =
            [VaryingSlot { varying: Some(0), comps: 3, section: Section::Generic, offset: 16, mask: 0x7 }];
        static POSITION_ONLY: [VaryingSlot; 1] =
            [VaryingSlot { varying: None, comps: 4, section: Section::Position, offset: 0, mask: 0xf }];
        let moved_fs = with_layout(&tv_fs, &AT_16);
        let slotless_fs = with_layout(&tv_fs, &[]);
        let slotless_vs = with_layout(&tv_vs, &POSITION_ONLY);

        /* a fragment input the shader declares and does not use is not one
         * the link pairs: the record does not list it, and the link holds
         * whether the vertex shader writes it or not */
        assert_eq!(unused_fs.num_varyings, 0, "the record lists no unused input");
        assert_eq!(link(&tri_vs, &unused_fs), (0, String::new()));
        assert_eq!(link(&tv_vs, &unused_fs), (0, String::new()));
        /* the pair that matches */
        assert_eq!(link(&tv_vs, &tv_fs), (0, String::new()));

        let enotsup = -crate::util::ENOTSUP;
        let cases = [
            (
                "not written",
                link(&tri_vs, &tv_fs),
                -EINVAL,
                "t: fragment varying 'vcol' is not written by the vertex shader",
            ),
            ("no vertex slot", link(&slotless_vs, &tv_fs), enotsup, &*format!("t: {MALI_NO_VERTEX_SLOT} ('vcol')")),
            ("no fragment slot: nothing to place", link(&tv_vs, &slotless_fs), 0, ""),
            ("different offsets", link(&tv_vs, &moved_fs), enotsup, &*format!("t: {MALI_LAYOUTS_DIFFER} ('vcol')")),
            /* the same offset 0, but the vertex shader stores x there */
            ("a part read", link(&tv_vs, &part_fs), enotsup, &*format!("t: {MALI_PARTIAL_READ} ('vcol')")),
        ];
        /* the vertex shader built for the fragment shader's reads */
        let built_for = |vs: &str, fs: &ShcShader| {
            let pr = crate::shaderc::online::mali::fragment_reads(MaliShader::of(fs).expect("a record"))
                .expect("the record's moved expressions encode");
            store::with_scratch(|s| {
                crate::shaderc::online::mali::compile_vs_for_reads(
                    vs.as_bytes(),
                    pr.reads(),
                    pr.moved(),
                    s,
                    store::keep(),
                )
                .map(|(sh, _)| sh)
            })
            .expect("the scratch arena is free")
            .expect("compiles for the program")
        };
        let part_vs = built_for(TV_VS, &part_fs);
        assert_eq!(link(&part_vs, &part_fs), (0, String::new()), "the program's vertex shader");
        /* a fragment shader with code the link moves (motion.rs): the
         * vertex shader alone is refused, the one built for it links */
        const FC_VS: &str = include_str!("../../../../tools/mali/testdata/fvg_fill_color_nodiscard_noscissor.vert");
        const FC_FS: &str = include_str!("../../../../tools/mali/testdata/fvg_fill_color_nodiscard_noscissor.frag");
        let fc_fs = stage(SHC_STAGE_FRAGMENT, FC_FS);
        let fc_vs = stage(SHC_STAGE_VERTEX, FC_VS);
        assert_eq!(
            link(&fc_vs, &fc_fs),
            (enotsup, format!("t: {MALI_NOT_BUILT_FOR_MOTION}")),
            "not built for the motion"
        );
        assert_eq!(link(&built_for(FC_VS, &fc_fs), &fc_fs), (0, String::new()), "built for the motion");
        for (what, (rc, log), want_rc, want_log) in &cases {
            assert_eq!((rc, log.as_str()), (want_rc, *want_log), "{what}");
        }
    }

    #[test]
    fn a_varying_order_is_its_key() {
        let o = VaryingOrder([2, 1, 0, NO_RANK, NO_RANK, NO_RANK, NO_RANK]);
        assert_eq!(o.key(), 0x8fff_f012);
        assert_eq!(VaryingOrder::of_key(o.key()), Some(o));
        assert_eq!(o.rank(0), 2);
        assert_eq!(o.rank(ORDER_LOCATIONS as u8), NO_RANK);
        /* a blend key (bit 31 clear, or bits 28-30 set) is not an order */
        for k in [0, 1, 0x7fff_ffff, 0x9000_0000, 0xffff_ffff] {
            assert_eq!(VaryingOrder::of_key(k), None, "{k:#x}");
        }
    }

    /// gl_test case 209's pairs through the back end's own entries: the
    /// fragment shader in the vertex shader's order links as it is; in the
    /// reverse order it is compiled again for the vertex shader's order and
    /// takes the program's place, each read at its vertex output's offset.
    #[test]
    fn a_fragment_shader_in_another_order_is_compiled_for_the_vertex_one() {
        use crate::shaderc::online::backend::{release, SHC_ONLINE};
        const VO_VS: &str = include_str!("../../../../tools/mali/testdata/varyorder.vert");
        const VO_FS: &str = include_str!("../../../../tools/mali/testdata/varyorder.frag");
        const VO_REV: &str = include_str!("../../../../tools/mali/testdata/varyorder_rev.frag");
        let _serial = store::test_lock();
        let compile = |stage: u32, src: &str| {
            let text = format!("{src}\0");
            let mut sh = ShcShader::ZERO;
            let mut log = [0 as c_char; 256];
            // SAFETY: a NUL-terminated source, a real output and a log of the announced size.
            let rc =
                unsafe { (SHC_ONLINE.compile.unwrap())(stage, text.as_ptr().cast(), &mut sh, log.as_mut_ptr(), 256) };
            // SAFETY: the writer NUL-terminated the log.
            assert_eq!(rc, 0, "{}", String::from_utf8_lossy(unsafe { cstr_bytes(log.as_ptr()) }));
            sh
        };
        /* the generic offsets of locations 0, 1, 2 */
        let offsets = |sh: &ShcShader| {
            let r = MaliShader::of(sh).expect("a Mali record");
            let at = |loc: u8| {
                r.varyings.iter().find(|s| s.section == Section::Generic && s.varying == Some(loc)).map(|s| s.offset)
            };
            (at(0), at(1), at(2))
        };
        let vs = compile(SHC_STAGE_VERTEX, VO_VS);
        /* the vertex layout: v_a at 0, v_b at 16 (the vec4s in order), v_uv at 32 */
        assert_eq!(offsets(&vs), (Some(0), Some(32), Some(16)));

        let fwd = compile(SHC_STAGE_FRAGMENT, VO_FS);
        let mut linked = fwd;
        let mut log = [0 as c_char; 256];
        // SAFETY: two valid shaders and a log of the announced size.
        assert_eq!(unsafe { link_shaders_mali(&vs, &mut linked, log.as_mut_ptr(), 256, "t") }, 0);
        assert_eq!(linked.priv_, fwd.priv_, "the vertex order's fragment shader is the program's as it is");

        let rev = compile(SHC_STAGE_FRAGMENT, VO_REV);
        /* compiled alone, its own declaration order: v_b (location 0) first */
        assert_eq!(offsets(&rev), (Some(0), Some(32), Some(16)));
        let mut prog = rev;
        crate::shaderc::online::backend::retain(&rev);
        // SAFETY: as above.
        let rc = unsafe { link_shaders_mali(&vs, &mut prog, log.as_mut_ptr(), 256, "t") };
        // SAFETY: the writer NUL-terminated the log.
        assert_eq!(rc, 0, "{}", String::from_utf8_lossy(unsafe { cstr_bytes(log.as_ptr()) }));
        assert_ne!(prog.priv_, rev.priv_, "a fragment shader of the program's own");
        /* the reverse declarations: v_b at location 0, v_uv 1, v_a 2 */
        assert_eq!(offsets(&prog), (Some(16), Some(32), Some(0)), "the vertex shader's offsets");
        /* linked again (a relink, a draw's variant): it pairs as it is */
        let again = prog;
        // SAFETY: as above.
        assert_eq!(unsafe { link_shaders_mali(&vs, &mut prog, log.as_mut_ptr(), 256, "t") }, 0);
        assert_eq!(prog.priv_, again.priv_);
        for sh in [&prog, &rev, &fwd, &vs] {
            release(sh);
        }
    }
}
