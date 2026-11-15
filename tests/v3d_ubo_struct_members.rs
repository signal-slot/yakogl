// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! std140 structs and arrays of structs inside a 16384-byte uniform block on
//! the V3D. The block is
//!
//! ```text
//! struct T { vec3 v; float f; mat3 m; vec2 w; };   // std140: 80 bytes
//! uniform Big { vec4 pad[3]; T s; T a[203]; vec4 tail; };
//! ```
//!
//! 48 + 80 + 203 * 80 + 16 = 16384 bytes. (A struct is padded to a vec4, so
//! no `T` can end its last field exactly at byte 16384: `tail` is the last
//! dword, `a[202].w` the last struct's last field, ending at byte 16376.)
//!
//! Two probes: the block table the front end hands the GL core (data size,
//! offset, array stride and matrix stride of each member, against the std140
//! rules written out here, not taken from the compiler), and the QPU
//! interpreter reading the first and last dword and the first and last
//! struct fields by constant index from a block whose dword k holds k + 0.5.
//! The value reaches the TLB as `gl_FragDepth`.
//!
//! Probe, not a feature: a case that fails today is `#[ignore]`d with the
//! limiting code.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_ubo_struct_members
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

/// Elements of `a`: 48 + 80 + 80 * 203 + 16 = 16384.
const N: usize = 203;
const BLOCK_BYTES: u32 = 16384;
/// std140 size of `T`.
const T_SIZE: u32 = 80;

const DECL: &str = "struct T { highp vec3 v; highp float f; highp mat3 m; highp vec2 w; };\n";

/// A fragment shader with the block of `members` and `gl_FragDepth = expr`.
fn fs(members: &str, expr: &str) -> String {
    format!(
        "#version 300 es\n{DECL}uniform Big {{ {members} }};\n\
         in highp vec2 v_coord;\nlayout(location = 0) out mediump vec4 o_color;\n\
         void main (void)\n{{\n    o_color = vec4(1.0);\n    gl_FragDepth = {expr};\n}}\n"
    )
}

/// The full block of the header.
const FULL: &str = "highp vec4 pad[3]; T s; T a[203]; highp vec4 tail;";

/// (name, offset, array stride, matrix stride, array size) of a leaf.
type Leaf = (String, u32, u32, u32, u32);

/// The block's leaves of `FULL` by the std140 rules (ES 3.0 section
/// 2.12.6.4), worked out by hand.
fn expected() -> Vec<Leaf> {
    fn push_t(v: &mut Vec<Leaf>, name: String, base: u32) {
        v.push((format!("{name}.v"), base, 0, 0, 1));
        v.push((format!("{name}.f"), base + 12, 0, 0, 1));
        v.push((format!("{name}.m"), base + 16, 0, 16, 1));
        v.push((format!("{name}.w"), base + 64, 0, 0, 1));
    }
    let mut v = vec![("pad".to_string(), 0, 16, 0, 3)];
    push_t(&mut v, "s".into(), 48);
    for k in 0..N {
        push_t(&mut v, format!("a[{k}]"), 48 + T_SIZE + k as u32 * T_SIZE);
    }
    v.push(("tail".into(), BLOCK_BYTES - 16, 0, 0, 1));
    v
}

/// What the front end reflects for `src`: the block's data size and its
/// members.
fn table(src: &str) -> Result<(u32, Vec<Leaf>), String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let mut mem = vec![0u8; 64 << 20];
    let a = Arena::new(&mut mem);
    let fs_ = a.alloc_str(src).expect("arena");
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs_).map_err(|e| format!("lower: {}", e.msg))?;
    let r = &f.reflection;
    let size = r.blocks.first().ok_or("no block")?.data_size;
    let members = r
        .block_members
        .iter()
        .map(|m| (m.name.to_string(), m.offset, m.array_stride, m.matrix_stride, u32::from(m.array_size)))
        .collect();
    Ok((size, members))
}

/// The distinct value of std140 block dword `k`.
fn value(k: usize) -> f32 {
    k as f32 + 0.5
}

