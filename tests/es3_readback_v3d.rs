// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glReadPixels of integer and float colour attachments through the GL API
//! over the real V3D back end on the host (t-readback-int): the step that
//! ends most dEQP-GLES3 fbo.* and fragment_out.* cases on the Pi 4
//! (docs/es3/pi4-gles3-fails.tsv), a read of GL_RGBA_INTEGER / GL_INT or
//! GL_UNSIGNED_INT, or of GL_RGBA / GL_FLOAT.
//!
//! For every sized integer colour format and every float format the GL core
//! makes renderbuffer storage of (R16F, RG16F, RGB16F, RGBA16F, R32F), a
//! renderbuffer is cleared with glClearBuffer{i,ui,f}v and read back with
//! every (format, type) pair ES 3.0 4.3.1 lists for the class, the pair
//! GL_IMPLEMENTATION_COLOR_READ_FORMAT / _TYPE names, and every other pair of
//! the pixel format and type enums (GL_INVALID_OPERATION for a known pair the
//! buffer does not take, GL_INVALID_ENUM for a name that is none). The same
//! formats as texture attachments: the ones the GL core takes (the
//! four-channel integer classes, R16F, RG16F, RGBA16F, R32F) read the same;
//! the narrow integer textures and RGB16F are incomplete attachments today
//! (pinned with their expected results in tests/es3_narrow_int_rt_v3d.rs and
//! tests/es3_rgb16f_attach_v3d.rs), so no readback of them begins.
//!
//! The rig is tests/es3_pbo_rb_clears_v3d.rs' `gl_rig`: the job builder and
//! the GPU stood in for. The stand-in GPU applies a job's whole-target clear
//! words to every render target's raster store, in the output format the
//! target names (`V3dJobClear::clear_rt[n]`, as `state::clear_words` packs
//! them), and runs nothing else; so what is asserted is the clear value the
//! GL core handed the back end, packed as the tile buffer stores it, read
//! back through the GL core's glReadPixels. The plain tests pin today's
//! answers; the `#[ignore]`d ones state ES 3.0's where the two differ, each
//! naming its cause in src/gl (plan #186's files, not edited here).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_readback_v3d
//! ```
#![cfg(all(
    feature = "host-test",
    feature = "gpu-v3d",
    feature = "shaderc-online",
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#![allow(dead_code)]

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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_readback_v3d");
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
    use yakogl::gl::framebuffer::*;
    use yakogl::gl::state::*;
    use yakogl::gl::types::*;
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
    use yakogl::v3d::backend::*;
    use yakogl::v3d::kick::V3dKick;

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
        draws: u32,
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
        jobs().push(SeenJob { target: t, clear: None, draws: 0 });
        t.width != 0 && t.height != 0
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, _d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
        // SAFETY: the backend passes its live job.
        let j = unsafe { &mut *j };
        j.draws += 1;
        // SAFETY: the job's BCL is live.
        unsafe { (*j.bcl).used += 120 };
        if let Some(job) = jobs().last_mut() {
            job.draws += 1;
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

    /// The texel the tile buffer stores for a render target of output format
    /// `fmt` cleared with the words `w` (`state::clear_words`): the float rows
    /// and the integer rows, as many bytes as the texel has. Empty for a
    /// format the stand-in does not model.
    fn clear_texel(fmt: u32, w: [u32; 4]) -> Vec<u8> {
        let words = |n: usize| -> Vec<u8> { w[..n].iter().flat_map(|x| x.to_le_bytes()).collect() };
        match fmt {
            V3D_OUTPUT_IMAGE_FORMAT_RGBA16F => words(2),
            V3D_OUTPUT_IMAGE_FORMAT_R16F => words(1)[..2].to_vec(),
            V3D_OUTPUT_IMAGE_FORMAT_R32F => words(1),
            V3D_OUTPUT_IMAGE_FORMAT_R8I | V3D_OUTPUT_IMAGE_FORMAT_R8UI => words(1)[..1].to_vec(),
            V3D_OUTPUT_IMAGE_FORMAT_RG8I | V3D_OUTPUT_IMAGE_FORMAT_RG8UI => words(1)[..2].to_vec(),
            V3D_OUTPUT_IMAGE_FORMAT_RGBA8I | V3D_OUTPUT_IMAGE_FORMAT_RGBA8UI => words(1),
            V3D_OUTPUT_IMAGE_FORMAT_R16I | V3D_OUTPUT_IMAGE_FORMAT_R16UI => words(1)[..2].to_vec(),
            V3D_OUTPUT_IMAGE_FORMAT_RG16I | V3D_OUTPUT_IMAGE_FORMAT_RG16UI => words(1),
            V3D_OUTPUT_IMAGE_FORMAT_RGBA16I | V3D_OUTPUT_IMAGE_FORMAT_RGBA16UI => words(2),
            V3D_OUTPUT_IMAGE_FORMAT_R32I | V3D_OUTPUT_IMAGE_FORMAT_R32UI => words(1),
            V3D_OUTPUT_IMAGE_FORMAT_RG32I | V3D_OUTPUT_IMAGE_FORMAT_RG32UI => words(2),
            V3D_OUTPUT_IMAGE_FORMAT_RGBA32I | V3D_OUTPUT_IMAGE_FORMAT_RGBA32UI => words(4),
            V3D_OUTPUT_IMAGE_FORMAT_RGB10_A2UI => {
                /* 16UI in the tile buffer, packed to 10/10/10/2 at the store */
                let (r, g, b, a) = (w[0] & 0x3ff, w[0] >> 16 & 0x3ff, w[1] & 0x3ff, w[1] >> 16 & 3);
                (r | g << 10 | b << 20 | a << 30).to_le_bytes().to_vec()
            }
            V3D_OUTPUT_IMAGE_FORMAT_RGBA8 => words(1),
            _ => Vec::new(),
        }
    }

    /// `texel` into every pixel of a `width` x `height` raster store at
    /// `address`, rows `stride` bytes apart.
    fn fill(address: u32, stride: u32, width: u32, height: u32, texel: &[u8]) {
        if texel.is_empty() || address == 0 {
            return;
        }
        with_platform(|p| {
            let row = (stride as usize).max(width as usize * texel.len());
            let base = platform_cpu(p, u64::from(address), row * height as usize);
            for y in 0..height as usize {
                for x in 0..width as usize {
                    // SAFETY: inside the span platform_cpu checked.
                    unsafe {
                        core::ptr::copy_nonoverlapping(texel.as_ptr(), base.add(y * row + x * texel.len()), texel.len())
                    };
                }
            }
        });
    }

    /// The "GPU": every job completes inside its submit (as src/osal/linux.c),
    /// and a job that clears a colour render target (RT 0 or an RT 1..3 of a
    /// several-attachment job) leaves the clear's texel in every pixel of that
    /// target's raster store: the ES 2.0 job's `rgba8` word, a typed job's
    /// `clear_rt[n]` words packed by the output format the target names.
    /// Nothing else runs: a clear quad (a scissored or masked clear) is a
    /// draw, and draws leave memory alone.
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
        if !t.typed && !t.rgb565 && !c.load_rt0 && t.stride as usize >= t.width as usize * 4 {
            /* the ES 2.0 job: RGBA8 words of `rgba8` */
            fill(t.address, t.stride, t.width, t.height, &c.rgba8.to_le_bytes());
        }
        if t.typed {
            if !c.load_rt0 {
                fill(t.address, t.stride, t.width, t.height, &clear_texel(t.rt_format[0], c.clear_rt[0]));
            }
            for n in 1..t.rts.min(4) as usize {
                if !c.load_rt[n - 1] {
                    let rt = t.rt[n - 1];
                    fill(rt.address, rt.stride, t.width, t.height, &clear_texel(t.rt_format[n], c.clear_rt[n]));
                }
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

    const CANVAS: (i32, i32) = (16, 16);

    static LOCK: Mutex<()> = Mutex::new(());
    static ONCE: Once = Once::new();
    static mut CTX: *mut GlContext = core::ptr::null_mut();

    /// The one context and backend of the file, set up once; the tests take
    /// turns, each starting with a clean error state and an empty log.
    pub fn rig() -> MutexGuard<'static, ()> {
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
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glFinish();
            while glGetError() != GL_NO_ERROR {}
        }
        with_platform(|p| p.log.clear());
        g
    }

    pub fn logs() -> Vec<String> {
        with_platform(|p| p.log.clone())
    }

    pub fn err() -> GLenum {
        // SAFETY: a plain GL call.
        unsafe { glGetError() }
    }

    /* ---- the formats ----------------------------------------------------------- */

    use yakogl::gl::es3::*;

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Class {
        /// Signed integer: read as RGBA_INTEGER / INT, cleared with glClearBufferiv.
        Int,
        /// Unsigned integer: RGBA_INTEGER / UNSIGNED_INT, glClearBufferuiv.
        Uint,
        /// Floating point: RGBA / FLOAT (and the bytes of RGBA / UNSIGNED_BYTE),
        /// glClearBufferfv.
        Float,
    }

    /// A colour format of the tests: GL name, class, channels the format has,
    /// bits a colour channel (10 for RGB10_A2UI, whose alpha has 2).
    #[derive(Clone, Copy, Debug)]
    pub struct Fmt {
        pub name: &'static str,
        pub gl: GLenum,
        pub class: Class,
        pub channels: usize,
        pub bits: u32,
    }

    const fn f(name: &'static str, gl: GLenum, class: Class, channels: usize, bits: u32) -> Fmt {
        Fmt { name, gl, class, channels, bits }
    }

    /// Every sized integer colour format of ES 3.0 (table 3.13, all
    /// colour-renderable).
    pub const INTS: [Fmt; 19] = [
        f("R8I", GL_R8I, Class::Int, 1, 8),
        f("R8UI", GL_R8UI, Class::Uint, 1, 8),
        f("R16I", GL_R16I, Class::Int, 1, 16),
        f("R16UI", GL_R16UI, Class::Uint, 1, 16),
        f("R32I", GL_R32I, Class::Int, 1, 32),
        f("R32UI", GL_R32UI, Class::Uint, 1, 32),
        f("RG8I", GL_RG8I, Class::Int, 2, 8),
        f("RG8UI", GL_RG8UI, Class::Uint, 2, 8),
        f("RG16I", GL_RG16I, Class::Int, 2, 16),
        f("RG16UI", GL_RG16UI, Class::Uint, 2, 16),
        f("RG32I", GL_RG32I, Class::Int, 2, 32),
        f("RG32UI", GL_RG32UI, Class::Uint, 2, 32),
        f("RGBA8I", GL_RGBA8I, Class::Int, 4, 8),
        f("RGBA8UI", GL_RGBA8UI, Class::Uint, 4, 8),
        f("RGBA16I", GL_RGBA16I, Class::Int, 4, 16),
        f("RGBA16UI", GL_RGBA16UI, Class::Uint, 4, 16),
        f("RGBA32I", GL_RGBA32I, Class::Int, 4, 32),
        f("RGBA32UI", GL_RGBA32UI, Class::Uint, 4, 32),
        f("RGB10_A2UI", GL_RGB10_A2UI, Class::Uint, 4, 10),
    ];

    /// The float colour formats the GL core makes renderbuffer storage of
    /// (GL_EXT_color_buffer_half_float's four, and R32F; ES 3.0 itself has no
    /// colour-renderable float format). RG16F and RGB16F are held in the
    /// RGBA16F surface, four halves a pixel.
    pub const FLOATS: [Fmt; 5] = [
        f("R16F", GL_R16F, Class::Float, 1, 16),
        f("RG16F", GL_RG16F, Class::Float, 2, 16),
        f("RGB16F", GL_RGB16F, Class::Float, 3, 16),
        f("RGBA16F", GL_RGBA16F, Class::Float, 4, 16),
        f("R32F", GL_R32F, Class::Float, 1, 32),
    ];

    /// The clear colour of a format: four values inside the class's range, a
    /// negative one and the extremes for a signed class, the maximum for an
    /// unsigned one, exact halves for the float class.
    impl Fmt {
        pub fn clear_ints(&self) -> [i32; 4] {
            match self.bits {
                8 => [-100, 127, -128, 5],
                16 => [-30000, 32767, -32768, 7],
                _ => [-2_000_000_000, i32::MAX, i32::MIN, 9],
            }
        }
        pub fn clear_uints(&self) -> [u32; 4] {
            match self.bits {
                8 => [200, 255, 0, 5],
                10 => [1000, 1023, 0, 3],
                16 => [60000, 65535, 0, 7],
                _ => [4_000_000_000, u32::MAX, 0, 9],
            }
        }
        pub const CLEAR_FLOATS: [f32; 4] = [0.5, -0.25, 0.75, 0.125];

        /// The pair ES 3.0 4.3.1 lists for the class.
        pub fn pair(&self) -> (GLenum, GLenum) {
            match self.class {
                Class::Int => (GL_RGBA_INTEGER, GL_INT),
                Class::Uint => (GL_RGBA_INTEGER, GL_UNSIGNED_INT),
                Class::Float => (GL_RGBA, GL_FLOAT),
            }
        }

        /// Is (format, type) a pair the class's read buffer takes? For the
        /// float class the GL core's implementation pair RGBA / UNSIGNED_BYTE
        /// counts too (`the_implementation_pair_is_accepted`).
        pub fn takes(&self, format: GLenum, ty: GLenum) -> bool {
            (format, ty) == self.pair() || (self.class == Class::Float && (format, ty) == (GL_RGBA, GL_UNSIGNED_BYTE))
        }

        /// What one pixel of the cleared buffer reads through the class's
        /// pair (ES 3.0 4.3.1): the clear's value in the channels the format
        /// has, 0, 0, 1 for the ones it lacks. Integer words as their bits, a
        /// float's as `f32::to_bits`.
        pub fn expect(&self) -> [u32; 4] {
            let mut w = match self.class {
                Class::Int => self.clear_ints().map(|v| v as u32),
                Class::Uint => self.clear_uints(),
                Class::Float => Self::CLEAR_FLOATS.map(f32::to_bits),
            };
            let one = if self.class == Class::Float { 1.0f32.to_bits() } else { 1 };
            for (ch, slot) in w.iter_mut().enumerate().skip(self.channels) {
                *slot = if ch == 3 { one } else { 0 };
            }
            w
        }

        /// The bytes RGBA / UNSIGNED_BYTE hands out for a float buffer:
        /// each channel of `expect` clamped to [0, 1] and rounded.
        pub fn expect_bytes(&self) -> [u8; 4] {
            self.expect().map(|w| (f32::from_bits(w).clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
        }

        pub fn clear(&self, drawbuffer: GLint) {
            // SAFETY: plain GL calls with four-element arrays.
            unsafe {
                match self.class {
                    Class::Int => glClearBufferiv(GL_COLOR, drawbuffer, self.clear_ints().as_ptr()),
                    Class::Uint => glClearBufferuiv(GL_COLOR, drawbuffer, self.clear_uints().as_ptr()),
                    Class::Float => glClearBufferfv(GL_COLOR, drawbuffer, Self::CLEAR_FLOATS.as_ptr()),
                }
            }
        }
    }

    /* ---- framebuffers and reads ------------------------------------------------ */

    pub const W: GLsizei = 8;
    pub const H: GLsizei = 4;

    /// A complete framebuffer of one renderbuffer per format, bound, with
    /// every attachment a draw buffer. Dropped: the names go.
    pub struct Fb {
        pub fbo: GLuint,
        pub rbs: Vec<GLuint>,
    }

    impl Fb {
        pub fn new(formats: &[Fmt]) -> Fb {
            let mut fbo = 0;
            let mut rbs = Vec::new();
            // SAFETY: plain GL calls with room for one name each.
            unsafe {
                glGenFramebuffers(1, &mut fbo);
                glBindFramebuffer(GL_FRAMEBUFFER, fbo);
                for (n, f) in formats.iter().enumerate() {
                    let mut rb = 0;
                    glGenRenderbuffers(1, &mut rb);
                    glBindRenderbuffer(GL_RENDERBUFFER, rb);
                    glRenderbufferStorage(GL_RENDERBUFFER, f.gl, W, H);
                    assert_eq!(glGetError(), GL_NO_ERROR, "{}: renderbuffer storage", f.name);
                    glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + n as u32, GL_RENDERBUFFER, rb);
                    rbs.push(rb);
                }
                let names: Vec<_> = formats.iter().map(|f| f.name).collect();
                assert_eq!(glGetError(), GL_NO_ERROR, "{names:?}: {:?}", logs());
                assert_eq!(
                    glCheckFramebufferStatus(GL_FRAMEBUFFER),
                    GL_FRAMEBUFFER_COMPLETE,
                    "{names:?}: {:?}",
                    logs()
                );
                let bufs: Vec<GLenum> = (0..formats.len() as u32).map(|n| GL_COLOR_ATTACHMENT0 + n).collect();
                glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
                glViewport(0, 0, W, H);
                glDisable(GL_SCISSOR_TEST);
                glColorMask(1, 1, 1, 1);
            }
            Fb { fbo, rbs }
        }
    }

    impl Drop for Fb {
        fn drop(&mut self) {
            // SAFETY: names this framebuffer made.
            unsafe {
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                glDeleteFramebuffers(1, &self.fbo);
                glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            }
        }
    }

    /// The whole W x H read buffer through (format, type): the error, and the
    /// buffer as 32-bit words (16 bytes a pixel is room for any type), the
    /// words a refused read left untouched at 0xdead_beef.
    pub fn read(format: GLenum, ty: GLenum) -> (GLenum, Vec<u32>) {
        let mut out = vec![0xdead_beefu32; (W * H) as usize * 4];
        // SAFETY: room for W x H pixels of 16 bytes.
        unsafe {
            while glGetError() != GL_NO_ERROR {}
            glReadPixels(0, 0, W, H, format, ty, out.as_mut_ptr() as *mut c_void);
            (glGetError(), out)
        }
    }

    pub fn read_error(format: GLenum, ty: GLenum) -> GLenum {
        read(format, ty).0
    }

    /// GL_IMPLEMENTATION_COLOR_READ_FORMAT and _TYPE of the read buffer.
    pub fn implementation_pair() -> (GLenum, GLenum) {
        let (mut fmt, mut ty) = (0, 0);
        // SAFETY: GLint out-parameters.
        unsafe {
            glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut fmt);
            glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut ty);
            assert_eq!(glGetError(), GL_NO_ERROR);
        }
        (fmt as GLenum, ty as GLenum)
    }

    /// Nothing the sequence did was refused by the back end by name, and no
    /// GL_OUT_OF_MEMORY (the error that aborts a dEQP session) was raised.
    pub fn nothing_refused(what: &str) {
        let refused: Vec<_> = logs().into_iter().filter(|l| l.contains("refus") || l.contains("0x505")).collect();
        assert!(refused.is_empty(), "{what}: {refused:?}");
    }

    /* ---- every pair of the pixel enums ----------------------------------------- */

    /// The pixel formats of ES 3.0 table 3.3 (and ES 2.0's), plus one name
    /// that is none.
    pub const FORMATS: [(GLenum, &str); 14] = [
        (GL_RGBA, "RGBA"),
        (GL_RGB, "RGB"),
        (GL_RG, "RG"),
        (GL_RED, "RED"),
        (GL_ALPHA, "ALPHA"),
        (GL_LUMINANCE, "LUMINANCE"),
        (GL_LUMINANCE_ALPHA, "LUMINANCE_ALPHA"),
        (GL_RGBA_INTEGER, "RGBA_INTEGER"),
        (GL_RGB_INTEGER, "RGB_INTEGER"),
        (GL_RG_INTEGER, "RG_INTEGER"),
        (GL_RED_INTEGER, "RED_INTEGER"),
        (GL_DEPTH_COMPONENT, "DEPTH_COMPONENT"),
        (GL_DEPTH_STENCIL, "DEPTH_STENCIL"),
        (UNKNOWN, "unknown 0x1234"),
    ];

    /// The pixel types of table 3.2, plus one that is none.
    pub const TYPES: [(GLenum, &str); 17] = [
        (GL_UNSIGNED_BYTE, "UNSIGNED_BYTE"),
        (GL_BYTE, "BYTE"),
        (GL_UNSIGNED_SHORT, "UNSIGNED_SHORT"),
        (GL_SHORT, "SHORT"),
        (GL_UNSIGNED_INT, "UNSIGNED_INT"),
        (GL_INT, "INT"),
        (GL_HALF_FLOAT, "HALF_FLOAT"),
        (GL_FLOAT, "FLOAT"),
        (GL_UNSIGNED_SHORT_5_6_5, "UNSIGNED_SHORT_5_6_5"),
        (GL_UNSIGNED_SHORT_4_4_4_4, "UNSIGNED_SHORT_4_4_4_4"),
        (GL_UNSIGNED_SHORT_5_5_5_1, "UNSIGNED_SHORT_5_5_5_1"),
        (GL_UNSIGNED_INT_2_10_10_10_REV, "UNSIGNED_INT_2_10_10_10_REV"),
        (GL_UNSIGNED_INT_10F_11F_11F_REV, "UNSIGNED_INT_10F_11F_11F_REV"),
        (GL_UNSIGNED_INT_5_9_9_9_REV, "UNSIGNED_INT_5_9_9_9_REV"),
        (GL_UNSIGNED_INT_24_8, "UNSIGNED_INT_24_8"),
        (GL_FLOAT_32_UNSIGNED_INT_24_8_REV, "FLOAT_32_UNSIGNED_INT_24_8_REV"),
        (UNKNOWN, "unknown 0x1234"),
    ];

    /// A value that is no pixel format or type enum.
    pub const UNKNOWN: GLenum = 0x1234;

    /// Every pair of FORMATS x TYPES on the bound read buffer of `f`: the
    /// pairs the class takes read without an error, every other known pair is
    /// GL_INVALID_OPERATION (the depth formats and depth-only types too: they
    /// are names of tables 3.2 and 3.3, so theirs is the combination error),
    /// a name that is none GL_INVALID_ENUM, and a refused read writes
    /// nothing. The differences from that, as text.
    pub fn pairs_differing_from_the_spec(f: &Fmt) -> Vec<String> {
        let mut wrong = Vec::new();
        for (format, fname) in FORMATS {
            for (ty, tname) in TYPES {
                let (e, px) = read(format, ty);
                let want = if f.takes(format, ty) {
                    GL_NO_ERROR
                } else if format == UNKNOWN || ty == UNKNOWN {
                    GL_INVALID_ENUM
                } else {
                    GL_INVALID_OPERATION
                };
                if e != want {
                    wrong.push(format!("{} {fname}/{tname}: error {e:#x}, ES 3.0 4.3.1 {want:#x}", f.name));
                }
                if e != GL_NO_ERROR && px.iter().any(|w| *w != 0xdead_beef) {
                    wrong.push(format!("{} {fname}/{tname}: refused with {e:#x}, yet the buffer was written", f.name));
                }
                assert_ne!(e, GL_OUT_OF_MEMORY, "{} {fname}/{tname}", f.name);
            }
        }
        wrong
    }
}

