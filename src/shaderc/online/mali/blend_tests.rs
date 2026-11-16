// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The blend shader builder against Mesa 26.2.1's drm-shim references
//! (tools/mali/testdata/pan_blend_ref/, tools/mali/pan_blend_ref.py): every
//! state of dEQP-GLES2 fragment_ops.blend (and one colour-mask state) that
//! Mesa draws in Shader mode is built from its GL state as src/mali/blend.rs
//! `shader_key` makes the key (RGBA8, one sample, an fp16 colour: dEQP's
//! mediump varying) and its words are compared with the reference's.
//!
//! Feature set: --no-default-features --features
//! host-test,gpu-mali,shaderc-online,no-shader-table.

use super::{shader, Base, Channel, Factor, Func, Key};
use crate::shaderc::online::store;
use std::collections::HashMap;
use std::path::PathBuf;

const RGBA8_TB_RGBA: u32 = 0xed000;

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

/// One line of states.tsv: its id and key.
fn key_of(line: &str) -> (usize, Key) {
    let f: Vec<&str> = line.split('\t').collect();
    let e: Vec<u32> = f[2].split(' ').map(hex).collect();
    let m: Vec<u8> = f[3].split(' ').map(|b| b.parse().expect("a mask bit")).collect();
    let ch = |f: u32, s: u32, d: u32| {
        /* shader_key: a MIN or MAX channel's factors are ONE, ONE */
        let (s, d) = if matches!(func(f), Func::Min | Func::Max) { (ONE, ONE) } else { (factor(s), factor(d)) };
        Channel { func: func(f), src: s, dst: d }
    };
    let key = Key {
        rgb: ch(e[0], e[1], e[2]),
        alpha: ch(e[3], e[4], e[5]),
        mask: m[0] | m[1] << 1 | m[2] << 2 | m[3] << 3,
        samples: 1,
        colour_f32: false,
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

/// Every reference blend shader: built, compared, tallied. Refused and
/// differing keys are the pinned lists above, by state id.
#[test]
fn the_builder_makes_mesa_s_blend_shaders_for_every_reference_state() {
    let states: HashMap<usize, Key> = read("states.tsv").lines().map(key_of).collect();
    let (mut compared, mut equal, mut other) = (0, 0, 0);
    let (mut refused, mut differ) = (Vec::new(), Vec::new());
    for line in read("blend.tsv").lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let id: usize = f[0].parse().expect("an id");
        if f[2] != "shader" {
            other += 1;
            assert_eq!(f[7], "-", "state {id}: only a shader state has words");
            continue;
        }
        let key = states[&id];
        let mesa = reference_words(f[7]);
        compared += 1;
        let ours = store::with_scratch(|sc| {
            let mut out = vec![0u64; 64];
            shader(&mut out, sc, key, RGBA8_TB_RGBA).map(|b| trim(b.code.to_vec()))
        })
        .expect("the scratch arena is free");
        match ours {
            Err(e) => refused.push((id, format!("{e:?}"))),
            Ok(w) if w == mesa => equal += 1,
            Ok(_) => differ.push(id),
        }
    }
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
