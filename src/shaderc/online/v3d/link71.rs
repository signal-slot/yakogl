// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GL link entry for the V3D 7.1 (the Raspberry Pi 5's BCM2712 D0, V3D
//! 7.1.10): [`codegen::link_sources`] and [`codegen::compile_program`] with
//! the 7.1 chain ([`chain71::compile_program71_keyed`]) and the 7.1
//! packaging ([`super::abi71::program_shader71`]) in place of the 4.2
//! ones. Built in every `gpu-v3d` build, as the 7.1 compiler is; nothing
//! calls it until a 7.1 table does (backend71), and the 4.2 entry is
//! unchanged.
//!
//! What goes in and comes out is the 4.2 entry's, step for step:
//!
//! * [`link_sources71`]: both sources through the front end again, the
//!   vertex shader with the link's attribute map (`lower_vertex_mapped`, a
//!   copy of codegen.rs's private one), the vertex stage's `c = 0.5` remap
//!   (`lodkey::remap_mag_switch`), the fragment shader with the zero-varying
//!   list, then [`compile_program71_shader`] with [`ProgramKey::LINK`] and
//!   the vertex key, and the fragment stage's block table with both stages'
//!   uniform rows.
//! * [`compile_program71_shader`]: `blend_key != 0` refused as
//!   `NotImplemented(CodeGen)` (the TLB blends), the fragment stage's record
//!   ([`v3d_abi::stage_shader`]), the program compiled for the 7.1 VPM
//!   ([`chain71::TARGET71`]) with the program key's vertex sampler key and
//!   the compile's fragment one (`shaderc::compile_lod_key`). Lines and
//!   Points go to the chain as they are: chain71 compiles both (ce7e5209,
//!   Mesa's is_points / is_lines keys), where the 4.2 chain refuses them;
//!   a program that does not read `gl_PointCoord` is the Triangles program
//!   at both.
//!
//! # Where 7.1 has no equivalent yet
//!
//! * The 32-bit float render-target variant (`shaderc::compile_rt_f32`,
//!   chain.rs `compile_program_rt`'s `rt_f32`): chain71 has no `rt_f32`
//!   parameter, so a non-zero key is refused by name ([`RT_F32_NOT_YET`])
//!   rather than compiled as the plain program.
//! * A refusal of a 7.1-only stage ([`V3d71Error`] other than `Chain`)
//!   has no [`BackEndError`] variant of its own (codegen.rs is the 4.2
//!   entry's and stays as it is): [`compile_error71`] carries it as
//!   `BackEndError::Program(V3dError::NotYet(..))` with a name that says it
//!   is the 7.1 stage's, or as `V3dError::Verify` (verify71 shares 4.2's
//!   `VerifyError`). The GL core's log prints the name, and -ENOTSUP, as for
//!   any `BackEnd` refusal.

use super::alloc71::Alloc71Error;
use super::chain::{Prim, V3dError};
use super::chain71::{self, V3d71Error};
use super::sched71::Sched71Error;
use super::select71::Select71Error;
use crate::shaderc::online::arena::Arena;
use crate::shaderc::online::codegen::{self, BackEndError, Blocks, CompileError, ProgramKey, Token};
use crate::shaderc::online::v3d_abi;
use crate::shaderc::ShcShader;

/// The refusal of a non-zero `compile_rt_f32` key at 7.1.
pub const RT_F32_NOT_YET: &str = "the 32-bit float render-target variant (rt_f32) at 7.1";

/// A [`V3d71Error`] as the link entry's [`CompileError`] (the module
/// comment says why each 7.1-only refusal is a named `NotYet`).
pub fn compile_error71(e: V3d71Error) -> CompileError {
    let not_yet = |what: &'static str| CompileError::from(V3dError::NotYet(what));
    match e {
        V3d71Error::Chain(e) => CompileError::from(e),
        V3d71Error::OutOfScratch => CompileError::from(V3dError::OutOfScratch),
        V3d71Error::NotYet(what) => not_yet(what),
        V3d71Error::Select(s) => match s {
            Select71Error::NotYet(what) | Select71Error::NotOn71(what) => not_yet(what),
            Select71Error::ViewportXyMissing => not_yet("7.1 selection: the viewport-scaled x and y pair is missing"),
            Select71Error::OutOfScratch => CompileError::from(V3dError::OutOfScratch),
        },
        V3d71Error::Alloc(a) => match a {
            Alloc71Error::NotYet(what) => not_yet(what),
            Alloc71Error::DoesNotFit => {
                not_yet("more live values than 2 threads have registers at 7.1 (no spilling yet)")
            }
            Alloc71Error::OutOfScratch => CompileError::from(V3dError::OutOfScratch),
            _ => not_yet("7.1 register allocation refused the program"),
        },
        V3d71Error::Sched(s) => match s {
            Sched71Error::NotYet(what) => not_yet(what),
            Sched71Error::OutOfScratch => CompileError::from(V3dError::OutOfScratch),
            _ => not_yet("7.1 scheduling refused the program"),
        },
        V3d71Error::Encode(..) => not_yet("a 7.1 instruction does not encode"),
        V3d71Error::Verify(v) => CompileError::from(V3dError::Verify(v)),
    }
}

