// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES2.functional.fbo.render.recreate_colorbuffer.rebind_{rbo_rgb565,
//! tex2d_rgb}_{stencil_index8,depth_component16_stencil_index8} through the
//! GL core on the Mali mock (rebind-pin, plan #406, issue #11329).
//!
//! The four cases are docs/es2-cts-failures.md M.1 row 14 ("Image comparison
//! failed", no refusal): they passed at 37305608 and fail since a2482523.
//! This test replays their GL sequence, es2fFboRenderTest.cpp
//! `RecreateBuffersTest<GL_COLOR_BUFFER_BIT>` with `rebind` (the calls of
//! tools/mali/testdata/pan_zs_ref/zs_seq.py `recreate`, with the colour as the
//! recreated buffer), into the real Mali back end over a mock of its OSAL:
//!
//! 1. a 128x128 framebuffer object of the colour attachment (a `GL_RGB565`
//!    renderbuffer or a `GL_RGB` texture), then (a `GL_DEPTH_COMPONENT16`
//!    and) a `GL_STENCIL_INDEX8` renderbuffer;
//! 2. clear all to (1, 0, 0, 1), depth test on, the textured quad at z 0;
//! 3. a scissored stencil clear to 1 of the middle quarter;
//! 4. the colour object deleted while bound, made anew under the same name
//!    and attached again (`rebind`);
//! 5. clear the colour only to (0, 0, 1, 0), stencil test `GL_EQUAL` 0, the
//!    second quad (z 1 to -1), glFinish.
//!
//! From the command stream it decodes every frame as the GPU reads it (each
//! RUN_IDVS's Depth/stencil and Blend, the RUN_FRAGMENT's framebuffer
//! descriptor, its ZS/CRC extension and the render target) and checks the
//! second frame against the first and against Mesa's rules (pan_job.c
//! batch per framebuffer state, pan_fb_preload.c preload of what is not
//! cleared, st_cb_clear.c:224-240 a stencil-only clear writes no colour):
//!
//! - live (`the_colour_rebind_beside_a_stencil_buffer_keeps_its_frames`): no
//!   draw is refused, `glGetError` stays 0, the framebuffer is complete and
//!   the case makes two frames. Frame 1 is cleared (no colour preload, clear
//!   words red with alpha 1) and its quad writes RGB (0c3a7729). Frame 2
//!   draws into the new colour object (a new writeback address) with frame
//!   1's internal format, writeback format and swizzle, and its last draw is
//!   the quad (stencil EQUAL 0, RGB). The scissored stencil clear's quad
//!   writes no colour (e918e015). The stencil plane under that quad holds 1
//!   in the middle quarter: frame 2 starts from the stencil clear 0 and draws
//!   the held scissored clear (what the tree does), or frame 1 draws it and
//!   writes the plane back, and frame 2 preloads it (what Mesa's batches do).
//!   Either one passes.
//! - ignored (`the_recreated_colour_is_cleared_with_alpha_1`, issue #11329):
//!   before the quad, every pixel of the recreated colour must read
//!   (0, 0, 1, 1). The format has no alpha, so it reads 1. The held
//!   scissored stencil clear opens frame 2, so the tree preloads the new,
//!   never written object (`Pre Frame 0` 2, Intersect; clear words 0) and
//!   draws the colour clear as a quad. Since 0c3a7729 that quad's colour
//!   mask is 0x7, so the GL core's RGBA8 storage keeps the alpha it had
//!   before the clear (0 on the mock). Mesa opens the new batch with the
//!   colour clear: no preload, the clear in the clear words.
//!
//! The `depth_component16_stencil_index8` variants: in the ES 3.0 build the
//! GL core reports the D16 + S8 pair `GL_FRAMEBUFFER_UNSUPPORTED` (as
//! Mesa's ES 3 context does); the test pins that status and that no frame
//! is made. In a `report-es2` build (the board's dEQP-GLES2 build) they are
//! replayed like the stencil_index8 ones.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_rebind_stencil_gl
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;
use yakogl::backend;
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glDisableVertexAttribArray, glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::cs::Instr;
use yakogl::mali::genxml::v10::*;
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OsalMaliKick, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;

