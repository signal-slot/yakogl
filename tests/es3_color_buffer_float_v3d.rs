// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 4.2's float colour targets through the GL API over the real V3D
//! back end on the host (plan #360, item `v3d-cbf-test-draft`): what the 16F
//! targets do today, and what the V3D arm (item `v3d-arm`) must do once the
//! `color-buffer-float` Cargo feature claims `GL_EXT_color_buffer_float`:
//! draw, clear and read back R16F, RG16F, RGBA16F, R32F, RG32F, RGBA32F and
//! R11F_G11F_B10F (docs/notes/es3-float-targets.md, docs/notes/v3d-fragment-out.md).
//!
//! The rig is tests/es3_readback_v3d.rs': the job builder and the GPU stood
//! in for. The stand-in GPU applies a job's whole-target clear words to every
//! render target's raster store in the output format the target names
//! (`V3dJobClear::clear_rt[n]`, as `state::clear_words` packs them) and runs
//! nothing else; what is asserted is the render-target word the GL core
//! handed the back end (`V3dJobTarget::rt_format`, and the internal type and
//! bpp the row of that output format carries), the clear words, and the clear
//! read back through glReadPixels. Host: API, state and words count, pictures
//! do not.
//!
//! The plain tests pass today in the build without the feature. The
//! `#[ignore = "v3d-arm"]` ones state the claimed path (run with
//! `--features color-buffer-float` once `cbf-feature` has declared it) with
//! the expectations taken from Mesa 26.2.1 (read only), the file:line of each
//! in its comment:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_color_buffer_float_v3d
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,color-buffer-float --test es3_color_buffer_float_v3d -- --ignored
//! ```
//!
//! Mesa evidence, `third_party/mesa-26.2.1/src` (the tile buffer a format
//! uses is `v3dX(get_internal_type_bpp_for_output_format)`,
//! gallium/drivers/v3d/v3dx_format_table.c:220-340):
//!
//! | format | output image format (broadcom/cle/v3d_packet.xml) | internal type, bpp |
//! |---|---|---|
//! | R16F | 20 (:247) | 16F, 64 (:263-269, "to make sure the TLB doesn't throw away the alpha channel") |
//! | RG16F | 19 (:246) | 16F, 64 (:263-269) |
//! | RGBA16F | 18 (:245) | 16F, 64 (:253-262) |
//! | R11F_G11F_B10F | 8 (:235) | 16F, 64 (:253-262: the TLB packs the halves at the store) |
//! | R32F | 11 (:238) | 32F, 32 (:327-330) |
//! | RG32F | 10 (:237) | 32F, 64 (:323-326) |
//! | RGBA32F | 9 (:236) | 32F, 128 (:319-322) |
//!
//! The renderable formats of the V3D's pipe screen are the format table rows
//! :90-101 and :146 (v3dx_format_table.c): R16F, R32F, RG16F, RG32F,
//! RGBA16F, RGBA32F and R11G11B10_FLOAT all carry a render-target output
//! format.
#![cfg(all(
    feature = "host-test",
    feature = "gpu-v3d",
    feature = "shaderc-online",
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#![allow(dead_code)]
// `color-buffer-float` joins Cargo.toml with item `cbf-feature`.
#![allow(unexpected_cfgs)]

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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_color_buffer_float_v3d");
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
            /* the formats src/v3d/ffi.rs has no constant for yet (the V3D arm adds them) */
            OIF_RGBA32F => words(4),
            OIF_RG32F => words(2),
            OIF_RG16F => words(1),
            OIF_R11F_G11F_B10F => {
                /* 16F in the tile buffer, packed at the store: the halves'
                 * exponent and top mantissa bits, a negative one as 0 */
                let h = [w[0] & 0xffff, w[0] >> 16, w[1] & 0xffff];
                let up = |h: u32, drop: u32| if h & 0x8000 != 0 { 0 } else { (h & 0x7fff) >> drop };
                (up(h[0], 4) | up(h[1], 4) << 11 | up(h[2], 5) << 22).to_le_bytes().to_vec()
            }
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

    /* `V3D_OUTPUT_IMAGE_FORMAT_*` of broadcom/cle/v3d_packet.xml:235-247 that
     * src/v3d/ffi.rs does not name yet. */
    pub const OIF_R11F_G11F_B10F: u32 = 8;
    pub const OIF_RGBA32F: u32 = 9;
    pub const OIF_RG32F: u32 = 10;
    pub const OIF_RG16F: u32 = 19;

    /// The tile buffer a format takes (`V3D_INTERNAL_TYPE_*`, `V3D_INTERNAL_BPP_*`).
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct Tlb {
        pub output: u32,
        pub internal_type: u32,
        pub bpp: u32,
        /// Bytes a pixel in memory.
        pub bytes: u32,
    }

    /// A float colour format of the tests: its channels, and the claimed path's
    /// row from Mesa (`mesa`), and today's row (`today`, `None` where the GL
    /// core makes no renderbuffer storage of the format today).
    #[derive(Clone, Copy, Debug)]
    pub struct Fmt {
        pub name: &'static str,
        pub gl: GLenum,
        pub channels: usize,
        pub mesa: Tlb,
        pub today: Option<Tlb>,
    }

    const fn tlb(output: u32, internal_type: u32, bpp: u32, bytes: u32) -> Tlb {
        Tlb { output, internal_type, bpp, bytes }
    }

    const T16: u32 = V3D_INTERNAL_TYPE_16F;
    const T32: u32 = V3D_INTERNAL_TYPE_32F;
    const B32: u32 = V3D_INTERNAL_BPP_32;
    const B64: u32 = V3D_INTERNAL_BPP_64;
    const B128: u32 = V3D_INTERNAL_BPP_128;

    /// The seven formats of the plan. Mesa rows: v3dx_format_table.c:253-269
    /// (16F, 64 bpp), :319-330 (32F at 128, 64 and 32 bpp). Today's rows:
    /// `state::rt_format` (RG16F and RGB16F are held in the RGBA16F surface,
    /// four halves a pixel; the GL core makes no storage of the 32F pair and
    /// R11F_G11F_B10F).
    pub const FLOATS: [Fmt; 7] = [
        Fmt {
            name: "R16F",
            gl: GL_R16F,
            channels: 1,
            mesa: tlb(V3D_OUTPUT_IMAGE_FORMAT_R16F, T16, B64, 2),
            today: Some(tlb(V3D_OUTPUT_IMAGE_FORMAT_R16F, T16, B64, 2)),
        },
        Fmt {
            name: "RG16F",
            gl: GL_RG16F,
            channels: 2,
            mesa: tlb(OIF_RG16F, T16, B64, 4),
            today: Some(tlb(V3D_OUTPUT_IMAGE_FORMAT_RGBA16F, T16, B64, 8)),
        },
        Fmt {
            name: "RGBA16F",
            gl: GL_RGBA16F,
            channels: 4,
            mesa: tlb(V3D_OUTPUT_IMAGE_FORMAT_RGBA16F, T16, B64, 8),
            today: Some(tlb(V3D_OUTPUT_IMAGE_FORMAT_RGBA16F, T16, B64, 8)),
        },
        Fmt {
            name: "R32F",
            gl: GL_R32F,
            channels: 1,
            mesa: tlb(V3D_OUTPUT_IMAGE_FORMAT_R32F, T32, B32, 4),
            today: Some(tlb(V3D_OUTPUT_IMAGE_FORMAT_R32F, T32, B32, 4)),
        },
        Fmt { name: "RG32F", gl: GL_RG32F, channels: 2, mesa: tlb(OIF_RG32F, T32, B64, 8), today: None },
        Fmt { name: "RGBA32F", gl: GL_RGBA32F, channels: 4, mesa: tlb(OIF_RGBA32F, T32, B128, 16), today: None },
        Fmt {
            name: "R11F_G11F_B10F",
            gl: GL_R11F_G11F_B10F,
            channels: 3,
            mesa: tlb(OIF_R11F_G11F_B10F, T16, B64, 4),
            today: None,
        },
    ];

    /// RGB16F: a three-channel half-float format the GL core holds in the
    /// RGBA16F surface today; ES 3.0 with GL_EXT_color_buffer_half_float
    /// makes it renderable (listed apart: the plan's seven are the others).
    pub const RGB16F: Fmt = Fmt {
        name: "RGB16F",
        gl: GL_RGB16F,
        channels: 3,
        mesa: tlb(V3D_OUTPUT_IMAGE_FORMAT_RGBA16F, T16, B64, 8),
        today: Some(tlb(V3D_OUTPUT_IMAGE_FORMAT_RGBA16F, T16, B64, 8)),
    };

    /// The exact half of a value of the clear colours below (a normal number,
    /// its mantissa fitting ten bits).
    pub fn half(x: f32) -> u32 {
        let b = x.to_bits();
        let e = ((b >> 23) & 0xff) as i32 - 127 + 15;
        assert!((1..31).contains(&e) && b & 0x1fff == 0, "{x} is not an exact half");
        (b >> 16 & 0x8000) | (e as u32) << 10 | (b >> 13 & 0x3ff)
    }

    impl Fmt {
        /// Four exact halves and floats; R11F_G11F_B10F has no sign bit, so
        /// its colour is positive.
        pub fn clear_color(&self) -> [f32; 4] {
            if self.gl == GL_R11F_G11F_B10F {
                [0.5, 0.25, 0.75, 0.125]
            } else {
                [0.5, -0.25, 0.75, 0.125]
            }
        }

        /// What RGBA / FLOAT reads of the cleared buffer (ES 3.0 4.3.1: the
        /// channels a format lacks read 0, 0, 1).
        pub fn expect(&self) -> [f32; 4] {
            let mut c = self.clear_color();
            for (ch, v) in c.iter_mut().enumerate().skip(self.channels) {
                *v = if ch == 3 { 1.0 } else { 0.0 };
            }
            c
        }

        /// The colour the GL core hands the back end for the clear: the
        /// clear's, with the alpha of a format without one forced to 1
        /// (`fb_target_alpha_bits`).
        pub fn job_color(&self) -> [f32; 4] {
            let mut c = self.clear_color();
            if self.channels < 4 {
                c[3] = 1.0;
            }
            c
        }

        /// The words the V3D job carries as `clear_rt[0]` for this format's
        /// tile buffer: two halves a word for the 16F type (R11F_G11F_B10F
        /// too, packed at the store), the floats' bits for the 32F type
        /// (4 words at 128 bpp, 2 at 64, 1 at 32; the rest 0).
        pub fn mesa_clear_words(&self) -> [u32; 4] {
            let c = self.job_color();
            if self.mesa.internal_type == T16 {
                let h = c.map(half);
                [h[0] | h[1] << 16, h[2] | h[3] << 16, 0, 0]
            } else {
                let w = c.map(f32::to_bits);
                match self.mesa.bpp {
                    B128 => w,
                    B64 => [w[0], w[1], 0, 0],
                    _ => [w[0], 0, 0, 0],
                }
            }
        }

        pub fn clear(&self) {
            let c = self.clear_color();
            // SAFETY: a plain GL call with a four-element array.
            unsafe { glClearBufferfv(GL_COLOR, 0, c.as_ptr()) };
        }
    }

    /// The render target the last job carried, and its clear.
    pub fn last_job() -> SeenJob {
        jobs().last().cloned().expect("a job ran")
    }

    /// The tile buffer a job's RT 0 used, from the output format the target
    /// names: the row `state::rt_format` has for any surface of that output.
    pub fn tlb_of(job: &SeenJob) -> Option<Tlb> {
        let out = job.target.rt_format[0];
        (0..=255u8).find_map(|s| yakogl::v3d::state::rt_format(s).filter(|r| r.output == out)).map(|r| Tlb {
            output: r.output,
            internal_type: r.internal_type,
            bpp: r.internal_bpp,
            bytes: r.bytes,
        })
    }

    /// The extension list, as glGetString and glGetStringi give it.
    pub fn extensions() -> Vec<String> {
        // SAFETY: GL_EXTENSIONS is NUL-terminated text, and so is each indexed one.
        unsafe {
            let all = CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char).to_string_lossy().into_owned();
            let mut n = 0;
            glGetIntegerv(GL_NUM_EXTENSIONS, &mut n);
            let indexed: Vec<String> = (0..n as GLuint)
                .map(|i| CStr::from_ptr(glGetStringi(GL_EXTENSIONS, i) as *const c_char).to_string_lossy().into_owned())
                .collect();
            let listed: Vec<String> = all.split_whitespace().map(str::to_string).collect();
            assert_eq!(listed, indexed, "glGetString and glGetStringi agree");
            listed
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
}