use gl_rig::*;
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;

/* ---- the integer class ------------------------------------------------------ */

/// Every sized integer format: a clear of in-range values (negative and the
/// extremes for a signed one), read through the class's pair, is the clear's
/// values in the channels the format has and 0, 0, 1 in the ones it lacks,
/// a signed channel sign-extended; a window of the buffer lands at its place
/// and the memory past a window hanging over the edge is left alone. The back
/// end refuses nothing by name and nothing is GL_OUT_OF_MEMORY.
#[test]
fn integer_formats_read_their_clear_through_the_integer_pair() {
    let _g = gl_rig::rig();
    for f in INTS {
        let _fb = Fb::new(&[f]);
        f.clear(0);
        assert_eq!(gl_rig::err(), GL_NO_ERROR, "{}: the clear", f.name);
        let (format, ty) = f.pair();
        let (e, px) = read(format, ty);
        assert_eq!(e, GL_NO_ERROR, "{}: {:?}", f.name, gl_rig::logs());
        let want = f.expect();
        for (i, p) in px.chunks(4).enumerate() {
            assert_eq!(p, want, "{} pixel {i}: {p:x?}, the clear is {want:x?}", f.name);
        }
        /* a 3 x 2 window at (2, 1), and one of 4 pixels hanging over the right edge */
        let mut win = vec![0xdead_beefu32; 3 * 2 * 4];
        let mut over = vec![0xdead_beefu32; 4 * 4];
        // SAFETY: room for 3 x 2 and 4 x 1 pixels of four words.
        unsafe {
            glReadPixels(2, 1, 3, 2, format, ty, win.as_mut_ptr() as *mut c_void);
            assert_eq!(glGetError(), GL_NO_ERROR, "{}: a window", f.name);
            glReadPixels(W - 2, 0, 4, 1, format, ty, over.as_mut_ptr() as *mut c_void);
            assert_eq!(glGetError(), GL_NO_ERROR, "{}: over the edge", f.name);
        }
        assert!(win.chunks(4).all(|p| p == want), "{}: the window", f.name);
        assert!(over[..8].chunks(4).all(|p| p == want), "{}: the two pixels inside", f.name);
        assert!(over[8..].iter().all(|w| *w == 0xdead_beef), "{}: past the edge is untouched", f.name);
        nothing_refused(f.name);
    }
}

