// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `gl_FragDepth` on the V3D online compiler (dEQP-GLES3
//! `shaders.fragdepth.write.*` and `.compare.*`, 15 cases refused as
//! `NotYet("gl_FragDepth")` before): the shader's depth goes to the TLB as
//! Mesa's `emit_frag_end` writes it (nir_to_vir.c:2097-2108, Mesa 26.2.1),
//! `mov tlbu, <depth>` with the per-pixel depth specifier
//! `TLB_TYPE_DEPTH | TLB_V42_DEPTH_TYPE_PER_PIXEL | TLB_SAMPLE_MODE_PER_PIXEL`
//! in `0xffffff00` (`select::TLB_Z_PER_PIXEL`, 0xffffff8c; the `ver >= 42`
//! branch, the same byte on V3D 4.2 and 7.1), after the last thread switch
//! and before the colour writes, in place of the pass-through Z from the
//! FEP. The program data says `writes_z` and `disable_ez` (prog_data: the
//! shader record's "fragment shader does Z writes" and "turn off early Z
//! test", which the back end's shader record takes from them;
//! `src/v3d/backend.rs`).
//!
//! Pinned here: the eight dEQP writers build and link through `SHC_ONLINE`
//! (what glCompileShader and glLinkProgram reach); the code has one TLBU
//! depth write with its config word and no pass-through write; the written
//! value comes out of the QPU interpreter (`sim::run`), for a constant, a
//! varying, a conditional and a loop-carried depth, lane by lane; and the
//! 7.1 interpreter (`sim71::run71`) records the same specifier and value
//! for Mesa's 7.1 form of the write. The interpreters have no depth buffer,
//! so the depth test itself is not modelled; the shader-record and config
//! words are the back end's (`src/v3d/`), pinned there.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_fragdepth_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::c_char;
use std::sync::Mutex;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::reflect::Reflection;
use yakogl::shaderc::online::v3d::chain::{compile_program, Code, Prim, Target, V3dProgram};
use yakogl::shaderc::online::v3d::codec71::encode71;
use yakogl::shaderc::online::v3d::isa71::{Alu71, Instr71, MulAlu71, MulOp71, Operand, OutPack, Unpack71};
use yakogl::shaderc::online::v3d::listing::write_instr;
use yakogl::shaderc::online::v3d::select::{tlb_config_f16, TLB_Z_FROM_FEP, TLB_Z_PER_PIXEL};
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, TlbWrite, Varying, LANES};
use yakogl::shaderc::online::v3d::sim71;
use yakogl::shaderc::online::{lower, sema};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

const TARGET: Target = Target { vpm_size: 65536 };

// ---- the dEQP sources (es3fFragDepthTests.cpp) ------------------------------

const VS_COORD: &str = "#version 300 es\n\
in highp vec4 a_position;\nin highp vec2 a_coord;\nout highp vec2 v_coord;\n\
void main (void)\n{\n    gl_Position = a_position;\n    v_coord = a_coord;\n}\n";

fn fragdepth(body: &str) -> String {
    format!(
        "#version 300 es\nuniform highp vec4 u_color;\nuniform highp float uf_half;\nuniform bool ub_true;\n\
         uniform int ui_two;\nuniform highp float uf_fourth;\nin highp vec2 v_coord;\n\
         layout(location = 0) out mediump vec4 o_color;\n{body}\n"
    )
}

/// The eight `gl_FragDepth` writers of `fragdepth.write.*`; `.compare.*`
/// uses the same fragment sources (tests/es3_shaderc_repro.rs
/// `fragdepth_writes_lower_to_a_depth_output` has the front end's half).
const BODIES: [(&str, &str); 8] = [
    ("const", "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = 0.5;\n}"),
    ("uniform", "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = uf_half;\n}"),
    ("dynamic", "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = (v_coord.x+v_coord.y)*0.5;\n}"),
    ("fragcoord_z", "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = gl_FragCoord.z;\n}"),
    (
        "uniform_conditional_write",
        "void main (void)\n{\n    o_color = u_color;\n    if (ub_true)\n        gl_FragDepth = (v_coord.x+v_coord.y)*0.5;\n}",
    ),
    (
        "dynamic_conditional_write",
        "void main (void)\n{\n    o_color = u_color;\n    mediump float d = (v_coord.x+v_coord.y)*0.5f;\n    if (v_coord.y < 0.5)\n        gl_FragDepth = d;\n    else\n        gl_FragDepth = 1.0 - d;\n}",
    ),
    (
        "uniform_loop_write",
        "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = 0.0;\n    for (int i = 0; i < ui_two; i++)\n        gl_FragDepth += uf_fourth;\n}",
    ),
    (
        "write_in_function",
        "void myfunc (highp vec2 coord)\n{\n    gl_FragDepth = (coord.x+coord.y)*0.5;\n}\nvoid main (void)\n{\n    o_color = u_color;\n    myfunc(v_coord);\n}",
    ),
];