use gl_rig::*;
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;

/// The formats the GL core takes as colour targets today without the
/// extension of the 32-bit formats: the 16F ones.
fn half_formats() -> [Fmt; 4] {
    [FLOATS[0], FLOATS[1], FLOATS[2], RGB16F]
}

/// What RGBA / FLOAT reads of a cleared 16F buffer today: `Fmt::expect`,
/// except that RG16F's blue is the storage's (the clear's), not 0
/// (tests/es3_readback_v3d.rs `rg16f_reads_its_missing_blue_as_0`).
fn read_today(f: &Fmt) -> [f32; 4] {
    let mut want = f.expect();
    if f.gl == GL_RG16F {
        want[2] = f.clear_color()[2];
    }
    want
}

/// A scissored clear into `f`'s bound buffer: a quad draw, not a whole-target clear.
fn scissored_clear(f: &Fmt) {
    // SAFETY: plain GL calls with a four-element array.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(1, 1, 2, 2);
        glClearBufferfv(GL_COLOR, 0, f.clear_color().as_ptr());
        glDisable(GL_SCISSOR_TEST);
    }
}

/* ---- the extension list ------------------------------------------------------ */

/// Without the feature the ES 3.0 build lists GL_EXT_color_buffer_half_float
/// and not GL_EXT_color_buffer_float (docs/notes/es3-float-targets.md).
#[cfg(not(feature = "color-buffer-float"))]
#[test]
fn the_default_build_claims_half_float_and_not_float() {
    let _g = gl_rig::rig();
    let ext = extensions();
    assert!(ext.iter().any(|e| e == "GL_EXT_color_buffer_half_float"), "{ext:?}");
    assert!(!ext.iter().any(|e| e == "GL_EXT_color_buffer_float"), "{ext:?}");
}

