// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A shadow-sampler draw on the V3D 4.2 compares with the texture's compare
//! function (item v3d-shadow, docs/notes/v3d-shadow.md (a)): through the
//! real GL core and the real V3D back end (the job builder stood in for, as
//! in tests/es3_v3d_refusal_gl.rs; the texture and sampler records packed by
//! the real driver/src/v3d/v3d_tex.c, built by this test).
//!
//! tests/es3_shadow_v3d.rs runs the shadow look-up forms on the QPU
//! interpreter with the compare function handed in; what it cannot show is
//! that a draw's sampler record carries GL_TEXTURE_COMPARE_FUNC. Before this
//! item `sampler_for` packed with `v3d_tex_pack_sampler` and keyed its cache
//! on the `V3dSampler` alone, so every shadow draw on the Pi 4 compared
//! NEVER (RC2 of docs/es3-cts-failures.md, the texture.shadow cases).
//!
//! Here, for a 2x2 GL_DEPTH_COMPONENT32F texture under
//! GL_COMPARE_REF_TO_TEXTURE and each of the eight functions: the draw's
//! sampler record holds the function in its "Depth Compare Function" field
//! (bits 4-6 of word 0, v3d_packet.xml SAMPLER_STATE), eight draws of one
//! job hold eight records; and the FS of tests/es3_shadow_v3d.rs's
//! `texture(sampler2DShadow, vec3)` form, run on the interpreter with the
//! function read back from that record, gives `reference OP depth` (0 or 1,
//! ES 3.0.6 3.8.15.1) at each texel. The other 2D shadow look-up forms each
//! draw with the function in their record; a sampler object's compare
//! overrides the texture's; a fixed-point depth (bit 3 of `compare_op`)
//! packs the same field; compare off packs NEVER (V3D 4.2 has no enable
//! bit: a look-up without TMUDREF does not compare).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_shadow_v3d_draw
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
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::texref::{CompareFunc, Filter, Level, MinFilter, Sampler, Texture, Wrap};
use yakogl::shaderc::online::v3d::chain::{compile_program_keyed, Prim, Target, V3dProgram};
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, LANES};
use yakogl::shaderc::online::{lower, sema};
use yakogl::shaderc::LodKey;
use yakogl::v3d::backend::yakogl_backend_v3d;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;
use yakogl::v3d::texture::SAMPLER_COMPARE_SHIFT;

/* ---- the job builder: records each draw's FS sampler bindings ---- */

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The FS sampler bindings of every draw handed to the job builder.
static DRAWS: Mutex<Vec<Vec<V3dJobTmu>>> = Mutex::new(Vec::new());

fn log_lines() -> Vec<String> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn draws() -> Vec<Vec<V3dJobTmu>> {
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
    let tmu = if d.tmu.is_null() || d.num_tmu == 0 {
        Vec::new()
    } else {
        // SAFETY: `num_tmu` records, live for the call.
        unsafe { std::slice::from_raw_parts(d.tmu, d.num_tmu as usize) }.to_vec()
    };
    DRAWS.lock().unwrap_or_else(|e| e.into_inner()).push(tmu);
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_shadow_v3d_draw");
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

/// `n` bytes of GPU memory at `gpu`, inside one allocation.
fn gpu_bytes(gpu: u32, n: usize) -> Vec<u8> {
    with_platform(|p| {
        let (base, a) = p.allocs.range(..=u64::from(gpu)).next_back().expect("an allocation at the address");
        let off = (u64::from(gpu) - base) as usize;
        assert!(off + n <= a.size, "{n} bytes at {gpu:#x} inside the allocation");
        // SAFETY: inside the allocation (checked above).
        unsafe { std::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off), n) }.to_vec()
    })
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
use yakogl::gl::es3::{glBindSampler, glDeleteSamplers, glGenSamplers, glSamplerParameteri};
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::texture::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};

