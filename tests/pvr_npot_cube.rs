// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (AM62P): the levels of a cube map sampled through the TPU's own
//! cube (`gl::native_cube`, `pvr_native_cube`, on by default), pinned face
//! by face and level by level for a non-power-of-two side with a mip chain
//! (dEQP-GLES3 `texture.specification.{teximage2d,texstorage2d,
//! texsubimage2d}.*cube_57x57*`, 2 and 6 levels) beside a power-of-two
//! control (64x64, 2 and 7 levels; `cube_64x64_*` pass on the board).
//!
//! Each case uploads six faces with a distinct value for every (face,
//! level, x, y), draws one quad through a `samplerCube` with
//! NEAREST_MIPMAP_NEAREST on a stand-in back end with the PowerVR's flags,
//! takes the texture bind the draw hands the back end and the descriptor
//! the PowerVR back end builds from it
//! (`yakogl::pvr::backend::texture_descriptor`), and then reads the bytes
//! through a model of the TPU that knows only those words:
//!
//! - IMAGE_WORD0 (texstate.xml): textype at [0:2] (CUBE = 3), width - 1 at
//!   [34:47], height - 1 at [48:61], the API extent;
//! - the twiddled IMAGE_WORD1: num_mip_levels at [0:3], depth (layers - 1)
//!   at [4:14], mipmaps_present at [15], texaddr (>> 2) at [16:53];
//! - image meta dword 4: one face's bytes (the TPU's layer stride);
//! - SAMPLER_WORD0: minlod at [13:22], maxlod at [23:32] (u4.6).
//!
//! The layout the model walks is Mesa 26.2.1 pvr's for a twiddled image
//! (`pvr_image.c`, not vendored; the rules gl/native_cube.rs and
//! tests/pvr_npot_levels.rs cite): the physical side is the API side
//! rounded up to a power of two, level l is u_minify(physical, l) square in
//! Morton order (y the low bit), the levels follow each other from the
//! face's start, and face k starts k layer strides from texaddr. A
//! NEAREST_MIPMAP_NEAREST look-up at an integer LOD L reads level
//! min(L, maxlod, num_mip_levels - 1), or level 0 without mipmaps_present.
//! Every texel of every GL level of every face must read the value uploaded
//! for it ("the bytes land where the descriptor points"), and the centre
//! and corner texels of each face read at LOD L must give level L's value
//! ("a draw sampling each level returns that level's colour"; the host
//! model executes no texel fetch, so the TPU's walk is this model's).
//!
//! What it found (fix-npot-cube, plan #388). The native copy's layout is
//! Mesa's for the NPOT side too: level 0 of the 57 cube lands where the
//! descriptor points on every face
//! (`npot_cube_level_0_lands_where_the_descriptor_points`), and the 64
//! control lands right at every level. What is wrong at 57x57 is upstream
//! of the copy: the GL core keeps ONE level of storage for a NPOT cube on
//! the PowerVR (src/gl/texture.rs `npot_mip_chains` is false for six
//! stacked faces without native layers, so a level above 0 is recorded and
//! its texels dropped: the `npot_level && t.chain_levels == 1` returns of
//! the glTexImage2D and glTexSubImage2D level paths). The texture then has
//! `levels` 1, the draw samples one level, and every LOD reads level 0's
//! texels. The two NPOT tests that sample levels >= 1 are `#[ignore]`d with
//! that cause until the GL core holds the chain; they are the pin for that
//! change (run them with `-- --include-ignored`). A trial of that change
//! on the glTexImage2D path alone (a NPOT cube laid out as a packed chain,
//! as `stacked_decoded_array` lays out a decoded array, and grown there)
//! passed both for the glTexImage2D cases with gl/native_cube.rs as it is; the
//! glTexStorage2D path keeps one level and needs its own arm.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_npot_cube
//! ```
//!
//! Needs the in-process compiler (the quad's programs are not in the
//! offline table), as tests/pvr_npot_levels.rs does.

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::{GlesDraw, GlesHeap, GlesTextureBind, YakoglBackend, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE};
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- the stand-in back end: it records the texture bind of each draw ------- */

