// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D back end reads uniform blocks from the host arrays only
//! (`YakoglBackend::uniforms_host_only`, set in src/v3d/backend.rs), so the GL
//! core's draw path (`draw::draw`, "uniforms: upload each stage's block")
//! skips `staging_reserve` and the `put` calls for it
//! (docs/notes/pi4-terrain-uniforms.md, "Proposed fix"). Pinned here, on the
//! real V3D back end and GL core:
//!
//! * with the capability set, a draw with uniforms in both stages reserves no
//!   staging bytes for them (no GPU allocation of the program's uniform
//!   staging size exists, and none is created), and the blocks the back end
//!   hands the job builder hold the values the application set, as before;
//! * with the capability cleared (a bitwise copy of the V3D table, the rest
//!   of it the real back end's), the upload is unchanged: the staging buffer
//!   is allocated and holds the same words as the host arrays.
//!
//! The stand-in builder draws nothing, so no pixel is checked: the words the
//! real builder would copy are, and they are the ones tests/v3d_uniform_host.rs
//! pins across draws, frames and program switches.
//!
//! The rig is the one of tests/v3d_lazy_raster_readers.rs without the render
//! target and the TFU: the real back end and GL core, the real
//! driver/src/v3d/v3d_tex.c built with `cc`, GPU memory is host memory, the
//! job builder stood in for.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_uniform_host_only
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

mod v3d {
    use core::ffi::{c_char, c_int, c_void, CStr};
    use std::collections::BTreeMap;
    use std::ffi::CString;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use yakogl::backend::YakoglBackend;
    use yakogl::gl::consts::*;
    use yakogl::gl::context::{self, GlContext};
    use yakogl::gl::draw::glDrawArrays;
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::state::*;
    use yakogl::gl::types::*;
    use yakogl::gl::vertex::*;
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
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
            let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_uniform_host_only");
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

    /// One draw as the back end handed it to the job builder: the words of
    /// the two uniform blocks it read (the real builder copies them into the
    /// job's data arena at this call), and whether the GL core's staging copy
    /// held the same words at that moment.
    #[derive(Clone, Debug, Default)]
    pub struct SeenDraw {
        pub vs: Vec<u32>,
        pub fs: Vec<u32>,
        /// Some program's staging buffer holds `vs` at offset 0 (true when
        /// the VS has no block).
        pub staged_vs: bool,
        /// ... and `fs` at the 16-byte aligned offset after the VS block.
        pub staged_fs: bool,
    }

    static DRAWS: Mutex<Vec<SeenDraw>> = Mutex::new(Vec::new());

    pub fn draws() -> MutexGuard<'static, Vec<SeenDraw>> {
        DRAWS.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The size of a program's uniform staging buffer
    /// (`MAX_UNIFORM_DWORDS * 4 * 2 + 16`, src/gl/draw.rs).
    const STAGING_BYTES: usize = yakogl::gl::context::MAX_UNIFORM_DWORDS * 8 + 16;

