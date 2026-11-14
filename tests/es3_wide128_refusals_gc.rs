// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the refusals of a draw that samples an RGBA32F / UI / I
//! texture, pinned by name (`wide128_select` and `w128_variant_for`,
//! src/gl/draw.rs).
//!
//! A draw that needs a 128-bit plane variant it cannot have is refused:
//! `backend::note_draw_refused` counts it, no draw reaches the stream and GL
//! stays at GL_NO_ERROR (drawn with the plain shader it would read the B,A
//! plane as R,G). Each scenario pins, through the GL API: the refusal text,
//! that no kick was submitted, `glGetError() == GL_NO_ERROR`, and that the
//! next draw without the triggering state draws normally.
//!
//! | refusal | trigger | here |
//! |---|---|---|
//! | its fragment shader needs a front-facing twin and a 128-bit plane key at once | an FS reading `gl_FrontFacing` under `glFrontFace(GL_CCW)` on the top-down default surface | pinned |
//! | out of shader memory (fragment stage, vertex stage) | the variant's USC upload fails | pinned, both |
//! | no 128-bit plane variant of its shaders | the draw after a variant the compiler refused (the slot remembers it) | unreachable |
//! | the vertex / fragment shader does not compile | the keyed compile refuses a shape | unreachable |
//! | the fragment shader does not link / the link compiled it again | the keyed FS fails to link | unreachable |
//!
//! Why the last three cannot be reached through the GL API:
//! - The keyed compile refuses an offset, a gradient or a shadow look-up and a
//!   companion on a unit the shader samples itself
//!   (src/shaderc/online/vivante/wide128.rs). But the plain compile of the GC7000
//!   does not select an ES 3.00 offset, gradient or integer-shadow look-up at
//!   all ("an ES 3.00 texture look-up ... is not selected yet"): glCompileShader
//!   fails, so no program with one can be linked or drawn. A shadow sampler
//!   takes a depth texture, no 128-bit class. The companions are packed after
//!   the stage's own slots, so none is a unit the shader samples. Past the
//!   32-slot array a binding has no companion and is not keyed (it samples the
//!   black texel, by name), so that is no refusal. The keyed compile can also
//!   refuse for want of room (not remembered) and out of memory.
//! - Those compile failures are the only way a slot is remembered as missing,
//!   so the text "no 128-bit plane variant of its shaders" is unreachable too.
//!   It becomes reachable when the plain compile selects those look-ups; the
//!   scenario is then: the draw is refused with the compiler's own text, the
//!   next draw with the same key with this one, a plain texture draws.
//! - The keyed fragment shader is compiled from the source the program linked,
//!   with its varyings packed as the link packed them, so nothing in the API
//!   makes the second link differ.
//! - `note_draw_refused` logs its text for the first refusal of a process
//!   only: each scenario here runs alone in a fresh process (the test re-runs
//!   itself, `YAKOGL_W128_REFUSAL_CHILD`) and reads that line.
//! - A blend variant is no trigger of its own: the GC7000 blends in its pixel
//!   engine and its only fragment key is the winding twin.
//! - RGBA32F is held as RGBA16F by `sized_format` (src/gl/texture.rs) whatever
//!   the mask says, so the scenarios use RGBA32UI; the key is per binding, not
//!   per class.
//!
//! The scenarios need RGBA32UI in the sampled mask. The mask hook of the
//! in-crate tests (`backend::TEST_EXTRA_SAMPLED`) is `#[cfg(test)]`, so no
//! integration test can add it: until GC_SAMPLED_TEXFMTS holds the classes
//! (item gc-w128-flip) the class is closed, which nothing here can refuse; the
//! scenarios then say so on stderr and run the control (the same shader and
//! state with an RGBA8UI texture draws normally) alone. Before the flip they
//! were run with GLES_TEXFMT_RGBA32F, RGBA32UI and RGBA32I added to
//! GC_SAMPLED_TEXFMTS locally (reverted).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_refusals_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};

/* ---- the stand-in OSAL (as tests/es3_wide128_planes_gc.rs) -------------- */

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

/// Armed: the shader heap refuses every allocation (the plane variant's upload).
static FAIL_SHADER_HEAP: AtomicBool = AtomicBool::new(false);
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
    if size == 0
        || size > 1 << 30
        || (FAIL_SHADER_HEAP.load(Ordering::Relaxed) && heap_base == yakogl::backend::heap_base(GLES_HEAP_SHADER))
    {
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

const VS_PLAIN: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
const VS_FETCH: &str = "#version 300 es\nprecision highp int;\nprecision highp usampler2D;\nuniform usampler2D s;\nin vec4 p;\nvoid main() { gl_Position = p + vec4(texelFetch(s, ivec2(0), 0)) * 0.0; }\n";
const FS_FETCH: &str = "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp usampler2D;\nuniform usampler2D s;\nout vec4 o;\nvoid main() { o = vec4(texelFetch(s, ivec2(0), 0)); }\n";
const FS_FACING: &str = "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp usampler2D;\nuniform usampler2D s;\nout vec4 o;\nvoid main() { o = gl_FrontFacing ? vec4(texelFetch(s, ivec2(0), 0)) : vec4(0.0); }\n";
const FS_PLAIN: &str = "#version 300 es\nprecision highp float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";

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

/// Is RGBA32UI in the back end's sampled mask (so the GL core stores it and
/// the draw keys it)?
fn open() -> bool {
    (sampled_texfmts() as u64) & (gles_texfmt_bit(GLES_TEXFMT_RGBA32UI) as u64) != 0
}

/// A 2x2 unsigned-integer texture on unit 0, nearest: the 128-bit class
/// (`wide`) or the 32-bit RGBA8UI that samples as one plane.
fn bind_texture(wide: bool) {
    let texels = [7u32; 16];
    // SAFETY: plain GL calls; the 2x2 image holds 16 words (RGBA32UI) or 4 texels of 4 bytes (RGBA8UI).
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        if wide {
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA32UI as GLint,
                2,
                2,
                0,
                GL_RGBA_INTEGER,
                GL_UNSIGNED_INT,
                texels.as_ptr().cast(),
            );
        } else {
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA8UI as GLint,
                2,
                2,
                0,
                GL_RGBA_INTEGER,
                GL_UNSIGNED_BYTE,
                texels.as_ptr().cast(),
            );
        }
    }
    no_error("glTexImage2D");
}

