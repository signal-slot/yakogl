// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The graphicsfuzz metamorphic case `bubblesort_flag` (plan #388, issue
//! #313) through the PowerVR online back end and the USC byte model.
//!
//! On the AM62P two dEQP-GLES3 cases failed with "Nondeterministic
//! rendering": functional.shaders.metamorphic (graphicsfuzz) bubblesort_flag
//! variant_1 and variant_2. On the host they pass. This test pins the host
//! result: the recipient and both variants, built by the online chain and run
//! in the byte model at every pixel centre of a 16x16 image (the CTS draw: a
//! full-screen quad, `injectionSwitch` = (0, 1), `resolution` = image size),
//!
//! - render a recipient image that is not constant,
//! - render every image the same on a second draw of itself,
//! - render both variants equal to the recipient.
//!
//! The board's failure is therefore not a difference in the compiled program;
//! it matches the 4 GPU recoveries image 11 recorded. HOST-ONLY: no GPU
//! arithmetic.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_metamorph_bubblesort
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_vertex};
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::ShcShader;

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

const SIZE: u32 = 16;

/// VK-GL-CTS 93bca01861b0e3ef3c387027a9791e6d065f900c, external/graphicsfuzz/data/gles3/graphicsfuzz/vertexShader.glsl, byte for byte.
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

/// VK-GL-CTS 93bca01861b0e3ef3c387027a9791e6d065f900c, external/graphicsfuzz/data/gles3/graphicsfuzz/bubblesort_flag/recipient.frag, byte for byte.
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

uniform vec2 resolution;

bool checkSwap(float a, float b)
{
    return gl_FragCoord.y < resolution.y / 2.0 ? a > b : a < b;
}
void main()
{
    float data[10];
    for(
        int i = 0;
        i < 10;
        i ++
    )
        {
            data[i] = float(10 - i) * injectionSwitch.y;
        }
    for(
        int i = 0;
        i < 9;
        i ++
    )
        {
            for(
                int j = 0;
                j < 10;
                j ++
            )
                {
                    if(j < i + 1)
                        {
                            continue;
                        }
                    bool doSwap = checkSwap(data[i], data[j]);
                    if(doSwap)
                        {
                            float temp = data[i];
                            data[i] = data[j];
                            data[j] = temp;
                        }
                }
        }
    if(gl_FragCoord.x < resolution.x / 2.0)
        {
            gl_FragColor = vec4(data[0] / 10.0, data[5] / 10.0, data[9] / 10.0, 1.0);
        }
    else
        {
            gl_FragColor = vec4(data[5] / 10.0, data[9] / 10.0, data[0] / 10.0, 1.0);
        }
}
"####;

/// VK-GL-CTS 93bca01861b0e3ef3c387027a9791e6d065f900c, external/graphicsfuzz/data/gles3/graphicsfuzz/bubblesort_flag/variant_1.frag, byte for byte.
const VARIANT_1: &str = r####"#version 100

// Copyright Alastair F. Donaldson and Hugues Evrard, Imperial College London, 2017
// Defect found using GLFuzz - https://www.graphicsfuzz.com/
//
// Gives wrong image on:
// NVIDIA SHIELD Android TV
// Model number: P2571
// GL_VERSION: OpenGL ES 3.2 NVIDIA 361.00
// GL_VENDOR: NVIDIA Corporation
// GL_RENDERER: NVIDIA Tegra
// Android version: 7.0

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

uniform vec2 resolution;

bool checkSwap(float a, float b)
{
    return gl_FragCoord.y < resolution.y / 2.0 ? a > b : a < b;
}
void main()
{
    float data[10];
    for(
        int i = 0;
        i < 10;
        i ++
    )
        {
            data[i] = float(10 - i) * injectionSwitch.y;
        }
    for(
        int i = 0;
        i < 9;
        i ++
    )
        {
            for(
                int j = 0;
                j < 10;
                j ++
            )
                {
                    if(j < i + 1)
                        {
                            if(false)
                                {
                                    continue;
                                }
                            continue;
                            discard;
                        }
                    bool doSwap = checkSwap(data[i], data[j]);
                    if(doSwap)
                        {
                            float temp = data[i];
                            data[i] = data[j];
                            data[j] = temp;
                        }
                }
        }
    if(gl_FragCoord.x < resolution.x / 2.0)
        {
            gl_FragColor = vec4(data[0] / 10.0, data[5] / 10.0, data[9] / 10.0, 1.0);
        }
    else
        {
            gl_FragColor = vec4(data[5] / 10.0, data[9] / 10.0, data[0] / 10.0, 1.0);
        }
}
"####;

/// VK-GL-CTS 93bca01861b0e3ef3c387027a9791e6d065f900c, external/graphicsfuzz/data/gles3/graphicsfuzz/bubblesort_flag/variant_2.frag, byte for byte.
const VARIANT_2: &str = r####"#version 100

