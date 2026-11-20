// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The linked program's `ShcShader` for the V3D 7.1 (the Raspberry Pi 5's
//! BCM2712 D0, V3D 7.1.10; `gpu-v3d` builds, beside the 4.2 [`super::v3d_abi`]).
//!
//! [`program_shader71`] is [`super::v3d_abi::program_shader`] for a
//! [`V3dProgram71`]: the same record, laid out the same way, so the GL core
//! and a 7.1 table read it as they read a 4.2 one. A compiled stage
//! (glCompileShader) has no code and no generation, so it stays
//! [`super::v3d_abi::stage_shader`]'s; only the link differs.
//!
//! # The record, field by field
//!
//! * `usc` / `usc_dw`: FS, VS, CS back to back, each whole 64-bit words
//!   (little endian, low dword first), so each starts at a multiple of 8
//!   bytes. **The words are 7.1 encodings** (`codec71::encode71`): they do
//!   not decode as 4.2 instructions, and a 4.2 table must never upload them.
//!   The record says nothing of the generation; the build that links it
//!   (the 7.1 table) knows it.
//! * `v3d.fs/vs/cs.code_off`, `code_bytes`: as at 4.2. The 7.1.10 draw-index
//!   record's code addresses are 29-bit fields holding address bits 3..31
//!   (src/v3d/record71.rs `Kind::Address`), so the 8-byte alignment of each
//!   program is what that record needs too.
//! * `uniforms` / `num_uniforms`: the uniform stream, `(contents, data)` with
//!   Mesa's `quniform_contents` numbering, which 7.1 shares. At 7.1 no entry
//!   points into a default-attribute BO (there is none: Mesa's
//!   `v3d_device_needs_default_attribute_values` is `ver <= 42`); the
//!   streams of the six gl_test programs are tools/v3d/gltest-shaders71's.
//! * `fs.num_inputs`: the record's `number_of_varyings_in_fragment_shader`.
//! * `fs.flat_shade_flags*`, `noperspective_flags*`, `centroid_flags*`: not
//!   in the 7.1.10 record (it has none); the separate `FLAT_SHADE_FLAGS`,
//!   `NON_PERSPECTIVE_FLAGS` and `CENTROID_FLAGS` packets carry them, as at
//!   4.2.
//! * `fs.threads`, `vs.threads`, `cs.threads`: 4 sets the record's
//!   `*_4_way_threadable`; v3d_job71.c accepts 1, 2 or 4.
//! * `*.single_seg`: the record's `*_start_in_final_thread_section`.
//! * `fs.writes_z`, `fs.disable_ez`: copied into the record's
//!   `fragment_shader_does_z_writes` and `turn_off_early_z_test`
//!   (docs/notes/v3d71-record-u11.md section 4 item 4; `Never defer FEP
//!   depth writes` stays 0 and is not the program's).
//! * `fs.uses_center_w`, `fs.uses_pid`, `fs.lock_scoreboard_on_first_thrsw`:
//!   the record's bits of the same meaning (`uses_pid` sets both primitive-id
//!   bits). `fs.uses_implicit_point_line_varyings`: the record's
//!   `disable_implicit_point_line_varyings` is its inverse; v3d_job71.c
//!   refuses a set bit on anything but points (chain71 compiles triangles
//!   only, so it is always 0 here).
//! * `fs.rt_mask`: as at 4.2 (the render targets the program writes).
//! * `vs/cs.vpm_input_size`, `vpm_output_size`: the record's VPM segment
//!   sizes. `vcm_cache_size`: the `VCM_CACHE_SIZE` packet's batch counts, as
//!   at 4.2.
//! * `vs/cs.separate_segments`: **4.2 only.** The 7.1 record has no
//!   separate-segments flag (a non-zero input size is the separate case).
//!   chain71 always gives `false` and it is written as `false`, which is
//!   what v3d_job71.c `vs_ok71` requires (it refuses a set flag).
//! * `vs/cs.uses_biid`: the D0 record has base-instance bits Mesa 26.2.1
//!   never sets; chain71 gives `false`, written `false` (v3d_job71.c refuses
//!   a set one).
//! * `vs/cs.uses_vid`, `uses_iid`: the record's `vertex_id_read_by_*` and
//!   `instance_id_read_by_*`.
//! * The draw-index record's `*_basevertex` and `*_drawindex` bits
//!   (record71.rs `CS_BASEVERTEX`.. `VS_DRAWINDEX`) have no field here: the
//!   ES 2.0 / 3.0 programs read neither gl_BaseVertex nor gl_DrawID, so they
//!   are 0 (record71.rs rule 3, docs/notes/v3d71-record-u11.md section 4).
//! * `vs/cs.writes_psiz`: the record's `point_size_in_shaded_vertex_data`
//!   (from the vertex program, v3d_job71.c `vs_writes_psiz`).
//! * `vs/cs.vattr_sizes`, `num_records`, `record_of_location`: attribute
//!   record i reads `vattr_sizes[i]` values, as at 4.2 (v3d_job71.h `attrs`).
//!   A read location whose array is disabled has no default-attribute BO to
//!   fall back on at 7.1: the table gives it a stride-0 record of its own
//!   (src/v3d/attr71.rs, "A disabled attribute the program reads").
//! * `prim`: [`SHC_V3D_PRIM_TRIANGLES`], the only class chain71 compiles.
//!
//! A value that does not fit a field is refused by name
//! ([`V3dAbiError`]), never truncated; everything the record points at is in
//! the keep arena.

