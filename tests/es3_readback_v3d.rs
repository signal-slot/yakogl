// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glReadPixels of integer and float colour attachments through the GL API
//! over the V3D back end on the host (ES 3.0 4.3.2; the dEQP-GLES3 fbo.* and
//! fragment_out.* cases that end with a readback of such an attachment).
//!
//! The rig is tests/es3_int_rt_v3d.rs's `gl_api`: the real GL core and the
//! real V3D back end, the job builder stood in for. The host has no tile
//! buffer, so a clear leaves the surface's memory alone; the values come from
//! the surface's memory itself (`poke`: the platform's allocation of the
//! renderbuffer, written the way the PBE would have stored the texels), and
//! what is asserted is the GL core's half: which (format, type) pairs a read
//! buffer accepts, the error of every other, the values each pair hands out
//! and the implementation's own pair. The clear itself reaching the job as
//! the right words is in tests/es3_int_rt_v3d.rs and src/v3d/backend_tests.rs.
//!
//! What the core does today that ES 3.0 4.3.2 does not is pinned, and the
//! spec's result is an `#[ignore]`d test naming its cause.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_readback_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

#[allow(dead_code)]
mod gl_api {
    use core::ffi::{c_char, c_int, c_void, CStr};
    use std::collections::BTreeMap;
    use std::ffi::CString;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::{Mutex, MutexGuard, Once, OnceLock};
    use yakogl::gl::consts::*;
    use yakogl::gl::context;
    use yakogl::gl::es3::*;
    use yakogl::gl::framebuffer::*;
    use yakogl::gl::state::*;
    use yakogl::gl::types::*;
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
    use yakogl::v3d::backend::yakogl_backend_v3d;
    use yakogl::v3d::ffi::*;
    use yakogl::v3d::kick::V3dKick;

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
        // SAFETY: the job's BCL is live; the draw's slices are as long as it says.
        unsafe {
            (*j.bcl).used += 120;
            let rec = SeenDraw {
                st: *d.st,
                fs_storage: core::slice::from_raw_parts(d.fs_storage.words, d.fs_storage.count as usize).to_vec(),
                fs_uniforms: core::slice::from_raw_parts((*d.fs).uniforms, (*d.fs).num_uniforms as usize).to_vec(),
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
            let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_readback_v3d");
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

    /* ---- the formats ---- */

    const W: GLsizei = 6;
    const H: GLsizei = 4;

    /// A colour format of the tests: its GL name, what it stores a pixel
    /// (channels, bits a channel, signed, float).
    #[derive(Clone, Copy)]
    struct Fmt {
        gl: GLenum,
        name: &'static str,
        channels: usize,
        bits: usize,
        signed: bool,
        float: bool,
    }

    const fn i(gl: GLenum, name: &'static str, channels: usize, bits: usize, signed: bool) -> Fmt {
        Fmt { gl, name, channels, bits, signed, float: false }
    }

    /// Every sized integer colour format of ES 3.0 (table 3.13).
    const INTS: [Fmt; 19] = [
        i(GL_R8I, "R8I", 1, 8, true),
        i(GL_R8UI, "R8UI", 1, 8, false),
        i(GL_R16I, "R16I", 1, 16, true),
        i(GL_R16UI, "R16UI", 1, 16, false),
        i(GL_R32I, "R32I", 1, 32, true),
        i(GL_R32UI, "R32UI", 1, 32, false),
        i(GL_RG8I, "RG8I", 2, 8, true),
        i(GL_RG8UI, "RG8UI", 2, 8, false),
        i(GL_RG16I, "RG16I", 2, 16, true),
        i(GL_RG16UI, "RG16UI", 2, 16, false),
        i(GL_RG32I, "RG32I", 2, 32, true),
        i(GL_RG32UI, "RG32UI", 2, 32, false),
        i(GL_RGBA8I, "RGBA8I", 4, 8, true),
        i(GL_RGBA8UI, "RGBA8UI", 4, 8, false),
        i(GL_RGBA16I, "RGBA16I", 4, 16, true),
        i(GL_RGBA16UI, "RGBA16UI", 4, 16, false),
        i(GL_RGBA32I, "RGBA32I", 4, 32, true),
        i(GL_RGBA32UI, "RGBA32UI", 4, 32, false),
        i(GL_RGB10_A2UI, "RGB10_A2UI", 4, 10, false),
    ];

    /// The float colour formats the GL core makes colour-renderable
    /// (GL_EXT_color_buffer_float's, as ES 3.0 itself has none).
    const FLOATS: [Fmt; 3] = [
        Fmt { gl: GL_R16F, name: "R16F", channels: 1, bits: 16, signed: true, float: true },
        Fmt { gl: GL_R32F, name: "R32F", channels: 1, bits: 32, signed: true, float: true },
        Fmt { gl: GL_RGBA16F, name: "RGBA16F", channels: 4, bits: 16, signed: true, float: true },
    ];

    impl Fmt {
        fn bytes(&self) -> usize {
            if self.bits == 10 {
                4
            } else {
                self.channels * self.bits / 8
            }
        }
        /// The value of channel `ch` of pixel (x, y) the tests store: inside
        /// the format's range, negative for a signed one, near its ends.
        fn value(&self, x: usize, y: usize, ch: usize) -> u32 {
            let bits = self.bits;
            let max = if bits == 32 { u32::MAX } else { (1u32 << bits) - 1 };
            let n = (x * 7 + y * 13 + ch * 3) as u32;
            let v = match (x + y + ch) % 4 {
                0 => n,
                1 => max - n,
                2 => max / 2 + n,
                _ => (max / 2).wrapping_add(1).wrapping_sub(n),
            } & max;
            if bits == 10 && ch == 3 {
                v & 3
            } else {
                v
            }
        }
        /// What a pixel of the pattern is read as: four words, the channels
        /// the format lacks 0,0,0,1, a signed channel sign-extended.
        fn expect(&self, x: usize, y: usize) -> [u32; 4] {
            let mut w = [0, 0, 0, 1];
            for ch in 0..self.channels {
                let v = self.value(x, y, ch);
                w[ch] = if self.signed && self.bits < 32 {
                    let s = 32 - self.bits;
                    (((v << s) as i32) >> s) as u32
                } else {
                    v
                };
            }
            w
        }
        /// The bytes the pixel engine stores for the pattern's pixel.
        fn store(&self, x: usize, y: usize) -> Vec<u8> {
            if self.bits == 10 {
                let v: Vec<u32> = (0..4).map(|ch| self.value(x, y, ch)).collect();
                return (v[0] | v[1] << 10 | v[2] << 20 | v[3] << 30).to_le_bytes().to_vec();
            }
            (0..self.channels)
                .flat_map(|ch| self.value(x, y, ch).to_le_bytes().into_iter().take(self.bits / 8))
                .collect()
        }
    }

    /* ---- a framebuffer on the host ---- */

    /// A framebuffer of renderbuffers, one per entry; the first of each is
    /// COLOR_ATTACHMENT0 on.
    struct Rig {
        fb: GLuint,
        rbs: Vec<GLuint>,
        /// The platform's memory of each attachment: its GPU address.
        mem: Vec<u64>,
    }

    /// The platform allocation backing a renderbuffer of `bytes`: the one of
    /// that size nobody has claimed yet, the newest.
    fn find_alloc(bytes: usize, taken: &[u64], same: usize) -> u64 {
        with_platform(|p| {
            /* the newest `same` allocations of that size, oldest first: the
             * attachments made in order took them in order */
            let mut of_size: Vec<u64> = p.allocs.iter().filter(|(_, a)| a.size == bytes).map(|(g, _)| *g).collect();
            assert!(of_size.len() >= same, "no {same} allocations of {bytes} bytes");
            let at = of_size.len() - same;
            of_size = of_size.split_off(at);
            of_size.into_iter().find(|g| !taken.contains(g)).expect("an unclaimed allocation")
        })
    }

    fn rig_with(formats: &[Fmt]) -> Rig {
        let mut r = Rig { fb: 0, rbs: Vec::new(), mem: Vec::new() };
        // SAFETY: plain GL calls with room for one name each.
        unsafe {
            glGenFramebuffers(1, &mut r.fb);
            glBindFramebuffer(GL_FRAMEBUFFER, r.fb);
            for (n, f) in formats.iter().enumerate() {
                let mut rb = 0;
                glGenRenderbuffers(1, &mut rb);
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, f.gl, W, H);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + n as u32, GL_RENDERBUFFER, rb);
                r.rbs.push(rb);
            }
            let log = seen(|s| s.log.clone());
            assert_eq!(glGetError(), GL_NO_ERROR, "{log:?}");
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{log:?}");
            let bufs: Vec<GLenum> = (0..formats.len() as u32).map(|n| GL_COLOR_ATTACHMENT0 + n).collect();
            glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        }
        for f in formats {
            /* the rows are the pair-padded pitch apart (W is even) */
            let bytes = (W * H) as usize * f.gl_bpp();
            let same = formats.iter().filter(|o| (W * H) as usize * o.gl_bpp() == bytes).count();
            let g = find_alloc(bytes, &r.mem, same);
            r.mem.push(g);
        }
        r
    }

