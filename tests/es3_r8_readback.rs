// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glReadPixels of R8 and RG8 colour images after a clear (t-r8-readback;
//! docs/notes/v3d-fbo-blit.md section 3.2, G4, draft 4): ES 3.0 4.3.2 reads
//! (r, 0, 0, 1) from an R8 image and (r, g, 0, 1) from an RG8 one. The GL
//! core stores both as RGBA8 bytes (`GLES_SURF_RGBA8888`).
//!
//! Measured on the V3D host back end (job builder faked as in
//! es3_r8_readback.rs, whose "GPU" here also fills an untyped RGBA8 target
//! with the clear's `rgba8` word). See the results in the tests below: the
//! plain tests pin today's bytes, the `#[ignore]`d ones state the ES 3.0
//! bytes.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_r8_readback
//! ```
#![cfg(all(
    feature = "host-test",
    feature = "gpu-v3d",
    feature = "shaderc-online",
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_r8_readback");
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
    use yakogl::gl::es3::*;
    use yakogl::gl::framebuffer::*;
    use yakogl::gl::state::*;
    use yakogl::gl::texture::*;
    use yakogl::gl::types::*;
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
    trampoline!(v3d_tex_pack_sampler_compare(out: *mut u8, s: *const V3dSampler, compare_op: u32) -> bool);
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
        if !t.typed && !c.load_rt0 && !t.rgb565 && t.address != 0 {
            /* an untyped RGBA8 target: the clear word in every pixel */
            let texel = c.rgba8.to_le_bytes();
            with_platform(|p| {
                let row = (t.stride as usize).max(t.width as usize * 4);
                let base = platform_cpu(p, u64::from(t.address), row * t.height as usize);
                for y in 0..t.height as usize {
                    for x in 0..t.width as usize {
                        // SAFETY: inside the span platform_cpu checked.
                        unsafe { core::ptr::copy_nonoverlapping(texel.as_ptr(), base.add(y * row + x * 4), 4) };
                    }
                }
            });
        }
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

    /// A SIDE x SIDE image of `internal` (a renderbuffer, or a texture) as the
    /// only attachment of a new framebuffer, bound; the name pair is returned.
    const SIDE: i32 = 8;

    pub fn image(internal: GLenum, texture: bool) -> (GLuint, GLuint) {
        let (mut img, mut fbo) = (0u32, 0u32);
        // SAFETY: plain GL calls with live out pointers.
        unsafe {
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            if texture {
                let (f, t) = if internal == GL_R8 { (GL_RED, GL_UNSIGNED_BYTE) } else { (GL_RG, GL_UNSIGNED_BYTE) };
                glGenTextures(1, &mut img);
                glBindTexture(GL_TEXTURE_2D, img);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
                glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, SIDE, SIDE, 0, f, t, core::ptr::null());
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, img, 0);
            } else {
                glGenRenderbuffers(1, &mut img);
                glBindRenderbuffer(GL_RENDERBUFFER, img);
                glRenderbufferStorage(GL_RENDERBUFFER, internal, SIDE, SIDE);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, img);
            }
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
            glViewport(0, 0, SIDE, SIDE);
        }
        (img, fbo)
    }

    /// glClearBufferfv (1, 1, 0, 1) of the bound framebuffer, as dEQP's
    /// blit cases clear their image.
    pub fn clear_yellow() {
        let c = [1.0f32, 1.0, 0.0, 1.0];
        // SAFETY: plain GL calls; `c` has 4 floats.
        unsafe { glClearBufferfv(GL_COLOR, 0, c.as_ptr()) };
    }

    /// The bound framebuffer read as RGBA/UNSIGNED_BYTE: SIDE x SIDE pixels.
    pub fn read() -> Vec<[u8; 4]> {
        let mut b = vec![0x77u8; (SIDE * SIDE * 4) as usize];
        // SAFETY: plain GL calls; `b` holds SIDE x SIDE RGBA bytes.
        unsafe {
            glFinish();
            glReadPixels(0, 0, SIDE, SIDE, GL_RGBA, GL_UNSIGNED_BYTE, b.as_mut_ptr() as *mut c_void);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        b.chunks(4).map(|p| [p[0], p[1], p[2], p[3]]).collect()
    }

    pub fn release(img: GLuint, fbo: GLuint, texture: bool) {
        // SAFETY: names made by `image`.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fbo);
            if texture {
                glDeleteTextures(1, &img);
            } else {
                glDeleteRenderbuffers(1, &img);
            }
        }
    }

    pub fn probe(internal: GLenum, texture: bool) -> Vec<[u8; 4]> {
        let _g = rig();
        let (img, fbo) = image(internal, texture);
        clear_yellow();
        let px = read();
        release(img, fbo, texture);
        px
    }

    /// The default framebuffer cleared to (r=0.5, g=0.25, b=0.75, 1), blitted
    /// into the image of `internal` (cleared yellow first) from
    /// 16x16 to the left-top 4x4 square, and read back.
    pub fn blit_probe(internal: GLenum, texture: bool) -> Vec<[u8; 4]> {
        let _g = rig();
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, 16, 16);
            glClearColor(0.5, 0.25, 0.75, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        let (img, fbo) = image(internal, texture);
        clear_yellow();
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, 0);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, fbo);
            glBlitFramebuffer(0, 0, 16, 16, 0, 0, 4, 4, GL_COLOR_BUFFER_BIT, GL_LINEAR);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        }
        let px = read();
        release(img, fbo, texture);
        px
    }

    const NOTE: &str = "docs/notes/v3d-fbo-blit.md 3.2 (G4): GL core read_pixels returns the stored RGBA8 bytes";

    /// A clear of (1, 1, 0, 1) of an R8 image: today green is stored and read.
    fn clear_r8(texture: bool) {
        assert!(probe(GL_R8, texture).iter().all(|p| *p == [255, 255, 0, 255]));
    }
    /// ES 3.0: an R8 image reads (r, 0, 0, 1).
    fn clear_r8_es(texture: bool) {
        let px = probe(GL_R8, texture);
        assert!(px.iter().all(|p| *p == [255, 0, 0, 255]), "{NOTE}: {:?}", px[0]);
    }
    /// An RG8 image reads (r, g, 0, 1): the clear's own blue and alpha are
    /// already the right ones, so today's bytes are ES's.
    fn clear_rg8(texture: bool) {
        assert!(probe(GL_RG8, texture).iter().all(|p| *p == [255, 255, 0, 255]));
    }

    #[test]
    fn r8_renderbuffer_clear_reads_the_stored_green_today() {
        clear_r8(false);
    }
    #[test]
    fn r8_texture_clear_reads_the_stored_green_today() {
        clear_r8(true);
    }
    #[test]
    #[ignore = "G4: GL core read_pixels returns RGBA8 as stored (plan #186); docs/notes/v3d-fbo-blit.md 3.2"]
    fn r8_renderbuffer_clear_reads_r_0_0_1() {
        clear_r8_es(false);
    }
    #[test]
    #[ignore = "G4: GL core read_pixels returns RGBA8 as stored (plan #186); docs/notes/v3d-fbo-blit.md 3.2"]
    fn r8_texture_clear_reads_r_0_0_1() {
        clear_r8_es(true);
    }
    #[test]
    fn rg8_renderbuffer_clear_reads_r_g_0_1() {
        clear_rg8(false);
    }
    #[test]
    fn rg8_texture_clear_reads_r_g_0_1() {
        clear_rg8(true);
    }

    /// The blitted 4x4 square holds the source's R (and G for RG8) with
    /// blue 0 and alpha 255 (the blit codec masks by the format's bits); the
    /// rest of the image is what the clear left.
    fn blit_check(internal: GLenum, texture: bool, rest: [u8; 4]) {
        let px = blit_probe(internal, texture);
        let g = if internal == GL_RG8 { 64 } else { 0 };
        for y in 0..SIDE as usize {
            for x in 0..SIDE as usize {
                let want = if x < 4 && y < 4 { [128, g, 0, 255] } else { rest };
                assert_eq!(px[y * SIDE as usize + x], want, "({x},{y}) {NOTE}");
            }
        }
    }

    #[test]
    fn r8_renderbuffer_blit_from_default_leaves_the_stored_green_today() {
        blit_check(GL_R8, false, [255, 255, 0, 255]);
    }
    #[test]
    fn r8_texture_blit_from_default_leaves_the_stored_green_today() {
        blit_check(GL_R8, true, [255, 255, 0, 255]);
    }
    #[test]
    #[ignore = "G4: the unblitted part of an R8 image reads green 255 (plan #186); docs/notes/v3d-fbo-blit.md 3.2"]
    fn r8_renderbuffer_blit_from_default_reads_r_0_0_1() {
        blit_check(GL_R8, false, [255, 0, 0, 255]);
    }
    #[test]
    #[ignore = "G4: the unblitted part of an R8 image reads green 255 (plan #186); docs/notes/v3d-fbo-blit.md 3.2"]
    fn r8_texture_blit_from_default_reads_r_0_0_1() {
        blit_check(GL_R8, true, [255, 0, 0, 255]);
    }
    #[test]
    fn rg8_renderbuffer_blit_from_default_reads_r_g_0_1() {
        blit_check(GL_RG8, false, [255, 255, 0, 255]);
    }
    #[test]
    fn rg8_texture_blit_from_default_reads_r_g_0_1() {
        blit_check(GL_RG8, true, [255, 255, 0, 255]);
    }
}