// SAFETY (Send): GlesTextureBind holds a raw `cpu` pointer; the tests read it
// only on the thread that drew, while the context lives.
struct SendBind(GlesTextureBind);
unsafe impl Send for SendBind {}
static BIND_SLOT: Mutex<Option<SendBind>> = Mutex::new(None);

unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_h: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the core passes a live job.
    let d = unsafe { &*d };
    if d.num_textures > 0 {
        *BIND_SLOT.lock().unwrap_or_else(|e| e.into_inner()) = Some(SendBind(d.textures[0]));
    }
    0
}

/// The PowerVR's flags (src/pvr/backend.rs `yakogl_backend_pvr`): packed mip
/// levels (`mip_level_align` 0), the TPU walks no layers itself.
static PVR_SHAPE: YakoglBackend = YakoglBackend {
    name: c"pvr npot cube stand-in".as_ptr() as *const c_char,
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
    max_samples: 0,
};

/// Pointer-valued device addresses: a GPU address is its CPU pointer.
unsafe extern "C" fn o_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice());
    let cpu = mem.as_mut_ptr() as *mut c_void;
    // SAFETY: `out` is the core's.
    unsafe { *out = OsalGpuMem { cpu, gpu: cpu as u64, phys: cpu as u64, size, impl_: cpu } };
    0
}
unsafe extern "C" fn o_gpu_free(_m: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn o_cache(_cpu: *mut c_void, _n: usize) {}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(o_gpu_alloc),
    gpu_free: Some(o_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(o_cache),
    cache_invd: Some(o_cache),
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

static SERIAL: Mutex<()> = Mutex::new(());

/// A fresh context on the PowerVR shape, for the length of `case`.
fn on_powervr(case: impl FnOnce()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &PVR_SHAPE;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    *BIND_SLOT.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe { yakogl::gl::context::gl_context_destroy(ctx) };
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
}

/* ---- GL plumbing ---------------------------------------------------------------- */

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

const VS: &str = "attribute vec4 p;\nvarying vec3 v;\nvoid main() { v = vec3(p.xy, 1.0); gl_Position = p; }\n";
const FS: &str = "precision mediump float;\nuniform samplerCube s;\nvarying vec3 v;\n\
                  void main() { gl_FragColor = textureCube(s, v); }\n";
static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// One quad sampling the cube on unit 0; the bind the back end was handed.
fn draw_quad() -> GlesTextureBind {
    *BIND_SLOT.lock().unwrap_or_else(|e| e.into_inner()) = None;
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    no_error("the quad");
    BIND_SLOT.lock().unwrap_or_else(|e| e.into_inner()).take().expect("the quad was drawn with a texture").0
}

/* ---- the cubes ------------------------------------------------------------------- */

/// How the case specifies its levels (the three texture.specification groups).
#[derive(Clone, Copy, Debug)]
enum Spec {
    /// glTexImage2D per face and level.
    Image,
    /// glTexStorage2D, then glTexSubImage2D of each whole level.
    Storage,
    /// glTexImage2D of every level with zeros, then glTexSubImage2D of each
    /// level in two halves.
    SubImage,
}

const SPECS: [Spec; 3] = [Spec::Image, Spec::Storage, Spec::SubImage];
const BPP: usize = 4;

/// GL's side of level `l` of a `side` cube: max(side >> l, 1).
fn gl_side(side: u32, l: u32) -> u32 {
    (side >> l).max(1)
}

/// The RGBA8 value uploaded for texel (x, y) of `face`, level `l`: no two
/// (face, level) pairs share the first byte, and inside a level x and y
/// tell the texel apart.
fn value(face: u32, l: u32, x: u32, y: u32) -> [u8; 4] {
    [(face * 16 + l + 1) as u8, x as u8, y as u8, 0xA5]
}

/// Rows `y0..y1` of level `l` of `face`, RGBA8, UNPACK_ALIGNMENT 1.
fn rows(face: u32, side: u32, l: u32, y0: u32, y1: u32) -> Vec<u8> {
    let n = gl_side(side, l);
    (y0..y1).flat_map(|y| (0..n).flat_map(move |x| value(face, l, x, y))).collect()
}

fn upload_cube(side: u32, levels: u32, spec: Spec) -> GLuint {
    let mut name: GLuint = 0;
    let target = |face: u32| GL_TEXTURE_CUBE_MAP_POSITIVE_X + face;
    /* glTexSubImage2D of rows y0..y1 of level l of face */
    let sub = |face: u32, l: u32, y0: u32, y1: u32| {
        let px = rows(face, side, l, y0, y1);
        let n = gl_side(side, l) as GLsizei;
        // SAFETY: `px` is the (y1 - y0) rows of n texels named.
        unsafe {
            glTexSubImage2D(
                target(face),
                l as GLint,
                0,
                y0 as GLint,
                n,
                (y1 - y0) as GLsizei,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr().cast(),
            )
        };
    };
    let image = |face: u32, l: u32, px: &[u8]| {
        let n = gl_side(side, l) as GLsizei;
        // SAFETY: `px` is the level's n x n texels.
        unsafe {
            glTexImage2D(
                target(face),
                l as GLint,
                GL_RGBA8 as GLint,
                n,
                n,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr().cast(),
            )
        };
    };
    // SAFETY: plain GL calls.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_CUBE_MAP, name);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        /* a chain short of 1x1 is complete up to its last level (the
         * texture.specification cases' 2-level cubes) */
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAX_LEVEL, levels as GLint - 1);
        if matches!(spec, Spec::Storage) {
            glTexStorage2D(GL_TEXTURE_CUBE_MAP, levels as GLsizei, GL_RGBA8, side as GLsizei, side as GLsizei);
        }
    }
    for face in 0..6 {
        for l in 0..levels {
            let n = gl_side(side, l);
            match spec {
                Spec::Image => image(face, l, &rows(face, side, l, 0, n)),
                Spec::Storage => sub(face, l, 0, n),
                Spec::SubImage => {
                    image(face, l, &vec![0u8; (n * n) as usize * BPP]);
                    sub(face, l, 0, n / 2);
                    sub(face, l, n / 2, n);
                }
            }
            no_error(&format!("{spec:?} {side}x{side}: face {face} level {l}"));
        }
    }
    name
}

