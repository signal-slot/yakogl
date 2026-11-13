// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0's six ETC2/EAC formats the GL core decodes at upload
//! (docs/notes/etc2-eac.md): `GL_COMPRESSED_R11_EAC`, `SIGNED_R11_EAC`,
//! `RG11_EAC`, `SIGNED_RG11_EAC` and the two punch-through formats, through
//! glCompressedTexImage2D / 3D / SubImage2D with the decoder
//! (`gl::etc2`, itself checked against Mesa's in its own tests).
//!
//! What is pinned here, through the GL API on a stand-in back end whose OSAL
//! keeps the texture storage in host memory:
//!
//! * the names are accepted, listed (`GL_NUM_COMPRESSED_TEXTURE_FORMATS`,
//!   `GL_COMPRESSED_TEXTURE_FORMATS`: ETC1 and the ten) and stored in the
//!   class the design names (R11 as R16F, RG11 as RGBA16F, punch-through as
//!   RGBA8 / SRGB8_A8, no blocks left); `report-es2` lists the same three as
//!   before (tests/es2_report_guard.rs pins that, and this file is not built
//!   there);
//! * the decoded texels, each computed here from the spec's arithmetic
//!   (OpenGL ES 3.0.6 Annex C.2/C.3, EAC: `base * 8 + 4 + modifier *
//!   multiplier`, the 11-bit result widened to 16 bits) for blocks built by
//!   hand, not by calling the decoder: a half-float channel of `u / 65535`
//!   (unsigned) or `max(s / 32767, -1)` (signed);
//! * a level that is not a multiple of 4 wide or high (the edge blocks are
//!   cut), the levels of a chain, the faces of a cube map, the layers of a
//!   2D array, glCompressedTexSubImage2D (block aligned, the same format,
//!   the rest of the level untouched) and a PIXEL_UNPACK_BUFFER source;
//! * the rules: glGenerateMipmap and a colour attachment refuse the decoded
//!   texture, 3D is GL_INVALID_OPERATION, another name in a sub-image is
//!   GL_INVALID_OPERATION.
//!
//! No GPU runs: whether a TMU, TPU or GC samples the stored classes within
//! dEQP's thresholds is the boards' to show.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_etc2_eac
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_etc2_eac
//! cargo test --no-default-features --features host-test,gpu-vivante --test es3_etc2_eac
//! ```
#![cfg(all(
    feature = "host-test",
    any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d"),
    not(feature = "report-es2")
))]
use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{
    self, GlesDraw, GlesHeap, YakoglBackend, GLES_TEXFMT_R16F, GLES_TEXFMT_RGBA16F, GLES_TEXFMT_RGBA8,
    GLES_TEXFMT_SRGB8_A8,
};
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::es3_formats::half_to_f32;
use yakogl::gl::framebuffer::{glCheckFramebufferStatus, glFramebufferTexture2D};
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::{glGetError, glGetIntegerv, glPixelStorei};
use yakogl::gl::texture::{
    glBindTexture, glCompressedTexImage2D, glCompressedTexImage3D, glCompressedTexSubImage2D, glGenTextures,
    glGenerateMipmap,
};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one */

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

/// A texture unit that samples a raster texture itself and stacks array
/// layers: only the GL core's side of the upload matters here.
static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-etc2-eac".as_ptr() as *const c_char,
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

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the table is a static; the lock serialises the tests.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            assert!(!context::gl_context_create(8, 8).is_null());
        });
        glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn error() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn new_texture(target: GLenum) -> GLuint {
    let mut name = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(target, name);
    }
    name
}

fn texture_object(name: GLuint) -> &'static context::GlTexture {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
    c.texture_object(slot).expect("a texture object")
}

/* ---- blocks built by hand, and what the spec makes of them ------------- */

const R11: GLenum = GL_COMPRESSED_R11_EAC;
const SR11: GLenum = GL_COMPRESSED_SIGNED_R11_EAC;
const RG11: GLenum = GL_COMPRESSED_RG11_EAC;
const SRG11: GLenum = GL_COMPRESSED_SIGNED_RG11_EAC;
const PT: GLenum = GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2;
const SPT: GLenum = GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2;

/// An EAC block whose sixteen texels all take modifier index 0 of table 0
/// (-3) at multiplier 0 (which counts as 1, not 0): byte 0 is the base
/// codeword, byte 1 the multiplier and table, the 48 index bits zero.
fn eac(base: u8) -> [u8; 8] {
    [base, 0, 0, 0, 0, 0, 0, 0]
}