use super::arena::Arena;
use super::v3d::chain71::{V3dProgram71, VsProg71};
use super::v3d::select::Uniform;
use super::v3d_abi::V3dAbiError;
use crate::shaderc::{ShcShader, ShcV3d, ShcV3dFs, ShcV3dUniform, ShcV3dVs, SHC_V3D_NO_RECORD, SHC_V3D_PRIM_TRIANGLES};

/// The uniform stream of one program, in keep (v3d_abi.rs `stream`).
fn stream71(u: &[Uniform], keep: &'static Arena<'static>) -> Result<(*const ShcV3dUniform, u32), V3dAbiError> {
    if u.is_empty() {
        return Ok((core::ptr::null(), 0));
    }
    let out = keep.alloc_filled(u.len(), ShcV3dUniform { contents: 0, data: 0 }).map_err(|_| V3dAbiError::OutOfKeep)?;
    for (d, s) in out.iter_mut().zip(u) {
        *d = ShcV3dUniform { contents: s.contents, data: s.data };
    }
    Ok((out.as_ptr(), u32::try_from(u.len()).map_err(|_| V3dAbiError::TooLarge)?))
}

/// A vertex or coordinate program's half (v3d_abi.rs `vs_half`).
fn vs_half71(p: &VsProg71<'_>, code_off: u32, keep: &'static Arena<'static>) -> Result<ShcV3dVs, V3dAbiError> {
    let (uniforms, num_uniforms) = stream71(p.code.uniforms, keep)?;
    Ok(ShcV3dVs {
        code_off,
        code_bytes: u32::try_from(p.code.words.len() * 8).map_err(|_| V3dAbiError::TooLarge)?,
        uniforms,
        num_uniforms,
        vpm_input_size: p.vpm_input_size,
        vpm_output_size: p.vpm_output_size,
        vcm_cache_size: p.vcm_cache_size,
        threads: u8::try_from(p.threads).map_err(|_| V3dAbiError::TooLarge)?,
        single_seg: u8::from(p.single_seg),
        /* 4.2 only; chain71 gives false (see the module comment) */
        separate_segments: u8::from(p.separate_segments),
        uses_vid: u8::from(p.uses_vid),
        uses_iid: u8::from(p.uses_iid),
        uses_biid: u8::from(p.uses_biid),
        writes_psiz: u8::from(p.writes_psiz),
        vattr_sizes: p.vattr_sizes,
    })
}

