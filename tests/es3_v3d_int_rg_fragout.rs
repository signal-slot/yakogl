// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): fragment shader outputs narrower than the wide
//! store of an R / RG integer texture (gap G2 of docs/notes/v3d-format-matrix.md,
//! plan #383).
//!
//! An R or RG integer texture is drawn as its store's wide four-channel
//! surface with the format's channels as the write mask. A GLSL ES 3.00
//! fragment shader may declare `out int` / `out uint` (R formats),
//! `out ivec2` / `out uvec2` (RG formats) or `out ivec4` / `out uvec4`. For
//! each of the twelve formats this file:
//!
//! - compiles and links, through the GL API, a program of the format's own
//!   output type and one of `ivec4` / `uvec4`, and draws it into a TEXTURE_2D
//!   at attachment 0: the draw is not refused and not an error, it reaches
//!   the back end on the wide surface of the store, and only the format's
//!   channels are written (`color_mask`, the job state's colour mask);
//! - compiles the same programs standalone and runs them on the QPU
//!   interpreter (`sim::run`, as tests/es3_fragdepth_v3d_sim.rs): the TLB
//!   write is the 32-bit integer one (`select::tlb_config_32b`: render target
//!   0, the declared component count, TLB_TYPE_F32_COLOR, the one 32-bit type
//!   of V3D 4.2) for every width including 8 and 16 bits, carries the
//!   output's values as they are (the tile buffer narrows them), and a
//!   narrower output writes fewer words than the wide surface has channels;
//! - pins what a float `out vec4` into an integer target does today (ES 3.0
//!   leaves the values undefined; the draw must not crash).
//!
//! What it cannot see: the GPU. The back end here records each draw
//! (`state::refusal`, `state::unsupported`, `state::job_state`) and the
//! interpreter has no tile buffer, so the narrowing of the words to the
//! store is not seen. Nothing here is verified on hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_int_rg_fragout
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::v3d::state;

use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::tlb_config_32b;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, LANES};
use yakogl::shaderc::online::{lower, sema};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

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

/// What the back end was handed by one call, and what the V3D made of it.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, w, h, format).
    targets: Vec<(u64, u32, u32, u8)>,
    color_mask: u8,
    /// The job state's (colormask, rt_colormask_own, rt_colormask) of a
    /// draw the V3D takes.
    job: Option<(u32, bool, u32)>,
    /// `state::refusal` / `state::unsupported`, or the job builder's reason.
    refused: Option<&'static str>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// Records each draw; a clear of an integer target stores the low bits of
/// the four raw values in the target's layout, little endian, rows tightly
/// packed (the stand-in's own layout: the GL core reads back what it was told).
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    let refused = state::refusal(d).or_else(|| state::unsupported(d)).or_else(|| {
        if d.index_count != 0 {
            state::job_state(d).err()
        } else {
            None
        }
    });
    let job = (d.index_count != 0)
        .then(|| state::job_state(d).ok())
        .flatten()
        .map(|st| (st.colormask, st.rt_colormask_own, st.rt_colormask));
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.w, t.h, t.format)).collect(),
        color_mask: d.color_mask,
        job,
        refused,
    });
    let t = &d.targets[0];
    let Some(l) = backend::gles_surf_int_layout(t.format) else { return 0 };
    if !d.do_clear || t.gpu == 0 {
        return 0;
    }
    let bpp = backend::gles_surf_bytes_per_pixel(t.format) as usize;
    let v = d.clear_rgba.map(f32::to_bits);
    let step = l.bits as usize / 8;
    let mut px = vec![0u8; bpp];
    for ch in 0..l.channels as usize {
        px[ch * step..ch * step + step].copy_from_slice(&v[ch].to_le_bytes()[..step]);
    }
    let base = t.gpu as *mut u8;
    for y in 0..t.h as usize {
        for x in 0..t.w as usize {
            core::ptr::copy_nonoverlapping(px.as_ptr(), base.add((y * t.w as usize + x) * bpp), bpp);
        }
    }
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

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-v3d-int-rg-fragout".as_ptr() as *const c_char,
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

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glColorMask(1, 1, 1, 1);
        glDisable(GL_BLEND);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

