// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014 The Android Open Source Project
//! The exact 100 cases of dEQP-GLES2.functional.fragment_ops.random, their
//! generator ported from dEQP, drawn through the GL core into the Mali back
//! end over a mock OSAL (fragrandom-cases, plan #373 cause 3).
//!
//! tests/mali_fragment_random.rs draws random states from the same state
//! space but not dEQP's cases. This file ports the generator itself, so the
//! draws are the ones the board runs: VK-GL-CTS commit
//! 93bca01861b0e3ef3c387027a9791e6d065f900c (the commit tools/cts/deps.toml
//! pins as DEQP_WORK's), from these files, read verbatim:
//!
//! - modules/gles2/functional/es2fRandomFragmentOpTests.cpp: the 100 cases
//!   (case `n` has seed `n * 10`), the 10 iterations of a case (iteration
//!   seed `deUint32Hash(seed) ^ deInt32Hash(iter) ^ deInt32Hash(baseSeed)`,
//!   base seed 0, the `--deqp-base-seed` default), the 64x64 viewport at a
//!   random place in the render target, 3 render commands an iteration, the
//!   clear (0.25, 0.5, 0.75, 1), depth 1, stencil 0, `setGLState` and
//!   `renderQuad`;
//! - modules/glshared/glsInteractionTestUtil.cpp and .hpp:
//!   `computeRandomRenderState` (its ES 2.0 branch), `computeRandomQuad`,
//!   `computeRandomRenderCommands`, `getRandomColor`, `RenderState`;
//! - modules/glshared/glsFragmentOpUtil.cpp and .hpp: `QuadRenderer` (the
//!   GLSL ES 1.00 program, the four-vertex quad and its six u8 indices) and
//!   `IntegerQuad`;
//! - framework/delibs/debase/deRandom.c, framework/delibs/decpp/deRandom.hpp
//!   and framework/delibs/debase/deInt32.h: de::Random (xorshift128 with
//!   deRandom_init's seeding, getFloat, getBool, getInt, choose by reservoir
//!   sampling) and deInt32Hash.
//!
//! Those sources are Copyright 2014 The Android Open Source Project, under
//! the Apache License 2.0, as this file is.
//!
//! The render target is the board run's: a 256x256 pbuffer
//! (tools/cts/mali-deqp-run.sh, DEQP_SURFACE), here the GL core's default
//! surface (RGBA8 with depth and stencil). The case's own reference
//! rendering and image compare are left out: no pixels are checked here
//! (src/mali/frame_blend_tests.rs compares frames with Mesa's).
//!
//! For each case, each of its 10 iterations and each of their 3 quads, the
//! test makes dEQP's GL calls and asserts that
//!
//! - the draw is taken: `glGetError` is `GL_NO_ERROR` and `draws_refused`
//!   does not move (a Mali refusal, NOT_BUILT_* or another, reaches the GL
//!   core as `-ENOTSUP` and is both);
//! - the draw reaches the back end once, with the generated state: the test
//!   puts a copy of the Mali table in `g_backend` whose `draw` records the
//!   `GlesDraw` the GL core hands the Mali and passes it on to the Mali's
//!   own. It compares the enables (scissor, stencil, depth, blend), the
//!   scissor box and viewport, the blend equations, factors and constant,
//!   the depth function and write mask, both faces' stencil function,
//!   reference (clamped to 0..255, ES 2.0 4.1.5), compare mask, operations
//!   and write mask (their low 8 bits: the stencil buffer has 8), and the
//!   colour mask. Dither has no `GlesDraw` field and is not compared.
//!
//! No case is refused today, so the one test covers all 100. A change that
//! makes the Mali refuse a case moves that case out of
//! `fragment_random_cases` into a test of its own marked `#[ignore]` with
//! the refusal's name.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_fragment_random_cases
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::cell::RefCell;
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::OnceLock;
use yakogl::backend::{self, GlesDraw, YakoglBackend};
use yakogl::backend::{GlesCmp, GlesStencilOp};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::*;
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

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let n = ALLOCS.fetch_add(1, Ordering::Relaxed) + 1;
    assert!(size <= 16 << 20);
    let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr().cast::<c_void>();
    *out = OsalGpuMem { cpu, gpu: 0x40_0000_0000 + n * (16 << 20), phys: 0, size, impl_: n as *mut c_void };
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
unsafe extern "C" fn m_submit(_kick: *const c_void, _len: usize) -> i64 {
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

/* ---- the draws the GL core hands the Mali -------------------------------- */

type DrawFn = unsafe extern "C" fn(*const GlesDraw) -> i32;

/// The Mali table's own `draw`.
static MALI_DRAW: OnceLock<DrawFn> = OnceLock::new();

thread_local! {
    /// Every draw the GL core handed the back end, in order.
    static DRAWS: RefCell<Vec<GlesDraw>> = const { RefCell::new(Vec::new()) };
}

/// The recording `draw`: keep a copy, then the Mali's own.
unsafe extern "C" fn recording_draw(d: *const GlesDraw) -> i32 {
    DRAWS.with(|v| v.borrow_mut().push(*d));
    (MALI_DRAW.get().expect("the Mali draw"))(d)
}

/* ---- de::Random (deRandom.c, deRandom.hpp) and deInt32Hash (deInt32.h) --- */

struct Random {
    x: u32,
    y: u32,
    z: u32,
    w: u32,
}

impl Random {
    /// deRandom_init.
    fn new(seed: u32) -> Self {
        Random {
            x: ((seed as i32).wrapping_neg() as u32) ^ 123456789,
            y: 362436069u32.wrapping_mul(seed),
            z: 521288629 ^ (seed >> 7),
            w: 88675123 ^ (seed << 3),
        }
    }
    /// deRandom_getUint32.
    fn get_u32(&mut self) -> u32 {
        let w = self.w;
        let t = self.x ^ (self.x << 11);
        self.x = self.y;
        self.y = self.z;
        self.z = w;
        self.w = (w ^ (w >> 19)) ^ (t ^ (t >> 8));
        self.w
    }
    /// deRandom_getFloat: [0, 1[.
    fn get_float(&mut self) -> f32 {
        (self.get_u32() & 0x0FFF_FFFF) as f32 / (0x0FFF_FFFFu32 + 1) as f32
    }
    /// deRandom_getBool.
    fn get_bool(&mut self) -> bool {
        (self.get_u32() & 0xFF_FFFF) < 0x80_0000
    }
    /// de::Random::getInt: [min, max].
    fn get_int(&mut self, min: i32, max: i32) -> i32 {
        assert!(min <= max);
        if min == i32::MIN && max == i32::MAX {
            self.get_u32() as i32
        } else {
            min.wrapping_add((self.get_u32() % ((max as u32).wrapping_sub(min as u32).wrapping_add(1))) as i32)
        }
    }
    /// de::Random::choose<T>(first, last): reservoir sampling of one item.
    fn choose<T: Copy>(&mut self, items: &[T]) -> T {
        let mut val = items[0];
        for (ndx, item) in items.iter().enumerate().skip(1) {
            if self.get_int(0, ndx as i32) < 1 {
                val = *item;
            }
        }
        val
    }
}

/// deInt32Hash (deUint32Hash is the same on the bits).
fn de_int32_hash(a: i32) -> u32 {
    let mut key = a as u32;
    key = (key ^ 61) ^ (key >> 16);
    key = key.wrapping_add(key << 3);
    key ^= key >> 4;
    key = key.wrapping_mul(0x27d4_eb2d);
    key ^= key >> 15;
    key
}

/// deCeilFloatToInt32.
fn ceil_to_i32(v: f32) -> i32 {
    v.ceil() as i32
}

/* ---- glsInteractionTestUtil --------------------------------------------- */

#[derive(Clone, Copy, Debug, Default)]
struct BlendState {
    equation: GLenum,
    src_func: GLenum,
    dst_func: GLenum,
}

#[derive(Clone, Copy, Debug, Default)]
struct StencilState {
    function: GLenum,
    reference: i32,
    compare_mask: u32,
    stencil_fail_op: GLenum,
    depth_fail_op: GLenum,
    depth_pass_op: GLenum,
    write_mask: u32,
}

/// rr::FACETYPE_BACK and rr::FACETYPE_FRONT: the indices of `stencil`.
const FACE_BACK: usize = 0;
const FACE_FRONT: usize = 1;

#[derive(Clone, Copy, Debug, Default)]
struct RenderState {
    scissor_test: bool,
    /// rr::WindowRectangle: left, bottom, width, height.
    scissor: [i32; 4],
    stencil_test: bool,
    stencil: [StencilState; 2],
    depth_test: bool,
    depth_func: GLenum,
    depth_write: bool,
    blend: bool,
    blend_rgb: BlendState,
    blend_a: BlendState,
    blend_color: [f32; 4],
    dither: bool,
    color_mask: [bool; 4],
}

/// gls::FragmentOpUtil::IntegerQuad: viewport coordinates, depth in [0, 1].
#[derive(Clone, Copy, Debug, Default)]
struct IntegerQuad {
    pos_a: [i32; 2],
    pos_b: [i32; 2],
    color: [[f32; 4]; 4],
    depth: [f32; 4],
}

#[derive(Clone, Copy, Debug, Default)]
struct RenderCommand {
    quad: IntegerQuad,
    state: RenderState,
}

const COMPARE_FUNCS: [GLenum; 8] =
    [GL_NEVER, GL_ALWAYS, GL_LESS, GL_LEQUAL, GL_EQUAL, GL_GEQUAL, GL_GREATER, GL_NOTEQUAL];
const STENCIL_OPS: [GLenum; 8] =
    [GL_KEEP, GL_ZERO, GL_REPLACE, GL_INCR, GL_DECR, GL_INVERT, GL_INCR_WRAP, GL_DECR_WRAP];
const BLEND_FUNCS: [GLenum; 15] = [
    GL_ZERO,
    GL_ONE,
    GL_SRC_COLOR,
    GL_ONE_MINUS_SRC_COLOR,
    GL_DST_COLOR,
    GL_ONE_MINUS_DST_COLOR,
    GL_SRC_ALPHA,
    GL_ONE_MINUS_SRC_ALPHA,
    GL_DST_ALPHA,
    GL_ONE_MINUS_DST_ALPHA,
    GL_CONSTANT_COLOR,
    GL_ONE_MINUS_CONSTANT_COLOR,
    GL_CONSTANT_ALPHA,
    GL_ONE_MINUS_CONSTANT_ALPHA,
    GL_SRC_ALPHA_SATURATE,
];
const BLEND_EQUATIONS_ES2: [GLenum; 3] = [GL_FUNC_ADD, GL_FUNC_SUBTRACT, GL_FUNC_REVERSE_SUBTRACT];
const BLEND_FUNCS_DST_ES2: [GLenum; 14] = [
    GL_ZERO,
    GL_ONE,
    GL_SRC_COLOR,
    GL_ONE_MINUS_SRC_COLOR,
    GL_DST_COLOR,
    GL_ONE_MINUS_DST_COLOR,
    GL_SRC_ALPHA,
    GL_ONE_MINUS_SRC_ALPHA,
    GL_DST_ALPHA,
    GL_ONE_MINUS_DST_ALPHA,
    GL_CONSTANT_COLOR,
    GL_ONE_MINUS_CONSTANT_COLOR,
    GL_CONSTANT_ALPHA,
    GL_ONE_MINUS_CONSTANT_ALPHA,
];

/// getRandomColor.
fn random_color(rnd: &mut Random) -> [f32; 4] {
    const COMPONENTS: [f32; 7] = [0.0, 0.2, 0.4, 0.5, 0.6, 0.8, 1.0];
    let r = rnd.choose(&COMPONENTS);
    let g = rnd.choose(&COMPONENTS);
    let b = rnd.choose(&COMPONENTS);
    let a = rnd.choose(&COMPONENTS);
    [r, g, b, a]
}

/// computeRandomRenderState, the `glu::ApiType::es(2, 0)` branch.
fn compute_random_render_state(rnd: &mut Random, target_width: i32, target_height: i32) -> RenderState {
    const SCISSOR_TEST_PROBABILITY: f32 = 0.2;
    const STENCIL_TEST_PROBABILITY: f32 = 0.4;
    const DEPTH_TEST_PROBABILITY: f32 = 0.6;
    const BLEND_PROBABILITY: f32 = 0.4;
    const DITHER_PROBABILITY: f32 = 0.5;
    const DEPTH_WRITE_PROBABILITY: f32 = 0.7;
    const COLOR_WRITE_PROBABILITY: f32 = 0.7;
    const MIN_STENCIL_VAL: i32 = -3;
    const MAX_STENCIL_VAL: i32 = 260;
    const MAX_SCISSOR_OUT_OF_BOUNDS: i32 = 10;
    const MIN_SCISSOR_SIZE: f32 = 0.7;

    let mut state = RenderState {
        scissor_test: rnd.get_float() < SCISSOR_TEST_PROBABILITY,
        stencil_test: rnd.get_float() < STENCIL_TEST_PROBABILITY,
        depth_test: rnd.get_float() < DEPTH_TEST_PROBABILITY,
        blend: rnd.get_float() < BLEND_PROBABILITY,
        dither: rnd.get_float() < DITHER_PROBABILITY,
        ..Default::default()
    };

    if state.scissor_test {
        let min_w = ceil_to_i32(MIN_SCISSOR_SIZE * target_width as f32);
        let min_h = ceil_to_i32(MIN_SCISSOR_SIZE * target_height as f32);
        let max_w = target_width + 2 * MAX_SCISSOR_OUT_OF_BOUNDS;
        let max_h = target_height + 2 * MAX_SCISSOR_OUT_OF_BOUNDS;
        let w = rnd.get_int(min_w, max_w);
        let h = rnd.get_int(min_h, max_h);
        let x = rnd.get_int(-MAX_SCISSOR_OUT_OF_BOUNDS, target_width + MAX_SCISSOR_OUT_OF_BOUNDS - w);
        let y = rnd.get_int(-MAX_SCISSOR_OUT_OF_BOUNDS, target_height + MAX_SCISSOR_OUT_OF_BOUNDS - h);
        state.scissor = [x, y, w, h];
    }

    if state.stencil_test {
        for s in &mut state.stencil {
            s.function = rnd.choose(&COMPARE_FUNCS);
            s.reference = rnd.get_int(MIN_STENCIL_VAL, MAX_STENCIL_VAL);
            s.compare_mask = rnd.get_u32();
            s.stencil_fail_op = rnd.choose(&STENCIL_OPS);
            s.depth_fail_op = rnd.choose(&STENCIL_OPS);
            s.depth_pass_op = rnd.choose(&STENCIL_OPS);
            s.write_mask = rnd.get_u32();
        }
    }

    if state.depth_test {
        state.depth_func = rnd.choose(&COMPARE_FUNCS);
        state.depth_write = rnd.get_float() < DEPTH_WRITE_PROBABILITY;
    }

    if state.blend {
        state.blend_rgb.equation = rnd.choose(&BLEND_EQUATIONS_ES2);
        state.blend_rgb.src_func = rnd.choose(&BLEND_FUNCS);
        state.blend_rgb.dst_func = rnd.choose(&BLEND_FUNCS_DST_ES2);
        state.blend_a.equation = rnd.choose(&BLEND_EQUATIONS_ES2);
        state.blend_a.src_func = rnd.choose(&BLEND_FUNCS);
        state.blend_a.dst_func = rnd.choose(&BLEND_FUNCS_DST_ES2);
        state.blend_color = random_color(rnd);
    }

    for m in &mut state.color_mask {
        *m = rnd.get_float() < COLOR_WRITE_PROBABILITY;
    }
    state
}

/// computeRandomQuad.
fn compute_random_quad(rnd: &mut Random, target_width: i32, target_height: i32) -> IntegerQuad {
    const DEPTH_VALUES: [f32; 8] = [0.0, 0.2, 0.4, 0.5, 0.51, 0.6, 0.8, 0.95];
    const MAX_OUT_OF_BOUNDS: i32 = 0;
    const MIN_SIZE: f32 = 0.5;

    let min_w = ceil_to_i32(MIN_SIZE * target_width as f32);
    let min_h = ceil_to_i32(MIN_SIZE * target_height as f32);
    let max_w = target_width + 2 * MAX_OUT_OF_BOUNDS;
    let max_h = target_height + 2 * MAX_OUT_OF_BOUNDS;

    let width = rnd.get_int(min_w, max_w);
    let height = rnd.get_int(min_h, max_h);
    let x = rnd.get_int(-MAX_OUT_OF_BOUNDS, target_width + MAX_OUT_OF_BOUNDS - width);
    let y = rnd.get_int(-MAX_OUT_OF_BOUNDS, target_height + MAX_OUT_OF_BOUNDS - height);

    let flip_x = rnd.get_bool();
    let flip_y = rnd.get_bool();

    let depth = rnd.choose(&DEPTH_VALUES);

    let mut quad = IntegerQuad {
        pos_a: [if flip_x { x + width - 1 } else { x }, if flip_y { y + height - 1 } else { y }],
        pos_b: [if flip_x { x } else { x + width - 1 }, if flip_y { y } else { y + height - 1 }],
        ..Default::default()
    };
    for c in &mut quad.color {
        *c = random_color(rnd);
    }
    quad.depth = [depth; 4];
    quad
}

/// computeRandomRenderCommands.
fn compute_random_render_commands(rnd: &mut Random, n: usize, target_w: i32, target_h: i32) -> Vec<RenderCommand> {
    (0..n)
        .map(|_| {
            let state = compute_random_render_state(rnd, target_w, target_h);
            let quad = compute_random_quad(rnd, target_w, target_h);
            RenderCommand { quad, state }
        })
        .collect()
}

/* ---- es2fRandomFragmentOpTests ------------------------------------------ */

const VIEWPORT_WIDTH: i32 = 64;
const VIEWPORT_HEIGHT: i32 = 64;
const NUM_CALLS_PER_ITERATION: usize = 3;
const NUM_ITERATIONS_PER_CASE: i32 = 10;
const CASES: u32 = 100;
/// `--deqp-base-seed`'s default (tcuCommandLine.cpp).
const BASE_SEED: i32 = 0;
/// The board's pbuffer (DEQP_SURFACE).
const RT_SIZE: i32 = 256;

/// One iteration of a case: the viewport's place and the commands, as
/// RandomFragmentOpCase::iterate draws them.
struct Iteration {
    viewport_x: i32,
    viewport_y: i32,
    commands: Vec<RenderCommand>,
}

fn iteration(case: u32, iter: i32) -> Iteration {
    let seed = case * NUM_ITERATIONS_PER_CASE as u32;
    let iter_seed = de_int32_hash(seed as i32) ^ de_int32_hash(iter) ^ de_int32_hash(BASE_SEED);
    let mut rnd = Random::new(iter_seed);
    let width = RT_SIZE.min(VIEWPORT_WIDTH);
    let height = RT_SIZE.min(VIEWPORT_HEIGHT);
    let viewport_x = rnd.get_int(0, RT_SIZE - width);
    let viewport_y = rnd.get_int(0, RT_SIZE - height);
    let commands = compute_random_render_commands(&mut rnd, NUM_CALLS_PER_ITERATION, width, height);
    Iteration { viewport_x, viewport_y, commands }
}

/* ---- the GL side -------------------------------------------------------- */

/// glsFragmentOpUtil's GLSL ES 1.00 program.
const VERT_SRC: &str =
    "attribute highp vec4 a_position;\nattribute mediump vec4 a_color;\nvarying mediump vec4 v_color;\n\
     void main()\n{\n    gl_Position = a_position;\n    v_color = a_color;\n}\n";
const FRAG_SRC: &str = "varying mediump vec4 v_color;\nvoid main()\n{\n    gl_FragColor = v_color;\n}\n";

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

/// The program's attribute locations: a_position, a_color.
struct Program {
    position: GLuint,
    color: GLuint,
}

/// The recording back end, the context with its 256x256 default surface,
/// and QuadRenderer's program in use.
fn setup() -> Program {
    backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
    // SAFETY: static tables and one context; this is the only test of the
    // file. The table copy is bitwise (pointers and plain data, nothing
    // dropped) and leaked, so it outlives every draw.
    unsafe {
        osal_set_ops(&OSAL);
        let mali: &YakoglBackend = &yakogl::mali::backend::yakogl_backend_mali;
        MALI_DRAW.set(mali.draw.expect("the Mali table has a draw")).ok();
        let mut table: YakoglBackend = core::ptr::read(mali);
        table.draw = Some(recording_draw);
        backend::g_backend = Box::leak(Box::new(table));
        assert_eq!(yakogl::mali::backend::mali_init(), 0);
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert!(!yakogl::gl::context::gl_context_create(RT_SIZE as u32, RT_SIZE as u32).is_null());
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VERT_SRC));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FRAG_SRC));
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        let position = glGetAttribLocation(p, c"a_position".as_ptr());
        let color = glGetAttribLocation(p, c"a_color".as_ptr());
        assert!(position >= 0 && color >= 0, "attribute locations {position}, {color}");
        glUseProgram(p);
        assert_eq!(glGetError(), GL_NO_ERROR);
        Program { position: position as GLuint, color: color as GLuint }
    }
}

