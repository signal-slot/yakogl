// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (AM62P): the level layout of a non-power-of-two 2D chain, pinned
//! level by level for dEQP-GLES2 `texture.completeness.2d.npot_size`
//! (item am62p-es2-r1c; the record is docs/notes/pvr-npot-levels.md).
//!
//! docs/es2-cts-failures.md (section H.5) names a risk no test covers: the
//! TPU finds level l of an NPOT chain by its own arithmetic, and only this
//! CTS case (255x255 with every level, a 63x63 level 2 at a 64-texel pitch)
//! and glmark2's terrain reach it. This file replays the CTS case
//! (`Incomplete2DSizeCase::createTexture`: RGBA8, UNPACK_ALIGNMENT 1,
//! REPEAT, NEAREST_MIPMAP_NEAREST, one `glTexImage2D` for each of the eight
//! levels 255, 127, ... 1 in that order, then one quad) on a stand-in back
//! end with the PowerVR's flags, and pins what the PowerVR back end is handed.
//!
//! ## The rules the expected values come from
//!
//! They are derived here from the layout rules, not read off the code's
//! output. The TPU is given level 0's address, its stride and the level
//! count, and walks the rest itself (Mesa 26.2.1 pvr,
//! src/imagination/vulkan/pvr_image.c, as recorded in
//! docs/notes/pvr-render-pool.md and cited by
//! src/gl/texture.rs `pot_chain_level_pitch`):
//!
//! 1. `pvr_image_plane_init_physical_extent`: the extent of a multi-level
//!    image is rounded up to a power of two per axis: 255 -> 256.
//! 2. `pvr_image_plane_setup_mip_levels`: level l is `u_minify(256, l)` =
//!    256 >> l texels wide and high (physical), its row pitch is that width
//!    times the texel size, its offset is the sum of the sizes of the levels
//!    before it, and the layer is sized "as if a full mip chain until 1x1x1
//!    were present", i.e. nine physical levels 256 .. 1 although GL's chain
//!    of 255 has only eight (255, 127, 63, 31, 15, 7, 3, 1).
//! 3. A level is stored in the top-left of its physical extent, rows
//!    `pitch` apart; the rest of the physical extent is padding.
//!
//! With 4 bytes a texel, level l holds 4 * (256 >> l)^2 bytes:
//!
//! | level | GL size | physical | pitch (texels) | size (bytes) | offset |
//! |-------|---------|----------|----------------|--------------|--------|
//! | 0     | 255     | 256      | 256            | 262144       | 0      |
//! | 1     | 127     | 128      | 128            | 65536        | 262144 |
//! | 2     | 63      | 64       | 64             | 16384        | 327680 |
//! | 3     | 31      | 32       | 32             | 4096         | 344064 |
//! | 4     | 15      | 16       | 16             | 1024         | 348160 |
//! | 5     | 7       | 8        | 8              | 256          | 349184 |
//! | 6     | 3       | 4        | 4              | 64           | 349440 |
//! | 7     | 1       | 2        | 2              | 16           | 349504 |
//! | (8)   | -       | 1        | 1              | 4            | 349520 |
//!
//! The layer is 349524 bytes (4 * (4^9 - 1) / 3).
//!
//! The texture words the TPU reads (texstate.xml IMAGE_WORD0 and the STRIDE
//! form of IMAGE_WORD1, the field positions that src/pvr/backend.rs
//! `build_tex_desc` documents and that Mesa's pvr_arch_tex_state.c sets for
//! a linear image):
//!
//! - word 0: textype STRIDE (4) at [0:2], width - 1 at [34:47], height - 1 at
//!   [48:61] -- the API extent (255), not the physical one;
//! - word 1: stride - 1 at [0:14] -- level 0's physical width (256) minus
//!   one, 255; mipmaps_present at [15]; texaddr (level 0 >> 2) at [16:53];
//!   alpha_msb at [59]; num_mip_levels at [60:63] -- GL's eight.
//!
//! The words are the ones the back end really writes: `tpu_words` calls
//! `yakogl::pvr::backend::texture_descriptor`, the host tests' entry point to
//! `build_tex_desc`, with the bind the stand-in back end receives. The
//! unit tests of src/pvr/backend.rs pin the same word layout for 800x600.
//!
//! What this cannot see: the hardware. Whether the TPU really reads level 2
//! at 327680 with a 64-texel pitch is the board's to say (dEQP-GLES2
//! `texture.completeness.2d.npot_size` on the AM62P, and glmark2 terrain).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_npot_levels
//! ```
//!
//! Needs the in-process compiler (the quad's two programs are not in the
//! offline table), as tests/es2_npot.rs does.

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::{GlesDraw, GlesHeap, GlesTextureBind, YakoglBackend};
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
    name: c"pvr npot levels stand-in".as_ptr() as *const c_char,
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