/* ---- the Mali over a mock OSAL ------------------------------------------ */

/// The OSAL's answers: the board's (panthor-frame on the T6).
const GPU: OsalMaliGpu = OsalMaliGpu::T6;
const HEAP: OsalMaliHeap =
    OsalMaliHeap { ctx_gpu: 0x8000_0001_5000, first_chunk_gpu: 0x8000_00a0_0000, chunk_size: 2 << 20, reserved: 0 };

/// Buffers made so far: each one's GPU VA is its number times 16 MiB, far
/// from any CPU address. Nothing is given back (a few MiB for the run).
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static FENCES: AtomicI64 = AtomicI64::new(0);
/// Every buffer: (GPU VA, CPU address, size).
static BUFFERS: Mutex<Vec<(u64, usize, usize)>> = Mutex::new(Vec::new());
/// Every submit: the stream's GPU VA and length in bytes.
static KICKS: Mutex<Vec<(u64, usize)>> = Mutex::new(Vec::new());
/// One replay at a time: the context and the kick log are shared.
static ONE: Mutex<()> = Mutex::new(());

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let n = ALLOCS.fetch_add(1, Ordering::Relaxed) + 1;
    assert!(size <= 16 << 20);
    let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr().cast::<c_void>();
    let gpu = 0x40_0000_0000 + n * (16 << 20);
    BUFFERS.lock().unwrap().push((gpu, cpu as usize, size));
    *out = OsalGpuMem { cpu, gpu, phys: 0, size, impl_: n as *mut c_void };
    0
}
unsafe extern "C" fn m_free(mem: *mut OsalGpuMem) -> i32 {
    *mem = OsalGpuMem::ZERO;
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn m_init(_arg: *mut c_void) -> i32 {
    0
}
unsafe extern "C" fn m_submit(kick: *const c_void, len: usize) -> i64 {
    assert_eq!(len, core::mem::size_of::<OsalMaliKick>());
    let k = &*kick.cast::<OsalMaliKick>();
    KICKS.lock().unwrap().push((k.stream_gpu, k.stream_bytes as usize));
    FENCES.fetch_add(1, Ordering::Relaxed) + 1
}
unsafe extern "C" fn m_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}
unsafe extern "C" fn m_completed() -> i64 {
    FENCES.load(Ordering::Relaxed)
}
unsafe extern "C" fn m_query(what: u32, out: *mut c_void, sz: usize) -> i32 {
    match what {
        OSAL_QUERY_MALI_GPU if sz == core::mem::size_of::<OsalMaliGpu>() => *out.cast::<OsalMaliGpu>() = GPU,
        OSAL_QUERY_MALI_TILER_HEAP if sz == core::mem::size_of::<OsalMaliHeap>() => *out.cast::<OsalMaliHeap>() = HEAP,
        _ => return -ENOTSUP,
    }
    0
}

/// The OSAL's log hook: the library only calls it as `log("%s", line)`, so a
/// two-pointer function takes its place (the same calling convention for
/// these arguments).
unsafe extern "C" fn m_log(_fmt: *const c_char, line: *const c_char) {
    eprintln!("osal log: {}", core::ffi::CStr::from_ptr(line).to_string_lossy());
}

