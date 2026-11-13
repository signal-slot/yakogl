// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against the V3D refusals of b7c1484e (v3d refusals of
//! legal GL state answer -ENOTSUP, GL_INVALID_OPERATION, not -EINVAL,
//! GL_OUT_OF_MEMORY). The Pi 4 whole dEQP-GLES2 tree (46f93767) predates it,
//! and its stage-1 promise is 0 Fail and only the 34 NotSupported of
//! tools/cts/es2-accepted.txt. tests/es3_v3d_refusal_gl.rs pins the ES 3.0
//! answers; this file replays the ES 2.0 state that sits next to each refusal
//! site, through the real GL core (a `report-es2` build, GLSL ES 1.00
//! shaders) and the real V3D back end, and asserts that the call or the draw is
//! accepted: GL_NO_ERROR and `v3d_refused_draws()` unchanged. The job builder
//! is stood in for (as in es3_v3d_refusal_gl.rs); the texture layout and
//! tiling code is the real driver/src/v3d/v3d_tex.c.
//!
//! Every refusal site b7c1484e moved to -ENOTSUP, whether ES 2.0 reaches it,
//! and the test that asserts it (or why it cannot be reached):
//!
//! | site (src/v3d) | ES 2.0 | here |
//! | --- | --- | --- |
//! | draw_call: 4x multisampling, no colour target | no: the core makes no multisampled renderbuffer (glRenderbufferStorageMultisample is ES 3.0) and dEQP-GLES2 asks for a 0-sample surface | - |
//! | draw_call: target outside the 32-bit address space | no: a guard (needs a target above 4 GiB) | - |
//! | draw_call: target past 2048 a side | by the API yes (GL_MAX_RENDERBUFFER_SIZE), by dEQP-GLES2 no (no case clears or draws a target past 256 a side) | `a_2048_target_is_accepted`; the 4096 one is #[ignore] |
//! | extra_targets: attachments of different sizes / past the 32-bit space / 4x several attachments / 4x an ES 3.0 format | no: ES 2.0 has one colour attachment (no GL_EXT_draw_buffers) and no 4x target | - |
//! | draw_call: a colour output with no attachment slot | no: gl_FragColor / gl_FragData[0] are output 0, the slot every target has | `fragcolor_and_fragdata_zero_have_a_slot` |
//! | begin_job: a 64/128-bpp target with more tiles than the pools hold | no: the 64-bit target (GL_EXT_color_buffer_half_float) is only reachable past 2048 a side, which the 2048 cap already stops | `a_half_float_target_draws` |
//! | state::unsupported: a second index list in one draw | no: the GL core never sets `index_count2` | - |
//! | record: depth/stencil read an earlier job wrote, no buffer to keep it | yes: a depth or stencil test in a second job | `depth_and_stencil_across_jobs` |
//! | record: gl_InstanceID of an instance drawn alone | no: GLSL ES 1.00 has no gl_InstanceID, and no instanced draw | - |
//! | record: a non-float32 attribute | yes: BYTE, UNSIGNED_BYTE, SHORT, UNSIGNED_SHORT, FIXED, normalized or not | `every_attribute_type_draws` |
//! | record: a vertex array / index buffer outside the 32-bit space | no: a guard (needs memory above 4 GiB) | `index_types_draw` for the indices |
//! | record: the job builder's refusal (the TF1/TF2 stop) | no: transform feedback is ES 3.0 (the stand-in builder never refuses) | - |
//! | texture::GAPS: a texel format not sampled (ETC2, RGB10A2) | ETC1 (GL_OES_compressed_ETC1_RGB8_texture) yes, the rest ES 3.0 | `etc1_texture_samples` |
//! | texture::GAPS: a 16-bit format read by a 32-bit look-up and the converse | the depth texture (GL_OES_depth_texture) is ES 2.0's one 32-bit class | `depth_texture_samples` |
//! | tex_state_for: tiled-copy gaps (unstamped EGLImage, over budget, fill refused, address) | over budget: yes, by texture.size.2d.2048x2048_* (22,376,448 bytes tiled with the chain): accepted with the 24 MiB of image T5 (texture-2048.conf, the rig answers that budget), refused -ENOTSUP with the 16 MiB of the other images, which is why the tree splits it into T5; an EGLImage and the address are not dEQP-GLES2 state | `a_large_texture_samples` |
//! | black_typed_state: an incomplete 32-bit-class texture | yes: an incomplete depth texture | `incomplete_depth_texture_reads_black` |
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_refusals
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

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, _d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_copy_reasons");
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
    g
}

