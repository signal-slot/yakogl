// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the two-plane join of a 128-bit texture (RGBA32F /
//! RGBA32UI / RGBA32I) under `texelFetch`, `textureLod`, `textureSize` and
//! the look-ups the compiler does not take
//! (docs/notes/gc-128bit-texture-sampling.md, docs/notes/gc-128bit-swizzle.md
//! section 5; `src/shaderc/online/vivante/wide128.rs`).
//!
//! Each shader is compiled plain, then through `wide128_variant` with a key
//! naming the 128-bit units and their companion slots, and the emitted
//! instructions are read back with the decoder:
//!
//! - `texelFetch` (`txf`) and `textureLod` (`texldl`) on `sampler2D`,
//!   `isampler2D`, `usampler2D`, `sampler3D`, `sampler2DArray` and the
//!   integer 3D / array types: two look-ups of the same opcode, the first on
//!   the unit, the second on the companion, both with the same coordinate and
//!   the same level operand, each writing `.xy`, then
//!   `mov tA.__zw, tB.xyxy` and the joined vec4 (xy from plane 0, zw from
//!   plane 1);
//! - `textureSize` is the plain shader to the word (the size is a uniform
//!   the driver loads; no look-up, so the companion is not needed);
//! - two 128-bit units and an ordinary one: each join on its own companion,
//!   the ordinary unit's look-up untouched, and a key for one unit splits
//!   only that unit's look-ups;
//! - `textureGrad`, `textureOffset`, `texelFetchOffset` and
//!   `textureLodOffset`: the online compiler does not select them yet, on
//!   any sampler, so a 128-bit unit never reaches the join's own refusals
//!   ("a gradient / an offset look-up on a 128-bit sampler is not split",
//!   `split_planes`). The compiler's refusal text is pinned instead; when the
//!   compiler takes them these cases fail and must move to pins of the
//!   joined form (the same offset or gradient on both look-ups).
//!
//! `textureGather` is out of scope (item gc-w128-gather). It needs no board
//! and no GL state: the key is the one the draw builds, and the draw's side
//! is in tests/es3_wide128_gc.rs and tests/es3_wide128_planes_gc.rs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_texfetch_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use std::sync::Mutex;
use yakogl::shaderc::online::backend::{forget_compiled, SHC_ONLINE};
use yakogl::shaderc::online::vivante::decode::decode_instr;
use yakogl::shaderc::online::vivante::listing::body_text;
use yakogl::shaderc::{release_shader, wide128_variant, LodKey, ShcShader, Wide128Planes, SHC_STAGE_FRAGMENT};

/// Held across every compile and every read of its code: the plane key is
/// process-wide, and `forget_compiled` resets the arena the code lives in.
static COMPILING: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    let g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    forget_compiled();
    g
}

fn text(log: &[core::ffi::c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

/// The fragment shader's compile as the GL core makes it, or the log.
fn try_compile(src: &str) -> Result<ShcShader, String> {
    let src = std::ffi::CString::new(src).unwrap();
    let mut sh = ShcShader::ZERO;
    let mut log = [0 as core::ffi::c_char; 512];
    let compile = SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(SHC_STAGE_FRAGMENT, src.as_ptr(), &mut sh, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(text(&log));
    }
    Ok(sh)
}

fn compile(src: &str) -> ShcShader {
    try_compile(src).unwrap_or_else(|e| panic!("{e}\n{src}"))
}

/// `sh` under `planes`, or the refusal's log.
fn variant(sh: &ShcShader, planes: Wide128Planes) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as core::ffi::c_char; 512];
    match wide128_variant(sh, planes, LodKey::NONE, &mut out, &mut log) {
        0 => Ok(out),
        _ => Err(text(&log)),
    }
}

fn listing(sh: &ShcShader) -> Vec<String> {
    sh.usc_words()
        .chunks(4)
        .map(|c| {
            let mut s = String::new();
            body_text(&mut s, &decode_instr(&c.try_into().unwrap()).expect("decodes")).expect("text");
            s
        })
        .collect()
}

