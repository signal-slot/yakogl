// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the swizzle lane-source key of the 128-bit plane
//! join (docs/notes/gc-128bit-swizzle.md sections 4, 5.2 and 5.3), inert: the
//! GL core still keys the identity and refuses a swizzled binding
//! (`wide128_select` in src/gl/draw.rs), so these drive the compiler
//! directly, through the free functions of `yakogl::shaderc` as the GL core
//! calls them, on shaders the online back end compiled.
//!
//! - the identity key is the join as it was: the same code, and the same
//!   record (the compile-cache key is today's); `no_alpha` alone is the
//!   lane sources `[0,1,2,ONE]`;
//! - (B,G,R,A) reads lane 0 from the companion and lane 2 from the first
//!   look-up; (A,A,A,A) reads the companion only;
//! - ZERO and ONE are constants typed by the sampler: floats on `sampler2D`,
//!   integers 0 and 1 on `usampler2D` / `isampler2D`, never 0x3f800000;
//! - the nine dEQP texture.swizzle patterns, as a table;
//! - two units with different swizzles, and two keys of one unit, are
//!   independent records, a vertex-stage key likewise;
//! - `textureSize` and `texelFetch` on a swizzled unit.
//!
//! What it cannot see: the GPU, and the GL core's key (a later item).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_swizzle_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use std::sync::Mutex;
use yakogl::shaderc::online::backend::{forget_compiled, SHC_ONLINE};
use yakogl::shaderc::online::vivante::decode::decode_instr;
use yakogl::shaderc::online::vivante::wide128::{compile_planes, IDENTITY, ONE, ZERO};
use yakogl::shaderc::{
    release_shader, wide128_sensitivity, wide128_variant, LodKey, ShcShader, Wide128Planes, SHC_STAGE_FRAGMENT,
    SHC_STAGE_VERTEX, SHC_VIV_CONST_LITERAL,
};

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

/// The shader object's compile of `src`, as the GL core makes it.
fn compile(stage: u32, src: &str) -> ShcShader {
    let src = std::ffi::CString::new(src).unwrap();
    let mut sh = ShcShader::ZERO;
    let mut log = [0 as core::ffi::c_char; 512];
    let compile = SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, src.as_ptr(), &mut sh, log.as_mut_ptr(), log.len()) };
    assert_eq!(rc, 0, "{}", text(&log));
    sh
}

/// `sh` under `planes` (and no sampler-state key), or the refusal's log.
fn variant(sh: &ShcShader, planes: Wide128Planes) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as core::ffi::c_char; 512];
    match wide128_variant(sh, planes, LodKey::NONE, &mut out, &mut log) {
        0 => Ok(out),
        rc => {
            assert!(rc < 0, "{rc}");
            Err(text(&log))
        }
    }
}

/// The `TEX_ID` of every look-up of `sh`, in program order.
fn units(sh: &ShcShader) -> Vec<(&'static str, u32)> {
    sh.usc_words()
        .chunks(4)
        .map(|c| decode_instr(&c.try_into().unwrap()).expect("decodes"))
        .filter(|i| i.op().name.starts_with("tex") || i.op().name == "txf")
        .map(|i| (i.op().name, i.get("TEX_ID").expect("a look-up names its unit")))
        .collect()
}

fn fs(decl: &str, out: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nin vec2 uv;\n{decl}\nout highp {out} o;\nvoid main() {{ {body} }}\n"
    )
}

/// The instruction listing of `sh`, one line per instruction.
fn listing(sh: &ShcShader) -> Vec<String> {
    sh.usc_words()
        .chunks(4)
        .map(|c| {
            let mut s = String::new();
            yakogl::shaderc::online::vivante::listing::body_text(
                &mut s,
                &decode_instr(&c.try_into().unwrap()).expect("decodes"),
            )
            .expect("text");
            s
        })
        .collect()
}

/// GL's swizzle names as lane sources.
const R: u8 = 0;
const G: u8 = 1;
const B: u8 = 2;
const A: u8 = 3;

/// What a lane holds after the joined code ran: a channel of a plane (the
/// R,G plane's x,y; the B,A plane's x,y), or a literal word.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lane {
    Plane(u8, u8),
    Lit(u32),
}
use Lane::{Lit, Plane};

const F1: u32 = 0x3f80_0000;

/// The lanes the join must write for the lane sources `src`, `zero` and
/// `one` being the sampler's constants.
fn expect(src: [u8; 4], zero: u32, one: u32) -> [Lane; 4] {
    src.map(|s| match s {
        0..=3 => Plane(s / 2, s % 2),
        ZERO => Lit(zero),
        ONE => Lit(one),
        _ => unreachable!(),
    })
}

