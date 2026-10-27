// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! run-on-board.sh holds the board lock the kexec recipe takes
//! (driver/tools/kexec-zephyr/board_lock.sh), both ways: a GL run refuses
//! while another holds the board, and a second taker (the recipe sources
//! the file the same way) refuses while a GL run holds it. Each refusal
//! names the holder. On 2026-10-19 a jump rebooted the T6 between two
//! gl_test runs: the board had been handed over by chat only.
//!
//! No board and no build: the lock file is a temporary one (BOARD_LOCK),
//! and a stub `cargo` first on PATH stands for the build the script runs
//! while it holds the lock. Needs bash and flock(1): board_lock.sh names
//! the holder from the record the holder writes into the lock file, and
//! nothing here runs lsof (CI runner A has none).

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("board-lock-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn tools_present() {
    for t in ["bash", "flock"] {
        let ok = Command::new("sh").args(["-c", &format!("command -v {t}")]).status().is_ok_and(|s| s.success());
        assert!(ok, "{t} is missing: board_lock.sh needs it; the lock cannot be tested");
    }
}

/// Wait until `path` exists (a marker the other process writes once it
/// holds the lock).
fn wait_for(path: &Path, what: &str) {
    let t0 = Instant::now();
    while !path.exists() {
        assert!(t0.elapsed() < Duration::from_secs(30), "{what}: no marker {} in 30 s", path.display());
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A process that holds the lock as the kexec recipe does (sources
/// board_lock.sh), then waits.
fn holder(lock: &Path, marker: &Path) -> Child {
    let script = format!(
        ". {}/driver/tools/kexec-zephyr/board_lock.sh && : > {} && sleep 60",
        root().display(),
        marker.display()
    );
    Command::new("bash")
        .args(["-c", &script])
        .env("BOARD_LOCK", lock)
        .env("LOCK_WAIT", "0")
        .env("HOLD_MAX", "120")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn stop(mut c: Child) {
    let _ = c.kill();
    let _ = c.wait();
}

/// Kill every process that still has `lock` open (a holder's `sleep`
/// inherits the descriptor), found by its descriptors in /proc and killed
/// by its exact pid, so the next test starts from nothing.
fn release(lock: &Path) {
    let Ok(lock) = fs::canonicalize(lock) else { return };
    let Ok(procs) = fs::read_dir("/proc") else { return };
    for p in procs.flatten() {
        let name = p.file_name();
        let Some(pid) = name.to_str().filter(|n| n.bytes().all(|b| b.is_ascii_digit())) else { continue };
        let Ok(fds) = fs::read_dir(p.path().join("fd")) else { continue };
        if fds.flatten().any(|fd| fs::read_link(fd.path()).is_ok_and(|t| t == lock)) {
            let _ = Command::new("kill").arg(pid).status();
        }
    }
}

#[test]
fn a_gl_run_refuses_while_another_holds_the_board_and_names_it() {
    tools_present();
    let d = scratch("gl-refuses");
    let lock = d.join("board.lock");
    let held = d.join("held");
    let h = holder(&lock, &held);
    wait_for(&held, "the holder");
    let hpid = h.id();

    /* the stub cargo would say so if the script ever got past the lock */
    let bin = d.join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("cargo"), "#!/bin/sh\necho 'stub cargo: run-on-board.sh got past the lock' >&2\nexit 99\n")
        .unwrap();
    Command::new("chmod").arg("+x").arg(bin.join("cargo")).status().unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    let out = Command::new(root().join("tools/mali/panthor/run-on-board.sh"))
        .arg("panthor-hello")
        .env("BOARD_HOST", "root@board-lock-test")
        .env("BOARD_LOCK", &lock)
        .env("PATH", &path)
        .env_remove("LOCK_WAIT")
        .env_remove("HOLD_MAX")
        .output()
        .unwrap();
    stop(h);
    release(&lock);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "run-on-board.sh must refuse a held board (exit 1); stderr:\n{err}");
    assert!(err.contains("board_lock: busy"), "the refusal is board_lock.sh's; stderr:\n{err}");
    assert!(err.contains(&format!("pid {hpid} ")), "the refusal names the holder, pid {hpid}; stderr:\n{err}");
    assert!(!err.contains("stub cargo"), "the script went past the lock; stderr:\n{err}");
}

#[test]
fn a_jump_refuses_while_a_gl_run_holds_the_board_and_names_it() {
    tools_present();
    let d = scratch("jump-refuses");
    let lock = d.join("board.lock");
    let building = d.join("building");
    /* the stub cargo: run-on-board.sh holds the lock while it builds, so a
     * build that waits is a GL session holding the board */
    let bin = d.join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("cargo"), format!("#!/bin/sh\n: > {}\nexec sleep 60\n", building.display())).unwrap();
    Command::new("chmod").arg("+x").arg(bin.join("cargo")).status().unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    let mut gl = Command::new(root().join("tools/mali/panthor/run-on-board.sh"))
        .arg("panthor-gltest")
        .env("BOARD_HOST", "root@board-lock-test")
        .env("BOARD_LOCK", &lock)
        .env("PATH", &path)
        .env_remove("LOCK_WAIT")
        .env_remove("HOLD_MAX")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let glpid = gl.id();
    wait_for(&building, "run-on-board.sh");
    let line = BufReader::new(gl.stdout.take().unwrap()).lines().next().and_then(Result::ok).unwrap_or_default();

    let second = d.join("second");
    let jump = holder(&lock, &second);
    let out = jump.wait_with_output().unwrap();
    stop(gl);
    release(&lock);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        line.starts_with("run-on-board.sh: holding the board lock "),
        "run-on-board.sh says it holds the lock: {line:?}"
    );
    assert_eq!(out.status.code(), Some(1), "the second taker must refuse (exit 1); stderr:\n{err}");
    assert!(!second.exists(), "the second taker went past the lock");
    assert!(err.contains(&format!("pid {glpid} ")), "the refusal names run-on-board.sh, pid {glpid}; stderr:\n{err}");
    /* board_lock_holder prints the holder's command cut at 100 characters,
     * which an absolute path can fill before the script's name: the pid is
     * the identifier, and run-on-board.sh prints its own next to the lock */
    assert!(line.ends_with(&format!("(pid {glpid})")), "run-on-board.sh prints its pid with the lock: {line:?}");
}
