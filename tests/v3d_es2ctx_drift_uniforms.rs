// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against 3cbecbb9 (records and the GL program take more
//! than 255 active uniforms). The change moved how a program's uniforms are
//! counted and addressed, for every GPU and the V3D too: the shader record's
//! `num_uniforms` is 16 bits (src/shaderc/online/v3d_abi.rs), a program's
//! locations are a range of one pool per context that a relink or a delete
//! gives back and the ranges after it move down, and a location is
//! `index | element << 16` (was `<< 8`, src/gl/program.rs). dEQP-GLES2
//! functional.uniform_api.* (info_query.*, value.initial.*, value.assigned.*,
//! random.*) reads exactly these answers: the location of each name,
//! glGetActiveUniform's name, size and type, glGetUniform*v's values,
//! GL_ACTIVE_UNIFORMS and GL_ACTIVE_UNIFORM_MAX_LENGTH. The Pi 4 whole tree
//! (46f93767) predates the change.
//!
//! Through the real GL core (a `report-es2` build, GLSL ES 1.00 programs) and
//! the real V3D back end (rig of tests/v3d_es2ctx_refusals.rs: the job
//! builder is stood in for, the texture layout is the real
//! driver/src/v3d/v3d_tex.c) this file pins:
//!
//! - a program with struct, array, array-of-struct, sampler and bool
//!   uniforms in both stages: GL_ACTIVE_UNIFORMS, GL_ACTIVE_UNIFORM_MAX_LENGTH,
//!   every glGetActiveUniform (name, size, type; a truncated name);
//! - one distinct location per active name and array element, -1 for the
//!   names that are not active uniforms;
//! - the initial value 0 of every uniform, and the value read back after
//!   each setter (bool through the int and the float setters, samplers
//!   through glUniform1i, arrays through one call and per element);
//! - the setter errors of ES 2.0 2.10.4 next to them;
//! - a program of 274 active uniforms (past the old 255; 273 in the fragment
//!   stage), its locations past the index byte and their values;
//! - the location pool: deleting and relinking a program leaves the
//!   locations and values of the programs after it as they were;
//! - an array member of a struct (`s.m[0]` active, `s.m` and `s.m[1]`
//!   located and set) and a bool array;
//! - programs of one struct-array element per vec4 row near
//!   GL_MAX_VERTEX_UNIFORM_VECTORS / GL_MAX_FRAGMENT_UNIFORM_VECTORS (638
//!   active uniforms; at both limits, 766, is a known defect, `#[ignore]`d);
//! - draws with these programs are accepted (no GL error, no refused draw),
//!   and the job's vertex and fragment uniform blocks hold the values set.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_drift_uniforms
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test", feature = "report-es2"))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
use yakogl::v3d::backend::yakogl_backend_v3d;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/* ---- the job builder: records, draws nothing ---- */

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn log_lines() -> Vec<String> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clone()
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

/// The GL uniform blocks (vertex, fragment) of the last draw the job builder
/// was handed, as raw words.
static STREAMS: Mutex<(Vec<u32>, Vec<u32>)> = Mutex::new((Vec::new(), Vec::new()));

