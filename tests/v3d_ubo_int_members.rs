// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Do `int`, `uint`, `bool` and `ivec4` / `uvec4` members read right at the
//! very end of a 16384-byte std140 block on the V3D? The block is 1023 vec4
//! of padding (seven `vec4 m[128]` and one `vec4 m7[127]`: an array is at most
//! 255 long, `UniformRow::array_len` is a u8) and then the tail: `vec3 p; T t;`
//! (`t` at byte 16380, the last dword) for a scalar `T`, or `T t` at byte
//! 16368 for a four-component one. A fragment shader writes the tail to
//! `gl_FragDepth` (`float(t)`, or a `bool` as 0.25 / 0.75), and the QPU
//! interpreter runs it with a uniform stream built from the block (dword k =
//! k + 0.5 as f32, the tail dwords set by the case).
//!
//! Offsets: `Reflection::block_members` is the table behind
//! `glGetActiveUniformsiv(GL_UNIFORM_OFFSET)` and `GL_UNIFORM_BLOCK_DATA_SIZE`
//! (the GL core copies it); each member's offset is checked against std140
//! and the block's size against 16384 here, without a GL context.
//!
//! Probe, not a feature: a case that fails today is `#[ignore]`d with the
//! reason. Only the `contents == 1` (QUNIFORM_UNIFORM) stream words are
//! modelled.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_ubo_int_members
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

/// A block of `padding_vec4` vec4 as members `m0 ..` of at most 128, then
/// `tail_decl`, and `gl_FragDepth = depth_expr`.
fn fs(padding_vec4: usize, tail_decl: &str, depth_expr: &str) -> String {
    let mut members = String::new();
    let (mut left, mut m) = (padding_vec4, 0);
    while left > 0 {
        let n = left.min(128);
        members += &format!("highp vec4 m{m}[{n}]; ");
        left -= n;
        m += 1;
    }
    format!(
        "#version 300 es\nuniform Big {{ {members}{tail_decl} }};\n\
         in highp vec2 v_coord;\nlayout(location = 0) out mediump vec4 o_color;\n\
         void main (void)\n{{\n    o_color = vec4(1.0);\n    gl_FragDepth = {depth_expr};\n}}\n"
    )
}

/// The distinct value of block dword `k`.
fn value(k: usize) -> f32 {
    k as f32 + 0.5
}

/// Dwords one element of a member of GL type `t` takes in the default block
/// (the tail members and the vec4 padding only).
fn comps(t: u16) -> usize {
    match t {
        0x8B51 => 3,                   // FLOAT_VEC3
        0x8B52 | 0x8B55 | 0x8DC8 => 4, // FLOAT_VEC4 INT_VEC4 UNSIGNED_INT_VEC4
        _ => 1,
    }
}

/// The depth lane 0 wrote, the block's members as (name, std140 byte offset)
/// and the block's data size.
struct Out {
    depth: f32,
    members: Vec<(String, u32)>,
    size: u32,
}

/// Compile `fs`, run it with the last member's dwords set to `tail` and every
/// other dword k holding `value(k)`; or why the form is not run.
fn run(fs: &str, tail: &[u32]) -> Result<Out, String> {
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
    let desc = refl.blocks.first().ok_or("no block reflected")?;
    let first = usize::from(desc.first_member);
    let table = &refl.block_members[first..first + usize::from(desc.num_members)];
    let mut block = vec![0u32; usize::from(refl.uniform_dwords)];
    // Buffer dword -> default-block dword, member by member.
    let last = table.len() - 1;
    for (mi, m) in table.iter().enumerate() {
        let row = &refl.uniforms[usize::from(m.row)];
        let per = comps(m.gl_type);
        for e in 0..usize::from(m.array_size) {
            for (c, &t) in tail.iter().enumerate().take(per) {
                let buf = m.offset as usize / 4 + e * (m.array_stride as usize / 4) + c;
                let at = usize::from(row.dword_off) + e * per + c;
                block[at] = if mi == last { t } else { value(buf).to_bits() };
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
    Ok(Out {
        depth: f32::from_bits(w.value[0].ok_or("undefined depth")?),
        members: table.iter().map(|m| (m.name.to_string(), m.offset)).collect(),
        size: desc.data_size,
    })
}

/// The padding's offsets are std140's (`m{i}` after 128 * 16 * i bytes), the
/// tail `t` is at `tail_off`, and the block is 16384 bytes.
fn check_layout(o: &Out, tail_off: u32) {
    assert_eq!(o.size, 16384, "GL_UNIFORM_BLOCK_DATA_SIZE");
    let (t, pads) = o.members.split_last().unwrap();
    for (i, (name, off)) in pads.iter().enumerate() {
        if name.starts_with('m') {
            assert_eq!(*off, i as u32 * 128 * 16, "GL_UNIFORM_OFFSET of {name}");
        }
    }
    assert_eq!(t.0, "t");
    assert_eq!(t.1, tail_off, "GL_UNIFORM_OFFSET of the tail");
}

fn scalar_case(ty: &str, expr: &str, word: u32, want: f32) {
    let src = fs(1023, &format!("highp vec3 p; highp {ty} t;"), expr);
    let o = run(&src, &[word]).unwrap_or_else(|why| panic!("{why}\n{src}"));
    check_layout(&o, 16380);
    assert_eq!(o.depth, want, "{ty} tail, word {word:#x}");
}

fn vector_case(ty: &str, c: usize, word: [u32; 4], want: f32) {
    let src = fs(1023, &format!("highp {ty} t;"), &format!("float(t.{})", ["x", "y", "z", "w"][c]));
    let o = run(&src, &word).unwrap_or_else(|why| panic!("{why}\n{src}"));
    check_layout(&o, 16368);
    assert_eq!(o.depth, want, "{ty} tail, .{c}");
}

#[test]
fn int_last_dword_positive() {
    scalar_case("int", "float(t)", 4095, 4095.0);
}

#[test]
fn int_last_dword_negative() {
    scalar_case("int", "float(t)", (-12345i32) as u32, -12345.0);
}

#[test]
fn uint_last_dword() {
    scalar_case("uint", "float(t)", 0x00ff_ffff, 16777215.0);
}

#[test]
fn bool_last_dword_one_is_true() {
    scalar_case("bool", "t ? 0.25 : 0.75", 1, 0.25);
}

#[test]
fn bool_last_dword_zero_is_false() {
    scalar_case("bool", "t ? 0.25 : 0.75", 0, 0.75);
}

#[test]
fn bool_last_dword_any_nonzero_is_true() {
    scalar_case("bool", "t ? 0.25 : 0.75", 0x8000_0000, 0.25);
}

#[test]
fn ivec4_last_vec4_every_component() {
    let w = [(-7i32) as u32, 70000, (-80000i32) as u32, 4095];
    for (c, want) in [-7.0, 70000.0, -80000.0, 4095.0].into_iter().enumerate() {
        vector_case("ivec4", c, w, want);
    }
}

#[test]
fn uvec4_last_vec4_every_component() {
    let w = [1, 0x00ff_ffff, 65536, 4095];
    for (c, want) in [1.0, 16777215.0, 65536.0, 4095.0].into_iter().enumerate() {
        vector_case("uvec4", c, w, want);
    }
}
