// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glScissor and glViewport with four colour targets (the RGBA8 draw buffers
//! 0-3 of one framebuffer), through the GL API and the real GL core, on the
//! host, under the four-target stand-in `backend::HOST_DRAW_TARGETS` = 4
//! (what DRAW_TARGETS = 4 would give; docs/es3-cts-failures.md RC14). The
//! neighbours hold the full-surface clears of attachment 3
//! (tests/v3d_clear_mrt4.rs, tests/es3_mrt4_clear_pvr.rs); this file holds
//! what a box does to it: attachment 3 is touched only inside the scissor
//! box and the viewport, as attachments 0-2 are.
//!
//! Each scene first clears all four attachments to one background colour
//! (unscissored), then runs one box operation, then reads every attachment
//! back with glReadPixels and compares it with an image computed from the GL
//! rules alone (ES 3.0 4.2.3: glClear and glClearBuffer* are bounded by the
//! scissor box and not by the viewport; 2.14.1 and 4.1.2: a draw is bounded
//! by the viewport and the scissor box). What each back end did is read
//! from what it records:
//!
//! - V3D (`gpu-v3d`): the REAL back end (`yakogl_backend_v3d`) with the job
//!   builder (driver/src/v3d/v3d_job.c) stood in for by fakes that record
//!   each job (the TLB clears of the render targets it does not load, every
//!   draw's scissor box, viewport and TLB outputs) and a "GPU" that applies
//!   them to the raster of all four render targets at submit, as
//!   tests/es3_fragops_scissor_v3d.rs does for RT 0. The fake fragments are
//!   a model: a draw paints its viewport and scissor box with the colours
//!   the scene says its shader writes, a clear quad with the colours in its
//!   fragment block; what is real is the back end's decision (loaded or
//!   cleared, the boxes it hands the job builder, which RTs it writes).
//! - PowerVR (`gpu-powervr`): the PowerVR back end's own draw needs the
//!   board's job memory, so a stand-in back end takes the GL core's
//!   `GlesDraw` (as tests/es3_msaa_pvr.rs does) and applies its contract to
//!   the four targets' memory: `do_clear` with `clear_target_mask`,
//!   `clear_quad` and `scissor_rect`, and a draw inside `viewport` and
//!   `scissor_rect`. What is pinned is therefore the GL core's half: the box
//!   it sends for attachment 3 as for attachments 0-2. What the PowerVR
//!   back end then does with them is not run here (see the report).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_scissor
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_scissor
//! ```
//!
//! The file compiles to nothing under report-es2 (three draw buffers).

#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    any(feature = "gpu-v3d", feature = "gpu-powervr"),
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};

use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- what both back ends report ------------------------------------------ */

/// What one back-end operation was, as the back end recorded it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A whole-surface clear (the TLB's, the background object's): no box.
    Clear,
    /// A clear drawn as a quad, inside a scissor box.
    Quad,
    /// An application draw.
    Draw,
}

/// `targets`: bit n is attachment n written or cleared. Boxes are
/// `[x, y, w, h]` in window pixels, as GL gives them (V3D's min/max pair is
/// converted).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rec {
    kind: Kind,
    targets: u8,
    scissor: Option<[i32; 4]>,
    viewport: Option<[i32; 4]>,
}

/// One pixel, RGBA8.
type Px = [u8; 4];

const W: i32 = 16;
const H: i32 = 12;

