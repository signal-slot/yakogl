// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RASTERIZER_DISCARD with four colour targets over the V3D host back end
//! (rdisc4-v3d, plan #368 RC14), the draw-target count at four through the
//! stand-in `backend::HOST_DRAW_TARGETS` (one place, `set_draw_targets`; the
//! `knob` item swaps its body for `v3d_debug_set_draw_targets`). The rig is
//! tests/es3_mrt4_v3d.rs's: the real GL core and V3D back end, the job
//! builder stood in for (every job's target, clear words and draw count are
//! seen), and a stand-in GPU that leaves each cleared target's texel in its
//! raster store, so a read-back of attachment 3 shows what a clear did. A
//! draw leaves memory alone there, so what a draw did is read from the jobs:
//! a job with `draws > 0` is one that rasterised.
//!
//! Pinned (ES 3.0 section 3.1 "Discarding Primitives Before Rasterization",
//! section 4.2.3 "Clearing the Buffers"):
//! * with RASTERIZER_DISCARD enabled a draw through a four-output fragment
//!   shader makes no job draw at all and the four attachments keep their
//!   contents (read back through each of the four read buffers);
//! * after glDisable the next draw is a job that configures four render
//!   targets, loads the earlier contents of RT 0..3 and stores RT 3;
//! * glClear and glClearBuffer* (colour attachment 3 included) "have no
//!   effect" under discard (4.2.3: the commands are ignored; the spec's
//!   words are "if RASTERIZER_DISCARD is enabled, clears have no effect"):
//!   attachment 3 reads its old contents, and a clear made under discard
//!   does not come back with the first draw after glDisable. The GL core
//!   does this today (src/gl/state.rs `glClear`, which glClearBuffer* go
//!   through), so the pins are not ignored.
//!
//! Run with exactly
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,test-knobs --test es3_mrt4_rdiscard_v3d
//! ```
//! tests/v3d_frame_rt4.rs' and the Mesa simulator's tests/v3d_sim_mesa.rs';
//! here the job target (`rts`, `rt[2]`, `rt_format`) and clear
//! (`load_rt[2]`, `rgba8_rt[2]`, `clear_rt[3]`) the back end hands the frame
//! builder are what "RT 3 configured, cleared and stored" means.
#![cfg(all(
    feature = "host-test",
    feature = "gpu-v3d",
    feature = "shaderc-online",
    feature = "no-shader-table",
    feature = "test-knobs"
))]
#![allow(dead_code)]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use yakogl::v3d::ffi::*;

/// The one place the draw-target count is set: the host stand-in
/// `backend::HOST_DRAW_TARGETS` today. The `knob` item replaces this body
/// with `v3d_debug_set_draw_targets(n)` (and `backend::TEST_DRAW_TARGETS`).
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
    use yakogl::gl::es3::*;
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

    #[derive(Clone, Debug)]
    pub struct SeenJob {
        pub target: V3dJobTarget,
        pub clear: Option<V3dJobClear>,
        pub draws: u32,
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

    /* ---- framebuffers and reads ------------------------------------------------ */

    pub const W: GLsizei = 8;
    pub const H: GLsizei = 4;
    pub const RGBA8: GLenum = GL_RGBA8;

    /// A complete framebuffer of one W x H renderbuffer per format on colour
    /// attachments 0.., bound, every attachment a draw buffer. Dropped: the
    /// names go.
    pub struct Fb {
        pub fbo: GLuint,
        pub rbs: Vec<GLuint>,
    }

    impl Fb {
        pub fn new(formats: &[GLenum]) -> Fb {
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
                    glRenderbufferStorage(GL_RENDERBUFFER, *f, W, H);
                    assert_eq!(glGetError(), GL_NO_ERROR, "renderbuffer storage {f:#x}");
                    glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + n as u32, GL_RENDERBUFFER, rb);
                    rbs.push(rb);
                }
                assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
                assert_eq!(
                    glCheckFramebufferStatus(GL_FRAMEBUFFER),
                    GL_FRAMEBUFFER_COMPLETE,
                    "{formats:x?}: {:?}",
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

    /// The clear colour of RGBA8 attachment `n`: none equals another.
    pub fn colour(n: usize) -> [f32; 4] {
        [(n as f32 + 1.0) * 0.2, 1.0 - n as f32 * 0.25, 0.5, 1.0]
    }

    /// The bytes RGBA / UNSIGNED_BYTE gives for `colour(n)`.
    pub fn bytes(n: usize) -> [u8; 4] {
        colour(n).map(|v| (v * 255.0 + 0.5) as u8)
    }

    /// glClearBufferfv of each of four RGBA8 attachments to its own `colour`.
    pub fn clear_four() {
        for n in 0..4 {
            // SAFETY: a four-element array.
            unsafe { glClearBufferfv(GL_COLOR, n as GLint, colour(n).as_ptr()) };
        }
    }

    /// The whole W x H read buffer as RGBA / UNSIGNED_BYTE: the error and the
    /// bytes (0xee where a refused read wrote nothing).
    pub fn read_bytes() -> (GLenum, Vec<u8>) {
        let mut out = vec![0xeeu8; (W * H) as usize * 4];
        // SAFETY: room for W x H pixels of 4 bytes.
        unsafe {
            while glGetError() != GL_NO_ERROR {}
            glReadPixels(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE, out.as_mut_ptr() as *mut c_void);
            (glGetError(), out)
        }
    }

    /// The same for the integer pair (RGBA_INTEGER with `ty`).
    pub fn read_words(ty: GLenum) -> (GLenum, Vec<u32>) {
        let mut out = vec![0xdead_beefu32; (W * H) as usize * 4];
        // SAFETY: room for W x H pixels of 16 bytes.
        unsafe {
            while glGetError() != GL_NO_ERROR {}
            glReadPixels(0, 0, W, H, GL_RGBA_INTEGER, ty, out.as_mut_ptr() as *mut c_void);
            (glGetError(), out)
        }
    }

    /// Every 4-byte pixel of `px` is `want`.
    pub fn all_pixels(px: &[u8], want: [u8; 4]) -> bool {
        px.chunks(4).all(|p| p == want)
    }
}

use gl_rig::*;
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::{glDrawArrays, glDrawArraysInstanced, glDrawElements};
use yakogl::gl::es3::glClearBufferfv;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};

