// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glRenderbufferStorage(Multisample) of every ES 3.0 colour-, depth- and
//! stencil-renderable sized format (tables 3.13 and 3.14) through the GL API
//! over the real V3D back end (job builder faked, as tests/es3_formats_v3d.rs
//! `gl_rig`), then glCheckFramebufferStatus, one glClear and one draw, at
//! 1x1, 64x64, 127x33 and the largest size the host takes, with samples 0
//! and 4 (v3d-early-stop-repro, rb-formats-v3d).
//!
//! R32F is not colour-renderable in ES 3.0 (only GL_EXT_color_buffer_float
//! makes it so): its rows run while a 1x1 R32F renderbuffer is complete and
//! are skipped otherwise (extension unlisted, storage GL_INVALID_ENUM).
//!
//! A legal state that a back end refuses must not surface as
//! GL_OUT_OF_MEMORY: dEQP aborts the session on it (src/gl/draw.rs).
//! Asserted: no OOM for any legal state at these sizes; samples 4 is
//! GL_INVALID_OPERATION (GL_MAX_SAMPLES is 1, ES 3.0 4.4.2), a size past
//! GL_MAX_RENDERBUFFER_SIZE GL_INVALID_VALUE. Today's behaviour that is
//! wrong is pinned, the expected result is in an `#[ignore]`d test with
//! its cause. The per-format table is printed by `format_table`
//! (`--nocapture`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rb_formats_v3d
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "gpu-v3d",
    feature = "shaderc-online",
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#![allow(dead_code, unused_imports)]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use yakogl::backend::*;
use yakogl::v3d::ffi::*;

/* ---- the real layout and packers, built from driver/src/v3d/v3d_tex.c ------ */

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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_rb_formats_v3d");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_tex-{}.so", std::process::id()));
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

mod gl_rig {
    #![allow(clippy::too_many_arguments)]
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{MutexGuard, Once};
    use yakogl::gl::consts::*;
    use yakogl::gl::context::{self, GlContext};
    use yakogl::gl::draw::glDrawArrays;
    use yakogl::gl::framebuffer::*;
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::state::*;
    use yakogl::gl::texture::*;
    use yakogl::gl::types::*;
    use yakogl::gl::vertex::*;
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
    use yakogl::v3d::backend::*;
    use yakogl::v3d::kick::V3dKick;

    /// The trampolines: the library's `extern "C"` texture functions
    /// (src/v3d/ffi.rs), each handed to the real C. `v3d_tex_pack_state` also
    /// records the state it packed, by the record's address, for the checks.
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