static OSAL: OsalOps = OsalOps {
    init: Some(m_init),
    shutdown: None,
    gpu_alloc: Some(m_alloc),
    gpu_free: Some(m_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: Some(m_submit),
    wait: Some(m_wait),
    completed: Some(m_completed),
    reg_read: None,
    reg_write: None,
    query: Some(m_query),
    // SAFETY: see `m_log`.
    log: Some(unsafe {
        core::mem::transmute::<
            unsafe extern "C" fn(*const c_char, *const c_char),
            unsafe extern "C" fn(*const c_char, ...),
        >(m_log)
    }),
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// `len` bytes of the buffer holding GPU address `gpu`.
fn mem(gpu: u64, len: usize) -> Vec<u8> {
    let b = BUFFERS.lock().unwrap();
    let &(base, cpu, _) = b
        .iter()
        .find(|&&(base, _, size)| gpu >= base && gpu + len as u64 <= base + size as u64)
        .unwrap_or_else(|| panic!("no buffer holds 0x{gpu:x}+{len}"));
    // SAFETY: the buffer is leaked, `size` bytes long, and the range is inside it.
    unsafe { core::slice::from_raw_parts((cpu + (gpu - base) as usize) as *const u8, len).to_vec() }
}

/* ---- what the back end built --------------------------------------------- */

#[derive(Debug)]
struct DrawOut {
    zsd: DepthStencil,
    /// The Blend's equation colour mask.
    colour_mask: u32,
}

#[derive(Debug)]
struct FrameOut {
    draws: Vec<DrawOut>,
    params: FramebufferParameters,
    /// The ZS/CRC extension, when the framebuffer has one.
    zs: Option<ZsCrcExtension>,
    rt: RgbRenderTarget,
}

/// The ZS/CRC extension's members of linear (not AFBC) depth/stencil planes.
const LINEAR_ZS: [&str; 13] = [
    "ZS.Write Format",
    "ZS.Block Format",
    "ZS.MSAA",
    "ZS.Clean Tile Write Enable",
    "ZS.Base",
    "ZS.Row Stride",
    "ZS.Surface Stride",
    "S.Write Format",
    "S.Block Format",
    "S.MSAA",
    "S.Base",
    "S.Row Stride",
    "S.Surface Stride",
];

/// The frames of the kicks since `from`, read as the GPU would: the stream's
/// register moves are followed, each RUN_IDVS reads the Depth/stencil and the
/// Blend pointers, and RUN_FRAGMENT the framebuffer descriptor.
fn frames(from: usize) -> Vec<FrameOut> {
    let kicks = KICKS.lock().unwrap()[from..].to_vec();
    let mut out = Vec::new();
    for (gpu, bytes) in kicks {
        let raw = mem(gpu, bytes);
        let mut regs = [0u32; 128];
        let reg64 = |regs: &[u32; 128], r: usize| u64::from(regs[r]) | u64::from(regs[r + 1]) << 32;
        let mut draws = Vec::new();
        for w in raw.as_chunks::<8>().0 {
            match Instr::decode(u64::from_le_bytes(*w)) {
                Ok(Instr::Move48(m)) => {
                    regs[m.destination as usize] = m.immediate as u32;
                    regs[m.destination as usize + 1] = (m.immediate >> 32) as u32;
                }
                Ok(Instr::Move32(m)) => regs[m.destination as usize] = m.immediate,
                Ok(Instr::RunIdvs(_)) => {
                    let zsd = reg64(&regs, 52);
                    let blend = reg64(&regs, 50) & !0xf;
                    let zsd = DepthStencil::unpack(&mem(zsd, DepthStencil::LENGTH).try_into().unwrap()).unwrap();
                    /* the Blend's `Equation` is bits 32..=63 */
                    let eq = BlendEquation::unpack(&mem(blend + 4, 4).try_into().unwrap()).unwrap();
                    draws.push(DrawOut { zsd, colour_mask: eq.color_mask });
                }
                Ok(Instr::RunFragment(_)) => {
                    let p = reg64(&regs, 40);
                    let fbd = p & !0x3f;
                    let params = FramebufferParameters::unpack(&mem(fbd, 64).try_into().unwrap()).unwrap();
                    /* the framebuffer, its ZS/CRC extension, the render target */
                    let ext = fbd + Framebuffer::LENGTH as u64;
                    let zs = (p & 2 != 0).then(|| {
                        ZsCrcExtension::unpack_as(&mem(ext, ZsCrcExtension::LENGTH).try_into().unwrap(), &LINEAR_ZS)
                            .unwrap()
                    });
                    let rt_at = ext + if p & 2 != 0 { 64 } else { 0 };
                    let rt = RgbRenderTarget::unpack_as(
                        &mem(rt_at, RgbRenderTarget::LENGTH).try_into().unwrap(),
                        &["Writeback Format"],
                    )
                    .unwrap();
                    out.push(FrameOut { draws: core::mem::take(&mut draws), params, zs, rt });
                }
                _ => {}
            }
        }
    }
    out
}

/* ---- the case -------------------------------------------------------------- */

/// dEQP's SingleTex2DShader, as zs_seq.py has it.
const VS: &str = "attribute vec4 a_position;\nattribute vec2 a_coord;\nvarying vec2 v_coord;\n\
                  void main() { gl_Position = a_position; v_coord = a_coord; }\n";
const FS: &str = "precision mediump float;\nvarying vec2 v_coord;\nuniform sampler2D u_sampler0;\n\
                  void main() { gl_FragColor = texture2D(u_sampler0, v_coord); }\n";

static COORD: [f32; 8] = [0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];
static INDICES: [u16; 6] = [0, 2, 1, 2, 1, 3];
const W: GLsizei = 128;

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
        assert_eq!(ok, 1, "the shader does not compile");
        s
    }
}

