// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level glClear and glClearBuffer{f,i,ui}v reaching the fourth colour
//! attachment on the V3D host back end (plan #368, RC14, item clear4-v3d),
//! with the GL core told to offer four draw buffers (`backend::HOST_DRAW_TARGETS`
//! = 4, the stand-in for `DRAW_TARGETS` = 4). The V3D counterpart of
//! tests/es3_mrt4_clear_pvr.rs; the job-level words of the same clears are
//! tests/v3d_clear_mrt4.rs'. The clear path is docs/notes/v3d-mrt-clear.md.
//!
//! The rig is tests/es3_mrt4_copytex_v3d.rs': the stand-in GPU applies a job's
//! clear words to each render target's own store and leaves a loaded target
//! alone, so every slot can be read back and compared. A clear that is a quad
//! (scissored or colour-masked) is a draw, and draws leave the stand-in's
//! memory alone: for those the test pins the recorded job (four loaded
//! targets, one quad, the GL's box and mask, the colour block) and that no
//! slot was TLB-cleared; the quad's own pixels are the board's.
//!
//! Pinned: float, signed and unsigned integer attachment 3 under
//! glClearBufferfv/iv/uiv with drawbuffer 3, the other slots unchanged;
//! glClear of all four slots; glColorMask and a scissor box limiting a clear
//! of slot 3; a clear of draw buffer 3 while it is GL_NONE changing nothing.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_clear_v3d
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
        /// The state, fragment colour block and vertex count of each draw (a clear quad).
        pub quads: Vec<(V3dJobState, Vec<u32>, u32)>,
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
        jobs().push(SeenJob { target: t, clear: None, draws: 0, quads: Vec::new() });
        t.width != 0 && t.height != 0
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
        // SAFETY: the backend passes its live job.
        let j = unsafe { &mut *j };
        j.draws += 1;
        // SAFETY: the job's BCL is live.
        unsafe { (*j.bcl).used += 120 };
        // SAFETY: a draw whose state and colour block live for the call.
        let (st, words, count) = unsafe {
            let d = &*d;
            let n = d.fs_storage.count as usize;
            let w = if d.fs_storage.words.is_null() || n == 0 {
                Vec::new()
            } else {
                core::slice::from_raw_parts(d.fs_storage.words, n).to_vec()
            };
            (*d.st, w, d.count)
        };
        if let Some(job) = jobs().last_mut() {
            job.draws += 1;
            job.quads.push((st, words, count));
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
        yakogl::backend::HOST_DRAW_TARGETS.store(4, core::sync::atomic::Ordering::Relaxed);
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
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;

/* ---- what a slot holds ----------------------------------------------------- */

#[derive(Clone, Copy, PartialEq, Debug)]
enum Class {
    Float,
    Int,
    Uint,
}

fn class(f: GLenum) -> Class {
    match f {
        GL_RGBA8I | GL_RGBA16I | GL_RGBA32I => Class::Int,
        GL_RGBA8UI | GL_RGBA16UI | GL_RGBA32UI => Class::Uint,
        _ => Class::Float,
    }
}

/// The value slot `n` is first cleared to (a baseline no other slot shares),
/// as the four words the class reads back: bytes for a float slot.
fn base(n: usize, f: GLenum) -> [u32; 4] {
    match class(f) {
        Class::Float => bytes(n).map(u32::from),
        Class::Uint => [10 + n as u32, 20 + n as u32, 30 + n as u32, 40 + n as u32],
        Class::Int => [10 + n as i32, 20 + n as i32, 30 + n as i32, 40 + n as i32].map(|v| -v as u32),
    }
}

/// The value slot 3 is cleared to by the test (distinct from every baseline).
fn fresh(f: GLenum) -> [u32; 4] {
    match class(f) {
        Class::Float => [0.9f32, 0.1, 0.7, 0.3].map(|v| (v * 255.0 + 0.5) as u32),
        Class::Uint => [101, 102, 103, 104],
        Class::Int => [-101i32, -102, -103, -104].map(|v| v as u32),
    }
}

/// glClearBuffer of draw buffer `n` to `v` (as `fresh` / `base` give it).
fn clear_slot(n: usize, f: GLenum, v: [u32; 4]) {
    // SAFETY: four words each.
    unsafe {
        match class(f) {
            Class::Float => {
                let c = v.map(|b| b as f32 / 255.0);
                glClearBufferfv(GL_COLOR, n as GLint, c.as_ptr())
            }
            Class::Uint => glClearBufferuiv(GL_COLOR, n as GLint, v.as_ptr()),
            Class::Int => glClearBufferiv(GL_COLOR, n as GLint, v.as_ptr() as *const GLint),
        }
    }
}

/// Every pixel of attachment `n` as its class reads it: the words of pixel
/// (0, 0) when all pixels agree, or the first that differs in the message.
fn slot(n: usize, f: GLenum) -> [u32; 4] {
    // SAFETY: a plain GL call.
    unsafe { glReadBuffer(GL_COLOR_ATTACHMENT0 + n as GLenum) };
    let px: Vec<[u32; 4]> = match class(f) {
        Class::Float => {
            let (e, p) = read_bytes();
            assert_eq!(e, GL_NO_ERROR, "slot {n}: {:?}", logs());
            p.chunks(4).map(|c| [c[0], c[1], c[2], c[3]].map(u32::from)).collect()
        }
        c => {
            let (e, p) = read_words(if c == Class::Int { GL_INT } else { GL_UNSIGNED_INT });
            assert_eq!(e, GL_NO_ERROR, "slot {n}: {:?}", logs());
            p.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect()
        }
    };
    let first = px[0];
    if let Some(bad) = px.iter().position(|p| *p != first) {
        panic!("slot {n} ({f:#x}) is not uniform: pixel 0 {first:?}, pixel {bad} {:?}", px[bad]);
    }
    first
}

/// Clear every slot to its baseline, bind the four-slot framebuffer.
fn framebuffer(formats: [GLenum; 4]) -> Fb {
    let fb = Fb::new(&formats);
    for (n, f) in formats.iter().enumerate() {
        clear_slot(n, *f, base(n, *f));
    }
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    for (n, f) in formats.iter().enumerate() {
        assert_eq!(slot(n, *f), base(n, *f), "slot {n} baseline");
    }
    fb
}

/// Slots 0..2 hold their baseline and slot 3 holds `want3`.
fn assert_slots(what: &str, formats: [GLenum; 4], want3: [u32; 4]) {
    for (n, f) in formats.iter().enumerate() {
        let want = if n == 3 { want3 } else { base(n, *f) };
        assert_eq!(slot(n, *f), want, "{what}: slot {n} ({f:#x})");
    }
}

/// Jobs recorded since `mark`.
fn since(mark: usize) -> Vec<SeenJob> {
    jobs()[mark..].to_vec()
}

fn mark() -> usize {
    jobs().len()
}

const NONE3: [GLenum; 4] = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_NONE];
const ALL4: [GLenum; 4] = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];