/// On an integer read buffer every pair but the class's own is
/// GL_INVALID_OPERATION (a name that is none GL_INVALID_ENUM), and a refused
/// read writes nothing: RGBA / UNSIGNED_BYTE, RGBA / FLOAT, the other
/// signedness, the narrower integer types, RED_INTEGER and the rest.
#[test]
fn integer_formats_refuse_every_other_pair() {
    let _g = gl_rig::rig();
    let mut wrong = Vec::new();
    for f in INTS {
        let _fb = Fb::new(&[f]);
        f.clear(0);
        wrong.extend(pairs_differing_from_the_spec(&f));
    }
    assert!(wrong.is_empty(), "{} pairs differ from the spec:\n{}", wrong.len(), wrong.join("\n"));
}

/* ---- the float class --------------------------------------------------------- */

/// What a float format's clear reads back as RGBA / FLOAT today: the clear
/// in the channels the format has; a lacking blue is 0 and a lacking alpha 1
/// for R16F, R32F and RGB16F; RG16F's lacking blue is the clear's blue
/// (`rg16f_reads_its_missing_blue_as_0` below). RGBA / UNSIGNED_BYTE is the
/// same, clamped to [0, 1] and rounded to a byte.
fn float_read_today(f: &Fmt) -> [f32; 4] {
    let mut want = f.expect().map(f32::from_bits);
    if f.gl == GL_RG16F {
        want[2] = Fmt::CLEAR_FLOATS[2];
    }
    want
}