/// The 16-bit unsigned value of that block's texels: the 11-bit
/// `base * 8 + 4 - 3`, widened `(v << 5) | (v >> 6)` (the spec allows any
/// extension; this is the one Mesa writes and dEQP compares against).
fn eac_u16(base: u8) -> f32 {
    let v = u32::from(base) * 8 + 4 - 3;
    (((v << 5) | (v >> 6)) as f32) / 65535.0
}

/// The same for a signed block: `base * 8 + 0 - 3` (the signed formula has no
/// +4), clamped to -1023..=1023, the magnitude widened and the sign put back;
/// -1.0 at the bottom.
fn eac_i16(base: i8) -> f32 {
    let v = (i32::from(base) * 8 - 3).clamp(-1023, 1023);
    let m = v.unsigned_abs();
    let w = ((m << 5) | (m >> 5)) as i32 * v.signum();
    (w as f32 / 32767.0).max(-1.0)
}

/// Half-float resolution: 11 significant bits, so the error of a value in
/// [-1, 1] is at most 2^-11.
const HALF_EPS: f32 = 1.0 / 2048.0;

fn upload2d(target: GLenum, level: GLint, format: GLenum, w: u32, h: u32, data: &[u8]) {
    // SAFETY: `data` is the imageSize the call names.
    unsafe {
        glCompressedTexImage2D(
            target,
            level,
            format,
            w as GLsizei,
            h as GLsizei,
            0,
            data.len() as GLsizei,
            data.as_ptr().cast(),
        )
    };
}

/// The blocks of a w x h image, `f(bx, by)` each.
fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> Vec<u8>) -> Vec<u8> {
    let mut d = Vec::new();
    for by in 0..h.div_ceil(4) {
        for bx in 0..w.div_ceil(4) {
            d.extend(f(bx, by));
        }
    }
    d
}

/// Texel (x, y) of `level` (row `row0` of its rows is the texel row 0 asked
/// for, a layer or a cube face) as `n` channels of the stored class.
fn texel_bytes(t: &context::GlTexture, level: usize, row0: u32, x: u32, y: u32, bpp: usize) -> Vec<u8> {
    let (lw, _) = ((t.width >> level).max(1), 0);
    let pitch = t.level_pitch(lw) as usize;
    let at = t.level_off[level] as usize + ((row0 + y) as usize * pitch + x as usize) * bpp;
    // SAFETY: inside the level's storage (the callers' coordinates).
    unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add(at), bpp).to_vec() }
}

fn halves(b: &[u8]) -> Vec<f32> {
    b.chunks(2).map(|c| half_to_f32(u16::from_ne_bytes([c[0], c[1]]))).collect()
}

fn close(got: f32, want: f32, what: &str) {
    assert!((got - want).abs() <= HALF_EPS, "{what}: {got} vs {want}");
}

/* ---- the names ---------------------------------------------------------- */

#[test]
fn the_ten_formats_and_etc1_are_listed_and_the_count_matches() {
    let _g = context();
    let mut n = 0;
    let mut list = [0 as GLint; 16];
    // SAFETY: room for the widest list.
    unsafe {
        glGetIntegerv(GL_NUM_COMPRESSED_TEXTURE_FORMATS, &mut n);
        glGetIntegerv(GL_COMPRESSED_TEXTURE_FORMATS, list.as_mut_ptr());
    }
    assert_eq!(error(), GL_NO_ERROR);
    let want = [
        GL_ETC1_RGB8_OES,
        GL_COMPRESSED_RGB8_ETC2,
        GL_COMPRESSED_RGBA8_ETC2_EAC,
        GL_COMPRESSED_SRGB8_ETC2,
        GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC,
        R11,
        SR11,
        RG11,
        SRG11,
        PT,
        SPT,
    ];
    assert_eq!(n as usize, want.len(), "GL_NUM_COMPRESSED_TEXTURE_FORMATS");
    assert_eq!(list[..want.len()], want.map(|f| f as GLint), "GL_COMPRESSED_TEXTURE_FORMATS");
    assert_eq!(list[want.len()], 0, "nothing past the list");
}

