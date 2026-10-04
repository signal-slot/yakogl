// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The `ShcShader` assembly for the V3D 4.2 (`gpu-v3d` builds only).
//!
//! The V3D compiles a program, not a stage (docs/online-shaderc/v3d-backend.md,
//! "M6: the GL core on the V3D"), so there are two records:
//!
//! * [`stage_shader`]: what `glCompileShader` produces. The reflection
//!   (attributes, varyings, uniforms and the uniform block's size) and no code:
//!   `usc_dw` is 0 and `v3d.linked` is 0. The GL core uploads nothing for it
//!   (`upload_prog_gpu` skips a stage whose `usc_dw` is 0) and never reads its
//!   code.
//! * [`program_shader`]: what `glLinkProgram` puts in place of the program's
//!   fragment shader. The fragment stage's reflection, and the three QPU
//!   programs of [`V3dProgram`] back to back in `usc` (FS, VS, CS, each at a
//!   multiple of 8 bytes), with `v3d` saying where each starts, its prog_data
//!   and its uniform stream, and which attribute record each GL attribute
//!   location is fetched through.
//!
//! Everything the returned shader points at is allocated from the keep arena
//! the caller has open (`store::begin_keep`), so it is freed with the record.
//! A value that does not fit a field is refused by name, never truncated.

use super::arena::{Arena, ArenaError};
use super::ir::{self, IoKind};
use super::reflect;
use super::v3d::chain::{V3dProgram, VsProg};
use crate::shaderc::{
    ShcShader, ShcUniform, ShcV3d, ShcV3dFs, ShcV3dUniform, ShcV3dVs, ShcVarying, SHC_NO_BLEND_CONSTS, SHC_NO_META,
    SHC_NO_PSIZE, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX, SHC_V3D_NO_RECORD, SHC_V3D_PRIM_TRIANGLES,
};

/// Why a record could not be assembled. One variant per cause.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum V3dAbiError {
    /// More attributes than `ShcShader::attribs` holds.
    TooManyAttribs { got: usize },
    /// More varyings than `ShcShader::varyings` holds.
    TooManyVaryings { got: usize },
    /// More uniform rows than `num_uniforms` (a `u8`) counts.
    TooManyUniforms { got: usize },
    /// A name as long as the 32-byte field or longer: it would be cut, and a
    /// cut name is paired and matched on (`shaderc::link`).
    NameTooLong { len: usize },
    /// An attribute location outside 0..16.
    AttribLocation { location: u8 },
    /// The reflection and the IR disagree about the uniform block's size; the
    /// GL core sizes the block from one and the program indexes it by the
    /// other.
    UniformBlockMismatch { reflection: u16, module: u16 },
    /// A program's code, stream or offset does not fit its field.
    TooLarge,
    /// A stage number that is neither vertex nor fragment.
    BadStage(u32),
    /// The keep arena is full: raise `store::KEEP_BYTES`.
    OutOfKeep,
    /// The scratch arena is full: raise `store::SCRATCH_BYTES`.
    OutOfScratch,
}

impl From<ArenaError> for V3dAbiError {
    fn from(_: ArenaError) -> Self {
        V3dAbiError::OutOfScratch
    }
}

/// Copy `name` into a 32-byte field, refusing a name the field would cut.
fn set_name(field: &mut [core::ffi::c_char], name: &str) -> Result<(), V3dAbiError> {
    if name.len() >= field.len() {
        return Err(V3dAbiError::NameTooLong { len: name.len() });
    }
    crate::util::set_name(field, name.as_bytes());
    Ok(())
}

/// The attribute locations the IR reads, as a mask.
fn locations_read(m: &ir::Module<'_>) -> Result<u16, V3dAbiError> {
    let mut mask = 0u16;
    for op in m.instrs {
        if let ir::Op::LoadInput { slot: ir::IoSlot::Attrib(loc), .. } = op {
            if *loc >= 16 {
                return Err(V3dAbiError::AttribLocation { location: *loc });
            }
            mask |= 1 << loc;
        }
    }
    Ok(mask)
}