/// setGLState.
fn set_gl_state(state: &RenderState, viewport_x: i32, viewport_y: i32) {
    // SAFETY: plain GL calls.
    unsafe {
        if state.scissor_test {
            glEnable(GL_SCISSOR_TEST);
            let [l, b, w, h] = state.scissor;
            glScissor(viewport_x + l, viewport_y + b, w, h);
        } else {
            glDisable(GL_SCISSOR_TEST);
        }

        if state.stencil_test {
            glEnable(GL_STENCIL_TEST);
            for face in [FACE_BACK, FACE_FRONT] {
                let gl_face = if face == FACE_BACK { GL_BACK } else { GL_FRONT };
                let s = &state.stencil[face];
                glStencilFuncSeparate(gl_face, s.function, s.reference, s.compare_mask);
                glStencilOpSeparate(gl_face, s.stencil_fail_op, s.depth_fail_op, s.depth_pass_op);
                glStencilMaskSeparate(gl_face, s.write_mask);
            }
        } else {
            glDisable(GL_STENCIL_TEST);
        }

        if state.depth_test {
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(state.depth_func);
            glDepthMask(state.depth_write as GLboolean);
        } else {
            glDisable(GL_DEPTH_TEST);
        }

        if state.blend {
            glEnable(GL_BLEND);
            glBlendEquationSeparate(state.blend_rgb.equation, state.blend_a.equation);
            glBlendFuncSeparate(
                state.blend_rgb.src_func,
                state.blend_rgb.dst_func,
                state.blend_a.src_func,
                state.blend_a.dst_func,
            );
            let [r, g, b, a] = state.blend_color;
            glBlendColor(r, g, b, a);
        } else {
            glDisable(GL_BLEND);
        }

        if state.dither {
            glEnable(GL_DITHER);
        } else {
            glDisable(GL_DITHER);
        }

        let m = state.color_mask;
        glColorMask(m[0] as GLboolean, m[1] as GLboolean, m[2] as GLboolean, m[3] as GLboolean);
    }
}

