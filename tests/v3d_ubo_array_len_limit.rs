// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! How long one uniform block member array can be on the V3D today.
//!
//! `UniformRow::array_len` is a `u8` (src/shaderc/online/reflect.rs:59, and
//! src/shaderc/mod.rs:948 for the copy the GL core reads). The front end
//! fills it with `fit_u8(count, ..)` (reflect.rs:813 for a block member,
//! reflect.rs:1120 for a default-block uniform; `fit_u8` is reflect.rs:1481),
//! and `fit_u8` fails with "too many of this kind of declaration" past 255.
//! So a block member array is at most 255 elements long, and a 16384-byte
//! block (1024 vec4) cannot declare `vec4[1024]`; it needs several members.
//!
//! This file pins, on the V3D build without report-es2 and on the host QPU sim
//! (the rig of tests/v3d_ubo_dynamic_index_limit.rs, constant indexes):
//!
//! * `vec4[255]` and `float[255]` in a block compile, the reflection holds the
//!   declared length (`array_len == 255`, the row's dwords 1020 or 255: a silent
//!   truncation would show there, which is what `GL_UNIFORM_SIZE` reads), and
//!   the first and last element read their dwords.
//! * `vec4[256]` and `vec4[1024]` are refused with the `fit_u8` text (the GL
//!   core's link log is the compile error). Nothing is accepted and cut short.
//! * `vec4[255] x 4 + vec4[4]` fills 1024 vec4 = 16384 bytes, and reads at
//!   the first and last dword.
//!
//! When `array_len` is widened the two refusal cases become reads (check
//! `array_len` against the declared length, 256 and 1024, and read the last
//! element); the rest stays as it is.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_ubo_array_len_limit
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use std::sync::Mutex;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::TLB_Z_PER_PIXEL;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, LANES};
use yakogl::shaderc::online::{lower, sema};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

const TARGET: Target = Target { vpm_size: 65536 };

const VS: &str = "#version 300 es\nin highp vec4 a_position;\nin highp vec2 a_coord;\nout highp vec2 v_coord;\nvoid main (void)\n{\n    gl_Position = a_position;\n    v_coord = a_coord;\n}\n";

/// The refusal of a count past a u8 (reflect.rs `fit_u8`).
const TOO_MANY: &str = "too many of this kind of declaration";

/// The distinct value of block dword `k`.
fn value(k: usize) -> f32 {
    k as f32 + 0.5
}

/// One uniform row as the reflection holds it: name, `array_len`, dwords.
type Row = (String, u8, u16);

/// The depth lane 0 writes for `fs`, and the reflection's rows, or why the
/// form is not run (compile error, a stream word the model does not carry).
fn run(fs: &str) -> Result<(f32, Vec<Row>), String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 64 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).expect("arena");
    let fs_ = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs_).map_err(|e| format!("lower: {}", e.msg))?;
    let p =
        compile_program(&v.module, &f.module, Prim::Triangles, TARGET, &a).map_err(|e| format!("compile: {e:?}"))?;
    let refl = &f.reflection;
    let block: Vec<u32> = (0..usize::from(refl.uniform_dwords)).map(|k| value(k).to_bits()).collect();
    let mut stream = Vec::new();
    for u in p.fs.code.uniforms.iter() {
        stream.push(match u.contents {
            0 => u.data,
            1 => *block.get(u.data as usize).ok_or(format!("stream dword {} past the block", u.data))?,
            c => return Err(format!("uniform contents {c} (UNIFA / UBO address) not modelled by this test")),
        });
    }
    let zero = [0u32; LANES];
    let one = [1.0f32.to_bits(); LANES];
    let varyings = p.fs.input_slots.iter().map(|_| Varying { vary: zero, c: zero }).collect();
    let inputs = FsInputs { w: one, w_centroid: one, z: zero, x: zero, y: zero, back_facing: [false; LANES], varyings };
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let o = sim::run(p.fs.code.words, &stream, &Inputs::Fragment(Box::new(inputs)), &env)
        .map_err(|e| format!("sim: {e}"))?;
    let w = o.tlb.iter().find(|w| w.config == Some(TLB_Z_PER_PIXEL)).ok_or("no depth write")?;
    let depth = f32::from_bits(w.value[0].ok_or("undefined depth")?);
    let rows = refl.uniforms.iter().map(|r| (r.name.to_string(), r.array_len, r.dwords)).collect();
    Ok((depth, rows))
}

