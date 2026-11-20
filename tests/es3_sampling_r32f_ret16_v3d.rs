// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `sampler3D` and `sampler2DArray` units that really sample an R32F
//! texture on the V3D (plan #382 item r16samp, issue #1261): the rule
//! 080240bc left standing when it exempted size-only units. A bound unit a
//! look-up reads (a TMU P0/P1 word names it) must have the program's return
//! size agree with the format's (`texture::refusal_for`, RET16_OF_32 in
//! src/v3d/texture.rs); a unit only `textureSize` asks of has no TMU output
//! to agree with and is exempt (tests/es3_shadow_texsize_v3d.rs,
//! tests/es3_shadow_array_texsize_v3d.rs). This file pins that rule for the
//! two sampler types the size-only half (item r16plain) opens up.
//!
//! What the plan expected — a sampling `texture()` draw of an R32F 3D or
//! 2D-array texture refused at the draw with RET16_OF_32 — does not happen
//! on main, and was checked before pinning: a sampling draw never reaches
//! the 16-bit key over a 32-bit texture. `v3d_sensitivity`
//! (src/shaderc/online/backend.rs) marks every look-up binding below 16 of
//! either stage (the bindings the key carries: CAP_SAMPLERS 16, pinned by
//! its `the_lod_key_layout_is_pinned`), the GL core's `stage_lod_key`
//! (src/gl/draw.rs) sets `LodKey::ret32` for such a binding from the bound
//! texture's format, and the compiler then writes TMU Config P1 bit 0 for it
//! (select.rs; the vertex stage since the v3d-qpu-grad fix,
//! tests/es3_vs_ret32_v3d.rs, docs/notes/v3d-vs-ret32.md). So the four draws
//! here (sampler3D and sampler2DArray, fragment and vertex stage, over an
//! R32F texture) are accepted, each with its unit 0 P1 word carrying the
//! bit — `texture_units` (src/v3d/backend.rs) sees a 32-bit look-up and the
//! R32F return size agrees. That is the behaviour the rule intends: the
//! refusal fires on a 16-bit *key*, and for a consistent draw the core
//! never builds one. tests/es3_shadow_array_texsize_v3d.rs says the same
//! for the 2D array's shadow look-ups ("no draw reaches the 16-bit key over
//! R32F"), and its `an_array_shadow_look_up_of_the_same_array_still_draws`
//! accepts the matching look-up draw; this file carries the pin across to
//! plain 3D and 2D-array samplers, both stages. RET16_OF_32 itself is
//! pinned at the predicate level, as those files pin it:
//! `refusal_for(bind, false)` is the constant verbatim for a 3D and a 2D
//! array binding, `refusal_for(bind, true)` accepts.
//!
//! The control (the same fragment shader over an RGBA8 3D texture) is
//! keyed the other way: its P1 word has bit 0 clear, the 16-bit output the
//! RGBA8 return size agrees with — the key follows the bound format, and no
//! draw meets RET16_OF_32 either way. A second observation this file pins:
//! the streams of these `texture()` look-ups hold no `QUNIFORM_TEXTURE_DEPTH`
//! or `_ARRAY_SIZE` word — words 46/47, which the job builder does not write
//! (issue #1261, tests/es3_texsize3d_v3d.rs), are asked only by `textureSize`
//! and `textureGrad`. So on main the job builder's gap for them gates the
//! 3D/array *size* draws (dEQP S32a/S32b), not these sampling draws. The
//! job builder here is stood in for, as in the shadow files: it records the
//! draw's bindings and streams, and writes no uniform words.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_sampling_r32f_ret16_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::backend::{GlesTextureBind, GLES_TEXFMT_R32F, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D_ARRAY, GLES_TEXTYPE_3D};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::ir::Op;
use yakogl::shaderc::online::v3d::select::{QUNIFORM_TEXTURE_ARRAY_SIZE, QUNIFORM_TEXTURE_DEPTH};
use yakogl::shaderc::online::{lower, sema};
use yakogl::v3d::backend::yakogl_backend_v3d;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;
use yakogl::v3d::texture;