fn streams() -> (Vec<u32>, Vec<u32>) {
    STREAMS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// A stage's GL uniform block, empty when the draw names none.
///
/// # Safety
/// `s` is a block of a live draw: `count` words at `words`.
unsafe fn block(s: V3dUniformStorage) -> Vec<u32> {
    if s.words.is_null() || s.count == 0 {
        return Vec::new();
    }
    // SAFETY: the caller's block, as long as it says.
    unsafe { core::slice::from_raw_parts(s.words, s.count as usize) }.to_vec()
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes its live job and a draw whose blocks live for the call.
    unsafe {
        (*j).draws += 1;
        (*(*j).bcl).used += 120;
        let s = (block((*d).vs_storage), block((*d).fs_storage));
        *STREAMS.lock().unwrap_or_else(|e| e.into_inner()) = s;
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_es2ctx_drift_uniforms");
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
    trampoline!(v3d_tex_pack_sampler(out: *mut u8, s: *const V3dSampler) -> bool);
    trampoline!(v3d_tex_pack_sampler_compare(out: *mut u8, s: *const V3dSampler, compare_op: u32) -> bool);
}

#[no_mangle]
unsafe extern "C" fn v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool {
    // SAFETY: the C function's signature.
    let f: unsafe extern "C" fn(*mut u8, *const V3dTexState) -> bool =
        unsafe { std::mem::transmute(sym("v3d_tex_pack_state")) };
    // SAFETY: the library's call, passed on; `s` is its live state.
    unsafe { f(out, s) }
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
    use yakogl::osal::{
        OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_TILED_BUDGET, OSAL_QUERY_V3D_VPM_BYTES,
    };
    /* the kept tiled copies of image T5 (driver/samples/zephyr/deqp/texture-2048.conf),
     * the one Pi 4 image that samples the 2048x2048 textures of texture.size.2d */
    if what == OSAL_QUERY_V3D_TILED_BUDGET && out_sz == 4 {
        // SAFETY: a u32 out-parameter.
        unsafe { *(out as *mut u32) = 25_165_824 };
        return 0;
    }
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

/* ---- struct members that are arrays, bool arrays ---- */

const VS_SM: &str = "attribute vec4 p;\n\
    struct T { vec2 m[3]; float k; };\n\
    uniform T s;\n\
    uniform bool ba[2];\n\
    void main() {\n\
      vec4 a = p + vec4(s.m[0] + s.m[1] + s.m[2], s.k, 0.0);\n\
      if (ba[0]) a.x += 1.0;\n\
      if (ba[1]) a.y += 1.0;\n\
      gl_Position = a;\n\
    }\n";

/// The words of `w` lie one after the other somewhere in `s`.
fn holds(s: &[u32], w: &[u32]) -> bool {
    !w.is_empty() && s.windows(w.len()).any(|x| x == w)
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|f| f.to_bits()).collect()
}

/// `s.m[1]`, `s.m` and `ba` (ES 2.0 2.10.4: an array member of a struct is
/// one active uniform named `s.m[0]`; its own name is its element 0; each
/// element has its location), their values, the setter errors on them, and
/// a draw whose uniform blocks hold the values set.
#[test]
fn array_members_of_a_struct_and_bool_arrays() {
    let _g = rig();
    let p = link(VS_SM, FS_PLAIN);
    let want: Vec<(String, GLint, GLenum)> =
        [("ba[0]", 2, GL_BOOL), ("col", 1, GL_FLOAT_VEC4), ("s.k", 1, GL_FLOAT), ("s.m[0]", 3, GL_FLOAT_VEC2)]
            .iter()
            .map(|&(n, s, t)| (n.to_owned(), s, t))
            .collect();
    assert_eq!(iv(p, GL_ACTIVE_UNIFORMS), 4);
    assert_eq!(actives(p), want);
    /* "s.m[0]" and its NUL */
    assert_eq!(iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), 7);

    let m: Vec<GLint> = (0..3).map(|e| loc(p, &format!("s.m[{e}]"))).collect();
    assert!(m.iter().all(|&l| l >= 0), "{m:?}");
    assert!(m[0] != m[1] && m[1] != m[2] && m[0] != m[2], "{m:?}");
    assert_eq!(loc(p, "s.m"), m[0]);
    assert_eq!(loc(p, "ba"), loc(p, "ba[0]"));
    assert!(loc(p, "ba[1]") >= 0 && loc(p, "ba[1]") != loc(p, "ba[0]"));
    for name in ["s.m[3]", "s", "s.m[1].x", "ba[2]"] {
        assert_eq!(loc(p, name), -1, "{name}");
    }

    // SAFETY: plain GL calls with the program current; each array holds what its call reads.
    unsafe {
        glUseProgram(p);
        glUniform2f(m[1], 3.5, -1.25);
        assert_eq!(err(), GL_NO_ERROR);
    }
    assert_eq!(getf(p, m[0], 2), [0.0, 0.0]);
    assert_eq!(getf(p, m[1], 2), [3.5, -1.25]);
    assert_eq!(getf(p, m[2], 2), [0.0, 0.0]);
    let sm = [1.5f32, 2.5, 3.5, 4.5, 5.5, 6.5];
    // SAFETY: as above.
    unsafe {
        glUniform2fv(loc(p, "s.m"), 3, sm.as_ptr());
        glUniform1f(loc(p, "s.k"), 9.75);
        glUniform1iv(loc(p, "ba"), 2, [0, 5].as_ptr());
        glUniform4f(loc(p, "col"), 0.125, 0.25, 0.375, 0.5);
        assert_eq!(err(), GL_NO_ERROR);
        glUniform1f(m[1], 1.0);
        assert_eq!(err(), GL_INVALID_OPERATION, "float setter on a vec2 element");
        glUniform2i(m[1], 1, 2);
        assert_eq!(err(), GL_INVALID_OPERATION, "ivec2 setter on a vec2 element");
        glUniform1f(-1, 1.0);
        assert_eq!(err(), GL_NO_ERROR, "location -1 is ignored");
    }
    for e in 0..3 {
        assert_eq!(getf(p, m[e], 2), sm[e * 2..e * 2 + 2], "s.m[{e}]");
    }
    assert_eq!(getf(p, loc(p, "s.k"), 1), [9.75]);
    assert_eq!(geti(p, loc(p, "ba[0]"), 1), [0]);
    assert_eq!(geti(p, loc(p, "ba[1]"), 1), [1], "bool set to 5 reads 1");
    assert_eq!(getf(p, loc(p, "ba[1]"), 1), [1.0]);

    draws(p, "struct array members");
    let (vs, fs) = streams();
    for e in 0..3 {
        assert!(holds(&vs, &bits(&sm[e * 2..e * 2 + 2])), "s.m[{e}] in the VS block {vs:x?}");
    }
    assert!(holds(&vs, &bits(&[9.75])), "s.k in the VS block {vs:x?}");
    assert!(holds(&fs, &bits(&[0.125, 0.25, 0.375, 0.5])), "col in the FS block {fs:x?}");
    delete(p);
}

/// The uniform blocks of a draw with the mixed program hold the values the
/// setters stored (what the shaders read; dEQP-GLES2 uniform_api.value.*
/// renders with them).
#[test]
fn the_draw_carries_the_values_set() {
    let _g = rig();
    let p = link(VS_MIX, FS_MIX);
    let arr: [f32; 12] = core::array::from_fn(|i| i as f32 + 100.5);
    // SAFETY: plain GL calls with the program current; each array holds what its call reads.
    unsafe {
        glUseProgram(p);
        glUniform1f(loc(p, "s.f"), 1.5);
        glUniform3f(loc(p, "s.v"), 2.0, -3.0, 4.25);
        glUniform4fv(loc(p, "arr"), 3, arr.as_ptr());
        glUniform3f(loc(p, "lights[0].c"), 0.5, 0.25, 0.125);
        glUniform1f(loc(p, "lights[0].w"), 8.0);
        glUniform3f(loc(p, "lights[1].c"), 9.0, 10.0, 11.0);
        glUniform1f(loc(p, "lights[1].w"), -2.0);
    }
    assert_eq!(err(), GL_NO_ERROR, "the setters");
    draws(p, "the mixed program");
    let (vs, fs) = streams();
    assert!(holds(&vs, &bits(&[1.5])), "s.f in the VS block {vs:x?}");
    assert!(holds(&vs, &bits(&[2.0, -3.0, 4.25])), "s.v in the VS block {vs:x?}");
    for e in 0..3 {
        assert!(holds(&vs, &bits(&arr[e * 4..e * 4 + 4])), "arr[{e}] in the VS block {vs:x?}");
    }
    assert!(holds(&fs, &bits(&[0.5, 0.25, 0.125])), "lights[0].c in the FS block {fs:x?}");
    assert!(holds(&fs, &bits(&[8.0])), "lights[0].w in the FS block {fs:x?}");
    assert!(holds(&fs, &bits(&[9.0, 10.0, 11.0])), "lights[1].c in the FS block {fs:x?}");
    assert!(holds(&fs, &bits(&[-2.0])), "lights[1].w in the FS block {fs:x?}");

    /* a value changed between two draws reaches the second */
    // SAFETY: a plain GL call with the program current.
    unsafe { glUniform3f(loc(p, "s.v"), 7.0, 7.5, 8.0) };
    draws(p, "the mixed program, again");
    let (vs, _) = streams();
    assert!(holds(&vs, &bits(&[7.0, 7.5, 8.0])), "the new s.v in the VS block {vs:x?}");
    delete(p);
}

/* ---- near the limits ---- */

fn limit(pname: GLenum) -> usize {
    let mut v = -1;
    // SAFETY: a plain GL call with room for one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "glGetIntegerv {pname:#x}");
    v as usize
}

/// `vec4 r` = the sum of `a[0].a` .. `a[n - 1].a`, added 16 terms at a time
/// into one variable (one expression of hundreds of terms tests the
/// parser's depth, and many partial sums the register file, not the
/// uniforms).
fn sum_of(n: usize, a: char) -> String {
    let mut out = " vec4 r = vec4(0.0);\n".to_owned();
    for c in 0..n.div_ceil(16) {
        let t: Vec<String> = (c * 16..n.min(c * 16 + 16)).map(|i| format!("{a}[{i}].a")).collect();
        out += &format!(" r += {};\n", t.join(" + "));
    }
    out
}

fn near_value(stage: usize, i: usize) -> [f32; 4] {
    let b = (stage * 1000 + i) as f32 * 4.0;
    [b + 0.5, b + 1.5, b + 2.5, b + 3.5]
}

/// GL_MAX_VERTEX_UNIFORM_VECTORS and GL_MAX_FRAGMENT_UNIFORM_VECTORS, checked
/// against the ES 2.0 minimums (table 6.19: 128 and 16).
fn limits() -> (usize, usize) {
    let (mv, mf) = (limit(GL_MAX_VERTEX_UNIFORM_VECTORS), limit(GL_MAX_FRAGMENT_UNIFORM_VECTORS));
    assert!(mv >= 128 && mf >= 16, "the ES 2.0 minimums: {mv} {mf}");
    (mv, mf)
}

/// One struct-array element per vec4 row, `nv` rows in the vertex stage and
/// `nf` in the fragment stage: the link (ES 2.0 Appendix A.7: what fits the
/// reported vectors under the packing rules links), one active uniform per
/// row, each row its own location, the values read back and carried by the
/// draw, locations past the old index byte once there are more than 256.
fn rows_program(nv: usize, nf: usize) {
    let (vsum, fsum) = (sum_of(nv, 'v'), sum_of(nf, 'f'));
    let vs = format!(
        "attribute vec4 p;\nstruct V {{ vec4 a; }};\nuniform V v[{nv}];\nvoid main() {{\n{vsum} gl_Position = p + r;\n}}\n"
    );
    let fs = format!(
        "precision mediump float;\nstruct F {{ vec4 a; }};\nuniform F f[{nf}];\n\
         void main() {{\n{fsum} gl_FragColor = r;\n}}\n"
    );
    let p = link(&vs, &fs);
    assert_eq!(iv(p, GL_ACTIVE_UNIFORMS), (nv + nf) as GLint);
    let names = |stage: usize| -> Vec<String> {
        let (c, n) = if stage == 0 { ('v', nv) } else { ('f', nf) };
        (0..n).map(|i| format!("{c}[{i}].a")).collect()
    };
    let mut want: Vec<(String, GLint, GLenum)> =
        (0..2).flat_map(|s| names(s).into_iter().map(|n| (n, 1, GL_FLOAT_VEC4))).collect();
    want.sort();
    assert_eq!(actives(p), want);
    let longest = names(0).iter().chain(names(1).iter()).map(String::len).max().unwrap();
    assert_eq!(iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), longest as GLint + 1);

    let mut seen = BTreeMap::new();
    // SAFETY: a plain GL call.
    unsafe { glUseProgram(p) };
    for s in 0..2 {
        for (i, name) in names(s).iter().enumerate() {
            let l = loc(p, name);
            assert!(l >= 0, "{name}: {l}");
            assert!(seen.insert(l, name.clone()).is_none(), "{name} shares location {l:#x}");
            assert_eq!(getf(p, l, 4), [0.0; 4], "{name} before");
            let [x, y, z, w] = near_value(s, i);
            // SAFETY: a plain GL call with the program current.
            unsafe { glUniform4f(l, x, y, z, w) };
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "the setters");
    if nv + nf > 256 {
        assert!(seen.keys().any(|&l| l & 0xffff > 255), "{seen:?}");
    }
    for s in 0..2 {
        for (i, name) in names(s).iter().enumerate() {
            assert_eq!(getf(p, loc(p, name), 4), near_value(s, i), "{name}");
        }
    }
    draws(p, "near the limits");
    let (vsb, fsb) = streams();
    for i in 0..nv {
        assert!(holds(&vsb, &bits(&near_value(0, i))), "v[{i}].a in the VS block");
    }
    for i in 0..nf {
        assert!(holds(&fsb, &bits(&near_value(1, i))), "f[{i}].a in the FS block");
    }
    delete(p);
}

/// One row under the vertex limit and 255 fragment rows: 638 active
/// uniforms on the 384 + 384 rows V3D reports, past the old 255 records in
/// one stage and in the program.
#[test]
fn a_program_near_the_vertex_uniform_limit() {
    let _g = rig();
    let (mv, mf) = limits();
    rows_program(mv - 1, (mf - 1).min(255));
}

/// One row under each stage's limit (383 + 383 on V3D 4.2). Each stage alone
/// at 383 rows links, and 383 + 255 or 255 + 383 does; from about 640 rows
/// together on, glLinkProgram fails with "online shaderc: this fragment
/// shader does not fit the scratch arena (0 bytes left); raise
/// shaderc::online::store::SCRATCH_BYTES" (4 MiB on the host). ES 2.0
/// Appendix A.7 says a program within the reported vectors links.
#[test]
#[ignore = "defect: a program at both stages' uniform limits overflows the online compiler's scratch arena"]
fn a_program_near_both_uniform_limits() {
    let _g = rig();
    let (mv, mf) = limits();
    rows_program(mv - 1, mf - 1);
}

/* ---- the context ---- */

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The one context and back end of the module; the tests take turns and
/// each starts with nothing recorded and nothing pending.
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
        /* a test that failed halfway leaves its state behind */
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        for cap in [GL_BLEND, GL_DEPTH_TEST, GL_STENCIL_TEST, GL_SCISSOR_TEST, GL_CULL_FACE] {
            glDisable(cap);
        }
        glColorMask(1, 1, 1, 1);
        glViewport(0, 0, W, H);
        while glGetError() != GL_NO_ERROR {}
    }
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
    *STREAMS.lock().unwrap_or_else(|e| e.into_inner()) = (Vec::new(), Vec::new());
    g
}

