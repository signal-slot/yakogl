// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014 The Android Open Source Project
// SPDX-FileCopyrightText: The Khronos Group Inc.
//! How much of GLSL ES 3.00 the online shader compiler takes, measured over
//! the dEQP `data/gles3/shaders/*.test` corpus (docs/es3/shaderc-coverage.md).
//!
//! Two parts:
//!
//! * `inline_cases` always runs. A few short cases in the `.test` format,
//!   copied from VK-GL-CTS (Apache-2.0, Copyright (c) 2014 The Android Open
//!   Source Project and The Khronos Group Inc., `data/gles3/shaders/`), go
//!   through the same parser, template expansion and compile/link path as the
//!   corpus and must come out as the case says.
//! * `corpus` runs only when `DEQP_GLES3_TEST_DIR` names the directory of the
//!   `.test` files (a VK-GL-CTS checkout's `data/gles3/shaders`). Without it
//!   the test prints that it skipped. It measures, per file: expected-pass
//!   compiled, expected-pass refused (the first error), expected-fail
//!   correctly refused, expected-fail wrongly accepted. It asserts nothing
//!   about the numbers (they move as the compiler grows); it prints the table
//!   and, with `DEQP_GLES3_REPORT=<file>`, writes one tab-separated line per
//!   unit that is not an expected-pass success (file, unit, expectation,
//!   verdict, first line of the log).
//!
//! The case format and the `${...}` expansion follow dEQP's
//! `glsShaderLibrary` (framework/opengl/gluShaderLibrary.cpp and
//! modules/glshared/glsShaderLibraryCase.cpp): a `both` case is two programs
//! (`_vertex`, `_fragment`), a `values` block supplies the inputs and outputs
//! the generated stage and the comparison code are written from, and a case
//! with `vertex` and `fragment` is one program. The unit counted is one
//! program, i.e. one dEQP test case. Each is compiled stage by stage and
//! linked through `SHC_ONLINE`, which is what glCompileShader and
//! glLinkProgram reach.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_shaderc_corpus
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_shaderc_corpus
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_shaderc_corpus
//! DEQP_GLES3_TEST_DIR=<VK-GL-CTS>/data/gles3/shaders \
//!     cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online \
//!     --test es3_shaderc_corpus -- --nocapture
//! ```
//!
//! GLSL ES 3.00 is refused by a `report-es2` build (pp.rs out_of_scope), so
//! the file is empty there.
#![cfg(all(
    feature = "shaderc-online",
    not(feature = "report-es2"),
    any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d")
))]
use core::ffi::c_char;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Mutex;
#[path = "deqp_shader_library/mod.rs"]
mod deqp_shader_library;
use deqp_shader_library::{parse_file, programs, Case, Expect};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The compiler keeps its output in one arena that every compile resets.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

// ---- the front end under test ---------------------------------------------

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err(text_of(&log))
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    Linked,
    CompileFail(String),
    LinkFail(String),
}

/// glCompileShader on both stages, then glLinkProgram.
fn build(vs: &str, fs: &str) -> Outcome {
    yakogl::shaderc::online::backend::forget_compiled();
    let vso = match compile(SHC_STAGE_VERTEX, vs) {
        Ok(o) => o,
        Err(m) => return Outcome::CompileFail(format!("vertex: {m}")),
    };
    let mut fso = match compile(SHC_STAGE_FRAGMENT, fs) {
        Ok(o) => o,
        Err(m) => return Outcome::CompileFail(format!("fragment: {m}")),
    };
    let Some(link) = yakogl::shaderc::online::SHC_ONLINE.link else {
        return Outcome::Linked;
    };
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Outcome::Linked
    } else {
        Outcome::LinkFail(text_of(&log))
    }
}

// The .test reader and the source generator are in deqp_shader_library/mod.rs.

// ---- running and scoring ----------------------------------------------------

#[derive(Debug, PartialEq, Clone, Copy)]
enum Verdict {
    /// expected to build, built
    PassCompiled,
    /// expected to build, refused
    PassRefused,
    /// expected to fail, refused as expected
    FailRefused,
    /// expected to fail, built (or failed at the wrong step)
    FailAccepted,
    /// the harness cannot form the program
    Skipped,
}

fn judge(expect: Expect, o: &Outcome) -> Verdict {
    match (expect, o) {
        (Expect::Pass, Outcome::Linked) => Verdict::PassCompiled,
        (Expect::Pass, _) => Verdict::PassRefused,
        (Expect::CompileFail, Outcome::CompileFail(_)) => Verdict::FailRefused,
        (Expect::LinkFail, Outcome::LinkFail(_)) => Verdict::FailRefused,
        (Expect::CompileOrLinkFail, Outcome::CompileFail(_) | Outcome::LinkFail(_)) => Verdict::FailRefused,
        _ => Verdict::FailAccepted,
    }
}

