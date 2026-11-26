// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What a host test of the V3D 7.1 table (`yakogl::v3d::table71`,
//! `yakogl_backend_v3d71`) stands on: a mock of the OSAL the table calls, and
//! the C job builder its FFI reaches. Used by tests/v3d71_table.rs and
//! tests/v3d71_table_draw.rs (`mod v3d71_table_rig;`), built with
//! `--features host-test,gpu-v3d71,shaderc-online,no-shader-table`.
//!
//! # The OSAL
//!
//! [`rig`] installs an `OsalOps` over one block of host memory that stands
//! for the GPU carve-out: `gpu_alloc` hands out pieces of it at 32-bit GPU
//! addresses from [`GPU_BASE`] (never reused, so a test can read what a job
//! named after the table freed it), `query` answers the VPM size, the hub
//! identity ([`set_ident`]) and the draw budget ([`set_draws_per_job`]),
//! `submit` records the `V3dKick` with a copy of the whole memory as the job
//! found it ([`kicks`]), `wait` completes the fence it is given and
//! `completed` reports the last one. Every log line is kept ([`log_lines`]).
//!
//! # The C builder
//!
//! The library calls `v3d_job71_begin`, `v3d_job71_draw`, `v3d_job71_end`
//! and `v3d_job71_alloc_sizes` (src/v3d/ffi71.rs) and `cargo test` links no
//! C. The four symbols are defined here and forward to
//! driver/src/v3d/v3d_job71.c over v3d_frame.c at `-DV3D_CL_VER=71`, the
//! files the Pi 5 image builds, compiled into a shared library when the
//! first job is built (the flags of tests/v3d71_backend71_shapes.rs's
//! harness) and loaded with `dlopen`. So a job the table submits here holds
//! the bytes the product builder writes. A test that builds no job needs no
//! C compiler.
#![allow(dead_code)]

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};

use yakogl::backend;
use yakogl::osal::{
    osal_set_ops, OsalGpuMem, OsalOps, OSAL_QUERY_V3D_DRAWS_PER_JOB, OSAL_QUERY_V3D_HUB_IDENT, OSAL_QUERY_V3D_JOB_OPEN,
    OSAL_QUERY_V3D_VPM_BYTES,
};
use yakogl::util::{EINVAL, ENOMEM, ENOTSUP};
use yakogl::v3d::ffi71::{V3dCl, V3dJob71, V3dJob71Clear, V3dJob71Draw, V3dJob71DrawLayout, V3dJob71Target};
use yakogl::v3d::kick::{V3dKick, V3D_KICK_MAGIC};
use yakogl::v3d::table71::yakogl_backend_v3d71;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

/// The first GPU address of the mock carve-out.
pub const GPU_BASE: u32 = 0x1000_0000;
/// Its size.
pub const GPU_BYTES: usize = 8 * 1024 * 1024;
/// The board's HUB_IDENT1 and HUB_IDENT3 (tools/v3d/pi5/README.md).
pub const PI5_IDENT: [u32; 2] = [0x0008_1117, 0x0002_0a10];

/// One submitted job: the descriptor and the memory as the submit found it.
#[derive(Clone)]
pub struct Kick {
    pub kick: V3dKick,
    pub fence: i64,
    mem: Vec<u8>,
}

impl Kick {
    /// `len` bytes at GPU address `gpu`, as the job would read them.
    pub fn at(&self, gpu: u32, len: usize) -> &[u8] {
        let off = (gpu - GPU_BASE) as usize;
        &self.mem[off..off + len]
    }
    pub fn bcl(&self) -> &[u8] {
        self.at(self.kick.bcl_start, (self.kick.bcl_end - self.kick.bcl_start) as usize)
    }
    pub fn rcl(&self) -> &[u8] {
        self.at(self.kick.rcl_start, (self.kick.rcl_end - self.kick.rcl_start) as usize)
    }
}

struct World {
    mem: Vec<u8>,
    next: usize,
    /// Every allocation: (GPU address, bytes, freed).
    allocs: Vec<(u32, usize, bool)>,
    kicks: Vec<Kick>,
    fence: i64,
    completed: i64,
    waits: u32,
    ident: Option<[u32; 2]>,
    vpm: u32,
    draws_per_job: u32,
    job_open: Vec<bool>,
    log: Vec<String>,
}