/// A compiled stage: reflection, no code (see the module comment).
pub fn stage_shader(
    stage: u32,
    m: &ir::Module<'_>,
    r: &reflect::Reflection<'_>,
    scratch: &Arena<'_>,
    keep: &'static Arena<'static>,
) -> Result<ShcShader, V3dAbiError> {
    if stage != SHC_STAGE_VERTEX && stage != SHC_STAGE_FRAGMENT {
        return Err(V3dAbiError::BadStage(stage));
    }
    if r.uniform_dwords != m.uniform_dwords {
        return Err(V3dAbiError::UniformBlockMismatch { reflection: r.uniform_dwords, module: m.uniform_dwords });
    }
    let mut sh = ShcShader::ZERO;

    /* The varyings the stage writes (vertex) or reads (fragment), by name:
     * what the link pairs and checks. The code does not come from them.
     *
     * A fragment shader lists only the varyings it statically uses (a load
     * anywhere in its IR, taken or not), as the other GPUs' records list
     * only what their code reads: link_online (backend.rs) takes this list
     * as the varyings the shader READS, and GLSL ES 1.00 section 4.3.5
     * requires the vertex shader to declare only those ("Only those varying
     * variables statically used (i.e. read) in the fragment shader must be
     * declared in the vertex shader"; its table: vertex "No reference",
     * fragment "Declares; no static use": Allowed). A declared, unread
     * varying listed here was refused as undeclared (dEQP
     * shaders.linkage.varying_1). */
    let reads = |io: &&ir::IoDecl<'_>| {
        stage == SHC_STAGE_VERTEX
            || m.instrs
                .iter()
                .any(|op| matches!(op, ir::Op::LoadInput { slot: ir::IoSlot::Varying(l), .. } if *l == io.location))
    };
    let ios = if stage == SHC_STAGE_VERTEX { m.outputs } else { m.inputs };
    let varyings = ios.iter().filter(|io| io.kind == IoKind::Varying).filter(reads);
    let n = varyings.clone().count();
    if n > sh.varyings.len() {
        return Err(V3dAbiError::TooManyVaryings { got: n });
    }
    for (dst, io) in sh.varyings.iter_mut().zip(varyings) {
        let dst: &mut ShcVarying = dst;
        set_name(&mut dst.name, io.name)?;
        dst.location = io.location;
        dst.components = io.components;
        dst.interp = io.interp;
    }
    sh.num_varyings = n as u8;

    if stage == SHC_STAGE_VERTEX {
        /* The attributes the IR reads: glGetActiveAttrib's list. `vi_reg` is
         * the location; the backend maps it to an attribute record through
         * the linked program's `record_of_location`. A matrix is one row,
         * split per column by the GL core (draw.rs vertex_inputs). */
        let read = locations_read(m)?;
        let active = |a: &&reflect::AttribRow<'_>| {
            let cols = super::v3d::chain::attribute_locations(a.gl_type);
            (0..cols).any(|k| {
                let loc = a.location as u32 + k as u32;
                loc < 16 && read & (1 << loc) != 0
            })
        };
        /* One row per location: attributes the link bound to one location
         * (glBindAttribLocation aliasing, `compile_vs_linked`) are fetched
         * through that location's one record, so the location is listed
         * once, as wide as the widest of them, and counts once against the
         * GL core's GL_MAX_VERTEX_ATTRIBS check. The GL core answers the
         * attribute queries from its own copy of the declarations. Without
         * aliasing every location is one row already. */
        let mut n = 0usize;
        for a in r.attribs.iter().filter(active) {
            if a.location >= 16 {
                return Err(V3dAbiError::AttribLocation { location: a.location });
            }
            if let Some(kept) = sh.attribs[..n].iter_mut().find(|d| d.location == a.location) {
                if a.components > kept.components {
                    kept.components = a.components;
                }
                continue;
            }
            let dst = sh.attribs.get_mut(n).ok_or(V3dAbiError::TooManyAttribs { got: n + 1 })?;
            set_name(&mut dst.name, a.name)?;
            dst.location = a.location;
            dst.components = a.components;
            dst.vi_reg = a.location;
            dst.format = 0; /* float32: the only format the attribute records take (M6) */
            dst.gl_type = a.gl_type;
            n += 1;
        }
        sh.num_attribs = n as u8;
        sh.vs_psize_off = SHC_NO_PSIZE;
    } else {
        sh.num_outputs = 1;
        sh.meta_off = SHC_NO_META;
        /* The TLB blends on this GPU: no constants in the uniform block. */
        sh.blend_consts_off = SHC_NO_BLEND_CONSTS;
        sh.uses_discard = u8::from(m.uses_discard);
        sh.reads_front_facing = u8::from(m.uses_front_facing);
    }

    let rows: &[ShcUniform] = reflect::uniform_rows(r, scratch)?;
    if !rows.is_empty() {
        let n = u8::try_from(rows.len()).map_err(|_| V3dAbiError::TooManyUniforms { got: rows.len() })?;
        let kept = keep.alloc_copy(rows).map_err(|_| V3dAbiError::OutOfKeep)?;
        sh.uniforms = kept.as_ptr();
        sh.num_uniforms = n;
    }
    sh.uniform_dwords = r.uniform_dwords;
    sh.v3d = ShcV3d::ZERO;
    Ok(sh)
}

