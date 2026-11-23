// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Changing only `GL_TEXTURE_BASE_LEVEL` between two V3D draws of one
//! texture (plan #382 item baserebind, issue #1261).
//!
//! Every case draws one IMMUTABLE texture three times through a fragment
//! `textureSize(s, 0)`: at `GL_TEXTURE_BASE_LEVEL` 0, then at 1 — the only
//! thing changed between the draws, no upload and no other state change —
//! then back at 0. Nothing is uploaded or touched between the draws, so
//! the rebind changes nothing the cache could see except the base level.
//!
//! Today the three draws are indistinguishable to the back end:
//!
//! - the draw does not fill `GlesTextureBind::base_level` (src/backend.rs,
//!   offset 3): src/gl/draw.rs:3611 says it outright, "the draw does not
//!   apply GL_TEXTURE_BASE_LEVEL (`GlTexture::base_level`)", so every draw
//!   hands the back end a bind with base_level 0;
//! - `texture::state` (src/v3d/texture.rs) packs the 24-byte texture state
//!   record with `base_level: 0` and `max_level = levels - 1` whatever GL
//!   says (Mesa's v3dx_state.c programs Base Level from the GL base level);
//! - the words 44-47 read `V3dJobTmu`'s `width`/`height`/`depth`/
//!   `array_size`, which `texture_units` (src/v3d/backend.rs) fills from
//!   the LEVEL-0 sizes — tests/es3_texsize_base_level_v3d.rs pinned that
//!   for one draw at BASE_LEVEL 1;
//! - and the key a kept texture copy lives under, `TexKey`
//!   (src/v3d/backend.rs:189: base_gpu, width, height, levels, bgra,
//!   format, content_gen, layers, type_, depth_id), has no base level, so
//!   draw 2 of an immutable texture (one stamp, laid out once) hits draw
//!   1's key exactly and reuses its tiled copy. A stale cache therefore
//!   passes every existing base-level pin: each of them sets the base
//!   level once before its single draw. This file draws, changes only the
//!   base level, and draws again.
//!
//! What the harness sees: each draw's `V3dJobTmu` records as the fake job
//! builder gets them, and byte 15 of each draw's 24-byte packed texture
//! state record in the fake GPU memory (Base Level bits 7:4, Max Level
//! bits 3:0), read as tests/es3_sample_base_level_v3d.rs reads it. So the
//! TMU state's base level IS observable here, and today it is 0 in all
//! three draws.
//!
//! This file pins today's behaviour. The asserts named
//! `..._flip_when_the_base_level_is_applied` are the BASE_LEVEL 1 draw's
//! size words and its record's base level; their doc comments give the
//! post-fix values. After basefix — which fills `GlesTextureBind::base_level`
//! from GL_TEXTURE_BASE_LEVEL, applies the base level to the size words
//! and to `texture::state`, and adds it to `TexKey` so a kept copy follows
//! the base level — draw 2 hands the level-1 words (8, 8, 1, 1),
//! (8, 8, 4, 1) and (8, 8, 1, 5) (a 2D array's layer count is never
//! minified; dEQP es3fShaderTextureFunctionTests.cpp:1408-1411 and :1511,
//! Mesa's v3d_uniforms.c `u_minify(first_level)`) and a record with
//! base_level 1; draws 1 and 3 are the level-0 values today and after.
//!
//! The textures are `glTexStorage` 16x16 (3D 8 deep, halved per level; 2D
//! array 5 layers, the same at every level) with 5 levels, every level
//! written once by a one-texel `glTexSubImage*` before the first draw (a
//! texture never written has `levels_written` 1 and its sampled chain is
//! capped to that), and a NEAREST_MIPMAP_NEAREST min filter, so
//! the record's Max Level shows the chain (4) and a base level applied to
//! it reads as a change; NEAREST alone would sample level 0 only and pack
//! Max Level 0. Nothing is uploaded or changed after the first draw.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_base_level_rebind_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
use yakogl::v3d::backend::yakogl_backend_v3d;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/* ---- the job builder: records each draw's sampler bindings ---- */

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// One draw handed to the job builder: its fragment-stage sampler bindings.
#[derive(Clone)]
struct Draw {
    tmu: Vec<V3dJobTmu>,
}
static DRAWS: Mutex<Vec<Draw>> = Mutex::new(Vec::new());