const VS: &str = "#version 300 es\nlayout(location = 0) in highp vec4 a_pos;\nout highp vec4 v;\n\
                  void main() { v = a_pos; gl_Position = a_pos; }\n";

/// tests/es3_shadow_v3d.rs's fragment shader of a shadow look-up `expr` of
/// `t`, the reference `u_ref`, the divisor `u_q`, the gradients `gx`, `gy`.
fn fs_of(sampler: &str, expr: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nuniform highp {sampler} t;\nuniform highp float u_ref, u_q;\n\
         uniform highp vec3 gx, gy;\nin highp vec4 v;\nout vec4 o;\nvoid main() {{ o = vec4({expr}); }}\n"
    )
}

/// The 2D shadow look-up forms of tests/es3_shadow_v3d.rs's FORMS, written
/// over its `fs_of` names.
const FORMS_2D: [&str; 16] = [
    "texture(t, vec3(v.xy, u_ref))",
    "texture(t, vec3(v.xy, u_ref), u_q)",
    "textureProj(t, vec4(v.xy, u_ref, u_q))",
    "textureProj(t, vec4(v.xy, u_ref, u_q), u_q)",
    "textureLod(t, vec3(v.xy, u_ref), u_q)",
    "textureOffset(t, vec3(v.xy, u_ref), ivec2(1, -1))",
    "textureOffset(t, vec3(v.xy, u_ref), ivec2(-8, 7), u_q)",
    "textureProjOffset(t, vec4(v.xy, u_ref, u_q), ivec2(2, 2))",
    "textureProjOffset(t, vec4(v.xy, u_ref, u_q), ivec2(2, 2), u_q)",
    "textureLodOffset(t, vec3(v.xy, u_ref), u_q, ivec2(-1, 0))",
    "textureProjLod(t, vec4(v.xy, u_ref, u_q), u_q)",
    "textureProjLodOffset(t, vec4(v.xy, u_ref, u_q), u_q, ivec2(1, 1))",
    "textureGrad(t, vec3(v.xy, u_ref), gx.xy, gy.xy)",
    "textureGradOffset(t, vec3(v.xy, u_ref), gx.xy, gy.xy, ivec2(1, 2))",
    "textureProjGrad(t, vec4(v.xy, u_ref, u_q), gx.xy, gy.xy)",
    "textureProjGradOffset(t, vec4(v.xy, u_ref, u_q), gx.xy, gy.xy, ivec2(-2, 3))",
];

/// The depths of the 2x2 texture (row 0 first), as tests/es3_shadow_v3d.rs's.
const DEPTHS: [f32; 4] = [0.25, 0.5, 0.5, 0.75];

/// GL's compare functions, in the order of the field's enum
/// (v3d_packet.xml:3-12) and of `CompareFunc`.
const GL_FUNCS: [GLenum; 8] = [GL_NEVER, GL_LESS, GL_EQUAL, GL_LEQUAL, GL_GREATER, GL_NOTEQUAL, GL_GEQUAL, GL_ALWAYS];
const FUNCS: [CompareFunc; 8] = [
    CompareFunc::Never,
    CompareFunc::Less,
    CompareFunc::Equal,
    CompareFunc::Lequal,
    CompareFunc::Greater,
    CompareFunc::NotEqual,
    CompareFunc::Gequal,
    CompareFunc::Always,
];

/// ES 3.0.6 3.8.15.1: `reference OP depth`, 1.0 when it passes.
fn compare(func: GLenum, reference: f32, d: f32) -> f32 {
    let pass = match func {
        GL_NEVER => false,
        GL_LESS => reference < d,
        GL_EQUAL => reference == d,
        GL_LEQUAL => reference <= d,
        GL_GREATER => reference > d,
        GL_NOTEQUAL => reference != d,
        GL_GEQUAL => reference >= d,
        GL_ALWAYS => true,
        f => panic!("not a compare function: {f:#x}"),
    };
    f32::from(u8::from(pass))
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

/// Make `fs` (with `VS`) the current program, its sampler `t` on unit 0,
/// `u_ref` = `reference`, `u_q` = 1, the gradients 0.
fn program(fs: &str, reference: f32) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls; NUL-terminated names.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{fs} did not link");
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"t".as_ptr()), 0);
        glUniform1f(glGetUniformLocation(p, c"u_ref".as_ptr()), reference);
        glUniform1f(glGetUniformLocation(p, c"u_q".as_ptr()), 1.0);
        glDeleteShader(v);
        glDeleteShader(f);
        p
    }
}