/* ---- helpers ---- */

use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::texture::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::v3d::backend::v3d_refused_draws;

const W: GLsizei = 32;
const H: GLsizei = 32;

const VS: &str = "attribute vec4 p;\nvarying vec2 t;\nvoid main() { gl_Position = p; t = p.xy * 0.5 + 0.5; }\n";
const FS_COLOR: &str = "precision mediump float;\nvarying vec2 t;\nvoid main() { gl_FragColor = vec4(t, 0.5, 1.0); }\n";
const FS_FRAGDATA: &str =
    "precision mediump float;\nvarying vec2 t;\nvoid main() { gl_FragData[0] = vec4(t, 0.5, 1.0); }\n";
const FS_TEX: &str = "precision mediump float;\nvarying vec2 t;\nuniform sampler2D s;\n\
                      void main() { gl_FragColor = texture2D(s, t); }\n";
const FS_CUBE: &str = "precision mediump float;\nvarying vec2 t;\nuniform samplerCube s;\n\
                       void main() { gl_FragColor = textureCube(s, vec3(t, 0.5)); }\n";

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
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

/// Make `fs` (with `VS`) the current program; sampler `s`, if any, reads unit 0.
fn program(fs: &str) {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
        let s = glGetUniformLocation(p, c"s".as_ptr());
        if s >= 0 {
            glUniform1i(s, 0);
        }
    }
}

/// The one rule of the file: what `f` does is accepted. No GL error and the
/// back end refused no draw (`v3d_refused_draws` is read as a difference).
fn accepted(what: &str, f: impl FnOnce()) {
    let base = v3d_refused_draws();
    f();
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    let (e, n) = (err(), v3d_refused_draws().wrapping_sub(base));
    assert_eq!((e, n), (GL_NO_ERROR, 0), "{what}: error, refused draws; {:?}", log_lines());
}

static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

fn draw_triangle() {
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
}

fn bind_fbo() -> GLuint {
    let mut fb = 0;
    // SAFETY: plain GL calls with room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glViewport(0, 0, W, H);
    }
    fb
}

fn renderbuffer(attachment: GLenum, format: GLenum, w: GLsizei, h: GLsizei) {
    // SAFETY: plain GL calls with room for one name.
    unsafe {
        let mut rb = 0;
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, GL_RENDERBUFFER, rb);
    }
}

fn complete() -> bool {
    // SAFETY: a plain GL call.
    let ok = unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) == GL_FRAMEBUFFER_COMPLETE };
    if !ok {
        eprintln!("skipped: the framebuffer is not complete (the GL core's answer, not a back-end refusal)");
    }
    ok
}

fn unbind_fbo(fb: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
    }
}

/// A 2D texture bound to unit 0: `w` x `h` of `format`/`type_` filled with
/// 0x7f, the filter and wrap given.
fn texture(
    target: GLenum,
    internal: GLenum,
    format: GLenum,
    type_: GLenum,
    w: GLsizei,
    h: GLsizei,
    min: GLenum,
) -> GLuint {
    let mut t = 0;
    let texels = vec![0x7fu8; (w * h) as usize * 16];
    // SAFETY: plain GL calls; the buffer holds 16 bytes a texel.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(target, t);
        let faces: &[GLenum] = if target == GL_TEXTURE_CUBE_MAP {
            &[
                GL_TEXTURE_CUBE_MAP_POSITIVE_X,
                GL_TEXTURE_CUBE_MAP_POSITIVE_X + 1,
                GL_TEXTURE_CUBE_MAP_POSITIVE_X + 2,
                GL_TEXTURE_CUBE_MAP_POSITIVE_X + 3,
                GL_TEXTURE_CUBE_MAP_POSITIVE_X + 4,
                GL_TEXTURE_CUBE_MAP_POSITIVE_X + 5,
            ]
        } else {
            &[GL_TEXTURE_2D]
        };
        for f in faces {
            glTexImage2D(*f, 0, internal as GLint, w, h, 0, format, type_, texels.as_ptr() as *const c_void);
        }
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, min as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
    t
}