fn log_lines() -> Vec<String> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn draws() -> Vec<Draw> {
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// `n` records at `p`, or none.
fn records(p: *const V3dJobTmu, n: u32) -> Vec<V3dJobTmu> {
    if p.is_null() || n == 0 {
        Vec::new()
    } else {
        // SAFETY: the draw's `n` records, live for the call.
        unsafe { std::slice::from_raw_parts(p, n as usize) }.to_vec()
    }
}

#[no_mangle]
unsafe extern "C" fn v3d_job_begin(
    j: *mut V3dJob,
    t: *const V3dJobTarget,
    bcl: *mut V3dCl,
    rcl: *mut V3dCl,
    ind: *mut V3dCl,
) -> bool {
    // SAFETY: the backend passes its live job, target and lists.
    let (j, t) = unsafe { (&mut *j, *t) };
    *j = V3dJob { bcl, rcl, ind, t, draws: 0, defaults: 0, broken: false, pack: V3dJobPack::ZERO };
    // SAFETY: as above.
    unsafe {
        (*bcl).used = 16;
        (*ind).used = 256;
    }
    t.width != 0 && t.height != 0
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
    let d = unsafe { &*d };
    let draw = Draw { tmu: records(d.tmu, d.num_tmu) };
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).push(draw);
    // SAFETY: the backend passes its live job.
    unsafe {
        (*j).draws += 1;
        (*(*j).bcl).used += 120;
    }
    V3D_JOB_OK
}

#[no_mangle]
unsafe extern "C" fn v3d_job_end(
    j: *mut V3dJob,
    _c: *const V3dJobClear,
    _tile_list_start: *mut u32,
    _tile_list_end: *mut u32,
) -> bool {
    // SAFETY: the backend passes its live job; its lists are live.
    unsafe {
        (*(*j).bcl).used += 1;
        (*(*j).rcl).used = 200;
    }
    true
}

#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling(width: u32, height: u32, t: *mut V3dTiling) {
    // SAFETY: the backend's out-parameter, passed on.
    unsafe { v3d_frame_tiling_bpp(width, height, 1, false, 0, t) }
}

#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling_rts(width: u32, height: u32, rts: u32, t: *mut V3dTiling) {
    // SAFETY: the backend's out-parameter, passed on.
    unsafe { v3d_frame_tiling_bpp(width, height, rts, false, 0, t) }
}

#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling_ms(width: u32, height: u32, rts: u32, msaa: bool, t: *mut V3dTiling) {
    // SAFETY: the backend's out-parameter, passed on.
    unsafe { v3d_frame_tiling_bpp(width, height, rts, msaa, 0, t) }
}

/// The tile-size table of v3d_frame.c, as the backend's unit tests fake it.
#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling_bpp(
    width: u32,
    height: u32,
    rts: u32,
    msaa: bool,
    max_bpp: u32,
    t: *mut V3dTiling,
) {
    const SIZES: [(u32, u32); 7] = [(64, 64), (64, 32), (32, 32), (32, 16), (16, 16), (16, 8), (8, 8)];
    let idx = match rts {
        0 | 1 => 0,
        2 => 1,
        _ => 2,
    } + if msaa { 2 } else { 0 }
        + max_bpp.min(2) as usize;
    let (tw, th) = SIZES[idx];
    let tiles = V3dTiling {
        tile_w: tw,
        tile_h: th,
        tiles_x: width.div_ceil(tw),
        tiles_y: height.div_ceil(th),
        supertile_w: 1,
        supertile_h: 1,
        frame_w_supertiles: width.div_ceil(tw),
        frame_h_supertiles: height.div_ceil(th),
    };
    // SAFETY: the backend's out-parameter.
    unsafe { *t = tiles };
}

