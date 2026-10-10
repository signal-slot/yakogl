// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2024 Imagination Technologies Ltd.
// SPDX-FileCopyrightText: 2025 Imagination Technologies Ltd.
// Ported from Mesa 26.2.1 src/imagination/pco/pco_legalize.c and pco_map.py (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! A fragment shader's varyings: where they land, and the groups that
//! evaluate them.
//!
//! A varying is not a register the program reads. The coefficient file holds
//! each component's plane equation, and an iterator evaluates it at the
//! fragment into temporaries. pco does that with `ditrp` (iterate with the
//! perspective divide by W, whose coefficients sit at 0..3), and wraps each
//! one exactly as `pco_legalize.c` says: a `fence` before and after
//! (`try_legalize_ditr_fence`), each fence lowered to `flush_p0` and two
//! `br.next`, one under `if(p0)` and one under `if(!p0)` (`legalize_fence`),
//! and a `wdf` on data-return channel 0 after (`pco_schedule.c`). Every group
//! here is transcribed from those passes and `pco_map.py`'s `group_map`s
//! (Mesa, MIT, read and transcribed) and checked against pco's bytes.
//!
//! The layout, measured with pco_shc on a three-varying shader (vec2, vec4,
//! float): W at coefficients 0..3, then each varying the program reads, in
//! declaration order, four coefficients per component -- 4, 12, 28.

use super::assemble::{close, BuildError, Parts};
use super::decode::{Group, MAX_INSTRS};
use super::instr::instr_region;
use super::isa::{BitSetId, Region, PH_0, PH_2_TST, PH_CTRL};
use super::legalise::{choose_src_variant, fill_srcs};
use super::operands::Operand;
use super::sel::Mods;

const ALU_MAIN: u32 = 0;
const ALU_CONTROL: u32 = 3;

/// The value `name` has in the generated enum `enum_name`.
pub(super) fn ev(enum_name: &str, name: &str) -> Result<u32, BuildError> {
    super::isa_tables::ENUMS
        .iter()
        .find(|e| e.name == enum_name)
        .and_then(|e| e.elems.iter().find(|(_, n)| *n == name))
        .map(|(v, _)| *v)
        .ok_or(BuildError::Unsupported("an enum value the ISA tables do not have"))
}

pub(super) fn hset(h: &mut Region, f: &'static str, v: u32) -> Result<(), BuildError> {
    h.set(f, v).map_err(|_| BuildError::Unsupported("a header field that does not take its value"))
}

