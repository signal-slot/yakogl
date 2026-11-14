// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0's varying limits on the V3D (Pi 4), through the GL core, the REAL
//! V3D back end and the REAL job builder (plan #359, docs/notes/v3d-varyings.md).
//!
//! The V3D build answers MAX_VARYING_VECTORS 15, MAX_VARYING_COMPONENTS and
//! MAX_FRAGMENT_INPUT_COMPONENTS 60 and MAX_VERTEX_OUTPUT_COMPONENTS 64
//! (`src/gl/context.rs`). This file checks that a program at those limits
//! compiles, links and draws: `driver/src/v3d/v3d_job.c`, `v3d_frame.c`,
//! `v3d_tex.c` and the Mesa tiling are built with `cc` and loaded with
//! `dlopen` (as tests/es3_flatshade_guard.rs), the job builder's input (the
//! draw's `V3dJobDraw`) and its verdict are recorded, and the draw's binning
//! command list is decoded for the flat-shade flags:
//!
//! - 15 vec4 varyings of GLSL ES 3.00 and 15 of GLSL ES 1.00 draw with 60
//!   fragment inputs, the job builder taking them (it took 24 before);
//! - a vertex shader with 16 vec4 outputs (64 components) and the point size
//!   feeds 15 of them as GL_POINTS;
//! - a flat vec4 that would be inputs 56-59 is moved into the one 24-bit word
//!   of flat flags the shader record carries (FLAT_SHADE_FLAGS 0xf), and 28
//!   flat components fail the link by name rather than drawing smooth;
//! - an ES 1.00 program with 16 varying vectors fails the link (Appendix A
//!   packing against MAX_VARYING_VECTORS 15).
//!
//! What no host test can see: what the silicon interpolates. That is the
//! board's (dEQP-GLES3 `shaders.linkage.varying.*` and the varying limits'
//! rows on the Pi 4).
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

/* ---- the real job builder and layout code, built with cc ------------------ */

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// `driver/src/v3d/v3d_job.c`, `v3d_frame.c`, `v3d_tex.c` and the Mesa
/// tiling, as a shared object of this test's own, reached through the
/// handle only (RTLD_LOCAL, `-Bsymbolic`: its own calls stay inside it).
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

/// The trampolines: the library's `extern "C"` functions
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

/// What the job builder is handed for each draw, and what it answered.
#[derive(Clone, Copy, Debug)]
struct SeenDraw {
    prim: u32,
    num_inputs: u32,
    flat_shade_flags: u32,
    centroid_flags: u32,
    psiz: bool,
    /// V3D_JOB_OK, _FULL or _REFUSED.
    result: i32,
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, layout: *mut c_void) -> i32 {
    // SAFETY: the C function's own signature, as src/v3d/ffi.rs declares it.
    let f: unsafe extern "C" fn(*mut V3dJob, *const V3dJobDraw, *mut c_void) -> i32 =
        unsafe { std::mem::transmute(sym("v3d_job_draw")) };
    // SAFETY: the library's call, passed on.
    let result = unsafe { f(j, d, layout) };
    // SAFETY: the backend passes a live draw; its fragment program lives for the call.
    let rec = unsafe {
        let d = &*d;
        SeenDraw {
            prim: d.prim,
            num_inputs: (*d.fs).num_inputs,
            flat_shade_flags: (*d.fs).flat_shade_flags,
            centroid_flags: (*d.fs).centroid_flags,
            psiz: d.vs_writes_psiz,
            result,
        }
    };
    PLATFORM.lock().unwrap_or_else(|e| e.into_inner()).as_mut().expect("the platform").draws.push(rec);
    result
}

/* ---- the platform: memory that answers, jobs that finish at once ---------- */

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

/* ---- the context ---------------------------------------------------------- */

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

fn geti(pname: GLenum) -> GLint {
    let mut v = [0; 4];
    // SAFETY: room for the one value these pnames yield.
    unsafe { glGetIntegerv(pname, v.as_mut_ptr()) };
    v[0]
}

/* ---- the binning command list, decoded ------------------------------------ */

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

/// The packets of a binning list, as (opcode, bytes). The table is the
/// BCL's: a byte that is no packet of it stops the walk with the bytes seen.
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

/// What one draw's list says, and what the job builder was handed.
#[derive(Debug)]
struct Draw {
    /// The 24-bit flag word of every FLAT_SHADE_FLAGS and its varying
    /// offset (the word index, bits 0-3 of byte 1), and how many
    /// ZERO_ALL_FLAT_SHADE_FLAGS.
    flat_flags: Vec<(u32, u8)>,
    flat_zeroed: usize,
    /// Each GL_SHADER_STATE (one per recorded draw).
    shader_states: usize,
    seen: Vec<SeenDraw>,
}