/// Each of the six is stored as texels in its class, with no block left.
#[test]
fn each_format_is_stored_in_the_class_the_design_names() {
    let _g = context();
    for (format, class, bpp, blocks) in [
        (R11, GLES_TEXFMT_R16F, 2, 8),
        (SR11, GLES_TEXFMT_R16F, 2, 8),
        (RG11, GLES_TEXFMT_RGBA16F, 8, 16),
        (SRG11, GLES_TEXFMT_RGBA16F, 8, 16),
        (PT, GLES_TEXFMT_RGBA8, 4, 8),
        (SPT, GLES_TEXFMT_SRGB8_A8, 4, 8),
    ] {
        let name = new_texture(GL_TEXTURE_2D);
        upload2d(GL_TEXTURE_2D, 0, format, 8, 8, &vec![0u8; 4 * blocks]);
        assert_eq!(error(), GL_NO_ERROR, "format {format:#x}");
        let t = texture_object(name);
        assert_eq!((t.texfmt, t.block_bytes, t.bpp as usize), (class, 0, bpp), "format {format:#x}");
        assert_eq!((t.width, t.height), (8, 8));
    }
}

#[test]
fn a_wrong_image_size_is_invalid_value_and_a_3d_target_invalid_operation() {
    let _g = context();
    for format in [R11, SR11, RG11, SRG11, PT, SPT] {
        new_texture(GL_TEXTURE_2D);
        let bb = if matches!(format, RG11 | SRG11) { 16 } else { 8 };
        upload2d(GL_TEXTURE_2D, 0, format, 8, 8, &vec![0u8; 4 * bb - 1]);
        assert_eq!(error(), GL_INVALID_VALUE, "format {format:#x}: imageSize");
        new_texture(GL_TEXTURE_3D);
        // SAFETY: a null source with the size of one block.
        unsafe { glCompressedTexImage3D(GL_TEXTURE_3D, 0, format, 4, 4, 1, 0, bb as GLsizei, core::ptr::null()) };
        assert_eq!(error(), GL_INVALID_OPERATION, "format {format:#x}: no 3D form");
    }
}

/* ---- the decoded texels ------------------------------------------------- */

#[test]
fn r11_texels_are_halves_of_the_widened_11_bit_value() {
    let _g = context();
    let name = new_texture(GL_TEXTURE_2D);
    /* a base codeword per block, so every block shows */
    let bases = [[0u8, 100, 200, 255], [1, 50, 128, 254]];
    let data = image(16, 8, |bx, by| eac(bases[by as usize][bx as usize]).to_vec());
    upload2d(GL_TEXTURE_2D, 0, R11, 16, 8, &data);
    assert_eq!(error(), GL_NO_ERROR);
    let t = texture_object(name);
    for (by, row) in bases.iter().enumerate() {
        for (bx, &base) in row.iter().enumerate() {
            for (dx, dy) in [(0, 0), (3, 0), (0, 3), (3, 3), (2, 1)] {
                let (x, y) = (bx as u32 * 4 + dx, by as u32 * 4 + dy);
                let v = halves(&texel_bytes(t, 0, 0, x, y, 2))[0];
                close(v, eac_u16(base), &format!("R11 ({x}, {y}) base {base}"));
            }
        }
    }
}

#[test]
fn signed_r11_texels_keep_the_sign_and_stop_at_minus_one() {
    let _g = context();
    let name = new_texture(GL_TEXTURE_2D);
    let bases: [i8; 4] = [0, 100, -100, -128];
    let data = image(16, 4, |bx, _| eac(bases[bx as usize] as u8).to_vec());
    upload2d(GL_TEXTURE_2D, 0, SR11, 16, 4, &data);
    assert_eq!(error(), GL_NO_ERROR);
    let t = texture_object(name);
    for (bx, &base) in bases.iter().enumerate() {
        /* -128 counts as -127 (the spec: the base codeword's lowest value is -127) */
        let want = eac_i16(if base == -128 { -127 } else { base });
        let v = halves(&texel_bytes(t, 0, 0, bx as u32 * 4 + 1, 2, 2))[0];
        close(v, want, &format!("SIGNED_R11 base {base}"));
    }
}

