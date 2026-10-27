// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! panthor-gltest: gl_test (driver/tools/pvrlab/gl_test.c, the GL suite
//! every board runs) on the NanoPC-T6 under Armbian, through YakoGL's
//! `gpu-mali` backend and the development OSAL ([`yakogl_mali_panthor::osal`]).
//! Milestone L3 of driver/docs/nanopc-t6-mali.md, section "L3: gl_test under
//! Armbian", which gives the design.
//!
//! Usage: panthor-gltest [--set | --sweep | <case>...] [--timeout S]
//!        panthor-gltest --one <case>
//!
//! - `--set` (the default): the regression set, gl_test.c's own list
//!   (`gl_test_set_cases`).
//! - `--sweep`: every case number 0..=255; the numbers with no case on this
//!   build (gl_test returns -2) are counted, not listed.
//! - `<case>...`: those cases, in that order.
//! - `--one <case>`: the child: one case through gl_test's walker
//!   (`gl_test_set_list` + `gl_test_run`), then one `result` line.
//!
//! Each case runs in a process of its own, one after the other (one GL
//! client at a time). That is what makes each case's refusal visible: the
//! library logs the first refused draw of a process only, and a `glog_once!`
//! line once per process, so in one long process a case would inherit the
//! silence of the case that refused first. It also keeps a case that faults
//! the group from deciding the verdicts of the cases after it.
//!
//! The verdict of a case (printed as a `verdict` line, then the table and the
//! histogram of refusal strings at the end):
//!
//! | verdict | when |
//! | --- | --- |
//! | `NOT-STARTED` | the child printed nothing, not even its build line: the harness's environment, not the case |
//! | `FAULT` | GPU work lost, the group not in state 0, the VM unusable, the child killed by a signal, or no `result` line |
//! | `HANG` | the child did not finish within the timeout (killed) |
//! | `REFUSED` | a draw was refused, or rc is not 0 and a shader or a program was refused; the first such string is the row's |
//! | `PASS` | rc 0 |
//! | `FAIL` | rc -1 with nothing refused; gl_test's first bad check, got and want |
//! | `SKIP` | rc -3: gl_test skipped the case by name |
//! | `NO-CASE` | rc -2: no such case on this build |
//!
//! The refusal strings are whole, never the 255-byte log line (a cut
//! string can hide the real refuser), and the child prints each as a line
//! of its own:
//!
//! - `draw refused: <why>`: the Mali backend's first refusal of the process
//!   (`yakogl::mali::backend::first_refusal`, the `&'static str` itself).
//!   A draw the GL core refused before the backend (src/gl/draw.rs) has
//!   only the library's log line, `yakogl: a draw was refused: <why> (key
//!   ...)`; that one is marked truncated when it fills the line.
//! - `compile refused: <log>`: gl_test's `glCompileShader` calls are
//!   wrapped (`build.rs`: `-Wl,--wrap`); after a failed compile the child
//!   compiles the same source again through the same shader back end
//!   (`shaderc::backend().compile`) into an 8 KiB log, because the GL info
//!   log the library keeps is 256 bytes (`INFO_LOG_SIZE`).
//! - `link refused: <log>`: gl_test's `glLinkProgram` calls are wrapped the
//!   same way; the program's info log is read whole, and marked truncated
//!   when it fills the library's 256 bytes (the link log has no second
//!   source).
//!
//! A child that printed nothing at all, not even its build line, never
//! started: `NOT-STARTED` with its exit status (qemu-user with no binfmt
//! for the target reads exit status 127 here), never a verdict on a case.

use std::collections::BTreeMap;
use std::ffi::{c_char, c_int, c_uint};
use std::io::Read;
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};
use yakogl::gl::consts::*;
use yakogl::gl::program::{glGetProgramInfoLog, glGetProgramiv};
use yakogl::gl::shader::{glGetShaderSource, glGetShaderiv};
use yakogl::gl::types::*;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
// gl_test.c's walker (gl_test_set_list, gl_test_set_cases, gl_test_runner):
// linked for its `#[no_mangle]` exports, which gl_test.o calls.
use yakogl_gltest_runner as _;
use yakogl_mali_panthor::osal;