/* ---- the twelve formats -------------------------------------------------- */

/// One of the twelve R and RG integer formats.
#[derive(Clone, Copy)]
struct Row {
    name: &'static str,
    ifmt: GLenum,
    channels: u8,
    signed: bool,
    /// The format's own surface (the renderbuffer's).
    surf: u8,
}

const fn row(name: &'static str, ifmt: GLenum, channels: u8, signed: bool, surf: u8) -> Row {
    Row { name, ifmt, channels, signed, surf }
}

const ROWS: [Row; 12] = [
    row("R8I", GL_R8I, 1, true, GLES_SURF_R8I),
    row("R8UI", GL_R8UI, 1, false, GLES_SURF_R8UI),
    row("RG8I", GL_RG8I, 2, true, GLES_SURF_RG8I),
    row("RG8UI", GL_RG8UI, 2, false, GLES_SURF_RG8UI),
    row("R16I", GL_R16I, 1, true, GLES_SURF_R16I),
    row("R16UI", GL_R16UI, 1, false, GLES_SURF_R16UI),
    row("RG16I", GL_RG16I, 2, true, GLES_SURF_RG16I),
    row("RG16UI", GL_RG16UI, 2, false, GLES_SURF_RG16UI),
    row("R32I", GL_R32I, 1, true, GLES_SURF_R32I),
    row("R32UI", GL_R32UI, 1, false, GLES_SURF_R32UI),
    row("RG32I", GL_RG32I, 2, true, GLES_SURF_RG32I),
    row("RG32UI", GL_RG32UI, 2, false, GLES_SURF_RG32UI),
];

/* ---- attachments, a program, a draw, a clear, a read-back ----------------- */

/// What carries the format at attachment 0.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Texture2d,
    /// Layer 1 of a 3-layer TEXTURE_2D_ARRAY.
    ArrayLayer,
    /// Level 1 of a 3-level, 8x8 TEXTURE_2D.
    Level1,
}

/// A bound framebuffer with the format at attachment 0, and whatever else
/// was attached; dropped, it unbinds and deletes them.
struct Fbo {
    fb: GLuint,
    tex: Vec<GLuint>,
    rbs: Vec<GLuint>,
    /// Width of the attached level (the read-back's size).
    w: GLsizei,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(self.tex.len() as GLsizei, self.tex.as_ptr());
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

fn fbo(r: &Row, kind: Kind) -> Fbo {
    let (mut fb, mut name) = (0, 0);
    let mut f = Fbo { fb: 0, tex: vec![], rbs: vec![], w: 8 };
    // SAFETY: room for one name each; the layer and level named are there.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        f.fb = fb;
        match kind {
            Kind::Texture2d | Kind::Level1 => {
                let (levels, level) = if matches!(kind, Kind::Level1) { (3, 1) } else { (1, 0) };
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D, name);
                glTexStorage2D(GL_TEXTURE_2D, levels, r.ifmt, 8, 8);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, level);
                f.tex.push(name);
                f.w = 8 >> level;
            }
            Kind::ArrayLayer => {
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D_ARRAY, name);
                glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, r.ifmt, 8, 8, 3);
                glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, name, 0, 1);
                f.tex.push(name);
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "{} {kind:?}: storage and attach", r.name);
    f
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

/// Make `fs` (with the trivial vertex shader) current.
fn program(fs: &str) {
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(GL_FRAGMENT_SHADER, fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
    }
}

/// Draw one triangle; what the back end was handed (None: no draw reached it)
/// and the GL error the call left.
fn draw() -> (Option<Seen>, GLenum) {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    SEEN.lock().unwrap().clear();
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    let e = err();
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    (s.into_iter().rev().find(|s| s.draws), e)
}

/* ---- the shaders ---------------------------------------------------------- */

