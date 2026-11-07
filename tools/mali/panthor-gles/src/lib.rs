// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! libGLESv2.so.2 of YakoGL's Mali-G610 back end over panthor, for dEQP on
//! the NanoPC-T6's Armbian (README.md beside this crate).
//!
//! The code is the `yakogl` crate's: its `#[no_mangle]` GL and EGL entry
//! points are this shared object's exports, with the `gpu-mali` backend and
//! the in-process compiler, on the development OSAL of `yakogl-mali-panthor`
//! (the `OsalOps` table panthor-gltest runs on). libEGL.so.1 is a forwarder
//! build.sh links beside it, as CMakeLists.txt does for the C pair: EGL and
//! GL share one process-wide state, so they are one object.
//!
//! What this crate adds is the instrument the dEQP runner reads, on stderr:
//!
//! - `yakogl-mali: loaded build=<git describe> lib=<path of this .so>`, once,
//!   at the first `eglInitialize` (the OSAL's `init`): only this library's
//!   EGL reaches it, so the line proves the run used YakoGL's pair and not
//!   the board's glvnd + Mesa, which has the same two SONAMEs.
//! - `yakogl-mali-exit: draws=<n> refused=<n> first_refused=<name or -> gpu_faults=<n> job_timeouts=<n>`,
//!   at process exit ([`exit_line`] says what each count is).
//!
//! It gets them without touching the library: the load-time constructor
//! installs bitwise copies of three tables with a few entries wrapped (the
//! OSAL's `init`, `wait` and `log`, the backend's `draw`, the shader back
//! end's `compile`, `link` and `compile_vs_linked`) and points `g_osal`,
//! `g_backend` and `g_shaderc` at them. Every other entry is the original.

use core::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use yakogl::backend::{GlesDraw, YakoglBackend};
use yakogl::osal::OsalOps;
use yakogl::shaderc::{ShcAttribMap, ShcBackend, ShcShader};
use yakogl_mali_panthor::osal as panthor;

/// `git describe --always --dirty` of the tree this was built from.
pub const BUILD: &str = env!("YAKOGL_MALI_GLES_BUILD");

/// The prefixes of the two contract lines.
pub const LOADED: &str = "yakogl-mali: loaded";
pub const EXIT: &str = "yakogl-mali-exit:";

/// The log the wrapped compiles write into: far past the GL core's 256
/// bytes, so a refusal's name is whole (panthor-gltest's FULL_LOG).
const FULL_LOG: usize = 8192;

/// panthor's group state bits (panthor_drm.h; tools/mali/panthor/src/uapi.rs).
const GROUP_TIMEDOUT: u32 = 1 << 0;
const GROUP_FATAL_FAULT: u32 = 1 << 1;

/* ---- the counts ---------------------------------------------------------- */

/// Draws the backend recorded (`draw` returned 0 with vertices to draw).
static DRAWS: AtomicU64 = AtomicU64::new(0);
/// Shaders and programs the shader back end refused (`-ENOTSUP`, the
/// compiler's "not lowered yet": a real compile error is `-EINVAL`).
static SHADERS_REFUSED: AtomicU32 = AtomicU32::new(0);
/// Waits that found the GPU work lost to a fault, and to a timeout.
static GPU_FAULTS: AtomicU32 = AtomicU32::new(0);
static JOB_TIMEOUTS: AtomicU32 = AtomicU32::new(0);
static ANNOUNCED: AtomicBool = AtomicBool::new(false);

/// The first refusal of the process, by kind, with its name whole where the
/// library has it whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A refused draw: the reason of the library's "a draw was refused"
    /// line, which the OSAL's log line limit may have cut.
    Draw(String),
    /// A shader the compiler refused: its log.
    Compile(String),
    /// A program the compiler refused at link: its log.
    Link(String),
}

static FIRST: Mutex<Option<Refusal>> = Mutex::new(None);

/// Keep `r` if it is the first refusal of the process.
pub fn note_first(r: Refusal) {
    let mut g = FIRST.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(r);
    }
}