fn extension(name: &str) -> bool {
    let has = extension_listed(name);
    if !has {
        eprintln!("skipped: {name} is not advertised");
    }
    has
}

fn extension_listed(name: &str) -> bool {
    // SAFETY: GL_EXTENSIONS is a NUL-terminated string.
    let s = unsafe { CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char) };
    s.to_string_lossy().split(' ').any(|e| e == name)
}

/* ---- the cases ---- */

/// Sanity: this is an ES 2.0 context. GLSL ES 3.00 does not compile and
/// GL_VERSION says ES 2.0 (the Pi 4 tree's build, `report-es2`).
#[test]
fn the_context_is_es2() {
    let _g = rig();
    // SAFETY: plain GL calls.
    let major = unsafe {
        let mut m = 0;
        glGetIntegerv(GL_MAJOR_VERSION, &mut m);
        m
    };
    assert_eq!(err(), GL_INVALID_ENUM, "GL_MAJOR_VERSION is an ES 3.0 query");
    assert_eq!(major, 0);
}

/// "a colour output with no attachment slot": gl_FragColor and gl_FragData[0]
/// are output 0, on the default surface and on an FBO with one attachment of
/// every ES 2.0 renderbuffer format.
#[test]
fn fragcolor_and_fragdata_zero_have_a_slot() {
    let _g = rig();
    for fs in [FS_COLOR, FS_FRAGDATA] {
        accepted("the default framebuffer", || {
            program(fs);
            draw_triangle();
        });
    }
    for fmt in [GL_RGBA4, GL_RGB5_A1, GL_RGB565] {
        let fb = bind_fbo();
        renderbuffer(GL_COLOR_ATTACHMENT0, fmt, W, H);
        assert!(complete(), "{fmt:#x}");
        accepted(&format!("renderbuffer {fmt:#x}"), || {
            // SAFETY: plain GL calls.
            unsafe {
                glClearColor(0.1, 0.2, 0.3, 0.4);
                glClear(GL_COLOR_BUFFER_BIT);
            }
            for fs in [FS_COLOR, FS_FRAGDATA] {
                program(fs);
                draw_triangle();
            }
        });
        unbind_fbo(fb);
    }
    /* a texture attachment, RGBA and RGB */
    for (internal, format) in [(GL_RGBA, GL_RGBA), (GL_RGB, GL_RGB)] {
        let fb = bind_fbo();
        let t = texture(GL_TEXTURE_2D, internal, format, GL_UNSIGNED_BYTE, W, H, GL_LINEAR);
        // SAFETY: plain GL call.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0) };
        if complete() {
            accepted(&format!("texture target {format:#x}"), || {
                // SAFETY: a plain GL call.
                unsafe { glClear(GL_COLOR_BUFFER_BIT) };
                program(FS_FRAGDATA);
                draw_triangle();
            });
        }
        unbind_fbo(fb);
    }
}

/// The 2048 a side the tile memory is sized for: the largest target that is
/// accepted.
#[test]
fn a_2048_target_is_accepted() {
    let _g = rig();
    let fb = bind_fbo();
    renderbuffer(GL_COLOR_ATTACHMENT0, GL_RGBA4, 2048, 16);
    assert!(complete());
    accepted("2048x16 clear", || {
        // SAFETY: a plain GL call.
        unsafe { glClear(GL_COLOR_BUFFER_BIT) };
    });
    unbind_fbo(fb);
}

