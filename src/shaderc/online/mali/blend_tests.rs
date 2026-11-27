// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The blend shader builder against Mesa 26.2.1's drm-shim references
//! (tools/mali/testdata/pan_blend_ref/, tools/mali/pan_blend_ref.py): every
//! state of dEQP-GLES2 fragment_ops.blend (and one colour-mask state) that
//! Mesa draws in Shader mode is built from its GL state as src/mali/blend.rs
//! `shader_key` makes the key (RGBA8, one sample, an fp16 colour: dEQP's
//! mediump varying) and its words are compared with the reference's; and
//! the same states drawn into an RGB565 and an XRGB8888 target (rgb565/,
//! xrgb8888/), each state's factors as st/mesa's `fix_xrgb_alpha` leaves
//! them (src/mali/blend.rs `without_dst_alpha`).
//!
//! Feature set: --no-default-features --features
//! host-test,gpu-mali,shaderc-online,no-shader-table.

use super::{shader, Base, Channel, Factor, Func, Key, Target};
use crate::shaderc::online::store;
use std::collections::HashMap;
use std::path::PathBuf;

/// The states the reference holds with a blend shader (`shader` in
/// blend.tsv).
const SHADER_STATES: usize = 1404;
/// The states Mesa blends in the fixed-function unit, or not at all (the
/// builder is not asked for them: src/mali/blend.rs picks the descriptor).
const NOT_SHADER_STATES: usize = 801;
/// Shader states whose key the builder refuses (state ids of states.tsv).
const REFUSED: &[usize] = &[];
/// Shader states whose words differ from Mesa's (state ids of states.tsv).
/// A follow-up fix empties this list.
const DIFFER: &[usize] = &[];
/// The RGB565 and XRGB8888 references: the states Mesa draws with a blend
/// shader (fewer than RGBA8's: `fix_xrgb_alpha` makes 283 of them fixed
/// function), and those it does not.
const NO_ALPHA_SHADER_STATES: usize = 1121;
const NO_ALPHA_NOT_SHADER_STATES: usize = 1084;

const ONE: Factor = Factor { base: Base::One, invert: false };

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata/pan_blend_ref")
}

fn read(name: &str) -> String {
    std::fs::read_to_string(dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).expect("a hex number")
}

fn factor(gl: u32) -> Factor {
    let (base, invert) = match gl {
        0x0000 => (Base::One, true),
        0x0001 => (Base::One, false),
        0x0300 => (Base::SrcColor, false),
        0x0301 => (Base::SrcColor, true),
        0x0302 => (Base::SrcAlpha, false),
        0x0303 => (Base::SrcAlpha, true),
        0x0304 => (Base::DstAlpha, false),
        0x0305 => (Base::DstAlpha, true),
        0x0306 => (Base::DstColor, false),
        0x0307 => (Base::DstColor, true),
        0x0308 => (Base::SrcAlphaSaturate, false),
        0x8001 => (Base::ConstColor, false),
        0x8002 => (Base::ConstColor, true),
        0x8003 => (Base::ConstAlpha, false),
        0x8004 => (Base::ConstAlpha, true),
        _ => panic!("GL factor {gl:#x}"),
    };
    Factor { base, invert }
}

fn func(gl: u32) -> Func {
    match gl {
        0x8006 => Func::Add,
        0x800a => Func::Subtract,
        0x800b => Func::ReverseSubtract,
        0x8007 => Func::Min,
        0x8008 => Func::Max,
        _ => panic!("GL equation {gl:#x}"),
    }
}

/// `fix_xrgb_alpha` (st_atom_blend.c:211-225) of a GL factor.
fn fix_xrgb_alpha(gl: u32) -> u32 {
    match gl {
        0x0304 => 0x0001,
        0x0305 | 0x0308 => 0x0000,
        f => f,
    }
}

/// One line of states.tsv: its id and key, for `target`.
fn key_of(line: &str, target: Target) -> (usize, Key) {
    let f: Vec<&str> = line.split('\t').collect();
    let e: Vec<u32> = f[2].split(' ').map(hex).collect();
    let m: Vec<u8> = f[3].split(' ').map(|b| b.parse().expect("a mask bit")).collect();
    let fix = |f: u32| if target.has_alpha() { f } else { fix_xrgb_alpha(f) };
    let ch = |f: u32, s: u32, d: u32| {
        /* shader_key: a MIN or MAX channel's factors are ONE, ONE */
        let (s, d) =
            if matches!(func(f), Func::Min | Func::Max) { (ONE, ONE) } else { (factor(fix(s)), factor(fix(d))) };
        Channel { func: func(f), src: s, dst: d }
    };
    let key = Key {
        rgb: ch(e[0], e[1], e[2]),
        alpha: ch(e[3], e[4], e[5]),
        mask: m[0] | m[1] << 1 | m[2] << 2 | m[3] << 3,
        samples: 1,
        colour_f32: false,
        target,
    };
    (f[0].parse().expect("an id"), key)
}

