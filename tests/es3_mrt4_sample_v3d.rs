// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Sampling a texture while a different texture is COLOR_ATTACHMENT3 on V3D
//! (item loop4-v3d, plan #368 RC14): feedback-free four-target draws.
//!
//! Frame 1 renders four targets into the textures T0..T3 of one framebuffer
//! (a `glClearBuffer*` of each, then a four-output draw). Frame 2 binds T3 (or
//! T0) as a `sampler2D` and draws into a second four-target framebuffer, every
//! output a function of `texture(s, uv)`. The case where the sampled texture is
//! also attached to the draw framebuffer is a feedback loop, undefined in
//! ES 3.0, and is left out.
//!
//! What the rig can see: the GL core and the V3D back end are real, the job
//! builder (v3d_job.c) and the GPU are the stand-ins of tests/es3_mrt4_v3d.rs.
//! The stand-in GPU applies a job's clear words to every render target's
//! store at the kick and runs no shader, so the pixels a frame-2 shader would
//! write are not observable. What is pinned instead:
//!
//! - the job order: frame 1's job (four render targets, a draw, RT 3 stored
//!   to T3's image) is begun and kicked before frame 2's job is begun, with no
//!   `glFinish` in between;
//! - at the moment frame 2's draw reaches the job builder, the texel at the
//!   sampled texture's base pointer, read from the unit's TMU texture record
//!   (`V3dJobTmu::tex_state`, base address above the flag bits 0..5), is
//!   already the clear colour frame 1's RT 3 (or RT 0) store left there: the
//!   store happened before the sample;
//! - the sampled image is none of frame 2's four stores (a texture's image is
//!   its own allocation, filled from the raster store, so its base is not the
//!   store's address), frame 2's four stores are four distinct images none of
//!   which frame 1 wrote, and both jobs configure four render targets.
//!
//! Real pixels, and the flush or dependency between the jobs on the GPU, are
//! the board's (gl_test case mrt4 and the dEQP rows of RC14).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_sample_v3d
//! ```
#![cfg(all(
    feature = "host-test",
    feature = "gpu-v3d",
    feature = "shaderc-online",
    feature = "no-shader-table",
    not(feature = "report-es2")
))]
#![allow(dead_code)]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use yakogl::v3d::ffi::*;

