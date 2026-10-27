// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! One rule the test suite has to keep about itself, checked by reading the
//! crate's own source.
//!
//! **A test that mutates a library global must hold the serialising guard.**
//!
//! The library's state is `static`: the context pool, the EGL display and its
//! surfaces, the pending clear, the shader tables. `testutil::TestGl` owns
//! that state for the length of a test -- it takes `SERIAL`, resets every
//! static, and resets them again on the way out. Tests that hold it are
//! therefore alone with the library.
//!
//! A test that touches those statics WITHOUT taking the guard is not alone
//! with anything. It runs on another of the harness's threads, at a moment
//! nothing controls, in the middle of whatever a guarded test is doing. One
//! test did exactly that (`4ab66f0`): it called `egl::reset_for_tests()`,
//! which throws away the EGL display, the surface table, the current
//! bindings and the error word. About one whole-suite run in a hundred it
//! landed inside another EGL test, which then failed on a call that should
//! have succeeded -- in a DIFFERENT test each time, so the suite looked
//! flaky rather than wrong.
//!
//! Nothing in the compiler can express "call this only while holding that
//! mutex", so this module reads the source instead. It is a blunt instrument
//! -- it matches text -- and the second test is what keeps it honest: a scan
//! that cannot fail says nothing when it passes, so its sensitivity is proved
//! on a sample containing exactly one offender.

use std::format;
use std::path::{Path, PathBuf};
use std::string::{String, ToString};
use std::vec::Vec;

/// Calls that mutate a library global. Not exhaustive by construction -- it
/// is the set that has bitten us -- so add to it when something new does.
/* A SECOND family this scan does not cover, recorded because it cost a flake
 * on main and the next person should know the gap is deliberate rather than
 * an oversight.
 *
 * These names are library state a test MUTATES through a named function. The
 * other family is a library DIAGNOSTIC a test reads: `vivante/alloc.rs` kept
 * ten process-wide `Global`s of "what the last allocation measured", and a
 * test calling `allocate_and_build` and then `last_overlap()` could read
 * another thread's numbers in between -- about once in forty runs under load,
 * the same rate as 4ab66f0. No function name appears in the test body, so no
 * list of mutator names can find it.
 *
 * It is fixed there by making the instrument per-thread rather than by taking
 * a guard, which removes the hazard instead of policing it, and is pinned by
 * a threaded test rather than by a flake rate. A new process-global
 * instrument would reintroduce it and this scan would stay silent; prefer a
 * thread-local, or a guard, and say which. */
const MUTATORS: [&str; 4] = ["reset_for_tests(", "osal_set_ops(", "g_backend =", "g_shaderc ="];

/// Ways a test takes the guard. `TestGl::new` is the real one; the rest are
/// this suite's aliases for it (`draw.rs` calls it `Gl`, the backend rigs
/// `Rig`, and several modules wrap it in a local `fresh()`).
const GUARDS: [&str; 4] = ["TestGl::new", "fresh()", "Gl::new", "Rig::new"];

/// The source with comments and string literals blanked to spaces, so that a
/// `{` inside a format string cannot derail brace matching and a mention of
/// `reset_for_tests()` in a comment is not read as a call. Lengths and line
/// breaks are preserved, and the result is still valid UTF-8 because a
/// blanked region is replaced whole.
fn blank_noise(src: &str) -> String {
    let b = src.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0usize;

    fn blank(out: &mut Vec<u8>, c: u8) {
        out.push(if c == b'\n' { b'\n' } else { b' ' });
    }

    while i < b.len() {
        /* a line comment */
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                blank(&mut out, b[i]);
                i += 1;
            }
            continue;
        }
        /* a block comment, which Rust allows to nest */
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            let mut depth = 1usize;
            out.push(b' ');
            out.push(b' ');
            i += 2;
            while i < b.len() && depth > 0 {
                if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
                    depth += 1;
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                } else if b[i] == b'*' && i + 1 < b.len() && b[i + 1] == b'/' {
                    depth -= 1;
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                } else {
                    blank(&mut out, b[i]);
                    i += 1;
                }
            }
            continue;
        }
        /* a raw string (r"..", r#".."#, br#".."#), but not an identifier
         * that happens to end in r or b */
        let ident_before = i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
        if !ident_before && (b[i] == b'r' || (b[i] == b'b' && i + 1 < b.len() && b[i + 1] == b'r')) {
            let mut j = i + if b[i] == b'b' { 2 } else { 1 };
            let start = j;
            while j < b.len() && b[j] == b'#' {
                j += 1;
            }
            let hashes = j - start;
            if j < b.len() && b[j] == b'"' {
                while i <= j {
                    out.push(b[i]);
                    i += 1;
                }
                while i < b.len() {
                    let ends = b[i] == b'"' && b[i + 1..].iter().take(hashes).filter(|c| **c == b'#').count() == hashes;
                    if ends {
                        out.extend(std::iter::repeat_n(b' ', hashes + 1));
                        i += hashes + 1;
                        break;
                    }
                    blank(&mut out, b[i]);
                    i += 1;
                }
                continue;
            }
        }
        /* an ordinary string, escapes and all */
        if b[i] == b'"' {
            out.push(b'"');
            i += 1;
            while i < b.len() {
                if b[i] == b'\\' && i + 1 < b.len() {
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                    continue;
                }
                if b[i] == b'"' {
                    out.push(b'"');
                    i += 1;
                    break;
                }
                blank(&mut out, b[i]);
                i += 1;
            }
            continue;
        }
        /* a char literal, which may hold a brace. A lifetime ('a) is not one:
         * it has no closing quote where a char literal does. */
        if b[i] == b'\'' && i + 2 < b.len() && b[i + 1] != b'\\' && b[i + 2] == b'\'' {
            out.push(b'\'');
            out.push(b' ');
            out.push(b'\'');
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).expect("blanking replaces whole regions, so UTF-8 survives")
}