/// With the feature the build lists both, and the V3D really draws the
/// formats behind the claim (the tests below).
#[test]
#[ignore = "v3d-arm: needs --features color-buffer-float; GL_EXT_color_buffer_float is not claimed yet"]
fn the_feature_claims_float_beside_half_float() {
    let _g = gl_rig::rig();
    let ext = extensions();
    for name in ["GL_EXT_color_buffer_float", "GL_EXT_color_buffer_half_float"] {
        assert!(ext.iter().any(|e| e == name), "{name} in {ext:?}");
    }
}

/* ---- the 16F targets today ---------------------------------------------------- */

/// R16F, RG16F, RGB16F and RGBA16F renderbuffers are complete, clear and read
/// back as floats, through the render-target word and tile buffer
/// `state::rt_format` gives them (16F at 64 bpp, Mesa's rows
/// v3dx_format_table.c:253-269): R16F its own output format, the others the
/// RGBA16F surface's.
#[test]
fn half_float_targets_clear_and_read_back_today() {
    let _g = gl_rig::rig();
    for f in half_formats() {
        let _fb = Fb::new(&[f]);
        f.clear();
        assert_eq!(gl_rig::err(), GL_NO_ERROR, "{}: the clear", f.name);
        let (e, px) = read(GL_RGBA, GL_FLOAT);
        assert_eq!(e, GL_NO_ERROR, "{}: {:?}", f.name, gl_rig::logs());
        let want = read_today(&f);
        for (i, p) in px.chunks(4).enumerate() {
            assert_eq!([p[0], p[1], p[2], p[3]].map(f32::from_bits), want, "{} pixel {i}", f.name);
        }
        let job = last_job();
        assert!(job.target.typed, "{}: a typed job", f.name);
        assert_eq!(tlb_of(&job), f.today, "{}: the render-target row", f.name);
        let h = f.job_color().map(half);
        let c = job.clear.expect("the job ended");
        assert_eq!(c.clear_rt[0], [h[0] | h[1] << 16, h[2] | h[3] << 16, 0, 0], "{}: the clear words", f.name);
        nothing_refused(f.name);
    }
}

