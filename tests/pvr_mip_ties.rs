// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Where the pack's tie rule decides a byte of the PowerVR's three-levels-a-
//! render mip chain (`pvr_backend_set_mip_three_levels`, off by default;
//! docs/notes/pvr-mip-three-levels.md), on the host.
//!
//! The note's first Open question: does the 8-bit pack (`pck.u8888`, round
//! to nearest) round a tie up, as `Q = floor(v * 255 + 0.5)` in
//! tools/shaders/internal/es/mipgen3.frag does, or to even? And does the
//! TPU's bilinear equal the ALU's on odd steps? A board read-back can only
//! answer the first at texels whose value lies exactly on a tie, so this
//! file counts them and pins the chain's bytes under each rule.
//!
//! The model is tests/pvr_mip_three_levels.rs's (docs/notes/pvr-mip-tail-e1.md
//! appendix B's): level 0 is `smooth` (gradients plus noise of 0..6) or
//! `random` (xorshift bytes), 800x600 (terrain's window and case 216's
//! chain) or 256x256. The TPU's sample is ES 3.0.6 3.8.10's four-weight sum
//! over value / 255 in f32. A level kick (and a render's first level, `o0`)
//! is the pack of that sample. An ALU level (`o1`, `o2`: levels 3, 4, 6, 7, 9
//! of 800x600 and 3, 4, 6, 7 of 256x256) is mipgen3.frag's arithmetic written
//! out here in f32, as the IR evaluator runs it: the leaves are `Q` of the
//! TPU's sample, `mix(a, b, t)` is `t * (b - a) + a` with two roundings
//! (src/shaderc/online/builtin_call.rs), and each level is `Q` of the two
//! `mix`es over the 0..255 values of the level above. The test does not need
//! the online compiler: under the round-half-up rule this model gives every
//! hash tests/pvr_mip_three_levels.rs pins with the evaluator (`EVALUATED`
//! below), and the test checks mipgen3.frag still writes `Q` as it did.
//!
//! Three arms, each a (pack rule, the program's `Q` rule):
//!
//! - `up` (up, up): the host model; mipgen3.frag as written on a pack that
//!   rounds ties up.
//! - `even` (even, even): a pack that rounds ties to even, with a `Q` that
//!   follows it (`roundEven(v * 255.0)`), the macro part (b) would take.
//! - `mixed` (even, up): mipgen3.frag as written on a pack that rounds ties
//!   to even. The leaves of `o1` and `o2` are then not the bytes `o0` stores
//!   wherever a sample is a tie.
//!
//! Per level, the table pins: the texels where the level's own rounding sees
//! a tie in some channel (the pack of the sample for a TPU level, the last
//! `Q` for an ALU level, in the `up` arm), the FNV-1a 64 of the level's bytes
//! in each arm, and how many texels differ from `up` in `even` and in
//! `mixed`. The ALU outputs are n / 255 in f32, and n / 255 * 255 is never on
//! a tie, so the pack of an ALU output never decides a byte; the test checks
//! that too. Ties are common: about half of level 1's texels (36867 of
//! 120000 on 800x600 smooth) and up to two thirds of an ALU level's,
//! and in `even` many of them move by 1. The model's f32 sample
//! need not tie where the board's fixed-point filter does, so the board
//! texels to compare are the `TIE_PROBES` below and the whole-level hashes.
//!
//! **What a board read-back should probe.** Upload a level 0 the board and
//! this file agree on (`level0`), glGenerateMipmap, read each level back
//! through a framebuffer, and compare:
//!
//! - Level 1 and 2 against the `up` and `even` hashes: these are TPU samples
//!   (`o0` and a level kick, the same with the knob off or on), so they
//!   read the pack's rule with the knob off (arm A) already. On the 256x256
//!   chain every even step's weights are 1/4; a texel ties when the four
//!   bytes sum to 2 mod 4 and the f32 sum lands on it. `TIE_PROBES` pins
//!   the first texels per level where `up` and `even` differ: those, and
//!   only those, tell the rules apart.
//! - The ALU levels (3, 4, 6, 7, 9) with the knob on (arm E), against `up`
//!   and `mixed`: if the pack rounds to even, these levels read `mixed`, and
//!   `Q` must become `roundEven` (the `even` arm) for the chain to equal the
//!   knob-off chain at those texels. A level that reads neither tells the
//!   second question (the TPU's bilinear is not the ALU's); the note's
//!   `MAX_D` = 1 bounds that move.
//!
//! How the pinned values were generated: once, from this commit's code, by
//! running the test with `PINNED` empty (it then prints the tables instead
//! of asserting, `-- --nocapture`) and pasting the output.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test pvr_mip_ties
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr"))]

