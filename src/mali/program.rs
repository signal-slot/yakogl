// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A compiled stage as the frame builder takes it: the Valhall back end's
//! record ([`MaliShader`], src/shaderc/online/mali/) turned into the
//! [`ShaderMeta`] of src/mali/frame.rs, and its code as bytes.
//!
//! The FAU words keep the compiler's order (Mesa uploads `fau->words` in
//! order, Mesa 26.2.1 pan_cmdstream.c:1580-1651): a sysval word becomes a
//! relocation of the stage's sysval block, a promoted immediate a constant.
//! The sysval block is Mesa's: a fragment shader's first slot is the blend
//! constants whether it blends or not (pan_nir_lower_sysvals.c:258-261),
//! then the viewport scale and offset in the order the compiler's
//! `Sysval::base_word` gives them (scale, offset).

use super::frame::{FrameError, PushWord, SecondaryProgram, ShaderMeta, Sysval, UboRow};
use super::genxml::v10::{ShaderRegisterAllocation, ShaderStage};
use crate::shaderc::online::mali::fau::{FauWord, FAU_MAX_WORDS};
use crate::shaderc::online::mali::select::MAX_UBO_ROWS;
use crate::shaderc::online::mali::{bir, MaliShader, Secondary, Section};
use crate::shaderc::{ShcShader, ShcUniform, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX, SHC_UNIFORM_NO_BLOCK};

/// The record names another stage than the one it is used as, or its code
/// is not the record's (a compiler or caller defect).
pub const INVALID_RECORD: &str = "mali program: the stage's record and its use or its code do not agree";

/// `pan_register_allocation` (Mesa 26.2.1 src/panfrost/lib/pan_shader.h:33-38).
pub fn register_allocation(work_reg_count: u8) -> ShaderRegisterAllocation {
    if work_reg_count <= 32 {
        ShaderRegisterAllocation::_32_PER_THREAD
    } else {
        ShaderRegisterAllocation::_64_PER_THREAD
    }
}

/// The sysvals a stage's block can hold, in block order.
const SYSVALS: [Sysval; 3] = [Sysval::BlendConstants, Sysval::ViewportScale, Sysval::ViewportOffset];

/// A stage's meta, with the arrays the frame builder borrows. It copies what
/// it takes from the record, so it does not borrow the record.
#[derive(Clone, Copy, Debug)]
pub struct StageMeta {
    stage: ShaderStage,
    /// The record's preloaded r48-r63 ([`MaliShader::preload_r48_r63`]).
    preload: u32,
    work_reg_count: u8,
    secondary: Option<Secondary>,
    no_psiz_offset: Option<u32>,
    push: [PushWord; FAU_MAX_WORDS],
    npush: usize,
    sysvals: [Sysval; 3],
    nsysvals: usize,
    /// The uniform block the shader loads ([`MaliShader::ubo`]).
    ubo: [UboRow; MAX_UBO_ROWS],
    nubo: usize,
    varyings: u32,
    varying_bytes: u32,
    samples_texture: bool,
    helper_threads: bool,
    /// The fragment program discards ([`MaliShader::can_discard`]).
    can_discard: bool,
    /// The fragment program's colour outputs, bit i = render target i
    /// ([`MaliShader::colour_outputs`]); 0 for a vertex shader.
    colour_outputs: u32,
}