extern "C" {
    fn gl_test_run(
        gl_case: c_int,
        pretransform: c_int,
        ppp_ctrl: c_uint,
        debug: c_int,
        no_coalesce: c_int,
        panel: c_int,
    ) -> c_int;
    fn gl_test_set_list(cases: *const c_int, n: c_uint, stored: *mut c_uint) -> c_int;
    fn gl_test_set_cases(cases: *mut *const c_int) -> c_uint;
    static gl_test_first_bad_check: c_int;
    static gl_test_first_bad_got: u32;
    static gl_test_first_bad_want: u32;
    static mut stdout: *mut libc::FILE;
    /* the library's own entry points, behind the --wrap of build.rs */
    fn __real_glCompileShader(shader: GLuint);
    fn __real_glLinkProgram(program: GLuint);
}

/// The log the second compile writes into: far past the library's 256.
const FULL_LOG: usize = 8192;
/// The library's info log size (src/gl/shader.rs `INFO_LOG_SIZE`): a link
/// log of this length, its NUL counted, may have been cut.
const GL_INFO_LOG_SIZE: GLint = 256;
const DRAW_REFUSED: &str = "panthor-gltest: draw refused: ";
const COMPILE_REFUSED: &str = "panthor-gltest: compile refused: ";
const LINK_REFUSED: &str = "panthor-gltest: link refused: ";
const TRUNCATED: &str = " [TRUNCATED at the log line limit]";

