// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! How much STACK the in-process compiler uses, per phase and per shader.
//!
//! `scratch_survey.rs` measures the heap-free arena; nothing measured the
//! stack until am62p-board found the compiler overflowing a 64 KiB Zephyr
//! main-thread stack into the idle thread's (4 of 4 runs faulted; 0 of 1 at
//! 256 KiB). A Zephyr thread's stack is sized by whoever creates the thread,
//! so the compiler has to publish what it needs -- and a figure nobody
//! measured is the one that faults on a board.
//!
//! The instrument is a painted stack: a phase runs on a thread of known size
//! right after a frame the size of the measuring window has been filled with a
//! pattern and returned, so the frames the phase then pushes land in painted
//! memory, and the lowest byte it overwrote says how deep it went. The figure
//! is the depth below the measuring frame, i.e. what the phase itself needs.
//!
//! It is a HOST measurement: x86_64 frames, this profile's optimisation (the
//! test profile is `opt-level = 1`; run with `--release` for `opt-level = 2`,
//! the Zephyr images' setting). An aarch64 target's frames differ, which is
//! why the published requirement carries a margin over the figure printed
//! here rather than the figure itself.
//!
//! ```sh
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online \
//!     --test stack_survey -- --nocapture
//! ```

#![cfg(all(feature = "shaderc-online", not(feature = "no-shader-table")))]

use yakogl::shaderc::mesa_reference::SHC_SOURCE_TABLE;
use yakogl::shaderc::online::{arena::Arena, codegen, lower, parser, pp, reflect, sema, sema::Stage, store};
use yakogl::shaderc::{SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The measuring window. Every phase must stay inside it, or it is reported
/// as having filled it (a lower bound, never a figure).
const WINDOW: usize = 4 << 20;
const PAINT: u8 = 0xa5;

/// Fill a `WINDOW`-byte frame with the pattern and return its bounds. Never
/// inlined, so the frame is really pushed where the next call's frames go.
#[inline(never)]
fn paint() -> (usize, usize) {
    let mut buf = [0u8; WINDOW];
    for b in buf.iter_mut() {
        // SAFETY: a byte of a live local array.
        unsafe { core::ptr::write_volatile(b, PAINT) };
    }
    let lo = buf.as_ptr() as usize;
    core::hint::black_box(&buf);
    (lo, lo + WINDOW)
}

/// Run `f` and say how many bytes of stack below this frame it touched.
#[inline(never)]
fn depth_of<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let (lo, hi) = paint();
    let r = core::hint::black_box(f());
    let mut p = lo;
    /* The painted frame has returned, but it is this thread's own stack and
     * still mapped; nothing else writes it but the phase just run. */
    // SAFETY: [lo, hi) is this thread's stack, mapped for the thread's life.
    while p < hi && unsafe { core::ptr::read_volatile(p as *const u8) } == PAINT {
        p += 1;
    }
    (r, hi - p)
}

#[derive(Default, Clone, Copy)]
struct Worst {
    bytes: usize,
    at: usize,
}

const PHASES: [&str; 6] = ["pp", "parse", "sema", "reflect", "lower", "compile (all, back end included)"];

fn survey(sources: &[(String, Stage, String)]) -> [Worst; 6] {
    let mut worst = [Worst::default(); 6];
    let mut scratch = vec![0u8; 8 << 20];
    for (k, (_, stage, text)) in sources.iter().enumerate() {
        let mut note = |i: usize, d: usize| {
            if d > worst[i].bytes {
                worst[i] = Worst { bytes: d, at: k };
            }
        };
        {
            let a = Arena::new(&mut scratch);
            let (pre, d) = depth_of(|| pp::preprocess(&a, text));
            note(0, d);
            let Ok(pre) = pre else { continue };
            let (ast, d) = depth_of(|| parser::parse(&a, pre.tokens).and_then(|t| parser::resolve_io(&a, t, *stage)));
            note(1, d);
            let Ok(ast) = ast else { continue };
            let (types, d) = depth_of(|| sema::check(&a, &ast, *stage));
            note(2, d);
            let Ok(types) = types else { continue };
            let (refl, d) = depth_of(|| reflect::reflect(&a, &ast));
            note(3, d);
            let Ok(refl) = refl else { continue };
            let (_, d) = depth_of(|| lower::lower(&a, &ast, *stage, &refl, types).is_ok());
            note(4, d);
        }
        let st = if *stage == Stage::Vertex { SHC_STAGE_VERTEX } else { SHC_STAGE_FRAGMENT };
        /* The front end above runs in this part's own arena; the compile
         * runs in the process's one scratch arena and keep store, which the
         * other parts of this binary use too. */
        let _compile = compile_lock();
        let (compiled, d) =
            depth_of(|| store::with_scratch(|s| codegen::compile(st, text.as_bytes(), 0, s, store::keep()).is_ok()));
        /* A refused arena ran no compile: its depth would be a figure for nothing. */
        assert!(compiled.is_some(), "the scratch arena was busy for {}: the depth measured no compile", sources[k].0);
        note(5, d);
        yakogl::shaderc::online::backend::forget_compiled();
    }
    worst
}

