// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What the V3D and the GC7000UL back ends do today with a draw into an
//! integer colour attachment beside a float/unorm one (RGBA8 + R32UI) and
//! beside another integer one (R32UI + R32I), through the GL API, the real GL
//! core and the real back end, with a fragment shader that writes every
//! output.
//!
//! A draw into mixed integer and float/unorm attachments is about to carry a
//! new fragment-variant key from `current_blend_key` (src/gl/draw.rs), for the
//! PowerVR. The V3D and the Vivante back ends never receive that key, so
//! neither may change with it. This file pins what they do now, so that a
//! change that reaches them shows. It changes nothing in src/. The PowerVR's
//! counterpart is tests/es3_int_rt.rs.
//!
//! Pinned today:
//!
//! - V3D: both mixes are drawn: no GL error, no refusal (`v3d_refused_draws`
//!   unchanged), one typed job of two render targets, one draw, the TLB output
//!   image format of each target Mesa's row for its surface. The job builder
//!   is a stand-in that records and draws nothing, so no pixel value is read
//!   (the compiler's TLB words are in tests/es3_int_rt_v3d.rs).
//! - GC7000UL: both mixes are drawn (no GL error) and reach the pixel engine,
//!   the second target in the 8-RT bank (`PE_RT_CONFIG_8(1)`). The stand-in
//!   OSAL's PE model only stores 64-bit pixels (tests/es3_int_rt_mrt_gc.rs),
//!   so no pixel value is read.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_int_mixed_other_gpus
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_int_mixed_other_gpus
//! ```
#![cfg(all(
    feature = "shaderc-online",
    not(feature = "report-es2"),
    any(feature = "gpu-vivante", feature = "gpu-v3d"),
    not(feature = "gpu-powervr")
))]
#![allow(dead_code)]

#[cfg(feature = "gpu-v3d")]
mod v3d {
    use core::ffi::{c_char, c_int, c_void, CStr};
    use std::collections::BTreeMap;
    use std::ffi::CString;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::{Mutex, MutexGuard, Once, OnceLock};
    use yakogl::backend::{GLES_SURF_R32I, GLES_SURF_R32UI, GLES_SURF_RGBA8888};
    use yakogl::gl::consts::*;
    use yakogl::gl::context;
    use yakogl::gl::draw::glDrawArrays;
    use yakogl::gl::framebuffer::*;
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::state::*;
    use yakogl::gl::types::*;
    use yakogl::gl::vertex::*;
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
    use yakogl::v3d::backend::{v3d_refused_draws, yakogl_backend_v3d};
    use yakogl::v3d::ffi::*;
    use yakogl::v3d::kick::V3dKick;
    use yakogl::v3d::state;

    /* ---- the job builder: records, draws nothing ---- */

    #[derive(Clone, Debug)]
    struct SeenDraw {
        st: V3dJobState,
        /// The fragment block: the clear colour as raw words.
        fs_storage: Vec<u32>,
        fs_uniforms: Vec<V3dUniform>,
        count: u32,
    }

    #[derive(Default)]
    struct Seen {
        jobs: Vec<V3dJobTarget>,
        clears: Vec<V3dJobClear>,
        draws: Vec<SeenDraw>,
        log: Vec<String>,
    }

    static SEEN: Mutex<Seen> =
        Mutex::new(Seen { jobs: Vec::new(), clears: Vec::new(), draws: Vec::new(), log: Vec::new() });