/// The values a shader writes: channel n of the output is `VALS[n]` (signed
/// formats: negative ones as well).
const VALS_I: [i32; 4] = [1, -2, 3, -4];

/// The output types a shader may declare for the format: the format's own
/// component count first, then the four-component one.
fn out_types(r: &Row) -> [(&'static str, u8); 2] {
    match (r.channels, r.signed) {
        (1, true) => [("int", 1), ("ivec4", 4)],
        (1, false) => [("uint", 1), ("uvec4", 4)],
        (_, true) => [("ivec2", 2), ("ivec4", 4)],
        (_, false) => [("uvec2", 2), ("uvec4", 4)],
    }
}

/// The expression of `comps` components of the values.
fn value(ty: &str, comps: u8, signed: bool) -> String {
    let v: Vec<String> = VALS_I[..comps as usize]
        .iter()
        .map(|&n| if signed { format!("{n}") } else { format!("{}u", n.unsigned_abs()) })
        .collect();
    if comps == 1 {
        v[0].clone()
    } else {
        format!("{ty}({})", v.join(", "))
    }
}

fn fs_text(r: &Row, ty: &str, comps: u8) -> String {
    format!(
        "#version 300 es\nprecision highp int;\nlayout(location = 0) out highp {ty} o0;\n\
         void main() {{ o0 = {}; }}\n",
        value(ty, comps, r.signed)
    )
}

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

/// What the format's wide surface and mask owe a draw taken on a texture.
fn expect_wide_draw(r: &Row, what: &str) {
    let (seen, e) = draw();
    assert_eq!(e, GL_NO_ERROR, "{} {what}: the draw", r.name);
    let s = seen.unwrap_or_else(|| panic!("{} {what}: the back end saw no draw", r.name));
    assert_eq!(s.refused, None, "{} {what}: refused", r.name);
    assert_eq!(s.targets.len(), 1, "{} {what}", r.name);
    let l = backend::gles_surf_int_layout(s.targets[0].3)
        .unwrap_or_else(|| panic!("{} {what}: not an integer surface", r.name));
    let want = backend::gles_surf_int_layout(r.surf).unwrap();
    assert_eq!((l.bits, l.signed, l.channels), (want.bits, want.signed, 4), "{} {what}: the wide surface", r.name);
    assert_eq!(s.color_mask, (1u8 << r.channels) - 1, "{} {what}: the format's channels", r.name);
    let m = u32::from(s.color_mask);
    assert_eq!(s.job, Some((m, false, 0)), "{} {what}: the job's colour mask", r.name);
}

/// The TLB writes of `fs` run for one quad on the QPU interpreter: (config of
/// each write, its lane-0 value).
fn tlb_of(fs: &str) -> Vec<(Option<u32>, Option<u32>)> {
    static COMPILING: Mutex<()> = Mutex::new(());
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).expect("arena");
    let fs_ = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs_)
        .unwrap_or_else(|e| panic!("the FS lowers: {}\n{fs}", e.msg));
    let p = compile_program(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, &a)
        .unwrap_or_else(|e| panic!("the program compiles: {e:?}\n{fs}"));
    assert_eq!(p.fs.num_inputs, 0, "{fs}");
    let stream: Vec<u32> = p.fs.code.uniforms.iter().map(|u| u.data).collect();
    assert!(p.fs.code.uniforms.iter().all(|u| u.contents == 0), "constants only\n{fs}");
    let one = [1.0f32.to_bits(); LANES];
    let zero = [0u32; LANES];
    let inputs =
        FsInputs { w: one, w_centroid: one, z: zero, x: zero, y: zero, back_facing: [false; LANES], varyings: vec![] };
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let o = sim::run(p.fs.code.words, &stream, &Inputs::Fragment(Box::new(inputs)), &env)
        .unwrap_or_else(|e| panic!("the program runs: {e}\n{fs}"));
    o.tlb.iter().map(|w| (w.config, w.value[0])).collect()
}