/// renderQuad and QuadRenderer::render: the quad's viewport, its four
/// vertices in NDC and its six u8 indices, drawn from client arrays. Returns
/// the GL error and how many draws were refused.
fn render_quad(prog: &Program, quad: &IntegerQuad, base_x: i32, base_y: i32) -> (GLenum, u32) {
    let flip_x = quad.pos_b[0] < quad.pos_a[0];
    let flip_y = quad.pos_b[1] < quad.pos_a[1];
    let viewport_x = quad.pos_a[0].min(quad.pos_b[0]);
    let viewport_y = quad.pos_a[1].min(quad.pos_b[1]);
    let viewport_w = (quad.pos_a[0] - quad.pos_b[0]).abs() + 1;
    let viewport_h = (quad.pos_a[1] - quad.pos_b[1]).abs() + 1;
    let a = [if flip_x { 1.0f32 } else { -1.0 }, if flip_y { 1.0 } else { -1.0 }];
    let b = [if flip_x { -1.0f32 } else { 1.0 }, if flip_y { -1.0 } else { 1.0 }];
    let depth = quad.depth.map(|d| d * 2.0 - 1.0);
    let position: [f32; 16] =
        [a[0], a[1], depth[0], 1.0, a[0], b[1], depth[1], 1.0, b[0], a[1], depth[2], 1.0, b[0], b[1], depth[3], 1.0];
    let colors: [f32; 16] = quad.color.as_flattened().try_into().unwrap();
    let indices: [u8; 6] = [0, 2, 1, 1, 2, 3];
    let refused = backend::draws_refused();
    // SAFETY: the arrays outlive the draw, which reads them before it returns.
    let e = unsafe {
        glViewport(base_x + viewport_x, base_y + viewport_y, viewport_w, viewport_h);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        glEnableVertexAttribArray(prog.position);
        glVertexAttribPointer(prog.position, 4, GL_FLOAT, 0, 0, position.as_ptr() as *const c_void);
        glEnableVertexAttribArray(prog.color);
        glVertexAttribPointer(prog.color, 4, GL_FLOAT, 0, 0, colors.as_ptr() as *const c_void);
        glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_BYTE, indices.as_ptr() as *const c_void);
        glGetError()
    };
    (e, backend::draws_refused() - refused)
}