fn trim(mut v: Vec<u64>) -> Vec<u64> {
    while v.last() == Some(&0) {
        v.pop();
    }
    v
}

fn reference_words(file: &str) -> Vec<u64> {
    trim(read(&format!("{file}.words")).lines().map(|l| u64::from_str_radix(l.trim(), 16).expect("a word")).collect())
}

/// The reference states of `sub` (a sub directory, "" for RGBA8) built for
/// `target` and compared: the count compared, the states not drawn with a
/// shader, those refused (with the error) and those whose words differ.
struct Tally {
    compared: usize,
    equal: usize,
    other: usize,
    refused: Vec<(usize, String)>,
    differ: Vec<usize>,
}

fn compare(sub: &str, target: Target) -> Tally {
    let states: HashMap<usize, Key> = read("states.tsv").lines().map(|l| key_of(l, target)).collect();
    let (mut compared, mut equal, mut other) = (0, 0, 0);
    let (mut refused, mut differ) = (Vec::new(), Vec::new());
    for line in read(&format!("{sub}blend.tsv")).lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let id: usize = f[0].parse().expect("an id");
        if f[2] != "shader" {
            other += 1;
            assert_eq!(f[7], "-", "state {id}: only a shader state has words");
            continue;
        }
        let key = states[&id];
        let mesa = reference_words(&format!("{sub}{}", f[7]));
        compared += 1;
        let ours = store::with_scratch(|sc| {
            let mut out = vec![0u64; 64];
            shader(&mut out, sc, key, target.conversion()).map(|b| trim(b.code.to_vec()))
        })
        .expect("the scratch arena is free");
        match ours {
            Err(e) => refused.push((id, format!("{e:?}"))),
            Ok(w) if w == mesa => equal += 1,
            Ok(_) => differ.push(id),
        }
    }
    Tally { compared, equal, other, refused, differ }
}

/// Every reference blend shader: built, compared, tallied. Refused and
/// differing keys are the pinned lists above, by state id.
#[test]
fn the_builder_makes_mesa_s_blend_shaders_for_every_reference_state() {
    let Tally { compared, equal, other, refused, differ } = compare("", Target::Rgba8);
    eprintln!(
        "blend references: {compared} compared, {equal} equal, {} refused, {} differ",
        refused.len(),
        differ.len()
    );
    assert_eq!(compared, SHADER_STATES);
    assert_eq!(other, NOT_SHADER_STATES);
    let refused_ids: Vec<usize> = refused.iter().map(|r| r.0).collect();
    assert_eq!(refused_ids, REFUSED, "refused keys: {refused:?}");
    assert_eq!(differ, DIFFER, "differing keys");
    assert_eq!(equal, SHADER_STATES - REFUSED.len() - DIFFER.len());
}

/// The RGB565 and XRGB8888 references: every blend shader Mesa draws into
/// them is the builder's, word for word (the tile format, the descriptor's
/// channel count, a destination alpha of 1 and no alpha written), and the
/// key `shader` takes refuses another target's conversion.
#[test]
fn the_builder_makes_mesa_s_blend_shaders_into_rgb565_and_xrgb8888() {
    for (sub, target) in [("rgb565/", Target::Rgb565), ("xrgb8888/", Target::Xrgb8888)] {
        let t = compare(sub, target);
        eprintln!(
            "{sub} blend references: {} compared, {} equal, {} refused, {} differ",
            t.compared,
            t.equal,
            t.refused.len(),
            t.differ.len()
        );
        assert_eq!(t.compared, NO_ALPHA_SHADER_STATES, "{sub}");
        assert_eq!(t.other, NO_ALPHA_NOT_SHADER_STATES, "{sub}");
        assert_eq!(t.refused, Vec::new(), "{sub}: refused keys");
        assert_eq!(t.differ, Vec::<usize>::new(), "{sub}: differing keys");
        assert_eq!(t.equal, NO_ALPHA_SHADER_STATES, "{sub}");
    }
    /* the conversion must be the key's target's */
    let (_, key) = key_of(read("states.tsv").lines().nth(10).expect("state 10"), Target::Rgb565);
    let r = store::with_scratch(|sc| {
        let mut out = vec![0u64; 64];
        shader(&mut out, sc, key, Target::Rgba8.conversion()).map(|_| ())
    })
    .expect("the scratch arena is free");
    assert_eq!(r, Err(super::MaliError::BlendVariantNotLowered));
}