/* ---- the TPU's view, from the descriptor alone ----------------------------------- */

/// The fields of the descriptor the model walks.
#[derive(Debug)]
struct Words {
    textype: u64,
    width: u32,
    height: u32,
    num_mip_levels: u32,
    depth: u32,
    mipmaps_present: bool,
    texaddr: u64,
    layer: u32,
    minlod: u32,
    maxlod: u32,
}

fn words(b: &GlesTextureBind) -> Words {
    let w = yakogl::pvr::backend::texture_descriptor(b);
    let w0 = u64::from(w[0]) | u64::from(w[1]) << 32;
    let w1 = u64::from(w[2]) | u64::from(w[3]) << 32;
    let s0 = u64::from(w[8]) | u64::from(w[9]) << 32;
    Words {
        textype: w0 & 7,
        width: ((w0 >> 34) & 0x3fff) as u32 + 1,
        height: ((w0 >> 48) & 0x3fff) as u32 + 1,
        num_mip_levels: (w1 & 0xf) as u32,
        depth: ((w1 >> 4) & 0x7ff) as u32,
        mipmaps_present: (w1 >> 15) & 1 != 0,
        texaddr: ((w1 >> 16) & ((1u64 << 38) - 1)) << 2,
        layer: w[4],
        minlod: ((s0 >> 13) & 0x3ff) as u32,
        maxlod: ((s0 >> 23) & 0x3ff) as u32,
    }
}

/// Morton index of (x, y) in a `p`-square twiddled level, y the low bit.
fn twiddle(x: u32, y: u32, p: u32) -> u32 {
    (0..p.max(1).trailing_zeros()).fold(0, |o, b| o | ((y >> b) & 1) << (2 * b) | ((x >> b) & 1) << (2 * b + 1))
}

impl Words {
    /// The physical side: the API side rounded up to a power of two.
    fn phys(&self) -> u32 {
        self.width.next_power_of_two()
    }

    /// The level a NEAREST_MIPMAP_NEAREST look-up at integer LOD `lod` reads.
    fn level_at(&self, lod: u32) -> u32 {
        if !self.mipmaps_present {
            return 0;
        }
        ((lod * 64).clamp(self.minlod, self.maxlod) / 64).min(self.num_mip_levels.max(1) - 1)
    }