/// A scissored clear is a quad draw: the job draws into the same
/// render-target row as the whole-target clear.
#[test]
fn half_float_targets_draw_a_job_today() {
    let _g = gl_rig::rig();
    for f in [FLOATS[0], FLOATS[2]] {
        let _fb = Fb::new(&[f]);
        f.clear();
        scissored_clear(&f);
        assert_eq!(gl_rig::err(), GL_NO_ERROR, "{}: the scissored clear", f.name);
        let (e, _) = read(GL_RGBA, GL_FLOAT);
        assert_eq!(e, GL_NO_ERROR, "{}", f.name);
        let draw = jobs().iter().rev().find(|j| j.draws > 0).cloned();
        let draw = draw.unwrap_or_else(|| panic!("{}: no job drew: {:?}", f.name, logs()));
        assert_eq!(tlb_of(&draw), f.today, "{}: the draw's render-target row", f.name);
        nothing_refused(f.name);
    }
}

/// Without the feature the plan's wide float formats are not renderbuffer
/// storage: the storage call is GL_INVALID_ENUM and a framebuffer on it is not
/// complete (tests/es3_readback_v3d.rs
/// `the_other_float_formats_are_no_renderbuffer_storage`; R32F is left to
/// the plan's R32F items).
#[cfg(not(feature = "color-buffer-float"))]
#[test]
fn the_wide_float_formats_are_not_targets_without_the_claim() {
    let _g = gl_rig::rig();
    for f in FLOATS.iter().filter(|f| f.today.is_none()) {
        let (mut rb, mut fbo) = (0, 0);
        // SAFETY: plain GL calls with room for one name each.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, f.gl, W, H);
            assert_eq!(glGetError(), GL_INVALID_ENUM, "{}: renderbuffer storage", f.name);
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            assert_ne!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{}", f.name);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fbo);
            glDeleteRenderbuffers(1, &rb);
            while glGetError() != GL_NO_ERROR {}
        }
    }
}

