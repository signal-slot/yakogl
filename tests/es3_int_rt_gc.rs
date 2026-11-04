// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): ES 3.0 integer colour render targets (item int-rt-gc,
//! docs/es3/int-rt-gc.md).
//!
//! The GL core accepts the nineteen colour-renderable integer formats and
//! refuses a draw into one a back end has not learned (tests/es3_int_rt.rs,
//! `backend::INTEGER_RT_FORMATS`). This file holds the GC7000's half:
//!
//! - the pixel engine format and the fragment stage's output mode of every
//!   integer surface, against the rnndb header the back end is written from
//!   (driver/src/vivante/hw/state_3d.xml.h, read here by name so that a wrong
//!   constant in src/vivante/regs.rs shows);
//! - RGBA32I and RGBA32UI, which the PE has no format for, refused by name
//!   (`rt_format_drawable`, GL_INVALID_OPERATION) and never drawn;
//! - the four 64-bit surfaces (RGBA16I/UI, RG32I/UI), drawn as RGBA16F is:
//!   into the 4x4-tiled shadow of `src/vivante/wide_target.rs` (gl_test case
//!   44: the PE misplaces 64-bit pixels in a LINEAR target), its address,
//!   stride and SINGLE_BUFFER(2) in the stream and the shadow copied back;
//! - the state words of a real draw and a real glClearBufferuiv through the
//!   GL core and the Vivante back end on a stand-in OSAL that keeps every
//!   submitted stream (as tests/gc_npot_repeat.rs): PE_COLOR_FORMAT,
//!   PS_CONTROL_EXT, no unorm saturate, no blend, a second target's
//!   PE_RT_CONFIG and output mode, and the clear's four raw integers in the
//!   clear shader's constants;
//! - that the online compiler stores an `ivec4`/`uvec4` fragment output as
//!   the register's integer bits, unconverted: the encoded program run in the
//!   interpreter of tests/common.
//!
//! What it cannot see: the GPU. Whether the PE stores the integers as the
//! mode says is the board's (docs/es3/int-rt-gc.md, "For the board").
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante --test es3_int_rt_gc
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_int_rt_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", not(feature = "report-es2")))]

#[cfg(feature = "shaderc-online")]
mod common;

/// The online compiler's store (scratch and keep extents) is one static for
/// the process. A compile frees the previous compile's extents
/// (`forget_compiled`, a back end's `gl_context_destroy`), so every test that
/// compiles, in `gl_path` or in `compiler`, takes this one lock. Two locks, one
/// per module, let a `compiler` test free extents under a `gl_path` compile:
/// "did not compile", or the "extent freed twice" assert in `store.rs`.
#[cfg(feature = "shaderc-online")]
static STORE: std::sync::Mutex<()> = std::sync::Mutex::new(());

use yakogl::backend::*;
use yakogl::vivante::backend::{pe_color_format, GC_INTEGER_RT_FORMATS, GC_INT_RT_128BPP, GC_INT_RT_64BPP};
use yakogl::vivante::regs::*;
use yakogl::vivante::shader::ps_color_output_mode;

/* ---- the rnndb header, by name -------------------------------------------- */

/// The value of `#define <name>` in driver/src/vivante/hw/state_3d.xml.h.
fn rnndb(name: &str) -> u32 {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/driver/src/vivante/hw/state_3d.xml.h");
    let text = std::fs::read_to_string(path).expect("the vendored rnndb header");
    let line = text
        .lines()
        .find(|l| l.split_whitespace().nth(1) == Some(name) && l.starts_with("#define"))
        .unwrap_or_else(|| panic!("{name} is in state_3d.xml.h"));
    let v = line.split_whitespace().nth(2).expect("a value");
    u32::from_str_radix(v.trim_start_matches("0x"), 16).expect("hex")
}