/// Each format's own output type and `ivec4` / `uvec4`: the program links
/// through the GL API, the draw into a TEXTURE_2D at attachment 0 is taken on
/// the wide surface with the format's channels as the write mask, and the
/// same for the texture's level 1 and a 2D-array layer.
#[test]
fn narrow_and_four_component_outputs_draw_into_the_wide_store() {
    let _g = context();
    for r in ROWS {
        for kind in [Kind::Texture2d, Kind::ArrayLayer, Kind::Level1] {
            let _f = fbo(&r, kind);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} {kind:?}", r.name);
            for (ty, comps) in out_types(&r) {
                program(&fs_text(&r, ty, comps));
                expect_wide_draw(&r, &format!("{kind:?}, out {ty}"));
            }
        }
    }
}

/// The program of the output type links (glGetProgramiv LINK_STATUS, done by
/// `program`) and reports the output at location 0 through
/// glGetFragDataLocation.
#[test]
fn the_programs_link_and_name_their_output() {
    let _g = context();
    for r in ROWS {
        for (ty, comps) in out_types(&r) {
            program(&fs_text(&r, ty, comps));
            let mut cur: GLint = 0;
            // SAFETY: one int out; the name is NUL-terminated.
            let loc = unsafe {
                glGetIntegerv(GL_CURRENT_PROGRAM, &mut cur);
                glGetFragDataLocation(cur as GLuint, c"o0".as_ptr())
            };
            assert_eq!(loc, 0, "{} out {ty}", r.name);
            assert_eq!(err(), GL_NO_ERROR, "{} out {ty}", r.name);
        }
    }
}

/// The TLB write of each shader, run on the QPU interpreter: RT 0, the
/// declared component count, the 32-bit type (also for 8- and 16-bit formats:
/// the tile buffer narrows), the first word through TLBU and the others
/// through TLB, each the output's value as it is. A narrower output writes
/// fewer words than the wide surface has channels; the absent ones are never
/// written, and the colour mask has them off.
#[test]
fn the_tlb_write_is_the_32_bit_integer_write_of_the_declared_components() {
    for r in ROWS {
        for (ty, comps) in out_types(&r) {
            let w = tlb_of(&fs_text(&r, ty, comps));
            let what = format!("{} out {ty}", r.name);
            assert_eq!(w.len(), comps as usize, "{what}: one word per component: {w:x?}");
            assert_eq!(w[0].0, Some(tlb_config_32b(0, comps as u32)), "{what}: RT 0, {comps} words, 32-bit: {w:x?}");
            assert!(w[1..].iter().all(|x| x.0.is_none()), "{what}: the other words through TLB: {w:x?}");
            for (n, x) in w.iter().enumerate() {
                let want = if r.signed { VALS_I[n] as u32 } else { VALS_I[n].unsigned_abs() };
                assert_eq!(x.1, Some(want), "{what}: word {n}");
            }
        }
    }
}

/// A float output into an integer target: ES 3.0 4.2.2 leaves the values
/// undefined. The answer at the tip, pinned: the program links, the draw is
/// not an error and is taken on the same wide surface with the format's
/// channels (the GL core does not look at the output's type), and the shader
/// writes its four floats as f16 pairs (two TLB words, the F16 type), not as
/// 32-bit integers. The draw does not crash; what the tile buffer makes of
/// the halves is the hardware's.
#[test]
fn a_float_output_into_an_integer_target_is_drawn_and_does_not_crash() {
    let _g = context();
    for r in ROWS {
        let _f = fbo(&r, Kind::Texture2d);
        let fs = "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o0;\n\
                  void main() { o0 = vec4(1.0, 2.0, 3.0, 4.0); }\n";
        program(fs);
        expect_wide_draw(&r, "out vec4");
        let w = tlb_of(fs);
        assert!(!w.is_empty(), "{}: the shader writes the TLB", r.name);
        assert_ne!(w[0].0, Some(tlb_config_32b(0, 4)), "{}: a float output is not the 32-bit write: {w:x?}", r.name);
        assert_eq!(w.len(), 2, "{}: four floats as two f16-pair words: {w:x?}", r.name);
    }
}
