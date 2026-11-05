// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The redaction scan's self-test (tools/release/redaction-scan.py
//! --self-test): its classes, the masking, the allow rules, the "not run"
//! private classes and the exit codes, on a throwaway fixture repository.
//! Release checklist line 8 runs the scan itself on the snapshot commit.

use std::process::Command;

#[test]
fn redaction_scan_self_test() {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/release/redaction-scan.py");
    let out = Command::new("python3")
        .args(["-I", script, "--self-test"])
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