fn decode(bcl: &[u8], seen: Vec<SeenDraw>) -> Draw {
    let (flat, zero, gs) = (opcode("FLAT_SHADE_FLAGS"), opcode("ZERO_ALL_FLAT_SHADE_FLAGS"), opcode("GL_SHADER_STATE"));
    let mut d = Draw { flat_flags: vec![], flat_zeroed: 0, shader_states: 0, seen };
    for (op, b) in packets(bcl) {
        match op {
            o if o == flat => d.flat_flags.push((u32::from_le_bytes([b[2], b[3], b[4], 0]), b[1] & 0xf)),
            o if o == zero => d.flat_zeroed += 1,
            o if o == gs => d.shader_states += 1,
            _ => {}
        }
    }
    d
}

/* ---- the programs ----------------------------------------------------------- */

/// `n` vec4 varyings `v0..`, flat for each index in `flat`; GLSL ES 3.00 or
/// 1.00. The vertex shader writes `outs` of them (at least `n`), the
/// fragment shader reads `n` and sums them, so each is an input.
fn sources(n: usize, outs: usize, flat: &[usize], es3: bool, psiz: bool) -> (String, String) {
    let (attr, out, inn) = if es3 { ("in", "out", "in") } else { ("attribute", "varying", "varying") };
    let mut vs = String::new();
    let mut fs = String::new();
    if es3 {
        vs.push_str("#version 300 es\n");
        fs.push_str("#version 300 es\n");
    }
    fs.push_str("precision highp float;\n");
    vs.push_str(&format!("{attr} vec4 p;\n"));
    for i in 0..outs {
        let q = if flat.contains(&i) { "flat " } else { "" };
        vs.push_str(&format!("{q}{out} vec4 v{i};\n"));
        if i < n {
            fs.push_str(&format!("{q}{inn} vec4 v{i};\n"));
        }
    }
    if es3 {
        fs.push_str("layout(location = 0) out vec4 o;\n");
    }
    vs.push_str("void main() {\ngl_Position = p;\n");
    if psiz {
        vs.push_str("gl_PointSize = 1.0;\n");
    }
    for i in 0..outs {
        vs.push_str(&format!("v{i} = p + vec4({i}.0);\n"));
    }
    vs.push_str("}\n");
    fs.push_str("void main() {\nvec4 acc = vec4(0.0);\n");
    for i in 0..n {
        fs.push_str(&format!("acc += v{i};\n"));
    }
    fs.push_str(&format!("{} = acc / 1024.0;\n}}\n", if es3 { "o" } else { "gl_FragColor" }));
    (vs, fs)
}

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

/// Link the pair: the program, or the link's info log.
fn link(vs: &str, fs: &str) -> Result<GLuint, String> {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls on handles just made; the log buffer is ours.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        glDeleteShader(v);
        glDeleteShader(f);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok == 1 {
            return Ok(p);
        }
        let mut log = [0 as c_char; 1024];
        let mut n: GLsizei = 0;
        glGetProgramInfoLog(p, log.len() as GLsizei, &mut n, log.as_mut_ptr());
        glDeleteProgram(p);
        Err(CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned())
    }
}

static POS: [f32; 12] = [-0.8, -0.8, 0.0, 1.0, 0.8, -0.8, 0.0, 1.0, 0.0, 0.8, 0.0, 1.0];

/// Draw three vertices of `prog` as `mode` into an RGBA8 renderbuffer (one
/// job), finish, and decode the list.
fn draw(prog: GLuint, mode: GLenum) -> Draw {
    let (mut rb, mut fb) = (0, 0);
    // SAFETY: plain GL calls with room for one name each; POS is a static.
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
        glDrawArrays(mode, 0, 3);
        let e = glGetError();
        glFinish();
        glUseProgram(0);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteProgram(prog);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(1, &rb);
        e
    };
    assert_eq!(e, GL_NO_ERROR, "glDrawArrays(0x{mode:x}): {:?}", with_platform(|p| p.log.clone()));
    let (bcls, seen) = with_platform(|p| (p.bcls.concat(), p.draws.clone()));
    assert!(!bcls.is_empty(), "a job was submitted: {:?}", with_platform(|p| p.log.clone()));
    decode(&bcls, seen)
}

/// The job builder took exactly one draw with `inputs` fragment inputs.
fn one_recorded_draw(d: &Draw, inputs: u32) -> SeenDraw {
    assert_eq!(d.seen.len(), 1, "one draw handed to the job builder: {:?}", d.seen);
    let s = d.seen[0];
    assert_eq!(s.result, V3D_JOB_OK, "the job builder took the draw: {s:?}");
    assert_eq!(s.num_inputs, inputs, "fragment inputs");
    assert_eq!(d.shader_states, 1, "its GL_SHADER_STATE is in the list");
    s
}

/* ---- the tests -------------------------------------------------------------- */