/// Every integer surface: its rnndb PE format and output mode, or `None`
/// where the PE has no format (the 128-bit ones).
fn expected(s: u8) -> Option<(&'static str, &'static str)> {
    Some(match s {
        GLES_SURF_R8I => ("PE_FORMAT_R8I", "COLOR_OUTPUT_MODE_I8"),
        GLES_SURF_R8UI => ("PE_FORMAT_R8I", "COLOR_OUTPUT_MODE_U8"),
        GLES_SURF_RG8I => ("PE_FORMAT_G8R8I", "COLOR_OUTPUT_MODE_I8"),
        GLES_SURF_RG8UI => ("PE_FORMAT_G8R8I", "COLOR_OUTPUT_MODE_U8"),
        GLES_SURF_RGBA8I => ("PE_FORMAT_A8B8G8R8I", "COLOR_OUTPUT_MODE_I8"),
        GLES_SURF_RGBA8UI => ("PE_FORMAT_A8B8G8R8I", "COLOR_OUTPUT_MODE_U8"),
        GLES_SURF_R16I => ("PE_FORMAT_R16I", "COLOR_OUTPUT_MODE_I16"),
        GLES_SURF_R16UI => ("PE_FORMAT_R16I", "COLOR_OUTPUT_MODE_U16"),
        GLES_SURF_RG16I => ("PE_FORMAT_G16R16I", "COLOR_OUTPUT_MODE_I16"),
        GLES_SURF_RG16UI => ("PE_FORMAT_G16R16I", "COLOR_OUTPUT_MODE_U16"),
        GLES_SURF_RGBA16I => ("PE_FORMAT_A16B16G16R16I", "COLOR_OUTPUT_MODE_I16"),
        GLES_SURF_RGBA16UI => ("PE_FORMAT_A16B16G16R16I", "COLOR_OUTPUT_MODE_U16"),
        /* no 32-bit integer PE format: the float one of the same width,
         * stored bit for bit by the UIF32 mode */
        GLES_SURF_R32I | GLES_SURF_R32UI => ("PE_FORMAT_R32F", "COLOR_OUTPUT_MODE_UIF32"),
        GLES_SURF_RG32I | GLES_SURF_RG32UI => ("PE_FORMAT_G32R32F", "COLOR_OUTPUT_MODE_UIF32"),
        GLES_SURF_RGB10_A2UI => ("PE_FORMAT_A2B10G10R10UI", "COLOR_OUTPUT_MODE_A2B10G10R10UI"),
        GLES_SURF_RGBA32I | GLES_SURF_RGBA32UI => return None,
        _ => panic!("{} is not an integer surface", gles_surf_name(s)),
    })
}

/// The pixel size the PE format implies, from its name: what the GL core's
/// storage (`gles_surf_bytes_per_pixel`) must match, or the PE would walk
/// the surface at another pitch.
fn pe_format_bytes(name: &str) -> u32 {
    match name {
        "PE_FORMAT_R8I" => 1,
        "PE_FORMAT_G8R8I" | "PE_FORMAT_R16I" => 2,
        "PE_FORMAT_A8B8G8R8I" | "PE_FORMAT_G16R16I" | "PE_FORMAT_R32F" | "PE_FORMAT_A2B10G10R10UI" => 4,
        "PE_FORMAT_A16B16G16R16I" | "PE_FORMAT_G32R32F" => 8,
        _ => unreachable!("{name}"),
    }
}

/// Is the surface drawn? Every one but the 128-bit ones (no PE format).
fn drawn(s: u8) -> bool {
    expected(s).is_some()
}

/// Is a single-target draw into the surface rendered through the tiled
/// shadow (`src/vivante/wide_target.rs`)? The 64-bit ones, as RGBA16F.
fn shadowed(s: u8) -> bool {
    gles_surf_bytes_per_pixel(s) == 8
}

#[test]
fn every_integer_surface_takes_the_rnndb_pe_format_and_output_mode() {
    let mut drawable = 0u32;
    let mut wide = 0u32;
    for s in GLES_SURF_INT_FIRST..=GLES_SURF_INT_LAST {
        let name = gles_surf_name(s);
        let bit = 1u32 << (s - GLES_SURF_INT_FIRST);
        match expected(s) {
            Some((pe, mode)) => {
                let (fmt, ext) = pe_color_format(s);
                assert_eq!(fmt, rnndb(pe), "{name}: PE format {pe}");
                /* every one is past the 4-bit FORMAT field: FORMAT_EXT with FORMAT_MASK */
                assert!(ext && fmt >= 0x10, "{name}: an extended format");
                assert_eq!(ps_color_output_mode(s), rnndb(mode), "{name}: {mode}");
                assert_eq!(gles_surf_bytes_per_pixel(s), pe_format_bytes(pe), "{name}: pixel size of {pe}");
                assert_eq!(rt_format_drawable(s), drawn(s), "{name}");
                assert!(drawn(s), "{name}");
                drawable |= bit;
                if shadowed(s) {
                    wide |= bit;
                }
            }
            None => {
                assert!(!rt_format_drawable(s), "{name}: the PE has no format; refused by name");
                assert_eq!(GC_INTEGER_RT_FORMATS & bit, 0, "{name}");
                assert_ne!(GC_INT_RT_128BPP & bit, 0, "{name}");
            }
        }
    }
    assert_eq!(GC_INTEGER_RT_FORMATS, drawable);
    assert_eq!(GC_INT_RT_64BPP, wide, "GC_INT_RT_64BPP is the four 64-bit surfaces");
    assert_eq!(GC_INTEGER_RT_FORMATS & GC_INT_RT_64BPP, GC_INT_RT_64BPP, "the 64-bit ones are drawn");
    assert_eq!(INTEGER_RT_FORMATS, drawable, "the build's mask is the back end's");
    assert_eq!((drawable.count_ones(), wide.count_ones(), GC_INT_RT_128BPP.count_ones()), (17, 4, 2));
}

