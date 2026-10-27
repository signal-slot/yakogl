// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every CPU reader of a sampled render target's level 0, on the V3D back
//! end: the safety net for the lazy raster store of
//! docs/notes/v3d-lazy-raster.md (docs/notes/v3d-sampled-copies.md, "Ranked
//! fix proposals" 1).
//!
//! A job rendering into a texture that is sampled (one with a render image,
//! `RtImage`) stores the UIF render image alone (v3d_job.h `uif_only`): the
//! raster (linear) storage of level 0 is written from the image only when a
//! CPU path needs it (`rt_level0_back`), and a job that loads the colour
//! meanwhile loads the image (`uif_load`). Each test renders a known picture
//! into a texture through an FBO, then reads level 0 through ONE CPU path and
//! checks the exact texels, so a path that finds the storage stale fails
//! here. The texels are read through GL (glReadPixels of an FBO), never from
//! the GL core's storage directly, which is stale on purpose; one test
//! (`the_render_stores_the_image_alone`) checks that it is.
//!
//! The rig is the one of tests/mipmap_offer_guard.rs (the real back end and
//! GL core, the real driver/src/v3d/v3d_tex.c built with `cc`, GPU memory is
//! host memory, the job builder stood in for). The stand-in GPU follows the
//! job (`store_target`): the tile buffer starts from what the job loads (the
//! raster storage, or the image with `uif_load`) or its clear colour, the
//! picture of the current `PHASE` is drawn over all of it or its left half
//! (`HALF`), and it is stored into the raster storage only when the job
//! stores it, and into the render image when the job names one.
//!
//! Every test runs with the TFU off (the default, `YAKOGL_V3D_TFU` unset) and
//! on, except the CPU-filter one, which is the TFU-off path by definition.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_lazy_raster_readers
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "gpu-v3d",
    feature = "shaderc-online",
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
/* the stand-ins take the C functions' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

/// The texels and the filter of tests/mipmap_offer_guard.rs.
pub mod common {
    /// Texel (x, y) of the render of frame `phase`: every channel a different
    /// ramp, so no average comes out right by accident.
    pub fn texel(phase: u32, x: u32, y: u32) -> [u8; 4] {
        [
            (x * 3 + phase * 29) as u8,
            (y * 5 + phase * 7) as u8,
            ((x ^ y) * 11 + phase) as u8,
            (255 - ((x + y) & 63)) as u8,
        ]
    }

    /// Level 0 of frame `phase`: `h` rows of `w` texels, no padding.
    pub fn level0(phase: u32, w: u32, h: u32) -> Vec<u8> {
        (0..h).flat_map(|y| (0..w).flat_map(move |x| texel(phase, x, y))).collect()
    }

    /// One level of `w` x `h` RGBA8 texels into the next, as the GL core's
    /// glGenerateMipmap filters 8-bit channels: each side halved (rounded
    /// down) unless it is 1, the box two texels along a halved side and one
    /// along a side of 1, the sum's average rounded half up.
    pub fn box_level(src: &[u8], w: u32, h: u32) -> Vec<u8> {
        let (dw, dh) = ((w / 2).max(1), (h / 2).max(1));
        let (sx, sy) = (if w > 1 { 2 } else { 1 }, if h > 1 { 2 } else { 1 });
        let mut out = Vec::with_capacity((dw * dh * 4) as usize);
        for y in 0..dh {
            for x in 0..dw {
                for c in 0..4 {
                    let sum: u32 = (0..sy)
                        .flat_map(|j| (0..sx).map(move |i| (j, i)))
                        .map(|(j, i)| u32::from(src[(((y * sy + j) * w + x * sx + i) * 4 + c) as usize]))
                        .sum();
                    out.push(((sum + sx * sy / 2) / (sx * sy)) as u8);
                }
            }
        }
        out
    }
}

mod v3d {
    use core::ffi::{c_char, c_int, c_void, CStr};
    use std::collections::BTreeMap;
    use std::ffi::CString;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use super::common;

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
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf, OsalV3dTfu};
    use yakogl::v3d::backend::*;
    use yakogl::v3d::ffi::*;
    use yakogl::v3d::kick::V3dKick;

    /* ---- the real layout code, built from driver/src/v3d/v3d_tex.c ------------- */

    extern "C" {
        fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    /// `driver/src/v3d/v3d_tex.c` and the Mesa tiling it includes, as a shared
    /// object of this test's own; its symbols are reached through the handle
    /// only (RTLD_LOCAL), so the trampolines below are the ones the library
    /// links.
    fn lib() -> *mut c_void {
        static L: OnceLock<usize> = OnceLock::new();
        *L.get_or_init(|| {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("depth_resolve_v3d");
            std::fs::create_dir_all(&dir).expect("the scratch directory");
            let so = dir.join(format!("v3d_tex-{}.so", std::process::id()));
            /* the compiler's own temporary files next to the object: a host
             * whose /tmp is full must not fail the check */
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

    /// Every texture state packed, by the record's CPU address.
    static STATES: Mutex<Vec<(usize, V3dTexState)>> = Mutex::new(Vec::new());

    #[no_mangle]
    unsafe extern "C" fn v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool {
        // SAFETY: the C function's signature.
        let f: unsafe extern "C" fn(*mut u8, *const V3dTexState) -> bool =
            unsafe { std::mem::transmute(sym("v3d_tex_pack_state")) };
        // SAFETY: the library's call, passed on; `s` is its live state.
        let ok = unsafe { f(out, s) };
        if ok {
            // SAFETY: the backend's live state.
            STATES.lock().unwrap_or_else(|e| e.into_inner()).push((out as usize, unsafe { *s }));
        }
        ok
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
        /* v3d_job_begin's rule: the image-only store and the image load
         * need an image */
        if (t.uif_only || t.uif_load) && t.uif_address == 0 {
            j.broken = true;
            return false;
        }
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
            /* the tile list is written from the job's target as it is now
             * (v3d_job.c emit_tile_list reads j->t): the back end drops the
             * render image from an open job whose image it frees */
            job.target = j.t;
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
    /// and a job into the target the test renders into stores its picture.
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
        store_target(&job);
        fence as i64
    }

    unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
        0
    }

    unsafe extern "C" fn p_completed() -> i64 {
        with_platform(|p| p.kicks.len() as i64)
    }

    unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
        // SAFETY: the backend's query arguments, passed on.
        if let Some(rc) = unsafe { tfu_query(what, out, out_sz) } {
            return rc;
        }
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
        -22 /* the default budgets, no clock */
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

    /* ---- the TFU, as src/osal/linux.c models it ------------------------------------ */

    /// How the platform answers the TFU queries.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    #[allow(dead_code)] /* `Failing` is mipmap_offer_guard's */
    enum Tfu {
        /// The default (`YAKOGL_V3D_TFU` unset): -ENOTSUP, the back end never
        /// asks for a job.
        Off,
        /// `YAKOGL_V3D_TFU=1`: jobs run, as linux.c's model runs them.
        On,
        /// The unit is there and every job fails.
        Failing,
    }

    static MODE: AtomicU8 = AtomicU8::new(0);

    fn mode() -> Tfu {
        [Tfu::Off, Tfu::On, Tfu::Failing][usize::from(MODE.load(Ordering::SeqCst))]
    }

    /// The layout of a `w` x `h` render image of `levels` levels, as
    /// `texture::rt_layout` lays it out: level 0 without the UIF XOR.
    fn rt_layout(w: u32, h: u32, levels: u32) -> Option<V3dTexLayout> {
        let mut l = V3dTexLayout::default();
        // SAFETY: a live layout to fill.
        if !unsafe { v3d_tex_layout(w, h, levels, 4, &mut l) } {
            return None;
        }
        if l.slices[0].tiling == V3D_TEX_TILING_UIF_XOR {
            l.slices[0].tiling = V3D_TEX_TILING_UIF_NO_XOR;
        }
        Some(l)
    }

    /// One TFU mipmap job, as linux.c's `tfu_run` runs it: the job words are
    /// checked against the packer's own for the layout they name, then levels
    /// 1.. of the image are made from its level 0 with the 8-bit box filter.
    fn tfu_run(q: &OsalV3dTfu) -> i32 {
        let (w, h) = (q.ios & 0xffff, q.ios >> 16);
        let levels = ((q.icfg >> 5) & 0xf) + 1;
        let level0 = q.ioa & !0x3f;
        let Some(l) = rt_layout(w, h, levels) else { return -22 };
        let Some(base) = level0.checked_sub(l.slices[0].offset) else { return -22 };
        let mut j = V3dTexTfuJob::default();
        // SAFETY: a live layout and job.
        if levels < 2 || !unsafe { v3d_tex_tfu_pack_mipmap(&l, V3D_TFU_TTYPE_RGBA8, base, &mut j) } {
            return -22;
        }
        let words = [j.icfg, j.iia, j.iis, j.ica, j.iua, j.ioa, j.ios, j.coef0];
        if words != [q.icfg, q.iia, q.iis, q.ica, q.iua, q.ioa, q.ios, q.coef0] {
            return -22;
        }
        with_platform(|p| {
            let image = platform_cpu(p, u64::from(base), l.size as usize);
            let mut rows = vec![0u8; (w * h * 4) as usize];
            // SAFETY: the image's live mapping holds the laid-out chain; `rows` is level 0's size.
            assert!(unsafe { v3d_tex_load_box(image, &l, 0, 0, 0, w, h, rows.as_mut_ptr(), w * 4) });
            let (mut lw, mut lh) = (w, h);
            for level in 1..levels {
                rows = common::box_level(&rows, lw, lh);
                (lw, lh) = ((lw / 2).max(1), (lh / 2).max(1));
                // SAFETY: as above; `rows` is the level's rows.
                unsafe { v3d_tex_store_level(image, &l, level, rows.as_ptr(), lw * 4) };
            }
        });
        0
    }

    /// The TFU's answers to the back end's queries (`OSAL_QUERY_V3D_TFU_MIN_BYTES`
    /// at init, `OSAL_QUERY_V3D_TFU_RUN` for a job); `None` for any other query.
    unsafe fn tfu_query(what: u32, out: *mut c_void, out_sz: usize) -> Option<i32> {
        use yakogl::osal::{OSAL_QUERY_V3D_TFU_MIN_BYTES, OSAL_QUERY_V3D_TFU_RUN};
        match what {
            OSAL_QUERY_V3D_TFU_MIN_BYTES if out_sz == 4 => Some(if mode() == Tfu::Off {
                -95
            } else {
                // SAFETY: a u32 out-parameter.
                unsafe { *(out as *mut u32) = 4096 };
                0
            }),
            OSAL_QUERY_V3D_TFU_RUN => Some(if mode() == Tfu::Off {
                -95
            } else if out_sz != size_of::<OsalV3dTfu>() {
                -22
            } else {
                // SAFETY: the backend's in/out struct of that size.
                let q = unsafe { &mut *(out as *mut OsalV3dTfu) };
                q.rc = if mode() == Tfu::Failing { -5 } else { tfu_run(q) };
                0
            }),
            _ => None,
        }
    }

    /// The render pass of a job on the target the test is rendering into
    /// (`TARGET`), as the stand-in GPU runs it, following the job's tile
    /// list (v3d_job.c `emit_tile_list`): the tile buffer starts from what
    /// the job loads (`load_rt0`: the raster storage, or the render image
    /// with `uif_load`) or from its clear colour; the picture of frame
    /// `PHASE` is drawn over it, the whole target or its left half (`HALF`);
    /// then it is stored into the raster storage unless the job stores the
    /// image alone (`uif_only`), and into the render image (`uif_address`) in
    /// its layout. A CPU reader that finds the storage stale fails a test.
    fn store_target(job: &SeenJob) {
        let t = &job.target;
        let (w, h) = (t.width, t.height);
        if t.address == 0 || TARGET.load(Ordering::SeqCst) != (u64::from(w) << 32 | u64::from(h)) {
            return;
        }
        let clear = job.clear.expect("the job was ended");
        let picture = common::level0(PHASE.load(Ordering::SeqCst), w, h);
        let cols = if HALF.load(Ordering::SeqCst) { w / 2 } else { w };
        with_platform(|p| {
            /* the image's layout: the one whose allocation (the layout's size
             * and a page) starts a page before level 0's base */
            let image = (t.uif_address != 0).then(|| {
                let found = (1..=16).find_map(|levels| {
                    let l = rt_layout(w, h, levels)?;
                    let base = t.uif_address.checked_sub(l.slices[0].offset)?;
                    let (start, a) = p.allocs.range(..=u64::from(base)).next_back()?;
                    (start.next_multiple_of(4096) == u64::from(base) && a.size == l.size as usize + 4096)
                        .then_some((l, base))
                });
                let (l, base) = found.expect("the render image is an allocation of a layout the job's address names");
                assert_eq!(t.uif_memory_format, l.slices[0].tiling, "the image's memory format is the layout's tiling");
                (l, base)
            });
            assert!(!t.uif_load || image.is_some(), "a job loads an image it names");
            let raster = platform_cpu(p, u64::from(t.address), (t.stride * h) as usize);
            /* the tile buffer, rows of `w` texels */
            let mut tb = vec![0u8; (w * h * 4) as usize];
            if !clear.load_rt0 {
                tb = clear.rgba8.to_le_bytes().repeat((w * h) as usize);
            } else if let Some((l, base)) = image.filter(|_| t.uif_load) {
                let img = platform_cpu(p, u64::from(base), l.size as usize);
                // SAFETY: the image's live mapping holds the laid-out chain; `tb` is level 0's size.
                assert!(unsafe { v3d_tex_load_box(img, &l, 0, 0, 0, w, h, tb.as_mut_ptr(), w * 4) });
            } else {
                for y in 0..h as usize {
                    // SAFETY: row `y` of the raster target, `w` texels inside `stride * h` bytes.
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            raster.add(y * t.stride as usize),
                            tb.as_mut_ptr().add(y * w as usize * 4),
                            w as usize * 4,
                        )
                    };
                }
            }
            for y in 0..h as usize {
                let row = y * w as usize * 4;
                tb[row..row + cols as usize * 4].copy_from_slice(&picture[row..row + cols as usize * 4]);
            }
            if !t.uif_only {
                for y in 0..h as usize {
                    // SAFETY: as the load above.
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            tb.as_ptr().add(y * w as usize * 4),
                            raster.add(y * t.stride as usize),
                            w as usize * 4,
                        )
                    };
                }
            }
            if let Some((l, base)) = image {
                let img = platform_cpu(p, u64::from(base), l.size as usize);
                // SAFETY: the image's live mapping holds the laid-out chain; `tb` is level 0.
                assert!(unsafe { v3d_tex_store_box(img, &l, 0, 0, 0, w, h, tb.as_ptr(), w * 4) });
            }
        });
    }

    /* ---- the context ------------------------------------------------------------- */

    const CANVAS: (i32, i32) = (40, 30);

    static LOCK: Mutex<()> = Mutex::new(());
    static TARGET: AtomicU64 = AtomicU64::new(0);
    static PHASE: AtomicU32 = AtomicU32::new(0);
    /// The job renders only the left half of the target.
    static HALF: AtomicBool = AtomicBool::new(false);
    static mut CTX: *mut GlContext = core::ptr::null_mut();

    /// A back end started on a platform that answers the TFU queries as
    /// `tfu` says, and a context on it; torn down when dropped.
    struct Rig {
        _g: MutexGuard<'static, ()>,
    }

    fn rig(tfu: Tfu) -> Rig {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        MODE.store(tfu as u8, Ordering::SeqCst);
        /* a fresh back end counts kicks from 1; memory the previous one left
         * allocated stays (the GL core may still point at it) */
        with_platform(|p| {
            p.kicks.clear();
            p.log.clear();
        });
        jobs().clear();
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_v3d;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(CANVAS.0 as u32, CANVAS.1 as u32);
            assert!(!CTX.is_null(), "{:?}", logs());
        }
        Rig { _g: g }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            // SAFETY: the context `rig` made, under the lock.
            unsafe { context::gl_context_destroy(CTX) };
            yakogl::backend::shutdown();
        }
    }

    #[allow(dead_code)]
    fn ctx() -> &'static mut GlContext {
        // SAFETY: set in `rig`, used under the lock.
        unsafe { &mut *CTX }
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
        let name = CString::new("a_position").unwrap();
        // SAFETY: plain GL calls on valid handles.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glBindAttribLocation(p, 0, name.as_ptr());
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link failed: {:?}", logs());
            p
        }
    }

    /// The render pass's program draws a constant (the stand-in GPU stores the
    /// picture); the scene's samples unit 0.
    const FILL_VS: &str = "attribute vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";
    const FILL_FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";
    const SAMPLE_VS: &str = "attribute vec4 a_position;\nvarying vec2 v_coord;\n\
        void main() { v_coord = a_position.xy * 0.5 + 0.5; gl_Position = a_position; }\n";
    const SAMPLE_FS: &str = "precision mediump float;\nuniform sampler2D Tex;\nvarying vec2 v_coord;\n\
        void main() { gl_FragColor = texture2D(Tex, v_coord); }\n";

    /* a quad over the whole target at z = 0, (x, y, z) per vertex */
    static QUAD: [f32; 18] = [
        -1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, //
        -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0,
    ];

    fn draw_quad() {
        // SAFETY: plain GL calls; the array is a static.
        unsafe {
            glVertexAttribPointer(0, 3, GL_FLOAT, 0, 12, QUAD.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 6);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
    }

    /// What glmark2 terrain and refract make: an RGBA8 texture of `w` x `h`,
    /// level 0 only (glTexImage2D with no data), sampled with a mipmap filter,
    /// and the FBO with it as the colour attachment.
    fn render_target(w: i32, h: i32) -> (GLuint, GLuint) {
        let (mut tex, mut fbo) = (0, 0);
        // SAFETY: room for one name each; a null image.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        (tex, fbo)
    }

    /// The render images' counters since init: (chains the TFU made, samples
    /// read from an image, chains handed back to the GL core's storage).
    fn rt_counters() -> (u32, u32, u32) {
        let (mut chains, mut direct, mut back) = (0u32, 0u32, 0u32);
        // SAFETY: live out-parameters.
        unsafe {
            v3d_rt_images(
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                &mut direct,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                &mut chains,
                &mut back,
            )
        };
        (chains, direct, back)
    }

    /// Level `l` of `tex` as the storage holds it: rows of `w >> l` texels, 4
    /// bytes each, without the row padding.
    #[allow(dead_code)]
    fn level_rows(tex: GLuint, l: u32) -> Vec<u8> {
        let t = &ctx().objs_ref().textures[tex as usize];
        let (lw, lh) = ((t.width >> l).max(1) as usize, (t.height >> l).max(1) as usize);
        let (pitch, off) = (t.level_pitch_at(l) as usize, t.level_off[l as usize] as usize);
        // SAFETY: the texture's live storage, `mem.size` bytes.
        let mem = unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, t.mem.size) };
        (0..lh).flat_map(|y| mem[off + y * pitch * 4..off + (y * pitch + lw) * 4].iter().copied()).collect()
    }

    /* ---- the readers ---------------------------------------------------------------- */

    /// glmark2 refract's shape at a size the host test can afford: POT, 64x32.
    const W: u32 = 64;
    const H: u32 = 32;

    /// A rendered-into texture, its FBO and the two programs.
    struct Scene {
        tex: GLuint,
        fbo: GLuint,
        fill: GLuint,
        sample: GLuint,
    }

    fn scene() -> Scene {
        let (fill, sample) = (program(FILL_VS, FILL_FS), program(SAMPLE_VS, SAMPLE_FS));
        let (tex, fbo) = render_target(W as i32, H as i32);
        /* sampled without a mipmap filter: level 0 alone is complete, so the
         * draws of `sample_it` sample the target (with the mipmap filter and
         * no chain it is incomplete, sampled as the stand-in texture, and no
         * render image is ever made) */
        // SAFETY: plain GL calls on a live name.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
        }
        TARGET.store(u64::from(W) << 32 | u64::from(H), Ordering::SeqCst);
        HALF.store(false, Ordering::SeqCst);
        Scene { tex, fbo, fill, sample }
    }

    /// The render pass of frame `phase` into `fbo`: the job is left open, as
    /// the scene's own draws leave it until something submits it.
    fn render_into(s: &Scene, fbo: GLuint, phase: u32) {
        PHASE.store(phase, Ordering::SeqCst);
        // SAFETY: plain GL calls on live names.
        unsafe {
            glUseProgram(s.fill);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glViewport(0, 0, W as i32, H as i32);
        }
        draw_quad();
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, CANVAS.0, CANVAS.1);
        }
    }

    fn render(s: &Scene, phase: u32) {
        render_into(s, s.fbo, phase);
    }

    /// The scene's draw of the window system's framebuffer that samples `tex`
    /// (the GPU's reader of the target), then glFinish.
    fn sample_it(s: &Scene, tex: GLuint) {
        let name = CString::new("Tex").unwrap();
        // SAFETY: plain GL calls on live names.
        unsafe {
            glUseProgram(s.sample);
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, tex);
            glUniform1i(glGetUniformLocation(s.sample, name.as_ptr()), 0);
        }
        draw_quad();
        // SAFETY: plain GL call.
        unsafe { glFinish() };
    }

    fn fbo_of(tex: GLuint, level: i32) -> GLuint {
        let mut fbo = 0;
        // SAFETY: plain GL calls on live names.
        unsafe {
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, level);
            let st = glCheckFramebufferStatus(GL_FRAMEBUFFER);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            assert_eq!(st, GL_FRAMEBUFFER_COMPLETE, "level {level} as a colour attachment: {:?}", logs());
        }
        fbo
    }

    /// glReadPixels of the whole `w` x `h` colour attachment of `fbo`.
    fn read_fbo(fbo: GLuint, w: u32, h: u32) -> Vec<u8> {
        let mut px = vec![0xa5u8; (w * h * 4) as usize];
        // SAFETY: plain GL calls; `px` is the size of the read.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glReadPixels(0, 0, w as i32, h as i32, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
            let e = glGetError();
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            assert_eq!(e, GL_NO_ERROR, "glReadPixels: {:?}", logs());
        }
        px
    }

    /// The first texel where `got` and `want` (rows of `w` texels) differ.
    fn assert_texels(got: &[u8], want: &[u8], w: u32, what: &str) {
        assert_eq!(got.len(), want.len(), "{what}: the size");
        if let Some(i) = got.iter().zip(want).position(|(a, b)| a != b) {
            let t = i / 4;
            panic!(
                "{what}: texel ({}, {}) channel {} is {} where {} was rendered: {:?}",
                t as u32 % w,
                t as u32 / w,
                i % 4,
                got[i],
                want[i],
                logs()
            );
        }
    }

    /// An empty `w` x `h` RGBA8 texture (zero texels), bound to unit 0.
    fn blank_texture(w: u32, h: u32) -> GLuint {
        let mut tex = 0;
        let zero = vec![0u8; (w * h * 4) as usize];
        // SAFETY: plain GL calls; `zero` is the image.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as GLint,
                w as i32,
                h as i32,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                zero.as_ptr() as *const c_void,
            );
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        tex
    }

    /// `body` once with the TFU off and once with it on, each on a fresh rig.
    fn both_tfu_modes(body: impl Fn(Tfu)) {
        for tfu in [Tfu::Off, Tfu::On] {
            let _r = rig(tfu);
            body(tfu);
        }
    }

    /// The lazy store itself: once the target has a render image, a render
    /// into it stores the image alone and leaves the GL core's storage as it
    /// was; the first CPU reader writes level 0 back, and the next render
    /// (which does not clear) loads the image.
    #[test]
    fn the_render_stores_the_image_alone() {
        both_tfu_modes(|tfu| {
            let s = scene();
            render(&s, 1);
            sample_it(&s, s.tex);
            render(&s, 2);
            sample_it(&s, s.tex);
            let job = jobs().iter().rev().find(|j| j.target.uif_address != 0).cloned().expect("a job stored the image");
            assert!(job.target.uif_only && !job.target.uif_load, "{tfu:?}: image alone, loaded from the storage");
            assert_texels(&level_rows(s.tex, 0), &common::level0(1, W, H), W, &format!("{tfu:?}: storage untouched"));
            /* the render after it loads the image, the live copy */
            render(&s, 3);
            sample_it(&s, s.tex);
            let job = jobs().iter().rev().find(|j| j.target.uif_address != 0).cloned().unwrap();
            assert!(job.target.uif_only && job.target.uif_load, "{tfu:?}: image alone, loaded from the image");
            assert_texels(&read_fbo(s.fbo, W, H), &common::level0(3, W, H), W, &format!("{tfu:?}: read back"));
            assert_texels(&level_rows(s.tex, 0), &common::level0(3, W, H), W, &format!("{tfu:?}: written back"));
            assert!(rt_counters().1 >= 2, "{tfu:?}: the draws sampled the render image");
            /* a target a CPU reader needed stores its raster again: a reader
             * every frame costs one write-back, not one a frame */
            render(&s, 4);
            sample_it(&s, s.tex);
            let job = jobs().iter().rev().find(|j| j.target.uif_address != 0).cloned().unwrap();
            assert!(!job.target.uif_only && !job.target.uif_load, "{tfu:?}: raster and image, loaded from the storage");
            assert_texels(&level_rows(s.tex, 0), &common::level0(4, W, H), W, &format!("{tfu:?}: stored again"));
        });
    }

    /// Reader 1, `glReadPixels` from the FBO the target was rendered through.
    /// The target was sampled once (its render image exists and is the GPU's
    /// copy) and then rendered into again: the read must see the new picture.
    #[test]
    fn read_pixels_from_the_fbo() {
        both_tfu_modes(|tfu| {
            let s = scene();
            render(&s, 1);
            assert_texels(&read_fbo(s.fbo, W, H), &common::level0(1, W, H), W, &format!("{tfu:?}: first render"));
            sample_it(&s, s.tex);
            assert_texels(&read_fbo(s.fbo, W, H), &common::level0(1, W, H), W, &format!("{tfu:?}: after a sample"));
        });
    }

    /// Reader 2, `glReadPixels` after a second render: the picture of the
    /// second render, not the first one's (the stale-image case).
    #[test]
    fn read_pixels_after_a_second_render() {
        both_tfu_modes(|tfu| {
            let s = scene();
            render(&s, 1);
            sample_it(&s, s.tex);
            render(&s, 2);
            assert_texels(
                &read_fbo(s.fbo, W, H),
                &common::level0(2, W, H),
                W,
                &format!("{tfu:?}: render, sample, render"),
            );
            sample_it(&s, s.tex);
            render(&s, 3);
            sample_it(&s, s.tex);
            render(&s, 4);
            assert_texels(&read_fbo(s.fbo, W, H), &common::level0(4, W, H), W, &format!("{tfu:?}: four frames"));
        });
    }

    /// Reader 3, `glCopyTexImage2D` from the FBO into a texture of its own, the
    /// whole target and then a rectangle of the second render.
    #[test]
    fn copy_tex_image_2d_from_the_target() {
        both_tfu_modes(|tfu| {
            let s = scene();
            render(&s, 1);
            sample_it(&s, s.tex);
            render(&s, 2);
            let mut dst = 0;
            // SAFETY: plain GL calls on live names.
            unsafe {
                glGenTextures(1, &mut dst);
                glBindTexture(GL_TEXTURE_2D, dst);
                glBindFramebuffer(GL_FRAMEBUFFER, s.fbo);
                glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 0, 0, W as i32, H as i32, 0);
                assert_eq!(glGetError(), GL_NO_ERROR, "glCopyTexImage2D: {:?}", logs());
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
            }
            let want = common::level0(2, W, H);
            assert_texels(&read_fbo(fbo_of(dst, 0), W, H), &want, W, &format!("{tfu:?}: the whole target"));
            /* a rectangle, with an origin that is not 0: (8, 4) 16x8 */
            let mut part = 0;
            // SAFETY: plain GL calls on live names.
            unsafe {
                glGenTextures(1, &mut part);
                glBindTexture(GL_TEXTURE_2D, part);
                glBindFramebuffer(GL_FRAMEBUFFER, s.fbo);
                glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 8, 4, 16, 8, 0);
                assert_eq!(glGetError(), GL_NO_ERROR, "glCopyTexImage2D rectangle: {:?}", logs());
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
            }
            let rect: Vec<u8> = (4..12u32).flat_map(|y| (8..24u32).flat_map(move |x| common::texel(2, x, y))).collect();
            assert_texels(&read_fbo(fbo_of(part, 0), 16, 8), &rect, 16, &format!("{tfu:?}: a rectangle"));
        });
    }

    /// Reader 4, `glCopyTexSubImage2D` from the FBO into part of a texture that
    /// holds other texels: the rectangle is the render's, the rest is kept.
    #[test]
    fn copy_tex_sub_image_2d_from_the_target() {
        both_tfu_modes(|tfu| {
            let s = scene();
            render(&s, 1);
            sample_it(&s, s.tex);
            render(&s, 2);
            let dst = blank_texture(W, H);
            // SAFETY: plain GL calls on live names.
            unsafe {
                glBindFramebuffer(GL_FRAMEBUFFER, s.fbo);
                /* source (8, 4) 16x8 to (2, 3) of the destination */
                glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 2, 3, 8, 4, 16, 8);
                assert_eq!(glGetError(), GL_NO_ERROR, "glCopyTexSubImage2D: {:?}", logs());
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
            }
            let mut want = vec![0u8; (W * H * 4) as usize];
            for y in 0..8u32 {
                for x in 0..16u32 {
                    let o = (((y + 3) * W + x + 2) * 4) as usize;
                    want[o..o + 4].copy_from_slice(&common::texel(2, x + 8, y + 4));
                }
            }
            assert_texels(&read_fbo(fbo_of(dst, 0), W, H), &want, W, &format!("{tfu:?}"));
        });
    }

    /// Reader 5, a partial `glTexSubImage2D` on level 0 of the rendered-into
    /// texture, then a draw that samples it, then the readback: the patch over
    /// the render, the render around it. The patch is also read with no sample
    /// in between.
    #[test]
    fn partial_tex_sub_image_2d_then_sample_then_read() {
        both_tfu_modes(|tfu| {
            let s = scene();
            /* frame 0 into the storage, then frame 1 into the render image
             * alone: the storage is stale when the patch lands */
            render(&s, 0);
            sample_it(&s, s.tex);
            render(&s, 1);
            sample_it(&s, s.tex);
            let patch: Vec<u8> = (0..8u32).flat_map(|y| (0..16u32).flat_map(move |x| common::texel(9, x, y))).collect();
            let upload = |tex: GLuint| {
                // SAFETY: plain GL calls; `patch` is 16x8 texels.
                unsafe {
                    glBindTexture(GL_TEXTURE_2D, tex);
                    glTexSubImage2D(
                        GL_TEXTURE_2D,
                        0,
                        8,
                        4,
                        16,
                        8,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        patch.as_ptr() as *const c_void,
                    );
                    assert_eq!(glGetError(), GL_NO_ERROR, "glTexSubImage2D: {:?}", logs());
                }
            };
            let mut want = common::level0(1, W, H);
            for y in 0..8u32 {
                for x in 0..16u32 {
                    let o = (((y + 4) * W + x + 8) * 4) as usize;
                    want[o..o + 4].copy_from_slice(&common::texel(9, x, y));
                }
            }
            upload(s.tex);
            assert_texels(&read_fbo(s.fbo, W, H), &want, W, &format!("{tfu:?}: patch, read"));
            /* the patch changed what the GPU's copy must hold: sample, patch
             * again over the sampled picture, sample, read */
            sample_it(&s, s.tex);
            upload(s.tex);
            sample_it(&s, s.tex);
            assert_texels(&read_fbo(s.fbo, W, H), &want, W, &format!("{tfu:?}: patch, sample, read"));
            /* and a render after the patch replaces all of it */
            render(&s, 2);
            sample_it(&s, s.tex);
            assert_texels(&read_fbo(s.fbo, W, H), &common::level0(2, W, H), W, &format!("{tfu:?}: patch, render"));
        });
    }

    /// Reader 8, `glBlitFramebuffer` (ES 3.0) with the target as the read
    /// framebuffer, the whole of it into a texture of its own.
    #[test]
    fn blit_framebuffer_from_the_target() {
        both_tfu_modes(|tfu| {
            let s = scene();
            render(&s, 1);
            sample_it(&s, s.tex);
            render(&s, 2);
            let dst = blank_texture(W, H);
            let dfbo = fbo_of(dst, 0);
            // SAFETY: plain GL calls on live names.
            unsafe {
                glBindFramebuffer(GL_READ_FRAMEBUFFER, s.fbo);
                glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dfbo);
                let (w, h) = (W as i32, H as i32);
                glBlitFramebuffer(0, 0, w, h, 0, 0, w, h, GL_COLOR_BUFFER_BIT, GL_NEAREST);
                let e = glGetError();
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                assert_eq!(e, GL_NO_ERROR, "glBlitFramebuffer: {:?}", logs());
            }
            assert_texels(&read_fbo(dfbo, W, H), &common::level0(2, W, H), W, &format!("{tfu:?}: whole"));
        });
    }

    /// Reader 6, glGenerateMipmap with the GL core's CPU filter
    /// (`YAKOGL_V3D_TFU` unset), then level 1 read through an FBO: the box
    /// filter of the picture last rendered, after two frames.
    #[test]
    fn cpu_filtered_mipmap_then_read_level_1() {
        let _r = rig(Tfu::Off);
        let s = scene();
        for phase in 1..=2 {
            render(&s, phase);
            // SAFETY: plain GL calls on live names.
            unsafe {
                glBindTexture(GL_TEXTURE_2D, s.tex);
                glGenerateMipmap(GL_TEXTURE_2D);
                assert_eq!(glGetError(), GL_NO_ERROR, "glGenerateMipmap: {:?}", logs());
            }
            sample_it(&s, s.tex);
            let want = common::box_level(&common::level0(phase, W, H), W, H);
            let l1 = fbo_of(s.tex, 1);
            assert_texels(&read_fbo(l1, W / 2, H / 2), &want, W / 2, &format!("level 1 of frame {phase}"));
            assert_eq!(rt_counters().0, 0, "the CPU filter made no chain");
            // SAFETY: a name this test made.
            unsafe { glDeleteFramebuffers(1, &l1) };
        }
        /* level 0 is still the render */
        assert_texels(&read_fbo(s.fbo, W, H), &common::level0(2, W, H), W, "level 0 after the chains");
    }

    /// Reader 7, the texture attached to another FBO and rendered into only on
    /// the left half: the other half keeps the previous picture, read through
    /// either FBO.
    #[test]
    fn reattached_texture_rendered_on_half() {
        both_tfu_modes(|tfu| {
            let s = scene();
            render(&s, 1);
            sample_it(&s, s.tex);
            let other = fbo_of(s.tex, 0);
            HALF.store(true, Ordering::SeqCst);
            render_into(&s, other, 2);
            /* the job is submitted (and the stand-in stores) before HALF is reset */
            // SAFETY: plain GL call.
            unsafe { glFinish() };
            HALF.store(false, Ordering::SeqCst);
            let want: Vec<u8> = (0..H)
                .flat_map(|y| (0..W).flat_map(move |x| common::texel(if x < W / 2 { 2 } else { 1 }, x, y)))
                .collect();
            assert_texels(&read_fbo(other, W, H), &want, W, &format!("{tfu:?}: through the second FBO"));
            assert_texels(&read_fbo(s.fbo, W, H), &want, W, &format!("{tfu:?}: through the first FBO"));
            /* sampled, then the other half rendered by the first FBO */
            sample_it(&s, s.tex);
            render(&s, 3);
            assert_texels(&read_fbo(other, W, H), &common::level0(3, W, H), W, &format!("{tfu:?}: both halves"));
        });
    }
}
