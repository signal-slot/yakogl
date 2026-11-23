// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The linked program's `ShcShader` for the V3D 7.1 (the Raspberry Pi 5's
//! BCM2712 D0, V3D 7.1.10; `gpu-v3d` builds, beside the 4.2
//! [`crate::shaderc::online::v3d_abi`]).
//!
//! [`program_shader71`] is [`crate::shaderc::online::v3d_abi::program_shader`]
//! for a [`V3dProgram71`]: the same record, laid out the same way, so the GL
//! core and a 7.1 table read it as they read a 4.2 one. A compiled stage
//! (glCompileShader) has no code and no generation, so it stays
//! [`crate::shaderc::online::v3d_abi::stage_shader`]'s; only the link differs.
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
//!   refuses a set bit on anything but points (chain71 sets it only for a
//!   points program whose fragment shader reads `gl_PointCoord`).
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

use super::chain71::{V3dProgram71, VsProg71};
use super::select::Uniform;
use crate::shaderc::online::arena::Arena;
use crate::shaderc::online::v3d_abi::V3dAbiError;
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
/// [`crate::shaderc::online::v3d_abi::stage_shader`] in the same keep transaction) with the
/// 7.1 code and the V3D half added, in [`crate::shaderc::online::v3d_abi::program_shader`]'s
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

#[cfg(test)]
mod tests {
    //! [`program_shader71`]'s record, tested against the 4.2 contract it
    //! copies: small `tools/shaders/es100` programs are compiled at both
    //! generations, their records assembled by [`crate::shaderc::online::v3d_abi::program_shader`]
    //! and [`program_shader71`], and compared field for field on everything
    //! the two generations derive from the same shared front half (chain71.rs
    //! carries the same prepare / `select_fragment_keyed` / `select_vertex_keyed`
    //! / `vpm_fields` / attribute-record code as chain.rs, so the prog_data,
    //! the VPM figures and the attribute table must come out identical). The
    //! code blobs are then read back out of the keep arena and compared with
    //! the chain's own words, byte for byte, and the fields the 7.1 record
    //! pins by design (the module comment) are pinned here.
    //!
    //! The program's own outputs — its words, its uniform stream's load
    //! order, and the thread count its allocation chose — are each
    //! generation's, and are not compared across generations: each is
    //! checked against its own generation's chain ([`Both`]), stream entries
    //! compared as sets across generations with the measured differences
    //! pinned ([`STREAM_PINS`], [`THREAD_PINS`]), exact in both directions
    //! like the refusal lists.
    //!
    //! ```text
    //! cargo test --lib --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table abi71
    //! ```

    use super::*;
    use crate::shaderc::online::lower;
    use crate::shaderc::online::sema::Stage;
    use crate::shaderc::online::store;
    use crate::shaderc::online::v3d::{chain, chain71};
    use crate::shaderc::online::v3d_abi;
    use crate::shaderc::SHC_STAGE_FRAGMENT;
    use crate::util::name_bytes;
    use std::format;
    use std::string::String;
    use std::vec::Vec;

    /// 4.2's VPM as the 4.2 chain measures it (the figure `tools/v3d/v3d_shc`
    /// reports for the Pi 4; diff71.rs `TARGET42`).
    const TARGET42: chain::Target = chain::Target { vpm_size: 65536 };

    macro_rules! es100 {
        ($f:literal) => {
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tools/shaders/es100/", $f))
        };
    }

    /// The stub of the other stage for a single-stage source: a vertex shader
    /// that only places, and a fragment shader that paints `v_color`. Both
    /// generations are given the same text, as diff71.rs pairs its programs.
    const TRIVIAL_VS: &str = "#version 100\nattribute vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n";
    const READ_V_COLOR: &str =
        "#version 100\nprecision mediump float;\nvarying mediump vec4 v_color;\nvoid main() { gl_FragColor = v_color; }\n";