/// The register constants the back end writes are the rnndb's.
#[test]
fn the_new_register_constants_are_the_rnndb_values() {
    for (ours, name) in [
        (GC_PE_FORMAT_G32R32F, "PE_FORMAT_G32R32F"),
        (GC_PE_FORMAT_R8I, "PE_FORMAT_R8I"),
        (GC_PE_FORMAT_G8R8I, "PE_FORMAT_G8R8I"),
        (GC_PE_FORMAT_A8B8G8R8I, "PE_FORMAT_A8B8G8R8I"),
        (GC_PE_FORMAT_R16I, "PE_FORMAT_R16I"),
        (GC_PE_FORMAT_G16R16I, "PE_FORMAT_G16R16I"),
        (GC_PE_FORMAT_A16B16G16R16I, "PE_FORMAT_A16B16G16R16I"),
        (GC_PE_FORMAT_A2B10G10R10UI, "PE_FORMAT_A2B10G10R10UI"),
        (GC_PS_OUTPUT_MODE_A2B10G10R10UI, "COLOR_OUTPUT_MODE_A2B10G10R10UI"),
        (GC_PS_OUTPUT_MODE_U8, "COLOR_OUTPUT_MODE_U8"),
        (GC_PS_OUTPUT_MODE_U16, "COLOR_OUTPUT_MODE_U16"),
        (GC_PS_OUTPUT_MODE_I8, "COLOR_OUTPUT_MODE_I8"),
        (GC_PS_OUTPUT_MODE_I16, "COLOR_OUTPUT_MODE_I16"),
        (GC_PS_OUTPUT_MODE_UIF32, "COLOR_OUTPUT_MODE_UIF32"),
    ] {
        assert_eq!(ours, rnndb(name), "{name}");
    }
}

/// The non-integer surfaces keep what they had: the ES 2.0 formats and the
/// float ones, byte for byte.
#[test]
fn the_other_surfaces_are_unchanged() {
    for (s, fmt, ext, mode) in [
        (GLES_SURF_RGBA8888, GC_PE_FORMAT_A8R8G8B8, false, GC_PS_OUTPUT_MODE_NORMAL),
        (GLES_SURF_XRGB8888, GC_PE_FORMAT_X8R8G8B8, false, GC_PS_OUTPUT_MODE_NORMAL),
        (GLES_SURF_RGB565, GC_PE_FORMAT_R5G6B5, false, GC_PS_OUTPUT_MODE_NORMAL),
        (GLES_SURF_RGBA16F, GC_PE_FORMAT_A16B16G16R16F, true, GC_PS_OUTPUT_MODE_NORMAL),
        (GLES_SURF_R16F, GC_PE_FORMAT_R16F, true, GC_PS_OUTPUT_MODE_NORMAL),
        (GLES_SURF_R32F, GC_PE_FORMAT_R32F, true, GC_PS_OUTPUT_MODE_UIF32),
        (GLES_SURF_SRGB8_A8, GC_PE_FORMAT_A8R8G8B8, false, GC_PS_OUTPUT_MODE_NORMAL),
    ] {
        assert_eq!(pe_color_format(s), (fmt, ext), "{}", gles_surf_name(s));
        assert_eq!(ps_color_output_mode(s), mode, "{}", gles_surf_name(s));
    }
}

/* ---- the GL path: a real draw and clear, the stream decoded --------------- */

