// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `Reason71`, the fixed 192-byte refusal text of the product build
//! (docs/notes/v3d71-nostd.md section 4, item N2).
//!
//! * The fixed buffer itself (`FixedReason71`, what `Reason71` is without
//!   host-test): `Display`, `Deref<Target = str>`, `PartialEq<str>`, a mark
//!   when the text is cut short, a cut at a character boundary.
//! * Every `*_REFUSALS` table fits 192 bytes uncut, and each formatted
//!   refusal (record71, ident71, prim71, limits71's "no tile fits", format71's
//!   `Debug`, rt71 and attr71's texts) fits at its widest numbers.
//! * With host-test `reason()` gives a `String` of the same text.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d71,shaderc-online,no-shader-table --test v3d71_reason71
//! ```
#![allow(dead_code)]

mod backend {
    pub use yakogl::backend::*;
}

#[path = "../src/v3d/admit71.rs"]
mod admit71;
#[path = "../src/v3d/attr71.rs"]
#[allow(clippy::if_same_then_else)]
mod attr71;
#[path = "../src/v3d/backend71.rs"]
mod backend71;
#[path = "../src/v3d/clearplan71.rs"]
mod clearplan71;
#[path = "../src/v3d/draw71.rs"]
mod draw71;
#[path = "../src/v3d/ffi71.rs"]
mod ffi71;
#[path = "../src/v3d/fill71.rs"]
mod fill71;
#[path = "../src/v3d/format71.rs"]
mod format71;
#[path = "../src/v3d/glue71.rs"]
mod glue71;
#[path = "../src/v3d/ident71.rs"]
mod ident71;
#[path = "../src/v3d/limits71.rs"]
mod limits71;
#[path = "../src/v3d/prim71.rs"]
mod prim71;
#[path = "../src/v3d/record71.rs"]
mod record71;
#[path = "../src/v3d/rt71.rs"]
#[allow(clippy::eq_op)]
mod rt71;
#[path = "../src/v3d/uniform71.rs"]
mod uniform71;
#[path = "../src/v3d/zskeep71.rs"]
mod zskeep71;

use admit71::{fixed_reason, reason, FixedReason71, REASON_CAP, REASON_CUT_MARK};
use core::fmt::Write;
use record71::Stage;

/// Write `s` through the fixed buffer and demand it fits whole.
fn fits(what: &str, s: &str) {
    let r = fixed_reason(format_args!("{s}"));
    assert!(!r.cut(), "{what}: {} bytes cut at {REASON_CAP}: {s}", s.len());
    assert_eq!(&*r, s, "{what}");
}

#[test]
fn the_capacity_is_192() {
    assert_eq!(REASON_CAP, 192);
    assert_eq!(REASON_CUT_MARK, "...");
}

#[test]
fn a_short_text_is_kept_whole() {
    let r = fixed_reason(format_args!("{} of {}", 3, "four"));
    assert!(!r.cut());
    assert_eq!(r.as_str(), "3 of four");
    assert_eq!(&*r, "3 of four");
    assert_eq!(r.len(), 9); // Deref<Target = str>
    assert!(r == *"3 of four");
    assert!(r == "3 of four");
    assert_eq!(r.to_string(), "3 of four");
    assert_eq!(format!("{r}"), "3 of four");
    assert_eq!(format!("{r:?}"), "\"3 of four\"");
    assert_eq!(r.clone(), r);
    assert_ne!(r, fixed_reason(format_args!("3 of five")));
    assert_eq!(FixedReason71::empty().as_str(), "");
}

#[test]
fn exactly_the_capacity_is_not_cut() {
    let s = "x".repeat(REASON_CAP);
    let r = fixed_reason(format_args!("{s}"));
    assert!(!r.cut());
    assert_eq!(r.len(), REASON_CAP);
    assert_eq!(&*r, s);
}

#[test]
fn a_longer_text_is_cut_and_marked() {
    let s = "y".repeat(REASON_CAP + 1);
    let r = fixed_reason(format_args!("{s}"));
    assert!(r.cut());
    assert_eq!(r.len(), REASON_CAP);
    assert!(r.ends_with(REASON_CUT_MARK));
    assert_eq!(&r[..REASON_CAP - 3], &s[..REASON_CAP - 3]);
    // pieces written one after another cut the same way
    let r = fixed_reason(format_args!("{}{}{}", &s[..100], &s[..100], "tail"));
    assert!(r.cut());
    assert_eq!(r.len(), REASON_CAP);
    assert!(r.ends_with(REASON_CUT_MARK));
}

#[test]
fn a_cut_lands_on_a_character_boundary() {
    // 2-byte characters: a cut at an odd byte count must back up
    for pad in 0..4 {
        let s = format!("{}{}", "a".repeat(pad), "é".repeat(REASON_CAP));
        let r = fixed_reason(format_args!("{s}"));
        assert!(r.cut());
        assert!(r.ends_with(REASON_CUT_MARK));
        assert!(r.len() <= REASON_CAP);
        let body = &r[..r.len() - 3];
        assert!(s.starts_with(body));
    }
}