/// Every float format: the clear read as RGBA / FLOAT and as RGBA /
/// UNSIGNED_BYTE, as today's GL core answers (`float_read_today`).
#[test]
fn float_formats_read_their_clear_as_floats_and_bytes() {
    let _g = gl_rig::rig();
    for f in FLOATS {
        let _fb = Fb::new(&[f]);
        f.clear(0);
        assert_eq!(gl_rig::err(), GL_NO_ERROR, "{}: the clear", f.name);
        let (e, px) = read(GL_RGBA, GL_FLOAT);
        assert_eq!(e, GL_NO_ERROR, "{}: {:?}", f.name, gl_rig::logs());
        let want = float_read_today(&f);
        for (i, p) in px.chunks(4).enumerate() {
            let got = [p[0], p[1], p[2], p[3]].map(f32::from_bits);
            assert_eq!(got, want, "{} pixel {i}", f.name);
        }
        let (e, px) = read(GL_RGBA, GL_UNSIGNED_BYTE);
        assert_eq!(e, GL_NO_ERROR, "{}: RGBA/UNSIGNED_BYTE", f.name);
        let bytes: Vec<u8> = px.iter().take((W * H) as usize).flat_map(|w| w.to_ne_bytes()).collect();
        let want = want.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
        for (i, p) in bytes.chunks(4).enumerate() {
            assert_eq!(p, want, "{} pixel {i}: bytes", f.name);
        }
        nothing_refused(f.name);
    }
}