    /// Whether some program's staging buffer holds `words` at byte `off`.
    fn staged_at(words: &[u32], off: usize) -> bool {
        if words.is_empty() {
            return true;
        }
        with_platform(|p| {
            p.allocs.values().filter(|a| a.size == STAGING_BYTES).any(|a| {
                // SAFETY: the allocation's live u64 backing, `size` bytes.
                let bytes = unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u8, a.size) };
                words.iter().enumerate().all(|(i, w)| bytes[off + i * 4..off + i * 4 + 4] == w.to_ne_bytes())
            })
        })
    }

    fn block(s: &V3dUniformStorage) -> Vec<u32> {
        if s.words.is_null() || s.count == 0 {
            Vec::new()
        } else {
            // SAFETY: the backend's live host array, `count` dwords.
            unsafe { std::slice::from_raw_parts(s.words, s.count as usize) }.to_vec()
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
        let (j, d) = unsafe { (&mut *j, &*d) };
        j.draws += 1;
        // SAFETY: the job's BCL is live.
        unsafe { (*j.bcl).used += 120 };
        let (vs, fs) = (block(&d.vs_storage), block(&d.fs_storage));
        let fs_off = (vs.len() * 4 + 15) & !15;
        let seen = SeenDraw { staged_vs: staged_at(&vs, 0), staged_fs: staged_at(&fs, fs_off), vs, fs };
        draws().push(seen);
        V3D_JOB_OK
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_end(
        j: *mut V3dJob,
        _c: *const V3dJobClear,
        _tile_list_start: *mut u32,
        _tile_list_end: *mut u32,
    ) -> bool {
        // SAFETY: the backend passes its live job.
        let j = unsafe { &mut *j };
        // SAFETY: the job's lists are live.
        unsafe {
            (*j.bcl).used += 1;
            (*j.rcl).used = 200;
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
        fence as i64
    }

    unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
        0
    }

    unsafe extern "C" fn p_completed() -> i64 {
        with_platform(|p| p.kicks.len() as i64)
    }

    unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
        use yakogl::osal::{OSAL_QUERY_V3D_TFU_MIN_BYTES, OSAL_QUERY_V3D_TFU_RUN};
        if what == OSAL_QUERY_V3D_TFU_MIN_BYTES || what == OSAL_QUERY_V3D_TFU_RUN {
            return -95; /* -ENOTSUP: no TFU, as the host's default */
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

    /* ---- the context ------------------------------------------------------------- */

    const CANVAS: (i32, i32) = (40, 30);

    static LOCK: Mutex<()> = Mutex::new(());
    static mut CTX: *mut GlContext = core::ptr::null_mut();

    /// A back end started on the platform above, and a context on it; torn
    /// down when dropped.
    pub struct Rig {
        _g: MutexGuard<'static, ()>,
    }

    pub fn rig() -> Rig {
        rig_with(&yakogl_backend_v3d)
    }

    /// The V3D back end as it is, with the capability cleared: the table of a
    /// back end that reads the GPU copy of the blocks.
    pub fn table_without_capability() -> &'static YakoglBackend {
        // SAFETY: a bitwise copy of a static table of raw pointers and
        // function pointers; the copy is leaked, so it outlives every user.
        let mut t = unsafe { core::ptr::read(&yakogl_backend_v3d) };
        t.uniforms_host_only = false;
        Box::leak(Box::new(t))
    }

    pub fn rig_with(table: &'static YakoglBackend) -> Rig {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        /* a fresh back end counts kicks from 1; memory the previous one left
         * allocated stays (the GL core may still point at it) */
        with_platform(|p| {
            p.kicks.clear();
            p.log.clear();
        });
        draws().clear();
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = table;
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
    /* ---- the programs and the values ---------------------------------------------- */

    /// Both stages with a block: a mat4, a vec4, an int and a vec4 array in the
    /// VS; a vec4, an int and a float array in the FS. Every uniform is read by
    /// every component, so none is optimised out.
    const VS_A: &str = "attribute vec4 a_position;\n\
        uniform mat4 u_mvp;\nuniform vec4 u_off;\nuniform int u_k;\nuniform vec4 u_arr[4];\n\
        void main() {\n\
            gl_Position = u_mvp * a_position + u_off * float(u_k) + u_arr[0] + u_arr[1] + u_arr[2] + u_arr[3];\n\
        }\n";
    const FS_A: &str = "precision mediump float;\nuniform vec4 u_color;\nuniform int u_sel;\nuniform float u_f[3];\n\
        void main() { gl_FragColor = u_color * float(u_sel) + vec4(u_f[0], u_f[1], u_f[2], 0.0); }\n";
    /// Distinct, exactly representable floats: `n` of them for `seed`, no two
    /// seeds sharing a value.
    pub fn floats(seed: u32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (seed * 4096 + i as u32 + 1) as f32 * 0.125).collect()
    }

    pub fn bits(f: &[f32]) -> Vec<u32> {
        f.iter().map(|v| v.to_bits()).collect()
    }

    /// The words an `int` uniform of value `k` is stored as.
    pub fn int_word(k: i32) -> u32 {
        k as u32
    }

    /// `run` appears in `words` as consecutive dwords.
    pub fn has_run(words: &[u32], run: &[u32]) -> bool {
        words.windows(run.len()).any(|w| w == run)
    }

    /// `word` appears anywhere in `words`.
    pub fn has_word(words: &[u32], word: u32) -> bool {
        words.contains(&word)
    }

    fn loc(p: GLuint, name: &str) -> GLint {
        let c = CString::new(name).unwrap();
        // SAFETY: a valid program and a NUL-terminated name.
        let l = unsafe { glGetUniformLocation(p, c.as_ptr()) };
        assert!(l >= 0, "uniform {name} has a location");
        l
    }

    /// The values of program A's uniforms for one draw.
    #[derive(Clone)]
    pub struct ValuesA {
        pub mvp: Vec<f32>,
        pub off: Vec<f32>,
        pub k: i32,
        pub arr: Vec<f32>,
        pub color: Vec<f32>,
        pub sel: i32,
        pub f: Vec<f32>,
    }

    pub fn values_a(seed: u32) -> ValuesA {
        ValuesA {
            mvp: floats(seed * 8, 16),
            off: floats(seed * 8 + 1, 4),
            k: 100 + seed as i32,
            arr: floats(seed * 8 + 2, 16),
            color: floats(seed * 8 + 3, 4),
            sel: 200 + seed as i32,
            f: floats(seed * 8 + 4, 3),
        }
    }

    /// A program A and its locations.
    pub struct ProgA {
        pub p: GLuint,
        l: [GLint; 7],
    }

    pub fn prog_a() -> ProgA {
        let p = program(VS_A, FS_A);
        let l = ["u_mvp", "u_off", "u_k", "u_arr", "u_color", "u_sel", "u_f"].map(|n| loc(p, n));
        ProgA { p, l }
    }

    impl ProgA {
        /// Sets every uniform of the program (it must be in use).
        pub fn set(&self, v: &ValuesA) {
            // SAFETY: plain GL calls; the arrays hold the counts passed.
            unsafe {
                glUniformMatrix4fv(self.l[0], 1, 0, v.mvp.as_ptr());
                glUniform4fv(self.l[1], 1, v.off.as_ptr());
                glUniform1i(self.l[2], v.k);
                glUniform4fv(self.l[3], 4, v.arr.as_ptr());
                glUniform4fv(self.l[4], 1, v.color.as_ptr());
                glUniform1i(self.l[5], v.sel);
                glUniform1fv(self.l[6], 3, v.f.as_ptr());
                assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
            }
        }
    }

    /// Draws the quad with the current program and returns the draw the back
    /// end saw (the last one the job builder was handed).
    pub fn draw() -> SeenDraw {
        let before = draws().len();
        draw_quad();
        let d = draws();
        assert_eq!(d.len(), before + 1, "one draw reached the job builder: {:?}", logs());
        d[before].clone()
    }

    /// Submits the open job: the end of the frame.
    pub fn end_frame() {
        // SAFETY: plain GL call.
        unsafe { glFinish() };
    }

    pub fn use_program(p: GLuint) {
        // SAFETY: a valid program.
        unsafe { glUseProgram(p) };
    }

    /// The draw received exactly the values `v` of program A: every uniform's
    /// words are in its blocks, in the order the application laid them out
    /// (matrices, vectors and arrays as consecutive dwords).
    pub fn assert_a(d: &SeenDraw, v: &ValuesA, what: &str) {
        assert!(has_run(&d.vs, &bits(&v.mvp)), "{what}: the mat4 in the VS block {:x?}", d.vs);
        assert!(has_run(&d.vs, &bits(&v.off)), "{what}: the vec4 in the VS block");
        assert!(has_word(&d.vs, int_word(v.k)), "{what}: the int in the VS block {:x?}", d.vs);
        assert!(has_run(&d.vs, &bits(&v.arr)), "{what}: the vec4[4] in the VS block");
        assert!(has_run(&d.fs, &bits(&v.color)), "{what}: the vec4 in the FS block {:x?}", d.fs);
        assert!(has_word(&d.fs, int_word(v.sel)), "{what}: the int in the FS block");
        assert!(has_run(&d.fs, &bits(&v.f)), "{what}: the float[3] in the FS block");
    }

    /// Live GPU allocations of the program's uniform staging size (the buffer
    /// is allocated once with the program, at its first draw).
    pub fn staging_allocs() -> usize {
        with_platform(|p| p.allocs.values().filter(|a| a.size == STAGING_BYTES).count())
    }

    /// Bytes written into those allocations: nonzero ones (the host memory
    /// starts zeroed, and the stand-in GPU writes nothing).
    pub fn staging_bytes_written() -> usize {
        with_platform(|p| {
            p.allocs
                .values()
                .filter(|a| a.size == STAGING_BYTES)
                // SAFETY: the allocation's live u64 backing, `size` bytes.
                .map(|a| unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u8, a.size) })
                .map(|b| b.iter().filter(|&&x| x != 0).count())
                .sum()
        })
    }

    /* ---- the cases ------------------------------------------------------------------ */

    /// The capability is the V3D's, and a draw with uniforms in both stages
    /// reserves no staging bytes for them yet reaches the job builder with
    /// every value the application set.
    #[test]
    fn v3d_draw_reserves_no_staging_for_uniforms() {
        let _r = rig();
        assert!(yakogl::backend::uniforms_host_only(), "the V3D table sets the capability");
        let base = staging_allocs();
        let a = prog_a();
        use_program(a.p);
        let vals = [values_a(1), values_a(2)];
        let seen: Vec<SeenDraw> = vals
            .iter()
            .map(|v| {
                a.set(v);
                draw()
            })
            .collect();
        end_frame();
        for (d, v) in seen.iter().zip(&vals) {
            assert_a(d, v, "a V3D draw");
            assert!(!d.vs.is_empty() && !d.fs.is_empty(), "both stages have a block");
            assert!(!d.staged_vs && !d.staged_fs, "no staging copy of the blocks");
        }
        /* the program's buffer exists (its addresses are set), but no draw wrote
         * a byte into it and none reserved a fresh one */
        assert!(staging_allocs() <= base + 1, "no reserve beyond the program's own buffer");
        assert_eq!(staging_bytes_written(), 0, "no byte of the blocks was staged");
    }

    /// Without the capability the GL core uploads as it always did: the staging
    /// buffer exists and holds the words the back end reads from the arrays.
    #[test]
    fn a_backend_without_the_capability_still_gets_the_upload() {
        let _r = rig_with(table_without_capability());
        assert!(!yakogl::backend::uniforms_host_only());
        let base = staging_allocs();
        let a = prog_a();
        use_program(a.p);
        let v = values_a(3);
        a.set(&v);
        let d = draw();
        end_frame();
        assert_a(&d, &v, "a draw with the upload");
        assert!(d.staged_vs && d.staged_fs, "the staging copy holds both blocks");
        assert!(staging_allocs() > base, "the staging buffer was allocated");
        assert!(staging_bytes_written() > 0, "the blocks were written into it");
    }
}
