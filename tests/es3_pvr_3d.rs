// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR's native 3D texture (`pvr_native_3d`, `gl::native_vol`),
//! against docs/notes/pvr-3d-native.md: a sampler3D compiles to one `smp`
//! with `dmn` 3d over the three coordinates (the TPU picks the level,
//! filters across slices and wraps every axis), the record says native 3D
//! (`native_cube` bit 2), `textureSize` reads the image word, and with the
//! switch off the grid of slices (two 2D samples mixed, level 0) is what
//! comes out, as before.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use yakogl::backend::{
    self, GlesDraw, GlesHeap, GlesTextureBind, YakoglBackend, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_3D,
    GLES_TEXTYPE_3D_TWIDDLED, GLES_WRAP_CLAMP_TO_EDGE, GLES_WRAP_MIRRORED_REPEAT, GLES_WRAP_REPEAT,
};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::names::{self, Kind};
use yakogl::gl::native_vol::{
    self, chain_levels, copy_bytes, level_extent, level_texel_off, level_texels, physical_extent, twiddle3d,
    twiddle_extent,
};
use yakogl::gl::state::{glGetError, glPixelStorei};
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage3D};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_vertex};
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::ShcShader;

/// The compiler keeps its output in one arena that every compile resets,
/// and the switches are process-wide: one compile at a time.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The switch on or off for the closure, and back at its default after it,
/// panic or not.
fn with_native_3d<T>(on: bool, f: impl FnOnce() -> T) -> T {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    struct Off;
    impl Drop for Off {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_3d(yakogl::shaderc::PVR_NATIVE_3D_DEFAULT);
        }
    }
    let _off = Off;
    yakogl::shaderc::pvr_set_native_3d(i32::from(on));
    f()
}

/// One shader through the online PowerVR chain: the record (its code kept
/// by `store::begin_keep` for the closure) or the refusal.
fn compile_with<T>(stage: Stage, src: &str, f: impl FnOnce(&ShcShader, &[u8]) -> T) -> Result<T, String> {
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, stage, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = if stage == Stage::Fragment {
            compile_fragment(&l.module, &l.reflection, a)
        } else {
            compile_vertex(&l.module, &l.reflection, a)
        }
        .map_err(|e| format!("chain: {e:?}"))?;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        Ok(f(&rec, &image))
    })
    .unwrap()
}

/// The `smp` fields of every sample in a program, by name, in program order.
fn smps(image: &[u8]) -> Vec<BTreeMap<&'static str, u32>> {
    const NAMES: [&str; 11] =
        ["fcnorm", "dmn", "chan", "lodm", "pplod", "proj", "nncoords", "soo", "tao", "sbmode", "sno"];
    let mut off = 0usize;
    let mut out = Vec::new();
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        for (_, ph, r) in g.instrs() {
            if ops_for(r, ph).any(|o| o.name == "smp") {
                out.push(NAMES.iter().map(|n| (*n, r.get(n).unwrap_or(0))).collect());
            }
        }
        off += g.total;
    }
    out
}

fn fragment(decl: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\n{decl}\nuniform vec3 c;\nuniform vec3 g;\n\
         uniform float l;\nuniform ivec3 p;\nuniform int il;\nout vec4 o;\nvoid main() {{ {body} }}\n"
    )
}

fn vertex(decl: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\n{decl}\nuniform vec3 c;\n\
         uniform float l;\nuniform ivec3 p;\nuniform int il;\nin vec4 a;\nout vec4 v;\n\
         void main() {{ gl_Position = a; {body} }}\n"
    )
}

/// An `smp` field's enumerated value by name, from the ISA tables
/// (pco_isa.py): `lodm` normal/bias/replace/gradients, `dmn` 1d/2d/3d.
fn enum_value(field: &str, name: &str) -> u32 {
    yakogl::shaderc::online::powervr::isa_tables::ENUMS
        .iter()
        .find(|e| e.name == field)
        .and_then(|e| e.elems.iter().find(|(_, n)| *n == name))
        .map(|(v, _)| *v)
        .unwrap_or_else(|| panic!("{field} has no value {name}"))
}

/// One row per form of a sampler3D look-up: (the look-up, lodm, pplod,
/// nncoords). The level word is in the data block for a bias or a replaced
/// level (`pplod`), absent for the implicit one.
const FORMS: &[(&str, &str, u32, u32)] = &[
    ("texture(s, c)", "normal", 0, 0),
    ("texture(s, c, l)", "bias", 1, 0),
    ("textureLod(s, c, l)", "replace", 1, 0),
    ("textureProj(s, vec4(c, 2.0))", "normal", 0, 0),
    ("texelFetch(s, p, il)", "replace", 1, 1),
];