/// The four queries answer ES 3.0's minimums, and GLSL ES 3.00's
/// gl_MaxVertexOutputVectors / gl_MaxFragmentInputVectors (the queries over
/// four, ES 3.00 7.3) exist.
#[test]
fn the_v3d_answers_es3_0s_varying_limits() {
    let _g = rig();
    assert_eq!(geti(GL_MAX_VARYING_VECTORS), 15);
    assert_eq!(geti(GL_MAX_VARYING_COMPONENTS), 60);
    assert_eq!(geti(GL_MAX_FRAGMENT_INPUT_COMPONENTS), 60);
    assert_eq!(geti(GL_MAX_VERTEX_OUTPUT_COMPONENTS), 64);
    // SAFETY: a plain GL call.
    assert_eq!(unsafe { glGetError() }, GL_NO_ERROR);
    let vs = "#version 300 es\nin vec4 p;\nout vec4 v;\nvoid main() { gl_Position = p; \
              v = vec4(float(gl_MaxVertexOutputVectors), 0.0, 0.0, 0.0); }\n";
    let fs = "#version 300 es\nprecision highp float;\nin vec4 v;\nout vec4 o;\n\
              void main() { o = v + vec4(float(gl_MaxFragmentInputVectors)); }\n";
    /* the constants fold to 16 and 15; that the pair compiles is the check
     * that the names exist (tests/es3_caps.rs compares the values) */
    let p = link(vs, fs).expect("the gl_Max*Vectors pair links");
    let d = draw(p, GL_TRIANGLES);
    one_recorded_draw(&d, 4);
}

/// 15 vec4 varyings of GLSL ES 3.00: 60 fragment inputs, which the job
/// builder took 24 of before (v3d_job.c draw_is_supported).
#[test]
fn fifteen_vec4_varyings_draw_with_60_inputs() {
    let _g = rig();
    let (vs, fs) = sources(15, 15, &[], true, false);
    let d = draw(link(&vs, &fs).expect("links"), GL_TRIANGLES);
    let s = one_recorded_draw(&d, 60);
    assert_eq!((s.flat_shade_flags, s.centroid_flags), (0, 0));
    assert_eq!(d.flat_zeroed, 1, "ZERO_ALL_FLAT_SHADE_FLAGS: {d:?}");
    assert!(d.flat_flags.is_empty());
}

/// The same 15 vectors from GLSL ES 1.00 (gl_test's varyings15): the
/// Appendix A packing takes them against MAX_VARYING_VECTORS 15, and 16 is
/// refused at link by name.
#[test]
fn an_es100_program_packs_15_varying_vectors_and_refuses_16() {
    let _g = rig();
    let (vs, fs) = sources(15, 15, &[], false, false);
    let d = draw(link(&vs, &fs).expect("links"), GL_TRIANGLES);
    one_recorded_draw(&d, 60);
    let (vs, fs) = sources(16, 16, &[], false, false);
    let log = link(&vs, &fs).expect_err("16 varying vectors do not fit 15");
    assert!(log.contains("MAX_VARYING_VECTORS = 15"), "{log}");
}

/// A vertex shader with 16 vec4 outputs (MAX_VERTEX_OUTPUT_COMPONENTS 64) and
/// the point size, 15 of them read: 4 + 1 + 60 VPM rows, past the 64 the
/// vertex output stopped at before, drawn as points.
#[test]
fn sixteen_vec4_outputs_and_the_point_size_feed_15_as_points() {
    let _g = rig();
    let (vs, fs) = sources(15, 16, &[], true, true);
    let d = draw(link(&vs, &fs).expect("links"), GL_POINTS);
    let s = one_recorded_draw(&d, 60);
    assert!(s.psiz, "the vertex shader writes the point size");
    assert_eq!(s.prim, V3D_PRIM_POINTS);
}

/// A flat vec4 declared last of 15 would be inputs 56-59, past the one word
/// of flat flags the shader record carries: the compiler puts it first, so
/// the list's FLAT_SHADE_FLAGS is word 0 (varying offset 0) with bits 0-3,
/// the higher words zeroed by its action (the job builder's only form).
#[test]
fn a_late_flat_varying_lands_in_word_0_of_the_flat_flags() {
    let _g = rig();
    let (vs, fs) = sources(15, 15, &[14], true, false);
    let d = draw(link(&vs, &fs).expect("links"), GL_TRIANGLES);
    let s = one_recorded_draw(&d, 60);
    assert_eq!(s.flat_shade_flags, 0xf, "v14's components are inputs 0-3");
    assert_eq!(d.flat_flags, vec![(0xf, 0)], "{d:?}");
    assert_eq!(d.flat_zeroed, 0);
}

/// 24 flat components fill the word; 28 have no bit to go in, and the link
/// fails by name instead of drawing some of them smooth.
#[test]
fn more_than_24_flat_components_fail_the_link_by_name() {
    let _g = rig();
    let six = [9, 10, 11, 12, 13, 14];
    let (vs, fs) = sources(15, 15, &six, true, false);
    let d = draw(link(&vs, &fs).expect("24 flat components link"), GL_TRIANGLES);
    let s = one_recorded_draw(&d, 60);
    assert_eq!(s.flat_shade_flags, 0x00ff_ffff);
    assert_eq!(d.flat_flags, vec![(0x00ff_ffff, 0)], "{d:?}");
    let seven = [8, 9, 10, 11, 12, 13, 14];
    let (vs, fs) = sources(15, 15, &seven, true, false);
    let log = link(&vs, &fs).expect_err("28 flat components do not fit one word");
    assert!(log.contains("more than 24 flat and centroid varying components"), "{log}");
}
