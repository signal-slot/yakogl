// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR: the vertex PDS program with up to PVR_PDS_VERTEX_MAX_STREAMS (16)
//! per-instance streams, each with its own divisor (docs/notes/
//! instanceid-per-draw.md, fix P3). tests/es3_pvr_divisor_pds_many.c runs
//! the REAL generator (driver/src/pvr_pds_render_gen.c over Mesa's
//! pvr_pds.c, built without NDEBUG so Mesa's asserts are live) for 1, 8 and
//! 16 streams, one process per case, and this checks:
//!
//! - that the generation succeeds (no refusal, no assert);
//! - that every stream's data words hold Mesa's multiplier (word 8s+3, after
//!   the stride at 8s+2) and its code the instance divide on the temps Mesa's
//!   allocator gives it (the shift is an immediate of SFTLP32/SFTLP64);
//! - that the data and code sizes fit what src/pvr/backend.rs gives the
//!   draw's vertex program (`VDATA_DWORDS`, `VCODE_DWORDS` of its per-draw
//!   scratch: 21 code dwords plus, under the feature, Mesa's five-dword
//!   instance divide for each of the 16 streams, 101), and that the temps
//!   are the back end's own count (`pds_vertex_temps`);
//! - that a stream set above Mesa's 32 PDS temps (16 streams of odd
//!   divisors: 34; the mixed 16: 33) is refused by the generator, exit 1,
//!   before Mesa's pvr_pds_get_temps() asserts (it did, pvr_pds.c:305, and
//!   overran silently under the driver's NDEBUG), and that the back end
//!   draws such a draw instance by instance with its per-instance arrays as
//!   constants instead (`instance_divides_overrun_temps`,
//!   `attribs_flatten_instance`). Only 16 divided streams can get there: 15
//!   take at most 32 temps whatever their divisors (a shift's padding temp
//!   costs what a multiply does), so a draw needs every one of its 16
//!   attributes per instance with a divisor above 1.
//!
//! Two sets of divisors: "mixed" (3, 2, 5, 4, 7, 0xffff, 11, 8, 13, 17, 16,
//! 19, 23, 64, 29, 0x8000) and "odd" (16 divisors none of which is a power of
//! two, so every stream takes Mesa's multiply and two temps).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_divisor_pds_many
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,pvr-instance-divisor --test es3_pvr_divisor_pds_many
//! ```
//!
//! Without the feature the back end hands the generator no divisor
//! (`VDIVIDE_CODE_DWORDS` 0, the scratch as before): the generator's cases
//! run, the back end's fit and fallback ones are the feature's.