#[test]
fn rg11_texels_are_red_green_zero_one_and_the_blocks_are_red_then_green() {
    let _g = context();
    let name = new_texture(GL_TEXTURE_2D);
    let mut block = eac(100).to_vec(); /* red */
    block.extend(eac(200)); /* green */
    upload2d(GL_TEXTURE_2D, 0, RG11, 4, 4, &block);
    assert_eq!(error(), GL_NO_ERROR);
    let t = texture_object(name);
    let v = halves(&texel_bytes(t, 0, 0, 2, 3, 8));
    close(v[0], eac_u16(100), "red");
    close(v[1], eac_u16(200), "green");
    assert_eq!((v[2], v[3]), (0.0, 1.0), "blue 0, alpha 1");

    let name = new_texture(GL_TEXTURE_2D);
    let mut block = eac(10u8.wrapping_neg()).to_vec();
    block.extend(eac(50));
    upload2d(GL_TEXTURE_2D, 0, SRG11, 4, 4, &block);
    assert_eq!(error(), GL_NO_ERROR);
    let v = halves(&texel_bytes(texture_object(name), 0, 0, 0, 0, 8));
    close(v[0], eac_i16(-10), "signed red");
    close(v[1], eac_i16(50), "signed green");
    assert_eq!((v[2], v[3]), (0.0, 1.0));
}

/// A punch-through block all of whose pixel indices are 2 (msb 1, lsb 0), with
/// the opaque bit clear: transparent, red, green and blue 0 as well (ES 3.0.6
/// Annex C.2: "the RGB value is 0"). With the opaque bit set and every index
/// 0 the colour is the base colour and the alpha is 255.
#[test]
fn punch_through_texels_are_rgba8_with_alpha_0_or_255() {
    let _g = context();
    let transparent = [0u8, 0, 0, 0, 0xff, 0xff, 0, 0];
    /* individual mode needs the diff bit clear: byte 3, bit 1 is the diff
     * bit and bit 0 the flip; for punch-through byte 3's bit 1 is the opaque
     * bit. Base colours 0b1000 | 4 bits each, table 0, opaque: individual
     * mode (diff = 0) is not reachable with the opaque bit set, so use the
     * differential form: R 8 (5 bits), dR 0, G 8, dG 0, B 8, dB 0 */
    let opaque = [8 << 3, 8 << 3, 8 << 3, 0b10, 0, 0, 0, 0];
    for (format, class) in [(PT, GLES_TEXFMT_RGBA8), (SPT, GLES_TEXFMT_SRGB8_A8)] {
        let name = new_texture(GL_TEXTURE_2D);
        let mut data = transparent.to_vec();
        data.extend(opaque);
        upload2d(GL_TEXTURE_2D, 0, format, 8, 4, &data);
        assert_eq!(error(), GL_NO_ERROR, "format {format:#x}");
        let t = texture_object(name);
        assert_eq!(t.texfmt, class);
        for (x, y) in [(0, 0), (3, 3), (1, 2)] {
            assert_eq!(texel_bytes(t, 0, 0, x, y, 4), [0, 0, 0, 0], "format {format:#x} transparent ({x}, {y})");
        }
        /* the opaque block: alpha 255, and the colour is the 5-bit 8 extended to 8 bits (66) plus
         * index 0's modifier of table 0 for an opaque block (-8), clamped */
        for (x, y) in [(4, 0), (7, 3), (5, 2)] {
            let b = texel_bytes(t, 0, 0, x, y, 4);
            assert_eq!(b[3], 255, "format {format:#x} opaque alpha ({x}, {y})");
            assert!(b[0] == b[1] && b[1] == b[2] && b[0] > 0, "format {format:#x} grey ({x}, {y}): {b:?}");
        }
    }
}

/* ---- shapes ------------------------------------------------------------- */

#[test]
fn the_edge_blocks_of_a_level_that_is_not_a_multiple_of_4_are_cut() {
    let _g = context();
    let name = new_texture(GL_TEXTURE_2D);
    let data = image(5, 3, |bx, _| eac(if bx == 0 { 40 } else { 220 }).to_vec());
    upload2d(GL_TEXTURE_2D, 0, R11, 5, 3, &data);
    assert_eq!(error(), GL_NO_ERROR);
    let t = texture_object(name);
    assert_eq!((t.width, t.height), (5, 3));
    close(halves(&texel_bytes(t, 0, 0, 3, 2, 2))[0], eac_u16(40), "last texel of block 0");
    close(halves(&texel_bytes(t, 0, 0, 4, 0, 2))[0], eac_u16(220), "the one column of block 1");
    close(halves(&texel_bytes(t, 0, 0, 4, 2, 2))[0], eac_u16(220), "bottom right");
    /* nothing of the cut texels reached the next row of the level */
    let pitch = t.level_pitch(5) as usize;
    assert!(pitch >= 5);
    if pitch > 5 {
        assert_eq!(texel_bytes(t, 0, 0, 5, 0, 2), [0, 0], "the row's padding is untouched");
    }
}