/// On a float read buffer RGBA / FLOAT and the implementation's RGBA /
/// UNSIGNED_BYTE read; every other pair is GL_INVALID_OPERATION (the integer
/// pairs, HALF_FLOAT, RGB / FLOAT, the packed types), a name that is none
/// GL_INVALID_ENUM, and a refused read writes nothing.
#[test]
fn float_formats_refuse_every_other_pair() {
    let _g = gl_rig::rig();
    let mut wrong = Vec::new();
    for f in FLOATS {
        let _fb = Fb::new(&[f]);
        f.clear(0);
        wrong.extend(pairs_differing_from_the_spec(&f));
    }
    assert!(wrong.is_empty(), "{} pairs differ from the spec:\n{}", wrong.len(), wrong.join("\n"));
}

/// ES 3.0 4.3.1: the channels a format lacks read 0, 0, 1. RG16F lacks
/// blue; today its blue reads what the RGBA16F storage holds, the clear's own
/// blue (the clear colour reaches the back end with all four channels, and
/// the read hands the four halves out).
/// Cause: src/gl/framebuffer.rs `read_float_row` applies no channel mask to
/// a float target (`read_int_row`'s caller does for the integer class). RGB16F
/// and R16F/R32F come out right because the clear's alpha is forced to 1 for a
/// format without alpha (`fb_target_alpha_bits`) and R16F/R32F store one
/// channel.
#[test]
#[ignore = "GL core: RGBA/FLOAT of an RG16F buffer reads the storage's blue, not 0 (read_float_row, plan #186)"]
fn rg16f_reads_its_missing_blue_as_0() {
    let _g = gl_rig::rig();
    let f = FLOATS[1];
    let _fb = Fb::new(&[f]);
    f.clear(0);
    let (e, px) = read(GL_RGBA, GL_FLOAT);
    assert_eq!(e, GL_NO_ERROR);
    assert_eq!([px[0], px[1], px[2], px[3]].map(f32::from_bits), f.expect().map(f32::from_bits));
}