/// The ES 2.0 API takes a target of GL_MAX_RENDERBUFFER_SIZE (4096) a side,
/// which the back end refuses (-ENOTSUP, GL_INVALID_OPERATION; before
/// b7c1484e -EINVAL, GL_OUT_OF_MEMORY, ending a dEQP session). No
/// dEQP-GLES2 case clears or draws such a target (the fbo.* cases are 1 to 257
/// a side, the texture.size ones render into the 256x256 surface), so this is
/// no Pi 4 tree risk: kept as the pin of a known gap.
#[test]
#[ignore = "a 4096-wide renderbuffer is refused by the V3D (past the 2048 tile memory): GL_INVALID_OPERATION; no dEQP-GLES2 case draws one"]
fn a_4096_target_is_accepted() {
    let _g = rig();
    let fb = bind_fbo();
    renderbuffer(GL_COLOR_ATTACHMENT0, GL_RGBA4, 4096, 16);
    assert!(complete());
    accepted("4096x16 clear", || {
        // SAFETY: a plain GL call.
        unsafe { glClear(GL_COLOR_BUFFER_BIT) };
    });
    unbind_fbo(fb);
}

/// GL_EXT_color_buffer_half_float: a 16-bit-float target, 64 bits a texel.
#[test]
fn a_half_float_target_draws() {
    let _g = rig();
    if !extension("GL_EXT_color_buffer_half_float") {
        return;
    }
    let fb = bind_fbo();
    let t = texture(GL_TEXTURE_2D, GL_RGBA, GL_RGBA, GL_HALF_FLOAT_OES, W, H, GL_NEAREST);
    // SAFETY: plain GL call.
    unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0) };
    if complete() {
        accepted("a half-float target", || {
            // SAFETY: a plain GL call.
            unsafe { glClear(GL_COLOR_BUFFER_BIT) };
            program(FS_COLOR);
            draw_triangle();
        });
    }
    unbind_fbo(fb);
}

/// "the draw reads depth/stencil an earlier job wrote, and the target has no
/// buffer to keep it in": a depth or stencil test in the second job of a
/// target, on every ES 2.0 depth/stencil attachment, and on the default
/// surface.
#[test]
fn depth_and_stencil_across_jobs() {
    let _g = rig();
    let pass = |what: &str| {
        accepted(what, || {
            // SAFETY: plain GL calls.
            unsafe {
                glEnable(GL_DEPTH_TEST);
                glDepthFunc(GL_LESS);
                glEnable(GL_STENCIL_TEST);
                glStencilFunc(GL_EQUAL, 0, 0xff);
                glStencilOp(GL_KEEP, GL_INCR_WRAP, GL_INCR);
                glClearDepthf(1.0);
                glClearStencil(0);
                glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            }
            program(FS_COLOR);
            draw_triangle();
            // SAFETY: a plain GL call: ends the first job.
            unsafe { glFinish() };
            draw_triangle();
            // SAFETY: plain GL calls.
            unsafe {
                glFinish();
                glStencilFunc(GL_ALWAYS, 0, 0xff);
                glDepthFunc(GL_ALWAYS);
            }
            draw_triangle();
        });
        // SAFETY: plain GL calls.
        unsafe {
            glDisable(GL_DEPTH_TEST);
            glDisable(GL_STENCIL_TEST);
        }
    };
    pass("the default surface");
    /* depth only, stencil only, both packed, and a depth texture */
    type Attach = Box<dyn Fn()>;
    let cases: Vec<(&str, Attach)> = vec![
        ("DEPTH_COMPONENT16", Box::new(|| renderbuffer(GL_DEPTH_ATTACHMENT, GL_DEPTH_COMPONENT16, W, H))),
        ("STENCIL_INDEX8", Box::new(|| renderbuffer(GL_STENCIL_ATTACHMENT, GL_STENCIL_INDEX8, W, H))),
        (
            "DEPTH16 + STENCIL8",
            Box::new(|| {
                renderbuffer(GL_DEPTH_ATTACHMENT, GL_DEPTH_COMPONENT16, W, H);
                renderbuffer(GL_STENCIL_ATTACHMENT, GL_STENCIL_INDEX8, W, H);
            }),
        ),
        (
            "DEPTH24_STENCIL8_OES (one renderbuffer on both points: report-es2 has no GL_DEPTH_STENCIL_ATTACHMENT)",
            Box::new(|| {
                // SAFETY: plain GL calls with room for one name.
                unsafe {
                    let mut rb = 0;
                    glGenRenderbuffers(1, &mut rb);
                    glBindRenderbuffer(GL_RENDERBUFFER, rb);
                    glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8_OES, W, H);
                    glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
                    glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb);
                }
            }),
        ),
    ];
    for (name, attach) in cases {
        let fb = bind_fbo();
        renderbuffer(GL_COLOR_ATTACHMENT0, GL_RGB565, W, H);
        attach();
        if complete() {
            pass(name);
        }
        unbind_fbo(fb);
    }
    /* the colour target a texture, the depth a depth texture */
    if extension("GL_OES_depth_texture") {
        let fb = bind_fbo();
        renderbuffer(GL_COLOR_ATTACHMENT0, GL_RGBA4, W, H);
        let d = texture(GL_TEXTURE_2D, GL_DEPTH_COMPONENT, GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT, W, H, GL_NEAREST);
        // SAFETY: plain GL call.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, d, 0) };
        if complete() {
            pass("a depth texture");
        }
        unbind_fbo(fb);
    }
}