fn imm(tok: &str) -> u32 {
    let (v, ty) = tok.split_once(':').expect("an immediate");
    if ty.starts_with('f') {
        v.parse::<f32>().expect("a float").to_bits()
    } else {
        v.parse::<i32>().expect("an integer") as u32
    }
}

/// Run the listing of `sh` on symbolic texels and return `t1`, the result
/// register of a one-output shader: a look-up writes the lanes of its
/// destination mask from the plane its unit names (`unit` the R,G plane,
/// `companion` the B,A plane), a `mov` writes lane `k` of its mask from
/// `src.swizzle[k]`, a literal comes from the constant file or the
/// instruction. The coordinate conversion of a `texelFetch` is skipped.
fn lanes_of(sh: &ShcShader, unit: u32, companion: u32) -> [Lane; 4] {
    let consts: Vec<u32> = sh
        .viv
        .const_contents()
        .iter()
        .zip(sh.viv.const_data())
        .map(|(&k, &d)| if k == SHC_VIV_CONST_LITERAL { d } else { u32::MAX })
        .collect();
    let mut regs: std::collections::HashMap<String, [Option<Lane>; 4]> = Default::default();
    let comp = |c: char| "xyzw".find(c).unwrap();
    for line in listing(sh) {
        let f: Vec<&str> = line.split(", ").collect();
        let (op, dst) = f[0].split_once(' ').expect("an operand");
        let (op, dst) = (op.trim(), dst.trim());
        let (reg, mask) = dst.split_once('.').unwrap_or((dst, "xyzw"));
        let mask: Vec<usize> = mask.chars().filter(|c| *c != '_').map(comp).collect();
        let mut new = regs.get(reg).copied().unwrap_or([None; 4]);
        match op {
            "texld" | "texldl" | "txf" => {
                let n: u32 = f[1].split('.').next().unwrap().trim_start_matches("tex").parse().unwrap();
                let plane = if n == unit {
                    0
                } else if n == companion {
                    1
                } else {
                    panic!("a look-up on unit {n}: {line}")
                };
                for &k in &mask {
                    new[k] = Some(Plane(plane, k as u8));
                }
            }
            "mov" => {
                let src = f[3];
                for &k in &mask {
                    new[k] = Some(if src.contains(':') {
                        Lit(imm(src))
                    } else {
                        let (r, sw) = src.split_once('.').unwrap_or((src, "xyzw"));
                        let c = comp(sw.chars().nth(k).unwrap());
                        if let Some(u) = r.strip_prefix('u') {
                            Lit(consts[4 * u.parse::<usize>().unwrap() + c])
                        } else {
                            regs[r][c].unwrap_or_else(|| panic!("{r} read before it is written: {line}"))
                        }
                    });
                }
            }
            _ => continue,
        }
        regs.insert(reg.to_string(), new);
    }
    regs["t1"].map(|l| l.expect("the result is written"))
}

fn sampler_fs(sampler: &str, out: &str, body: &str) -> String {
    fs(&format!("uniform highp {sampler} s;"), out, body)
}

/// Every sampler kind the join serves: (declaration, output type, the
/// constant zero's word, the constant one's word).
const KINDS: [(&str, &str, u32, u32); 3] =
    [("sampler2D", "vec4", 0, F1), ("usampler2D", "uvec4", 0, 1), ("isampler2D", "ivec4", 0, 1)];