/* ---- the implementation's pair ---------------------------------------------- */

/// GL_IMPLEMENTATION_COLOR_READ_FORMAT / _TYPE of every integer and float
/// buffer is a pair its glReadPixels takes: RGBA_INTEGER with INT or
/// UNSIGNED_INT for the integer classes (the class's one pair), RGBA with
/// UNSIGNED_BYTE for the float class (beside RGBA / FLOAT).
#[test]
fn the_implementation_pair_is_accepted() {
    let _g = gl_rig::rig();
    for f in INTS.iter().chain(FLOATS.iter()) {
        let _fb = Fb::new(&[*f]);
        f.clear(0);
        let pair = implementation_pair();
        let want = match f.class {
            Class::Float => (GL_RGBA, GL_UNSIGNED_BYTE),
            _ => f.pair(),
        };
        assert_eq!(pair, want, "{}", f.name);
        assert_eq!(read_error(pair.0, pair.1), GL_NO_ERROR, "{}: the pair it names", f.name);
    }
}

/* ---- several attachments ----------------------------------------------------- */

/// Three attachments of the three classes (GL_MAX_COLOR_ATTACHMENTS is 3 on
/// the V3D build), each cleared through its own glClearBuffer: the pair, the
/// values and the implementation pair follow glReadBuffer, each read buffer
/// refuses the other classes' pairs, and GL_NONE reads nothing.
#[test]
fn the_read_buffer_picks_the_attachments_class() {
    let _g = gl_rig::rig();
    let mut max = 0;
    // SAFETY: a GLint out-parameter.
    unsafe { glGetIntegerv(GL_MAX_COLOR_ATTACHMENTS, &mut max) };
    assert!(max >= 3, "GL_MAX_COLOR_ATTACHMENTS {max}");
    let fs = [INTS[13], FLOATS[3], INTS[4]]; /* RGBA8UI, RGBA16F, R32I */
    let _fb = Fb::new(&fs);
    for (n, f) in fs.iter().enumerate() {
        f.clear(n as GLint);
        assert_eq!(gl_rig::err(), GL_NO_ERROR, "{}: the clear of draw buffer {n}", f.name);
    }
    for (n, f) in fs.iter().enumerate() {
        // SAFETY: a plain GL call.
        unsafe { glReadBuffer(GL_COLOR_ATTACHMENT0 + n as u32) };
        assert_eq!(gl_rig::err(), GL_NO_ERROR, "{}", f.name);
        let (format, ty) = f.pair();
        let (e, px) = read(format, ty);
        assert_eq!(e, GL_NO_ERROR, "{}: {:?}", f.name, gl_rig::logs());
        let want = if f.class == Class::Float { float_read_today(f).map(f32::to_bits) } else { f.expect() };
        for (i, p) in px.chunks(4).enumerate() {
            assert_eq!(p, want, "{} pixel {i}", f.name);
        }
        let pair = implementation_pair();
        assert_eq!(pair, if f.class == Class::Float { (GL_RGBA, GL_UNSIGNED_BYTE) } else { f.pair() }, "{}", f.name);
        for other in [INTS[13], FLOATS[3], INTS[4]].iter().filter(|o| o.class != f.class) {
            let (of, ot) = other.pair();
            assert_eq!(read_error(of, ot), GL_INVALID_OPERATION, "{}: {}'s pair", f.name, other.name);
        }
    }
    // SAFETY: a plain GL call.
    unsafe { glReadBuffer(GL_NONE) };
    assert_eq!(read_error(GL_RGBA_INTEGER, GL_UNSIGNED_INT), GL_INVALID_OPERATION, "GL_NONE reads nothing");
    assert_eq!(read_error(GL_RGBA, GL_UNSIGNED_BYTE), GL_INVALID_OPERATION, "GL_NONE reads nothing");
    nothing_refused("three attachments");
}

