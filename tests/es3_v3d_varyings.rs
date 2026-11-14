// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D's (Pi 4) shader-interface limits at ES 3.0's minimum: 15 varying
//! vectors (60 components in and out of the fragment stage) and 64 vertex
//! output components, through the GL core and the REAL V3D back end with the
//! REAL job builder (`driver/src/v3d/v3d_job.c`, built with `cc` and loaded
//! with `dlopen`, as tests/es3_flatshade_guard.rs does).
//!
//! What is pinned:
//! - the four limits and their derived queries read 15 / 60 / 60 / 64 through
//!   `glGetIntegerv`, `glGetFloatv` and `glGetInteger64v` alike (the dEQP
//!   `implementation_limits` and `state_query.integers*` rows; the float and
//!   64-bit forms are the integer answer converted, with no path of their own
//!   that could fail where the integer one is right);
//! - a program of fifteen vec4 varyings, a third of them flat and a third
//!   centroid, links and draws with 60 fragment inputs, and the binning list
//!   carries the flags in the three 24-input words the packets hold
//!   (FLAT_SHADE_FLAGS and CENTROID_FLAGS at varying offsets 0, 1 and 2, as
//!   Mesa's v3dx_emit.c emit_varying_flags writes them);
//! - gl_PointSize beside the fifteen (65 VPM rows) draws;
//! - a vertex shader writing sixteen vec4 beside a fragment shader reading
//!   fifteen links and draws (64 vertex output components).
//!
//! What no host test can see: the hardware's interpolation of the 60 inputs.
//! That is the Pi 4's (`implementation_limits` and the `shaders.linkage`
//! rows).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_varyings
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
/* the fakes and trampolines take the C functions' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
use yakogl::v3d::backend::yakogl_backend_v3d;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/// ES 3.0 table 6.31: fifteen vec4 in, and 64 components out.
const N: usize = 15;

/* ---- the real job builder and layout code, built with cc -------------------------- */

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// The V3D job builder, frame and texture code and the Mesa tiling, as a
/// shared object of this test's own, reached through the handle only
/// (RTLD_LOCAL, `-Bsymbolic`: its own calls stay inside it).
fn lib() -> *mut c_void {
    static L: OnceLock<usize> = OnceLock::new();
    *L.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_v3d_varyings");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_job-{}.so", std::process::id()));
        /* the compiler's own temporary files next to the object */
        let st = Command::new("cc")
            .current_dir(&root)
            .env("TMPDIR", &dir)
            .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror", "-Wl,-Bsymbolic"])
            .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
            .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
            .arg("-Ithird_party/mesa-26.2.1/include")
            .args(["driver/src/v3d/v3d_job.c", "driver/src/v3d/v3d_frame.c"])
            .args(["driver/src/v3d/v3d_tex.c", "driver/src/v3d/v3d_tiling_mesa.c", "-lm", "-o"])
            .arg(&so)
            .status()
            .expect("cc runs");
        assert!(st.success(), "the V3D job builder builds");
        let path = CString::new(so.to_str().expect("a utf-8 path")).unwrap();
        // SAFETY: a path of a library this test just built; RTLD_NOW.
        let h = unsafe { dlopen(path.as_ptr(), 2) };
        assert!(!h.is_null(), "the job builder loads");
        let _ = std::fs::remove_file(&so);
        h as usize
    }) as *mut c_void
}

fn sym(name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    // SAFETY: a handle `lib` made and a NUL-terminated name.
    let p = unsafe { dlsym(lib(), c.as_ptr()) };
    assert!(!p.is_null(), "{name} in the job builder");
    p
}