/// A control group holding one operation and no sections.
pub(super) fn control(
    op: &'static str,
    extra: &[(&'static str, u32)],
    cc: &str,
    ctrlop: &str,
) -> Result<Group, BuildError> {
    let r = instr_region(op, PH_CTRL, &[Mods::default(); 3], extra).map_err(BuildError::Form)?;
    let slot = super::pack::slot_of(PH_CTRL)
        .ok_or(BuildError::Phases(super::pack::NotPackable::UnknownPhase { phase: PH_CTRL }))?;
    let mut instrs: [Option<(u8, Region)>; MAX_INSTRS] = [None; MAX_INSTRS];
    instrs[slot as usize] = Some((PH_CTRL, r));
    let hv = super::pack::header_variant_of(ALU_CONTROL).ok_or(BuildError::Unsupported("no control header"))?;
    let mut hdr = Region::new(BitSetId::IgrpHdr, hv);
    hset(&mut hdr, "cc", ev("cc", cc)?)?;
    hset(&mut hdr, "ctrlop", ev("ctrlop", ctrlop)?)?;
    let parts = Parts { alutype: ALU_CONTROL, instrs, lower: None, upper: None, iss: None, dst: None, align_pad: 0 };
    close(&parts, hdr).map_err(BuildError::Group)
}

/// `flush_p0`: `group_map(O_FLUSH_P0)`. An `imadd32` of three zeros with p0
/// as its carry-in, and a `tst` greater-than-zero on the result as u32 that
/// writes p0 and ends phase 2. No destination.
fn flush_p0() -> Result<Group, BuildError> {
    let mut instrs: [Option<(u8, Region)>; MAX_INSTRS] = [None; MAX_INSTRS];
    let m = [Mods::default(); 3];
    let madd = instr_region("imadd32", PH_0, &m, &[("cin", 1), ("s", 0)]).map_err(BuildError::Form)?;
    let tst = instr_region(
        "tst",
        PH_2_TST,
        &m,
        &[("tst_op", ev("tst_op", "gz")?), ("pwen", 1), ("type", ev("tst_type", "u32")?), ("p2end", 1), ("elem", 0)],
    )
    .map_err(BuildError::Form)?;
    for (ph, r) in [(PH_0, madd), (PH_2_TST, tst)] {
        let slot =
            super::pack::slot_of(ph).ok_or(BuildError::Phases(super::pack::NotPackable::UnknownPhase { phase: ph }))?;
        instrs[slot as usize] = Some((ph, r));
    }
    const SPECIAL: u32 = 0;
    let zero = Some(Operand { bank: SPECIAL, index: 0 });
    let ops = [zero, zero, zero, None, None, None];
    let v = choose_src_variant(&ops, 0).map_err(BuildError::Fit)?;
    let mut lower = Region::new(BitSetId::Src, v);
    fill_srcs(&mut lower, &ops, 0).map_err(BuildError::Fit)?;
    let set = super::isa::bit_set(BitSetId::Src);
    let uv = set
        .variants
        .iter()
        .enumerate()
        .filter(|(_, v)| v.upper_src == Some(true))
        .min_by_key(|(_, v)| v.bytes)
        .map(|(i, _)| i as u16)
        .ok_or(BuildError::Unsupported("no upper source variant"))?;
    let upper = Some(Region::new(BitSetId::Src, uv));
    let iss = Some(Region::new(BitSetId::Iss, 0));
    let hv = super::pack::header_variant_of(ALU_MAIN).ok_or(BuildError::Unsupported("no main header"))?;
    let mut hdr = Region::new(BitSetId::IgrpHdr, hv);
    let oporg =
        super::pack::oporg_for(&[PH_0, PH_2_TST]).ok_or(BuildError::Phases(super::pack::NotPackable::NotAKnownSet))?;
    hset(&mut hdr, "oporg", oporg)?;
    let parts = Parts { alutype: ALU_MAIN, instrs, lower: Some(lower), upper, iss, dst: None, align_pad: 0 };
    close(&parts, hdr).map_err(BuildError::Group)
}

/// `br.next` under `cc`: a branch to the group that follows it, whose
/// offset is the branch group's own length.
fn br_next(cc: &str) -> Result<Group, BuildError> {
    /* The length does not depend on the offset's value: build once to
     * measure, then with the offset. */
    let probe = control("br.next", &[("offset", 0)], cc, "b")?;
    control("br.next", &[("offset", probe.total as u32)], cc, "b")
}

/// A loop's back edge: `br` under `if(p0)` by `offset` bytes from the branch
/// group's own start, backward. pco's own back edge has this form
/// (filter_image.frag: `cc=if(p0)`, `bpred=cc`, offset -648 from 0x524 to
/// 0x29c, its body's first group). A forward or zero offset is refused: a
/// back edge only ever goes back (the forward branches past a branched
/// `if`'s arm are `cf::branch_none_active`'s).
pub fn branch_back(offset: i64) -> Result<Group, BuildError> {
    if offset >= 0 {
        return Err(BuildError::Unsupported("a back edge that does not go back"));
    }
    let off = i32::try_from(offset).map_err(|_| BuildError::Unsupported("a back edge too long to encode"))?;
    control("br.next", &[("offset", off as u32)], "if(p0)", "b")
}

/// One read varying: its components land in temporaries `dest..dest+count`
/// from coefficient `coff`.
///
/// [`IterMode::Flat`]: the varying is not iterated. Its plane's constant term is the
/// value, and pco reads it with one `mbyp` per component from coefficient
/// `coff + 4 * c + 2` (`ROGUE_USC_COEFFICIENT_SET_C`; Mesa `pco_trans_nir.c`,
/// `INTERP_MODE_FLAT`; pco_shc build-new on a `flat in vec2`, 2026-09-23:
/// coefficients 14 and 18 for a flat varying at 12), after the iterations.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Iterated {
    pub dest: u16,
    pub coff: u16,
    /// Where W's coefficients start, which a perspective iterate divides
    /// by: 0, or 4 when gl_FragCoord.z's set comes first.
    pub woff: u16,
    pub count: u8,
    pub mode: IterMode,
}

/// How an input's value comes out of its coefficients.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IterMode {
    /// `ditrp`: iterated and divided by the W plane (a smooth varying).
    Perspective,
    /// `ditrp` at the centroid (`iter_mode` centroid): a `centroid in`, as
    /// pco iterates one (pco_trans_nir.c trans_load_input_fs:
    /// PCO_ITR_MODE_CENTROID for `var->data.centroid`). Single-sampled, the
    /// centroid of a covered pixel is its centre, so this is
    /// [`IterMode::Perspective`]'s value there; multisampled, the hardware
    /// moves the point inside the covered samples.
    Centroid,
    /// `ditr`: iterated with no divide -- pco's form for `gl_PointCoord`
    /// (pco_shc build-new: `ditr p=none`, count 2, after W's four).
    Linear,
    /// Not iterated: the plane's constant, one move per component.
    Flat,
}