/* ---- the other errors of the call ------------------------------------------- */

/// A negative size is GL_INVALID_VALUE whatever the pair, and the pair's
/// GL_INVALID_OPERATION comes before anything is read.
#[test]
fn a_negative_size_is_invalid_value() {
    let _g = gl_rig::rig();
    let f = INTS[13];
    let _fb = Fb::new(&[f]);
    f.clear(0);
    let mut px = [0xdead_beefu32; 4];
    // SAFETY: one pixel of four words; negative sizes read nothing.
    unsafe {
        glReadPixels(0, 0, -1, 1, GL_RGBA_INTEGER, GL_UNSIGNED_INT, px.as_mut_ptr() as *mut c_void);
        assert_eq!(glGetError(), GL_INVALID_VALUE);
        glReadPixels(0, 0, 1, -1, GL_RGBA_INTEGER, GL_UNSIGNED_INT, px.as_mut_ptr() as *mut c_void);
        assert_eq!(glGetError(), GL_INVALID_VALUE);
        glReadPixels(0, 0, 1, 1, GL_RGBA_INTEGER, GL_INT, px.as_mut_ptr() as *mut c_void);
        assert_eq!(glGetError(), GL_INVALID_OPERATION);
    }
    assert_eq!(px, [0xdead_beef; 4], "nothing was written");
}

/// A framebuffer whose colour attachment was taken away is not complete, and
/// a read of it is GL_INVALID_FRAMEBUFFER_OPERATION through RGBA /
/// UNSIGNED_BYTE; through an integer pair today's GL core answers
/// GL_INVALID_OPERATION, the pair checked against the absent buffer (taken
/// for a fixed-point one) before completeness.
#[test]
fn an_incomplete_framebuffer_is_refused() {
    let _g = gl_rig::rig();
    let _fb = Fb::new(&[INTS[13]]);
    let mut px = [0xdead_beefu32; 4];
    // SAFETY: one pixel of four words; plain GL calls.
    unsafe {
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, 0);
        assert_ne!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        assert_eq!(glGetError(), GL_INVALID_FRAMEBUFFER_OPERATION);
        glReadPixels(0, 0, 1, 1, GL_RGBA_INTEGER, GL_UNSIGNED_INT, px.as_mut_ptr() as *mut c_void);
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "today: the pair before completeness");
    }
    assert_eq!(px, [0xdead_beef; 4], "nothing was written");
}

/// ES 3.0 4.3.1: reading an incomplete framebuffer is
/// GL_INVALID_FRAMEBUFFER_OPERATION whatever the pair.
/// Cause: src/gl/framebuffer.rs `read_pixels` checks (format, type) against
/// `fb_target_format` before `target_loc` reports the framebuffer incomplete.
#[test]
#[ignore = "GL core: read_pixels checks the pair before the framebuffer's completeness (src/gl/framebuffer.rs, plan #186)"]
fn an_incomplete_framebuffer_is_invalid_framebuffer_operation_whatever_the_pair() {
    let _g = gl_rig::rig();
    let _fb = Fb::new(&[INTS[13]]);
    let mut px = [0u32; 4];
    // SAFETY: one pixel of four words; plain GL calls.
    unsafe {
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, 0);
        glReadPixels(0, 0, 1, 1, GL_RGBA_INTEGER, GL_UNSIGNED_INT, px.as_mut_ptr() as *mut c_void);
        assert_eq!(glGetError(), GL_INVALID_FRAMEBUFFER_OPERATION);
    }
}

/* ---- the float formats without renderbuffer storage ------------------------- */

/// RG32F, RGB32F, RGBA32F and R11F_G11F_B10F are no renderbuffer storage
/// here (ES 3.0 table 3.13 makes no float format colour-renderable, and
/// GL_EXT_color_buffer_float is not claimed), so there is no attachment of
/// them to read: the storage call is GL_INVALID_ENUM and a framebuffer on it
/// is not complete. The dEQP cases that render into them are not-pass for
/// this reason, not for a readback one.
#[test]
fn the_other_float_formats_are_no_renderbuffer_storage() {
    let _g = gl_rig::rig();
    for (gl, name) in
        [(GL_RG32F, "RG32F"), (GL_RGB32F, "RGB32F"), (GL_RGBA32F, "RGBA32F"), (GL_R11F_G11F_B10F, "R11F_G11F_B10F")]
    {
        let (mut rb, mut fbo) = (0, 0);
        // SAFETY: plain GL calls with room for one name each.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, gl, W, H);
            assert_eq!(glGetError(), GL_INVALID_ENUM, "{name}: renderbuffer storage");
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            assert_ne!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{name}");
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fbo);
            glDeleteRenderbuffers(1, &rb);
            while glGetError() != GL_NO_ERROR {}
        }
    }
}

