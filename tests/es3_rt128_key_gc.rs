// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the GL core's render-target plane key per draw for
//! 128-bit colour attachments (`rt_planes_key`, `wide128_select` and
//! `w128_variant_for`, src/gl/draw.rs; docs/notes/gc-128bit-rt-shader.md
//! section 2 and section 5 item 3).
//!
//! A draw into an RGBA32I/UI attachment needs its fragment shader compiled
//! with the colour output split into two plane stores
//! (`shaderc::rt_planes_variant`). The key is bit `n` for each colour output
//! `n` whose target is such an attachment (target `k` is output
//! `k + out_shift`); the vertex stage has none. It lives in the same per-
//! program variant cache as the sampler plane key (`ProgW128Variant::rt`),
//! two slots, the least recently used one evicted.
//!
//! The key is inert: `rt_format_drawable` and `refuse_undrawable_targets`
//! still refuse an RGBA32I/UI attachment before the draw asks for it (pinned
//! here). The scenarios set the key through the host-test hook
//! `set_rt_planes_test_targets` (the draw targets it names count as 128-bit
//! for the key alone, the RGBA8 targets are drawn as RGBA8) and read it back
//! with `rt_planes_draw_state`.
//!
//! What the compiler does with the key today: a keyed output the shader
//! stores is refused until its second plane has a register (gc-rt128-codegen),
//! so a keyed draw of a shader writing that output is refused by name; a key
//! naming only outputs the shader does not write compiles (a distinct record,
//! the module untouched) and is drawn. `note_draw_refused` logs its text for
//! a process's first refusal only, so the scenarios that pin a text run alone
//! in a child process (`YAKOGL_RT128_KEY_CHILD`).
//!
//! A fragment stage with a sampler plane key and a render-target key at once
//! (an RGBA32UI texture sampled into a 128-bit target) is one slot holding
//! both keys, refused by name: no compiler entry joins them yet.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_rt128_key_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::sync::Mutex;
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::online::vivante::wide128_rt::RtPlanes;
use yakogl::shaderc::Wide128Planes;
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};

/* ---- the stand-in OSAL (as tests/gc_tiled_copy_reuse.rs) ---------------- */

struct Alloc {
    mem: Box<[u64]>,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    faults: Vec<String>,
}

/// The OSAL log lines, in order.
static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer, the back end and `g_backend` are process-global.
static SERIAL: Mutex<()> = Mutex::new(());

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
        p.allocs.insert(gpu, Alloc { mem });
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        if !m.impl_.is_null() && p.allocs.remove(&m.gpu).is_none() {
            p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu));
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        with_platform(|p| p.faults.push(format!("submit of a {len}-byte descriptor")));
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            p.faults.push(format!("submit with magic 0x{:x}", k.magic));
            return -22;
        }
        let Some(a) = p.allocs.get(&k.stream_gpu) else {
            p.faults.push(format!("the kick's stream 0x{:x} is no allocation", k.stream_gpu));
            return -22;
        };
        // SAFETY: the allocation holds at least `stream_bytes` bytes.
        let words: &[u32] =
            unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
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