    /// The address of texel (x, y) of level `l` of `face`.
    fn texel_addr(&self, face: u32, l: u32, x: u32, y: u32) -> u64 {
        let p = self.phys();
        let before: u32 = (0..l).map(|k| (p >> k).max(1).pow(2)).sum();
        let at = before + twiddle(x, y, (p >> l).max(1));
        self.texaddr + u64::from(face) * u64::from(self.layer) + u64::from(at) * BPP as u64
    }
}

/// The 40 bits of a device address texaddr holds (texstate.xml: [16:53],
/// shift 2). The stand-in's addresses are host pointers, wider than that.
const DEV_MASK: u64 = (1 << 40) - 1;

/// The 4 bytes at device address `a` (40 bits, as the TPU sees it) of the
/// copy bound in `b`: the stand-in OSAL's CPU pointer, its high bits put
/// back from the bind.
fn read(b: &GlesTextureBind, a: u64) -> [u8; 4] {
    let host = (b.base_gpu & !DEV_MASK) | a;
    assert!(host >= b.base_gpu && host + BPP as u64 <= b.base_gpu + u64::from(b.bytes), "0x{a:x} is outside the copy");
    // SAFETY: inside the copy the draw bound, which lives in the native
    // cube's cache while the context does.
    unsafe { *(host as *const [u8; 4]) }
}

/// The draw's bind of a `side` cube of `levels` levels and its words, with
/// the checks every case shares: the native cube's six faces at the API
/// extent, texaddr the copy.
fn bound(side: u32, levels: u32, spec: Spec) -> (GlesTextureBind, Words) {
    upload_cube(side, levels, spec);
    let b = draw_quad();
    let what = format!("{spec:?} {side}x{side} x{levels}");
    assert_eq!(b.type_, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE, "{what}: the native cube's copy is bound");
    assert_eq!(b.incomplete, 0, "{what}: complete");
    let w = words(&b);
    assert_eq!((w.textype, w.width, w.height, w.depth), (3, side, side, 5), "{what}: {w:?}");
    assert_eq!(u64::from(w.layer) * 6, u64::from(b.bytes), "{what}: six faces of the layer stride");
    assert_eq!(w.layer % 4096, 0, "{what}: faces on 4 KiB boundaries");
    assert_eq!(w.texaddr, b.base_gpu & DEV_MASK, "{what}: texaddr is the copy");
    (b, w)
}

/// Every texel of levels `0..upto` of every face read through the words:
/// the mismatches, counted, with the first few named.
fn misplaced(b: &GlesTextureBind, w: &Words, side: u32, upto: u32) -> (u32, Vec<String>) {
    let (mut bad, mut first) = (0u32, Vec::new());
    for face in 0..6 {
        for l in 0..upto {
            let n = gl_side(side, l);
            for y in 0..n {
                for x in 0..n {
                    let (got, want) = (read(b, w.texel_addr(face, l, x, y)), value(face, l, x, y));
                    if got != want {
                        bad += 1;
                        if first.len() < 4 {
                            first.push(format!("face {face} level {l} ({x}, {y}): {got:?}, want {want:?}"));
                        }
                    }
                }
            }
        }
    }
    (bad, first)
}

/// Every GL level of every face lands where the descriptor points.
fn every_level_lands(side: u32, levels: u32, spec: Spec) {
    on_powervr(|| {
        let (b, w) = bound(side, levels, spec);
        let what = format!("{spec:?} {side}x{side} x{levels}");
        assert_eq!((b.mip_filter, b.mip_levels as u32), (1, levels), "{what}: GL's chain reaches the back end");
        assert!(w.mipmaps_present && w.num_mip_levels >= levels, "{what}: the image word holds the chain: {w:?}");
        let (bad, first) = misplaced(&b, &w, side, levels);
        assert_eq!(bad, 0, "{what}: texels not where the descriptor points: {first:#?}");
    });
}