/// gl_test's glCompileShader (build.rs wraps the calls of gl_test.o only).
/// After a failed compile, the same source through the same back end into
/// a log that holds the whole message.
///
/// # Safety
///
/// A GL entry point: the C caller's contract.
#[no_mangle]
pub unsafe extern "C" fn __wrap_glCompileShader(shader: GLuint) {
    __real_glCompileShader(shader);
    let (mut ok, mut len, mut ty) = (0, 0, 0);
    glGetShaderiv(shader, GL_COMPILE_STATUS, &mut ok);
    if ok == 1 {
        return;
    }
    glGetShaderiv(shader, GL_SHADER_SOURCE_LENGTH, &mut len);
    glGetShaderiv(shader, GL_SHADER_TYPE, &mut ty);
    let mut src = vec![0u8; len.max(1) as usize];
    glGetShaderSource(shader, len.max(1), core::ptr::null_mut(), src.as_mut_ptr().cast());
    let Some(compile) = yakogl::shaderc::backend().and_then(|b| b.compile) else {
        println!("{COMPILE_REFUSED}(no shader back end to compile again: the GL log only){TRUNCATED}");
        return;
    };
    let stage = if ty as GLenum == GL_VERTEX_SHADER { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
    let mut log = vec![0u8; FULL_LOG];
    // SAFETY: all-zero is a valid ShcShader (src/gl/shader.rs makes it so).
    let mut out: ShcShader = core::mem::zeroed();
    let rc = compile(stage, src.as_ptr().cast(), &mut out, log.as_mut_ptr().cast(), FULL_LOG);
    if rc == 0 {
        yakogl::shaderc::release_shader(&out);
        println!("{COMPILE_REFUSED}the GL core refused it, but the same source compiled again (an instrument defect)");
        return;
    }
    let n = log.iter().position(|&b| b == 0).unwrap_or(FULL_LOG);
    let cut = if n >= FULL_LOG - 1 { TRUNCATED } else { "" };
    println!("{COMPILE_REFUSED}{}{cut}", String::from_utf8_lossy(&log[..n]));
}

/// gl_test's glLinkProgram (wrapped as above): a failed link's whole log.
///
/// # Safety
///
/// A GL entry point: the C caller's contract.
#[no_mangle]
pub unsafe extern "C" fn __wrap_glLinkProgram(program: GLuint) {
    __real_glLinkProgram(program);
    let (mut ok, mut len) = (0, 0);
    glGetProgramiv(program, GL_LINK_STATUS, &mut ok);
    if ok == 1 {
        return;
    }
    glGetProgramiv(program, GL_INFO_LOG_LENGTH, &mut len);
    let mut log = vec![0u8; len.max(1) as usize];
    let mut n: GLsizei = 0;
    glGetProgramInfoLog(program, len.max(1), &mut n, log.as_mut_ptr().cast::<c_char>());
    let cut = if len >= GL_INFO_LOG_SIZE { TRUNCATED } else { "" };
    println!("{LINK_REFUSED}{}{cut}", String::from_utf8_lossy(&log[..n.max(0) as usize]));
}

/// The case's rc as gl_test's walker reports it; i32::MIN until it does.
static CASE_RC: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(i32::MIN);

/// gl_test.c's weak hook, told each case's rc as the walker finishes it.
#[no_mangle]
pub extern "C" fn gl_test_case_result(_c: c_int, rc: c_int) {
    CASE_RC.store(rc, std::sync::atomic::Ordering::Relaxed);
}

/// The OSAL's log line limit (src/osal/mod.rs `OSAL_LOG_LINE_MAX`, with
/// its NUL): a string this long may have been cut.
const LOG_LINE_FULL: usize = yakogl::osal::OSAL_LOG_LINE_MAX - 1;
const RESULT: &str = "panthor-gltest: result ";

fn child(case: c_int) -> ExitCode {
    // SAFETY: C stdio's stdout, made line-buffered before anything is
    // written, so gl_test's printf lines and Rust's println lines reach the
    // pipe in the order they were made.
    unsafe { libc::setvbuf(stdout, core::ptr::null_mut(), libc::_IOLBF, 0) };
    println!("panthor-gltest: build {}", yakogl_mali_panthor::BUILD);
    osal::install();
    let lost0 = yakogl::backend::yakogl_gpu_work_lost();
    let refused0 = yakogl::backend::yakogl_draws_refused();
    let list = [case];
    // SAFETY: the suite's entry points, on this thread, in order; the list
    // outlives the call that copies it.
    let rc = unsafe {
        let which = gl_test_set_list(list.as_ptr(), 1, core::ptr::null_mut());
        gl_test_run(which, 0, 0, 0, 0, 0)
    };
    // SAFETY: flush C's buffered output before Rust's next line.
    unsafe { libc::fflush(core::ptr::null_mut()) };
    let case_rc = CASE_RC.load(std::sync::atomic::Ordering::Relaxed);
    let lost = yakogl::backend::yakogl_gpu_work_lost().wrapping_sub(lost0);
    let refused = yakogl::backend::yakogl_draws_refused().wrapping_sub(refused0);
    // SAFETY: plain reads of gl_test's globals after the case.
    let (bad, got, want) = unsafe {
        (
            core::ptr::read_volatile(&raw const gl_test_first_bad_check),
            core::ptr::read_volatile(&raw const gl_test_first_bad_got),
            core::ptr::read_volatile(&raw const gl_test_first_bad_want),
        )
    };
    let group = match osal::group_state() {
        Some(Ok((s, f))) => format!("{s:#x}/{f:#x}"),
        Some(Err(e)) => format!("error:{}", e.to_string().replace(' ', "_")),
        None => "absent".into(),
    };
    let vm = match osal::vm_usable() {
        Some(Ok(u)) => u.to_string(),
        Some(Err(e)) => format!("error:{}", e.to_string().replace(' ', "_")),
        None => "absent".into(),
    };
    if let Some(why) = yakogl::mali::backend::first_refusal() {
        println!("{DRAW_REFUSED}{why}");
    }
    match osal::counts() {
        Some((subs, open, live, live_hw, bytes, bytes_hw)) => println!(
            "panthor-gltest: osal submits {subs}, unwaited fences {open}, BOs live {live} (high-water {live_hw}), bytes live {bytes} (high-water {bytes_hw})"
        ),
        None => println!("panthor-gltest: osal absent (init failed)"),
    }
    if let Some((held, held_hw)) = osal::pending() {
        println!("panthor-gltest: osal BOs held for a kick {held} (high-water {held_hw})");
    }
    if let Some((windows, windows_hw)) = osal::windows() {
        println!("panthor-gltest: osal map_phys windows live {windows} (high-water {windows_hw})");
    }
    let case_rc = if case_rc == i32::MIN { "absent".to_string() } else { case_rc.to_string() };
    println!(
        "{RESULT}case {case} run_rc {rc} case_rc {case_rc} draws_refused {refused} work_lost {lost} group {group} vm {vm} first_bad {bad} got {got:#010x} want {want:#010x}"
    );
    ExitCode::SUCCESS
}

/// What one child reported.
#[derive(Debug, Default, PartialEq, Eq)]
struct Report {
    /// The child's build line was seen: it started.
    started: bool,
    /// The child's `result` line was seen.
    result: bool,
    run_rc: i32,
    case_rc: Option<i32>,
    refused: u32,
    lost: u32,
    group: String,
    vm: String,
    first_bad: i32,
    got: String,
    want: String,
    name: Option<String>,
    /// Draw refusals, compile refusals, link refusals, in output order.
    refusals: Vec<String>,
    draw_refusals: usize,
    /// The backend's first refused draw, whole (`DRAW_REFUSED`): the row's
    /// reason when present, ahead of the library's cut log line.
    backend_draw: Option<String>,
}

fn field<'a>(words: &'a [&'a str], key: &str) -> Option<&'a str> {
    words.iter().position(|w| *w == key).and_then(|i| words.get(i + 1)).copied()
}

/// The refusal string a line carries, if it carries one, and whether it is
/// a refused draw.
fn refusal(line: &str) -> Option<(String, bool)> {
    if let Some(r) = line.strip_prefix(DRAW_REFUSED) {
        return Some((format!("draw: {r}"), true));
    }
    if let Some(r) = line.strip_prefix(COMPILE_REFUSED) {
        return Some((format!("compile: {r}"), false));
    }
    if let Some(r) = line.strip_prefix(LINK_REFUSED) {
        return Some((format!("link: {r}"), false));
    }
    /* the GL core's refusals have the library's line only */
    if let Some(r) = line.strip_prefix("yakogl: yakogl: a draw was refused: ") {
        let full = line.len() >= LOG_LINE_FULL + "yakogl: ".len();
        let why = r.rsplit_once(" (key ").map_or(r, |(w, _)| w);
        return Some((format!("draw: {why}{}", if full { TRUNCATED } else { "" }), true));
    }
    None
}

fn parse(case: i32, out: &str) -> Option<Report> {
    let mut rep = Report::default();
    let name_mark = format!("gl_test: case {case} ");
    for line in out.lines() {
        rep.started |= line.starts_with("panthor-gltest: build ");
        if line.starts_with(DRAW_REFUSED) && rep.backend_draw.is_none() {
            rep.backend_draw = refusal(line).map(|(s, _)| s);
        }
        if let Some((s, draw)) = refusal(line) {
            rep.draw_refusals += usize::from(draw);
            rep.refusals.push(s);
        }
        if let Some(r) = line.strip_prefix(&name_mark) {
            if let Some((n, _)) = r.split_once(": ") {
                rep.name.get_or_insert_with(|| n.to_string());
            }
        }
        if let Some(r) = line.strip_prefix(RESULT) {
            let w: Vec<&str> = r.split_whitespace().collect();
            rep.run_rc = field(&w, "run_rc")?.parse().ok()?;
            rep.case_rc = field(&w, "case_rc").and_then(|s| s.parse().ok());
            rep.refused = field(&w, "draws_refused")?.parse().ok()?;
            rep.lost = field(&w, "work_lost")?.parse().ok()?;
            rep.group = field(&w, "group")?.to_string();
            rep.vm = field(&w, "vm")?.to_string();
            rep.first_bad = field(&w, "first_bad")?.parse().ok()?;
            rep.got = field(&w, "got")?.to_string();
            rep.want = field(&w, "want")?.to_string();
            rep.result = true;
        }
    }
    (rep.started || rep.result).then_some(rep)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Verdict {
    Pass,
    Fail(String),
    Refused(String),
    Skip,
    NoCase,
    Fault(String),
    Hang,
    NotStarted(String),
}

impl Verdict {
    fn word(&self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::Fail(_) => "FAIL",
            Verdict::Refused(_) => "REFUSED",
            Verdict::Skip => "SKIP",
            Verdict::NoCase => "NO-CASE",
            Verdict::Fault(_) => "FAULT",
            Verdict::Hang => "HANG",
            Verdict::NotStarted(_) => "NOT-STARTED",
        }
    }
}

/// The verdict of a child that exited (see the module table).
fn classify(rep: Option<&Report>, signalled: Option<i32>) -> Verdict {
    if let Some(s) = signalled {
        return Verdict::Fault(format!("killed by signal {s}"));
    }
    let Some(r) = rep else {
        return Verdict::NotStarted("the child printed nothing, not even its build line".into());
    };
    if !r.result {
        return Verdict::Fault("no result line".into());
    }
    let Some(rc) = r.case_rc else {
        return Verdict::Fault(format!(
            "no case ran: gl_test_run returned {} before it (the OSAL, the backend or the context did not come up)",
            r.run_rc
        ));
    };
    if r.lost != 0 || r.group != "0x0/0x0" || r.vm != "true" {
        return Verdict::Fault(format!(
            "work lost {}, group state/fatal queues {}, vm usable {}",
            r.lost, r.group, r.vm
        ));
    }
    if r.refused != 0 {
        let first = r.backend_draw.clone().or_else(|| r.refusals.iter().find(|s| s.starts_with("draw: ")).cloned());
        return Verdict::Refused(first.unwrap_or_else(|| {
            "draw: refused, and no reason line in the output of this process (an instrument defect: the first refusal of a process is logged)".to_string()
        }));
    }
    match rc {
        0 => Verdict::Pass,
        -2 => Verdict::NoCase,
        -3 => Verdict::Skip,
        _ if !r.refusals.is_empty() => Verdict::Refused(r.refusals[0].clone()),
        _ => Verdict::Fail(format!("rc {rc}, first bad check {} got {} want {}", r.first_bad, r.got, r.want)),
    }
}

fn run_child(exe: &std::path::Path, case: i32, timeout: Duration) -> (Verdict, Option<String>) {
    let mut ch = match Command::new(exe)
        .args(["--one", &case.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return (Verdict::Fault(format!("spawn: {e}")), None),
    };
    let mut so = ch.stdout.take().expect("piped");
    let mut se = ch.stderr.take().expect("piped");
    let ro = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = so.read_to_string(&mut s);
        s
    });
    let re = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = se.read_to_string(&mut s);
        s
    });
    let t0 = Instant::now();
    let status = loop {
        match ch.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) if t0.elapsed() >= timeout => {
                let _ = ch.kill();
                let _ = ch.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break None,
        }
    };
    let out = ro.join().unwrap_or_default();
    let err = re.join().unwrap_or_default();
    print!("{out}");
    if !err.is_empty() {
        for l in err.lines() {
            println!("stderr: {l}");
        }
    }
    let rep = parse(case, &out);
    let verdict = match status {
        None => Verdict::Hang,
        Some(st) => {
            use std::os::unix::process::ExitStatusExt;
            match classify(rep.as_ref(), st.signal()) {
                Verdict::Fault(s) if !rep.as_ref().is_some_and(|r| r.result) => Verdict::Fault(format!("{s} ({st})")),
                Verdict::NotStarted(s) => Verdict::NotStarted(format!("{s} ({st})")),
                v => v,
            }
        }
    };
    (verdict, rep.and_then(|r| r.name))
}