/// The uniform stream of one program, in keep.
fn stream(
    u: &[super::v3d::select::Uniform],
    keep: &'static Arena<'static>,
) -> Result<(*const ShcV3dUniform, u32), V3dAbiError> {
    if u.is_empty() {
        return Ok((core::ptr::null(), 0));
    }
    let out = keep.alloc_filled(u.len(), ShcV3dUniform { contents: 0, data: 0 }).map_err(|_| V3dAbiError::OutOfKeep)?;
    for (d, s) in out.iter_mut().zip(u) {
        *d = ShcV3dUniform { contents: s.contents, data: s.data };
    }
    Ok((out.as_ptr(), u32::try_from(u.len()).map_err(|_| V3dAbiError::TooLarge)?))
}

fn vs_half(p: &VsProg<'_>, code_off: u32, keep: &'static Arena<'static>) -> Result<ShcV3dVs, V3dAbiError> {
    let (uniforms, num_uniforms) = stream(p.code.uniforms, keep)?;
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
        separate_segments: u8::from(p.separate_segments),
        uses_vid: u8::from(p.uses_vid),
        uses_iid: u8::from(p.uses_iid),
        uses_biid: u8::from(p.uses_biid),
        writes_psiz: u8::from(p.writes_psiz),
        vattr_sizes: p.vattr_sizes,
    })
}

/// The linked program: `fs_stage` (the fragment stage's record, built by
/// [`stage_shader`] in the same keep transaction) with the code and the V3D
/// half added.
pub fn program_shader(
    fs_stage: ShcShader,
    p: &V3dProgram<'_>,
    keep: &'static Arena<'static>,
) -> Result<ShcShader, V3dAbiError> {
    let mut sh = fs_stage;
    /* FS, VS, CS back to back; each program is whole 64-bit words, so each
     * starts at a multiple of 8 bytes from the first. */
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

    let (fs_uniforms, fs_num_uniforms) = stream(p.fs.code.uniforms, keep)?;
    let fs = ShcV3dFs {
        code_off: offs[0],
        code_bytes: u32::try_from(p.fs.code.words.len() * 8).map_err(|_| V3dAbiError::TooLarge)?,
        uniforms: fs_uniforms,
        num_uniforms: fs_num_uniforms,
        num_inputs: p.fs.num_inputs,
        flat_shade_flags: p.fs.flat_shade_flags,
        noperspective_flags: p.fs.noperspective_flags,
        centroid_flags: p.fs.centroid_flags,
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
        vs: vs_half(&p.vs, offs[1], keep)?,
        cs: vs_half(&p.cs, offs[2], keep)?,
    };
    Ok(sh)
}