#[no_mangle]
unsafe extern "C" fn v3d_frame_alloc_sizes(t: *const V3dTiling, _d: u32, _page: u32, ta: *mut u32, ts: *mut u32) {
    // SAFETY: the backend's parameters.
    unsafe {
        let n = (*t).tiles_x * (*t).tiles_y;
        *ta = n * 128 + 8192;
        *ts = n * 256;
    }
}

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// `driver/src/v3d/v3d_tex.c` and the Mesa tiling it includes, as a shared
/// object of this test's own, reached through the handle only (RTLD_LOCAL).
fn lib() -> *mut c_void {
    static L: OnceLock<usize> = OnceLock::new();
    *L.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_base_level_rebind_v3d");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_tex-{}.so", std::process::id()));
        /* the compiler's own temporary files next to the object */
        let st = Command::new("cc")
            .current_dir(&root)
            .env("TMPDIR", &dir)
            .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
            .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
            .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
            .arg("-Ithird_party/mesa-26.2.1/include")
            .args(["driver/src/v3d/v3d_tex.c", "driver/src/v3d/v3d_tiling_mesa.c", "-lm", "-o"])
            .arg(&so)
            .status()
            .expect("cc runs");
        assert!(st.success(), "driver/src/v3d/v3d_tex.c builds");
        let path = CString::new(so.to_str().expect("a utf-8 path")).unwrap();
        // SAFETY: a path of a library this test just built; RTLD_NOW.
        let h = unsafe { dlopen(path.as_ptr(), 2) };
        assert!(!h.is_null(), "the layout code loads");
        let _ = std::fs::remove_file(&so);
        h as usize
    }) as *mut c_void
}

fn sym(name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    // SAFETY: a handle `lib` made and a NUL-terminated name.
    let p = unsafe { dlsym(lib(), c.as_ptr()) };
    assert!(!p.is_null(), "{name} in v3d_tex.c");
    p
}

/// The trampolines: the library's `extern "C"` texture functions
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

    trampoline!(v3d_tex_layout(width: u32, height: u32, levels: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
    trampoline!(v3d_tex_layout_msaa(width: u32, height: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
    trampoline!(v3d_tex_store_level(image: *mut u8, l: *const V3dTexLayout, level: u32, src: *const u8, stride: u32));
    trampoline!(v3d_tex_tfu_pack(l: *const V3dTexLayout, ttype: u32, src: u32, stride: u32, dst: u32,
                                 j: *mut V3dTexTfuJob) -> bool);
    trampoline!(v3d_tex_tfu_pack_mipmap(l: *const V3dTexLayout, ttype: u32, image: u32, j: *mut V3dTexTfuJob) -> bool);
    trampoline!(v3d_tex_layer_stride(l: *const V3dTexLayout) -> u32);
    trampoline!(v3d_tex_layers_size(l: *const V3dTexLayout, layers: u32, out: *mut u32) -> bool);
    trampoline!(v3d_tex_store_box(image: *mut u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                  src: *const u8, stride: u32) -> bool);
    trampoline!(v3d_tex_load_box(image: *const u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                 dst: *mut u8, stride: u32) -> bool);
    trampoline!(v3d_tex_state_from_layout(s: *mut V3dTexState, l: *const V3dTexLayout));
    trampoline!(v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool);
    trampoline!(v3d_tex_pack_sampler(out: *mut u8, s: *const V3dSampler) -> bool);
    trampoline!(v3d_tex_pack_sampler_compare(out: *mut u8, s: *const V3dSampler, compare_op: u32) -> bool);
}

/* ---- the platform: memory that answers, jobs that finish at once ---- */

struct Alloc {
    mem: Box<[u64]>,
    size: usize,
}
struct Platform {
    next: u64,
    allocs: BTreeMap<u64, Alloc>,
    submitted: usize,
}
static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(|| Platform { next: 0, allocs: BTreeMap::new(), submitted: 0 }))
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
unsafe extern "C" fn p_submit(_kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<V3dKick>() {
        return -22;
    }
    with_platform(|p| {
        p.submitted += 1;
        p.submitted as i64
    })
}
unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}
unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.submitted as i64)
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
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push(s);
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