fn setup() -> std::sync::MutexGuard<'static, ()> {
    let guard = ONE.lock().unwrap_or_else(|e| e.into_inner());
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
        // SAFETY: static tables and one context, made once for the file's tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &yakogl::mali::backend::yakogl_backend_mali;
            assert_eq!(yakogl::mali::backend::mali_init(), 0);
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(W as u32, W as u32).is_null());
        }
    });
    guard
}

/// A 64x64 GL_RGB texture of a ramp, LINEAR and CLAMP_TO_EDGE (zs_seq.py's
/// stand-in for dEQP's metaballs and quads patterns).
///
/// # Safety
/// Plain GL calls on the current context.
unsafe fn ramp_texture() -> GLuint {
    let mut t = 0;
    let texels: Vec<u8> = (0..3 * 64 * 64).map(|i| i as u8).collect();
    glGenTextures(1, &mut t);
    glBindTexture(GL_TEXTURE_2D, t);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB as GLint, 64, 64, 0, GL_RGB, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
    t
}

/// The colour attachment under name `name` (0: a new one): a GL_RGB565
/// renderbuffer or a GL_RGB texture of 128x128, attached to the bound
/// framebuffer.
///
/// # Safety
/// Plain GL calls on the current context.
unsafe fn colour(texture: bool, name: GLuint) -> GLuint {
    let mut n = name;
    if texture {
        if n == 0 {
            glGenTextures(1, &mut n);
        }
        glBindTexture(GL_TEXTURE_2D, n);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB as GLint, W, W, 0, GL_RGB, GL_UNSIGNED_BYTE, core::ptr::null());
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, n, 0);
    } else {
        if n == 0 {
            glGenRenderbuffers(1, &mut n);
        }
        glBindRenderbuffer(GL_RENDERBUFFER, n);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB565, W, W);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, n);
    }
    n
}

/// sglr::drawQuad of the program `p` from (-1,-1,z0) to (1,1,z1).
///
/// # Safety
/// Plain GL calls on the current context; the vertex data outlives the
/// draw, which the caller's glFinish completes.
unsafe fn quad(p: GLuint, z0: f32, z1: f32, keep: &mut Vec<Box<[f32]>>) {
    let mid = 0.5 * (z0 + z1);
    let pos: Box<[f32]> = Box::new([-1.0, -1.0, z0, 1.0, -1.0, 1.0, mid, 1.0, 1.0, -1.0, mid, 1.0, 1.0, 1.0, z1, 1.0]);
    glUseProgram(p);
    glBindBuffer(GL_ARRAY_BUFFER, 0);
    glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
    glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, pos.as_ptr().cast::<c_void>());
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, COORD.as_ptr().cast::<c_void>());
    glEnableVertexAttribArray(1);
    glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, INDICES.as_ptr().cast::<c_void>());
    glDisableVertexAttribArray(0);
    glDisableVertexAttribArray(1);
    keep.push(pos);
}