/// [`codegen::compile_program`] at 7.1: the two stages' IR and a key in, the
/// program's 7.1 code and its ABI record out, as the `ShcShader` that
/// replaces the program's fragment shader.
pub fn compile_program71_shader(
    vs: &crate::shaderc::online::lower::Lowered<'_>,
    fs: &crate::shaderc::online::lower::Lowered<'_>,
    key: ProgramKey,
    scratch: &Arena<'_>,
    keep: &'static Arena<'static>,
) -> Result<ShcShader, CompileError> {
    /* the TLB blends: no blend variant exists on this GPU */
    if key.blend_key != 0 {
        return Err(CompileError::NotImplemented(codegen::Stage::CodeGen));
    }
    /* chain71 compiles all three (the 4.2 chain refuses Lines and Points) */
    let prim = match key.prim {
        codegen::ProgramPrim::Triangles => Prim::Triangles,
        codegen::ProgramPrim::Lines => Prim::Lines,
        codegen::ProgramPrim::Points => Prim::Points,
    };
    let fs_stage =
        v3d_abi::stage_shader(crate::shaderc::SHC_STAGE_FRAGMENT, &fs.module, &fs.reflection, scratch, keep)?;
    /* no rt_f32 parameter at 7.1 yet: a draw-time f32 variant is refused,
     * never built as the plain program */
    if crate::shaderc::compile_rt_f32() != 0 {
        return Err(V3dError::NotYet(RT_F32_NOT_YET).into());
    }
    let prog = chain71::compile_program71_keyed(
        &vs.module,
        &fs.module,
        prim,
        chain71::TARGET71,
        key.vs_lod,
        crate::shaderc::compile_lod_key(),
        scratch,
    )
    .map_err(compile_error71)?;
    Ok(super::abi71::program_shader71(fs_stage, &prog, keep)?)
}

/// codegen.rs `lower_vertex_mapped`, copied (it is private there): the
/// vertex shader's front end with each attribute's loads and reflection row
/// moved to the location the link decided.
fn lower_vertex_mapped71<'a>(
    glsl: &[u8],
    map: Option<&crate::shaderc::ShcAttribMap>,
    scratch: &'a Arena<'a>,
) -> Result<crate::shaderc::online::lower::Lowered<'a>, CompileError> {
    let l = codegen::lower_source(crate::shaderc::SHC_STAGE_VERTEX, glsl, &[], scratch)?;
    let Some(map) = map else {
        return Ok(l);
    };
    let crate::shaderc::online::lower::Lowered { module, reflection } = l;
    let module = scratch.alloc_copy(core::slice::from_ref(&module))?;
    let (m, rows) = crate::shaderc::online::powervr::abi::with_attrib_map(scratch, &module[0], reflection.attribs, map)
        .map_err(|e| CompileError::BackEnd(BackEndError::Attrib(e)))?;
    Ok(crate::shaderc::online::lower::Lowered {
        module: *m,
        reflection: crate::shaderc::online::reflect::Reflection { attribs: rows, ..reflection },
    })
}

