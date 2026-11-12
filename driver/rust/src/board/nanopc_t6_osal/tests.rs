// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The OSAL over the Z4 MCU model (nanopc_t6_submit's `Z4`: Z3's T6 with
//! the synthetic firmware, and the firmware's side of CSG 0 / CS 0, the
//! ring and the sync object), first op by op, then with the GL stack on
//! top: the L2 triangle (`the_l2_triangle_draws_through_the_osal`).

use super::*;
use crate::board::nanopc_t6_fw::tests::{backings, B, TABLES_PA};
use crate::board::nanopc_t6_submit::tests::{VM_TABLES_PA, Z4};
use crate::board::nanopc_t6_submit::{BLUE, RED, STEP_NONE};
use crate::gpu::mali::csf::RK3588_TIMER_HZ;
use crate::gpu::mali::fw::synth;
use crate::gpu::mali::pgtable::ENTRIES;
use crate::gpu::mali::sched::job::{mov48, sync_add64};
use crate::gpu::mali::sched::RING_MIN_SIZE;

/// The carve-out of the tests: 16 MiB, the backend's two frame slots
/// (about 1.8 MiB each) and the GL objects with room to spare.
const CARVE: u64 = 16 << 20;

/// The model, its OSAL and the table pools both live on.
struct Rig {
    z: Box<Z4>,
    osal: Box<Osal>,
    o: SubmitUp,
    _pool: Vec<Table>,
    _vm_pool: Vec<Table>,
}

fn rig() -> Rig {
    let blob = synth::small();
    let z = Box::new(Z4::with_regions(
        blob.clone(),
        &[
            (QUEUE_VA, QUEUE_VM_BYTES),
            (HEAP_POOL_VA, HEAP_POOL_SIZE),
            (HEAP_CHUNK_VA, u64::from(HEAP_CHUNK_SIZE)),
            (CARVE_VA, CARVE),
        ],
    ));
    let vm = z.backings();
    let mut pool = vec![[0u64; ENTRIES]; 16];
    let mut vm_pool = vec![[0u64; ENTRIES]; 32];
    let mut osal = Box::new(Osal::EMPTY);
    let mut o = SubmitUp::default();
    let mem = OsalMem {
        queue: crate::board::nanopc_t6_submit::tests::QUEUE,
        ring: vm[0],
        heap_pool: vm[1],
        heap_chunks: vm[2],
        carve: vm[3],
        vm_pool: &mut vm_pool,
        vm_tables_pa: VM_TABLES_PA,
    };
    let r = osal.up(
        &*z,
        B,
        &blob,
        &backings(&blob),
        &mut pool,
        TABLES_PA,
        RK3588_TIMER_HZ,
        mem,
        &|a, n| z.t6.clean(a, n),
        &mut |_| {},
        &mut |_| {},
        &mut GpuUp::default(),
        &mut o,
    );
    assert_eq!((r, o.step, o.refusal, o.detail), (Ok(()), STEP_NONE, 0, 0));
    Rig { z, osal, o, _pool: pool, _vm_pool: vm_pool }
}

fn no_delay(_: u32) {}

/// The group comes up as Z4's does, over the four regions, and the two
/// queries answer the device and the heap.
#[test]
fn the_osal_comes_up_and_answers_the_two_queries() {
    let r = rig();
    assert!(r.osal.is_up());
    // the group's start as Z4's record has it (nanopc_t6_submit's test)
    assert_eq!((r.o.csg_req, r.o.csg_ack, r.o.cs_req, r.o.cs_ack), (0x11, 0x11, 0x0511, 0x0011));
    // 4 KiB pages only: L0, the L1 of the low 512 GiB, the queue's L2 and
    // L3; the heap's L1, L2 and the pool's L3, the chunk's L3; the
    // carve-out's L2 and its eight L3s (16 MiB)
    assert_eq!(r.o.vm_tables_used, 17);
    let g = r.osal.gpu_info().unwrap();
    assert_eq!(g.gpu_id, crate::gpu::mali::G610_GPU_ID);
    // the G610's 96 registers of which the firmware keeps 92 (csf.rs's test)
    assert_eq!((g.cs_reg_count, g.unpreserved_cs_reg_count), (96, 4));
    let h = r.osal.heap().unwrap();
    assert_eq!((h.ctx_gpu, h.first_chunk_gpu, h.chunk_size), (HEAP_POOL_VA, HEAP_CHUNK_VA, HEAP_CHUNK_SIZE));
    assert_eq!(r.osal.fences(), (0, 0));
}