/// One look-up of a listing: its mnemonic, unit, destination register and
/// the operands after the unit (coordinate, level, ...), as the listing
/// prints them.
#[derive(Debug, PartialEq)]
struct Look {
    op: String,
    unit: u32,
    dst: String,
    dst_mask: String,
    rest: String,
}

fn lookups(ws: &[String]) -> Vec<Look> {
    ws.iter()
        .filter(|t| {
            let m = t.split_whitespace().next().unwrap_or("");
            m == "txf" || m.starts_with("texld")
        })
        .map(|t| {
            let op = t.split_whitespace().next().unwrap().to_string();
            let after = t[op.len()..].trim_start();
            let (dst, rest) = after.split_once(", tex").expect("a unit operand");
            let (unit, rest) = rest.split_once('.').expect("a unit swizzle");
            let (reg, mask) = dst.split_once('.').unwrap_or((dst, ""));
            Look {
                op,
                unit: unit.parse().expect("a unit number"),
                dst: reg.to_string(),
                dst_mask: mask.to_string(),
                rest: rest.split_once(", ").expect("operands after the unit").1.to_string(),
            }
        })
        .collect()
}

const HEAD: &str = "#version 300 es\nprecision highp float;\nprecision highp int;\n\
    precision highp sampler3D;\nprecision highp sampler2DArray;\n\
    precision highp isampler2D;\nprecision highp usampler2D;\n\
    precision highp isampler3D;\nprecision highp usampler3D;\n\
    precision highp isampler2DArray;\nprecision highp usampler2DArray;\n\
    in vec2 uv;\n";

fn fs(decl: &str, out: &str, expr: &str) -> String {
    format!("{HEAD}{decl}\nout highp {out} o;\nvoid main() {{ o = {out}({expr}); }}\n")
}

/// Every sampler class of the 128-bit family and the result type it reads.
const CLASSES: [(&str, &str); 3] = [("sampler", "vec4"), ("isampler", "ivec4"), ("usampler", "uvec4")];

/// The split of a look-up listing, checked against its plain program:
/// two look-ups of the opcode `op`, the unit's on `.xy`, the companion's on
/// `.xy` with every other operand the same as the plain look-up's, then the
/// join `mov tA.__zw, tB.xyxy` and the final copy into the plain look-up's
/// register. Returns the split listing for further checks.
fn check_join(what: &str, plain: &ShcShader, split: &ShcShader, op: &str, unit: u32, companion: u32) {
    let (p, s) = (listing(plain), listing(split));
    let (pl, sl) = (lookups(&p), lookups(&s));
    assert_eq!(pl.len(), 1, "{what}: one plain look-up: {p:#?}");
    assert_eq!(sl.len(), 2, "{what}: two plane look-ups: {s:#?}");
    assert_eq!(pl[0].op, op, "{what}: {p:#?}");
    assert_eq!(pl[0].unit, unit, "{what}: {p:#?}");
    let (lo, hi) = (&sl[0], &sl[1]);
    assert_eq!((lo.op.as_str(), lo.unit), (op, unit), "{what}: the R,G plane on the unit: {s:#?}");
    assert_eq!((hi.op.as_str(), hi.unit), (op, companion), "{what}: the B,A plane on the companion: {s:#?}");
    assert_eq!(lo.rest, pl[0].rest, "{what}: the first look-up keeps the plain operands: {s:#?}");
    assert_eq!(hi.rest, lo.rest, "{what}: the same coordinate and level on both planes: {s:#?}");
    assert_eq!((lo.dst_mask.as_str(), hi.dst_mask.as_str()), ("xy__", "xy__"), "{what}: {s:#?}");
    assert_ne!(lo.dst, hi.dst, "{what}: the planes land in two registers: {s:#?}");
    let join = format!("{}.__zw, void, void, {}.xyxy", lo.dst, hi.dst);
    assert!(s.iter().any(|t| t.starts_with("mov") && t.contains(&join)), "{what}: {join}: {s:#?}");
    /* in the first look-up's own register, or copied there by a last mov */
    let last = s.last().unwrap();
    assert!(
        lo.dst == pl[0].dst
            || (last.starts_with("mov") && last.contains(&format!("{}.xyzw", lo.dst)) && last.contains(&pl[0].dst)),
        "{what}: the joined vec4 goes where the plain look-up's result went ({}): {s:#?}",
        pl[0].dst
    );
    /* nothing else of the plain program moved: its non-look-up words are
     * all still there, in order */
    let rest =
        |l: &[String]| l.iter().filter(|t| !t.starts_with("tex") && !t.starts_with("txf")).cloned().collect::<Vec<_>>();
    let (pr, sr) = (rest(&p), rest(&s));
    assert!(pr.iter().all(|t| sr.contains(t)), "{what}: the plain program's other words survive: {p:#?} / {s:#?}");
}