/* ---- the state a draw reaches the back end with ------------------------- */

fn cmp(f: GLenum) -> GlesCmp {
    match f {
        GL_NEVER => backend::GLES_CMP_NEVER,
        GL_LESS => backend::GLES_CMP_LESS,
        GL_EQUAL => backend::GLES_CMP_EQUAL,
        GL_LEQUAL => backend::GLES_CMP_LEQUAL,
        GL_GREATER => backend::GLES_CMP_GREATER,
        GL_NOTEQUAL => backend::GLES_CMP_NOTEQUAL,
        GL_GEQUAL => backend::GLES_CMP_GEQUAL,
        GL_ALWAYS => backend::GLES_CMP_ALWAYS,
        _ => panic!("compare function 0x{f:x}"),
    }
}

fn sop(op: GLenum) -> GlesStencilOp {
    match op {
        GL_KEEP => backend::GLES_SOP_KEEP,
        GL_ZERO => backend::GLES_SOP_ZERO,
        GL_REPLACE => backend::GLES_SOP_REPLACE,
        GL_INCR => backend::GLES_SOP_INCR_SAT,
        GL_DECR => backend::GLES_SOP_DECR_SAT,
        GL_INVERT => backend::GLES_SOP_INVERT,
        GL_INCR_WRAP => backend::GLES_SOP_INCR_WRAP,
        GL_DECR_WRAP => backend::GLES_SOP_DECR_WRAP,
        _ => panic!("stencil op 0x{op:x}"),
    }
}

