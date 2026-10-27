// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The triangle's two shader programs as Mesa 25.0.7 compiled them on the
//! board (Debian 25.0.7-2+deb13u1, the build the traces ran): fixed test
//! input for milestone L1 until our Valhall back end
//! (src/shaderc/online/mali/) produces them.
//!
//! The words are the ones of tools/mali/testdata/board-25.0.7/tri.expect.json
//! (regions `vs.position` and `fs.fragment`, cut from the board's
//! `tri.pandecode`) and the bytes of the trace excerpt
//! tools/mali/board-dump/tri-frame-2026-10-18.txt at GPU VA 0x7fffffffc000
//! and 0x7fffffffc080; the tests below check both. The metadata is the
//! trace's Shader Program descriptors ("Register allocation: 32 Per Thread",
//! preload 0x3800 and 0x3000) and the position shader's push words, as
//! src/mali/frame_tests.rs takes them.
//!
//! Neither program holds a GPU address: the position shader reads its
//! attribute through the resource table and its viewport through the FAU,
//! the fragment shader writes a constant red and blends through the blend
//! descriptor the draw names.

use yakogl::mali::frame::{PushWord, ShaderMeta, Sysval};
use yakogl::mali::genxml::v10::{ShaderRegisterAllocation, ShaderStage};

/// The IDVS position shader (`LD_ATTR_IMM.v2` of attribute 0, `FMA` by
/// the viewport scale and offset in u0..u3, `MOV r2, u4`, `MOV r3, 1.0`,
/// `STORE.i128.istream`).
pub const VS_WORDS: [u64; 7] = [
    0x0866_8004_1201_7d7c,
    0x00b2_c000_0081_8040,
    0x00b2_c100_0083_8241,
    0x0091_c200_0000_0084,
    0x0091_c300_0000_00d0,
    0x105e_8404_4000_0d7b,
    0x7861_4008_b900_0044,
];

/// The fragment shader (`ATEST`, then `BLEND` of the constant
/// (1, 0, 0, 1) in f16 to target 0).
pub const FS_WORDS: [u64; 7] = [
    0x4110_c03c_0000_00c0,
    0x087d_bc02_08ea_407c,
    0x6910_c000_003c_00c0,
    0x4891_c100_0000_00db,
    0x787f_4004_333c_00f0,
    0x0110_f000_0000_00c0,
    0x002f_c110_0000_f1c0,
];

/// The binary of `words`, little-endian, as the GPU reads it.
pub fn bytes(words: &[u64]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// The position shader's sysval block: the viewport scale and offset.
pub const VS_SYSVALS: [Sysval; 2] = [Sysval::ViewportScale, Sysval::ViewportOffset];
/// The five words the binary reads as u0..u4: scale.x, offset.x, scale.y,
/// offset.y, offset.z (the pairing is Mesa's bi_opt_reorder_push; the
/// trace's FAU holds 32.0 32.0 32.0 32.0 0.5 for a 64-pixel viewport,
/// tri.expect.json `fau`).
pub const VS_PUSH: [PushWord; 5] = [
    PushWord::Sysval { sysval: 0, component: 0 },
    PushWord::Sysval { sysval: 1, component: 0 },
    PushWord::Sysval { sysval: 0, component: 1 },
    PushWord::Sysval { sysval: 1, component: 1 },
    PushWord::Sysval { sysval: 1, component: 2 },
];

/// The meta of one of the two programs.
pub fn meta(stage: ShaderStage) -> ShaderMeta<'static> {
    let vertex = stage == ShaderStage::VERTEX;
    ShaderMeta {
        stage,
        register_allocation: ShaderRegisterAllocation::_32_PER_THREAD,
        preload: if vertex { 0x3800 } else { 0x3000 },
        sysvals: if vertex { &VS_SYSVALS } else { &[] },
        push: if vertex { &VS_PUSH } else { &[] },
        varyings: 0,
        varying_bytes: 0,
        secondary: None,
        no_psiz_offset: None,
        colour_outputs: if vertex { 0 } else { 1 },
        samples_texture: false,
        requires_helper_threads: false,
        can_discard: false,
        writes_depth: false,
        writes_stencil: false,
        writes_coverage: false,
        writes_global: false,
        ubo: &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECT: &str = include_str!("../../testdata/board-25.0.7/tri.expect.json");
    const FRAME: &str = include_str!("../../board-dump/tri-frame-2026-10-18.txt");

    /// The words of one region of tri.expect.json, in order.
    fn region(name: &str) -> Vec<u64> {
        let at = EXPECT.find(&format!("\"{name}\"")).unwrap_or_else(|| panic!("no region {name}"));
        let rest = &EXPECT[at..];
        let words = &rest[rest.find("\"words\"").expect("a words list")..];
        let list = &words[words.find('[').expect("[") + 1..words.find(']').expect("]")];
        list.split(',').map(|w| u64::from_str_radix(w.trim().trim_matches('"'), 16).expect("hex")).collect()
    }

    /// `len` bytes at offset `start` of the trace excerpt's hexdump of the
    /// buffer at `va`, from consecutive printed rows (a row the dump folds
    /// into `*` is not read).
    fn dump(va: u64, start: usize, len: usize) -> Vec<u8> {
        let head = format!("Buffer: memory_{va:x} gpu {va:x}");
        let at = FRAME.find(&head).unwrap_or_else(|| panic!("no {head}"));
        let mut out = Vec::new();
        for line in FRAME[at..].lines().skip(1) {
            if line.starts_with("Buffer:") || line.starts_with("##") {
                break;
            }
            let mut it = line.split_whitespace();
            let Some(off) = it.next().and_then(|o| usize::from_str_radix(o, 16).ok()) else {
                continue;
            };
            if off == start + out.len() {
                out.extend(it.map(|b| u8::from_str_radix(b, 16).expect("a hex byte")));
            }
            if out.len() >= len {
                break;
            }
        }
        assert!(out.len() >= len, "{len} bytes at {va:#x}+{start:#x}: the dump holds {}", out.len());
        out.truncate(len);
        out
    }

    #[test]
    fn the_programs_are_the_board_s() {
        assert_eq!(region("vs.position"), VS_WORDS);
        assert_eq!(region("fs.fragment"), FS_WORDS);
        assert_eq!(dump(0x7fff_ffff_c000, 0, 56), bytes(&VS_WORDS));
        assert_eq!(dump(0x7fff_ffff_c000, 0x80, 56), bytes(&FS_WORDS));
    }
}