/// "an attribute format other than float32 reached the backend": every
/// vertex attribute type of ES 2.0, normalized or not, from client memory
/// and from a buffer.
#[test]
fn every_attribute_type_draws() {
    let _g = rig();
    program(FS_COLOR);
    let bytes: [i8; 16] = [-1, -1, 0, 1, 3, -1, 0, 1, -1, 3, 0, 1, 0, 0, 0, 0];
    let shorts: [i16; 12] = [-1, -1, 0, 1, 3, -1, 0, 1, -1, 3, 0, 1];
    let fixed: [i32; 12] =
        [-0x10000, -0x10000, 0, 0x10000, 0x30000, -0x10000, 0, 0x10000, -0x10000, 0x30000, 0, 0x10000];
    let sets: [(GLenum, &str, *const c_void, usize); 5] = [
        (GL_BYTE, "BYTE", bytes.as_ptr() as *const c_void, 12),
        (GL_UNSIGNED_BYTE, "UNSIGNED_BYTE", bytes.as_ptr() as *const c_void, 12),
        (GL_SHORT, "SHORT", shorts.as_ptr() as *const c_void, 24),
        (GL_UNSIGNED_SHORT, "UNSIGNED_SHORT", shorts.as_ptr() as *const c_void, 24),
        (GL_FIXED, "FIXED", fixed.as_ptr() as *const c_void, 48),
    ];
    for (ty, name, data, size) in sets {
        for norm in [0, 1] {
            for from_buffer in [false, true] {
                accepted(&format!("{name} normalized {norm} buffer {from_buffer}"), || {
                    // SAFETY: the arrays outlive the draws; plain GL calls.
                    unsafe {
                        let ptr = if from_buffer {
                            let mut b = 0;
                            glGenBuffers(1, &mut b);
                            glBindBuffer(GL_ARRAY_BUFFER, b);
                            glBufferData(GL_ARRAY_BUFFER, size as isize, data, GL_STATIC_DRAW);
                            core::ptr::null()
                        } else {
                            glBindBuffer(GL_ARRAY_BUFFER, 0);
                            data
                        };
                        glVertexAttribPointer(0, 4, ty, norm, 0, ptr);
                        glEnableVertexAttribArray(0);
                        glDrawArrays(GL_TRIANGLES, 0, 3);
                        glBindBuffer(GL_ARRAY_BUFFER, 0);
                    }
                });
            }
        }
    }
    /* a stride, and a float array of fewer components than the shader reads */
    accepted("stride and size", || {
        // SAFETY: TRI is static; plain GL calls.
        unsafe {
            glVertexAttribPointer(0, 2, GL_FLOAT, 0, 16, TRI.as_ptr() as *const c_void);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
    });
}

/// The index types of glDrawElements, and the primitives the core converts.
#[test]
fn index_types_draw() {
    let _g = rig();
    program(FS_COLOR);
    let b: [u8; 6] = [0, 1, 2, 0, 1, 2];
    let s: [u16; 6] = [0, 1, 2, 0, 1, 2];
    let i: [u32; 6] = [0, 1, 2, 0, 1, 2];
    for mode in [GL_TRIANGLES, GL_TRIANGLE_STRIP, GL_TRIANGLE_FAN, GL_LINES, GL_LINE_STRIP, GL_LINE_LOOP, GL_POINTS] {
        accepted(&format!("mode {mode:#x}"), || {
            // SAFETY: TRI and the index arrays are live; plain GL calls.
            unsafe {
                glBindBuffer(GL_ARRAY_BUFFER, 0);
                glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
                glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
                glEnableVertexAttribArray(0);
                glDrawElements(mode, 3, GL_UNSIGNED_BYTE, b.as_ptr() as *const c_void);
                glDrawElements(mode, 6, GL_UNSIGNED_SHORT, s.as_ptr() as *const c_void);
                glDrawArrays(mode, 0, 3);
                if extension("GL_OES_element_index_uint") {
                    glDrawElements(mode, 6, GL_UNSIGNED_INT, i.as_ptr() as *const c_void);
                }
            }
        });
    }
    /* an element buffer */
    accepted("an element buffer", || {
        // SAFETY: plain GL calls; the data is live.
        unsafe {
            let mut e = 0;
            glGenBuffers(1, &mut e);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, e);
            glBufferData(GL_ELEMENT_ARRAY_BUFFER, 12, s.as_ptr() as *const c_void, GL_STATIC_DRAW);
            glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, core::ptr::null());
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        }
    });
}