fn factor(f: GLenum) -> u8 {
    (match f {
        GL_ZERO => SHC_BF_ZERO,
        GL_ONE => SHC_BF_ONE,
        GL_SRC_COLOR => SHC_BF_SRC_COLOR,
        GL_ONE_MINUS_SRC_COLOR => SHC_BF_INV_SRC_COLOR,
        GL_DST_COLOR => SHC_BF_DST_COLOR,
        GL_ONE_MINUS_DST_COLOR => SHC_BF_INV_DST_COLOR,
        GL_SRC_ALPHA => SHC_BF_SRC_ALPHA,
        GL_ONE_MINUS_SRC_ALPHA => SHC_BF_INV_SRC_ALPHA,
        GL_DST_ALPHA => SHC_BF_DST_ALPHA,
        GL_ONE_MINUS_DST_ALPHA => SHC_BF_INV_DST_ALPHA,
        GL_CONSTANT_COLOR => SHC_BF_CONST_COLOR,
        GL_ONE_MINUS_CONSTANT_COLOR => SHC_BF_INV_CONST_COLOR,
        GL_CONSTANT_ALPHA => SHC_BF_CONST_ALPHA,
        GL_ONE_MINUS_CONSTANT_ALPHA => SHC_BF_INV_CONST_ALPHA,
        GL_SRC_ALPHA_SATURATE => SHC_BF_SRC_ALPHA_SATURATE,
        _ => panic!("blend factor 0x{f:x}"),
    }) as u8
}