/* ---- the context ---- */

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The one context and back end of the module; the tests take turns and
/// each starts with nothing recorded and nothing pending. The lock also
/// serialises the interpreter's own compiles with the GL core's.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the module.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_v3d;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", log_lines());
            assert!(!context::gl_context_create(32, 32).is_null(), "{:?}", log_lines());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    g
}

/* ---- helpers ---- */

use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::texture::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};

/// The error flag, drained: the first error since the last call.
fn err() -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        let e = glGetError();
        while glGetError() != GL_NO_ERROR {}
        e
    }
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text} did not compile");
        s
    }
}

/// The plain vertex shader of the fragment cases.
const VS: &str = "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\nout highp vec4 v;\n\
                  void main() { v = a_pos; gl_Position = a_pos; }\n";

/// Make `vs` + `fs` the current program, its sampler `s` on unit 0.
fn program(vs: &str, fs: &str) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls; NUL-terminated names.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{vs}{fs} did not link");
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glDeleteShader(v);
        glDeleteShader(f);
        p
    }
}

/// A full-screen triangle into the default framebuffer: the GL error it
/// raises.
fn draw_triangle() -> GLenum {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
    err()
}

/// The three draws since `rig`, once the jobs are done, in order.
fn the_three_draws() -> Vec<Draw> {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    let d = draws();
    assert_eq!(d.len(), 3, "three draws reached the job builder: {:?}", log_lines());
    d
}

/* ---- the textures: immutable 16x16 (3D 8 deep, array 5 layers), 5 levels ---- */

/// Levels 0..=4: 16, 8, 4, 2, 1 texels on a side; the 3D depth 8, 4, 2, 1,
/// 1; an array's 5 layers the same at every level.
const SIZE: usize = 16;
const DEPTH: usize = 8;
const LAYERS: usize = 5;
const LEVELS: GLsizei = 5;
/// The record's Max Level with the whole chain sampled: LEVELS - 1.
const MAX_LEVEL: u32 = 4;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Tex2d,
    Tex3d,
    Array,
}

impl Kind {
    fn target(self) -> GLenum {
        match self {
            Kind::Tex2d => GL_TEXTURE_2D,
            Kind::Tex3d => GL_TEXTURE_3D,
            Kind::Array => GL_TEXTURE_2D_ARRAY,
        }
    }
    /// The sampler type of the shader and the `textureSize` it asks.
    fn sampler(self) -> (&'static str, &'static str) {
        match self {
            Kind::Tex2d => ("sampler2D", "vec3(vec2(textureSize(s, 0)), 0.0)"),
            Kind::Tex3d => ("sampler3D", "vec3(textureSize(s, 0))"),
            Kind::Array => ("sampler2DArray", "vec3(textureSize(s, 0))"),
        }
    }
}

/// One RGBA8 texel.
static TEXEL: [u8; 4] = [0x40, 0x60, 0x80, 0xFF];