/// Replay one case; the frames it made, and the status
/// `glCheckFramebufferStatus` gave.
fn replay(texture: bool, depth16: bool) -> (Vec<FrameOut>, GLenum) {
    let kicks_before = KICKS.lock().unwrap().len();
    let refused_before = backend::draws_refused();
    let mut keep = Vec::new();
    // SAFETY: plain GL calls; the vertex data is kept until after glFinish.
    let status = unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glBindAttribLocation(p, 0, c"a_position".as_ptr());
        glBindAttribLocation(p, 1, c"a_coord".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "the program does not link");
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"u_sampler0".as_ptr()), 0);
        let (metaballs, quads) = (ramp_texture(), ramp_texture());

        /* es2fFboRenderTest.cpp's Framebuffer: colour, (depth,) stencil */
        let mut fb = 0;
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        let cb = colour(texture, 0);
        let mut rb = [0; 2];
        glGenRenderbuffers(2, rb.as_mut_ptr());
        if depth16 {
            glBindRenderbuffer(GL_RENDERBUFFER, rb[0]);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, W, W);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb[0]);
        }
        glBindRenderbuffer(GL_RENDERBUFFER, rb[1]);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_STENCIL_INDEX8, W, W);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb[1]);
        let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        if status != GL_FRAMEBUFFER_COMPLETE {
            /* dEQP's checkCompleteness: NotSupported, nothing is drawn */
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            assert_eq!(glGetError(), GL_NO_ERROR, "a GL error in the case");
            return (frames(kicks_before), status);
        }

        /* the first quad */
        glViewport(0, 0, W, W);
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClearStencil(0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glEnable(GL_DEPTH_TEST);
        glBindTexture(GL_TEXTURE_2D, quads);
        quad(p, 0.0, 0.0, &mut keep);
        /* the stencil of the middle quarter to 1 */
        glEnable(GL_SCISSOR_TEST);
        glScissor(W / 4, W / 4, W / 2, W / 2);
        glClearStencil(1);
        glClear(GL_STENCIL_BUFFER_BIT);
        glDisable(GL_SCISSOR_TEST);

        /* the colour recreated while bound, and attached again */
        if texture {
            glDeleteTextures(1, &cb);
        } else {
            glDeleteRenderbuffers(1, &cb);
        }
        assert_eq!(colour(texture, cb), cb);
        let status2 = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        assert_eq!(status2, GL_FRAMEBUFFER_COMPLETE, "the framebuffer after the rebind");

        /* the recreated colour cleared, the second quad stencil-tested */
        glClearColor(0.0, 0.0, 1.0, 0.0);
        glClearStencil(0);
        glClear(GL_COLOR_BUFFER_BIT);
        glEnable(GL_STENCIL_TEST);
        glStencilFunc(GL_EQUAL, 0, 0xff);
        glBindTexture(GL_TEXTURE_2D, metaballs);
        quad(p, 1.0, -1.0, &mut keep);
        glDisable(GL_STENCIL_TEST);
        glDisable(GL_DEPTH_TEST);
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR, "a GL error in the case");
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        status
    };
    drop(keep);
    assert_eq!(backend::draws_refused(), refused_before, "a draw was refused");
    (frames(kicks_before), status)
}

/// The four cases: name, colour is a texture, a depth renderbuffer too.
const CASES: [(&str, bool, bool); 4] = [
    ("rebind_rbo_rgb565_stencil_index8", false, false),
    ("rebind_tex2d_rgb_stencil_index8", true, false),
    ("rebind_rbo_rgb565_depth_component16_stencil_index8", false, true),
    ("rebind_tex2d_rgb_depth_component16_stencil_index8", true, true),
];

