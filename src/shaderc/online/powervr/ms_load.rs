// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The per-sample colour load of a kept 4x surface: the program a 4x FBO
//! needs when it is drawn into again after a flush (docs/notes/
//! es3-msaa-renderbuffers.md (c1) and (f2)). Nothing uses it yet: the
//! PowerVR back end's 4x FBOs (pvr-msaa-fbo) bind it.
//!
//! It is the 1x frame load (`PVR_INTERNAL_FBLOAD`,
//! tools/shaders/internal/es/fbload.frag) with its one `texelFetch` turned
//! into the multisample fetch of the sample this invocation shades, as
//! Mesa's pvr load program does it (Mesa 26.2.1 src/imagination/vulkan/
//! pvr_usc.c:1197-1231: `nncoords`, `GLSL_SAMPLER_DIM_MS`,
//! `ms_index = nir_load_sample_id`, no level word). GLSL ES 3.00 has no
//! `sampler2DMS`, so the change is made to the selected program
//! ([`chain::compile_fragment_rewritten`]): the fetch's level word goes,
//! `ms` is set, and the sample index is the `samp_num` special register,
//! which `chain::expand_samples` shifts into the lookup word's `MS_NUM`.
//!
//! The interface is the 1x load's, unchanged: the attachment's image and
//! sampler words in texture slot 0 (shared 0..8 and 8..12), `u_flip` as the
//! uniform block, gl_FragCoord's y pair after it, one RGBA8 colour in pixel
//! output 0. The back end moves the output writes to another register as it
//! moves the 1x program's (`src/pvr/backend.rs` `fbload_program_at`). Two
//! things are the caller's, not the program's: the image descriptor names
//! the 4x surface (its sample count is the descriptor's, not the
//! program's), and the draw runs at the sample rate (pco's
//! `uses_sample_shading`), so that `samp_num` walks 0..4.
//!
//! The source is GLSL ES 3.00 (`texelFetch`), so a `report-es2` build refuses
//! it ([`MsLoadError::Front`]): such a build has no 4x FBO either.

use super::chain::{self, ChainError};
use super::decode::{for_each_group, Group};
use super::isa::ops_for;
use super::isa_tables::ENUMS;
use super::operands::srcs;
use super::sel::{Bank, Mods, SelDst, SelGroup, SelOp, SelSrc, SmpLod};
use super::select::Selected;
use crate::shaderc::online::arena::Arena;
use crate::shaderc::ShcShader;

/// The sample count the load reads per pixel: one invocation per sample.
pub const MS_LOAD_SAMPLES: u8 = 4;

/// `samp_num`: the sample the invocation shades (Mesa 26.2.1
/// src/imagination/pco/pco_isa.py; `select.rs`'s `SR_SAMP_NUM`).
pub const SR_SAMP_NUM: u16 = 53;

/// The 1x frame load's source, which this program starts from.
pub const FBLOAD_ES_FRAG: &str = include_str!("../../../../tools/shaders/internal/es/fbload.frag");