fn on_gc7000(case: impl FnOnce()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(osal());
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
    let faults = with_platform(|p| {
        /* leaked, not freed, as in gc_npot_repeat.rs: addresses keep counting up */
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        core::mem::take(&mut p.faults)
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
}

/* ---- GL plumbing -------------------------------------------------------- */

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
        if ok != 1 {
            let mut log = [0 as c_char; 512];
            glGetShaderInfoLog(s, 512, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("{text:?} did not compile: {}", CStr::from_ptr(log.as_ptr()).to_string_lossy());
        }
        s
    }
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

type LogFn = unsafe extern "C" fn(*const c_char, ...);

/// The OSAL's log hook: the library only ever calls it with ("%s", line)
/// (`osal::log_str`), so a two-argument function stands in for the variadic.
unsafe extern "C" fn p_log(_fmt: *const c_char, line: *const c_char) {
    // SAFETY: `line` is the NUL-terminated string `log_str` passes.
    let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push(s);
}

fn log_lines() -> Vec<String> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn osal() -> &'static OsalOps {
    // SAFETY: see p_log: the library only ever calls it with ("%s", line).
    let log = unsafe { core::mem::transmute::<unsafe extern "C" fn(*const c_char, *const c_char), LogFn>(p_log) };
    Box::leak(Box::new(OsalOps {
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
        log: Some(log),
        wait_vblank: None,
        present: None,
        lock: None,
        unlock: None,
        thread_id: None,
        display_busy: None,
    }))
}

/* ---- the scenarios ------------------------------------------------------ */

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
/// Writes colour output 0 only.
const FS_0: &str =
    "#version 300 es\nprecision highp float;\nout vec4 o;\nvoid main() { o = vec4(0.25, 0.5, 0.75, 1.0); }\n";
/// Reads gl_FrontFacing: the GC7000's front-facing twin under the winding
/// that selects it.
const FS_FACING: &str =
    "#version 300 es\nprecision highp float;\nout vec4 o;\nvoid main() { o = gl_FrontFacing ? vec4(1.0) : vec4(0.0); }\n";
/// Samples an unsigned-integer texture on binding 0 and writes output 0.
const FS_FETCH: &str = "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp usampler2D;\nuniform usampler2D s;\nout vec4 o;\nvoid main() { o = vec4(texelFetch(s, ivec2(0), 0)); }\n";
/// Writes colour output 1 only.
const FS_1: &str =
    "#version 300 es\nprecision highp float;\nlayout(location = 1) out vec4 o;\nvoid main() { o = vec4(1.0); }\n";
/// Writes an unsigned-integer output 0.
const FS_UINT: &str = "#version 300 es\nprecision highp int;\nout uvec4 o;\nvoid main() { o = uvec4(1u); }\n";

fn program(vs: &str, fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

/// A framebuffer of `formats.len()` 16x16 colour renderbuffers, attachment
/// `i` of `formats[i]`, with the draw buffers `bufs`; bound. Returns whether
/// it is complete.
fn framebuffer(formats: &[GLenum], bufs: &[GLenum]) -> bool {
    // SAFETY: plain GL calls; the arrays outlive them.
    unsafe {
        let mut fb: GLuint = 0;
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for (i, &f) in formats.iter().enumerate() {
            let mut rb: GLuint = 0;
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, f, 16, 16);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb);
        }
        glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        no_error("the framebuffer");
        glCheckFramebufferStatus(GL_FRAMEBUFFER) == GL_FRAMEBUFFER_COMPLETE
    }
}

/// What one quad did: the kicks it submitted (the last one's words), the
/// draws refused meanwhile, and the GL error after it.
struct Drew {
    kicks: usize,
    last: Vec<u32>,
    refused: u32,
    error: GLenum,
}

fn draw(p: GLuint) -> Drew {
    let (k0, r0) = (with_platform(|p| p.kicks.len()), draws_refused());
    // SAFETY: plain GL calls; QUAD is a static.
    let error = unsafe {
        glViewport(0, 0, 16, 16);
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        let e = glGetError();
        glFinish();
        e
    };
    no_error("glFinish");
    let (kicks, last) = with_platform(|p| (p.kicks.len() - k0, p.kicks.last().cloned().unwrap_or_default()));
    Drew { kicks, last, refused: draws_refused().wrapping_sub(r0), error }
}

fn drew(what: &str, d: &Drew) {
    assert_eq!(d.error, GL_NO_ERROR, "{what}: GL error 0x{:x}", d.error);
    assert_eq!(d.refused, 0, "{what}: refused");
    assert!(d.kicks >= 1, "{what}: no kick");
}

fn refused(what: &str, d: &Drew) {
    assert_eq!(d.error, GL_NO_ERROR, "{what}: a refused draw is no GL error (0x{:x})", d.error);
    assert_eq!(d.refused, 1, "{what}: counted once");
    assert_eq!(d.kicks, 0, "{what}: no draw reaches the stream");
}

/// Whether the draw's kick names the USC at `gpu` (the GC7000 states the
/// fragment shader's instruction address as a 32-bit word).
fn names_usc(d: &Drew, gpu: u64) -> bool {
    gpu != 0 && d.last.contains(&(gpu as u32))
}

/// The text `note_draw_refused` logged for the process's first refusal.
fn first_refusal_text() -> String {
    let lines = log_lines();
    let l = lines
        .iter()
        .find(|l| l.contains("a draw was refused: "))
        .unwrap_or_else(|| panic!("no refusal logged: {lines:?}"));
    let rest = l.split("a draw was refused: ").nth(1).unwrap();
    rest.split(" (key 0x").next().unwrap().to_string()
}

fn state(p: GLuint) -> RtPlanesDrawState {
    rt_planes_draw_state(p).expect("a draw asked for the render-target key")
}