/// The trampolines: the library's `extern "C"` functions (src/v3d/ffi.rs),
/// each handed to the real C.
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

    trampoline!(v3d_job_begin(j: *mut V3dJob, t: *const V3dJobTarget, bcl: *mut V3dCl, rcl: *mut V3dCl, ind: *mut V3dCl) -> bool);
    trampoline!(v3d_job_end(j: *mut V3dJob, c: *const V3dJobClear, tile_list_start: *mut u32, tile_list_end: *mut u32) -> bool);
    trampoline!(v3d_frame_tiling(width: u32, height: u32, t: *mut V3dTiling));
    trampoline!(v3d_frame_tiling_rts(width: u32, height: u32, rts: u32, t: *mut V3dTiling));
    trampoline!(v3d_frame_tiling_ms(width: u32, height: u32, rts: u32, msaa: bool, t: *mut V3dTiling));
    trampoline!(v3d_frame_tiling_bpp(width: u32, height: u32, rts: u32, msaa: bool, max_bpp: u32, t: *mut V3dTiling));
    trampoline!(v3d_frame_alloc_sizes(t: *const V3dTiling, draws: u32, page_size: u32, tile_alloc_size: *mut u32, tile_state_size: *mut u32));
    trampoline!(v3d_tex_layout(width: u32, height: u32, levels: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
    trampoline!(v3d_tex_layout_msaa(width: u32, height: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
    trampoline!(v3d_tex_store_level(image: *mut u8, l: *const V3dTexLayout, level: u32, src: *const u8, src_stride: u32));
    trampoline!(v3d_tex_tfu_pack(l: *const V3dTexLayout, ttype: u32, src_gpu: u32, src_stride: u32, dst_gpu: u32,
                                 j: *mut V3dTexTfuJob) -> bool);
    trampoline!(v3d_tex_tfu_pack_mipmap(l: *const V3dTexLayout, ttype: u32, image_gpu: u32, j: *mut V3dTexTfuJob) -> bool);
    trampoline!(v3d_tex_layer_stride(l: *const V3dTexLayout) -> u32);
    trampoline!(v3d_tex_layers_size(l: *const V3dTexLayout, layers: u32, out: *mut u32) -> bool);
    trampoline!(v3d_tex_store_box(image: *mut u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                  src: *const u8, src_stride: u32) -> bool);
    trampoline!(v3d_tex_load_box(image: *const u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                 dst: *mut u8, dst_stride: u32) -> bool);
    trampoline!(v3d_tex_state_from_layout(s: *mut V3dTexState, l: *const V3dTexLayout));
    trampoline!(v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool);
    trampoline!(v3d_tex_pack_sampler(out: *mut u8, s: *const V3dSampler) -> bool);
    trampoline!(v3d_tex_pack_sampler_compare(out: *mut u8, s: *const V3dSampler, compare_op: u32) -> bool);
}

/// What the job builder is handed for each draw.
#[derive(Clone, Copy, Debug)]
struct SeenDraw {
    num_inputs: u32,
    flat: [u32; 3],
    centroid: [u32; 3],
    vs_output_size: u32,
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes a live draw; its fragment and vertex programs live for the call.
    let rec = unsafe {
        let d = &*d;
        let f = &*d.fs;
        SeenDraw {
            num_inputs: f.num_inputs,
            flat: [f.flat_shade_flags, f.flat_shade_flags_hi[0], f.flat_shade_flags_hi[1]],
            centroid: [f.centroid_flags, f.centroid_flags_hi[0], f.centroid_flags_hi[1]],
            vs_output_size: (*d.vs).vpm_output_size,
        }
    };
    PLATFORM.lock().unwrap_or_else(|e| e.into_inner()).as_mut().expect("the platform").draws.push(rec);
    // SAFETY: the C function's own signature, as src/v3d/ffi.rs declares it.
    let f: unsafe extern "C" fn(*mut V3dJob, *const V3dJobDraw, *mut c_void) -> i32 =
        unsafe { std::mem::transmute(sym("v3d_job_draw")) };
    // SAFETY: the library's call, passed on.
    unsafe { f(j, d, layout) }
}

/* ---- the platform: memory that answers, jobs that finish at once ------------------- */

struct Alloc {
    mem: Box<[u64]>,
    size: usize,
}

#[derive(Default)]
struct Platform {
    next: u64,
    allocs: BTreeMap<u64, Alloc>,
    /// The binning command list of each kick, as submitted.
    bcls: Vec<Vec<u8>>,
    draws: Vec<SeenDraw>,
    log: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
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

/// A kick: keep the binning command list it names.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<V3dKick>() {
        return -22;
    }
    // SAFETY: the back end passes a live V3dKick of that size.
    let k = unsafe { *(kick as *const V3dKick) };
    with_platform(|p| {
        let bytes = p.allocs.range(..=k.bcl_start as u64).next_back().and_then(|(base, a)| {
            let (lo, hi) = ((k.bcl_start as u64 - base) as usize, (k.bcl_end as u64 - base) as usize);
            // SAFETY: the allocation owns `size` bytes behind its u64 backing.
            let all = unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u8, a.size) };
            all.get(lo..hi).map(|s| s.to_vec())
        });
        p.bcls.push(bytes.unwrap_or_default());
        p.bcls.len() as i64
    })
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.bcls.len() as i64)
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
    with_platform(|p| p.log.push(s));
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

