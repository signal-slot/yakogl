// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 depth resolve: is there a faster unorm24-to-float conversion that
//! is bit-identical to `ds_retention::unorm24_depth` for all 2^24 inputs
//! (docs/notes/gc-unorm24.md)?
//!
//! The reference is `((v as f64) / 16_777_215.0) as f32`, written out here and
//! asserted equal to the crate's public `ds_retention::unorm24_depth` (which
//! `ds_cache::resolve_depth` calls per texel). Each candidate is compared bit
//! for bit over every `v` in `0..2^24`. A candidate marked exact must have no
//! mismatch; one marked not exact must have at least one (so the note's
//! verdict cannot rot). The first mismatches are printed.
//!
//! The exhaustive test takes about a second at the test profile's opt-level 3
//! (the profile every row of tools/check.sh uses), so it is not `#[ignore]`d.
//! The timing test prints, asserts nothing about time:
//!
//! ```text
//! cargo test --release --test gc_unorm24_exhaustive -- --nocapture --test-threads 1
//! ```

use std::hint::black_box;
use std::time::Instant;
use yakogl::ds_retention::unorm24_depth;

const N: u32 = 1 << 24;
const MAXV: f32 = 16_777_215.0;

fn reference(v: u32) -> f32 {
    ((v & 0xff_ffff) as f64 / 16_777_215.0) as f32
}

const RCP64: f64 = 1.0 / 16_777_215.0;
const RCP32: f32 = 1.0 / MAXV;

fn f64_recip_mul(v: u32) -> f32 {
    ((v & 0xff_ffff) as f64 * RCP64) as f32
}

fn f32_recip_mul(v: u32) -> f32 {
    (v & 0xff_ffff) as f32 * RCP32
}

/// f32 reciprocal multiply, then one Markstein step: the exact residual
/// `v - q * (2^24 - 1)` (one fma) times the reciprocal, added to q.
fn f32_fma_correct(v: u32) -> f32 {
    let x = (v & 0xff_ffff) as f32;
    let q = x * RCP32;
    let r = (-q).mul_add(MAXV, x);
    r.mul_add(RCP32, q)
}

/// The same step in f64 on the f64 reciprocal product, rounded to f32 last.
fn f64_fma_correct(v: u32) -> f32 {
    let x = (v & 0xff_ffff) as f64;
    let q = x * RCP64;
    let r = (-q).mul_add(16_777_215.0, x);
    r.mul_add(RCP64, q) as f32
}

/// Two 4096-entry tables (`hi * 4096 / D`, `lo / D`, in f64), summed in f64.
struct Split {
    hi: Vec<f64>,
    lo: Vec<f64>,
}

impl Split {
    fn new() -> Self {
        Split {
            hi: (0..4096).map(|i| (i * 4096) as f64 / 16_777_215.0).collect(),
            lo: (0..4096).map(|i| i as f64 / 16_777_215.0).collect(),
        }
    }
    fn get(&self, v: u32) -> f32 {
        let v = v & 0xff_ffff;
        (self.hi[(v >> 12) as usize] + self.lo[(v & 0xfff) as usize]) as f32
    }
}

/// The full 2^24-entry table (64 MiB), built from the reference.
fn full_table() -> Vec<f32> {
    (0..N).map(reference).collect()
}

struct Report {
    name: &'static str,
    mismatches: u64,
    first: Vec<(u32, f32, f32)>,
}

fn compare(name: &'static str, f: impl Fn(u32) -> f32) -> Report {
    let mut mismatches = 0u64;
    let mut first = Vec::new();
    for v in 0..N {
        let (a, b) = (reference(v), f(v));
        if a.to_bits() != b.to_bits() {
            mismatches += 1;
            if first.len() < 3 {
                first.push((v, a, b));
            }
        }
    }
    Report { name, mismatches, first }
}

#[test]
fn reference_is_the_crates_conversion() {
    for v in 0..N {
        assert_eq!(unorm24_depth(v).to_bits(), reference(v).to_bits(), "v = {v:#x}");
    }
    // The crate masks to 24 bits; the high byte (stencil) must not leak in.
    assert_eq!(unorm24_depth(0xffff_ffff).to_bits(), 1.0f32.to_bits());
    assert_eq!(unorm24_depth(0x0100_0000), 0.0);
}

#[test]
fn candidates_over_all_2_24_inputs() {
    let split = Split::new();
    let table = full_table();
    // (candidate, is it bit-exact?). A changed verdict must change the note.
    let reports: Vec<(Report, bool)> = vec![
        (compare("f64 multiply by f64 reciprocal", f64_recip_mul), EXACT_F64_RECIP),
        (compare("f32 multiply by f32 reciprocal", f32_recip_mul), EXACT_F32_RECIP),
        (compare("f32 reciprocal + fma correction", f32_fma_correct), EXACT_F32_FMA),
        (compare("f64 reciprocal + fma correction", f64_fma_correct), EXACT_F64_FMA),
        (compare("split lookup 2 x 4096 (f64 sum)", |v| split.get(v)), EXACT_SPLIT),
        (compare("full 2^24-entry f32 table", |v| table[(v & 0xff_ffff) as usize]), true),
    ];
    let mut bad = Vec::new();
    for (r, exact) in &reports {
        println!("{:36} mismatches {:>9} of {N}", r.name, r.mismatches);
        for (v, want, got) in &r.first {
            println!(
                "    v = {v:#08x} ({v}): want {want:e} ({:#010x}), got {got:e} ({:#010x})",
                want.to_bits(),
                got.to_bits()
            );
        }
        if (r.mismatches == 0) != *exact {
            bad.push(r.name);
        }
    }
    assert!(bad.is_empty(), "verdict changed for {bad:?}");
}

const EXACT_F64_RECIP: bool = true;
const EXACT_F32_RECIP: bool = false;
const EXACT_F32_FMA: bool = true;
const EXACT_F64_FMA: bool = true;
const EXACT_SPLIT: bool = true;

/// What each candidate costs on a 1024 x 1024 surface of D24S8 words, as
/// `resolve_depth` reads it (printed, not asserted; only the bit-exact ones
/// matter for a replacement).
#[test]
fn time_1024_square_conversions() {
    const PIXELS: usize = 1024 * 1024;
    let words: Vec<u32> = (0..PIXELS as u32).map(|i| i.wrapping_mul(2_654_435_761).rotate_left(7) & !0xff).collect();
    let split = Split::new();
    let table = full_table();
    let mut dst = vec![0.0f32; PIXELS];
    let mut run = |name: &str, f: &dyn Fn(u32) -> f32| {
        let mut best = f64::MAX;
        for _ in 0..30 {
            let t = Instant::now();
            for (o, w) in dst.iter_mut().zip(black_box(&words)) {
                *o = f(*w >> 8);
            }
            best = best.min(t.elapsed().as_secs_f64() * 1e3);
        }
        black_box(&dst);
        println!("1024x1024 {name:36} best {best:.3} ms");
    };
    run("reference f64 divide", &reference);
    run("f64 multiply by f64 reciprocal", &f64_recip_mul);
    run("f32 multiply by f32 reciprocal", &f32_recip_mul);
    run("f32 reciprocal + fma correction", &f32_fma_correct);
    run("f64 reciprocal + fma correction", &f64_fma_correct);
    run("split lookup 2 x 4096", &|v| split.get(v));
    run("full 2^24-entry table", &|v| table[(v & 0xff_ffff) as usize]);
}