/// The other slots of a float layout, then slot 3 of each class.
fn layouts() -> Vec<[GLenum; 4]> {
    vec![
        [RGBA8; 4],
        [RGBA8, RGBA8, RGBA8, GL_RGBA8UI],
        [RGBA8, RGBA8, RGBA8, GL_RGBA8I],
        [RGBA8, GL_RGBA8UI, GL_RGBA8I, RGBA8],
        [RGBA8, GL_RGBA8I, GL_RGBA8UI, GL_RGBA8UI],
        [GL_RGBA8I, GL_RGBA8I, GL_RGBA8I, GL_RGBA8I],
    ]
}

/// The one clear job of a TLB clear of draw buffer 3 alone: four targets,
/// RTs 0-2 loaded and RT 3 cleared, no quad.
fn assert_tlb_clear_of_3(what: &str, jobs: &[SeenJob]) {
    assert_eq!(jobs.len(), 1, "{what}: one job: {:?}", logs());
    let (t, c) = (&jobs[0].target, jobs[0].clear.as_ref().expect("an ended job"));
    assert_eq!(t.rts, 4, "{what}: four render targets");
    assert!(c.load_rt0 && c.load_rt[0] && c.load_rt[1], "{what}: RTs 0-2 are loaded, not cleared");
    assert_eq!(jobs[0].draws, 0, "{what}: no quad: the TLB clears");
}

