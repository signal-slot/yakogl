// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A V3D draw whose uniform block is backed by less than its 16384 bytes
//! (plan #375, docs/notes/ubo-limits.md): a fragment shader reads the first
//! and the last dword of a std140 block of 1024 vec4, and the buffer bound to
//! its binding point is shorter than that, or absent. ES 3.0 2.11.7 makes the
//! values such a draw reads undefined and asks for no error; the driver must
//! not read outside the buffer on the host, nor panic.
//!
//! What the driver does (src/gl/program.rs:736-819, `fill_uniform_blocks` and
//! `copy_block_member`, called from the draw at src/gl/draw.rs:3907; the
//! staging at src/gl/draw.rs:3920-3960 only copies the program's host array):
//!
//! * the bytes it reads are the bound range clamped to the buffer as it is at
//!   draw time (`start = min(offset, size)`, `len = min(range, size - start)`,
//!   the whole rest for glBindBufferBase), so a range past the buffer's end
//!   or a buffer redefined smaller after the bind reads only what exists.
//! * a member dword whose 4 bytes are not inside those bytes is skipped
//!   (`data.get(at..at + 4)` is `None`): neither zero-filled nor read. The
//!   program's array keeps what it held: 0 after the link (the arrays are
//!   calloc'd at src/gl/program.rs:2069-2074), or the value an earlier draw
//!   copied there (stale).
//! * no buffer at the binding point, the name 0, or a buffer without a data
//!   store (no CPU view): the whole block is skipped the same way.
//! * the draw reports no error (GL_NO_ERROR) in every case.
//!
//! The rig is the one of tests/v3d_ubo_full_stage.rs (the real back end and
//! GL core, GPU memory is host memory, the job builder stood in for); the
//! draw's array is what the back end hands the job builder as `fs_storage`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_ubo_short_buffer
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
            let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_uniform_host");
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
    trampoline!(v3d_tex_pack_sampler_compare(out: *mut u8, s: *const V3dSampler, compare_op: u32) -> bool);
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
    #[allow(dead_code)]
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

    /// Draws the quad with the current program and returns the draw the back
    /// end handed the job builder.
    fn draw() -> SeenDraw {
        let before = draws().len();
        draw_quad();
        let d = draws();
        assert_eq!(d.len(), before + 1, "one draw reached the job builder: {:?}", logs());
        d[before].clone()
    }

    /// Submits the open job: the end of the frame.
    fn end_frame() {
        // SAFETY: plain GL call.
        unsafe { glFinish() };
    }

    fn use_program(p: GLuint) {
        // SAFETY: a valid program.
        unsafe { glUseProgram(p) };
    }

    /* ---- the programs ---------------------------------------------------------- */

    use yakogl::gl::buffer::*;
    use yakogl::gl::es3::{glBindBufferBase, glBindBufferRange};

    const VS_PLAIN: &str = "#version 300 es\nin highp vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";

    /// A std140 block `Big` of 16 members `highp vec4 m<k>[64]` (16384 bytes,
    /// 4096 dwords); the shader reads its first and last vec4.
    fn fs_big() -> String {
        let members: String = (0..16).map(|m| format!("highp vec4 m{m}[64]; ")).collect();
        format!(
            "#version 300 es\nuniform Big {{ {members}}};\nlayout(location = 0) out highp vec4 o;\nvoid main() {{ o = m0[0] + m15[63]; }}\n"
        )
    }

    const BLOCK_DWORDS: usize = 4096;

    /// The value of buffer dword `k`: distinct, non-zero, exact in f32.
    fn value(k: usize) -> f32 {
        ((k + 1) as f32) * 0.25
    }

    /// A program reading `Big`, its block at binding 0, in use.
    fn big_program() -> GLuint {
        let p = program(VS_PLAIN, &fs_big());
        let name = CString::new("Big").unwrap();
        // SAFETY: plain GL calls on a live program.
        unsafe {
            let idx = glGetUniformBlockIndex(p, name.as_ptr());
            assert_ne!(idx, u32::MAX, "block Big");
            glUniformBlockBinding(p, idx, 0);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        use_program(p);
        p
    }

    /// A buffer of `dwords` dwords, dword k holding `value(k)`; left bound to
    /// GL_UNIFORM_BUFFER.
    fn buffer(dwords: usize) -> GLuint {
        let data: Vec<f32> = (0..dwords).map(value).collect();
        let mut buf = 0;
        // SAFETY: plain GL calls; `data` is `dwords * 4` bytes.
        unsafe {
            glGenBuffers(1, &mut buf);
            glBindBuffer(GL_UNIFORM_BUFFER, buf);
            glBufferData(GL_UNIFORM_BUFFER, (dwords * 4) as GLsizeiptr, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        buf
    }

    fn bind_base(buf: GLuint) {
        // SAFETY: a plain GL call.
        unsafe {
            glBindBufferBase(GL_UNIFORM_BUFFER, 0, buf);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
    }

    fn bind_range(buf: GLuint, offset: usize, size: usize) {
        // SAFETY: a plain GL call.
        unsafe {
            glBindBufferRange(GL_UNIFORM_BUFFER, 0, buf, offset as GLintptr, size as GLsizeiptr);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
    }

    /// Where the block's dword 0 sits in the draw's fragment array: a program
    /// of the same source drawn with a full 16384-byte buffer, whose first and
    /// last dwords are checked there. Every program of `fs_big` lays out alike.
    fn block_at() -> usize {
        let p = big_program();
        let buf = buffer(BLOCK_DWORDS);
        bind_base(buf);
        let d = draw();
        end_frame();
        let first = value(0).to_bits();
        let at = d.fs.iter().position(|&w| w == first).expect("the block's first dword in the draw");
        assert!(d.fs.len() >= at + BLOCK_DWORDS, "the whole block is in the draw: {} from {at}", d.fs.len());
        for k in [1, 3, 4092, 4095] {
            assert_eq!(d.fs[at + k], value(k).to_bits(), "full block, dword {k}");
        }
        // SAFETY: plain GL calls on live names.
        unsafe {
            glBindBufferBase(GL_UNIFORM_BUFFER, 0, 0);
            glDeleteBuffers(1, &buf);
            glUseProgram(0);
            glDeleteProgram(p);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        at
    }

    /// The block's 4096 dwords as the draw handed them on.
    fn block_of(d: &SeenDraw, at: usize) -> &[u32] {
        assert!(d.fs.len() >= at + BLOCK_DWORDS, "the whole block is in the draw: {} from {at}", d.fs.len());
        &d.fs[at..at + BLOCK_DWORDS]
    }

    /// Dwords `first..first + n` of the block hold the buffer's dwords from `src`.
    fn assert_read(b: &[u32], first: usize, n: usize, src: usize, what: &str) {
        for (i, &w) in b[first..first + n].iter().enumerate() {
            assert_eq!(w, value(src + i).to_bits(), "{what}: block dword {}", first + i);
        }
    }

    /// Dwords `from..` of the block are 0 (skipped on a fresh program).
    fn assert_zero_from(b: &[u32], from: usize, what: &str) {
        if let Some(k) = (from..BLOCK_DWORDS).find(|&k| b[k] != 0) {
            panic!("{what}: block dword {k} is 0x{:x}, not the 0 the link left", b[k]);
        }
    }

    /* ---- the cases ------------------------------------------------------------- */

    /// (a) glBindBufferBase of a 64-byte buffer: the draw has no error, the
    /// first 16 dwords are the buffer's, the rest of the block stays 0.
    #[test]
    fn base_64_byte_buffer() {
        let _r = rig();
        let at = block_at();
        big_program();
        bind_base(buffer(16));
        let d = draw();
        end_frame();
        let b = block_of(&d, at);
        assert_read(b, 0, 16, 0, "64-byte buffer");
        assert_zero_from(b, 16, "64-byte buffer");
    }

    /// A buffer of 6 bytes: dword 0 is read, the half dword behind it is not.
    #[test]
    fn base_6_byte_buffer_reads_no_partial_dword() {
        let _r = rig();
        let at = block_at();
        big_program();
        let buf = buffer(2);
        let src = [value(0), value(1)];
        // SAFETY: plain GL calls; the source is 8 bytes, 6 of them taken.
        unsafe {
            glBufferData(GL_UNIFORM_BUFFER, 6, src.as_ptr() as *const c_void, GL_STATIC_DRAW);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        bind_base(buf);
        let d = draw();
        end_frame();
        let b = block_of(&d, at);
        assert_read(b, 0, 1, 0, "6-byte buffer");
        assert_zero_from(b, 1, "6-byte buffer");
    }

    /// The skipped dwords keep what an earlier draw put there: a full buffer,
    /// then a 64-byte one, leaves dwords 16.. at the full buffer's values.
    #[test]
    fn base_64_byte_buffer_after_a_full_one_keeps_the_stale_tail() {
        let _r = rig();
        let at = block_at();
        big_program();
        bind_base(buffer(BLOCK_DWORDS));
        let _ = draw();
        let short = buffer(16);
        let src: Vec<f32> = (0..16).map(|k| value(1000 + k)).collect();
        // SAFETY: plain GL call; the source is 64 bytes.
        unsafe {
            glBufferSubData(GL_UNIFORM_BUFFER, 0, 64, src.as_ptr() as *const c_void);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        bind_base(short);
        let d = draw();
        end_frame();
        let b = block_of(&d, at);
        assert_read(b, 0, 16, 1000, "the 64-byte buffer");
        assert_read(b, 16, BLOCK_DWORDS - 16, 16, "the stale tail of the full buffer");
    }

    /// (b) glBindBufferRange of 256 bytes at byte 1024 of a 16384-byte buffer:
    /// the block's first 64 dwords are the buffer's 256.., the rest stays 0.
    #[test]
    fn range_256_bytes_inside_a_larger_buffer() {
        let _r = rig();
        let at = block_at();
        big_program();
        bind_range(buffer(BLOCK_DWORDS), 1024, 256);
        let d = draw();
        end_frame();
        let b = block_of(&d, at);
        assert_read(b, 0, 64, 256, "256-byte range at 1024");
        assert_zero_from(b, 64, "256-byte range at 1024");
    }

    /// A 16384-byte range from byte 16000 of a 16384-byte buffer (legal at the
    /// bind: ES 3.0 leaves the size to the draw): clamped to the 384 bytes left.
    #[test]
    fn range_past_the_end_of_the_buffer_is_clamped() {
        let _r = rig();
        let at = block_at();
        big_program();
        bind_range(buffer(BLOCK_DWORDS), 16000, 16384);
        let d = draw();
        end_frame();
        let b = block_of(&d, at);
        assert_read(b, 0, 96, 4000, "the 384 bytes from 16000");
        assert_zero_from(b, 96, "the range past the end");
    }

    /// A range whose offset lies past the end of the buffer reads nothing.
    #[test]
    fn range_offset_past_the_end_reads_nothing() {
        let _r = rig();
        let at = block_at();
        big_program();
        bind_range(buffer(16), 1024, 256);
        let d = draw();
        end_frame();
        assert_zero_from(block_of(&d, at), 0, "offset past the end");
    }

    /// The buffer redefined to 64 bytes after a full-range bind: the draw reads
    /// the buffer as it is now.
    #[test]
    fn buffer_shrunk_after_the_bind() {
        let _r = rig();
        let at = block_at();
        big_program();
        bind_range(buffer(BLOCK_DWORDS), 0, 16384);
        let src: Vec<f32> = (0..16).map(value).collect();
        // SAFETY: plain GL calls; the source is 64 bytes.
        unsafe {
            glBufferData(GL_UNIFORM_BUFFER, 64, src.as_ptr() as *const c_void, GL_STATIC_DRAW);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        let d = draw();
        end_frame();
        let b = block_of(&d, at);
        assert_read(b, 0, 16, 0, "the shrunk buffer");
        assert_zero_from(b, 16, "the shrunk buffer");
    }

    /// (c) Nothing bound at the block's binding point: the draw has no error
    /// and the block stays 0.
    #[test]
    fn no_buffer_bound() {
        let _r = rig();
        let at = block_at();
        big_program();
        let d = draw();
        end_frame();
        assert_zero_from(block_of(&d, at), 0, "no buffer bound");
    }

    /// A buffer name without a data store at the binding point: the same.
    #[test]
    fn buffer_without_data_store() {
        let _r = rig();
        let at = block_at();
        big_program();
        let mut buf = 0;
        // SAFETY: a plain GL call.
        unsafe { glGenBuffers(1, &mut buf) };
        bind_base(buf);
        let d = draw();
        end_frame();
        assert_zero_from(block_of(&d, at), 0, "no data store");
    }

    /// The binding point cleared after a full draw: the next draw has no error
    /// and the block keeps the earlier values.
    #[test]
    fn unbound_after_a_full_draw_keeps_the_values() {
        let _r = rig();
        let at = block_at();
        big_program();
        bind_base(buffer(BLOCK_DWORDS));
        let _ = draw();
        bind_base(0);
        let d = draw();
        end_frame();
        assert_read(block_of(&d, at), 0, BLOCK_DWORDS, 0, "the earlier full buffer");
    }
}