const MIPGEN3: &str = include_str!("../tools/shaders/internal/es/mipgen3.frag");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Rule {
    /// round half up: floor(x + 0.5)
    Up,
    /// round half to even
    Even,
}

/// An arm: the pack's rule (TPU levels) and the program's `Q` rule (the
/// leaves and the ALU levels).
#[derive(Clone, Copy)]
struct Arm {
    name: &'static str,
    pack: Rule,
    q: Rule,
}

const ARMS: [Arm; 3] = [
    Arm { name: "up", pack: Rule::Up, q: Rule::Up },
    Arm { name: "even", pack: Rule::Even, q: Rule::Even },
    Arm { name: "mixed", pack: Rule::Even, q: Rule::Up },
];

/// `x` (0..255 units) on a tie.
fn tie(x: f32) -> bool {
    x - x.floor() == 0.5
}

/// `x` (0..255 units) to a byte by `r`.
fn round(r: Rule, x: f32) -> u8 {
    let y = match r {
        Rule::Up => (x + 0.5).floor(),
        Rule::Even => x.round_ties_even(),
    };
    y.clamp(0.0, 255.0) as u8
}

/// One RGBA8 level, its GL extent, tightly packed.
#[derive(Clone)]
struct Level {
    w: u32,
    h: u32,
    px: Vec<[u8; 4]>,
}

impl Level {
    fn at(&self, x: i64, y: i64) -> [u8; 4] {
        let x = x.clamp(0, self.w as i64 - 1) as u32;
        let y = y.clamp(0, self.h as i64 - 1) as u32;
        self.px[(y * self.w + x) as usize]
    }
}

/// A level and the texels where its own rounding met a tie (row-major
/// indices).
struct Made {
    l: Level,
    ties: Vec<u32>,
}

/// The TPU model's bilinear sample of `l` (CLAMP_TO_EDGE) at `c`:
/// tests/pvr_mip_three_levels.rs's `sample`.
fn sample(l: &Level, c: [f32; 2]) -> [f32; 4] {
    let u = c[0] * l.w as f32 - 0.5;
    let v = c[1] * l.h as f32 - 0.5;
    let (i0, j0) = (u.floor(), v.floor());
    let (a, b) = (u - i0, v - j0);
    let (i0, j0) = (i0 as i64, j0 as i64);
    let t = [l.at(i0, j0), l.at(i0 + 1, j0), l.at(i0, j0 + 1), l.at(i0 + 1, j0 + 1)];
    let mut out = [0.0f32; 4];
    for (ch, o) in out.iter_mut().enumerate() {
        let g = |k: usize| t[k][ch] as f32 / 255.0;
        *o = (1.0 - a) * (1.0 - b) * g(0) + a * (1.0 - b) * g(1) + (1.0 - a) * b * g(2) + a * b * g(3);
    }
    out
}

/// A level kick of `l` to `w` x `h` (or `o0`, or the leaves of `o1`): each
/// texel the sample at (p + 0.5) * (1/w, 1/h), times 255, rounded by `r`.
fn level_kick(l: &Level, w: u32, h: u32, r: Rule) -> Made {
    let s = [1.0 / w as f32, 1.0 / h as f32];
    let mut px = Vec::with_capacity((w * h) as usize);
    let mut ties = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let v = sample(l, [(x as f32 + 0.5) * s[0], (y as f32 + 0.5) * s[1]]).map(|c| c * 255.0);
            if v.iter().any(|&c| tie(c)) {
                ties.push(y * w + x);
            }
            px.push(v.map(|c| round(r, c)));
        }
    }
    Made { l: Level { w, h, px }, ties }
}