/* ---- the context --------------------------------------------------------------------- */

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The one context and back end of the file; the tests take turns and each
/// starts with nothing recorded and nothing pending.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_v3d;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", with_platform(|p| p.log.clone()));
            assert!(!context::gl_context_create(32, 32).is_null(), "{:?}", with_platform(|p| p.log.clone()));
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    with_platform(|p| {
        p.bcls.clear();
        p.draws.clear();
    });
    g
}

/* ---- the binning command list, decoded ----------------------------------------------- */

/// `(opcode, length)` of every packet of the generated V3D 4.2 header
/// (`#define V3D42_<NAME>_opcode` and `_length`), by name.
fn packet_table() -> &'static BTreeMap<String, (u8, usize)> {
    static T: OnceLock<BTreeMap<String, (u8, usize)>> = OnceLock::new();
    T.get_or_init(|| {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/driver/src/v3d/generated/v3d_packet_v42_pack.h");
        let text = std::fs::read_to_string(path).expect("the generated V3D 4.2 packet header");
        let mut op: BTreeMap<String, u8> = BTreeMap::new();
        let mut len: BTreeMap<String, usize> = BTreeMap::new();
        for l in text.lines() {
            let mut it = l.split_whitespace();
            if it.next() != Some("#define") {
                continue;
            }
            let (Some(name), Some(v)) = (it.next(), it.next()) else { continue };
            let Some(name) = name.strip_prefix("V3D42_") else { continue };
            if let (Some(n), Ok(v)) = (name.strip_suffix("_opcode"), v.parse::<u8>()) {
                op.insert(n.to_string(), v);
            } else if let (Some(n), Ok(v)) = (name.strip_suffix("_length"), v.parse::<usize>()) {
                len.insert(n.to_string(), v);
            }
        }
        op.into_iter().filter_map(|(n, o)| len.get(&n).map(|&l| (n, (o, l)))).collect()
    })
}

fn opcode(name: &str) -> u8 {
    packet_table().get(name).unwrap_or_else(|| panic!("{name} in the packet header")).0
}

/// The packets of a binning list, as (opcode, bytes). The table is the BCL's:
/// a byte that is no packet of it stops the walk with the bytes seen.
fn packets(bcl: &[u8]) -> Vec<(u8, &[u8])> {
    let by_op: BTreeMap<u8, usize> = packet_table()
        .iter()
        // the render list's packets share the opcode space above the BCL's
        .filter(|(n, _)| !n.starts_with("TILE_") || n.starts_with("TILE_BINNING") || n.starts_with("TILE_COORD"))
        .map(|(_, &(o, l))| (o, l))
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bcl.len() {
        let Some(&l) = by_op.get(&bcl[i]) else { panic!("opcode {} at {i}: {:02x?}", bcl[i], &bcl[..i]) };
        out.push((bcl[i], &bcl[i..(i + l).min(bcl.len())]));
        i += l;
    }
    out
}