    fn seen<R>(f: impl FnOnce(&mut Seen) -> R) -> R {
        f(&mut SEEN.lock().unwrap_or_else(|e| e.into_inner()))
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
        seen(|s| s.jobs.push(t));
        t.width != 0 && t.height != 0
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
        // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
        let (j, d) = unsafe { (&mut *j, &*d) };
        j.draws += 1;
        /// A slice the draw names, empty when it names none (an application
        /// draw with no fragment block hands a null pointer and a zero count).
        unsafe fn words<T: Clone>(p: *const T, n: u32) -> Vec<T> {
            if p.is_null() || n == 0 {
                return Vec::new();
            }
            // SAFETY: the caller's slice, as long as the draw says.
            unsafe { core::slice::from_raw_parts(p, n as usize) }.to_vec()
        }
        // SAFETY: the job's BCL is live; the draw's slices are as long as it says.
        unsafe {
            (*j.bcl).used += 120;
            let rec = SeenDraw {
                st: *d.st,
                fs_storage: words(d.fs_storage.words, d.fs_storage.count),
                fs_uniforms: words((*d.fs).uniforms, (*d.fs).num_uniforms),
                count: d.count,
            };
            seen(|s| s.draws.push(rec));
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
        // SAFETY: the backend passes its live job and clear; its lists are live.
        unsafe {
            seen(|s| s.clears.push(*c));
            (*(*j).bcl).used += 1;
            (*(*j).rcl).used = 200;
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

    /// The tile-size table of v3d_frame.c, as the backend's unit tests fake it.
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
    unsafe extern "C" fn v3d_frame_alloc_sizes(t: *const V3dTiling, _d: u32, _page: u32, ta: *mut u32, ts: *mut u32) {
        // SAFETY: the backend's parameters.
        unsafe {
            let n = (*t).tiles_x * (*t).tiles_y;
            *ta = n * 128 + 8192;
            *ts = n * 256;
        }
    }

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
            let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_copy_reasons");
            std::fs::create_dir_all(&dir).expect("the scratch directory");
            let so = dir.join(format!("v3d_tex-{}.so", std::process::id()));
            /* the compiler's own temporary files next to the object */
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

    /* The signatures are src/v3d/ffi.rs's, kept as written (see
     * tests/depth_resolve_v3d.rs: rustfmt releases wrap these differently). */
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
        // SAFETY: the library's call, passed on; `s` is its live state.
        unsafe { f(out, s) }
    }

    /* ---- the platform: memory that answers, jobs that finish at once ---- */

    struct Alloc {
        mem: Box<[u64]>,
        size: usize,
    }
    struct Platform {
        next: u64,
        allocs: BTreeMap<u64, Alloc>,
        submitted: usize,
    }
    static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

    fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
        let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
        f(g.get_or_insert_with(|| Platform { next: 0, allocs: BTreeMap::new(), submitted: 0 }))
    }

    unsafe extern "C" fn p_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
        with_platform(|p| {
            if size == 0 || size > 256 << 20 {
                return -12;
            }
            let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
            let gpu = 0x0100_0000 + p.next;
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
    unsafe extern "C" fn p_submit(_kick: *const c_void, len: usize) -> i64 {
        if len != size_of::<V3dKick>() {
            return -22;
        }
        with_platform(|p| {
            p.submitted += 1;
            p.submitted as i64
        })
    }
    unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
        0
    }
    unsafe extern "C" fn p_completed() -> i64 {
        with_platform(|p| p.submitted as i64)
    }
    unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
        use yakogl::osal::{OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_VPM_BYTES};
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
        if what == OSAL_QUERY_V3D_BIN_POOL {
            return -61;
        }
        -22
    }
    /// The library logs every line as `log("%s", line)` (`osal::log_str`).
    unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
        // SAFETY: the library passes a NUL-terminated line.
        let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
        seen(|x| x.log.push(s));
    }
    unsafe extern "C" fn p_init(_platform_arg: *mut c_void) -> i32 {
        0
    }

    type LogFn = unsafe extern "C" fn(*const c_char, ...);

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

    /* ---- the context ---- */

    static LOCK: Mutex<()> = Mutex::new(());
    static ONCE: Once = Once::new();