    /// `(name, vertex text, fragment text)`. Pairs that are paired in the
    /// tree keep their names; a single-stage source is paired with the stub.
    /// Each program puts a different record field to work:
    ///
    /// * `hello`: the plain program — a mat4 uniform block (16 dwords) and
    ///   one attribute record;
    /// * `clipping`: a varying, three attribute records, and `gl_PointSize`
    ///   (`writes_psiz`);
    /// * `matvary`: a `mat2` varying — two input slots for one declaration,
    ///   and the varying after it moves up by one;
    /// * `pointcoord`: `gl_PointCoord` — the two implicit point varyings
    ///   (`uses_implicit_point_line_varyings`, `num_inputs` 2 with no varying
    ///   declared);
    /// * `psiz_if`: `gl_PointSize` through an if (prepare's if-conversion),
    ///   a second path to `writes_psiz`;
    /// * `fragz`: `gl_FragCoord.z` (the Z read a `fragcoord` program takes)
    ///   beside a uniform;
    /// * `tri_discard`: a discard (the stage record's `uses_discard` and the
    ///   prog_data's `disable_ez`);
    /// * `textured`: a look-up (the uniform stream's texture entries,
    ///   `rt_mask`) and a varying the fragment stage declares-writes but
    ///   never reads, which is no input;
    /// * `unibranch`: a uniform branch — at 7.1 the scheduled build
    ///   (`schedule71`) rather than `pack71`, same record either way;
    /// * `attrib_hole`: sixteen attribute locations, `num_records` at its
    ///   bound, a float attribute (`vattr_sizes` 1) among vec4s — and the
    ///   program whose vertex and coordinate allocation our 4.2 answers 2
    ///   threads to and alloc71 4 ([`THREAD_PINS`]);
    /// * `varyings15`: fifteen vec4 varyings — the input-count end; it is
    ///   the program spill71 opened at 7.1 by making rf1..rf3 reusable
    ///   after the fragment payload reads.
    static PAIRS: &[(&str, &str, &str)] = &[
        ("hello", es100!("hello.vert"), es100!("hello.frag")),
        ("clipping", es100!("clipping.vert"), es100!("clipping.frag")),
        ("matvary", es100!("matvary.vert"), es100!("matvary.frag")),
        ("pointcoord", TRIVIAL_VS, es100!("pointcoord.frag")),
        ("psiz_if", es100!("psiz_if.vert"), READ_V_COLOR),
        ("fragz", TRIVIAL_VS, es100!("fragz.frag")),
        ("tri_discard", es100!("tri_discard.vert"), es100!("tri_discard.frag")),
        ("textured", es100!("textured.vert"), es100!("textured.frag")),
        ("unibranch", TRIVIAL_VS, es100!("unibranch.frag")),
        ("attrib_hole", es100!("attrib_hole_float.vert"), READ_V_COLOR),
        ("varyings15", es100!("varyings15.vert"), es100!("varyings15.frag")),
    ];