    impl Fmt {
        fn gl_bpp(&self) -> usize {
            if self.float {
                self.channels * self.bits / 8
            } else {
                self.bytes()
            }
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            // SAFETY: plain GL calls on names this rig made.
            unsafe {
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                glDeleteFramebuffers(1, &self.fb);
                glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            }
        }
    }

    /// Write `bytes_of(x, y)` into pixel (x, y) of the attachment's memory
    /// (row y of the memory is window row H - 1 - y of a framebuffer object?
    /// No: an FBO is not flipped, row y is row y).
    fn poke(r: &Rig, n: usize, f: &Fmt, bytes_of: impl Fn(usize, usize) -> Vec<u8>) {
        let gpu = r.mem[n];
        with_platform(|p| {
            let a = p.allocs.get_mut(&gpu).expect("the allocation");
            let base = a.mem.as_mut_ptr() as *mut u8;
            let bpp = f.gl_bpp();
            for y in 0..H as usize {
                for x in 0..W as usize {
                    let b = bytes_of(x, y);
                    assert_eq!(b.len(), bpp);
                    // SAFETY: inside the allocation of W x H pixels.
                    unsafe {
                        core::ptr::copy_nonoverlapping(b.as_ptr(), base.add((y * W as usize + x) * bpp), bpp);
                    }
                }
            }
        });
    }