/// Every form compiles, in a fragment shader, to one `smp` with `dmn` 3d,
/// no `tao`, four channels, `fcnorm` for a float sampler and the row's level
/// mode; the record says native 3D (bit 2) and neither cube nor array.
#[test]
fn every_form_samples_the_tpus_3d_texture() {
    for (e, mode, pplod, nncoords) in FORMS {
        let src = fragment("uniform highp sampler3D s;", &format!("o = {e};"));
        let (flag, fields) = with_native_3d(true, || {
            compile_with(Stage::Fragment, &src, |rec, image| (rec.native_cube, smps(image)))
                .unwrap_or_else(|m| panic!("{e}: {m}"))
        });
        assert_eq!(flag, 4, "{e}: the record says native 3D");
        assert_eq!(fields.len(), 1, "{e}: one sample, not two slices mixed");
        let want: BTreeMap<&str, u32> = [
            ("fcnorm", 1),
            ("dmn", enum_value("dmn", "3d")),
            ("chan", 4),
            ("lodm", enum_value("lodm", mode)),
            ("pplod", *pplod),
            ("proj", 0),
            ("nncoords", *nncoords),
            ("soo", 0),
            ("tao", 0),
            ("sbmode", 0),
            ("sno", 0),
        ]
        .into_iter()
        .collect();
        assert_eq!(fields[0], want, "{e}");
    }
}

/// A vertex shader samples the same way (textureLod is its only form).
#[test]
fn a_vertex_shader_samples_the_volume_too() {
    let src = vertex("uniform highp sampler3D s;", "v = textureLod(s, c, l);");
    let (flag, fields) = with_native_3d(true, || {
        compile_with(Stage::Vertex, &src, |rec, image| (rec.native_cube, smps(image))).unwrap()
    });
    assert_eq!(flag, 4);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0]["dmn"], enum_value("dmn", "3d"));
    assert_eq!(fields[0]["lodm"], enum_value("lodm", "replace"));
}

/// With the switch off the volume stays the grid of slices: the two slices
/// around the depth, sampled as 2D images and mixed -- two `smp`, `dmn` 2d,
/// the record clear.
#[test]
fn off_the_volume_stays_the_grid_of_slices() {
    let src = fragment("uniform highp sampler3D s;", "o = texture(s, c);");
    let (flag, fields) = with_native_3d(false, || {
        compile_with(Stage::Fragment, &src, |rec, image| (rec.native_cube, smps(image))).unwrap()
    });
    assert_eq!(flag, 0);
    assert_eq!(fields.len(), 2, "two slices");
    assert!(fields.iter().all(|f| f["dmn"] == enum_value("dmn", "2d")));
}

/// A dead volume look-up takes its sample and its third coordinate with it:
/// the program compiles and the record says nothing. A program with a cube,
/// an array-free volume and a native cube says both bits (5).
#[test]
fn a_dead_sample_sets_nothing_and_a_cube_beside_a_volume_sets_both_bits() {
    let dead = fragment("uniform highp sampler3D s;", "vec4 t = texture(s, c); o = vec4(c, 1.0);");
    let flag = with_native_3d(true, || compile_with(Stage::Fragment, &dead, |rec, _| rec.native_cube).unwrap());
    assert_eq!(flag, 0, "a dead look-up: compiled, nothing native left");
    let both =
        fragment("uniform highp sampler3D s;\nuniform highp samplerCube k;", "o = texture(s, c) + texture(k, c);");
    let flag = with_native_3d(true, || {
        yakogl::shaderc::pvr_set_native_cube(1);
        let f = compile_with(Stage::Fragment, &both, |rec, _| rec.native_cube);
        yakogl::shaderc::pvr_set_native_cube(yakogl::shaderc::PVR_NATIVE_CUBE_DEFAULT);
        f.unwrap()
    });
    assert_eq!(flag, 5, "a native cube (bit 0) and a native volume (bit 2) in one program");
}