/// An IMMUTABLE `kind` texture bound on unit 0 (`glTexStorage2D/3D`, every
/// level then written once by a one-texel `glTexSubImage*`),
/// NEAREST_MIPMAP_NEAREST / NEAREST, and `GL_TEXTURE_BASE_LEVEL` `base`.
/// The tests change `base` and nothing else between their draws.
///
/// The per-level `glTexSubImage*` calls matter for what the record shows:
/// a texture whose texels were never written has `levels_written` 1
/// (src/gl/texture.rs, `texture_alloc_laid_out`; a `glTexSubImage*` with
/// null pixels returns before it records the level), `sampling_fields`
/// (src/gl/draw.rs) caps the sampled chain to that, and the packed record
/// comes out at Max Level 0. Writing every level once makes the whole chain
/// sampled, so the record's Max Level is `MAX_LEVEL` and a base level
/// applied to it reads as a change of the record. The uploads all happen
/// once, before the first draw; nothing is uploaded between the draws.
fn texture(kind: Kind, base: GLint) -> GLuint {
    let target = kind.target();
    let mut t = 0;
    // SAFETY: plain GL calls; TEXEL outlives each upload.
    unsafe {
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(target, t);
        match kind {
            Kind::Tex2d => glTexStorage2D(target, LEVELS, GL_RGBA8, SIZE as GLsizei, SIZE as GLsizei),
            Kind::Tex3d => glTexStorage3D(target, LEVELS, GL_RGBA8, SIZE as GLsizei, SIZE as GLsizei, DEPTH as GLsizei),
            Kind::Array => {
                glTexStorage3D(target, LEVELS, GL_RGBA8, SIZE as GLsizei, SIZE as GLsizei, LAYERS as GLsizei)
            }
        }
        let p = TEXEL.as_ptr() as *const c_void;
        for level in 0..LEVELS {
            match kind {
                Kind::Tex2d => glTexSubImage2D(target, level, 0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, p),
                Kind::Tex3d | Kind::Array => {
                    glTexSubImage3D(target, level, 0, 0, 0, 1, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, p)
                }
            }
        }
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(target, GL_TEXTURE_BASE_LEVEL, base);
    }
    assert_eq!(err(), GL_NO_ERROR, "the storage of {kind:?}: {:?}", log_lines());
    t
}

/// What one draw handed the job builder: the unit 0 `V3dJobTmu`'s
/// `(width, height, depth, array_size)` — what uniform words 44-47 read —
/// and the `(base_level, max_level)` of its 24-byte texture state record.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Seen {
    words: (u32, u32, u32, u32),
    levels: (u32, u32),
}

/// The `(base_level, max_level)` fields of the 24-byte texture state
/// record at GPU address `tex_state`: byte 15, Base Level bits 7:4 and
/// Max Level bits 3:0 (the record `v3d_tex_pack_state` writes, as
/// tests/es3_sample_base_level_v3d.rs reads it).
fn record_levels(tex_state: u32) -> (u32, u32) {
    let b15 = with_platform(|p| {
        let gpu = u64::from(tex_state);
        let (base_addr, a) = p.allocs.range(..=gpu).next_back().expect("an allocation below the record");
        let off = (gpu - base_addr) as usize;
        assert!(off + V3D_TEX_STATE_BYTES <= a.size, "the record lies in its allocation");
        // SAFETY: inside the allocation (checked above).
        unsafe { *(a.mem.as_ptr() as *const u8).add(off + 15) }
    });
    (u32::from(b15 >> 4), u32::from(b15 & 15))
}

/// The three draws of one immutable `kind` texture with a fragment
/// `textureSize(s, 0)`, in order: BASE_LEVEL 0, then BASE_LEVEL 1 changed
/// with a single glTexParameteri — no upload, no other state change —
/// then back at 0. Every draw is accepted (no GL error, no refusal), the
/// unit has its texture record, and each draw's `Seen` is recorded; the
/// texture and program are deleted only after the records were read.
fn rebind(kind: Kind) -> [Seen; 3] {
    let target = kind.target();
    let t = texture(kind, 0);
    let (ty, query) = kind.sampler();
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp {ty};\nuniform {ty} s;\nin highp vec4 v;\n\
         out vec4 o;\nvoid main() {{ o = vec4({query}, 1.0); }}\n"
    );
    let p = program(VS, &fs);

    // Draw 1: BASE_LEVEL 0, as the texture was made.
    let e1 = draw_triangle();
    // Only GL_TEXTURE_BASE_LEVEL changes.
    // SAFETY: a plain GL call on the bound texture.
    unsafe { glTexParameteri(target, GL_TEXTURE_BASE_LEVEL, 1) };
    let e2 = draw_triangle();
    // And back to 0.
    // SAFETY: a plain GL call on the bound texture.
    unsafe { glTexParameteri(target, GL_TEXTURE_BASE_LEVEL, 0) };
    let e3 = draw_triangle();

    let lines = log_lines();
    for (n, e) in [(1, e1), (2, e2), (3, e3)] {
        assert_eq!(e, GL_NO_ERROR, "{kind:?} draw {n}: the draw is accepted: {lines:?}");
    }
    assert!(!lines.iter().any(|l| l.contains("refused")), "{kind:?}: {lines:?}");

    let ds = the_three_draws();
    let mut seen = [Seen { words: (0, 0, 0, 0), levels: (0, 0) }; 3];
    for (n, d) in ds.iter().enumerate() {
        let u = d
            .tmu
            .iter()
            .find(|u| u.unit == 0)
            .unwrap_or_else(|| panic!("{kind:?} draw {n}: unit 0 bound: {:?}", d.tmu));
        assert_ne!(u.tex_state, 0, "{kind:?} draw {n}: the unit has its texture record");
        seen[n] = Seen { words: (u.width, u.height, u.depth, u.array_size), levels: record_levels(u.tex_state) };
    }
    // SAFETY: plain GL calls.
    unsafe {
        glDeleteProgram(p);
        glDeleteTextures(1, &t);
    }
    seen
}

