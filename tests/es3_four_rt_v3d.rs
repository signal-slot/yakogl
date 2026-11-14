// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Four draw buffers on the V3D host back end (t-four-rt-v3d), through the
//! GL API, the real GL core and the real V3D back end, with the job builder
//! stood in for (the harness of tests/es3_int_rt_v3d.rs `gl_api`) and the
//! tile rule taken from the real driver/src/v3d/v3d_frame.c.
//!
//! `backend::DRAW_TARGETS` is 3 for the V3D (src/backend.rs, RC14), so the
//! GL core reports GL_MAX_DRAW_BUFFERS = GL_MAX_COLOR_ATTACHMENTS = 3 and
//! refuses a fourth attachment before the back end sees it. The V3D side
//! walks up to `GLES_MAX_TARGETS` = 4 (`extra_targets`, `Target::rts`,
//! `V3dJobTarget::rt[3]`, the compiler's `MAX_RTS`). Design piece 4 of
//! docs/notes/v3d-fragment-out.md lists the refusals a four-target draw
//! could reach on the V3D, and what each does today:
//!
//! - "a colour output with no attachment slot": not reached. `rt_mask`
//!   0b1111 against `rts` = 4 passes (`rt_mask >> rts == 0`).
//! - "4x multisampling of several colour attachments": not reached, an FBO
//!   is single-sampled.
//! - "a scissored or masked colour clear of several colour attachments":
//!   gone (v3d-mrt-clear). A glClear under scissor or colour mask on 2-4
//!   attachments is one clear quad whose program writes every attachment
//!   (`src/v3d/clear_mrt.rs`, up to its `MAX_RTS` of four), pinned below.
//! - "colour attachments of different sizes": reached only by attachments of
//!   different sizes (docs/notes/v3d-mrt-sizes.md), not by their count.
//!
//! So nothing in `src/v3d` refuses a four-target draw. What blocks
//! DRAW_TARGETS = 4 for the V3D is the constant itself (src/backend.rs, 3 in
//! its V3D arm, "no board has drawn four").
//! The tests pin the GL core's three-target limit (the build as it is), the
//! job a draw of three attachments makes (rts, the fragment program's
//! rt_mask, the tile size), and the job a four-attachment draw makes when a
//! stand-in reports four (`backend::HOST_DRAW_TARGETS`: what the constant at
//! 4 would do). The build's own four-target result is the `#[ignore]`d
//! expected test.
//!
//! Tile size, Mesa `v3d_choose_tile_size` (third_party/mesa-26.2.1/src/
//! broadcom/common/v3d_util.c, the V3D 4.x branch): 64x64, one table step
//! for 2 render targets, two for 3 or 4, one more per step of the largest
//! internal bpp (32 bpp none, 64 one, 128 two). Four RGBA8 targets are
//! 32x32; with a 128-bpp one among them, 16x16.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_four_rt_v3d
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

mod gl_api {
    use core::ffi::{c_char, c_int, c_void, CStr};
    use std::collections::BTreeMap;
    use std::ffi::CString;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::{Mutex, MutexGuard, Once, OnceLock};
    use yakogl::gl::buffer::glBindBuffer;
    use yakogl::gl::consts::*;
    use yakogl::gl::context;
    use yakogl::gl::draw::*;
    use yakogl::gl::framebuffer::*;
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::state::*;
    use yakogl::gl::types::*;
    use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
    use yakogl::v3d::backend::yakogl_backend_v3d;
    use yakogl::v3d::ffi::*;
    use yakogl::v3d::kick::V3dKick;

    /* ---- the job builder: records, draws nothing ---- */

    #[derive(Default)]
    struct Seen {
        jobs: Vec<V3dJobTarget>,
        clears: Vec<V3dJobClear>,
        /// The vertex count of each draw the job builder was handed.
        draws: Vec<u32>,
        tilings: Vec<Tiling>,
        /// The fragment program colour outputs of each draw (`ShcV3dFs::rt_mask`).
        rt_masks: Vec<u8>,
        log: Vec<String>,
    }

    /// One call of the tile rule: what was asked, the tile it answered.
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Tiling {
        rts: u32,
        msaa: bool,
        max_bpp: u32,
        tile: (u32, u32),
    }