/// `textureSize` of a native volume reads the image word: no `smp`, and the
/// program compiles with a level argument. The word is the twiddled copy's
/// (a grid's is a stride image with no depth), so the record asks for that
/// copy (bit 2) even though nothing is sampled.
#[test]
fn texture_size_reads_the_image_word_and_samples_nothing() {
    let src = fragment("uniform highp sampler3D s;", "o = vec4(textureSize(s, il), 1.0);");
    let (flag, n) = with_native_3d(true, || {
        compile_with(Stage::Fragment, &src, |rec, image| (rec.native_cube, smps(image).len())).unwrap()
    });
    assert_eq!((flag, n), (4, 0));
}

/// The sampler words of a descriptor as the TPU reads them (texstate.xml
/// SAMPLER_WORD0): (minlod, maxlod, mipfilter, addrmode u, v, w).
fn sampler(w: &[u32]) -> (u64, u64, u64, u64, u64, u64) {
    let q = w[8] as u64 | (w[9] as u64) << 32;
    ((q >> 13) & 0x3ff, (q >> 23) & 0x3ff, (q >> 40) & 1, (q >> 41) & 7, (q >> 44) & 7, (q >> 56) & 7)
}

/// A native 3D bind of `w x h x d` (level 0), `levels` levels.
fn volume_bind(w: u32, h: u32, d: u16, levels: u8, mip_filter: u8) -> GlesTextureBind {
    let phys = physical_extent(w, h, u32::from(d));
    GlesTextureBind {
        format: GLES_TEXFMT_RGBA8,
        type_: GLES_TEXTYPE_3D_TWIDDLED,
        layers: d,
        base_gpu: 0x80_0000_9000,
        width: w,
        height: h,
        stride_texels: w,
        bytes: copy_bytes(phys, 4, u32::from(levels)),
        linear_filter: 1,
        min_linear: 1,
        mip_filter,
        mip_levels: levels,
        wrap_s: GLES_WRAP_REPEAT,
        wrap_t: GLES_WRAP_REPEAT,
        wrap_r: GLES_WRAP_REPEAT,
        ..GlesTextureBind::ZERO
    }
}

/// (a) A mipmapped NPOT volume: the levels follow each other in the
/// physical chain (63x57x67: physical 64x64x128, eight levels to 1x1x1, each
/// at the offset `native_vol` computes), and `native_vol::copy` writes each
/// texel of each API level at its level's offset plus `twiddle3d` over the
/// level's API extent (`twiddle_extent`: its size halved, at least 1), not
/// over the physical level's extent, inside the level's padded extent: no
/// level writes into another, nothing else is written, and the copy's size
/// is the last level's end. 3x7x5 is a size of dEQP's
/// texture.filtering.3d.sizes; 17x9x5 is one whose level 1 (8x4x2, in a
/// 16x8x4 level) twiddles differently over the two extents, so a copy in
/// the physical order fails here.
#[test]
fn an_npot_mipmapped_volume_writes_every_level_at_its_offset() {
    let phys = physical_extent(63, 57, 67);
    assert_eq!(phys, (64, 64, 128));
    let n = chain_levels(phys);
    assert_eq!(n, 8, "128 -> 1");
    let mut off = 0;
    for l in 0..n {
        assert_eq!(level_texel_off(phys, l), off, "level {l} follows the one before");
        let e = level_extent(phys, l);
        assert_eq!(e, ((64 >> l).max(1), (64 >> l).max(1), (128 >> l).max(1)));
        off += level_texels(e);
    }
    assert_eq!(copy_bytes(phys, 4, 1), level_texels(phys) * 4, "no mipmap filter: level 0 alone");
    for levels in 2..=n {
        assert_eq!(copy_bytes(phys, 4, levels), off * 4, "{levels} levels: the whole chain");
    }
    /* the two orders differ where an API level rounds to fewer cubes than
     * the physical level on an axis that is not the last to drop out:
     * 17x9x5's level 1, texel (4, 0, 0) */
    let p17 = physical_extent(17, 9, 5);
    assert_eq!((twiddle_extent(17, 9, 5, 1), level_extent(p17, 1)), ((8, 4, 2), (16, 8, 4)));
    assert_ne!(twiddle3d(4, 0, 0, twiddle_extent(17, 9, 5, 1)), twiddle3d(4, 0, 0, level_extent(p17, 1)));
    for size in [(3, 7, 5), (17, 9, 5), (63, 57, 67)] {
        copy_lands_at_the_twiddle_extent(size);
    }
}