/* ---- the job builder: records each draw's sampler bindings ---- */

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// One draw handed to the job builder: its FS and VS sampler bindings, and
/// the uniform streams of its fragment, vertex and coordinate programs (the
/// words the C job builder would write, kept as `(contents, data)`).
#[derive(Clone)]
struct Draw {
    tmu: Vec<V3dJobTmu>,
    vs_tmu: Vec<V3dJobTmu>,
    fs: Vec<V3dUniform>,
    vs: Vec<V3dUniform>,
    cs: Vec<V3dUniform>,
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

/// `count` uniform words at `p`, or none (the C reads the program streams
/// through `v3d_job_draw`'s `fs`/`vs`/`cs`; the fake copies them).
fn words(p: *const V3dUniform, n: u32) -> Vec<V3dUniform> {
    if p.is_null() || n == 0 {
        Vec::new()
    } else {
        // SAFETY: a program's stream of `n` words, live for the call.
        unsafe { std::slice::from_raw_parts(p, n as usize) }.to_vec()
    }
}

/// A fragment program's stream, or none for a null record.
fn fs_words(p: *const V3dFsProg) -> Vec<V3dUniform> {
    if p.is_null() {
        Vec::new()
    } else {
        // SAFETY: the draw's live program record.
        let p = unsafe { &*p };
        words(p.uniforms, p.num_uniforms)
    }
}

/// A vertex or coordinate program's stream, or none for a null record.
fn vs_prog_words(p: *const V3dVsProg) -> Vec<V3dUniform> {
    if p.is_null() {
        Vec::new()
    } else {
        // SAFETY: the draw's live program record.
        let p = unsafe { &*p };
        words(p.uniforms, p.num_uniforms)
    }
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
    let d = unsafe { &*d };
    let draw = Draw {
        tmu: records(d.tmu, d.num_tmu),
        vs_tmu: records(d.vs_tmu, d.num_vs_tmu),
        fs: fs_words(d.fs),
        vs: vs_prog_words(d.vs),
        cs: vs_prog_words(d.cs),
    };
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_sampling_r32f_ret16_v3d");
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

/// RET16_OF_32, verbatim (src/v3d/texture.rs).
const RET16_OF_32: &str = "a texture of 32-bit channels (R32F, RGBA32F, RGBA32UI, RGBA32I) read by a look-up \
                           compiled for the 16-bit TMU output";

/// The number of `Op::Tex` instructions of a fragment shader reading
/// `sampler` `s` in `body`.
fn tex_ops(sampler: &str, body: &str) -> usize {
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp {sampler};\nuniform {sampler} s;\n\
         in highp vec3 v_texCoord;\nlayout(location = 0) out highp vec4 o;\nvoid main() {{ {body} }}\n"
    );
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let fs = a.alloc_str(&fs).unwrap();
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).unwrap_or_else(|e| panic!("fragment: {}", e.msg));
    f.module.instrs.iter().filter(|op| matches!(op, Op::Tex { .. })).count()
}

/// An R32F texture `type_` with `layers` layers/slices: the binding a draw
/// hands the back end for the textures below.
fn r32_bind(type_: u8, layers: u16) -> GlesTextureBind {
    GlesTextureBind {
        format: GLES_TEXFMT_R32F,
        type_,
        layers,
        base_gpu: 0x1_0000_0000,
        width: 4,
        height: 4,
        stride_texels: 4,
        bytes: 4 * 4 * 4 * layers as u32,
        mip_levels: 1,
        ..GlesTextureBind::ZERO
    }
}

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

const PASS_FS: &str =
    "#version 300 es\nprecision highp float;\nin highp vec4 v;\nout vec4 o;\nvoid main() { o = v; }\n";

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