    static SEEN: Mutex<Seen> = Mutex::new(Seen {
        jobs: Vec::new(),
        clears: Vec::new(),
        draws: Vec::new(),
        tilings: Vec::new(),
        rt_masks: Vec::new(),
        log: Vec::new(),
    });

    fn seen<R>(f: impl FnOnce(&mut Seen) -> R) -> R {
        f(&mut SEEN.lock().unwrap_or_else(|e| e.into_inner()))
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
        seen(|s| s.jobs.push(t));
        t.width != 0 && t.height != 0
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
        // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
        let (j, d) = unsafe { (&mut *j, &*d) };
        j.draws += 1;
        // SAFETY: the job's BCL is live; the draw's slices are as long as it says.
        unsafe {
            (*j.bcl).used += 120;
            seen(|s| s.draws.push(d.count));
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
        // SAFETY: the backend passes its live job and clear; its lists are live.
        unsafe {
            seen(|s| s.clears.push(*c));
            (*(*j).bcl).used += 1;
            (*(*j).rcl).used = 200;
        }
        true
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
            let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_four_rt");
            std::fs::create_dir_all(&dir).expect("the scratch directory");
            let so = dir.join(format!("v3d_four_rt-{}.so", std::process::id()));
            /* the compiler's own temporary files next to the object */
            let st = Command::new("cc")
                .current_dir(&root)
                .env("TMPDIR", &dir)
                .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
                .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
                .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
                .arg("-Ithird_party/mesa-26.2.1/include")
                .args([
                    "driver/src/v3d/v3d_tex.c",
                    "driver/src/v3d/v3d_tiling_mesa.c",
                    "driver/src/v3d/v3d_frame.c",
                    "-lm",
                    "-o",
                ])
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
        trampoline!(v3d_frame_alloc_sizes(t: *const V3dTiling, draws: u32, page: u32, ta: *mut u32, ts: *mut u32));
    }

    /// The real tile rule (driver/src/v3d/v3d_frame.c), each call recorded:
    /// the render-target count, MSAA and largest internal bpp the back end
    /// asked for, and the tile it got.
    fn tiling(rts: u32, msaa: bool, max_bpp: u32, t: *const V3dTiling) {
        // SAFETY: the library filled `t` just before.
        let t = unsafe { *t };
        seen(|s| s.tilings.push(Tiling { rts, msaa, max_bpp, tile: (t.tile_w, t.tile_h) }));
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling(width: u32, height: u32, t: *mut V3dTiling) {
        // SAFETY: the C function's signature, filling the backend's out-parameter.
        let f: unsafe extern "C" fn(u32, u32, *mut V3dTiling) = unsafe { std::mem::transmute(sym("v3d_frame_tiling")) };
        // SAFETY: as above.
        unsafe { f(width, height, t) };
        tiling(1, false, 0, t);
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_rts(width: u32, height: u32, rts: u32, t: *mut V3dTiling) {
        // SAFETY: the C function's signature, filling the backend's out-parameter.
        let f: unsafe extern "C" fn(u32, u32, u32, *mut V3dTiling) =
            unsafe { std::mem::transmute(sym("v3d_frame_tiling_rts")) };
        // SAFETY: as above.
        unsafe { f(width, height, rts, t) };
        tiling(rts, false, 0, t);
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_ms(width: u32, height: u32, rts: u32, msaa: bool, t: *mut V3dTiling) {
        // SAFETY: the C function's signature, filling the backend's out-parameter.
        let f: unsafe extern "C" fn(u32, u32, u32, bool, *mut V3dTiling) =
            unsafe { std::mem::transmute(sym("v3d_frame_tiling_ms")) };
        // SAFETY: as above.
        unsafe { f(width, height, rts, msaa, t) };
        tiling(rts, msaa, 0, t);
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_bpp(
        width: u32,
        height: u32,
        rts: u32,
        msaa: bool,
        max_bpp: u32,
        t: *mut V3dTiling,
    ) {
        // SAFETY: the C function's signature, filling the backend's out-parameter.
        let f: unsafe extern "C" fn(u32, u32, u32, bool, u32, *mut V3dTiling) =
            unsafe { std::mem::transmute(sym("v3d_frame_tiling_bpp")) };
        // SAFETY: as above.
        unsafe { f(width, height, rts, msaa, max_bpp, t) };
        tiling(rts, msaa, max_bpp, t);
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
        seen(|x| x.log.push(s));
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

    /// The V3D back end with its draw entry watched: the program's colour
    /// outputs (`rt_mask`) are the compiler's, and no job-builder call
    /// carries them.
    unsafe extern "C" fn spy_draw(d: *const yakogl::backend::GlesDraw) -> i32 {
        // SAFETY: the GL core hands a live draw; its program is live or null.
        unsafe {
            let dd = &*d;
            if dd.index_count != 0 {
                if let Some(fs) = dd.fs.as_ref() {
                    seen(|s| s.rt_masks.push(fs.v3d.fs.rt_mask));
                }
            }
            (yakogl_backend_v3d.draw.expect("the V3D draws"))(d)
        }
    }

    fn spy_backend() -> &'static yakogl::backend::YakoglBackend {
        let b = yakogl::backend::YakoglBackend { draw: Some(spy_draw), ..yakogl_backend_v3d };
        Box::leak(Box::new(b))
    }

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
                yakogl::backend::g_backend = spy_backend();
                assert_eq!(yakogl::backend::init(), 0, "{:?}", seen(|s| s.log.clone()));
                assert!(!context::gl_context_create(32, 32).is_null(), "{:?}", seen(|s| s.log.clone()));
            }
        });
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            while glGetError() != GL_NO_ERROR {}
        }
        seen(|s| *s = Seen::default());
        yakogl::backend::HOST_DRAW_TARGETS.store(0, core::sync::atomic::Ordering::Relaxed);
        g
    }