/// The frames of one case, or None for a D16 + S8 case the ES 3.0 build
/// reports unsupported (pinned here).
fn case_frames(name: &str, texture: bool, depth16: bool) -> Option<Vec<FrameOut>> {
    let (frames, status) = replay(texture, depth16);
    if depth16 && !cfg!(feature = "report-es2") {
        assert_eq!(status, GL_FRAMEBUFFER_UNSUPPORTED, "{name}: glCheckFramebufferStatus");
        assert!(frames.is_empty(), "{name}: a frame was built");
        return None;
    }
    assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{name}");
    for (i, f) in frames.iter().enumerate() {
        let p = &f.params;
        eprintln!(
            "{name} frame {}: pre_frame_0 {:?} s_clear {} s w/p/u {}/{}/{} z w/p/u {}/{}/{} zs {} rt fmt {:?}/{:?} \
             swz {} we {} clear 0x{:08x} wb 0x{:x}",
            i + 1,
            p.pre_frame_0,
            p.s_clear,
            p.s_write_enable,
            p.s_preload_enable,
            p.s_unload_enable,
            p.z_write_enable,
            p.z_preload_enable,
            p.z_unload_enable,
            f.zs.as_ref().map_or("none".into(), |z| format!("s 0x{:x} zs 0x{:x}", z.s.base, z.zs.base)),
            f.rt.internal_format,
            f.rt.writeback_format,
            f.rt.swizzle,
            f.rt.write_enable,
            f.rt.clear.color_0,
            f.rt.writeback_buffer.base,
        );
        for d in &f.draws {
            let z = &d.zsd;
            eprintln!(
                "  draw: mask 0x{:x} stencil {} func {:?} ref {} pass {:?} depth {:?} zw {}",
                d.colour_mask,
                z.stencil_test_enable,
                z.front_compare_function,
                z.front_reference_value,
                z.front_depth_pass,
                z.depth_function,
                z.depth_write_enable
            );
        }
    }
    assert_eq!(frames.len(), 2, "{name}: one frame before the rebind and one after");
    Some(frames)
}

/// The scissored stencil clear's quad: ALWAYS, REPLACE with reference 1,
/// no colour.
fn is_stencil_clear_to_1(d: &DrawOut) -> bool {
    let z = &d.zsd;
    z.stencil_test_enable
        && z.front_compare_function == Func::ALWAYS
        && z.front_depth_pass == StencilOp::REPLACE
        && z.front_reference_value == 1
}