/// Compile `expr` over `decl` plain and under key unit 0 -> companion 5,
/// and check the join of one look-up with opcode `op`.
fn check(what: &str, decl: &str, out: &str, expr: &str, op: &str) {
    let plain = compile(&fs(decl, out, expr));
    let split = variant(&plain, Wide128Planes::NONE.with(0, 5)).unwrap_or_else(|e| panic!("{what}: {e}"));
    check_join(what, &plain, &split, op, 0, 5);
    release_shader(&split);
    release_shader(&plain);
}

/* ---- (a) texelFetch ------------------------------------------------------ */

/// `texelFetch(s, ivec2, lod)` on a 2D unit of each class: two `txf`, one per
/// plane, with the same integer coordinate and the same level (the `2.0`
/// immediate) and the xy|zw join.
#[test]
fn texel_fetch_2d_is_two_txf_joined() {
    let _g = serial();
    for (prefix, out) in CLASSES {
        let decl = format!("uniform highp {prefix}2D s;");
        for lod in ["0", "2"] {
            check(&format!("{prefix}2D lod {lod}"), &decl, out, &format!("texelFetch(s, ivec2(uv), {lod})"), "txf");
        }
    }
}

/// The level is an immediate on both look-ups: a level other than the
/// first's would be a different texel on the second plane.
#[test]
fn texel_fetch_level_is_the_same_immediate_on_both_planes() {
    let _g = serial();
    let plain = compile(&fs("uniform highp sampler2D s;", "vec4", "texelFetch(s, ivec2(uv), 2)"));
    let split = variant(&plain, Wide128Planes::NONE.with(0, 5)).expect("splits");
    let l = lookups(&listing(&split));
    assert_eq!(l.len(), 2);
    for x in &l {
        assert!(x.rest.contains("2.0:f20"), "{x:?}");
    }
    release_shader(&split);
    release_shader(&plain);
}

/// 3D and 2D-array fetches take the third integer coordinate on both
/// planes; the integer classes too.
#[test]
fn texel_fetch_3d_and_array_carry_the_layer_to_both_planes() {
    let _g = serial();
    for (prefix, out) in CLASSES {
        for dim in ["3D", "2DArray"] {
            let decl = format!("uniform highp {prefix}{dim} s;");
            check(&format!("{prefix}{dim}"), &decl, out, "texelFetch(s, ivec3(uv, 1), 1)", "txf");
        }
    }
}

/* ---- (b) textureLod, textureGrad ---------------------------------------- */