/// A `w` x `h` x `d` texture on unit 0: an R32F or an RGBA8 `GL_TEXTURE_3D`
/// or `GL_TEXTURE_2D_ARRAY`, NEAREST, one level.
fn upload(target: GLenum, r32: bool, w: usize, h: usize, d: usize) -> GLuint {
    let mut t = 0;
    // SAFETY: plain GL calls; the texels outlive the upload.
    unsafe {
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(target, t);
        if r32 {
            let texels: Vec<f32> = (0..w * h * d).map(|i| i as f32 / (w * h * d) as f32).collect();
            glTexImage3D(
                target,
                0,
                GL_R32F as GLint,
                w as GLsizei,
                h as GLsizei,
                d as GLsizei,
                0,
                GL_RED,
                GL_FLOAT,
                texels.as_ptr() as *const c_void,
            );
        } else {
            let texels: Vec<u8> = (0..w * h * d * 4).map(|i| (i % 251) as u8).collect();
            glTexImage3D(
                target,
                0,
                GL_RGBA8 as GLint,
                w as GLsizei,
                h as GLsizei,
                d as GLsizei,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                texels.as_ptr() as *const c_void,
            );
        }
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
    }
    assert_eq!(err(), GL_NO_ERROR, "the upload: {:?}", log_lines());
    t
}

/// The draw, its GL error, the log: what the back end did with a unit 0
/// `target` texture (`r32` its format) sampled by `vs` + `fs`.
fn sample_draw(target: GLenum, r32: bool, vs: &str, fs: &str) -> (GLenum, Vec<String>) {
    let t = upload(target, r32, 4, 4, 3);
    let p = program(vs, fs);
    let e = draw_triangle();
    let lines = log_lines();
    // SAFETY: plain GL calls.
    unsafe {
        glDeleteProgram(p);
        glDeleteTextures(1, &t);
    }
    (e, lines)
}

/* ---- the rule at the predicate level ---- */

#[test]
fn a_16_bit_look_up_of_a_32_bit_3d_or_array_texture_is_refused_ret16_of_32() {
    for (what, b) in [("3D", r32_bind(GLES_TEXTYPE_3D, 3)), ("2D array", r32_bind(GLES_TEXTYPE_2D_ARRAY, 4))] {
        let why = texture::refusal_for(&b, false).expect("refused");
        assert_eq!(why, RET16_OF_32, "{what}");
        assert!(texture::GAPS.contains(&why), "{what}: a backend gap: -ENOTSUP, not -EINVAL");
        assert_eq!(texture::refusal_for(&b, true), None, "{what}: the 32-bit key accepts the texture");
        assert!(texture::return_32(&b), "{what}: an R32F texture returns 32 bits");
    }
}

#[test]
fn a_16_bit_format_is_not_refused_that_way() {
    for (what, b) in [("3D", r32_bind(GLES_TEXTYPE_3D, 3)), ("2D array", r32_bind(GLES_TEXTYPE_2D_ARRAY, 4))] {
        let rgba8 = GlesTextureBind { format: GLES_TEXFMT_RGBA8, ..b };
        assert_eq!(texture::refusal_for(&rgba8, false), None, "{what}");
    }
}

#[test]
fn a_sampling_look_up_of_a_3d_or_array_sampler_is_a_tex() {
    assert_eq!(tex_ops("sampler3D", "o = texture(s, v_texCoord);"), 1, "sampler3D texture()");
    assert_eq!(tex_ops("sampler2DArray", "o = texture(s, v_texCoord);"), 1, "sampler2DArray texture()");
}

/* ---- the draws ---- */

/// The one draw since `rig`, once the job is done.
fn the_draw() -> Draw {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    let d = draws();
    assert_eq!(d.len(), 1, "one draw reached the job builder: {:?}", log_lines());
    d.into_iter().next().unwrap()
}

/// The `V3D_QUNIFORM_TMU_CONFIG_P1` word naming `unit` in any of `streams`,
/// if the program has one.
fn p1_of(streams: [&[V3dUniform]; 3], unit: u32) -> Option<u32> {
    streams
        .iter()
        .flat_map(|s| s.iter())
        .find(|w| w.contents == V3D_QUNIFORM_TMU_CONFIG_P1 && w.data >> 24 == unit)
        .map(|w| w.data)
}

