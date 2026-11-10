// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The redaction scan's self-test (tools/release/redaction-scan.py
//! --self-test): its classes, the masking, the allow rules, the "not run"
//! private classes and the exit codes, on a throwaway fixture repository.
//! Release checklist line 8 runs the scan itself on the snapshot commit.
//!
//! The scan also runs from here on a commit of a throwaway repository, so a
//! scan that stops seeing a home path fails the test. This file is in the
//! scanned tree: it builds every fixture value at run time and never holds
//! one literally.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/release/redaction-scan.py");

/// redaction-scan.py's exit codes: an H/M finding, and no finding with the
/// private classes not run (the clean result of a public-class scan).
const EXIT_FINDING: i32 = 1;
const EXIT_NOT_RUN: i32 = 3;

#[test]
fn redaction_scan_self_test() {
    let out = Command::new("python3")
        .args(["-I", SCRIPT, "--self-test"])
        .env_remove("YAKOGL_REDACTION_PRIVATE")
        .env_remove("YAKOGL_REDACTION_PRIVATE_FILE")
        .output()
        .expect("python3 runs");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "redaction-scan.py --self-test failed:\n{text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("self-test passed"), "{text}");
}

/// One scan's result: the exit code, the summary line's H/M count and the
/// whole stdout (the scan masks every hit in it).
struct Scan {
    code: i32,
    hm: usize,
    stdout: String,
}

impl Scan {
    /// The finding count on a class's line, e.g.
    /// `[M] home-path      Home path: 1 finding(s)`; 0 for `ok`.
    fn class_findings(&self, class: &str) -> usize {
        let line = self
            .stdout
            .lines()
            .find(|l| l.split_whitespace().nth(1) == Some(class))
            .unwrap_or_else(|| panic!("no {class} line in:\n{}", self.stdout));
        let state = line.split_once(": ").map_or("", |(_, s)| s);
        let mut words = state.split_whitespace();
        match (words.next(), words.next()) {
            (Some(n), Some(w)) if w.starts_with("finding(s)") => {
                n.parse().unwrap_or_else(|_| panic!("unparsable class line: {line}"))
            }
            _ => 0,
        }
    }
}

/// Run the scan's public classes on `rev` of the repository at `repo` (the
/// scan finds the repository from its working directory). The private
/// values (YAKOGL_REDACTION_PRIVATE, _FILE) are removed from the child's
/// environment, so the private classes report "not run".
fn scan(repo: &Path, rev: &str) -> Scan {
    let out = Command::new("python3")
        .args(["-I", SCRIPT, rev])
        .current_dir(repo)
        .env_remove("YAKOGL_REDACTION_PRIVATE")
        .env_remove("YAKOGL_REDACTION_PRIVATE_FILE")
        .output()
        .unwrap_or_else(|e| panic!("python3 must be installed to run redaction-scan.py: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let summary = stdout.lines().rev().find_map(|l| l.strip_prefix("summary: ")).unwrap_or_else(|| {
        panic!("no summary line from redaction-scan.py {rev}:\n{stdout}\n{}", String::from_utf8_lossy(&out.stderr))
    });
    let hm = summary
        .split_once(" H/M finding(s)")
        .and_then(|(n, _)| n.parse().ok())
        .unwrap_or_else(|| panic!("unparsable summary line: {summary}"));
    Scan { code: out.status.code().expect("redaction-scan.py exited by a signal"), hm, stdout }
}

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git must be installed for this test: {e}"));
    assert!(out.status.success(), "git {args:?} failed:\n{}", String::from_utf8_lossy(&out.stderr));
}

/// A fresh git repository under CARGO_TARGET_TMPDIR, unique per run and
/// call, with a local example.invalid identity and one commit holding `file`.
fn fixture_repo(tag: &str, file: &str, text: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "redaction-scan-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the fixture directory");
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.name", "Fixture"]);
    let email = format!("{}@{}", "fixture", "example.invalid");
    git(&dir, &["config", "user.email", &email]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    std::fs::write(dir.join(file), text).expect("write the fixture file");
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "fixture"]);
    dir
}

#[test]
fn redaction_scan_reports_a_committed_home_path() {
    // Built at run time, so this source file holds no home path.
    let home = format!("/{}/{}/x", "home", "fixture");
    let repo = fixture_repo("home", "notes.txt", &format!("see {home}\n"));
    let s = scan(&repo, "HEAD");
    assert_eq!(s.code, EXIT_FINDING, "exit code:\n{}", s.stdout);
    assert!(s.hm >= 1, "no H/M finding:\n{}", s.stdout);
    assert!(s.class_findings("home-path") >= 1, "no home-path finding:\n{}", s.stdout);
    assert!(s.stdout.contains("notes.txt:1 "), "the hit names no file:\n{}", s.stdout);
    assert!(!s.stdout.contains(&home), "the hit is not masked:\n{}", s.stdout);
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn redaction_scan_passes_a_clean_commit() {
    let repo = fixture_repo("clean", "notes.txt", "nothing private here\n");
    let s = scan(&repo, "HEAD");
    // Without private values a clean tree exits 3, never 0 (not run, not passed).
    assert_eq!(s.code, EXIT_NOT_RUN, "exit code:\n{}", s.stdout);
    assert_eq!(s.hm, 0, "{}", s.stdout);
    assert_eq!(s.class_findings("home-path"), 0, "{}", s.stdout);
    let _ = std::fs::remove_dir_all(&repo);
}

/// The repository itself, public classes only: a home path, address, login or
/// e-mail added to any tracked file fails here before it reaches the audit.
/// The failure names each finding's file:line and class, never the hit.
#[test]
fn redaction_scan_finds_nothing_in_this_tree() {
    let s = scan(Path::new(env!("CARGO_MANIFEST_DIR")), "HEAD");
    // 3 is the clean result without private values; 0 when they are set.
    assert!(s.code == 0 || s.code == EXIT_NOT_RUN || s.hm > 0, "exit code {}:\n{}", s.code, s.stdout);
    if s.hm > 0 {
        let mut class = "";
        let mut found = Vec::new();
        for l in s.stdout.lines() {
            if l.starts_with('[') {
                // Only the H and M classes' hit lines are findings.
                class = match l.get(..4) {
                    Some("[H] " | "[M] ") => l[4..].split_whitespace().next().unwrap_or(""),
                    _ => "",
                };
            } else if !class.is_empty() && l.starts_with("      ") && !l.trim_start().starts_with("info ") {
                found.push(format!("{} {class}", l.split_whitespace().next().unwrap_or("")));
            }
        }
        panic!("{} H/M finding(s) in the tree:\n{}", s.hm, found.join("\n"));
    }
    assert_eq!(s.hm, 0, "{}", s.stdout);
}
