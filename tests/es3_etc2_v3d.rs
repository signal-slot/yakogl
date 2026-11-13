// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ETC2/EAC on the V3D 4.2 through the GL API (plan #354 item 4, design
//! docs/notes/etc2-eac.md): the GL core over the real V3D back end, the job
//! builder faked (tests/es3_formats_v3d.rs' `gl_rig`).
//!
//! * The four formats the TMU reads as blocks (RGB8_ETC2, SRGB8_ETC2,
//!   RGBA8_ETC2_EAC, SRGB8_ALPHA8_ETC2_EAC): `V3D_SAMPLED_TEXFMTS` holds their
//!   classes, so a draw that samples one is no longer refused as "a texel
//!   format the V3D does not sample"; its texture record is Mesa's ETC2/EAC
//!   row (v3dx_format_table.c:159-168) and the blocks are in the tiled copy.
//! * The six formats the GL core decodes at upload (R11/RG11 EAC, signed and
//!   unsigned, and the two punch-through ones): a draw samples the class each
//!   lands in (R16F, RGBA16F, RGBA8, SRGB8_ALPHA8).
//! * A shadow draw: the sampler record carries the texture's compare function
//!   (`v3d::texture::pack_sampler`, docs/notes/v3d-shadow.md (a)).
//!
//! What it cannot show: that the TMU decodes the blocks (silicon; the Pi 4's
//! board session), and a picture: the host back end has no GPU, and the
//! readback of a draw is black.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_etc2_v3d
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_formats_v3d");
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