// Copyright Alastair F. Donaldson and Hugues Evrard, Imperial College London, 2017
// Defect found using GLFuzz - https://www.graphicsfuzz.com/
//
// Gives wrong image on:
// NVIDIA SHIELD Android TV
// Model number: P2571
// GL_VERSION: OpenGL ES 3.2 NVIDIA 361.00
// GL_VENDOR: NVIDIA Corporation
// GL_RENDERER: NVIDIA Tegra
// Android version: 7.0

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

uniform vec2 resolution;

bool checkSwap(float a, float b)
{
    (true ? 1.0 : 1.0) + vec4(1.0);
    return gl_FragCoord.y < resolution.y / 2.0 ? a > b : a < b;
}
void main()
{
    float data[10];
    for(
        int i = 0;
        i < 10;
        i ++
    )
        {
            data[i] = float(10 - i) * injectionSwitch.y;
        }
    for(
        int i = 0;
        i < 9;
        i ++
    )
        {
            for(
                int j = 0;
                j < 10;
                j ++
            )
                {
                    if(j < i + 1)
                        {
                            continue;
                        }
                    bool doSwap = checkSwap(data[i], data[j]);
                    if(doSwap)
                        {
                            float temp = data[i];
                            data[i] = data[j];
                            data[j] = temp;
                        }
                }
        }
    if(gl_FragCoord.x < resolution.x / 2.0)
        {
            gl_FragColor = vec4(data[0] / 10.0, data[5] / 10.0, data[9] / 10.0, 1.0);
        }
    else
        {
            gl_FragColor = vec4(data[5] / 10.0, data[9] / 10.0, data[0] / 10.0, 1.0);
        }
}
"####;

/// What the host needs of one compiled fragment program.
struct Program {
    image: Vec<u8>,
    /// Offsets of `injectionSwitch` and `resolution` in the uniform file.
    switch_off: usize,
    resolution_off: usize,
}

fn uniform_off(sh: &ShcShader, name: &str) -> usize {
    // SAFETY: `uniforms` points at `num_uniforms` rows that live in `keep`.
    let rows = unsafe { core::slice::from_raw_parts(sh.uniforms, sh.num_uniforms as usize) };
    let row = rows
        .iter()
        .find(|u| {
            let n: Vec<u8> = u.name.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
            n == name.as_bytes()
        })
        .unwrap_or_else(|| panic!("uniform {name} is active"));
    row.dword_off as usize
}

/// The vertex shader through the chain (it must build), then the fragment
/// shader `src` through the chain.
fn compile(src: &str) -> Program {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let vt = a.alloc_str(VERTEX).unwrap();
        let vl = lower::compile_to_ir(a, Stage::Vertex, vt).unwrap_or_else(|e| panic!("vertex front end: {}", e.msg));
        compile_vertex(&vl.module, &vl.reflection, a).unwrap_or_else(|e| panic!("vertex chain: {e:?}"));
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).unwrap_or_else(|e| panic!("front end: {}", e.msg));
        let rec = compile_fragment(&l.module, &l.reflection, a).unwrap_or_else(|e| panic!("chain: {e:?}"));
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        Program {
            image: code.iter().flat_map(|w| w.to_le_bytes()).collect(),
            switch_off: uniform_off(&rec, "injectionSwitch"),
            resolution_off: uniform_off(&rec, "resolution"),
        }
    })
    .unwrap()
}

/// One draw: the program at every pixel centre, the pixel output dwords of
/// each pixel in order.
fn draw(p: &Program) -> Vec<Vec<(u32, u32)>> {
    let mut img = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut inp = usc_exec::sentinel_inputs(1.0e30);
            inp.shared[p.switch_off] = 0.0f32.to_bits();
            inp.shared[p.switch_off + 1] = 1.0f32.to_bits();
            inp.shared[p.resolution_off] = (SIZE as f32).to_bits();
            inp.shared[p.resolution_off + 1] = (SIZE as f32).to_bits();
            let (xp, yp) = ((x as f32 + 0.5).to_bits(), (y as f32 + 0.5).to_bits());
            inp.frag_xy = [xp, xp, yp, yp];
            let out = usc_exec::run_binary(&p.image, &inp, true).unwrap_or_else(|e| panic!("pixel ({x}, {y}): {e}"));
            img.push(out.into_iter().collect());
        }
    }
    img
}

#[test]
fn bubblesort_flag_host_renders_alike() {
    let recipient = compile(RECIPIENT);
    let v1 = compile(VARIANT_1);
    let v2 = compile(VARIANT_2);

    let r = draw(&recipient);
    assert!(r.iter().any(|px| px != &r[0]), "the recipient image is not constant");
    assert_eq!(r, draw(&recipient), "recipient: a second draw equals the first");

    for (name, p) in [("variant_1", &v1), ("variant_2", &v2)] {
        let img = draw(p);
        assert_eq!(img, draw(p), "{name}: a second draw equals the first");
        assert_eq!(img, r, "{name} renders the recipient's image");
    }
}