/* ---- a GL context on the host for `native_vol::copy`: host memory, and a
 * stand-in back end with the PowerVR's texture layout (no native layers, a
 * 3D texture's slices in a grid, levels unaligned) that draws nothing ---- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
    0
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

static GRID: YakoglBackend = YakoglBackend {
    name: c"es3-pvr-3d-grid".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_ok),
    finish: Some(b_ok),
    finish_previous: None,
    kick_id: None,
    kick_pending: None,
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
};

/// The one GL context of this file, current, with [`GRID`] as the back end.
fn gl_context() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static ONCE: std::sync::Once = std::sync::Once::new();
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the tables are statics; the lock serialises the GL users.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &GRID;
            assert!(!context::gl_context_create(8, 8).is_null());
        });
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

/// Texel (x, y, z) of level `l`: its coordinates and level, so no two
/// texels of the volumes here are the same.
fn texel(l: u32, x: u32, y: u32, z: u32) -> [u8; 4] {
    [x as u8, y as u8, z as u8, 0x80 | l as u8]
}

/// A `w x h x d` RGBA8 volume with every level of its chain uploaded
/// (glTexImage3D, texels [`texel`]), copied by `native_vol::copy` with all
/// its levels: each texel of level l is at `level_texel_off(l) +
/// twiddle3d(x, y, z, twiddle_extent(l))`, inside level l's padded extent,
/// and every texel of the copy that is no level's is zero.
fn copy_lands_at_the_twiddle_extent((w, h, d): (u32, u32, u32)) {
    let _g = gl_context();
    let levels = 32 - w.max(h).max(d).leading_zeros();
    let mut name = 0;
    // SAFETY: room for one name; each level's pixels hold its texels.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_3D, name);
        for l in 0..levels {
            let (lw, lh, ld) = twiddle_extent(w, h, d, l);
            assert_eq!((lw, lh, ld), ((w >> l).max(1), (h >> l).max(1), (d >> l).max(1)), "API level {l}");
            let mut px = Vec::new();
            for z in 0..ld {
                for y in 0..lh {
                    for x in 0..lw {
                        px.extend_from_slice(&texel(l, x, y, z));
                    }
                }
            }
            glTexImage3D(
                GL_TEXTURE_3D,
                l as GLint,
                GL_RGBA8 as GLint,
                lw as GLsizei,
                lh as GLsizei,
                ld as GLsizei,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            );
            assert_eq!(glGetError(), GL_NO_ERROR, "{w}x{h}x{d} level {l}");
        }
    }
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
    let t = c.texture_object(slot).expect("a texture object");
    assert_ne!(t.grid_cols, 0, "{w}x{h}x{d}: the PowerVR's grid of slices");
    let (gpu, bytes) = native_vol::copy(t, 0, levels).unwrap_or_else(|| panic!("{w}x{h}x{d}: copied"));
    let phys = physical_extent(w, h, d);
    assert_eq!(bytes, copy_bytes(phys, 4, levels), "{w}x{h}x{d}: the copy's size");
    // SAFETY: the host OSAL's GPU address is its CPU pointer, `bytes` long.
    let out = unsafe { core::slice::from_raw_parts(gpu as *const u8, bytes as usize) };
    let mut written = vec![false; (bytes / 4) as usize];
    for l in 0..levels {
        let ext = twiddle_extent(w, h, d, l);
        let (base, room) = (level_texel_off(phys, l), level_texels(level_extent(phys, l)));
        for z in 0..ext.2 {
            for y in 0..ext.1 {
                for x in 0..ext.0 {
                    let i = twiddle3d(x, y, z, ext);
                    assert!(i < room, "{w}x{h}x{d} level {l} ({x}, {y}, {z}) = {i} leaves its {room} texels");
                    let at = (base + i) as usize;
                    assert!(!written[at], "{w}x{h}x{d} level {l} ({x}, {y}, {z}) collides at {i}");
                    written[at] = true;
                    assert_eq!(
                        out[at * 4..at * 4 + 4],
                        texel(l, x, y, z),
                        "{w}x{h}x{d} level {l} ({x}, {y}, {z}) at {base} + {i}"
                    );
                }
            }
        }
    }
    for (at, _) in written.iter().enumerate().filter(|(_, w)| !**w) {
        assert_eq!(out[at * 4..at * 4 + 4], [0; 4], "{w}x{h}x{d}: texel {at} of the copy is no level's");
    }
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &name) };
}

/// (b) The descriptor of the NPOT volume carries level 0's API size, the
/// depth, the level count and the LOD range, so LINEAR_MIPMAP_LINEAR walks
/// above level 0: maxlod is 64 per level less one (u4.6), minlod stays 0 and
/// the mip filter bit is set only for a linear mix between levels.
#[test]
fn the_descriptor_carries_the_level_count_and_the_lod_range() {
    let t = volume_bind(63, 57, 67, 7, 2);
    let w = yakogl::pvr::backend::texture_descriptor(&t);
    let iw0 = w[0] as u64 | (w[1] as u64) << 32;
    let iw1 = w[2] as u64 | (w[3] as u64) << 32;
    assert_eq!(iw0 & 7, 2, "TEXTYPE_3D");
    assert_eq!(((iw0 >> 34) & 0x3fff, (iw0 >> 48) & 0x3fff), (62, 56), "level 0's API size");
    assert_eq!(iw1 & 0xf, 7, "seven levels");
    assert_eq!((iw1 >> 4) & 0x7ff, 66, "depth - 1");
    assert_eq!((iw1 >> 15) & 1, 1, "mipmaps_present");
    let (minlod, maxlod, mip, ..) = sampler(&w);
    assert_eq!((minlod, maxlod, mip), (0, 6 * 64, 1), "LOD 0 to 6, mixing levels");
    for levels in 2u8..=8 {
        let w = yakogl::pvr::backend::texture_descriptor(&volume_bind(63, 57, 67, levels, 2));
        let s = sampler(&w);
        assert_eq!(w[2] & 0xf, u32::from(levels));
        assert_eq!((s.0, s.1, s.2), (0, u64::from(levels - 1) * 64, 1), "{levels} levels");
    }
    /* MIPMAP_NEAREST: the levels are walked but not mixed */
    let n = sampler(&yakogl::pvr::backend::texture_descriptor(&volume_bind(63, 57, 67, 7, 1)));
    assert_eq!((n.1, n.2), (6 * 64, 0));
    /* no mipmap filter: one level, LOD 0 */
    let w1 = yakogl::pvr::backend::texture_descriptor(&volume_bind(63, 57, 67, 1, 0));
    let s1 = sampler(&w1);
    assert_eq!((w1[2] & 0xf, (w1[3] >> 15) & 1, s1.1, s1.2), (1, 0, 0, 0));
}

