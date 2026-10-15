// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES2 texture.vertex.2d on the PowerVR (AM62P): what the 45 cases
//! hand the back end, replayed on the host
//! (docs/notes/es2-pvr-vertex-texture.md).
//!
//! The board record ab43a20c5:tools/cts/log/board-sessions/am62p-es2-r1c.md has all 45
//! `texture.vertex.2d.*` cases "Fail (Image comparison failed)" while the 45
//! `texture.vertex.cube.*` cases pass. These tests replay one cell of each
//! case group as es2fVertexTextureTests.cpp draws it (VK-GL-CTS
//! modules/gles2/functional/es2fVertexTextureTests.cpp):
//!
//! - the case's own shaders (`Vertex2DTextureCase::init`, :540-559, and
//!   `VertexCubeTextureCase::init`);
//! - two 128x128 GL_RGB / GL_UNSIGNED_BYTE textures with eight levels each
//!   (:585-626);
//! - one cell: a quad grid of GRID_SIZE_2D = 127 (2D) or GRID_SIZE_CUBE = 63
//!   (cube) quads a side, four vertices a quad, from client arrays, drawn
//!   with glDrawElements of u16 indices (:78-79, :246-354, :790-794).
//!
//! A stand-in back end records what the GL core hands it. The real PowerVR
//! back end cannot run here (its C command generators are linked only in
//! the C builds; its rig is `cfg(test)` of the library), so the tests pin
//! the GL core's side and compare it with the PowerVR's documented limit.
//!
//! What they show (the doc has the numbers):
//! - a 2D cell stages 1,225,804 bytes of client data (indices and two
//!   arrays), which the PowerVR copies into the frame's GENERAL arena
//!   (src/pvr/backend.rs record_draw). The default arena is 1024 KiB since
//!   the frame-set default (zephyr/Kconfig YAKOGL_PVR_ARENA_GEN_KB), so the
//!   draw does not fit an empty frame as one piece. Since a6fc6f14 the
//!   back end draws it: draw_dispatch cuts an indexed draw whose staged
//!   bytes pass half an empty arena into index pieces (draw_indexed_pieces;
//!   its tests are in src/pvr/backend_tests.rs).
//!   A cube cell stages 365,148 bytes and fits;
//! - the vertex-stage texture binding is right (size, stride, levels and
//!   their offsets, filters, wraps): not the texture state;
//! - the compiled vertex program samples at `u_lod` and `a_texCoord` on the
//!   host USC model: not the compiler.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es2_pvr_vertex_texture
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::{GlesAttrib, GlesDraw, GlesHeap, GlesTextureBind, YakoglBackend, GLES_TEXTYPE_2D};
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::ShcShader;

mod usc_exec;

/* ------------------------------------------------------- the stand-in -- */

/// One draw as the stand-in saw it.
#[derive(Clone, Copy)]
struct Seen {
    index_bytes: u32,
    index_count: u32,
    attribs: [GlesAttrib; 2],
    num_attribs: u8,
    vs_tex: GlesTextureBind,
    num_vs_tex: u8,
    vs_uniform0: u32,
    vs: *const ShcShader,
}

// SAFETY: `vs` names the linked program's shader, which the test reads on
// the thread that made it, while the context lives.
unsafe impl Send for Seen {}

static DRAWS: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn s_init() -> i32 {
    0
}
unsafe extern "C" fn s_heap_base(_h: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn s_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 {
        return 0; /* a clear */
    }
    let vs_uniform0 =
        if d.vs_uniform_cpu.is_null() || d.vs_uniform_dwords == 0 { 0 } else { *(d.vs_uniform_cpu as *const u32) };
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).push(Seen {
        index_bytes: d.index_bytes,
        index_count: d.index_count,
        attribs: [d.attribs[0], d.attribs[1]],
        num_attribs: d.num_attribs,
        vs_tex: d.vs_textures[0],
        num_vs_tex: d.num_vs_textures,
        vs_uniform0,
        vs: d.vs,
    });
    0
}
unsafe extern "C" fn s_ok() -> i32 {
    0
}
unsafe extern "C" fn s_kick_id() -> u32 {
    1
}
unsafe extern "C" fn s_kick_pending(_id: u32) -> bool {
    false
}