/// Why the load was not built.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MsLoadError {
    /// A sample count other than [`MS_LOAD_SAMPLES`].
    Samples(u8),
    /// The front end refused the 1x source.
    Front(&'static str),
    Chain(ChainError),
    /// The assembled program is not the one meant (decode or check).
    Verify(&'static str),
}

/// The built program and where its two multisample parts sit.
#[derive(Clone, Copy)]
pub struct MsLoad {
    /// The record, its code in the keep arena (as every compile's).
    pub shader: ShcShader,
    /// The sample count it was built for.
    pub samples: u8,
    /// Byte offset of the one `smp` (with `sno`).
    pub smp_at: usize,
    /// Byte offset of the group that reads `samp_num` for the lookup word.
    pub sample_index_at: usize,
}

/// The selected program's change: its single fetch becomes the per-sample
/// multisample fetch. Anything else than one plain level-0 fetch is refused,
/// so a change to the 1x source cannot slip through unseen.
fn per_sample_fetch<'b>(sel: Selected<'b>, a: &'b Arena<'b>) -> Result<Selected<'b>, ChainError> {
    let refuse = ChainError::Build(super::assemble::BuildError::Unsupported(
        "a multisample load program whose 1x source is not one plain texelFetch",
    ));
    let groups = a.alloc_copy(sel.groups).map_err(|_| ChainError::OutOfScratch)?;
    let mut n = 0;
    for g in groups.iter_mut() {
        match g.op {
            SelOp::Sample {
                binding,
                proj: false,
                lod: SmpLod::Replace,
                fetch: true,
                offset: None,
                cube: false,
                layer: false,
                gather: false,
                ms: false,
            } => {
                g.op = SelOp::Sample {
                    binding,
                    proj: false,
                    lod: SmpLod::Implicit,
                    fetch: true,
                    offset: None,
                    cube: false,
                    layer: false,
                    gather: false,
                    ms: true,
                };
                g.srcs[2] = Some(SelSrc::Fixed { bank: Bank::Special, reg: SR_SAMP_NUM, mods: Mods::default() });
                n += 1;
            }
            SelOp::Sample { .. } => return Err(refuse),
            _ => {}
        }
    }
    if n != 1 {
        return Err(refuse);
    }
    Ok(Selected { groups, ..sel })
}

fn bank_name(v: u32) -> Option<&'static str> {
    ENUMS.iter().find(|e| e.name == "regbank").and_then(|e| e.name_of(v))
}

fn is_smp(g: &Group) -> bool {
    g.instrs().any(|(_, ph, r)| ops_for(r, ph).any(|o| o.name == "smp"))
}

/// Does `g` read special register `reg` through any source slot?
fn reads_special(g: &Group, reg: u16) -> bool {
    [g.lower.as_ref(), g.upper.as_ref()]
        .into_iter()
        .flatten()
        .flat_map(|r| srcs(r).into_iter().flatten())
        .any(|o| bank_name(o.bank) == Some(Bank::Special.isa_name()) && o.index == u32::from(reg))
}

/// Check the assembled code: exactly one `smp`, and it passes
/// [`super::verify::check_group`] as the multisample fetch of texture slot 0
/// (implicit level, `nncoords`, `sno`); some group before it reads
/// `samp_num`.
fn verify(bytes: &[u8]) -> Result<(usize, usize), MsLoadError> {
    let mut smp = None;
    let mut index_at = None;
    let mut count = 0;
    for_each_group(bytes, |off, g| {
        if is_smp(g) {
            count += 1;
            smp.get_or_insert((off, *g));
        } else if smp.is_none() && index_at.is_none() && reads_special(g, SR_SAMP_NUM) {
            index_at = Some(off);
        }
        Ok(())
    })
    .map_err(|_| MsLoadError::Verify("the program does not decode"))?;
    let (Some((at, g)), 1) = (smp, count) else {
        return Err(MsLoadError::Verify("not exactly one smp"));
    };
    let index_at = index_at.ok_or(MsLoadError::Verify("no samp_num read before the smp"))?;
    /* the data block and the result registers are the chain's choice: read
     * them back, and check everything else against the meant group */
    let lo = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
    let up = g.upper.as_ref().map(srcs).unwrap_or([None; 6]);
    let temp = |o: Option<super::operands::Operand>| {
        o.filter(|o| bank_name(o.bank) == Some(Bank::Temp.isa_name())).map(|o| o.index as u16)
    };
    let (Some(data), Some(dest)) = (temp(lo[1]), temp(up[4])) else {
        return Err(MsLoadError::Verify("the smp's data block or result is not in temporaries"));
    };
    let shared = |reg: u16| Some(SelSrc::Fixed { bank: Bank::Shared, reg, mods: Mods::default() });
    let meant = SelGroup {
        op: SelOp::Sample {
            binding: 0,
            proj: false,
            lod: SmpLod::Implicit,
            fetch: true,
            offset: None,
            cube: false,
            layer: false,
            gather: false,
            ms: true,
        },
        ty: super::sel::OpType::F32,
        dst: Some(SelDst::Fixed { bank: Bank::Temp, reg: dest }),
        srcs: [shared(0), Some(SelSrc::Fixed { bank: Bank::Temp, reg: data, mods: Mods::default() }), shared(8)],
    };
    match super::verify::check_group(&meant, "smp", &bytes[at..at + g.total]) {
        Ok(n) if n == g.total => Ok((at, index_at)),
        _ => Err(MsLoadError::Verify("the smp is not the per-sample multisample fetch of slot 0")),
    }
}

