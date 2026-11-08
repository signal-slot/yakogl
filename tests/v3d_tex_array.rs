// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Sampling a 2D-array or 3D texture on the V3D (docs/notes/v3d-fbo-render.md
//! section 4, D2; docs/notes/v3d-tex-layout-es3.md).
//!
//! The note was written when `texture::refusal_for` answered "a 2D array or
//! 3D texture is not sampled yet" and the online compiler refused the
//! look-ups. Both are lifted since 8ae0a44b (`Op::Tex` takes `D2Array` and
//! `D3`, TMUI / TMUR are written, `refusal_for` returns `None` for the two
//! types), so this file pins what is true now, not the by-name refusal:
//! * `refusal` accepts an array and a 3D binding, and still refuses the
//!   malformed ones (no layer, 16384 texels wide);
//! * the layout of a 5-layer RGBA8 array and a 5-slice 3D texture: the
//!   per-layer offset and stride, Image Depth and Array Stride of the record;
//! * through the GL API over the real back end (job builder faked): a draw
//!   sampling each kind reaches the job with a texture record whose Image
//!   Depth is 5, raises no GL error and logs no refusal;
//! * one `#[ignore]` test for what is still open: layers rendered through a
//!   framebuffer, then the array sampled.
//!
//! The rig is tests/es3_formats_v3d.rs's `gl_rig`, copied.
//!
//! Run: cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_tex_array

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

use yakogl::backend::*;
use yakogl::v3d::ffi::*;
use yakogl::v3d::texture;

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