/// mipgen3.frag's ALU level (`level2`, and `o2` over it) of `l` to `w` x `h`,
/// rounded by `r`; each output n is then n * (1/255) and the pack's.
fn alu_level(l: &Level, w: u32, h: u32, q: Rule, pack: Rule) -> Made {
    let s = [1.0 / w as f32, 1.0 / h as f32];
    let ext = [l.w as f32, l.h as f32];
    let mix = |a: f32, b: f32, t: f32| t * (b - a) + a;
    let mut px = Vec::with_capacity((w * h) as usize);
    let mut ties = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let p = [x as f32, y as f32];
            let mut i0 = [0i64; 2];
            let mut i1 = [0i64; 2];
            let mut f = [0f32; 2];
            for k in 0..2 {
                let u = (p[k] + 0.5) * s[k] * ext[k] - 0.5;
                let fl = u.floor();
                f[k] = u - fl;
                i0[k] = fl.clamp(0.0, ext[k] - 1.0) as i64;
                i1[k] = (fl + 1.0).clamp(0.0, ext[k] - 1.0) as i64;
            }
            let t = [l.at(i0[0], i0[1]), l.at(i1[0], i0[1]), l.at(i0[0], i1[1]), l.at(i1[0], i1[1])];
            let mut v = [0f32; 4];
            for (ch, o) in v.iter_mut().enumerate() {
                let g = |k: usize| t[k][ch] as f32;
                *o = mix(mix(g(0), g(1), f[0]), mix(g(2), g(3), f[0]), f[1]);
            }
            if v.iter().any(|&c| tie(c)) {
                ties.push(y * w + x);
            }
            px.push(v.map(|c| {
                let n = round(q, c);
                let out = n as f32 * (1.0 / 255.0) * 255.0;
                assert!(!tie(out), "an ALU output {n} / 255 on a tie");
                let b = round(pack, out);
                assert_eq!(b, n, "the pack of {n} / 255");
                b
            }));
        }
    }
    Made { l: Level { w, h, px }, ties }
}

/// The sides of a `w` x `h` chain's `n` levels (GL).
fn sides(w: u32, h: u32, n: usize) -> Vec<(u32, u32)> {
    (0..n).map(|l| ((w >> l).max(1), (h >> l).max(1))).collect()
}

/// tests/pvr_mip_three_levels.rs's `level0`.
fn level0(w: u32, h: u32, kind: &str) -> Level {
    let mut s: u32 = 0x1234_5678;
    let mut rnd = || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        s
    };
    let mut px = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let r = rnd().to_le_bytes();
            px.push(if kind == "random" {
                r
            } else {
                let g = |a: u32, b: u32, n: u8| (((a * 255) / b.max(1)) as u8).wrapping_add(n % 7);
                [g(x, w, r[0]), g(y, h, r[1]), g(x + y, w + h, r[2]), 255]
            });
        }
    }
    Level { w, h, px }
}

/// `mip_chain_gpu`'s grouping with the knob on: level 1 alone, then up to
/// three levels a render (tests/pvr_mip_three_levels.rs's `groups`).
fn groups(n: usize) -> Vec<Vec<usize>> {
    let mut g = vec![vec![1]];
    let mut l = 2;
    while l < n {
        let m = (n - l).min(3);
        g.push((l..l + m).collect());
        l += m;
    }
    g
}

/// The knob-on chain in one arm: levels 0..n, and whether each is an ALU
/// level.
fn chain(base: &Level, n: usize, arm: Arm) -> Vec<(Made, bool)> {
    let sz = sides(base.w, base.h, n);
    let mut out: Vec<(Made, bool)> = vec![(Made { l: base.clone(), ties: Vec::new() }, false)];
    for g in groups(n) {
        let src = out[g[0] - 1].0.l.clone();
        let (w1, h1) = sz[g[0]];
        out.push((level_kick(&src, w1, h1, arm.pack), false));
        if g.len() > 1 {
            /* o1's leaves: level l+1 as the program's Q makes it */
            let leaves = level_kick(&src, w1, h1, arm.q).l;
            let mut above = leaves;
            for &l in &g[1..] {
                let m = alu_level(&above, sz[l].0, sz[l].1, arm.q, arm.pack);
                above = m.l.clone();
                out.push((m, true));
            }
        }
    }
    assert_eq!(out.len(), n);
    out
}