/// Pointer-valued device addresses: `base_gpu` is the CPU address of level 0.
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
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}, the CTS expects GL_NO_ERROR");
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

const VS: &str = "attribute vec4 p;\nvarying vec2 v;\nvoid main() { v = p.xy * 0.5 + 0.5; gl_Position = p; }\n";
const FS: &str =
    "precision mediump float;\nuniform sampler2D s;\nvarying vec2 v;\nvoid main() { gl_FragColor = texture2D(s, v); }\n";
static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// The CTS's renderQuad: one quad sampling texture unit 0. Returns the bind.
fn render_quad() -> GlesTextureBind {
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
    no_error("renderQuad");
    BIND_SLOT.lock().unwrap_or_else(|e| e.into_inner()).take().expect("the quad was drawn with a texture").0
}

/* ---- the expected layout, from the rules in the header ---------------------------- */

const BPP: usize = 4;
const GL_LEVELS: usize = 8; /* 255, 127, 63, 31, 15, 7, 3, 1 */

/// GL's size of level `l` of the 255 chain: max(255 >> l, 1).
fn gl_side(l: usize) -> usize {
    (255usize >> l).max(1)
}
/// Rules 1 and 2: the physical side of level `l`, u_minify(256, l).
fn phys_side(l: usize) -> usize {
    (256usize >> l).max(1)
}

/// The table of the header, written out: level offsets in bytes.
const OFFSETS: [usize; GL_LEVELS] = [0, 262144, 327680, 344064, 348160, 349184, 349440, 349504];
const PITCHES: [usize; GL_LEVELS] = [256, 128, 64, 32, 16, 8, 4, 2];
const LAYER_BYTES: usize = 349524;

/// The CTS's createTexture for the 255 chain: level l is filled with the byte
/// `l + 1` so that a texel's level can be told from its place in the storage.
fn upload_chain() -> GLuint {
    let mut name: GLuint = 0;
    // SAFETY: one name; plain GL calls.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_2D, name);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
    no_error("set texturing state");
    for l in 0..GL_LEVELS {
        let s = gl_side(l);
        let texels = vec![(l + 1) as u8; s * s * BPP];
        // SAFETY: `texels` holds the s x s image.
        unsafe {
            glTexImage2D(
                GL_TEXTURE_2D,
                l as GLint,
                GL_RGBA as GLint,
                s as GLsizei,
                s as GLsizei,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                texels.as_ptr().cast(),
            );
        }
        no_error(&format!("glTexImage2D(level {l}, {s}x{s})"));
    }
    name
}

/// The texture words the TPU reads: image words 0 and 1 as the PowerVR back
/// end writes them (`build_tex_desc`, through `texture_descriptor`).
fn tpu_words(b: &GlesTextureBind) -> (u64, u64) {
    let w = yakogl::pvr::backend::texture_descriptor(b);
    (u64::from(w[0]) | u64::from(w[1]) << 32, u64::from(w[2]) | u64::from(w[3]) << 32)
}

/* ---- the tests --------------------------------------------------------------------- */

/// The derivation itself: the table of the header follows from rules 1 and 2.
#[test]
fn the_expected_table_follows_from_the_rules() {
    let mut off = 0;
    for l in 0..GL_LEVELS {
        assert_eq!(OFFSETS[l], off, "level {l} offset");
        assert_eq!(PITCHES[l], phys_side(l), "level {l} pitch");
        off += BPP * phys_side(l) * phys_side(l);
    }
    /* the layer counts the physical 1x1 level GL's chain does not have */
    assert_eq!(off + BPP, LAYER_BYTES);
}

/// Every level the CTS uploads lands at Mesa's offset and pitch, in the
/// order the CTS uploads them (level 1 grows the one-level texture into its
/// chain, which must keep level 0's texels).
#[test]
fn the_255_chain_has_every_level_at_the_tpus_offset_and_pitch() {
    on_powervr(|| {
        let name = upload_chain();
        let ctx = yakogl::gl::context::current().expect("a current context");
        let t = &ctx.objs_ref().textures[name as usize];
        assert_eq!(t.chain_levels as usize, GL_LEVELS, "GL's eight levels, not the nine of the physical chain");
        for l in 0..GL_LEVELS {
            assert_eq!(t.level_off[l] as usize, OFFSETS[l], "level {l} offset");
            assert_eq!(t.level_pitch_at(l as u32) as usize, PITCHES[l], "level {l} pitch");
        }
        assert!(t.mem.size >= LAYER_BYTES, "storage {} holds the {LAYER_BYTES}-byte layer", t.mem.size);

        /* the texels: a level's own byte inside its GL extent, the padding of
         * its physical extent untouched (the storage is cleared) */
        // SAFETY: the allocation is `mem.size` bytes long.
        let mem = unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, t.mem.size) };
        for l in 0..GL_LEVELS {
            let (gs, ps) = (gl_side(l), phys_side(l));
            for y in 0..ps {
                for x in 0..ps {
                    let at = OFFSETS[l] + (y * PITCHES[l] + x) * BPP;
                    let want = if x < gs && y < gs { (l + 1) as u8 } else { 0 };
                    assert_eq!(&mem[at..at + BPP], &[want; BPP], "level {l} texel ({x},{y})");
                }
            }
        }
    });
}