/* ---- the tables ---------------------------------------------------------------- */

/// The integer table is the 19 sized integer colour formats of ES 3.0 and
/// the float one the five of `is_renderbuffer_color_format`, each named once.
#[test]
fn the_tables_hold_every_format_once() {
    let mut names: Vec<_> = INTS.iter().chain(FLOATS.iter()).map(|f| f.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 24);
    assert_eq!(INTS.iter().filter(|f| f.class == Class::Int).count(), 9);
    assert_eq!(INTS.iter().filter(|f| f.class == Class::Uint).count(), 10);
}

/* ---- texture attachments ------------------------------------------------------ */

/// A W x H texture of `f` (level 0, NEAREST filters) as colour attachment 0
/// of a new framebuffer, bound; the two names.
fn texture_fb(f: &Fmt) -> (GLuint, GLuint) {
    use yakogl::gl::texture::*;
    let int = f.class != Class::Float;
    let base = match (f.channels, int) {
        (1, true) => GL_RED_INTEGER,
        (2, true) => GL_RG_INTEGER,
        (_, true) => GL_RGBA_INTEGER,
        (1, false) => GL_RED,
        (2, false) => GL_RG,
        (3, false) => GL_RGB,
        _ => GL_RGBA,
    };
    let ty = match (f.class, f.bits) {
        (Class::Int, 8) => GL_BYTE,
        (Class::Int, 16) => GL_SHORT,
        (Class::Int, _) => GL_INT,
        (Class::Uint, 8) => GL_UNSIGNED_BYTE,
        (Class::Uint, 16) => GL_UNSIGNED_SHORT,
        (Class::Uint, 10) => GL_UNSIGNED_INT_2_10_10_10_REV,
        (Class::Uint, _) => GL_UNSIGNED_INT,
        (Class::Float, 16) => GL_HALF_FLOAT,
        (Class::Float, _) => GL_FLOAT,
    };
    let (mut tex, mut fbo) = (0, 0);
    // SAFETY: plain GL calls with room for one name each; null pixels only allocate.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, f.gl as GLint, W, H, 0, base, ty, core::ptr::null());
        assert_eq!(glGetError(), GL_NO_ERROR, "{}: the texture", f.name);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        glViewport(0, 0, W, H);
    }
    (tex, fbo)
}

fn release_texture_fb(tex: GLuint, fbo: GLuint) {
    use yakogl::gl::texture::glDeleteTextures;
    // SAFETY: names `texture_fb` made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
    }
}

/// A texture attachment reads its clear as the renderbuffer does: the
/// four-channel integer formats, RGB10_A2UI, R16F, RG16F (its blue the
/// clear's, as above), RGBA16F and R32F, through the class's pair and the
/// implementation's.
#[test]
fn texture_attachments_read_their_clear_the_same() {
    let _g = gl_rig::rig();
    let complete: Vec<&Fmt> =
        INTS.iter().filter(|f| f.channels == 4).chain(FLOATS.iter().filter(|f| f.channels != 3)).collect();
    assert_eq!(complete.len(), 11);
    for f in complete {
        let (tex, fbo) = texture_fb(f);
        // SAFETY: a plain GL call.
        assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_COMPLETE, "{}", f.name);
        f.clear(0);
        assert_eq!(gl_rig::err(), GL_NO_ERROR, "{}: the clear", f.name);
        let (format, ty) = f.pair();
        let (e, px) = read(format, ty);
        assert_eq!(e, GL_NO_ERROR, "{}: {:?}", f.name, gl_rig::logs());
        let want = if f.class == Class::Float { float_read_today(f).map(f32::to_bits) } else { f.expect() };
        for (i, p) in px.chunks(4).enumerate() {
            assert_eq!(p, want, "{} texture pixel {i}", f.name);
        }
        let pair = implementation_pair();
        assert_eq!(read_error(pair.0, pair.1), GL_NO_ERROR, "{}: the implementation's pair", f.name);
        nothing_refused(f.name);
        release_texture_fb(tex, fbo);
    }
}

/// The one- and two-channel integer textures and the RGB16F texture are
/// GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT today (the GL core admits only the
/// four-channel integer classes and not GL_RGB16F as a texture attachment:
/// pinned, with the expected results, in tests/es3_narrow_int_rt_v3d.rs and
/// tests/es3_rgb16f_attach_v3d.rs), so no readback of them begins: the read
/// is refused (the pair check or completeness, see
/// `an_incomplete_framebuffer_is_refused`) and writes nothing.
#[test]
fn narrow_integer_and_rgb16f_textures_are_incomplete_today() {
    let _g = gl_rig::rig();
    let incomplete: Vec<&Fmt> =
        INTS.iter().filter(|f| f.channels < 4).chain(FLOATS.iter().filter(|f| f.channels == 3)).collect();
    assert_eq!(incomplete.len(), 13);
    for f in incomplete {
        let (tex, fbo) = texture_fb(f);
        // SAFETY: a plain GL call.
        let status = unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) };
        assert_eq!(status, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{}", f.name);
        let (format, ty) = f.pair();
        let (e, px) = read(format, ty);
        assert!(matches!(e, GL_INVALID_OPERATION | GL_INVALID_FRAMEBUFFER_OPERATION), "{}: {e:#x}", f.name);
        assert!(px.iter().all(|w| *w == 0xdead_beef), "{}: nothing is written", f.name);
        release_texture_fb(tex, fbo);
    }
}