// ---- glCompileShader + glLinkProgram --------------------------------------

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

/// glCompileShader on both stages, then glLinkProgram; the linked fragment
/// record's `(writes_z, disable_ez)`.
fn build(vs: &str, fs: &str) -> Result<(u8, u8), String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::shaderc::online::backend::forget_compiled();
    let vso = compile(SHC_STAGE_VERTEX, vs).map_err(|m| format!("vertex: {m}"))?;
    let mut fso = compile(SHC_STAGE_FRAGMENT, fs).map_err(|m| format!("fragment: {m}"))?;
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        assert_eq!(fso.v3d.linked, 1, "the V3D record is the linked program's");
        Ok((fso.v3d.fs.writes_z, fso.v3d.fs.disable_ez))
    } else {
        Err(format!("link: {}", text_of(&log)))
    }
}

/// Every dEQP fragdepth writer builds and links, and the record that reaches
/// the back end (`ShcShader`, `v3d_abi.rs`) says writes_z and disable_ez.
#[test]
fn the_deqp_fragdepth_writers_build_and_link() {
    for (name, body) in BODIES {
        let fs = fragdepth(body);
        let got = build(VS_COORD, &fs).unwrap_or_else(|e| panic!("fragdepth.write.{name}: {e}\n{fs}"));
        assert_eq!(got, (1, 1), "fragdepth.write.{name}: (writes_z, disable_ez)");
    }
    /* and a shader with no depth write keeps early tests */
    let fs = fragdepth("void main (void)\n{\n    o_color = u_color;\n}");
    assert_eq!(build(VS_COORD, &fs), Ok((0, 0)), "no depth write: no Z write, early tests");
}

// ---- the code -----------------------------------------------------------------

fn listing(c: &Code<'_>) -> Vec<String> {
    c.instrs
        .iter()
        .map(|i| {
            let mut s = String::new();
            write_instr(&mut s, i).unwrap();
            s
        })
        .collect()
}

/// The uniform each instruction that takes one consumes, in order
/// (`chain.rs` tests' `uniform_consumers`): an ALU word with a uniform
/// signal or a TLBU write, or a branch on a uniform.
fn tlbu_configs(c: &Code<'_>) -> Vec<u32> {
    use yakogl::shaderc::online::v3d::isa::Instr;
    let mut out = Vec::new();
    let mut next = 0usize;
    for i in c.instrs {
        let takes = match i {
            Instr::Alu(a) => {
                a.sig.ldunif
                    || a.sig.ldunifrf
                    || a.sig.ldunifa
                    || a.sig.ldunifarf
                    || (a.add.magic && a.add.waddr == 8)
                    || (a.mul.magic && a.mul.waddr == 8)
            }
            Instr::Branch(b) => b.ub,
        };
        if !takes {
            continue;
        }
        let u = c.uniforms[next];
        next += 1;
        if let Instr::Alu(a) = i {
            if (a.add.magic && a.add.waddr == 8) || (a.mul.magic && a.mul.waddr == 8) {
                assert_eq!(u.contents, 0, "a TLBU config is a constant");
                out.push(u.data);
            }
        }
    }
    assert_eq!(next, c.uniforms.len(), "uniforms nothing consumes");
    out
}

/// `f` on the compiled program of `vs` + `fs`, with the fragment stage's
/// reflection (the uniform block's layout).
fn with_program<R>(vs: &str, fs: &str, f: impl FnOnce(&V3dProgram<'_>, &Reflection<'_>) -> R) -> R {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(vs).expect("arena");
    let fs = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let f_ = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| e.msg.to_string()).expect("the FS lowers");
    let p = compile_program(&v.module, &f_.module, Prim::Triangles, TARGET, &a)
        .unwrap_or_else(|e| panic!("the program compiles: {e:?}\n{fs}"));
    f(&p, &f_.reflection)
}