fn equation(e: GLenum) -> u8 {
    (match e {
        GL_FUNC_ADD => SHC_BLEND_ADD,
        GL_FUNC_SUBTRACT => SHC_BLEND_SUBTRACT,
        GL_FUNC_REVERSE_SUBTRACT => SHC_BLEND_REVERSE_SUBTRACT,
        _ => panic!("blend equation 0x{e:x}"),
    }) as u8
}

/// The differences between the draw the back end got and the command's
/// state; empty when they agree.
fn compare(d: &GlesDraw, it: &Iteration, cmd: &RenderCommand) -> Vec<String> {
    let s = &cmd.state;
    let mut diff = Vec::new();
    let mut check = |what: &str, got: String, want: String| {
        if got != want {
            diff.push(format!("{what}: {got}, generated {want}"));
        }
    };
    let q = &cmd.quad;
    let viewport = [
        it.viewport_x + q.pos_a[0].min(q.pos_b[0]),
        it.viewport_y + q.pos_a[1].min(q.pos_b[1]),
        (q.pos_a[0] - q.pos_b[0]).abs() + 1,
        (q.pos_a[1] - q.pos_b[1]).abs() + 1,
    ];
    check("index_count", format!("{}", d.index_count), "6".into());
    check("viewport", format!("{:?}", d.viewport), format!("{viewport:?}"));

    check("scissor", format!("{}", d.scissor), format!("{}", s.scissor_test));
    if s.scissor_test {
        let [l, b, w, h] = s.scissor;
        let want = [it.viewport_x + l, it.viewport_y + b, w, h];
        check("scissor_rect", format!("{:?}", d.scissor_rect), format!("{want:?}"));
    }

    check("stencil", format!("{}", d.stencil), format!("{}", s.stencil_test));
    if s.stencil_test {
        let f = &s.stencil[FACE_FRONT];
        let b = &s.stencil[FACE_BACK];
        let face = |x: &StencilState| {
            format!(
                "func {} ref {} cmp 0x{:02x} ops {} {} {} write 0x{:02x}",
                cmp(x.function),
                x.reference.clamp(0, 255),
                x.compare_mask as u8,
                sop(x.stencil_fail_op),
                sop(x.depth_fail_op),
                sop(x.depth_pass_op),
                x.write_mask as u8
            )
        };
        let front = format!(
            "func {} ref {} cmp 0x{:02x} ops {} {} {} write 0x{:02x}",
            d.stencil_func,
            d.stencil_ref,
            d.stencil_cmp_mask,
            d.stencil_sfail,
            d.stencil_dpfail,
            d.stencil_dppass,
            d.stencil_write_mask
        );
        let back = format!(
            "func {} ref {} cmp 0x{:02x} ops {} {} {} write 0x{:02x}",
            d.stencil_back_func,
            d.stencil_back_ref,
            d.stencil_back_cmp_mask,
            d.stencil_back_sfail,
            d.stencil_back_dpfail,
            d.stencil_back_dppass,
            d.stencil_back_write_mask
        );
        check("front_cw", format!("{}", d.front_cw), "false".into());
        check("stencil front", front, face(f));
        check("stencil back", back, face(b));
    }

    check("depth_test", format!("{}", d.depth_test), format!("{}", s.depth_test));
    if s.depth_test {
        check("depth_func", format!("{}", d.depth_func), format!("{}", cmp(s.depth_func)));
        check("depth_write", format!("{}", d.depth_write), format!("{}", s.depth_write));
    }

    check("blend", format!("{}", d.blend), format!("{}", s.blend));
    if s.blend {
        let got = format!(
            "eq {}/{} rgb {}:{} a {}:{}",
            d.blend_eq_rgb, d.blend_eq_alpha, d.blend_src_rgb, d.blend_dst_rgb, d.blend_src_alpha, d.blend_dst_alpha
        );
        let want = format!(
            "eq {}/{} rgb {}:{} a {}:{}",
            equation(s.blend_rgb.equation),
            equation(s.blend_a.equation),
            factor(s.blend_rgb.src_func),
            factor(s.blend_rgb.dst_func),
            factor(s.blend_a.src_func),
            factor(s.blend_a.dst_func)
        );
        check("blend equations and factors", got, want);
        check("blend_color", format!("{:?}", d.blend_color), format!("{:?}", s.blend_color));
    }

    let mask = s.color_mask.iter().enumerate().fold(0u8, |m, (i, &on)| m | (on as u8) << i);
    check("color_mask", format!("0x{:x}", d.color_mask), format!("0x{mask:x}"));
    check("color_write", format!("{}", d.color_write), format!("{}", mask != 0));
    diff
}

