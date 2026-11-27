// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Outcome (b).2 of docs/notes/gc-depth-sample-wire.md: the GPU pass that
//! resolves a D24S8 surface, sampled as A8R8G8B8, into R32F texels. This
//! pins that its fragment program compiles for the GC7000 through the online
//! shaderc path the back end uses, and stays small.
//!
//! Feature set: `--no-default-features --features host-test,gpu-vivante,shaderc-online`.

#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::c_char;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT};

/// Bounds: the survey measured about 12 instructions and 3 temps.
const MAX_INSTRUCTIONS: u32 = 16;
const MAX_TEMPS: u16 = 4;

/// The CPU's `(a*65536 + r*256 + g) / 16777215`, from the unorm8 read of the
/// A8R8G8B8 view (`c/255` rounded back to a byte), result in `.r`.
const DEPTH_RESOLVE_FS: &str = r#"precision highp float;
uniform highp sampler2D u_depth;
varying highp vec2 v_uv;
void main()
{
	highp vec4 t = floor(texture2D(u_depth, v_uv) * 255.0 + 0.5);
	highp float d = (t.a * 65536.0 + t.r * 256.0 + t.g) * (1.0 / 16777215.0);
	gl_FragColor = vec4(d, 0.0, 0.0, 1.0);
}
"#;

#[test]
fn depth_resolve_fragment_program_compiles_within_bounds() {
    yakogl::shaderc::online::backend::forget_compiled();
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    let text = format!("{DEPTH_RESOLVE_FS}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc =
        unsafe { compile(SHC_STAGE_FRAGMENT, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    let msg: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    assert_eq!(rc, 0, "the depth-resolve fragment program must compile: {msg}");
    // A GC instruction is four dwords.
    assert_eq!(out.usc_dw % 4, 0, "whole instructions: {} dwords", out.usc_dw);
    let instructions = out.usc_dw / 4;
    eprintln!("depth resolve fs: {instructions} instructions, {} temps", out.temps);
    assert!(instructions > 0, "an empty program");
    assert!(instructions <= MAX_INSTRUCTIONS, "{instructions} instructions, bound {MAX_INSTRUCTIONS}");
    assert!(out.temps <= MAX_TEMPS, "{} temps, bound {MAX_TEMPS}", out.temps);
}