/* ---- the proof ---------------------------------------------------------------- */

fn geti(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one int.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

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
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

/// The fragment shader of one output per entry of `types`.
fn fs_of(types: &[&str]) -> String {
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
    fs + "}\n"
}

/// Link and use a program of `types` outputs.
fn program(types: &[&str]) {
    let v = compile(GL_VERTEX_SHADER, VS);
    let f = compile(GL_FRAGMENT_SHADER, &fs_of(types));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{:?}", logs());
        glUseProgram(p);
    }
}

fn draw_triangle() {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
}

/// The jobs begun after `mark`.
fn jobs_since(mark: usize) -> Vec<SeenJob> {
    jobs()[mark..].to_vec()
}

fn mark() -> usize {
    jobs().len()
}

/// The RGBA8 word of `bytes(n)` as the job's `rgba8_rt` holds it.
fn word(n: usize) -> u32 {
    u32::from_le_bytes(bytes(n))
}

/* ---- the proof ---------------------------------------------------------------- */

fn fb_cleared() -> Fb {
    let fb = Fb::new(&[RGBA8; 4]);
    clear_four();
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    fb
}

/// Attachment `n` read back as RGBA / UNSIGNED_BYTE.
fn read_att(n: u32) -> Vec<u8> {
    // SAFETY: a plain GL call.
    unsafe { glReadBuffer(GL_COLOR_ATTACHMENT0 + n) };
    let (e, px) = read_bytes();
    assert_eq!(e, GL_NO_ERROR, "read of attachment {n}: {:?}", logs());
    px
}

/// All four attachments still hold their `colour(n)` clear.
fn assert_untouched(what: &str) {
    for n in 0..4 {
        let px = read_att(n);
        assert!(
            all_pixels(&px, bytes(n as usize)),
            "{what}: attachment {n} reads {:?}, wants {:?}",
            &px[..4],
            bytes(n as usize)
        );
    }
}

fn set_discard(on: bool) {
    // SAFETY: plain GL calls.
    unsafe {
        if on {
            glEnable(GL_RASTERIZER_DISCARD)
        } else {
            glDisable(GL_RASTERIZER_DISCARD)
        }
    }
    assert_eq!(err(), GL_NO_ERROR);
}

fn rasterised(js: &[SeenJob]) -> usize {
    js.iter().map(|j| j.draws as usize).sum()
}

/// (1) A four-output draw under discard rasterises nothing.
#[test]
fn a_draw_under_discard_changes_none_of_the_four() {
    let _g = rig();
    let _fb = fb_cleared();
    program(&["vec4"; 4]);
    set_discard(true);
    let m = mark();
    draw_triangle();
    set_discard(false);
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    assert_eq!(rasterised(&jobs_since(m)), 0, "no job draw under RASTERIZER_DISCARD");
    assert_untouched("after a discarded draw");
}

/// The state is per context, not per draw: several draws, an indexed one
/// too, all discarded.
#[test]
fn a_every_kind_of_draw_is_discarded() {
    let _g = rig();
    let _fb = fb_cleared();
    program(&["vec4"; 4]);
    set_discard(true);
    let m = mark();
    static IDX: [u16; 3] = [0, 1, 2];
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI and IDX are static and outlive the draws.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 2);
        glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, IDX.as_ptr() as *const c_void);
        glFinish();
    }
    set_discard(false);
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    assert_eq!(rasterised(&jobs_since(m)), 0);
    assert_untouched("after discarded draws");
}