/// One varying flags packet: its varying offset (words of 24 inputs), the
/// actions on the lower and higher words (0 unchanged, 1 zeroed, 2 set) and
/// the 24-bit word.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct Flags {
    offset: u8,
    lower: u8,
    higher: u8,
    word: u32,
}

/// What the one draw's list says.
#[derive(Debug)]
struct Draw {
    flat: Vec<Flags>,
    flat_zeroed: usize,
    centroid: Vec<Flags>,
    centroid_zeroed: usize,
    /// What the job builder was handed.
    seen: Vec<SeenDraw>,
}

fn decode(bcl: &[u8], seen: Vec<SeenDraw>) -> Draw {
    let op = |n| opcode(n);
    let (flat, zflat, centroid, zcentroid) =
        (op("FLAT_SHADE_FLAGS"), op("ZERO_ALL_FLAT_SHADE_FLAGS"), op("CENTROID_FLAGS"), op("ZERO_ALL_CENTROID_FLAGS"));
    let flags = |b: &[u8]| Flags {
        offset: b[1] & 0xf,
        lower: b[1] >> 4 & 3,
        higher: b[1] >> 6,
        word: u32::from_le_bytes([b[2], b[3], b[4], 0]),
    };
    let mut d = Draw { flat: vec![], flat_zeroed: 0, centroid: vec![], centroid_zeroed: 0, seen };
    for (o, b) in packets(bcl) {
        match o {
            o if o == flat => d.flat.push(flags(b)),
            o if o == zflat => d.flat_zeroed += 1,
            o if o == centroid => d.centroid.push(flags(b)),
            o if o == zcentroid => d.centroid_zeroed += 1,
            _ => {}
        }
    }
    d
}

/* ---- the programs --------------------------------------------------------------------- */

/// The interpolation qualifier of varying `i`: smooth, flat, centroid in turn.
fn qual(i: usize) -> &'static str {
    ["", "flat ", "centroid "][i % 3]
}

/// A vertex shader writing `n` vec4 varyings and a fragment shader reading
/// the first `reads` of them (all summed, so each is an input), with
/// `point_size` adding a gl_PointSize write.
fn sources(n: usize, reads: usize, point_size: bool) -> (String, String) {
    let mut vs = String::from("#version 300 es\nin vec4 p;\nin vec4 c;\n");
    let mut fs = String::from("#version 300 es\nprecision mediump float;\nout vec4 o;\n");
    for i in 0..n {
        vs += &format!("{}out vec4 v{i};\n", qual(i));
        if i < reads {
            fs += &format!("{}in vec4 v{i};\n", qual(i));
        }
    }
    vs += "void main() {\n    gl_Position = p;\n";
    if point_size {
        vs += "    gl_PointSize = 1.0;\n";
    }
    for i in 0..n {
        vs += &format!("    v{i} = c * {:.4};\n", (i + 1) as f32 / 16.0);
    }
    vs += "}\n";
    fs += "void main() {\n    vec4 s = vec4(0.0);\n";
    for i in 0..reads {
        fs += &format!("    s += v{i};\n");
    }
    fs += "    o = s / 8.0;\n}\n";
    (vs, fs)
}

static POS: [f32; 12] = [-0.8, -0.8, 0.0, 1.0, 0.8, -0.8, 0.0, 1.0, 0.0, 0.8, 0.0, 1.0];
static COL: [f32; 12] = [1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0];

fn shader(ty: GLenum, src: &str) -> GLuint {
    let c = CString::new(src).unwrap();
    // SAFETY: plain GL calls; the string lives for the call.
    unsafe {
        let s = glCreateShader(ty);
        let srcs = [c.as_ptr()];
        glShaderSource(s, 1, srcs.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "compile {ty:#x}: {src}\n{:?}", with_platform(|p| p.log.clone()));
        s
    }
}