/// Component `c` of a flat varying: a move from its plane's C coefficient.
pub fn flat_group(v: Iterated, c: u8) -> Result<Group, BuildError> {
    use super::sel::{Bank, OpType, SelDst, SelGroup, SelOp, SelSrc};
    super::assemble::build_group(&SelGroup {
        op: SelOp::Copy,
        ty: OpType::F32,
        dst: Some(SelDst::Fixed { bank: Bank::Temp, reg: v.dest + u16::from(c) }),
        srcs: [
            Some(SelSrc::Fixed { bank: Bank::Coeff, reg: v.coff + 4 * u16::from(c) + 2, mods: Mods::default() }),
            None,
            None,
        ],
    })
}

/// How many groups [`iterate_group`] builds for one varying.
pub const GROUPS_PER_VARYING: usize = 8;

/// Group `k` of the sequence that evaluates `v`: a fence (`flush_p0`,
/// `br.next` if p0, `br.next` if !p0), `ditrp` (`encode_map(O_DITRP)`: W at
/// coefficient 0, pixel centre, channel 0, no saturate), a fence, `wdf`.
/// Built one at a time so that no caller holds a whole sequence of groups:
/// the builder runs on Zephyr's stack.
pub fn iterate_group(v: Iterated, k: usize) -> Result<Group, BuildError> {
    match k {
        0 | 4 => flush_p0(),
        1 | 5 => br_next("if(p0)"),
        2 | 6 => br_next("if(!p0)"),
        3 if v.mode == IterMode::Linear => control(
            "ditr",
            &[
                ("dest", v.dest as u32),
                ("coff", v.coff as u32),
                ("mode", ev("iter_mode", "pixel")?),
                ("count", v.count as u32),
                ("f16", 0),
                ("drc", 0),
                ("sat", 0),
            ],
            "e1_zx",
            "ditr",
        ),
        3 => control(
            "ditrp",
            &[
                ("dest", v.dest as u32),
                ("coff", v.coff as u32),
                ("woff", v.woff as u32),
                ("mode", ev("iter_mode", if v.mode == IterMode::Centroid { "centroid" } else { "pixel" })?),
                ("count", v.count as u32),
                ("f16", 0),
                ("drc", 0),
                ("sat", 0),
            ],
            "e1_zx",
            "ditr",
        ),
        7 => control("wdf", &[], "e1_zx", "wdf"),
        _ => Err(BuildError::Unsupported("past the end of a varying's groups")),
    }
}