/// The real packer: the 24 bytes of TEXTURE_SHADER_STATE, or None when it
/// refuses the record (a type no row of its table has, a field too wide).
fn pack(s: &V3dTexState) -> Option<[u8; V3D_TEX_STATE_BYTES]> {
    // SAFETY: the C function's signature (v3d_tex.h).
    let f: unsafe extern "C" fn(*mut u8, *const V3dTexState) -> bool =
        unsafe { std::mem::transmute(sym("v3d_tex_pack_state")) };
    let mut out = [0u8; V3D_TEX_STATE_BYTES];
    // SAFETY: 24 writable bytes and a live state.
    unsafe { f(out.as_mut_ptr(), s) }.then_some(out)
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

fn bind(format: u8, w: u32, h: u32) -> GlesTextureBind {
    GlesTextureBind {
        width: w,
        height: h,
        stride_texels: (w + 1) & !1,
        layers: 1,
        mip_levels: 1,
        format,
        ..GlesTextureBind::ZERO
    }
}

/// A 2D array of RGBA8, `layers` layers and `levels` levels.
fn array(w: u32, h: u32, layers: u32, levels: u32) -> GlesTextureBind {
    GlesTextureBind {
        type_: GLES_TEXTYPE_2D_ARRAY,
        layers: layers as u16,
        mip_levels: levels as u8,
        mip_filter: u8::from(levels > 1),
        ..bind(GLES_TEXFMT_RGBA8, w, h)
    }
}

/// A 3D texture of RGBA8, `depth` deep (`layers`, src/backend.rs), `levels`
/// levels.
fn vol(w: u32, h: u32, depth: u32, levels: u32) -> GlesTextureBind {
    GlesTextureBind { type_: GLES_TEXTYPE_3D, ..array(w, h, depth, levels) }
}

/// The dimension fields of a packed record (v3d_packet.xml, Texture Shader
/// State, V3D 4.2): (Array Stride (64-byte aligned) bits 32..57, Image
/// Width 58..71, Image Height 72..85, Image Depth 86..99).
fn decode_dims(rec: &[u8; V3D_TEX_STATE_BYTES]) -> (u32, u32, u32, u32) {
    let mut lo = [0u8; 16];
    lo.copy_from_slice(&rec[..16]);
    let x = u128::from_le_bytes(lo);
    let field = |start: u32, size: u32| ((x >> start) & ((1u128 << size) - 1)) as u32;
    (field(32, 26), field(58, 14), field(72, 14), field(86, 14))
}

/* ---- the GL API over the V3D back end ---------------------------------------
 * The real back end (`yakogl_backend_v3d`) behind the GL core, the job
 * builder (v3d_job.c) stood in for by fakes, as tests/depth_resolve_v3d.rs
 * does, and a "GPU" that stores a typed clear into the raster target. */

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

    const VS: &str =
        "#version 300 es\nlayout(location = 0) in vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";
    const FS: &str = concat!(
        "#version 300 es\nprecision highp float;\nuniform sampler2D s;\nout vec4 o;\n",
        "void main() { o = texture(s, vec2(0.5)); }\n"
    );
    static QUAD: [f32; 18] = [
        -1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, //
        -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0,
    ];

    /// The GL error a draw sampling an array or 3D texture leaves for the
    /// application: none, the refusal is lifted. While the draw was refused it
    /// was GL_OUT_OF_MEMORY (the back end's -EINVAL); v3d-refuse-enotsup makes
    /// a refusal of legal state GL_INVALID_OPERATION.
    const DRAW_ERROR: GLenum = GL_NO_ERROR;

    const FS_ARRAY: &str = concat!(
        "#version 300 es\nprecision highp float;\nuniform highp sampler2DArray s;\nout vec4 o;\n",
        "void main() { o = texture(s, vec3(0.5, 0.5, 3.0)) + vec4(float(textureSize(s, 0).z)); }\n"
    );
    const FS_3D: &str = concat!(
        "#version 300 es\nprecision highp float;\nuniform highp sampler3D s;\nout vec4 o;\n",
        "void main() { o = texture(s, vec3(0.5, 0.5, 0.7)) + vec4(float(textureSize(s, 0).z)); }\n"
    );

    /// A 4x4x5 RGBA8 texture of `target` bound to unit 0, sampled by a draw
    /// into the canvas: the GL error after the draw, the draw's texture record
    /// and the back end's log.
    fn sample(target: GLenum, fs: &str) -> (GLenum, Option<[u8; V3D_TEX_STATE_BYTES]>, Vec<String>) {
        let prog = program(VS, fs);
        let mut tex = 0u32;
        let texels = vec![0x40u8; 4 * 4 * 5 * 4];
        // SAFETY: plain GL calls; `texels` is 4 x 4 x 5 RGBA8.
        let err = unsafe {
            glGenTextures(1, &mut tex);
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(target, tex);
            glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glTexImage3D(target, 0, GL_RGBA8 as GLint, 4, 4, 5, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, CANVAS.0, CANVAS.1);
            glUseProgram(prog);
            glUniform1i(glGetUniformLocation(prog, c"s".as_ptr()), 0);
            glVertexAttribPointer(0, 3, GL_FLOAT, 0, 12, QUAD.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 6);
            glFinish();
            glGetError()
        };
        let rec = last_tmu_record();
        let log = logs();
        // SAFETY: names made above.
        unsafe {
            glDeleteTextures(1, &tex);
            glDeleteProgram(prog);
        }
        (err, rec, log)
    }

    fn refusals(log: &[String]) -> Vec<&String> {
        log.iter()
            .filter(|l| l.contains("not sampled") || l.contains("does not sample") || l.contains("refus"))
            .collect()
    }

    /// A draw sampling a 2D array, then a 3D texture: no GL error, a texture
    /// record in the job with Image Depth 5 and the layout's Array Stride, no
    /// refusal logged; the context draws a plain 2D texture afterwards.
    #[test]
    fn a_draw_sampling_an_array_or_a_3d_texture_reaches_the_job() {
        let _g = rig();
        for (target, fs) in [(GL_TEXTURE_2D_ARRAY, FS_ARRAY), (GL_TEXTURE_3D, FS_3D)] {
            let (err, rec, log) = sample(target, fs);
            assert_eq!(err, DRAW_ERROR, "0x{target:x}: {log:?}");
            assert!(refusals(&log).is_empty(), "0x{target:x}: {:?}", refusals(&log));
            let rec = rec.unwrap_or_else(|| panic!("0x{target:x}: the draw carries a texture record: {log:?}"));
            let (stride, w, h, depth) = decode_dims(&rec);
            assert_eq!((w, h, depth), (4, 4, 5), "0x{target:x}");
            let l;
            let want = if target == GL_TEXTURE_3D {
                l = texture::layout(&vol(4, 4, 5, 1)).expect("lays out");
                l.slices[0].size / 64
            } else {
                l = texture::layout(&array(4, 4, 5, 1)).expect("lays out");
                // SAFETY: a live layout.
                let stride = unsafe { v3d_tex_layer_stride(&l) };
                stride / 64
            };
            assert_eq!(stride, want, "0x{target:x}");
            assert_eq!(decode(&rec).0, 4, "RGBA8_UNORM");
        }
        /* still usable: a plain 2D texture draws */
        let prog = program(VS, FS);
        let mut tex = 0u32;
        // SAFETY: plain GL calls; the texels are 2x2 RGBA8.
        let err = unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            let texels = [0x40u8; 16];
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA8 as GLint,
                2,
                2,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                texels.as_ptr().cast(),
            );
            glUseProgram(prog);
            glUniform1i(glGetUniformLocation(prog, c"s".as_ptr()), 0);
            glDrawArrays(GL_TRIANGLES, 0, 6);
            glFinish();
            let e = glGetError();
            glDeleteTextures(1, &tex);
            glDeleteProgram(prog);
            e
        };
        assert_eq!(err, GL_NO_ERROR, "{:?}", logs());
        assert_eq!(decode_dims(&last_tmu_record().expect("a record")).3, 1);
    }

    /// Layers rendered through a framebuffer, then the whole array sampled: 5
    /// render jobs at `base + layer * stride` and a window job whose record
    /// has Image Depth 5 (docs/notes/v3d-fbo-render.md section 4, "Tests").
    /// Open: the rig's fake job builder records targets, and whether a layer's
    /// job address is the array's base plus the layer stride is the V3D back
    /// end's render-image rule, not asserted yet. Plan #192's shaderc half
    /// (TMUI / TMUR writes, textureSize) and `refusal_for` returning `None`
    /// have landed (8ae0a44b); this is the remaining test of the note.
    #[test]
    #[ignore = "fails today: the five layer clears do not each make a job on the rig; see the comment"]
    fn a_rendered_layer_is_sampled_with_the_arrays_stride() {
        let _g = rig();
        let (mut tex, mut fbo) = (0u32, 0u32);
        // SAFETY: plain GL calls on names made here.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
            glTexImage3D(
                GL_TEXTURE_2D_ARRAY,
                0,
                GL_RGBA8 as GLint,
                8,
                8,
                5,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                core::ptr::null(),
            );
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            for layer in 0..5 {
                glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, layer);
                glViewport(0, 0, 8, 8);
                glClearColor(layer as f32 / 4.0, 0.0, 0.0, 1.0);
                glClear(GL_COLOR_BUFFER_BIT);
            }
            glFinish();
        }
        let n = jobs().len();
        assert!(n >= 5, "one job per layer");
        let b = array(8, 8, 5, 1);
        let l = texture::layout(&b).unwrap();
        // SAFETY: a live layout.
        let stride = unsafe { v3d_tex_layer_stride(&l) };
        let addrs: Vec<u32> = jobs()[n - 5..].iter().map(|j| j.target.address).collect();
        for (i, a) in addrs.iter().enumerate() {
            assert_eq!(*a, addrs[0] + i as u32 * stride, "layer {i}");
        }
    }
}

