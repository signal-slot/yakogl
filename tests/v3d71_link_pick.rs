// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A `gpu-v3d71` build links GL programs through link71: the two programs of
//! gl_test's six cases (`color_uv` and `clear`, docs/notes/v3d71-gl-table.md
//! section 5) go through the public shaderc entry (`SHC_ONLINE.link`, what
//! glLinkProgram reaches), and the program's code must be the 7.1 chain's:
//! the words of `link_sources71` on the same sources, and not the 4.2
//! chain's (`codegen::link_sources`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d71,shaderc-online,no-shader-table --test v3d71_link_pick
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d71"))]

use core::ffi::c_char;
use std::sync::Mutex;

use yakogl::shaderc::online::codegen;
use yakogl::shaderc::online::store;
use yakogl::shaderc::online::v3d::link71::link_sources71;
use yakogl::shaderc::{LodKey, ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
use yakogl::v3d::backend::{CLEAR_FS, CLEAR_VS};

static COLOR_UV_VS: &str = include_str!("../tools/shaders/gl_test_es100/color_uv.vert");
static COLOR_UV_FS: &str = include_str!("../tools/shaders/gl_test_es100/color_uv.frag");

static SERIAL: Mutex<()> = Mutex::new(());

fn c_source(s: &'static [u8]) -> &'static str {
    core::str::from_utf8(s.strip_suffix(b"\0").expect("NUL-terminated")).expect("UTF-8")
}

fn compile(stage: u32, src: &str) -> ShcShader {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("compiles");
    // SAFETY: NUL-terminated source, real output, log of the announced size.
    let rc = unsafe { f(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    assert_eq!(rc, 0, "compile {stage}");
    out
}

/// The program's three QPU programs back to back (`usc`, `usc_dw`).
fn code_of(sh: &ShcShader) -> Vec<u32> {
    sh.usc_words().to_vec()
}

fn linked_through_entry(vs: &str, fs: &str) -> Vec<u32> {
    let vso = compile(SHC_STAGE_VERTEX, vs);
    let mut fso = compile(SHC_STAGE_FRAGMENT, fs);
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("links");
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the announced size.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    assert_eq!(rc, 0, "link");
    code_of(&fso)
}

fn direct(vs: &str, fs: &str, f: bool) -> Vec<u32> {
    let r = store::with_scratch(|a| {
        let built = if f {
            link_sources71(vs.as_bytes(), None, LodKey::default(), fs.as_bytes(), &[], a, store::keep())
        } else {
            codegen::link_sources(vs.as_bytes(), None, LodKey::default(), fs.as_bytes(), &[], a, store::keep())
        };
        built.map(|(sh, _)| code_of(&sh))
    });
    r.expect("scratch").expect("links")
}

#[test]
fn both_programs_link_through_link71() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for (name, vs, fs) in [("color_uv", COLOR_UV_VS, COLOR_UV_FS), ("clear", c_source(CLEAR_VS), c_source(CLEAR_FS))] {
        yakogl::shaderc::online::backend::forget_compiled();
        let got = linked_through_entry(vs, fs);
        assert!(!got.is_empty(), "{name}: no code");
        assert_eq!(got, direct(vs, fs, true), "{name}: not link71's binary");
        assert_ne!(got, direct(vs, fs, false), "{name}: equals the 4.2 binary");
    }
}