/* ---- glClearBuffer{f,i,ui}v of draw buffer 3 ---------------------------------- */

#[test]
fn clearbuffer_of_drawbuffer_3_changes_slot_3_only() {
    let _g = rig();
    for formats in layouts() {
        let _fb = framebuffer(formats);
        let want = fresh(formats[3]);
        let m = mark();
        clear_slot(3, formats[3], want);
        // SAFETY: a plain GL call.
        unsafe { glFinish() };
        assert_eq!(err(), GL_NO_ERROR, "{formats:x?}: {:?}", logs());
        assert_tlb_clear_of_3(&format!("{formats:x?}"), &since(m));
        assert_slots(&format!("{formats:x?}"), formats, want);
    }
}

/// Each class and width of slot 3 beside float slots 0-2.
#[test]
fn clearbuffer_of_drawbuffer_3_every_integer_width() {
    let _g = rig();
    for f3 in [GL_RGBA8I, GL_RGBA16I, GL_RGBA32I, GL_RGBA8UI, GL_RGBA16UI, GL_RGBA32UI] {
        let formats = [RGBA8, RGBA8, RGBA8, f3];
        let _fb = framebuffer(formats);
        let want = fresh(f3);
        clear_slot(3, f3, want);
        // SAFETY: a plain GL call.
        unsafe { glFinish() };
        assert_eq!(err(), GL_NO_ERROR, "{f3:#x}: {:?}", logs());
        assert_slots(&format!("slot 3 {f3:#x}"), formats, want);
    }
}

/// A clear of 3 leaves 0..2 alone even when each of them was cleared to a
/// colour the clear of 3 would show if it leaked.
/// The colour the clear of draw buffer `k` gives (distinct per slot).
fn latest(k: usize) -> [u32; 4] {
    let k = k as u32;
    [10 + k * 50, 200 - k * 40, 30 + k * 10, 255]
}

#[test]
fn clearbuffer_of_each_drawbuffer_leaves_the_other_three() {
    let _g = rig();
    let formats = [RGBA8; 4];
    let _fb = framebuffer(formats);
    for k in 0..4 {
        let v = latest(k);
        clear_slot(k, RGBA8, v);
        for (n, f) in formats.iter().enumerate() {
            // after the clear of k, slots 0..=k hold their latest, the rest the baseline
            let want = if n == k {
                v
            } else if n < k {
                latest(n)
            } else {
                base(n, *f)
            };
            assert_eq!(slot(n, *f), want, "after the clear of {k}: slot {n}");
        }
    }
}

/* ---- glClear with four slots bound ----------------------------------------------- */

#[test]
fn glclear_clears_all_four_slots() {
    let _g = rig();
    let formats = [RGBA8; 4];
    let _fb = framebuffer(formats);
    let c = [0.25f32, 0.5, 0.75, 1.0];
    let m = mark();
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(c[0], c[1], c[2], c[3]);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    let j = since(m);
    assert_eq!(j.len(), 1, "one job: {:?}", logs());
    assert_eq!((j[0].target.rts, j[0].draws), (4, 0), "four targets, the TLB's clear");
    let c0 = j[0].clear.as_ref().expect("an ended job");
    assert!(
        !c0.load_rt0 && !c0.load_rt[0] && !c0.load_rt[1] && !c0.load_rt[2],
        "no RT is loaded: all four are cleared"
    );
    let want = c.map(|v| (v * 255.0 + 0.5) as u32);
    for n in 0..4 {
        assert_eq!(slot(n, RGBA8), want, "slot {n}");
    }
}

/// glClear with an integer slot 3 beside float slots: the float slots take the
/// colour, the integer slot is cleared too (its value is undefined in ES 3.0
/// 4.2.3, so only that the call and the job are clean is pinned).
#[test]
fn glclear_with_an_integer_slot_3_is_clean_and_clears_the_float_slots() {
    let _g = rig();
    for f3 in [GL_RGBA8UI, GL_RGBA8I] {
        let formats = [RGBA8, RGBA8, RGBA8, f3];
        let _fb = framebuffer(formats);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.25, 0.5, 0.75, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glFinish();
        }
        assert_eq!(err(), GL_NO_ERROR, "{f3:#x}: {:?}", logs());
        for n in 0..3 {
            assert_eq!(slot(n, RGBA8), [64, 128, 191, 255], "{f3:#x}: slot {n}");
        }
    }
}