/// A 2x2 depth texture on unit 0 of `internal` (`DEPTHS`, NEAREST, clamped),
/// with GL_COMPARE_REF_TO_TEXTURE.
fn depth_texture(internal: GLenum) -> GLuint {
    let mut t = 0;
    // SAFETY: plain GL calls; the texels outlive the upload.
    unsafe {
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, t);
        if internal == GL_DEPTH_COMPONENT32F {
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                internal as GLint,
                2,
                2,
                0,
                GL_DEPTH_COMPONENT,
                GL_FLOAT,
                DEPTHS.as_ptr() as *const c_void,
            );
        } else {
            let texels = DEPTHS.map(|d| (d * 65535.0).round() as u16);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                internal as GLint,
                2,
                2,
                0,
                GL_DEPTH_COMPONENT,
                GL_UNSIGNED_SHORT,
                texels.as_ptr() as *const c_void,
            );
        }
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE as GLint);
    }
    assert_eq!(err(), GL_NO_ERROR, "the depth texture: {:?}", log_lines());
    t
}

fn set_func(func: GLenum) {
    // SAFETY: plain GL call.
    unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_FUNC, func as GLint) };
}

/// A full-screen triangle into the default framebuffer; the draw raises no
/// error.
fn draw_triangle(what: &str) {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
    assert_eq!(err(), GL_NO_ERROR, "{what}: {:?}", log_lines());
}

/// The sampler record of the FS binding of unit 0 of each recorded draw,
/// once the job is done: (its address, its "Depth Compare Function" field).
fn compare_fields() -> Vec<(u32, u32)> {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    draws()
        .iter()
        .filter(|tmu| !tmu.is_empty())
        .map(|tmu| {
            let u = tmu.iter().find(|u| u.unit == 0).expect("unit 0 sampled");
            let w0 = u32::from_le_bytes(gpu_bytes(u.sampler, V3D_SAMPLER_STATE_BYTES)[..4].try_into().unwrap());
            (u.sampler, (w0 >> SAMPLER_COMPARE_SHIFT) & 7)
        })
        .collect()
}

/* ---- the interpreter: the look-up under the record's function ---- */

/// The Raspberry Pi 4's VPM (tests/v3d_sim_mesa.rs TARGET).
const TARGET: Target = Target { vpm_size: 65536 };

/// `VS` + `fs` compiled for the V3D with binding 0 a depth texture (its
/// 32-bit return), as the GL core keys it, handed to `f`.
fn with_program<R>(fs: &str, f: impl FnOnce(&V3dProgram<'_>) -> R) -> R {
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).expect("arena");
    let fs = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let fr = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let fk = LodKey { ret32: 1, ..LodKey::NONE };
    let old = yakogl::shaderc::set_compile_lod_key(fk);
    let p = compile_program_keyed(&v.module, &fr.module, Prim::Triangles, TARGET, LodKey::NONE, fk, &a);
    yakogl::shaderc::set_compile_lod_key(old);
    let p = p.unwrap_or_else(|e| panic!("the program compiles: {e:?}"));
    f(&p)
}