/// Pages come first fit from the carve-out's start; a free is held until
/// the kick being recorded completes, and its pages are taken again only
/// then.
#[test]
fn allocations_are_carve_pages_and_a_free_waits_for_the_kick_being_recorded() {
    let mut r = rig();
    let a = r.osal.alloc(100).unwrap();
    assert_eq!((a.gpu, a.size, a.handle), (CARVE_VA, 4096, 1));
    let b = r.osal.alloc(5000).unwrap();
    assert_eq!((b.gpu, b.size, b.handle), (CARVE_VA + 0x1000, 8192, 2));
    assert_eq!((b.cpu - a.cpu, b.phys - a.phys), (0x1000, 0x1000));
    assert_eq!(r.osal.alloc(0), Err(-EINVAL));
    assert_eq!(r.osal.alloc(CARVE as usize + 1), Err(-ENOMEM));

    // freed while kick 1 is being recorded: held
    assert_eq!(r.osal.free(&*r.z, a.gpu, a.handle), 0);
    assert_eq!(r.osal.free(&*r.z, a.gpu, a.handle), -EINVAL, "a second free of the same pages");
    assert_eq!(r.osal.free(&*r.z, b.gpu + 0x1000, 1), -EINVAL, "not an allocation's first page");
    assert_eq!(r.osal.held(), 1);
    let c = r.osal.alloc(1).unwrap();
    assert_eq!(c.gpu, CARVE_VA + 0x3000, "page 0 is still held");

    // kick 1 recorded and completed: the pages go back
    let s = stream(&mut r);
    let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    assert_eq!(f, 1);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), 0);
    assert_eq!(r.osal.held(), 0);
    assert_eq!(r.osal.alloc(1).unwrap().gpu, CARVE_VA);
    assert_eq!(r.osal.live_pages, 5);
}

/// A 24-byte stream in the carve-out that adds 1 to the word 32 bytes from
/// its start: `MOV48 d0, word`, `MOV48 d2, 1`, `SYNC_ADD64 d0, d2`.
fn stream(r: &mut Rig) -> Alloc {
    let s = r.osal.alloc(64).unwrap();
    let word = s.gpu + 32;
    for (i, w) in [mov48(0, word), mov48(2, 1), sync_add64(0, 2), 0].iter().enumerate() {
        // SAFETY: the allocation's CPU view, 4 KiB, the model's RAM
        unsafe { ((s.cpu + 8 * i) as *mut u64).write_volatile(*w) };
    }
    s
}

fn word(s: &Alloc) -> u64 {
    // SAFETY: as in `stream`
    unsafe { ((s.cpu + 32) as *const u64).read_volatile() }
}

/// Forty kicks, more than the 4 KiB ring holds (32 jobs of 128 bytes): the
/// ring wraps, a full ring waits for its oldest job, every fence completes
/// in order, each kick's stream ran once. The model's firmware publishes
/// `extract` as the board's does, late (`PUBLISH_LAG`).
#[test]
fn kicks_run_on_the_ring_and_their_fences_complete() {
    forty_kicks(rig());
}

/// The same forty kicks while the firmware never publishes `extract`: the
/// ring's room comes from the sync object alone.
#[test]
fn a_full_ring_does_not_wait_for_extract() {
    let mut r = rig();
    r.z.publish_lag = Some(u32::MAX);
    let r = forty_kicks(r);
    assert_eq!(r.z.output_extract(), 0, "extract was never published");
}

fn forty_kicks(mut r: Rig) -> Rig {
    let s = stream(&mut r);
    let jobs = RING_MIN_SIZE / 128;
    assert_eq!(jobs, 32);
    for k in 1..=40 {
        assert_eq!(r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay), k);
    }
    let (sub, done) = r.osal.fences();
    assert_eq!(sub, 40);
    assert!(done >= 40 - u64::from(jobs), "the full ring waited: {done}");
    assert_eq!(r.osal.wait(&*r.z, 40, -1, &mut no_delay), 0);
    assert_eq!(r.osal.completed(&*r.z), 40);
    assert_eq!((word(&s), r.z.calls()), (40, 40));
    assert_eq!(r.osal.wait(&*r.z, 7, 0, &mut no_delay), 0, "an older fence");
    assert_eq!(r.osal.wait(&*r.z, 41, -1, &mut no_delay), -EINVAL, "never submitted");
    assert_eq!(r.osal.dead(), None);
    r
}

