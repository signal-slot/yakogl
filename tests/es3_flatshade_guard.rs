// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Flat shading on the V3D (Pi 4) and the GC7000UL (i.MX8MP): a regression
//! guard, not a fix (docs/notes/es3-flatshading.md, sections 3 and 4).
//!
//! ES 3.0 2.18: a `flat` varying takes its value from the provoking vertex,
//! the LAST vertex of a primitive. Both back ends already get that from the
//! hardware, with no index rewriting, and every GL mode reaches them native.
//! This file pins the words that say so, per mode (`GL_POINTS`, `GL_LINES`,
//! `GL_LINE_STRIP`, `GL_LINE_LOOP`, `GL_TRIANGLES`, `GL_TRIANGLE_STRIP`,
//! `GL_TRIANGLE_FAN`) and for a line drawn at width 5:
//!
//! - V3D: through the GL core and the REAL V3D back end, with the REAL job
//!   builder (`driver/src/v3d/v3d_job.c`, built with `cc` with the frame and
//!   texture code and loaded with `dlopen`), the draw's binning command list
//!   is decoded. `CFG_BITS.direct3d_provoking_vertex` is clear (GL's last
//!   vertex; Mesa v3dx_emit.c:336-337 sets it from `flatshade_first`, 0 for
//!   GL), `FLAT_SHADE_FLAGS` carries the flat components (the draw's smooth
//!   program has `ZERO_ALL_FLAT_SHADE_FLAGS`), `VERTEX_ARRAY_PRIMS` names the
//!   native primitive (the loop and the fan are not rewritten), and
//!   `LINE_WIDTH` is the width asked for. The job builder's input, the
//!   draw's `V3dJobDraw`, is checked alongside. A program with 15 vec4
//!   varyings (ES 3.0's MAX_VARYING_VECTORS, plan #359) is drawn too: its
//!   flat and centroid inputs past 24 and 48 are Mesa's flag words 1 and 2,
//!   one `FLAT_SHADE_FLAGS` / `CENTROID_FLAGS` packet per non-zero word with
//!   its varying offset and the actions of v3dx_emit.c emit_varying_flags.
//! - GC7000UL: through the GL core and the REAL Vivante back end on a
//!   stand-in OSAL that keeps every stream (as tests/es3_int_rt_gc.rs), the
//!   state words and the draw command: `PA_SYSTEM_MODE` with
//!   PROVOKING_VERTEX_LAST, `PA_CONFIG` with the FLAT shade model exactly when
//!   a fragment input is flat (PROVOKING_VERTEX_LAST governs the flat model
//!   only), the per-component interpolation nibbles of
//!   `GL_HALTI5_SHADER_ATTRIBUTES`, the native primitive of `DRAW_INSTANCED`
//!   and, for width 5, `PA_CONFIG.WIDE_LINE` with half the width in
//!   `PA_LINE_WIDTH` and `PA_WIDE_LINE_WIDTH0/1`.
//!
//! What no host test can see: which vertex the silicon takes. That is the
//! board's (the nine `rasterization.flatshading.*` rows, and gl_test 89 on the
//! GC7000UL). Pi 4: the nine rows passed at `0ff58329`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_flatshade_guard
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_flatshade_guard
//! ```
#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    any(feature = "gpu-vivante", feature = "gpu-v3d"),
    not(feature = "report-es2")
))]
/* the fakes and trampolines take the C functions' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

use yakogl::gl::consts::*;
use yakogl::gl::types::*;

/// The seven GL primitive modes with the vertices a draw of each takes, and
/// the number of primitives it makes.
const MODES: [(&str, GLenum, GLsizei); 7] = [
    ("points", GL_POINTS, 3),
    ("lines", GL_LINES, 4),
    ("line_strip", GL_LINE_STRIP, 4),
    ("line_loop", GL_LINE_LOOP, 4),
    ("triangles", GL_TRIANGLES, 6),
    ("triangle_strip", GL_TRIANGLE_STRIP, 5),
    ("triangle_fan", GL_TRIANGLE_FAN, 5),
];

fn is_line(mode: GLenum) -> bool {
    matches!(mode, GL_LINES | GL_LINE_STRIP | GL_LINE_LOOP)
}

/// dEQP flatshading's shape: one colour per vertex, interpolated or flat.
const VS_FLAT: &str = "#version 300 es\nin vec4 p;\nin vec4 c;\nflat out vec4 v;\n\
    void main() { gl_Position = p; gl_PointSize = 1.0; v = c; }\n";
const FS_FLAT: &str = "#version 300 es\nprecision mediump float;\nflat in vec4 v;\nout vec4 o;\n\
    void main() { o = v; }\n";
const VS_SMOOTH: &str = "#version 300 es\nin vec4 p;\nin vec4 c;\nout vec4 v;\n\
    void main() { gl_Position = p; gl_PointSize = 1.0; v = c; }\n";
const FS_SMOOTH: &str = "#version 300 es\nprecision mediump float;\nin vec4 v;\nout vec4 o;\n\
    void main() { o = v; }\n";
/// One flat and one smooth varying in a program.
const VS_MIXED: &str = "#version 300 es\nin vec4 p;\nin vec4 c;\nflat out vec4 vf;\nout vec4 vs;\n\
    void main() { gl_Position = p; gl_PointSize = 1.0; vf = c; vs = c * 0.5; }\n";
const FS_MIXED: &str = "#version 300 es\nprecision mediump float;\nflat in vec4 vf;\nin vec4 vs;\nout vec4 o;\n\
    void main() { o = vf + vs; }\n";

/// 15 vec4 varyings (ES 3.0's MAX_VARYING_VECTORS, 60 fragment inputs):
/// `v0`, `v6` and `v12` flat (inputs 0-3, 24-27 and 48-51, one in each of
/// the V3D's 24-bit flag words), `v13` centroid (inputs 52-55), the rest
/// smooth.
#[cfg(feature = "gpu-v3d")]
fn wide_sources() -> &'static (String, String) {
    static S: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
    S.get_or_init(|| {
        let qual = |i: usize| match i {
            0 | 6 | 12 => "flat ",
            13 => "centroid ",
            _ => "",
        };
        let mut vs = String::from("#version 300 es\nin vec4 p;\nin vec4 c;\n");
        let mut fs = String::from("#version 300 es\nprecision mediump float;\n");
        let (mut wv, mut rf) = (String::new(), String::from("vec4 a = vec4(0.0);\n"));
        for i in 0..15 {
            vs.push_str(&format!("{}out vec4 v{i};\n", qual(i)));
            fs.push_str(&format!("{}in vec4 v{i};\n", qual(i)));
            wv.push_str(&format!("v{i} = c + vec4({i}.0);\n"));
            rf.push_str(&format!("a += v{i};\n"));
        }
        vs.push_str(&format!("void main() {{\n{wv}gl_Position = p; gl_PointSize = 1.0;\n}}\n"));
        fs.push_str(&format!("out vec4 o;\nvoid main() {{\n{rf}o = a / 16.0;\n}}\n"));
        (vs, fs)
    })
}

/// Which programs a test draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Flat,
    Smooth,
    Mixed,
    #[cfg(feature = "gpu-v3d")]
    Wide,
}

fn sources(k: Kind) -> (&'static str, &'static str) {
    match k {
        Kind::Flat => (VS_FLAT, FS_FLAT),
        Kind::Smooth => (VS_SMOOTH, FS_SMOOTH),
        Kind::Mixed => (VS_MIXED, FS_MIXED),
        #[cfg(feature = "gpu-v3d")]
        Kind::Wide => (wide_sources().0.as_str(), wide_sources().1.as_str()),
    }
}

/// Positions and one colour per vertex (distinct per vertex, as dEQP's).
static POS: [f32; 28] = [
    -0.8, -0.8, 0.0, 1.0, 0.8, -0.8, 0.0, 1.0, 0.8, 0.8, 0.0, 1.0, -0.8, 0.8, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.5, -0.5,
    0.0, 1.0, -0.5, 0.5, 0.0, 1.0,
];
static COL: [f32; 28] = [
    1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0,
    1.0, 0.5, 0.5, 0.5, 1.0,
];

#[cfg(feature = "gpu-vivante")]
mod gc {
    use super::*;
    use core::ffi::{c_char, c_void};
    use std::collections::BTreeMap;
    use std::ffi::CString;
    use std::sync::Mutex;
    use yakogl::gl::draw::*;
    use yakogl::gl::framebuffer::*;
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::state::*;
    use yakogl::gl::vertex::*;
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
    use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
    use yakogl::vivante::regs::*;

    /* the stand-in OSAL: host memory, and every stream the back end submits */

    #[derive(Default)]
    struct Platform {
        allocs: BTreeMap<u64, Box<[u64]>>,
        bump: BTreeMap<u64, u64>,
        fence: i64,
        kicks: Vec<Vec<u32>>,
    }

    static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

    /// The online compiler keeps one store per process; every test takes this.
    static STORE: Mutex<()> = Mutex::new(());

    fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
        let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
        f(g.get_or_insert_with(Platform::default))
    }

    unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
        if size == 0 || size > 1 << 30 {
            return -12;
        }
        with_platform(|p| {
            let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
            let at = p.bump.entry(heap_base).or_insert(0);
            let gpu = heap_base + 0x1000 + *at;
            *at += (size as u64).div_ceil(4096) * 4096;
            let cpu = mem.as_mut_ptr() as *mut c_void;
            // SAFETY: the caller passes a writable descriptor.
            unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
            p.allocs.insert(gpu, mem);
            0
        })
    }

    unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
        // SAFETY: the caller passes a live descriptor; it is cleared (the OSAL contract).
        unsafe {
            let m = *mem;
            with_platform(|p| p.allocs.remove(&m.gpu));
            *mem = OsalGpuMem::ZERO;
        }
        0
    }

    unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

    unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
        if len != core::mem::size_of::<GcKick>() {
            return -22;
        }
        // SAFETY: the back end passes a live GcKick of that size.
        let k = unsafe { *(kick as *const GcKick) };
        with_platform(|p| {
            if k.magic != GC_KICK_MAGIC {
                return -22;
            }
            let Some(a) = p.allocs.get(&k.stream_gpu) else { return -22 };
            // SAFETY: the allocation holds at least `stream_bytes` bytes.
            let words: &[u32] =
                unsafe { core::slice::from_raw_parts(a.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
            p.kicks.push(words.to_vec());
            p.fence += 1;
            p.fence
        })
    }

    unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
        0
    }

    unsafe extern "C" fn p_completed() -> i64 {
        with_platform(|p| p.fence)
    }

    unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
        use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
        if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
            return 0;
        }
        -22
    }

    static OSAL: OsalOps = OsalOps {
        init: None,
        shutdown: None,
        gpu_alloc: Some(p_gpu_alloc),
        gpu_free: Some(p_gpu_free),
        mmu_map: None,
        map_phys: None,
        gpu_write: None,
        gpu_read: None,
        cache_flush: Some(p_cache),
        cache_invd: Some(p_cache),
        submit: Some(p_submit),
        wait: Some(p_wait),
        completed: Some(p_completed),
        reg_read: None,
        reg_write: None,
        query: Some(p_query),
        log: None,
        wait_vblank: None,
        present: None,
        lock: None,
        unlock: None,
        thread_id: None,
        display_busy: None,
    };

    /// What a stream says: every `(register, value)` a LOAD_STATE writes, in
    /// order, and the header word of every DRAW_INSTANCED.
    #[derive(Default)]
    struct Stream {
        writes: Vec<(u32, u32)>,
        draws: Vec<u32>,
    }

    impl Stream {
        fn of(words: &[u32]) -> Stream {
            let mut s = Stream::default();
            let mut i = 0usize;
            while i < words.len() {
                let w = words[i];
                if w & 0xf800_0000 == GC_OP_LOAD_STATE {
                    let count = ((w >> 16) & 0x3ff) as usize;
                    let base = (w & 0xffff) * 4;
                    for k in 0..count {
                        if let Some(v) = words.get(i + 1 + k) {
                            s.writes.push((base + 4 * k as u32, *v));
                        }
                    }
                    i += 1 + count;
                    continue;
                }
                if w & 0xf800_0000 == GC_OP_DRAW_INSTANCED {
                    s.draws.push(w);
                }
                i += match w & 0xf800_0000 {
                    GC_OP_DRAW_INSTANCED => 4,
                    GC_OP_STALL => 2,
                    _ => 1,
                };
            }
            s
        }

        /// The last value written to `reg`.
        fn reg(&self, reg: u32) -> u32 {
            self.writes.iter().rev().find(|(r, _)| *r == reg).map(|(_, v)| *v).unwrap_or_else(|| panic!("{reg:#x}"))
        }

        /// The primitive type of the one draw.
        fn prim(&self) -> u32 {
            assert_eq!(self.draws.len(), 1, "one draw: {:x?}", self.draws);
            (self.draws[0] >> 16) & 0xf
        }
    }

    /// Run `case` with the Vivante back end up on the stand-in OSAL and a
    /// fresh 64x64 context.
    fn on_gc7000(case: impl FnOnce()) {
        let _g = STORE.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: the guard serialises the tests; the tables are statics.
        let ctx = unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
            let c = yakogl::gl::context::gl_context_create(64, 64);
            assert!(!c.is_null(), "context pool exhausted");
            c
        };
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
        // SAFETY: the context made above.
        unsafe {
            yakogl::gl::context::gl_context_destroy(ctx);
            yakogl::vivante::backend::gc_shutdown();
        }
        with_platform(|p| {
            /* leaked, not freed, as tests/gc_npot_repeat.rs: an address a
             * static still names must stay memory */
            for (_, m) in core::mem::take(&mut p.allocs) {
                core::mem::forget(m);
            }
        });
        if let Err(e) = r {
            std::panic::resume_unwind(e);
        }
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

    fn program(k: Kind) -> GLuint {
        let (vs, fs) = sources(k);
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
            glBindAttribLocation(p, 0, c"p".as_ptr());
            glBindAttribLocation(p, 1, c"c".as_ptr());
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link failed");
            p
        }
    }

    /// Draw `mode` with `n` vertices of `k`'s program at `width` into an
    /// RGBA8 renderbuffer, finish, and read what the one stream says.
    fn draw(k: Kind, mode: GLenum, n: GLsizei, width: f32) -> Stream {
        let kicks0 = with_platform(|p| p.kicks.len());
        // SAFETY: plain GL calls; POS and COL are statics.
        let e = unsafe {
            let (mut rb, mut fb) = (0, 0);
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 16, 16);
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            glViewport(0, 0, 16, 16);
            glUseProgram(program(k));
            glLineWidth(width);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
            glEnableVertexAttribArray(1);
            glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, COL.as_ptr() as *const c_void);
            glDrawArrays(mode, 0, n);
            let e = glGetError();
            glFinish();
            e
        };
        assert_eq!(e, GL_NO_ERROR, "glDrawArrays(0x{mode:x}, {n}) at width {width}");
        let kicks: Vec<Vec<u32>> = with_platform(|p| p.kicks[kicks0..].to_vec());
        /* every kick's words, one stream: the draw is in one of them */
        let all: Vec<u32> = kicks.concat();
        Stream::of(&all)
    }

    /// The native primitive type each GL mode becomes: the core hands all
    /// seven to the back end as they are (`map_prim`), and the back end maps
    /// them one for one (`prim_type`). No loop closing, no fan split.
    fn native(mode: GLenum) -> u32 {
        match mode {
            GL_POINTS => GC_PRIM_POINTS,
            GL_LINES => GC_PRIM_LINES,
            GL_LINE_STRIP => GC_PRIM_LINE_STRIP,
            GL_LINE_LOOP => GC_PRIM_LINE_LOOP,
            GL_TRIANGLES => GC_PRIM_TRIANGLES,
            GL_TRIANGLE_STRIP => GC_PRIM_TRIANGLE_STRIP,
            GL_TRIANGLE_FAN => GC_PRIM_TRIANGLE_FAN,
            _ => unreachable!(),
        }
    }

    /// The interpolation nibbles of `GL_HALTI5_SHADER_ATTRIBUTES` for the
    /// first eight components (4 bits each, component 0 lowest).
    fn nibbles(s: &Stream) -> [u32; 8] {
        let w = s.reg(gl_halti5_shader_attributes(0));
        core::array::from_fn(|i| (w >> (4 * i)) & 0xf)
    }

    /// A flat varying, on every mode: the provoking vertex is the last, the
    /// flat shade model is selected, the four components are marked flat,
    /// and the mode reaches the draw command native.
    #[test]
    fn a_flat_varying_takes_the_last_vertex_in_every_mode() {
        for (name, mode, n) in MODES {
            on_gc7000(|| {
                let s = draw(Kind::Flat, mode, n, 1.0);
                let sys = s.reg(GC_PA_SYSTEM_MODE);
                assert_ne!(sys & GC_PA_SYSTEM_MODE_PROVOKING_VERTEX_LAST, 0, "{name}: PA_SYSTEM_MODE {sys:#x}");
                let cfg = s.reg(GC_PA_CONFIG);
                assert_eq!(
                    cfg & GC_PA_CONFIG_SHADE_MODEL_SMOOTH,
                    GC_PA_CONFIG_SHADE_MODEL_FLAT,
                    "{name}: PA_CONFIG {cfg:#x}"
                );
                assert_eq!(nibbles(&s)[..4], [GC_INTERP_FLAT; 4], "{name}: the four components are flat");
                assert_eq!(s.prim(), native(mode), "{name}: the native primitive");
            });
        }
    }

    /// The same program without `flat` keeps the smooth model: the flat
    /// model would interpolate nothing. The provoking vertex word is the
    /// same in both (Mesa sets it unconditionally).
    #[test]
    fn a_smooth_varying_keeps_the_smooth_model() {
        for (name, mode, n) in MODES {
            on_gc7000(|| {
                let s = draw(Kind::Smooth, mode, n, 1.0);
                let cfg = s.reg(GC_PA_CONFIG);
                assert_eq!(
                    cfg & GC_PA_CONFIG_SHADE_MODEL_SMOOTH,
                    GC_PA_CONFIG_SHADE_MODEL_SMOOTH,
                    "{name}: PA_CONFIG {cfg:#x}"
                );
                assert!(nibbles(&s).iter().all(|&x| x != GC_INTERP_FLAT), "{name}: no flat component");
                assert_ne!(s.reg(GC_PA_SYSTEM_MODE) & GC_PA_SYSTEM_MODE_PROVOKING_VERTEX_LAST, 0, "{name}");
                assert_eq!(s.prim(), native(mode), "{name}");
            });
        }
    }

    /// One flat and one smooth varying: the flat model for the draw (the
    /// smooth model takes the first vertex for a flat varying), and only the
    /// flat varying's four components are marked flat, so the smooth one
    /// still interpolates.
    #[test]
    fn a_flat_and_a_smooth_varying_mark_only_the_flat_components() {
        for (name, mode, n) in MODES {
            on_gc7000(|| {
                let s = draw(Kind::Mixed, mode, n, 1.0);
                let cfg = s.reg(GC_PA_CONFIG);
                assert_eq!(
                    cfg & GC_PA_CONFIG_SHADE_MODEL_SMOOTH,
                    GC_PA_CONFIG_SHADE_MODEL_FLAT,
                    "{name}: PA_CONFIG {cfg:#x}"
                );
                let flat = nibbles(&s).iter().filter(|&&x| x == GC_INTERP_FLAT).count();
                assert_eq!(flat, 4, "{name}: nibbles {:x?}", nibbles(&s));
                assert_ne!(s.reg(GC_PA_SYSTEM_MODE) & GC_PA_SYSTEM_MODE_PROVOKING_VERTEX_LAST, 0, "{name}");
            });
        }
    }

    /// A line at width 5 (the `_wide` rows): the native wide-line path, half
    /// the width in each width register, the flat model and the last vertex
    /// untouched, and the native line primitive. At width 1 the same words
    /// hold 0.5.
    #[test]
    fn a_wide_flat_line_keeps_the_flat_model_and_the_native_primitive() {
        for (name, mode, n) in MODES.into_iter().filter(|m| is_line(m.1)) {
            for (width, half) in [(1.0f32, 0.5f32), (5.0, 2.5)] {
                on_gc7000(|| {
                    let s = draw(Kind::Flat, mode, n, width);
                    let cfg = s.reg(GC_PA_CONFIG);
                    assert_ne!(cfg & GC_PA_CONFIG_WIDE_LINE, 0, "{name} at {width}: WIDE_LINE");
                    assert_eq!(
                        cfg & GC_PA_CONFIG_SHADE_MODEL_SMOOTH,
                        GC_PA_CONFIG_SHADE_MODEL_FLAT,
                        "{name} at {width}"
                    );
                    for reg in [GC_PA_LINE_WIDTH, GC_PA_WIDE_LINE_WIDTH0, GC_PA_WIDE_LINE_WIDTH1] {
                        assert_eq!(s.reg(reg), half.to_bits(), "{name} at {width}: register {reg:#x}");
                    }
                    assert_ne!(s.reg(GC_PA_SYSTEM_MODE) & GC_PA_SYSTEM_MODE_PROVOKING_VERTEX_LAST, 0, "{name}");
                    assert_eq!(s.prim(), native(mode), "{name} at {width}");
                });
            }
        }
    }
}