/* ---- glColorMask --------------------------------------------------------------------- */

const MASK: [u8; 4] = [1, 0, 0, 1];

fn mask_bits() -> u32 {
    MASK.iter().enumerate().map(|(i, b)| u32::from(*b) << i).sum()
}

/// A colour-masked glClear of four slots is one quad over four loaded targets
/// with the GL's mask and one colour a slot in the block; no slot is TLB-cleared.
#[test]
fn masked_glclear_is_one_quad_over_four_loaded_targets() {
    let _g = rig();
    let formats = [RGBA8; 4];
    let _fb = framebuffer(formats);
    let m = mark();
    // SAFETY: plain GL calls.
    unsafe {
        glColorMask(MASK[0], MASK[1], MASK[2], MASK[3]);
        glClearColor(0.9, 0.1, 0.7, 0.3);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
        glColorMask(1, 1, 1, 1);
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    let j = since(m);
    assert_eq!(j.len(), 1, "one job: {:?}", logs());
    assert_eq!((j[0].target.rts, j[0].draws), (4, 1), "four targets, one quad");
    let c = j[0].clear.as_ref().expect("an ended job");
    assert!(c.load_rt0 && c.load_rt.iter().all(|l| *l), "all four targets are loaded");
    let (st, words, count) = &j[0].quads[0];
    assert_eq!(*count, 6, "a quad");
    assert_eq!(st.colormask & 0xf, mask_bits(), "the GL's colour mask");
    assert_eq!(words.len(), 16, "four colours in the block");
    assert_slots("quad leaves the stand-in's memory", formats, base(3, RGBA8));
}

/// glColorMask limiting a glClearBuffer of draw buffer 3, float, signed and
/// unsigned: one quad of RT 3 alone (one colour, the value as given), no TLB clear.
#[test]
fn masked_clearbuffer_of_drawbuffer_3_is_a_quad_of_rt3_alone() {
    let _g = rig();
    for f3 in [RGBA8, GL_RGBA8I, GL_RGBA8UI] {
        let formats = [RGBA8, RGBA8, RGBA8, f3];
        let _fb = framebuffer(formats);
        let want = fresh(f3);
        let m = mark();
        // SAFETY: a plain GL call.
        unsafe { glColorMask(MASK[0], MASK[1], MASK[2], MASK[3]) };
        clear_slot(3, f3, want);
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            glColorMask(1, 1, 1, 1);
        }
        assert_eq!(err(), GL_NO_ERROR, "{f3:#x}: {:?}", logs());
        let j = since(m);
        assert_eq!((j.len(), j[0].draws, j[0].target.rts), (1, 1, 4), "{f3:#x}: one quad over four targets");
        let c = j[0].clear.as_ref().expect("an ended job");
        assert!(c.load_rt0 && c.load_rt.iter().all(|l| *l), "{f3:#x}: all four are loaded");
        let (st, words, _) = &j[0].quads[0];
        assert_eq!(st.colormask & 0xf, mask_bits(), "{f3:#x}: the GL's mask");
        assert_eq!(words.len(), 4, "{f3:#x}: one colour in the block");
        assert_slots(&format!("{f3:#x}: no TLB clear"), formats, base(3, f3));
    }
}

/// The mask is back to all-on afterwards: the next unmasked clear is the TLB's.
#[test]
fn the_mask_back_on_the_clear_is_the_tlbs_again() {
    let _g = rig();
    let formats = [RGBA8; 4];
    let _fb = framebuffer(formats);
    // SAFETY: plain GL calls.
    unsafe {
        glColorMask(MASK[0], MASK[1], MASK[2], MASK[3]);
        glColorMask(1, 1, 1, 1);
    }
    let want = fresh(RGBA8);
    let m = mark();
    clear_slot(3, RGBA8, want);
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    assert_tlb_clear_of_3("unmasked", &since(m));
    assert_slots("unmasked", formats, want);
}