/// `textureLod` on 2D, 3D and 2D-array units of each class: two `texldl`
/// with the same coordinate register and the same level (`1.5:f20`).
#[test]
fn texture_lod_is_two_texldl_with_the_same_level() {
    let _g = serial();
    for (prefix, out) in CLASSES {
        let d2 = format!("uniform highp {prefix}2D s;");
        check(&format!("{prefix}2D"), &d2, out, "textureLod(s, uv, 1.5)", "texldl");
        for dim in ["3D", "2DArray"] {
            let d = format!("uniform highp {prefix}{dim} s;");
            check(&format!("{prefix}{dim}"), &d, out, "textureLod(s, vec3(uv, 0.5), 1.5)", "texldl");
        }
    }
    let plain = compile(&fs("uniform highp sampler2D s;", "vec4", "textureLod(s, uv, 1.5)"));
    let split = variant(&plain, Wide128Planes::NONE.with(0, 5)).expect("splits");
    for x in lookups(&listing(&split)) {
        assert!(x.rest.contains("1.5:f20"), "the level is on both look-ups: {x:?}");
    }
    release_shader(&split);
    release_shader(&plain);
}

/// A projective look-up repeats the divide for the second plane: the coordinate
/// of each look-up is its own divide result, both the same value.
#[test]
fn texture_proj_divides_for_each_plane() {
    let _g = serial();
    let plain = compile(&fs("uniform highp sampler2D s;", "vec4", "textureProj(s, vec3(uv, 1.0))"));
    let split = variant(&plain, Wide128Planes::NONE.with(0, 5)).expect("splits");
    let s = listing(&split);
    let l = lookups(&s);
    assert_eq!(l.iter().map(|x| (x.op.as_str(), x.unit)).collect::<Vec<_>>(), [("texld", 0), ("texld", 5)], "{s:#?}");
    assert_eq!(l[0].rest, l[1].rest, "{s:#?}");
    assert_eq!(s.iter().filter(|t| t.starts_with("rcp")).count(), 2, "the divide runs once a plane: {s:#?}");
    release_shader(&split);
    release_shader(&plain);
}

/// `textureGrad` is not selected by the online compiler on any sampler (so
/// no 128-bit unit reaches the join's own gradient refusal): the compiler's
/// refusal is the pin.
#[test]
fn texture_grad_is_refused_by_the_compiler() {
    let _g = serial();
    for (prefix, out) in CLASSES {
        let src = fs(&format!("uniform highp {prefix}2D s;"), out, "textureGrad(s, uv, vec2(0.1), vec2(0.1))");
        let e = try_compile(&src).err().expect("a gradient look-up is not selected");
        assert!(
            e.contains("a shadow sampler, an integer result, a gradient, an offset or a cube fetch"),
            "{prefix}: {e}"
        );
        assert!(e.contains("is not selected yet"), "{prefix}: {e}");
    }
}

/* ---- (c) offsets --------------------------------------------------------- */

/// `textureOffset`, `texelFetchOffset` and `textureLodOffset` are not
/// selected by the online compiler (any sampler): the compiler's refusal is
/// the pin, and the join never gets an offset to carry.
#[test]
fn offset_lookups_are_refused_by_the_compiler() {
    let _g = serial();
    for expr in [
        "textureOffset(s, uv, ivec2(1, 0))",
        "texelFetchOffset(s, ivec2(uv), 0, ivec2(1, 0))",
        "textureLodOffset(s, uv, 1.0, ivec2(1, 0))",
    ] {
        for (prefix, out) in CLASSES {
            let e = try_compile(&fs(&format!("uniform highp {prefix}2D s;"), out, expr))
                .err()
                .expect("an offset look-up is not selected");
            assert!(e.contains("a gradient, an offset or a cube fetch) is not selected yet"), "{expr}: {e}");
        }
    }
}

/* ---- (d) textureSize ----------------------------------------------------- */

/// `textureSize(s, 0)` is no look-up: under a key the shader is the plain
/// one, word for word, with no second unit, so the size is read from the
/// primary unit alone (the companion plane has the texture's own size).
#[test]
fn texture_size_reads_one_plane() {
    let _g = serial();
    for (prefix, _) in CLASSES {
        for (dim, expr) in [
            ("2D", "vec4(textureSize(s, 0), 0, 0)"),
            ("3D", "vec4(textureSize(s, 0), 0)"),
            ("2DArray", "vec4(textureSize(s, 0), 0)"),
        ] {
            let plain = compile(&fs(&format!("uniform highp {prefix}{dim} s;"), "vec4", expr));
            let split = variant(&plain, Wide128Planes::NONE.with(0, 5)).expect("splits");
            assert_eq!(listing(&split), listing(&plain), "{prefix}{dim}: nothing to split");
            assert!(lookups(&listing(&split)).is_empty(), "{prefix}{dim}");
            assert_eq!(split.usc_words(), plain.usc_words(), "{prefix}{dim}");
            release_shader(&split);
            release_shader(&plain);
        }
    }
}