/// The tables above agree with Mesa's: the tile buffer's internal type and
/// bpp are one function of the output format (v3dx_format_table.c:220-340),
/// 16F at 64 bpp for every half format and R11F_G11F_B10F, 32F at 32, 64 and
/// 128 bpp for R32F, RG32F and RGBA32F; where today has a row it is Mesa's
/// but for the surface RG16F shares with RGBA16F.
#[test]
fn the_mesa_rows_of_the_tables_are_self_consistent() {
    for f in FLOATS {
        let m = f.mesa;
        let want = match f.gl {
            GL_RGBA32F => (V3D_INTERNAL_TYPE_32F, V3D_INTERNAL_BPP_128, 16),
            GL_RG32F => (V3D_INTERNAL_TYPE_32F, V3D_INTERNAL_BPP_64, 8),
            GL_R32F => (V3D_INTERNAL_TYPE_32F, V3D_INTERNAL_BPP_32, 4),
            GL_R16F => (V3D_INTERNAL_TYPE_16F, V3D_INTERNAL_BPP_64, 2),
            GL_RGBA16F => (V3D_INTERNAL_TYPE_16F, V3D_INTERNAL_BPP_64, 8),
            _ => (V3D_INTERNAL_TYPE_16F, V3D_INTERNAL_BPP_64, 4),
        };
        assert_eq!((m.internal_type, m.bpp, m.bytes), want, "{}", f.name);
        if let Some(t) = f.today {
            if f.gl != GL_RG16F {
                assert_eq!(t, m, "{}", f.name);
            }
        }
    }
}

/* ---- the claimed path (v3d-arm) ------------------------------------------------ */

/// Every format of the plan is renderbuffer storage and a complete colour
/// attachment, and glClearBufferfv leaves the V3D job the clear words of its
/// tile buffer (two halves a word for 16F and R11F_G11F_B10F, the floats'
/// bits for 32F at 1, 2 or 4 words) and the render-target row Mesa names
/// (v3dx_format_table.c:253-269, :319-330; the output format values
/// v3d_packet.xml:235-247).
#[test]
#[ignore = "v3d-arm: needs --features color-buffer-float; no RG32F, RGBA32F or R11F_G11F_B10F target yet, RG16F is the RGBA16F surface"]
fn every_float_format_clears_through_its_mesa_row() {
    let _g = gl_rig::rig();
    for f in FLOATS {
        let _fb = Fb::new(&[f]);
        f.clear();
        assert_eq!(gl_rig::err(), GL_NO_ERROR, "{}: the clear", f.name);
        let (e, _) = read(GL_RGBA, GL_FLOAT);
        assert_eq!(e, GL_NO_ERROR, "{}", f.name);
        let job = last_job();
        assert!(job.target.typed, "{}: a typed job", f.name);
        assert_eq!(job.target.rt_format[0], f.mesa.output, "{}: the output image format", f.name);
        assert_eq!(tlb_of(&job), Some(f.mesa), "{}: the internal type, bpp and bytes", f.name);
        let c = job.clear.expect("the job ended");
        assert_eq!(c.clear_rt[0], f.mesa_clear_words(), "{}: the clear words", f.name);
        nothing_refused(f.name);
    }
}

