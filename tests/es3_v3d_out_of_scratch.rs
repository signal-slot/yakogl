// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D compiler's scratch-arena refusal (plan #380, issue #1261; images
//! S16 and UA1 of tools/cts/pi4-gles3-tree.txt). On the Pi 4 at b1b5b845
//! dEQP-GLES3 `functional.shaders.metamorphic.synthetic.variant_3` (S16)
//! failed at glLinkProgram with
//!
//! ```text
//! online shaderc: the code generator refused this program (vertex + fragment): Program(Select(OutOfScratch))
//! ```
//!
//! The V3D compiles the linked program, not a stage (`link_sources`,
//! src/shaderc/online/backend.rs): both stages compile, the link fails with
//! -ENOTSUP and that text. It is the compiler's instruction selection
//! running out of its scratch arena, not a defect of the generated code (the
//! host tables list it as RC13, code generator). The program is small:
//! `recipient.frag`, the same program without the three early exits
//! (`if (injectionSwitch.x > injectionSwitch.y) { break; / return; }`),
//! links (the control). The fix belongs to plan #365
//! (src/shaderc/online/v3d/**); `variant_3_is_refused_with_select_out_of_scratch`
//! flips to `Outcome::Linked` then.
//!
//! UA1 `uniform_api.random.83` (same refusal on the board) is generated from
//! a seed by dEQP's uniform_api random generator and is not pinned here.
//!
//! The shader text is dEQP-GLES3's: VK-GL-CTS 93bca01
//! `external/graphicsfuzz/data/gles3/graphicsfuzz/` (`vertexShader.glsl`,
//! `synthetic/recipient.frag`, `synthetic/variant_3.frag`), Apache-2.0,
//! Copyright Alastair F. Donaldson and Hugues Evrard, Imperial College
//! London, 2017 (GraphicsFuzz), distributed by The Khronos Group Inc.
//! (Apache-2.0, as tools/mali/testdata/deqp_*.license).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_out_of_scratch
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
use core::ffi::c_char;
use std::sync::Mutex;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The compiler keeps its output in one arena that every compile resets.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// The refusal glLinkProgram reports, verbatim.
const OUT_OF_SCRATCH: &str =
    "online shaderc: the code generator refused this program (vertex + fragment): Program(Select(OutOfScratch))";

/// `vertexShader.glsl`, verbatim.
const VERTEX: &str = r####"#version 100

// Copyright Alastair F. Donaldson and Hugues Evrard, Imperial College London, 2017

attribute vec2 coord2d;

varying vec2 surfacePosition;

void main(void)
{
    gl_Position = vec4(coord2d, 0.0, 1.0);
    surfacePosition = coord2d;
}
"####;

/// `synthetic/recipient.frag`, verbatim.
const RECIPIENT: &str = r####"#version 100

// Copyright Alastair F. Donaldson and Hugues Evrard, Imperial College London, 2017

#ifdef GL_ES
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
precision highp int;
#else
precision mediump float;
precision mediump int;
#endif
#endif

uniform vec2 injectionSwitch;

void main()
{
    int r;
    int g;
    int b;
    int a;
    r = 100 * int(injectionSwitch.y);
    g = int(injectionSwitch.x) * int(injectionSwitch.y);
    b = 2 * int(injectionSwitch.x);
    a = g - int(injectionSwitch.x);
    for(
        int i = 0;
        i < 10;
        i ++
    )
        {
            r --;
            g ++;
            b ++;
            a ++;
            for(
                int j = 1;
                j < 10;
                j ++
            )
                {
                    a ++;
                    b ++;
                    g ++;
                    r --;
                }
        }
    float fr;
    float fg;
    float fb;
    float fa;
    fr = float(r / 100);
    fg = float(g / 100);
    fb = float(b / 100);
    fa = float(a / 100);
    gl_FragColor = vec4(r, g, b, a);
}
"####;

/// `synthetic/variant_3.frag`, verbatim.
const VARIANT_3: &str = r####"#version 100

// Copyright Alastair F. Donaldson and Hugues Evrard, Imperial College London, 2017
// Defect found using GLFuzz - https://www.graphicsfuzz.com/
//
// Causes black image to be rendered on:
// ASUS ZenPad 3S 10
// Model number: P027
// GL_VERSION: OpenGL ES 3.1 build 1.5@3830101
// GL_VENDOR: Imagination Technologies
// GL_RENDERER: PowerVR Rogue GX6250
// Android version: 6.0

#ifdef GL_ES
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
precision highp int;
#else
precision mediump float;
precision mediump int;
#endif
#endif

uniform vec2 injectionSwitch;

void main()
{
    int r;
    int g;
    int b;
    int a;
    r = 100 * int(injectionSwitch.y);
    g = int(injectionSwitch.x) * int(injectionSwitch.y);
    b = 2 * int(injectionSwitch.x);
    a = g - int(injectionSwitch.x);
    for(
        int i = 0;
        i < 10;
        i ++
    )
        {
            r --;
            g ++;
            b ++;
            a ++;
            for(
                int j = 1;
                j < 10;
                j ++
            )
                {
                    if(injectionSwitch.x > injectionSwitch.y)
                        {
                            break;
                        }
                    a ++;
                    if(injectionSwitch.x > injectionSwitch.y)
                        {
                            break;
                        }
                    b ++;
                    g ++;
                    r --;
                    if(injectionSwitch.x > injectionSwitch.y)
                        {
                            return;
                        }
                }
        }
    float fr;
    float fg;
    float fb;
    float fa;
    fr = float(r / 100);
    fg = float(g / 100);
    fb = float(b / 100);
    fa = float(a / 100);
    gl_FragColor = vec4(r, g, b, a);
}
"####;

#[derive(Debug, PartialEq)]
enum Outcome {
    Linked,
    CompileFail(String),
    LinkFail(String),
}

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err(text_of(&log))
    }
}

/// glCompileShader on both stages, then glLinkProgram.
fn build(vs: &str, fs: &str) -> Outcome {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::shaderc::online::backend::forget_compiled();
    let vso = match compile(SHC_STAGE_VERTEX, vs) {
        Ok(o) => o,
        Err(m) => return Outcome::CompileFail(format!("vertex: {m}")),
    };
    let mut fso = match compile(SHC_STAGE_FRAGMENT, fs) {
        Ok(o) => o,
        Err(m) => return Outcome::CompileFail(format!("fragment: {m}")),
    };
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the V3D back end links");
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Outcome::Linked
    } else {
        Outcome::LinkFail(text_of(&log))
    }
}

/// S16 variant_3 is refused at link with the scratch-arena text.
/// #365 flips this to `Outcome::Linked`.
#[test]
fn variant_3_is_refused_with_select_out_of_scratch() {
    assert_eq!(build(VERTEX, VARIANT_3), Outcome::LinkFail(OUT_OF_SCRATCH.to_string()));
}

/// Control: the recipient of the same family (no early exits) links.
#[test]
fn the_recipient_of_variant_3_links() {
    assert_eq!(build(VERTEX, RECIPIENT), Outcome::Linked);
}
