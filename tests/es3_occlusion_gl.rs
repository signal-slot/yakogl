// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 occlusion queries through the GL API on the host, against a back
//! end of the test's own (docs/es3/occlusion-query.md sections 4, 5 and 8):
//! the query state machine, the per-draw `GlesDraw::occlusion_query`,
//! RESULT / RESULT_AVAILABLE across several kicks, and the errors of ES 3.0
//! section 2.14. The unit tests in `src/gl/es3.rs` drive the crate's mock
//! back end; this file reaches the same code only through the public
//! `YakoglBackend` table, as a real back end does.
//!
//! The back end is a fake whose "GPU" counts samples: each draw it is handed
//! with a non-zero `occlusion_query` adds `samples` to the counter of that
//! handle, but only when the kick that holds the draw completes (a kick
//! completes on `wait_kick` / `finish`, or when the test says so). Its
//! `query_begin` and `query_result` assert the contract of `src/backend.h`:
//! no kick holding a draw counted on the handle is still unfinished.
//!
//! An ES 2.0-reporting build (`report-es2`) must answer as before:
//! `es2_build_keeps_its_answers` is the only test it runs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_occlusion_gl
//! ```

#![cfg(feature = "host-test")]
// the ES 2.0 report runs one test of the file; the helpers of the rest are idle there
#![cfg_attr(any(feature = "report-es2", not(feature = "shaderc-online")), allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend, GLES_QUERY_HANDLES};
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host ------------------------------------------------ */

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

/* ---- the fake back end ---------------------------------------------------- */

const HANDLES: usize = GLES_QUERY_HANDLES as usize;

struct Fake {
    /// The kick being recorded (ids start at 1; 0 is "completed" at start).
    cur: u32,
    completed: u32,
    /// Draws in the kick being recorded.
    cur_draws: u32,
    /// The counted draws of the kick being recorded: (handle, samples).
    cur_items: Vec<(u32, u64)>,
    /// Submitted, not yet completed: (kick id, counted draws).
    kicks: Vec<(u32, Vec<(u32, u64)>)>,
    counters: [u64; HANDLES],
    /// Handles query_begin was called for.
    begun: Vec<u32>,
    /// `occlusion_query` of every draw handed over, in order.
    seen: Vec<u32>,
    /// Samples each counted draw "passes".
    samples: u64,
    begin_rc: i32,
    wait_rc: i32,
    ends: Vec<u32>,
    /// query_end submits the open kick, as a tile-based back end may.
    submit_at_end: bool,
}

static FAKE: Mutex<Fake> = Mutex::new(Fake {
    cur: 1,
    completed: 0,
    cur_draws: 0,
    cur_items: Vec::new(),
    kicks: Vec::new(),
    counters: [0; HANDLES],
    begun: Vec::new(),
    seen: Vec::new(),
    samples: 1,
    begin_rc: 0,
    wait_rc: 0,
    ends: Vec::new(),
    submit_at_end: false,
});

fn fake() -> MutexGuard<'static, Fake> {
    FAKE.lock().unwrap_or_else(|e| e.into_inner())
}

impl Fake {
    fn submit(&mut self) {
        if self.cur_draws > 0 {
            let items = core::mem::take(&mut self.cur_items);
            self.kicks.push((self.cur, items));
            self.cur += 1;
            self.cur_draws = 0;
        }
    }
    fn complete_to(&mut self, id: u32) {
        let (done, rest): (Vec<_>, Vec<_>) = core::mem::take(&mut self.kicks).into_iter().partition(|k| k.0 <= id);
        self.kicks = rest;
        for (_, items) in done {
            for (h, n) in items {
                self.counters[h as usize] += n;
            }
        }
        self.completed = self.completed.max(id.min(self.cur - 1));
    }
    /// The contract of query_begin and query_result: no draw counted on
    /// `h` is still unrecorded-unsubmitted or in a kick that has not
    /// completed.
    fn settled(&self, h: u32) -> bool {
        !self.cur_items.iter().any(|i| i.0 == h) && !self.kicks.iter().any(|k| k.1.iter().any(|i| i.0 == h))
    }
}