/// The identity key is the join as it was, record and code: naming (R,G,B,A)
/// explicitly changes nothing, and no alpha alone is the lane sources
/// `[0,1,2,ONE]`.
#[test]
fn the_identity_key_is_the_join_as_it_was() {
    let _g = serial();
    for (sampler, out, _, one) in KINDS {
        let sh = compile(SHC_STAGE_FRAGMENT, &sampler_fs(sampler, out, "o = texture(s, uv);"));
        let plain = variant(&sh, Wide128Planes::NONE.with(0, 5)).expect("compiles");
        let ident = variant(&sh, Wide128Planes::NONE.with_swizzle(0, 5, IDENTITY)).expect("compiles");
        assert_eq!(ident.priv_, plain.priv_, "{sampler}: the compile-cache key is today's");
        assert_eq!(ident.usc_words(), plain.usc_words(), "{sampler}");
        assert_eq!(listing(&ident), listing(&plain), "{sampler}");
        assert_eq!(lanes_of(&ident, 0, 5), expect(IDENTITY, 0, one), "{sampler}");
        assert_eq!(units(&ident), [("texld", 0), ("texld", 5)], "{sampler}");
        assert!(compile_planes().is_empty(), "{sampler}: the key is put back");

        /* no alpha alone: the lane sources [R,G,B,ONE], the code the same */
        let na = variant(&sh, Wide128Planes::NONE.with(0, 5).without_alpha(0)).expect("compiles");
        let one_key = variant(&sh, Wide128Planes::NONE.with_swizzle(0, 5, [R, G, B, ONE])).expect("compiles");
        assert_eq!(one_key.usc_words(), na.usc_words(), "{sampler}");
        assert_eq!(lanes_of(&na, 0, 5), expect([R, G, B, ONE], 0, one), "{sampler}");
        assert_ne!(na.priv_, plain.priv_, "{sampler}");
        assert_ne!(one_key.priv_, plain.priv_, "{sampler}: a non-identity key is its own record");
        for s in [plain, ident, na, one_key, sh] {
            release_shader(&s);
        }
    }
}

/// (B,G,R,A): lane 0 is the companion's `.x`, lane 2 the first look-up's
/// `.x`; (A,A,A,A) reads the companion only, so the first look-up is gone.
#[test]
fn bgra_crosses_the_planes_and_all_alpha_reads_the_companion() {
    let _g = serial();
    for (sampler, out, zero, one) in KINDS {
        let sh = compile(SHC_STAGE_FRAGMENT, &sampler_fs(sampler, out, "o = texture(s, uv);"));
        let bgra = variant(&sh, Wide128Planes::NONE.with_swizzle(0, 5, [B, G, R, A])).expect("compiles");
        assert_eq!(lanes_of(&bgra, 0, 5), [Plane(1, 0), Plane(0, 1), Plane(0, 0), Plane(1, 1)], "{sampler}");
        assert_eq!(lanes_of(&bgra, 0, 5), expect([B, G, R, A], zero, one), "{sampler}");
        assert_eq!(units(&bgra), [("texld", 0), ("texld", 5)], "{sampler}");
        let all_a = variant(&sh, Wide128Planes::NONE.with_swizzle(0, 5, [A; 4])).expect("compiles");
        assert_eq!(lanes_of(&all_a, 0, 5), [Plane(1, 1); 4], "{sampler}");
        assert_eq!(units(&all_a), [("texld", 5)], "{sampler}: only the B,A plane is read");
        assert_ne!(bgra.priv_, all_a.priv_, "{sampler}");
        for s in [bgra, all_a, sh] {
            release_shader(&s);
        }
    }
}

/// ZERO and ONE are constants of the sampler's type: float words on a
/// `sampler2D`, the integers 0 and 1 on `usampler2D` / `isampler2D`, and the
/// float one's bit pattern never reaches an integer sampler.
#[test]
fn zero_and_one_are_typed_by_the_sampler() {
    let _g = serial();
    for (sampler, out, zero, one) in KINDS {
        let sh = compile(SHC_STAGE_FRAGMENT, &sampler_fs(sampler, out, "o = texture(s, uv);"));
        let v = variant(&sh, Wide128Planes::NONE.with_swizzle(0, 5, [ZERO, ONE, ZERO, ONE])).expect("compiles");
        assert_eq!(lanes_of(&v, 0, 5), [Lit(zero), Lit(one), Lit(zero), Lit(one)], "{sampler}");
        assert!(units(&v).is_empty(), "{sampler}: no look-up reads a plane: {:#?}", listing(&v));
        let words: Vec<u32> = v
            .viv
            .const_contents()
            .iter()
            .zip(v.viv.const_data())
            .filter(|(k, _)| **k == SHC_VIV_CONST_LITERAL)
            .map(|(_, d)| *d)
            .collect();
        assert_eq!(words.contains(&F1), sampler == "sampler2D", "{sampler}: {words:x?}");
        /* mixed with a plane lane, the constant is an instruction operand */
        let m = variant(&sh, Wide128Planes::NONE.with_swizzle(0, 5, [R, G, B, ONE])).expect("compiles");
        let l = listing(&m);
        let imm = if sampler == "sampler2D" { "1.0:f20" } else { "1:s20" };
        assert!(l.iter().any(|t| t.starts_with("mov") && t.contains(imm)), "{sampler}: {l:#?}");
        assert!(l.iter().all(|t| !t.contains("1065353216")), "{sampler}: {l:#?}");
        for s in [v, m, sh] {
            release_shader(&s);
        }
    }
}