/// A look-up at each LOD 0..levels reads that level's texels on every face
/// (the centre and the four corners of the level), and a LOD past the chain
/// reads its last level.
fn each_level_samples_its_colour(side: u32, levels: u32, spec: Spec) {
    on_powervr(|| {
        let (b, w) = bound(side, levels, spec);
        let what = format!("{spec:?} {side}x{side} x{levels}");
        for lod in 0..levels + 2 {
            let want_l = lod.min(levels - 1);
            let l = w.level_at(lod);
            assert_eq!(l, want_l, "{what}: LOD {lod} reads level {want_l}: {w:?}");
            let n = gl_side(side, l);
            for face in 0..6 {
                for (x, y) in [(n / 2, n / 2), (0, 0), (n - 1, 0), (0, n - 1), (n - 1, n - 1)] {
                    let got = read(&b, w.texel_addr(face, l, x, y));
                    assert_eq!(got, value(face, want_l, x, y), "{what}: LOD {lod} face {face} ({x}, {y})");
                }
            }
        }
    });
}

/* ---- the model itself ------------------------------------------------------------ */

/// The walk on hand-written words: Morton order, a 57 face's physical
/// chain 64 .. 1 (21845 texels, 4 KiB aligned: 90112 bytes), the LOD clamp.
#[test]
fn the_model_walks_mesas_twiddled_chain() {
    assert_eq!(twiddle(1, 0, 2), 2);
    assert_eq!(twiddle(0, 1, 2), 1);
    /* x 0b011, y 0b101: pairs (x, y) per bit from the bottom: (1, 1) (1, 0) (0, 1) */
    assert_eq!(twiddle(3, 5, 8), 0b01_10_11);
    let w = Words {
        textype: 3,
        width: 57,
        height: 57,
        num_mip_levels: 7,
        depth: 5,
        mipmaps_present: true,
        texaddr: 0x1000,
        layer: (21845 * 4u32).next_multiple_of(4096),
        minlod: 0,
        maxlod: 5 * 64,
    };
    assert_eq!((w.phys(), w.layer), (64, 90112));
    assert_eq!(w.texel_addr(0, 1, 0, 0), 0x1000 + 64 * 64 * 4);
    assert_eq!(w.texel_addr(2, 5, 0, 0), 0x1000 + 2 * 90112 + (4096 + 1024 + 256 + 64 + 16) * 4);
    assert_eq!((w.level_at(3), w.level_at(9)), (3, 5), "clamped to maxlod");
    assert_eq!(Words { mipmaps_present: false, ..w }.level_at(3), 0);
}

/* ---- the power-of-two control (cube_64x64_*: passes on the board) ---------------- */

#[test]
fn pot_cube_every_level_lands_where_the_descriptor_points() {
    for spec in SPECS {
        for levels in [2, 7] {
            every_level_lands(64, levels, spec);
        }
    }
}

#[test]
fn pot_cube_each_level_samples_its_colour() {
    for spec in SPECS {
        for levels in [2, 7] {
            each_level_samples_its_colour(64, levels, spec);
        }
    }
}

/* ---- the non-power-of-two side (cube_57x57_*) ------------------------------------ */

/// Level 0 of the 57 cube, every texel of every face, where the descriptor
/// points: the copy's NPOT placement (the 57 face in the top-left of its 64
/// physical face, Morton order over 64) is the TPU's.
#[test]
fn npot_cube_level_0_lands_where_the_descriptor_points() {
    for spec in SPECS {
        for levels in [2, 6] {
            on_powervr(|| {
                let (b, w) = bound(57, levels, spec);
                let (bad, first) = misplaced(&b, &w, 57, 1);
                assert_eq!(bad, 0, "{spec:?} 57x57 x{levels} level 0: {first:#?}");
            });
        }
    }
}

/// The GL core does not hold a NPOT cube's chain on the PowerVR yet
/// (src/gl/texture.rs `npot_mip_chains`; see the header): levels >= 1 have
/// no texels to copy.
#[test]
#[ignore = "levels >= 1 of a NPOT cube have no storage on the PowerVR (src/gl/texture.rs npot_mip_chains)"]
fn npot_cube_every_level_lands_where_the_descriptor_points() {
    for spec in SPECS {
        for levels in [2, 6] {
            every_level_lands(57, levels, spec);
        }
    }
}

/// As above: today every LOD reads level 0.
#[test]
#[ignore = "levels >= 1 of a NPOT cube have no storage on the PowerVR (src/gl/texture.rs npot_mip_chains)"]
fn npot_cube_each_level_samples_its_colour() {
    for spec in SPECS {
        for levels in [2, 6] {
            each_level_samples_its_colour(57, levels, spec);
        }
    }
}