/* ---- BASE_LEVEL 0 before and after the change: the controls ---- */

/// Control: the 16x16 `sampler2D` drawn at BASE_LEVEL 0 hands (16, 16, 1, 1)
/// both before the BASE_LEVEL 1 draw and after it is drawn, and the two
/// BASE_LEVEL 0 draws hand identical words. Holds today and after basefix.
#[test]
fn the_two_2d_draws_at_base_level_0_around_the_base_level_change_hand_identical_level_0_words() {
    let _g = rig();
    let seen = rebind(Kind::Tex2d);
    assert_eq!(seen[0].words, (16, 16, 1, 1));
    assert_eq!(seen[2].words, seen[0].words);
}

/// Control: the 16x16x8 `sampler3D` at BASE_LEVEL 0: (16, 16, 8, 1) before
/// the BASE_LEVEL 1 draw, and the same again after it. Holds today and
/// after basefix.
#[test]
fn the_two_3d_draws_at_base_level_0_around_the_base_level_change_hand_identical_level_0_words() {
    let _g = rig();
    let seen = rebind(Kind::Tex3d);
    assert_eq!(seen[0].words, (16, 16, 8, 1));
    assert_eq!(seen[2].words, seen[0].words);
}

/// Control: the 16x16x5-layer `sampler2DArray` at BASE_LEVEL 0: (16, 16, 1,
/// 5) before the BASE_LEVEL 1 draw, and the same again after it. Holds
/// today and after basefix.
#[test]
fn the_two_2d_array_draws_at_base_level_0_around_the_base_level_change_hand_identical_level_0_words() {
    let _g = rig();
    let seen = rebind(Kind::Array);
    assert_eq!(seen[0].words, (16, 16, 1, 5));
    assert_eq!(seen[2].words, seen[0].words);
}

/* ---- BASE_LEVEL 1 between the draws: today, pinned ---- */

/// The `sampler2D` drawn between the two BASE_LEVEL 0 draws, its BASE_LEVEL
/// the only thing that changed, hands the level-1 size (8, 8, 1, 1)
/// (dEQP es3fShaderTextureFunctionTests.cpp:1408-1411, :1511).
#[test]
fn the_2d_draw_at_base_level_1_between_the_draws_hands_the_level_1_words() {
    let _g = rig();
    let seen = rebind(Kind::Tex2d);
    assert_eq!(seen[1].words, (8, 8, 1, 1));
}

/// The `sampler3D` re-drawn at BASE_LEVEL 1 hands (8, 8, 4, 1): width,
/// height and depth minified.
#[test]
fn the_3d_draw_at_base_level_1_between_the_draws_hands_the_level_1_words() {
    let _g = rig();
    let seen = rebind(Kind::Tex3d);
    assert_eq!(seen[1].words, (8, 8, 4, 1));
}