/// The PowerVR's flags (src/pvr/backend.rs `yakogl_backend_pvr`): float32
/// attributes only, packed mip levels, one 2D image per binding.
static STAND_IN: YakoglBackend = YakoglBackend {
    name: c"vertex-texture stand-in".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(s_init),
    shutdown: None,
    draw: Some(s_draw),
    flush: Some(s_ok),
    finish: Some(s_ok),
    finish_previous: None,
    kick_id: Some(s_kick_id),
    kick_pending: Some(s_kick_pending),
    heap_base: Some(s_heap_base),
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
};

/* ---------------------------------------------------------------- OSAL -- */

/// Pointer-valued device addresses: a test reads texels through `base_gpu`.
unsafe extern "C" fn o_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice());
    let cpu = mem.as_mut_ptr() as *mut c_void;
    *out = OsalGpuMem { cpu, gpu: cpu as u64, phys: cpu as u64, size, impl_: cpu };
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
    lock: None,
    unlock: None,
    thread_id: None,
};

/// One test at a time: the back end, the OSAL and the GL core are statics.
static SERIAL: Mutex<()> = Mutex::new(());

/// The context `start` made; a failing assertion still frees its slot.
struct Ctx(*mut yakogl::gl::context::GlContext);

impl Drop for Ctx {
    fn drop(&mut self) {
        // SAFETY: the context `start` made.
        unsafe { yakogl::gl::context::gl_context_destroy(self.0) };
    }
}

/// A fresh 256x256 context on the stand-in (dEQP's MAX_2D_RENDER_WIDTH and
/// _HEIGHT); the guard serialises the tests.
fn start() -> (std::sync::MutexGuard<'static, ()>, Ctx) {
    let g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    // SAFETY: the guard serialises the tests; both tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &STAND_IN;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        let ctx = yakogl::gl::context::gl_context_create(256, 256);
        assert!(!ctx.is_null(), "context pool exhausted");
        ctx
    };
    (g, Ctx(ctx))
}

