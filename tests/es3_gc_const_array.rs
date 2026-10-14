// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A run-time index into a large constant array, on the GC7000UL.
//!
//! dEQP-GLES3 `shaders.large_constant_arrays.indexing.float_128_vertex` and
//! `vec4_128_vertex` (docs/es3-shaderc-coverage.md) were refused as
//! `Abi(LoadBalancingTempsUnmeasured { temps: 35 / 34, temps_max: 21 })`.
//! The front end reads `array[i]` as a chain `acc = (i == k) ? c_k : acc`
//! (lower.rs `select_among`), the constants in the constant file or as
//! immediates, and selection emits it compare, select, compare, select:
//! three temporaries whatever the length. `alloc::sink_definitions` then
//! moved each select past the next compare, so all 127 compares came first
//! and their results were live at once. The pass now leaves the chain in
//! order (its comment has the measurement).
//!
//! Not through the address register: `u[base][a.x]` read element 0 on the
//! i.MX8MP whatever the index (src/vivante/VIVANTE_NOTES.md, gl_test 108),
//! and Mesa etnaviv reads a large constant array from memory with `load`
//! (etnaviv_compiler_nir.c, `load_constant`), not through a0.
//!
//! The values are checked by running the ENCODED program in the
//! lane-faithful interpreter of `tests/common` for every index.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_gc_const_array
//! ```
#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

mod common;

use std::sync::Mutex;

use common::{run, Run};
use yakogl::shaderc::online::backend::{forget_compiled, SHC_ONLINE};
use yakogl::shaderc::online::vivante::isa::Instr;
use yakogl::shaderc::online::vivante::{decode, listing};
use yakogl::shaderc::{ShcShader, SHC_STAGE_VERTEX};

/// The compiler keeps its output in one arena that every compile resets.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// The dEQP corpus's longest arrays (large_constant_arrays.test).
const N: usize = 128;

/// Element `k` of a `float` (`lanes` 1) or `vec4` (`lanes` 4) array: values
/// no 20-bit immediate holds, so they go to the constant file as dEQP's do.
fn element(k: usize, lane: usize) -> f32 {
    ((k * 4 + lane) as f32 * 0.37 + 0.1).sin()
}

/// The GLSL text of an element, which parses back to exactly [`element`].
fn element_text(ty: &str, k: usize) -> String {
    if ty == "float" {
        format!("{:?}", element(k, 0))
    } else {
        let l: Vec<String> = (0..4).map(|j| format!("{:?}", element(k, j))).collect();
        format!("vec4({})", l.join(", "))
    }
}

/// The vertex shader of `indexing.<ty>_<n>_vertex`, as the corpus test
/// specializes a `both` case for its vertex stage (`specialize_vertex`): an
/// `int` input arrives as a float attribute, an output keeps its type.
fn deqp_vertex(ty: &str, n: usize) -> String {
    let mut s = String::from("#version 300 es\nprecision mediump float;\n\n");
    s += &format!("in highp vec4 dEQP_Position;\nin float a_in0;\nout {ty} out0;\n\n");
    s += &format!("void main()\n{{\n    const {ty} array[{n}] = {ty}[](\n");
    let els: Vec<String> = (0..n).map(|k| format!("    {}", element_text(ty, k))).collect();
    s += &els.join(",\n");
    s += ");\n\n    int in0 = int(a_in0);\n    out0 = array[in0];\n    gl_Position = dEQP_Position;\n}\n";
    s
}

/// One compiled vertex shader: what the record says, decoded.
struct Compiled {
    temps: u16,
    code: Vec<Instr>,
    /// The constant file, four dwords per register.
    consts: Vec<[u32; 4]>,
    in0_reg: u8,
    out0_reg: u8,
}