#[cfg(feature = "shaderc-online")]
mod gl_path {
    use super::*;
    use core::ffi::{c_char, c_void};
    use std::collections::BTreeMap;
    use std::ffi::CString;
    use std::sync::Mutex;
    use yakogl::gl::consts::*;
    use yakogl::gl::draw::*;
    use yakogl::gl::es3::*;
    use yakogl::gl::framebuffer::*;
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::state::*;
    use yakogl::gl::types::*;
    use yakogl::gl::vertex::*;
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
    use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};

    /* the stand-in OSAL: host memory, and every stream the back end submits */

    #[derive(Default)]
    struct Platform {
        allocs: BTreeMap<u64, Box<[u64]>>,
        bump: BTreeMap<u64, u64>,
        fence: i64,
        kicks: Vec<Vec<u32>>,
    }

    static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

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

    /// Every `(register, value)` a LOAD_STATE of the stream writes, in order.
    fn writes(words: &[u32]) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            if w & 0xf800_0000 == GC_OP_LOAD_STATE {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for k in 0..count {
                    if let Some(v) = words.get(i + 1 + k) {
                        out.push((base + 4 * k as u32, *v));
                    }
                }
                i += 1 + count;
                continue;
            }
            i += match w & 0xf800_0000 {
                GC_OP_DRAW_INSTANCED => 4,
                GC_OP_STALL => 2,
                _ => 1,
            };
        }
        out
    }

    /// Run `case` with the Vivante back end up on the stand-in OSAL and a
    /// fresh 64x64 context.
    fn on_gc7000(case: impl FnOnce()) {
        let _g = super::STORE.lock().unwrap_or_else(|e| e.into_inner());
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

    fn err() -> GLenum {
        // SAFETY: a plain GL call.
        unsafe { glGetError() }
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

    fn program(fs: &str) -> GLuint {
        const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
            glBindAttribLocation(p, 0, c"p".as_ptr());
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link failed");
            p
        }
    }

    /// The GL internal format, the surface it becomes, and the fragment
    /// output type that writes it.
    const FORMATS: [(GLenum, u8, &str); 19] = [
        (GL_R8I, GLES_SURF_R8I, "ivec4"),
        (GL_R8UI, GLES_SURF_R8UI, "uvec4"),
        (GL_R16I, GLES_SURF_R16I, "ivec4"),
        (GL_R16UI, GLES_SURF_R16UI, "uvec4"),
        (GL_R32I, GLES_SURF_R32I, "ivec4"),
        (GL_R32UI, GLES_SURF_R32UI, "uvec4"),
        (GL_RG8I, GLES_SURF_RG8I, "ivec4"),
        (GL_RG8UI, GLES_SURF_RG8UI, "uvec4"),
        (GL_RG16I, GLES_SURF_RG16I, "ivec4"),
        (GL_RG16UI, GLES_SURF_RG16UI, "uvec4"),
        (GL_RG32I, GLES_SURF_RG32I, "ivec4"),
        (GL_RG32UI, GLES_SURF_RG32UI, "uvec4"),
        (GL_RGBA8I, GLES_SURF_RGBA8I, "ivec4"),
        (GL_RGBA8UI, GLES_SURF_RGBA8UI, "uvec4"),
        (GL_RGBA16I, GLES_SURF_RGBA16I, "ivec4"),
        (GL_RGBA16UI, GLES_SURF_RGBA16UI, "uvec4"),
        (GL_RGBA32I, GLES_SURF_RGBA32I, "ivec4"),
        (GL_RGBA32UI, GLES_SURF_RGBA32UI, "uvec4"),
        (GL_RGB10_A2UI, GLES_SURF_RGB10_A2UI, "uvec4"),
    ];

    fn renderbuffer(internal: GLenum) -> GLuint {
        let mut rb = 0;
        // SAFETY: plain GL calls.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, internal, 16, 16);
        }
        assert_eq!(err(), GL_NO_ERROR, "glRenderbufferStorage(0x{internal:x})");
        rb
    }

    /// A framebuffer with `rbs` at colour attachments 0.., every one a draw buffer.
    fn framebuffer(rbs: &[GLuint]) -> GLuint {
        let mut fb = 0;
        let bufs: Vec<GLenum> = (0..rbs.len() as u32).map(|i| GL_COLOR_ATTACHMENT0 + i).collect();
        // SAFETY: plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            for (i, &rb) in rbs.iter().enumerate() {
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as u32, GL_RENDERBUFFER, rb);
            }
            glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        assert_eq!(err(), GL_NO_ERROR, "framebuffer");
        fb
    }

    static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

    /// Draw the quad with `fs`, finish, and return the writes of the streams
    /// submitted since `kicks0` (none if the draw was refused).
    fn draw(fs: &str) -> (GLenum, Vec<(u32, u32)>) {
        let kicks0 = with_platform(|p| p.kicks.len());
        // SAFETY: plain GL calls; QUAD is a static.
        let e = unsafe {
            glViewport(0, 0, 16, 16);
            glUseProgram(program(fs));
            /* GL_BLEND is ignored for an integer buffer (ES 3.0 4.1.8) */
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
            glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
            let e = glGetError();
            glFinish();
            e
        };
        (e, with_platform(|p| p.kicks[kicks0..].iter().flat_map(|k| writes(k)).collect()))
    }

    fn last(w: &[(u32, u32)], reg: u32) -> Option<u32> {
        w.iter().rev().find(|(r, _)| *r == reg).map(|(_, v)| *v)
    }

    fn fs_for(ty: &str) -> String {
        let one = if ty == "ivec4" { "ivec4(-3, 7, -100000, 2)" } else { "uvec4(3u, 7u, 4000000000u, 2u)" };
        format!("#version 300 es\nlayout(location = 0) out highp {ty} o;\nvoid main() {{ o = {one}; }}\n")
    }

    /// The pixel engine's surface words of a 16x16 single-target draw or
    /// clear. A 64-bit target renders into its tiled shadow (wide_target.rs):
    /// a page-aligned address, the padded row stride (16 pixels * 8 bytes),
    /// PE_LOGIC_OP SINGLE_BUFFER(2), no SUPER_TILED bit, and the shadow
    /// untiled back into the storage once the kick completed. Every other
    /// target is LINEAR, SINGLE_BUFFER(1), at its own 16-pixel pitch.
    fn assert_surface(name: &str, surf: u8, w: &[(u32, u32)], copied_back: u64) {
        let color = last(w, GC_PE_COLOR_FORMAT).unwrap();
        let tiled = rnndb("VIVS_PE_COLOR_FORMAT_SUPER_TILED") | rnndb("VIVS_PE_COLOR_FORMAT_SUPER_TILED_NEW");
        assert_eq!(color & tiled, 0, "{name}: no SUPER_TILED bit");
        /* FORMAT_EXT's own write mask (bit 31) clear, or the PE keeps its old
         * FORMAT_EXT (VIVANTE_NOTES.md, "RGBA16F targets") */
        assert_eq!(color & rnndb("VIVS_PE_COLOR_FORMAT_FORMAT_EXT_MASK"), 0, "{name}: FORMAT_EXT written");
        let logic = last(w, GC_PE_LOGIC_OP).expect("PE_LOGIC_OP");
        assert_eq!(logic & rnndb("VIVS_PE_LOGIC_OP_SINGLE_BUFFER_MASK"), 0, "{name}: SINGLE_BUFFER written");
        let single_mask = rnndb("VIVS_PE_LOGIC_OP_SINGLE_BUFFER__MASK");
        let single = (logic & single_mask) >> single_mask.trailing_zeros();
        let stride = last(w, rnndb("VIVS_PE_COLOR_STRIDE")).expect("PE_COLOR_STRIDE");
        let addr = last(w, pe_pipe_color_addr(0)).expect("PE_PIPE_COLOR_ADDR(0)");
        assert_eq!(last(w, pe_pipe_color_addr(1)), Some(addr), "{name}: both pipes, one surface");
        if shadowed(surf) {
            assert_eq!(single, 2, "{name}: SINGLE_BUFFER(2), the tiled 64-bit value");
            assert_eq!(stride, yakogl::vivante::wide_target::stride(16), "{name}: the shadow's stride");
            assert_eq!(stride, 16 * 8, "{name}");
            assert_eq!(addr & 0xfff, 0, "{name}: the shadow starts on a page");
            assert_eq!(copied_back, 16 * 16 * 8, "{name}: the shadow went back into the storage");
        } else {
            assert_eq!(single, 1, "{name}: SINGLE_BUFFER(1), linear");
            assert_eq!(stride, 16 * gles_surf_bytes_per_pixel(surf), "{name}: linear pitch");
            assert_eq!(copied_back, 0, "{name}: no shadow");
        }
    }

    /// Bytes untiled out of shadows since init.
    fn untiled() -> u64 {
        yakogl::vivante::wide_target::counts().1
    }

    /// One draw into each integer format: the PE format, the output mode,
    /// no saturate and no blend, the tiled shadow for the four 64-bit ones,
    /// or a refusal by name for the two the PE has no format for.
    #[test]
    fn a_draw_into_each_integer_target_programs_its_pe_format_and_output_mode() {
        for (internal, surf, ty) in FORMATS {
            on_gc7000(|| {
                let name = gles_surf_name(surf);
                let rb = renderbuffer(internal);
                framebuffer(&[rb]);
                let untiled0 = untiled();
                let (e, w) = draw(&fs_for(ty));
                let Some((pe, mode)) = expected(surf).filter(|_| drawn(surf)) else {
                    assert_eq!(e, GL_INVALID_OPERATION, "{name}: refused by name");
                    assert!(
                        !w.iter().any(|&(r, _)| r == GC_PE_COLOR_FORMAT),
                        "{name}: nothing reached the pixel engine"
                    );
                    return;
                };
                assert_eq!(e, GL_NO_ERROR, "{name}: drawn");
                let color =
                    last(&w, rnndb("VIVS_PE_COLOR_FORMAT")).unwrap_or_else(|| panic!("{name}: no PE_COLOR_FORMAT"));
                let ext_mask = rnndb("VIVS_PE_COLOR_FORMAT_FORMAT_EXT__MASK");
                assert_eq!((color & ext_mask) >> 24, rnndb(pe), "{name}: FORMAT_EXT is {pe}");
                assert_ne!(color & rnndb("VIVS_PE_COLOR_FORMAT_FORMAT_MASK"), 0, "{name}: FORMAT_MASK");
                /* the whole word, as etnaviv writes it with no blend
                 * (VIVANTE_NOTES.md "RGBA16F targets": 0x13010f10 for
                 * RGBA16F): FORMAT_EXT, FORMAT_MASK, every channel the
                 * target has (one without alpha keeps A out of COMPONENTS,
                 * and so loses OVERWRITE), no FORMAT_EXT_MASK; 0x1c010f10
                 * for RGBA16I/UI, 0x15000710 for RG32I/UI */
                let field = |mask: &str, v: u32| (v << rnndb(mask).trailing_zeros()) & rnndb(mask);
                let alpha = gles_surf_int_layout(surf).expect("an integer layout").channels == 4;
                let word = field("VIVS_PE_COLOR_FORMAT_FORMAT_EXT__MASK", rnndb(pe))
                    | rnndb("VIVS_PE_COLOR_FORMAT_FORMAT_MASK")
                    | field("VIVS_PE_COLOR_FORMAT_COMPONENTS__MASK", if alpha { 0xf } else { 0x7 })
                    | if alpha { rnndb("VIVS_PE_COLOR_FORMAT_OVERWRITE") } else { 0 };
                assert_eq!(color, word, "{name}: PE_COLOR_FORMAT");
                match surf {
                    GLES_SURF_RGBA16I | GLES_SURF_RGBA16UI => assert_eq!(color, 0x1c01_0f10, "{name}"),
                    GLES_SURF_RG32I | GLES_SURF_RG32UI => assert_eq!(color, 0x1500_0710, "{name}"),
                    _ => {}
                }
                let ext = last(&w, rnndb("VIVS_PS_CONTROL_EXT")).expect("PS_CONTROL_EXT");
                assert_eq!(ext & 0x7, rnndb(mode), "{name}: OUTPUT_MODE0 is {mode}");
                let ctl = last(&w, rnndb("VIVS_PS_CONTROL")).expect("PS_CONTROL");
                assert_eq!(ctl & rnndb("VIVS_PS_CONTROL_SATURATE_RT0"), 0, "{name}: an integer is not saturated");
                assert_eq!(last(&w, rnndb("VIVS_PE_ALPHA_CONFIG")), Some(0), "{name}: never blended");
                assert_surface(name, surf, &w, untiled() - untiled0);
            });
        }
    }

    /// The same draw into RGBA8 still saturates and blends: the integer
    /// rule did not leak into the ES 2.0 path.
    #[test]
    fn an_rgba8_target_still_saturates_and_blends() {
        on_gc7000(|| {
            let rb = renderbuffer(GL_RGBA8);
            framebuffer(&[rb]);
            let (e, w) =
                draw("#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(0.5); }\n");
            assert_eq!(e, GL_NO_ERROR);
            let color = last(&w, GC_PE_COLOR_FORMAT).unwrap();
            assert_eq!(color & 0xf, GC_PE_FORMAT_A8R8G8B8, "A8R8G8B8 in the 4-bit field");
            assert_eq!(last(&w, GC_PS_CONTROL_EXT).unwrap() & 0x7, GC_PS_OUTPUT_MODE_NORMAL);
            assert_ne!(last(&w, GC_PS_CONTROL).unwrap() & GC_PS_CONTROL_SATURATE_RT0, 0, "unorm saturates");
            assert_ne!(last(&w, GC_PE_ALPHA_CONFIG).unwrap(), 0, "blended");
        });
    }

    /// Two targets, RGBA8 and RG16UI, with GL_BLEND on: the second takes the
    /// 8-RT bank's PE_RT_CONFIG with the integer format and its own output
    /// mode; only the first saturates, and only the first is blended (ES 3.0
    /// 4.1.8: blending is ignored for an integer draw buffer).
    #[test]
    fn an_integer_second_target_takes_its_own_format_mode_and_no_blend() {
        on_gc7000(|| {
            let a = renderbuffer(GL_RGBA8);
            let b = renderbuffer(GL_RG16UI);
            framebuffer(&[a, b]);
            let fs = "#version 300 es\nprecision mediump float;\nlayout(location = 0) out vec4 c;\n\
                      layout(location = 1) out highp uvec4 u;\nvoid main() { c = vec4(0.5); u = uvec4(1u, 2u, 3u, 65535u); }\n";
            let (e, w) = draw(fs);
            assert_eq!(e, GL_NO_ERROR);
            let cfg = last(&w, pe_rt_config_8(1)).expect("PE_RT_CONFIG_8(0)");
            assert_eq!(cfg & rt_config_format(0x3f), rt_config_format(rnndb("PE_FORMAT_G16R16I")));
            let ext = last(&w, GC_PS_CONTROL_EXT).unwrap();
            assert_eq!(ext & 0x7, GC_PS_OUTPUT_MODE_NORMAL, "RT0 RGBA8");
            assert_eq!((ext >> 4) & 0x7, rnndb("COLOR_OUTPUT_MODE_U16"), "RT1 RG16UI");
            let ctl = last(&w, GC_PS_CONTROL).unwrap();
            assert_ne!(ctl & ps_control_saturate_rt(0), 0, "RT0 saturates");
            assert_eq!(ctl & ps_control_saturate_rt(1), 0, "RT1 does not");
            assert_ne!(last(&w, GC_PE_ALPHA_CONFIG).unwrap(), 0, "RT0 blended");
            assert_eq!(last(&w, pe_halti5_rt_alpha_config(1)), Some(0), "RT1 never blended");
        });
    }

    /// RGBA32UI as a second draw buffer is refused as the first is: by name,
    /// nothing drawn.
    #[test]
    fn a_128_bit_second_target_refuses_the_draw() {
        on_gc7000(|| {
            let a = renderbuffer(GL_RGBA8);
            let b = renderbuffer(GL_RGBA32UI);
            framebuffer(&[a, b]);
            let fs = "#version 300 es\nprecision mediump float;\nlayout(location = 0) out vec4 c;\n\
                      layout(location = 1) out highp uvec4 u;\nvoid main() { c = vec4(0.5); u = uvec4(1u); }\n";
            let (e, w) = draw(fs);
            assert_eq!(e, GL_INVALID_OPERATION);
            assert!(!w.iter().any(|&(r, _)| r == pe_rt_config_8(1)), "nothing reached the second target");
            assert!(!w.iter().any(|&(r, _)| r == GC_PE_COLOR_FORMAT), "nothing reached the pixel engine");
        });
    }

    /// A 64-bit integer target beside another draw buffer is refused by the
    /// back end: the 8-RT bank has no tiled bit of its own, so a draw with
    /// more than one target would keep the linear path, where the PE
    /// misplaces 64-bit pixels (gl_test case 44, wide_target.rs "Not
    /// covered"). Each of the four, as first and as second target: nothing
    /// reaches the pixel engine, no shadow is made.
    #[test]
    fn a_64_bit_target_beside_another_refuses_the_draw() {
        for (internal, ty) in [(GL_RGBA16I, "ivec4"), (GL_RGBA16UI, "uvec4"), (GL_RG32I, "ivec4"), (GL_RG32UI, "uvec4")]
        {
            for wide_first in [false, true] {
                on_gc7000(|| {
                    let a = renderbuffer(GL_RGBA8);
                    let b = renderbuffer(internal);
                    let (fs, rbs) = if wide_first {
                        (
                            format!(
                                "#version 300 es\nprecision mediump float;\nlayout(location = 0) out highp {ty} u;\n\
                                  layout(location = 1) out vec4 c;\nvoid main() {{ u = {ty}(1); c = vec4(0.5); }}\n"
                            ),
                            [b, a],
                        )
                    } else {
                        (format!("#version 300 es\nprecision mediump float;\nlayout(location = 0) out vec4 c;\n\
                                  layout(location = 1) out highp {ty} u;\nvoid main() {{ c = vec4(0.5); u = {ty}(1); }}\n"), [a, b])
                    };
                    framebuffer(&rbs);
                    let untiled0 = untiled();
                    let (e, w) = draw(&fs);
                    let what = format!("{internal:#x} first={wide_first}");
                    assert_eq!(e, GL_INVALID_OPERATION, "{what}");
                    assert!(
                        !w.iter().any(|&(r, _)| r == pe_rt_config_8(1)),
                        "{what}: nothing reached the second target"
                    );
                    assert!(
                        !w.iter().any(|&(r, _)| r == GC_PE_COLOR_FORMAT),
                        "{what}: nothing reached the pixel engine"
                    );
                    assert_eq!(untiled(), untiled0, "{what}: no shadow");
                });
            }
        }
    }

    /// glClearBufferuiv / glClearBufferiv: the clear quad runs with the
    /// target's integer format and output mode, and its constant is the four
    /// values' bits, which the clear shader moves to its output unconverted.
    #[test]
    fn an_integer_clear_writes_the_raw_values_in_the_integer_mode() {
        for (internal, surf, ty) in FORMATS {
            let Some((pe, mode)) = expected(surf).filter(|_| drawn(surf)) else { continue };
            on_gc7000(|| {
                let name = gles_surf_name(surf);
                let rb = renderbuffer(internal);
                framebuffer(&[rb]);
                let kicks0 = with_platform(|p| p.kicks.len());
                let untiled0 = untiled();
                let bits: [u32; 4] = [0x8000_0001, 7, 0xffff_fff0, 0x1234_5678];
                // SAFETY: four values; plain GL calls.
                unsafe {
                    if ty == "ivec4" {
                        glClearBufferiv(GL_COLOR, 0, bits.as_ptr() as *const GLint);
                    } else {
                        glClearBufferuiv(GL_COLOR, 0, bits.as_ptr());
                    }
                    assert_eq!(glGetError(), GL_NO_ERROR, "{name}: glClearBuffer");
                    glFinish();
                }
                let w: Vec<(u32, u32)> = with_platform(|p| p.kicks[kicks0..].iter().flat_map(|k| writes(k)).collect());
                let color = last(&w, GC_PE_COLOR_FORMAT).unwrap_or_else(|| panic!("{name}: no clear reached the PE"));
                assert_eq!((color >> 24) & 0x7f, rnndb(pe), "{name}: the clear's FORMAT_EXT");
                assert_eq!(last(&w, GC_PS_CONTROL_EXT).unwrap() & 0x7, rnndb(mode), "{name}: the clear's mode");
                assert_eq!(last(&w, GC_PS_CONTROL).unwrap() & GC_PS_CONTROL_SATURATE_RT0, 0, "{name}");
                assert_surface(name, surf, &w, untiled() - untiled0);
                /* the four values in a row in the fragment constant window */
                let ps: Vec<u32> = w
                    .iter()
                    .filter(|(r, _)| (GC_SH_HALTI5_UNIFORMS_PS..GC_SH_HALTI5_UNIFORMS_PS + 0x2000).contains(r))
                    .map(|(_, v)| *v)
                    .collect();
                assert!(ps.windows(4).any(|q| q == bits), "{name}: the clear constant is the raw values: {ps:x?}");
            });
        }
    }
}