/* ---- helpers ---- */

use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::v3d::backend::v3d_refused_draws;

const W: GLsizei = 32;
const H: GLsizei = 32;

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
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile; {:?}", log_lines());
        s
    }
}

/// A linked program of `vs` and `fs` (attribute `p` at 0), not made current.
fn link(vs: &str, fs: &str) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link; {:?}", log_lines());
        /* flagged for deletion with the program */
        glDeleteShader(v);
        glDeleteShader(f);
        assert_eq!(err(), GL_NO_ERROR, "link");
        p
    }
}

fn relink(p: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "relink; {:?}", log_lines());
    }
}

fn delete(p: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(0);
        glDeleteProgram(p);
    }
    assert_eq!(err(), GL_NO_ERROR, "delete");
}

fn iv(p: GLuint, pname: GLenum) -> GLint {
    let mut v = -7;
    // SAFETY: a plain GL call with room for one value.
    unsafe { glGetProgramiv(p, pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "glGetProgramiv {pname:#x}");
    v
}

fn loc(p: GLuint, name: &str) -> GLint {
    let n = CString::new(name).unwrap();
    // SAFETY: a plain GL call with a NUL-terminated name.
    let l = unsafe { glGetUniformLocation(p, n.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glGetUniformLocation {name}");
    l
}

/// Every glGetActiveUniform of `p`: (name, size, type), sorted by name.
fn actives(p: GLuint) -> Vec<(String, GLint, GLenum)> {
    let n = iv(p, GL_ACTIVE_UNIFORMS);
    let mut out = Vec::new();
    for i in 0..n as GLuint {
        let mut buf = [0 as c_char; 64];
        let (mut len, mut size, mut ty) = (-1, -1, 0);
        // SAFETY: a plain GL call; the buffer holds 64 bytes.
        unsafe { glGetActiveUniform(p, i, 64, &mut len, &mut size, &mut ty, buf.as_mut_ptr()) };
        assert_eq!(err(), GL_NO_ERROR, "glGetActiveUniform {i}");
        // SAFETY: the GL wrote a NUL-terminated name into `buf`.
        let s = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_str().unwrap().to_owned();
        assert_eq!(len as usize, s.len(), "length of {s}");
        out.push((s, size, ty));
    }
    out.sort();
    out
}

fn getf(p: GLuint, l: GLint, n: usize) -> Vec<f32> {
    let mut v = [-99.0f32; 16];
    // SAFETY: a plain GL call; 16 values is room for a mat4.
    unsafe { glGetUniformfv(p, l, v.as_mut_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glGetUniformfv {l:#x}");
    assert_eq!(v[n], -99.0, "glGetUniformfv {l:#x} wrote past {n} values");
    v[..n].to_vec()
}

fn geti(p: GLuint, l: GLint, n: usize) -> Vec<i32> {
    let mut v = [-99i32; 16];
    // SAFETY: a plain GL call; 16 values is room for a mat4.
    unsafe { glGetUniformiv(p, l, v.as_mut_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glGetUniformiv {l:#x}");
    assert_eq!(v[n], -99, "glGetUniformiv {l:#x} wrote past {n} values");
    v[..n].to_vec()
}

static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// A draw with `p` current is accepted: no GL error, no refused draw.
fn draws(p: GLuint, what: &str) {
    let base = v3d_refused_draws();
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glUseProgram(p);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    let (e, n) = (err(), v3d_refused_draws().wrapping_sub(base));
    assert_eq!((e, n), (GL_NO_ERROR, 0), "{what}: error, refused draws; {:?}", log_lines());
}

/* ---- the mixed program ---- */

const VS_MIX: &str = "attribute vec4 p;\n\
    struct S { float f; vec3 v; bool b; };\n\
    uniform S s;\n\
    uniform vec4 arr[3];\n\
    uniform mat3 m;\n\
    uniform bool flag;\n\
    uniform bvec2 bv;\n\
    uniform ivec3 iv;\n\
    void main() {\n\
      vec4 a = p + vec4(s.f, s.v) + arr[0] + arr[1] + arr[2] + vec4(m[0] + m[1] + m[2], 0.0);\n\
      if (flag) a += vec4(1.0);\n\
      if (s.b) a += vec4(2.0);\n\
      a += vec4(float(bv.x), float(bv.y), float(iv.x + iv.y), float(iv.z));\n\
      gl_Position = a;\n\
    }\n";

const FS_MIX: &str = "precision mediump float;\n\
    struct L { vec3 c; float w; };\n\
    uniform L lights[2];\n\
    uniform sampler2D tex;\n\
    uniform samplerCube cube;\n\
    uniform int count;\n\
    void main() {\n\
      vec4 a = texture2D(tex, vec2(0.5)) + textureCube(cube, vec3(1.0, 0.5, 0.5));\n\
      a += vec4(lights[0].c * lights[0].w + lights[1].c * lights[1].w, 0.0);\n\
      gl_FragColor = a + vec4(float(count));\n\
    }\n";

/// The active uniforms of VS_MIX + FS_MIX as glGetActiveUniform names them:
/// a struct is one active uniform per member, an array of structs one per
/// element member, an array of a basic type one entry named `[0]`.
const MIX: [(&str, GLint, GLenum); 15] = [
    ("arr[0]", 3, GL_FLOAT_VEC4),
    ("bv", 1, GL_BOOL_VEC2),
    ("count", 1, GL_INT),
    ("cube", 1, GL_SAMPLER_CUBE),
    ("flag", 1, GL_BOOL),
    ("iv", 1, GL_INT_VEC3),
    ("lights[0].c", 1, GL_FLOAT_VEC3),
    ("lights[0].w", 1, GL_FLOAT),
    ("lights[1].c", 1, GL_FLOAT_VEC3),
    ("lights[1].w", 1, GL_FLOAT),
    ("m", 1, GL_FLOAT_MAT3),
    ("s.b", 1, GL_BOOL),
    ("s.f", 1, GL_FLOAT),
    ("s.v", 1, GL_FLOAT_VEC3),
    ("tex", 1, GL_SAMPLER_2D),
];

/// Every name a location is asked for: the active ones and each array element.
const MIX_LOCS: [&str; 17] = [
    "arr[0]",
    "arr[1]",
    "arr[2]",
    "bv",
    "count",
    "cube",
    "flag",
    "iv",
    "lights[0].c",
    "lights[0].w",
    "lights[1].c",
    "lights[1].w",
    "m",
    "s.b",
    "s.f",
    "s.v",
    "tex",
];

/// Components of each name of MIX_LOCS (glGetUniform*v writes this many).
fn comps(name: &str) -> usize {
    match name {
        "m" => 9,
        n if n.starts_with("arr") => 4,
        "iv" | "s.v" | "lights[0].c" | "lights[1].c" => 3,
        "bv" => 2,
        _ => 1,
    }
}

/* ---- the cases ---- */

/// Sanity: this is an ES 2.0 context (GL_MAJOR_VERSION is an ES 3.0 query).
#[test]
fn the_context_is_es2() {
    let _g = rig();
    let mut m = 0;
    // SAFETY: a plain GL call.
    unsafe { glGetIntegerv(GL_MAJOR_VERSION, &mut m) };
    assert_eq!(err(), GL_INVALID_ENUM);
}

/// GL_ACTIVE_UNIFORMS, GL_ACTIVE_UNIFORM_MAX_LENGTH and every
/// glGetActiveUniform of the mixed program (dEQP-GLES2
/// uniform_api.info_query.*).
#[test]
fn active_uniforms_of_the_mixed_program() {
    let _g = rig();
    let p = link(VS_MIX, FS_MIX);
    assert_eq!(iv(p, GL_ACTIVE_UNIFORMS), MIX.len() as GLint);
    let want: Vec<(String, GLint, GLenum)> = MIX.iter().map(|&(n, s, t)| (n.to_owned(), s, t)).collect();
    assert_eq!(actives(p), want);
    /* "lights[0].c" and its NUL */
    assert_eq!(iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), 12);

    /* a short buffer: the name is cut and terminated, the length is what was written */
    let i = (0..MIX.len() as GLuint)
        .find(|&i| {
            let mut buf = [0 as c_char; 64];
            let (mut len, mut size, mut ty) = (0, 0, 0);
            // SAFETY: a plain GL call; the buffer holds 64 bytes.
            unsafe { glGetActiveUniform(p, i, 64, &mut len, &mut size, &mut ty, buf.as_mut_ptr()) };
            // SAFETY: a NUL-terminated name.
            unsafe { CStr::from_ptr(buf.as_ptr()) }.to_bytes() == b"lights[0].c"
        })
        .expect("lights[0].c is active");
    let mut buf = [0x55 as c_char; 8];
    let (mut len, mut size, mut ty) = (-1, -1, 0);
    // SAFETY: a plain GL call; 4 of the buffer's 8 bytes are offered.
    unsafe { glGetActiveUniform(p, i, 4, &mut len, &mut size, &mut ty, buf.as_mut_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
    // SAFETY: a NUL-terminated name.
    assert_eq!(unsafe { CStr::from_ptr(buf.as_ptr()) }.to_bytes(), b"lig");
    assert_eq!((len, size, ty), (3, 1, GL_FLOAT_VEC3));
    assert_eq!(buf[4], 0x55, "nothing past bufSize");

    /* an index past the last, and a negative bufSize */
    // SAFETY: plain GL calls; the buffer holds 8 bytes.
    unsafe {
        glGetActiveUniform(p, MIX.len() as GLuint, 8, &mut len, &mut size, &mut ty, buf.as_mut_ptr());
        assert_eq!(err(), GL_INVALID_VALUE, "index past the last");
        glGetActiveUniform(p, 0, -1, &mut len, &mut size, &mut ty, buf.as_mut_ptr());
        assert_eq!(err(), GL_INVALID_VALUE, "bufSize -1");
    }
    delete(p);
}

/// One distinct location per active name and array element; -1 for what
/// is not an active uniform (dEQP-GLES2 uniform_api.value.*, which takes
/// every location before it sets or reads).
#[test]
fn locations_of_the_mixed_program() {
    let _g = rig();
    let p = link(VS_MIX, FS_MIX);
    let mut seen = BTreeMap::new();
    for name in MIX_LOCS {
        let l = loc(p, name);
        assert!(l >= 0, "{name}: {l}");
        if let Some(other) = seen.insert(l, name) {
            panic!("{name} and {other} share location {l:#x}");
        }
    }
    /* the array's own name is its element 0 */
    assert_eq!(loc(p, "arr"), loc(p, "arr[0]"));
    for name in ["arr[3]", "s", "lights", "lights[0]", "lights[2].c", "s.x", "nope", "gl_DepthRange.near"] {
        assert_eq!(loc(p, name), -1, "{name}");
    }
    delete(p);
}

/// Every uniform reads 0 after the link, as an int and as a float (ES 2.0
/// 2.10.4; dEQP-GLES2 uniform_api.value.initial.*).
#[test]
fn values_start_at_zero() {
    let _g = rig();
    let p = link(VS_MIX, FS_MIX);
    for name in MIX_LOCS {
        let (l, n) = (loc(p, name), comps(name));
        assert_eq!(getf(p, l, n), vec![0.0; n], "{name} as float");
        assert_eq!(geti(p, l, n), vec![0; n], "{name} as int");
    }
    delete(p);
}

/// Each setter's value reads back (dEQP-GLES2 uniform_api.value.assigned.*):
/// struct members, array elements through one call and one by one, the
/// struct-array members, the matrix, bools through the int and the float
/// setters, samplers through glUniform1i.
#[test]
fn values_read_back() {
    let _g = rig();
    let p = link(VS_MIX, FS_MIX);
    // SAFETY: plain GL calls with the program current; each array holds what its call reads.
    unsafe {
        glUseProgram(p);
        glUniform1f(loc(p, "s.f"), 1.5);
        glUniform3f(loc(p, "s.v"), 2.0, -3.0, 4.25);
        glUniform1i(loc(p, "s.b"), 7);
        let arr: [f32; 12] = core::array::from_fn(|i| i as f32 + 0.5);
        glUniform4fv(loc(p, "arr"), 3, arr.as_ptr());
        let m: [f32; 9] = core::array::from_fn(|i| 10.0 + i as f32);
        glUniformMatrix3fv(loc(p, "m"), 1, 0, m.as_ptr());
        glUniform1f(loc(p, "flag"), 0.25);
        glUniform2i(loc(p, "bv"), 0, -4);
        glUniform3i(loc(p, "iv"), -1, 70000, 3);
        glUniform3f(loc(p, "lights[0].c"), 0.5, 0.25, 0.125);
        glUniform1f(loc(p, "lights[0].w"), 8.0);
        glUniform3fv(loc(p, "lights[1].c"), 1, [9.0f32, 10.0, 11.0].as_ptr());
        glUniform1fv(loc(p, "lights[1].w"), 1, [-2.0f32].as_ptr());
        glUniform1i(loc(p, "tex"), 3);
        glUniform1iv(loc(p, "cube"), 1, [5].as_ptr());
        glUniform1i(loc(p, "count"), -12);
    }
    assert_eq!(err(), GL_NO_ERROR, "the setters");
    assert_eq!(getf(p, loc(p, "s.f"), 1), [1.5]);
    assert_eq!(getf(p, loc(p, "s.v"), 3), [2.0, -3.0, 4.25]);
    assert_eq!(geti(p, loc(p, "s.b"), 1), [1], "bool set to 7 reads 1");
    assert_eq!(getf(p, loc(p, "s.b"), 1), [1.0]);
    for e in 0..3 {
        let base = e as f32 * 4.0 + 0.5;
        assert_eq!(getf(p, loc(p, &format!("arr[{e}]")), 4), [base, base + 1.0, base + 2.0, base + 3.0], "arr[{e}]");
    }
    assert_eq!(getf(p, loc(p, "m"), 9), (0..9).map(|i| 10.0 + i as f32).collect::<Vec<_>>());
    assert_eq!(geti(p, loc(p, "flag"), 1), [1], "bool set to 0.25 reads 1");
    assert_eq!(geti(p, loc(p, "bv"), 2), [0, 1]);
    assert_eq!(getf(p, loc(p, "bv"), 2), [0.0, 1.0]);
    assert_eq!(geti(p, loc(p, "iv"), 3), [-1, 70000, 3]);
    assert_eq!(getf(p, loc(p, "iv"), 3), [-1.0, 70000.0, 3.0]);
    assert_eq!(getf(p, loc(p, "lights[0].c"), 3), [0.5, 0.25, 0.125]);
    assert_eq!(getf(p, loc(p, "lights[0].w"), 1), [8.0]);
    assert_eq!(getf(p, loc(p, "lights[1].c"), 3), [9.0, 10.0, 11.0]);
    assert_eq!(getf(p, loc(p, "lights[1].w"), 1), [-2.0]);
    assert_eq!(geti(p, loc(p, "tex"), 1), [3]);
    assert_eq!(geti(p, loc(p, "cube"), 1), [5]);
    assert_eq!(geti(p, loc(p, "count"), 1), [-12]);

    /* one element through its own location; the others keep theirs */
    // SAFETY: plain GL calls with the program current.
    unsafe {
        glUniform4f(loc(p, "arr[1]"), -1.0, -2.0, -3.0, -4.0);
        /* from element 2 on, a count past the end is cut to the array */
        glUniform4fv(loc(p, "arr[2]"), 2, [7.0f32; 8].as_ptr());
        glUniform1f(loc(p, "flag"), 0.0);
    }
    assert_eq!(err(), GL_NO_ERROR, "the element setters");
    assert_eq!(getf(p, loc(p, "arr[0]"), 4), [0.5, 1.5, 2.5, 3.5]);
    assert_eq!(getf(p, loc(p, "arr[1]"), 4), [-1.0, -2.0, -3.0, -4.0]);
    assert_eq!(getf(p, loc(p, "arr[2]"), 4), [7.0; 4]);
    assert_eq!(geti(p, loc(p, "flag"), 1), [0]);
    /* every other uniform kept its value */
    assert_eq!(getf(p, loc(p, "s.v"), 3), [2.0, -3.0, 4.25]);
    assert_eq!(geti(p, loc(p, "count"), 1), [-12]);
    draws(p, "the mixed program");
    delete(p);
}

/// The ES 2.0 2.10.4 errors of the setters and getters next to those
/// answers (dEQP-GLES2 negative_api.shader.uniform*).
#[test]
fn setter_errors() {
    let _g = rig();
    let p = link(VS_MIX, FS_MIX);
    // SAFETY: plain GL calls with the program current; each array holds what its call reads.
    unsafe {
        glUseProgram(p);
        glUniform1f(loc(p, "count"), 1.0);
        assert_eq!(err(), GL_INVALID_OPERATION, "float setter on an int");
        glUniform1i(loc(p, "s.f"), 1);
        assert_eq!(err(), GL_INVALID_OPERATION, "int setter on a float");
        glUniform1f(loc(p, "tex"), 1.0);
        assert_eq!(err(), GL_INVALID_OPERATION, "float setter on a sampler");
        glUniform2f(loc(p, "s.v"), 1.0, 2.0);
        assert_eq!(err(), GL_INVALID_OPERATION, "vec2 setter on a vec3");
        glUniform4fv(loc(p, "m"), 1, [0.0f32; 16].as_ptr());
        assert_eq!(err(), GL_INVALID_OPERATION, "vec4 setter on a mat3");
        glUniform1iv(loc(p, "tex"), 2, [1, 2].as_ptr());
        assert_eq!(err(), GL_INVALID_OPERATION, "count 2 on a sampler that is not an array");
        glUniform3fv(loc(p, "lights[0].c"), 2, [0.0f32; 6].as_ptr());
        assert_eq!(err(), GL_INVALID_OPERATION, "count 2 on a struct member that is not an array");
        glUniform4fv(loc(p, "arr"), -1, [0.0f32; 4].as_ptr());
        assert_eq!(err(), GL_INVALID_VALUE, "count -1");
        glUniformMatrix3fv(loc(p, "m"), 1, 1, [0.0f32; 9].as_ptr());
        assert_eq!(err(), GL_INVALID_VALUE, "transpose in ES 2.0");
        glUniform1f(-1, 1.0);
        assert_eq!(err(), GL_NO_ERROR, "location -1 is ignored");
        glUniform1f(0x7fff, 1.0);
        assert_eq!(err(), GL_INVALID_OPERATION, "a location the program does not have");
        glUniform1f(-2, 1.0);
        assert_eq!(err(), GL_INVALID_OPERATION, "location -2");
        let mut v = [0.0f32; 4];
        glGetUniformfv(p, 0x7fff, v.as_mut_ptr());
        assert_eq!(err(), GL_INVALID_OPERATION, "reading a location the program does not have");
        glUseProgram(0);
        glUniform1f(loc(p, "s.f"), 1.0);
        assert_eq!(err(), GL_INVALID_OPERATION, "no current program");
    }
    /* none of the refused calls stored anything */
    assert_eq!(getf(p, loc(p, "s.f"), 1), [0.0]);
    assert_eq!(geti(p, loc(p, "tex"), 1), [0]);
    assert_eq!(getf(p, loc(p, "m"), 9), [0.0; 9]);
    delete(p);
}

/* ---- more than 255 active uniforms ---- */

const MANY_ELEMS: usize = 17;
const MANY_MEMBERS: usize = 16;

/// A fragment shader of `lights[17]` of a struct of 16 floats plus `count`:
/// 273 active uniforms, 272 dwords (inside the 384 vectors ES 2.0 reports).
fn fs_many() -> String {
    let members: String = (0..MANY_MEMBERS).map(|m| format!("float m{m}; ")).collect();
    let sum: String = (0..MANY_ELEMS).flat_map(|e| (0..MANY_MEMBERS).map(move |m| format!(" + l[{e}].m{m}"))).collect();
    format!(
        "precision mediump float;\nstruct L {{ {members}}};\nuniform L l[{MANY_ELEMS}];\nuniform int count;\n\
         void main() {{ float a = float(count){sum};\n gl_FragColor = vec4(a); }}\n"
    )
}

const VS_PLAIN: &str = "attribute vec4 p;\nuniform vec4 off;\nvoid main() { gl_Position = p + off; }\n";
const FS_PLAIN: &str = "precision mediump float;\nuniform vec4 col;\nvoid main() { gl_FragColor = col; }\n";

fn many_value(e: usize, m: usize) -> f32 {
    (e * 100 + m) as f32 + 0.5
}

/// Sets every `l[e].mN` of the current program `p` to `many_value`.
fn set_many(p: GLuint) {
    for e in 0..MANY_ELEMS {
        for m in 0..MANY_MEMBERS {
            // SAFETY: a plain GL call.
            unsafe { glUniform1f(loc(p, &format!("l[{e}].m{m}")), many_value(e, m)) };
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "the 272 setters");
}

fn check_many(p: GLuint, set: bool) {
    let mut seen = BTreeMap::new();
    for e in 0..MANY_ELEMS {
        for m in 0..MANY_MEMBERS {
            let name = format!("l[{e}].m{m}");
            let l = loc(p, &name);
            assert!(l >= 0, "{name}: {l}");
            assert!(seen.insert(l, name.clone()).is_none(), "{name} shares location {l:#x}");
            let want = if set { many_value(e, m) } else { 0.0 };
            assert_eq!(getf(p, l, 1), [want], "{name}");
        }
    }
    assert!(loc(p, "count") >= 0);
    assert!(!seen.contains_key(&loc(p, "count")));
    /* the names past the 255th have an index past the old index byte */
    assert!(seen.keys().any(|&l| l & 0xffff > 255), "{seen:?}");
}

/// 274 active uniforms (the old record and location table stopped at 255):
/// the link, GL_ACTIVE_UNIFORMS, the names, a location per name past the
/// index byte, the value of each (dEQP-GLES2 uniform_api.random.* draws
/// programs of struct arrays like this one).
#[test]
fn more_than_255_active_uniforms() {
    let _g = rig();
    let p = link(VS_PLAIN, &fs_many());
    let n = MANY_ELEMS * MANY_MEMBERS + 2; /* + count + off */
    assert_eq!(iv(p, GL_ACTIVE_UNIFORMS), n as GLint);
    let got = actives(p);
    let mut want: Vec<(String, GLint, GLenum)> =
        (0..MANY_ELEMS).flat_map(|e| (0..MANY_MEMBERS).map(move |m| (format!("l[{e}].m{m}"), 1, GL_FLOAT))).collect();
    want.push(("count".to_owned(), 1, GL_INT));
    want.push(("off".to_owned(), 1, GL_FLOAT_VEC4));
    want.sort();
    assert_eq!(got, want);
    /* "l[16].m15" and its NUL */
    assert_eq!(iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), 10);
    check_many(p, false);
    // SAFETY: plain GL calls.
    unsafe { glUseProgram(p) };
    set_many(p);
    // SAFETY: a plain GL call with the program current.
    unsafe { glUniform1i(loc(p, "count"), 42) };
    check_many(p, true);
    assert_eq!(geti(p, loc(p, "count"), 1), [42]);
    draws(p, "274 active uniforms");
    delete(p);
}

/// The location pool of one context: a program's range moves down when a
/// program before it is deleted or relinked, and its locations and values
/// stay as they were (dEQP-GLES2 keeps several programs alive while it
/// links, deletes and relinks others).
#[test]
fn the_location_pool_moves_programs_without_changing_their_answers() {
    let _g = rig();
    let first = link(VS_MIX, FS_MIX);
    let many = link(VS_PLAIN, &fs_many());
    let plain = link(VS_PLAIN, FS_PLAIN);
    // SAFETY: plain GL calls, each with its program current.
    unsafe {
        glUseProgram(many);
        set_many(many);
        glUseProgram(plain);
        glUniform4f(loc(plain, "off"), 0.25, 0.5, 0.75, 1.0);
        glUniform4f(loc(plain, "col"), 1.0, 2.0, 3.0, 4.0);
    }
    let many_locs: Vec<GLint> = (0..MANY_ELEMS * MANY_MEMBERS)
        .map(|i| loc(many, &format!("l[{}].m{}", i / MANY_MEMBERS, i % MANY_MEMBERS)))
        .collect();
    let plain_locs = (loc(plain, "off"), loc(plain, "col"));
    let plain_check = |what: &str| {
        assert_eq!((loc(plain, "off"), loc(plain, "col")), plain_locs, "{what}: plain's locations");
        assert_eq!(getf(plain, plain_locs.0, 4), [0.25, 0.5, 0.75, 1.0], "{what}: off");
        assert_eq!(getf(plain, plain_locs.1, 4), [1.0, 2.0, 3.0, 4.0], "{what}: col");
    };
    let many_locs_now = || -> Vec<GLint> {
        (0..MANY_ELEMS * MANY_MEMBERS)
            .map(|i| loc(many, &format!("l[{}].m{}", i / MANY_MEMBERS, i % MANY_MEMBERS)))
            .collect()
    };

    /* the first program goes: the two after it move down */
    delete(first);
    check_many(many, true);
    assert_eq!(many_locs_now(), many_locs, "many's locations after the delete");
    plain_check("after the delete");
    draws(many, "many, after the delete");
    draws(plain, "plain, after the delete");

    /* a new program takes a range after them; relinking `many` gives its
     * range back and takes a new one, its values start at 0 again */
    let second = link(VS_MIX, FS_MIX);
    relink(many);
    check_many(many, false);
    plain_check("after the relink");
    // SAFETY: plain GL calls with the program current.
    unsafe {
        glUseProgram(second);
        glUniform1i(loc(second, "count"), 9);
    }
    assert_eq!(geti(second, loc(second, "count"), 1), [9]);
    // SAFETY: a plain GL call.
    unsafe { glUseProgram(many) };
    set_many(many);
    check_many(many, true);
    assert_eq!(geti(second, loc(second, "count"), 1), [9], "second after many's relink and setters");
    plain_check("after many's setters");

    /* `many` goes: the ones after it keep their answers */
    delete(many);
    plain_check("after many's delete");
    assert_eq!(geti(second, loc(second, "count"), 1), [9], "second after many's delete");
    draws(second, "second, after many's delete");
    delete(plain);
    delete(second);
}