struct Unit {
    name: String,
    expect: Expect,
    verdict: Verdict,
    detail: String,
}

fn run_case(case: &Case) -> Vec<Unit> {
    programs(case)
        .into_iter()
        .map(|(suffix, srcs)| {
            let name = format!("{}{suffix}", case.name);
            let (verdict, detail) = match srcs {
                Err(why) => (Verdict::Skipped, why),
                Ok((vs, fs)) => {
                    let outcome = match std::panic::catch_unwind(|| build(&vs, &fs)) {
                        Ok(o) => o,
                        Err(_) => Outcome::CompileFail("PANIC in the compiler".into()),
                    };
                    let detail = match &outcome {
                        Outcome::Linked => "built".to_string(),
                        Outcome::CompileFail(m) => format!("compile: {m}"),
                        Outcome::LinkFail(m) => format!("link: {m}"),
                    };
                    (judge(case.expect, &outcome), detail)
                }
            };
            Unit { name, expect: case.expect, verdict, detail }
        })
        .collect()
}

// ---- part (a): inline cases --------------------------------------------------

/// Cases from VK-GL-CTS data/gles3/shaders (Apache-2.0, Copyright (c) 2014
/// The Android Open Source Project, The Khronos Group Inc.): declarations.test
/// `in_in_vertex_main` and `switch.test` `condition_float` (both compile_fail),
/// and a values-driven `both` case in the shape of conversions.test
/// `int_to_float` (pass).
const INLINE_CASES: &str = r#"
group inline "Inline cases"
    case in_in_vertex_main
        version 300 es
        expect compile_fail
        vertex ""
            #version 300 es
            ${VERTEX_DECLARATIONS}
            void main()
            {
                in mediump float val;
                gl_Position = vec4(1.0);
            }
        ""
        fragment ""
            #version 300 es
            ${FRAGMENT_DECLARATIONS}
            void main()
            {
                ${FRAG_COLOR} = vec4(1.0);
            }
        ""
    end

    case condition_float
        version 300 es
        expect compile_fail
        both ""
            #version 300 es
            precision mediump float;
            ${DECLARATIONS}

            void main ()
            {
                float c = 1.0;
                switch (c)
                {
                    case 1:
                        ${POSITION_FRAG_COLOR} = vec4(1.0);
                }

                ${OUTPUT}
            }
        ""
    end

    case int_to_float
        version 300 es
        values
        {
            input int in0 = [ 0 | 1 | 2 | 5 | 8 | 11 | -12 | -66 | -192 | 255 ];
            output float out0 = [ 0.0 | 1.0 | 2.0 | 5.0 | 8.0 | 11.0 | -12.0 | -66.0 | -192.0 | 255.0 ];
        }

        both ""
            #version 300 es
            precision mediump float;
            precision mediump int;

            ${DECLARATIONS}

            void main()
            {
                ${SETUP}
                out0 = float(in0);
                ${OUTPUT}
            }
        ""
    end
end
"#;

#[test]
fn inline_cases() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let cases = parse_file(INLINE_CASES);
    assert_eq!(cases.len(), 3);
    assert_eq!(cases[0].name, "inline.in_in_vertex_main");
    assert_eq!(cases[2].values.inputs, [("int".to_string(), "in0".to_string())]);
    let mut units = Vec::new();
    for c in &cases {
        units.extend(run_case(c));
    }
    // two programs from each `both` case, one from the complete case
    assert_eq!(units.len(), 5);
    for u in &units {
        assert!(
            matches!(u.verdict, Verdict::PassCompiled | Verdict::FailRefused),
            "{} (expect {:?}): {:?}: {}",
            u.name,
            u.expect,
            u.verdict,
            u.detail
        );
    }
}