/* ---- the compiler's half: integer outputs leave unconverted --------------- */

#[cfg(feature = "shaderc-online")]
mod compiler {
    use super::common::{run, Run};
    use yakogl::shaderc::online::backend::{forget_compiled, SHC_ONLINE};
    use yakogl::shaderc::online::vivante::isa::Instr;
    use yakogl::shaderc::online::vivante::{decode, listing};
    use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT};

    struct Compiled {
        code: Vec<Instr>,
        consts: Vec<[u32; 4]>,
        in_reg: Option<u8>,
        out_reg: u8,
    }

    fn compile(src: &str) -> Compiled {
        let _serial = super::STORE.lock().unwrap_or_else(|e| e.into_inner());
        forget_compiled();
        let text = std::ffi::CString::new(src).unwrap();
        let mut sh = ShcShader::ZERO;
        let mut log = [0 as core::ffi::c_char; 512];
        let compile = SHC_ONLINE.compile.expect("the online back end compiles");
        // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
        let rc = unsafe { compile(SHC_STAGE_FRAGMENT, text.as_ptr(), &mut sh, log.as_mut_ptr(), log.len()) };
        let log: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
        assert_eq!(rc, 0, "{src}: {log}");
        // SAFETY: the record's code, `usc_dw` dwords, alive until the next compile.
        let words = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
        let code = words.chunks(4).map(|c| decode::decode_instr(&c.try_into().unwrap()).expect("decodes")).collect();
        let mut consts = vec![[0u32; 4]; 512];
        for (k, &data) in sh.viv.const_data().iter().enumerate() {
            consts[k / 4][k % 4] = data;
        }
        let in_reg = (sh.viv.num_in > 0).then(|| sh.viv.in_[0].reg);
        Compiled { code, consts, in_reg, out_reg: sh.viv.ps_color_out_reg }
    }

    fn ops(c: &Compiled) -> Vec<String> {
        c.code
            .iter()
            .enumerate()
            .map(|(i, ins)| {
                let mut t = String::new();
                listing::instr_text(&mut t, i, ins).unwrap();
                t.split_whitespace().nth(5).unwrap_or("").to_string()
            })
            .collect()
    }

    fn output(c: &Compiled, input: [u32; 4]) -> [u32; 4] {
        let mut regs = vec![[0u32; 4]; 128];
        if let Some(r) = c.in_reg {
            regs[r as usize] = input;
        }
        match run(&c.code, &regs, &c.consts) {
            Run::Done { regs, .. } => regs[c.out_reg as usize],
            Run::Unsupported(why) => panic!("the interpreter cannot run it: {why}: {:?}", ops(c)),
        }
    }

    /// A flat integer input through integer arithmetic to a `uvec4` / `ivec4`
    /// output: the output register holds the integers' bits (no `i2f`, no
    /// float conversion on the way out), which the PE's integer mode stores.
    #[test]
    fn an_integer_output_is_the_registers_integer_bits() {
        let u = compile(
            "#version 300 es\nflat in highp uvec4 v;\nlayout(location = 0) out highp uvec4 o;\n\
             void main() { o = v * 3u + uvec4(1u, 2u, 3u, 4000000000u); }\n",
        );
        assert!(!ops(&u).iter().any(|o| o.starts_with("i2f") || o.starts_with("f2i")), "{:?}", ops(&u));
        let got = output(&u, [5, 0x1000_0000, 0, 100]);
        let want = [16u32, 0x3000_0002, 3, 4_000_000_300];
        assert_eq!(got, want);

        let i = compile(
            "#version 300 es\nflat in highp ivec4 v;\nlayout(location = 0) out highp ivec4 o;\n\
             void main() { o = v - ivec4(10, -20, 30, 2147483647); }\n",
        );
        assert!(!ops(&i).iter().any(|o| o.starts_with("i2f") || o.starts_with("f2i")), "{:?}", ops(&i));
        let got = output(&i, [3, (-5i32) as u32, 0, (-1i32) as u32]);
        let want = [(-7i32) as u32, 15, (-30i32) as u32, i32::MIN as u32];
        assert_eq!(got, want);
    }

    /// A float computed into an integer output: converted once, by the
    /// shader's own `int()` (`f2i`), and the result is an integer register.
    #[test]
    fn a_float_made_integer_is_converted_by_the_shader_only() {
        let c = compile(
            "#version 300 es\nprecision highp float;\nin vec4 f;\nlayout(location = 0) out highp ivec4 o;\n\
             void main() { o = ivec4(f * 255.0) - 7; }\n",
        );
        let ops = ops(&c);
        assert_eq!(ops.iter().filter(|o| o.starts_with("f2i")).count(), 1, "{ops:?}");
        assert!(!ops.iter().any(|o| o.starts_with("i2f")), "{ops:?}");
    }

    /// A uniform moved to an integer output (the clear shader's shape): one
    /// `mov`, bit for bit.
    #[test]
    fn a_uniform_moves_to_the_output_bit_for_bit() {
        let c = compile(
            "#version 300 es\nuniform highp uvec4 u;\nlayout(location = 0) out highp uvec4 o;\nvoid main() { o = u; }\n",
        );
        assert_eq!(ops(&c), ["mov"], "one move");
    }
}
