// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `layout(location = N)` against glBindAttribLocation, through the public GL
//! API on the host (item oq-v3d; docs/es3-cts-failures.md Appendix A, task T4).
//!
//! ES 3.0 section 2.12.3 and GLSL ES 3.00 section 4.3.8.1: an input with a
//! `layout(location = N)` qualifier is at N and a binding of its name is
//! ignored; a bound input without one is at the binding; the linker places
//! the rest where no qualifier or binding is. This reproduces dEQP-GLES3
//! `attribute_location.mixed.*`, `mixed_time.{pre_attach,pre_link,reattach,
//! relink,post_link}` and `mixed_max_attributes.*` (vec4 shape): the answers
//! of glGetAttribLocation, and the rows a draw feeds (one constant per
//! disabled array, recorded by the stand-in back end by `vi_reg`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_attrib_location
//! ```

#![cfg(feature = "host-test")]
// the cases need the in-process compiler and the ES 3.0 report; the helpers are idle elsewhere
#![cfg_attr(any(feature = "report-es2", not(feature = "shaderc-online")), allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one --- */

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

/// The attribute rows of the last draw that carry a constant: (`vi_reg`, the
/// first dword read as a float). A disabled array's current value reaches
/// the back end as a 16-byte zero-stride row (`GlesAttrib::cpu`).
static ROWS: Mutex<Vec<(u8, f32)>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    for a in &d.attribs[..(d.num_attribs as usize).min(d.attribs.len())] {
        if a.stride == 0 && a.cpu_bytes == 16 && !a.cpu.is_null() {
            let mut w = [0u8; 4];
            // SAFETY: `cpu` holds `cpu_bytes` bytes for the duration of the call.
            core::ptr::copy_nonoverlapping(a.cpu as *const u8, w.as_mut_ptr(), 4);
            ROWS.lock().unwrap().push((a.vi_reg, f32::from_ne_bytes(w)));
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
    lock: None,
    unlock: None,
    thread_id: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-attrib-location".as_ptr() as *const c_char,
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
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            #[cfg(feature = "shaderc-online")]
            {
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            }
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glUseProgram(0);
        while glGetError() != GL_NO_ERROR {}
    }
    ROWS.lock().unwrap().clear();
    g
}

/* ---- dEQP's shapes ---------------------------------------------------------- */

/// One vec4 input: its optional `layout(location)` and its optional
/// glBindAttribLocation index. The input is `a_<position in the list>`.
#[derive(Clone, Copy)]
struct Attr {
    layout: Option<u32>,
    bind: Option<u32>,
}

const fn a(layout: Option<u32>, bind: Option<u32>) -> Attr {
    Attr { layout, bind }
}

fn vertex_source(attrs: &[Attr]) -> String {
    let mut s = String::from("#version 300 es\n");
    for (i, at) in attrs.iter().enumerate() {
        match at.layout {
            Some(l) => s += &format!("layout(location = {l}) in highp vec4 a_{i};\n"),
            None => s += &format!("in highp vec4 a_{i};\n"),
        }
    }
    s += "void main() {\n  gl_Position = vec4(0.0)";
    for i in 0..attrs.len() {
        s += &format!(" + a_{i}");
    }
    s += ";\n}\n";
    s
}

const FS: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";

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

fn bind(p: GLuint, attrs: &[Attr]) {
    for (i, at) in attrs.iter().enumerate() {
        if let Some(b) = at.bind {
            let name = CString::new(format!("a_{i}")).unwrap();
            // SAFETY: a NUL-terminated name.
            unsafe { glBindAttribLocation(p, b, name.as_ptr()) };
        }
    }
}

fn link(p: GLuint) {
    // SAFETY: plain GL calls; a log buffer of the size passed.
    unsafe {
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let mut log = [0 as c_char; 512];
        glGetProgramInfoLog(p, 512, core::ptr::null_mut(), log.as_mut_ptr());
        let log = std::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned();
        assert_eq!(ok, 1, "the program did not link: {log}");
    }
}

fn locations(p: GLuint, n: usize) -> Vec<GLint> {
    (0..n)
        .map(|i| {
            let name = CString::new(format!("a_{i}")).unwrap();
            // SAFETY: a NUL-terminated name.
            unsafe { glGetAttribLocation(p, name.as_ptr()) }
        })
        .collect()
}