const FS3D: &str = "#version 300 es\nprecision highp float;\nuniform highp sampler3D s;\nin highp vec4 v;\n\
                    out vec4 o;\nvoid main() { o = texture(s, v.xyz); }\n";
const VS3D: &str = "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\nuniform highp sampler3D s;\n\
                    out highp vec4 v;\nvoid main() { v = texture(s, a_pos.xyz); gl_Position = a_pos; }\n";
const FSARRAY: &str = "#version 300 es\nprecision highp float;\nuniform highp sampler2DArray s;\nin highp vec4 v;\n\
                        out vec4 o;\nvoid main() { o = texture(s, vec3(v.xy, 0.0)); }\n";
const VSARRAY: &str = "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\nuniform highp sampler2DArray s;\n\
                        out highp vec4 v;\nvoid main() { v = texture(s, vec3(a_pos.xy, 0.0)); gl_Position = a_pos; }\n";

/// A fragment-stage look-up: the stream of the FS program has P0 and P1
/// words for unit 0, keyed 32 bits (`bit0`), and unit 0 reached the job
/// builder's FS table with its record and its 4x4 base level.
fn looked_up_fs(d: &Draw, bit0: u32) {
    let c: Vec<u32> = d.fs.iter().map(|w| w.contents).collect();
    assert!(
        c.contains(&V3D_QUNIFORM_TMU_CONFIG_P0) && c.contains(&V3D_QUNIFORM_TMU_CONFIG_P1),
        "the look-up writes P0 and P1: {c:?}"
    );
    let p1 = p1_of([&d.fs, &d.vs, &d.cs], 0).expect("a P1 word for unit 0");
    assert_eq!(p1 & 1, bit0, "TMU Config P1, Output Type bit: {p1:#010x}");
    let u = d.tmu.iter().find(|u| u.unit == 0).unwrap_or_else(|| panic!("unit 0 bound: {:?}", d.tmu));
    assert_ne!(u.tex_state, 0, "the unit has its texture record");
    assert_eq!((u.width, u.height), (4, 4), "the base level the size words read");
}

/// A vertex-stage look-up: the same in the VS streams and the VS table.
fn looked_up_vs(d: &Draw, bit0: u32) {
    let c: Vec<u32> = d.vs.iter().chain(d.cs.iter()).map(|w| w.contents).collect();
    assert!(
        c.contains(&V3D_QUNIFORM_TMU_CONFIG_P0) && c.contains(&V3D_QUNIFORM_TMU_CONFIG_P1),
        "the look-up writes P0 and P1: {c:?}"
    );
    let p1 = p1_of([&d.fs, &d.vs, &d.cs], 0).expect("a P1 word for unit 0");
    assert_eq!(p1 & 1, bit0, "TMU Config P1, Output Type bit: {p1:#010x}");
    let u = d.vs_tmu.iter().find(|u| u.unit == 0).unwrap_or_else(|| panic!("VS unit 0 bound: {:?}", d.vs_tmu));
    assert_ne!(u.tex_state, 0, "the unit has its texture record");
    assert_eq!((u.width, u.height), (4, 4), "the base level the size words read");
}

/// No `texture()` look-up asks a depth or a layer count: the streams of
/// these draws hold no word 46/47, so the job builder's gap for them
/// (tests/es3_texsize3d_v3d.rs, issue #1261) is not what these draws meet
/// on main — only `textureSize` and `textureGrad` emit those words.
fn no_depth_or_array_words(d: &Draw) {
    for (what, s) in [("fs", &d.fs), ("vs", &d.vs), ("cs", &d.cs)] {
        let c: Vec<u32> = s.iter().map(|w| w.contents).collect();
        assert!(
            !c.contains(&QUNIFORM_TEXTURE_DEPTH) && !c.contains(&QUNIFORM_TEXTURE_ARRAY_SIZE),
            "{what} asks a depth or layer count: {c:?}"
        );
    }
}