/* ---- the pure side: refusal, layout, state ----------------------------------- */

/// The two types are no longer refused by name; a malformed binding still is.
#[test]
fn an_array_and_a_3d_binding_are_sampled_and_a_malformed_one_is_not() {
    for b in [array(4, 4, 5, 1), vol(4, 4, 5, 1), array(64, 32, 4, 7), vol(32, 32, 8, 6)] {
        assert_eq!(texture::refusal(&b), None, "type {} layers {}", b.type_, b.layers);
    }
    let none = GlesTextureBind { layers: 0, ..array(4, 4, 5, 1) };
    assert!(texture::refusal(&none).is_some(), "no layer at all");
    let wide = array(1 << 14, 4, 5, 1);
    assert_eq!(texture::refusal(&wide), Some("a texture size the TMU cannot address (0, or 16384 or more)"));
}

/// 8x8 RGBA8, 5 layers, one level: layer `f` at `offset[0] + f * stride` with
/// the tree stride Mesa's cube_map_stride is, the record's Image Depth 5 and
/// Array Stride `stride / 64`. Read from the layout, not restated.
#[test]
fn a_5_layer_rgba8_array_has_a_stride_per_layer() {
    let b = array(8, 8, 5, 1);
    let l = texture::layout(&b).expect("lays out");
    // SAFETY: a live layout.
    let stride = unsafe { v3d_tex_layer_stride(&l) };
    assert_eq!(stride % 64, 0);
    assert!(stride >= l.size, "a layer holds its tree");
    for f in 0..5 {
        assert_eq!(texture::image_offset(&b, &l, 0, f), Some(l.slices[0].offset + f * stride), "layer {f}");
    }
    assert_eq!(texture::image_offset(&b, &l, 0, 5), None);
    assert_eq!(texture::image_bytes(&b, &l), Some(stride * 5));
    let st = texture::state(&b, &l, 0x0010_0000);
    assert_eq!((st.depth, st.array_stride_64, st.base), (5, stride / 64, 0x0010_0000 + l.slices[0].offset));
    assert_eq!(decode_dims(&pack(&st).expect("packs")), (stride / 64, 8, 8, 5));
}

/// 8x8x5 RGBA8: slice `s` at `offset[0] + s * size` (one slice apart, no tree
/// stride), the whole stack `size`, Image Depth 5 and Array Stride one slice
/// / 64.
#[test]
fn a_5_slice_3d_texture_has_a_stride_per_slice() {
    let b = vol(8, 8, 5, 1);
    let l = texture::layout(&b).expect("lays out");
    let slice = l.slices[0].size;
    for s in 0..5 {
        assert_eq!(texture::image_offset(&b, &l, 0, s), Some(l.slices[0].offset + s * slice), "slice {s}");
    }
    assert_eq!(texture::image_offset(&b, &l, 0, 5), None);
    assert_eq!(texture::image_bytes(&b, &l), Some(l.size));
    let st = texture::state(&b, &l, 0x0010_0000);
    assert_eq!((st.depth, st.array_stride_64), (5, slice / 64));
    assert_eq!(decode_dims(&pack(&st).expect("packs")), (slice / 64, 8, 8, 5));
    /* level 1 of a depth of 5 has 2 slices (texture::level_depth) */
    let b2 = vol(8, 8, 5, 2);
    let l2 = texture::layout(&b2).expect("lays out");
    assert_eq!(texture::level_depth(5, 1), 2);
    assert!(texture::image_offset(&b2, &l2, 1, 1).is_some());
    assert_eq!(texture::image_offset(&b2, &l2, 1, 2), None);
}