/// Let the "GPU" finish every submitted kick.
fn gpu_completes() {
    let mut f = fake();
    let id = f.cur - 1;
    f.complete_to(id);
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let mut f = fake();
    let h = (*d).occlusion_query;
    f.seen.push(h);
    f.cur_draws += 1;
    if h != 0 {
        let n = f.samples;
        f.cur_items.push((h, n));
    }
    0
}
unsafe extern "C" fn b_flush() -> i32 {
    fake().submit();
    0
}
unsafe extern "C" fn b_finish() -> i32 {
    let mut f = fake();
    f.submit();
    let id = f.cur - 1;
    f.complete_to(id);
    0
}
unsafe extern "C" fn b_kick_id() -> u32 {
    fake().cur
}
unsafe extern "C" fn b_kick_pending(id: u32) -> bool {
    id > fake().completed
}
unsafe extern "C" fn b_wait_kick(id: u32) -> i32 {
    let mut f = fake();
    f.complete_to(id);
    f.wait_rc
}
unsafe extern "C" fn b_query_begin(h: u32) -> i32 {
    let mut f = fake();
    assert!((1..GLES_QUERY_HANDLES).contains(&h), "handle {h}");
    assert!(f.settled(h), "query_begin({h}) while a kick counting on it is unfinished");
    if f.begin_rc != 0 {
        return f.begin_rc;
    }
    f.counters[h as usize] = 0;
    f.begun.push(h);
    0
}
unsafe extern "C" fn b_query_end(h: u32) {
    let mut f = fake();
    f.ends.push(h);
    if f.submit_at_end {
        f.submit();
    }
}
unsafe extern "C" fn b_query_result(h: u32, out: *mut u64) -> i32 {
    let f = fake();
    assert!(f.settled(h), "query_result({h}) while a kick counting on it is unfinished");
    *out = f.counters[h as usize];
    0
}

macro_rules! table {
    ($name:expr, $begin:expr, $end:expr, $result:expr) => {
        YakoglBackend {
            name: $name.as_ptr() as *const c_char,
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
            flush: Some(b_flush),
            finish: Some(b_finish),
            finish_previous: None,
            kick_id: Some(b_kick_id),
            kick_pending: Some(b_kick_pending),
            heap_base: Some(b_heap_base),
            counters: None,
            texture_released: None,
            surface_swapped: None,
            texture_uploaded: None,
            max_instance_divisor: 1,
            wait_kick: Some(b_wait_kick),
            samples_texel_copies: false,
            attachment_gone: None,
            depth_resolve: None,
            query_begin: $begin,
            query_end: $end,
            query_result: $result,
            generate_mipmap: None,
            return_mipmap_chain: None,
        }
    };
}

/// A back end that counts samples.
static COUNTING: YakoglBackend = table!(c"es3-oq", Some(b_query_begin), Some(b_query_end), Some(b_query_result));
/// A back end without the query entries.
static BARE: YakoglBackend = table!(c"es3-oq-bare", None, None, None);

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &COUNTING;
            #[cfg(feature = "shaderc-online")]
            {
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            }
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    reset();
    g
}