/* ---- a scissored clear of slot 3 ---------------------------------------------------------- */

const BOX: [GLint; 4] = [2, 1, 4, 2];

#[test]
fn scissored_clearbuffer_of_drawbuffer_3_is_a_quad_of_rt3_alone() {
    let _g = rig();
    for f3 in [RGBA8, GL_RGBA8I, GL_RGBA8UI] {
        let formats = [RGBA8, RGBA8, RGBA8, f3];
        let _fb = framebuffer(formats);
        let m = mark();
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_SCISSOR_TEST);
            glScissor(BOX[0], BOX[1], BOX[2], BOX[3]);
        }
        clear_slot(3, f3, fresh(f3));
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            glDisable(GL_SCISSOR_TEST);
        }
        assert_eq!(err(), GL_NO_ERROR, "{f3:#x}: {:?}", logs());
        let j = since(m);
        assert_eq!((j.len(), j[0].draws, j[0].target.rts), (1, 1, 4), "{f3:#x}: one quad over four targets");
        let c = j[0].clear.as_ref().expect("an ended job");
        assert!(c.load_rt0 && c.load_rt.iter().all(|l| *l), "{f3:#x}: all four are loaded");
        let (st, words, count) = &j[0].quads[0];
        assert_eq!(*count, 6, "{f3:#x}: a quad");
        assert!(st.scissor, "{f3:#x}: scissor test on");
        let b = [st.scissor_minx, st.scissor_miny, st.scissor_maxx, st.scissor_maxy];
        let want_box = [BOX[0], BOX[1], BOX[0] + BOX[2], BOX[1] + BOX[3]].map(|v| v as u32);
        assert_eq!(b, want_box, "{f3:#x}: the GL's box");
        assert_eq!(words.len(), 4, "{f3:#x}: one colour in the block");
        assert_eq!(st.colormask & 0xf, 0xf, "{f3:#x}: unmasked");
        assert_slots(&format!("{f3:#x}: no TLB clear"), formats, base(3, f3));
    }
}

/// A scissor box that holds the whole attachment is no scissor: the TLB clears.
#[test]
fn a_scissor_box_over_the_whole_attachment_is_the_tlbs_clear() {
    let _g = rig();
    let formats = [RGBA8; 4];
    let _fb = framebuffer(formats);
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(0, 0, W, H);
    }
    let want = fresh(RGBA8);
    let m = mark();
    clear_slot(3, RGBA8, want);
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        glDisable(GL_SCISSOR_TEST);
    }
    assert_tlb_clear_of_3("full box", &since(m));
    assert_slots("full box", formats, want);
}

#[test]
fn scissored_glclear_is_one_quad_writing_all_four_colours() {
    let _g = rig();
    let formats = [RGBA8; 4];
    let _fb = framebuffer(formats);
    let m = mark();
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(BOX[0], BOX[1], BOX[2], BOX[3]);
        glClearColor(0.9, 0.1, 0.7, 0.3);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
        glDisable(GL_SCISSOR_TEST);
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    let j = since(m);
    assert_eq!((j.len(), j[0].draws, j[0].target.rts), (1, 1, 4), "{:?}", logs());
    assert_eq!(j[0].quads[0].1.len(), 16, "four colours in the block");
    assert_slots("quad leaves the stand-in's memory", formats, base(3, RGBA8));
}

/* ---- draw buffer 3 is GL_NONE --------------------------------------------------------------- */