/// The draw-target count, through the host stand-in `backend::HOST_DRAW_TARGETS`
/// (the one place; the knob item's setter would replace this body).
fn set_draw_targets(n: u8) {
    yakogl::backend::HOST_DRAW_TARGETS.store(n, core::sync::atomic::Ordering::Relaxed);
}

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

    /// One draw as the job builder sees it: each sampler binding with the base
    /// pointer of its texture record and the first texel there at that moment.
    #[derive(Clone, Debug)]
    pub struct SeenDraw {
        pub tmu: Vec<(V3dJobTmu, u32, [u8; 4])>,
        /// Jobs submitted so far (kicks), when the draw reached the builder.
        pub kicks: usize,
    }

    #[derive(Clone, Debug)]
    pub struct SeenJob {
        pub target: V3dJobTarget,
        pub clear: Option<V3dJobClear>,
        pub draws: u32,
        pub seen: Vec<SeenDraw>,
        /// Jobs submitted when this one was begun.
        pub kicks_at_begin: usize,
    }

    static JOBS: Mutex<Vec<SeenJob>> = Mutex::new(Vec::new());

    pub fn jobs() -> MutexGuard<'static, Vec<SeenJob>> {
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
        let kicks_at_begin = with_platform(|p| p.kicks.len());
        jobs().push(SeenJob { target: t, clear: None, draws: 0, seen: Vec::new(), kicks_at_begin });
        t.width != 0 && t.height != 0
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
        // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
        let (j, d) = unsafe { (&mut *j, &*d) };
        let tmus: &[V3dJobTmu] =
            if d.tmu.is_null() { &[] } else { unsafe { std::slice::from_raw_parts(d.tmu, d.num_tmu as usize) } };
        let tmu = tmus
            .iter()
            .map(|u| {
                let (base, texel) = with_platform(|p| {
                    let gpu = u64::from(u.tex_state);
                    let rec = platform_cpu(p, gpu, V3D_TEX_STATE_BYTES);
                    let mut w = [0u8; 4];
                    // SAFETY: inside the record (platform_cpu checked the span).
                    unsafe { core::ptr::copy_nonoverlapping(rec, w.as_mut_ptr(), 4) };
                    let base = u32::from_le_bytes(w) & !63;
                    let t = platform_cpu(p, u64::from(base), 4);
                    let mut texel = [0u8; 4];
                    // SAFETY: inside the image (platform_cpu checked the span).
                    unsafe { core::ptr::copy_nonoverlapping(t, texel.as_mut_ptr(), 4) };
                    (base, texel)
                });
                (*u, base, texel)
            })
            .collect();
        let kicks = with_platform(|p| p.kicks.len());
        j.draws += 1;
        // SAFETY: the job's BCL is live.
        unsafe { (*j.bcl).used += 120 };
        if let Some(job) = jobs().last_mut() {
            job.draws += 1;
            job.seen.push(SeenDraw { tmu, kicks });
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
        if !t.typed && !t.rgb565 {
            /* the ES 2.0 job's RTs 1..3: RGBA8 words of `rgba8_rt` (RGBA8 targets only) */
            for n in 1..t.rts.min(4) as usize {
                let rt = t.rt[n - 1];
                if !c.load_rt[n - 1] && rt.address != 0 {
                    fill(rt.address, rt.stride, t.width, t.height, &c.rgba8_rt[n - 1].to_le_bytes());
                }
            }
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
        super::set_draw_targets(4);
        g
    }

    pub fn logs() -> Vec<String> {
        with_platform(|p| p.log.clone())
    }

    pub fn err() -> GLenum {
        // SAFETY: a plain GL call.
        unsafe { glGetError() }
    }

    /* ---- framebuffers of four textures ------------------------------------------ */

    pub const W: GLsizei = 8;
    pub const H: GLsizei = 4;

    /// A complete framebuffer of four W x H RGBA8 textures (NEAREST, one
    /// level) on colour attachments 0..3, bound, all four draw buffers.
    pub struct TexFb {
        pub fbo: GLuint,
        pub tex: [GLuint; 4],
    }

    impl TexFb {
        pub fn new() -> TexFb {
            let mut fbo = 0;
            let mut tex = [0; 4];
            // SAFETY: plain GL calls with room for the names.
            unsafe {
                glGenFramebuffers(1, &mut fbo);
                glBindFramebuffer(GL_FRAMEBUFFER, fbo);
                glGenTextures(4, tex.as_mut_ptr());
                for (n, t) in tex.iter().enumerate() {
                    glBindTexture(GL_TEXTURE_2D, *t);
                    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
                    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
                    glTexImage2D(
                        GL_TEXTURE_2D,
                        0,
                        GL_RGBA8 as GLint,
                        W,
                        H,
                        0,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        core::ptr::null(),
                    );
                    glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + n as u32, GL_TEXTURE_2D, *t, 0);
                }
                assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
                assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
                let bufs: Vec<GLenum> = (0..4).map(|n| GL_COLOR_ATTACHMENT0 + n).collect();
                glDrawBuffers(4, bufs.as_ptr());
                glViewport(0, 0, W, H);
                glDisable(GL_SCISSOR_TEST);
                glColorMask(1, 1, 1, 1);
            }
            TexFb { fbo, tex }
        }

        pub fn bind(&self) {
            // SAFETY: a plain GL call.
            unsafe { glBindFramebuffer(GL_FRAMEBUFFER, self.fbo) };
        }
    }

    impl Drop for TexFb {
        fn drop(&mut self) {
            // SAFETY: names this framebuffer made.
            unsafe {
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                glDeleteFramebuffers(1, &self.fbo);
                glDeleteTextures(4, self.tex.as_ptr());
            }
        }
    }

    /// The clear colour of attachment `n`: none equals another.
    pub fn colour(n: usize) -> [f32; 4] {
        [(n as f32 + 1.0) * 0.2, 1.0 - n as f32 * 0.25, 0.5, 1.0]
    }

    /// The bytes RGBA / UNSIGNED_BYTE gives for `colour(n)`.
    pub fn bytes(n: usize) -> [u8; 4] {
        colour(n).map(|v| (v * 255.0 + 0.5) as u8)
    }
}

use gl_rig::*;
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};

fn compile(ty: GLenum, text: &str) -> GLuint {
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile: {:?}", logs());
        s
    }
}

fn link(vs: &str, fs: &str) -> GLuint {
    let v = compile(GL_VERTEX_SHADER, vs);
    let f = compile(GL_FRAGMENT_SHADER, fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{:?}", logs());
        p
    }
}

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

/// Frame 1's shader: four constant outputs.
const FS_WRITE: &str = "#version 300 es\nprecision highp float;\n\
    layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\n\
    layout(location = 2) out vec4 o2;\nlayout(location = 3) out vec4 o3;\n\
    void main() { o0 = vec4(0.1, 0.2, 0.3, 1.0); o1 = vec4(0.2, 0.3, 0.4, 1.0);\n\
    o2 = vec4(0.3, 0.4, 0.5, 1.0); o3 = vec4(0.4, 0.5, 0.6, 1.0); }\n";

/// Frame 2's shader: each of four outputs a function of the one texel.
const FS_SAMPLE: &str = "#version 300 es\nprecision highp float;\nuniform sampler2D s;\n\
    layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\n\
    layout(location = 2) out vec4 o2;\nlayout(location = 3) out vec4 o3;\n\
    void main() { vec4 t = texture(s, vec2(0.5, 0.5));\n\
    o0 = t; o1 = t.bgra; o2 = vec4(1.0) - t; o3 = t * 0.5; }\n";

fn draw_triangle() {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
}

fn mark() -> usize {
    jobs().len()
}