/// The ES 2.0 texture formats, every wrap and filter, power-of-two and not,
/// with a mipmap chain and without (incomplete: sampled black).
#[test]
fn every_texture_format_samples() {
    let _g = rig();
    let formats: [(&str, GLenum, GLenum, GLenum); 9] = [
        ("RGBA8", GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE),
        ("RGB8", GL_RGB, GL_RGB, GL_UNSIGNED_BYTE),
        ("LUMINANCE_ALPHA", GL_LUMINANCE_ALPHA, GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE),
        ("LUMINANCE", GL_LUMINANCE, GL_LUMINANCE, GL_UNSIGNED_BYTE),
        ("ALPHA", GL_ALPHA, GL_ALPHA, GL_UNSIGNED_BYTE),
        ("RGB565", GL_RGB, GL_RGB, GL_UNSIGNED_SHORT_5_6_5),
        ("RGBA4444", GL_RGBA, GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4),
        ("RGBA5551", GL_RGBA, GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1),
        ("RGBA half float", GL_RGBA, GL_RGBA, GL_HALF_FLOAT_OES),
    ];
    program(FS_TEX);
    for (name, internal, format, ty) in formats {
        if ty == GL_HALF_FLOAT_OES && !extension("GL_OES_texture_half_float") {
            continue;
        }
        for (w, h) in [(16, 16), (13, 7)] {
            for min in [GL_NEAREST, GL_LINEAR, GL_NEAREST_MIPMAP_LINEAR, GL_LINEAR_MIPMAP_LINEAR] {
                for wrap in [GL_REPEAT, GL_CLAMP_TO_EDGE, GL_MIRRORED_REPEAT] {
                    accepted(&format!("{name} {w}x{h} min {min:#x} wrap {wrap:#x}"), || {
                        texture(GL_TEXTURE_2D, internal, format, ty, w, h, min);
                        // SAFETY: plain GL calls.
                        unsafe {
                            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, wrap as GLint);
                            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, wrap as GLint);
                            if min >= GL_NEAREST_MIPMAP_NEAREST && (w, h) == (16, 16) {
                                glGenerateMipmap(GL_TEXTURE_2D);
                            }
                        }
                        draw_triangle();
                    });
                }
            }
        }
    }
}

