// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES3 `pbo.renderbuffer.<format>_clears` on the host, through the GL
//! API over the real V3D back end (the job builder and the GPU stood in for,
//! as tests/es3_formats_v3d.rs' `gl_rig` does): for every ES 3.0
//! colour-renderable sized renderbuffer format, a renderbuffer, several
//! scissored glClearBuffer clears, glReadPixels into a PIXEL_PACK_BUFFER, a
//! map and a compare. Pi 4 image M1 stopped at `rg16i_clears` (a
//! GL_OUT_OF_MEMORY that aborts dEQP; docs/es3/pi4-gles3-fails.tsv).
//!
//! The stand-in GPU applies the job's full-target clear words and nothing
//! else: a scissored clear is a draw on the V3D (the clear quad), which no
//! interpreter runs here. So the pixels of a scissored clear are not
//! judged; what is held is what the GL API gives the application at each
//! step (errors, framebuffer status, the back end's refusals by name in the
//! platform log) and what the back end handed the job builder.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_pbo_rb_clears_v3d
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_pbo_rb_clears_v3d");
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

/// The real packer: the 24 bytes of TEXTURE_SHADER_STATE, or None when it
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
        if !t.typed && !t.rgb565 && !c.load_rt0 && t.address != 0 && t.stride as usize >= t.width as usize * 4 {
            /* the ES 2.0 job: RGBA8 words of `rgba8` */
            with_platform(|p| {
                let row = t.stride as usize;
                let base = platform_cpu(p, u64::from(t.address), row * t.height as usize);
                for y in 0..t.height as usize {
                    for x in 0..t.width as usize {
                        // SAFETY: inside the span platform_cpu checked.
                        unsafe {
                            core::ptr::copy_nonoverlapping(c.rgba8.to_le_bytes().as_ptr(), base.add(y * row + x * 4), 4)
                        };
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
                /* the integer rows: the clear words' bytes, as many as the texel has */
                V3D_OUTPUT_IMAGE_FORMAT_R8I | V3D_OUTPUT_IMAGE_FORMAT_R8UI => w[0].to_le_bytes()[..1].to_vec(),
                V3D_OUTPUT_IMAGE_FORMAT_RG8I | V3D_OUTPUT_IMAGE_FORMAT_RG8UI => w[0].to_le_bytes()[..2].to_vec(),
                V3D_OUTPUT_IMAGE_FORMAT_RGBA8I | V3D_OUTPUT_IMAGE_FORMAT_RGBA8UI => w[0].to_le_bytes().to_vec(),
                V3D_OUTPUT_IMAGE_FORMAT_R16I | V3D_OUTPUT_IMAGE_FORMAT_R16UI => w[0].to_le_bytes()[..2].to_vec(),
                V3D_OUTPUT_IMAGE_FORMAT_RG16I | V3D_OUTPUT_IMAGE_FORMAT_RG16UI => w[0].to_le_bytes().to_vec(),
                V3D_OUTPUT_IMAGE_FORMAT_RGBA16I | V3D_OUTPUT_IMAGE_FORMAT_RGBA16UI => {
                    w[..2].iter().flat_map(|x| x.to_le_bytes()).collect()
                }
                V3D_OUTPUT_IMAGE_FORMAT_R32I | V3D_OUTPUT_IMAGE_FORMAT_R32UI => w[0].to_le_bytes().to_vec(),
                V3D_OUTPUT_IMAGE_FORMAT_RG32I | V3D_OUTPUT_IMAGE_FORMAT_RG32UI => {
                    w[..2].iter().flat_map(|x| x.to_le_bytes()).collect()
                }
                V3D_OUTPUT_IMAGE_FORMAT_RGBA32I | V3D_OUTPUT_IMAGE_FORMAT_RGBA32UI => {
                    w.iter().flat_map(|x| x.to_le_bytes()).collect()
                }
                V3D_OUTPUT_IMAGE_FORMAT_RGB10_A2UI => {
                    let (r, g, b, a) = (w[0] & 0x3ff, w[0] >> 16 & 0x3ff, w[1] & 0x3ff, w[1] >> 16 & 3);
                    (r | g << 10 | b << 20 | a << 30).to_le_bytes().to_vec()
                }
                V3D_OUTPUT_IMAGE_FORMAT_RGB10_A2 => {
                    /* the 16F words of the clear, to the 10/10/10/2 unorm the TLB stores */
                    let half = |h: u32| half_to_f32(h).clamp(0.0, 1.0);
                    let q = |h: u32, n: f32| (half(h) * n + 0.5) as u32;
                    let (r, g, b, a) =
                        (q(w[0] & 0xffff, 1023.0), q(w[0] >> 16, 1023.0), q(w[1] & 0xffff, 1023.0), q(w[1] >> 16, 3.0));
                    (r | g << 10 | b << 20 | a << 30).to_le_bytes().to_vec()
                }
                V3D_OUTPUT_IMAGE_FORMAT_RGBA8 => w[0].to_le_bytes().to_vec(),
                V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8 => {
                    /* 16F words of the linear clear; the TLB's store encodes red, green and blue */
                    let h = [w[0] & 0xffff, w[0] >> 16, w[1] & 0xffff, w[1] >> 16].map(half_to_f32);
                    let un = |f: f32| (f * 255.0 + 0.5) as u8;
                    vec![
                        yakogl::gl::srgb::encode(h[0]),
                        yakogl::gl::srgb::encode(h[1]),
                        yakogl::gl::srgb::encode(h[2]),
                        un(h[3]),
                    ]
                }
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

    fn half_to_f32(h: u32) -> f32 {
        let (e, m) = ((h >> 10) & 31, h & 0x3ff);
        if e == 0 {
            m as f32 / 1024.0 / 16384.0
        } else {
            (1.0 + m as f32 / 1024.0) * 2f32.powi(e as i32 - 15)
        }
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

    /* ---- pbo.renderbuffer.<format>_clears ----------------------------------- */

    use yakogl::gl::buffer::*;
    use yakogl::gl::es3::*;

    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Class {
        /// Fixed point: read as RGBA / UNSIGNED_BYTE, cleared with glClearBufferfv.
        Norm,
        /// Fixed point 10/10/10/2, read as RGBA / UNSIGNED_INT_2_10_10_10_REV.
        Packed,
        Int,
        Uint,
    }

    /// (name as dEQP, internal format, class, channels that exist, bits a channel)
    const FORMATS: &[(&str, GLenum, Class, usize, u32)] = &[
        ("r8", GL_R8, Class::Norm, 1, 8),
        ("rg8", GL_RG8, Class::Norm, 2, 8),
        ("rgb8", GL_RGB8, Class::Norm, 3, 8),
        ("rgb565", GL_RGB565, Class::Norm, 3, 5),
        ("rgba4", GL_RGBA4, Class::Norm, 4, 4),
        ("rgb5_a1", GL_RGB5_A1, Class::Norm, 4, 5),
        ("rgba8", GL_RGBA8, Class::Norm, 4, 8),
        ("srgb8_alpha8", GL_SRGB8_ALPHA8, Class::Norm, 4, 8),
        ("rgb10_a2", GL_RGB10_A2, Class::Packed, 4, 10),
        ("rgb10_a2_ubyte", GL_RGB10_A2, Class::Norm, 4, 10),
        ("rgb10_a2ui", GL_RGB10_A2UI, Class::Uint, 4, 10),
        ("r8i", GL_R8I, Class::Int, 1, 8),
        ("r8ui", GL_R8UI, Class::Uint, 1, 8),
        ("r16i", GL_R16I, Class::Int, 1, 16),
        ("r16ui", GL_R16UI, Class::Uint, 1, 16),
        ("r32i", GL_R32I, Class::Int, 1, 32),
        ("r32ui", GL_R32UI, Class::Uint, 1, 32),
        ("rg8i", GL_RG8I, Class::Int, 2, 8),
        ("rg8ui", GL_RG8UI, Class::Uint, 2, 8),
        ("rg16i", GL_RG16I, Class::Int, 2, 16),
        ("rg16ui", GL_RG16UI, Class::Uint, 2, 16),
        ("rg32i", GL_RG32I, Class::Int, 2, 32),
        ("rg32ui", GL_RG32UI, Class::Uint, 2, 32),
        ("rgba8i", GL_RGBA8I, Class::Int, 4, 8),
        ("rgba8ui", GL_RGBA8UI, Class::Uint, 4, 8),
        ("rgba16i", GL_RGBA16I, Class::Int, 4, 16),
        ("rgba16ui", GL_RGBA16UI, Class::Uint, 4, 16),
        ("rgba32i", GL_RGBA32I, Class::Int, 4, 32),
        ("rgba32ui", GL_RGBA32UI, Class::Uint, 4, 32),
    ];

    const W: i32 = 16;
    const H: i32 = 16;
    /// Scissored clears, in order: (x, y, w, h, value index).
    const RECTS: [(i32, i32, i32, i32); 3] = [(1, 2, 5, 4), (8, 3, 6, 9), (2, 10, 11, 5)];

    #[derive(Clone, PartialEq, Debug)]
    enum Outcome {
        Pass,
        /// The framebuffer is not complete (the status).
        Incomplete(GLenum),
        /// A GL error at a step, with the platform log lines the step added.
        Error(&'static str, GLenum, Vec<String>),
        /// The pixels outside every scissored clear are not the full clear's.
        Wrong(String),
    }

    /// Bits of channel `c` of a format of `bits` a channel: RGB10_A2's alpha is 2.
    fn chbits(bits: u32, c: usize) -> u32 {
        if bits == 10 && c == 3 {
            2
        } else {
            bits
        }
    }

    /// Channel values of clear number `n` (0 the full clear), within the format's range.
    fn value(class: Class, bits: u32, n: usize) -> [i64; 4] {
        let pick = |c: usize| {
            let b = chbits(bits, c).min(32);
            let max: i64 = if class == Class::Int { (1i64 << (b - 1)) - 1 } else { (1i64 << b) - 1 };
            let min = if class == Class::Int { -max - 1 } else { 0 };
            match (n + c) % 4 {
                0 => max / 3,
                1 => max / 5 + 1,
                2 => {
                    if class == Class::Int {
                        min / 2
                    } else {
                        max / 7
                    }
                }
                _ => max,
            }
        };
        [pick(0), pick(1), pick(2), pick(3)]
    }

    fn clear(class: Class, bits: u32, n: usize) {
        let v = value(class, bits, n);
        // SAFETY: a plain GL call with a four-element array.
        unsafe {
            match class {
                Class::Norm | Class::Packed => {
                    let f: [f32; 4] = core::array::from_fn(|c| v[c] as f32 / ((1i64 << chbits(bits, c)) - 1) as f32);
                    glClearBufferfv(GL_COLOR, 0, f.as_ptr());
                }
                Class::Int => {
                    let i = v.map(|x| x as i32);
                    glClearBufferiv(GL_COLOR, 0, i.as_ptr());
                }
                Class::Uint => {
                    let u = v.map(|x| x as u32);
                    glClearBufferuiv(GL_COLOR, 0, u.as_ptr());
                }
            }
        }
    }

    fn run_format(internal: GLenum, class: Class, chans: usize, bits: u32) -> Outcome {
        let (mut rb, mut fbo, mut pbo) = (0u32, 0u32, 0u32);
        let (read_fmt, read_ty, unit) = match class {
            Class::Norm => (GL_RGBA, GL_UNSIGNED_BYTE, 1usize),
            Class::Packed => (GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV, 1),
            Class::Int => (GL_RGBA_INTEGER, GL_INT, 4),
            Class::Uint => (GL_RGBA_INTEGER, GL_UNSIGNED_INT, 4),
        };
        let bytes = (W * H) as usize * 4 * unit;
        let log0 = logs().len();
        let fail = |step: &'static str| {
            // SAFETY: a plain GL call.
            let e = unsafe { glGetError() };
            (e != GL_NO_ERROR).then(|| Outcome::Error(step, e, logs()[log0..].to_vec()))
        };
        // SAFETY: plain GL calls on names made here; `out` is `bytes` long and the
        // map is read before the unmap.
        let outcome = unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, internal, W, H);
            if let Some(o) = fail("glRenderbufferStorage") {
                return o;
            }
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
            if status != GL_FRAMEBUFFER_COMPLETE {
                return Outcome::Incomplete(status);
            }
            glViewport(0, 0, W, H);
            glDisable(GL_SCISSOR_TEST);
            clear(class, bits, 0);
            if let Some(o) = fail("glClearBuffer (full)") {
                return o;
            }
            glEnable(GL_SCISSOR_TEST);
            for (i, (x, y, w, h)) in RECTS.iter().enumerate() {
                glScissor(*x, *y, *w, *h);
                clear(class, bits, i + 1);
                if let Some(o) = fail("glClearBuffer (scissored)") {
                    return o;
                }
            }
            glDisable(GL_SCISSOR_TEST);
            glGenBuffers(1, &mut pbo);
            glBindBuffer(GL_PIXEL_PACK_BUFFER, pbo);
            glBufferData(GL_PIXEL_PACK_BUFFER, bytes as isize, core::ptr::null(), GL_STREAM_READ);
            if let Some(o) = fail("glBufferData") {
                return o;
            }
            glReadPixels(0, 0, W, H, read_fmt, read_ty, core::ptr::null_mut());
            if let Some(o) = fail("glReadPixels") {
                return o;
            }
            let p = glMapBufferRange(GL_PIXEL_PACK_BUFFER, 0, bytes as isize, GL_MAP_READ_BIT) as *const u8;
            if let Some(o) = fail("glMapBufferRange") {
                return o;
            }
            assert!(!p.is_null(), "{internal:#x}: map returned null without an error");
            let out = std::slice::from_raw_parts(p, bytes).to_vec();
            glUnmapBuffer(GL_PIXEL_PACK_BUFFER);
            if let Some(o) = fail("glUnmapBuffer") {
                return o;
            }
            judge(&out, internal, class, chans, bits, unit)
        };
        // SAFETY: names made above.
        unsafe {
            glBindBuffer(GL_PIXEL_PACK_BUFFER, 0);
            glDeleteBuffers(1, &pbo);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fbo);
            glDeleteRenderbuffers(1, &rb);
        }
        outcome
    }

    /// Only the pixels no scissored clear touched are judged: they must hold
    /// the full clear (the stand-in GPU runs no clear-quad draw).
    fn judge(out: &[u8], internal: GLenum, class: Class, chans: usize, bits: u32, unit: usize) -> Outcome {
        let v = value(class, bits, 0);
        for y in 0..H {
            for x in 0..W {
                if RECTS.iter().any(|&(rx, ry, rw, rh)| x >= rx && x < rx + rw && y >= ry && y < ry + rh) {
                    continue;
                }
                /* only the channels the format has: a missing one is the GL core's
                 * to mask, and dEQP's reference ignores it too */
                #[allow(clippy::needless_range_loop)]
                for c in 0..chans {
                    let at = ((y * W + x) as usize * 4 + c) * unit;
                    let got: i64 = match class {
                        Class::Norm => i64::from(out[at]),
                        Class::Packed => {
                            let w = u32::from_le_bytes(out[(y * W + x) as usize * 4..][..4].try_into().unwrap());
                            i64::from(w >> (10 * c as u32) & if c == 3 { 3 } else { 0x3ff })
                        }
                        Class::Int => i64::from(i32::from_le_bytes(out[at..at + 4].try_into().unwrap())),
                        Class::Uint => i64::from(u32::from_le_bytes(out[at..at + 4].try_into().unwrap())),
                    };
                    let m = (1i64 << chbits(bits, c)) - 1;
                    let (want, tol) = match class {
                        Class::Norm if internal == GL_SRGB8_ALPHA8 && c < 3 => {
                            /* the TLB encodes the linear clear on the store */
                            (i64::from(yakogl::gl::srgb::encode(v[c] as f32 / m as f32)), 1)
                        }
                        /* sub-8-bit and 10-bit channels land within a step of the byte */
                        Class::Norm => ((v[c] * 255 + m / 2) / m, 255 / m / 2 + 1),
                        Class::Packed => (v[c], 1),
                        _ => (v[c], 0),
                    };
                    if (got - want).abs() > tol {
                        return Outcome::Wrong(format!(
                            "({x},{y}) channel {c}: {got}, the full clear is {want}; pixel {:?}",
                            &out[(y * W + x) as usize * 4 * unit..][..4 * unit]
                        ));
                    }
                }
            }
        }
        Outcome::Pass
    }

    /// Today's outcome of every format, by dEQP name: everything passes but
    /// RGB10_A2, whose two reads the GL core gets wrong (see the ignored
    /// tests below).
    fn pinned(name: &str) -> Outcome {
        match name {
            "rgb10_a2" => {
                Outcome::Error("glReadPixels", GL_INVALID_OPERATION, vec!["GL error 0x502 in glReadPixels".into()])
            }
            "rgb10_a2_ubyte" => {
                Outcome::Wrong("(0,0) channel 1: 53, the full clear is 51; pixel [85, 53, 35, 201]".into())
            }
            _ => Outcome::Pass,
        }
    }

    fn outcome_of(name: &str) -> Outcome {
        let _g = rig();
        let &(_, internal, class, chans, bits) = FORMATS.iter().find(|f| f.0 == name).expect("a format of the table");
        let o = run_format(internal, class, chans, bits);
        /* whatever the outcome: never an out-of-memory (the error that aborts dEQP),
         * and the back end refuses nothing by name */
        if let Outcome::Error(_, e, _) = &o {
            assert_ne!(*e, GL_OUT_OF_MEMORY, "{name}");
        }
        let refused: Vec<_> = logs().into_iter().filter(|l| l.contains("refus")).collect();
        assert!(refused.is_empty(), "{name}: {refused:?}");
        // SAFETY: a plain GL call.
        unsafe { while glGetError() != GL_NO_ERROR {} }
        o
    }

    /// The pbo.renderbuffer.<format>_clears sequence of every ES 3.0
    /// colour-renderable sized format, each as today's tip does it.
    #[test]
    fn every_format_is_as_pinned() {
        let mut bad = Vec::new();
        for &(name, ..) in FORMATS {
            let got = outcome_of(name);
            if got != pinned(name) {
                bad.push(format!("{name}: {got:?}"));
            }
        }
        assert!(bad.is_empty(), "{bad:#?}");
    }

    /// The table holds the 28 formats of the group (RGB10_A2 read twice).
    #[test]
    fn the_table_is_the_es3_renderbuffer_formats() {
        assert_eq!(FORMATS.len(), 29);
        let mut names: Vec<_> = FORMATS.iter().map(|f| f.0).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 29);
    }

    /// Fix: RGB10_A2 reads as RGBA / UNSIGNED_INT_2_10_10_10_REV, the format
    /// and type ES 3.0 4.3.2 gives the implementation-chosen read of the
    /// class, and glReadPixels takes them (today GL_INVALID_OPERATION, a GL
    /// core refusal, src/gl/framebuffer.rs).
    #[test]
    #[ignore = "GL core: glReadPixels of RGB10_A2 with UNSIGNED_INT_2_10_10_10_REV is GL_INVALID_OPERATION"]
    fn rgb10_a2_reads_as_the_packed_word() {
        assert_eq!(outcome_of("rgb10_a2"), Outcome::Pass);
    }

    /// Fix: RGBA / UNSIGNED_BYTE of an RGB10_A2 buffer is each 10-bit field
    /// scaled to a byte (ES 3.0 4.3.2); today the buffer's four raw bytes come back.
    #[test]
    #[ignore = "GL core: glReadPixels RGBA/UNSIGNED_BYTE of RGB10_A2 returns the packed word's bytes, not the fields"]
    fn rgb10_a2_reads_as_bytes() {
        assert_eq!(outcome_of("rgb10_a2_ubyte"), Outcome::Pass);
    }
}