/// `fs` at texture coordinates (`s`, `t`), its reference `reference`, on the
/// interpreter with sampler 0 = the 2x2 depth texture under NEAREST and the
/// record's compare function `field`: the colour's red, from the f16 the FS
/// writes.
fn run_fs(fs: &str, s: f32, t: f32, reference: f32, field: u32) -> f32 {
    let faces = vec![DEPTHS.map(|d| [d, 0.0, 0.0, 1.0]).to_vec()];
    let tex = Texture { cube: false, min: MinFilter::Nearest, levels: vec![Level { width: 2, height: 2, faces }] };
    let sampler =
        Sampler { mag: Filter::Nearest, min: MinFilter::Nearest, wrap_s: Wrap::ClampToEdge, wrap_t: Wrap::ClampToEdge };
    with_program(fs, |p| {
        let user = [reference, 1.0];
        let u: Vec<u32> =
            p.fs.code
                .uniforms
                .iter()
                .map(|u| match u.contents {
                    0 | 40 | 41 => u.data,
                    1 => user.get(u.data as usize).copied().unwrap_or(0.0).to_bits(),
                    2..=5 => 1.0f32.to_bits(),
                    44..=47 if u.data == 0 => [2, 2, 1, 1][(u.contents - 44) as usize],
                    /* QUNIFORM_FB_WPOS_Y_SCALE: a window target */
                    0x1_0000 => (-1.0f32).to_bits(),
                    c => panic!("uniform contents {c} (data {}) not modelled by this test", u.data),
                })
                .collect();
        let one = [1.0f32.to_bits(); LANES];
        let z = [0u32; LANES];
        let varyings = [s, t, 0.0, 1.0].iter().map(|c| sim::Varying { vary: [c.to_bits(); LANES], c: z }).collect();
        let inputs = FsInputs { w: one, w_centroid: one, z, x: z, y: z, back_facing: [false; LANES], varyings };
        let env = Env {
            textures: vec![tex],
            samplers: vec![sampler],
            compare: vec![Some(FUNCS[field as usize])],
            threads: p.fs.threads,
            ..Env::default()
        };
        let o = sim::run(p.fs.code.words, &u, &Inputs::Fragment(Box::new(inputs)), &env)
            .unwrap_or_else(|e| panic!("the fragment program runs: {e}"));
        assert_eq!(o.tmu_lookups, 1, "one look-up");
        let word = o.tlb[0].value[0].expect("a defined value");
        sim::half_to_f32((word & 0xffff) as u16)
    })
}

/* ---- the draws ---- */

/// Eight draws of one job, each with GL_TEXTURE_COMPARE_FUNC one of the
/// eight: each draw's sampler record carries its function, eight records;
/// and `texture(sampler2DShadow, vec3)` under the function read back from
/// the record gives `reference OP depth` at each texel centre (reference
/// 0.5 against 0.25, 0.5, 0.5, 0.75: every function tells LESS from EQUAL
/// from GREATER). Revert check: `sampler_for` packing with
/// `v3d_tex_pack_sampler` gives every record NEVER, all 0; keying its
/// cache on the `V3dSampler` alone gives every draw the first draw's NEVER.
#[test]
fn each_compare_function_reaches_the_drawn_sampler_record() {
    let _g = rig();
    let fs = fs_of("sampler2DShadow", FORMS_2D[0]);
    let p = program(&fs, 0.5);
    let t = depth_texture(GL_DEPTH_COMPONENT32F);
    for func in GL_FUNCS {
        set_func(func);
        draw_triangle(&format!("compare {func:#x}"));
    }
    let got = compare_fields();
    assert_eq!(got.len(), 8, "eight textured draws: {:?}", log_lines());
    for (k, &(rec, field)) in got.iter().enumerate() {
        assert_eq!(field, k as u32, "GL function {:#x}: the record's field", GL_FUNCS[k]);
        assert!(got[..k].iter().all(|&(r, _)| r != rec), "GL function {:#x}: a record of its own", GL_FUNCS[k]);
    }
    let centres = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];
    for (k, &(_, field)) in got.iter().enumerate() {
        for (i, (s, t)) in centres.into_iter().enumerate() {
            let want = compare(GL_FUNCS[k], 0.5, DEPTHS[i]);
            assert_eq!(run_fs(&fs, s, t, 0.5, field), want, "GL function {:#x} at texel {i}", GL_FUNCS[k]);
        }
    }
    // SAFETY: plain GL calls.
    unsafe {
        glDeleteTextures(1, &t);
        glDeleteProgram(p);
    }
}