impl World {
    fn new() -> World {
        World {
            mem: vec![0; GPU_BYTES],
            next: 0,
            allocs: vec![],
            kicks: vec![],
            fence: 0,
            completed: 0,
            waits: 0,
            ident: Some(PI5_IDENT),
            vpm: 65536,
            draws_per_job: 0,
            job_open: vec![],
            log: vec![],
        }
    }
}

static WORLD: Mutex<Option<World>> = Mutex::new(None);
static LOCK: Mutex<()> = Mutex::new(());

fn world<R>(f: impl FnOnce(&mut World) -> R) -> R {
    let mut w = WORLD.lock().unwrap_or_else(|e| e.into_inner());
    f(w.get_or_insert_with(World::new))
}

unsafe extern "C" fn p_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    world(|w| {
        let at = w.next.next_multiple_of(4096);
        if size == 0 || at + size > w.mem.len() {
            return -ENOMEM;
        }
        w.next = at + size;
        let gpu = GPU_BASE + at as u32;
        w.allocs.push((gpu, size, false));
        // SAFETY: `out` is the library's live out-parameter; the block is
        // inside `mem`, which is never reallocated while the world lives.
        unsafe {
            *out = OsalGpuMem {
                cpu: w.mem.as_mut_ptr().add(at) as *mut c_void,
                gpu: u64::from(gpu),
                phys: u64::from(gpu),
                size,
                impl_: core::ptr::null_mut(),
            };
        }
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the library passes a live allocation record.
    let gpu = unsafe { (*mem).gpu } as u32;
    world(|w| match w.allocs.iter_mut().find(|a| a.0 == gpu && !a.2) {
        Some(a) => {
            a.2 = true;
            0
        }
        None => -EINVAL,
    })
}

unsafe extern "C" fn p_cache_flush(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn p_submit(desc: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<V3dKick>() {
        return i64::from(-EINVAL);
    }
    // SAFETY: the library passes a live V3dKick of the size checked.
    let kick = unsafe { *(desc as *const V3dKick) };
    if kick.magic != V3D_KICK_MAGIC {
        return i64::from(-EINVAL);
    }
    world(|w| {
        w.fence += 1;
        let k = Kick { kick, fence: w.fence, mem: w.mem.clone() };
        w.kicks.push(k);
        w.fence
    })
}

unsafe extern "C" fn p_wait(fence: i64, _timeout_us: i64) -> i32 {
    world(|w| {
        w.waits += 1;
        w.completed = w.completed.max(fence);
        0
    })
}

unsafe extern "C" fn p_completed() -> i64 {
    world(|w| w.completed)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    world(|w| {
        // SAFETY: the library passes an out-parameter of `out_sz` bytes.
        unsafe {
            match (what, out_sz) {
                (OSAL_QUERY_V3D_VPM_BYTES, 4) => *(out as *mut u32) = w.vpm,
                (OSAL_QUERY_V3D_HUB_IDENT, 8) => match w.ident {
                    Some(id) => *(out as *mut [u32; 2]) = id,
                    None => return -ENOTSUP,
                },
                (OSAL_QUERY_V3D_DRAWS_PER_JOB, 4) if w.draws_per_job != 0 => *(out as *mut u32) = w.draws_per_job,
                (OSAL_QUERY_V3D_JOB_OPEN, 4) => w.job_open.push(*(out as *const u32) != 0),
                _ => return -ENOTSUP,
            }
        }
        0
    })
}

/// The library logs every line as `log("%s", line)` (`osal::log_str`).
unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
    // SAFETY: the library passes a NUL-terminated line.
    let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
    world(|w| w.log.push(s));
}

type LogFn = unsafe extern "C" fn(*const c_char, ...);

fn osal() -> &'static OsalOps {
    static OPS: OnceLock<OsalOps> = OnceLock::new();
    OPS.get_or_init(|| {
        // SAFETY: see p_log2: the library only ever calls it with ("%s", line).
        let log = unsafe { core::mem::transmute::<unsafe extern "C" fn(*const c_char, *const c_char), LogFn>(p_log2) };
        OsalOps {
            init: None,
            shutdown: None,
            gpu_alloc: Some(p_gpu_alloc),
            gpu_free: Some(p_gpu_free),
            mmu_map: None,
            map_phys: None,
            gpu_write: None,
            gpu_read: None,
            cache_flush: Some(p_cache_flush),
            cache_invd: None,
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
        }
    })
}

