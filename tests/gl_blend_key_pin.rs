// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The fragment variant keys the GL core gives a draw that is not a mix of an
//! integer colour attachment and another class, pinned before the mixed
//! per-target key lands (`current_blend_key`, src/gl/draw.rs): a draw whose
//! attachments keep one class, or a single target, must keep the key it has
//! today.
//!
//! `current_blend_key` is private, so the observation is the variant record
//! the back end's `draw` receives: `GlesDraw::fs` is the program's base
//! shader (blend key 0) when the draw needs no variant, else the variant the
//! shader back end built for the key, whose `ShcShader::blend_key` is the key
//! asked for. The record also gives `num_outputs`, and the draw gives the
//! formats of the targets it writes. Each case draws a triangle through the
//! GL API on the host into an FBO with the given renderbuffers and draw
//! buffers, in the PowerVR online configuration (HOST-ONLY: no GPU runs).
//!
//! The expected keys come from the public builders of `yakogl::shaderc`
//! (`shc_blend_key`, `shc_blend_key_target`, `shc_blend_key_int_rt`,
//! `shc_blend_key_mask_off`, `SHC_BLEND_KEY_SRGB_ENCODED`), not from numbers.
//! Mixes of an integer target with another class are left out on purpose:
//! their key changes with the mixed per-target key.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test gl_blend_key_pin
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend, GLES_SURF_R32UI, GLES_SURF_RGBA8888};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::{
    shc_blend_key, shc_blend_key_int_rt, shc_blend_key_mask_off, shc_blend_key_target, ShcIntRt, SHC_BF_INV_SRC_ALPHA,
    SHC_BF_SRC_ALPHA, SHC_BLEND_ADD, SHC_BLEND_KEY_REPLACE, SHC_BLEND_KEY_SRGB_ENCODED, SHC_RT_RGBA8, SHC_RT_SRGB,
};

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

/// What the last draw handed the back end: the variant record of its
/// fragment shader and the formats of the targets it writes.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Drawn {
    blend_key: u32,
    num_outputs: u8,
    formats: Vec<u8>,
}

static DRAWN: Mutex<Option<Drawn>> = Mutex::new(None);

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let fs = d.fs.as_ref().expect("a draw carries its fragment shader");
    let n = usize::from(d.num_targets.max(1));
    let formats = d.targets[..n].iter().map(|t| t.format).collect();
    *DRAWN.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Drawn { blend_key: fs.blend_key, num_outputs: fs.num_outputs, formats });
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
    name: c"gl-blend-key-pin".as_ptr() as *const c_char,
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
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(64, 64).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glUseProgram(0);
        glDisable(GL_BLEND);
        glColorMask(1, 1, 1, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

/* ---- the draw ---------------------------------------------------------- */

const VS: &str = "#version 300 es\nprecision highp float;\nin vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n";

/// A fragment shader writing one output per target slot: `Some(true)` an
/// unsigned integer one, `Some(false)` a float one, `None` no output (a
/// GL_NONE slot).
fn fs_for(slots: &[Option<bool>]) -> String {
    let mut s = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
    for (i, o) in slots.iter().enumerate() {
        match o {
            Some(true) => s += &format!("layout(location = {i}) out uvec4 o{i};\n"),
            Some(false) => s += &format!("layout(location = {i}) out vec4 o{i};\n"),
            None => {}
        }
    }
    s += "void main() {\n";
    for (i, o) in slots.iter().enumerate() {
        match o {
            Some(true) => s += &format!("    o{i} = uvec4({}u);\n", i + 1),
            Some(false) => s += &format!("    o{i} = vec4(0.25, 0.5, 0.75, 0.5);\n"),
            None => {}
        }
    }
    s + "}\n"
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source and a log buffer.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0 as c_char; 1024];
            glGetShaderInfoLog(s, 1024, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("compile: {}", CStr::from_ptr(log.as_ptr()).to_string_lossy());
        }
        s
    }
}