/// Frame 1 into `a`: glClearBufferfv of the four attachments to `colour(n)`
/// and a four-output draw, one job; NO glFinish. The job it began.
fn frame1(a: &TexFb) -> SeenJob {
    a.bind();
    let m = mark();
    let p = link(VS, FS_WRITE);
    for n in 0..4 {
        // SAFETY: a four-element array.
        unsafe { glClearBufferfv(GL_COLOR, n as GLint, colour(n).as_ptr()) };
    }
    // SAFETY: a plain GL call.
    unsafe { glUseProgram(p) };
    draw_triangle();
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    /* the draw's job is the one the next bind submits: end it */
    // SAFETY: a plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    let js: Vec<SeenJob> = jobs()[m..].to_vec();
    js.iter().rev().find(|j| j.draws > 0).cloned().expect("frame 1's job with the draw")
}

/// Frame 2: `src` bound on unit 0 as `s`, four outputs into `b`; no
/// glFinish before it. The job it began, with its draw.
fn frame2(src: GLuint, b: &TexFb) -> SeenJob {
    let m = mark();
    let p = link(VS, FS_SAMPLE);
    b.bind();
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(p);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, src);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
    }
    draw_triangle();
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    let js: Vec<SeenJob> = jobs()[m..].to_vec();
    js.iter().find(|j| j.draws > 0).cloned().expect("frame 2's job with the draw")
}

/// The four stores of a four-target job: RT 0's image and RT 1..3's rasters.
fn stores(j: &SeenJob) -> [u32; 4] {
    [j.target.address, j.target.rt[0].address, j.target.rt[1].address, j.target.rt[2].address]
}

fn check_frame(j1: &SeenJob, j2: &SeenJob, sampled: usize) {
    assert_eq!(j1.target.rts, 4, "frame 1 configures four render targets");
    assert_eq!(j2.target.rts, 4, "frame 2 configures four render targets");
    let (s1, s2) = (stores(j1), stores(j2));
    assert!(s1.iter().all(|a| *a != 0), "frame 1 stores four targets: {s1:x?}");
    assert!(s2.iter().all(|a| *a != 0), "frame 2 stores four targets: {s2:x?}");
    for a in s2 {
        assert!(!s1.contains(&a), "frame 2 writes no image frame 1 wrote ({a:#x}): no feedback");
    }
    let mut d = s2.to_vec();
    d.sort();
    d.dedup();
    assert_eq!(d.len(), 4, "four distinct stores in frame 2");
    /* the job order: frame 1 was kicked before frame 2 began, without a glFinish */
    assert!(j2.kicks_at_begin > j1.kicks_at_begin, "frame 1's job was submitted before frame 2's began");
    let d = j2.seen.first().expect("frame 2's draw");
    let (u, base, texel) = d.tmu.iter().find(|(u, _, _)| u.unit == 0).expect("unit 0 bound at the draw");
    assert!(u.tex_state != 0, "the unit has its texture record");
    /* a texture's image is its own allocation: the store reaches it by the
     * back end's copy of the raster, so the base is not the store's address */
    assert!(!s2.contains(base), "the sampled image is none of frame 2's four stores (no feedback)");
    assert!(d.kicks > j1.kicks_at_begin, "frame 1 was kicked by the time frame 2's draw was built");
    assert_eq!(*texel, bytes(sampled), "frame 1's RT {sampled} store is in the sampled image at the draw");
}

/// (1) T3 sampled, a second four-target framebuffer written.
#[test]
fn a_t3_sampled_into_a_second_four_target_framebuffer() {
    let _g = rig();
    let (a, b) = (TexFb::new(), TexFb::new());
    let j1 = frame1(&a);
    let j2 = frame2(a.tex[3], &b);
    check_frame(&j1, &j2, 3);
}

/// (2) Roles swapped: T0 sampled, the new T3 written.
#[test]
fn b_t0_sampled_into_a_second_four_target_framebuffer() {
    let _g = rig();
    let (a, b) = (TexFb::new(), TexFb::new());
    let j1 = frame1(&a);
    let j2 = frame2(a.tex[0], &b);
    check_frame(&j1, &j2, 0);
}

/// The sampled attachment is the written one next time round: after frame 2,
/// frame 3 samples frame 2's T3 (a chain), still without a feedback loop.
#[test]
fn c_chain_of_three_frames() {
    let _g = rig();
    let (a, b, c) = (TexFb::new(), TexFb::new(), TexFb::new());
    let j1 = frame1(&a);
    let j2 = frame2(a.tex[3], &b);
    check_frame(&j1, &j2, 3);
    /* frame 2 drew, it did not clear: its T3 holds frame 1's clear until the
     * stand-in GPU is told otherwise, so only the order and the images are pinned */
    let j3 = frame2(b.tex[3], &c);
    assert_eq!(j3.target.rts, 4);
    let d = j3.seen.first().expect("frame 3's draw");
    let (_, base, _) = d.tmu.iter().find(|(u, _, _)| u.unit == 0).expect("unit 0 bound");
    assert!(!stores(&j3).contains(base), "frame 3 samples none of its own four stores");
    assert!(j3.kicks_at_begin > j2.kicks_at_begin, "frame 2's job was submitted before frame 3's began");
}