fn byte(f: f32) -> u8 {
    (f.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

fn bytes(c: [f32; 4]) -> Px {
    c.map(byte)
}

/// The back end under test and its fakes.
#[cfg(feature = "gpu-v3d")]
mod be {
    use super::*;
    use core::ffi::{c_int, CStr};
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::OnceLock;
    use yakogl::gl::context;
    use yakogl::osal::OsalV3dCpuOf;
    use yakogl::shaderc::online::v3d::select::{tlb_config_32b, tlb_config_f16};
    use yakogl::v3d::backend::*;
    use yakogl::v3d::ffi::*;
    use yakogl::v3d::kick::V3dKick;

    /* ---- the real layout and packers, built from driver/src/v3d/v3d_tex.c ---- */

    /* ---- the real layout and packers, built from driver/src/v3d/v3d_tex.c ------ */

    extern "C" {
        fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    /// `driver/src/v3d/v3d_tex.c` and the Mesa tiling it includes, as a shared
    /// object of this test's own, reached through the handle only (RTLD_LOCAL),
    /// as tests/es3_formats_v3d.rs builds it. The back end's own start-up lays
    /// out its black texture with it.
    fn lib() -> *mut c_void {
        static L: OnceLock<usize> = OnceLock::new();
        *L.get_or_init(|| {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_mrt4_scissor");
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
    trampoline!(v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool);
    trampoline!(v3d_tex_pack_sampler(out: *mut u8, s: *const V3dSampler) -> bool);
    trampoline!(v3d_tex_pack_sampler_compare(out: *mut u8, s: *const V3dSampler, compare_op: u32) -> bool);
}

    /* ---- the job builder, stood in for (driver/src/v3d/v3d_job.c) ------------ */

    /// One draw of a job: the boxes and the TLB outputs it writes.
    #[derive(Clone, Debug)]
    struct SeenDraw {
        scissor: Option<[u32; 4]>,
        vp: V3dViewport,
        /// Bit n: RT n is written (RT 0 always; RT n > 0 by its TLB config).
        writes: u8,
        fs_storage: Vec<u32>,
    }

    #[derive(Clone, Debug)]
    struct SeenJob {
        target: V3dJobTarget,
        clear: Option<V3dJobClear>,
        draws: Vec<SeenDraw>,
    }

    static JOBS: Mutex<Vec<SeenJob>> = Mutex::new(Vec::new());
    static RECS: Mutex<Vec<Rec>> = Mutex::new(Vec::new());
    /// The colours the scene's fragment shader writes, when it draws.
    static FRAG: Mutex<Option<[Px; 4]>> = Mutex::new(None);

    fn jobs() -> MutexGuard<'static, Vec<SeenJob>> {
        JOBS.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The recorded operations since the last call, and forget them.
    pub fn take() -> Vec<Rec> {
        core::mem::take(&mut *RECS.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// The colours (RGBA8) an application draw's four outputs carry.
    pub fn set_frag(c: Option<[Px; 4]>) {
        *FRAG.lock().unwrap_or_else(|e| e.into_inner()) = c;
    }

    /// Does the draw's program write RT `rt` (1..3: its TLB config; RT 0
    /// always)?
    fn writes_rt(u: &[V3dUniform], rt: u32) -> bool {
        rt == 0
            || (1..=4).any(|n| {
                u.contains(&V3dUniform { contents: V3D_QUNIFORM_CONSTANT, data: tlb_config_32b(rt, n) })
                    || u.contains(&V3dUniform { contents: V3D_QUNIFORM_CONSTANT, data: tlb_config_f16(rt, n) })
            })
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
        jobs().push(SeenJob { target: t, clear: None, draws: Vec::new() });
        t.width != 0 && t.height != 0
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
        // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
        let (j, d) = unsafe { (&mut *j, &*d) };
        // SAFETY: the draw's state, program and block are live for the call.
        let (st, uniforms, storage) = unsafe {
            let st = &*d.st;
            let n = (*d.fs).num_uniforms as usize;
            let u = if n == 0 || (*d.fs).uniforms.is_null() {
                Vec::new()
            } else {
                core::slice::from_raw_parts((*d.fs).uniforms, n).to_vec()
            };
            let s = if d.fs_storage.words.is_null() || d.fs_storage.count == 0 {
                Vec::new()
            } else {
                core::slice::from_raw_parts(d.fs_storage.words, d.fs_storage.count as usize).to_vec()
            };
            (st, u, s)
        };
        j.draws += 1;
        // SAFETY: the job's BCL is live.
        unsafe { (*j.bcl).used += 120 };
        let writes = (0..4u32).filter(|&n| writes_rt(&uniforms, n)).fold(0u8, |a, n| a | 1 << n);
        if let Some(job) = jobs().last_mut() {
            job.draws.push(SeenDraw {
                scissor: st.scissor.then_some([st.scissor_minx, st.scissor_miny, st.scissor_maxx, st.scissor_maxy]),
                vp: st.vp,
                writes,
                fs_storage: storage,
            });
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

    /// The tile-size table of v3d_frame.c (Mesa's v3d_choose_tile_size), as the
    /// backend's unit tests fake it.
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

    fn loaded(c: &V3dJobClear, n: usize) -> bool {
        if n == 0 {
            c.load_rt0
        } else {
            c.load_rt[n - 1]
        }
    }

    /// RT n's raster: address and row stride.
    fn raster(t: &V3dJobTarget, n: usize) -> (u32, u32) {
        if n == 0 {
            (t.address, t.stride)
        } else {
            (t.rt[n - 1].address, t.rt[n - 1].stride)
        }
    }

    fn isect(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
        let (x0, y0) = (a[0].max(b[0]), a[1].max(b[1]));
        let (x1, y1) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
        [x0, y0, (x1 - x0).max(0), (y1 - y0).max(0)]
    }

    /// Paint `rect` (clipped to the target) of a raster with `px`.
    fn paint(p: &mut Platform, t: &V3dJobTarget, n: usize, rect: [i32; 4], px: Px) {
        let (addr, stride) = raster(t, n);
        if addr == 0 {
            return;
        }
        let r = isect(rect, [0, 0, t.width as i32, t.height as i32]);
        let row = (stride as usize).max(t.width as usize * 4);
        let base = platform_cpu(p, u64::from(addr), row * t.height as usize);
        for y in r[1]..r[1] + r[3] {
            for x in r[0]..r[0] + r[2] {
                // SAFETY: inside the span platform_cpu checked.
                unsafe { core::ptr::copy_nonoverlapping(px.as_ptr(), base.add(y as usize * row + x as usize * 4), 4) };
            }
        }
    }

    /// The "GPU": every job completes inside its submit. A render target the
    /// job does not load starts from its TLB clear word, one it loads keeps
    /// the raster; the draws then paint (the fake fragments, see the header)
    /// and the job's record goes to `take`.
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
        let rts = t.rts.clamp(1, 4) as usize;
        let frag = *FRAG.lock().unwrap_or_else(|e| e.into_inner());
        let mut recs = Vec::new();
        let mut cleared = 0u8;
        with_platform(|p| {
            for n in 0..rts {
                if !loaded(&c, n) {
                    cleared |= 1 << n;
                    let w = if n == 0 { c.rgba8 } else { c.rgba8_rt[n - 1] };
                    paint(p, &t, n, [0, 0, t.width as i32, t.height as i32], w.to_le_bytes());
                }
            }
            for d in &job.draws {
                let v = |i: usize| {
                    let (s, tr) = (d.vp.scale[i].abs(), d.vp.translate[i]);
                    ((tr - s).round() as i32, (2.0 * s).round() as i32)
                };
                let (vx, vw) = v(0);
                let (vy, vh) = v(1);
                let vp = [vx, vy, vw, vh];
                let sc = d.scissor.map(|[a, b, e, f]| [a as i32, b as i32, (e - a) as i32, (f - b) as i32]);
                let rect = sc.map_or(vp, |s| isect(vp, s));
                let (kind, mask) = if frag.is_some() {
                    (Kind::Draw, d.writes)
                } else if d.fs_storage.len() == 16 {
                    (Kind::Quad, 0xf)
                } else if d.writes & 0xe != 0 {
                    (Kind::Quad, d.writes & 0xe)
                } else {
                    (Kind::Quad, 1)
                };
                let mut word = 0;
                for n in (0..rts).filter(|n| mask >> n & 1 != 0) {
                    let px = match frag {
                        Some(f) => f[n],
                        None => {
                            let w = &d.fs_storage[4 * word..4 * word + 4];
                            word += 1;
                            bytes([
                                f32::from_bits(w[0]),
                                f32::from_bits(w[1]),
                                f32::from_bits(w[2]),
                                f32::from_bits(w[3]),
                            ])
                        }
                    };
                    paint(p, &t, n, rect, px);
                }
                recs.push(Rec { kind, targets: mask, scissor: sc, viewport: Some(vp) });
            }
        });
        if cleared != 0 {
            recs.insert(0, Rec { kind: Kind::Clear, targets: cleared, scissor: None, viewport: None });
        }
        RECS.lock().unwrap_or_else(|e| e.into_inner()).extend(recs);
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

    /* ---- the context ---------------------------------------------------------- */

    static LOCK: Mutex<()> = Mutex::new(());
    static ONCE: Once = Once::new();

    /// The rig's lock; the four-target stand-in goes back to 0 when it drops.
    pub struct Rig(#[allow(dead_code)] MutexGuard<'static, ()>);

    impl Drop for Rig {
        fn drop(&mut self) {
            yakogl::backend::HOST_DRAW_TARGETS.store(0, Ordering::Relaxed);
        }
    }

    /// The one context and back end of the file, set up once; the tests take
    /// turns and each starts with nothing recorded.
    pub fn rig() -> Rig {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        yakogl::backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
        ONCE.call_once(|| {
            // SAFETY: statics; the lock serialises every GL call of the file.
            unsafe {
                osal_set_ops(osal());
                yakogl::backend::g_backend = &yakogl_backend_v3d;
                assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
                assert!(!context::gl_context_create(32, 32).is_null(), "{:?}", logs());
            }
        });
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            while glGetError() != GL_NO_ERROR {}
        }
        set_frag(None);
        take();
        Rig(g)
    }

    pub fn logs() -> Vec<String> {
        with_platform(|p| p.log.clone())
    }
}

/// The back end under test: a stand-in that applies the GL core's `GlesDraw`
/// contract to the targets' memory (see the header).
#[cfg(all(feature = "gpu-powervr", not(feature = "gpu-v3d")))]
mod be {
    use super::*;
    use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, GLES_MAX_TARGETS};

    unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
        let mem = Box::leak(vec![0u8; size].into_boxed_slice());
        let p = mem.as_mut_ptr();
        // SAFETY: the caller passes a writable descriptor.
        unsafe {
            *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void }
        };
        0
    }
    unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
        0
    }
    unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
    unsafe extern "C" fn b_init() -> i32 {
        0
    }
    unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
        0x1000_0000
    }
    unsafe extern "C" fn b_ok() -> i32 {
        0
    }

    static RECS: Mutex<Vec<Rec>> = Mutex::new(Vec::new());
    /// The colours the scene's fragment shader writes, when it draws.
    static FRAG: Mutex<Option<[Px; 4]>> = Mutex::new(None);

    /// The recorded operations since the last call, and forget them.
    pub fn take() -> Vec<Rec> {
        core::mem::take(&mut *RECS.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// The colours (RGBA8) an application draw's four outputs carry.
    pub fn set_frag(c: Option<[Px; 4]>) {
        *FRAG.lock().unwrap_or_else(|e| e.into_inner()) = c;
    }

    fn isect(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
        let (x0, y0) = (a[0].max(b[0]), a[1].max(b[1]));
        let (x1, y1) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
        [x0, y0, (x1 - x0).max(0), (y1 - y0).max(0)]
    }

    /// Paint `rect` (clipped to the target) of target `t` with `px`.
    unsafe fn paint(t: &backend::GlesDrawTarget, rect: [i32; 4], px: Px) {
        let r = isect(rect, [0, 0, t.w as i32, t.h as i32]);
        let base = t.gpu as *mut u8;
        for y in r[1]..r[1] + r[3] {
            for x in r[0]..r[0] + r[2] {
                // SAFETY: inside the w x h x 4 bytes the renderbuffer's allocation holds.
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        px.as_ptr(),
                        base.add((y as usize * t.w as usize + x as usize) * 4),
                        4,
                    )
                };
            }
        }
    }

    unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
        // SAFETY: the GL core passes its live draw.
        let d = unsafe { &*d };
        let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
        let all = (1u8 << n) - 1;
        let sc = d.scissor.then_some(d.scissor_rect);
        let mut recs = RECS.lock().unwrap_or_else(|e| e.into_inner());
        if d.do_clear {
            let mask = if d.clear_target_mask == 0 { all } else { d.clear_target_mask & all };
            let rect = if d.clear_quad {
                sc.unwrap_or([0, 0, d.target_w as i32, d.target_h as i32])
            } else {
                [0, 0, d.target_w as i32, d.target_h as i32]
            };
            for (i, t) in d.targets[..n].iter().enumerate().filter(|(i, _)| mask >> i & 1 != 0) {
                let _ = i;
                // SAFETY: the targets are memory this file allocated.
                unsafe { paint(t, rect, bytes(d.clear_rgba)) };
            }
            let kind = if d.clear_quad { Kind::Quad } else { Kind::Clear };
            recs.push(Rec { kind, targets: mask, scissor: if d.clear_quad { sc } else { None }, viewport: None });
        }
        if d.index_count != 0 {
            let vp = d.viewport;
            let rect = sc.map_or(vp, |s| isect(vp, s));
            let frag = *FRAG.lock().unwrap_or_else(|e| e.into_inner());
            for (i, t) in d.targets[..n].iter().enumerate() {
                if let Some(f) = frag {
                    // SAFETY: as above.
                    unsafe { paint(t, rect, f[i]) };
                }
            }
            recs.push(Rec { kind: Kind::Draw, targets: all, scissor: sc, viewport: Some(vp) });
        }
        0
    }

    static OSAL: OsalOps = OsalOps {
        init: None,
        shutdown: None,
        gpu_alloc: Some(m_gpu_alloc),
        gpu_free: Some(m_gpu_free),
        mmu_map: None,
        map_phys: None,
        gpu_write: None,
        gpu_read: None,
        cache_flush: Some(m_cache),
        cache_invd: Some(m_cache),
        submit: None,
        wait: None,
        completed: None,
        reg_read: None,
        reg_write: None,
        query: None,
        log: None,
        wait_vblank: None,
        present: None,
        lock: None,
        unlock: None,
        thread_id: None,
        display_busy: None,
    };

    static BACKEND: YakoglBackend = YakoglBackend {
        name: c"es3-mrt4-scissor".as_ptr() as *const c_char,
        native_attrib_formats: 0,
        rt_rgba8888_is_bgra: false,
        mip_level_align: 0,
        compressed_blocks_in_rows: false,
        native_layered_textures: false,
        front_facing_shader_variant: false,
        etc1_decoded_at_upload: false,
        init: Some(b_init),
        shutdown: None,
        draw: Some(b_draw),
        flush: Some(b_ok),
        finish: Some(b_ok),
        finish_previous: None,
        kick_id: None,
        kick_pending: None,
        heap_base: Some(b_heap_base),
        counters: None,
        texture_released: None,
        surface_swapped: None,
        texture_uploaded: None,
        max_instance_divisor: 1,
        wait_kick: None,
        samples_texel_copies: false,
        attachment_gone: None,
        depth_resolve: None,
        query_begin: None,
        query_end: None,
        query_result: None,
        generate_mipmap: None,
        return_mipmap_chain: None,
        linear_row_align: 0,
        uniforms_host_only: false,
        ds_write: None,
        stencil_resolve: None,
        max_samples: 0,
    };

    static LOCK: Mutex<()> = Mutex::new(());
    static ONCE: Once = Once::new();

    /// The rig's lock; the four-target stand-in goes back to 0 when it drops.
    pub struct Rig(#[allow(dead_code)] MutexGuard<'static, ()>);

    impl Drop for Rig {
        fn drop(&mut self) {
            backend::HOST_DRAW_TARGETS.store(0, Ordering::Relaxed);
        }
    }

    /// The one context of the file, set up once; the tests take turns and
    /// each starts with nothing recorded.
    pub fn rig() -> Rig {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
        ONCE.call_once(|| {
            // SAFETY: both tables are statics; the lock above serialises the tests.
            unsafe {
                osal_set_ops(&OSAL);
                backend::g_backend = &BACKEND;
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
                assert!(!yakogl::gl::context::gl_context_create(32, 32).is_null());
            }
        });
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            while glGetError() != GL_NO_ERROR {}
        }
        set_frag(None);
        take();
        Rig(g)
    }

    pub fn logs() -> Vec<String> {
        Vec::new()
    }
}

/* ---- the scenes ------------------------------------------------------------- */

use be::{logs, rig, set_frag, take};

const BG: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// glClear's colour, and glClearBuffer's per draw buffer.
const CLEAR: [f32; 4] = [0.0, 1.0, 1.0, 0.0];
const CLEAR_EACH: [[f32; 4]; 4] =
    [[1.0, 0.0, 1.0, 1.0], [1.0, 1.0, 0.0, 1.0], [0.0, 1.0, 1.0, 1.0], [1.0, 0.0, 0.0, 0.0]];
/// What the fragment shader's outputs 0-3 are.
const DRAW: [[f32; 4]; 4] = [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0]];

/// The scissor box of the clears, and a viewport that lies inside the target
/// and is not the box.
const BOX: [i32; 4] = [3, 2, 9, 7];
const VP: [i32; 4] = [2, 1, 10, 8];
/// A second box: it cuts the viewport on the left and the top.
const BOX2: [i32; 4] = [5, 4, 10, 8];

const VS: &str = "#version 300 es\nin vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";

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

/// A fragment shader with four vec4 outputs of constant colours.
fn program() -> GLuint {
    let mut fs = String::from("#version 300 es\nprecision highp float;\n");
    for i in 0..4 {
        fs += &format!("layout(location = {i}) out vec4 o{i};\n");
    }
    fs += "void main() {\n";
    for (i, c) in DRAW.iter().enumerate() {
        fs += &format!("  o{i} = vec4({:?}, {:?}, {:?}, {:?});\n", c[0], c[1], c[2], c[3]);
    }
    fs += "}\n";
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, &fs));
    let name = CString::new("a_position").unwrap();
    // SAFETY: plain GL calls on valid handles.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, name.as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed: {:?}", logs());
        p
    }
}