    /// One pair compiled at both generations: the chain's programs and the
    /// records assembled from them in the keep arena.
    struct Both<'a> {
        p42: chain::V3dProgram<'a>,
        p71: V3dProgram71<'a>,
        sh42: ShcShader,
        sh71: ShcShader,
    }

    /// Thread counts the two allocations answer differently for the same
    /// program: `(name, stage, the 4.2 count, the 7.1 count)`. Neither back
    /// end spills yet (chain.rs 99-101's comment, chain71.rs's `DoesNotFit`
    /// note), so the counts are each allocation's fit answer; `attrib_hole`'s
    /// sixteen attribute vec4s fit alloc71's 4-thread registers and not
    /// alloc.rs's, which drops to 2. Checked exactly, in both directions: a
    /// count that moves (an alloc71 or alloc change, threads71-study among
    /// them) fails this by name.
    const THREAD_PINS: &[(&str, &str, u32, u32)] = &[("attrib_hole", "vs", 2, 4), ("attrib_hole", "cs", 2, 4)];

    /// Uniform-stream entry-set differences between the generations:
    /// `(name, stage, entries only in 4.2, entries only in 7.1)`. Every
    /// program here streams the same `(contents, data)` multiset through
    /// both records, in each schedule's own load order; the one entry
    /// difference measured is `hello`'s fs: the frag writes
    /// `vec4(1.0, 1.0, 0.66, 1.0)`, and the 7.1 program loads the 1.0 as a
    /// `QUNIFORM_CONSTANT` while 4.2's stream holds only the 0.66 (how 4.2
    /// gets 1.0 into its program is its code's business — the stream and
    /// the words are generation-owned). Exact in both directions.
    type StreamPin = (&'static str, &'static str, &'static [(u32, u32)], &'static [(u32, u32)]);
    const STREAM_PINS: &[StreamPin] = &[("hello", "fs", &[], &[(0, 1.0f32.to_bits())])];

    /// Entries of `a` not matched by an equal entry of `b` (a multiset
    /// difference, sorted for comparing against a pin).
    fn only(a: &[(u32, u32)], b: &[(u32, u32)]) -> Vec<(u32, u32)> {
        let mut rest = a.to_vec();
        for e in b {
            if let Some(k) = rest.iter().position(|&x| x == *e) {
                rest.remove(k);
            }
        }
        rest.sort();
        rest
    }

    /// One stage's two thread counts: equal where unpinned, and against the
    /// pinned pair where [`THREAD_PINS`] names this program and stage.
    fn note_threads(diffs: &mut Vec<String>, name: &str, tag: &str, t42: u8, t71: u8) {
        match THREAD_PINS.iter().find(|k| k.0 == name && k.1 == tag) {
            Some(k) => note(
                diffs,
                name,
                &format!("{tag}.threads (pinned {} / {})", k.2, k.3),
                (u32::from(t42), u32::from(t71)),
                (k.2, k.3),
            ),
            None => note(diffs, name, &format!("{tag}.threads"), u32::from(t42), u32::from(t71)),
        }
    }

    /// The front end once, then `chain::compile_program` and `v3d_abi::program_shader`
    /// and `chain71::compile_program71` and [`program_shader71`], each stage
    /// shader record its own `v3d_abi::stage_shader` (what the 7.1 link takes,
    /// link71.rs `compile_program71_shader`). A refusal is a panic naming the
    /// stage: every [`PAIRS`] program compiles at both generations, and one
    /// that stopped compiling says so.
    fn compile_both<'a>(name: &str, vs_src: &str, fs_src: &str, a: &'a Arena<'a>) -> Both<'a> {
        let mut lowered = Vec::new();
        for (stage, src) in [(Stage::Vertex, vs_src), (Stage::Fragment, fs_src)] {
            match lower::compile_to_ir(a, stage, a.alloc_str(src).expect("scratch")) {
                Ok(l) => lowered.push(l),
                Err(e) => panic!("{name}: front end: {}", e.msg),
            }
        }
        let vs = &a.alloc_filled(1, lowered[0].module).expect("scratch")[0];
        let fs = &a.alloc_filled(1, lowered[1].module).expect("scratch")[0];
        let keep = store::keep();
        let p42 = chain::compile_program(vs, fs, chain::Prim::Triangles, TARGET42, a)
            .unwrap_or_else(|e| panic!("{name}: 4.2 refuses: {e:?}"));
        let sh42 = v3d_abi::program_shader(
            v3d_abi::stage_shader(SHC_STAGE_FRAGMENT, &lowered[1].module, &lowered[1].reflection, a, keep)
                .unwrap_or_else(|e| panic!("{name}: 4.2 stage: {e:?}")),
            &p42,
            keep,
        )
        .unwrap_or_else(|e| panic!("{name}: 4.2 record: {e:?}"));
        let p71 = chain71::compile_program71(vs, fs, chain::Prim::Triangles, chain71::TARGET71, a)
            .unwrap_or_else(|e| panic!("{name}: 7.1 refuses: {e:?}"));
        let sh71 = program_shader71(
            v3d_abi::stage_shader(SHC_STAGE_FRAGMENT, &lowered[1].module, &lowered[1].reflection, a, keep)
                .unwrap_or_else(|e| panic!("{name}: 7.1 stage: {e:?}")),
            &p71,
            keep,
        )
        .unwrap_or_else(|e| panic!("{name}: 7.1 record: {e:?}"));
        Both { p42, p71, sh42, sh71 }
    }

    /// One shared field: record a disagreement, with both values.
    fn note<T: PartialEq + core::fmt::Debug>(diffs: &mut Vec<String>, name: &str, field: &str, a: T, b: T) {
        if a != b {
            diffs.push(format!("{name}: {field}: 4.2 {a:?} / 7.1 {b:?}"));
        }
    }

    /// Every field of the two records whose meaning the generations share,
    /// as disagreements to print together.
    fn record_diffs(name: &str, b: &Both<'_>) -> Vec<String> {
        let mut diffs = Vec::new();
        let (x, y) = (&b.sh42, &b.sh71);
        /* The fragment stage's half: what glCompileShader produced and the
         * link keeps (v3d_abi.rs: the program record is the stage record
         * with the code and the V3D half added). */
        note(&mut diffs, name, "num_varyings", x.num_varyings, y.num_varyings);
        for i in 0..(x.num_varyings.max(y.num_varyings)) as usize {
            let (p, q) = (&x.varyings[i], &y.varyings[i]);
            note(&mut diffs, name, &format!("varying {i} name"), name_bytes(&p.name), name_bytes(&q.name));
            note(
                &mut diffs,
                name,
                &format!("varying {i}"),
                (p.location, p.components, p.interp),
                (q.location, q.components, q.interp),
            );
        }
        note(&mut diffs, name, "num_outputs", x.num_outputs, y.num_outputs);
        note(&mut diffs, name, "uses_discard", x.uses_discard, y.uses_discard);
        note(&mut diffs, name, "reads_front_facing", x.reads_front_facing, y.reads_front_facing);
        note(&mut diffs, name, "uniform_dwords", x.uniform_dwords, y.uniform_dwords);
        note(&mut diffs, name, "num_uniforms (stage)", x.num_uniforms, y.num_uniforms);
        note(&mut diffs, name, "num_attribs", x.num_attribs, y.num_attribs);
        note(&mut diffs, name, "meta_off", x.meta_off, y.meta_off);
        note(&mut diffs, name, "blend_consts_off", x.blend_consts_off, y.blend_consts_off);
        note(&mut diffs, name, "vs_psize_off", x.vs_psize_off, y.vs_psize_off);
        /* The V3D half. `linked` and `prim` agree because both records say
         * "a linked program, triangles" (the 7.1 one always, abi71.rs
         * `prim`); the record tables agree because chain71.rs copies
         * chain.rs's attribute-record code unchanged. */
        note(&mut diffs, name, "linked", x.v3d.linked, y.v3d.linked);
        note(&mut diffs, name, "prim", x.v3d.prim, y.v3d.prim);
        note(&mut diffs, name, "num_records", x.v3d.num_records, y.v3d.num_records);
        note(&mut diffs, name, "record_of_location", x.v3d.record_of_location, y.v3d.record_of_location);
        /* fs prog_data: every field chain71.rs `compile_program71_keyed`
         * fills from the same `select_fragment_keyed` answer as chain.rs
         * `fs_prog` does. */
        let (fx, fy) = (&x.v3d.fs, &y.v3d.fs);
        note(&mut diffs, name, "fs.num_inputs", fx.num_inputs, fy.num_inputs);
        note(
            &mut diffs,
            name,
            "fs.flat/centroid",
            (fx.flat_shade_flags, fx.centroid_flags),
            (fy.flat_shade_flags, fy.centroid_flags),
        );
        note(
            &mut diffs,
            name,
            "fs.*_hi",
            (fx.flat_shade_flags_hi, fx.centroid_flags_hi),
            (fy.flat_shade_flags_hi, fy.centroid_flags_hi),
        );
        note(
            &mut diffs,
            name,
            "fs.noperspective",
            (fx.noperspective_flags, fx.noperspective_flags_hi),
            (fy.noperspective_flags, fy.noperspective_flags_hi),
        );
        /* Thread counts are each allocation's fit answer (see `THREAD_PINS`). */
        note_threads(&mut diffs, name, "fs", fx.threads, fy.threads);
        note(&mut diffs, name, "fs.single_seg", fx.single_seg, fy.single_seg);
        note(&mut diffs, name, "fs.writes_z/disable_ez", (fx.writes_z, fx.disable_ez), (fy.writes_z, fy.disable_ez));
        note(&mut diffs, name, "fs.uses_center_w", fx.uses_center_w, fy.uses_center_w);
        note(
            &mut diffs,
            name,
            "fs.lock/pid",
            (fx.lock_scoreboard_on_first_thrsw, fx.uses_pid),
            (fy.lock_scoreboard_on_first_thrsw, fy.uses_pid),
        );
        note(
            &mut diffs,
            name,
            "fs.implicit point/line",
            fx.uses_implicit_point_line_varyings,
            fy.uses_implicit_point_line_varyings,
        );
        note(&mut diffs, name, "fs.rt_mask", fx.rt_mask, fy.rt_mask);
        /* vs and cs prog_data: chain71.rs `vs_prog71` against chain.rs
         * `vs_prog` — the same `vpm_fields`, the same `Selected`. */
        for (tag, vx, vy) in [("vs", &x.v3d.vs, &y.v3d.vs), ("cs", &x.v3d.cs, &y.v3d.cs)] {
            note_threads(&mut diffs, name, tag, vx.threads, vy.threads);
            note(&mut diffs, name, &format!("{tag}.single_seg"), vx.single_seg, vy.single_seg);
            note(
                &mut diffs,
                name,
                &format!("{tag}.vpm"),
                (vx.vpm_input_size, vx.vpm_output_size, vx.vcm_cache_size),
                (vy.vpm_input_size, vy.vpm_output_size, vy.vcm_cache_size),
            );
            note(
                &mut diffs,
                name,
                &format!("{tag}.separate/biid"),
                (vx.separate_segments, vx.uses_biid),
                (vy.separate_segments, vy.uses_biid),
            );
            note(&mut diffs, name, &format!("{tag}.vid/iid"), (vx.uses_vid, vx.uses_iid), (vy.uses_vid, vy.uses_iid));
            note(&mut diffs, name, &format!("{tag}.writes_psiz"), vx.writes_psiz, vy.writes_psiz);
            note(&mut diffs, name, &format!("{tag}.vattr_sizes"), vx.vattr_sizes, vy.vattr_sizes);
        }
        /* The uniform streams are the program's own — its words in
         * `(contents, data)` form — so, like the code, they are compared
         * with the right party rather than across generations: within a
         * generation the record's stream is the chain's program's, entry
         * for entry in order (`stream71` copies `Code71::uniforms`;
         * v3d_abi.rs's `stream` does the same at 4.2), and across
         * generations what is shared is the entry *set* — Mesa's quniform
         * numbering is shared (abi71.rs `uniforms` bullet), while each
         * schedule loads the entries in its own order, the stream being
         * popped in program order. The entry differences measured here
         * are pinned in [`STREAM_PINS`], exact in both directions. */
        for (tag, ux, uy, xp, yp) in [
            ("fs", fx.uniform_stream(), fy.uniform_stream(), b.p42.fs.code.uniforms, b.p71.fs.code.uniforms),
            (
                "vs",
                x.v3d.vs.uniform_stream(),
                y.v3d.vs.uniform_stream(),
                b.p42.vs.code.uniforms,
                b.p71.vs.code.uniforms,
            ),
            (
                "cs",
                x.v3d.cs.uniform_stream(),
                y.v3d.cs.uniform_stream(),
                b.p42.cs.code.uniforms,
                b.p71.cs.code.uniforms,
            ),
        ] {
            let s42: Vec<(u32, u32)> = ux.iter().map(|u| (u.contents, u.data)).collect();
            let s71: Vec<(u32, u32)> = uy.iter().map(|u| (u.contents, u.data)).collect();
            note(
                &mut diffs,
                name,
                &format!("{tag} stream copies the 4.2 program"),
                s42.clone(),
                xp.iter().map(|u| (u.contents, u.data)).collect(),
            );
            note(
                &mut diffs,
                name,
                &format!("{tag} stream copies the 7.1 program"),
                s71.clone(),
                yp.iter().map(|u| (u.contents, u.data)).collect(),
            );
            let (o42, o71) = (only(&s42, &s71), only(&s71, &s42));
            let (e42, e71) = match STREAM_PINS.iter().find(|k| k.0 == name && k.1 == tag) {
                Some(k) => (k.2.to_vec(), k.3.to_vec()),
                None => (Vec::new(), Vec::new()),
            };
            note(&mut diffs, name, &format!("{tag} stream entry sets"), (o42, o71), (e42, e71));
        }
        diffs
    }

    /// The record `program_shader71` builds is `program_shader`'s, field for
    /// field, for the same program: the tables, the prog_data, the VPM
    /// figures and the flags; the thread counts and the uniform streams
    /// against their pins. Fails naming every field that disagrees; the code
    /// itself is generation-owned and is checked by
    /// [`the_code_blob_holds_the_program_words_byte_for_byte`].
    #[test]
    fn the_71_record_is_the_42_record_field_for_field() {
        let _keep = store::test_keep();
        let mut all: Vec<String> = Vec::new();
        for (name, vs_src, fs_src) in PAIRS {
            store::with_scratch(|a| {
                let b = compile_both(name, vs_src, fs_src, a);
                all.extend(record_diffs(name, &b));
                std::println!(
                    "{name}: words fs {}|{} vs {}|{} cs {}|{}; threads fs {}|{} vs {}|{} cs {}|{} (4.2|7.1)",
                    b.p42.fs.code.words.len(),
                    b.p71.fs.code.words.len(),
                    b.p42.vs.code.words.len(),
                    b.p71.vs.code.words.len(),
                    b.p42.cs.code.words.len(),
                    b.p71.cs.code.words.len(),
                    b.sh42.v3d.fs.threads,
                    b.sh71.v3d.fs.threads,
                    b.sh42.v3d.vs.threads,
                    b.sh71.v3d.vs.threads,
                    b.sh42.v3d.cs.threads,
                    b.sh71.v3d.cs.threads,
                );
            })
            .expect("scratch");
        }
        assert!(all.is_empty(), "the records disagree:\n{}", all.join("\n"));
    }

    /// Read the record's `usc` back as whole 64-bit words.
    ///
    /// SAFETY: `usc` and `usc_dw` name an allocation of the keep arena
    /// `program_shader` / `program_shader71` made and the [`store::test_keep`]
    /// of the calling test still holds open.
    fn usc_words(sh: &ShcShader) -> Vec<u64> {
        let dwords = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
        dwords.as_chunks::<2>().0.iter().map(|d| (d[1] as u64) << 32 | d[0] as u64).collect()
    }

    /// The blob is the chain's own words, byte for byte and in the record's
    /// order: FS at 0, then VS, then CS, each whole words little endian
    /// (low dword first) at a multiple of 8 — the alignment the 7.1.10
    /// draw-index record's 29-bit code addresses need (record71.rs
    /// `Kind::Address`, abi71.rs `code_off` bullet). Checked at both
    /// generations: the layout is the 4.2 one the 7.1 record copies, and the
    /// words are each generation's encodings — a 7.1 blob never matches the
    /// 4.2 program and the other way round, which the last assert of each
    /// pair says for at least one program.
    #[test]
    fn the_code_blob_holds_the_program_words_byte_for_byte() {
        let _keep = store::test_keep();
        for (name, vs_src, fs_src) in PAIRS {
            store::with_scratch(|a| {
                let b = compile_both(name, vs_src, fs_src, a);
                let mut differ = false;
                for (tag, sh, words) in [
                    ("4.2", &b.sh42, [b.p42.fs.code.words, b.p42.vs.code.words, b.p42.cs.code.words]),
                    ("7.1", &b.sh71, [b.p71.fs.code.words, b.p71.vs.code.words, b.p71.cs.code.words]),
                ] {
                    let total: usize = words.iter().map(|w| w.len()).sum();
                    assert!(total > 0, "{name}: {tag} compiled to nothing");
                    assert_eq!(sh.usc_dw as usize, total * 2, "{name}: {tag} usc_dw");
                    let v = usc_words(sh);
                    let (off, len) = (
                        [sh.v3d.fs.code_off, sh.v3d.vs.code_off, sh.v3d.cs.code_off],
                        [sh.v3d.fs.code_bytes, sh.v3d.vs.code_bytes, sh.v3d.cs.code_bytes],
                    );
                    assert_eq!(off, [0, len[0], len[0] + len[1]], "{name}: {tag} program offsets");
                    assert_eq!(off[2] + len[2], sh.usc_dw * 4, "{name}: {tag} the last program ends the blob");
                    for (k, what) in ["fs", "vs", "cs"].iter().enumerate() {
                        assert_eq!(off[k] % 8, 0, "{name}: {tag} {what} starts at {}", off[k]);
                        assert_eq!(len[k] as usize, words[k].len() * 8, "{name}: {tag} {what} code_bytes");
                        assert_eq!(
                            &v[off[k] as usize / 8..(off[k] + len[k]) as usize / 8],
                            words[k],
                            "{name}: {tag} {what} words"
                        );
                    }
                    differ |= words != [b.p42.fs.code.words, b.p42.vs.code.words, b.p42.cs.code.words];
                }
                assert!(
                    differ,
                    "{name}: the two generations' programs are identical words — not true of any real pair"
                );
            })
            .expect("scratch");
        }
    }

    /// The fields whose value the 7.1 record fixes by design (the module
    /// comment), on every [`PAIRS`] program:
    ///
    /// * `linked` 1 and `prim` [`SHC_V3D_PRIM_TRIANGLES`]: a linked program,
    ///   the only class chain71 compiles;
    /// * fs `single_seg` 0, `lock_scoreboard_on_first_thrsw` 0, `uses_pid` 0,
    ///   and vs/cs `uses_biid` 0: chain71 gives `false` for each, and
    ///   v3d_job71.c refuses a set one;
    /// * vs/cs `separate_segments` 0: the 7.1 record has no such flag and
    ///   v3d_job71.c `vs_ok71` refuses a set one;
    /// * vs/cs `vpm_input_size` 0: `vpm_fields` gives the shared segment
    ///   (chain.rs 337-358, the figure a non-zero input size would replace
    ///   at 7.1);
    /// * `threads` 1, 2 or 4 — the record's 4-way-threadable sets v3d_job71.c
    ///   accepts;
    /// * every program's code non-empty and every `code_off` 8-aligned;
    /// * an entry of `record_of_location` is a record index below
    ///   `num_records`, or [`SHC_V3D_NO_RECORD`]; a stream with 0 entries
    ///   has a null pointer.
    #[test]
    fn the_71_fields_the_record_pins_are_pinned() {
        let _keep = store::test_keep();
        for (name, vs_src, fs_src) in PAIRS {
            store::with_scratch(|a| {
                let b = compile_both(name, vs_src, fs_src, a);
                let y = &b.sh71;
                assert_eq!((y.v3d.linked, y.v3d.prim), (1, SHC_V3D_PRIM_TRIANGLES), "{name}");
                assert_eq!(
                    (y.v3d.fs.single_seg, y.v3d.fs.lock_scoreboard_on_first_thrsw, y.v3d.fs.uses_pid),
                    (0, 0, 0),
                    "{name}"
                );
                for (tag, v) in [("vs", &y.v3d.vs), ("cs", &y.v3d.cs)] {
                    assert_eq!((v.separate_segments, v.uses_biid, v.vpm_input_size), (0, 0, 0), "{name}: {tag}");
                    assert!(
                        [1, 2, 4].contains(&u32::from(v.threads)),
                        "{name}: {tag} runs at {} threads — v3d_job71.c accepts 1, 2 or 4",
                        v.threads
                    );
                    assert!(v.code_bytes > 0, "{name}: {tag} has no code");
                    assert_eq!(v.code_off % 8, 0, "{name}: {tag} code_off {}", v.code_off);
                    assert!((v.num_uniforms == 0) == v.uniforms.is_null(), "{name}: {tag} stream");
                }
                assert!(
                    u32::from(y.v3d.fs.threads) == 1
                        || u32::from(y.v3d.fs.threads) == 2
                        || u32::from(y.v3d.fs.threads) == 4,
                    "{name}: fs threads"
                );
                assert!(
                    y.v3d.fs.code_bytes > 0 && y.v3d.fs.code_off.is_multiple_of(8) && y.usc_dw > 0,
                    "{name}: fs code"
                );
                assert!((y.v3d.fs.num_uniforms == 0) == y.v3d.fs.uniforms.is_null(), "{name}: fs stream");
                assert!(y.v3d.num_records <= 16, "{name}");
                for (loc, &r) in y.v3d.record_of_location.iter().enumerate() {
                    assert!(r == SHC_V3D_NO_RECORD || r < y.v3d.num_records, "{name}: location {loc} -> record {r}");
                }
            })
            .expect("scratch");
        }
    }

    /// A program built by hand, so the record's own arithmetic is tested
    /// apart from the chain: the attribute table maps location to record
    /// exactly as given, values that do not fit a field are refused by name
    /// and never truncated, the attribute-location bound is the record's,
    /// the uniform stream is copied entry for entry, and the blob's byte
    /// order is little endian.
    #[test]
    fn the_record_assembles_the_program_it_is_given() {
        let _keep = store::test_keep();
        let keep = store::keep();

        /// An empty stage's [`chain71::VsProg71`] / [`chain71::FsProg71`].
        fn vs(tag_words: &'static [u64]) -> VsProg71<'static> {
            VsProg71 {
                code: chain71::Code71 {
                    words: tag_words,
                    instrs: &[],
                    uniforms: &[],
                    back_edges: &[],
                    fwd_branches: &[],
                },
                threads: 2,
                single_seg: false,
                vpm_input_size: 0,
                vpm_output_size: 1,
                separate_segments: false,
                vcm_cache_size: 2,
                uses_vid: false,
                uses_iid: false,
                uses_biid: false,
                writes_psiz: false,
                vattr_sizes: [0; 16],
                tmu_dirty_rcl: false,
            }
        }
        fn prog<'a>(attrs: &'a [(u8, Option<u8>)]) -> V3dProgram71<'a> {
            V3dProgram71 {
                fs: chain71::FsProg71 {
                    code: chain71::Code71 {
                        words: &[],
                        instrs: &[],
                        uniforms: &[],
                        back_edges: &[],
                        fwd_branches: &[],
                    },
                    threads: 2,
                    single_seg: false,
                    num_inputs: 0,
                    input_slots: &[],
                    flat_shade_flags: 0,
                    noperspective_flags: 0,
                    centroid_flags: 0,
                    flat_shade_flags_hi: 0,
                    noperspective_flags_hi: 0,
                    centroid_flags_hi: 0,
                    writes_z: false,
                    disable_ez: false,
                    uses_center_w: false,
                    lock_scoreboard_on_first_thrsw: false,
                    uses_implicit_point_line_varyings: false,
                    uses_pid: false,
                    rt_mask: 1,
                    force_per_sample_msaa: false,
                    tmu_dirty_rcl: false,
                    uses_discard: false,
                },
                vs: vs(&[]),
                cs: vs(&[]),
                attributes: attrs,
            }
        }

        /* The record table: location to record, `None` keeps
         * SHC_V3D_NO_RECORD, and num_records is the highest index + 1. */
        let sh = program_shader71(ShcShader::ZERO, &prog(&[(3, Some(5)), (7, Some(0)), (9, None)]), keep)
            .expect("assembles");
        assert_eq!(sh.v3d.num_records, 6);
        assert_eq!((sh.v3d.record_of_location[3], sh.v3d.record_of_location[7]), (5, 0));
        assert_eq!(sh.v3d.record_of_location[9], SHC_V3D_NO_RECORD);
        assert_eq!(sh.v3d.record_of_location.iter().filter(|&&r| r != SHC_V3D_NO_RECORD).count(), 2);
        assert_eq!(sh.usc_dw, 0, "no words, no blob");
        assert!(sh.v3d.fs.uniforms.is_null() && sh.v3d.vs.uniforms.is_null() && sh.v3d.cs.uniforms.is_null());

        /* A location outside the record's 0..16 is refused by name. */
        assert!(matches!(
            program_shader71(ShcShader::ZERO, &prog(&[(17, Some(0))]), keep),
            Err(V3dAbiError::AttribLocation { location: 17 })
        ));

        /* A thread count that does not fit the record's field is refused,
         * never truncated. */
        let mut p = prog(&[]);
        p.vs.threads = 300;
        assert!(matches!(program_shader71(ShcShader::ZERO, &p, keep), Err(V3dAbiError::TooLarge)));

        /* Words, streams and the blob's order. */
        const FS_WORDS: &[u64] = &[0x0000_0001_0000_0002, 3];
        const VS_WORDS: &[u64] = &[4, 5, 6];
        const CS_WORDS: &[u64] = &[7];
        const STREAM: &[Uniform] = &[Uniform { contents: 9, data: 65 }, Uniform::constant(7)];
        let mut p = prog(&[]);
        p.fs.code.words = FS_WORDS;
        p.vs.code.words = VS_WORDS;
        p.cs.code.words = CS_WORDS;
        p.vs.code.uniforms = STREAM;
        let sh = program_shader71(ShcShader::ZERO, &p, keep).expect("assembles");
        assert_eq!((sh.usc_dw, sh.v3d.fs.code_bytes), (12, 16), "6 words");
        assert_eq!((sh.v3d.vs.code_off, sh.v3d.vs.code_bytes, sh.v3d.cs.code_off), (16, 24, 40));
        /* little endian, as the QPU fetches a 64-bit word (the module
         * comment's `usc` bullet) */
        assert_eq!(&usc_words(&sh)[..6], &[0x0000_0001_0000_0002, 3, 4, 5, 6, 7][..]);
        // SAFETY: as in usc_words; 4 is within the 12 dwords the record counts.
        let dwords = unsafe { core::slice::from_raw_parts(sh.usc, 4) };
        assert_eq!(dwords, &[0x0000_0002u32, 0x0000_0001, 3, 0][..], "low dword first");
        assert_eq!(
            sh.v3d.vs.uniform_stream(),
            &[ShcV3dUniform { contents: 9, data: 65 }, ShcV3dUniform { contents: 0, data: 7 }][..]
        );
        assert_eq!(sh.v3d.vs.num_uniforms, 2);
        assert_eq!(sh.v3d.cs.num_uniforms, 0);
    }
}