/// The V3D path through the in-process back end, by the entry points the GL
/// core calls (`SHC_ONLINE.compile` for glCompileShader, `.link` for
/// glLinkProgram), with the keep region's accounting.
#[cfg(test)]
mod tests {
    use super::super::backend::{release, retain, SHC_ONLINE};
    use super::super::store;
    use crate::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX, SHC_V3D_NO_RECORD};
    use crate::util::{cstr_bytes, EINVAL, ENOTSUP};
    use core::ffi::c_char;
    use std::string::String;
    use std::vec::Vec;

    static PASSTHROUGH_VS: &str = include_str!("../../../tools/shaders/gl_test_es100/passthrough.vert");
    static INTERP_FS: &str = include_str!("../../../tools/shaders/gl_test_es100/interp_color.frag");

    /// femtovg's programs in the order its `new_from_context` builds them:
    /// the sets (glyph texture, no discard) (1,0), (0,0), (1,1), (0,1), each
    /// its ShaderType order, Stencil, FilterImage and TextureCopyUnclipped
    /// only without the glyph texture, TextureCopyUnclipped never without
    /// discard (so the 30th program's fragment shader is the 15th's text).
    /// The sources are the ones femtovg passes (tools/shaders/femtovg).
    static FEMTOVG_VS: &str = include_str!("../../../tools/shaders/femtovg/main.vert");
    static FEMTOVG_FS: [(&str, &str); 30] = {
        macro_rules! fs {
            ($n:literal) => {
                ($n, include_str!(concat!("../../../tools/shaders/femtovg/", $n, ".frag")))
            };
        }
        [
            fs!("fill_gradient.glyph"),
            fs!("fill_image.glyph"),
            fs!("fill_image_gradient.glyph"),
            fs!("fill_color.glyph"),
            fs!("fill_gradient_conic.glyph"),
            fs!("fill_image_gradient_conic.glyph"),
            fs!("fill_gradient"),
            fs!("fill_image"),
            fs!("stencil"),
            fs!("fill_image_gradient"),
            fs!("filter_image"),
            fs!("fill_color"),
            fs!("texture_copy_unclipped"),
            fs!("fill_gradient_conic"),
            fs!("fill_image_gradient_conic"),
            fs!("fill_gradient.glyph.nodiscard"),
            fs!("fill_image.glyph.nodiscard"),
            fs!("fill_image_gradient.glyph.nodiscard"),
            fs!("fill_color.glyph.nodiscard"),
            fs!("fill_gradient_conic.glyph.nodiscard"),
            fs!("fill_image_gradient_conic.glyph.nodiscard"),
            fs!("fill_gradient.nodiscard"),
            fs!("fill_image.nodiscard"),
            fs!("stencil.nodiscard"),
            fs!("fill_image_gradient.nodiscard"),
            fs!("filter_image.nodiscard"),
            fs!("fill_color.nodiscard"),
            fs!("texture_copy_unclipped"),
            fs!("fill_gradient_conic.nodiscard"),
            fs!("fill_image_gradient_conic.nodiscard"),
        ]
    };

    fn compile(stage: u32, text: &str) -> (i32, ShcShader, String) {
        let mut src = Vec::from(text.as_bytes());
        src.push(0);
        let mut sh = ShcShader::ZERO;
        let mut log = [0 as c_char; 512];
        let f = SHC_ONLINE.compile.unwrap();
        // SAFETY: a NUL-terminated source, a real output and log.
        let rc = unsafe { f(stage, src.as_ptr() as *const c_char, &mut sh, log.as_mut_ptr(), log.len()) };
        // SAFETY: the writer NUL-terminated the log.
        (rc, sh, String::from_utf8_lossy(unsafe { cstr_bytes(log.as_ptr()) }).into_owned())
    }

    /// dEQP-GLES2 attribute_location.bind_aliasing.max_cond_<ty> (or, with
    /// `inactive`, max_inactive_<ty>)'s vertex shader, as
    /// glsAttributeLocationTests.cpp generates it (generateVertexShaderTemplate,
    /// BindMaxAliasingAttributeTest / BindInactiveAliasingAttributeTest, with
    /// GL_MAX_VERTEX_ATTRIBS 16): pairs `a_n` and `a_(n+16)`, declared in turn,
    /// read under `u_A != 0.0` and `u_A == 0.0` (max_cond) or `u_A == 0.0` and
    /// `0 != 0` (max_inactive), and the location dEQP binds both of pair n
    /// to: 16 - cols * (n + 1). The texts' lengths are the Pi 4 record's
    /// (record_SW_AL_c9eb1078, "[vertex stage, N bytes]"), pinned below.
    fn deqp_bind_aliasing_vs(ty: &str, cols: u8, inactive: bool) -> (String, Vec<(String, u8)>) {
        let mut s = String::from("#version 100\nvarying mediump vec4 v_color;\n");
        let mut body = String::new();
        let mut binds = Vec::new();
        let vec4_of = |n: &str| match ty {
            "vec2" => format!("vec4({n}.xy, {n}.yx)"),
            "vec3" => format!("vec4({n}.xyz, {n}.x)"),
            _ => format!("vec4({n})"),
        };
        let conds = if inactive { ["u_A == 0.0", "0 != 0"] } else { ["u_A != 0.0", "u_A == 0.0"] };
        for n in 0..16 / cols {
            let loc = 16 - cols * (n + 1);
            for (name, cond) in [(format!("a_{n}"), conds[0]), (format!("a_{}", n + 16), conds[1])] {
                s += &format!("attribute mediump {ty} {name};\n");
                body += &format!("\tif ({cond})\n\t{{\n\t\tcolor += {};\n\t}}\n", vec4_of(&name));
                binds.push((name, loc));
            }
        }
        s += "\nuniform mediump float u_A;\n\nvoid main (void)\n{\n\tmediump vec4 color = vec4(0.0);\n\n";
        s += &body;
        s += "\n\tv_color = color;\n\tgl_Position = color;\n}\n";
        (s, binds)
    }

    /// dEQP-GLES2 attribute_location.bind_aliasing.max_cond_* and
    /// max_inactive_* (14 cases, 14 of 57 failures on the Pi 4 at c9eb1078:
    /// "Abi(AttribLocation { location: 16 })" at glCompileShader). Each of
    /// dEQP's vertex shaders declares more locations than the V3D's 16, so
    /// its compile leaves the layout to the link (`layout_at_link`, every
    /// declared attribute); with dEQP's bindings (each pair on one location;
    /// as the GL core places them, an inactive attribute takes none) the
    /// relink succeeds and the program links, fetching each bound location
    /// through one record.
    #[test]
    fn deqp_bind_aliasing_links_with_the_links_layout() {
        use crate::shaderc::{ShcAttribBinding, ShcAttribMap, SHC_ATTRIB_NO_LOCATION};
        const FS: &str = "varying mediump vec4 v_color;\nvoid main (void) { gl_FragColor = v_color; }";
        /* (type, locations, columns the shader reads): `vec4(m)` takes a
         * matrix's first four components in column order (ES 1.00 5.4.2),
         * so a mat2 reads both its columns, a mat3 two of its three and a
         * mat4 one of its four; an unread column needs no record */
        let types = [
            ("float", 1u8, 1),
            ("vec2", 1, 1),
            ("vec3", 1, 1),
            ("vec4", 1, 1),
            ("mat2", 2, 2),
            ("mat3", 3, 2),
            ("mat4", 4, 1),
        ];
        /* the source lengths the Pi 4 logged, max_cond then max_inactive */
        let board = [(2580, 2516), (2922, 2858), (2922, 2858), (2548, 2484), (1352, 1320), (908, 888), (760, 744)];
        let mut linked = 0;
        for ((ty, cols, read), lens) in types.into_iter().zip(board) {
            for inactive in [false, true] {
                let what = format!("{} {ty}", if inactive { "max_inactive" } else { "max_cond" });
                let (src, binds) = deqp_bind_aliasing_vs(ty, cols, inactive);
                assert_eq!(src.len(), if inactive { lens.1 } else { lens.0 }, "{what}: not dEQP's text");
                let (rc, vs, log) = compile(SHC_STAGE_VERTEX, &src);
                assert_eq!(rc, 0, "{what}: {log}");
                assert_eq!((vs.layout_at_link, vs.num_attribs), (1, 0), "{what}: deferred to the link");
                // SAFETY: the record's list of num_declared entries.
                let declared = unsafe { core::slice::from_raw_parts(vs.declared, vs.num_declared as usize) };
                assert_eq!(declared.len(), binds.len(), "{what}: every declared attribute");
                let mut map = ShcAttribMap::ZERO;
                map.count = declared.len() as u8;
                for (i, d) in declared.iter().enumerate() {
                    let name: String = d.name.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
                    let loc = binds.iter().find(|b| b.0 == name).expect("bound").1;
                    let location = if d.active != 0 { loc } else { SHC_ATTRIB_NO_LOCATION };
                    map.b[i] = ShcAttribBinding { location, ..*d };
                }
                let mut text = Vec::from(src.as_bytes());
                text.push(0);
                let mut out = ShcShader::ZERO;
                let mut clog = [0 as c_char; 512];
                let entry = SHC_ONLINE.compile_vs_linked.expect("registered on the V3D");
                // SAFETY: a NUL-terminated source, a map, an output and a log of the announced size.
                let rc =
                    unsafe { entry(text.as_ptr() as *const c_char, &map, &mut out, clog.as_mut_ptr(), clog.len()) };
                // SAFETY: the writer NUL-terminated the log.
                let clog = String::from_utf8_lossy(unsafe { cstr_bytes(clog.as_ptr()) }).into_owned();
                assert_eq!(rc, 0, "{what}: relink: {clog}");
                let pairs = usize::from(16 / cols);
                assert_eq!(usize::from(out.num_attribs), pairs, "{what}: one row per bound location");
                let (_, mut fs, _) = compile(SHC_STAGE_FRAGMENT, FS);
                let (rc, llog) = link(&out, &mut fs);
                assert_eq!(rc, 0, "{what}: link: {llog}");
                assert_eq!(usize::from(fs.v3d.num_records), pairs * read, "{what}: a record per column read");
                linked += 1;
                release(&fs);
                release(&out);
                release(&vs);
            }
        }
        assert_eq!(linked, 14, "all 14 cases");
    }

    /// dEQP-GLES2 shaders.linkage.varying_1 and varying_illegal_usage_1
    /// (data/gles2/shaders/linkage.test, expanded as dEQP does:
    /// tools/es100_oracle.py specialize_complete). A varying the fragment
    /// shader declares and never reads needs no vertex declaration (GLSL ES
    /// 1.00 section 4.3.5: "Only those varying variables statically used
    /// (i.e. read) in the fragment shader must be declared in the vertex
    /// shader"); one it reads does, even in code that never runs ("static use
    /// means that after preprocessing the shader includes at least one
    /// statement that accesses the varying, even if that statement is never
    /// actually executed"). The V3D's fragment record listed every declared
    /// varying, and link_online refused varying_1 (the Pi 4 at 2c87c2e4).
    #[test]
    fn a_declared_unread_fragment_varying_needs_no_vertex_declaration() {
        const VS: &str =
            "attribute highp vec4 dEQP_Position;\n\nvoid main()\n{\n    gl_Position = dEQP_Position;\n\n}\n";
        /* varying_1: expect pass */
        const FS_1: &str =
            "precision mediump float;\nvarying mediump float var;\nbool isOk (float a, float b, float eps) \
            { return (abs(a-b) <= (eps*abs(b) + eps)); }\nuniform float ref_out0;\nfloat out0;\n\nvoid main()\n{\n    \
            out0 = 1.0;\n    bool RES = isOk(out0, ref_out0, 0.05);\ngl_FragColor = vec4(RES, RES, RES, 1.0);\n\n}\n";
        /* varying_illegal_usage_1: expect link_fail */
        const FS_USE: &str = "varying mediump float var;\nvoid main()\n{\n    gl_FragColor = vec4(var);\n}\n";
        /* the same use in a branch never taken: still a static use */
        const FS_DEAD: &str = "precision mediump float;\nvarying mediump float var;\nuniform float u;\n\
            void main() { gl_FragColor = vec4(1.0); if (u > 1.0 && u < 0.0) gl_FragColor = vec4(var); }";
        let (rc, vs, log) = compile(SHC_STAGE_VERTEX, VS);
        assert_eq!(rc, 0, "{log}");
        let run = |fs_src: &str| -> (i32, String) {
            let (rc, mut fs, log) = compile(SHC_STAGE_FRAGMENT, fs_src);
            assert_eq!(rc, 0, "compiles (dEQP expects a link verdict, not a compile one): {log}");
            let listed = fs.num_varyings;
            let (rc, log) = link(&vs, &mut fs);
            release(&fs);
            (rc, format!("{listed} listed; {log}"))
        };
        let (rc, log) = run(FS_1);
        assert_eq!(rc, 0, "varying_1 links: {log}");
        assert!(log.starts_with("0 listed"), "the unread varying is not an input: {log}");
        for (what, src) in [("varying_illegal_usage_1", FS_USE), ("a read in a branch never taken", FS_DEAD)] {
            let (rc, log) = run(src);
            assert_eq!(rc, -EINVAL, "{what} must not link: {log}");
            assert!(log.contains("fragment varying 'var' is not declared in the vertex shader"), "{what}: {log}");
        }
        release(&vs);
    }

    fn link(vs: &ShcShader, fs: &mut ShcShader) -> (i32, String) {
        let mut log = [0 as c_char; 512];
        let f = SHC_ONLINE.link.unwrap();
        // SAFETY: two valid distinct shaders and a log of the announced size.
        let rc = unsafe { f(vs, fs, log.as_mut_ptr(), log.len()) };
        // SAFETY: as above.
        (rc, String::from_utf8_lossy(unsafe { cstr_bytes(log.as_ptr()) }).into_owned())
    }

    /// femtovg builds its 30 programs at canvas creation and keeps them all,
    /// deleting each program's shader objects right after its link
    /// (renderer/opengl/program.rs). All 30 must link, and the keep region
    /// must hold them without the freed stage records fragmenting it.
    ///
    /// Each program as the GL core makes it: two shader objects compiled, the
    /// program retaining both copies at link (gl::program), the link turning
    /// the fragment copy into the program record, then both shader objects
    /// deleted. Prints the region's in-use figure, its largest free run and
    /// its high-water after each program.
    ///
    /// The negative control is the same sequence with every record placed at
    /// the bottom (store::set_single_ended, the placement before
    /// store::KeepEnd): it runs out at program 22 while about 230 KB is
    /// free, as it did on ea8ab0d6 (v3d-slint's report, 2026-09-24).
    #[test]
    fn femtovgs_thirty_programs_link_and_stay_alive_together() {
        let _s = store::test_lock();
        struct Restore(bool);
        impl Drop for Restore {
            fn drop(&mut self) {
                store::set_single_ended(self.0);
            }
        }
        let _restore = Restore(store::set_single_ended(false));
        /* One pass: how many programs linked, the programs the code
         * generator refused for a part it does not build yet (-ENOTSUP: a
         * compiler gap, its own outcome, never read as the region's), and the
         * failure that ended the pass. */
        let run = |single: bool| -> (usize, Vec<&str>, String) {
            store::set_single_ended(single);
            let before = store::keep_in_use();
            let mut programs: Vec<(ShcShader, ShcShader)> = Vec::new();
            let mut gaps = Vec::new();
            let mut failure = String::new();
            for (i, (name, text)) in FEMTOVG_FS.iter().enumerate() {
                let (rc, vs_obj, log) = compile(SHC_STAGE_VERTEX, FEMTOVG_VS);
                assert_eq!(rc, 0, "main.vert: {log}");
                let (rc, fs_obj, log) = compile(SHC_STAGE_FRAGMENT, text);
                assert_eq!(rc, 0, "{name}: {log}");
                let (prog_vs, mut prog_fs) = (vs_obj, fs_obj);
                retain(&prog_vs);
                retain(&prog_fs);
                let (rc, log) = link(&prog_vs, &mut prog_fs);
                release(&vs_obj);
                release(&fs_obj);
                if rc == -ENOTSUP {
                    gaps.push(*name);
                    release(&prog_vs);
                    release(&prog_fs);
                    continue;
                }
                if rc != 0 {
                    failure = std::format!("program {} ({name}): link rc {rc}: {log}", i + 1);
                    release(&prog_vs);
                    release(&prog_fs);
                    break;
                }
                programs.push((prog_vs, prog_fs));
                std::println!(
                    "{} {:2} {name}: in use {}, largest free run {}, high-water {}",
                    if single { "one end " } else { "two ends" },
                    i + 1,
                    store::keep_in_use(),
                    store::keep_largest_free(),
                    store::keep_used()
                );
            }
            let n = programs.len();
            for (v, f) in &programs {
                release(f);
                release(v);
            }
            assert_eq!(store::keep_in_use(), before, "every record was given back");
            (n, gaps, failure)
        };
        let start = store::keep_in_use();
        let (n, gaps, failure) = run(false);
        std::println!("two ends: {n} programs linked; refused for a compiler gap: {gaps:?}");
        /* No gaps since the counted loop (ea8ab0d6): before it, the two
         * filter_image programs were refused here. A gap that comes back is
         * a compiler regression, and says so here rather than as memory. */
        assert!(gaps.is_empty(), "the chain refused femtovg programs: {gaps:?}");
        assert_eq!(failure, "", "every program links and stays alive");
        assert_eq!(n, 30, "all of femtovg's programs are held at once");
        let (n1, gaps1, failure) = run(true);
        std::println!("one end: {n1} programs; {failure}");
        assert!(gaps.starts_with(&gaps1), "the same gaps, as far as the pass got: {gaps1:?}");
        assert!(
            failure.contains("keep arena"),
            "the negative control (one end) runs out, by the keep region: {failure}"
        );
        assert!(n1 < n);
        /* The count moves with the programs' size: 21 on ea8ab0d6; 22 since
         * the fragment shaders' look-up switch is their last pair and a
         * THRSW merges back onto placed words (v3d-m5/vid, 2026-09-26:
         * program 22 fits with 10688 bytes left, 23 fails with 8542). */
        if start == 0 && gaps.is_empty() {
            assert_eq!(n1, 22, "from an empty region program 23 fails, as measured on v3d-m5/vid: {failure}");
        }
    }

    /// A stage is its reflection and no code: nothing to upload, and the
    /// compile status is the front end's verdict.
    #[test]
    fn a_compiled_stage_carries_reflection_and_no_code() {
        let _s = store::test_lock();
        let (rc, vs, log) = compile(SHC_STAGE_VERTEX, PASSTHROUGH_VS);
        assert_eq!(rc, 0, "{log}");
        assert_eq!((vs.usc_dw, vs.v3d.linked), (0, 0));
        assert_eq!(vs.num_attribs, 2);
        let names: Vec<&[u8]> = vs.attribs[..2].iter().map(|a| crate::util::name_bytes(&a.name)).collect();
        assert_eq!(names, [&b"a_pos"[..], b"a_color"]);
        assert_eq!((vs.attribs[0].vi_reg, vs.attribs[1].vi_reg), (vs.attribs[0].location, vs.attribs[1].location));
        assert_eq!(vs.num_varyings, 1);
        let (rc, fs, log) = compile(SHC_STAGE_FRAGMENT, INTERP_FS);
        assert_eq!(rc, 0, "{log}");
        assert_eq!((fs.usc_dw, fs.num_varyings, fs.num_outputs), (0, 1, 1));
        /* an invalid shader still fails at glCompileShader */
        let (rc, _, log) = compile(SHC_STAGE_FRAGMENT, "void main(){ this is not glsl }");
        assert_eq!(rc, -EINVAL, "{log}");
        release(&vs);
        release(&fs);
    }

    /// glLinkProgram makes the fragment shader the program: three QPU
    /// programs back to back, their prog_data and uniform streams, and the
    /// attribute records by location.
    #[test]
    fn linking_builds_the_program_into_the_fragment_shader() {
        let _s = store::test_lock();
        let before = store::keep_in_use();
        let (_, vs, _) = compile(SHC_STAGE_VERTEX, PASSTHROUGH_VS);
        let (_, mut fs, _) = compile(SHC_STAGE_FRAGMENT, INTERP_FS);
        let stage_priv = fs.priv_;
        let (rc, log) = link(&vs, &mut fs);
        assert_eq!(rc, 0, "{log}");
        assert_ne!(fs.priv_, stage_priv, "the program is a record of its own");
        let p = fs.v3d;
        assert_eq!(p.linked, 1);
        let bytes = fs.usc_dw * 4;
        assert!(bytes > 0 && bytes % 8 == 0);
        /* FS at 0, then VS, then CS, back to back */
        assert_eq!(p.fs.code_off, 0);
        assert_eq!(p.vs.code_off, p.fs.code_bytes);
        assert_eq!(p.cs.code_off, p.vs.code_off + p.vs.code_bytes);
        assert_eq!(p.cs.code_off + p.cs.code_bytes, bytes);
        assert_eq!(p.fs.num_inputs, 4, "v_color's four components");
        assert!(p.vs.num_uniforms > 0 && p.cs.num_uniforms > 0, "the viewport scales are uniforms");
        /* a_pos and a_color: two records, by location */
        assert_eq!(p.num_records, 2);
        let loc = |i: usize| vs.attribs[i].location as usize;
        assert_ne!(p.record_of_location[loc(0)], SHC_V3D_NO_RECORD);
        assert_ne!(p.record_of_location[loc(1)], SHC_V3D_NO_RECORD);
        assert_eq!(p.record_of_location.iter().filter(|&&r| r != SHC_V3D_NO_RECORD).count(), 2);
        /* the FS stage's reflection carries over */
        assert_eq!(fs.num_varyings, 1);

        /* a second program of the same pair is the cache's answer */
        let (_, mut fs2, _) = compile(SHC_STAGE_FRAGMENT, INTERP_FS);
        assert_eq!(link(&vs, &mut fs2).0, 0);
        assert_eq!(fs2.priv_, fs.priv_, "one record, two holders");
        for sh in [&fs, &fs2, &vs] {
            release(sh);
        }
        assert_eq!(store::keep_in_use(), before, "the last holders gave the records back");
    }

    /// What the chain does not build fails the link, with the chain's reason
    /// and the errno that says "not implemented", never "your shader is
    /// wrong".
    #[test]
    fn a_program_the_chain_refuses_fails_the_link_by_name() {
        let _s = store::test_lock();
        let before = store::keep_in_use();
        let (_, vs, _) = compile(SHC_STAGE_VERTEX, PASSTHROUGH_VS);
        let (rc, mut fs, log) = compile(
            SHC_STAGE_FRAGMENT,
            "precision mediump float; varying vec4 v_color; void main(){ vec4 c = v_color; for (float f = 0.0; f < 2.0; f += 1.0) { if (c.x > 0.5) break; gl_FragColor = c; c = c * v_color; } }",
        );
        assert_eq!(rc, 0, "the stage itself is valid: {log}");
        let (rc, log) = link(&vs, &mut fs);
        assert_eq!(rc, -ENOTSUP, "{log}");
        assert!(log.contains("loop"), "the log names the missing part: {log}");
        /* the program refused, not "this fragment shader": a vertex shader's
         * refusal read as the fragment's and sent a triage to the wrong
         * shader (dEQP vertex_arrays, gl_PointSize, 2026-09-24) */
        assert!(log.contains("refused this program (vertex + fragment)"), "{log}");
        assert!(!log.contains("fragment shader"), "{log}");
        release(&fs);
        release(&vs);
        assert_eq!(store::keep_in_use(), before);
    }

    /// GLSL ES 1.00 section 4.3.5 still applies: a varying the fragment
    /// shader reads must be declared by the vertex shader.
    #[test]
    fn an_undeclared_varying_fails_the_link() {
        let _s = store::test_lock();
        let (_, vs, _) = compile(SHC_STAGE_VERTEX, "attribute vec4 a_pos; void main(){ gl_Position = a_pos; }");
        let (_, mut fs, _) = compile(SHC_STAGE_FRAGMENT, INTERP_FS);
        let (rc, log) = link(&vs, &mut fs);
        assert_eq!(rc, -EINVAL, "{log}");
        assert!(log.contains("not declared in the vertex shader"), "{log}");
        release(&fs);
        release(&vs);
    }
}