/// A `textureSize` and a look-up on the same 128-bit unit: only the look-up
/// is split; the size stays a read of the unit's own size constant.
#[test]
fn texture_size_beside_a_lookup_splits_only_the_lookup() {
    let _g = serial();
    let plain =
        compile(&fs("uniform highp sampler2D s;", "vec4", "texelFetch(s, ivec2(uv), 0) * float(textureSize(s, 0).x)"));
    let split = variant(&plain, Wide128Planes::NONE.with(0, 5)).expect("splits");
    let (p, s) = (listing(&plain), listing(&split));
    assert_eq!(lookups(&p).len(), 1, "{p:#?}");
    let l = lookups(&s);
    assert_eq!(l.iter().map(|x| (x.op.as_str(), x.unit)).collect::<Vec<_>>(), [("txf", 0), ("txf", 5)], "{s:#?}");
    /* the size is the one i2f of the uniform's x (the register the
     * allocator gives it may differ) */
    let size = |l: &[String]| {
        l.iter().filter(|t| t.starts_with("i2f")).map(|t| t.split_once(", ").unwrap().1.to_string()).collect::<Vec<_>>()
    };
    assert_eq!(size(&p), ["u0.xxxx, void, void"], "{p:#?}");
    assert_eq!(size(&p), size(&s), "the size read is the same word: {s:#?}");
    release_shader(&split);
    release_shader(&plain);
}

/// The size is the texture's: a `textureSize` at a level other than the
/// literal 0 is the compiler's refusal (a pin on the compiler's text, as the
/// join cannot see it).
#[test]
fn texture_size_at_a_computed_level_is_refused_by_the_compiler() {
    let _g = serial();
    let e = try_compile(&fs("uniform highp sampler2D s;", "vec4", "vec4(textureSize(s, 1), 0, 0)"))
        .err()
        .expect("a non-literal-0 level");
    assert!(e.contains("textureSize at a level other than a literal 0 is not selected yet"), "{e}");
}

/* ---- (e) two 128-bit units and an ordinary one --------------------------- */

const THREE: &str = "uniform highp sampler2D s;\nuniform highp usampler2D t;\nuniform highp sampler2D p;";