/// The compiler's scratch arena and keep store are one per process, and
/// `store::with_scratch` refuses a second caller (`None`) rather than
/// waiting: the parts of this binary that run at once take turns at the
/// compile, while their front ends, each in its own arena, overlap.
fn compile_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Every source of the survey, with the part it belongs to: the reference
/// table by stage, each directory under `tools/shaders` (its files at any
/// depth), the files at the top of `tools/shaders`, and the adversarial
/// shapes.
fn corpus() -> Vec<(&'static str, (String, Stage, String))> {
    let mut v: Vec<(&'static str, (String, Stage, String))> = SHC_SOURCE_TABLE
        .iter()
        .map(|e| {
            let st = if e.name.contains(".vert") { Stage::Vertex } else { Stage::Fragment };
            let part = if st == Stage::Vertex { "table vertex" } else { "table fragment" };
            (part, (e.name.to_string(), st, e.text.trim_end_matches('\0').to_string()))
        })
        .collect();
    fn walk(dir: &std::path::Path, out: &mut Vec<(String, Stage, String)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, out);
                continue;
            }
            let st = match p.extension().and_then(|x| x.to_str()) {
                Some("vert") => Stage::Vertex,
                Some("frag") => Stage::Fragment,
                _ => continue,
            };
            if let Ok(t) = std::fs::read_to_string(&p) {
                out.push((p.display().to_string(), st, t));
            }
        }
    }
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tools/shaders"));
    let mut walked = Vec::new();
    walk(root, &mut walked);
    for src in walked {
        let rel = std::path::Path::new(&src.0).strip_prefix(root).expect("a file under tools/shaders");
        let part = match rel.components().count() {
            1 => "tools/shaders",
            _ => {
                let dir = rel.components().next().and_then(|c| c.as_os_str().to_str()).expect("a directory name");
                PARTS.iter().map(|p| p.0).find(|p| p.strip_prefix("tools/shaders/") == Some(dir)).unwrap_or_else(|| {
                    panic!("tools/shaders/{dir} is no part of this survey: add it to PARTS with its count")
                })
            }
        };
        v.push((part, src));
    }
    v.extend(adversarial().into_iter().map(|x| ("adversarial", x)));
    v
}

/// Shapes no corpus shader has but a generated one can: nesting at the depth
/// the parser still accepts. The corpus says what applications need; these
/// say what the compiler can be MADE to need, which is what a stack must be
/// sized for when the shaders are not known in advance.
fn adversarial() -> Vec<(String, Stage, String)> {
    let fs = "precision mediump float;\nuniform float u;\n";
    let mut v = Vec::new();
    for depth in [30usize, 60, 100, 120] {
        /* An expression nested in parentheses. */
        let e = format!("{}u{}", "(u + ".repeat(depth), ")".repeat(depth));
        v.push((
            format!("adversarial: {depth} nested parentheses"),
            Stage::Fragment,
            format!("{fs}void main() {{ gl_FragColor = vec4({e}); }}"),
        ));
        /* Statements nested in ifs. */
        let ifs = format!("{}x = x + 1.0;{}", "if (u > 0.5) { ".repeat(depth), " }".repeat(depth));
        v.push((
            format!("adversarial: {depth} nested ifs"),
            Stage::Fragment,
            format!("{fs}void main() {{ float x = 0.0; {ifs} gl_FragColor = vec4(x); }}"),
        ));
    }
    /* Calls inlined to the inliner's own limit, each with a body of its own. */
    let mut calls = String::from(fs);
    calls.push_str("float f0(float x) { return x * u + 1.0; }\n");
    for k in 1..16 {
        calls.push_str(&format!("float f{k}(float x) {{ return f{}(x * u + {k}.0) + f{}(x); }}\n", k - 1, k - 1));
    }
    calls.push_str("void main() { gl_FragColor = vec4(f3(u)); }\n");
    v.push(("adversarial: calls inlined 4 deep, twice each".into(), Stage::Fragment, calls.clone()));
    v.push(("adversarial: calls inlined 15 deep".into(), Stage::Fragment, calls.replace("f3(u)", "f15(u)")));
    v
}