/// The depth write's config byte: Mesa's `TLB_TYPE_DEPTH (2 << 6) |
/// TLB_V42_DEPTH_TYPE_PER_PIXEL (1 << 3) | TLB_SAMPLE_MODE_PER_PIXEL (1 << 2)`
/// in 0xffffff00, and not the pass-through form (INVARIANT, 0 << 3).
#[test]
fn the_depth_specifier_is_mesas() {
    assert_eq!(TLB_Z_PER_PIXEL, 0xffff_ff8c);
    assert_eq!(TLB_Z_FROM_FEP, 0xffff_ff84);
    assert_eq!(TLB_Z_PER_PIXEL ^ TLB_Z_FROM_FEP, 1 << 3, "per pixel against invariant is the depth-type bit");
}

/// Each writer's code: one TLBU write with the per-pixel depth config, no
/// pass-through write, after the last thread switch and before the colour
/// (a VFPACK to the TLB), with writes_z and disable_ez set and no discard.
#[test]
fn each_writer_has_one_per_pixel_depth_write_before_the_colour() {
    for (name, body) in BODIES {
        let fs = fragdepth(body);
        with_program(VS_COORD, &fs, |p, _| {
            assert_eq!(
                (p.fs.uses_discard, p.fs.writes_z, p.fs.disable_ez),
                (false, true, true),
                "fragdepth.write.{name}: (uses_discard, writes_z, disable_ez)"
            );
            let configs = tlbu_configs(&p.fs.code);
            let ops = listing(&p.fs.code);
            assert_eq!(
                configs.iter().filter(|&&c| c == TLB_Z_PER_PIXEL).count(),
                1,
                "fragdepth.write.{name}: one depth write, {configs:#x?}\n{ops:#?}"
            );
            assert!(!configs.contains(&TLB_Z_FROM_FEP), "fragdepth.write.{name}: no pass-through write {configs:#x?}");
            /* o_color is RT 0, vec4: the TLB's default config, so its two
             * VFPACK words go through TLB, not TLBU, and the depth write is
             * the program's one TLBU write. */
            assert_eq!(tlb_config_f16(0, 4), !0);
            assert_eq!(configs, [TLB_Z_PER_PIXEL], "fragdepth.write.{name}");
            let z = ops.iter().position(|o| o.contains("tlbu")).expect("the depth write");
            let colour = ops.iter().position(|o| o.contains("vfpack")).expect("a colour write");
            let thrsw: Vec<usize> = ops.iter().enumerate().filter(|(_, o)| o.contains("thrsw")).map(|e| e.0).collect();
            assert!(z < colour, "fragdepth.write.{name}: the depth before the colour\n{ops:#?}");
            /* after the last-thread-switch pair; the one THRSW past it is
             * the program end, three instructions from the last */
            assert!(
                thrsw.iter().filter(|&&t| t < z).count() >= 2,
                "fragdepth.write.{name}: the last-thread-switch pair\n{ops:#?}"
            );
            assert_eq!(
                thrsw.iter().filter(|&&t| t > z).copied().collect::<Vec<_>>(),
                [ops.len() - 3],
                "fragdepth.write.{name}: the depth after the last thread switch\n{ops:#?}"
            );
        });
    }
}

// ---- the written value, on the QPU interpreter -------------------------------