/// One line of text: every run of whitespace (a multi-line compiler log)
/// becomes one space.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The name a refusal is printed as: the gl_test sweep's histogram words
/// (`draw: `, `compile: `, `link: ` and the whole string). A draw's reason
/// from the log line is replaced by the Mali backend's own string when that
/// is the same reason whole (the log line cut it).
pub fn refusal_name(r: &Refusal, backend_whole: Option<&str>) -> String {
    match r {
        Refusal::Draw(why) => {
            let why = match backend_whole {
                Some(w) if w.starts_with(why.as_str()) => w,
                _ => why.as_str(),
            };
            format!("draw: {}", one_line(why))
        }
        Refusal::Compile(log) => format!("compile: {}", one_line(log)),
        Refusal::Link(log) => format!("link: {}", one_line(log)),
    }
}

/// The reason in the library's first-refused-draw line
/// (src/backend.rs `note_draw_refused`), as the OSAL's log hook gets it.
pub fn draw_refusal_of_log(line: &str) -> Option<&str> {
    let r = line.strip_prefix("yakogl: a draw was refused: ")?;
    Some(r.rsplit_once(" (key ").map_or(r, |(w, _)| w))
}

/// The loaded line.
pub fn loaded_line(build: &str, lib: &str) -> String {
    format!("{LOADED} build={build} lib={lib}")
}

/// The exit line. `draws`: draws the backend recorded. `refused`: draws the
/// GL core or the backend refused (`yakogl_draws_refused`) plus shaders and
/// programs the compiler refused. `first`: the first of those, by name,
/// else `-`; it is the only field that may hold spaces, so a reader takes
/// it as everything between `first_refused=` and ` gpu_faults=`.
/// `gpu_faults`, `job_timeouts`: waits that found the work lost to a GPU
/// fault and to a timeout.
pub fn exit_line(draws: u64, refused: u64, first: Option<&str>, gpu_faults: u32, job_timeouts: u32) -> String {
    let first = first.map(one_line).filter(|s| !s.is_empty()).unwrap_or_else(|| "-".into());
    format!("{EXIT} draws={draws} refused={refused} first_refused={first} gpu_faults={gpu_faults} job_timeouts={job_timeouts}")
}

/// What one OSAL wait says about the GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitOutcome {
    /// Done, or an error that is not the GPU's (a fence never submitted).
    Fine,
    /// The work was lost to a timeout: the wait's own (rc 1), or panthor's
    /// job timeout (the group TIMEDOUT).
    Timeout,
    /// The work was lost to a fault (the group FATAL_FAULT, or -EIO with
    /// no group state to tell).
    Fault,
}

/// Classify a wait's rc (panthor's `o_wait`: 0, 1 on timeout, -EIO on a
/// timed-out or faulted group) with the group's state read after it.
pub fn classify_wait(rc: i32, group_state: Option<u32>) -> WaitOutcome {
    match rc {
        0 => WaitOutcome::Fine,
        r if r > 0 => WaitOutcome::Timeout,
        r if r == -libc::EIO => match group_state {
            Some(s) if s & GROUP_TIMEDOUT != 0 => WaitOutcome::Timeout,
            _ => WaitOutcome::Fault,
        },
        _ => WaitOutcome::Fine,
    }
}