#[test]
fn text_after_a_cut_is_dropped() {
    let mut r = FixedReason71::empty();
    r.write_str(&"z".repeat(REASON_CAP + 5)).unwrap();
    r.write_str("more").unwrap();
    assert!(r.cut());
    assert!(!r.contains("more"));
}

#[test]
fn host_test_reason_is_a_string_of_the_same_text() {
    let n = 7u32;
    let r: String = reason(format_args!("no tile size fits {n} colour bytes per pixel"));
    assert_eq!(r, fixed_reason(format_args!("no tile size fits {n} colour bytes per pixel")).as_str());
    // what the three tests build
    let x = admit71::Refusal { module: "fill71", reason: "text".to_string() };
    assert_eq!(x.to_string(), "fill71: text");
}

#[test]
fn every_refusals_table_fits() {
    for (t, name) in [
        (&fill71::FILL71_REFUSALS[..], "FILL71_REFUSALS"),
        (&fill71::FILL71_TEX_REFUSALS[..], "FILL71_TEX_REFUSALS"),
        (&glue71::GLUE71_REFUSALS[..], "GLUE71_REFUSALS"),
        (&glue71::GLUE71_TEX_REFUSALS[..], "GLUE71_TEX_REFUSALS"),
        (&backend71::FILLED_REFUSALS[..], "FILLED_REFUSALS"),
    ] {
        assert!(!t.is_empty(), "{name}");
        for s in t {
            fits(name, s);
        }
    }
}

#[test]
fn record71_refusals_fit_at_their_widest() {
    use record71::Refusal as R;
    let stages = [Stage::Fragment, Stage::Vertex, Stage::Coordinate];
    let widest = "min output segments in play";
    let mut all = vec![
        R::NoDrawIndexRecord { ver: u32::MAX, rev: u32::MAX },
        R::NoDrawIndexRecord { ver: 42, rev: 0 },
        R::TooManyVaryings(u32::MAX),
    ];
    for s in stages {
        all.extend([
            R::CodeAddressUnaligned(s),
            R::BadThreadCount(s, u32::MAX),
            R::VpmFieldRange(s, widest),
            R::BaseVertex(s),
            R::DrawIndex(s),
        ]);
    }
    for r in &all {
        let s = r.to_string();
        fits("record71", &s);
        assert_eq!(r.message(), s); // the host-test adapter
        let a = fixed_reason(format_args!("{r}"));
        assert_eq!(&*a, s);
        assert!(!a.cut());
    }
}

#[test]
fn ident71_refusals_fit_at_their_widest() {
    use ident71::Refusal as R;
    for r in [
        R::V3d42 { iprev: u32::MAX },
        R::OldIprev { iprev: u32::MAX },
        R::UnknownVersion { tver: u32::MAX, rev: u32::MAX, ver: u32::MAX },
    ] {
        fits("ident71", &r.to_string());
    }
}

#[test]
fn prim71_refusals_fit_at_their_widest() {
    use prim71::Refusal71 as R;
    for r in [
        R::Mode(u32::MAX),
        R::IndexType(u32::MAX),
        R::ZeroCount,
        R::ZeroInstances,
        R::Overflow { field: "Index of First Vertex", value: u64::MAX, bits: u32::MAX },
        R::BaseVertex(i64::MIN),
        R::IndexRange { end: u64::MAX, size: u64::MAX },
        R::IndexAlign { offset: u64::MAX, index_bytes: u64::MAX },
    ] {
        fits("prim71", &r.to_string());
    }
}

#[test]
fn the_modules_own_texts_fit() {
    use rt71::Refusal71 as Rt;
    for r in [Rt::FormatNotRenderable, Rt::NotAColourFormat, Rt::UnknownFormat, Rt::TooManyTargets, Rt::TargetTooLarge]
    {
        fits("rt71", r.as_str());
    }
    use attr71::Refusal71 as At;
    for r in [At::ConstantAttribute, At::Type(u32::MAX), At::TooManyAttributes(usize::MAX)] {
        fits("attr71", r.message());
    }
    use format71::Refusal71 as F;
    for r in [F::UnknownFormat, F::UnsizedTypeNotListed, F::NoThreeChannelRow, F::StencilTexturing] {
        fits("format71", &format!("{r:?}"));
    }
}

#[test]
fn the_tile_size_text_fits_at_the_widest_number() {
    let s = format!("no tile size fits {} colour bytes per pixel", u32::MAX);
    fits("limits71", &s);
    // reached through the admission with a format that has no tile size
    let r = admit71::admit_texture(0, 0).unwrap_err();
    fits("admit_texture", &r.reason);
}

#[test]
fn a_cut_refusal_displays_with_its_mark() {
    // the fixed buffer inside the Display of a refusal's text
    let long = "w".repeat(300);
    let r = fixed_reason(format_args!("{}: {long}", "admit71"));
    assert!(r.cut());
    assert!(r.starts_with("admit71: w"));
    assert!(format!("{r}").ends_with("..."));
}