/// `s` (unit 0) and `t` (unit 1) are 128-bit with companions 6 and 7, `p`
/// (unit 2) is an ordinary texture: look-ups come out as
/// s,s',t,t',p, each pair joined on its own companion, and `p`'s `txf`
/// is the plain program's.
#[test]
fn two_wide_units_and_an_ordinary_one_each_get_their_companion() {
    let _g = serial();
    let src =
        fs(THREE, "vec4", "texelFetch(s, ivec2(uv), 1) + vec4(textureLod(t, uv, 2.0)) + texelFetch(p, ivec2(uv), 1)");
    let plain = compile(&src);
    let p = lookups(&listing(&plain));
    let mut units: Vec<_> = p.iter().map(|x| (x.op.as_str(), x.unit)).collect();
    units.sort_by_key(|u| u.1);
    assert_eq!(units, [("txf", 0), ("texldl", 1), ("txf", 2)]);
    /* the split program has the plain look-ups and each wide unit's
     * companion (the scheduler may reorder them, so by unit) */
    let expect = |key: &[(u32, u32)]| {
        let mut v = Vec::new();
        for x in &p {
            v.push((x.op.clone(), x.unit));
            if let Some(&(_, c)) = key.iter().find(|k| k.0 == x.unit) {
                v.push((x.op.clone(), c));
            }
        }
        v.sort_by_key(|o| o.1);
        v
    };
    let key = Wide128Planes::NONE.with(0, 6).with(1, 7);
    let split = variant(&plain, key).expect("splits");
    let s = listing(&split);
    let l = lookups(&s);
    let order = l.iter().map(|x| (x.op.clone(), x.unit)).collect::<Vec<_>>();
    let mut order = order;
    order.sort_by_key(|o| o.1);
    assert_eq!(order, expect(&[(0, 6), (1, 7)]), "{s:#?}");
    for i in 0..l.len() {
        let companion = match l[i].unit {
            6 | 7 => continue,
            2 => {
                let q = p.iter().find(|q| q.unit == 2).unwrap();
                let imm = |r: &str| r.split_once(", ").unwrap().1.to_string();
                assert_eq!(imm(&l[i].rest), imm(&q.rest), "the ordinary look-up keeps its level: {s:#?}");
                assert_eq!(l[i].dst_mask, q.dst_mask, "and writes all lanes, unsplit: {s:#?}");
                continue;
            }
            0 => 6,
            _ => 7,
        };
        let (lo, hi) = (&l[i], &l[i + 1]);
        assert_eq!(hi.unit, companion, "{s:#?}");
        assert_eq!(hi.rest, lo.rest, "the same coordinate and level on the companion: {s:#?}");
        let join = format!("{}.__zw, void, void, {}.xyxy", lo.dst, hi.dst);
        assert!(s.iter().any(|t| t.starts_with("mov") && t.contains(&join)), "{join}: {s:#?}");
    }
    assert_eq!(s.iter().filter(|t| t.contains(".__zw, void, void")).count(), 2, "one join each: {s:#?}");
    release_shader(&split);

    /* a key for the second unit only: s and p stay as they were */
    let one = variant(&plain, Wide128Planes::NONE.with(1, 7)).expect("splits");
    let o = listing(&one);
    let order = lookups(&o).iter().map(|x| (x.op.clone(), x.unit)).collect::<Vec<_>>();
    let mut order = order;
    order.sort_by_key(|o| o.1);
    assert_eq!(order, expect(&[(1, 7)]), "{o:#?}");
    release_shader(&one);
    release_shader(&plain);
}

/// The same unit sampled twice with different looks: each look-up gets its
/// own pair on the one companion, none shares a result register.
#[test]
fn two_lookups_of_one_wide_unit_each_split() {
    let _g = serial();
    let plain =
        compile(&fs("uniform highp sampler2D s;", "vec4", "texelFetch(s, ivec2(uv), 0) + textureLod(s, uv, 1.0)"));
    let split = variant(&plain, Wide128Planes::NONE.with(0, 4)).expect("splits");
    let s = listing(&split);
    let order = lookups(&s).iter().map(|x| (x.op.clone(), x.unit)).collect::<Vec<_>>();
    let want = [("txf", 0), ("txf", 4), ("texldl", 0), ("texldl", 4)].map(|(a, b)| (a.to_string(), b));
    assert_eq!(order, want, "{s:#?}");
    assert_eq!(s.iter().filter(|t| t.contains(".__zw, void, void")).count(), 2, "{s:#?}");
    release_shader(&split);
    release_shader(&plain);
}

/// A companion slot a shader samples itself is refused by name, for a fetch
/// as for a `texture()` (docs/notes/gc-128bit-texture-sampling.md).
#[test]
fn a_companion_on_a_sampled_unit_is_refused_by_name() {
    let _g = serial();
    let plain = compile(&fs(
        "uniform highp sampler2D s;\nuniform highp sampler2D t;",
        "vec4",
        "texelFetch(s, ivec2(uv), 0) + texelFetch(t, ivec2(uv), 0)",
    ));
    let e = variant(&plain, Wide128Planes::NONE.with(0, 1)).err().expect("refused");
    assert!(e.contains("second plane is on a unit the shader samples"), "{e}");
    release_shader(&plain);
}