fn program(slots: &[Option<bool>]) -> GLuint {
    let vs = shader(GL_VERTEX_SHADER, VS);
    let fs = shader(GL_FRAGMENT_SHADER, &fs_for(slots));
    // SAFETY: plain GL calls on handles this test owns.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, vs);
        glAttachShader(p, fs);
        glLinkProgram(p);
        glDeleteShader(vs);
        glDeleteShader(fs);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0 as c_char; 2048];
            glGetProgramInfoLog(p, 2048, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("link: {}", CStr::from_ptr(log.as_ptr()).to_string_lossy());
        }
        p
    }
}

/// The settings a case draws under.
#[derive(Clone, Copy, Default)]
struct State {
    /// glBlendFunc(SRC_ALPHA, ONE_MINUS_SRC_ALPHA) with GL_BLEND on.
    blend: bool,
    /// glColorMask(1, 0, 1, 1): green off.
    mask_green_off: bool,
}

/// Draw one triangle into an FBO whose attachment `i` is a renderbuffer of
/// `formats[i]` (0: none; the draw buffer is GL_NONE) and whose fragment
/// shader writes the matching output; the record the back end received.
fn draw(formats: &[GLenum], st: State) -> Drawn {
    let slots: Vec<Option<bool>> = formats
        .iter()
        .map(|&f| match f {
            0 => None,
            GL_R32UI => Some(true),
            _ => Some(false),
        })
        .collect();
    let p = program(&slots);
    static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0];
    let bufs: Vec<GLenum> = formats
        .iter()
        .enumerate()
        .map(|(i, &f)| if f == 0 { GL_NONE } else { GL_COLOR_ATTACHMENT0 + i as u32 })
        .collect();
    // SAFETY: plain GL calls on objects of this context; every pointer stays
    // alive through the draw.
    unsafe {
        let (mut fbo, mut rbs) = (0, vec![0; formats.len()]);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glGenRenderbuffers(rbs.len() as i32, rbs.as_mut_ptr());
        for (i, &f) in formats.iter().enumerate() {
            if f == 0 {
                continue;
            }
            glBindRenderbuffer(GL_RENDERBUFFER, rbs[i]);
            glRenderbufferStorage(GL_RENDERBUFFER, f, 8, 8);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as u32, GL_RENDERBUFFER, rbs[i]);
        }
        glDrawBuffers(bufs.len() as i32, bufs.as_ptr());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        if st.blend {
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        }
        if st.mask_green_off {
            glColorMask(1, 0, 1, 1);
        }
        glViewport(0, 0, 8, 8);
        glUseProgram(p);
        let a = glGetAttribLocation(p, c"a_pos".as_ptr());
        assert!(a >= 0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(a as u32, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(a as u32);
        assert_eq!(glGetError(), GL_NO_ERROR, "the setup");
        *DRAWN.lock().unwrap_or_else(|e| e.into_inner()) = None;
        glDrawArrays(GL_TRIANGLES, 0, 3);
        assert_eq!(glGetError(), GL_NO_ERROR, "the draw");
        glDisableVertexAttribArray(a as u32);
        glUseProgram(0);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteRenderbuffers(rbs.len() as i32, rbs.as_ptr());
        glDeleteProgram(p);
    }
    DRAWN.lock().unwrap_or_else(|e| e.into_inner()).take().expect("the back end drew")
}

/* ---- the keys the builders give ------------------------------------------ */

const R32UI: ShcIntRt = ShcIntRt { channels: 1, bits: 32, signed: false };

fn blend_key() -> u32 {
    shc_blend_key(
        SHC_BF_SRC_ALPHA,
        SHC_BF_INV_SRC_ALPHA,
        SHC_BF_SRC_ALPHA,
        SHC_BF_INV_SRC_ALPHA,
        SHC_BLEND_ADD,
        SHC_BLEND_ADD,
    )
}

/// A draw that needs no variant (the plain replace into RGBA8) hands the back
/// end the program's own shader, blend key 0.
const NO_VARIANT: u32 = 0;