/// The linked 7.1 program: `fs_stage` (the fragment stage's record, built by
/// [`super::v3d_abi::stage_shader`] in the same keep transaction) with the
/// 7.1 code and the V3D half added, in [`super::v3d_abi::program_shader`]'s
/// layout (the module comment says what each field means at 7.1).
pub fn program_shader71(
    fs_stage: ShcShader,
    p: &V3dProgram71<'_>,
    keep: &'static Arena<'static>,
) -> Result<ShcShader, V3dAbiError> {
    let mut sh = fs_stage;
    let parts = [p.fs.code.words, p.vs.code.words, p.cs.code.words];
    let total: usize = parts.iter().map(|w| w.len()).sum();
    let dwords = keep.alloc_filled(total * 2, 0u32).map_err(|_| V3dAbiError::OutOfKeep)?;
    let mut at = 0usize;
    let mut offs = [0u32; 3];
    for (k, words) in parts.iter().enumerate() {
        offs[k] = u32::try_from(at * 4).map_err(|_| V3dAbiError::TooLarge)?;
        for w in words.iter() {
            /* little endian, as the QPU fetches a 64-bit word */
            dwords[at] = *w as u32;
            dwords[at + 1] = (*w >> 32) as u32;
            at += 2;
        }
    }
    sh.usc = dwords.as_ptr();
    sh.usc_dw = u32::try_from(dwords.len()).map_err(|_| V3dAbiError::TooLarge)?;

    let (fs_uniforms, fs_num_uniforms) = stream71(p.fs.code.uniforms, keep)?;
    let fs = ShcV3dFs {
        code_off: offs[0],
        code_bytes: u32::try_from(p.fs.code.words.len() * 8).map_err(|_| V3dAbiError::TooLarge)?,
        uniforms: fs_uniforms,
        num_uniforms: fs_num_uniforms,
        num_inputs: p.fs.num_inputs,
        flat_shade_flags: p.fs.flat_shade_flags,
        noperspective_flags: p.fs.noperspective_flags,
        centroid_flags: p.fs.centroid_flags,
        flat_shade_flags_hi: p.fs.flat_shade_flags_hi,
        noperspective_flags_hi: p.fs.noperspective_flags_hi,
        centroid_flags_hi: p.fs.centroid_flags_hi,
        threads: u8::try_from(p.fs.threads).map_err(|_| V3dAbiError::TooLarge)?,
        single_seg: u8::from(p.fs.single_seg),
        writes_z: u8::from(p.fs.writes_z),
        disable_ez: u8::from(p.fs.disable_ez),
        uses_center_w: u8::from(p.fs.uses_center_w),
        lock_scoreboard_on_first_thrsw: u8::from(p.fs.lock_scoreboard_on_first_thrsw),
        uses_implicit_point_line_varyings: u8::from(p.fs.uses_implicit_point_line_varyings),
        uses_pid: u8::from(p.fs.uses_pid),
        rt_mask: p.fs.rt_mask,
    };

    let mut record_of_location = [SHC_V3D_NO_RECORD; 16];
    let mut num_records = 0u8;
    for &(loc, rec) in p.attributes {
        if loc >= 16 {
            return Err(V3dAbiError::AttribLocation { location: loc });
        }
        if let Some(r) = rec {
            record_of_location[loc as usize] = r;
            num_records = num_records.max(r + 1);
        }
    }
    sh.v3d = ShcV3d {
        linked: 1,
        prim: SHC_V3D_PRIM_TRIANGLES,
        num_records,
        record_of_location,
        fs,
        vs: vs_half71(&p.vs, offs[1], keep)?,
        cs: vs_half71(&p.cs, offs[2], keep)?,
    };
    Ok(sh)
}