#![cfg(all(
    feature = "host-test",
    feature = "gpu-powervr",
    not(feature = "no-shader-table"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

use std::path::PathBuf;
use std::process::Output;

use yakogl::pvr::backend::{pds_vertex_temps, PDS_DIVIDE_CODE_DWORDS, PDS_VERTEX_MAX_TEMPS};

const MIXED: [u32; 16] = [3, 2, 5, 4, 7, 0xffff, 11, 8, 13, 17, 16, 19, 23, 64, 29, 0x8000];
const ODD: [u32; 16] = [3, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15, 17, 19, 23, 0xfffe, 0xffff];

/// src/pvr/backend.rs's per-draw vertex program scratch, pinned: the lines
/// must still read so (checked in [`backend_allocation`]), and with
/// PVR_PDS_VERTEX_MAX_STREAMS = 16 they are 138 data and 21 code dwords,
/// plus 5 code dwords a stream (80) under the feature.
const VDATA_LINE: &str = "const VDATA_DWORDS: usize = 8 * PVR_PDS_VERTEX_MAX_STREAMS as usize + 8 + 2;";
const VCODE_LINE: &str =
    "const VCODE_DWORDS: usize = PVR_PDS_VERTEX_MAX_STREAMS as usize + 4 + 1 + VDIVIDE_CODE_DWORDS;";
const VDIVIDE_ON_LINE: &str =
    "const VDIVIDE_CODE_DWORDS: usize = PDS_DIVIDE_CODE_DWORDS as usize * PVR_PDS_VERTEX_MAX_STREAMS as usize;";
const VDIVIDE_OFF_LINE: &str = "const VDIVIDE_CODE_DWORDS: usize = 0;";
const VDATA_DWORDS: u32 = 8 * 16 + 8 + 2;
const VCODE_DWORDS: u32 = 16 + 4 + 1 + if cfg!(feature = "pvr-instance-divisor") { 5 * 16 } else { 0 };
/// Mesa's PDS temp block (pvr_pds.c PVR_PDS_TEMPS_BLOCK_SIZE).
const PDS_TEMPS: u32 = 32;

fn backend_allocation() -> (u32, u32) {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/pvr/backend.rs")).unwrap();
    for line in [VDATA_LINE, VCODE_LINE, VDIVIDE_ON_LINE, VDIVIDE_OFF_LINE] {
        assert!(src.lines().any(|l| l.trim() == line), "src/pvr/backend.rs: no longer reads `{line}`");
    }
    let ffi = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/pvr/ffi.rs")).unwrap();
    assert!(ffi.contains("pub const PVR_PDS_VERTEX_MAX_STREAMS: u32 = 16;"));
    /* the generator checks the same limits (pvr_pds_render_gen.h) */
    let h = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/driver/src/pvr_pds_render_gen.h")).unwrap();
    for line in [
        "#define PVR_PDS_VERTEX_MAX_DATA_DW (8u * PVR_PDS_VERTEX_MAX_STREAMS + 8u + 2u)",
        "#define PVR_PDS_VERTEX_MAX_CODE_DW (PVR_PDS_VERTEX_MAX_STREAMS + 4u + 1u)",
        "#define PVR_PDS_VERTEX_DIVIDE_CODE_DW 5u",
        "#define PVR_PDS_VERTEX_MAX_TEMPS 32u",
    ] {
        assert!(h.lines().any(|l| l.trim() == line), "pvr_pds_render_gen.h: no longer reads `{line}`");
    }
    assert_eq!((PDS_DIVIDE_CODE_DWORDS, PDS_VERTEX_MAX_TEMPS), (5, PDS_TEMPS));
    (VDATA_DWORDS, VCODE_DWORDS)
}

fn build(tag: &str) -> (PathBuf, PathBuf) {
    let root = env!("CARGO_MANIFEST_DIR");
    let drv = "driver";
    let mesa = "third_party/mesa-26.2.1";
    let imagination = format!("{mesa}/src/imagination");
    let dirs = [
        format!("{drv}/src"),
        format!("{drv}/compat"),
        format!("{drv}/src/pvr/generated"),
        format!("{mesa}/include"),
        format!("{mesa}/src"),
        imagination.clone(),
        format!("{imagination}/include"),
        format!("{imagination}/common"),
        format!("{imagination}/vulkan"),
        format!("{imagination}/vulkan/pds"),
        format!("{imagination}/vulkan/pds/pvr_pds_programs"),
    ];
    // One directory per call: the tests run in parallel and remove theirs.
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("yakogl-divisor-many-{}-{tag}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("div_pds_many");
    let cc = std::process::Command::new("cc")
        .args(["-std=c11", "-O1"])
        .args(dirs.iter().map(|d| format!("-I{root}/{d}")))
        .arg(format!("{root}/tests/es3_pvr_divisor_pds_many.c"))
        .arg(format!("{root}/{drv}/src/pvr_pds_render_gen.c"))
        .arg(format!("{root}/{drv}/compat/pvr_mesa_pds.c"))
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("cc");
    assert!(cc.status.success(), "{}", String::from_utf8_lossy(&cc.stderr));
    (dir, exe)
}

/// Mesa's multiplier data word (pvr_pipeline_pds.c:846-864): 0 for a power
/// of two (one SFTLP32 by log2 d), else (2^(shift+31) + d - 1) / d with
/// shift = ceil(log2 d).
fn mesa_multiplier(d: u32) -> u32 {
    if d.is_power_of_two() {
        return 0;
    }
    let shift = 32 - (d - 1).leading_zeros();
    (((1u64 << (shift + 31)) + u64::from(d) - 1) / u64::from(d)) as u32
}

struct Case {
    data: u32,
    code: u32,
    temps: u32,
}

fn text(o: &Output) -> String {
    format!(
        "status {:?}\nstdout:\n{}\nstderr:\n{}",
        o.status,
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn divisors(set: &str) -> &'static [u32; 16] {
    if set == "mixed" {
        &MIXED
    } else {
        &ODD
    }
}

/// Runs the generator for the first `n` divisors of `set`.
fn run_case(set: &str, n: usize) -> (Output, String) {
    let (dir, exe) = build(&format!("{set}{n}"));
    let out = std::process::Command::new(&exe).args([set, &n.to_string()]).output().expect("run");
    let _ = std::fs::remove_dir_all(&dir);
    let all = text(&out);
    eprintln!("{set} n={n}:\n{all}");
    (out, all)
}

/// Generates the program for the first `n` divisors of `set`; asserts the
/// generation succeeded and every stream's words; returns the sizes.
fn generate(set: &str, n: usize) -> Case {
    let (out, all) = run_case(set, n);
    assert!(out.status.success(), "{set} n={n}: the generator failed or asserted\n{all}");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let mut lines = stdout.lines();
    let sizes = lines.next().expect("sizes line");
    let field = |l: &str, k: &str| -> String {
        l.split_whitespace()
            .find_map(|w| w.strip_prefix(&format!("{k}=")).map(str::to_string))
            .unwrap_or_else(|| panic!("no {k} in {l}"))
    };
    let num = |l: &str, k: &str| field(l, k).parse::<u32>().unwrap();
    let case = Case { data: num(sizes, "data"), code: num(sizes, "code"), temps: num(sizes, "temps") };
    assert_eq!(case.temps, num(sizes, "want_temps"), "{set} n={n}: temps_used is not Mesa's allocation\n{all}");
    let divs = divisors(set);
    assert_eq!(case.temps, pds_vertex_temps(divs[..n].iter().copied()), "{set} n={n}: the back end's temp count");
    let streams: Vec<&str> = lines.collect();
    assert_eq!(streams.len(), n, "{all}");
    for (s, l) in streams.iter().enumerate() {
        let d = divs[s];
        assert_eq!(num(l, "s"), s as u32, "{l}");
        assert_eq!(num(l, "d"), d, "{l}");
        let mult = u32::from_str_radix(field(l, "mult").trim_start_matches("0x"), 16).unwrap();
        assert_eq!(mult, mesa_multiplier(d), "{set} n={n} stream {s} (divisor {d}): multiplier word\n{all}");
        assert_eq!(field(l, "stride_ok"), "1", "{set} n={n} stream {s}: stride word\n{all}");
        assert_eq!(field(l, "code_ok"), "1", "{set} n={n} stream {s} (divisor {d}): instance divide code\n{all}");
    }
    case
}

fn assert_fits(set: &str, n: usize, c: &Case) {
    let (vdata, vcode) = backend_allocation();
    eprintln!(
        "{set} n={n}: data {} / {vdata} dw, code {} / {vcode} dw, temps {} / {PDS_TEMPS}",
        c.data, c.code, c.temps
    );
    assert!(c.data <= vdata, "{set} n={n}: data {} dwords > VDATA_DWORDS {vdata}", c.data);
    assert!(c.code <= vcode, "{set} n={n}: code {} dwords > VCODE_DWORDS {vcode}", c.code);
    assert!(c.temps <= PDS_TEMPS);
}

#[test]
fn the_multiplier_formula_divides() {
    for d in MIXED.iter().chain(ODD.iter()).copied().filter(|d| !d.is_power_of_two()) {
        let m = u64::from(mesa_multiplier(d));
        let shift = 32 - (d - 1).leading_zeros();
        // Exact below 2^31 only (tests/es3_pvr_divisor_magic.rs): every id
        // a draw has, its instance count being a GLsizei.
        for i in (0u64..100_000).chain((0..1000).map(|k| 0x7fff_ffff - k * 2_000_029)) {
            assert_eq!((i * m) >> (shift + 31), i / u64::from(d), "d={d} i={i}");
        }
    }
}

#[test]
fn one_stream_generates_and_fits() {
    for set in ["mixed", "odd"] {
        let c = generate(set, 1);
        assert_fits(set, 1, &c);
    }
}

/// Every stream count below 16 generates with the words above (Mesa's 32
/// temps hold IR0, IR1 and up to 15 streams of two temps) and, under the
/// feature, fits the back end's scratch; the sizes are printed for the
/// report (`-- --nocapture`).
#[test]
fn up_to_fifteen_streams_generate() {
    for set in ["mixed", "odd"] {
        for n in 1..=15 {
            let c = generate(set, n);
            println!(
                "{set} n={n}: data {} dw, code {} dw, temps {} (backend: {VDATA_DWORDS} / {VCODE_DWORDS} dw)",
                c.data, c.code, c.temps
            );
            if cfg!(feature = "pvr-instance-divisor") {
                assert_fits(set, n, &c);
            }
        }
    }
}

#[test]
fn eight_streams_generate() {
    for set in ["mixed", "odd"] {
        generate(set, 8);
    }
}

/// 41 (mixed) / 53 (odd) code dwords: above the 21 of the scratch before the
/// fix, inside its 101 under the feature (without it the back end hands no
/// divisor, so no such program is made).
#[cfg(feature = "pvr-instance-divisor")]
#[test]
fn eight_streams_fit() {
    for set in ["mixed", "odd"] {
        let c = generate(set, 8);
        assert_fits(set, 8, &c);
    }
}

/// 16 streams above Mesa's 32 temps: the generator refuses them (exit 1,
/// its -1) before Mesa's allocator runs, which asserted (pvr_pds.c:305)
/// and, built with NDEBUG as the driver is, overran its temp block. The back
/// end does not hand it such a set ([`sixteen_divided_attribs_are_drawn_per_instance`]).
fn assert_refused(set: &str, want: u32) {
    let (out, all) = run_case(set, 16);
    assert_eq!(out.status.code(), Some(1), "{set} n=16: a refusal, not an assert or a program\n{all}");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let line = format!("generator refused want_temps={want}");
    assert!(stdout.lines().any(|l| l == line), "{set} n=16: `{line}`\n{all}");
    assert_eq!(pds_vertex_temps(divisors(set).iter().copied()), want, "{set}: the back end's temp count");
    assert!(want > PDS_TEMPS);
}

#[test]
fn sixteen_mixed_streams_are_refused_not_asserted() {
    assert_refused("mixed", 33);
}

#[test]
fn sixteen_odd_streams_are_refused_not_asserted() {
    assert_refused("odd", 34);
}

/// The back end's temp count is Mesa's allocator's: IR0 and IR1, a shift one
/// temp, a multiply two on a 64-bit boundary, divisor 0 or 1 none.
#[test]
fn the_back_end_counts_the_temps_as_mesa() {
    assert_eq!(pds_vertex_temps([]), 2);
    assert_eq!(pds_vertex_temps([0, 1, 1]), 2);
    assert_eq!(pds_vertex_temps([2]), 3);
    assert_eq!(pds_vertex_temps([3]), 4);
    assert_eq!(pds_vertex_temps([2, 3]), 6, "the multiply after a shift starts on an even temp");
    assert_eq!(pds_vertex_temps([3, 2]), 5);
    assert_eq!(pds_vertex_temps([3; 15]), 32);
    assert_eq!(pds_vertex_temps([3; 16]), 34);
    /* 15 streams never need more than 32, whatever their divisors */
    for mask in 0u32..(1 << 15) {
        let divs = (0..15).map(|b| if mask & (1 << b) != 0 { 4 } else { 5 });
        assert!(pds_vertex_temps(divs) <= PDS_TEMPS, "shift mask {mask:#x}");
    }
}

#[cfg(feature = "pvr-instance-divisor")]
mod fallback {
    use super::*;
    use yakogl::backend::{GlesAttrib, GlesDraw};
    use yakogl::pvr::backend::{attribs_flatten_instance, instance_divides_overrun_temps};

    /// A draw of `divs.len()` buffer arrays at vtxin 4 * s, the per-instance
    /// ones (divisor != 0) with that divisor; the register order reversed
    /// against the attribute order, as the vertex fetch sorts them.
    fn draw(divs: &[u32]) -> GlesDraw {
        let mut d = GlesDraw::ZERO;
        let n = divs.len();
        for (s, &dv) in divs.iter().enumerate() {
            d.attribs[n - 1 - s] = GlesAttrib {
                vi_reg: (4 * s) as u8,
                components: 4,
                stride: 16 + 4 * s as u32,
                offset: 64,
                buffer_gpu: 0x100000 * (s as u64 + 1),
                per_instance: (dv != 0) as u8,
                divisor: dv as u16,
                buffer_bytes: 1 << 20,
                ..GlesAttrib::ZERO
            };
        }
        d.num_attribs = n as u8;
        d.instance_count = 100;
        d
    }

    /// Exactly the stream sets above Mesa's temps take the per-instance
    /// draw: 16 divided arrays of either set; not 15 of them, not 16 with one
    /// per vertex, not 16 of divisor 1.
    #[test]
    fn sixteen_divided_attribs_are_drawn_per_instance() {
        assert!(instance_divides_overrun_temps(&draw(&MIXED)));
        assert!(instance_divides_overrun_temps(&draw(&ODD)));
        assert!(!instance_divides_overrun_temps(&draw(&MIXED[..15])));
        assert!(!instance_divides_overrun_temps(&draw(&ODD[..15])));
        let mut one_vertex = ODD;
        one_vertex[7] = 0;
        assert!(!instance_divides_overrun_temps(&draw(&one_vertex)));
        assert!(!instance_divides_overrun_temps(&draw(&[1; 16])));
        /* 16 of divisor 2: one temp each, 18 */
        assert!(!instance_divides_overrun_temps(&draw(&[2; 16])));
    }

    /// Each instance's draw reads, from every per-instance array, element
    /// instance / divisor as a constant (per vertex, stride 0); per-vertex
    /// arrays are untouched.
    #[test]
    fn a_flattened_instance_reads_element_instance_over_divisor() {
        let mut divs = ODD;
        divs[3] = 0;
        let d = draw(&divs);
        for inst in [0u32, 1, 2, 3, 37, 99, 0xfffe, 0x1_0000, 0x2_0003] {
            let mut dd = d;
            attribs_flatten_instance(&mut dd, inst);
            for (a, f) in d.attribs.iter().zip(&dd.attribs).take(16) {
                if a.per_instance == 0 {
                    let key = |x: &GlesAttrib| (x.offset, x.stride, x.per_instance, x.divisor, x.cpu_bytes);
                    assert_eq!(key(a), key(f), "a per-vertex array is untouched");
                    continue;
                }
                let e = inst / u32::from(a.divisor);
                assert_eq!((f.per_instance, f.divisor, f.stride), (0, 0, 0), "instance {inst}");
                assert_eq!(f.offset, a.offset + e * a.stride, "instance {inst}, divisor {}", a.divisor);
                assert_eq!((f.buffer_gpu, f.vi_reg, f.components), (a.buffer_gpu, a.vi_reg, a.components));
            }
        }
    }

    /// A staged (client) array: the copy starts at the element and holds one.
    #[test]
    fn a_flattened_staged_array_copies_its_element() {
        let data: Vec<f32> = (0..64).map(|i| i as f32).collect();
        let mut d = GlesDraw::ZERO;
        d.attribs[0] = GlesAttrib {
            components: 4,
            stride: 16,
            cpu: data.as_ptr() as *const core::ffi::c_void,
            cpu_bytes: 256,
            per_instance: 1,
            divisor: 3,
            ..GlesAttrib::ZERO
        };
        d.num_attribs = 1;
        for inst in [0u32, 5, 47] {
            let mut dd = d;
            attribs_flatten_instance(&mut dd, inst);
            let a = &dd.attribs[0];
            assert_eq!((a.per_instance, a.stride, a.cpu_bytes), (0, 0, 16));
            // SAFETY: inside `data` (element inst / 3 < 16).
            let first = unsafe { *(a.cpu as *const f32) };
            assert_eq!(first, (4 * (inst / 3)) as f32);
        }
        /* an element past the staged bytes empties the array */
        let mut dd = d;
        attribs_flatten_instance(&mut dd, 3 * 17);
        assert_eq!(dd.attribs[0].cpu_bytes, 0);
    }
}