/// Link the pair: the program, or the log of the link that failed.
fn link(vs: &str, fs: &str) -> Result<GLuint, String> {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls on handles just made.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glBindAttribLocation(p, 1, c"c".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        glDeleteShader(v);
        glDeleteShader(f);
        if ok != 1 {
            let mut log = [0 as c_char; 512];
            glGetProgramInfoLog(p, 512, core::ptr::null_mut(), log.as_mut_ptr());
            glDeleteProgram(p);
            return Err(CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned());
        }
        Ok(p)
    }
}

/// Draw one triangle with the program of `vs` and `fs` into an RGBA8
/// renderbuffer (one job), finish, and decode the list.
fn draw(vs: &str, fs: &str) -> Draw {
    with_platform(|p| {
        p.bcls.clear();
        p.draws.clear();
    });
    let prog = link(vs, fs).unwrap_or_else(|e| panic!("link: {e}\n{vs}\n{fs}"));
    let (mut rb, mut fb) = (0, 0);
    // SAFETY: plain GL calls with room for one name each; POS and COL are statics.
    let e = unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 16, 16);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        let log = with_platform(|p| p.log.clone());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{log:?}");
        glViewport(0, 0, 16, 16);
        glUseProgram(prog);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, COL.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        let e = glGetError();
        glFinish();
        glUseProgram(0);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteProgram(prog);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(1, &rb);
        e
    };
    assert_eq!(e, GL_NO_ERROR, "glDrawArrays");
    let (bcls, seen) = with_platform(|p| (p.bcls.concat(), p.draws.clone()));
    assert!(!bcls.is_empty(), "a job was submitted: {:?}", with_platform(|p| p.log.clone()));
    decode(&bcls, seen)
}

/* ---- the limits ------------------------------------------------------------------------ */

/// The four limits read the ES 3.0 minimum through the three query forms.
#[test]
fn the_four_limits_read_15_60_60_64_in_every_query_form() {
    let _g = rig();
    for (name, pname, want) in [
        ("GL_MAX_VARYING_VECTORS", GL_MAX_VARYING_VECTORS, 15),
        ("GL_MAX_VARYING_COMPONENTS", GL_MAX_VARYING_COMPONENTS, 60),
        ("GL_MAX_FRAGMENT_INPUT_COMPONENTS", GL_MAX_FRAGMENT_INPUT_COMPONENTS, 60),
        ("GL_MAX_VERTEX_OUTPUT_COMPONENTS", GL_MAX_VERTEX_OUTPUT_COMPONENTS, 64),
    ] {
        let (mut i, mut f, mut l) = (-1 as GLint, -1.0 as GLfloat, -1 as GLint64);
        // SAFETY: one value each, as every one of these pnames yields.
        unsafe {
            glGetIntegerv(pname, &mut i);
            glGetFloatv(pname, &mut f);
            glGetInteger64v(pname, &mut l);
            assert_eq!(glGetError(), GL_NO_ERROR, "{name}");
        }
        assert_eq!((i, f, l), (want, want as f32, want as i64), "{name}: integer, float, 64-bit");
    }
    assert_eq!(context::MAX_VARYING_VECTORS, N);
}

/* ---- the draws ------------------------------------------------------------------------- */

