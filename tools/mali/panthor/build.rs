// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Stamps the binaries with `git describe --always --dirty` (CLAUDE.md:
//! anything that produces a row prints its own build first), and compiles
//! gl_test.c for `panthor-gltest`.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// gl_test.c, the GL suite every board runs, as the Mali arm
/// (driver/docs/nanopc-t6-mali.md, "L3: gl_test under Armbian"): the
/// hosted printf arm (`GL_TEST_USER`), the GLSL ES 1.00 shader set (the
/// in-process compiler takes no `#version 450`), every warning an error as
/// in the gate's other arms. `panthor-gltest` links it; its GL and OSAL
/// symbols are the `yakogl` crate's.
///
/// The compiler is run here rather than through the `cc` crate: a new
/// dependency changes Cargo.lock, and a Cargo.lock change selects every
/// row of tools/check.sh for what is a Mali tool.
///
/// The compiler: for the host, `$CC` or `cc`. For another target, the one
/// named by `CC_<target>`, `TARGET_CC` or `CARGO_TARGET_<TARGET>_LINKER`
/// (`run-on-board.sh` sets the last: aarch64-linux-gnu-gcc). With none the
/// file is not compiled: the crate is then only checked or linted for that
/// target (the gate's aarch64 clippy row; the runners carry no cross gcc),
/// and no link can happen there anyway, because the default linker cannot
/// link the target. The host build compiles the same source with the same
/// flags, so the gate compiles the Mali arm on every run of the crate.
fn gl_test() {
    let env = |k: &str| {
        println!("cargo:rerun-if-env-changed={k}");
        std::env::var(k).ok().filter(|v| !v.is_empty())
    };
    let target = std::env::var("TARGET").unwrap_or_default();
    let host = std::env::var("HOST").unwrap_or_default();
    let compiler = if target == host {
        Some(env("CC").unwrap_or_else(|| "cc".into()))
    } else {
        env(&format!("CC_{}", target.replace('-', "_")))
            .or_else(|| env("TARGET_CC"))
            .or_else(|| env(&format!("CARGO_TARGET_{}_LINKER", target.to_uppercase().replace('-', "_"))))
    };
    let Some(compiler) = compiler else {
        return;
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let pvrlab = root.join("driver/tools/pvrlab");
    let src = pvrlab.join("gl_test.c");
    for dep in [
        src.clone(),
        pvrlab.join("gl_test_shaders_es100.h"),
        root.join("include"),
        root.join("src/gl_internal.h"),
        root.join("src/backend.h"),
        root.join("src/osal/osal_internal.h"),
    ] {
        println!("cargo:rerun-if-changed={}", dep.display());
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let obj = out.join("gl_test.o");
    let opt = if std::env::var("PROFILE").as_deref() == Ok("release") { "-O2" } else { "-O0" };
    let mut cc = Command::new(&compiler);
    cc.args([
        "-std=gnu11",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-g",
        opt,
        "-fPIC",
        "-ffunction-sections",
        "-fdata-sections",
    ])
    .args(["-DGL_TEST_USER=1", "-DYAKOGL_GPU_MALI=1", "-DGL_TEST_ES100=1"]);
    for inc in [root.join("include"), root.join("src"), root.join("src/osal"), pvrlab.clone(), root.join("driver/src")]
    {
        cc.arg("-I").arg(inc);
    }
    cc.arg("-c").arg(&src).arg("-o").arg(&obj);
    run(cc, "compile gl_test.c");
    let ar = if target == host {
        env("AR").unwrap_or_else(|| "ar".into())
    } else {
        /* the cross gcc's own ar, beside it: aarch64-linux-gnu-gcc -> -ar */
        compiler.strip_suffix("gcc").map_or_else(|| "ar".into(), |p| format!("{p}ar"))
    };
    let lib = out.join("libyakogl_gl_test.a");
    let _ = std::fs::remove_file(&lib);
    let mut a = Command::new(&ar);
    a.arg("crs").arg(&lib).arg(&obj);
    run(a, "archive gl_test.o");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=yakogl_gl_test");
    /* panthor-gltest reads a refused compile's or link's whole message
     * after gl_test's own call (its __wrap_ functions); the calls inside the
     * library are not gl_test.o's and stay direct */
    for f in ["glCompileShader", "glLinkProgram"] {
        println!("cargo:rustc-link-arg-bin=panthor-gltest=-Wl,--wrap={f}");
    }
}

/// Run a build step; its failure stops the build with its output.
fn run(mut c: Command, what: &str) {
    match c.output() {
        Ok(o) if o.status.success() => {}
        Ok(o) => panic!(
            "{what}: {c:?} failed ({}):\n{}{}",
            o.status,
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
        Err(e) => panic!("{what}: {c:?} could not run: {e}"),
    }
}

fn main() {
    gl_test();
    let build = git(&["describe", "--always", "--dirty"]).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=YAKOGL_MALI_PANTHOR_BUILD={build}");
    // Run again when HEAD moves or the branch it names moves: the stamp must
    // name the commit. Not the index: `git describe --dirty` (above) and every
    // `git status` of a gate row rewrite it when a checkout's stat data is
    // stale, so watching it reran this script between `cargo test --no-run`
    // and `cargo nextest run`, which then printed "Compiling" into the test
    // log. A staged edit is a source change, which `src` below covers.
    // A path that does not exist is never watched: cargo reruns the script on
    // every build for a missing one (packed-refs is absent in a repository
    // with loose refs only).
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
    // The binaries link the library (the frame builder): a library edit must
    // restamp them too.
    println!("cargo:rerun-if-changed=../../../src");
}