/// The nine `swizzleCases` of dEQP's es3fTextureSwizzleTests.cpp:287-295
/// (docs/notes/gc-texture-swizzle-wrap.md section 6, case 3), through the
/// key: each lane's source in the emitted code, on all three sampler kinds.
#[test]
fn the_nine_deqp_swizzles_through_the_key() {
    let _g = serial();
    const PATTERNS: [([u8; 4], &str); 9] = [
        ([R; 4], "all_red"),
        ([G; 4], "all_green"),
        ([B; 4], "all_blue"),
        ([A; 4], "all_alpha"),
        ([ZERO; 4], "all_zero"),
        ([ONE; 4], "all_one"),
        ([B, G, R, A], "bgra"),
        ([A, B, G, R], "abgr"),
        ([ONE, ONE, R, G], "one_one_red_green"),
    ];
    for (sampler, out, zero, one) in KINDS {
        let sh = compile(SHC_STAGE_FRAGMENT, &sampler_fs(sampler, out, "o = texture(s, uv);"));
        let mut records = Vec::new();
        for (sw, name) in PATTERNS {
            let v = variant(&sh, Wide128Planes::NONE.with_swizzle(0, 5, sw)).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(lanes_of(&v, 0, 5), expect(sw, zero, one), "{sampler} {name}");
            /* a plane is read only where a lane names it */
            let want: Vec<(&str, u32)> = [(0, 0), (5, 2)]
                .into_iter()
                .filter(|(_, lo)| sw.iter().any(|s| (*lo..*lo + 2).contains(s)))
                .map(|(u, _)| ("texld", u))
                .collect();
            assert_eq!(units(&v), want, "{sampler} {name}");
            records.push(v);
        }
        for (k, a) in records.iter().enumerate() {
            for b in &records[k + 1..] {
                assert_ne!(a.priv_, b.priv_, "{sampler}: two swizzles are two records");
            }
        }
        for s in records {
            release_shader(&s);
        }
        release_shader(&sh);
    }
}

/// Two units in one draw with different swizzles are independent, and two
/// draws of one program with different swizzles on a unit are two records,
/// the same swizzle one.
#[test]
fn two_units_carry_their_own_swizzles() {
    let _g = serial();
    let sh = compile(
        SHC_STAGE_FRAGMENT,
        &fs(
            "uniform highp sampler2D a;\nuniform highp usampler2D b;",
            "vec4",
            "o = texture(a, uv) + vec4(texture(b, uv));",
        ),
    );
    let key = |a: [u8; 4], b: [u8; 4]| Wide128Planes::NONE.with_swizzle(0, 6, a).with_swizzle(1, 7, b);
    let ab = variant(&sh, key([B, G, R, A], [A, A, A, A])).expect("compiles");
    let mut u = units(&ab);
    u.sort();
    assert_eq!(u, [("texld", 0), ("texld", 6), ("texld", 7)], "{:#?}", listing(&ab));
    let again = variant(&sh, key([B, G, R, A], [A, A, A, A])).expect("compiles");
    assert_eq!(again.priv_, ab.priv_, "the same swizzles are one record");
    let other_b = variant(&sh, key([B, G, R, A], [ZERO, ONE, R, G])).expect("compiles");
    let other_a = variant(&sh, key([R, G, B, A], [A, A, A, A])).expect("compiles");
    assert_ne!(other_b.priv_, ab.priv_, "unit 1's swizzle is part of the key");
    assert_ne!(other_a.priv_, ab.priv_, "unit 0's swizzle is part of the key");
    assert_ne!(other_a.priv_, other_b.priv_);
    assert_ne!(other_b.usc_words(), ab.usc_words());
    /* unit 1 under (A,A,A,A) reads only its B,A plane (7), unit 0 both (0, 6) */
    let ident = variant(&sh, key(IDENTITY, IDENTITY)).expect("compiles");
    let plain = variant(&sh, Wide128Planes::NONE.with(0, 6).with(1, 7)).expect("compiles");
    assert_eq!(ident.priv_, plain.priv_, "two identity units are today's record");
    assert!(compile_planes().is_empty());
    for s in [ab, again, other_a, other_b, ident, plain, sh] {
        release_shader(&s);
    }
}