fn set_cases() -> Vec<i32> {
    let mut p: *const c_int = core::ptr::null();
    // SAFETY: gl_test.c hands back its static list and its length.
    unsafe {
        let n = gl_test_set_cases(&mut p);
        std::slice::from_raw_parts(p, n as usize).to_vec()
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--one") {
        return match args.get(1).and_then(|s| s.parse().ok()) {
            Some(c) => child(c),
            None => {
                eprintln!("panthor-gltest: --one takes a case number");
                ExitCode::from(2)
            }
        };
    }
    println!("panthor-gltest: build {}", yakogl_mali_panthor::BUILD);
    let (mut cases, mut sweep, mut timeout) = (Vec::new(), false, Duration::from_secs(60));
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--set" => cases.extend(set_cases()),
            "--sweep" => {
                sweep = true;
                cases.extend(0..=255);
            }
            "--timeout" => match it.next().and_then(|s| s.parse().ok()) {
                Some(s) => timeout = Duration::from_secs(s),
                None => {
                    eprintln!("panthor-gltest: --timeout takes seconds");
                    return ExitCode::from(2);
                }
            },
            s => match s.parse() {
                Ok(c) => cases.push(c),
                Err(_) => {
                    eprintln!("panthor-gltest: unknown argument {s}");
                    return ExitCode::from(2);
                }
            },
        }
    }
    if cases.is_empty() {
        cases = set_cases();
    }
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("panthor-gltest: current_exe: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("panthor-gltest: {} case(s), one process each, timeout {} s", cases.len(), timeout.as_secs());
    let mut rows = Vec::new();
    for &c in &cases {
        println!("panthor-gltest: === case {c}");
        let (v, name) = run_child(&exe, c, timeout);
        let what = match &v {
            Verdict::Fail(s) | Verdict::Refused(s) | Verdict::Fault(s) | Verdict::NotStarted(s) => s.as_str(),
            _ => "",
        };
        println!("panthor-gltest: verdict case {c} {}: {} {what}", name.as_deref().unwrap_or("?"), v.word());
        rows.push((c, name, v));
    }

    println!("panthor-gltest: table");
    let mut tally: BTreeMap<&str, usize> = BTreeMap::new();
    let mut hist: BTreeMap<String, Vec<i32>> = BTreeMap::new();
    for (c, name, v) in &rows {
        *tally.entry(v.word()).or_default() += 1;
        if sweep && *v == Verdict::NoCase {
            continue;
        }
        let what = match v {
            Verdict::Fail(s) | Verdict::Refused(s) | Verdict::Fault(s) | Verdict::NotStarted(s) => s.as_str(),
            _ => "",
        };
        println!("| {c} | {} | {} | {what} |", name.as_deref().unwrap_or("?"), v.word());
        if let Verdict::Refused(s) = v {
            hist.entry(s.clone()).or_default().push(*c);
        }
    }
    println!("panthor-gltest: refusal histogram (whole strings)");
    let mut h: Vec<_> = hist.into_iter().collect();
    h.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
    for (s, cs) in &h {
        let list: Vec<String> = cs.iter().map(i32::to_string).collect();
        println!("{:4}  {s}  (cases {})", cs.len(), list.join(" "));
    }
    let summary: Vec<String> = tally.iter().map(|(k, n)| format!("{k} {n}")).collect();
    println!("panthor-gltest: {} case(s) run: {}", rows.len(), summary.join(", "));
    let bad = rows.iter().any(|(_, _, v)| matches!(v, Verdict::Fault(_) | Verdict::Hang | Verdict::Fail(_)));
    if bad {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK_RESULT: &str = "panthor-gltest: result case 1 run_rc 0 case_rc 0 draws_refused 0 work_lost 0 group 0x0/0x0 vm true first_bad -1 got 0x00000000 want 0x00000000";

    #[test]
    fn a_passing_case_reads_pass() {
        let out = format!("gl_test: case 1 uniform: PASS\n{OK_RESULT}\n");
        let r = parse(1, &out).expect("a result line");
        assert_eq!(r.name.as_deref(), Some("uniform"));
        assert_eq!(classify(Some(&r), None), Verdict::Pass);
    }

    #[test]
    fn a_refused_draw_is_refused_with_its_whole_reason() {
        let long = "y".repeat(LOG_LINE_FULL + 40);
        let out = format!(
            "panthor-gltest: build x\nyakogl: yakogl: a draw was refused: {} (key 0x0); yakogl_draws_refused counts every one\n{DRAW_REFUSED}{long}\n{}",
            &long[..200],
            OK_RESULT.replace("case_rc 0 draws_refused 0", "case_rc -1 draws_refused 3")
        );
        let r = parse(1, &out).unwrap();
        /* the backend's own string wins over the library's (cut) line */
        assert_eq!(classify(Some(&r), None), Verdict::Refused(format!("draw: {long}")));
    }

    #[test]
    fn the_libraries_draw_line_alone_is_marked_when_full() {
        let full = format!("yakogl: yakogl: a draw was refused: {}", "z".repeat(LOG_LINE_FULL));
        let (s, draw) = refusal(&full).unwrap();
        assert!(draw);
        assert!(s.ends_with(TRUNCATED), "{s}");
        let short = "yakogl: yakogl: a draw was refused: gl core: no blit program (key 0x3); yakogl_draws_refused counts every one";
        assert_eq!(refusal(short), Some(("draw: gl core: no blit program".into(), true)));
    }

    #[test]
    fn a_compile_refusal_with_a_failed_case_is_refused_and_a_passed_one_passes() {
        let line = format!("{COMPILE_REFUSED}texture2D is not lowered yet");
        let failed = format!("{line}\n{}", OK_RESULT.replace("case_rc 0", "case_rc -1"));
        let r = parse(1, &failed).unwrap();
        assert_eq!(classify(Some(&r), None), Verdict::Refused("compile: texture2D is not lowered yet".into()));
        /* a case that asks for a compile error on purpose and passes */
        let passed = format!("{line}\n{OK_RESULT}");
        assert_eq!(classify(Some(&parse(1, &passed).unwrap()), None), Verdict::Pass);
        let link =
            format!("{LINK_REFUSED}varying v_x is not written\n{}", OK_RESULT.replace("case_rc 0", "case_rc -1"));
        assert_eq!(
            classify(parse(1, &link).as_ref(), None),
            Verdict::Refused("link: varying v_x is not written".into())
        );
    }

    #[test]
    fn a_child_that_printed_nothing_did_not_start() {
        assert!(matches!(classify(parse(1, "").as_ref(), None), Verdict::NotStarted(_)));
        /* started, then gone before its result line: the case's fault */
        let started = "panthor-gltest: build x\ngl_test: start";
        assert_eq!(classify(parse(1, started).as_ref(), None), Verdict::Fault("no result line".into()));
    }

    #[test]
    fn a_failed_check_names_got_and_want() {
        let out = OK_RESULT
            .replace("case_rc 0", "case_rc -1")
            .replace("first_bad -1 got 0x00000000", "first_bad 2 got 0xff0000ff");
        let r = parse(1, &out).unwrap();
        assert_eq!(
            classify(Some(&r), None),
            Verdict::Fail("rc -1, first bad check 2 got 0xff0000ff want 0x00000000".into())
        );
    }

    #[test]
    fn lost_work_a_group_fault_or_no_result_is_a_fault() {
        let lost = OK_RESULT.replace("work_lost 0", "work_lost 1");
        assert!(matches!(classify(parse(1, &lost).as_ref(), None), Verdict::Fault(_)));
        let group = OK_RESULT.replace("group 0x0/0x0", "group 0x2/0x1");
        assert!(matches!(classify(parse(1, &group).as_ref(), None), Verdict::Fault(_)));
        assert!(matches!(classify(parse(1, OK_RESULT).as_ref(), Some(11)), Verdict::Fault(_)));
        let absent = OK_RESULT.replace("case_rc 0", "case_rc absent");
        assert!(matches!(classify(parse(1, &absent).as_ref(), None), Verdict::Fault(_)));
    }

    #[test]
    fn skip_and_no_case_keep_their_own_words() {
        let skip = OK_RESULT.replace("case_rc 0", "case_rc -3");
        assert_eq!(classify(parse(1, &skip).as_ref(), None), Verdict::Skip);
        let none = OK_RESULT.replace("case_rc 0", "case_rc -2");
        assert_eq!(classify(parse(1, &none).as_ref(), None), Verdict::NoCase);
    }
}