/// gl_FragDepth, in the shapes of dEQP-GLES3 shaders.fragdepth.write (a
/// constant, a varying, a uniform, a conditional, a loop) and .compare (the
/// written value combined with gl_FragCoord.z), plus a discard before the
/// write. Authored here (not copied); each is a complete pass case.
const FRAGDEPTH_CASES: &str = r#"
group fragdepth "gl_FragDepth"
    case write_constant
        version 300 es
        expect pass
        vertex ""
            #version 300 es
            in highp vec4 a_position;
            in highp float a_depth;
            out highp float v_d;
            void main()
            {
                gl_Position = a_position;
                v_d = a_depth;
            }
        ""
        fragment ""
            #version 300 es
            precision mediump float;
            in highp float v_d;
            layout(location = 0) out mediump vec4 o_color;
            uniform highp float u_depth;
            void main()
            {
                o_color = vec4(1.0, 0.0, 0.0, 1.0);
                gl_FragDepth = 0.5;
            }
        ""
    end

    case write_varying
        version 300 es
        expect pass
        vertex ""
            #version 300 es
            in highp vec4 a_position;
            in highp float a_depth;
            out highp float v_d;
            void main()
            {
                gl_Position = a_position;
                v_d = a_depth;
            }
        ""
        fragment ""
            #version 300 es
            precision mediump float;
            in highp float v_d;
            layout(location = 0) out mediump vec4 o_color;
            uniform highp float u_depth;
            void main()
            {
                o_color = vec4(0.0, 1.0, 0.0, 1.0);
                gl_FragDepth = v_d;
            }
        ""
    end

    case write_uniform
        version 300 es
        expect pass
        vertex ""
            #version 300 es
            in highp vec4 a_position;
            in highp float a_depth;
            out highp float v_d;
            void main()
            {
                gl_Position = a_position;
                v_d = a_depth;
            }
        ""
        fragment ""
            #version 300 es
            precision mediump float;
            in highp float v_d;
            layout(location = 0) out mediump vec4 o_color;
            uniform highp float u_depth;
            void main()
            {
                o_color = vec4(0.0, 0.0, 1.0, 1.0);
                gl_FragDepth = u_depth;
            }
        ""
    end

    case write_conditional
        version 300 es
        expect pass
        vertex ""
            #version 300 es
            in highp vec4 a_position;
            in highp float a_depth;
            out highp float v_d;
            void main()
            {
                gl_Position = a_position;
                v_d = a_depth;
            }
        ""
        fragment ""
            #version 300 es
            precision mediump float;
            in highp float v_d;
            layout(location = 0) out mediump vec4 o_color;
            uniform highp float u_depth;
            void main()
            {
                o_color = vec4(1.0);
                if (v_d > 0.5)
                    gl_FragDepth = v_d;
                else
                    gl_FragDepth = 1.0 - v_d;
            }
        ""
    end

    case write_conditional_partial
        version 300 es
        expect pass
        vertex ""
            #version 300 es
            in highp vec4 a_position;
            in highp float a_depth;
            out highp float v_d;
            void main()
            {
                gl_Position = a_position;
                v_d = a_depth;
            }
        ""
        fragment ""
            #version 300 es
            precision mediump float;
            in highp float v_d;
            layout(location = 0) out mediump vec4 o_color;
            uniform highp float u_depth;
            void main()
            {
                o_color = vec4(1.0);
                gl_FragDepth = gl_FragCoord.z;
                if (v_d > 0.5)
                    gl_FragDepth = u_depth;
            }
        ""
    end

    case write_loop
        version 300 es
        expect pass
        vertex ""
            #version 300 es
            in highp vec4 a_position;
            in highp float a_depth;
            out highp float v_d;
            void main()
            {
                gl_Position = a_position;
                v_d = a_depth;
            }
        ""
        fragment ""
            #version 300 es
            precision mediump float;
            in highp float v_d;
            layout(location = 0) out mediump vec4 o_color;
            uniform highp float u_depth;
            void main()
            {
                highp float d = 0.0;
                for (int i = 0; i < 4; i++)
                {
                    d += v_d * 0.25;
                    gl_FragDepth = d;
                }
                o_color = vec4(d);
            }
        ""
    end

    case write_discard
        version 300 es
        expect pass
        vertex ""
            #version 300 es
            in highp vec4 a_position;
            in highp float a_depth;
            out highp float v_d;
            void main()
            {
                gl_Position = a_position;
                v_d = a_depth;
            }
        ""
        fragment ""
            #version 300 es
            precision mediump float;
            in highp float v_d;
            layout(location = 0) out mediump vec4 o_color;
            uniform highp float u_depth;
            void main()
            {
                o_color = vec4(1.0);
                if (v_d < 0.25)
                    discard;
                gl_FragDepth = v_d;
            }
        ""
    end

    case compare_frag_coord
        version 300 es
        expect pass
        vertex ""
            #version 300 es
            in highp vec4 a_position;
            in highp float a_depth;
            out highp float v_d;
            void main()
            {
                gl_Position = a_position;
                v_d = a_depth;
            }
        ""
        fragment ""
            #version 300 es
            precision mediump float;
            in highp float v_d;
            layout(location = 0) out mediump vec4 o_color;
            uniform highp float u_depth;
            void main()
            {
                o_color = vec4(gl_FragCoord.z);
                gl_FragDepth = gl_FragCoord.z * 0.5 + v_d * 0.5;
            }
        ""
    end