/// A quad over the whole clip space at z = 0.
static QUAD: [f32; 18] = [
    -1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, //
    -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0,
];

/// A framebuffer of four RGBA8 renderbuffers, all four draw buffers on,
/// cleared to the background; the back end's record of that is dropped.
struct Scene {
    fb: GLuint,
    rbs: [GLuint; 4],
}

impl Scene {
    fn new() -> Scene {
        let mut s = Scene { fb: 0, rbs: [0; 4] };
        // SAFETY: plain GL calls with room for the names.
        unsafe {
            glGenRenderbuffers(4, s.rbs.as_mut_ptr());
            glGenFramebuffers(1, &mut s.fb);
            glBindFramebuffer(GL_FRAMEBUFFER, s.fb);
            let mut bufs = [0; 4];
            for (i, rb) in s.rbs.iter().enumerate() {
                glBindRenderbuffer(GL_RENDERBUFFER, *rb);
                glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, W, H);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, *rb);
                bufs[i] = GL_COLOR_ATTACHMENT0 + i as GLenum;
            }
            glDrawBuffers(4, bufs.as_ptr());
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
            glViewport(0, 0, W, H);
            glDisable(GL_SCISSOR_TEST);
            glClearColor(BG[0], BG[1], BG[2], BG[3]);
            glClear(GL_COLOR_BUFFER_BIT);
            glFinish();
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        take();
        s
    }

    fn scissor(&self, b: [i32; 4]) {
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_SCISSOR_TEST);
            glScissor(b[0], b[1], b[2], b[3]);
        }
    }

    fn viewport(&self, b: [i32; 4]) {
        // SAFETY: a plain GL call.
        unsafe { glViewport(b[0], b[1], b[2], b[3]) };
    }

    /// glClear of the colour buffers.
    fn clear(&self) {
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(CLEAR[0], CLEAR[1], CLEAR[2], CLEAR[3]);
            glClear(GL_COLOR_BUFFER_BIT);
        }
    }

    /// glClearBufferfv of draw buffer `k`.
    fn clear_buffer(&self, k: usize) {
        // SAFETY: plain GL call; four floats.
        unsafe { glClearBufferfv(GL_COLOR, k as GLint, CLEAR_EACH[k].as_ptr()) };
    }

    /// A draw of the whole clip space, every output a colour of its own.
    fn draw(&self) {
        set_frag(Some(DRAW.map(bytes)));
        let p = program();
        // SAFETY: plain GL calls; QUAD is static.
        unsafe {
            glUseProgram(p);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glVertexAttribPointer(0, 3, GL_FLOAT, 0, 12, QUAD.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 6);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
            glFinish();
            glUseProgram(0);
            set_frag(None);
            glDeleteProgram(p);
        }
    }

    /// Flush, and the GL error of the whole scene.
    fn finish(&self) {
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
    }

    /// Attachment `k` as glReadPixels sees it, row 0 at window y = 0.
    fn read(&self, k: usize) -> Vec<Px> {
        let mut v = vec![[0u8; 4]; (W * H) as usize];
        // SAFETY: plain GL calls; room for W x H RGBA8 pixels.
        unsafe {
            glReadBuffer(GL_COLOR_ATTACHMENT0 + k as GLenum);
            glReadPixels(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE, v.as_mut_ptr() as *mut c_void);
            assert_eq!(glGetError(), GL_NO_ERROR, "read {k}: {:?}", logs());
        }
        v
    }
}