/// The fields of a packed record (v3d_packet.xml, Texture Shader State,
/// V3D 4.2): (Texture type, [Swizzle R, G, B, A], sRGB).
fn decode(rec: &[u8; V3D_TEX_STATE_BYTES]) -> (u32, [u8; 4], bool) {
    let mut lo = [0u8; 16];
    lo.copy_from_slice(&rec[..16]);
    let x = u128::from_le_bytes(lo);
    let field = |start: u32, size: u32| ((x >> start) & ((1u128 << size) - 1)) as u32;
    (
        field(100, 7),
        [field(108, 3) as u8, field(111, 3) as u8, field(114, 3) as u8, field(117, 3) as u8],
        field(3, 1) != 0,
    )
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

    /* ---- the pins ---------------------------------------------------------- */

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

    const SIDE: i32 = 8;

    /// The last draw's first texture shader state record, as the TMU reads it.
    fn last_tmu_record() -> Option<[u8; V3D_TEX_STATE_BYTES]> {
        let t = jobs().iter().flat_map(|j| j.tmu.iter().flatten()).last().cloned()?;
        let mut rec = [0u8; V3D_TEX_STATE_BYTES];
        with_platform(|p| {
            let src = platform_cpu(p, u64::from(t.tex_state), V3D_TEX_STATE_BYTES);
            // SAFETY: platform_cpu checked the span.
            unsafe { core::ptr::copy_nonoverlapping(src, rec.as_mut_ptr(), V3D_TEX_STATE_BYTES) };
        });
        Some(rec)
    }

    /// Is `needle` anywhere in the GPU memory the back end was given?
    fn memory_has(needle: &[u8]) -> bool {
        with_platform(|p| {
            p.allocs.values().any(|a| {
                // SAFETY: the allocation's own u64 backing, `size` bytes of it in use.
                let m = unsafe { std::slice::from_raw_parts(a.mem.as_ptr() as *const u8, a.size) };
                m.windows(needle.len()).any(|w| w == needle)
            })
        })
    }

    const VS: &str =
        "#version 300 es\nlayout(location = 0) in vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";
    const FS: &str = "#version 300 es\nprecision highp float;\nuniform sampler2D s;\nout vec4 o;\n\
        void main() { o = texture(s, vec2(0.5)); }\n";
    static QUAD: [f32; 18] = [
        -1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, //
        -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0,
    ];

    /// `n` bytes from a seed (every bit pattern a block holds turns up).
    fn bytes(seed: u32, n: usize) -> Vec<u8> {
        let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(0x9e37_79b9) | 1;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s >> 11) as u8
            })
            .collect()
    }

    /// Texel (0, 0) of an 8x8 image of `blocks` of format `f`, as the class it
    /// is stored in holds it: halves for the EAC formats (`etc2`'s 16-bit
    /// values over 65535, or 32767 signed; RG11 as RGBA16F (r, g, 0, 1)), the
    /// RGBA8 texel for the punch-through ones.
    fn decoded_texel(f: GLenum, blocks: &[u8]) -> Vec<u8> {
        use yakogl::gl::es3_formats::f32_to_half;
        use yakogl::gl::etc2;
        let half = |v: f32| f32_to_half(v).to_le_bytes();
        let (w, h) = (SIDE as usize, SIDE as usize);
        match f {
            GL_COMPRESSED_R11_EAC
            | GL_COMPRESSED_SIGNED_R11_EAC
            | GL_COMPRESSED_RG11_EAC
            | GL_COMPRESSED_SIGNED_RG11_EAC => {
                let ch = if matches!(f, GL_COMPRESSED_R11_EAC | GL_COMPRESSED_SIGNED_R11_EAC) { 1 } else { 2 };
                let signed = matches!(f, GL_COMPRESSED_SIGNED_R11_EAC | GL_COMPRESSED_SIGNED_RG11_EAC);
                let v: Vec<f32> = if signed {
                    let mut o = vec![0i16; w * h * ch];
                    assert!(etc2::decode_eac11_signed_image(w, h, ch, blocks, &mut o));
                    o[..ch].iter().map(|&x| (f32::from(x) / 32767.0).max(-1.0)).collect()
                } else {
                    let mut o = vec![0u16; w * h * ch];
                    assert!(etc2::decode_eac11_unsigned_image(w, h, ch, blocks, &mut o));
                    o[..ch].iter().map(|&x| f32::from(x) / 65535.0).collect()
                };
                if ch == 1 {
                    half(v[0]).to_vec()
                } else {
                    [half(v[0]), half(v[1]), [0, 0], half(1.0)].concat()
                }
            }
            _ => {
                let mut o = vec![0u8; w * h * 4];
                assert!(etc2::decode_rgb_image(w, h, blocks, true, &mut o));
                o[..4].to_vec()
            }
        }
    }

    /// Draw the canvas quad with the texture bound to unit 0 and the program
    /// `prog` (sampler `s` on unit 0): the draw reaches the job builder with a
    /// texture record and the back end refuses nothing.
    fn draw_sampling(prog: GLuint) {
        let before = jobs().iter().map(|j| j.tmu.len()).sum::<usize>();
        // SAFETY: plain GL calls; the quad lives for the call.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, CANVAS.0, CANVAS.1);
            glUseProgram(prog);
            glUniform1i(glGetUniformLocation(prog, c"s".as_ptr()), 0);
            glVertexAttribPointer(0, 3, GL_FLOAT, 0, 12, QUAD.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 6);
            glFinish();
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        let after: Vec<_> = jobs().iter().flat_map(|j| j.tmu.iter().cloned()).collect();
        assert!(after.len() > before, "the draw reached the job builder: {:?}", logs());
        assert!(after.last().is_some_and(|t| !t.is_empty()), "the draw carries a texture record: {:?}", logs());
        let refused: Vec<_> =
            logs().into_iter().filter(|l| l.contains("does not sample") || l.contains("refus")).collect();
        assert!(refused.is_empty(), "{refused:?}");
    }

    fn bind_2d() -> GLuint {
        let mut tex = 0u32;
        // SAFETY: plain GL calls on a name made here.
        unsafe {
            glGenTextures(1, &mut tex);
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        }
        tex
    }

    fn compressed_8x8(internalformat: GLenum, blocks: &[u8]) {
        // SAFETY: `blocks` is the 8x8 image, imageSize bytes.
        unsafe {
            glCompressedTexImage2D(
                GL_TEXTURE_2D,
                0,
                internalformat,
                SIDE,
                SIDE,
                0,
                blocks.len() as GLsizei,
                blocks.as_ptr() as *const c_void,
            );
            assert_eq!(glGetError(), GL_NO_ERROR, "0x{internalformat:x}: {:?}", logs());
        }
    }

    /* Mesa's rows (v3dx_format_table.c:159-168): the type, swizzle and sRGB bit of
     * the record of each ETC2/EAC class, and the bytes of a block. */
    const XYZW: [u8; 4] = [2, 3, 4, 5];
    const XYZ1: [u8; 4] = [2, 3, 4, 1];
    const X001: [u8; 4] = [2, 0, 0, 1];

    /// The four formats the TMU reads as blocks: the draw is not refused (the
    /// switch-on: V3D_SAMPLED_TEXFMTS holds their classes), the record is the
    /// ETC2/EAC row (RGB8_ETC2 type 32, RGBA8_ETC2_EAC type 38, the sRGB bit for
    /// the two sRGB names only) and the blocks are in the tiled copy the TMU
    /// reads, byte for byte.
    #[test]
    fn the_four_block_formats_reach_the_tmu_as_their_etc2_rows() {
        let _g = rig();
        let rows: [(GLenum, usize, u32, [u8; 4], bool); 4] = [
            (GL_COMPRESSED_RGB8_ETC2, 8, V3D_TEXTURE_DATA_FORMAT_RGB8_ETC2, XYZ1, false),
            (GL_COMPRESSED_SRGB8_ETC2, 8, V3D_TEXTURE_DATA_FORMAT_RGB8_ETC2, XYZ1, true),
            (GL_COMPRESSED_RGBA8_ETC2_EAC, 16, V3D_TEXTURE_DATA_FORMAT_RGBA8_ETC2_EAC, XYZW, false),
            (GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC, 16, V3D_TEXTURE_DATA_FORMAT_RGBA8_ETC2_EAC, XYZW, true),
        ];
        let prog = program(VS, FS);
        for (i, (f, bb, type_, swizzle, srgb)) in rows.into_iter().enumerate() {
            let tex = bind_2d();
            let blocks = bytes(40 + i as u32, 4 * bb); /* 8x8 is 2x2 blocks */
            compressed_8x8(f, &blocks);
            draw_sampling(prog);
            let rec = last_tmu_record().expect("the draw carries a texture record");
            assert_eq!(decode(&rec), (type_, swizzle, srgb), "0x{f:x}");
            for b in blocks.chunks(bb) {
                assert!(memory_has(b), "0x{f:x}: a block is in the texture's memory");
            }
            // SAFETY: a name made above.
            unsafe { glDeleteTextures(1, &tex) };
        }
        // SAFETY: a name made above.
        unsafe { glDeleteProgram(prog) };
    }

    /// The six formats the GL core decodes at upload: a draw is not refused
    /// and samples the class each lands in, as that class's row of Mesa's
    /// table (R16F and RGBA16F, whose texels the GL core wrote as halves;
    /// RGBA8, and with the sRGB bit for the punch-through sRGB name), and the
    /// decoded texels are in the memory the TMU reads.
    #[test]
    fn the_six_decoded_formats_are_sampled_as_their_classes() {
        let _g = rig();
        let rows: [(GLenum, usize, u32, [u8; 4], bool); 6] = [
            (GL_COMPRESSED_R11_EAC, 8, V3D_TEXTURE_DATA_FORMAT_R16F, X001, false),
            (GL_COMPRESSED_SIGNED_R11_EAC, 8, V3D_TEXTURE_DATA_FORMAT_R16F, X001, false),
            (GL_COMPRESSED_RG11_EAC, 16, V3D_TEXTURE_DATA_FORMAT_RGBA16F, XYZW, false),
            (GL_COMPRESSED_SIGNED_RG11_EAC, 16, V3D_TEXTURE_DATA_FORMAT_RGBA16F, XYZW, false),
            (GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, 8, V3D_TEXTURE_DATA_FORMAT_RGBA8, XYZW, false),
            (GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2, 8, V3D_TEXTURE_DATA_FORMAT_RGBA8, XYZW, true),
        ];
        let prog = program(VS, FS);
        for (i, (f, bb, type_, swizzle, srgb)) in rows.into_iter().enumerate() {
            let tex = bind_2d();
            let blocks = bytes(60 + i as u32, 4 * bb);
            compressed_8x8(f, &blocks);
            draw_sampling(prog);
            let rec = last_tmu_record().expect("the draw carries a texture record");
            assert_eq!(decode(&rec), (type_, swizzle, srgb), "0x{f:x}: the class's row");
            /* the first decoded texel, as the class stores it, is in the
             * texture's memory: the RG11 and R11 formats' (0, 0) are the
             * halves of `etc2`'s values, the punch-through's four bytes */
            let texel = decoded_texel(f, &blocks);
            assert!(memory_has(&texel), "0x{f:x}: the decoded texel {texel:?} is in the texture's memory");
            // SAFETY: a name made above.
            unsafe { glDeleteTextures(1, &tex) };
        }
        // SAFETY: a name made above.
        unsafe { glDeleteProgram(prog) };
    }

    /// A depth texture compared by `func` (GL_TEXTURE_COMPARE_MODE
    /// GL_COMPARE_REF_TO_TEXTURE) is sampled by a `sampler2DShadow` look-up:
    /// the sampler record of the draw names the function (Depth Compare
    /// Function, bits 4-6 of its first word, v3d_packet.xml:1728), and another
    /// function in the same job is another record. The look-up forms are the
    /// 2D ones of tests/es3_shadow_v3d.rs (`texture`, `textureProj`,
    /// `textureOffset`, `textureLod`), each its own program; a texture with the
    /// compare mode off is the compare-off record.
    #[test]
    fn a_shadow_draw_compares_with_the_texture_objects_function() {
        const FORMS: [&str; 4] = [
            "texture(s, vec3(0.5, 0.5, 0.5))",
            "textureProj(s, vec4(0.5, 0.5, 0.5, 1.0))",
            "textureOffset(s, vec3(0.5, 0.5, 0.5), ivec2(1, 0))",
            "textureLod(s, vec3(0.5, 0.5, 0.5), 0.0)",
        ];
        let _g = rig();
        let mut tex = 0u32;
        let depth = vec![0x40u8; (SIDE * SIDE * 2) as usize];
        // SAFETY: plain GL calls; `depth` is a SIDE x SIDE image of 16-bit depth.
        unsafe {
            glGenTextures(1, &mut tex);
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_DEPTH_COMPONENT16 as GLint,
                SIDE,
                SIDE,
                0,
                GL_DEPTH_COMPONENT,
                GL_UNSIGNED_SHORT,
                depth.as_ptr() as *const c_void,
            );
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        let function_of_last_draw = || -> u32 {
            let t = jobs().iter().flat_map(|j| j.tmu.iter().flatten()).last().cloned().expect("a texture record");
            let mut rec = [0u8; 4];
            with_platform(|p| {
                let src = platform_cpu(p, u64::from(t.sampler), 4);
                // SAFETY: platform_cpu checked the span.
                unsafe { core::ptr::copy_nonoverlapping(src, rec.as_mut_ptr(), 4) };
            });
            (u32::from_le_bytes(rec) >> 4) & 7
        };
        for form in FORMS {
            let fs = format!(
                "#version 300 es\nprecision highp float;\nprecision highp sampler2DShadow;\n\
                 uniform sampler2DShadow s;\nout vec4 o;\nvoid main() {{ o = vec4({form}); }}\n"
            );
            let prog = program(VS, &fs);
            let mut seen = Vec::new();
            // SAFETY: a texture bound above.
            unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE as GLint) };
            for func in [GL_LEQUAL, GL_GREATER, GL_NOTEQUAL] {
                // SAFETY: a texture bound above.
                unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_FUNC, func as GLint) };
                draw_sampling(prog);
                seen.push(function_of_last_draw());
            }
            assert_eq!(seen, [3, 4, 5], "{form}: GL_LEQUAL, GL_GREATER, GL_NOTEQUAL are the record's 3, 4, 5");
            // SAFETY: a texture bound above.
            unsafe { glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_MODE, GL_NONE as GLint) };
            draw_sampling(prog);
            assert_eq!(function_of_last_draw(), 0, "{form}: no compare is the compare-off record");
            // SAFETY: a name made above.
            unsafe { glDeleteProgram(prog) };
        }
        // SAFETY: a name made above.
        unsafe { glDeleteTextures(1, &tex) };
    }
}