/// [`codegen::link_sources`] at 7.1: the same arguments and the same
/// answer, the program compiled by [`compile_program71_shader`].
pub fn link_sources71<'s>(
    vs_glsl: &[u8],
    vs_map: Option<&crate::shaderc::ShcAttribMap>,
    vs_key: crate::shaderc::LodKey,
    fs_glsl: &[u8],
    zero: &[&str],
    scratch: &'s Arena<'s>,
    keep: &'static Arena<'static>,
) -> Result<(ShcShader, Blocks<'s>), CompileError> {
    let vs = lower_vertex_mapped71(vs_glsl, vs_map, scratch)?;
    /* the vertex stage's c = 0.5 remap (`lodkey`): in the vertex stage's key
     * only, the fragment stage's explicit levels are not remapped */
    let vs = crate::shaderc::online::lodkey::remap_mag_switch(scratch, vs, vs_key.mag_switch)
        .map_err(|e| CompileError::Rejected { line: e.line, tok: Token::new(e.tok), msg: e.msg })?;
    let fs = codegen::lower_source(crate::shaderc::SHC_STAGE_FRAGMENT, fs_glsl, zero, scratch)?;
    let sh = compile_program71_shader(&vs, &fs, ProgramKey { vs_lod: vs_key, ..ProgramKey::LINK }, scratch, keep)?;
    Ok((sh, (fs.reflection.blocks, fs.reflection.block_members, [vs.reflection.uniforms, fs.reflection.uniforms])))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::codegen::ProgramPrim;
    use crate::shaderc::online::store;
    use crate::shaderc::{LodKey, ShcAttribMap};

    const VS: &[u8] = b"attribute vec4 a_pos;
attribute vec2 a_uv;
uniform mat4 u_mvp;
varying vec2 v_uv;
void main() { v_uv = a_uv; gl_Position = u_mvp * a_pos; }
";
    const FS: &[u8] = b"precision mediump float;
uniform vec4 u_tint;
varying vec2 v_uv;
void main() { gl_FragColor = u_tint * vec4(v_uv, 0.0, 1.0); }
";
    const FS_PNTC: &[u8] = b"precision mediump float;
uniform vec4 u_tint;
varying vec2 v_uv;
void main() { gl_FragColor = u_tint * vec4(gl_PointCoord, v_uv.x, 1.0); }
";

    fn name(n: &[core::ffi::c_char]) -> String {
        n.iter().take_while(|&&c| c != 0).map(|&c| c as u8 as char).collect()
    }

    /// The code words of the record's fs, vs and cs.
    fn words(sh: &ShcShader) -> [std::vec::Vec<u64>; 3] {
        // SAFETY: the record's `usc_dw` dwords, kept alive by the open keep.
        let dw = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
        let get = |off: u32, bytes: u32| {
            let (a, n) = (off as usize / 4, bytes as usize / 4);
            dw[a..a + n].chunks(2).map(|c| c[0] as u64 | (c[1] as u64) << 32).collect()
        };
        let p = &sh.v3d;
        [get(p.fs.code_off, p.fs.code_bytes), get(p.vs.code_off, p.vs.code_bytes), get(p.cs.code_off, p.cs.code_bytes)]
    }

    /// Names and locations a record carries: attributes, varyings.
    type Names = (std::vec::Vec<(String, u8)>, std::vec::Vec<(String, u8)>);

    fn names(sh: &ShcShader) -> Names {
        let attribs = sh.attribs[..sh.num_attribs as usize].iter().map(|a| (name(&a.name), a.location)).collect();
        let varyings = sh.varyings[..sh.num_varyings as usize].iter().map(|v| (name(&v.name), v.location)).collect();
        (attribs, varyings)
    }

    /// The record and the uniform rows' counts (vertex, fragment).
    fn link(vs: &[u8], fs: &[u8], map: Option<&ShcAttribMap>) -> Result<(ShcShader, [usize; 2]), CompileError> {
        store::with_scratch(|a| {
            link_sources71(vs, map, LodKey::NONE, fs, &[], a, store::keep())
                .map(|(s, b)| (s, [b.2[0].len(), b.2[1].len()]))
        })
        .expect("scratch")
    }

    fn link42(vs: &[u8], fs: &[u8]) -> Result<(ShcShader, [usize; 2]), CompileError> {
        store::with_scratch(|a| {
            codegen::link_sources(vs, None, LodKey::NONE, fs, &[], a, store::keep())
                .map(|(s, b)| (s, [b.2[0].len(), b.2[1].len()]))
        })
        .expect("scratch")
    }

    /// The fragment words of the pair through chain71 directly.
    fn direct(fs_src: &[u8], prim: Prim) -> std::vec::Vec<u64> {
        store::with_scratch(|a| {
            let v = codegen::lower_source(crate::shaderc::SHC_STAGE_VERTEX, VS, &[], a).unwrap();
            let f = codegen::lower_source(crate::shaderc::SHC_STAGE_FRAGMENT, fs_src, &[], a).unwrap();
            let p = chain71::compile_program71_keyed(
                &v.module,
                &f.module,
                prim,
                chain71::TARGET71,
                LodKey::NONE,
                crate::shaderc::compile_lod_key(),
                a,
            )
            .expect("chain71");
            p.fs.code.words.to_vec()
        })
        .expect("scratch")
    }

    #[test]
    fn a_pair_links_to_the_chains_words() {
        let _k = store::test_keep();
        let (sh, _) = link(VS, FS, None).expect("links");
        assert!(sh.usc_dw > 0);
        assert_eq!(words(&sh)[0], direct(FS, Prim::Triangles));
    }

    #[test]
    fn a_bad_pair_is_an_error_not_a_panic() {
        let _k = store::test_keep();
        /* no main in either stage */
        assert!(link(b"attribute vec4 a;\n", FS, None).is_err());
        assert!(link(VS, b"precision mediump float;\nuniform vec4 u;\n", None).is_err());
        assert!(link(b"", b"", None).is_err());
        assert!(link(b"void main( {", FS, None).is_err());
        /* a varying the fragment shader declares with another type: whatever
         * the answer, no panic */
        let fs = b"precision mediump float;\nvarying vec3 v_uv;\nvoid main() { gl_FragColor = vec4(v_uv, 1.0); }\n";
        let _ = link(VS, fs, None);
    }

    #[test]
    fn the_prim_keys_reach_the_chain() {
        let _k = store::test_keep();
        let build = |fs: &[u8], prim: ProgramPrim| {
            store::with_scratch(|a| {
                let vs = codegen::lower_source(crate::shaderc::SHC_STAGE_VERTEX, VS, &[], a).unwrap();
                let fs = codegen::lower_source(crate::shaderc::SHC_STAGE_FRAGMENT, fs, &[], a).unwrap();
                let key = ProgramKey { prim, ..ProgramKey::LINK };
                compile_program71_shader(&vs, &fs, key, a, store::keep()).map(|sh| words(&sh))
            })
            .expect("scratch")
        };
        for fs in [FS, FS_PNTC] {
            let tri = build(fs, ProgramPrim::Triangles).expect("triangles");
            assert_eq!(tri[0], direct(fs, Prim::Triangles));
            for (k, p) in [(ProgramPrim::Lines, Prim::Lines), (ProgramPrim::Points, Prim::Points)] {
                let got = build(fs, k).unwrap_or_else(|e| panic!("{k:?}: {e:?}"));
                assert_eq!(got[0], direct(fs, p), "{k:?}");
                assert_eq!(got[1], tri[1], "{k:?}: the vertex stage does not depend on the primitive");
            }
        }
        /* Points is word for word Triangles; Lines of a gl_PointCoord reader is not */
        let t = build(FS_PNTC, ProgramPrim::Triangles).unwrap();
        let p = build(FS_PNTC, ProgramPrim::Lines).unwrap();
        assert_ne!(t[0], p[0]);
        /* the blend variants are refused before the chain runs */
        let r = store::with_scratch(|a| {
            let vs = codegen::lower_source(crate::shaderc::SHC_STAGE_VERTEX, VS, &[], a).unwrap();
            let fs = codegen::lower_source(crate::shaderc::SHC_STAGE_FRAGMENT, FS, &[], a).unwrap();
            let key = ProgramKey { blend_key: 3, ..ProgramKey::LINK };
            compile_program71_shader(&vs, &fs, key, a, store::keep()).map(|_| ())
        })
        .expect("scratch");
        assert_eq!(r, Err(CompileError::NotImplemented(codegen::Stage::CodeGen)));
    }

    #[test]
    fn the_names_match_the_4_2_entry() {
        let _k = store::test_keep();
        let (s71, n71) = link(VS, FS, None).expect("7.1");
        let (s42, n42) = link42(VS, FS).expect("4.2");
        let (a, b) = (names(&s71), names(&s42));
        assert_eq!(a, b);
        assert_eq!(a.1.len(), 1);
        assert_eq!(a.1[0].0, "v_uv");
        assert_eq!(n71, n42);
        assert_eq!(
            (s71.num_varyings, s71.num_uniforms, s71.uniform_dwords, s71.num_outputs, s71.num_attribs),
            (s42.num_varyings, s42.num_uniforms, s42.uniform_dwords, s42.num_outputs, s42.num_attribs)
        );
        assert_eq!(s71.v3d.record_of_location, s42.v3d.record_of_location);
    }

    #[test]
    fn the_error_names_are_the_7_1_ones() {
        use super::super::verify::VerifyError;
        let v = VerifyError { ip: 1, rule: "r" };
        assert_eq!(compile_error71(V3d71Error::NotYet("x")), CompileError::from(V3dError::NotYet("x")));
        assert_eq!(compile_error71(V3d71Error::OutOfScratch), CompileError::from(V3dError::OutOfScratch));
        assert_eq!(compile_error71(V3d71Error::Verify(v)), CompileError::from(V3dError::Verify(v)));
        assert_eq!(
            compile_error71(V3d71Error::Alloc(Alloc71Error::DoesNotFit)),
            CompileError::from(V3dError::NotYet(
                "more live values than 2 threads have registers at 7.1 (no spilling yet)"
            ))
        );
    }
}