/// Every 2D shadow look-up form of tests/es3_shadow_v3d.rs draws with the
/// texture's function in its record (GL_GEQUAL, 6, then GL_LESS, 1: the
/// draws alternate, and each finds its function's record again).
#[test]
fn every_2d_shadow_form_draws_with_the_textures_function() {
    let _g = rig();
    let t = depth_texture(GL_DEPTH_COMPONENT32F);
    for form in FORMS_2D {
        let p = program(&fs_of("sampler2DShadow", form), 0.5);
        for func in [GL_GEQUAL, GL_LESS] {
            set_func(func);
            draw_triangle(form);
        }
        // SAFETY: plain GL call.
        unsafe { glDeleteProgram(p) };
    }
    let got = compare_fields();
    assert_eq!(got.len(), 2 * FORMS_2D.len(), "{:?}", log_lines());
    for (k, &(rec, field)) in got.iter().enumerate() {
        let form = FORMS_2D[k / 2];
        assert_eq!(field, if k % 2 == 0 { 6 } else { 1 }, "{form}");
        assert_eq!(rec, got[k % 2].0, "{form}: one record per function in the job");
    }
    assert_ne!(got[0].0, got[1].0);
    // SAFETY: plain GL call.
    unsafe { glDeleteTextures(1, &t) };
}

/// A sampler object's compare mode and function replace the texture's (ES
/// 3.0.6 3.8.2), and unbinding it brings the texture's back; compare off
/// on the texture packs NEVER, the compare-off record (V3D 4.2 has no
/// enable bit); a fixed-point depth texture (GL_DEPTH_COMPONENT16, bit 3
/// of `compare_op`) packs the same field as a float one.
#[test]
fn a_sampler_objects_compare_and_a_fixed_point_depth_reach_the_record() {
    let _g = rig();
    let p = program(&fs_of("sampler2DShadow", FORMS_2D[0]), 0.5);
    let t = depth_texture(GL_DEPTH_COMPONENT32F);
    set_func(GL_EQUAL);
    let mut s = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenSamplers(1, &mut s);
        for (pname, value) in [
            (GL_TEXTURE_MIN_FILTER, GL_NEAREST),
            (GL_TEXTURE_MAG_FILTER, GL_NEAREST),
            (GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE),
            (GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE),
            (GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE),
            (GL_TEXTURE_COMPARE_FUNC, GL_GREATER),
        ] {
            glSamplerParameteri(s, pname, value as GLint);
        }
        glBindSampler(0, s);
    }
    draw_triangle("the sampler object's GL_GREATER");
    // SAFETY: plain GL call.
    unsafe { glBindSampler(0, 0) };
    draw_triangle("the texture's GL_EQUAL");
    // SAFETY: plain GL call.
    unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_MODE, GL_NONE as GLint) };
    draw_triangle("compare off");
    let fixed = depth_texture(GL_DEPTH_COMPONENT16);
    set_func(GL_LEQUAL);
    draw_triangle("a fixed-point depth's GL_LEQUAL");
    let got: Vec<u32> = compare_fields().iter().map(|&(_, f)| f).collect();
    assert_eq!(got, [4, 2, 0, 3], "GREATER, EQUAL, compare off, LEQUAL: {:?}", log_lines());
    // SAFETY: plain GL calls.
    unsafe {
        glDeleteSamplers(1, &s);
        glDeleteTextures(1, &t);
        glDeleteTextures(1, &fixed);
        glDeleteProgram(p);
    }
}