/// (2) glDisable: the next draw writes all four (a job that configures four
/// render targets, loads the contents of RT 0..3 and stores RT 3).
#[test]
fn b_the_next_draw_after_disable_writes_all_four() {
    let _g = rig();
    let _fb = fb_cleared();
    program(&["vec4"; 4]);
    set_discard(true);
    draw_triangle();
    set_discard(false);
    let m = mark();
    draw_triangle();
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    let js = jobs_since(m);
    let j = js.iter().rev().find(|j| j.draws > 0).expect("a job with the draw");
    let c = j.clear.expect("the job ended");
    assert_eq!(j.target.rts, 4, "four render targets configured");
    assert!(j.target.rt.iter().all(|r| r.address != 0), "RT 1..3 stored: {:?}", j.target.rt);
    assert!(c.load_rt0 && c.load_rt == [true; 3], "the contents before the draw are loaded: {c:?}");
}

/// The discarded draw did not leave the job's state half-set: draw, discard,
/// draw again, and the second draw is the one job.
#[test]
fn b_discard_toggled_between_draws() {
    let _g = rig();
    let _fb = fb_cleared();
    program(&["vec4"; 4]);
    for round in 0..2 {
        set_discard(true);
        let m = mark();
        draw_triangle();
        assert_eq!(rasterised(&jobs_since(m)), 0, "round {round}: discarded");
        set_discard(false);
        let m = mark();
        draw_triangle();
        assert_eq!(rasterised(&jobs_since(m)), 1, "round {round}: drawn");
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
}

/// (3) glClear under discard has no effect (ES 3.0 4.2.3).
#[test]
fn c_glclear_under_discard_has_no_effect() {
    let _g = rig();
    let _fb = fb_cleared();
    set_discard(true);
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.9, 0.1, 0.1, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
    }
    set_discard(false);
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    assert_untouched("after glClear under discard");
}

/// (3) glClearBufferfv of draw buffer 3 (and 0..2) under discard has no
/// effect on attachment 3, nor on the others.
#[test]
fn c_glclearbuffer_of_attachment_3_under_discard_has_no_effect() {
    let _g = rig();
    let _fb = fb_cleared();
    set_discard(true);
    let v = [0.0f32, 0.0, 1.0, 1.0];
    // SAFETY: plain GL calls; four-element array.
    unsafe {
        glClearBufferfv(GL_COLOR, 3, v.as_ptr());
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "no error is raised for a clear under discard: {:?}", logs());
    assert_untouched("after glClearBufferfv(3) under discard");
    // SAFETY: plain GL calls.
    unsafe {
        for n in 0..4 {
            glClearBufferfv(GL_COLOR, n, v.as_ptr());
        }
        glFinish();
    }
    set_discard(false);
    assert_untouched("after glClearBufferfv(0..3) under discard");
}

/// (3) A clear made under discard is dropped, not parked: the draw after
/// glDisable loads attachment 3 (no pending clear) and the contents survive.
#[test]
fn c_a_clear_under_discard_does_not_come_back() {
    let _g = rig();
    let _fb = fb_cleared();
    program(&["vec4"; 4]);
    set_discard(true);
    let v = [0.0f32, 0.0, 1.0, 1.0];
    // SAFETY: plain GL calls.
    unsafe {
        glClearBufferfv(GL_COLOR, 3, v.as_ptr());
        glClear(GL_COLOR_BUFFER_BIT);
    }
    set_discard(false);
    let m = mark();
    draw_triangle();
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    let js = jobs_since(m);
    let j = js.iter().rev().find(|j| j.draws > 0).expect("a job with the draw");
    let c = j.clear.expect("the job ended");
    assert!(c.load_rt[2] && c.load_rt0, "RT 3 is loaded, the discarded clear is not applied: {c:?}");
    assert_untouched("after the draw following a discarded clear");
}

/// Clearing works as before once discard is off again.
#[test]
fn c_clears_work_after_disable() {
    let _g = rig();
    let _fb = fb_cleared();
    set_discard(true);
    let v = [0.0f32, 0.0, 1.0, 1.0];
    // SAFETY: plain GL calls.
    unsafe { glClearBufferfv(GL_COLOR, 3, v.as_ptr()) };
    set_discard(false);
    // SAFETY: plain GL calls.
    unsafe {
        glClearBufferfv(GL_COLOR, 3, v.as_ptr());
        glFinish();
    }
    let px = read_att(3);
    assert!(all_pixels(&px, [0, 0, 255, 255]), "attachment 3 reads {:?}", &px[..4]);
    for n in 0..3 {
        assert!(all_pixels(&read_att(n), bytes(n as usize)), "attachment {n} untouched");
    }
}
