// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A kept loop carries only the locals live at its head (#7104).
//!
//! The shape is Qt Quick 3D's light loop as SPIRV-Cross writes it: a loop
//! over a uniform light count that indexes uniform arrays, calls helper
//! functions through `param_N` temporaries, and accumulates under an `if`.
//! Before #7104 the PowerVR selector gave every local the body writes a
//! home register live over the whole body and copied it back at the back
//! edge, which kept each temporary and every `if` join of it alive to the
//! end of the loop: Qt's PrincipledMaterial needed 497 temporaries of 252.
//! Here `BLOCKS` copies of the per-light work make a program the old rule
//! refuses (`OutOfRegisters`, 393 temporaries) and the new one compiles in
//! 43 (measured 2026-11-16).
//!
//! What is checked: the program compiles within the temporaries file, and
//! what crosses a pass is still carried: the accumulator, and `last`, which
//! every pass writes and only the code after the loop reads (the unroller's
//! guarded pass must keep it). The byte model's colour equals the IR
//! interpreter's for light counts 0, 1, 3 and 8, the cap of the arrays.
//!
//! HOST-ONLY: the byte model (tests/usc_exec) runs the program, the IR
//! interpreter (tests/usc_exec/ir.rs) gives the answer.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_loop_carried
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::ir::IoSlot;
use yakogl::shaderc::online::powervr::chain::compile_fragment;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Copies of the per-light work in the loop body.
const BLOCKS: usize = 6;

/// The light loop: `blocks` copies of SPIRV-Cross-shaped per-light work,
/// each through its own `param_N` temporaries and helper calls, joined
/// into `acc` under an `if` on the attenuation. `last` is written in every
/// pass and read after the loop.
fn light_loop(blocks: usize) -> String {
    let mut body = String::new();
    for b in 0..blocks {
        let p = 4 * b;
        body += &format!(
            "        vec3 param_{p} = u_pos[i].xyz - u_p.xyz * {k1:.1};\n\
             \x20       float param_{p1} = u_pos[i].w + {k2:.1};\n\
             \x20       float a_{b} = atten(param_{p}, param_{p1});\n\
             \x20       if (a_{b} > 0.0) {{\n\
             \x20           vec3 param_{p2} = normalize(param_{p});\n\
             \x20           vec3 param_{p3} = u_col[i].rgb * {k3:.1};\n\
             \x20           vec3 s_{b} = shade(n, param_{p2}, param_{p3}, u_col[i].a + {k1:.1});\n\
             \x20           acc += s_{b} * a_{b};\n\
             \x20       }}\n",
            p1 = p + 1,
            p2 = p + 2,
            p3 = p + 3,
            k1 = 1.0 + b as f32,
            k2 = 0.5 * b as f32,
            k3 = 1.0 / (1.0 + b as f32),
        );
    }
    format!(
        "#version 300 es\n\
         precision highp float;\n\
         uniform int u_count;\n\
         uniform vec4 u_pos[8];\n\
         uniform vec4 u_col[8];\n\
         uniform vec4 u_p;\n\
         layout(location = 0) out vec4 o;\n\
         float atten(vec3 d, float r) {{ float l = length(d); float a = clamp(1.0 - l / r, 0.0, 1.0); return a * a; }}\n\
         vec3 shade(vec3 n, vec3 l, vec3 c, float k) {{\n\
         \x20   float d = max(dot(n, l), 0.0);\n\
         \x20   vec3 h = normalize(l + vec3(0.0, 0.0, 1.0));\n\
         \x20   float s = pow(max(dot(n, h), 0.0), k);\n\
         \x20   return c * (d + s);\n\
         }}\n\
         void main() {{\n\
         \x20   vec3 n = normalize(u_p.xyz + vec3(0.1));\n\
         \x20   vec3 acc = vec3(0.0);\n\
         \x20   float last = 0.0;\n\
         \x20   for (int i = 0; i < u_count; i++) {{\n\
         {body}\
         \x20       last = u_pos[i].w;\n\
         \x20   }}\n\
         \x20   o = vec4(acc * 0.125, last * 0.0625);\n\
         }}\n"
    )
}

/// The uniform `name`'s dword `k` for a draw of `count` lights.
fn uniform(count: i32, name: &str, k: u32) -> u32 {
    let light = (k / 4) as f32;
    let lane = k % 4;
    match name {
        "u_count" => count as u32,
        "u_pos" => [0.3 + 0.2 * light, -0.1 * light, 0.4, 2.0 + light][lane as usize].to_bits(),
        "u_col" => [0.2 + 0.05 * light, 0.5, 0.9 - 0.1 * light, 8.0][lane as usize].to_bits(),
        "u_p" => [0.1f32, 0.2, -0.3, 1.0][lane as usize].to_bits(),
        _ => 0,
    }
}

/// `src` compiled for the PowerVR: its temporaries, and for each light
/// count the colour the byte model computes and the one the IR gives
/// (RGBA8, as the default variant stores it).
fn run(src: &str, counts: &[i32]) -> Result<(u16, Vec<(u32, u32)>), String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = compile_fragment(&l.module, &l.reflection, a).map_err(|e| format!("chain: {e:?}"))?;
        // SAFETY: the record's code and uniform rows live in `keep`.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let name_of = |n: &[core::ffi::c_char]| -> String {
            n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
        };
        let mut out = Vec::new();
        for &count in counts {
            let mut inp = usc_exec::sentinel_inputs(1.0e30);
            for u in rows.iter() {
                for k in 0..u.dwords as u32 {
                    inp.shared[u.dword_off as usize + k as usize] = uniform(count, &name_of(&u.name), k);
                }
            }
            let ours = *usc_exec::run_binary(&image, &inp, true)?.get(&0).ok_or("no pixel")?;
            let ir_in = usc_exec::ir::IrInputs {
                input: &|_, _| 0,
                uniform: &|d| {
                    let u = l.reflection.uniforms.iter().find(|u| (u.dword_off..u.dword_off + u.dwords).contains(&d));
                    u.map_or(0, |u| uniform(count, u.name, (d - u.dword_off) as u32))
                },
                marker: &|_| 0,
            };
            let want = usc_exec::ir::run_ir(&l.module, &ir_in)?;
            let (_, lanes) = want.outputs.iter().find(|(s, _)| *s == IoSlot::FragColor(0)).ok_or("no colour")?;
            let mut px = 0u32;
            for (c, lane) in lanes.iter().enumerate() {
                let x = f32::from_bits(lane.ok_or("a lane not stored")?.0);
                px |= ((x.clamp(0.0, 1.0) * 255.0).round() as u32) << (8 * c);
            }
            out.push((ours, px));
        }
        Ok((rec.temps, out))
    })
    .unwrap()
}

#[test]
fn a_light_loop_of_inlined_temporaries_fits_and_computes_the_irs_colour() {
    let src = light_loop(BLOCKS);
    let (temps, pixels) = run(&src, &[0, 1, 3, 8]).unwrap_or_else(|e| panic!("{e}\n{src}"));
    assert!(temps <= 96, "{temps} temporaries: the per-pass temporaries are carried again");
    for (count, (ours, ir)) in [0, 1, 3, 8].iter().zip(&pixels) {
        assert_eq!(ours, ir, "{count} lights: ours {ours:08x}, the IR {ir:08x}");
    }
    /* the loop ran: more lights, more light, and `last` is the last pass's */
    assert_ne!(pixels[1].0, pixels[0].0, "one light adds nothing");
    assert_ne!(pixels[3].0 >> 24, pixels[1].0 >> 24, "`last` is not the last pass's value");
}