    /// The one context and back end of the module; the tests take turns and
    /// each starts with nothing recorded and nothing pending.
    fn rig() -> MutexGuard<'static, ()> {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        ONCE.call_once(|| {
            // SAFETY: statics; the lock serialises every GL call of the module.
            unsafe {
                osal_set_ops(osal());
                yakogl::backend::g_backend = &yakogl_backend_v3d;
                assert_eq!(yakogl::backend::init(), 0, "{:?}", seen(|s| s.log.clone()));
                assert!(!context::gl_context_create(32, 32).is_null(), "{:?}", seen(|s| s.log.clone()));
            }
        });
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            while glGetError() != GL_NO_ERROR {}
        }
        seen(|s| *s = Seen::default());
        g
    }

    const W: GLsizei = 24;
    const H: GLsizei = 16;

    /// A program writing each `(location, type)` output a constant: "vec4",
    /// "ivec4" or "uvec4".
    fn mixed_program(outs: &[(u32, &str)]) -> GLuint {
        let mut fs = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
        for (loc, t) in outs {
            fs += &format!("layout(location = {loc}) out {t} o{loc};\n");
        }
        fs += "void main() {\n";
        for (loc, t) in outs {
            fs += &match *t {
                "vec4" => format!("  o{loc} = vec4(0.5, 1.0, 0.25, 0.75);\n"),
                "ivec4" => format!("  o{loc} = ivec4(-1, 2, -3, 4);\n"),
                _ => format!("  o{loc} = uvec4(1u, 2u, 3u, 4u);\n"),
            };
        }
        fs += "}\n";
        let compile = |ty: GLenum, src: &str| {
            let c = CString::new(src).unwrap();
            // SAFETY: plain GL calls; the string lives for the call.
            unsafe {
                let s = glCreateShader(ty);
                let srcs = [c.as_ptr()];
                glShaderSource(s, 1, srcs.as_ptr(), core::ptr::null());
                glCompileShader(s);
                let mut ok: GLint = 0;
                glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
                assert_eq!(ok, 1, "compile: {src}\n{:?}", seen(|s| s.log.clone()));
                s
            }
        };
        let v = compile(
            GL_VERTEX_SHADER,
            "#version 300 es\nin vec4 a_position;\nvoid main() { gl_Position = a_position; }\n",
        );
        let f = compile(GL_FRAGMENT_SHADER, &fs);
        // SAFETY: plain GL calls on handles just made.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glBindAttribLocation(p, 0, c"a_position".as_ptr());
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link: {:?}", seen(|s| s.log.clone()));
            glUseProgram(p);
            p
        }
    }

    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

    /// Draw a triangle over `formats` (renderbuffers at attachments 0.., all
    /// of them draw buffers) with a program writing `outs`; return the GL
    /// error after the draw, the draws the back end refused, and the job
    /// builder's record of the draw.
    fn mixed_draw(formats: &[GLenum], outs: &[(u32, &str)]) -> (GLenum, u32, Seen) {
        let _g = rig();
        let refused0 = v3d_refused_draws();
        let n = formats.len();
        let (mut fb, mut rbs) = (0, vec![0; n]);
        // SAFETY: plain GL calls with room for the names; TRI is static.
        let error = unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenRenderbuffers(n as GLsizei, rbs.as_mut_ptr());
            let mut bufs = Vec::new();
            for (i, &f) in formats.iter().enumerate() {
                glBindRenderbuffer(GL_RENDERBUFFER, rbs[i]);
                glRenderbufferStorage(GL_RENDERBUFFER, f, W, H);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rbs[i]);
                bufs.push(GL_COLOR_ATTACHMENT0 + i as GLenum);
            }
            glDrawBuffers(n as GLsizei, bufs.as_ptr());
            assert_eq!(glGetError(), GL_NO_ERROR, "{formats:#x?}: {:?}", seen(|s| s.log.clone()));
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{formats:#x?}");
            glViewport(0, 0, W, H);
            let p = mixed_program(outs);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            seen(|s| *s = Seen::default());
            glDrawArrays(GL_TRIANGLES, 0, 3);
            let e = glGetError();
            glFinish();
            let e = if e == GL_NO_ERROR { glGetError() } else { e };
            glUseProgram(0);
            glDeleteProgram(p);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteRenderbuffers(n as GLsizei, rbs.as_ptr());
            e
        };
        (error, v3d_refused_draws().wrapping_sub(refused0), seen(core::mem::take))
    }

    /// The TLB output image format Mesa's table gives the surface.
    fn rt_output(surf: u8) -> u32 {
        state::rt_format(surf).expect("a TLB row").output
    }

    /// The one typed job of a two-target draw with these surfaces.
    fn assert_one_typed_job(r: (GLenum, u32, Seen), surfs: [u8; 2]) {
        let (error, refused, s) = r;
        assert_eq!(error, GL_NO_ERROR, "{:?}", s.log);
        assert_eq!(refused, 0, "no draw refused: {:?}", s.log);
        assert_eq!(s.jobs.len(), 1, "one job: {:?}", s.log);
        assert_eq!(s.draws.len(), 1, "one draw");
        let j = s.jobs[0];
        assert!(j.typed, "a typed job");
        assert_eq!(j.rts, 2);
        assert_eq!(j.rt_format[0], rt_output(surfs[0]));
        assert_eq!(j.rt_format[1], rt_output(surfs[1]));
    }

    /// RGBA8 + R32UI: drawn, one typed job of two render targets.
    #[test]
    fn rgba8_beside_r32ui_is_one_typed_job_of_two_targets() {
        let r = mixed_draw(&[GL_RGBA8, GL_R32UI], &[(0, "vec4"), (1, "uvec4")]);
        assert_one_typed_job(r, [GLES_SURF_RGBA8888, GLES_SURF_R32UI]);
    }

    /// R32UI + R32I: drawn, one typed job of two render targets.
    #[test]
    fn r32ui_beside_r32i_is_one_typed_job_of_two_targets() {
        let r = mixed_draw(&[GL_R32UI, GL_R32I], &[(0, "uvec4"), (1, "ivec4")]);
        assert_one_typed_job(r, [GLES_SURF_R32UI, GLES_SURF_R32I]);
    }
}