/// The parts of the corpus, how many sources each holds, and the test that
/// surveys it: one test per part, so the harness (and cargo-nextest) runs
/// them at once, and no part can be listed without its test. A source added
/// or removed fails `the_parts_cover_the_corpus` until the count here is
/// re-derived, so the survey never covers less than it says. The reference
/// tables differ per GPU (`tests/reference/`).
macro_rules! parts {
    ($($test:ident: $part:literal, $count:expr;)*) => {
        const PARTS: [(&str, usize); [$($part),*].len()] = [$(($part, $count)),*];
        $(
            #[test]
            fn $test() {
                stack_by_phase($part);
            }
        )*
    };
}
parts! {
    the_compilers_stack_by_phase_table_vertex: "table vertex", 219; // both GPUs' tables, by chance
    the_compilers_stack_by_phase_table_fragment: "table fragment", if cfg!(feature = "gpu-vivante") { 331 } else { 335 };
    the_compilers_stack_by_phase_shaders_top: "tools/shaders", 20;
    the_compilers_stack_by_phase_shaders_es100: "tools/shaders/es100", 113;
    the_compilers_stack_by_phase_shaders_es300: "tools/shaders/es300", 30;
    the_compilers_stack_by_phase_shaders_femtovg: "tools/shaders/femtovg", 36;
    the_compilers_stack_by_phase_shaders_gl_test_es100: "tools/shaders/gl_test_es100", 14;
    the_compilers_stack_by_phase_shaders_gltest: "tools/shaders/gltest", 63;
    the_compilers_stack_by_phase_shaders_internal: "tools/shaders/internal", 8;
    the_compilers_stack_by_phase_shaders_offarm: "tools/shaders/offarm", 13;
    the_compilers_stack_by_phase_shaders_qt: "tools/shaders/qt", 169;
    the_compilers_stack_by_phase_shaders_qt3d: "tools/shaders/qt3d", 332;
    the_compilers_stack_by_phase_adversarial: "adversarial", 10;
}
/* The GC7000 table rows: 179 + 151 until 2026-10-12, when 220 were appended
 * (issue #22; tools/vivante/README.md, "The table against its sources"). */
const TOTAL: usize = if cfg!(feature = "gpu-vivante") { 1358 } else { 1362 };

/// The parts together are the whole corpus: every source is in exactly one
/// part (`corpus` refuses a directory no part names), each part holds the
/// count pinned for it, and the counts add up to the pinned total.
#[test]
fn the_parts_cover_the_corpus() {
    let all = corpus();
    let mut seen = std::collections::BTreeMap::<&str, usize>::new();
    for (part, _) in &all {
        *seen.entry(part).or_default() += 1;
    }
    for (part, want) in PARTS {
        assert_eq!(seen.remove(part).unwrap_or(0), want, "the part {part:?} changed size; re-derive PARTS");
    }
    assert!(seen.is_empty(), "sources in no part: {seen:?}");
    assert_eq!(PARTS.iter().map(|p| p.1).sum::<usize>(), TOTAL, "PARTS does not add up to the pinned total");
    assert_eq!(all.len(), TOTAL, "the corpus changed size; re-derive PARTS and TOTAL");
}

/// Survey one part on a thread whose stack holds the measuring window.
fn stack_by_phase(part: &'static str) {
    let want = PARTS.iter().find(|p| p.0 == part).expect("a part of PARTS").1;
    let sources: Vec<(String, Stage, String)> =
        corpus().into_iter().filter(|(p, _)| *p == part).map(|(_, x)| x).collect();
    let n = sources.len();
    assert_eq!(n, want, "the part {part:?} holds {n} sources, pinned at {want}");
    let worst = std::thread::Builder::new()
        .stack_size(WINDOW * 3)
        .spawn(move || {
            let w = survey(&sources);
            (w, sources)
        })
        .expect("a measuring thread")
        .join()
        .expect("the survey ran");
    let (w, sources) = worst;
    println!(
        "\n=== stack by phase, {part}: {n} sources (host {}, painted window {} KiB) ===",
        std::env::consts::ARCH,
        WINDOW / 1024
    );
    for (i, p) in PHASES.iter().enumerate() {
        let at = &sources[w[i].at].0;
        println!("  {:>8} bytes  {p:<34} worst: {at}", w[i].bytes);
    }
    for x in &w {
        assert!(x.bytes < WINDOW, "a phase filled the measuring window; the figure is only a lower bound");
    }
    /* Tripwires, not targets: the published requirement (frontend.md,
     * "Stack") is 256 KiB for the thread that compiles, measured 2026-09-23
     * at 104 KiB for the whole compile on the GC7000 build and 76 KiB on the
     * PowerVR one, with the front end's own worst -- adversarial nesting at
     * the parser's and the inliner's limits -- under 90 KiB. A change that
     * moves a figure past these has changed what every Zephyr thread that
     * compiles must reserve, and has to say so here and in the docs. Each
     * part is held to them, so the whole corpus is. */
    let front = w[..5].iter().map(|x| x.bytes).max().unwrap_or(0);
    assert!(front <= 96 * 1024, "the front end needs {front} bytes of stack, past its 96 KiB tripwire");
    assert!(w[5].bytes <= 160 * 1024, "a compile needs {} bytes of stack, past the 160 KiB tripwire", w[5].bytes);
}