#[cfg(feature = "gpu-v3d")]
mod v3d {
    use super::*;
    use core::ffi::{c_char, c_int, c_void, CStr};
    use std::collections::BTreeMap;
    use std::ffi::CString;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::{Mutex, MutexGuard, Once, OnceLock};
    use yakogl::gl::context;
    use yakogl::gl::draw::glDrawArrays;
    use yakogl::gl::framebuffer::*;
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::state::*;
    use yakogl::gl::vertex::*;
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
    use yakogl::v3d::backend::yakogl_backend_v3d;
    use yakogl::v3d::ffi::*;
    use yakogl::v3d::kick::V3dKick;

    /* ---- the real job builder and layout code, built with cc ------------------ */

    extern "C" {
        fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    /// `driver/src/v3d/v3d_job.c`, `v3d_frame.c`, `v3d_tex.c` and the Mesa
    /// tiling, as a shared object of this test's own, reached through the
    /// handle only (RTLD_LOCAL, `-Bsymbolic`: its own calls stay inside it).
    fn lib() -> *mut c_void {
        static L: OnceLock<usize> = OnceLock::new();
        *L.get_or_init(|| {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_flatshade_guard");
            std::fs::create_dir_all(&dir).expect("the scratch directory");
            let so = dir.join(format!("v3d_job-{}.so", std::process::id()));
            /* the compiler's own temporary files next to the object */
            let st = Command::new("cc")
                .current_dir(&root)
                .env("TMPDIR", &dir)
                .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror", "-Wl,-Bsymbolic"])
                .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
                .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
                .arg("-Ithird_party/mesa-26.2.1/include")
                .args(["driver/src/v3d/v3d_job.c", "driver/src/v3d/v3d_frame.c"])
                .args(["driver/src/v3d/v3d_tex.c", "driver/src/v3d/v3d_tiling_mesa.c", "-lm", "-o"])
                .arg(&so)
                .status()
                .expect("cc runs");
            assert!(st.success(), "the V3D job builder builds");
            let path = CString::new(so.to_str().expect("a utf-8 path")).unwrap();
            // SAFETY: a path of a library this test just built; RTLD_NOW.
            let h = unsafe { dlopen(path.as_ptr(), 2) };
            assert!(!h.is_null(), "the job builder loads");
            let _ = std::fs::remove_file(&so);
            h as usize
        }) as *mut c_void
    }

    fn sym(name: &str) -> *mut c_void {
        let c = CString::new(name).unwrap();
        // SAFETY: a handle `lib` made and a NUL-terminated name.
        let p = unsafe { dlsym(lib(), c.as_ptr()) };
        assert!(!p.is_null(), "{name} in the job builder");
        p
    }

    /// The trampolines: the library's `extern "C"` functions
    /// (src/v3d/ffi.rs), each handed to the real C.
    macro_rules! trampoline {
        ($name:ident ( $($arg:ident : $ty:ty),* ) $(-> $ret:ty)?) => {
            #[no_mangle]
            unsafe extern "C" fn $name($($arg: $ty),*) $(-> $ret)? {
                // SAFETY: the C function's own signature, as src/v3d/ffi.rs declares it.
                let f: unsafe extern "C" fn($($ty),*) $(-> $ret)? =
                    unsafe { std::mem::transmute(sym(stringify!($name))) };
                // SAFETY: the library's call, passed on.
                unsafe { f($($arg),*) }
            }
        };
    }

    /* The signatures are src/v3d/ffi.rs's, kept as written (see
     * tests/depth_resolve_v3d.rs: rustfmt releases wrap these differently). */
    #[rustfmt::skip]
    mod trampolines {
        use super::*;

        trampoline!(v3d_job_begin(j: *mut V3dJob, t: *const V3dJobTarget, bcl: *mut V3dCl, rcl: *mut V3dCl, ind: *mut V3dCl) -> bool);
        trampoline!(v3d_job_end(j: *mut V3dJob, c: *const V3dJobClear, tile_list_start: *mut u32, tile_list_end: *mut u32) -> bool);
        trampoline!(v3d_frame_tiling(width: u32, height: u32, t: *mut V3dTiling));
        trampoline!(v3d_frame_tiling_rts(width: u32, height: u32, rts: u32, t: *mut V3dTiling));
        trampoline!(v3d_frame_tiling_ms(width: u32, height: u32, rts: u32, msaa: bool, t: *mut V3dTiling));
        trampoline!(v3d_frame_tiling_bpp(width: u32, height: u32, rts: u32, msaa: bool, max_bpp: u32, t: *mut V3dTiling));
        trampoline!(v3d_frame_alloc_sizes(t: *const V3dTiling, draws: u32, page_size: u32, tile_alloc_size: *mut u32, tile_state_size: *mut u32));
        trampoline!(v3d_tex_layout(width: u32, height: u32, levels: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
        trampoline!(v3d_tex_layout_msaa(width: u32, height: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
        trampoline!(v3d_tex_store_level(image: *mut u8, l: *const V3dTexLayout, level: u32, src: *const u8, src_stride: u32));
        trampoline!(v3d_tex_tfu_pack(l: *const V3dTexLayout, ttype: u32, src_gpu: u32, src_stride: u32, dst_gpu: u32,
                                     j: *mut V3dTexTfuJob) -> bool);
        trampoline!(v3d_tex_tfu_pack_mipmap(l: *const V3dTexLayout, ttype: u32, image_gpu: u32, j: *mut V3dTexTfuJob) -> bool);
        trampoline!(v3d_tex_layer_stride(l: *const V3dTexLayout) -> u32);
        trampoline!(v3d_tex_layers_size(l: *const V3dTexLayout, layers: u32, out: *mut u32) -> bool);
        trampoline!(v3d_tex_store_box(image: *mut u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                      src: *const u8, src_stride: u32) -> bool);
        trampoline!(v3d_tex_load_box(image: *const u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                     dst: *mut u8, dst_stride: u32) -> bool);
        trampoline!(v3d_tex_state_from_layout(s: *mut V3dTexState, l: *const V3dTexLayout));
        trampoline!(v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool);
        trampoline!(v3d_tex_pack_sampler(out: *mut u8, s: *const V3dSampler) -> bool);
        trampoline!(v3d_tex_pack_sampler_compare(out: *mut u8, s: *const V3dSampler, compare_op: u32) -> bool);
    }

    /// What the job builder is handed for each draw: the input its real
    /// `v3d_job_draw` turns into packets.
    #[derive(Clone, Copy, Debug)]
    struct SeenDraw {
        prim: u32,
        line_width: f32,
        flat_shade_flags: u32,
        count: u32,
        num_inputs: u32,
        /// The three 24-bit words of each flag set (V3dFsProg).
        flat_words: [u32; 3],
        centroid_words: [u32; 3],
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, layout: *mut c_void) -> i32 {
        // SAFETY: the backend passes a live draw; its state and fragment program live for the call.
        let rec = unsafe {
            let d = &*d;
            SeenDraw {
                prim: d.prim,
                line_width: (*d.st).line_width,
                flat_shade_flags: (*d.fs).flat_shade_flags[0],
                count: d.count,
                num_inputs: (*d.fs).num_inputs,
                flat_words: (*d.fs).flat_shade_flags,
                centroid_words: (*d.fs).centroid_flags,
            }
        };
        PLATFORM.lock().unwrap_or_else(|e| e.into_inner()).as_mut().expect("the platform").draws.push(rec);
        // SAFETY: the C function's own signature, as src/v3d/ffi.rs declares it.
        let f: unsafe extern "C" fn(*mut V3dJob, *const V3dJobDraw, *mut c_void) -> i32 =
            unsafe { std::mem::transmute(sym("v3d_job_draw")) };
        // SAFETY: the library's call, passed on.
        unsafe { f(j, d, layout) }
    }

    /* ---- the platform: memory that answers, jobs that finish at once ---------- */

    struct Alloc {
        mem: Box<[u64]>,
        size: usize,
    }

    #[derive(Default)]
    struct Platform {
        next: u64,
        allocs: BTreeMap<u64, Alloc>,
        /// The binning command list of each kick, as submitted.
        bcls: Vec<Vec<u8>>,
        draws: Vec<SeenDraw>,
        log: Vec<String>,
    }

    static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

    fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
        let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
        f(g.get_or_insert_with(Platform::default))
    }

    unsafe extern "C" fn p_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
        with_platform(|p| {
            if size == 0 || size > 256 << 20 {
                return -12;
            }
            let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
            let gpu = 0x0100_0000 + p.next;
            p.next += (size as u64).div_ceil(4096) * 4096;
            let cpu = mem.as_mut_ptr() as *mut c_void;
            // SAFETY: the caller passes a writable descriptor.
            unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
            p.allocs.insert(gpu, Alloc { mem, size });
            0
        })
    }

    unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
        // SAFETY: the caller passes its descriptor.
        let m = unsafe { *mem };
        if !m.impl_.is_null() {
            with_platform(|p| p.allocs.remove(&m.gpu));
        }
        0
    }

    unsafe extern "C" fn p_gpu_write(mem: *mut OsalGpuMem, off: usize, src: *const c_void, n: usize) -> i32 {
        // SAFETY: the caller passes its descriptor.
        let m = unsafe { *mem };
        if m.cpu.is_null() || off.checked_add(n).is_none_or(|e| e > m.size) {
            return -22;
        }
        // SAFETY: inside the allocation (checked above).
        unsafe { core::ptr::copy_nonoverlapping(src as *const u8, (m.cpu as *mut u8).add(off), n) };
        0
    }

    unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

    /// A kick: keep the binning command list it names.
    unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
        if len != size_of::<V3dKick>() {
            return -22;
        }
        // SAFETY: the back end passes a live V3dKick of that size.
        let k = unsafe { *(kick as *const V3dKick) };
        with_platform(|p| {
            let bytes = p.allocs.range(..=k.bcl_start as u64).next_back().and_then(|(base, a)| {
                let (lo, hi) = ((k.bcl_start as u64 - base) as usize, (k.bcl_end as u64 - base) as usize);
                // SAFETY: the allocation owns `size` bytes behind its u64 backing.
                let all = unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u8, a.size) };
                all.get(lo..hi).map(|s| s.to_vec())
            });
            p.bcls.push(bytes.unwrap_or_default());
            p.bcls.len() as i64
        })
    }

    unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
        0
    }

    unsafe extern "C" fn p_completed() -> i64 {
        with_platform(|p| p.bcls.len() as i64)
    }

    unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
        use yakogl::osal::{OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_VPM_BYTES};
        if what == OSAL_QUERY_V3D_VPM_BYTES && out_sz == 4 {
            // SAFETY: a u32 out-parameter.
            unsafe { *(out as *mut u32) = 65536 };
            return 0;
        }
        if what == OSAL_QUERY_V3D_CPU_OF_GPU && out_sz == size_of::<OsalV3dCpuOf>() {
            // SAFETY: the backend's in/out struct of that size.
            let q = unsafe { &mut *(out as *mut OsalV3dCpuOf) };
            return with_platform(|p| match p.allocs.range(..=q.gpu).next_back() {
                Some((base, a)) if q.gpu + q.bytes <= base + a.size as u64 => {
                    // SAFETY: inside the allocation.
                    q.cpu = unsafe { (a.mem.as_ptr() as *mut u8).add((q.gpu - base) as usize) } as *mut c_void;
                    0
                }
                _ => -22,
            });
        }
        if what == OSAL_QUERY_V3D_BIN_POOL {
            return -61;
        }
        -22
    }

    /// The library logs every line as `log("%s", line)` (`osal::log_str`).
    unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
        // SAFETY: the library passes a NUL-terminated line.
        let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
        with_platform(|p| p.log.push(s));
    }

    unsafe extern "C" fn p_init(_platform_arg: *mut c_void) -> i32 {
        0
    }

    type LogFn = unsafe extern "C" fn(*const c_char, ...);

    fn osal() -> &'static OsalOps {
        // SAFETY: see p_log2: the library only ever calls it with ("%s", line).
        let log = unsafe { core::mem::transmute::<unsafe extern "C" fn(*const c_char, *const c_char), LogFn>(p_log2) };
        Box::leak(Box::new(OsalOps {
            init: Some(p_init),
            shutdown: None,
            gpu_alloc: Some(p_gpu_alloc),
            gpu_free: Some(p_gpu_free),
            mmu_map: None,
            map_phys: None,
            gpu_write: Some(p_gpu_write),
            gpu_read: None,
            cache_flush: Some(p_cache),
            cache_invd: Some(p_cache),
            submit: Some(p_submit),
            wait: Some(p_wait),
            completed: Some(p_completed),
            reg_read: None,
            reg_write: None,
            query: Some(p_query),
            log: Some(log),
            wait_vblank: None,
            present: None,
            lock: None,
            unlock: None,
            thread_id: None,
            display_busy: None,
        }))
    }

    /* ---- the context ---------------------------------------------------------- */

    static LOCK: Mutex<()> = Mutex::new(());
    static ONCE: Once = Once::new();

    /// The one context and back end of the module; the tests take turns and
    /// each starts with nothing recorded and nothing pending.
    fn rig() -> MutexGuard<'static, ()> {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        ONCE.call_once(|| {
            // SAFETY: statics; the lock serialises every GL call of the module.
            unsafe {
                osal_set_ops(osal());
                yakogl::backend::g_backend = &yakogl_backend_v3d;
                assert_eq!(yakogl::backend::init(), 0, "{:?}", with_platform(|p| p.log.clone()));
                assert!(!context::gl_context_create(32, 32).is_null(), "{:?}", with_platform(|p| p.log.clone()));
            }
        });
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            while glGetError() != GL_NO_ERROR {}
        }
        with_platform(|p| {
            p.bcls.clear();
            p.draws.clear();
        });
        g
    }

    /* ---- the binning command list, decoded ------------------------------------ */

    /// `(opcode, length)` of every packet of the generated V3D 4.2 header
    /// (`#define V3D42_<NAME>_opcode` and `_length`), by name.
    fn packet_table() -> &'static BTreeMap<String, (u8, usize)> {
        static T: OnceLock<BTreeMap<String, (u8, usize)>> = OnceLock::new();
        T.get_or_init(|| {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/driver/src/v3d/generated/v3d_packet_v42_pack.h");
            let text = std::fs::read_to_string(path).expect("the generated V3D 4.2 packet header");
            let mut op: BTreeMap<String, u8> = BTreeMap::new();
            let mut len: BTreeMap<String, usize> = BTreeMap::new();
            for l in text.lines() {
                let mut it = l.split_whitespace();
                if it.next() != Some("#define") {
                    continue;
                }
                let (Some(name), Some(v)) = (it.next(), it.next()) else { continue };
                let Some(name) = name.strip_prefix("V3D42_") else { continue };
                if let (Some(n), Ok(v)) = (name.strip_suffix("_opcode"), v.parse::<u8>()) {
                    op.insert(n.to_string(), v);
                } else if let (Some(n), Ok(v)) = (name.strip_suffix("_length"), v.parse::<usize>()) {
                    len.insert(n.to_string(), v);
                }
            }
            op.into_iter().filter_map(|(n, o)| len.get(&n).map(|&l| (n, (o, l)))).collect()
        })
    }

    fn opcode(name: &str) -> u8 {
        packet_table().get(name).unwrap_or_else(|| panic!("{name} in the packet header")).0
    }

    /// The packets of a binning list, as (opcode, bytes). The table is the
    /// BCL's: a byte that is no packet of it stops the walk with the bytes seen.
    fn packets(bcl: &[u8]) -> Vec<(u8, &[u8])> {
        let by_op: BTreeMap<u8, usize> = packet_table()
            .iter()
            // the render list's packets share the opcode space above the BCL's
            .filter(|(n, _)| !n.starts_with("TILE_") || n.starts_with("TILE_BINNING") || n.starts_with("TILE_COORD"))
            .map(|(_, &(o, l))| (o, l))
            .collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bcl.len() {
            let Some(&l) = by_op.get(&bcl[i]) else { panic!("opcode {} at {i}: {:02x?}", bcl[i], &bcl[..i]) };
            out.push((bcl[i], &bcl[i..(i + l).min(bcl.len())]));
            i += l;
        }
        out
    }

    /// What the one draw's list says.
    #[derive(Debug)]
    struct Draw {
        /// `direct3d_provoking_vertex` of every CFG_BITS (byte 3, bit 5).
        provoking: Vec<bool>,
        /// The 24-bit flag word of every FLAT_SHADE_FLAGS, and how many
        /// ZERO_ALL_FLAT_SHADE_FLAGS.
        flat_flags: Vec<u32>,
        flat_zeroed: usize,
        /// Every FLAT_SHADE_FLAGS and CENTROID_FLAGS as (varying offset,
        /// lower action, higher action, flag word) (byte 1: bits 0-3, 4-5,
        /// 6-7; v3d_packet_v42_pack.h), and how many ZERO_ALL_* of each.
        flat_packets: Vec<(u8, u8, u8, u32)>,
        centroid_packets: Vec<(u8, u8, u8, u32)>,
        centroid_zeroed: usize,
        noperspective_zeroed: usize,
        /// The `mode` of every VERTEX_ARRAY_PRIMS.
        modes: Vec<u32>,
        /// The float of every LINE_WIDTH.
        widths: Vec<f32>,
        /// What the job builder was handed.
        seen: Vec<SeenDraw>,
    }

    fn decode(bcl: &[u8], seen: Vec<SeenDraw>) -> Draw {
        let (cfg, flat, zero, prims, width) = (
            opcode("CFG_BITS"),
            opcode("FLAT_SHADE_FLAGS"),
            opcode("ZERO_ALL_FLAT_SHADE_FLAGS"),
            opcode("VERTEX_ARRAY_PRIMS"),
            opcode("LINE_WIDTH"),
        );
        let (centroid, centroid_zero, noper_zero) =
            (opcode("CENTROID_FLAGS"), opcode("ZERO_ALL_CENTROID_FLAGS"), opcode("ZERO_ALL_NON_PERSPECTIVE_FLAGS"));
        let mut d = Draw {
            provoking: vec![],
            flat_flags: vec![],
            flat_zeroed: 0,
            flat_packets: vec![],
            centroid_packets: vec![],
            centroid_zeroed: 0,
            noperspective_zeroed: 0,
            modes: vec![],
            widths: vec![],
            seen,
        };
        let flags = |b: &[u8]| (b[1] & 0xf, (b[1] >> 4) & 3, b[1] >> 6, u32::from_le_bytes([b[2], b[3], b[4], 0]));
        for (op, b) in packets(bcl) {
            match op {
                o if o == cfg => d.provoking.push(b[3] & 0x20 != 0),
                o if o == flat => {
                    d.flat_flags.push(u32::from_le_bytes([b[2], b[3], b[4], 0]));
                    d.flat_packets.push(flags(b));
                }
                o if o == zero => d.flat_zeroed += 1,
                o if o == centroid => d.centroid_packets.push(flags(b)),
                o if o == centroid_zero => d.centroid_zeroed += 1,
                o if o == noper_zero => d.noperspective_zeroed += 1,
                o if o == prims => d.modes.push(b[1] as u32),
                o if o == width => d.widths.push(f32::from_le_bytes([b[1], b[2], b[3], b[4]])),
                _ => {}
            }
        }
        d
    }

    fn shader(ty: GLenum, src: &str) -> GLuint {
        let c = CString::new(src).unwrap();
        // SAFETY: plain GL calls; the string lives for the call.
        unsafe {
            let s = glCreateShader(ty);
            let srcs = [c.as_ptr()];
            glShaderSource(s, 1, srcs.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "compile {ty:#x}: {src}\n{:?}", with_platform(|p| p.log.clone()));
            s
        }
    }

    fn program(k: Kind) -> GLuint {
        let (vs, fs) = sources(k);
        let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
        // SAFETY: plain GL calls on handles just made.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glBindAttribLocation(p, 0, c"p".as_ptr());
            glBindAttribLocation(p, 1, c"c".as_ptr());
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link: {:?}", with_platform(|p| p.log.clone()));
            glUseProgram(p);
            glDeleteShader(v);
            glDeleteShader(f);
            p
        }
    }

    /// Draw `mode` with `n` vertices of `k`'s program at `width` into an
    /// RGBA8 renderbuffer (one job), finish, and decode the list.
    fn draw(k: Kind, mode: GLenum, n: GLsizei, width: f32) -> Draw {
        with_platform(|p| {
            p.bcls.clear();
            p.draws.clear();
        });
        let (mut rb, mut fb) = (0, 0);
        // SAFETY: plain GL calls with room for one name each; POS and COL are statics.
        let e = unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 16, 16);
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            let log = with_platform(|p| p.log.clone());
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{log:?}");
            glViewport(0, 0, 16, 16);
            let prog = program(k);
            glLineWidth(width);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
            glEnableVertexAttribArray(1);
            glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, COL.as_ptr() as *const c_void);
            glDrawArrays(mode, 0, n);
            let e = glGetError();
            glFinish();
            glLineWidth(1.0);
            glUseProgram(0);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteProgram(prog);
            glDeleteFramebuffers(1, &fb);
            glDeleteRenderbuffers(1, &rb);
            e
        };
        assert_eq!(e, GL_NO_ERROR, "glDrawArrays(0x{mode:x}, {n}) at width {width}");
        let (bcls, seen) = with_platform(|p| (p.bcls.concat(), p.draws.clone()));
        assert!(!bcls.is_empty(), "a job was submitted: {:?}", with_platform(|p| p.log.clone()));
        decode(&bcls, seen)
    }

    fn native(mode: GLenum) -> u32 {
        match mode {
            GL_POINTS => V3D_PRIM_POINTS,
            GL_LINES => V3D_PRIM_LINES,
            GL_LINE_STRIP => V3D_PRIM_LINE_STRIP,
            GL_LINE_LOOP => V3D_PRIM_LINE_LOOP,
            GL_TRIANGLES => V3D_PRIM_TRIANGLES,
            GL_TRIANGLE_STRIP => V3D_PRIM_TRIANGLE_STRIP,
            GL_TRIANGLE_FAN => V3D_PRIM_TRIANGLE_FAN,
            _ => unreachable!(),
        }
    }

    /// The generated header's primitive codes are the ABI's.
    #[test]
    fn the_primitive_codes_are_the_headers() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/driver/src/v3d/generated/v3d_packet_v42_pack.h");
        let text = std::fs::read_to_string(path).expect("the generated V3D 4.2 packet header");
        for (name, ours) in [
            ("V3D_PRIM_POINTS", V3D_PRIM_POINTS),
            ("V3D_PRIM_LINES", V3D_PRIM_LINES),
            ("V3D_PRIM_LINE_LOOP", V3D_PRIM_LINE_LOOP),
            ("V3D_PRIM_LINE_STRIP", V3D_PRIM_LINE_STRIP),
            ("V3D_PRIM_TRIANGLES", V3D_PRIM_TRIANGLES),
            ("V3D_PRIM_TRIANGLE_STRIP", V3D_PRIM_TRIANGLE_STRIP),
            ("V3D_PRIM_TRIANGLE_FAN", V3D_PRIM_TRIANGLE_FAN),
        ] {
            let line = text.lines().find(|l| l.split_whitespace().next() == Some(name)).expect(name);
            let v: u32 = line.split('=').nth(1).unwrap().trim().trim_end_matches(',').parse().unwrap();
            assert_eq!(ours, v, "{name}");
        }
    }

    /// A flat varying, on every mode: GL's provoking vertex (the last), the
    /// flat components in FLAT_SHADE_FLAGS, and the native primitive.
    #[test]
    fn a_flat_varying_takes_the_last_vertex_in_every_mode() {
        let _g = rig();
        for (name, mode, n) in MODES {
            let d = draw(Kind::Flat, mode, n, 1.0);
            assert!(!d.provoking.is_empty(), "{name}: a CFG_BITS in the list");
            assert!(
                d.provoking.iter().all(|&p| !p),
                "{name}: direct3d_provoking_vertex must be clear: {:?}",
                d.provoking
            );
            assert_eq!(d.flat_zeroed, 0, "{name}: flat flags are not zeroed");
            assert_eq!(d.flat_flags, vec![0xf], "{name}: the four components of the vec4 are flat");
            assert_eq!(d.modes, vec![native(mode)], "{name}: the native primitive, not rewritten");
            assert_eq!(d.seen.len(), 1, "{name}: one draw");
            assert_eq!(d.seen[0].prim, native(mode), "{name}: the draw's primitive");
            assert_eq!(d.seen[0].flat_shade_flags, 0xf, "{name}: the draw's flat flags");
            assert_eq!(d.seen[0].count, n as u32, "{name}: the vertices, none added for a loop or a fan");
        }
    }

    /// The same program without `flat`: no flat flags in the list (the zeroing
    /// packet), the provoking vertex bit still clear.
    #[test]
    fn a_smooth_varying_has_no_flat_flags() {
        let _g = rig();
        for (name, mode, n) in MODES {
            let d = draw(Kind::Smooth, mode, n, 1.0);
            assert!(d.provoking.iter().all(|&p| !p), "{name}: {:?}", d.provoking);
            assert!(d.flat_flags.is_empty(), "{name}: flat flags {:x?}", d.flat_flags);
            assert_eq!(d.flat_zeroed, 1, "{name}: ZERO_ALL_FLAT_SHADE_FLAGS");
            assert_eq!(d.seen[0].flat_shade_flags, 0, "{name}");
            assert_eq!(d.modes, vec![native(mode)], "{name}");
        }
    }

    /// One flat and one smooth varying: only the flat varying's four
    /// components are flagged, so the smooth one still interpolates.
    #[test]
    fn a_flat_and_a_smooth_varying_flag_only_the_flat_components() {
        let _g = rig();
        for (name, mode, n) in MODES {
            let d = draw(Kind::Mixed, mode, n, 1.0);
            assert_eq!(d.flat_flags.len(), 1, "{name}: {:x?}", d.flat_flags);
            assert_eq!(d.flat_flags[0].count_ones(), 4, "{name}: flags {:#x}", d.flat_flags[0]);
            assert!(d.provoking.iter().all(|&p| !p), "{name}");
            assert_eq!(d.modes, vec![native(mode)], "{name}");
        }
    }

    /// A line at width 5 (the `_wide` rows): LINE_WIDTH is the width asked
    /// for (the V3D draws wide lines natively), and the flat flags, the
    /// provoking vertex and the native line primitive are the same as at
    /// width 1.
    #[test]
    fn a_wide_flat_line_keeps_the_flat_flags_and_the_native_primitive() {
        let _g = rig();
        for (name, mode, n) in MODES.into_iter().filter(|m| is_line(m.1)) {
            for width in [1.0f32, 5.0] {
                let d = draw(Kind::Flat, mode, n, width);
                assert_eq!(d.widths, vec![width], "{name} at {width}: LINE_WIDTH");
                assert_eq!(d.seen[0].line_width, width, "{name} at {width}: the draw's width");
                assert!(d.provoking.iter().all(|&p| !p), "{name} at {width}");
                assert_eq!(d.flat_flags, vec![0xf], "{name} at {width}");
                assert_eq!(d.modes, vec![native(mode)], "{name} at {width}");
            }
        }
    }

    /// 15 vec4 varyings, the ES 3.0 MAX_VARYING_VECTORS the V3D reports: the
    /// job builder takes the 60 inputs (it refused more than 24 before plan
    /// #359) and writes Mesa's packets for flag words 1 and 2
    /// (v3dx_emit.c:177-208): the first non-zero word of a kind zeroes the
    /// higher words, and the lower ones too unless it is word 0; a later word
    /// leaves both; the offset is the word's index.
    #[test]
    fn fifteen_varyings_put_flags_past_24_and_48_in_words_1_and_2() {
        const UNCHANGED: u8 = 0;
        const ZEROED: u8 = 1;
        let _g = rig();
        let d = draw(Kind::Wide, GL_TRIANGLES, 6, 1.0);
        assert_eq!(d.seen.len(), 1, "one draw");
        assert_eq!(d.seen[0].num_inputs, 60, "15 vec4 varyings");
        assert_eq!(d.seen[0].flat_words, [0xf, 0xf, 0xf], "v0, v6 and v12: inputs 0-3, 24-27, 48-51");
        assert_eq!(d.seen[0].centroid_words, [0, 0, 0xf0], "v13: inputs 52-55");
        assert_eq!(
            d.flat_packets,
            vec![(0, UNCHANGED, ZEROED, 0xf), (1, UNCHANGED, UNCHANGED, 0xf), (2, UNCHANGED, UNCHANGED, 0xf)],
            "FLAT_SHADE_FLAGS, one per word"
        );
        assert_eq!(d.flat_zeroed, 0);
        assert_eq!(d.centroid_packets, vec![(2, ZEROED, ZEROED, 0xf0)], "CENTROID_FLAGS, word 2 alone");
        assert_eq!(d.centroid_zeroed, 0);
        assert_eq!(d.noperspective_zeroed, 1, "ZERO_ALL_NON_PERSPECTIVE_FLAGS");
        assert_eq!(d.modes, vec![native(GL_TRIANGLES)]);
    }
}
