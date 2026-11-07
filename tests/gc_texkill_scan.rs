// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GC7000 fragment program's TEXKILL scan (`vivante::shader::discards`,
//! the `shader_discards` input of docs/notes/gc-tiled-wire.md's early-depth
//! rule): it finds the `discard` of a compiled test shader, finds none in the
//! mipmap quad's or the clear quad's program, and agrees with the decoder on
//! the corpus of compiled shaders.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_texkill_scan
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::c_char;
use yakogl::shaderc::online::vivante::decode::for_each_instr;
use yakogl::shaderc::online::vivante::isa_tables::OPS;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT};
use yakogl::vivante::internal::{GC_INTERNAL_CLEAR_PS, GC_INTERNAL_CLEAR_VS, GC_INTERNAL_MIPGEN_PS, GC_INTERNAL_TABLE};
use yakogl::vivante::shader::{discards, DiscardCache};

const DISCARD_FRAG: &str = "#version 100
precision mediump float;
varying vec2 uv;
uniform sampler2D t;
void main() { vec4 c = texture2D(t, uv); if (c.a < 0.5) discard; gl_FragColor = c; }
";
const PLAIN_FRAG: &str = "#version 100
precision mediump float;
varying vec2 uv;
uniform sampler2D t;
void main() { gl_FragColor = texture2D(t, uv) * 0.5; }
";

/// The program's words, copied out: the compiler's scratch arena is reset by
/// the next compile (so this binary holds one #[test], src/test_rules.rs).
fn compile_fs(text: &str) -> Vec<u32> {
    yakogl::shaderc::online::backend::forget_compiled();
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    let src = format!("{text}\0");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc =
        unsafe { compile(SHC_STAGE_FRAGMENT, src.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    let msg: Vec<u8> = log.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    assert_eq!(rc, 0, "the test shader compiles: {}", String::from_utf8_lossy(&msg));
    out.usc_words().to_vec()
}

/// The decoder's answer: some instruction's mnemonic is `texkill`.
fn decoder_says(words: &[u32]) -> bool {
    let mut found = false;
    for_each_instr(words, |_, ins| {
        found |= ins.op().mnemonic == "texkill";
        Ok(())
    })
    .expect("the program decodes");
    found
}

fn finds_the_discard_of_a_compiled_shader() {
    let sh = compile_fs(DISCARD_FRAG);
    assert!(!sh.is_empty());
    assert!(discards(&sh), "the discard is a TEXKILL");
    assert!(decoder_says(&sh));
}

fn finds_none_in_a_shader_without_discard() {
    let sh = compile_fs(PLAIN_FRAG);
    assert!(!sh.is_empty());
    assert!(!discards(&sh));
    assert!(!decoder_says(&sh));
}

fn finds_none_in_mipgen_or_the_clear_quad() {
    for idx in [GC_INTERNAL_MIPGEN_PS, GC_INTERNAL_CLEAR_PS, GC_INTERNAL_CLEAR_VS] {
        let w = GC_INTERNAL_TABLE[idx].shader.usc_words();
        assert!(!w.is_empty());
        assert!(!discards(w), "{}", GC_INTERNAL_TABLE[idx].source_name());
    }
}

fn every_texkill_form_is_found_and_nothing_else() {
    let mut n = 0;
    for op in OPS.iter().filter(|o| o.mnemonic == "texkill") {
        let b = op.bits;
        let w = [b as u32, (b >> 32) as u32, (b >> 64) as u32, (b >> 96) as u32];
        assert!(discards(&w), "{}", op.name);
        let mut two = vec![0u32; 4];
        two.extend_from_slice(&w);
        assert!(discards(&two), "{} after another instruction", op.name);
        n += 1;
    }
    assert!(n >= 3);
    assert!(!discards(&[]));
    assert!(!discards(&[0x17, 0, 0]), "a partial instruction is not one");
    // the same low six bits with the opcode's bit 6 set is another opcode
    assert!(!discards(&[0x17, 0, 1 << 16, 0]));
}

fn the_scan_agrees_with_the_decoder_over_the_corpus() {
    let (mut kills, mut checked) = (0, 0);
    for b in yakogl::shaderc::mesa_reference::SHC_BLOB_TABLE.iter() {
        let w = b.shader.usc_words();
        if w.is_empty() || for_each_instr(w, |_, _| Ok(())).is_err() {
            continue;
        }
        assert_eq!(discards(w), decoder_says(w), "{}", b.source_name());
        checked += 1;
        kills += discards(w) as usize;
    }
    assert!(checked > 0 && kills > 0, "{checked} programs, {kills} with a discard");
}

fn the_cache_scans_once_per_address() {
    let kill = compile_fs(DISCARD_FRAG);
    let plain = compile_fs(PLAIN_FRAG);
    let mut c = DiscardCache::new();
    assert!(c.discards(0x1000, &kill));
    // the cached answer wins over the words handed in
    assert!(c.discards(0x1000, &plain));
    assert!(!c.discards(0x2010, &plain));
    c.forget(0x1000);
    assert!(!c.discards(0x1000, &plain), "forgotten, so scanned again");
    // two addresses sharing a slot evict each other and both stay correct
    assert!(c.discards(0x1000 + 16 * 16, &kill));
    assert!(!c.discards(0x1000, &plain));
}

#[test]
fn the_texkill_scan() {
    finds_the_discard_of_a_compiled_shader();
    finds_none_in_a_shader_without_discard();
    finds_none_in_mipgen_or_the_clear_quad();
    every_texkill_form_is_found_and_nothing_else();
    the_scan_agrees_with_the_decoder_over_the_corpus();
    the_cache_scans_once_per_address();
}