/// FNV-1a 64 of a level's bytes, row by row.
fn fnv(l: &Level) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in l.px.iter().flatten() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// tests/pvr_mip_three_levels.rs's `PINNED`, from the IR evaluator running
/// mipgen3.frag: the `up` arm must give each of them.
const EVALUATED: &[((u32, u32), &str, usize, u64)] = &[
    ((800, 600), "smooth", 3, 0xece446206df20ee4),
    ((800, 600), "smooth", 4, 0x69f64f1984addd68),
    ((800, 600), "smooth", 5, 0xf69c45a2438d03c1),
    ((800, 600), "smooth", 6, 0xce4e80c2d55c3c89),
    ((800, 600), "smooth", 7, 0xb61ca876a6721137),
    ((800, 600), "smooth", 8, 0x9e39aa2fea4e1873),
    ((800, 600), "smooth", 9, 0x462d0ab58659e773),
    ((800, 600), "random", 3, 0xe100f3daa244b1ce),
    ((800, 600), "random", 4, 0x1f1ad6576632884d),
    ((800, 600), "random", 5, 0xebd88f5e67888b64),
    ((800, 600), "random", 6, 0x5f793b8b5453ce74),
    ((800, 600), "random", 7, 0xf9b437c9b33e4645),
    ((800, 600), "random", 8, 0x16333fe9063052ce),
    ((800, 600), "random", 9, 0x2a78b6a985056d13),
    ((256, 256), "smooth", 3, 0x184d3bcc20542f7f),
    ((256, 256), "smooth", 4, 0x48dfa29f93b91d8f),
    ((256, 256), "smooth", 5, 0xd44ba099ca0285e3),
    ((256, 256), "smooth", 6, 0x57476e639bd97a29),
    ((256, 256), "smooth", 7, 0xe2a25b7c0d29e0ed),
    ((256, 256), "smooth", 8, 0x9d8c8bad2b3f7f86),
    ((256, 256), "random", 3, 0x8792bfaf12da810d),
    ((256, 256), "random", 4, 0xd22e048b30cba11e),
    ((256, 256), "random", 5, 0xb8350a066037d9d9),
    ((256, 256), "random", 6, 0x46672176f9ca04ba),
    ((256, 256), "random", 7, 0x60edefb83806be71),
    ((256, 256), "random", 8, 0x540fce9fed04dc64),
];