/// A look-up of a 32-bit texture draws: the core keys the binding for the
/// 32-bit TMU output (`stage_lod_key` sets `LodKey::ret32` from the bound
/// format, and the compiler writes P1 bit 0 for it), so `texture_units`
/// compares the look-up against the 32-bit key — never 16 bits — and
/// RET16_OF_32 stands for the look-up the key would give (above), not for
/// this draw. Observed on main: the draw is accepted, the P1 word of unit 0
/// carries the bit, the unit reaches the job builder.
#[test]
fn a_sampler3d_look_up_of_a_32_bit_3d_texture_draws_keyed_32_bit_fragment() {
    let _g = rig();
    let (e, lines) = sample_draw(GL_TEXTURE_3D, true, VS, FS3D);
    let d = the_draw();
    assert_eq!(e, GL_NO_ERROR, "the look-up draws: {lines:?}");
    assert!(!lines.iter().any(|l| l.contains(RET16_OF_32)), "no RET16_OF_32 refusal: {lines:?}");
    looked_up_fs(&d, 1);
    no_depth_or_array_words(&d);
}

#[test]
fn a_sampler3d_look_up_of_a_32_bit_3d_texture_draws_keyed_32_bit_vertex() {
    let _g = rig();
    let (e, lines) = sample_draw(GL_TEXTURE_3D, true, VS3D, PASS_FS);
    let d = the_draw();
    assert_eq!(e, GL_NO_ERROR, "the look-up draws: {lines:?}");
    assert!(!lines.iter().any(|l| l.contains(RET16_OF_32)), "no RET16_OF_32 refusal: {lines:?}");
    looked_up_vs(&d, 1);
    no_depth_or_array_words(&d);
}

#[test]
fn a_sampler2darray_look_up_of_a_32_bit_array_texture_draws_keyed_32_bit_fragment() {
    let _g = rig();
    let (e, lines) = sample_draw(GL_TEXTURE_2D_ARRAY, true, VS, FSARRAY);
    let d = the_draw();
    assert_eq!(e, GL_NO_ERROR, "the look-up draws: {lines:?}");
    assert!(!lines.iter().any(|l| l.contains(RET16_OF_32)), "no RET16_OF_32 refusal: {lines:?}");
    looked_up_fs(&d, 1);
    no_depth_or_array_words(&d);
}

#[test]
fn a_sampler2darray_look_up_of_a_32_bit_array_texture_draws_keyed_32_bit_vertex() {
    let _g = rig();
    let (e, lines) = sample_draw(GL_TEXTURE_2D_ARRAY, true, VSARRAY, PASS_FS);
    let d = the_draw();
    assert_eq!(e, GL_NO_ERROR, "the look-up draws: {lines:?}");
    assert!(!lines.iter().any(|l| l.contains(RET16_OF_32)), "no RET16_OF_32 refusal: {lines:?}");
    looked_up_vs(&d, 1);
    no_depth_or_array_words(&d);
}

/// The control: the same shader over an RGBA8 3D texture is keyed for the
/// 16-bit output instead — the key follows the bound format, and the unit
/// is not refused for RET16_OF_32 either way. (With the C job builder these
/// 3D and array streams carry no word 46/47, as `no_depth_or_array_words`
/// records; a size-only 3D or array draw does hit that gap on main until
/// words 46/47 are written, pinned by tests/es3_texsize3d_v3d.rs.)
#[test]
fn an_rgba8_3d_look_up_is_keyed_16_bit_and_not_refused_that_way() {
    let _g = rig();
    let (e, lines) = sample_draw(GL_TEXTURE_3D, false, VS, FS3D);
    let d = the_draw();
    assert_eq!(e, GL_NO_ERROR, "the look-up draws: {lines:?}");
    assert!(!lines.iter().any(|l| l.contains(RET16_OF_32)), "no RET16_OF_32 refusal: {lines:?}");
    looked_up_fs(&d, 0);
    no_depth_or_array_words(&d);
}
