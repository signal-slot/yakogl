// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Stamps the library with `git describe --always --dirty` of the tree it was
//! built from: the `build=` of its `yakogl-mali: loaded` line.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    let build = git(&["describe", "--always", "--dirty"]).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=YAKOGL_MALI_GLES_BUILD={build}");
    // As tools/mali/panthor/build.rs: run again when HEAD or the branch it
    // names moves (not the index, which every `git status` rewrites); a path
    // that does not exist is never watched (cargo would rerun every build).
    let mut watch = vec!["HEAD".to_string(), "packed-refs".to_string()];
    if let Some(r) = git(&["symbolic-ref", "-q", "HEAD"]) {
        watch.push(r);
    }
    for p in watch {
        if let Some(path) = git(&["rev-parse", "--git-path", &p]) {
            if std::path::Path::new(&path).exists() {
                println!("cargo:rerun-if-changed={path}");
            }
        }
    }
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../panthor/src");
    println!("cargo:rerun-if-changed=../../../src");
}