end
"#;

/// Each `FRAGDEPTH_CASES` program compiles and links on the back end this
/// build carries (the corpus is one GPU per build: PowerVR, GC7000UL, V3D).
#[test]
fn fragdepth_cases() {
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let cases = parse_file(FRAGDEPTH_CASES);
    let names: Vec<&str> = cases.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "fragdepth.write_constant",
            "fragdepth.write_varying",
            "fragdepth.write_uniform",
            "fragdepth.write_conditional",
            "fragdepth.write_conditional_partial",
            "fragdepth.write_loop",
            "fragdepth.write_discard",
            "fragdepth.compare_frag_coord",
        ]
    );
    for c in &cases {
        let units = run_case(c);
        assert_eq!(units.len(), 1, "{}", c.name);
        let u = &units[0];
        assert_eq!(u.verdict, Verdict::PassCompiled, "{}: {}", u.name, u.detail);
        assert_eq!(u.detail, "built", "{}", u.name);
    }
}

// ---- part (b): the corpus ----------------------------------------------------

#[derive(Default)]
struct Counts {
    pass_compiled: usize,
    pass_refused: usize,
    fail_refused: usize,
    fail_accepted: usize,
    skipped: usize,
}

#[test]
fn corpus() {
    let Some(dir) = std::env::var_os("DEQP_GLES3_TEST_DIR") else {
        println!("es3_shaderc_corpus: DEQP_GLES3_TEST_DIR is not set; the dEQP corpus part is skipped");
        return;
    };
    let _g = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let skip = |why: String| println!("es3_shaderc_corpus: corpus part skipped: {} {why}", Path::new(&dir).display());
    let entries = match std::fs::read_dir(Path::new(&dir)) {
        Ok(e) => e,
        Err(e) => return skip(format!("is not readable ({e})")),
    };
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("test"))
        .collect();
    files.sort();
    if files.is_empty() {
        return skip("has no .test files".into());
    }

    // Parse first, so a malformed file panics with its message.
    let mut parsed: Vec<(String, Vec<Case>)> = Vec::new();
    for p in &files {
        match std::fs::read_to_string(p) {
            Ok(text) => parsed.push((p.file_stem().unwrap().to_string_lossy().into_owned(), parse_file(&text))),
            Err(e) => return skip(format!("has an unreadable file {} ({e})", p.display())),
        }
    }

    // A compile that panics must not print a backtrace per case.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let mut table: BTreeMap<String, Counts> = BTreeMap::new();
    let mut report = String::new();
    for (file, cases) in &parsed {
        let counts = table.entry(file.clone()).or_default();
        for case in cases {
            for u in run_case(case) {
                match u.verdict {
                    Verdict::PassCompiled => counts.pass_compiled += 1,
                    Verdict::PassRefused => counts.pass_refused += 1,
                    Verdict::FailRefused => counts.fail_refused += 1,
                    Verdict::FailAccepted => counts.fail_accepted += 1,
                    Verdict::Skipped => counts.skipped += 1,
                }
                if u.verdict != Verdict::PassCompiled {
                    let first = u.detail.lines().next().unwrap_or("");
                    let _ = writeln!(report, "{file}\t{}\t{:?}\t{:?}\t{first}", u.name, u.expect, u.verdict);
                }
            }
        }
    }
    std::panic::set_hook(hook);

    println!("{:<28}{:>8}{:>8}{:>10}{:>10}{:>8}", "file", "pass ok", "pass no", "fail ok", "fail acc", "skipped");
    let mut sum = Counts::default();
    for (f, c) in &table {
        println!(
            "{f:<28}{:>8}{:>8}{:>10}{:>10}{:>8}",
            c.pass_compiled, c.pass_refused, c.fail_refused, c.fail_accepted, c.skipped
        );
        sum.pass_compiled += c.pass_compiled;
        sum.pass_refused += c.pass_refused;
        sum.fail_refused += c.fail_refused;
        sum.fail_accepted += c.fail_accepted;
        sum.skipped += c.skipped;
    }
    println!(
        "{:<28}{:>8}{:>8}{:>10}{:>10}{:>8}",
        "total", sum.pass_compiled, sum.pass_refused, sum.fail_refused, sum.fail_accepted, sum.skipped
    );
    if let Some(path) = std::env::var_os("DEQP_GLES3_REPORT") {
        std::fs::write(path, &report).expect("the report file is writable");
    }
    let total = sum.pass_compiled + sum.pass_refused + sum.fail_refused + sum.fail_accepted + sum.skipped;
    assert!(total > 1000, "only {total} programs came out of {} files", files.len());
}