fn used(s: &RtPlanesDrawState) -> Vec<W128SlotState> {
    s.slots.iter().flatten().copied().collect()
}

fn rt(mask: u8) -> RtPlanes {
    RtPlanes { mask }
}

/// Run the case on the stand-in OSAL with the Vivante back end up, the
/// test hook cleared before and after.
fn on_gc(case: impl FnOnce()) {
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            set_rt_planes_test_targets(0);
        }
    }
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
    set_rt_planes_test_targets(0);
    let _clear = Clear;
    on_gc7000(case);
}

/// The first refusal of a process is the only one whose text is logged, so
/// a scenario that pins its text runs alone in a child process of this
/// binary: true in the child (run the body); in the parent this runs the
/// child, checks it passed, and returns false.
fn in_child(name: &str) -> bool {
    const VAR: &str = "YAKOGL_RT128_KEY_CHILD";
    if std::env::var(VAR).as_deref() == Ok(name) {
        return true;
    }
    let out = std::process::Command::new(std::env::current_exe().expect("the test binary"))
        .args(["--exact", name, "--test-threads=1", "--nocapture"])
        .env(VAR, name)
        .output()
        .expect("the child runs");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{name} in a fresh process failed:\n{text}");
    assert!(text.contains("1 passed"), "{name} did not run in the child:\n{text}");
    if text.contains("only the control ran") {
        eprintln!("{name}: RGBA32UI is not in GC_SAMPLED_TEXFMTS; only the control ran (gc-w128-flip)");
    }
    false
}

/// Without the hook, a draw into RGBA8 asks for the key and gets the empty
/// one: no slot, the program's own fragment shader. Before any draw (and
/// after a relink) there is nothing to read.
#[test]
fn an_rgba8_draw_has_the_empty_key_and_no_slot() {
    on_gc(|| {
        let p = program(VS, FS_0);
        assert_eq!(rt_planes_draw_state(p), None, "no draw asked yet");
        let d = draw(p);
        drew("RGBA8", &d);
        let s = state(p);
        assert_eq!(s.last, [RtPlanes::NONE; 2]);
        assert!(used(&s).is_empty(), "nothing compiled");
        assert_eq!(s.evicted, 0);
        assert!(names_usc(&d, s.own_fs_usc_gpu), "the program's own fragment shader");
        // SAFETY: plain GL call.
        unsafe { glLinkProgram(p) };
        assert_eq!(rt_planes_draw_state(p), None, "a link drops the cache");
    });
}

/// Inert: a real RGBA32UI or RGBA32I attachment is still refused before the
/// draw asks for the key (`refuse_undrawable_targets`, GL_INVALID_OPERATION).
#[test]
fn a_real_rgba32_attachment_is_still_refused_before_the_key() {
    on_gc(|| {
        for f in [GL_RGBA32UI, GL_RGBA32I] {
            let p = program(VS, FS_UINT);
            assert!(framebuffer(&[f], &[GL_COLOR_ATTACHMENT0]), "0x{f:x}: complete");
            let k0 = with_platform(|p| p.kicks.len());
            // SAFETY: plain GL calls; QUAD is a static.
            let e = unsafe {
                glViewport(0, 0, 16, 16);
                glUseProgram(p);
                glEnableVertexAttribArray(0);
                glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
                glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
                let e = glGetError();
                glFinish();
                e
            };
            assert_eq!(e, GL_INVALID_OPERATION, "0x{f:x}: refused as an undrawable target");
            assert_eq!(with_platform(|p| p.kicks.len()), k0, "0x{f:x}: no kick");
            assert_eq!(rt_planes_draw_state(p), None, "0x{f:x}: the key was never asked");
            // SAFETY: plain GL call.
            unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
        }
    });
}