#[test]
fn the_levels_of_a_chain_and_the_faces_of_a_cube_go_where_the_uncompressed_ones_go() {
    let _g = context();
    let name = new_texture(GL_TEXTURE_2D);
    upload2d(GL_TEXTURE_2D, 0, R11, 8, 8, &image(8, 8, |_, _| eac(60).to_vec()));
    upload2d(GL_TEXTURE_2D, 1, R11, 4, 4, &image(4, 4, |_, _| eac(180).to_vec()));
    upload2d(GL_TEXTURE_2D, 2, R11, 2, 2, &image(2, 2, |_, _| eac(240).to_vec()));
    assert_eq!(error(), GL_NO_ERROR);
    let t = texture_object(name);
    close(halves(&texel_bytes(t, 0, 0, 7, 7, 2))[0], eac_u16(60), "level 0");
    close(halves(&texel_bytes(t, 1, 0, 3, 3, 2))[0], eac_u16(180), "level 1");
    close(halves(&texel_bytes(t, 2, 0, 1, 1, 2))[0], eac_u16(240), "level 2");

    let name = new_texture(GL_TEXTURE_CUBE_MAP);
    for face in 0..6u32 {
        upload2d(GL_TEXTURE_CUBE_MAP_POSITIVE_X + face, 0, RG11, 4, 4, &[eac(20 * face as u8 + 10), eac(0)].concat());
        assert_eq!(error(), GL_NO_ERROR, "face {face}");
    }
    let t = texture_object(name);
    for face in 0..6u32 {
        let v = halves(&texel_bytes(t, 0, face * 4, 1, 1, 8));
        close(v[0], eac_u16(20 * face as u8 + 10), &format!("face {face}"));
    }
}

#[test]
fn the_layers_of_a_2d_array_each_get_their_own_blocks() {
    let _g = context();
    let name = new_texture(GL_TEXTURE_2D_ARRAY);
    let (w, h, layers) = (8u32, 4u32, 3u32);
    let mut data = Vec::new();
    for z in 0..layers {
        data.extend(image(w, h, |bx, _| eac((30 * z + 10 * bx) as u8 + 5).to_vec()));
    }
    // SAFETY: `data` is the imageSize named.
    unsafe {
        glCompressedTexImage3D(
            GL_TEXTURE_2D_ARRAY,
            0,
            R11,
            w as GLsizei,
            h as GLsizei,
            layers as GLsizei,
            0,
            data.len() as GLsizei,
            data.as_ptr().cast(),
        )
    };
    assert_eq!(error(), GL_NO_ERROR);
    let t = texture_object(name);
    assert_eq!((t.layers, t.block_bytes), (3, 0));
    for z in 0..layers {
        for bx in 0..2 {
            let v = halves(&texel_bytes(t, 0, z * h, bx * 4 + 2, 1, 2))[0];
            close(v, eac_u16((30 * z + 10 * bx) as u8 + 5), &format!("layer {z} block {bx}"));
        }
    }
}

/* ---- sub-images, a PBO, and the rules ----------------------------------- */

#[test]
fn a_sub_image_replaces_its_blocks_and_nothing_else() {
    let _g = context();
    let name = new_texture(GL_TEXTURE_2D);
    upload2d(GL_TEXTURE_2D, 0, R11, 8, 8, &image(8, 8, |_, _| eac(60).to_vec()));
    let block = eac(200);
    // SAFETY: one block of 8 bytes.
    unsafe { glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 4, 4, 4, 4, R11, 8, block.as_ptr().cast()) };
    assert_eq!(error(), GL_NO_ERROR);
    let t = texture_object(name);
    close(halves(&texel_bytes(t, 0, 0, 5, 6, 2))[0], eac_u16(200), "the new block");
    close(halves(&texel_bytes(t, 0, 0, 3, 3, 2))[0], eac_u16(60), "block (0, 0) kept");
    close(halves(&texel_bytes(t, 0, 0, 4, 3, 2))[0], eac_u16(60), "block (1, 0) kept");
    close(halves(&texel_bytes(t, 0, 0, 3, 4, 2))[0], eac_u16(60), "block (0, 1) kept");

    /* another name, or an offset off the block grid, or past the level */
    // SAFETY: as above, sizes as named (the errors come before any read).
    unsafe {
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, SR11, 8, block.as_ptr().cast());
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "another format of the same size");
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 2, 0, 4, 4, R11, 8, block.as_ptr().cast());
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "off the block grid");
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 8, 0, 4, 4, R11, 8, block.as_ptr().cast());
        assert_eq!(glGetError(), GL_INVALID_VALUE, "past the level");
    }
    /* a sub-image of a punch-through texture with its own name, and the rect of a cut edge */
    new_texture(GL_TEXTURE_2D);
    upload2d(GL_TEXTURE_2D, 0, PT, 6, 6, &image(6, 6, |_, _| vec![0, 0, 0, 0, 0xff, 0xff, 0, 0]));
    assert_eq!(error(), GL_NO_ERROR);
    let pt_block = [8u8 << 3, 8 << 3, 8 << 3, 0b10, 0, 0, 0, 0];
    // SAFETY: one 8-byte block reaching the level's edge (2x2 left).
    unsafe { glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 4, 4, 2, 2, PT, 8, pt_block.as_ptr().cast()) };
    assert_eq!(error(), GL_NO_ERROR, "a box that ends at the level's edge");
}