/// The TLB writes of `fs` for one quad of pixels whose `v_coord` is
/// `coords` (lane n: `coords[n]`), the uniforms `uf_half` = 0.5,
/// `ub_true` = true, `ui_two` = 2, `uf_fourth` = 0.25, `u_color` = (1, 0,
/// 0, 1), and the payload Z `z`.
fn run_fs(fs: &str, coords: [[f32; 2]; LANES], z: f32) -> Vec<TlbWrite> {
    with_program(VS_COORD, fs, |p, refl| {
        let ops = listing(&p.fs.code);
        /* The uniform block, laid out as the reflection says (dword
         * offsets): QUNIFORM_UNIFORM (contents 1) reads dword `data` of it;
         * a constant (contents 0) is its data. */
        let mut block = vec![0u32; usize::from(refl.uniform_dwords)];
        for (name, words) in [
            ("u_color", &[1.0f32.to_bits(), 0, 0, 1.0f32.to_bits()][..]),
            ("uf_half", &[0.5f32.to_bits()]),
            ("ub_true", &[1]),
            ("ui_two", &[2]),
            ("uf_fourth", &[0.25f32.to_bits()]),
        ] {
            let Some(row) = refl.uniforms.iter().find(|r| r.name == name) else { continue };
            for (k, w) in words.iter().enumerate().take(usize::from(row.dwords)) {
                block[usize::from(row.dword_off) + k] = *w;
            }
        }
        let uniforms: Vec<u32> =
            p.fs.code
                .uniforms
                .iter()
                .map(|u| match u.contents {
                    0 => u.data,
                    1 => *block.get(u.data as usize).unwrap_or_else(|| panic!("uniform dword {} of the block", u.data)),
                    c => panic!("uniform contents {c} not modelled by this test\n{ops:#?}"),
                })
                .collect();
        let one = [1.0f32.to_bits(); LANES];
        let zero = [0u32; LANES];
        /* v_coord is the one varying (VARYING_SLOT_VAR0 + location 0),
         * its components the shader reads in input order; W is 1 so the
         * smooth value is C + vary * W with C = 0 and vary = the value. A
         * shader that reads none has no input. */
        assert_eq!(p.fs.num_inputs as usize, p.fs.input_slots.len());
        let varyings =
            p.fs.input_slots
                .iter()
                .map(|&(slot, c)| {
                    assert!(slot == 32 && c < 2, "v_coord's components only: {:?}\n{ops:#?}", p.fs.input_slots);
                    Varying { vary: core::array::from_fn(|l| coords[l][usize::from(c)].to_bits()), c: zero }
                })
                .collect();
        let inputs = FsInputs {
            w: one,
            w_centroid: one,
            z: [z.to_bits(); LANES],
            x: zero,
            y: zero,
            back_facing: [false; LANES],
            varyings,
        };
        let env = Env { threads: p.fs.threads, ..Env::default() };
        sim::run(p.fs.code.words, &uniforms, &Inputs::Fragment(Box::new(inputs)), &env)
            .unwrap_or_else(|e| panic!("the fragment program runs: {e}\n{ops:#?}"))
            .tlb
    })
}

fn body_of(name: &str) -> String {
    let body = BODIES.iter().find(|b| b.0 == name).unwrap_or_else(|| panic!("{name}")).1;
    fragdepth(body)
}

/// The first TLB write is the depth, config `TLB_Z_PER_PIXEL`, one f32 per
/// lane; the two after it are the colour's f16 pairs through TLB.
fn depth_of(writes: &[TlbWrite]) -> [f32; LANES] {
    assert_eq!(writes.len(), 3, "the depth, then the colour's two words: {writes:#x?}");
    assert_eq!(writes[0].config, Some(TLB_Z_PER_PIXEL), "{writes:#x?}");
    assert!(writes[1].config.is_none() && writes[2].config.is_none(), "{writes:#x?}");
    core::array::from_fn(|l| f32::from_bits(writes[0].value[l].expect("a defined depth")))
}

fn coords(f: impl Fn(usize) -> [f32; 2]) -> [[f32; 2]; LANES] {
    core::array::from_fn(f)
}

#[test]
fn a_constant_depth_comes_out_as_written() {
    let w = run_fs(&body_of("const"), coords(|_| [0.0, 0.0]), 0.75);
    assert_eq!(depth_of(&w), [0.5; LANES]);
    assert_eq!(w[0].value, [Some(0x3f00_0000); LANES], "0.5 as f32, not f16, not the payload Z");
}

#[test]
fn a_uniform_depth_comes_out_as_written() {
    assert_eq!(depth_of(&run_fs(&body_of("uniform"), coords(|_| [0.0, 0.0]), 0.75)), [0.5; LANES]);
}

/// `(v_coord.x + v_coord.y) * 0.5`, per lane.
#[test]
fn a_varying_depth_comes_out_per_lane() {
    let c = coords(|l| [l as f32 / 16.0, 0.5]);
    let want: [f32; LANES] = core::array::from_fn(|l| (c[l][0] + c[l][1]) * 0.5);
    assert_eq!(depth_of(&run_fs(&body_of("dynamic"), c, 0.75)), want);
    assert_eq!(depth_of(&run_fs(&body_of("write_in_function"), c, 0.75)), want);
}

/// `gl_FragDepth = gl_FragCoord.z`: the payload Z, through the shader's own
/// per-pixel write (not the pass-through form; writes_z either way).
#[test]
fn the_payload_z_written_back_goes_through_the_per_pixel_write() {
    assert_eq!(depth_of(&run_fs(&body_of("fragcoord_z"), coords(|_| [0.0, 0.0]), 0.375)), [0.375; LANES]);
}