/// Target 0 counted as 128-bit: the fragment key is { mask: 1 }, the vertex
/// key empty; the shader stores output 0, which the compiler refuses until
/// its second plane has a register, so the draw is refused by that name and
/// the slot remembers it. A draw to RGBA8 then has the empty key and draws
/// the program's own shader with the slot kept; the key again finds the slot
/// (no compile) and is refused by the slot's name.
#[test]
fn a_keyed_target_gives_the_fragment_key_and_the_slot_is_reused() {
    if !in_child("a_keyed_target_gives_the_fragment_key_and_the_slot_is_reused") {
        return;
    }
    on_gc(|| {
        let p = program(VS, FS_0);
        set_rt_planes_test_targets(1);
        let d = draw(p);
        refused("output 0 keyed", &d);
        assert_eq!(first_refusal_text(), "the fragment shader does not compile");
        let s = state(p);
        assert_eq!(s.last, [RtPlanes::NONE, rt(1)], "the fragment stage alone");
        let slots = used(&s);
        assert_eq!(slots.len(), 1, "one slot");
        assert_eq!((slots[0].planes, slots[0].rt), ([Wide128Planes::NONE; 2], rt(1)));
        assert!(slots[0].missing, "remembered: the compiler refused the key");
        assert!(log_lines().iter().any(|l| l.contains("targets 0x1")), "the variant's log names the targets");

        set_rt_planes_test_targets(0);
        let d = draw(p);
        drew("RGBA8 after the key", &d);
        let s = state(p);
        assert_eq!(s.last, [RtPlanes::NONE; 2]);
        assert!(names_usc(&d, s.own_fs_usc_gpu), "the program's own fragment shader");
        assert_eq!(used(&s), slots, "the slot is kept, not the plain shader replaced");

        set_rt_planes_test_targets(1);
        let d = draw(p);
        refused("the key again", &d);
        let s = state(p);
        assert_eq!((used(&s), s.evicted), (slots, 0), "the same slot, nothing compiled again");
    });
}

/// A key naming an output the shader does not write compiles (the pass
/// leaves the module as it is, under its own record) and is drawn: the
/// variant's fragment USC, not the program's; the next draw to plain targets
/// draws the program's own. Target 1 of two RGBA8 attachments, the shader
/// writing output 0 only.
#[test]
fn a_key_on_an_unwritten_output_draws_its_variant() {
    on_gc(|| {
        assert!(framebuffer(&[GL_RGBA8, GL_RGBA8], &[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1]), "complete");
        let p = program(VS, FS_0);
        set_rt_planes_test_targets(0b10);
        let d = draw(p);
        drew("output 1 keyed, not written", &d);
        let s = state(p);
        assert_eq!(s.last, [RtPlanes::NONE, rt(0b10)]);
        let slots = used(&s);
        assert_eq!(slots.len(), 1);
        assert!(!slots[0].missing, "compiled, linked and uploaded");
        assert_ne!(slots[0].fs_usc_gpu, s.own_fs_usc_gpu);
        assert!(names_usc(&d, slots[0].fs_usc_gpu), "the variant is drawn");
        assert!(!names_usc(&d, s.own_fs_usc_gpu), "not the program's own");

        set_rt_planes_test_targets(0);
        let d = draw(p);
        drew("plain targets", &d);
        let s = state(p);
        assert!(names_usc(&d, s.own_fs_usc_gpu), "the program's own");
        assert!(!names_usc(&d, slots[0].fs_usc_gpu), "not the variant");
        assert_eq!(used(&s), slots, "the variant kept");

        set_rt_planes_test_targets(0b10);
        let d = draw(p);
        drew("the key again", &d);
        assert!(names_usc(&d, slots[0].fs_usc_gpu), "the same upload");
        assert_eq!(state(p).evicted, 0);
    });
}

/// Target `k` is colour output `k + out_shift`: with draw buffers
/// (NONE, ATTACHMENT1) the draw's target 0 is output 1, so the key is bit 1.
#[test]
fn the_key_follows_the_leading_draw_buffers_dropped() {
    on_gc(|| {
        assert!(framebuffer(&[GL_RGBA8, GL_RGBA8], &[GL_NONE, GL_COLOR_ATTACHMENT1]), "complete");
        let p = program(VS, FS_1);
        set_rt_planes_test_targets(1);
        let _ = draw(p);
        assert_eq!(state(p).last, [RtPlanes::NONE, rt(0b10)], "target 0 is output 1");
    });
}

/// Two slots: three keys alternated evict the least recently used one,
/// counted; a key used again is found, not compiled.
#[test]
fn the_third_alternation_evicts_the_oldest() {
    on_gc(|| {
        assert!(framebuffer(&[GL_RGBA8, GL_RGBA8], &[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1]), "complete");
        let p = program(VS, FS_0);
        let keys = |p: GLuint| {
            let mut k: Vec<u8> = used(&state(p)).iter().map(|v| v.rt.mask).collect();
            k.sort();
            k
        };
        for m in [0b01, 0b10] {
            set_rt_planes_test_targets(m);
            let _ = draw(p);
        }
        assert_eq!((keys(p), state(p).evicted), (vec![0b01, 0b10], 0), "two keys, two slots");
        set_rt_planes_test_targets(0b11);
        let _ = draw(p);
        assert_eq!((keys(p), state(p).evicted), (vec![0b10, 0b11], 1), "the third evicts the oldest (0b01)");
        set_rt_planes_test_targets(0b10);
        let _ = draw(p);
        assert_eq!(state(p).evicted, 1, "a held key is found");
        set_rt_planes_test_targets(0b01);
        let _ = draw(p);
        assert_eq!((keys(p), state(p).evicted), (vec![0b01, 0b10], 2), "0b11, now the oldest, goes");
    });
}