/// (c) REPEAT and MIRRORED_REPEAT reach the sampler on S, T and R: each axis
/// has its own mode, none is clamped, and they do not leak into each other.
#[test]
fn repeat_and_mirrored_repeat_reach_the_sampler_on_every_axis() {
    const REPEAT: u64 = 0;
    const FLIP: u64 = 1;
    const CLAMP: u64 = 2;
    let modes = [(GLES_WRAP_REPEAT, REPEAT), (GLES_WRAP_MIRRORED_REPEAT, FLIP), (GLES_WRAP_CLAMP_TO_EDGE, CLAMP)];
    for (ms, es) in modes {
        for (mt, et) in modes {
            for (mr, er) in modes {
                let t = GlesTextureBind { wrap_s: ms, wrap_t: mt, wrap_r: mr, ..volume_bind(63, 57, 67, 7, 2) };
                let s = sampler(&yakogl::pvr::backend::texture_descriptor(&t));
                assert_eq!((s.3, s.4, s.5), (es, et, er), "s={ms} t={mt} r={mr}");
            }
        }
    }
}

/// (d) With the switch off the texture is a grid of slices: the descriptor is
/// the stride 2D image it always was -- not TEXTYPE_3D, one level, the third
/// axis clamped whatever WRAP_R says (the grid's depth is the shader's).
#[test]
fn off_the_descriptor_is_the_grids_stride_image() {
    let t = GlesTextureBind {
        type_: GLES_TEXTYPE_3D,
        height: 8 * 4,
        bytes: 8 * 8 * 4 * 4 * 4,
        ..volume_bind(8, 8, 4, 1, 0)
    };
    let w = yakogl::pvr::backend::texture_descriptor(&t);
    assert_eq!(w[0] & 7, 4, "a STRIDE image, not TEXTYPE_3D");
    let (_, maxlod, mip, _, _, wr) = sampler(&w);
    assert_eq!((maxlod, mip), (0, 0), "level 0 only");
    assert_eq!(wr, 2, "the third axis stays CLAMP_TO_EDGE under REPEAT");
    /* the same bind, native: REPEAT on R is the TPU's */
    let n =
        sampler(&yakogl::pvr::backend::texture_descriptor(&GlesTextureBind { type_: GLES_TEXTYPE_3D_TWIDDLED, ..t }));
    assert_eq!(n.5, 0);
}