/// What one quad did: the kicks it submitted, the draws refused meanwhile,
/// and the GL error after it.
struct Drew {
    kicks: usize,
    refused: u32,
    error: GLenum,
}

fn draw(p: GLuint) -> Drew {
    let (k0, r0) = (with_platform(|p| p.kicks.len()), draws_refused());
    // SAFETY: plain GL calls; QUAD is a static.
    let error = unsafe {
        glViewport(0, 0, 64, 64);
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
    Drew { kicks: with_platform(|p| p.kicks.len()) - k0, refused: draws_refused().wrapping_sub(r0), error }
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

/// Run the case on the stand-in OSAL with the Vivante back end up.
fn on_gc(case: impl FnOnce()) {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
    FAIL_SHADER_HEAP.store(false, Ordering::Relaxed);
    on_gc7000(case);
}

/// The first refusal of a process is the only one whose text is logged, so
/// a scenario that pins its text runs alone in a child process of this
/// binary: true in the child (run the body); in the parent this runs the
/// child, checks it passed, and returns false.
fn in_child(name: &str) -> bool {
    const VAR: &str = "YAKOGL_W128_REFUSAL_CHILD";
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

/// The GC7000's front-facing twin and a plane key cannot be had at once: an FS
/// reading gl_FrontFacing under the winding that selects the twin is
/// refused by name; with the other winding the same draw is keyed and drawn.
#[test]
fn a_front_facing_twin_and_a_plane_key_at_once_is_refused() {
    if !in_child("a_front_facing_twin_and_a_plane_key_at_once_is_refused") {
        return;
    }
    on_gc(|| {
        let p = program(VS_PLAIN, FS_FACING);
        // SAFETY: plain GL call.
        unsafe { glFrontFace(GL_CCW) }; /* the default surface is stored top-down: CCW takes the twin */
        /* the control: the twin alone, a plain texture */
        bind_texture(false);
        drew("the twin with an RGBA8UI texture", &draw(p));
        if !open() {
            eprintln!("only the control ran");
            return;
        }
        bind_texture(true);
        let d = draw(p);
        refused("the twin and a plane key", &d);
        assert_eq!(
            first_refusal_text(),
            "its fragment shader needs a front-facing twin and a 128-bit plane key at once"
        );
        /* the next draw, without the winding that takes the twin */
        // SAFETY: plain GL call.
        unsafe { glFrontFace(GL_CW) };
        drew("the plane key alone", &draw(p));
        let (_, planes) = wide128_draw_state(p).expect("the draw asked the sensitivity");
        assert!(!planes[1].is_empty(), "the fragment stage is keyed");
    });
}

/// A variant that cannot be uploaded (the shader heap is out of memory) is
/// refused by name, and unlike a compile refusal it is not remembered: the
/// next draw, once memory is back, is keyed and drawn.
#[test]
fn a_variant_that_cannot_be_uploaded_is_refused_and_asked_for_again() {
    if !in_child("a_variant_that_cannot_be_uploaded_is_refused_and_asked_for_again") {
        return;
    }
    on_gc(|| {
        let p = program(VS_PLAIN, FS_FETCH);
        bind_texture(false);
        drew("the control", &draw(p));
        if !open() {
            eprintln!("only the control ran");
            return;
        }
        bind_texture(true);
        FAIL_SHADER_HEAP.store(true, Ordering::Relaxed);
        let d = draw(p);
        FAIL_SHADER_HEAP.store(false, Ordering::Relaxed);
        refused("the variant's upload", &d);
        assert_eq!(first_refusal_text(), "out of shader memory");
        drew("the next draw, memory back", &draw(p));
        let (_, planes) = wide128_draw_state(p).expect("the draw asked the sensitivity");
        assert!(!planes[1].is_empty(), "the fragment stage is keyed");
    });
}

/// The same for a vertex stage's variant.
#[test]
fn a_vertex_variant_that_cannot_be_uploaded_is_refused_and_asked_for_again() {
    if !in_child("a_vertex_variant_that_cannot_be_uploaded_is_refused_and_asked_for_again") {
        return;
    }
    on_gc(|| {
        let p = program(VS_FETCH, FS_PLAIN);
        bind_texture(false);
        drew("the control", &draw(p));
        if !open() {
            eprintln!("only the control ran");
            return;
        }
        bind_texture(true);
        FAIL_SHADER_HEAP.store(true, Ordering::Relaxed);
        let d = draw(p);
        FAIL_SHADER_HEAP.store(false, Ordering::Relaxed);
        refused("the vertex variant's upload", &d);
        assert_eq!(first_refusal_text(), "out of shader memory");
        drew("the next draw, memory back", &draw(p));
        let (_, planes) = wide128_draw_state(p).expect("the draw asked the sensitivity");
        assert!(!planes[0].is_empty(), "the vertex stage is keyed");
    });
}