/// The coefficient each listed varying starts at: W's four, then four per
/// component in list order. `comps[i]` is varying `i`'s width.
pub fn coefficient_offsets(comps: &[u8], offs: &mut [u16]) -> Result<(), BuildError> {
    if offs.len() < comps.len() {
        return Err(BuildError::Unsupported("more varyings than the caller made room for"));
    }
    let mut at = 4u16;
    for (o, c) in offs.iter_mut().zip(comps) {
        if *c == 0 || *c > 4 {
            return Err(BuildError::Unsupported("a varying that is not one to four components"));
        }
        *o = at;
        at = at.checked_add(4 * *c as u16).ok_or(BuildError::Unsupported("the coefficients do not fit"))?;
    }
    Ok(())
}

/// How one `smp` reads ([`build_smp`]): the operands' meaning, beside the
/// registers.
#[derive(Clone, Copy, Debug)]
pub struct SmpForm {
    /// `texture2DProj`: the data block holds (x, y, q).
    pub proj: bool,
    /// The level: implicit, replaced, biased or from gradients.
    pub lod: super::sel::SmpLod,
    /// Texel coordinates (`nncoords`), a `texelFetch`.
    pub fetch: bool,
    /// A lookup word with a constant offset ends the data block.
    pub offset: bool,
    /// The TPU's own cube: three coordinates, `dmn` 3d.
    pub cube: bool,
    /// An integer destination: no `fcnorm`.
    pub integer: bool,
}