/// A vertex stage has its own key: a swizzle there reaches the vertex
/// look-up (an explicit-level one).
#[test]
fn a_vertex_stage_key_carries_its_own_swizzle() {
    let _g = serial();
    for (sampler, out, zero, one) in KINDS {
        let flat = if sampler == "sampler2D" { "" } else { "flat " };
        let vs = compile(
            SHC_STAGE_VERTEX,
            &format!(
                "#version 300 es\nuniform highp {sampler} s;\nin vec4 p;\n{flat}out highp {out} v;\n\
                 void main() {{ v = textureLod(s, p.xy, 0.0); gl_Position = p; }}\n"
            ),
        );
        let ident = variant(&vs, Wide128Planes::NONE.with(0, 3)).expect("compiles");
        assert_eq!(lanes_of(&ident, 0, 3), expect(IDENTITY, zero, one), "{sampler}");
        let bgra = variant(&vs, Wide128Planes::NONE.with_swizzle(0, 3, [B, G, R, A])).expect("compiles");
        assert_eq!(lanes_of(&bgra, 0, 3), expect([B, G, R, A], zero, one), "{sampler}");
        assert_eq!(units(&bgra), [("texldl", 0), ("texldl", 3)], "{sampler}");
        let all_a = variant(&vs, Wide128Planes::NONE.with_swizzle(0, 3, [A; 4])).expect("compiles");
        assert_eq!(units(&all_a), [("texldl", 3)], "{sampler}");
        let mixed = variant(&vs, Wide128Planes::NONE.with_swizzle(0, 3, [B, G, R, ONE])).expect("compiles");
        assert_eq!(lanes_of(&mixed, 0, 3), expect([B, G, R, ONE], zero, one), "{sampler}");
        for (a, b) in [(&ident, &bgra), (&bgra, &all_a), (&bgra, &mixed)] {
            assert_ne!(a.priv_, b.priv_, "{sampler}");
        }
        for s in [ident, bgra, all_a, mixed, vs] {
            release_shader(&s);
        }
    }
}

/// `textureSize` reads the primary unit and is not keyed: a shader that only
/// asks the size has no sensitivity and its variant under a swizzle is its
/// own code. `texelFetch` is a look-up like `texture`: split and swizzled
/// the same way.
#[test]
fn texture_size_is_untouched_and_texel_fetch_is_swizzled() {
    let _g = serial();
    let size =
        compile(SHC_STAGE_FRAGMENT, &sampler_fs("sampler2D", "vec4", "o = vec4(vec2(textureSize(s, 0)), 0.0, 1.0);"));
    assert_eq!(wide128_sensitivity(&size), 0);
    let v = variant(&size, Wide128Planes::NONE.with_swizzle(0, 5, [A, B, G, R])).expect("compiles");
    assert_eq!(v.usc_words(), size.usc_words(), "the size is as the size was");
    assert!(units(&v).is_empty());

    for (sampler, out, zero, one) in KINDS {
        let body = if sampler == "sampler2D" {
            "o = texelFetch(s, ivec2(uv), 0);".to_string()
        } else {
            format!("o = {out}(texelFetch(s, ivec2(uv), 0));")
        };
        let sh = compile(SHC_STAGE_FRAGMENT, &sampler_fs(sampler, out, &body));
        assert_eq!(wide128_sensitivity(&sh), 1, "{sampler}");
        for sw in [IDENTITY, [B, G, R, A], [A, B, G, R], [ONE, ZERO, R, G]] {
            let t = variant(&sh, Wide128Planes::NONE.with_swizzle(0, 5, sw)).expect("compiles");
            assert_eq!(lanes_of(&t, 0, 5), expect(sw, zero, one), "{sampler} {sw:?}: {:#?}", listing(&t));
            assert!(units(&t).iter().all(|(op, _)| *op == "txf"), "{sampler} {sw:?}");
            release_shader(&t);
        }
        release_shader(&sh);
    }
    /* the size beside a swizzled fetch in one shader: its conversion is
     * unchanged by the key */
    let both = compile(
        SHC_STAGE_FRAGMENT,
        &sampler_fs("sampler2D", "vec4", "o = texelFetch(s, ivec2(uv), 0) + vec4(vec2(textureSize(s, 0)), 0.0, 1.0);"),
    );
    let plain = variant(&both, Wide128Planes::NONE.with(0, 5)).expect("compiles");
    let swz = variant(&both, Wide128Planes::NONE.with_swizzle(0, 5, [A, B, G, R])).expect("compiles");
    let pick = |sh: &ShcShader| -> Vec<String> { listing(sh).into_iter().filter(|t| t.starts_with("i2f")).collect() };
    assert_eq!(pick(&plain), pick(&swz));
    assert_eq!(pick(&plain).len(), 1);
    for s in [v, size, both, plain, swz] {
        release_shader(&s);
    }
}