fn io_name(name: &[core::ffi::c_char; 32]) -> String {
    name.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

/// Compile through `SHC_ONLINE`, the path glCompileShader takes.
fn compile(src: &str) -> Result<Compiled, String> {
    let _serial = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    forget_compiled();
    let text = std::ffi::CString::new(src).unwrap();
    let mut sh = ShcShader::ZERO;
    let mut log = [0 as core::ffi::c_char; 512];
    let compile = SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(SHC_STAGE_VERTEX, text.as_ptr(), &mut sh, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect());
    }
    // SAFETY: the record's code, `usc_dw` dwords, alive until the next compile.
    let words = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
    let code = words.chunks(4).map(|c| decode::decode_instr(&c.try_into().unwrap()).expect("decodes")).collect();
    let mut consts = vec![[0u32; 4]; 512];
    for (k, (&kind, &data)) in sh.viv.const_contents().iter().zip(sh.viv.const_data()).enumerate() {
        assert!(
            matches!(kind, yakogl::shaderc::SHC_VIV_CONST_LITERAL | yakogl::shaderc::SHC_VIV_CONST_UNUSED),
            "a constant of kind {kind} in a shader with no uniform"
        );
        consts[k / 4][k % 4] = data;
    }
    let find = |io: &[yakogl::shaderc::ShcVivIo], name: &str| {
        io.iter().find(|x| io_name(&x.name) == name).map(|x| x.reg).unwrap_or_else(|| panic!("no {name}"))
    };
    Ok(Compiled {
        temps: sh.temps,
        in0_reg: find(&sh.viv.in_[..sh.viv.num_in as usize], "a_in0"),
        out0_reg: find(&sh.viv.out[..sh.viv.num_out as usize], "out0"),
        code,
        consts,
    })
}

fn op_names(c: &Compiled) -> Vec<String> {
    c.code
        .iter()
        .enumerate()
        .map(|(i, ins)| {
            let mut t = String::new();
            listing::instr_text(&mut t, i, ins).unwrap();
            t.split_whitespace().nth(5).unwrap_or("").to_string()
        })
        .collect()
}

/// The two programs dEQP-GLES3 refused on the GC7000 compile, and their
/// temporaries do not grow with the array: the 16-element array needs as
/// many as the 128-element one.
#[test]
fn the_deqp_128_element_vertex_programs_compile_in_as_many_temps_as_16() {
    for ty in ["float", "vec4"] {
        let small = compile(&deqp_vertex(ty, 16)).unwrap_or_else(|e| panic!("{ty}[16]: {e}"));
        let large = compile(&deqp_vertex(ty, N)).unwrap_or_else(|e| panic!("{ty}[{N}]: {e}"));
        assert_eq!(large.temps, small.temps, "{ty}: temporaries of [{N}] against [16]");
        assert!(large.temps <= 4, "{ty}[{N}]: {} temporaries", large.temps);
    }
}

/// The emitted chain keeps each compare next to its select: never more than
/// two compares in a row (the chain's head compares twice before its first
/// select), N - 1 of each in all, and no address register.
#[test]
fn the_chain_interleaves_compare_and_select() {
    for ty in ["float", "vec4"] {
        let c = compile(&deqp_vertex(ty, N)).unwrap_or_else(|e| panic!("{ty}[{N}]: {e}"));
        let ops = op_names(&c);
        let count = |p: &str| ops.iter().filter(|o| o.starts_with(p)).count();
        assert_eq!(count("cmp."), N - 1, "{ty}: compares in {ops:?}");
        assert_eq!(count("select."), N - 1, "{ty}: selects in {ops:?}");
        assert_eq!(count("movar") + count("movai"), 0, "{ty}: no address register");
        let mut run_len = 0;
        for o in &ops {
            run_len = if o.starts_with("cmp.") { run_len + 1 } else { 0 };
            assert!(run_len <= 2, "{ty}: {run_len} compares in a row: {ops:?}");
        }
    }
}

/// `out0 = array[in0]` for every in-range index, by running the encoded
/// program: each element comes back bit for bit.
#[test]
fn every_element_reads_back() {
    for (ty, lanes) in [("float", 1usize), ("vec4", 4)] {
        let c = compile(&deqp_vertex(ty, N)).unwrap_or_else(|e| panic!("{ty}[{N}]: {e}"));
        for k in 0..N {
            let mut regs = vec![[0u32; 4]; 128];
            regs[c.in0_reg as usize] = [(k as f32).to_bits(); 4];
            let out = match run(&c.code, &regs, &c.consts) {
                Run::Done { regs, .. } => regs[c.out0_reg as usize],
                Run::Unsupported(why) => panic!("{ty}: the interpreter cannot run it: {why}"),
            };
            for (lane, got) in out.iter().enumerate().take(lanes) {
                let want = element(k, lane);
                assert_eq!(f32::from_bits(*got), want, "{ty}[{k}] lane {lane}");
            }
        }
    }
}