/// The depth lane 0 writes for `fs`, from a block whose std140 dword k is
/// `value(k)`, or why the form is not run.
fn run(fs: &str) -> Result<f32, String> {
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
    // The default-block image: each leaf's row takes its dwords from the
    // std140 buffer (a column of a matrix at its matrix stride).
    let mut block = vec![0u32; usize::from(refl.uniform_dwords)];
    for m in refl.block_members.iter() {
        let row = &refl.uniforms[usize::from(m.row)];
        // Packed row: a mat3 is 3 columns strided to 4 dwords, an array of
        // vectors `array_size` elements of `dwords / array_size`.
        let (cols, per_col) =
            if m.matrix_stride != 0 { (3, 3) } else { (1, usize::from(row.dwords) / usize::from(m.array_size)) };
        let col_dwords = if m.matrix_stride != 0 { 4 } else { per_col };
        for e in 0..usize::from(m.array_size) {
            for c in 0..cols {
                for i in 0..per_col {
                    let src = (m.offset + e as u32 * m.array_stride + c as u32 * m.matrix_stride) as usize / 4 + i;
                    let dst = usize::from(row.dword_off) + (e * cols + c) * col_dwords + i;
                    block[dst] = value(src).to_bits();
                }
            }
        }
    }
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
    Ok(f32::from_bits(w.value[0].ok_or("undefined depth")?))
}

fn read(members: &str, expr: &str, dword: usize) {
    let src = fs(members, expr);
    let got = run(&src).unwrap_or_else(|why| panic!("{expr}: {why}\n{src}"));
    assert_eq!(got, value(dword), "{expr}: std140 dword {dword}");
}

// ---- a struct member, no array of structs ----

/// `pad[3]` then `s` at byte 48: one struct, 128 bytes.
const SMALL: &str = "highp vec4 pad[3]; T s;";

#[test]
fn struct_member_table() {
    let (size, got) = table(&fs(SMALL, "s.f")).expect("a struct member reflects");
    assert_eq!(size, 48 + T_SIZE);
    let want: Vec<_> = expected().into_iter().take(1 + 4).collect();
    assert_eq!(got, want);
}

#[test]
fn struct_member_reads() {
    // s.v at byte 48 (dword 12), s.f at 60 (15), s.m starts at 64, its column 2 at 96 (24), s.w at 112 (28).
    read(SMALL, "s.v.x", 12);
    read(SMALL, "s.f", 15);
    read(SMALL, "s.m[2].z", 26);
    read(SMALL, "s.w.y", 29);
}

// ---- arrays of structs ----

/// The block of the header: data size 16384, and every leaf of `s` and of
/// each of the 203 elements of `a`, at its std140 offset and strides.
#[test]
fn full_block_table() {
    let (size, got) = table(&fs(FULL, "tail.x")).expect("the full block reflects");
    assert_eq!(size, BLOCK_BYTES);
    assert_eq!(got, expected());
}

/// The same table with 2 elements of `a` and nothing before it.
#[test]
fn array_of_structs_small_table() {
    let (size, got) = table(&fs("T a[2];", "a[1].f")).expect("an array of structs reflects");
    assert_eq!(size, 2 * T_SIZE);
    let want: Vec<_> =
        expected().into_iter().skip(5).take(8).map(|(n, o, s, m, c)| (n, o - 48 - T_SIZE, s, m, c)).collect();
    assert_eq!(got, want);
}

#[test]
fn full_block_first_and_last_dword() {
    read(FULL, "pad[0].x", 0);
    read(FULL, "tail.w", 4095);
}

#[test]
fn full_block_struct_fields() {
    // s at dword 12; a[k] at dword 32 + 20 k, so a[202] at 4072.
    read(FULL, "s.v.x", 12);
    read(FULL, "a[0].f", 32 + 3);
    read(FULL, "a[202].v.z", 32 + 20 * 202 + 2);
    read(FULL, "a[202].m[2].z", 32 + 20 * 202 + 4 + 8 + 2);
    read(FULL, "a[202].w.y", 32 + 20 * 202 + 16 + 1);
}