/// The clear read back as RGBA / FLOAT: the clear's values in the channels
/// the format has and 0, 0, 1 in the ones it lacks (ES 3.0 4.3.1), exact for
/// the 16F, 32F and R11F_G11F_B10F values the clear uses; the memory past a
/// window hanging over the edge is left alone.
#[test]
#[ignore = "v3d-arm: needs --features color-buffer-float; the 32F pair and R11F_G11F_B10F are no colour surface yet (core-surf), RG16F's blue reads the storage's"]
fn every_float_format_reads_its_clear_back() {
    let _g = gl_rig::rig();
    for f in FLOATS {
        let _fb = Fb::new(&[f]);
        f.clear();
        let (e, px) = read(GL_RGBA, GL_FLOAT);
        assert_eq!(e, GL_NO_ERROR, "{}: {:?}", f.name, gl_rig::logs());
        for (i, p) in px.chunks(4).enumerate() {
            assert_eq!([p[0], p[1], p[2], p[3]].map(f32::from_bits), f.expect(), "{} pixel {i}", f.name);
        }
        let mut over = vec![0xdead_beefu32; 4 * 4];
        // SAFETY: room for 4 x 1 pixels of four words.
        unsafe {
            glReadPixels(W - 2, 0, 4, 1, GL_RGBA, GL_FLOAT, over.as_mut_ptr() as *mut c_void);
            assert_eq!(glGetError(), GL_NO_ERROR, "{}: over the edge", f.name);
        }
        assert!(over[8..].iter().all(|w| *w == 0xdead_beef), "{}: past the edge is untouched", f.name);
        nothing_refused(f.name);
    }
}

/// A scissored clear into each format is a draw job in the format's row; the
/// RGBA32F job is the 128-bpp tile buffer (v3d_job.h's max_bpp, as
/// `v3d_frame_tiling_bpp` takes it).
#[test]
#[ignore = "v3d-arm: needs --features color-buffer-float; the 32F pair and R11F_G11F_B10F draw nothing yet"]
fn every_float_format_draws_through_its_mesa_row() {
    let _g = gl_rig::rig();
    for f in FLOATS {
        let _fb = Fb::new(&[f]);
        f.clear();
        scissored_clear(&f);
        assert_eq!(gl_rig::err(), GL_NO_ERROR, "{}: the scissored clear", f.name);
        let (e, _) = read(GL_RGBA, GL_FLOAT);
        assert_eq!(e, GL_NO_ERROR, "{}", f.name);
        let draw = jobs().iter().rev().find(|j| j.draws > 0).cloned();
        let draw = draw.unwrap_or_else(|| panic!("{}: no job drew: {:?}", f.name, logs()));
        assert_eq!(tlb_of(&draw), Some(f.mesa), "{}: the draw's render-target row", f.name);
        nothing_refused(f.name);
    }
}

/// RGBA32F beside RGBA16F as two draw buffers: each keeps its own output
/// format (RT 0 and RT 1) and its own clear words.
#[test]
#[ignore = "v3d-arm: needs --features color-buffer-float; RGBA32F is not a colour surface yet"]
fn rgba32f_and_rgba16f_draw_buffers_keep_their_rows() {
    let _g = gl_rig::rig();
    let _fb = Fb::new(&[FLOATS[5], FLOATS[2]]);
    // SAFETY: plain GL calls with four-element arrays.
    unsafe {
        glClearBufferfv(GL_COLOR, 0, FLOATS[5].clear_color().as_ptr());
        glClearBufferfv(GL_COLOR, 1, FLOATS[2].clear_color().as_ptr());
    }
    let (e, _) = read(GL_RGBA, GL_FLOAT);
    assert_eq!(e, GL_NO_ERROR, "{:?}", logs());
    let job = last_job();
    assert_eq!(job.target.rts, 2);
    assert_eq!(job.target.rt_format[0], OIF_RGBA32F);
    assert_eq!(job.target.rt_format[1], V3D_OUTPUT_IMAGE_FORMAT_RGBA16F);
    let c = job.clear.expect("the job ended");
    assert_eq!(c.clear_rt[0], FLOATS[5].mesa_clear_words());
    assert_eq!(c.clear_rt[1], FLOATS[2].mesa_clear_words());
    nothing_refused("RGBA32F and RGBA16F");
}