/// A blend-variant fragment shader (the GC7000's front-facing twin) cannot
/// also take the render-target key: refused by name. The twin alone draws.
#[test]
fn a_blend_variant_with_a_target_key_is_refused_by_name() {
    if !in_child("a_blend_variant_with_a_target_key_is_refused_by_name") {
        return;
    }
    on_gc(|| {
        let p = program(VS, FS_FACING);
        // SAFETY: plain GL call.
        unsafe { glFrontFace(GL_CCW) }; /* the default surface is stored top-down: CCW takes the twin */
        drew("the twin alone", &draw(p));
        set_rt_planes_test_targets(1);
        let d = draw(p);
        refused("the twin and a target key", &d);
        assert_eq!(
            first_refusal_text(),
            "its fragment shader needs a front-facing twin and a 128-bit render-target key at once"
        );
        assert_eq!(state(p).last, [RtPlanes::NONE, rt(1)]);
        assert!(used(&state(p)).is_empty(), "nothing compiled");
        set_rt_planes_test_targets(0);
        drew("the twin again", &draw(p));
    });
}

fn bind_uint_texture(internal: GLenum) {
    let ty = if internal == GL_RGBA32UI { GL_UNSIGNED_INT } else { GL_UNSIGNED_BYTE };
    let texels = [7u32; 16];
    // SAFETY: plain GL calls; the 2x2 image fits `texels` in either class.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 2, 2, 0, GL_RGBA_INTEGER, ty, texels.as_ptr().cast());
    }
    no_error("glTexImage2D");
}

/// Sampling a 128-bit texture into a 128-bit target: one slot holding both
/// keys, refused by name (no compiler entry joins them), remembered. The
/// same program reading the texture into plain targets draws its plane-joined
/// variant from a slot of its own, and an RGBA8UI texture with the target key
/// has the empty sampler key.
#[test]
fn a_sampler_key_and_a_target_key_are_one_slot() {
    if !in_child("a_sampler_key_and_a_target_key_are_one_slot") {
        return;
    }
    on_gc(|| {
        let p = program(VS, FS_FETCH);
        set_rt_planes_test_targets(1);
        bind_uint_texture(GL_RGBA32UI);
        let d = draw(p);
        refused("a plane key and a target key", &d);
        assert_eq!(
            first_refusal_text(),
            "no compiler entry joins its 128-bit plane key and its 128-bit render-target key"
        );
        let s = state(p);
        assert_eq!(s.last, [RtPlanes::NONE, rt(1)]);
        let slots = used(&s);
        assert_eq!(slots.len(), 1, "one slot for the pair");
        assert_eq!((slots[0].planes, slots[0].rt), ([Wide128Planes::NONE, Wide128Planes::NONE.with(0, 1)], rt(1)));
        assert!(slots[0].missing, "remembered");
        refused("the pair again", &draw(p));
        assert_eq!(used(&state(p)), slots, "found, not compiled again");

        set_rt_planes_test_targets(0);
        let d = draw(p);
        drew("the plane key alone", &d);
        let s = state(p);
        assert_eq!(s.last, [RtPlanes::NONE; 2]);
        let joined = used(&s).into_iter().find(|v| v.rt.is_empty()).expect("a slot for the plane key");
        assert_eq!(joined.planes[1], Wide128Planes::NONE.with(0, 1));
        assert!(!joined.missing && names_usc(&d, joined.fs_usc_gpu), "the plane-joined variant is drawn");

        set_rt_planes_test_targets(1);
        bind_uint_texture(GL_RGBA8UI);
        let _ = draw(p);
        let s = state(p);
        assert_eq!(s.last, [RtPlanes::NONE, rt(1)]);
        assert!(used(&s).iter().any(|v| v.planes == [Wide128Planes::NONE; 2] && v.rt == rt(1)), "the target key alone");
        assert_eq!(s.evicted, 1, "a third key in two slots");
    });
}