    /// glReadPixels of the whole attachment; the error, the words read.
    fn read_words(format: GLenum, ty: GLenum) -> (GLenum, Vec<u32>) {
        let mut out = vec![0xdead_beefu32; (W * H) as usize * 4];
        // SAFETY: room for W x H pixels of four 32-bit words.
        unsafe {
            while glGetError() != GL_NO_ERROR {}
            glReadPixels(0, 0, W, H, format, ty, out.as_mut_ptr() as *mut c_void);
            (glGetError(), out)
        }
    }

    /// The error of a read of the given pair into a buffer big enough for any.
    fn read_error(format: GLenum, ty: GLenum) -> GLenum {
        read_words(format, ty).0
    }

    const FORMATS: [(GLenum, &str); 13] = [
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
    ];

    const TYPES: [(GLenum, &str); 17] = [
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
        (0x1234, "unknown 0x1234"),
    ];

    /// ES 3.0 4.3.2's accepted pairs for an integer read buffer: only
    /// RGBA_INTEGER with INT (signed) or UNSIGNED_INT (unsigned) -- and for
    /// RGB10_A2UI also UNSIGNED_INT_2_10_10_10_REV, which the spec lists as
    /// the packed read of that format (not asserted either way: see the
    /// `rgb10_a2ui_packed_read` test).
    fn int_pair_ok(f: &Fmt, format: GLenum, ty: GLenum) -> bool {
        format == GL_RGBA_INTEGER && ty == if f.signed { GL_INT } else { GL_UNSIGNED_INT }
    }

    /// The spec's error for a pair on a read buffer: INVALID_ENUM for a name
    /// ES 3.0 has no such parameter value for, INVALID_OPERATION for a
    /// combination the buffer does not take.
    fn spec_error(_format: GLenum, ty: GLenum, ok: bool) -> GLenum {
        if ok {
            GL_NO_ERROR
        } else if ty == 0x1234 {
            GL_INVALID_ENUM
        } else {
            GL_INVALID_OPERATION
        }
    }

    /* ---- the tests ---- */

