// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The hang guard of `panthor-gltest --sweep`: a hang must leave the board
//! usable, so the child and its whole process group are killed and reaped on
//! the timeout, the parent never blocks on the child's pipes, each verdict
//! line is flushed before the next case starts, and the sweep stops after
//! consecutive HANGs instead of piling work onto a wedged queue.
//!
//! No GPU: `PANTHOR_GLTEST_CHILD` names a shell script that stands in for the
//! `--one <case>` child (it gets the same arguments). Needs sh, sleep, setsid.

use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("gltest-guard-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// The stand-in child: `body` is a sh script, `$2` the case, `$D` the scratch dir.
fn stand_in(d: &Path, body: &str) -> PathBuf {
    let p = d.join("child.sh");
    fs::write(&p, format!("#!/bin/sh\nD='{}'\n{body}\n", d.display())).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn sweep(child: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_panthor-gltest"));
    c.args(args).env("PANTHOR_GLTEST_CHILD", child).stdin(Stdio::null());
    c
}

fn wait_for(path: &Path) -> String {
    let t0 = Instant::now();
    loop {
        if let Ok(s) = fs::read_to_string(path) {
            if !s.trim().is_empty() {
                return s.trim().to_string();
            }
        }
        assert!(t0.elapsed() < Duration::from_secs(20), "no {}", path.display());
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn alive(pid: &str) -> bool {
    // A zombie is not alive for a holder of /dev/dri, but it is not reaped
    // either: /proc/<pid> exists until the parent waits for it.
    Path::new(&format!("/proc/{pid}")).exists()
}

fn gone_within(pid: &str, secs: u64) -> bool {
    let t0 = Instant::now();
    while alive(pid) {
        if t0.elapsed() > Duration::from_secs(secs) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

fn kill(pid: &str) {
    let _ = Command::new("kill").args(["-9", pid]).status();
}

const PASS_CHILD: &str = r#"echo "panthor-gltest: build stand-in"
echo "gl_test: case $2 stub: PASS"
echo "panthor-gltest: result case $2 run_rc 0 case_rc 0 draws_refused 0 work_lost 0 group 0x0/0x0 vm true first_bad -1 got 0x00000000 want 0x00000000"
"#;

#[test]
fn a_timeout_kills_the_whole_group_and_reaps_the_child() {
    let d = scratch("kill");
    // The child starts a grandchild in its group, which keeps both pipes
    // open, and sleeps itself: both must be gone when the verdict is out.
    let child = stand_in(
        &d,
        r#"echo "panthor-gltest: build stand-in"
echo $$ > "$D/child.pid"
sleep 300 &
echo $! > "$D/grandchild.pid"
wait"#,
    );
    let t0 = Instant::now();
    let out = sweep(&child, &["--timeout", "1", "7"]).output().unwrap();
    let took = t0.elapsed();
    let text = String::from_utf8_lossy(&out.stdout);
    let (cpid, gpid) = (wait_for(&d.join("child.pid")), wait_for(&d.join("grandchild.pid")));
    let clean = !alive(&cpid) && gone_within(&gpid, 5);
    kill(&gpid);
    assert!(text.contains("panthor-gltest: verdict case 7 ?: HANG"), "the verdict line, unchanged:\n{text}");
    assert!(!alive(&cpid), "the child {cpid} is a zombie or still alive: it was not reaped");
    assert!(clean, "the grandchild {gpid} of the group outlived the timeout");
    assert!(took < Duration::from_secs(8), "the parent waited {took:?} on a 1 s timeout");
    assert_eq!(out.status.code(), Some(1), "a HANG fails the run");
}

#[test]
fn a_straggler_outside_the_group_does_not_block_the_parent() {
    let d = scratch("pipes");
    // setsid: the grandchild leaves the group, keeps the pipes, and outlives
    // the group kill. The parent must give up on the pipes, keep the output.
    let child = stand_in(
        &d,
        r#"echo "panthor-gltest: build stand-in"
echo "gl_test: case $2 stub: starting"
setsid sleep 300 &
echo $! > "$D/straggler.pid"
sleep 300"#,
    );
    let t0 = Instant::now();
    let out = sweep(&child, &["--timeout", "1", "7"]).output().unwrap();
    let took = t0.elapsed();
    kill(&wait_for(&d.join("straggler.pid")));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(took < Duration::from_secs(15), "the parent blocked {took:?} on a straggler's pipe");
    assert!(text.contains("gl_test: case 7 stub: starting"), "the output so far is kept:\n{text}");
    assert!(text.contains("verdict case 7 stub: HANG"), "the verdict names the case and says HANG:\n{text}");
    assert!(text.contains("an output pipe stayed open"), "the abandoned pipe is said:\n{text}");
}

#[test]
fn the_verdict_line_is_out_before_the_next_case_starts() {
    let d = scratch("flush");
    // Case 1 hangs; case 2 leaves a marker the moment it starts. The reader
    // sees case 1's verdict line while the sweep runs, and the marker is not
    // there yet: the line was flushed first, not at exit.
    let child = stand_in(
        &d,
        &format!(
            r#"echo "panthor-gltest: build stand-in"
if [ "$2" = 1 ]; then exec sleep 300; fi
: > "$D/case$2.started"
{PASS_CHILD}"#
        ),
    );
    let mut sw = sweep(&child, &["--timeout", "1", "1", "2"]).stdout(Stdio::piped()).spawn().unwrap();
    let mut lines = BufReader::new(sw.stdout.take().unwrap()).lines();
    let mut seen = false;
    for l in lines.by_ref() {
        let l = l.unwrap();
        if l.starts_with("panthor-gltest: verdict case 1 ") {
            assert!(l.contains(": HANG"), "{l}");
            assert!(!d.join("case2.started").exists(), "case 2 started before case 1's verdict was out");
            seen = true;
            break;
        }
    }
    let rest: Vec<String> = lines.map(Result::unwrap).collect();
    let _ = sw.wait();
    assert!(seen, "no verdict line for case 1");
    assert!(rest.iter().any(|l| l.starts_with("panthor-gltest: verdict case 2 stub: PASS")), "{rest:?}");
}

#[test]
fn the_sweep_stops_after_consecutive_hangs_and_says_why() {
    let d = scratch("stop");
    let child = stand_in(&d, r#"echo "panthor-gltest: build stand-in"; : > "$D/case$2.started"; exec sleep 300"#);
    let out = sweep(&child, &["--timeout", "1", "--max-wedged", "2", "1", "2", "3", "4"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(d.join("case1.started").exists() && d.join("case2.started").exists());
    assert!(!d.join("case3.started").exists(), "case 3 ran on a queue that hung twice:\n{text}");
    assert!(
        text.contains("panthor-gltest: STOPPING after case 2: 2 consecutive HANG/FAULT verdicts (--max-wedged 2)"),
        "{text}"
    );
    assert!(text.contains("2 case(s) not run"), "{text}");
    assert!(
        text.contains("panthor-gltest: table") && text.contains("| 2 | ? | HANG |"),
        "the table still prints:\n{text}"
    );
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn a_pass_between_hangs_resets_the_count() {
    let d = scratch("reset");
    let child = stand_in(
        &d,
        &format!(
            r#"echo "panthor-gltest: build stand-in"
if [ "$2" = 2 ] || [ "$2" = 4 ]; then
{PASS_CHILD}
exit 0
fi
exec sleep 300"#
        ),
    );
    let out = sweep(&child, &["--timeout", "1", "--max-wedged", "2", "1", "2", "3", "4"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(!text.contains("STOPPING"), "{text}");
    assert!(text.contains("4 case(s) run: HANG 2, PASS 2"), "{text}");
}

#[test]
fn a_child_that_exits_normally_keeps_the_verdict_format() {
    let d = scratch("pass");
    let child = stand_in(&d, PASS_CHILD);
    let out = sweep(&child, &["3"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("panthor-gltest: verdict case 3 stub: PASS \n"), "{text}");
    assert!(text.contains("panthor-gltest: 1 case(s) run: PASS 1"), "{text}");
    assert_eq!(out.status.code(), Some(0));
}