    /* The signatures are src/v3d/ffi.rs's. Kept as written: a macro call wider
     * than the width is wrapped by some rustfmt releases and left by others, so
     * the gate's `cargo fmt --check` must not depend on the host's release. */
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
        // SAFETY: the library's call, passed on.
        unsafe { f(out, s) }
    }

    /* ---- the job builder, stood in for (driver/src/v3d/v3d_job.c) -------------- */

    #[derive(Clone, Debug)]
    struct SeenJob {
        target: V3dJobTarget,
        clear: Option<V3dJobClear>,
        /// The FS texture records of each draw, in order.
        tmu: Vec<Vec<V3dJobTmu>>,
    }

    static JOBS: Mutex<Vec<SeenJob>> = Mutex::new(Vec::new());

    fn jobs() -> MutexGuard<'static, Vec<SeenJob>> {
        JOBS.lock().unwrap_or_else(|e| e.into_inner())
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
        jobs().push(SeenJob { target: t, clear: None, tmu: Vec::new() });
        t.width != 0 && t.height != 0
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
        // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
        let (j, d) = unsafe { (&mut *j, &*d) };
        let tmu = if d.tmu.is_null() || d.num_tmu == 0 {
            Vec::new()
        } else {
            // SAFETY: `num_tmu` records, live for the call.
            unsafe { std::slice::from_raw_parts(d.tmu, d.num_tmu as usize) }.to_vec()
        };
        j.draws += 1;
        // SAFETY: the job's BCL is live.
        unsafe { (*j.bcl).used += 120 };
        if let Some(job) = jobs().last_mut() {
            job.tmu.push(tmu);
        }
        V3D_JOB_OK
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_end(
        j: *mut V3dJob,
        c: *const V3dJobClear,
        _tile_list_start: *mut u32,
        _tile_list_end: *mut u32,
    ) -> bool {
        // SAFETY: the backend passes its live job and clear.
        let (j, c) = unsafe { (&mut *j, *c) };
        // SAFETY: the job's lists are live.
        unsafe {
            (*j.bcl).used += 1;
            (*j.rcl).used = 200;
        }
        if let Some(job) = jobs().last_mut() {
            job.clear = Some(c);
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

    /// The tile-size table of v3d_frame.c (Mesa's v3d_choose_tile_size), as
    /// the backend's unit tests fake it.
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
    unsafe extern "C" fn v3d_frame_alloc_sizes(
        t: *const V3dTiling,
        _draws: u32,
        _page: u32,
        ta: *mut u32,
        ts: *mut u32,
    ) {
        // SAFETY: the backend's parameters.
        unsafe {
            let n = (*t).tiles_x * (*t).tiles_y;
            *ta = n * 128 + 8192;
            *ts = n * 256;
        }
    }

    /* ---- the platform ------------------------------------------------------------ */

    /// GPU addresses are 32-bit (the V3D's MMU), from here up.
    const GPU_BASE: u64 = 0x0100_0000;

    struct Alloc {
        /// u64 backing: every u32 view of it is aligned.
        mem: Box<[u64]>,
        size: usize,
    }

    #[derive(Default)]
    struct Platform {
        next: u64,
        allocs: BTreeMap<u64, Alloc>,
        kicks: Vec<V3dKick>,
        log: Vec<String>,
    }

    static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

    fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
        let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
        f(g.get_or_insert_with(Platform::default))
    }

    /// The CPU view of `n` bytes at GPU address `gpu`.
    fn platform_cpu(p: &mut Platform, gpu: u64, n: usize) -> *mut u8 {
        let (base, a) = p.allocs.range_mut(..=gpu).next_back().expect("an allocation below the address");
        let off = (gpu - base) as usize;
        assert!(off + n <= a.size, "0x{gpu:x}+{n} is past its allocation");
        // SAFETY: inside the allocation.
        unsafe { (a.mem.as_mut_ptr() as *mut u8).add(off) }
    }

    unsafe extern "C" fn p_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
        with_platform(|p| {
            if size == 0 || size > 256 << 20 {
                return -12;
            }
            let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
            let gpu = GPU_BASE + p.next;
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
            with_platform(|p| {
                /* the 32-bit address space is a bump pointer: the top block
                 * given back returns its range, so a run of 256 MiB
                 * surfaces does not walk past 4 GiB (where the V3D back end
                 * refuses a target by name) */
                if p.allocs.remove(&m.gpu).is_some()
                    && m.gpu - GPU_BASE + (m.size as u64).div_ceil(4096) * 4096 == p.next
                {
                    p.next = m.gpu - GPU_BASE;
                }
            });
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

    /// The "GPU": every job completes inside its submit (as src/osal/linux.c),
    /// and a job that stores a typed colour target with a clear leaves the
    /// clear's texel (the tile buffer words `V3dJobClear::clear_rt[0]`, in the
    /// output format the target names) in every pixel of the raster store.
    unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
        if len != size_of::<V3dKick>() {
            return -22;
        }
        // SAFETY: the backend's live descriptor of that size.
        let k = unsafe { *(kick as *const V3dKick) };
        let fence = with_platform(|p| {
            p.kicks.push(k);
            p.kicks.len()
        });
        let job = jobs().get(fence - 1).cloned().expect("the job this kick runs was begun and ended");
        let (t, c) = (job.target, job.clear.expect("the job was ended"));
        if t.typed && !c.load_rt0 && t.address != 0 {
            let w = c.clear_rt[0];
            let texel: Vec<u8> = match t.rt_format[0] {
                V3D_OUTPUT_IMAGE_FORMAT_RGBA16F => w[..2].iter().flat_map(|x| x.to_le_bytes()).collect(),
                V3D_OUTPUT_IMAGE_FORMAT_R16F => w[0].to_le_bytes()[..2].to_vec(),
                V3D_OUTPUT_IMAGE_FORMAT_R32F => w[0].to_le_bytes().to_vec(),
                _ => Vec::new(),
            };
            if !texel.is_empty() {
                with_platform(|p| {
                    let row = (t.stride as usize).max(t.width as usize * texel.len());
                    let base = platform_cpu(p, u64::from(t.address), row * t.height as usize);
                    for y in 0..t.height as usize {
                        for x in 0..t.width as usize {
                            // SAFETY: inside the span platform_cpu checked.
                            unsafe {
                                core::ptr::copy_nonoverlapping(
                                    texel.as_ptr(),
                                    base.add(y * row + x * texel.len()),
                                    texel.len(),
                                )
                            };
                        }
                    }
                });
            }
        }
        fence as i64
    }

    unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
        0
    }

    unsafe extern "C" fn p_completed() -> i64 {
        with_platform(|p| p.kicks.len() as i64)
    }

    unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
        use yakogl::osal::{
            OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_JOB_OPEN, OSAL_QUERY_V3D_VPM_BYTES,
        };
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
        if what == OSAL_QUERY_V3D_JOB_OPEN && out_sz == 4 {
            return 0;
        }
        if what == OSAL_QUERY_V3D_BIN_POOL {
            return -61; /* ENODATA, as zephyr_v3d.c answers before the first job */
        }
        -22 /* no TFU, the default budgets, no clock */
    }

    /// The library logs every line as `log("%s", line)` (`osal::log_str`), so a
    /// two-pointer function takes it; on x86-64 and AArch64 Linux a variadic call
    /// passes those two arguments as a plain one does.
    unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
        // SAFETY: the library passes a NUL-terminated line.
        let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
        with_platform(|p| p.log.push(s));
    }

    type LogFn = unsafe extern "C" fn(*const c_char, ...);

    unsafe extern "C" fn p_init(_platform_arg: *mut c_void) -> i32 {
        0
    }

    /// The platform's table (made once, at run time: the log hook is a cast).
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

    /* ---- the context ------------------------------------------------------------- */

    /// pair of the same shape keeps the test quick.
    const CANVAS: (i32, i32) = (16, 16);

    static LOCK: Mutex<()> = Mutex::new(());
    static ONCE: Once = Once::new();
    static mut CTX: *mut GlContext = core::ptr::null_mut();

    /// The one context and backend of the file, set up once; the tests take
    /// turns.
    fn rig() -> MutexGuard<'static, ()> {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        ONCE.call_once(|| {
            // SAFETY: statics; the lock serialises every GL call of the file.
            unsafe {
                osal_set_ops(osal());
                yakogl::backend::g_backend = &yakogl_backend_v3d;
                assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
                CTX = context::gl_context_create(CANVAS.0 as u32, CANVAS.1 as u32);
                assert!(!CTX.is_null(), "{:?}", logs());
            }
        });
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            while glGetError() != GL_NO_ERROR {}
        }
        g
    }

    fn logs() -> Vec<String> {
        with_platform(|p| p.log.clone())
    }

    fn shader(type_: GLenum, src: &str) -> GLuint {
        let c = CString::new(src).unwrap();
        // SAFETY: plain GL calls; the string lives for the call.
        unsafe {
            let s = glCreateShader(type_);
            let srcs = [c.as_ptr()];
            glShaderSource(s, 1, srcs.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "compile 0x{type_:x}: {src}\n{:?}", logs());
            s
        }
    }

    fn program(vs: &str, fs: &str) -> GLuint {
        let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
        // SAFETY: plain GL calls on valid handles.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glBindAttribLocation(p, 0, c"a_position".as_ptr());
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link failed: {:?}", logs());
            p
        }
    }
    /* ---- the probe ---------------------------------------------------------- */

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Kind {
        Float,
        Int,
        Uint,
        Depth,
        Stencil,
        DepthStencil,
    }

    /// (name, internal format, kind): ES 3.0 table 3.13 (colour) and 3.14.
    pub const FORMATS: &[(&str, GLenum, Kind)] = &[
        ("R8", GL_R8, Kind::Float),
        ("RG8", GL_RG8, Kind::Float),
        ("RGB8", GL_RGB8, Kind::Float),
        ("RGB565", GL_RGB565, Kind::Float),
        ("RGBA4", GL_RGBA4, Kind::Float),
        ("RGB5_A1", GL_RGB5_A1, Kind::Float),
        ("RGBA8", GL_RGBA8, Kind::Float),
        ("RGB10_A2", GL_RGB10_A2, Kind::Float),
        ("SRGB8_ALPHA8", GL_SRGB8_ALPHA8, Kind::Float),
        ("RGB10_A2UI", GL_RGB10_A2UI, Kind::Uint),
        ("R8I", GL_R8I, Kind::Int),
        ("R8UI", GL_R8UI, Kind::Uint),
        ("R16I", GL_R16I, Kind::Int),
        ("R16UI", GL_R16UI, Kind::Uint),
        ("R32I", GL_R32I, Kind::Int),
        ("R32UI", GL_R32UI, Kind::Uint),
        ("RG8I", GL_RG8I, Kind::Int),
        ("RG8UI", GL_RG8UI, Kind::Uint),
        ("RG16I", GL_RG16I, Kind::Int),
        ("RG16UI", GL_RG16UI, Kind::Uint),
        ("RG32I", GL_RG32I, Kind::Int),
        ("RG32UI", GL_RG32UI, Kind::Uint),
        ("RGBA8I", GL_RGBA8I, Kind::Int),
        ("RGBA8UI", GL_RGBA8UI, Kind::Uint),
        ("RGBA16I", GL_RGBA16I, Kind::Int),
        ("RGBA16UI", GL_RGBA16UI, Kind::Uint),
        ("RGBA32I", GL_RGBA32I, Kind::Int),
        ("RGBA32UI", GL_RGBA32UI, Kind::Uint),
        ("DEPTH_COMPONENT16", GL_DEPTH_COMPONENT16, Kind::Depth),
        ("DEPTH_COMPONENT24", GL_DEPTH_COMPONENT24, Kind::Depth),
        ("DEPTH_COMPONENT32F", GL_DEPTH_COMPONENT32F, Kind::Depth),
        ("DEPTH24_STENCIL8", GL_DEPTH24_STENCIL8, Kind::DepthStencil),
        ("DEPTH32F_STENCIL8", GL_DEPTH32F_STENCIL8, Kind::DepthStencil),
        ("STENCIL_INDEX8", GL_STENCIL_INDEX8, Kind::Stencil),
    ];

    /// The float colour formats the GL core's extension list adds (not ES 3.0
    /// core renderable: reported, not asserted).
    pub const EXT_FORMATS: &[(&str, GLenum, Kind)] =
        &[("RGBA16F", GL_RGBA16F, Kind::Float), ("R16F", GL_R16F, Kind::Float), ("R32F", GL_R32F, Kind::Float)];

    fn bytes_per_pixel(f: GLenum, k: Kind) -> u64 {
        match k {
            Kind::Depth | Kind::Stencil | Kind::DepthStencil => 8,
            _ => match f {
                GL_RGBA32I | GL_RGBA32UI => 16,
                GL_RG32I | GL_RG32UI | GL_RGBA16I | GL_RGBA16UI | GL_RGBA16F => 8,
                _ => 4,
            },
        }
    }

    /// The four sizes of the item; the last is GL_MAX_RENDERBUFFER_SIZE, its
    /// side halved until the surface is at most 256 MiB (the rig's allocation limit; the host's allocations
    /// are zero-filled).
    pub fn sizes(f: GLenum, k: Kind) -> [(i32, i32); 4] {
        let mut max = 0;
        // SAFETY: a plain query with a live out pointer.
        unsafe { glGetIntegerv(GL_MAX_RENDERBUFFER_SIZE, &mut max) };
        let mut side = max as u64;
        while side * side * bytes_per_pixel(f, k) > 256 << 20 {
            side /= 2;
        }
        [(1, 1), (64, 64), (127, 33), (side as i32, side as i32)]
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct Probe {
        /// The first error of glRenderbufferStorage(Multisample).
        pub storage: GLenum,
        pub status: GLenum,
        /// The first error of the clear and of the draw.
        pub clear: GLenum,
        pub draw: GLenum,
    }

    impl Probe {
        pub fn oom(&self) -> bool {
            [self.storage, self.clear, self.draw].contains(&GL_OUT_OF_MEMORY)
        }

        /// C complete, U unsupported (storage taken, framebuffer not
        /// complete), E error (a GL error, not OOM), O OOM.
        pub fn letter(&self) -> char {
            if self.oom() {
                'O'
            } else if self.storage != GL_NO_ERROR || self.clear != GL_NO_ERROR || self.draw != GL_NO_ERROR {
                'E'
            } else if self.status != GL_FRAMEBUFFER_COMPLETE {
                'U'
            } else {
                'C'
            }
        }
    }

    fn first_error() -> GLenum {
        // SAFETY: plain queries.
        let e = unsafe { glGetError() };
        // SAFETY: as above.
        while unsafe { glGetError() } != GL_NO_ERROR {}
        e
    }

    fn fs_for(k: Kind) -> &'static str {
        match k {
            Kind::Int => {
                "#version 300 es\nprecision highp int;\nlayout(location = 0) out ivec4 o;\nvoid main() { o = ivec4(1, 2, 3, 4); }\n"
            }
            Kind::Uint => {
                "#version 300 es\nprecision highp int;\nlayout(location = 0) out uvec4 o;\nvoid main() { o = uvec4(1u, 2u, 3u, 4u); }\n"
            }
            _ => {
                "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(0.25, 0.5, 0.75, 1.0); }\n"
            }
        }
    }

    fn program_for(k: Kind) -> GLuint {
        static P: Mutex<[GLuint; 3]> = Mutex::new([0; 3]);
        let i = match k {
            Kind::Int => 1,
            Kind::Uint => 2,
            _ => 0,
        };
        let mut g = P.lock().unwrap_or_else(|e| e.into_inner());
        if g[i] == 0 {
            g[i] = program(
                "#version 300 es\nlayout(location = 0) in vec4 a_position;\nvoid main() { gl_Position = a_position; }\n",
                fs_for(k),
            );
        }
        g[i]
    }

    static QUAD: [f32; 18] = [
        -1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, //
        -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0,
    ];

    /// Storage, attach, status, one clear, one draw, tear down.
    pub fn probe(f: GLenum, k: Kind, w: i32, h: i32, samples: i32) -> Probe {
        let (mut rb, mut fbo) = (0u32, 0u32);
        let point = match k {
            Kind::Depth => GL_DEPTH_ATTACHMENT,
            Kind::Stencil => GL_STENCIL_ATTACHMENT,
            Kind::DepthStencil => GL_DEPTH_STENCIL_ATTACHMENT,
            _ => GL_COLOR_ATTACHMENT0,
        };
        let bits = match k {
            Kind::Depth => GL_DEPTH_BUFFER_BIT,
            Kind::Stencil => GL_STENCIL_BUFFER_BIT,
            Kind::DepthStencil => GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT,
            _ => GL_COLOR_BUFFER_BIT,
        };
        let prog = program_for(k);
        // SAFETY: plain GL calls on names made here; the quad lives for the call.
        unsafe {
            first_error();
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            if samples == 0 {
                glRenderbufferStorage(GL_RENDERBUFFER, f, w, h);
            } else {
                glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, f, w, h);
            }
            let storage = first_error();
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb);
            assert_eq!(first_error(), GL_NO_ERROR, "attach 0x{f:x}");
            let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
            assert_eq!(first_error(), GL_NO_ERROR);
            glViewport(0, 0, w, h);
            glClearColor(0.5, 0.25, 0.75, 1.0);
            glClearDepthf(1.0);
            glClearStencil(0);
            glClear(bits);
            let clear = first_error();
            glUseProgram(prog);
            if k == Kind::Depth || k == Kind::DepthStencil {
                glEnable(GL_DEPTH_TEST);
            }
            if k == Kind::Stencil || k == Kind::DepthStencil {
                glEnable(GL_STENCIL_TEST);
            }
            glVertexAttribPointer(0, 3, GL_FLOAT, 0, 12, QUAD.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 6);
            glFinish();
            let draw = first_error();
            glDisable(GL_DEPTH_TEST);
            glDisable(GL_STENCIL_TEST);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fbo);
            glBindRenderbuffer(GL_RENDERBUFFER, 0);
            glDeleteRenderbuffers(1, &rb);
            glFinish();
            first_error();
            Probe { storage, status, clear, draw }
        }
    }

    /// Whether `name` is in glGetString(GL_EXTENSIONS) (ES 2.0 style list;
    /// the GL core lists the extensions in it).
    fn extension_listed(name: &str) -> bool {
        // SAFETY: glGetString returns a NUL-terminated static string.
        let s = unsafe { CStr::from_ptr(glGetString(GL_EXTENSIONS).cast::<c_char>()) }.to_string_lossy().into_owned();
        s.split_whitespace().any(|e| e == name)
    }

    /// Whether a 1x1 R32F renderbuffer is a complete colour attachment. ES 3.0
    /// does not make R32F colour-renderable: only GL_EXT_color_buffer_float
    /// does. When it is not, the extension must not be listed and the
    /// storage call must be GL_INVALID_ENUM.
    fn r32f_renderable() -> bool {
        let (mut rb, mut fbo) = (0u32, 0u32);
        // SAFETY: plain GL calls on names made here.
        let (storage, status) = unsafe {
            first_error();
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_R32F, 1, 1);
            let storage = first_error();
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fbo);
            glBindRenderbuffer(GL_RENDERBUFFER, 0);
            glDeleteRenderbuffers(1, &rb);
            first_error();
            (storage, status)
        };
        let ok = storage == GL_NO_ERROR && status == GL_FRAMEBUFFER_COMPLETE;
        if !ok {
            assert!(!extension_listed("GL_EXT_color_buffer_float"), "R32F not renderable but GL_EXT_color_buffer_float listed");
            assert!(
                storage == GL_INVALID_ENUM || storage == GL_NO_ERROR,
                "R32F storage without the extension: 0x{storage:x}, want GL_INVALID_ENUM"
            );
        }
        ok
    }

    /// The extension formats this build draws: R32F only while it is a
    /// complete colour target; otherwise its row is skipped.
    fn ext_formats() -> Vec<&'static (&'static str, GLenum, Kind)> {
        let r32f = r32f_renderable();
        EXT_FORMATS
            .iter()
            .filter(|&&(name, f, _)| {
                let skip = f == GL_R32F && !r32f;
                if skip {
                    eprintln!("skip {name}: not colour-renderable without GL_EXT_color_buffer_float");
                }
                !skip
            })
            .collect()
    }

    fn all_formats() -> impl Iterator<Item = &'static (&'static str, GLenum, Kind)> {
        FORMATS.iter().chain(ext_formats())
    }

    /// Storage, a complete framebuffer, a clear and a draw: no error, for every
    /// format at the three small sizes, samples 0.
    #[test]
    fn every_format_is_complete_and_draws_at_small_sizes() {
        let _g = rig();
        for &(name, f, k) in all_formats() {
            for (w, h) in sizes(f, k).into_iter().take(3) {
                let p = probe(f, k, w, h, 0);
                assert_eq!(p.letter(), 'C', "{name} {w}x{h}: {p:?} {:?}", logs());
                assert_eq!(p.status, GL_FRAMEBUFFER_COMPLETE, "{name} {w}x{h}");
            }
        }
    }

    /// Four samples: GL_MAX_SAMPLES is 1, so glRenderbufferStorageMultisample
    /// is GL_INVALID_OPERATION (ES 3.0 4.4.2), the renderbuffer stays without
    /// storage, and nothing in the clear or the draw after it is an
    /// out-of-memory.
    #[test]
    fn four_samples_is_invalid_operation_never_oom() {
        let _g = rig();
        for &(name, f, k) in all_formats() {
            for (w, h) in sizes(f, k) {
                let p = probe(f, k, w, h, 4);
                assert_eq!(p.storage, GL_INVALID_OPERATION, "{name} {w}x{h}");
                assert_eq!(p.status, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{name} {w}x{h}: no storage");
                assert!(!p.oom(), "{name} {w}x{h}: {p:?}");
                /* a draw into the incomplete framebuffer */
                assert_eq!(p.draw, GL_INVALID_FRAMEBUFFER_OPERATION, "{name} {w}x{h}");
            }
        }
    }

    /// Today a clear of an incomplete framebuffer raises nothing at the call
    /// (the GL core records it as pending and the draw after it reports the
    /// incompleteness). Pinned; the next test is the rule.
    #[test]
    fn pinned_clear_of_an_incomplete_framebuffer_raises_nothing() {
        let _g = rig();
        let p = probe(GL_RGBA8, Kind::Float, 64, 64, 4);
        assert_eq!(p.status, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
        assert_eq!(p.clear, GL_NO_ERROR);
    }

    /// ES 3.0 4.2.3: glClear on a framebuffer that is not complete is
    /// GL_INVALID_FRAMEBUFFER_OPERATION. Cause: the GL core (src/gl/state.rs
    /// glClear) checks nothing; not the V3D's.
    #[test]
    #[ignore = "gl-clear-incomplete-fb: glClear records the clear without checking completeness"]
    fn clear_of_an_incomplete_framebuffer_is_invalid_framebuffer_operation() {
        let _g = rig();
        let p = probe(GL_RGBA8, Kind::Float, 64, 64, 4);
        assert_eq!(p.clear, GL_INVALID_FRAMEBUFFER_OPERATION);
    }

    /// The largest sizes. The V3D's tile memory is sized for 2048 on a side
    /// (`v3d::backend::MAX_TARGET`) while GL_MAX_RENDERBUFFER_SIZE is 4096
    /// (src/gl/state.rs): the storage is taken, the framebuffer is complete,
    /// the clear is taken, and the draw is refused with -ENOTSUP, which the
    /// GL core answers as GL_INVALID_OPERATION (v3d-refuse-enotsup; before
    /// it, -EINVAL and GL_OUT_OF_MEMORY). Pinned for every format.
    #[test]
    fn pinned_draw_past_2048_a_side_is_invalid_operation() {
        let _g = rig();
        let mut seen = 0;
        for &(name, f, k) in all_formats() {
            let (w, h) = sizes(f, k)[3];
            assert!(w >= 2048, "{name}: {w}");
            let p = probe(f, k, w, h, 0);
            assert_eq!((p.storage, p.status, p.clear), (GL_NO_ERROR, GL_FRAMEBUFFER_COMPLETE, GL_NO_ERROR), "{name}");
            if w > 2048 {
                assert_eq!(p.draw, GL_INVALID_OPERATION, "{name} {w}x{h}: the refusal of a legal state");
                seen += 1;
            } else {
                assert_eq!(p.draw, GL_NO_ERROR, "{name} {w}x{h}");
            }
        }
        assert!(seen > 20, "{seen} formats reach 4096");
    }

    /// The edge: 2048 a side draws, 2049 or 4096 on one side does not.
    #[test]
    fn pinned_the_2048_edge_in_each_dimension() {
        let _g = rig();
        let ok = |w, h| probe(GL_RGBA8, Kind::Float, w, h, 0).letter();
        assert_eq!((ok(2048, 16), ok(16, 2048), ok(2048, 2048)), ('C', 'C', 'C'));
        assert_eq!((ok(2049, 16), ok(16, 2049), ok(4096, 1)), ('E', 'E', 'E'));
    }

    /// What a legal state the V3D cannot do must be: not GL_OUT_OF_MEMORY
    /// (dEQP aborts the session on it), GL_FRAMEBUFFER_UNSUPPORTED or
    /// GL_INVALID_OPERATION. The V3D refuses past 2048 with -ENOTSUP
    /// (v3d-refuse-enotsup), which src/gl/draw.rs turns into
    /// GL_INVALID_OPERATION; the GL core's 4096 limit against the V3D's 2048
    /// is its own item (report 2048 for GL_MAX_RENDERBUFFER_SIZE on the V3D).
    #[test]
    fn draw_past_2048_a_side_is_not_out_of_memory() {
        let _g = rig();
        for &(name, f, k) in all_formats() {
            let (w, h) = sizes(f, k)[3];
            let p = probe(f, k, w, h, 0);
            assert!(!p.oom(), "{name} {w}x{h}: {p:?}");
            assert!(
                p.status == GL_FRAMEBUFFER_UNSUPPORTED || matches!(p.draw, GL_NO_ERROR | GL_INVALID_OPERATION),
                "{name} {w}x{h}: {p:?}"
            );
        }
    }

    /// The argument errors, before and independent of the back end.
    #[test]
    fn illegal_arguments_are_invalid_value_enum_or_operation() {
        let _g = rig();
        let (mut rb, mut max) = (0u32, 0);
        // SAFETY: plain GL calls on a name made here.
        unsafe {
            first_error();
            glGetIntegerv(GL_MAX_RENDERBUFFER_SIZE, &mut max);
            /* nothing bound: GL_INVALID_OPERATION */
            glBindRenderbuffer(GL_RENDERBUFFER, 0);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 4, 4);
            assert_eq!(first_error(), GL_INVALID_OPERATION, "no renderbuffer bound");
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            for (w, h) in [(-1, 4), (4, -1), (max + 1, 4), (4, max + 1)] {
                glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, w, h);
                assert_eq!(first_error(), GL_INVALID_VALUE, "{w}x{h}");
                glRenderbufferStorageMultisample(GL_RENDERBUFFER, 0, GL_RGBA8, w, h);
                assert_eq!(first_error(), GL_INVALID_VALUE, "{w}x{h} multisample");
            }
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, -1, GL_RGBA8, 4, 4);
            assert_eq!(first_error(), GL_INVALID_VALUE, "negative samples");
            /* formats ES 3.0 does not make renderbuffer-renderable */
            for f in [GL_RGBA32F, GL_RGB32F, GL_R11F_G11F_B10F, GL_RGB9_E5, GL_RGB, GL_RGBA, GL_DEPTH_COMPONENT] {
                glRenderbufferStorage(GL_RENDERBUFFER, f, 4, 4);
                assert_eq!(first_error(), GL_INVALID_ENUM, "format 0x{f:x}");
            }
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, 4, 4);
            assert_eq!(first_error(), GL_INVALID_OPERATION, "4 samples");
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8I, 4, 4);
            assert_eq!(first_error(), GL_INVALID_OPERATION, "4 samples, integer");
            glBindRenderbuffer(GL_RENDERBUFFER, 0);
            glDeleteRenderbuffers(1, &rb);
        }
    }

    /// The per-format table: C complete, U unsupported, E a GL error (other
    /// than out of memory), O out of memory, for the sizes 1x1, 64x64, 127x33
    /// and the largest, with samples 0 and 4. `--nocapture` prints it.
    #[test]
    fn format_table() {
        let _g = rig();
        let mut out = String::new();
        for &(name, f, k) in all_formats() {
            let sz = sizes(f, k);
            let mut row = format!("{name:20}");
            for samples in [0, 4] {
                row += &format!(" s{samples}:");
                for (w, h) in sz {
                    let p = probe(f, k, w, h, samples);
                    row.push(p.letter());
                    if p.letter() != 'C' {
                        row += &format!("[{:x},{:x},{:x},{:x}]", p.storage, p.status, p.clear, p.draw);
                    }
                }
            }
            row += &format!("  max {}x{}\n", sz[3].0, sz[3].1);
            out += &row;
        }
        println!("{out}");
    }
}
