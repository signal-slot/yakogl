// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! More than 32 locals on the Mali (the LOCALS family of plan #367: glmark2
//! effect2d fs ba8e's 5x3 box kernel, terrain fs 1637's noise), against the
//! Mesa 26.2.1 drm-shim references `tools/mali/testdata/gm2_locals_*`,
//! byte for byte (docs/notes/mali-local-arrays.md section 7). Mesa has no
//! count of locals (they are SSA values by then); the walk keeps lanes for
//! up to `MAX_LOCALS` = 256 of them, and the copies of those lanes it nests
//! per `if` and per `break` are in the scratch arena, so the stack a compile
//! takes does not grow with the cap (the survey below).
//!
//! Each pair is built as glCompileShader and glLinkProgram build it
//! (`tests/mali_ref/mod.rs`) and compared in every region. The references
//! were drawn without `--uniform-values`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_locals
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]
mod mali_ref;

/// 41 scalar locals of a vertex shader (40 products and the sum): the
/// count alone stood in the way; the 123 words of the varying program are
/// Mesa's.
#[test]
fn gm2_locals_scalar_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_locals_scalar");
}

/// terrain's noise reduced (36 locals of a highp fragment shader): past the
/// count, two of Mesa's rules the shape meets first: `abs(x2 - x1)` of `x2
/// = x1 + x0` is `abs(x0)` (nir_opt_algebraic.py:271-272,
/// `fadd(fneg(a), fadd(a, b))`), and `m - vec4(dot(x0, x0), ..)` is the
/// negated dot product's chain ending in `m` (:4004-4005, `FMA.f32 r4,
/// r4^.neg, r4^, u3.w0`; the `max(.., 0.0)` the last one's
/// `.clamp_0_inf`). 157 words.
#[test]
fn gm2_locals_noise_matches_its_reference() {
    mali_ref::assert_matches_unset("gm2_locals_noise");
}