/// Draw with every input fed the constant `i + 1` at its own location, and
/// check the back end was handed one row per input, each at its own
/// `vi_reg`, carrying that input's constant.
fn draw_and_check(p: GLuint, locs: &[GLint]) {
    ROWS.lock().unwrap().clear();
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(p);
        for (i, &l) in locs.iter().enumerate() {
            glVertexAttrib4f(l as GLuint, (i + 1) as f32, 0.0, 0.0, 1.0);
        }
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR);
        glUseProgram(0);
    }
    let rows = ROWS.lock().unwrap().clone();
    let mut regs: Vec<u8> = rows.iter().map(|r| r.0).collect();
    regs.sort_unstable();
    regs.dedup();
    assert_eq!(regs.len(), locs.len(), "one row per input, each on its own register: {rows:?}");
    let mut values: Vec<u32> = rows.iter().map(|r| r.1 as u32).collect();
    values.sort_unstable();
    assert_eq!(values, (1..=locs.len() as u32).collect::<Vec<_>>(), "every input's constant arrived: {rows:?}");
}

/// The locations dEQP expects: a qualifier wins, else the binding, and the
/// rest are anywhere that is free (checked as distinct and in range).
fn expect(attrs: &[Attr], got: &[GLint], binds_apply: bool) {
    let mut seen = [false; 16];
    for (i, at) in attrs.iter().enumerate() {
        let g = got[i];
        assert!((0..16).contains(&g), "a_{i} is active and has a location: {got:?}");
        match (at.layout, at.bind) {
            (Some(l), _) => assert_eq!(g, l as GLint, "a_{i}: the qualifier wins over the binding ({got:?})"),
            (None, Some(b)) if binds_apply => assert_eq!(g, b as GLint, "a_{i}: bound ({got:?})"),
            _ => {}
        }
        assert!(!seen[g as usize], "a_{i} shares location {g} with another input: {got:?}");
        seen[g as usize] = true;
    }
}

fn program(attrs: &[Attr]) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, &vertex_source(attrs)));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        p
    }
}

/// dEQP `mixed`: qualified inputs bound elsewhere, bound inputs, and an
/// input that is neither. a_0 is bound to 4 and qualified 3: dEQP expects 3.
const MIXED: [Attr; 6] =
    [a(Some(3), Some(4)), a(Some(5), Some(2)), a(None, Some(0)), a(None, Some(7)), a(Some(1), None), a(None, None)];

/* an ES 2.0-reporting build refuses `#version 300 es` sources (the report-es2 row) */
#[cfg(all(feature = "shaderc-online", not(feature = "report-es2")))]
mod cases {
    use super::*;

    #[test]
    fn mixed_the_qualifier_wins_over_the_binding() {
        let _g = context();
        let p = program(&MIXED);
        bind(p, &MIXED);
        link(p);
        let got = locations(p, MIXED.len());
        expect(&MIXED, &got, true);
        draw_and_check(p, &got);
    }

    #[test]
    fn mixed_time_pre_attach() {
        let _g = context();
        // SAFETY: plain GL calls.
        let p = unsafe { glCreateProgram() };
        bind(p, &MIXED);
        // SAFETY: plain GL calls.
        unsafe {
            glAttachShader(p, shader(GL_VERTEX_SHADER, &vertex_source(&MIXED)));
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        }
        link(p);
        let got = locations(p, MIXED.len());
        expect(&MIXED, &got, true);
        draw_and_check(p, &got);
    }

    #[test]
    fn mixed_time_pre_link() {
        let _g = context();
        let p = program(&MIXED);
        bind(p, &MIXED);
        link(p);
        let got = locations(p, MIXED.len());
        expect(&MIXED, &got, true);
        draw_and_check(p, &got);
    }

    #[test]
    fn mixed_time_reattach() {
        let _g = context();
        let p = program(&MIXED);
        bind(p, &MIXED);
        link(p);
        let first = locations(p, MIXED.len());
        let mut sh = [0 as GLuint; 2];
        let mut n = 0;
        // SAFETY: room for two names.
        unsafe { glGetAttachedShaders(p, 2, &mut n, sh.as_mut_ptr()) };
        assert_eq!(n, 2);
        // SAFETY: plain GL calls on the attached shaders.
        unsafe {
            for s in sh {
                glDetachShader(p, s);
            }
            for s in sh {
                glAttachShader(p, s);
            }
        }
        link(p);
        let got = locations(p, MIXED.len());
        expect(&MIXED, &got, true);
        assert_eq!(first, got, "the same shaders and bindings link to the same locations");
        draw_and_check(p, &got);
    }