/// Build, select and verify the per-sample load of one RGBA8 colour
/// attachment kept at `samples` samples (only [`MS_LOAD_SAMPLES`]). Every
/// intermediate lives in `scratch`; the code goes to the keep arena as a
/// compile's does, so the caller holds a keep transaction
/// (`store::begin_keep`) as it does around a compile.
pub fn build_ms_load(scratch: &Arena<'_>, samples: u8) -> Result<MsLoad, MsLoadError> {
    if samples != MS_LOAD_SAMPLES {
        return Err(MsLoadError::Samples(samples));
    }
    let src = scratch.alloc_str(FBLOAD_ES_FRAG).map_err(|_| MsLoadError::Chain(ChainError::OutOfScratch))?;
    let l = crate::shaderc::online::lower::compile_to_ir(scratch, crate::shaderc::online::sema::Stage::Fragment, src)
        .map_err(|e| MsLoadError::Front(e.msg))?;
    let shader = chain::compile_fragment_rewritten(&l.module, &l.reflection, scratch, per_sample_fetch)
        .map_err(MsLoadError::Chain)?;
    // SAFETY: the record's code is `usc_dw` dwords in the keep arena, alive
    // for as long as the caller's keep transaction.
    let words = unsafe { core::slice::from_raw_parts(shader.usc, shader.usc_dw as usize) };
    let buf = scratch.alloc_filled(words.len() * 4, 0u8).map_err(|_| MsLoadError::Chain(ChainError::OutOfScratch))?;
    for (c, w) in buf.as_chunks_mut::<4>().0.iter_mut().zip(words) {
        c.copy_from_slice(&w.to_le_bytes());
    }
    let (smp_at, sample_index_at) = verify(buf)?;
    Ok(MsLoad { shader, samples, smp_at, sample_index_at })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::store;

    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    fn bytes(sh: &ShcShader) -> std::vec::Vec<u8> {
        // SAFETY: the record's code, alive under the test's keep.
        let w = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
        w.iter().flat_map(|d| d.to_le_bytes()).collect()
    }

    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    fn build() -> MsLoad {
        store::with_scratch(|a| build_ms_load(a, MS_LOAD_SAMPLES)).expect("scratch").expect("builds")
    }

    /// The program assembles, decodes group by group to its end, and passes
    /// the check: one `smp`, the multisample fetch of slot 0 with `sno`,
    /// after a group that reads `samp_num` (the sample index the chain
    /// shifts into MS_NUM).
    #[test]
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    fn the_4x_load_assembles_and_verifies() {
        let _k = store::test_keep();
        let l = build();
        assert_eq!(l.samples, 4);
        let b = bytes(&l.shader);
        assert!(!b.is_empty());
        assert_eq!(verify(&b), Ok((l.smp_at, l.sample_index_at)));
        assert!(l.sample_index_at < l.smp_at);
        let mut smps = std::vec::Vec::new();
        let n = for_each_group(&b, |off, g| {
            if is_smp(g) {
                smps.push((off, *g));
            }
            Ok(())
        })
        .expect("decodes");
        assert!(n > 1);
        assert_eq!(smps.len(), 1, "exactly one smp");
        let (at, g) = smps[0];
        assert_eq!(at, l.smp_at);
        let (_, _, r) = g.instrs().find(|(_, ph, r)| ops_for(r, *ph).any(|o| o.name == "smp")).unwrap();
        assert_eq!(r.get("sno"), Some(1), "the multisample form");
        assert_eq!(r.get("nncoords"), Some(1), "a texel fetch");
        assert_eq!(r.get("pplod"), Some(0), "no level word");
        let mut text = std::string::String::new();
        super::super::listing::group_text(&mut text, at, &g).unwrap();
        assert!(text.contains(" sno=1 "), "{text}");
    }

    /// The sample index is the shaded sample's: `samp_num`, shifted to
    /// MS_NUM (bit 16, `sel::LOOKUP_MS_NUM_START`) by a group that reads it
    /// with the immediate 16, never a constant index folded into the word.
    #[test]
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    fn the_sample_index_is_samp_num_shifted_into_ms_num() {
        let _k = store::test_keep();
        let l = build();
        let b = bytes(&l.shader);
        let mut readers = 0;
        for_each_group(&b, |_, g| {
            if reads_special(g, SR_SAMP_NUM) {
                readers += 1;
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(readers, 1, "samp_num read once");
        let g = super::super::decode::decode_group(&b, l.sample_index_at).unwrap();
        let mut text = std::string::String::new();
        super::super::listing::group_text(&mut text, l.sample_index_at, &g).unwrap();
        assert!(text.contains("shl") || text.contains("lsl"), "a shift: {text}");
        assert_eq!(super::super::sel::LOOKUP_MS_NUM_START, 16);
    }

    /// The record is the 1x load's: same uniform words, one output, the
    /// y pair after the block, nothing iterated, no discard.
    #[test]
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    fn the_record_matches_the_1x_load_interface() {
        let _k = store::test_keep();
        let l = build();
        let one = store::with_scratch(|a| {
            let src = a.alloc_str(FBLOAD_ES_FRAG).unwrap();
            let m = crate::shaderc::online::lower::compile_to_ir(a, crate::shaderc::online::sema::Stage::Fragment, src)
                .unwrap();
            chain::compile_fragment(&m.module, &m.reflection, a).expect("the 1x load compiles")
        })
        .unwrap();
        let (s, o) = (&l.shader, &one);
        assert_eq!(s.uniform_dwords, o.uniform_dwords);
        assert_eq!(s.fragcoord_ytf_off1, o.fragcoord_ytf_off1);
        assert_eq!((s.num_varyings, s.uses_discard, s.num_outputs), (o.num_varyings, o.uses_discard, o.num_outputs));
        /* the 1x one has no multisample fetch and reads no samp_num */
        let b = bytes(o);
        assert!(verify(&b).is_err());
        let mut reads = false;
        for_each_group(&b, |_, g| {
            reads |= reads_special(g, SR_SAMP_NUM);
            Ok(())
        })
        .unwrap();
        assert!(!reads);
    }

    /// A build that reports ES 2.0 refuses the ES 3.00 source: it has no 4x
    /// load, as it has no 4x FBO to bind one to.
    #[test]
    #[cfg(feature = "report-es2")]
    fn a_report_es2_build_refuses_the_es3_source() {
        let _k = store::test_keep();
        let r = store::with_scratch(|a| build_ms_load(a, MS_LOAD_SAMPLES).err()).unwrap();
        assert!(matches!(r, Some(MsLoadError::Front(_))), "{r:?}");
    }

    /// Only 4 samples: the program is the 4x FBO's.
    #[test]
    fn other_sample_counts_are_refused() {
        let _k = store::test_keep();
        for n in [0u8, 1, 2, 8] {
            let r = store::with_scratch(|a| build_ms_load(a, n).err()).unwrap();
            assert_eq!(r, Some(MsLoadError::Samples(n)));
        }
    }

    /// The rewrite takes the one plain fetch and nothing else.
    #[test]
    fn the_rewrite_refuses_a_program_without_one_plain_fetch() {
        let mut buf = std::vec![0u8; 4 << 10];
        let a = Arena::new(&mut buf);
        let none = Selected { groups: &[], vregs: 0, outputs: &[], inputs: &[], discard: None, alpha_to_one: None };
        assert!(per_sample_fetch(none, &a).is_err());
    }
}