/// The other two references stop at the next family, not at the count:
/// the box kernel's 15 look-ups with mediump computed coordinates
/// (TEX_MULTI, plan #367 lower-fma) and the 144 uniform words of the 36
/// `vec4` products (Mesa loads past the FAU's 128 words from the UBO).
#[test]
fn gm2_locals_box_and_vsum_stop_at_their_next_family() {
    let pins = [
        ("gm2_locals_box", "a fragment shader texture look-up beside another"),
        ("gm2_locals_vsum", "a uniform past the FAU's 128 words"),
    ];
    let moved: Vec<String> = pins.iter().filter_map(|(n, t)| mali_ref::refusal_change(n, t)).collect();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

/// A chain of `n` float locals, each the one before, in either stage.
fn chain(n: usize, first: &str) -> String {
    (0..n)
        .map(|i| if i == 0 { format!("float a0 = {first}; ") } else { format!("float a{i} = a{}; ", i - 1) })
        .collect()
}

/// 256 locals compile in both stages, the 257th is the refusal.
#[test]
fn the_cap_is_256_locals_in_both_stages() {
    let vs =
        |n: usize| format!("attribute vec4 p; void main() {{ {} gl_Position = vec4(a{}); }}", chain(n, "p.x"), n - 1);
    let fs = |n: usize| {
        format!(
            "precision highp float; varying float v; void main() {{ {} gl_FragColor = vec4(a{}); }}",
            chain(n, "v"),
            n - 1
        )
    };
    const VS: &str = "attribute vec4 p; varying float v; void main() { v = p.x; gl_Position = p; }";
    const FS: &str = "precision highp float; void main() { gl_FragColor = vec4(1.0); }";
    mali_ref::compile_pair(&vs(256), FS).unwrap_or_else(|e| panic!("256 vertex locals: {e}"));
    mali_ref::compile_pair(VS, &fs(256)).unwrap_or_else(|e| panic!("256 fragment locals: {e}"));
    let text = "a shader with more than 256 local variables";
    mali_ref::assert_refused(&vs(257), FS, text);
    mali_ref::assert_refused(VS, &fs(257), text);
}

/// `src` with `n` locals no one reads declared first in `main`.
fn padded(src: &str, n: usize) -> String {
    let at = src.find("void main(").expect("main");
    let open = at + src[at..].find('{').expect("main's body") + 1;
    let pads: String = (0..n).map(|i| format!(" float zpad{i} = {i}.0;")).collect();
    format!("{}{pads}{}", &src[..open], &src[open..])
}

/// The walk's copies of the locals (an `if`'s sides, a kept loop's
/// `break`s, a guarded pass) hold every local of the module: references
/// with nested `if`s, kept loops with `break`s and unrolled guarded passes
/// compile to the same words with 190 more locals declared (each table
/// then 190 entries wider), so the copies are whole at any width.
#[test]
fn many_locals_leave_branches_and_loops_as_they_were() {
    for name in ["case87_databreak", "intloop_divnest", "cflow_148_texbranch", "fvg_filter_image", "gm2_locals_noise"] {
        let r = mali_ref::Reference::load_unset(name);
        let a = mali_ref::compile_pair(&r.vs, &r.fs).unwrap_or_else(|e| panic!("{name}: {e}"));
        let b = mali_ref::compile_pair(&padded(&r.vs, 190), &padded(&r.fs, 190))
            .unwrap_or_else(|e| panic!("{name} with 190 more locals: {e}"));
        for (stage, x, y) in [("vs", &a.vs, &b.vs), ("fs", &a.fs, &b.fs)] {
            assert_eq!(x.code, y.code, "{name} {stage}: the words moved with 190 more locals");
            assert_eq!(x.work_reg_count, y.work_reg_count, "{name} {stage}: registers");
            assert_eq!(x.secondary, y.secondary, "{name} {stage}: the varying program");
        }
    }
}

/// The measuring window of the painted stack (tests/stack_survey.rs's
/// instrument, which leaves the `no-shader-table` Mali build out).
const WINDOW: usize = 4 << 20;
const PAINT: u8 = 0xa5;

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
    // SAFETY: [lo, hi) is this thread's stack, mapped for the thread's life.
    while p < hi && unsafe { core::ptr::read_volatile(p as *const u8) } == PAINT {
        p += 1;
    }
    (r, hi - p)
}

/// The deepest stack any reference pair's compile and link takes stays
/// under 200 KiB with the cap at 256 locals. HOST-ONLY, x86_64, this
/// profile: measured 172 KiB (fvg_filter_image, its 24 guarded passes
/// nested), where the walk with its copies on the stack took 282 KiB at 32
/// locals and 1187 KiB at 256 (docs/notes/mali-local-arrays.md section 7).
/// The 256 KiB a compiling thread is given (zephyr/Kconfig) is the bound
/// this guards.
#[test]
fn the_stack_of_a_compile_does_not_grow_with_the_cap() {
    let worst = std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(|| {
            let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata");
            let mut names: Vec<String> = std::fs::read_dir(&dir)
                .expect("testdata")
                .filter_map(|e| e.ok()?.file_name().into_string().ok()?.strip_suffix(".expect.json").map(String::from))
                .filter(|n| dir.join(format!("{n}.vert")).exists() && dir.join(format!("{n}.frag")).exists())
                .collect();
            names.sort();
            let mut worst = (0, String::new());
            for n in names {
                let r = mali_ref::Reference::load(&n);
                let (_, d) = depth_of(|| mali_ref::compile_pair(&r.vs, &r.fs).is_ok());
                if d > worst.0 {
                    worst = (d, n);
                }
            }
            worst
        })
        .expect("the survey thread")
        .join()
        .expect("the survey");
    println!("mali_locals: deepest stack {} KiB ({})", worst.0 / 1024, worst.1);
    assert!(worst.0 <= 200 << 10, "{} takes {} KiB of stack", worst.1, worst.0 / 1024);
}