    #[test]
    fn mixed_time_relink() {
        let _g = context();
        let p = program(&MIXED);
        bind(p, &MIXED);
        link(p);
        /* new bindings for the unqualified inputs, and for a qualified one */
        let second = [
            a(Some(3), Some(9)),
            a(Some(5), Some(9)),
            a(None, Some(11)),
            a(None, Some(13)),
            a(Some(1), Some(8)),
            a(None, Some(14)),
        ];
        bind(p, &second);
        link(p);
        let got = locations(p, second.len());
        expect(&second, &got, true);
        draw_and_check(p, &got);
    }

    #[test]
    fn mixed_time_post_link_changes_nothing_until_the_next_link() {
        let _g = context();
        let p = program(&MIXED);
        bind(p, &MIXED);
        link(p);
        let before = locations(p, MIXED.len());
        let later: Vec<Attr> = (0..MIXED.len()).map(|i| a(None, Some(15 - i as u32))).collect();
        bind(p, &later);
        assert_eq!(locations(p, MIXED.len()), before, "a binding made after the link is not in effect");
        let got = before;
        draw_and_check(p, &got);
    }

    /// dEQP `mixed_max_attributes`, vec4: sixteen inputs, from the top
    /// location down alternately bound (unqualified) and qualified.
    /// a_0 is bound to 15 and a_15 is qualified 0: the compiler counts an
    /// unqualified input on from the one before it, so a_0 alone would sit at
    /// 0 and share that location with a_15.
    #[test]
    #[ignore = "needs reflect.rs to place unqualified inputs clear of qualified ones (docs/handoff/20261013-es3-attrib-location.md)"]
    fn mixed_max_attributes_vec4() {
        let _g = context();
        let attrs: Vec<Attr> =
            (0..16u32).map(|i| if i % 2 == 0 { a(None, Some(15 - i)) } else { a(Some(15 - i), None) }).collect();
        let p = program(&attrs);
        bind(p, &attrs);
        link(p);
        let got = locations(p, attrs.len());
        expect(&attrs, &got, true);
        for (i, g) in got.iter().enumerate() {
            assert_eq!(*g, 15 - i as GLint, "a_{i}");
        }
        draw_and_check(p, &got);
    }

    /// Unqualified, unbound inputs after qualified ones go where no
    /// qualifier is (ES 3.0 2.12.3: the linker chooses).
    #[test]
    #[ignore = "needs reflect.rs to place unqualified inputs clear of qualified ones (docs/handoff/20261013-es3-attrib-location.md)"]
    fn unqualified_unbound_inputs_avoid_every_qualifier() {
        let _g = context();
        let attrs = [a(None, None), a(Some(0), None), a(None, None), a(Some(2), None), a(None, None)];
        let p = program(&attrs);
        let got = {
            link(p);
            locations(p, attrs.len())
        };
        expect(&attrs, &got, true);
        draw_and_check(p, &got);
    }

    /// A qualifier in a comment is not an input's.
    #[test]
    fn qualifiers_in_comments_are_not_read() {
        let _g = context();
        let vs = "#version 300 es\n\
            // layout(location = 9) in vec4 a_0;\n\
            /* layout(location = 8) in vec4 a_1; */\n\
            layout(location = 6) in vec4 a_0;\n\
            in vec4 a_1;\n\
            void main() { gl_Position = a_0 + a_1; }\n";
        // SAFETY: plain GL calls.
        let p = unsafe {
            let p = glCreateProgram();
            glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
            p
        };
        // SAFETY: a NUL-terminated name.
        unsafe { glBindAttribLocation(p, 2, c"a_1".as_ptr()) };
        link(p);
        let got = locations(p, 2);
        assert_eq!(got, vec![6, 2], "a_0 is qualified 6; a_1 is bound to 2 and the commented 8 is not its own");
    }
}