/// One level: (size, level 0's content, level, ALU level, texels on a tie in
/// the `up` arm, FNV-1a 64 in `up`, `even`, `mixed`, texels differing from
/// `up` in `even`, in `mixed`).
type Pin = ((u32, u32), &'static str, usize, bool, usize, [u64; 3], [usize; 2]);

const PINNED: &[Pin] = &[
    // PINNED-BEGIN
    (
        (800, 600),
        "smooth",
        1,
        false,
        36867,
        [0x1b8d49f71c04e381, 0xcb0cf6589463e628, 0xcb0cf6589463e628],
        [18016, 18016],
    ),
    ((800, 600), "smooth", 2, false, 10799, [0xa7e4006ea581577d, 0x39089b2f00f3f606, 0x39089b2f00f3f606], [9081, 9081]),
    ((800, 600), "smooth", 3, true, 3752, [0xece446206df20ee4, 0xa2ecb9674f37d1b6, 0xef5c5b1e062ce764], [3937, 1143]),
    ((800, 600), "smooth", 4, true, 388, [0x69f64f1984addd68, 0x0e97f1b0c18139c2, 0xc52742233350677c], [1122, 288]),
    ((800, 600), "smooth", 5, false, 122, [0xf69c45a2438d03c1, 0x61c4b477f2098493, 0x4c08db0ca9f83b48], [316, 115]),
    ((800, 600), "smooth", 6, true, 12, [0xce4e80c2d55c3c89, 0x31c9e27245933ed4, 0x9b8d1084b96e5a23], [78, 15]),
    ((800, 600), "smooth", 7, true, 19, [0xb61ca876a6721137, 0x268888fb8d48eea7, 0xc27ef20f3d99ac35], [18, 5]),
    ((800, 600), "smooth", 8, false, 6, [0x9e39aa2fea4e1873, 0x40c9ab7be138fe91, 0xf3727e73418d28d3], [6, 6]),
    ((800, 600), "smooth", 9, true, 1, [0x462d0ab58659e773, 0x376037b8f2857b08, 0x2b610eb8eac79ee6], [1, 1]),
    (
        (800, 600),
        "random",
        1,
        false,
        41279,
        [0x91c2cfb805cbca0b, 0x93309aee7c420241, 0x93309aee7c420241],
        [22670, 22670],
    ),
    (
        (800, 600),
        "random",
        2,
        false,
        10498,
        [0xfc5fb8470eeb205e, 0xb3a8f33d1d6bc396, 0xb3a8f33d1d6bc396],
        [10506, 10506],
    ),
    ((800, 600), "random", 3, true, 3289, [0xe100f3daa244b1ce, 0xf81037c71cc9df8d, 0x829b19252fa054f4], [3944, 1432]),
    ((800, 600), "random", 4, true, 100, [0x1f1ad6576632884d, 0xb5d539f2586ee013, 0xaa435767dcc0a61f], [1014, 345]),
    ((800, 600), "random", 5, false, 38, [0xebd88f5e67888b64, 0x61e9f9b97f94ef35, 0xa664650f74a82092], [246, 86]),
    ((800, 600), "random", 6, true, 15, [0x5f793b8b5453ce74, 0x3005f1605b91a33a, 0xf920e28ae08f63b9], [64, 17]),
    ((800, 600), "random", 7, true, 8, [0xf9b437c9b33e4645, 0xbf91e4c4ba030129, 0x5b392e31fed7fc3f], [17, 5]),
    ((800, 600), "random", 8, false, 4, [0x16333fe9063052ce, 0x02546a7bd28dc672, 0x971d3279d2141825], [4, 3]),
    ((800, 600), "random", 9, true, 1, [0x2a78b6a985056d13, 0x2a78b5a985056b60, 0x2a78b5a985056b60], [1, 1]),
    ((256, 256), "smooth", 1, false, 7421, [0x56ebdd4fbbd2d78e, 0x6bbd06842bd34d65, 0x6bbd06842bd34d65], [3795, 3795]),
    ((256, 256), "smooth", 2, false, 1943, [0x870870e3820f6ccd, 0xd4dd6bbdd81595c2, 0xd4dd6bbdd81595c2], [1374, 1374]),
    ((256, 256), "smooth", 3, true, 638, [0x184d3bcc20542f7f, 0x6b0d53ac8d60ffbd, 0x62aefa11690ffdda], [344, 224]),
    ((256, 256), "smooth", 4, true, 63, [0x48dfa29f93b91d8f, 0xb53c3e4b132b0a93, 0xd715334bd25977e8], [45, 25]),
    ((256, 256), "smooth", 5, false, 2, [0xd44ba099ca0285e3, 0x4acda033d482d09a, 0x1a0ed36ddb34864c], [3, 1]),
    ((256, 256), "smooth", 6, true, 1, [0x57476e639bd97a29, 0x57476e639bd97a29, 0x57476e639bd97a29], [0, 0]),
    ((256, 256), "smooth", 7, true, 0, [0xe2a25b7c0d29e0ed, 0xe2a25b7c0d29e0ed, 0xe2a25b7c0d29e0ed], [0, 0]),
    ((256, 256), "smooth", 8, false, 0, [0x9d8c8bad2b3f7f86, 0x9d8c8bad2b3f7f86, 0x9d8c8bad2b3f7f86], [0, 0]),
    ((256, 256), "random", 1, false, 9464, [0xdf421779b16f1ab1, 0xa0a51eeb84766dc1, 0xa0a51eeb84766dc1], [5277, 5277]),
    ((256, 256), "random", 2, false, 2293, [0x47bbbd3160570eda, 0x21619d7aec12ace4, 0x21619d7aec12ace4], [2261, 2261]),
    ((256, 256), "random", 3, true, 698, [0x8792bfaf12da810d, 0xeed5a73057444ec5, 0x5746c5b0d5d8e708], [772, 332]),
    ((256, 256), "random", 4, true, 186, [0xd22e048b30cba11e, 0x72d592669e11bb6b, 0xe89fd6d3fc12af33], [230, 92]),
    ((256, 256), "random", 5, false, 37, [0xb8350a066037d9d9, 0xbfbd869e9b4bbf5d, 0xc0c9786672671086], [62, 37]),
    ((256, 256), "random", 6, true, 13, [0x46672176f9ca04ba, 0xefa30ec06660d4cd, 0xbfc0ed1671471b05], [16, 6]),
    ((256, 256), "random", 7, true, 2, [0x60edefb83806be71, 0xd08a66e10f1cb27b, 0xeb7a8e2b8f8a76ac], [4, 1]),
    ((256, 256), "random", 8, false, 0, [0x540fce9fed04dc64, 0x27e8cfa982d82977, 0x540fce9fed04dc64], [1, 0]),
    // PINNED-END
];

/// The first texels (x, y) of a level where `up` and `even` give different
/// bytes, at most four: (size, content, level, texels).
type Probe = ((u32, u32), &'static str, usize, &'static [(u32, u32)]);

const TIE_PROBES: &[Probe] = &[
    // PROBES-BEGIN
    ((800, 600), "smooth", 1, &[(0, 0), (4, 0), (9, 0), (11, 0)]),
    ((800, 600), "smooth", 2, &[(0, 0), (3, 0), (4, 0), (5, 0)]),
    ((800, 600), "smooth", 3, &[(0, 0), (1, 0), (2, 0), (3, 0)]),
    ((800, 600), "smooth", 4, &[(0, 0), (2, 0), (4, 0), (5, 0)]),
    ((800, 600), "smooth", 5, &[(1, 0), (2, 0), (3, 0), (4, 0)]),
    ((800, 600), "smooth", 6, &[(0, 0), (1, 0), (2, 0), (3, 0)]),
    ((800, 600), "smooth", 7, &[(0, 0), (1, 0), (2, 0), (3, 0)]),
    ((800, 600), "smooth", 8, &[(0, 0), (1, 0), (2, 0), (0, 1)]),
    ((800, 600), "smooth", 9, &[(0, 0)]),
    ((800, 600), "random", 1, &[(4, 0), (17, 0), (18, 0), (25, 0)]),
    ((800, 600), "random", 2, &[(1, 0), (2, 0), (4, 0), (5, 0)]),
    ((800, 600), "random", 3, &[(0, 0), (1, 0), (2, 0), (3, 0)]),
    ((800, 600), "random", 4, &[(1, 0), (4, 0), (6, 0), (7, 0)]),
    ((800, 600), "random", 5, &[(0, 0), (1, 0), (3, 0), (4, 0)]),
    ((800, 600), "random", 6, &[(0, 0), (1, 0), (2, 0), (3, 0)]),
    ((800, 600), "random", 7, &[(0, 0), (3, 0), (4, 0), (5, 0)]),
    ((800, 600), "random", 8, &[(1, 0), (2, 0), (1, 1), (2, 1)]),
    ((800, 600), "random", 9, &[(0, 0)]),
    ((256, 256), "smooth", 1, &[(2, 0), (5, 0), (12, 0), (13, 0)]),
    ((256, 256), "smooth", 2, &[(4, 0), (6, 0), (7, 0), (8, 0)]),
    ((256, 256), "smooth", 3, &[(3, 0), (5, 0), (9, 0), (11, 0)]),
    ((256, 256), "smooth", 4, &[(2, 0), (6, 0), (2, 1), (13, 1)]),
    ((256, 256), "smooth", 5, &[(4, 4), (7, 5), (2, 7)]),
    ((256, 256), "random", 1, &[(1, 0), (7, 0), (11, 0), (14, 0)]),
    ((256, 256), "random", 2, &[(0, 0), (1, 0), (3, 0), (4, 0)]),
    ((256, 256), "random", 3, &[(2, 0), (4, 0), (5, 0), (6, 0)]),
    ((256, 256), "random", 4, &[(1, 0), (2, 0), (3, 0), (4, 0)]),
    ((256, 256), "random", 5, &[(0, 0), (1, 0), (2, 0), (3, 0)]),
    ((256, 256), "random", 6, &[(0, 0), (1, 0), (2, 0), (3, 0)]),
    ((256, 256), "random", 7, &[(0, 0), (1, 0), (0, 1), (1, 1)]),
    ((256, 256), "random", 8, &[(0, 0)]),
    // PROBES-END
];

#[test]
fn the_tie_texels_and_each_rules_bytes_are_pinned_per_level() {
    /* the Q this file models; a change to the macro moves these lines */
    assert_eq!(
        MIPGEN3.matches("floor(textureLod(u_src, (q + 0.5) * u_l1.xy, 0.0) * 255.0 + 0.5)").count(),
        1,
        "mipgen3.frag's leaf Q"
    );
    assert_eq!(
        MIPGEN3.matches("floor(mix(mix(t00, t10, f.x), mix(t01, t11, f.x), f.y) + 0.5)").count(),
        2,
        "mipgen3.frag's level Q (o1, o2)"
    );
    let mut printed = Vec::new();
    let mut probes = Vec::new();
    let mut checked = 0;
    let mut evaluated = 0;
    for (w, h, n) in [(800u32, 600u32, 10usize), (256, 256, 9)] {
        for kind in ["smooth", "random"] {
            let base = level0(w, h, kind);
            let arms: Vec<Vec<(Made, bool)>> = ARMS.iter().map(|&a| chain(&base, n, a)).collect();
            for (l, (up, alu)) in arms[0].iter().enumerate().skip(1) {
                let alu = *alu;
                let hashes = [0, 1, 2].map(|a| fnv(&arms[a][l].0.l));
                let differ = |a: usize| up.l.px.iter().zip(&arms[a][l].0.l.px).filter(|(x, y)| x != y).count();
                let diffs = [differ(1), differ(2)];
                let what = format!("{w}x{h} {kind} level {l} ({}x{})", up.l.w, up.l.h);
                println!(
                    "{what}{}: {} of {} texels on a tie ({}); {} differ in {}, {} in {}",
                    if alu { ", ALU" } else { "" },
                    up.ties.len(),
                    up.l.px.len(),
                    ARMS[0].name,
                    diffs[0],
                    ARMS[1].name,
                    diffs[1],
                    ARMS[2].name
                );
                /* a byte can only move where some rounding met a tie */
                if up.ties.is_empty() && l == 1 {
                    assert_eq!(diffs, [0, 0], "{what}: moved without a tie");
                }
                printed.push(format!(
                    "    (({w}, {h}), \"{kind}\", {l}, {alu}, {}, [0x{:016x}, 0x{:016x}, 0x{:016x}], [{}, {}]),",
                    up.ties.len(),
                    hashes[0],
                    hashes[1],
                    hashes[2],
                    diffs[0],
                    diffs[1]
                ));
                let first: Vec<String> =
                    up.l.px
                        .iter()
                        .zip(&arms[1][l].0.l.px)
                        .enumerate()
                        .filter(|(_, (x, y))| x != y)
                        .take(4)
                        .map(|(i, _)| format!("({}, {})", i as u32 % up.l.w, i as u32 / up.l.w))
                        .collect();
                if !first.is_empty() {
                    probes.push(format!("    (({w}, {h}), \"{kind}\", {l}, &[{}]),", first.join(", ")));
                }
                if let Some(e) = EVALUATED.iter().find(|e| e.0 == (w, h) && e.1 == kind && e.2 == l) {
                    assert_eq!(hashes[0], e.3, "{what}: the up arm is not the evaluator's mipgen3.frag");
                    evaluated += 1;
                }
                if !PINNED.is_empty() {
                    let pin = PINNED
                        .iter()
                        .find(|p| p.0 == (w, h) && p.1 == kind && p.2 == l)
                        .unwrap_or_else(|| panic!("{what}: no pin"));
                    assert_eq!(pin.3, alu, "{what}: ALU level or not");
                    assert_eq!(pin.4, up.ties.len(), "{what}: texels on a tie");
                    assert_eq!(pin.5, hashes, "{what}: the up, even, mixed bytes moved");
                    assert_eq!(pin.6, diffs, "{what}: texels differing from up");
                    let probe = TIE_PROBES.iter().find(|p| p.0 == (w, h) && p.1 == kind && p.2 == l);
                    let got = first.join(", ");
                    let want = probe.map(|p| p.3.iter().map(|(x, y)| format!("({x}, {y})")).collect::<Vec<_>>());
                    assert_eq!(want.map(|v| v.join(", ")).unwrap_or_default(), got, "{what}: the probe texels");
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(evaluated, EVALUATED.len(), "every evaluated hash compared");
    if PINNED.is_empty() {
        println!("// PINNED-BEGIN\n{}\n// PINNED-END", printed.join("\n"));
        println!("// PROBES-BEGIN\n{}\n// PROBES-END", probes.join("\n"));
    } else {
        assert_eq!(checked, PINNED.len(), "every pinned level checked");
    }
}