#[test]
fn one_rgba8_target_replace_has_no_variant() {
    let _g = context();
    let d = draw(&[GL_RGBA8], State::default());
    assert_eq!(shc_blend_key_target(SHC_RT_RGBA8, 0), NO_VARIANT, "the builder's replace into RGBA8");
    assert_eq!(d.blend_key, NO_VARIANT);
    assert_eq!(d.num_outputs, 1);
    assert_eq!(d.formats, [GLES_SURF_RGBA8888]);
}

#[test]
fn one_rgba8_target_with_a_blend_keys_the_blend() {
    let _g = context();
    let d = draw(&[GL_RGBA8], State { blend: true, ..State::default() });
    assert_eq!(d.blend_key, blend_key() | shc_blend_key_target(SHC_RT_RGBA8, 0));
    assert_ne!(d.blend_key, NO_VARIANT);
    assert_eq!(d.num_outputs, 1);
    assert_eq!(d.formats, [GLES_SURF_RGBA8888]);
}

#[test]
fn one_rgba8_target_with_a_partial_mask_keys_the_mask() {
    let _g = context();
    let d = draw(&[GL_RGBA8], State { mask_green_off: true, ..State::default() });
    /* the replace's own bits (ONE/ZERO ADD) with the green channel's mask */
    assert_eq!(d.blend_key, SHC_BLEND_KEY_REPLACE | shc_blend_key_target(SHC_RT_RGBA8, 0b0010));
    assert_eq!(d.blend_key & shc_blend_key_mask_off(0b0010), shc_blend_key_mask_off(0b0010));
    assert_eq!(d.num_outputs, 1);
    assert_eq!(d.formats, [GLES_SURF_RGBA8888]);
}

#[test]
fn one_r32ui_target_keys_its_integer_layout() {
    let _g = context();
    let d = draw(&[GL_R32UI], State::default());
    assert_eq!(d.blend_key, shc_blend_key_int_rt(R32UI));
    assert_eq!(ShcIntRt::of_key(d.blend_key), Some(R32UI));
    assert_eq!(d.num_outputs, 1);
    assert_eq!(d.formats, [GLES_SURF_R32UI]);
}

#[test]
fn two_rgba8_targets_key_as_one() {
    let _g = context();
    let d = draw(&[GL_RGBA8, GL_RGBA8], State::default());
    assert_eq!(d.blend_key, NO_VARIANT);
    assert_eq!(d.num_outputs, 2);
    assert_eq!(d.formats, [GLES_SURF_RGBA8888, GLES_SURF_RGBA8888]);
}

#[test]
fn two_srgb_targets_key_the_encoded_rgba8_variant() {
    let _g = context();
    let d = draw(&[GL_SRGB8_ALPHA8, GL_SRGB8_ALPHA8], State::default());
    assert_eq!(d.blend_key, SHC_BLEND_KEY_SRGB_ENCODED);
    assert_eq!(d.blend_key, shc_blend_key_target(SHC_RT_SRGB, 0));
    assert_eq!(d.num_outputs, 2);
    assert_eq!(d.formats.len(), 2);
    assert_eq!(d.formats[0], d.formats[1]);
}

#[test]
fn two_r32ui_targets_key_the_integer_layout_once() {
    let _g = context();
    let d = draw(&[GL_R32UI, GL_R32UI], State::default());
    assert_eq!(d.blend_key, shc_blend_key_int_rt(R32UI));
    assert_eq!(d.num_outputs, 2);
    assert_eq!(d.formats, [GLES_SURF_R32UI, GLES_SURF_R32UI]);
}

#[test]
fn rgba8_none_rgba8_keys_as_one_rgba8() {
    let _g = context();
    let d = draw(&[GL_RGBA8, 0, GL_RGBA8], State::default());
    assert_eq!(d.blend_key, NO_VARIANT);
    /* the outputs are counted to the last slot written: location 2 */
    assert_eq!(d.num_outputs, 3);
    assert_eq!(d.formats[0], GLES_SURF_RGBA8888);
}