/// H.5: the 63x63 level 2 at a 64-texel pitch, alone.
#[test]
fn level_2_is_63x63_at_a_64_texel_pitch() {
    on_powervr(|| {
        let name = upload_chain();
        let ctx = yakogl::gl::context::current().expect("a current context");
        let t = &ctx.objs_ref().textures[name as usize];
        assert_eq!(gl_side(2), 63);
        assert_eq!(t.level_pitch_at(2), 64, "a 63-wide level is 64 texels a row");
        /* 4 * (256*256 + 128*128) bytes of levels 0 and 1 come first */
        assert_eq!(t.level_off[2], 327680);
        /* its last texel, (62,62), sits at row 62 of 64 texels */
        let last = 327680 + (62 * 64 + 62) * BPP;
        // SAFETY: inside the allocation (the layer test asserts its size).
        let px = unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add(last), BPP) };
        assert_eq!(px, &[3u8; BPP], "level 2 holds the byte 3 at (62,62)");
        /* and its 64th column is padding */
        // SAFETY: as above.
        let pad = unsafe { *(t.mem.cpu as *const u8).add(327680 + (62 * 64 + 63) * BPP) };
        assert_eq!(pad, 0, "column 63 of level 2 is padding");
    });
}

/// What the quad hands the PowerVR back end for the 255 chain, and the
/// texture words the TPU gets from it.
#[test]
fn the_bind_and_the_texture_words_of_the_255_chain() {
    on_powervr(|| {
        let name = upload_chain();
        let b = render_quad();
        let ctx = yakogl::gl::context::current().expect("a current context");
        let t = &ctx.objs_ref().textures[name as usize];

        assert_eq!(b.incomplete, 0, "the chain is complete");
        assert_eq!((b.width, b.height), (255, 255), "the API extent of level 0");
        assert_eq!(b.stride_texels, 256, "level 0's pitch is the physical width");
        assert_eq!(b.mip_filter, 1, "NEAREST_MIPMAP_NEAREST");
        assert_eq!(b.mip_levels as usize, GL_LEVELS, "GL's eight levels");
        assert_eq!(b.base_gpu, t.mem.gpu + OFFSETS[0] as u64, "level 0's address");
        assert_eq!(b.layers, 1);

        let (w0, w1) = tpu_words(&b);
        /* word 0: textype STRIDE, 254 and 254 */
        assert_eq!(w0 & 7, 4, "textype STRIDE");
        assert_eq!((w0 >> 34) & 0x3fff, 254, "width - 1");
        assert_eq!((w0 >> 48) & 0x3fff, 254, "height - 1");
        /* word 1: stride 255 and mipmaps_present = 0x80FF; alpha_msb and
         * eight levels: (8 << 1) | 1 = 0x11 from bit 59 up */
        assert_eq!(w1 & 0xffff, 0x80FF, "stride - 1 = 255, mipmaps_present");
        assert_eq!(w1 >> 59, 0x11, "alpha_msb at [59], num_mip_levels 8 at [60:63]");
        assert_eq!((w1 >> 16) & 0x3F_FFFF_FFFF, (b.base_gpu >> 2) & 0x3F_FFFF_FFFF, "texaddr: level 0");
    });
}

/// The same chain without a mipmap filter: the TPU is told one level and no
/// mipmaps_present, at the same stride (so a NEAREST sampler of the NPOT
/// base needs no level arithmetic at all).
#[test]
fn without_a_mipmap_filter_the_words_name_one_level_at_the_same_stride() {
    on_powervr(|| {
        upload_chain();
        // SAFETY: a plain GL call; the 2D texture just bound.
        unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint) };
        let b = render_quad();
        assert_eq!((b.mip_filter, b.stride_texels), (0, 256));
        let (_, w1) = tpu_words(&b);
        assert_eq!((w1 & 0xffff, w1 >> 59), (255, 0x03), "stride 255, no mipmaps_present, one level");
    });
}