/* ---- the cases ---------------------------------------------------------- */

/// Draw case `case`, all its iterations; the failures, empty when every draw
/// was taken with its generated state, and how many draws were compared.
fn run_case(prog: &Program, case: u32) -> (Vec<String>, u32) {
    let mut failures = Vec::new();
    let mut compared = 0;
    for iter in 0..NUM_ITERATIONS_PER_CASE {
        let it = iteration(case, iter);
        // SAFETY: plain GL calls.
        unsafe {
            /* reset default fragment state */
            glDisable(GL_SCISSOR_TEST);
            glColorMask(1, 1, 1, 1);
            glDepthMask(1);
            glStencilMask(!0u32);
            glClearColor(0.25, 0.5, 0.75, 1.0);
            glClearDepthf(1.0);
            glClearStencil(0);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            glViewport(it.viewport_x, it.viewport_y, VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
        }
        for (n, cmd) in it.commands.iter().enumerate() {
            set_gl_state(&cmd.state, it.viewport_x, it.viewport_y);
            DRAWS.with(|v| v.borrow_mut().clear());
            let (e, refused) = render_quad(prog, &cmd.quad, it.viewport_x, it.viewport_y);
            let at = format!("case {case} iteration {iter} quad {n}");
            if e != GL_NO_ERROR || refused != 0 {
                failures.push(format!("{at}: glGetError 0x{e:x}, {refused} refused; {:?}", cmd.state));
                continue;
            }
            let draws: Vec<GlesDraw> =
                DRAWS.with(|v| v.borrow().iter().filter(|d| d.index_count != 0 && !d.clear_quad).copied().collect());
            if draws.len() != 1 {
                failures.push(format!("{at}: {} draws reached the back end; {:?}", draws.len(), cmd.state));
                continue;
            }
            compared += 1;
            let diff = compare(&draws[0], &it, cmd);
            if !diff.is_empty() {
                failures.push(format!("{at}: {}; {:?}", diff.join("; "), cmd.state));
            }
        }
        /* the read-back's flush: the frame is submitted */
        let refused = backend::draws_refused();
        // SAFETY: plain GL calls.
        let e = unsafe {
            glFinish();
            glGetError()
        };
        if e != GL_NO_ERROR || backend::draws_refused() != refused {
            failures.push(format!("case {case} iteration {iter}: at the flush glGetError 0x{e:x}"));
        }
    }
    (failures, compared)
}

/// The port's own check over all 3000 generated commands: every viewport,
/// quad corner and quad size in the ranges dEQP's generator draws from, and
/// the share of states with each test enabled near dEQP's probability, so a
/// slip in the RNG or its floats shows here first.
fn check_generator() {
    let mut cases_with = [0u32; 4]; /* scissor, stencil, depth, blend */
    for case in 0..CASES {
        for iter in 0..NUM_ITERATIONS_PER_CASE {
            let it = iteration(case, iter);
            assert!((0..=RT_SIZE - VIEWPORT_WIDTH).contains(&it.viewport_x));
            assert!((0..=RT_SIZE - VIEWPORT_HEIGHT).contains(&it.viewport_y));
            for c in &it.commands {
                let q = &c.quad;
                for p in [q.pos_a, q.pos_b] {
                    assert!((0..VIEWPORT_WIDTH).contains(&p[0]) && (0..VIEWPORT_HEIGHT).contains(&p[1]));
                }
                assert!((q.pos_a[0] - q.pos_b[0]).abs() + 1 >= VIEWPORT_WIDTH / 2);
                let s = &c.state;
                for (k, on) in [s.scissor_test, s.stencil_test, s.depth_test, s.blend].into_iter().enumerate() {
                    cases_with[k] += on as u32;
                }
            }
        }
    }
    /* 3000 states at dEQP's probabilities 0.2, 0.4, 0.6, 0.4: well inside
     * these bounds unless the floats are wrong */
    let draws = (CASES * NUM_ITERATIONS_PER_CASE as u32 * NUM_CALLS_PER_ITERATION as u32) as f32;
    for (k, p) in [0.2f32, 0.4, 0.6, 0.4].into_iter().enumerate() {
        let got = cases_with[k] as f32 / draws;
        assert!((got - p).abs() < 0.05, "enable {k}: {got} of the states, dEQP's probability {p}");
    }
}

#[test]
fn fragment_random_cases() {
    check_generator();
    let prog = setup();
    let (mut failures, mut compared) = (Vec::new(), 0);
    for case in 0..CASES {
        let (f, n) = run_case(&prog, case);
        failures.extend(f);
        compared += n;
    }
    assert_eq!(backend::draws_refused(), 0, "no draw of the process was refused");
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
    assert_eq!(compared, CASES * NUM_ITERATIONS_PER_CASE as u32 * NUM_CALLS_PER_ITERATION as u32, "draws compared");
}