/// Fifteen vec4 varyings, one in three flat and one in three centroid: the
/// program has 60 inputs and the list carries the flags in three packets
/// each (word 0, 1 and 2: offsets 0, 1, 2), the first leaving the lower
/// words as they are and zeroing the higher, as Mesa's emit_varying_flags.
#[test]
fn fifteen_varyings_draw_with_their_flags_in_three_words() {
    let _g = rig();
    let (vs, fs) = sources(N, N, false);
    let d = draw(&vs, &fs);
    assert_eq!(d.seen.len(), 1, "one draw");
    let s = d.seen[0];
    assert_eq!(s.num_inputs, 60, "every component is an input");
    /* inputs are in `ldvary` order: the draw does not say which varying
     * each is, but five of fifteen vec4 are flat and five centroid, over
     * all three words */
    let ones = |w: [u32; 3]| w.iter().map(|x| x.count_ones()).sum::<u32>();
    assert_eq!((ones(s.flat), ones(s.centroid)), (20, 20), "flat {:x?}, centroid {:x?}", s.flat, s.centroid);
    assert!(s.flat.iter().all(|w| w >> 24 == 0) && s.centroid.iter().all(|w| w >> 24 == 0), "24 inputs a word");
    assert!(s.flat[1] != 0 && s.flat[2] != 0 && s.centroid[1] != 0 && s.centroid[2] != 0, "all three words used");
    for (name, packets, zeroed, words) in
        [("flat", &d.flat, d.flat_zeroed, s.flat), ("centroid", &d.centroid, d.centroid_zeroed, s.centroid)]
    {
        assert_eq!(zeroed, 0, "{name}: not zeroed");
        let want: Vec<Flags> = (0..3)
            .map(|w| Flags {
                offset: w as u8,
                /* the first word leaves the lower ones alone (there are none), the others
                 * follow it unchanged */
                lower: 0,
                higher: u8::from(w == 0),
                word: words[w],
            })
            .collect();
        assert_eq!(packets, &want, "{name}: one packet a word");
    }
}

/// gl_PointSize beside fifteen vec4: the point size and 60 varyings are 65
/// VPM rows, nine sectors; draws, no refusal.
#[test]
fn fifteen_varyings_and_a_point_size_draw() {
    let _g = rig();
    let (vs, fs) = sources(N, N, true);
    let d = draw(&vs, &fs);
    assert_eq!(d.seen.len(), 1);
    assert_eq!((d.seen[0].num_inputs, d.seen[0].vs_output_size), (60, 9), "{:?}", d.seen[0]);
}

/// A vertex shader writing sixteen vec4 (64 output components) beside a
/// fragment shader reading fifteen links and draws; the sixteenth is not an
/// input and takes no row.
#[test]
fn sixteen_vertex_outputs_beside_fifteen_fragment_inputs_link_and_draw() {
    let _g = rig();
    let (vs, fs) = sources(16, N, false);
    let d = draw(&vs, &fs);
    assert_eq!(d.seen[0].num_inputs, 60);
    assert_eq!(d.seen[0].vs_output_size, 8, "4 fixed rows and 60 varyings: 64 rows");
}

/// An ES 1.00 program of `n` vec4 varyings read by the fragment shader.
fn es100(n: usize) -> (String, String) {
    let mut vs = String::from("attribute vec4 p;\nattribute vec4 c;\n");
    let mut fs = String::from("precision mediump float;\n");
    for i in 0..n {
        vs += &format!("varying vec4 v{i};\n");
        fs += &format!("varying vec4 v{i};\n");
    }
    vs += "void main() {\n    gl_Position = p;\n";
    fs += "void main() {\n    vec4 s = vec4(0.0);\n";
    for i in 0..n {
        vs += &format!("    v{i} = c * {:.4};\n", (i + 1) as f32 / 16.0);
        fs += &format!("    s += v{i};\n");
    }
    (vs + "}\n", fs + "    gl_FragColor = s / 8.0;\n}\n")
}

/// The Appendix A budget is the reported limit: an ES 1.00 program of
/// fifteen vec4 links, one of sixteen does not, by the limit's name.
#[test]
fn es100_fifteen_link_and_sixteen_are_refused_by_the_limit() {
    let _g = rig();
    let (vs, fs) = es100(N);
    link(&vs, &fs).map(|p| unsafe { glDeleteProgram(p) }).expect("fifteen vec4 link");
    let (vs, fs) = es100(N + 1);
    let e = link(&vs, &fs).expect_err("sixteen vec4 do not");
    assert!(
        e.contains("GL_MAX_VARYING_VECTORS") || e.contains("MAX_VARYING_VECTORS = 15"),
        "the log names the limit: {e}"
    );
}
