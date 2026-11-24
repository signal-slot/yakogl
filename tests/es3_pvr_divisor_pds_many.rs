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
//!   scratch; the generator copies the whole program there unchecked).
//!
//! Two sets of divisors: "mixed" (3, 2, 5, 4, 7, 0xffff, 11, 8, 13, 17, 16,
//! 19, 23, 64, 29, 0x8000) and "odd" (16 divisors none of which is a power of
//! two, so every stream takes Mesa's multiply and two temps).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,pvr-instance-divisor --test es3_pvr_divisor_pds_many
//! ```
//!
//! The cases that fail on the current generator and back end are
//! `#[ignore]`d with the reason (run them with `-- --ignored`).

#![cfg(all(
    feature = "host-test",
    feature = "gpu-powervr",
    not(feature = "no-shader-table"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

use std::path::PathBuf;
use std::process::Output;

const MIXED: [u32; 16] = [3, 2, 5, 4, 7, 0xffff, 11, 8, 13, 17, 16, 19, 23, 64, 29, 0x8000];
const ODD: [u32; 16] = [3, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15, 17, 19, 23, 0xfffe, 0xffff];

/// src/pvr/backend.rs's per-draw vertex program scratch, pinned: the lines
/// must still read so (checked in [`backend_allocation`]), and with
/// PVR_PDS_VERTEX_MAX_STREAMS = 16 they are 138 data and 21 code dwords.
const VDATA_LINE: &str = "const VDATA_DWORDS: usize = 8 * PVR_PDS_VERTEX_MAX_STREAMS as usize + 8 + 2;";
const VCODE_LINE: &str = "const VCODE_DWORDS: usize = PVR_PDS_VERTEX_MAX_STREAMS as usize + 4 + 1;";
const VDATA_DWORDS: u32 = 8 * 16 + 8 + 2;
const VCODE_DWORDS: u32 = 16 + 4 + 1;
/// Mesa's PDS temp block (pvr_pds.c PVR_PDS_TEMPS_BLOCK_SIZE).
const PDS_TEMPS: u32 = 32;

fn backend_allocation() -> (u32, u32) {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/pvr/backend.rs")).unwrap();
    assert!(src.lines().any(|l| l.trim() == VDATA_LINE), "src/pvr/backend.rs: VDATA_DWORDS changed");
    assert!(src.lines().any(|l| l.trim() == VCODE_LINE), "src/pvr/backend.rs: VCODE_DWORDS changed");
    let ffi = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/pvr/ffi.rs")).unwrap();
    assert!(ffi.contains("pub const PVR_PDS_VERTEX_MAX_STREAMS: u32 = 16;"));
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

/// Generates the program for the first `n` divisors of `set`; asserts the
/// generation succeeded and every stream's words; returns the sizes.
fn generate(set: &str, n: usize) -> Case {
    let (dir, exe) = build(&format!("{set}{n}"));
    let out = std::process::Command::new(&exe).args([set, &n.to_string()]).output().expect("run");
    let _ = std::fs::remove_dir_all(&dir);
    let all = text(&out);
    eprintln!("{set} n={n}:\n{all}");
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
    let divs = if set == "mixed" { &MIXED } else { &ODD };
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
/// temps hold IR0, IR1 and up to 15 streams of two temps); the sizes are
/// printed for the report (`-- --nocapture`).
#[test]
fn up_to_fifteen_streams_generate() {
    for set in ["mixed", "odd"] {
        for n in 1..=15 {
            let c = generate(set, n);
            println!(
                "{set} n={n}: data {} dw, code {} dw, temps {} (backend: {VDATA_DWORDS} / {VCODE_DWORDS} dw)",
                c.data, c.code, c.temps
            );
        }
    }
}

#[test]
fn eight_streams_generate() {
    for set in ["mixed", "odd"] {
        generate(set, 8);
    }
}

#[test]
#[ignore = "defect: 8 per-instance divisor streams make a vertex PDS code segment of 41 (mixed) / 53 (odd) dwords; src/pvr/backend.rs VCODE_DWORDS is 21 and the generator copies the program into that scratch unchecked"]
fn eight_streams_fit() {
    for set in ["mixed", "odd"] {
        let c = generate(set, 8);
        assert_fits(set, 8, &c);
    }
}

#[test]
#[ignore = "defect: 16 per-instance divisor streams need more than Mesa's 32 PDS temps; pvr_pds_get_temps asserts (pvr_pds.c:305; the driver build has NDEBUG, so there it overruns silently)"]
fn sixteen_mixed_streams_generate() {
    generate("mixed", 16);
}

#[test]
#[ignore = "defect: 16 per-instance divisor streams need more than Mesa's 32 PDS temps; pvr_pds_get_temps asserts (pvr_pds.c:305; the driver build has NDEBUG, so there it overruns silently)"]
fn sixteen_mixed_streams_fit() {
    let c = generate("mixed", 16);
    assert_fits("mixed", 16, &c);
}

#[test]
#[ignore = "defect: 16 per-instance divisor streams need more than Mesa's 32 PDS temps; pvr_pds_get_temps asserts (pvr_pds.c:305; the driver build has NDEBUG, so there it overruns silently)"]
fn sixteen_odd_streams_generate() {
    generate("odd", 16);
}