/// ES 3.0 4.2.3: clearing a draw buffer that is GL_NONE has no effect and no
/// error: no job, and every slot, 3 included, keeps its value.
#[test]
fn clearbuffer_of_a_none_drawbuffer_3_leaves_every_slot() {
    let _g = rig();
    for formats in layouts() {
        let _fb = framebuffer(formats);
        // SAFETY: a four-enum array.
        unsafe { glDrawBuffers(4, NONE3.as_ptr()) };
        for scissor in [false, true] {
            if scissor {
                // SAFETY: plain GL calls.
                unsafe {
                    glEnable(GL_SCISSOR_TEST);
                    glScissor(BOX[0], BOX[1], BOX[2], BOX[3]);
                }
            }
            let m = mark();
            clear_slot(3, formats[3], fresh(formats[3]));
            // SAFETY: a plain GL call.
            unsafe { glFinish() };
            assert_eq!(err(), GL_NO_ERROR, "{formats:x?} scissor {scissor}: {:?}", logs());
            for job in since(m) {
                let c = job.clear.as_ref().expect("an ended job");
                assert!(
                    c.load_rt0 && c.load_rt.iter().all(|l| *l) && job.draws == 0,
                    "{formats:x?} scissor {scissor}: no clear of any RT: {job:?}"
                );
            }
            assert_slots(&format!("{formats:x?} scissor {scissor}"), formats, base(3, formats[3]));
        }
        // SAFETY: plain GL calls.
        unsafe {
            glDisable(GL_SCISSOR_TEST);
            glDrawBuffers(4, ALL4.as_ptr());
        }
    }
}

/// With draw buffer 3 GL_NONE, a clear of draw buffer 2 still clears slot 2
/// and only it; and glClear leaves slot 3 alone.
#[test]
fn a_none_drawbuffer_3_does_not_stop_the_other_clears() {
    let _g = rig();
    let formats = [RGBA8; 4];
    let _fb = framebuffer(formats);
    // SAFETY: a four-enum array.
    unsafe { glDrawBuffers(4, NONE3.as_ptr()) };
    let want = fresh(RGBA8);
    clear_slot(2, RGBA8, want);
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    assert_eq!(slot(2, RGBA8), want, "slot 2 is cleared");
    for n in [0, 1, 3] {
        assert_eq!(slot(n, RGBA8), base(n, RGBA8), "slot {n} is untouched");
    }
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.0, 0.0, 0.0, 0.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    for n in 0..3 {
        assert_eq!(slot(n, RGBA8), [0; 4], "glClear: slot {n}");
    }
    assert_eq!(slot(3, RGBA8), base(3, RGBA8), "glClear: slot 3 is GL_NONE and keeps its value");
}

/* ---- the count --------------------------------------------------------------------------------- */

/// At three draw targets a drawbuffer of 3 is GL_INVALID_VALUE and clears nothing.
#[test]
fn at_three_targets_drawbuffer_3_is_invalid_value() {
    let _g = rig();
    let formats = [RGBA8; 4];
    let _fb = framebuffer(formats);
    yakogl::backend::HOST_DRAW_TARGETS.store(3, core::sync::atomic::Ordering::Relaxed);
    let (f, u, i) = ([0.5f32; 4], [7u32; 4], [-7i32; 4]);
    let m = mark();
    // SAFETY: four words each.
    unsafe {
        glClearBufferfv(GL_COLOR, 3, f.as_ptr());
        assert_eq!(glGetError(), GL_INVALID_VALUE, "glClearBufferfv(COLOR, 3)");
        glClearBufferuiv(GL_COLOR, 3, u.as_ptr());
        assert_eq!(glGetError(), GL_INVALID_VALUE, "glClearBufferuiv(COLOR, 3)");
        glClearBufferiv(GL_COLOR, 3, i.as_ptr());
        assert_eq!(glGetError(), GL_INVALID_VALUE, "glClearBufferiv(COLOR, 3)");
        glFinish();
    }
    assert!(since(m).is_empty(), "a refused clear submits nothing");
    yakogl::backend::HOST_DRAW_TARGETS.store(4, core::sync::atomic::Ordering::Relaxed);
    assert_slots("three targets", formats, base(3, RGBA8));
}

/// ES 3.0 4.2.3: a drawbuffer of GL_MAX_DRAW_BUFFERS (4) is GL_INVALID_VALUE.
#[test]
fn a_drawbuffer_of_four_is_invalid_value() {
    let _g = rig();
    let _fb = framebuffer([RGBA8; 4]);
    let f = [0.5f32; 4];
    let m = mark();
    // SAFETY: four floats.
    unsafe {
        glClearBufferfv(GL_COLOR, 4, f.as_ptr());
        assert_eq!(glGetError(), GL_INVALID_VALUE);
        glFinish();
    }
    assert!(since(m).is_empty());
}