/// A cube map of every size class, complete and incomplete.
#[test]
fn a_cube_map_samples() {
    let _g = rig();
    program(FS_CUBE);
    for min in [GL_NEAREST, GL_LINEAR, GL_LINEAR_MIPMAP_LINEAR] {
        accepted(&format!("cube, min {min:#x}"), || {
            texture(GL_TEXTURE_CUBE_MAP, GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE, 16, 16, min);
            // SAFETY: a plain GL call.
            unsafe { glGenerateMipmap(GL_TEXTURE_CUBE_MAP) };
            draw_triangle();
        });
    }
    /* an incomplete cube (min filter reads levels that are not there) */
    accepted("an incomplete cube", || {
        texture(GL_TEXTURE_CUBE_MAP, GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE, 8, 8, GL_NEAREST_MIPMAP_LINEAR);
        draw_triangle();
    });
}

/// GL_OES_depth_texture: the one 32-bit-class texture of ES 2.0 (R32F), read
/// by a look-up compiled for the 32-bit TMU output.
#[test]
fn depth_texture_samples() {
    let _g = rig();
    if !extension("GL_OES_depth_texture") {
        return;
    }
    program(FS_TEX);
    for ty in [GL_UNSIGNED_SHORT, GL_UNSIGNED_INT] {
        accepted(&format!("depth texture type {ty:#x}"), || {
            texture(GL_TEXTURE_2D, GL_DEPTH_COMPONENT, GL_DEPTH_COMPONENT, ty, 16, 16, GL_NEAREST);
            draw_triangle();
        });
    }
}

/// An incomplete depth texture (the default min filter needs a mipmap chain):
/// the stand-in of an incomplete 32-bit-class texture, `black_typed_state`.
#[test]
fn incomplete_depth_texture_reads_black() {
    let _g = rig();
    if !extension("GL_OES_depth_texture") {
        return;
    }
    program(FS_TEX);
    accepted("an incomplete depth texture", || {
        texture(
            GL_TEXTURE_2D,
            GL_DEPTH_COMPONENT,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_SHORT,
            16,
            16,
            GL_NEAREST_MIPMAP_LINEAR,
        );
        draw_triangle();
    });
    /* and a texture with no image at all, as every unit starts */
    accepted("a texture with no image", || {
        // SAFETY: plain GL calls.
        unsafe {
            let mut t = 0;
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
        }
        draw_triangle();
    });
}

/// GL_OES_compressed_ETC1_RGB8_texture: the one compressed format of ES 2.0.
#[test]
fn etc1_texture_samples() {
    let _g = rig();
    if !extension("GL_OES_compressed_ETC1_RGB8_texture") {
        return;
    }
    program(FS_TEX);
    accepted("ETC1", || {
        let blocks = [0x55u8; 8 * 4];
        // SAFETY: plain GL calls; 4 blocks of 8 bytes are 8x8 texels.
        unsafe {
            let mut t = 0;
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_ETC1_RGB8_OES, 8, 8, 0, 32, blocks.as_ptr() as *const c_void);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
        }
        draw_triangle();
    });
}

/// A large (but dEQP-GLES2-sized: texture.size.2d up to 4096 a side) texture and
/// the biggest the core's tiled copy takes to the arena.
#[test]
fn a_large_texture_samples() {
    let _g = rig();
    program(FS_TEX);
    for (w, h) in [(4096, 1), (1, 4096), (2048, 1024), (512, 512)] {
        accepted(&format!("{w}x{h}"), || {
            texture(GL_TEXTURE_2D, GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE, w, h, GL_LINEAR);
            draw_triangle();
        });
    }
    /* texture.size.2d.2048x2048_rgba8888_mipmap: 22,376,448 bytes tiled, which
     * the 24 MiB budget of image T5 holds (the 16 MiB of the other images
     * refuses it by name, -ENOTSUP: the images are split for that) */
    accepted("2048x2048 with its chain", || {
        texture(GL_TEXTURE_2D, GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE, 2048, 2048, GL_LINEAR_MIPMAP_LINEAR);
        // SAFETY: a plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        draw_triangle();
    });
}

/// Render to a texture, sample it, render it into the default surface; the
/// colour buffer of the first job is the texture of the second.
#[test]
fn render_to_texture_then_sample() {
    let _g = rig();
    let fb = bind_fbo();
    let t = texture(GL_TEXTURE_2D, GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE, W, H, GL_LINEAR);
    // SAFETY: plain GL call.
    unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0) };
    assert!(complete());
    accepted("render to texture", || {
        program(FS_COLOR);
        draw_triangle();
    });
    unbind_fbo(fb);
    accepted("sample the render target", || {
        // SAFETY: plain GL calls.
        unsafe {
            glViewport(0, 0, W, H);
            glBindTexture(GL_TEXTURE_2D, t);
        }
        program(FS_TEX);
        draw_triangle();
    });
}