/// A wait shorter than the job takes returns 1 and leaves the job running;
/// the next wait sees it complete.
#[test]
fn a_short_wait_times_out_and_leaves_the_job_running() {
    let mut r = rig();
    let s = stream(&mut r);
    let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, 0, &mut no_delay), 1);
    assert_eq!(r.osal.completed(&*r.z), 0);
    assert_eq!(r.osal.dead(), None);
    assert_eq!(r.osal.wait(&*r.z, f, 1000, &mut no_delay), 0);
    assert_eq!((r.osal.completed(&*r.z), word(&s)), (1, 1));
}

/// A stream outside the carve-out, misaligned or of no whole words is
/// refused before the ring is touched; the OSAL goes on.
#[test]
fn a_kick_the_carve_out_does_not_hold_is_refused() {
    let mut r = rig();
    let s = stream(&mut r);
    let e = -i64::from(EINVAL);
    assert_eq!(r.osal.submit(&*r.z, HEAP_CHUNK_VA, 24, &mut no_delay), e);
    assert_eq!(r.osal.submit(&*r.z, s.gpu + 8, 24, &mut no_delay), e);
    assert_eq!(r.osal.submit(&*r.z, s.gpu, 0, &mut no_delay), e);
    assert_eq!(r.osal.submit(&*r.z, s.gpu, 20, &mut no_delay), e);
    assert_eq!(r.osal.submit(&*r.z, CARVE_VA + CARVE - 64, 128, &mut no_delay), e);
    assert_eq!(r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay), 1);
}

/// `map_phys` of the carve-out's own pages answers them where they are.
#[test]
fn a_window_is_the_carve_out_s_own_pages() {
    let mut r = rig();
    let a = r.osal.alloc(3 * 4096).unwrap();
    let w = r.osal.window(a.phys + 0x100, 0x2000).unwrap();
    assert_eq!((w.gpu, w.cpu, w.handle), (a.gpu + 0x100, a.cpu + 0x100, 0));
    assert_eq!(r.osal.window(a.phys - 1, 16), Err(-ENOTSUP));
    assert_eq!(r.osal.window(a.phys + CARVE - 8, 16), Err(-ENOTSUP));
    assert_eq!(r.osal.window(a.phys, 0), Err(-EINVAL));
}

/// The sections one after the other from page boundaries, in the image's
/// order; an arena too small for them is no placement.
#[test]
fn the_sections_are_placed_page_by_page_in_the_arena() {
    let blob = synth::small();
    let img = fw::parse(&blob).unwrap();
    let arena = Backing { pa: 0x2000_0000, cpu: 0x7000_0000, size: 1 << 20 };
    let (b, n) = place_sections(&blob, arena).unwrap();
    assert_eq!(n, img.sections().len());
    let mut at = 0;
    for (b, s) in b[..n].iter().zip(img.sections()) {
        assert_eq!((b.pa, b.cpu, b.size), (arena.pa + at, arena.cpu + at as usize, u64::from(s.va_size())));
        at += u64::from(s.va_size()).next_multiple_of(PAGE);
    }
    assert!(place_sections(&blob, Backing { size: at - 1, ..arena }).is_none());
    assert!(place_sections(b"not an image", arena).is_none());
}

mod gl {
    //! The L2 triangle (driver/docs/nanopc-t6-mali.md; panthor-gl's
    //! default program): EGL, a 64x64 pbuffer and an ES 2 context, an FBO
    //! on a 64x64 RGBA8 texture, cleared blue, tri.vert / tri.frag drawn
    //! over the left half, glReadPixels. The library's ops are the OSAL's
    //! methods over the model, as zephyr_mali.c's are the same methods
    //! through the C entries.

