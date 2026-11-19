// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GL core's per-target key list (`gl::target_keys`, from a context's
//! draw buffers) and the PowerVR back end's (`pvr::target_keys`, from a
//! `GlesDraw`) for the same attachments: the GL side gives `Some` exactly
//! where the back end gives `Ok`, with equal key lists and mixed key, and a
//! GL `None` is one of the back end's named refusals
//! (docs/notes/pvr-int-mixed-targets.md, sections 4 and 5).
//!
//! The GL side is a real context: renderbuffers of the formats attached to an
//! FBO, `glDrawBuffers` set (the minimal host OSAL and back end of
//! `gl_blend_key_pin.rs`; no draw is issued, the keys are read from the
//! context). The back-end side is a `GlesDraw` built field by field from the
//! `GLES_SURF_*` format each GL format maps to, as `pvr::target_keys`' own
//! unit tests do. The limit: the integration test cannot run
//! `pvr_backend_draw`, so the back end's recomputation in `backend.rs` is not
//! exercised, only the functions it calls.
//!
//! One difference is pinned, not resolved: `gl::target_keys::draw_target_keys`
//! does not check the layout, so for a list no end-of-tile program emits
//! (RGBA8 + RGBA32UI: two targets, the second spilled) the GL list is `Some`
//! while the back end refuses with `REFUSE_FOUR_REGISTERS`. The GL mixed key
//! is `None` there (`shc_blend_key_mixed` rejects the list), which agrees with
//! the refusal. A list whose third target spills to a tile buffer
//! (`target_key::shc_mixed_spans`, plan #325 item mrt-frameload) is a mix both
//! sides agree on.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test mixed_target_keys_parity
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::context::GlContext;
use yakogl::gl::framebuffer::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::pvr::target_keys as pvr_keys;

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
    name: c"mixed-target-keys-parity".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: None,
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

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static CTX: Mutex<usize> = Mutex::new(0);

/// The one context of the file, serialised by the guard.
fn context() -> (MutexGuard<'static, ()>, &'static GlContext) {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            let c = yakogl::gl::context::gl_context_create(64, 64);
            assert!(!c.is_null());
            /* R32F as GL_EXT_color_buffer_float makes it (unlisted in the build) */
            yakogl::gl::framebuffer::HOST_R32F_RT.store(true, core::sync::atomic::Ordering::Relaxed);
            *CTX.lock().unwrap() = c as usize;
        }
    });
    let p = *CTX.lock().unwrap() as *const GlContext;
    // SAFETY: the context lives for the process; the lock serialises users.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        (g, &*p)
    }
}

/// The back end's surface format of a GL colour format.
fn surf(f: u32) -> u8 {
    match f {
        GL_RGBA8 => GLES_SURF_RGBA8888,
        GL_RGBA16F => GLES_SURF_RGBA16F,
        GL_R32F => GLES_SURF_R32F,
        GL_R32UI => GLES_SURF_R32UI,
        GL_R32I => GLES_SURF_R32I,
        GL_RG32UI => GLES_SURF_RG32UI,
        GL_RGBA32UI => GLES_SURF_RGBA32UI,
        GL_R16UI => GLES_SURF_R16UI,
        GL_RGBA16UI => GLES_SURF_RGBA16UI,
        GL_SRGB8_ALPHA8 => GLES_SURF_SRGB8_A8,
        _ => panic!("no surface format for {f:#x}"),
    }
}

type GlKeys = (Option<([u32; 3], usize)>, Option<u32>);

/// The GL side: an FBO with a renderbuffer of `formats[i]` at attachment `i`
/// (`GL_NONE`: none), every present one a draw buffer, and the keys of it.
fn gl_side(formats: &[u32]) -> GlKeys {
    let (_g, c) = context();
    let bufs: Vec<u32> = formats
        .iter()
        .enumerate()
        .map(|(i, &f)| if f == GL_NONE { GL_NONE } else { GL_COLOR_ATTACHMENT0 + i as u32 })
        .collect();
    let mut fb = 0;
    // SAFETY: plain GL calls on the one context, room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for (i, &f) in formats.iter().enumerate() {
            if f == GL_NONE {
                continue;
            }
            let mut rb = 0;
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, f, 4, 4);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as u32, GL_RENDERBUFFER, rb);
        }
        glDrawBuffers(bufs.len() as i32, bufs.as_ptr());
    }
    let r = (yakogl::gl::target_keys::draw_target_keys(c), yakogl::gl::target_keys::draw_mixed_key(c));
    // SAFETY: as above.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
    }
    r
}

/// The back end's draw of the same attachments (`gpu` 0 for `GL_NONE`).
fn draw_of(formats: &[u32]) -> GlesDraw {
    let mut d = GlesDraw::ZERO;
    d.num_targets = formats.len() as u8;
    for (i, &f) in formats.iter().enumerate() {
        let (gpu, format) = if f == GL_NONE { (0, 0) } else { (0x1000 * (i as u64 + 1), surf(f)) };
        d.targets[i] = GlesDrawTarget { gpu, w: 4, h: 4, format, out_shift: 0 };
    }
    d.target_gpu = d.targets[0].gpu;
    d.target_format = d.targets[0].format;
    d
}