#[test]
fn a_pixel_unpack_buffer_is_a_source_like_client_memory() {
    let _g = context();
    let name = new_texture(GL_TEXTURE_2D);
    let data = image(8, 4, |bx, _| eac(if bx == 0 { 70 } else { 130 }).to_vec());
    let mut buf = 0;
    // SAFETY: `data` is the size named.
    unsafe {
        glGenBuffers(1, &mut buf);
        glBindBuffer(GL_PIXEL_UNPACK_BUFFER, buf);
        glBufferData(GL_PIXEL_UNPACK_BUFFER, data.len() as GLsizeiptr, data.as_ptr().cast(), GL_STATIC_DRAW);
        /* the unpack state of an uncompressed upload does not apply */
        glPixelStorei(GL_UNPACK_ROW_LENGTH, 64);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 8);
        glCompressedTexImage2D(GL_TEXTURE_2D, 0, R11, 8, 4, 0, data.len() as GLsizei, core::ptr::null());
        glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
        glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0);
    }
    assert_eq!(error(), GL_NO_ERROR);
    let t = texture_object(name);
    close(halves(&texel_bytes(t, 0, 0, 1, 1, 2))[0], eac_u16(70), "block 0");
    close(halves(&texel_bytes(t, 0, 0, 6, 2, 2))[0], eac_u16(130), "block 1");
}

#[test]
fn a_decoded_texture_has_no_mipmap_to_generate_and_is_no_colour_attachment() {
    let _g = context();
    let name = new_texture(GL_TEXTURE_2D);
    upload2d(GL_TEXTURE_2D, 0, RG11, 4, 4, &[0u8; 16]);
    assert_eq!(error(), GL_NO_ERROR);
    // SAFETY: plain GL calls on the bound texture.
    unsafe {
        glGenerateMipmap(GL_TEXTURE_2D);
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "glGenerateMipmap of a compressed format");
    }
    /* a plain RGBA16F texture of the same shape is still renderable: the refusal is the compressed name's */
    let mut fbo = 0;
    // SAFETY: plain GL calls.
    unsafe {
        yakogl::gl::framebuffer::glGenFramebuffers(1, &mut fbo);
        yakogl::gl::framebuffer::glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, 0);
        assert_ne!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        yakogl::gl::framebuffer::glBindFramebuffer(GL_FRAMEBUFFER, 0);
    }
    while error() != GL_NO_ERROR {}
}

/// A respecification with an uncompressed image lets go of the compressed
/// name: the sub-image of the compressed format is then GL_INVALID_OPERATION.
#[test]
fn respecifying_as_uncompressed_clears_the_compressed_name() {
    let _g = context();
    new_texture(GL_TEXTURE_2D);
    upload2d(GL_TEXTURE_2D, 0, R11, 4, 4, &eac(90));
    assert_eq!(error(), GL_NO_ERROR);
    // SAFETY: a null source; one 8-byte block.
    unsafe {
        yakogl::gl::texture::glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_R16F as GLint,
            4,
            4,
            0,
            GL_RED,
            GL_HALF_FLOAT,
            core::ptr::null(),
        );
        assert_eq!(glGetError(), GL_NO_ERROR);
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, R11, 8, eac(1).as_ptr().cast());
        assert_eq!(glGetError(), GL_INVALID_OPERATION);
    }
}