impl StageMeta {
    /// The meta of a vertex (`SHC_STAGE_VERTEX`) or fragment record.
    pub fn of(record: &MaliShader) -> Result<StageMeta, FrameError> {
        let stage = match record.stage {
            SHC_STAGE_VERTEX => ShaderStage::VERTEX,
            SHC_STAGE_FRAGMENT => ShaderStage::FRAGMENT,
            _ => return Err(FrameError::Invalid(INVALID_RECORD)),
        };
        if record.fau.len() > FAU_MAX_WORDS || record.ubo.len() > MAX_UBO_ROWS {
            return Err(FrameError::Invalid(INVALID_RECORD));
        }
        let mut ubo = [UboRow { dword: 0, word: 0, comps: 0, halves: false }; MAX_UBO_ROWS];
        for (i, r) in record.ubo.iter().enumerate() {
            ubo[i] = UboRow {
                dword: usize::from(r.dword),
                word: usize::from(r.word),
                comps: usize::from(r.comps),
                halves: r.halves,
            };
        }
        let kind = |w: &FauWord| match *w {
            FauWord::BlendConstant(_) => Some(Sysval::BlendConstants),
            FauWord::Sysval(bir::Sysval::ViewportScale, _) => Some(Sysval::ViewportScale),
            FauWord::Sysval(bir::Sysval::ViewportOffset, _) => Some(Sysval::ViewportOffset),
            FauWord::Constant(_)
            | FauWord::Uniform { .. }
            | FauWord::UniformHalves { .. }
            | FauWord::FbWposYTransform(_) => None,
        };
        let mut sysvals = [Sysval::BlendConstants; 3];
        let mut nsysvals = 0;
        for s in SYSVALS {
            let used = record.fau.iter().any(|w| kind(w) == Some(s));
            if used || (s == Sysval::BlendConstants && stage == ShaderStage::FRAGMENT) {
                sysvals[nsysvals] = s;
                nsysvals += 1;
            }
        }
        let mut push = [PushWord::Constant(0); FAU_MAX_WORDS];
        for (i, w) in record.fau.iter().enumerate() {
            push[i] = match *w {
                FauWord::Constant(c) => PushWord::Constant(c),
                FauWord::Uniform { dword } => PushWord::Uniform { dword: usize::from(dword) },
                FauWord::UniformHalves { lo, hi } => {
                    PushWord::UniformHalves { lo: usize::from(lo), hi: hi.map(usize::from) }
                }
                FauWord::FbWposYTransform(c) => PushWord::FbWposYTransform { component: usize::from(c) },
                FauWord::BlendConstant(c) | FauWord::Sysval(_, c) => {
                    let k = kind(w).expect("a sysval word");
                    let sysval = sysvals[..nsysvals].iter().position(|s| *s == k).expect("listed above");
                    PushWord::Sysval { sysval, component: usize::from(c) }
                }
            };
        }
        let generic = record.varyings.iter().filter(|v| v.section == Section::Generic);
        /* a vertex row of no component is a placeholder (every lane folded to
         * a constant Mesa's link moves into the fragment shader; the layout
         * writes nothing for it): not a varying written, as
         * pan_nir_collect_varyings.c counts only the slots it stores */
        let varyings = generic.clone().filter(|v| stage == ShaderStage::FRAGMENT || v.comps != 0).count() as u32;
        let varying_bytes = if stage == ShaderStage::VERTEX {
            generic.map(|v| u32::from(v.offset) + 4 * u32::from(v.comps)).max().unwrap_or(0)
        } else {
            0
        };
        Ok(StageMeta {
            stage,
            preload: record.preload_r48_r63(),
            work_reg_count: record.work_reg_count,
            secondary: record.secondary,
            no_psiz_offset: record.no_psiz_offset,
            push,
            npush: record.fau.len(),
            sysvals,
            nsysvals,
            ubo,
            nubo: record.ubo.len(),
            varyings,
            varying_bytes,
            samples_texture: record.sampler.is_some(),
            helper_threads: record.helper_threads,
            can_discard: record.can_discard,
            /* target 0 always: the frame needs it (its alpha is ATEST's),
             * as the meta said before the record carried the others */
            colour_outputs: if stage == ShaderStage::FRAGMENT { record.colour_outputs | 1 } else { 0 },
        })
    }