    use super::*;
    use core::ffi::c_void;
    use std::sync::atomic::{AtomicPtr, Ordering};
    use yakogl::gl::consts::*;
    use yakogl::gl::draw::glDrawArrays;
    use yakogl::gl::egl::{
        eglChooseConfig, eglCreateContext, eglCreatePbufferSurface, eglGetDisplay, eglInitialize, eglMakeCurrent,
    };
    use yakogl::gl::egl_consts::*;
    use yakogl::gl::framebuffer::glReadPixels;
    use yakogl::gl::framebuffer::{
        glBindFramebuffer, glCheckFramebufferStatus, glFramebufferTexture2D, glGenFramebuffers,
    };
    use yakogl::gl::program::{glAttachShader, glBindAttribLocation, glCreateProgram, glLinkProgram, glUseProgram};
    use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
    use yakogl::gl::state::{glClear, glClearColor, glFinish, glGetError, glViewport};
    use yakogl::gl::texture::{glBindTexture, glGenTextures, glTexImage2D};
    use yakogl::gl::types::*;
    use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
    use yakogl::osal::mali::{OsalMaliKick, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
    use yakogl::osal::{OsalGpuMem, OsalOps};

    const TRI_VS: &str = include_str!("../../../../../tools/mali/testdata/tri.vert");
    const TRI_FS: &str = include_str!("../../../../../tools/mali/testdata/tri.frag");
    /// Two triangles over the left half (tools/mali/panthor/src/l1.rs's
    /// `VERTICES`).
    const VERTICES: [f32; 12] = [-1.0, -1.0, 0.0, -1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 1.0, -1.0, 1.0];

    /// The rig and every allocation's (GPU VA, bytes), in order.
    struct Gl {
        r: Rig,
        allocs: Vec<(u64, usize)>,
    }

    static GL: AtomicPtr<Gl> = AtomicPtr::new(core::ptr::null_mut());

    fn with<R>(f: impl FnOnce(&mut Gl) -> R) -> R {
        let p = GL.load(Ordering::Acquire);
        assert!(!p.is_null(), "the rig is installed");
        // SAFETY: the test's Gl, alive while installed; the library calls
        // the ops from the test's thread only, one at a time
        f(unsafe { &mut *p })
    }

    fn fill(out: *mut OsalGpuMem, a: &Alloc) {
        // SAFETY: the library's out-parameter
        unsafe {
            *out = OsalGpuMem {
                cpu: a.cpu as *mut c_void,
                gpu: a.gpu,
                phys: a.phys,
                size: a.size,
                impl_: a.handle as *mut c_void,
            }
        };
    }

    unsafe extern "C" fn o_init(_: *mut c_void) -> i32 {
        use yakogl::shaderc::online::store;
        static LENT: std::sync::Once = std::sync::Once::new();
        LENT.call_once(|| {
            let scratch: &'static mut [u8] = Box::leak(vec![0u8; store::SCRATCH_BYTES].into_boxed_slice());
            let keep: &'static mut [u64] = Box::leak(vec![0u64; store::KEEP_BYTES / 8].into_boxed_slice());
            // SAFETY: both leaked, so they outlive every compile
            let rc = unsafe {
                (
                    store::shc_online_lend_scratch(scratch.as_mut_ptr(), scratch.len()),
                    store::shc_online_lend_keep(keep.as_mut_ptr().cast(), store::KEEP_BYTES),
                )
            };
            assert_eq!(rc, (0, 0));
        });
        0
    }

    unsafe extern "C" fn o_alloc(_: u64, size: usize, _: u32, out: *mut OsalGpuMem) -> i32 {
        with(|g| match g.r.osal.alloc(size) {
            Ok(a) => {
                g.allocs.push((a.gpu, size));
                fill(out, &a);
                0
            }
            Err(e) => e,
        })
    }

    unsafe extern "C" fn o_free(mem: *mut OsalGpuMem) -> i32 {
        // SAFETY: the library's descriptor
        let m = unsafe { &mut *mem };
        if m.impl_.is_null() {
            return 0;
        }
        let rc = with(|g| g.r.osal.free(&*g.r.z, m.gpu, m.impl_ as usize));
        *m = OsalGpuMem::ZERO;
        rc
    }

    unsafe extern "C" fn o_map_phys(_: u64, phys: u64, size: usize, _: u32, out: *mut OsalGpuMem) -> i32 {
        with(|g| match g.r.osal.window(phys, size) {
            Ok(a) => {
                fill(out, &Alloc { handle: 0, ..a });
                0
            }
            Err(e) => e,
        })
    }

    unsafe extern "C" fn o_cache(_: *mut c_void, _: usize) {}

    unsafe extern "C" fn o_submit(kick: *const c_void, len: usize) -> i64 {
        if kick.is_null() || len != size_of::<OsalMaliKick>() {
            return -i64::from(EINVAL);
        }
        // SAFETY: the backend's kick of the size checked
        let k = unsafe { *kick.cast::<OsalMaliKick>() };
        with(|g| g.r.osal.submit(&*g.r.z, k.stream_gpu, k.stream_bytes, &mut no_delay))
    }

    unsafe extern "C" fn o_wait(fence: i64, timeout_us: i64) -> i32 {
        with(|g| g.r.osal.wait(&*g.r.z, fence, timeout_us, &mut no_delay))
    }

    unsafe extern "C" fn o_completed() -> i64 {
        with(|g| g.r.osal.completed(&*g.r.z))
    }

    unsafe extern "C" fn o_query(what: u32, out: *mut c_void, sz: usize) -> i32 {
        with(|g| {
            // SAFETY: the caller's out-parameter of `sz` bytes, checked
            // against each answer's size
            unsafe {
                match what {
                    OSAL_QUERY_MALI_GPU if sz == size_of::<OsalMaliGpu>() => {
                        *out.cast::<OsalMaliGpu>() = g.r.osal.gpu_info().unwrap()
                    }
                    OSAL_QUERY_MALI_TILER_HEAP if sz == size_of::<OsalMaliHeap>() => {
                        *out.cast::<OsalMaliHeap>() = g.r.osal.heap().unwrap()
                    }
                    _ => return -ENOTSUP,
                }
            }
            0
        })
    }

    /// The log hook: the two arguments `osal::log_str` passes (`"%s"`,
    /// line), printed so a failing run shows the library's lines.
    extern "C" fn o_log(_fmt: *const core::ffi::c_char, line: *const core::ffi::c_char) {
        if !line.is_null() {
            // SAFETY: osal::log_str passes a NUL-terminated line
            println!("yakogl: {}", unsafe { std::ffi::CStr::from_ptr(line) }.to_string_lossy());
        }
    }

    /* `log` is a C variadic function, which stable Rust cannot define: a
     * one-instruction trampoline declared as the variadic function it is
     * tail-jumps to o_log with the argument registers untouched (the
     * leading pointer arguments are passed as in a non-variadic call on
     * x86-64 System V and AArch64 AAPCS64), as tools/mali/panthor's OSAL
     * and the library's own test hook do. */
    #[cfg(target_arch = "x86_64")]
    core::arch::global_asm!(
        ".pushsection .text.yd_osal_test_log,\"ax\",@progbits",
        ".globl yd_osal_test_log",
        ".hidden yd_osal_test_log",
        "yd_osal_test_log:",
        "jmp {log}",
        ".popsection",
        log = sym o_log,
    );
    #[cfg(target_arch = "aarch64")]
    core::arch::global_asm!(
        ".pushsection .text.yd_osal_test_log,\"ax\",%progbits",
        ".globl yd_osal_test_log",
        ".hidden yd_osal_test_log",
        "yd_osal_test_log:",
        "b {log}",
        ".popsection",
        log = sym o_log,
    );
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    extern "C" {
        fn yd_osal_test_log(fmt: *const core::ffi::c_char, ...);
    }
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    const LOG: Option<unsafe extern "C" fn(*const core::ffi::c_char, ...)> = Some(yd_osal_test_log);
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    const LOG: Option<unsafe extern "C" fn(*const core::ffi::c_char, ...)> = None;

    static OPS: OsalOps = OsalOps {
        init: Some(o_init),
        shutdown: None,
        gpu_alloc: Some(o_alloc),
        gpu_free: Some(o_free),
        mmu_map: None,
        map_phys: Some(o_map_phys),
        gpu_write: None,
        gpu_read: None,
        cache_flush: Some(o_cache),
        cache_invd: Some(o_cache),
        submit: Some(o_submit),
        wait: Some(o_wait),
        completed: Some(o_completed),
        reg_read: None,
        reg_write: None,
        query: Some(o_query),
        log: LOG,
        wait_vblank: None,
        present: None,
        lock: None,
        unlock: None,
        thread_id: None,
        display_busy: None,
    };

    /// A compiled shader of `src`.
    fn shader(kind: GLenum, src: &str) -> GLuint {
        // SAFETY: plain GL calls on the test's thread, with a current
        // context; the pointers live across the calls
        unsafe {
            let s = glCreateShader(kind);
            let p = src.as_ptr().cast::<GLchar>();
            let n = src.len() as GLint;
            glShaderSource(s, 1, &p, &n);
            glCompileShader(s);
            let mut ok = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "compile of {src:?}");
            s
        }
    }