#[cfg(feature = "gpu-vivante")]
mod gc {
    use core::ffi::{c_char, c_void};
    use std::collections::BTreeMap;
    use std::ffi::CString;
    use std::sync::Mutex;
    use yakogl::gl::consts::*;
    use yakogl::gl::draw::*;
    use yakogl::gl::framebuffer::*;
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::state::*;
    use yakogl::gl::types::*;
    use yakogl::gl::vertex::*;
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
    use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
    use yakogl::vivante::regs::*;

    /// The online compiler's store (scratch and keep extents) is one static for
    /// the process: every test takes this one lock (as tests/es3_int_rt_gc.rs).
    static STORE: Mutex<()> = Mutex::new(());

    /// The value of `#define <name>` in driver/src/vivante/hw/state_3d.xml.h.
    fn rnndb(name: &str) -> u32 {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/driver/src/vivante/hw/state_3d.xml.h");
        let text = std::fs::read_to_string(path).expect("the vendored rnndb header");
        let line = text
            .lines()
            .find(|l| l.split_whitespace().nth(1) == Some(name) && l.starts_with("#define"))
            .unwrap_or_else(|| panic!("{name} is in state_3d.xml.h"));
        let v = line.split_whitespace().nth(2).expect("a value");
        u32::from_str_radix(v.trim_start_matches("0x"), 16).expect("hex")
    }

    /* ---- the stand-in OSAL: host memory, every stream, a model of the PE ------ */

    /// The 64-bit pixel the modelled PE stores at (x, y) of the target.
    type Pattern = fn(u32, u32) -> u64;

    #[derive(Default)]
    struct Platform {
        allocs: BTreeMap<u64, Box<[u64]>>,
        bump: BTreeMap<u64, u64>,
        fence: i64,
        kicks: Vec<Vec<u32>>,
        /// While set, every kick that names a colour surface gets `(w, h,
        /// pattern)` stored into it, in 4x4 tiles at the stream's stride.
        model: Option<(u32, u32, Pattern)>,
    }

    static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

    fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
        let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
        f(g.get_or_insert_with(Platform::default))
    }

    unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
        if size == 0 || size > 1 << 30 {
            return -12;
        }
        with_platform(|p| {
            let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
            let at = p.bump.entry(heap_base).or_insert(0);
            let gpu = heap_base + 0x1000 + *at;
            *at += (size as u64).div_ceil(4096) * 4096;
            let cpu = mem.as_mut_ptr() as *mut c_void;
            // SAFETY: the caller passes a writable descriptor.
            unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
            p.allocs.insert(gpu, mem);
            0
        })
    }

    unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
        // SAFETY: the caller passes a live descriptor; it is cleared (the OSAL contract).
        unsafe {
            let m = *mem;
            with_platform(|p| p.allocs.remove(&m.gpu));
            *mem = OsalGpuMem::ZERO;
        }
        0
    }

    unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

    /// Byte offset of pixel (x, y) in a 4x4-tiled surface `tw` pixels wide at 8
    /// bytes a pixel: written here from the hardware's layout, not taken from
    /// `wide_target.rs`, so that a wrong copy there shows.
    fn tiled_byte(x: u32, y: u32, tw: u32) -> usize {
        (((y / 4) * (tw / 4) + x / 4) * 16 + (y % 4) * 4 + x % 4) as usize * 8
    }

    /// Every `(register, value)` a LOAD_STATE of the stream writes, in order.
    fn writes(words: &[u32]) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            if w & 0xf800_0000 == GC_OP_LOAD_STATE {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for k in 0..count {
                    if let Some(v) = words.get(i + 1 + k) {
                        out.push((base + 4 * k as u32, *v));
                    }
                }
                i += 1 + count;
                continue;
            }
            i += match w & 0xf800_0000 {
                GC_OP_DRAW_INSTANCED => 4,
                GC_OP_STALL => 2,
                _ => 1,
            };
        }
        out
    }

    unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
        if len != core::mem::size_of::<GcKick>() {
            return -22;
        }
        // SAFETY: the back end passes a live GcKick of that size.
        let k = unsafe { *(kick as *const GcKick) };
        with_platform(|p| {
            if k.magic != GC_KICK_MAGIC {
                return -22;
            }
            let Some(a) = p.allocs.get(&k.stream_gpu) else { return -22 };
            // SAFETY: the allocation holds at least `stream_bytes` bytes.
            let words: &[u32] =
                unsafe { core::slice::from_raw_parts(a.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
            let stream = words.to_vec();
            if let Some((w, h, pat)) = p.model {
                let ws = writes(&stream);
                let addr = ws.iter().rev().find(|(r, _)| *r == pe_pipe_color_addr(0)).map(|(_, v)| *v as u64);
                let stride = ws.iter().rev().find(|(r, _)| *r == rnndb("VIVS_PE_COLOR_STRIDE")).map(|(_, v)| *v);
                if let (Some(addr), Some(stride)) = (addr, stride) {
                    let tw = stride / 8;
                    if let Some((base, mem)) = p.allocs.range_mut(..=addr).next_back().map(|(g, m)| (*g, m)) {
                        let off = (addr - base) as usize;
                        // SAFETY: the shadow holds padded(w, h) 8-byte pixels from `off` on.
                        let bytes =
                            unsafe { core::slice::from_raw_parts_mut(mem.as_mut_ptr() as *mut u8, mem.len() * 8) };
                        for y in 0..h {
                            for x in 0..w {
                                let at = off + tiled_byte(x, y, tw);
                                bytes[at..at + 8].copy_from_slice(&pat(x, y).to_le_bytes());
                            }
                        }
                    }
                }
            }
            p.kicks.push(stream);
            p.fence += 1;
            p.fence
        })
    }

    unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
        0
    }

    unsafe extern "C" fn p_completed() -> i64 {
        with_platform(|p| p.fence)
    }

    unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
        use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
        if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
            return 0;
        }
        -22
    }

    static OSAL: OsalOps = OsalOps {
        init: None,
        shutdown: None,
        gpu_alloc: Some(p_gpu_alloc),
        gpu_free: Some(p_gpu_free),
        mmu_map: None,
        map_phys: None,
        gpu_write: None,
        gpu_read: None,
        cache_flush: Some(p_cache),
        cache_invd: Some(p_cache),
        submit: Some(p_submit),
        wait: Some(p_wait),
        completed: Some(p_completed),
        reg_read: None,
        reg_write: None,
        query: Some(p_query),
        log: None,
        wait_vblank: None,
        present: None,
        lock: None,
        unlock: None,
        thread_id: None,
        display_busy: None,
    };

    /// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
    /// 64x64 context.
    fn on_gc7000(case: impl FnOnce()) {
        let _g = STORE.lock().unwrap_or_else(|e| e.into_inner());
        with_platform(|p| p.model = None);
        // SAFETY: the guard serialises the tests; the tables are statics.
        let ctx = unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
            let c = yakogl::gl::context::gl_context_create(64, 64);
            assert!(!c.is_null(), "context pool exhausted");
            c
        };
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
        // SAFETY: the context made above.
        unsafe {
            yakogl::gl::context::gl_context_destroy(ctx);
            yakogl::vivante::backend::gc_shutdown();
        }
        with_platform(|p| {
            /* leaked, not freed, as tests/gc_npot_repeat.rs: an address a static
             * still names must stay memory */
            for (_, m) in core::mem::take(&mut p.allocs) {
                core::mem::forget(m);
            }
        });
        if let Err(e) = r {
            std::panic::resume_unwind(e);
        }
    }

    /* ---- GL helpers ------------------------------------------------------------ */

    fn err() -> GLenum {
        // SAFETY: a plain GL call.
        unsafe { glGetError() }
    }

    fn shader(ty: GLenum, text: &str) -> GLuint {
        let text = CString::new(text).unwrap();
        // SAFETY: plain GL calls; one NUL-terminated source.
        unsafe {
            let s = glCreateShader(ty);
            let p: [*const c_char; 1] = [text.as_ptr()];
            glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "{text:?} did not compile");
            s
        }
    }

    fn program(fs: &str) -> GLuint {
        const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
            glBindAttribLocation(p, 0, c"p".as_ptr());
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link failed");
            p
        }
    }

    fn renderbuffer(internal: GLenum, w: u32, h: u32) -> GLuint {
        let mut rb = 0;
        // SAFETY: plain GL calls.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, internal, w as GLsizei, h as GLsizei);
        }
        assert_eq!(err(), GL_NO_ERROR, "glRenderbufferStorage(0x{internal:x}, {w}x{h})");
        rb
    }

    /// A framebuffer with `rbs` at colour attachments 0.., and `bufs` (the
    /// attachments, or GL_NONE) as its draw buffers.
    fn framebuffer_with(rbs: &[GLuint], bufs: &[GLenum]) -> GLuint {
        let mut fb = 0;
        // SAFETY: plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            for (i, &rb) in rbs.iter().enumerate() {
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as u32, GL_RENDERBUFFER, rb);
            }
            glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        assert_eq!(err(), GL_NO_ERROR, "framebuffer");
        fb
    }

    /// A framebuffer with `rbs` at colour attachments 0.., every one a draw buffer.
    fn framebuffer(rbs: &[GLuint]) -> GLuint {
        let bufs: Vec<GLenum> = (0..rbs.len() as u32).map(|i| GL_COLOR_ATTACHMENT0 + i).collect();
        framebuffer_with(rbs, &bufs)
    }

    static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

    /// Draw the quad over the `w` x `h` viewport with `fs`, finish, and return
    /// the error and the writes of the streams submitted since (none if the draw
    /// was refused).
    fn draw(fs: &str, w: u32, h: u32) -> (GLenum, Vec<(u32, u32)>) {
        let kicks0 = with_platform(|p| p.kicks.len());
        // SAFETY: plain GL calls; QUAD is a static.
        let e = unsafe {
            glViewport(0, 0, w as GLsizei, h as GLsizei);
            glUseProgram(program(fs));
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
            glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
            let e = glGetError();
            glFinish();
            e
        };
        (e, with_platform(|p| p.kicks[kicks0..].iter().flat_map(|k| writes(k)).collect()))
    }

    fn last(w: &[(u32, u32)], reg: u32) -> Option<u32> {
        w.iter().rev().find(|(r, _)| *r == reg).map(|(_, v)| *v)
    }

    /// Bytes untiled out of shadows since init.
    fn untiled() -> u64 {
        yakogl::vivante::wide_target::counts().1
    }

    /// Did the draw reach the pixel engine at all?
    fn reached_pe(w: &[(u32, u32)]) -> bool {
        w.iter().any(|&(r, _)| r == GC_PE_COLOR_FORMAT || r == pe_rt_config_8(1) || r == pe_pipe_color_addr(0))
    }

    /// A fragment shader writing every output in `outs` (location, type).
    fn mrt_fs(outs: &[(u32, &str)]) -> String {
        let mut s = String::from("#version 300 es\nprecision highp float;\n");
        for (loc, ty) in outs {
            s += &format!("layout(location = {loc}) out highp {ty} o{loc};\n");
        }
        s += "void main() {\n";
        for (loc, ty) in outs {
            let v = match *ty {
                "vec4" => "vec4(0.5)".to_string(),
                t => format!("{t}(1)"),
            };
            s += &format!("  o{loc} = {v};\n");
        }
        s + "}\n"
    }

    /// The draw of two attachments: GL_NO_ERROR, and it reached the pixel
    /// engine with the second target in the 8-RT bank.
    fn assert_drawn(a: GLenum, b: GLenum, ta: &str, tb: &str) {
        on_gc7000(|| {
            let (w, h) = (16, 16);
            let ra = renderbuffer(a, w, h);
            let rb = renderbuffer(b, w, h);
            framebuffer(&[ra, rb]);
            let (e, ws) = draw(&mrt_fs(&[(0, ta), (1, tb)]), w, h);
            assert_eq!(e, GL_NO_ERROR, "{a:#x} + {b:#x}");
            assert!(reached_pe(&ws), "{a:#x} + {b:#x}: reached the pixel engine");
            assert!(last(&ws, pe_rt_config_8(1)).is_some(), "{a:#x} + {b:#x}: the second target is in the 8-RT bank");
        });
    }

    /// RGBA8 + R32UI: drawn.
    #[test]
    fn rgba8_beside_r32ui_is_drawn() {
        assert_drawn(GL_RGBA8, GL_R32UI, "vec4", "uvec4");
    }

    /// R32UI + R32I: drawn.
    #[test]
    fn r32ui_beside_r32i_is_drawn() {
        assert_drawn(GL_R32UI, GL_R32I, "uvec4", "ivec4");
    }
}