/// Eight texture units in one fragment shader (GL_MAX_TEXTURE_IMAGE_UNITS is
/// at least 8 in ES 2.0).
#[test]
fn eight_units_sample() {
    let _g = rig();
    let fs = "precision mediump float;\nvarying vec2 t;\nuniform sampler2D s0, s1, s2, s3, s4, s5, s6, s7;\n\
              void main() { gl_FragColor = texture2D(s0, t) + texture2D(s1, t) + texture2D(s2, t) + texture2D(s3, t)\n\
              + texture2D(s4, t) + texture2D(s5, t) + texture2D(s6, t) + texture2D(s7, t); }\n";
    accepted("eight units", || {
        program(fs);
        // SAFETY: plain GL calls.
        unsafe {
            let mut p = 0;
            glGetIntegerv(GL_CURRENT_PROGRAM, &mut p);
            for u in 0..8 {
                let name = CString::new(format!("s{u}")).unwrap();
                glUniform1i(glGetUniformLocation(p as GLuint, name.as_ptr()), u);
                glActiveTexture(GL_TEXTURE0 + u as u32);
                let mut t = 0;
                glGenTextures(1, &mut t);
                glBindTexture(GL_TEXTURE_2D, t);
                let texels = [0x40u8; 16];
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA as GLint,
                    2,
                    2,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    texels.as_ptr() as *const c_void,
                );
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            }
        }
        draw_triangle();
        // SAFETY: a plain GL call.
        unsafe { glActiveTexture(GL_TEXTURE0) };
    });
}

/// Every blend factor and equation of ES 2.0, the scissor and the colour
/// mask, on the surface and on an FBO (clears take the quad path).
#[test]
fn blend_scissor_mask_draw() {
    let _g = rig();
    program(FS_COLOR);
    let factors = [
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
    for fb_case in [false, true] {
        let fb = fb_case.then(|| {
            let fb = bind_fbo();
            renderbuffer(GL_COLOR_ATTACHMENT0, GL_RGB565, W, H);
            assert!(complete());
            fb
        });
        accepted("blend factors, equations, scissor and mask", || {
            // SAFETY: plain GL calls.
            unsafe {
                glEnable(GL_BLEND);
                glBlendColor(0.2, 0.4, 0.6, 0.8);
                for eq in [GL_FUNC_ADD, GL_FUNC_SUBTRACT, GL_FUNC_REVERSE_SUBTRACT] {
                    glBlendEquationSeparate(eq, GL_FUNC_ADD);
                    for s in factors {
                        glBlendFuncSeparate(s, GL_ONE_MINUS_SRC_ALPHA, s, GL_ONE);
                        draw_triangle();
                    }
                }
                glDisable(GL_BLEND);
                glEnable(GL_SCISSOR_TEST);
                glScissor(3, 4, 9, 7);
                glColorMask(1, 0, 1, 0);
                glClear(GL_COLOR_BUFFER_BIT);
                draw_triangle();
                glColorMask(1, 1, 1, 1);
                glDisable(GL_SCISSOR_TEST);
                glEnable(GL_CULL_FACE);
                for m in [GL_FRONT, GL_BACK, GL_FRONT_AND_BACK] {
                    glCullFace(m);
                    draw_triangle();
                }
                glDisable(GL_CULL_FACE);
                glEnable(GL_DEPTH_TEST);
                for f in [GL_NEVER, GL_LESS, GL_EQUAL, GL_LEQUAL, GL_GREATER, GL_NOTEQUAL, GL_GEQUAL, GL_ALWAYS] {
                    glDepthFunc(f);
                    draw_triangle();
                }
                glDisable(GL_DEPTH_TEST);
            }
        });
        if let Some(fb) = fb {
            unbind_fbo(fb);
        }
    }
}