    /// The L2 program through the OSAL over the model. The model runs each
    /// job the ring holds: the job's `CALL` runs the backend's stream at
    /// its VA in the carve-out, its `SYNC_ADD64` completes the fence, and
    /// after a `CALL` the model paints the texture as the triangle would
    /// (red left half, blue right): the pixels are the model's stand-in, the
    /// path to them is the OSAL's (the board run checks the GPU's own).
    #[test]
    fn the_l2_triangle_draws_through_the_osal() {
        let mut g = Box::new(Gl { r: rig(), allocs: vec![] });
        GL.store(&mut *g, Ordering::Release);
        let (lost0, refused0) = (yakogl::backend::yakogl_gpu_work_lost(), yakogl::backend::yakogl_draws_refused());
        // SAFETY: OPS is a static; plain EGL and GL calls on this thread
        unsafe {
            yakogl::osal::osal_set_ops(&OPS);
            let dpy = eglGetDisplay(core::ptr::null_mut());
            assert_ne!(eglInitialize(dpy, core::ptr::null_mut(), core::ptr::null_mut()), 0);
            let init_kicks = g.r.osal.fences().0;
            assert!(init_kicks >= 1, "the backend's queue-init stream ran");
            let cfg_attrs = [EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE];
            let mut cfg: EGLConfig = core::ptr::null_mut();
            let mut n = 0;
            assert!(eglChooseConfig(dpy, cfg_attrs.as_ptr(), &mut cfg, 1, &mut n) != 0 && n == 1);
            let surf = eglCreatePbufferSurface(dpy, cfg, [EGL_WIDTH, 64, EGL_HEIGHT, 64, EGL_NONE].as_ptr());
            let ctx =
                eglCreateContext(dpy, cfg, core::ptr::null_mut(), [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE].as_ptr());
            assert!(!surf.is_null() && !ctx.is_null() && eglMakeCurrent(dpy, surf, surf, ctx) != 0);

            let (mut tex, mut fbo) = (0, 0);
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            let before = g.allocs.len();
            let pattern = vec![0x5au8; 64 * 64 * 4];
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as GLint,
                64,
                64,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                pattern.as_ptr().cast(),
            );
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            /* the texture's storage: the one buffer glTexImage2D took, level 0
             * first, linear, 256-byte rows (its mip levels' rows after) */
            let texels = &g.allocs[before..];
            assert!(texels.len() == 1 && texels[0].1 >= 64 * 64 * 4, "{texels:x?}");
            g.r.z.paint_at(texels[0].0);

            glViewport(0, 0, 64, 64);
            glClearColor(0.0, 0.0, 1.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            let p = glCreateProgram();
            glAttachShader(p, shader(GL_VERTEX_SHADER, TRI_VS));
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, TRI_FS));
            glBindAttribLocation(p, 0, c"pos".as_ptr());
            glLinkProgram(p);
            glUseProgram(p);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, VERTICES.as_ptr().cast());
            glDrawArrays(GL_TRIANGLES, 0, 6);
            let mut px = vec![0u8; 64 * 64 * 4];
            glReadPixels(0, 0, 64, 64, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
            glFinish();
            assert_eq!(glGetError(), GL_NO_ERROR);

            let words: Vec<u32> = px.chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
            let red = words.iter().enumerate().filter(|&(i, &w)| w == RED && i % 64 < 32).count();
            let blue = words.iter().enumerate().filter(|&(i, &w)| w == BLUE && i % 64 >= 32).count();
            assert_eq!((red, blue), (2048, 2048));
            /* the frame went through the ring: one job a kick, each kick's
             * CALL run, every fence complete, nothing lost or refused */
            let (sub, done) = g.r.osal.fences();
            assert!(sub > init_kicks, "the frame was a kick of its own");
            assert_eq!((done, u64::from(g.r.z.calls())), (sub, sub));
            assert_eq!(g.r.osal.dead(), None);
            assert_eq!(yakogl::backend::yakogl_gpu_work_lost(), lost0);
            assert_eq!(yakogl::backend::yakogl_draws_refused(), refused0);
            yakogl::osal::osal_set_ops(core::ptr::null());
        }
        GL.store(core::ptr::null_mut(), Ordering::Release);
    }
}