    /// Every integer format reads back through its one pair as the words it
    /// stores, 0,0,0,1 for the channels it lacks, signed ones sign-extended;
    /// a window of the buffer lands at its place and the pixels outside the
    /// buffer are left alone.
    #[test]
    fn integer_formats_read_back_their_stored_words() {
        let _g = rig();
        for f in INTS {
            let r = rig_with(&[f]);
            poke(&r, 0, &f, |x, y| f.store(x, y));
            let ty = if f.signed { GL_INT } else { GL_UNSIGNED_INT };
            let (e, px) = read_words(GL_RGBA_INTEGER, ty);
            assert_eq!(e, GL_NO_ERROR, "{}", f.name);
            for y in 0..H as usize {
                for x in 0..W as usize {
                    let at = (y * W as usize + x) * 4;
                    assert_eq!(px[at..at + 4], f.expect(x, y), "{} ({x}, {y})", f.name);
                }
            }
            /* a 3 x 2 window at (2, 1) */
            let mut win = vec![0xdead_beefu32; 3 * 2 * 4];
            // SAFETY: room for 3 x 2 pixels of four words.
            unsafe { glReadPixels(2, 1, 3, 2, GL_RGBA_INTEGER, ty, win.as_mut_ptr() as *mut c_void) };
            assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "{}: a window", f.name);
            for y in 0..2 {
                for x in 0..3 {
                    let at = (y * 3 + x) * 4;
                    assert_eq!(win[at..at + 4], f.expect(x + 2, y + 1), "{} window ({x}, {y})", f.name);
                }
            }
            /* a window hanging over the right edge: the pixels outside stay */
            let mut over = vec![0xdead_beefu32; 4 * 4];
            // SAFETY: room for 4 x 1 pixels of four words.
            unsafe { glReadPixels(W - 2, 0, 4, 1, GL_RGBA_INTEGER, ty, over.as_mut_ptr() as *mut c_void) };
            assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "{}: over the edge", f.name);
            assert_eq!(over[0..4], f.expect(W as usize - 2, 0), "{}", f.name);
            assert_eq!(over[4..8], f.expect(W as usize - 1, 0), "{}", f.name);
            assert!(over[8..].iter().all(|w| *w == 0xdead_beef), "{}: outside the buffer is untouched", f.name);
        }
    }

    /// Every pair but the one is GL_INVALID_OPERATION on an integer buffer
    /// (or GL_INVALID_ENUM for a name that is none), and a refused read
    /// writes nothing.
    #[test]
    fn integer_formats_refuse_every_other_pair() {
        let _g = rig();
        let mut wrong = Vec::new();
        for f in INTS {
            let _r = rig_with(&[f]);
            for (format, fname) in FORMATS {
                for (ty, tname) in TYPES {
                    let ok = int_pair_ok(&f, format, ty);
                    let (e, px) = read_words(format, ty);
                    let want = spec_error(format, ty, ok);
                    /* the RGB10_A2UI packed read is the one pair the spec
                     * leaves to the implementation here */
                    if f.bits == 10 && format == GL_RGBA_INTEGER && ty == GL_UNSIGNED_INT_2_10_10_10_REV {
                        continue;
                    }
                    if e != want && !(want == GL_INVALID_ENUM && e == GL_INVALID_OPERATION && ty != 0x1234) {
                        wrong.push(format!("{} {fname}/{tname}: error {e:#x}, spec {want:#x}", f.name));
                    }
                    if e != GL_NO_ERROR {
                        assert!(px.iter().all(|w| *w == 0xdead_beef), "{} {fname}/{tname}: written", f.name);
                    }
                }
            }
        }
        assert!(wrong.is_empty(), "{} pairs differ from the spec:\n{}", wrong.len(), wrong.join("\n"));
    }

    /// Float buffers read RGBA with FLOAT as the stored values (R16F, R32F
    /// and RGBA16F, the channels they lack 0,0,0,1) and RGBA with
    /// UNSIGNED_BYTE clamped and rounded; RGBA_INTEGER is refused.
    #[test]
    fn float_formats_read_back_floats_and_bytes() {
        let _g = rig();
        let vals = |x: usize, y: usize, ch: usize| -> f32 {
            /* exact in a half: multiples of 1/16 up to 4, some negative */
            let v = ((x * 3 + y * 5 + ch * 2) % 64) as f32 / 16.0;
            if (x + ch) % 3 == 0 {
                -v
            } else {
                v
            }
        };
        let half = |v: f32| -> [u8; 2] {
            /* 1/16 steps below 4 are exact in a half: build the bits */
            let s = if v < 0.0 { 0x8000u16 } else { 0 };
            let a = v.abs();
            let h = if a == 0.0 {
                0
            } else {
                let e = a.log2().floor() as i32;
                let m = ((a / 2f32.powi(e) - 1.0) * 1024.0).round() as u16;
                (((e + 15) as u16) << 10) | m
            };
            (s | h).to_le_bytes()
        };
        for f in FLOATS {
            let r = rig_with(&[f]);
            poke(&r, 0, &f, |x, y| match f.gl {
                GL_R16F => half(vals(x, y, 0)).to_vec(),
                GL_R32F => vals(x, y, 0).to_le_bytes().to_vec(),
                _ => (0..4).flat_map(|ch| half(vals(x, y, ch))).collect(),
            });
            let (e, px) = read_words(GL_RGBA, GL_FLOAT);
            assert_eq!(e, GL_NO_ERROR, "{}", f.name);
            for y in 0..H as usize {
                for x in 0..W as usize {
                    let at = (y * W as usize + x) * 4;
                    let mut want = [0.0, 0.0, 0.0, 1.0];
                    for (ch, w) in want.iter_mut().enumerate().take(f.channels) {
                        *w = vals(x, y, ch);
                    }
                    let got: Vec<f32> = px[at..at + 4].iter().map(|w| f32::from_bits(*w)).collect();
                    assert_eq!(got, want, "{} ({x}, {y})", f.name);
                }
            }
            /* RGBA/UNSIGNED_BYTE: four bytes a pixel, clamped to [0, 1] */
            let mut b = vec![0u8; (W * H) as usize * 4];
            // SAFETY: room for W x H RGBA8 pixels.
            unsafe { glReadPixels(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE, b.as_mut_ptr() as *mut c_void) };
            assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "{}: RGBA/UNSIGNED_BYTE", f.name);
            for y in 0..H as usize {
                for x in 0..W as usize {
                    let at = (y * W as usize + x) * 4;
                    let mut want = [0.0f32, 0.0, 0.0, 1.0];
                    for (ch, w) in want.iter_mut().enumerate().take(f.channels) {
                        *w = vals(x, y, ch);
                    }
                    let want: Vec<u8> = want.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect();
                    assert_eq!(b[at..at + 4], want[..], "{} ({x}, {y}) bytes", f.name);
                }
            }
        }
    }

    /// A float buffer refuses the integer read and the types it cannot hold.
    #[test]
    fn float_formats_refuse_the_other_pairs() {
        let _g = rig();
        let mut wrong = Vec::new();
        for f in FLOATS {
            let _r = rig_with(&[f]);
            for (format, fname) in FORMATS {
                for (ty, tname) in TYPES {
                    /* the spec's pairs for a floating-point buffer: RGBA with
                     * FLOAT; RGBA with UNSIGNED_BYTE is the core's own
                     * addition (every buffer takes it), tolerated */
                    let ok = format == GL_RGBA && matches!(ty, GL_FLOAT | GL_UNSIGNED_BYTE);
                    let e = read_error(format, ty);
                    let want = spec_error(format, ty, ok);
                    if e != want && !(want == GL_INVALID_ENUM && e == GL_INVALID_OPERATION && ty != 0x1234) {
                        wrong.push(format!("{} {fname}/{tname}: error {e:#x}, spec {want:#x}", f.name));
                    }
                }
            }
        }
        assert!(wrong.is_empty(), "{} pairs differ from the spec:\n{}", wrong.len(), wrong.join("\n"));
    }

    /// GL_IMPLEMENTATION_COLOR_READ_FORMAT/TYPE of every integer and float
    /// buffer is a pair glReadPixels takes on it without an error: the
    /// integer pair of its class for an integer buffer, and for a float one
    /// RGBA with UNSIGNED_BYTE.
    #[test]
    fn the_implementation_pair_is_accepted() {
        let _g = rig();
        for f in INTS.iter().chain(FLOATS.iter()) {
            let _r = rig_with(&[*f]);
            let (mut fmt, mut ty) = (0, 0);
            // SAFETY: GLint out-parameters.
            unsafe {
                glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut fmt);
                glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut ty);
                assert_eq!(glGetError(), GL_NO_ERROR, "{}", f.name);
            }
            if f.float {
                assert_eq!((fmt as GLenum, ty as GLenum), (GL_RGBA, GL_UNSIGNED_BYTE), "{}", f.name);
            } else {
                let t = if f.signed { GL_INT } else { GL_UNSIGNED_INT };
                assert_eq!((fmt as GLenum, ty as GLenum), (GL_RGBA_INTEGER, t), "{}", f.name);
            }
            assert_eq!(read_error(fmt as GLenum, ty as GLenum), GL_NO_ERROR, "{}: the pair it names", f.name);
        }
    }

    /// With several colour attachments of both classes, the pair and the
    /// values follow glReadBuffer, and each read buffer refuses the other
    /// classes' pairs.
    #[test]
    fn the_read_buffer_picks_the_class() {
        let _g = rig();
        let fs = [INTS[13], FLOATS[2], INTS[4]]; /* RGBA8UI, RGBA16F, R32I: GL_MAX_COLOR_ATTACHMENTS is 3 */
        let r = rig_with(&fs);
        for (n, f) in fs.iter().enumerate() {
            if !f.float {
                poke(&r, n, f, |x, y| f.store(x, y));
            }
        }
        for (n, f) in fs.iter().enumerate() {
            // SAFETY: plain GL calls.
            unsafe { glReadBuffer(GL_COLOR_ATTACHMENT0 + n as u32) };
            assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "{}", f.name);
            if f.float {
                assert_eq!(read_error(GL_RGBA, GL_FLOAT), GL_NO_ERROR, "{}", f.name);
                assert_eq!(read_error(GL_RGBA_INTEGER, GL_INT), GL_INVALID_OPERATION, "{}", f.name);
                assert_eq!(read_error(GL_RGBA_INTEGER, GL_UNSIGNED_INT), GL_INVALID_OPERATION, "{}", f.name);
                continue;
            }
            let (ok, other) = if f.signed { (GL_INT, GL_UNSIGNED_INT) } else { (GL_UNSIGNED_INT, GL_INT) };
            let (e, px) = read_words(GL_RGBA_INTEGER, ok);
            assert_eq!(e, GL_NO_ERROR, "{}", f.name);
            assert_eq!(px[0..4], f.expect(0, 0), "{} (0, 0)", f.name);
            let at = (3 * W as usize + 4) * 4;
            assert_eq!(px[at..at + 4], f.expect(4, 3), "{} (4, 3)", f.name);
            assert_eq!(read_error(GL_RGBA_INTEGER, other), GL_INVALID_OPERATION, "{}: the other signedness", f.name);
            assert_eq!(read_error(GL_RGBA, GL_UNSIGNED_BYTE), GL_INVALID_OPERATION, "{}: a fixed-point read", f.name);
            assert_eq!(read_error(GL_RGBA, GL_FLOAT), GL_INVALID_OPERATION, "{}: a float read", f.name);
        }
        // SAFETY: plain GL calls.
        unsafe {
            glReadBuffer(GL_NONE);
        }
        assert_eq!(read_error(GL_RGBA_INTEGER, GL_UNSIGNED_INT), GL_INVALID_OPERATION, "GL_NONE reads nothing");
    }

    /// glClearBuffer{i,ui,f}v of each attachment, then a read: the clear
    /// reaches the job (the host draws nothing, so the memory is as the
    /// allocation left it, zero) and the read hands the zero words with
    /// 0,0,0,1 for the missing channels, the error state clean.
    #[test]
    fn a_cleared_attachment_reads_with_its_default_channels() {
        let _g = rig();
        for f in INTS.iter().chain(FLOATS.iter()) {
            let _r = rig_with(&[*f]);
            let words = [7u32, 8, 9, 10];
            // SAFETY: four clear values.
            unsafe {
                if f.float {
                    let v = [0.5f32, 0.25, 0.125, 1.0];
                    glClearBufferfv(GL_COLOR, 0, v.as_ptr());
                } else if f.signed {
                    glClearBufferiv(GL_COLOR, 0, words.as_ptr() as *const GLint);
                } else {
                    glClearBufferuiv(GL_COLOR, 0, words.as_ptr());
                }
            }
            let ty = if f.float {
                GL_FLOAT
            } else if f.signed {
                GL_INT
            } else {
                GL_UNSIGNED_INT
            };
            let (e, px) = read_words(if f.float { GL_RGBA } else { GL_RGBA_INTEGER }, ty);
            assert_eq!(e, GL_NO_ERROR, "{}: {:?}", f.name, seen(|s| s.log.clone()));
            assert!(seen(|s| s.clears.len()) >= 1, "{}: a job with the clear", f.name);
            /* the missing channels: G,B 0 and A 1 (a float's as 0.0 and 1.0) */
            let one = if f.float { 1.0f32.to_bits() } else { 1 };
            let want = [0, 0, 0, if f.channels == 4 { 0 } else { one }];
            assert_eq!(px[0..4], want, "{}: memory the host did not draw, with the format's default channels", f.name);
        }
    }

    /// GL_INVALID_OPERATION for a pair is raised before anything is read,
    /// and the other errors of the call keep their place: a negative size is
    /// GL_INVALID_VALUE, an incomplete framebuffer
    /// GL_INVALID_FRAMEBUFFER_OPERATION.
    #[test]
    fn the_other_errors_of_the_call() {
        let _g = rig();
        let f = INTS[13];
        let r = rig_with(&[f]);
        let mut px = [0u32; 4];
        // SAFETY: 1 x 1 pixel of four words; negative sizes read nothing.
        unsafe {
            glReadPixels(0, 0, -1, 1, GL_RGBA_INTEGER, GL_UNSIGNED_INT, px.as_mut_ptr() as *mut c_void);
            assert_eq!(glGetError(), GL_INVALID_VALUE);
            glReadPixels(0, 0, 1, -1, GL_RGBA_INTEGER, GL_UNSIGNED_INT, px.as_mut_ptr() as *mut c_void);
            assert_eq!(glGetError(), GL_INVALID_VALUE);
            /* a wrong pair and a negative size: the spec lists both, any may win */
            glReadPixels(0, 0, 1, 1, GL_RGBA_INTEGER, GL_INT, px.as_mut_ptr() as *mut c_void);
            assert_eq!(glGetError(), GL_INVALID_OPERATION);
            /* no attachment: incomplete */
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, 0);
            assert_ne!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
            assert_eq!(glGetError(), GL_INVALID_FRAMEBUFFER_OPERATION);
        }
        drop(r);
    }

    /// ES 3.0 4.3.2: reading an incomplete framebuffer is
    /// GL_INVALID_FRAMEBUFFER_OPERATION whatever the pair. Today the GL core
    /// checks the pair first against the format of the (absent) read buffer,
    /// which it takes for a fixed-point one, so an integer pair on a
    /// framebuffer with no attachment is GL_INVALID_OPERATION.
    /// Cause: `read_pixels` in src/gl/framebuffer.rs validates (format, type)
    /// before `target_loc`'s completeness answer.
    #[test]
    #[ignore = "GL core: read_pixels checks the pair before the framebuffer's completeness (src/gl/framebuffer.rs)"]
    fn an_incomplete_framebuffer_wins_over_the_pair() {
        let _g = rig();
        let _r = rig_with(&[INTS[13]]);
        let mut px = [0u32; 4];
        // SAFETY: 1 x 1 pixel of four words.
        unsafe {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, 0);
            glReadPixels(0, 0, 1, 1, GL_RGBA_INTEGER, GL_UNSIGNED_INT, px.as_mut_ptr() as *mut c_void);
            assert_eq!(glGetError(), GL_INVALID_FRAMEBUFFER_OPERATION);
        }
    }

    /// The float names the GL core does not make colour-renderable (RG32F,
    /// RGB32F, RGBA32F, R11F_G11F_B10F) leave the framebuffer incomplete, so
    /// there is nothing to read back from them. ES 3.0 itself has no float
    /// colour-renderable format; GL_EXT_color_buffer_float adds these, and
    /// the dEQP cases that use them are not-pass for this reason.
    #[test]
    fn the_other_float_formats_are_not_attachable_today() {
        let _g = rig();
        let names = [
            (GL_RG32F, "RG32F"),
            (GL_RGB32F, "RGB32F"),
            (GL_RGBA32F, "RGBA32F"),
            (GL_R11F_G11F_B10F, "R11F_G11F_B10F"),
        ];
        for (gl, name) in names {
            let (mut rb, mut fb) = (0, 0);
            // SAFETY: plain GL calls with room for one name each.
            unsafe {
                glGenRenderbuffers(1, &mut rb);
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, gl, W, H);
                let storage = glGetError();
                glGenFramebuffers(1, &mut fb);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
                let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
                assert!(
                    storage != GL_NO_ERROR || status != GL_FRAMEBUFFER_COMPLETE,
                    "{name}: attachable now -- this test pins the refusal, write its readback"
                );
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                glDeleteFramebuffers(1, &fb);
                glDeleteRenderbuffers(1, &rb);
                while glGetError() != GL_NO_ERROR {}
            }
        }
    }

    /// RG16F and RGB16F are held in RGBA16F storage (four halves a pixel).
    /// What the read hands out for the channels the format lacks: the storage
    /// has 0.25 in blue and 0.5 in alpha here.
    fn read_rg16f(gl: GLenum) -> Vec<f32> {
        let f = Fmt { gl, name: "", channels: 4, bits: 16, signed: true, float: true };
        let r = rig_with(&[f]);
        /* halves: 1.0 = 0x3c00, 0.25 = 0x3400, 0.5 = 0x3800, 2.0 = 0x4000 */
        poke(&r, 0, &f, |_, _| [0x3c00u16, 0x4000, 0x3400, 0x3800].iter().flat_map(|h| h.to_le_bytes()).collect());
        let (e, px) = read_words(GL_RGBA, GL_FLOAT);
        assert_eq!(e, GL_NO_ERROR);
        px[0..4].iter().map(|w| f32::from_bits(*w)).collect()
    }

    /// ES 3.0 4.3.2: the channels a format lacks read as 0, 0, 1. RG16F lacks
    /// blue and alpha, RGB16F alpha.
    /// Cause: `read_float_row` (src/gl/framebuffer.rs) hands out the four
    /// halves of the RGBA16F storage whatever the attachment's format.
    #[test]
    #[ignore = "GL core: RG16F/RGB16F readback returns the storage's blue/alpha, not 0/1 (read_float_row)"]
    fn rg16f_and_rgb16f_read_the_missing_channels_as_0_and_1() {
        let _g = rig();
        assert_eq!(read_rg16f(GL_RG16F), [1.0, 2.0, 0.0, 1.0]);
        assert_eq!(read_rg16f(GL_RGB16F), [1.0, 2.0, 0.25, 1.0]);
    }

    /// Today's answer: the storage's four halves.
    #[test]
    fn rg16f_and_rgb16f_read_the_storage_today() {
        let _g = rig();
        assert_eq!(read_rg16f(GL_RG16F), [1.0, 2.0, 0.25, 0.5]);
        assert_eq!(read_rg16f(GL_RGB16F), [1.0, 2.0, 0.25, 0.5]);
    }

    /// Today's answer for the same read, pinned.
    #[test]
    fn an_incomplete_framebuffer_with_an_integer_pair_is_invalid_operation_today() {
        let _g = rig();
        let _r = rig_with(&[INTS[13]]);
        let mut px = [0u32; 4];
        // SAFETY: 1 x 1 pixel of four words.
        unsafe {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, 0);
            glReadPixels(0, 0, 1, 1, GL_RGBA_INTEGER, GL_UNSIGNED_INT, px.as_mut_ptr() as *mut c_void);
            assert_eq!(glGetError(), GL_INVALID_OPERATION);
        }
    }

    /// GL_PACK_ROW_LENGTH and GL_PACK_SKIP_* place 16-byte pixels, whatever
    /// GL_PACK_ALIGNMENT says, for an integer read.
    #[test]
    fn pack_state_places_integer_rows() {
        let _g = rig();
        let f = INTS[13];
        let r = rig_with(&[f]);
        poke(&r, 0, &f, |x, y| f.store(x, y));
        let mut out = vec![0xdead_beefu32; 8 * 4 * 4];
        // SAFETY: 8 x 4 pixels of four words are room for the 3 x 2 read at
        // row length 8 with 1 pixel and 1 row skipped.
        unsafe {
            glPixelStorei(GL_PACK_ROW_LENGTH, 8);
            glPixelStorei(GL_PACK_SKIP_PIXELS, 1);
            glPixelStorei(GL_PACK_SKIP_ROWS, 1);
            glPixelStorei(GL_PACK_ALIGNMENT, 8);
            glReadPixels(1, 1, 3, 2, GL_RGBA_INTEGER, GL_UNSIGNED_INT, out.as_mut_ptr() as *mut c_void);
            let e = glGetError();
            glPixelStorei(GL_PACK_ROW_LENGTH, 0);
            glPixelStorei(GL_PACK_SKIP_PIXELS, 0);
            glPixelStorei(GL_PACK_SKIP_ROWS, 0);
            glPixelStorei(GL_PACK_ALIGNMENT, 4);
            assert_eq!(e, GL_NO_ERROR);
        }
        for y in 0..2 {
            for x in 0..3 {
                let at = ((1 + y) * 8 + 1 + x) * 4;
                assert_eq!(out[at..at + 4], f.expect(1 + x, 1 + y), "({x}, {y})");
            }
        }
        assert_eq!(out[0..4], [0xdead_beef; 4], "before the skipped pixels");
        assert_eq!(out[(1 * 8 + 4) * 4], 0xdead_beef, "past the row's width");
    }
}