impl Drop for Scene {
    fn drop(&mut self) {
        // SAFETY: names this scene made.
        unsafe {
            glDisable(GL_SCISSOR_TEST);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(4, self.rbs.as_ptr());
        }
    }
}

/// The four attachments' expected pixels.
struct Img([Vec<Px>; 4]);

impl Img {
    fn background() -> Img {
        Img(core::array::from_fn(|_| vec![bytes(BG); (W * H) as usize]))
    }

    /// Attachment `k`'s pixels in `rect` (clipped to the target) are `c`.
    fn fill(&mut self, k: usize, rect: [i32; 4], c: [f32; 4]) {
        let (x0, y0) = (rect[0].max(0), rect[1].max(0));
        let (x1, y1) = ((rect[0] + rect[2]).min(W), (rect[1] + rect[3]).min(H));
        for y in y0..y1 {
            for x in x0..x1 {
                self.0[k][(y * W + x) as usize] = bytes(c);
            }
        }
    }

    /// Every attachment is as expected, each named in the message.
    fn check(&self, what: &str, s: &Scene) {
        for (k, want) in self.0.iter().enumerate() {
            let got = s.read(k);
            if got != *want {
                let show = |v: &[Px]| {
                    (0..H)
                        .rev()
                        .map(|y| {
                            (0..W)
                                .map(|x| {
                                    format!(
                                        "{:02x}{:02x}{:02x}{:02x} ",
                                        v[(y * W + x) as usize][0],
                                        v[(y * W + x) as usize][1],
                                        v[(y * W + x) as usize][2],
                                        v[(y * W + x) as usize][3]
                                    )
                                })
                                .collect::<String>()
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                panic!(
                    "{what}: attachment {k} differs\nwant:\n{}\ngot:\n{}\n{:?}",
                    show(&self.0[k]),
                    show(&got),
                    logs()
                );
            }
        }
    }
}

/// (kind, targets, scissor) of each recorded operation.
fn shape(r: &[Rec]) -> Vec<(Kind, u8, Option<[i32; 4]>)> {
    r.iter().map(|r| (r.kind, r.targets, r.scissor)).collect()
}

/// `a` clipped to `b`.
fn clip(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    let (x0, y0) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x1, y1) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
    [x0, y0, (x1 - x0).max(0), (y1 - y0).max(0)]
}

/* ---- glClear and glClearBuffer under a scissor box ----------------------------- */

/// A scissored glClear reaches all four attachments, attachment 3 inside the
/// box only, as one clear quad over four targets with the GL's box.
#[test]
fn scissored_glclear_touches_attachment_3_inside_the_box_only() {
    let _g = rig();
    let s = Scene::new();
    s.scissor(BOX);
    s.clear();
    s.finish();
    let mut want = Img::background();
    for k in 0..4 {
        want.fill(k, BOX, CLEAR);
    }
    want.check("scissored glClear", &s);
    assert_eq!(shape(&take()), [(Kind::Quad, 0xf, Some(BOX))]);
}

/// glClearBufferfv of draw buffer k under a scissor box changes attachment k
/// inside the box and nothing else, for each k, attachment 3 as the others.
#[test]
fn scissored_clearbuffer_of_each_drawbuffer_touches_that_attachment_only() {
    let _g = rig();
    for (k, each) in CLEAR_EACH.iter().enumerate() {
        let s = Scene::new();
        s.scissor(BOX);
        s.clear_buffer(k);
        s.finish();
        let mut want = Img::background();
        want.fill(k, BOX, *each);
        want.check(&format!("scissored glClearBufferfv({k})"), &s);
        assert_eq!(shape(&take()), [(Kind::Quad, 1 << k, Some(BOX))], "draw buffer {k}");
    }
}

/// Two scissored glClearBufferfv in a row (draw buffers 3 then 0, different
/// boxes): each keeps what the other left, so attachment 3's box is its own.
#[test]
fn two_scissored_clearbuffers_keep_each_others_pixels() {
    let _g = rig();
    let s = Scene::new();
    s.scissor(BOX);
    s.clear_buffer(3);
    s.finish();
    s.scissor(BOX2);
    s.clear_buffer(0);
    s.finish();
    let mut want = Img::background();
    want.fill(3, BOX, CLEAR_EACH[3]);
    want.fill(0, BOX2, CLEAR_EACH[0]);
    want.check("two scissored clears", &s);
    assert_eq!(shape(&take()), [(Kind::Quad, 0x8, Some(BOX)), (Kind::Quad, 0x1, Some(BOX2))]);
}

/// A box that runs off the target on the right and the top is clamped to it;
/// one that starts left of and below the origin too.
#[test]
fn scissor_box_partly_outside_the_target_clamps_for_attachment_3() {
    let _g = rig();
    for (what, b) in [("off the far edges", [10, 8, 20, 20]), ("off the origin", [-4, -3, 8, 8])] {
        let s = Scene::new();
        s.scissor(b);
        s.clear();
        s.finish();
        let mut want = Img::background();
        for k in 0..4 {
            want.fill(k, b, CLEAR);
        }
        want.check(what, &s);
        let r = take();
        assert_eq!(r.len(), 1, "{what}: {r:?}");
        assert_eq!((r[0].kind, r[0].targets), (Kind::Quad, 0xf), "{what}");
    }
}

/// An empty box (no width) clears nothing in any attachment.
#[test]
fn empty_scissor_box_clears_no_attachment() {
    let _g = rig();
    let s = Scene::new();
    s.scissor([3, 2, 0, 5]);
    s.clear();
    s.clear_buffer(3);
    s.finish();
    Img::background().check("an empty box", &s);
}

/// The viewport does not bound glClear: with a viewport smaller than the
/// target and no scissor, glClear and glClearBufferfv of draw buffer 3 fill
/// their whole attachments, with no box.
#[test]
fn the_viewport_does_not_bound_a_clear_of_attachment_3() {
    let _g = rig();
    let s = Scene::new();
    s.viewport(VP);
    s.clear_buffer(3);
    s.finish();
    let mut want = Img::background();
    want.fill(3, [0, 0, W, H], CLEAR_EACH[3]);
    want.check("glClearBufferfv(3) under a viewport", &s);
    assert_eq!(shape(&take()), [(Kind::Clear, 0x8, None)]);
    s.clear();
    s.finish();
    for k in 0..4 {
        want.fill(k, [0, 0, W, H], CLEAR);
    }
    want.check("glClear under a viewport", &s);
    assert_eq!(shape(&take()), [(Kind::Clear, 0xf, None)]);
}

/* ---- a four-output draw under a viewport and a scissor box ---------------------- */

/// A draw of the whole clip space under a viewport smaller than the target
/// writes each of the four attachments, attachment 3 included, inside the
/// viewport rectangle only.
#[test]
fn four_output_draw_under_a_viewport_writes_attachment_3_inside_it_only() {
    let _g = rig();
    let s = Scene::new();
    s.viewport(VP);
    s.draw();
    let mut want = Img::background();
    for (k, draw) in DRAW.iter().enumerate() {
        want.fill(k, VP, *draw);
    }
    want.check("viewport draw", &s);
    let r = take();
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!((r[0].kind, r[0].targets, r[0].scissor, r[0].viewport), (Kind::Draw, 0xf, None, Some(VP)));
}

/// The same under a viewport and a scissor box that cuts it: the draw lands
/// in the two rectangles' intersection, in every attachment.
#[test]
fn four_output_draw_under_a_viewport_and_a_scissor_box_lands_in_both() {
    let _g = rig();
    let s = Scene::new();
    s.viewport(VP);
    s.scissor(BOX2);
    s.draw();
    let mut want = Img::background();
    for (k, draw) in DRAW.iter().enumerate() {
        want.fill(k, clip(VP, BOX2), *draw);
    }
    want.check("viewport and scissor draw", &s);
    let r = take();
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!((r[0].kind, r[0].targets, r[0].scissor, r[0].viewport), (Kind::Draw, 0xf, Some(BOX2), Some(VP)));
}

/// A scissored clear, then a viewport draw, then a scissored clear of draw
/// buffer 3 alone: the three operations leave each attachment as the three
/// rectangles say, attachment 3 last overwritten inside the second box only.
#[test]
fn clear_draw_clear_sequence_leaves_attachment_3_as_the_rectangles_say() {
    let _g = rig();
    let s = Scene::new();
    let mut want = Img::background();
    s.scissor(BOX);
    s.clear();
    s.finish();
    for k in 0..4 {
        want.fill(k, BOX, CLEAR);
    }
    take();
    unsafe { glDisable(GL_SCISSOR_TEST) };
    s.viewport(VP);
    s.draw();
    for (k, draw) in DRAW.iter().enumerate() {
        want.fill(k, VP, *draw);
    }
    take();
    s.scissor(BOX2);
    s.clear_buffer(3);
    s.finish();
    want.fill(3, BOX2, CLEAR_EACH[3]);
    want.check("clear, draw, clear", &s);
    assert_eq!(shape(&take()), [(Kind::Quad, 0x8, Some(BOX2))]);
}