#[test]
fn the_colour_rebind_beside_a_stencil_buffer_keeps_its_frames() {
    let _one = setup();
    for (name, texture, depth16) in CASES {
        let Some(f) = case_frames(name, texture, depth16) else { continue };
        let (a, b) = (&f[0], &f[1]);
        /* frame 1: cleared to (1, 0, 0, 1), alpha 1, no colour preload; the
         * quad writes RGB (the format has no alpha, 0c3a7729) */
        assert_eq!(a.params.pre_frame_0, PrePostFrameShaderMode::NEVER, "{name}: frame 1 colour preload");
        assert_eq!(a.rt.clear.color_0, 0xff00_00ff, "{name}: frame 1 clear colour (red, alpha 1)");
        let q = &a.draws[0];
        assert_eq!(q.colour_mask, 0x7, "{name}: frame 1 quad colour mask (RGB)");
        assert!(!q.zsd.stencil_test_enable, "{name}: frame 1 quad has no stencil test");
        /* frame 2: the recreated colour object, with frame 1's format */
        assert_ne!(a.rt.writeback_buffer.base, b.rt.writeback_buffer.base, "{name}: the recreated colour object");
        assert_eq!(b.rt.internal_format, a.rt.internal_format, "{name}: internal format");
        assert_eq!(b.rt.writeback_format, a.rt.writeback_format, "{name}: writeback format");
        assert_eq!(b.rt.swizzle, a.rt.swizzle, "{name}: swizzle");
        assert!(b.rt.write_enable, "{name}: frame 2 writes its colour back");
        /* its last draw is the case's quad: stencil EQUAL 0, RGB */
        let last = b.draws.last().unwrap_or_else(|| panic!("{name}: frame 2 has no draw"));
        assert!(last.zsd.stencil_test_enable, "{name}: frame 2 quad is stencil-tested");
        assert_eq!(last.zsd.front_compare_function, Func::EQUAL, "{name}: frame 2 quad stencil EQUAL");
        assert_eq!(last.zsd.front_reference_value, 0, "{name}: frame 2 quad reference 0");
        assert_eq!(last.colour_mask, 0x7, "{name}: frame 2 quad colour mask (RGB)");
        /* a stencil clear's quad writes no colour (e918e015, st_cb_clear.c:224-240) */
        for d in a.draws.iter().chain(&b.draws).filter(|d| is_stencil_clear_to_1(d)) {
            assert_eq!(d.colour_mask, 0, "{name}: the scissored stencil clear writes colour");
        }
        /* the plane the quad tests holds 1 in the middle quarter, 0 around
         * it: either frame 2 starts from the stencil clear of 0 and draws the
         * held scissored clear (as it does at e918e015), or frame 1 drew it,
         * wrote the plane back, and frame 2 preloads it (as Mesa's batches do) */
        let held = b.draws[..b.draws.len() - 1].iter().any(is_stencil_clear_to_1)
            && !(b.params.s_preload_enable || b.params.z_preload_enable)
            && b.params.s_clear == 0;
        let kept = a.draws.iter().any(is_stencil_clear_to_1)
            && (a.params.s_unload_enable || a.params.z_unload_enable)
            && (b.params.s_preload_enable || b.params.z_preload_enable);
        assert!(held || kept, "{name}: the stencil plane under frame 2's quad");
        assert!(b.zs.is_some() && b.params.has_zs_crc_extension, "{name}: frame 2 has its stencil plane");
    }
}

#[test]
#[ignore = "issue #11329: frame 2 preloads the recreated colour (Pre Frame 0 = 2, Intersect) and draws the colour \
            clear as a quad of colour mask 0x7, so the RGBA8 storage's alpha is never written 1"]
fn the_recreated_colour_is_cleared_with_alpha_1() {
    let _one = setup();
    for (name, texture, depth16) in CASES {
        let Some(f) = case_frames(name, texture, depth16) else { continue };
        let b = &f[1];
        /* glClear(GL_COLOR_BUFFER_BIT) of (0, 0, 1, 0) on a format without
         * alpha: every pixel of the new object reads (0, 0, 1, 1) before the
         * quad. Mesa's batch for the new framebuffer state opens with that
         * clear (pan_job.c: a batch per framebuffer state; panfrost_clear at
         * draw_count 0 is a free clear): no colour preload, the clear in the
         * render target's clear words, its R5G6B5 target stores no alpha. In
         * the GL core's RGBA8 storage the same frame is the clear words
         * 0xffff0000 (blue, alpha 1). A frame that preloads the new object
         * instead must write the clear's alpha, a clear quad of mask 0xf. */
        let cleared = b.params.pre_frame_0 == PrePostFrameShaderMode::NEVER && b.rt.clear.color_0 == 0xffff_0000;
        let quads: Vec<u32> = b.draws[..b.draws.len() - 1]
            .iter()
            .filter(|d| !is_stencil_clear_to_1(d) && d.colour_mask != 0)
            .map(|d| d.colour_mask)
            .collect();
        let quad_alpha = !quads.is_empty() && quads.iter().all(|m| m & 0x8 != 0);
        assert!(
            cleared || quad_alpha,
            "{name}: frame 2 leaves the recreated colour's alpha unwritten: pre_frame_0 {:?}, clear words 0x{:08x}, \
             colour clear quads of mask {:x?}",
            b.params.pre_frame_0,
            b.rt.clear.color_0,
            quads
        );
    }
}
