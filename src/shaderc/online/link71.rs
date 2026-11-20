// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GL link entry for the V3D 7.1 (the Raspberry Pi 5's BCM2712 D0, V3D
//! 7.1.10): [`codegen::link_sources`] and [`codegen::compile_program`] with
//! the 7.1 chain ([`chain71::compile_program71_keyed`]) and the 7.1
//! packaging ([`super::v3d_abi71::program_shader71`]) in place of the 4.2
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

use super::arena::Arena;
use super::codegen::{self, BackEndError, Blocks, CompileError, ProgramKey, Token};
use super::v3d::alloc71::Alloc71Error;
use super::v3d::chain::{Prim, V3dError};
use super::v3d::chain71::{self, V3d71Error};
use super::v3d::sched71::Sched71Error;
use super::v3d::select71::Select71Error;
use super::v3d_abi;
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
    vs: &super::lower::Lowered<'_>,
    fs: &super::lower::Lowered<'_>,
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
    Ok(super::v3d_abi71::program_shader71(fs_stage, &prog, keep)?)
}

/// codegen.rs `lower_vertex_mapped`, copied (it is private there): the
/// vertex shader's front end with each attribute's loads and reflection row
/// moved to the location the link decided.
fn lower_vertex_mapped71<'a>(
    glsl: &[u8],
    map: Option<&crate::shaderc::ShcAttribMap>,
    scratch: &'a Arena<'a>,
) -> Result<super::lower::Lowered<'a>, CompileError> {
    let l = codegen::lower_source(crate::shaderc::SHC_STAGE_VERTEX, glsl, &[], scratch)?;
    let Some(map) = map else {
        return Ok(l);
    };
    let super::lower::Lowered { module, reflection } = l;
    let module = scratch.alloc_copy(core::slice::from_ref(&module))?;
    let (m, rows) = super::powervr::abi::with_attrib_map(scratch, &module[0], reflection.attribs, map)
        .map_err(|e| CompileError::BackEnd(BackEndError::Attrib(e)))?;
    Ok(super::lower::Lowered { module: *m, reflection: super::reflect::Reflection { attribs: rows, ..reflection } })
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
    let vs = super::lodkey::remap_mag_switch(scratch, vs, vs_key.mag_switch).map_err(|e| CompileError::Rejected {
        line: e.line,
        tok: Token::new(e.tok),
        msg: e.msg,
    })?;
    let fs = codegen::lower_source(crate::shaderc::SHC_STAGE_FRAGMENT, fs_glsl, zero, scratch)?;
    let sh = compile_program71_shader(&vs, &fs, ProgramKey { vs_lod: vs_key, ..ProgramKey::LINK }, scratch, keep)?;
    Ok((sh, (fs.reflection.blocks, fs.reflection.block_members, [vs.reflection.uniforms, fs.reflection.uniforms])))
}