fn draws() -> Vec<Seen> {
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/* -------------------------------------------------------- the dEQP case -- */

/// `Vertex2DTextureCase::init`'s shaders, verbatim.
const VS_2D: &str = "attribute highp vec2 a_position;\n\
                     attribute highp vec2 a_texCoord;\n\
                     uniform highp sampler2D u_texture;\n\
                     uniform highp float u_lod;\n\
                     varying mediump vec4 v_color;\n\
                     \n\
                     void main()\n\
                     {\n\
                     \x20   gl_Position = vec4(a_position, 0.0, 1.0);\n\
                     \x20   v_color = texture2DLod(u_texture, a_texCoord, u_lod);\n\
                     }\n";

/// `VertexCubeTextureCase::init`'s vertex shader, verbatim.
const VS_CUBE: &str = "attribute highp vec2 a_position;\n\
                       attribute highp vec3 a_texCoord;\n\
                       uniform highp samplerCube u_texture;\n\
                       uniform highp float u_lod;\n\
                       varying mediump vec4 v_color;\n\
                       \n\
                       void main()\n\
                       {\n\
                       \x20   gl_Position = vec4(a_position, 0.0, 1.0);\n\
                       \x20   v_color = textureCubeLod(u_texture, a_texCoord, u_lod);\n\
                       }\n";

const FS: &str = "varying mediump vec4 v_color;\n\
                  \n\
                  void main()\n\
                  {\n\
                  \x20   gl_FragColor = v_color;\n\
                  }\n";

/// es2fVertexTextureTests.cpp:78-79.
const GRID_SIZE_2D: usize = 127;
const GRID_SIZE_CUBE: usize = 63;

/// The PowerVR's default GENERAL arena, in bytes: the board images' value,
/// zephyr/Kconfig `YAKOGL_PVR_ARENA_GEN_KB` (the back end's own default,
/// src/pvr/backend.rs ARENA_GEN_BYTES, is the same 1024 KiB; its rig pins
/// it in `the_default_arenas_are_1024_and_768_kib_and_bound_a_draws_client_indices`).
fn default_general_arena_bytes() -> u64 {
    let kconfig = include_str!("../zephyr/Kconfig");
    let at = kconfig.find("config YAKOGL_PVR_ARENA_GEN_KB").expect("the Kconfig symbol");
    let kib: u64 = kconfig[at..]
        .lines()
        .find_map(|l| l.trim().strip_prefix("default "))
        .and_then(|v| v.trim().parse().ok())
        .expect("its default");
    kib * 1024
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

fn program(vs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

/// The byte every texel of level `l` holds (each channel), so a read at a
/// level's offset says which level it is.
fn level_byte(l: usize) -> u8 {
    0x10 + 0x10 * l as u8
}

/// A 128x128 GL_RGB / GL_UNSIGNED_BYTE texture of eight levels, uploaded
/// level by level as glu::Texture2D::upload does (unpack alignment 1).
fn texture_2d() -> GLuint {
    let mut t = 0;
    // SAFETY: plain GL calls; each level's data outlives its glTexImage2D.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        for l in 0..8usize {
            let s = 128 >> l;
            let data = vec![level_byte(l); s * s * 3];
            glTexImage2D(
                GL_TEXTURE_2D,
                l as GLint,
                GL_RGB as GLint,
                s as GLsizei,
                s as GLsizei,
                0,
                GL_RGB,
                GL_UNSIGNED_BYTE,
                data.as_ptr() as *const c_void,
            );
        }
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    t
}

/// A cube map of six 64x64 GL_RGB faces, seven levels.
fn texture_cube() -> GLuint {
    let mut t = 0;
    // SAFETY: plain GL calls; each level's data outlives its glTexImage2D.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_CUBE_MAP, t);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        for face in 0..6 {
            for l in 0..7usize {
                let s = 64 >> l;
                let data = vec![level_byte(l); s * s * 3];
                glTexImage2D(
                    GL_TEXTURE_CUBE_MAP_POSITIVE_X + face,
                    l as GLint,
                    GL_RGB as GLint,
                    s as GLsizei,
                    s as GLsizei,
                    0,
                    GL_RGB,
                    GL_UNSIGNED_BYTE,
                    data.as_ptr() as *const c_void,
                );
            }
        }
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    t
}

/// `PosTexCoordQuadGrid`'s arrays for a grid of `n` x `n` quads: four
/// vertices a quad (positions in -1..1), `dims` texture coordinates a
/// vertex, six u16 indices a quad. The values are the case's shape; what the
/// tests read is the counts.
fn quad_grid(n: usize, dims: usize) -> (Vec<f32>, Vec<f32>, Vec<u16>) {
    let (mut pos, mut tc, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    for y in 0..n {
        for x in 0..n {
            let first = (pos.len() / 2) as u16;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                pos.push((x + dx) as f32 / n as f32 * 2.0 - 1.0);
                pos.push((y + dy) as f32 / n as f32 * 2.0 - 1.0);
                for c in 0..dims {
                    tc.push(if c == 2 { 1.0 } else { (x + y) as f32 / (2 * n) as f32 * 1.8 - 0.3 });
                }
            }
            idx.extend_from_slice(&[first, first + 1, first + 2, first + 1, first + 3, first + 2]);
        }
    }
    (pos, tc, idx)
}

/// One cell of the case (`renderCell` / `setupShaderInputs`): the grid from
/// client arrays, `u_lod`, the texture with its wrap and filter state, one
/// glDrawElements. Returns what the stand-in saw.
fn cell(prog: GLuint, target: GLenum, tex: GLuint, grid: usize, dims: usize, lod: f32, min: GLenum) -> Seen {
    let (pos, tc, idx) = quad_grid(grid, dims);
    let before = draws().len();
    // SAFETY: plain GL calls; the arrays outlive the draw.
    unsafe {
        glUseProgram(prog);
        glViewport(0, 0, 128, 128);
        let p = glGetAttribLocation(prog, c"a_position".as_ptr());
        let t = glGetAttribLocation(prog, c"a_texCoord".as_ptr());
        assert!(p >= 0 && t >= 0, "the case's attributes");
        glEnableVertexAttribArray(p as GLuint);
        glVertexAttribPointer(p as GLuint, 2, GL_FLOAT, 0, 0, pos.as_ptr() as *const c_void);
        glEnableVertexAttribArray(t as GLuint);
        glVertexAttribPointer(t as GLuint, dims as GLint, GL_FLOAT, 0, 0, tc.as_ptr() as *const c_void);
        glUniform1f(glGetUniformLocation(prog, c"u_lod".as_ptr()), lod);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(target, tex);
        glTexParameteri(target, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_T, GL_REPEAT as GLint);
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, min as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glUniform1i(glGetUniformLocation(prog, c"u_texture".as_ptr()), 0);
        glDrawElements(GL_TRIANGLES, idx.len() as GLsizei, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void);
        assert_eq!(glGetError(), GL_NO_ERROR, "the stand-in takes every draw");
        glFinish();
    }
    let all = draws();
    assert_eq!(all.len(), before + 1, "one draw");
    all[before]
}

/// The bytes the PowerVR copies into the frame's GENERAL arena for `s`
/// (src/pvr/backend.rs record_draw): the staged indices whole, then each
/// staged (client) array whole, each at a 16-byte boundary (`arena_put`'s
/// alignment). The per-draw state (descriptors, PPP words, uniforms) comes
/// on top, so this is a lower bound.
fn staged_bytes(s: &Seen) -> u64 {
    let up16 = |n: u32| (n as u64).div_ceil(16) * 16;
    let arrays: u64 = s.attribs[..s.num_attribs as usize]
        .iter()
        .filter(|a| !a.cpu.is_null() && a.cpu_bytes != 0)
        .map(|a| up16(a.cpu_bytes))
        .sum();
    up16(s.index_bytes) + arrays
}

/* ---------------------------------------------------------------- tests -- */

/// The cause. One texture.vertex.2d cell stages 193,548 bytes of indices
/// and two client arrays of 516,128 bytes (64,516 vertices of a vec2 each):
/// 1,225,804 bytes, the figure tools/cts/RESULTS.md records for the same
/// grid on the i.MX8MP. That is more than the PowerVR's default GENERAL
/// arena holds empty (1,048,576 bytes). The draw stays this size on the GL
/// core side; it is one glDrawElements and the core does not cut it. Since
/// a6fc6f14 the PowerVR back end draws it in pieces of its index list
/// (src/pvr/backend.rs draw_dispatch, draw_indexed_pieces), so every cell
/// of every 2D case is drawn. Before that the back end refused it ("does not
/// fit an empty frame ... not drawn", -ENOMEM) and all 45 cases read the
/// clear colour. This test pins the GL core's side: the draw stays this size.
#[test]
fn a_2d_cell_stages_more_than_an_empty_general_arena_holds() {
    let (_g, _ctx) = start();
    let prog = program(VS_2D);
    let tex = texture_2d();
    let s = cell(prog, GL_TEXTURE_2D, tex, GRID_SIZE_2D, 2, 1.5, GL_LINEAR_MIPMAP_LINEAR);

    let vertices = (GRID_SIZE_2D * GRID_SIZE_2D * 4) as u32;
    assert_eq!(s.index_count, (GRID_SIZE_2D * GRID_SIZE_2D * 6) as u32);
    assert_eq!(s.index_bytes, 193_548, "u16 indices, staged whole");
    assert_eq!(s.num_attribs, 2);
    for a in &s.attribs {
        assert!(!a.cpu.is_null(), "a client array: draw-time data the back end copies");
        assert_eq!((a.components, a.stride, a.cpu_bytes), (2, 8, vertices * 8), "64,516 vec2 vertices");
    }
    let staged = staged_bytes(&s);
    assert_eq!(s.index_bytes as u64 + 2 * vertices as u64 * 8, 1_225_804);
    assert_eq!(staged, 1_225_808, "with arena_put's 16-byte alignment");
    let arena = default_general_arena_bytes();
    assert_eq!(arena, 1_048_576, "the default since the frame-set measurement (dcaafa41)");
    assert!(staged > arena, "{staged} bytes do not fit an empty {arena}-byte arena");
    assert!(staged <= 4096 * 1024, "the 4096 KiB arena of the last clean tree (d60b9d76) held it");
}

/// The control. A texture.vertex.cube cell (63 x 63 quads, a vec3
/// coordinate) stages 365,148 bytes: it fits, and the 45 cube cases pass on
/// the same image.
#[test]
fn a_cube_cell_fits_an_empty_general_arena() {
    let (_g, _ctx) = start();
    let prog = program(VS_CUBE);
    let tex = texture_cube();
    let s = cell(prog, GL_TEXTURE_CUBE_MAP, tex, GRID_SIZE_CUBE, 3, 1.5, GL_LINEAR_MIPMAP_LINEAR);

    let vertices = (GRID_SIZE_CUBE * GRID_SIZE_CUBE * 4) as u64;
    assert_eq!(s.index_bytes as u64 + vertices * 8 + vertices * 12, 365_148);
    assert!(staged_bytes(&s) < default_general_arena_bytes(), "{} bytes", staged_bytes(&s));
}

/// Not the texture state. The vertex-stage binding of a 2D cell is the
/// texture as uploaded: 128x128, stride 128 (a power of two: cc78859's
/// non-power-of-two chain layout does not apply), eight levels packed back
/// to back (`mip_level_align` 0), the filters and wraps the cell set. Each
/// level's first texel, read at the offset the PowerVR's TPU walks to
/// (level l after levels 0..l-1, 4 bytes a texel), holds that level's data.
#[test]
fn the_vertex_stage_binding_is_the_textures_own_chain() {
    let (_g, _ctx) = start();
    let prog = program(VS_2D);
    let tex = texture_2d();
    let s = cell(prog, GL_TEXTURE_2D, tex, 1, 2, 1.5, GL_LINEAR_MIPMAP_LINEAR);

    assert_eq!(s.num_vs_tex, 1, "one vertex-stage binding");
    let t = s.vs_tex;
    assert_eq!((t.unit, t.type_, t.layers, t.incomplete), (0, GLES_TEXTYPE_2D, 1, 0));
    assert_eq!((t.width, t.height, t.stride_texels), (128, 128, 128));
    assert_eq!((t.linear_filter, t.min_linear, t.mip_filter, t.mip_levels), (1, 1, 2, 8));
    assert_eq!((t.wrap_s, t.wrap_t), (2, 0), "CLAMP_TO_EDGE / REPEAT, as set");
    assert_eq!(t.no_alpha, 1, "GL_RGB");
    let mut off = 0usize;
    for l in 0..8usize {
        let side = 128 >> l;
        // SAFETY: the stand-in's device addresses are host pointers to the
        // texture's storage, which holds the whole chain.
        let texel = unsafe { core::slice::from_raw_parts((t.base_gpu as *const u8).add(off), 4) };
        assert_eq!(texel, [level_byte(l), level_byte(l), level_byte(l), 0xff], "level {l} at byte {off}");
        off += side * side * 4;
    }
}

/// Not the compiler. The vertex program the 2D cell draws with, run on the
/// host USC model (tests/usc_exec): the look-up reads the descriptor at
/// sh0, `a_texCoord` from its vertex input registers and `u_lod` (the first
/// uniform after the 20-dword descriptor) as its explicit level, and
/// v_color is the model's texel there.
#[test]
fn the_vertex_program_samples_at_u_lod_and_a_texcoord_on_the_usc_model() {
    let (_g, _ctx) = start();
    let prog = program(VS_2D);
    let tex = texture_2d();
    for lod in [1.5f32, -2.0, 0.25] {
        let s = cell(prog, GL_TEXTURE_2D, tex, 1, 2, lod, GL_LINEAR_MIPMAP_LINEAR);
        assert_eq!(s.vs_uniform0, lod.to_bits(), "the cell's u_lod in the block");
        // SAFETY: the linked program's vertex shader, live with the context.
        let vs = unsafe { &*s.vs };
        let uniforms = vs.uniform_table();
        assert!(uniforms.iter().any(|u| u.dword_off == 0 && u.gl_type == GL_FLOAT as u16), "u_lod at dword 0");
        let tc = vs.attribs[..vs.num_attribs as usize].iter().find(|a| a.location == 1).expect("a_texCoord");
        let v_color = vs.varyings[..vs.num_varyings as usize].first().expect("v_color");

        let image: Vec<u8> = vs.usc_words().iter().flat_map(|w| w.to_le_bytes()).collect();
        let marker = 0x5eed_0010;
        let (u, v) = (0.3f32.to_bits(), 0.7f32.to_bits());
        let mut inp = usc_exec::sentinel_inputs(1.0e30);
        inp.shared[0..4].copy_from_slice(&usc_exec::tex_state(marker));
        inp.shared[20] = s.vs_uniform0;
        inp.vtxin[tc.vi_reg as usize] = u;
        inp.vtxin[tc.vi_reg as usize + 1] = v;
        let _ = usc_exec::take_levels();
        let out = usc_exec::run_binary(&image, &inp, true).expect("the model runs it");
        assert_eq!(usc_exec::take_levels(), [lod.to_bits()], "the explicit level is u_lod");
        let texel = usc_exec::marker_at_level(marker, lod.to_bits());
        for k in 0..4u32 {
            assert_eq!(
                out.get(&(v_color.vtxout_off as u32 + k)).copied(),
                Some(usc_exec::sample_model(texel, u, v, k)),
                "lod {lod}: v_color.{k}"
            );
        }
    }
}