/// A test's turn, ended (the table shut down) when the guard drops.
pub struct Rig(MutexGuard<'static, ()>);

impl Drop for Rig {
    fn drop(&mut self) {
        backend::shutdown();
    }
}

/// The tests take turns: one OSAL, one `g_backend`, a fresh carve-out, a
/// fresh log epoch (so a once-per-epoch line is logged again) and an empty
/// log each. The table is not initialised: the test calls `backend::init`.
pub fn rig() -> Rig {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    *WORLD.lock().unwrap_or_else(|e| e.into_inner()) = Some(World::new());
    // SAFETY: statics; the lock serialises every test of the binary.
    unsafe {
        osal_set_ops(osal());
        backend::g_backend = &yakogl_backend_v3d71;
    }
    yakogl::osal::new_log_epoch();
    Rig(g)
}

/// The hub identity the OSAL answers; `None`: an OSAL without the query.
pub fn set_ident(ident: Option<[u32; 2]>) {
    world(|w| w.ident = ident);
}

/// The VPM size the OSAL answers.
pub fn set_vpm(bytes: u32) {
    world(|w| w.vpm = bytes);
}

/// The draw budget the OSAL answers; 0: no answer (the table's default).
pub fn set_draws_per_job(n: u32) {
    world(|w| w.draws_per_job = n);
}

pub fn log_lines() -> Vec<String> {
    world(|w| w.log.clone())
}

/// The log lines that contain `needle`.
pub fn logs_with(needle: &str) -> Vec<String> {
    world(|w| w.log.iter().filter(|l| l.contains(needle)).cloned().collect())
}

/// The jobs submitted so far.
pub fn kicks() -> Vec<Kick> {
    world(|w| w.kicks.clone())
}

/// (submits, waits) the OSAL saw.
pub fn osal_counts() -> (usize, u32) {
    world(|w| (w.kicks.len(), w.waits))
}

/// The `OSAL_QUERY_V3D_JOB_OPEN` values the table gave, in order.
pub fn job_open_calls() -> Vec<bool> {
    world(|w| w.job_open.clone())
}

/// The allocations still held: (GPU address, bytes).
pub fn live_allocs() -> Vec<(u32, usize)> {
    world(|w| w.allocs.iter().filter(|a| !a.2).map(|a| (a.0, a.1)).collect())
}

/// GPU memory of the test's own (a surface, a vertex buffer): its address.
pub fn gpu_block(bytes: usize) -> u32 {
    let mut m = OsalGpuMem::ZERO;
    // SAFETY: a live out-parameter.
    assert_eq!(unsafe { p_gpu_alloc(0, bytes, 0, &mut m) }, 0, "the mock carve-out is full");
    m.gpu as u32
}

/// Write `bytes` at GPU address `gpu`.
pub fn gpu_write(gpu: u32, bytes: &[u8]) {
    world(|w| {
        let off = (gpu - GPU_BASE) as usize;
        w.mem[off..off + bytes.len()].copy_from_slice(bytes);
    });
}

/* ---- the C builder ------------------------------------------------------------ */

#[link(name = "dl")]
extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

const RTLD_NOW: c_int = 2;

type BeginFn = unsafe extern "C" fn(*mut V3dJob71, *const V3dJob71Target, *mut V3dCl, *mut V3dCl, *mut V3dCl) -> bool;
type DrawFn = unsafe extern "C" fn(*mut V3dJob71, *const V3dJob71Draw, *mut V3dJob71DrawLayout) -> i32;
type EndFn = unsafe extern "C" fn(*mut V3dJob71, *const V3dJob71Clear, *mut u32, *mut u32) -> bool;
type SizesFn = unsafe extern "C" fn(*const V3dJob71, u32, *mut u32, *mut u32);

struct CJob {
    begin: BeginFn,
    draw: DrawFn,
    end: EndFn,
    sizes: SizesFn,
}

fn find_cc() -> String {
    for c in [std::env::var("CC").unwrap_or_default(), "cc".to_string()] {
        let Some(prog) = c.split_whitespace().next() else { continue };
        if Command::new(prog).arg("--version").output().is_ok() {
            return c;
        }
    }
    panic!("no C compiler ($CC or cc): this test runs driver/src/v3d/v3d_job71.c");
}

fn cjob() -> &'static CJob {
    static LIB: OnceLock<CJob> = OnceLock::new();
    LIB.get_or_init(|| {
        let cc = find_cc();
        // One directory per process: the gate may run this binary in several
        // rows at once, all with the same CARGO_TARGET_TMPDIR.
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("v3d71_table_rig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let so = dir.join("libv3d_job71.so");
        let mesa = format!("{ROOT}/third_party/mesa-26.2.1");
        let mut parts = cc.split_whitespace();
        let out = Command::new(parts.next().unwrap())
            .args(parts)
            .args(["-shared", "-fPIC", "-O1", "-std=c11", "-Wall", "-Wextra", "-Werror"])
            .args(["-D_GNU_SOURCE", "-DHAVE_ENDIAN_H", "-DV3D_CL_VER=71"])
            .args(["-isystem", &format!("{mesa}/src"), "-isystem", &format!("{mesa}/include")])
            .args(["-isystem", &format!("{mesa}/src/broadcom")])
            .arg(format!("-I{ROOT}/driver/src/v3d"))
            .arg(format!("-I{ROOT}/driver/src/v3d/generated"))
            .arg(format!("{ROOT}/driver/src/v3d/v3d_job71.c"))
            .arg(format!("{ROOT}/driver/src/v3d/v3d_frame.c"))
            /* the library's own calls stay inside it, and it needs nothing
             * the two files do not define */
            .args(["-Wl,-Bsymbolic", "-Wl,-z,defs", "-o"])
            .arg(&so)
            .arg("-lm")
            .output()
            .expect("run the C compiler");
        assert!(out.status.success(), "v3d_job71 build failed:\n{}", String::from_utf8_lossy(&out.stderr));
        let path = CString::new(so.to_str().unwrap()).unwrap();
        // SAFETY: plain libc calls with NUL-terminated strings; the handle is
        // never closed, so the function pointers stay valid.
        unsafe {
            let h = dlopen(path.as_ptr(), RTLD_NOW);
            assert!(!h.is_null(), "dlopen: {}", CStr::from_ptr(dlerror()).to_string_lossy());
            let sym = |name: &CStr| {
                let p = dlsym(h, name.as_ptr());
                assert!(!p.is_null(), "{name:?} is not in the built library");
                p
            };
            CJob {
                begin: core::mem::transmute::<*mut c_void, BeginFn>(sym(c"v3d_job71_begin")),
                draw: core::mem::transmute::<*mut c_void, DrawFn>(sym(c"v3d_job71_draw")),
                end: core::mem::transmute::<*mut c_void, EndFn>(sym(c"v3d_job71_end")),
                sizes: core::mem::transmute::<*mut c_void, SizesFn>(sym(c"v3d_job71_alloc_sizes")),
            }
        }
    })
}

/// # Safety
/// `v3d_job71_begin`'s contract (driver/src/v3d/v3d_job71.h).
#[no_mangle]
pub unsafe extern "C" fn v3d_job71_begin(
    j: *mut V3dJob71,
    t: *const V3dJob71Target,
    bcl: *mut V3dCl,
    rcl: *mut V3dCl,
    ind: *mut V3dCl,
) -> bool {
    // SAFETY: the caller's contract, passed on to the C function.
    unsafe { (cjob().begin)(j, t, bcl, rcl, ind) }
}

/// # Safety
/// `v3d_job71_draw`'s contract.
#[no_mangle]
pub unsafe extern "C" fn v3d_job71_draw(
    j: *mut V3dJob71,
    d: *const V3dJob71Draw,
    layout: *mut V3dJob71DrawLayout,
) -> i32 {
    // SAFETY: as above.
    unsafe { (cjob().draw)(j, d, layout) }
}

/// # Safety
/// `v3d_job71_end`'s contract.
#[no_mangle]
pub unsafe extern "C" fn v3d_job71_end(
    j: *mut V3dJob71,
    c: *const V3dJob71Clear,
    tile_list_start: *mut u32,
    tile_list_end: *mut u32,
) -> bool {
    // SAFETY: as above.
    unsafe { (cjob().end)(j, c, tile_list_start, tile_list_end) }
}

/// # Safety
/// `v3d_job71_alloc_sizes`'s contract.
#[no_mangle]
pub unsafe extern "C" fn v3d_job71_alloc_sizes(
    j: *const V3dJob71,
    page_size: u32,
    tile_alloc_size: *mut u32,
    tile_state_size: *mut u32,
) {
    // SAFETY: as above.
    unsafe { (cjob().sizes)(j, page_size, tile_alloc_size, tile_state_size) }
}