/// `if (v_coord.y < 0.5) gl_FragDepth = d; else gl_FragDepth = 1.0 - d;`:
/// the arms merge per lane (a select, or a uniform branch's merge).
#[test]
fn a_conditional_depth_takes_each_lanes_arm() {
    let c = coords(|l| [0.25, if l % 2 == 0 { 0.25 } else { 0.75 }]);
    let want: [f32; LANES] = core::array::from_fn(|l| {
        let d = (c[l][0] + c[l][1]) * 0.5;
        if c[l][1] < 0.5 {
            d
        } else {
            1.0 - d
        }
    });
    assert_eq!(depth_of(&run_fs(&body_of("dynamic_conditional_write"), c, 0.75)), want);
    /* `if (ub_true) gl_FragDepth = ...` with ub_true true: written */
    let c = coords(|l| [l as f32 / 16.0, 0.5]);
    let want: [f32; LANES] = core::array::from_fn(|l| (c[l][0] + c[l][1]) * 0.5);
    assert_eq!(depth_of(&run_fs(&body_of("uniform_conditional_write"), c, 0.75)), want);
}

/// `gl_FragDepth = 0.0; for (i < ui_two) gl_FragDepth += uf_fourth;` with
/// ui_two 2 and uf_fourth 0.25: the depth is carried round the counted loop
/// (prepare.rs, a phi as the discard flag's) and comes out 0.5.
#[test]
fn a_loop_carried_depth_comes_out_summed() {
    assert_eq!(depth_of(&run_fs(&body_of("uniform_loop_write"), coords(|_| [0.0, 0.0]), 0.75)), [0.5; LANES]);
}

/// A depth write with a discard: the discard's late tests, and still the
/// shader's own depth (one per-pixel write, no pass-through one); the
/// discarded lanes' multisample flags are 0.
#[test]
fn a_depth_write_with_a_discard_keeps_the_per_pixel_write() {
    let fs = fragdepth(
        "void main (void)\n{\n    if (v_coord.x > 0.5) discard;\n    o_color = u_color;\n    gl_FragDepth = v_coord.y;\n}",
    );
    with_program(VS_COORD, &fs, |p, _| {
        assert_eq!((p.fs.uses_discard, p.fs.writes_z, p.fs.disable_ez), (true, true, true));
        assert_eq!(tlbu_configs(&p.fs.code), [TLB_Z_PER_PIXEL]);
    });
    let c = coords(|l| [if l < 8 { 0.0 } else { 1.0 }, l as f32 / 16.0]);
    let w = run_fs(&fs, c, 0.75);
    let want: [f32; LANES] = core::array::from_fn(|l| c[l][1]);
    assert_eq!(depth_of(&w), want);
}

// ---- V3D 7.1 --------------------------------------------------------------------

/// Mesa's 7.1 form of the write is the same `mov tlbu, <depth>` with the
/// same specifier (nir_to_vir.c:2097-2108 is one `ver >= 42` branch): the
/// 7.1 interpreter records the config word and the value for a program that
/// writes the payload Z (rf2 on 7.1 too) with it. The chain emits 4.2 code
/// only (chain.rs `encode_program`), so this is the 7.1 half the interpreter
/// can vouch for.
#[test]
fn the_71_interpreter_records_the_per_pixel_depth_write() {
    fn thrsw() -> Alu71 {
        let mut a = Alu71::nop();
        a.sig.thrsw = true;
        a
    }
    /* `mov tlbu, rf2` on the mul ALU: waddr 8 magic is TLBU (isa71
     * magic_waddr_name) */
    let mut mov = Alu71::nop();
    mov.mul = MulAlu71 {
        op: MulOp71::Mov,
        a: Operand { raddr: 2, unpack: Unpack71::None },
        b: Operand::default(),
        waddr: 8,
        magic: true,
        pack: OutPack::None,
    };
    let code: Vec<u64> = [thrsw(), thrsw(), mov, thrsw(), Alu71::nop(), Alu71::nop()]
        .iter()
        .map(|a| encode71(&Instr71::Alu(*a)).unwrap_or_else(|e| panic!("{a:?}: {e:?}")))
        .collect();
    let z = 0.625f32;
    let f = |x: f32| [x.to_bits(); LANES];
    let inputs = Inputs::Fragment(Box::new(FsInputs {
        w: f(1.0),
        w_centroid: f(1.0),
        z: f(z),
        x: f(1.0),
        y: f(1.0),
        back_facing: [false; LANES],
        varyings: Vec::new(),
    }));
    let o = sim71::run71(&code, &[TLB_Z_PER_PIXEL], &inputs, &Env::default()).expect("runs");
    assert_eq!(o.tlb.len(), 1);
    assert_eq!(o.tlb[0].config, Some(TLB_Z_PER_PIXEL));
    assert_eq!(o.tlb[0].value, [Some(z.to_bits()); LANES]);
}