    /* ---- the framebuffer, the program, the draw ---- */

    /// GL_NO_ERROR, or the first error of the queue (the queue is drained).
    fn err() -> GLenum {
        // SAFETY: plain GL calls.
        unsafe {
            let e = glGetError();
            while glGetError() != GL_NO_ERROR {}
            e
        }
    }

    /// An 8x8 renderbuffer of `format` on colour attachment `point` of the
    /// bound framebuffer; its name.
    fn attach(point: GLenum, format: GLenum) -> GLuint {
        let mut rb = 0;
        // SAFETY: plain GL calls with room for one name.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, format, 8, 8);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb);
        }
        rb
    }

    /// A bound framebuffer of 8x8 renderbuffers of `formats` on colour
    /// attachments 0.., all of them draw buffers; deleted when dropped.
    struct Fbo {
        fb: GLuint,
        rbs: Vec<GLuint>,
    }

    impl Fbo {
        fn new(formats: &[GLenum]) -> Fbo {
            let mut fb = 0;
            // SAFETY: room for one name.
            unsafe {
                glGenFramebuffers(1, &mut fb);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
            }
            let rbs: Vec<GLuint> =
                formats.iter().enumerate().map(|(i, f)| attach(GL_COLOR_ATTACHMENT0 + i as GLenum, *f)).collect();
            let bufs: Vec<GLenum> = (0..formats.len()).map(|i| GL_COLOR_ATTACHMENT0 + i as GLenum).collect();
            // SAFETY: `bufs` has `len` enums.
            unsafe {
                glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
                glViewport(0, 0, 8, 8);
            }
            let log = seen(|s| s.log.clone());
            assert_eq!(err(), GL_NO_ERROR, "{formats:#x?}: {log:?}");
            // SAFETY: plain GL call.
            let st = unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) };
            assert_eq!(st, GL_FRAMEBUFFER_COMPLETE, "{formats:#x?}: {log:?}");
            Fbo { fb, rbs }
        }
    }

    impl Drop for Fbo {
        fn drop(&mut self) {
            // SAFETY: names this test made.
            unsafe {
                glDisable(GL_SCISSOR_TEST);
                glColorMask(1, 1, 1, 1);
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                glDeleteFramebuffers(1, &self.fb);
                glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
                glUseProgram(0);
            }
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

    /// The current program: one `out` per entry of `types` ("vec4", "ivec4",
    /// "uvec4"), `layout(location = i)`.
    fn program(types: &[&str]) {
        let mut fs = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
        for (i, t) in types.iter().enumerate() {
            fs += &format!("layout(location = {i}) out {t} o{i};\n");
        }
        fs += "void main() {\n";
        for (i, t) in types.iter().enumerate() {
            fs += &match *t {
                "vec4" => format!("o{i} = vec4(0.25, 0.5, 0.75, 0.5);\n"),
                "ivec4" => format!("o{i} = ivec4(-1, 2, -3, 4);\n"),
                _ => format!("o{i} = uvec4(1u, 2u, 3u, 4u);\n"),
            };
        }
        fs += "}\n";
        let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
        let f = shader(GL_FRAGMENT_SHADER, &fs);
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1);
            glUseProgram(p);
        }
    }

    /// One triangle over the framebuffer, flushed. Returns what the job
    /// builder recorded.
    fn draw() -> Seen {
        static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
        seen(|s| *s = Seen::default());
        // SAFETY: TRI is static and outlives the draw.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glFinish();
        }
        let log = seen(|s| s.log.clone());
        assert_eq!(err(), GL_NO_ERROR, "{log:?}");
        seen(core::mem::take)
    }

    /// A framebuffer of `formats`, a program writing `outs`, one draw.
    fn run(formats: &[GLenum], outs: &[&str]) -> Seen {
        let fbo = Fbo::new(formats);
        program(outs);
        let s = draw();
        drop(fbo);
        s
    }

    fn get(pname: GLenum) -> GLint {
        let mut v = -1;
        // SAFETY: one GLint.
        unsafe { glGetIntegerv(pname, &mut v) };
        v
    }

    /// The job the draw made: one job, one draw. (rts, rt_mask, the tile the
    /// job asked the tile rule for: (render targets, largest internal bpp)
    /// and the tile it got).
    fn job(s: &Seen) -> (u32, u8, Tiling) {
        assert_eq!((s.jobs.len(), s.draws.clone()), (1, vec![3]), "one job, one draw: {:?}", s.log);
        let t = *s.tilings.last().expect("the job asked for its tile size");
        (s.jobs[0].rts, *s.rt_masks.last().expect("a draw"), t)
    }

    fn tile(rts: u32, max_bpp: u32, tile: (u32, u32)) -> Tiling {
        Tiling { rts, msaa: false, max_bpp, tile }
    }

    /// What `GLES_MAX_TARGETS` leaves of the build's own limit: the stand-in
    /// count is off.
    fn as_built() {
        yakogl::backend::HOST_DRAW_TARGETS.store(0, core::sync::atomic::Ordering::Relaxed);
    }

    /// A back end that writes `n` targets in one draw
    /// (`backend::HOST_DRAW_TARGETS`): what DRAW_TARGETS = n would be.
    fn stand_in(n: u8) {
        yakogl::backend::HOST_DRAW_TARGETS.store(n, core::sync::atomic::Ordering::Relaxed);
    }

    const RGBA8: GLenum = GL_RGBA8;
    const MIXED: [GLenum; 3] = [GL_RGBA8, GL_RG16I, GL_RGBA16F];

    /* ---- the limits, as the build is ---- */

    /// GL_MAX_DRAW_BUFFERS and GL_MAX_COLOR_ATTACHMENTS are
    /// `backend::DRAW_TARGETS`: 3 for the V3D (RC14).
    #[test]
    fn the_limits_are_three_today() {
        let _g = rig();
        as_built();
        assert_eq!(yakogl::backend::DRAW_TARGETS, 3);
        assert_eq!((get(GL_MAX_DRAW_BUFFERS), get(GL_MAX_COLOR_ATTACHMENTS)), (3, 3));
        assert_eq!(err(), GL_NO_ERROR);
    }

    /// A fourth attachment never reaches the back end: the GL core refuses
    /// GL_COLOR_ATTACHMENT3 (framebuffer.rs `is_color_point`), glDrawBuffers
    /// of four names is past GL_MAX_DRAW_BUFFERS, and the framebuffer stays
    /// three-attachment. dEQP fragment_out.random's four-target seeds stop at
    /// this (GL_INVALID_ENUM at "After framebuffer setup", docs/notes/
    /// v3d-fragment-out.md G5).
    #[test]
    fn a_fourth_attachment_is_refused_by_the_gl_core_today() {
        let _g = rig();
        as_built();
        let fbo = Fbo::new(&[RGBA8, RGBA8, RGBA8]);
        let rb = attach(GL_COLOR_ATTACHMENT0 + 3, RGBA8);
        assert_eq!(err(), GL_INVALID_ENUM, "glFramebufferRenderbuffer(GL_COLOR_ATTACHMENT3)");
        let bufs: [GLenum; 4] =
            [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT0 + 1, GL_COLOR_ATTACHMENT0 + 2, GL_COLOR_ATTACHMENT0 + 3];
        // SAFETY: four enums.
        unsafe { glDrawBuffers(4, bufs.as_ptr()) };
        assert_eq!(err(), GL_INVALID_VALUE, "glDrawBuffers(4, ..): past GL_MAX_DRAW_BUFFERS");
        // SAFETY: a name this test made.
        unsafe { glDeleteRenderbuffers(1, &rb) };
        drop(fbo);
    }

    /* ---- three attachments: the job as it is ---- */

    /// Three RGBA8 attachments, a program writing o0..o2: the job configures
    /// three render targets, the program's rt_mask is 0b111, and the tile
    /// rule is asked for 3 targets at 32 bpp: 32x32 (idx 2).
    #[test]
    fn three_rgba8_targets_make_rts_3_mask_111_tile_32x32() {
        let _g = rig();
        as_built();
        let s = run(&[RGBA8; 3], &["vec4"; 3]);
        assert_eq!(job(&s), (3, 0b111, tile(3, 0, (32, 32))));
    }

    /// {RGBA8, RG16I, RGBA16F}: the largest internal bpp is 64 (RGBA16F), one
    /// table step further down: 32x16 (idx 3). The integer output is its own
    /// class in the same program.
    #[test]
    fn three_mixed_targets_make_the_64_bpp_tile() {
        let _g = rig();
        as_built();
        let s = run(&MIXED, &["vec4", "ivec4", "vec4"]);
        assert_eq!(job(&s), (3, 0b111, tile(3, 1, (32, 16))));
        assert!(s.jobs[0].typed, "ES 3.0 formats: a typed job");
    }

    /// A 128-bpp target (RGBA32UI) among three: two steps for the bpp, 16x16
    /// (idx 4).
    #[test]
    fn three_targets_with_a_128_bpp_one_make_a_16x16_tile() {
        let _g = rig();
        as_built();
        let s = run(&[RGBA8, GL_RGBA32UI, RGBA8], &["vec4", "uvec4", "vec4"]);
        assert_eq!(job(&s), (3, 0b111, tile(3, 2, (16, 16))));
    }

    /// A draw writing one output of three attachments: the job still
    /// configures the framebuffer's three targets; rt_mask is the program's.
    #[test]
    fn a_program_writing_one_output_keeps_three_rts() {
        let _g = rig();
        as_built();
        let s = run(&[RGBA8; 3], &["vec4"]);
        assert_eq!(job(&s), (3, 0b001, tile(3, 0, (32, 32))));
    }

    /* ---- four attachments: a stand-in back end that reports four ---- */

    /// With `HOST_DRAW_TARGETS` = 4 (DRAW_TARGETS = 4 for the V3D) the GL core
    /// reports four and takes a fourth attachment, and nothing in `src/v3d`
    /// refuses the draw: rts 4 (the job builder has RT 1..3 slots), rt_mask
    /// 0b1111 against four slots, and the 4-RT tile of Mesa's rule (idx 2, as
    /// three targets): 32x32 for 32 bpp.
    #[test]
    fn a_stand_in_back_end_of_four_draws_four_targets() {
        let _g = rig();
        stand_in(4);
        assert_eq!((get(GL_MAX_DRAW_BUFFERS), get(GL_MAX_COLOR_ATTACHMENTS)), (4, 4));
        let s = run(&[RGBA8; 4], &["vec4"; 4]);
        let j = job(&s);
        as_built();
        assert_eq!(j, (4, 0b1111, tile(4, 0, (32, 32))));
        assert_eq!(s.jobs[0].rt.map(|r| r.address != 0), [true; 3], "RT 1..3 have their addresses");
    }

    /// The same with the other classes in the four: {RGBA8, RG16I, RGBA16F,
    /// RGBA32UI} is 128 bpp at the top, 16x16, and the program writes the
    /// four with their own classes.
    #[test]
    fn a_stand_in_back_end_of_four_takes_mixed_classes() {
        let _g = rig();
        stand_in(4);
        let s = run(&[RGBA8, GL_RG16I, GL_RGBA16F, GL_RGBA32UI], &["vec4", "ivec4", "vec4", "uvec4"]);
        let j = job(&s);
        as_built();
        assert_eq!(j, (4, 0b1111, tile(4, 2, (16, 16))));
        assert_eq!(s.jobs[0].rt_format[..4].iter().filter(|f| **f != 0).count(), 4, "four typed render targets");
    }

    /// A four-attachment draw writing only o3 (and o0): the slots are there,
    /// the mask is 0b1001.
    #[test]
    fn a_stand_in_back_end_of_four_with_a_sparse_mask() {
        let _g = rig();
        stand_in(4);
        let fbo = Fbo::new(&[RGBA8; 4]);
        let fs = "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 a;\n\
                  layout(location = 3) out vec4 b;\nvoid main() { a = vec4(1.0); b = vec4(0.5); }\n";
        let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
        let f = shader(GL_FRAGMENT_SHADER, fs);
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glLinkProgram(p);
            glUseProgram(p);
        }
        let s = draw();
        drop(fbo);
        let j = job(&s);
        as_built();
        assert_eq!(j, (4, 0b1001, tile(4, 0, (32, 32))));
    }

    /* ---- the several-attachment clear a scissor makes a quad ---- */

    /// glClear under glScissor on three (or, with the stand-in, four)
    /// attachments is one clear quad in one job that loads every attachment
    /// (the TLB clears none): no refusal, no error, whatever the count
    /// (tests/es3_mrt_clear_v3d.rs holds the mixed-class cases).
    #[test]
    fn a_scissored_clear_of_several_attachments_is_one_quad() {
        let _g = rig();
        for n in [3u8, 4] {
            stand_in(n);
            let fbo = Fbo::new(&vec![RGBA8; n as usize]);
            seen(|s| *s = Seen::default());
            let base = yakogl::v3d::backend::v3d_refused_draws();
            // SAFETY: plain GL calls.
            unsafe {
                glEnable(GL_SCISSOR_TEST);
                glScissor(1, 1, 4, 4);
                glClear(GL_COLOR_BUFFER_BIT);
                glFinish();
                glDisable(GL_SCISSOR_TEST);
            }
            let s = seen(core::mem::take);
            drop(fbo);
            as_built();
            assert_eq!(err(), GL_NO_ERROR, "{n} attachments: {:?}", s.log);
            assert_eq!(yakogl::v3d::backend::v3d_refused_draws().wrapping_sub(base), 0, "{n} attachments: {:?}", s.log);
            assert_eq!(
                (s.jobs.len(), s.draws.clone()),
                (1, vec![6]),
                "{n} attachments: one job, the quad: {:?}",
                s.log
            );
            assert_eq!(s.jobs[0].rts, u32::from(n), "{n} attachments");
            let c = &s.clears[0];
            assert!(c.load_rt0 && c.load_rt[..n as usize - 1].iter().all(|l| *l), "{n} attachments: all loaded: {c:?}");
        }
    }

    /* ---- the build's own four: the expected result ---- */

    /// What DRAW_TARGETS = 4 for the V3D must give with no stand-in: the
    /// limits 4, a fourth attachment accepted, and the draw of the job above
    /// (rts 4, rt_mask 0b1111, the 4-RT tile 32x32).
    #[test]
    #[ignore = "DRAW_TARGETS is 3 for the V3D (src/backend.rs, RC14); plan item for src/backend.rs (docs/notes/v3d-fragment-out.md piece 4)"]
    fn four_attachments_in_the_build() {
        let _g = rig();
        as_built();
        assert_eq!((get(GL_MAX_DRAW_BUFFERS), get(GL_MAX_COLOR_ATTACHMENTS)), (4, 4));
        let s = run(&[RGBA8; 4], &["vec4"; 4]);
        assert_eq!(job(&s), (4, 0b1111, tile(4, 0, (32, 32))));
    }

    /// The same with mixed classes and a 128-bpp target: the 16x16 tile.
    #[test]
    #[ignore = "DRAW_TARGETS is 3 for the V3D (src/backend.rs, RC14); plan item for src/backend.rs (docs/notes/v3d-fragment-out.md piece 4)"]
    fn four_mixed_attachments_in_the_build() {
        let _g = rig();
        as_built();
        let s = run(&[RGBA8, GL_RG16I, GL_RGBA16F, GL_RGBA32UI], &["vec4", "ivec4", "vec4", "uvec4"]);
        assert_eq!(job(&s), (4, 0b1111, tile(4, 2, (16, 16))));
    }
}