/// Count a wait.
fn count_wait(o: WaitOutcome) {
    match o {
        WaitOutcome::Fine => {}
        WaitOutcome::Timeout => {
            JOB_TIMEOUTS.fetch_add(1, Ordering::Relaxed);
        }
        WaitOutcome::Fault => {
            GPU_FAULTS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn group_state() -> Option<u32> {
    match panthor::group_state() {
        Some(Ok((s, _))) => Some(s),
        _ => None,
    }
}

/// The exit line of this process now. A group that faulted or timed out
/// with no wait to see it (the work was never waited for) counts once.
fn exit_line_now() -> String {
    let (mut faults, mut timeouts) = (GPU_FAULTS.load(Ordering::Relaxed), JOB_TIMEOUTS.load(Ordering::Relaxed));
    if faults == 0 && timeouts == 0 {
        match group_state() {
            Some(s) if s & GROUP_TIMEDOUT != 0 => timeouts = 1,
            Some(s) if s & GROUP_FATAL_FAULT != 0 => faults = 1,
            _ => {}
        }
    }
    let refused =
        u64::from(yakogl::backend::yakogl_draws_refused()) + u64::from(SHADERS_REFUSED.load(Ordering::Relaxed));
    let first = FIRST.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let name = first.map(|r| refusal_name(&r, yakogl::mali::backend::first_refusal()));
    exit_line(DRAWS.load(Ordering::Relaxed), refused, name.as_deref(), faults, timeouts)
}

/// One whole line on stderr, in one write (dEQP's own output is on stdout).
fn stderr_line(s: &str) {
    let mut b = Vec::with_capacity(s.len() + 1);
    b.extend_from_slice(s.as_bytes());
    b.push(b'\n');
    let mut off = 0;
    while off < b.len() {
        // SAFETY: a write of a live buffer's tail to fd 2.
        let n = unsafe { libc::write(2, b[off..].as_ptr().cast(), b.len() - off) };
        if n <= 0 {
            return;
        }
        off += n as usize;
    }
}

/// The path of the shared object this function is in.
fn this_object() -> String {
    // SAFETY: an all-zero Dl_info is valid; dladdr fills it for an address
    // of this object.
    unsafe {
        let mut info: libc::Dl_info = core::mem::zeroed();
        if libc::dladdr(this_object as *const c_void, &mut info) != 0 && !info.dli_fname.is_null() {
            let p = std::ffi::CStr::from_ptr(info.dli_fname).to_string_lossy().into_owned();
            return std::fs::canonicalize(&p).map_or(p, |c| c.display().to_string());
        }
    }
    "unknown".into()
}

/* ---- the OSAL: panthor's table with init, wait and log wrapped ----------- */

unsafe extern "C" fn w_init(arg: *mut c_void) -> i32 {
    if !ANNOUNCED.swap(true, Ordering::Relaxed) {
        stderr_line(&loaded_line(BUILD, &this_object()));
    }
    // SAFETY: panthor's init, with the caller's argument.
    panthor::OPS.init.map_or(-libc::ENODEV, |f| unsafe { f(arg) })
}

unsafe extern "C" fn w_wait(fence: i64, timeout_us: i64) -> i32 {
    // SAFETY: panthor's wait, with the caller's arguments.
    let rc = panthor::OPS.wait.map_or(-libc::ENODEV, |f| unsafe { f(fence, timeout_us) });
    let state = if rc == -libc::EIO { group_state() } else { None };
    count_wait(classify_wait(rc, state));
    rc
}

/// The log hook (the two arguments `osal::log_str` passes, `"%s"` and the
/// line): the first refused draw's reason is kept, then the line goes to
/// panthor's hook as it came.
extern "C" fn m_log(fmt: *const c_char, line: *const c_char) {
    if !line.is_null() {
        // SAFETY: osal::log_str passes a NUL-terminated line.
        let s = unsafe { std::ffi::CStr::from_ptr(line) }.to_string_lossy();
        if let Some(why) = draw_refusal_of_log(&s) {
            note_first(Refusal::Draw(why.to_string()));
        }
    }
    if let Some(f) = panthor::OPS.log {
        // SAFETY: panthor's variadic hook with the arguments it takes.
        unsafe { f(fmt, line) };
    }
}

/* `log` is a C variadic function, which stable Rust cannot define: the hook
 * is a one-instruction trampoline to m_log, as panthor's own
 * (tools/mali/panthor/src/osal.rs, which says why that is ABI-correct on
 * AArch64 AAPCS64 and x86-64 System V). */
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
core::arch::global_asm!(
    ".pushsection .text.yakogl_mali_gles_log,\"ax\",%progbits",
    ".globl yakogl_mali_gles_log",
    ".hidden yakogl_mali_gles_log",
    ".type yakogl_mali_gles_log,%function",
    "yakogl_mali_gles_log:",
    "b {log}",
    ".size yakogl_mali_gles_log, . - yakogl_mali_gles_log",
    ".popsection",
    log = sym m_log,
);
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
core::arch::global_asm!(
    ".pushsection .text.yakogl_mali_gles_log,\"ax\",@progbits",
    ".globl yakogl_mali_gles_log",
    ".hidden yakogl_mali_gles_log",
    ".type yakogl_mali_gles_log,@function",
    "yakogl_mali_gles_log:",
    "jmp {log}",
    ".size yakogl_mali_gles_log, . - yakogl_mali_gles_log",
    ".popsection",
    log = sym m_log,
);

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
extern "C" {
    fn yakogl_mali_gles_log(fmt: *const c_char, ...);
}

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
const LOG: Option<unsafe extern "C" fn(*const c_char, ...)> = Some(yakogl_mali_gles_log);
#[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
const LOG: Option<unsafe extern "C" fn(*const c_char, ...)> = None;

/* ---- the backend: draw wrapped ------------------------------------------- */

type DrawFn = unsafe extern "C" fn(*const GlesDraw) -> i32;
static INNER_DRAW: OnceLock<DrawFn> = OnceLock::new();

unsafe extern "C" fn w_draw(d: *const GlesDraw) -> i32 {
    let Some(f) = INNER_DRAW.get() else {
        return -libc::ENODEV;
    };
    // SAFETY: the backend's draw with the caller's table.
    let rc = unsafe { f(d) };
    // SAFETY: `d` is the caller's valid table (the backend just read it).
    if rc == 0 && !d.is_null() && unsafe { (*d).index_count } > 0 {
        DRAWS.fetch_add(1, Ordering::Relaxed);
    }
    rc
}

/* ---- the shader back end: compile, link, compile_vs_linked wrapped ------- */

/// The wrapped entries of the shader back end this build had.
#[derive(Clone, Copy)]
struct Shaders {
    compile: Option<unsafe extern "C" fn(u32, *const c_char, *mut ShcShader, *mut c_char, usize) -> i32>,
    link: Option<unsafe extern "C" fn(*const ShcShader, *mut ShcShader, *mut c_char, usize) -> i32>,
    compile_vs_linked: Option<
        unsafe extern "C" fn(
            *const c_char,
            *const ShcAttribMap,
            *const ShcShader,
            *mut ShcShader,
            *mut c_char,
            usize,
        ) -> i32,
    >,
}

static INNER_SHADERS: OnceLock<Shaders> = OnceLock::new();

/// Run `f` with a log of [`FULL_LOG`] bytes, hand the caller its own
/// (`log`, `log_sz`: NUL-terminated, cut as the back end would cut it), and
/// keep the whole log as the first refusal when `f` refused (`-ENOTSUP`).
///
/// # Safety
///
/// `log` is null or valid for `log_sz` bytes.
pub unsafe fn with_full_log(
    log: *mut c_char,
    log_sz: usize,
    kind: fn(String) -> Refusal,
    f: impl FnOnce(*mut c_char, usize) -> i32,
) -> i32 {
    let mut full = vec![0u8; FULL_LOG];
    let rc = f(full.as_mut_ptr().cast(), FULL_LOG);
    let n = full.iter().position(|&b| b == 0).unwrap_or(FULL_LOG);
    if !log.is_null() && log_sz > 0 {
        let k = n.min(log_sz - 1);
        // SAFETY: `k + 1 <= log_sz` bytes of the caller's buffer.
        unsafe {
            core::ptr::copy_nonoverlapping(full.as_ptr(), log.cast::<u8>(), k);
            *log.add(k) = 0;
        }
    }
    if rc == -yakogl::util::ENOTSUP {
        SHADERS_REFUSED.fetch_add(1, Ordering::Relaxed);
        note_first(kind(String::from_utf8_lossy(&full[..n]).into_owned()));
    }
    rc
}

unsafe extern "C" fn w_compile(
    stage: u32,
    glsl: *const c_char,
    out: *mut ShcShader,
    log: *mut c_char,
    log_sz: usize,
) -> i32 {
    let Some(f) = INNER_SHADERS.get().and_then(|s| s.compile) else {
        return -libc::ENODEV;
    };
    // SAFETY: the back end's compile with the caller's arguments and a log
    // of the size given; the caller's log is its own contract.
    unsafe { with_full_log(log, log_sz, Refusal::Compile, |l, n| f(stage, glsl, out, l, n)) }
}

unsafe extern "C" fn w_link(vs: *const ShcShader, fs: *mut ShcShader, log: *mut c_char, log_sz: usize) -> i32 {
    let Some(f) = INNER_SHADERS.get().and_then(|s| s.link) else {
        return -libc::ENODEV;
    };
    // SAFETY: as w_compile.
    unsafe { with_full_log(log, log_sz, Refusal::Link, |l, n| f(vs, fs, l, n)) }
}

unsafe extern "C" fn w_compile_vs_linked(
    glsl: *const c_char,
    map: *const ShcAttribMap,
    fs: *const ShcShader,
    out: *mut ShcShader,
    log: *mut c_char,
    log_sz: usize,
) -> i32 {
    let Some(f) = INNER_SHADERS.get().and_then(|s| s.compile_vs_linked) else {
        return -libc::ENODEV;
    };
    // SAFETY: as w_compile.
    unsafe { with_full_log(log, log_sz, Refusal::Link, |l, n| f(glsl, map, fs, out, l, n)) }
}

/* ---- installation -------------------------------------------------------- */

/* The copies the library's pointers are moved to. Bitwise copies of the
 * originals (they are plain tables of pointers), so an entry added to a
 * table later is carried over without this crate naming it. */
static mut OSAL_OUT: core::mem::MaybeUninit<OsalOps> = core::mem::MaybeUninit::uninit();
static mut BACKEND_OUT: core::mem::MaybeUninit<YakoglBackend> = core::mem::MaybeUninit::uninit();
static mut SHADERC_OUT: core::mem::MaybeUninit<ShcBackend> = core::mem::MaybeUninit::uninit();

/// Install the OSAL, the wrapped tables and the exit line. Once, before the
/// first EGL or GL call: the library's load-time constructor calls it.
///
/// # Safety
///
/// Single-threaded, before any GL or EGL call, at most once.
pub unsafe fn install() {
    // SAFETY: the caller's contract; each copy is of a live static table of
    // the same type, made before the pointer to it is published.
    unsafe {
        let o = (&raw mut OSAL_OUT).cast::<OsalOps>();
        core::ptr::copy_nonoverlapping(&panthor::OPS, o, 1);
        (*o).init = Some(w_init);
        (*o).wait = Some(w_wait);
        if panthor::OPS.log.is_some() {
            (*o).log = LOG;
        }
        yakogl::osal::osal_set_ops(o);

        let b = yakogl::backend::g_backend;
        if !b.is_null() {
            let c = (&raw mut BACKEND_OUT).cast::<YakoglBackend>();
            core::ptr::copy_nonoverlapping(b, c, 1);
            if let Some(d) = (*c).draw {
                let _ = INNER_DRAW.set(d);
                (*c).draw = Some(w_draw);
            }
            yakogl::backend::g_backend = c;
        }

        /* backend() installs the in-process compiler first (once) */
        if let Some(s) = yakogl::shaderc::backend() {
            let c = (&raw mut SHADERC_OUT).cast::<ShcBackend>();
            core::ptr::copy_nonoverlapping(s, c, 1);
            let _ =
                INNER_SHADERS.set(Shaders { compile: s.compile, link: s.link, compile_vs_linked: s.compile_vs_linked });
            if s.compile.is_some() {
                (*c).compile = Some(w_compile);
            }
            if s.link.is_some() {
                (*c).link = Some(w_link);
            }
            if s.compile_vs_linked.is_some() {
                (*c).compile_vs_linked = Some(w_compile_vs_linked);
            }
            yakogl::shaderc::g_shaderc = c;
        }

        libc::atexit(at_exit);
    }
}

extern "C" fn at_exit() {
    stderr_line(&exit_line_now());
}

#[cfg(not(test))]
extern "C" fn on_load() {
    // SAFETY: the dynamic loader runs this once, before the application can
    // reach an entry point of this object.
    unsafe { install() }
}

/// The load-time constructor (`DT_INIT_ARRAY`).
#[cfg(not(test))]
#[used]
#[link_section = ".init_array"]
static ON_LOAD: extern "C" fn() = on_load;

/// The stamp between NUL bytes, for a tool that looks for
/// `\0yakogl-build=<describe>\0` in the object (as run-on-board.sh does in
/// panthor's binaries).
#[used]
pub static BUILD_MARK: &str = concat!("\0yakogl-build=", env!("YAKOGL_MALI_GLES_BUILD"), "\0");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loaded_line_names_the_build_and_the_object() {
        assert_eq!(
            loaded_line("009f62a5-dirty", "/root/yakogl-mali/libGLESv2.so.2"),
            "yakogl-mali: loaded build=009f62a5-dirty lib=/root/yakogl-mali/libGLESv2.so.2"
        );
    }

    #[test]
    fn the_exit_line_has_its_fields_in_the_contract_order() {
        assert_eq!(
            exit_line(12, 0, None, 0, 0),
            "yakogl-mali-exit: draws=12 refused=0 first_refused=- gpu_faults=0 job_timeouts=0"
        );
        let l = exit_line(3, 2, Some("compile: texture2D is\n  not lowered yet"), 1, 4);
        assert_eq!(
            l,
            "yakogl-mali-exit: draws=3 refused=2 first_refused=compile: texture2D is not lowered yet gpu_faults=1 job_timeouts=4"
        );
        /* the reader's rule: the name is what lies between the two keys */
        let (_, rest) = l.split_once(" first_refused=").unwrap();
        let (name, tail) = rest.rsplit_once(" gpu_faults=").unwrap();
        assert_eq!(name, "compile: texture2D is not lowered yet");
        assert_eq!(tail, "1 job_timeouts=4");
        assert!(exit_line(0, 0, Some(" \n"), 0, 0).contains("first_refused=- "));
    }

    #[test]
    fn the_draw_reason_is_read_off_the_librarys_line() {
        let line = "yakogl: a draw was refused: mali: blend equation not lowered (key 0x3); yakogl_draws_refused counts every one";
        assert_eq!(draw_refusal_of_log(line), Some("mali: blend equation not lowered"));
        assert_eq!(draw_refusal_of_log("yakogl: GPU work lost: backend flush returned -5"), None);
    }

    #[test]
    fn a_cut_draw_reason_takes_the_backends_whole_string() {
        let whole = format!("mali: {}", "x".repeat(300));
        let cut = Refusal::Draw(whole[..200].to_string());
        assert_eq!(refusal_name(&cut, Some(&whole)), format!("draw: {whole}"));
        /* a GL core refusal is not the backend's first: its own reason */
        let core = Refusal::Draw("gl core: no blit program".into());
        assert_eq!(refusal_name(&core, Some(&whole)), "draw: gl core: no blit program");
        assert_eq!(
            refusal_name(&Refusal::Link("varying v_x\nis not written".into()), None),
            "link: varying v_x is not written"
        );
    }

    #[test]
    fn waits_count_faults_and_timeouts_apart() {
        assert_eq!(classify_wait(0, None), WaitOutcome::Fine);
        assert_eq!(classify_wait(1, None), WaitOutcome::Timeout);
        assert_eq!(classify_wait(-libc::EIO, Some(GROUP_TIMEDOUT)), WaitOutcome::Timeout);
        assert_eq!(classify_wait(-libc::EIO, Some(GROUP_TIMEDOUT | GROUP_FATAL_FAULT)), WaitOutcome::Timeout);
        assert_eq!(classify_wait(-libc::EIO, Some(GROUP_FATAL_FAULT)), WaitOutcome::Fault);
        assert_eq!(classify_wait(-libc::EIO, None), WaitOutcome::Fault);
        /* a fence never submitted is the caller's, not the GPU's */
        assert_eq!(classify_wait(-libc::EINVAL, None), WaitOutcome::Fine);
    }

    const CHILD: &str = "YAKOGL_MALI_GLES_TEST_CHILD";

    /// The child of [`a_process_prints_the_loaded_line_once_and_the_exit_line`]:
    /// what the load-time constructor does, then the first EGL calls dEQP
    /// makes. Nothing outside that child.
    #[test]
    fn child_install_and_initialize() {
        if std::env::var_os(CHILD).is_none() {
            return;
        }
        // SAFETY: this process's first and only install, before any EGL call.
        unsafe { install() };
        // SAFETY: EGL entry points with valid arguments.
        unsafe {
            let d = yakogl::gl::egl::eglGetDisplay(core::ptr::null_mut());
            /* no panthor device: init fails, after its loaded line */
            yakogl::gl::egl::eglInitialize(d, core::ptr::null_mut(), core::ptr::null_mut());
            yakogl::gl::egl::eglInitialize(d, core::ptr::null_mut(), core::ptr::null_mut());
        }
    }

    /// The contract lines, end to end in a process of their own: the loaded
    /// line at the first eglInitialize and not again, the exit line at exit.
    #[test]
    fn a_process_prints_the_loaded_line_once_and_the_exit_line() {
        let exe = std::env::current_exe().unwrap();
        let out = std::process::Command::new(exe)
            .args(["--exact", "tests::child_install_and_initialize", "--test-threads=1"])
            .env(CHILD, "1")
            .env("YAKOGL_MALI_DEV", "/nonexistent/renderD128")
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        let lines: Vec<&str> = err.lines().collect();
        let loaded: Vec<&&str> = lines.iter().filter(|l| l.starts_with(LOADED)).collect();
        assert_eq!(loaded.len(), 1, "{err}");
        assert!(loaded[0].starts_with(&format!("{LOADED} build={BUILD} lib=/")), "{err}");
        assert_eq!(
            lines.last().copied(),
            Some("yakogl-mali-exit: draws=0 refused=0 first_refused=- gpu_faults=0 job_timeouts=0"),
            "{err}"
        );
    }

    /// The full-log wrapper: the caller gets its cut copy, a refusal is
    /// counted with the whole log, and a plain compile error is not one.
    #[test]
    fn only_a_refused_compile_counts_and_its_log_is_whole() {
        let long = "y".repeat(1000);
        let mut small = [0x55u8; 16];
        let before = SHADERS_REFUSED.load(Ordering::Relaxed);
        let fill = |msg: &'static str, rc: i32| {
            move |l: *mut c_char, n: usize| {
                assert_eq!(n, FULL_LOG);
                // SAFETY: the wrapper's buffer of `n` bytes.
                unsafe {
                    core::ptr::copy_nonoverlapping(msg.as_ptr(), l.cast::<u8>(), msg.len());
                    *l.add(msg.len()) = 0;
                }
                rc
            }
        };
        let msg: &'static str = Box::leak(format!("refused: {long}").into_boxed_str());
        // SAFETY: `small` is valid for its length.
        let rc = unsafe {
            with_full_log(small.as_mut_ptr().cast(), small.len(), Refusal::Compile, fill(msg, -yakogl::util::ENOTSUP))
        };
        assert_eq!(rc, -yakogl::util::ENOTSUP);
        assert_eq!(&small[..15], &msg.as_bytes()[..15]);
        assert_eq!(small[15], 0);
        assert_eq!(SHADERS_REFUSED.load(Ordering::Relaxed), before + 1);
        // SAFETY: as above.
        let rc = unsafe {
            with_full_log(
                small.as_mut_ptr().cast(),
                small.len(),
                Refusal::Compile,
                fill("syntax error", -yakogl::util::EINVAL),
            )
        };
        assert_eq!(rc, -yakogl::util::EINVAL);
        assert_eq!(&small[..13], b"syntax error\0");
        assert_eq!(SHADERS_REFUSED.load(Ordering::Relaxed), before + 1);
        let first = FIRST.lock().unwrap().clone();
        assert_eq!(first, Some(Refusal::Compile(msg.to_string())));
    }
}