/// Back to a known state: no query active, every name deleted, the fake
/// empty and finished.
fn reset() {
    // SAFETY: plain GL calls on the one context.
    unsafe {
        yakogl::backend::g_backend = &COUNTING;
        fake().submit_at_end = false;
        for t in [GL_ANY_SAMPLES_PASSED, GL_ANY_SAMPLES_PASSED_CONSERVATIVE, GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN] {
            let mut v = 0;
            glGetQueryiv(t, GL_CURRENT_QUERY, &mut v);
            if v != 0 {
                glEndQuery(t);
            }
        }
        glFinish();
        let all: Vec<GLuint> = (1..32).collect();
        glDeleteQueries(all.len() as GLsizei, all.as_ptr());
        glUseProgram(0);
        glDisable(GL_RASTERIZER_DISCARD);
        while glGetError() != GL_NO_ERROR {}
    }
    let mut f = fake();
    f.seen.clear();
    f.begun.clear();
    f.counters = [0; HANDLES];
    f.samples = 1;
    f.begin_rc = 0;
    f.wait_rc = 0;
    f.ends.clear();
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn gen() -> GLuint {
    let mut q = 0;
    // SAFETY: one name.
    unsafe { glGenQueries(1, &mut q) };
    assert_ne!(q, 0);
    q
}

fn current(target: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one value.
    unsafe { glGetQueryiv(target, GL_CURRENT_QUERY, &mut v) };
    v
}

fn obj(q: GLuint, pname: GLenum) -> GLuint {
    let mut v = 77;
    // SAFETY: room for one value.
    unsafe { glGetQueryObjectuiv(q, pname, &mut v) };
    v
}

/* ---------------------------------------------------------------- ES 2.0 -- */

/// The ES 2.0 report answers as before: glBeginQuery refuses, whatever the
/// back end could do.
#[cfg(feature = "report-es2")]
#[test]
fn es2_build_keeps_its_answers() {
    let _g = context();
    let q = gen();
    // SAFETY: plain GL calls.
    unsafe {
        glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
        assert_eq!(err(), GL_INVALID_OPERATION);
        glBeginQuery(GL_ANY_SAMPLES_PASSED_CONSERVATIVE, q);
        assert_eq!(err(), GL_INVALID_OPERATION);
        glEndQuery(GL_ANY_SAMPLES_PASSED);
        assert_eq!(err(), GL_INVALID_OPERATION);
    }
    assert!(fake().begun.is_empty(), "the back end was not asked for a counter");
}

/* ------------------------------------------------------------- ES 3.0 -- */

#[cfg(all(not(feature = "report-es2"), feature = "shaderc-online"))]
mod es3 {
    use super::*;

    const VS: &str = "#version 300 es\nvoid main() { gl_Position = vec4(0.0); }\n";
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

    /// The current program; the draws below use it.
    fn program() {
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1);
            glUseProgram(p);
        }
    }

    fn draw() {
        // SAFETY: plain GL call.
        unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
    }

    #[test]
    fn a_count_above_zero_is_true_and_zero_is_false() {
        let _g = context();
        program();
        let (a, b) = (gen(), gen());
        // SAFETY: plain GL calls.
        unsafe {
            glBeginQuery(GL_ANY_SAMPLES_PASSED, a);
            assert_eq!(err(), GL_NO_ERROR);
            assert_eq!(current(GL_ANY_SAMPLES_PASSED), a as GLint);
            assert_eq!(current(GL_ANY_SAMPLES_PASSED_CONSERVATIVE), 0, "CURRENT_QUERY names the exact target");
            assert_eq!(glIsQuery(a), GL_TRUE_B);
            draw();
            glEndQuery(GL_ANY_SAMPLES_PASSED);
            assert_eq!((err(), current(GL_ANY_SAMPLES_PASSED)), (GL_NO_ERROR, 0));
            assert_eq!(obj(a, GL_QUERY_RESULT), GL_TRUE);
            assert_eq!(err(), GL_NO_ERROR);
            assert_eq!(obj(a, GL_QUERY_RESULT), GL_TRUE, "read again: the same answer");

            fake().samples = 0;
            glBeginQuery(GL_ANY_SAMPLES_PASSED_CONSERVATIVE, b);
            assert_eq!(current(GL_ANY_SAMPLES_PASSED_CONSERVATIVE), b as GLint);
            draw();
            draw();
            glEndQuery(GL_ANY_SAMPLES_PASSED_CONSERVATIVE);
            assert_eq!(obj(b, GL_QUERY_RESULT), GL_FALSE);
            assert_eq!(err(), GL_NO_ERROR);
        }
        let f = fake();
        assert_eq!(f.begun.len(), 2);
        assert_ne!(f.begun[0], f.begun[1], "each name has its own handle");
    }

    #[test]
    fn only_draws_inside_the_query_count_and_a_clear_is_not_a_draw() {
        let _g = context();
        program();
        let q = gen();
        // SAFETY: plain GL calls.
        unsafe {
            draw(); /* before */
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            draw();
            glClear(GL_COLOR_BUFFER_BIT);
            glEndQuery(GL_ANY_SAMPLES_PASSED);
            draw(); /* after */
            glFinish();
            assert_eq!(err(), GL_NO_ERROR);
        }
        let seen = fake().seen.clone();
        let counted: Vec<_> = seen.iter().filter(|h| **h != 0).collect();
        assert_eq!(counted.len(), 1, "exactly the draw inside: {seen:?}");
        assert_eq!(obj(q, GL_QUERY_RESULT), GL_TRUE);
        assert_eq!(fake().ends.len(), 1, "query_end called once");
    }

    #[test]
    fn rasterizer_discard_draws_are_not_counted() {
        let _g = context();
        program();
        let q = gen();
        // SAFETY: plain GL calls.
        unsafe {
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
            glEnable(GL_RASTERIZER_DISCARD);
            draw();
            glDisable(GL_RASTERIZER_DISCARD);
            glEndQuery(GL_ANY_SAMPLES_PASSED);
            glFinish();
        }
        assert!(fake().seen.iter().all(|h| *h == 0));
        assert_eq!(obj(q, GL_QUERY_RESULT), GL_FALSE);
    }

    #[test]
    fn a_result_spans_several_kicks_and_availability_follows_the_last_one() {
        let _g = context();
        program();
        let q = gen();
        // SAFETY: plain GL calls.
        unsafe {
            fake().samples = 3;
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
            draw();
            glFlush(); /* kick 1 */
            draw();
            glFlush(); /* kick 2 */
            draw(); /* kick 3, still being recorded at glEndQuery */
            glEndQuery(GL_ANY_SAMPLES_PASSED);
            assert_eq!(err(), GL_NO_ERROR);
        }
        let first = {
            let f = fake();
            assert_eq!((f.kicks.len(), f.cur_draws), (2, 1), "two kicks in flight, the third open");
            f.kicks[0].0
        };
        /* the first two complete; the poll submits the third, which runs */
        fake().complete_to(first + 1);
        assert_eq!(obj(q, GL_QUERY_RESULT_AVAILABLE), GL_FALSE);
        {
            let f = fake();
            assert_eq!((f.kicks.len(), f.cur_draws), (1, 0), "the poll submitted the open kick");
        }
        gpu_completes();
        assert_eq!(obj(q, GL_QUERY_RESULT_AVAILABLE), GL_TRUE);
        {
            let f = fake();
            let h = *f.begun.last().expect("the query was begun");
            assert_eq!(f.counters[h as usize], 9, "three draws of three samples");
        }
        assert_eq!(obj(q, GL_QUERY_RESULT), GL_TRUE);
    }

    #[test]
    fn reading_the_result_waits_for_the_query_kick_alone() {
        let _g = context();
        program();
        let q = gen();
        // SAFETY: plain GL calls.
        unsafe {
            fake().submit_at_end = true; /* the back end splits the frame at End */
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
            draw();
            glEndQuery(GL_ANY_SAMPLES_PASSED);
            draw(); /* recorded after the end, not part of the query */
        }
        assert_eq!(obj(q, GL_QUERY_RESULT), GL_TRUE, "the draw before End was waited for");
        let f = fake();
        assert_eq!(f.cur_draws, 1, "the later draw is still unsubmitted");
        assert_eq!(f.seen.last(), Some(&0), "and not counted");
    }

    #[test]
    fn a_draw_after_end_in_the_same_kick_is_not_counted() {
        let _g = context();
        program();
        let q = gen();
        // SAFETY: plain GL calls.
        unsafe {
            fake().samples = 0;
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
            draw();
            glEndQuery(GL_ANY_SAMPLES_PASSED);
            fake().samples = 5;
            draw(); /* same kick, after End */
        }
        assert_eq!(obj(q, GL_QUERY_RESULT), GL_FALSE);
        assert_eq!(err(), GL_NO_ERROR);
    }

    #[test]
    fn polling_without_a_flush_terminates() {
        let _g = context();
        program();
        let q = gen();
        // SAFETY: plain GL calls.
        unsafe {
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
            draw();
            glEndQuery(GL_ANY_SAMPLES_PASSED);
        }
        /* the first poll submits the kick; the "GPU" then finishes it */
        let mut polls = 0;
        while obj(q, GL_QUERY_RESULT_AVAILABLE) == GL_FALSE {
            polls += 1;
            assert!(polls < 3, "never available");
            gpu_completes();
        }
        assert_eq!(obj(q, GL_QUERY_RESULT), GL_TRUE);
    }

    #[test]
    fn a_query_without_draws_is_available_and_false() {
        let _g = context();
        let q = gen();
        // SAFETY: plain GL calls.
        unsafe {
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
            glEndQuery(GL_ANY_SAMPLES_PASSED);
        }
        assert_eq!(obj(q, GL_QUERY_RESULT_AVAILABLE), GL_TRUE);
        assert_eq!(obj(q, GL_QUERY_RESULT), GL_FALSE);
        assert_eq!(err(), GL_NO_ERROR);
    }

    #[test]
    fn a_name_can_count_again_and_starts_from_zero() {
        let _g = context();
        program();
        let q = gen();
        // SAFETY: plain GL calls.
        unsafe {
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
            draw();
            glEndQuery(GL_ANY_SAMPLES_PASSED);
            /* begun again before the first result was read: the counter is
             * reset only after the first use's kick (the fake asserts it) */
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
            assert_eq!(err(), GL_NO_ERROR);
            glEndQuery(GL_ANY_SAMPLES_PASSED);
            assert_eq!(obj(q, GL_QUERY_RESULT), GL_FALSE, "the second use counted nothing");
            /* the target is fixed by the first use */
            glBeginQuery(GL_ANY_SAMPLES_PASSED_CONSERVATIVE, q);
            assert_eq!(err(), GL_INVALID_OPERATION);
        }
    }

    #[test]
    fn the_errors_of_section_2_14() {
        let _g = context();
        program();
        let (a, b) = (gen(), gen());
        // SAFETY: plain GL calls.
        unsafe {
            glBeginQuery(GL_TRIANGLES, a);
            assert_eq!(err(), GL_INVALID_ENUM);
            glBeginQuery(GL_ANY_SAMPLES_PASSED, 0);
            assert_eq!(err(), GL_INVALID_OPERATION);
            glBeginQuery(GL_ANY_SAMPLES_PASSED, 31);
            assert_eq!(err(), GL_INVALID_OPERATION, "a name nobody generated");
            glEndQuery(GL_ANY_SAMPLES_PASSED);
            assert_eq!(err(), GL_INVALID_OPERATION, "nothing active");
            glEndQuery(GL_POINTS);
            assert_eq!(err(), GL_INVALID_ENUM);

            glBeginQuery(GL_ANY_SAMPLES_PASSED, a);
            assert_eq!(err(), GL_NO_ERROR);
            glBeginQuery(GL_ANY_SAMPLES_PASSED, b);
            assert_eq!(err(), GL_INVALID_OPERATION, "same target already active");
            glBeginQuery(GL_ANY_SAMPLES_PASSED_CONSERVATIVE, b);
            assert_eq!(err(), GL_INVALID_OPERATION, "the two occlusion targets share a binding point");
            glBeginQuery(GL_ANY_SAMPLES_PASSED, a);
            assert_eq!(err(), GL_INVALID_OPERATION, "id already active");
            glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, a);
            assert_eq!(err(), GL_INVALID_OPERATION, "id already active, other target");
            glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, b);
            assert_eq!(err(), GL_NO_ERROR, "another target does not collide");
            assert_eq!(current(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN), b as GLint);
            glEndQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN);
            glEndQuery(GL_ANY_SAMPLES_PASSED_CONSERVATIVE);
            assert_eq!(err(), GL_INVALID_OPERATION, "ended with the other occlusion target");
            assert_eq!(current(GL_ANY_SAMPLES_PASSED), a as GLint, "and still active");
            glEndQuery(GL_ANY_SAMPLES_PASSED);
            assert_eq!(err(), GL_NO_ERROR);
            glEndQuery(GL_ANY_SAMPLES_PASSED);
            assert_eq!(err(), GL_INVALID_OPERATION);

            /* b is a transform feedback query: it cannot begin as an occlusion query */
            glBeginQuery(GL_ANY_SAMPLES_PASSED, b);
            assert_eq!(err(), GL_INVALID_OPERATION);

            /* the object queries */
            glBeginQuery(GL_ANY_SAMPLES_PASSED, a);
            assert_eq!(err(), GL_NO_ERROR);
            let mut v = 77;
            glGetQueryObjectuiv(a, GL_QUERY_RESULT, &mut v);
            assert_eq!((err(), v), (GL_INVALID_OPERATION, 77), "active");
            glGetQueryObjectuiv(a, GL_QUERY_RESULT_AVAILABLE, &mut v);
            assert_eq!(err(), GL_INVALID_OPERATION, "active");
            glGetQueryObjectuiv(a, GL_CURRENT_QUERY, &mut v);
            assert_eq!(err(), GL_INVALID_ENUM);
            glEndQuery(GL_ANY_SAMPLES_PASSED);
            glGetQueryObjectuiv(0, GL_QUERY_RESULT, &mut v);
            assert_eq!(err(), GL_INVALID_OPERATION);
            let c = gen();
            glGetQueryObjectuiv(c, GL_QUERY_RESULT, &mut v);
            assert_eq!(err(), GL_INVALID_OPERATION, "generated, never begun");
            assert_eq!(glIsQuery(c), GL_FALSE_B);
            let mut vi = 0;
            glGetQueryiv(GL_ANY_SAMPLES_PASSED, GL_QUERY_RESULT, &mut vi);
            assert_eq!(err(), GL_INVALID_ENUM);
            glGetQueryiv(GL_TRIANGLES, GL_CURRENT_QUERY, &mut vi);
            assert_eq!(err(), GL_INVALID_ENUM);
        }
    }

    #[test]
    fn deleting_an_active_query_ends_it_and_its_handle_waits_before_reuse() {
        let _g = context();
        program();
        let q = gen();
        // SAFETY: plain GL calls.
        unsafe {
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
            draw();
            glDeleteQueries(1, &q);
            assert_eq!(err(), GL_NO_ERROR);
            assert_eq!(current(GL_ANY_SAMPLES_PASSED), 0);
            assert_eq!(glIsQuery(q), GL_FALSE_B);
            assert_eq!(fake().ends.len(), 1, "the deleted query was ended");
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
            assert_eq!(err(), GL_INVALID_OPERATION, "a deleted name is not a query");
            /* the name comes back with the same handle: query_begin (which
             * asserts it) runs only after the deleted query's kick */
            let again = gen();
            assert_eq!(again, q);
            glBeginQuery(GL_ANY_SAMPLES_PASSED, again);
            assert_eq!(err(), GL_NO_ERROR);
            glEndQuery(GL_ANY_SAMPLES_PASSED);
            assert_eq!(obj(again, GL_QUERY_RESULT), GL_FALSE, "nothing of the deleted query's count");
        }
        let f = fake();
        assert_eq!(f.begun.len(), 2);
        assert_eq!(f.begun[0], f.begun[1]);
    }

    #[test]
    fn many_queries_pending_at_once_keep_their_own_counts() {
        let _g = context();
        program();
        let mut names = Vec::new();
        // SAFETY: plain GL calls.
        unsafe {
            /* every generatable name, each left pending; odd ones draw */
            for i in 0..31 {
                let q = gen();
                glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
                assert_eq!(err(), GL_NO_ERROR);
                if i % 2 == 1 {
                    draw();
                }
                glEndQuery(GL_ANY_SAMPLES_PASSED);
                names.push(q);
            }
            let mut handles = fake().begun.clone();
            handles.sort_unstable();
            handles.dedup();
            assert_eq!(handles.len(), 31, "31 distinct handles");
            for (i, q) in names.iter().enumerate() {
                assert_eq!(obj(*q, GL_QUERY_RESULT), (i % 2) as GLuint, "query {i}");
            }
        }
    }

    #[test]
    fn a_back_end_that_cannot_allocate_gives_out_of_memory_and_no_active_query() {
        let _g = context();
        let q = gen();
        // SAFETY: plain GL calls.
        unsafe {
            fake().begin_rc = -12;
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
            assert_eq!(err(), GL_OUT_OF_MEMORY);
            assert_eq!(current(GL_ANY_SAMPLES_PASSED), 0);
            fake().begin_rc = 0;
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
            assert_eq!(err(), GL_NO_ERROR, "the next Begin succeeds");
            glEndQuery(GL_ANY_SAMPLES_PASSED);
            assert_eq!(err(), GL_NO_ERROR);
        }
    }

    #[test]
    fn lost_work_answers_true() {
        let _g = context();
        program();
        let q = gen();
        // SAFETY: plain GL calls.
        unsafe {
            fake().samples = 0;
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
            draw();
            glEndQuery(GL_ANY_SAMPLES_PASSED);
        }
        fake().wait_rc = -5;
        assert_eq!(obj(q, GL_QUERY_RESULT), GL_TRUE, "the conservative answer");
    }

    #[test]
    fn a_back_end_without_the_entries_keeps_the_invalid_operation() {
        let _g = context();
        let q = gen();
        // SAFETY: plain GL calls; the table is restored by the next test's reset.
        unsafe {
            yakogl::backend::g_backend = &BARE;
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
            assert_eq!(err(), GL_INVALID_OPERATION);
            assert_eq!(glIsQuery(q), GL_TRUE_B, "the name became an object, as before");
            glBeginQuery(GL_ANY_SAMPLES_PASSED_CONSERVATIVE, q);
            assert_eq!(err(), GL_INVALID_OPERATION);
            glEndQuery(GL_ANY_SAMPLES_PASSED);
            assert_eq!(err(), GL_INVALID_OPERATION);
            glGetQueryObjectuiv(q, GL_QUERY_RESULT, core::ptr::null_mut());
            assert_eq!(err(), GL_INVALID_OPERATION);
            /* the transform feedback target does not need the back end */
            glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, q);
            assert_eq!(err(), GL_NO_ERROR);
            glEndQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN);
            yakogl::backend::g_backend = &COUNTING;
        }
        assert!(fake().begun.is_empty());
    }
}