/// The `sampler2DArray` re-drawn at BASE_LEVEL 1 hands (8, 8, 1, 5): width and
/// height minified, the layer count as is.
#[test]
fn the_2d_array_draw_at_base_level_1_between_the_draws_hands_the_level_1_words() {
    let _g = rig();
    let seen = rebind(Kind::Array);
    assert_eq!(seen[1].words, (8, 8, 1, 5));
}

/* ---- the TMU record: today's base level 0, pinned ---- */

/// Control: the BASE_LEVEL 0 draws of the `sampler2D` read a texture state
/// record at Base Level 0, Max Level 4 (the whole chain), the first and the
/// third draw alike. Holds today and after basefix.
#[test]
fn the_two_2d_draws_at_base_level_0_around_the_base_level_change_read_the_tmu_record_at_base_level_0() {
    let _g = rig();
    let seen = rebind(Kind::Tex2d);
    assert_eq!(seen[0].levels, (0, MAX_LEVEL));
    assert_eq!(seen[2].levels, seen[0].levels);
}

/// Control: the `sampler3D` BASE_LEVEL 0 draws read Base Level 0, Max Level
/// 4, alike. Holds today and after basefix.
#[test]
fn the_two_3d_draws_at_base_level_0_around_the_base_level_change_read_the_tmu_record_at_base_level_0() {
    let _g = rig();
    let seen = rebind(Kind::Tex3d);
    assert_eq!(seen[0].levels, (0, MAX_LEVEL));
    assert_eq!(seen[2].levels, seen[0].levels);
}

/// Control: the `sampler2DArray` BASE_LEVEL 0 draws read Base Level 0, Max
/// Level 4, alike. Holds today and after basefix.
#[test]
fn the_two_2d_array_draws_at_base_level_0_around_the_base_level_change_read_the_tmu_record_at_base_level_0() {
    let _g = rig();
    let seen = rebind(Kind::Array);
    assert_eq!(seen[0].levels, (0, MAX_LEVEL));
    assert_eq!(seen[2].levels, seen[0].levels);
}

/// Pins today's behaviour: the `sampler2D` drawn between them, at BASE_LEVEL
/// 1, still reads a record at Base Level 0 — `texture::state` hardcodes it,
/// and the draw hands a bind whose base_level is 0 (src/gl/draw.rs:3611).
/// After the base level is applied the record is **(1, 4)** (Mesa's
/// v3dx_state.c programs Base Level from the GL base level; TexKey then
/// keeps the base level too, so the draw packs its own record instead of
/// reusing the earlier draw's). Flip the assert to (1, MAX_LEVEL) then.
#[test]
fn the_2d_draw_at_base_level_1_between_the_draws_still_reads_the_tmu_record_at_base_level_0_flip_when_the_base_level_is_applied(
) {
    let _g = rig();
    let seen = rebind(Kind::Tex2d);
    assert_eq!(seen[1].levels, (0, MAX_LEVEL));
}

/// Pins today's behaviour: the `sampler3D` at BASE_LEVEL 1 between the
/// draws reads Base Level 0 too. After the base level is applied the record
/// is **(1, 4)**. Flip the assert to (1, MAX_LEVEL) then.
#[test]
fn the_3d_draw_at_base_level_1_between_the_draws_still_reads_the_tmu_record_at_base_level_0_flip_when_the_base_level_is_applied(
) {
    let _g = rig();
    let seen = rebind(Kind::Tex3d);
    assert_eq!(seen[1].levels, (0, MAX_LEVEL));
}

/// Pins today's behaviour: the `sampler2DArray` at BASE_LEVEL 1 between the
/// draws reads Base Level 0 too. After the base level is applied the record
/// is **(1, 4)**. Flip the assert to (1, MAX_LEVEL) then.
#[test]
fn the_2d_array_draw_at_base_level_1_between_the_draws_still_reads_the_tmu_record_at_base_level_0_flip_when_the_base_level_is_applied(
) {
    let _g = rig();
    let seen = rebind(Kind::Array);
    assert_eq!(seen[1].levels, (0, MAX_LEVEL));
}