    /// The frame builder's meta.
    pub fn meta(&self) -> ShaderMeta<'_> {
        ShaderMeta {
            stage: self.stage,
            register_allocation: register_allocation(self.work_reg_count),
            preload: self.preload,
            sysvals: &self.sysvals[..self.nsysvals],
            push: &self.push[..self.npush],
            ubo: &self.ubo[..self.nubo],
            varyings: self.varyings,
            varying_bytes: self.varying_bytes,
            secondary: self.secondary.map(|s| SecondaryProgram {
                offset: s.offset,
                preload: (s.preload >> 48) as u32,
                register_allocation: register_allocation(s.work_reg_count),
            }),
            no_psiz_offset: self.no_psiz_offset,
            /* the back end writes gl_FragColor or `layout(location = i)`
             * outputs, each its own BLEND (frame.rs: DCD1's render target
             * mask, refused past the frame's targets); it discards where the
             * source does (nir_fs.rs `Fs::discard`) and writes neither
             * depth, stencil, coverage nor memory */
            colour_outputs: self.colour_outputs,
            samples_texture: self.samples_texture,
            requires_helper_threads: self.helper_threads,
            can_discard: self.can_discard,
            writes_depth: false,
            writes_stencil: false,
            writes_coverage: false,
            writes_global: false,
        }
    }
}

/// A compiled stage's code as the GPU reads it: the record's `usc` words
/// (32-bit, little endian), as bytes.
///
/// # Safety
/// `sh.usc` points at `sh.usc_dw` words that live as long as `sh` (the
/// compiler's keep arena).
pub unsafe fn code_bytes(sh: &ShcShader) -> Result<&[u8], FrameError> {
    if sh.usc.is_null() || sh.usc_dw == 0 {
        return Err(FrameError::Invalid(INVALID_RECORD));
    }
    // SAFETY: the caller's contract.
    Ok(unsafe { core::slice::from_raw_parts(sh.usc.cast::<u8>(), 4 * sh.usc_dw as usize) })
}

/// Room for a stage's uniform words as the GL core hands them: its block
/// (`MAX_UNIFORM_DWORDS`) and a blend variant's constants.
pub const UNIFORM_WORDS_MAX: usize = crate::gl::context::MAX_UNIFORM_DWORDS + crate::gl::context::BLEND_CONSTS_DWORDS;

/// A stage's uniform words as Mesa's panfrost uploads them: the GL core
/// stores a `bool` as 0 or 1 (every back end reads that), Mesa 26.2.1 stores
/// a true one as `~0` and the compiled shader relies on it (a mediump
/// compare is `ICMP_OR.v2u16.eq.m1 r, mask.h00, u.w.h00`, a 16-bit mask
/// against the word's low half; docs/notes/mali-deqp-boolpix.md, cause BW).
/// Every word of a `bool`, `bvecN`, array or struct member of them (the
/// table's rows are the leaves) that is not 0 becomes `0xffffffff`, in
/// `buf`; the other words, and the padding words (0), are as they were.
/// Returns `words` itself when the table names no bool or `words` does not
/// fit `buf`.
pub fn mali_uniform_words<'a>(
    table: &[ShcUniform],
    words: &'a [u32],
    buf: &'a mut [u32; UNIFORM_WORDS_MAX],
) -> &'a [u32] {
    use crate::gl::consts::{GL_BOOL, GL_BOOL_VEC2, GL_BOOL_VEC3, GL_BOOL_VEC4};
    let is_bool = |u: &ShcUniform| {
        matches!(u32::from(u.gl_type), GL_BOOL | GL_BOOL_VEC2 | GL_BOOL_VEC3 | GL_BOOL_VEC4)
            && u.dword_off != SHC_UNIFORM_NO_BLOCK
    };
    if words.len() > buf.len() || !table.iter().any(is_bool) {
        return words;
    }
    let out = &mut buf[..words.len()];
    out.copy_from_slice(words);
    for u in table.iter().filter(|u| is_bool(u)) {
        let at = usize::from(u.dword_off);
        let end = (at + usize::from(u.dwords.max(1))).min(out.len());
        if at < end {
            for w in &mut out[at..end] {
                if *w != 0 {
                    *w = !0;
                }
            }
        }
    }
    out
}

/// The uniform table of a stage's shader, or none for a null record.
///
/// # Safety
/// `sh` is null or a live shader record.
pub unsafe fn uniform_table<'a>(sh: *const ShcShader) -> &'a [ShcUniform] {
    // SAFETY: the caller's contract.
    unsafe { sh.as_ref() }.map_or(&[], |s| s.uniform_table())
}

#[cfg(test)]
#[path = "program_tests.rs"]
mod tests;