/// `texture2D` as pco emits it in a fragment shader: `smp` in the backend
/// phase, form `smp_brief` -- normalised coordinates (`fcnorm`), 2D, four
/// channels, the hardware's own LOD (`lodm = normal`), data-return channel 0
/// -- reading the image descriptor at `image`, the sample data block
/// from temporary `data` and the sampler state at `sampler`, and
/// writing the four channels from temporary `dest` (named in the upper
/// section's `s4`; `s3` is special register 0, as in all 2,665 such groups
/// of the corpus). No destination section: the result arrives on the data
/// channel, which is what the `wdf` after it waits for.
///
/// `image` and `sampler` are shared registers: named directly below sh256,
/// or through index register 0 (`idx0`, `3 | offset << 3`) past it, as
/// pco's legalisation writes them (`prepare::index_high_shared`).
///
/// `form.proj`: `texture2DProj`, pco's form for it -- the same operands with
/// `proj` set, which only the extended `smp_exta` form carries; the data
/// block then holds (x, y, q) and the hardware divides.
///
/// `form.integer`: an `isampler*` / `usampler*` look-up, `fcnorm` clear so
/// that the TPU hands back the texel's integers, as pco sets it from the
/// destination type (Mesa pco_nir_tex.c:236, `.fcnorm = base_type(dest_type)
/// == nir_type_float`).
pub fn build_smp(image: Operand, data: u16, sampler: Operand, dest: u16, form: SmpForm) -> Result<Group, BuildError> {
    use super::isa::PH_BACKEND;
    let SmpForm { proj, lod, fetch, offset, cube, integer } = form;
    let fcnorm = u32::from(!integer);
    let v = |f: &str, n: &str| super::shapes::value(BitSetId::Backend, f, n);
    /* a cube is the TPU's own (TEXTYPE_CUBE): three coordinates, `dmn` 3d,
     * as pco's program for a samplerCube has it */
    let dmn = if cube { "3d" } else { "2d" };
    /* the gradients (`lodm = gradients`) need no extended field */
    let brief_lodm = if lod == super::sel::SmpLod::Gradients { "gradients" } else { "normal" };
    let brief =
        [("fcnorm", fcnorm), ("drc", 0), ("dmn", v("dmn", dmn)?), ("chan", 4), ("lodm", v("lodm", brief_lodm)?)];
    /* The extended form: `proj` (texture2DProj), or a per-pixel level word
     * in the data block -- replacing the level (textureLod: `lodm` replace)
     * or added to it (a bias: `lodm` bias), with `pplod` either way, as pco
     * sets them. */
    let lodm = match lod {
        super::sel::SmpLod::Implicit => "normal",
        super::sel::SmpLod::Replace => "replace",
        super::sel::SmpLod::Bias => "bias",
        super::sel::SmpLod::Gradients => "gradients",
    };
    let exta = [
        ("fcnorm", fcnorm),
        ("drc", 0),
        ("dmn", v("dmn", dmn)?),
        ("chan", 4),
        ("lodm", v("lodm", lodm)?),
        ("pplod", u32::from(lod.in_block())),
        ("proj", u32::from(proj)),
        ("nncoords", u32::from(fetch)),
        ("soo", u32::from(offset)),
    ];
    if proj && lod.in_block() {
        return Err(BuildError::Unsupported("a projective sample with an explicit level or a bias"));
    }
    let extra: &[(&str, u32)] = if proj || lod.in_block() || offset { &exta } else { &brief };
    let r = instr_region("smp", PH_BACKEND, &[Mods::default(); 3], extra).map_err(BuildError::Form)?;
    let slot = super::pack::slot_of(PH_BACKEND)
        .ok_or(BuildError::Phases(super::pack::NotPackable::UnknownPhase { phase: PH_BACKEND }))?;
    let mut instrs: [Option<(u8, Region)>; MAX_INSTRS] = [None; MAX_INSTRS];
    instrs[slot as usize] = Some((PH_BACKEND, r));

    const SPECIAL: u32 = 0;
    const TEMP: u32 = 1;
    let lower_ops = [Some(image), Some(Operand { bank: TEMP, index: data as u32 }), Some(sampler), None, None, None];
    let lv = choose_src_variant(&lower_ops, 0).map_err(BuildError::Fit)?;
    let mut lower = Region::new(BitSetId::Src, lv);
    fill_srcs(&mut lower, &lower_ops, 0).map_err(BuildError::Fit)?;
    let upper_ops = [
        None,
        None,
        None,
        Some(Operand { bank: SPECIAL, index: 0 }),
        Some(Operand { bank: TEMP, index: dest as u32 }),
        None,
    ];
    let uv = choose_src_variant(&upper_ops, 0).map_err(BuildError::Fit)?;
    let mut upper = Region::new(BitSetId::Src, uv);
    fill_srcs(&mut upper, &upper_ops, 0).map_err(BuildError::Fit)?;

    let hv = super::pack::header_variant_of(ALU_MAIN).ok_or(BuildError::Unsupported("no main header"))?;
    let mut hdr = Region::new(BitSetId::IgrpHdr, hv);
    let oporg =
        super::pack::oporg_for(&[PH_BACKEND]).ok_or(BuildError::Phases(super::pack::NotPackable::NotAKnownSet))?;
    hset(&mut hdr, "oporg", oporg)?;
    let parts = Parts {
        alutype: ALU_MAIN,
        instrs,
        lower: Some(lower),
        upper: Some(upper),
        iss: Some(Region::new(BitSetId::Iss, 0)),
        dst: None,
        align_pad: 0,
    };
    close(&parts, hdr).map_err(BuildError::Group)
}

/// A control `nop` that ends the program (`miscctl` set, as pco's program
/// for a fragment shader that writes nothing carries it).
pub fn end_nop() -> Result<Group, BuildError> {
    let mut g = control("nop", &[], "e1_zx", "nop")?;
    hset(&mut g.hdr, "miscctl", 1)?;
    Ok(g)
}

/// `wdf` on data-return channel 0: the wait after a sample.
pub fn wait_data() -> Result<Group, BuildError> {
    control("wdf", &[], "e1_zx", "wdf")
}