/// `(name, body)` of every `#[test]` function in already-blanked source.
fn test_bodies(blanked: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let b = blanked.as_bytes();
    let mut from = 0usize;

    while let Some(rel) = blanked[from..].find("#[test]") {
        let at = from + rel;
        from = at + "#[test]".len();
        /* more attributes may sit between #[test] and the fn (#[cfg(..)],
         * #[should_panic], ...): the next `fn` is the one */
        let Some(frel) = blanked[from..].find("fn ") else { break };
        let name_at = from + frel + "fn ".len();
        let name: String = blanked[name_at..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if name.is_empty() {
            continue;
        }
        let Some(orel) = blanked[name_at..].find('{') else { break };
        let open = name_at + orel;
        let mut depth = 0usize;
        let mut i = open;
        while i < b.len() {
            if b[i] == b'{' {
                depth += 1;
            } else if b[i] == b'}' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            i += 1;
        }
        let end = i.min(b.len());
        found.push((name, blanked[open..end].to_string()));
        from = end;
    }
    found
}

fn mutates(body: &str) -> bool {
    MUTATORS.iter().any(|m| body.contains(m))
}

fn takes_guard(body: &str) -> bool {
    GUARDS.iter().any(|g| body.contains(g))
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn no_test_mutates_a_library_global_without_the_guard() {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
    let mut files = Vec::new();
    rs_files(root, &mut files);
    /* a walk that finds nothing would "pass" having read no code at all */
    assert!(files.len() > 10, "only {} source files under {}", files.len(), root.display());

    let mut offenders = Vec::new();
    for f in &files {
        let Ok(src) = std::fs::read_to_string(f) else { continue };
        for (name, body) in test_bodies(&blank_noise(&src)) {
            if mutates(&body) && !takes_guard(&body) {
                offenders.push(format!("{}: {}", f.display(), name));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these tests mutate a library global without taking the guard, so they run \
         concurrently with the tests that own that state (this module's documentation \
         says what that costs, and 4ab66f0 is the one that did):\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn the_scan_finds_the_one_offender_and_only_it() {
    /* The sensitivity proof, and the reason to believe the test above when it
     * passes. The sample holds one guarded call, one unguarded call, and two
     * shapes that must NOT be mistaken for either: a mention in a comment,
     * and a brace inside a string that would derail the brace matching. */
    const SAMPLE: &str = r#"
        #[test]
        fn guarded() {
            let gl = TestGl::new(8, 8);
            crate::gl::egl::reset_for_tests();
        }
        #[test]
        #[cfg(not(feature = "gpu-vivante"))]
        fn unguarded() {
            crate::gl::egl::reset_for_tests();
        }
        #[test]
        fn only_mentions_it() {
            /* reset_for_tests() is what the guarded ones call */
            let s = "reset_for_tests( with a brace { that must not derail us";
            assert!(!s.is_empty());
        }
    "#;

    let bodies = test_bodies(&blank_noise(SAMPLE));
    let names: Vec<&str> = bodies.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["guarded", "unguarded", "only_mentions_it"], "every test body is found, whole");

    let flagged: Vec<&str> =
        bodies.iter().filter(|(_, b)| mutates(b) && !takes_guard(b)).map(|(n, _)| n.as_str()).collect();
    assert_eq!(flagged, ["unguarded"], "exactly the unguarded call is flagged");
}

/* ---- the third family: a test binary that resets the compiler ---------- */

/// Calls that throw away state every `ShcShader` already handed out depends
/// on. `forget_compiled()` resets the compiler's keep arena, and the shader
/// handles are pointers into it.
const RESETTERS: [&str; 1] = ["forget_compiled("];

/// How a file may hold more than one test in spite of that: by serialising
/// them. `online_oracle.rs` takes a process-wide mutex for the length of one
/// compile AND the reads of its result (its `Compiled` guard holds the lock),
/// which is the shape to copy.
const SERIALISERS: [&str; 2] = ["COMPILING.lock", "Compiled {"];

/// An integration-test binary that resets the compiler runs ONE test.
///
/// `cargo` runs the `#[test]` functions of one binary on parallel threads. A
/// file whose helper calls `forget_compiled()` therefore has each test
/// pulling the arena out from under the others' results -- and the symptom is
/// not a crash, it is a number.
///
/// Two files did this on 2026-09-22. `temps_survey.rs` reported "10 sources
/// compile", then 108, then 10, on three runs of identical code, PASSING every
/// time; the true figure is 137. `usc1_triangle.rs` printed "no scratch"
/// instead of its report in five runs of six, also passing. One of them nearly
/// cost a false regression report: a single reading either side of a change
/// read 105 -> 10, which looks exactly like a pass that broke ninety-five
/// shaders and was noise.
///
/// So the rule is structural rather than advisory: one test per binary that
/// resets, or a serialiser that covers the reads as well as the compile.
#[test]
fn a_test_binary_that_resets_the_compiler_holds_one_test() {
    let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests"));
    let mut files = Vec::new();
    rs_files(dir, &mut files);
    assert!(files.len() > 5, "only {} files under {}", files.len(), dir.display());

    let mut offenders = Vec::new();
    let mut resetting_files = 0usize;
    for f in &files {
        let Ok(src) = std::fs::read_to_string(f) else { continue };
        let blanked = blank_noise(&src);
        if !RESETTERS.iter().any(|r| blanked.contains(r)) {
            continue;
        }
        resetting_files += 1;
        if SERIALISERS.iter().any(|g| blanked.contains(g)) {
            continue;
        }
        let n = test_bodies(&blanked).len();
        if n > 1 {
            offenders.push(format!("{}: {n} tests", f.display()));
        }
    }
    /* Vacuity: a scan that found no resetting file at all would pass having
     * checked nothing, and `forget_compiled` being renamed is exactly how
     * that would happen quietly. */
    assert!(resetting_files > 0, "no test file resets the compiler; has `forget_compiled` been renamed?");
    assert!(
        offenders.is_empty(),
        "these test binaries reset the compiler's keep arena and hold more than one          #[test], so their tests race over each other's results (see this function's          documentation for what that cost on 2026-09-22):\n  {}",
        offenders.join("\n  ")
    );
}

/// The sensitivity proof for the scan above.
#[test]
fn the_reset_scan_finds_the_offender_and_spares_the_rest() {
    const OFFENDER: &str = r#"
        fn compile() { backend::forget_compiled(); }
        #[test]
        fn one() { compile(); }
        #[test]
        fn two() { compile(); }
    "#;
    const SERIALISED: &str = r#"
        fn compile() { let _g = COMPILING.lock(); backend::forget_compiled(); }
        #[test]
        fn one() { compile(); }
        #[test]
        fn two() { compile(); }
    "#;
    const SINGLE: &str = r#"
        fn compile() { backend::forget_compiled(); }
        #[test]
        fn only() { compile(); }
    "#;
    const INNOCENT: &str = r#"
        #[test]
        fn one() { assert!(true); }
        #[test]
        fn two() { assert!(true); }
    "#;

    let verdict = |src: &str| {
        let b = blank_noise(src);
        RESETTERS.iter().any(|r| b.contains(r))
            && !SERIALISERS.iter().any(|g| b.contains(g))
            && test_bodies(&b).len() > 1
    };
    assert!(verdict(OFFENDER), "the shape that cost a day is not flagged");
    assert!(!verdict(SERIALISED), "a serialised binary is flagged");
    assert!(!verdict(SINGLE), "a one-test binary is flagged");
    assert!(!verdict(INNOCENT), "a binary that never resets is flagged");
}