/// A list the two sides both accept: equal lists and equal mixed key, which
/// is returned.
fn agree(formats: &[u32]) -> u32 {
    let (gl_list, gl_mixed) = gl_side(formats);
    let d = draw_of(formats);
    let (k, n) = pvr_keys::draw_target_keys(&d).expect("the back end accepts the list");
    let (gk, gn) = gl_list.expect("the GL side gives a list");
    assert_eq!((gn, &gk[..gn]), (n, &k[..n]), "{formats:x?}");
    assert_eq!(pvr_keys::draw_mixed_key(&d).ok(), gl_mixed, "{formats:x?}");
    gl_mixed.expect("a mix")
}

#[test]
fn rgba8_beside_r32ui_and_back() {
    let a = agree(&[GL_RGBA8, GL_R32UI]);
    let b = agree(&[GL_R32UI, GL_RGBA8]);
    assert_ne!(a, b, "the order of the targets is part of the key");
}

#[test]
fn the_modf_executor_pair() {
    agree(&[GL_R32F, GL_R32I]);
    agree(&[GL_R32F, GL_R32UI]);
}

#[test]
fn two_integer_layouts() {
    agree(&[GL_R32UI, GL_R32I]);
    agree(&[GL_R32UI, GL_RG32UI]);
}

#[test]
fn a_three_target_mix() {
    agree(&[GL_RGBA8, GL_R32UI, GL_R32I]);
    agree(&[GL_R32UI, GL_RGBA8, GL_RGBA16F]);
}

#[test]
fn not_a_mix_agrees_on_no_key() {
    let lists: [&[u32]; 5] = [
        &[GL_R32UI],
        &[GL_R32UI, GL_R32UI],
        &[GL_R32UI, GL_R32UI, GL_R32UI],
        &[GL_RGBA8, GL_RGBA8],
        &[GL_RGBA8, GL_RGBA16F],
    ];
    for f in lists {
        let (gl_list, gl_mixed) = gl_side(f);
        let d = draw_of(f);
        assert_eq!(gl_mixed, None, "{f:x?}");
        assert_eq!(pvr_keys::draw_mixed_key(&d), Err(pvr_keys::REFUSE_NOT_A_MIX), "{f:x?}");
        // both still list the targets
        let (k, n) = pvr_keys::draw_target_keys(&d).unwrap();
        let (gk, gn) = gl_list.unwrap();
        assert_eq!((gn, &gk[..gn]), (n, &k[..n]), "{f:x?}");
    }
}

#[test]
fn a_gl_none_gap_is_refused_by_name() {
    let f = [GL_RGBA8, GL_NONE, GL_R32UI];
    assert_eq!(gl_side(&f), (None, None));
    assert_eq!(pvr_keys::draw_target_keys(&draw_of(&f)), Err(pvr_keys::REFUSE_GL_NONE));
}

#[test]
fn srgb_beside_integer_is_refused_by_name() {
    for f in [[GL_SRGB8_ALPHA8, GL_R32UI], [GL_R32UI, GL_SRGB8_ALPHA8]] {
        assert_eq!(gl_side(&f), (None, None));
        assert_eq!(pvr_keys::draw_target_keys(&draw_of(&f)), Err(pvr_keys::REFUSE_SRGB));
    }
}

#[test]
fn past_four_registers_is_refused_by_name() {
    // RGBA8 (one register) + RGBA32UI (four): the GL list stays Some (the
    // pinned difference of the header) but there is no mixed key.
    let f = [GL_RGBA8, GL_RGBA32UI];
    let (gl_list, gl_mixed) = gl_side(&f);
    assert_eq!(gl_list.map(|l| l.1), Some(2));
    assert_eq!(gl_mixed, None);
    assert_eq!(pvr_keys::draw_target_keys(&draw_of(&f)), Err(pvr_keys::REFUSE_FOUR_REGISTERS));
    // three RGBA16F: two registers each, the third spilled to a tile
    // buffer; listed, but no mix (no integer target)
    let f = [GL_RGBA16F, GL_RGBA16F, GL_RGBA16F];
    assert_eq!(gl_side(&f).1, None);
    assert_eq!(pvr_keys::draw_target_keys(&draw_of(&f)).map(|l| l.1), Ok(3));
    assert_eq!(pvr_keys::draw_mixed_key(&draw_of(&f)), Err(pvr_keys::REFUSE_NOT_A_MIX));
}

#[test]
fn a_spilled_third_target_is_a_mix_both_sides_agree_on() {
    // R16UI (r0) + RGBA16UI (r2-r3) + RGBA8 (tile buffer 0):
    // fragment_out.random.50's list
    agree(&[GL_R16UI, GL_RGBA16UI, GL_RGBA8]);
    agree(&[GL_RGBA8, GL_R32UI, GL_RGBA32UI]);
}