/// A fragment shader with one std140 block of `members` (type, name, length),
/// writing `read` to `gl_FragDepth`.
fn fs(members: &[(&str, &str, usize)], read: &str) -> String {
    let body: String = members.iter().map(|(t, n, l)| format!("highp {t} {n}[{l}]; ")).collect();
    format!(
        "#version 300 es\nuniform Big {{ {body}}};\n\
         in highp vec2 v_coord;\nlayout(location = 0) out mediump vec4 o_color;\n\
         void main (void)\n{{\n    o_color = vec4(1.0);\n    gl_FragDepth = {read};\n}}\n"
    )
}

/// Reads `read`, checks it is `value(dword)`, and that every member's row
/// holds the declared length: `len` elements and `len * comps` dwords (the row
/// counts components: 4 for a vec4, 1 for a float; the block's std140 stride is
/// another matter, the reads check it).
fn read_ok(members: &[(&str, &str, usize)], read: &str, dword: usize) {
    let src = fs(members, read);
    let (got, rows) = run(&src).unwrap_or_else(|why| panic!("{read}: {why}\n{src}"));
    assert_eq!(got, value(dword), "{read} is dword {dword}\n{src}");
    for (ty, name, len) in members {
        let row = rows.iter().find(|r| r.0 == *name).unwrap_or_else(|| panic!("no row {name}: {rows:?}"));
        let comps = if *ty == "float" { 1 } else { 4 };
        assert_eq!((usize::from(row.1), usize::from(row.2)), (*len, *len * comps), "row {name}: a silent truncation?");
    }
}

/// The block is refused with the `fit_u8` text, not accepted cut short.
fn refused(members: &[(&str, &str, usize)]) {
    let src = fs(members, "m0[0].x");
    match run(&src) {
        Ok((v, rows)) => panic!("accepted ({v}), a silent truncation? rows {rows:?}\n{src}"),
        Err(why) => assert!(why.starts_with("lower: ") && why.contains(TOO_MANY), "log: {why}"),
    }
}

#[test]
fn vec4_255_first_and_last_element() {
    let m = [("vec4", "m0", 255)];
    read_ok(&m, "m0[0].x", 0);
    read_ok(&m, "m0[254].w", 254 * 4 + 3);
}

#[test]
fn float_255_first_and_last_element() {
    /* the compiler's own dword numbering (the rows' `dword_off`) is dense for
     * a float array: element e is dword e; the std140 stride is the GL core's
     * block table's business (reflect.rs block_table), not read here */
    let m = [("float", "m0", 255)];
    read_ok(&m, "m0[0]", 0);
    read_ok(&m, "m0[254]", 254);
}

#[test]
fn vec4_256_is_refused_not_cut_short() {
    refused(&[("vec4", "m0", 256)]);
}

#[test]
fn vec4_1024_is_refused_not_cut_short() {
    refused(&[("vec4", "m0", 1024)]);
}

/// 4 x vec4[255] + vec4[4] = 1024 vec4 = 16384 bytes, 4096 dwords.
#[test]
fn vec4_1024_as_four_255_and_a_4_reads_first_and_last_dword() {
    let m = [("vec4", "m0", 255), ("vec4", "m1", 255), ("vec4", "m2", 255), ("vec4", "m3", 255), ("vec4", "m4", 4)];
    read_ok(&m, "m0[0].x", 0);
    read_ok(&m, "m1[0].x", 255 * 4);
    read_ok(&m, "m3[254].w", 3 * 1020 + 254 * 4 + 3);
    read_ok(&m, "m4[0].x", 4 * 1020);
    read_ok(&m, "m4[3].w", 4095);
}
