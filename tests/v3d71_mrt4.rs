// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 four-target render state (`rt71`, `frame71`, `draw71`)
//! against Mesa 26.2.1's `tools/v3d/pi5/gl_mrt4.clif`.
//!
//! The shape is `mrt4` of tools/v3d/pi5/src/gl_shapes.py: four GL_RGBA8
//! textures on one FBO, `glDrawBuffers(4)`, a blue `glClear`, `glFinish`:
//! one job, no draw.
//!
//! * the four `TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1` records of the
//!   dump (number, base, stride, internal bpp, internal type and clamping,
//!   clear colour) equal what `rt71` gives for GL_RGBA8 and a blue clear;
//! * the four `STORE` packets of the generic tile list (buffer number,
//!   format, memory format, UIF height, channel flags, no clear bit) equal
//!   the plan `frame71` makes of the GL framebuffer, and so do the common
//!   packet's target count, the tile size and the `CLEAR_RT` that clears;
//! * the dump holds no per-draw state packet (no draw), so the blend and
//!   colour-write-mask packets `draw71` makes for four targets are pinned
//!   by layout; the missing four-target draw reference is an ignored test.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_mrt4
//! ```
#[path = "../src/v3d/draw71.rs"]
mod draw71;
#[path = "../src/v3d/frame71.rs"]
mod frame71;
#[path = "../src/v3d/rt71.rs"]
mod rt71;

use frame71::{Attach71, EndOp71, Frame71, Op71, Surface71, Transfer71, BUFFER_NONE, BUFFER_RT0};
use std::collections::BTreeSet;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const CLIF: &str = "tools/v3d/pi5/gl_mrt4.clif";
const GL_RGBA8: u32 = 0x8058;

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// One decoded packet of the dump: its name and (label, first value token).
type Pk = (String, Vec<(String, String)>);

fn parse() -> Vec<Pk> {
    let mut out: Vec<Pk> = vec![];
    for l in read(CLIF).lines() {
        let Some(rest) = l.strip_prefix("MESA: info:") else { continue };
        let t = rest.trim();
        if t.is_empty() || t.starts_with('@') {
            continue;
        }
        if let Some(body) = t.strip_prefix("/*") {
            let Some(at) = body.find("*/") else { continue };
            let label = body[..at].trim().trim_end_matches(':').trim().to_string();
            let value = body[at + 2..].trim();
            let tok = if value.starts_with('[') {
                // "[resource_0x3000+0x0] /* 0x00003000 */": the resource base
                value.trim_start_matches('[').split('+').next().unwrap().to_string()
            } else {
                value.split_whitespace().next().unwrap_or("").to_string()
            };
            if let Some(p) = out.last_mut() {
                p.1.push((label, tok));
            }
        } else if t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
            out.push((t.to_string(), vec![]));
        }
    }
    out
}

fn named<'a>(pks: &'a [Pk], name: &str) -> Vec<&'a Pk> {
    pks.iter().filter(|p| p.0 == name).collect()
}

fn num(p: &Pk, label: &str) -> u32 {
    let v = &p.1.iter().find(|(l, _)| l == label).unwrap_or_else(|| panic!("{}: no {label}", p.0)).1;
    v.parse().unwrap_or_else(|_| panic!("{} / {label}: {v}", p.0))
}

/// The hex address of a `resource_0xNNNN` token.
fn addr(p: &Pk) -> u32 {
    let v = &p.1.iter().find(|(l, _)| l == "Address").unwrap().1;
    u32::from_str_radix(v.rsplit("0x").next().unwrap(), 16).unwrap()
}

#[test]
fn the_shape_is_four_rgba8_targets_and_a_clear() {
    let py = read("tools/v3d/pi5/src/gl_shapes.py");
    assert!(py.contains("def mrt4(self):\n        self.mrt(4)"));
    assert!(py.contains("self.texture(GL_RGBA) for _ in range(n)"));
    assert!(py.contains("gl.glDrawBuffers(n, bufs)"));
    assert!(py.contains("gl.glClearColor(0.0, 0.0, 1.0, 1.0)"));
    let pks = parse();
    let common = named(&pks, "TILE_RENDERING_MODE_CFG_COMMON");
    assert_eq!(common.len(), 1);
    assert_eq!(num(common[0], "Number of Render Targets"), 4);
    assert_eq!((num(common[0], "Image Width (pixels)"), num(common[0], "Image Height (pixels)")), (64, 64));
    assert!(rt71::check_targets(4).is_ok());
    assert_eq!(rt71::check_targets(9), Err(rt71::Refusal71::TooManyTargets));
}

#[test]
fn the_four_render_target_configs() {
    let pks = parse();
    let common = named(&pks, "TILE_RENDERING_MODE_CFG_COMMON")[0];
    let (tw, th) = (8 << num(common, "Log2 Tile Width"), 8 << num(common, "Log2 Tile Height"));
    assert_eq!((tw, th), (64, 32));
    let rgba8 = rt71::rt_format71(GL_RGBA8).unwrap();
    // glClearColor(0, 0, 1, 1)
    let low = rt71::clear_words71(&rgba8, rt71::ClearColor::Float([0.0, 0.0, 1.0, 1.0])).unwrap().low;
    let parts = named(&pks, "TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1");
    assert_eq!(parts.len(), 4);
    let stride = rt71::rt_stride(tw, rgba8.bpp);
    assert_eq!(stride, 32);
    for (i, p) in parts.iter().enumerate() {
        let i = i as u32;
        assert_eq!(num(p, "Render Target number"), i);
        assert_eq!(num(p, "Base Address"), i * rt71::rt_base_step(th, stride), "target {i}: base");
        assert_eq!(num(p, "Stride"), stride, "target {i}: stride");
        assert_eq!(num(p, "Internal BPP"), rgba8.bpp, "target {i}: internal bpp");
        assert_eq!(num(p, "Internal Type and Clamping"), rgba8.type_clamp as u32, "target {i}: type and clamp");
        assert_eq!(num(p, "Clear Color low bits"), low, "target {i}: clear colour");
        // the packet rt71 builds decodes to the dump's numbers
        let pay = rt71::part1_payload(i, &rgba8, low, stride, num(p, "Base Address")).unwrap();
        let get = |n: &str| {
            let &(_, s, z) = rt71::PART1_FIELDS.iter().find(|x| x.0 == n).unwrap();
            (pay >> s) & ((1u64 << z) - 1)
        };
        assert_eq!(get("Stride") + 1, stride as u64);
        assert_eq!(get("Internal BPP"), rgba8.bpp as u64);
        assert_eq!(get("Internal Type and Clamping"), rgba8.type_clamp as u64);
        assert_eq!(get("Clear Color low bits"), low as u64);
    }
    // one format, one clamp, for every target
    let kinds: BTreeSet<(u32, u32)> =
        parts.iter().map(|p| (num(p, "Internal BPP"), num(p, "Internal Type and Clamping"))).collect();
    assert_eq!(kinds.len(), 1);
}

fn surface(address: u32) -> Surface71 {
    Surface71 { format: 27, memory_format: 4, height_in_ub_or_stride: 8, address, ..Default::default() }
}

/// The frame of the shape from the GL state alone: 64x64, the tile size of
/// the dump's common packet, four RGBA8 UIF textures cleared and stored.
fn gl_frame(addrs: [u32; 4]) -> Frame71 {
    let mut f = Frame71::new(64, 64, 64, 32);
    f.render_targets = 4;
    for (i, a) in addrs.iter().enumerate() {
        f.rt[i] = Some(Attach71 { surface: surface(*a), load: false, end: EndOp71::Store });
    }
    f
}

#[test]
fn the_four_stores_and_the_clear() {
    let pks = parse();
    // buffer 8 (NONE) is the two dummy tiles of the initial clear
    let (nones, rts): (Vec<&Pk>, Vec<&Pk>) =
        named(&pks, "STORE").into_iter().partition(|p| num(p, "Buffer to Store") == 8);
    assert_eq!(nones.len(), 2, "the two dummy tiles");
    assert_eq!(rts.len(), 4);
    for (i, p) in rts.iter().enumerate() {
        assert_eq!(num(p, "Buffer to Store"), i as u32);
        assert_eq!(num(p, "Memory Format"), 4, "UIF no XOR");
        assert_eq!(num(p, "Output Image Format"), 27, "rgba8");
        assert_eq!(num(p, "Height in UB or Stride"), 8);
        assert_eq!(num(p, "Dither Mode"), 0);
        assert_eq!(num(p, "Clear buffer being stored"), 0, "no store clear bit on 7.1");
        assert_eq!(num(p, "Channel Reverse"), 0);
        assert_eq!(num(p, "R/B swap"), 0);
        assert_eq!(num(p, "Flip Y"), 0);
        assert_eq!(num(p, "Decimate mode"), 0);
    }
    let addrs = [addr(rts[0]), addr(rts[1]), addr(rts[2]), addr(rts[3])];
    assert_eq!(addrs.iter().collect::<BTreeSet<_>>().len(), 4, "four distinct textures");

    let plan = frame71::plan(&gl_frame(addrs)).unwrap_or_else(|e| panic!("{}", e.message()));
    // the common packet: four targets, the dump's tile
    assert_eq!(plan.common.render_targets, 4);
    assert_eq!((plan.common.log2_tile_width, plan.common.log2_tile_height), (3, 2));
    assert!(plan.common.early_z_disable && !plan.common.msaa && !plan.common.double_buffer);
    // the generic list: a store per target in order, CLEAR_RT, END_TILE
    let mut want: Vec<Op71> =
        (0..4).map(|i| Op71::Store(Transfer71 { buffer: BUFFER_RT0 + i as u8, surface: surface(addrs[i]) })).collect();
    want.push(Op71::ClearRt);
    want.push(Op71::EndTile);
    assert_eq!(plan.list_stores, want);
    assert!(plan.list_loads.is_empty());
    // the initial clear: both dummy tiles store NONE, only the first clears
    let none = Op71::Store(Transfer71 { buffer: BUFFER_NONE, surface: Surface71::default() });
    assert_eq!(plan.initial_tiles.iter().filter(|o| **o == none).count(), 2);
    assert_eq!(plan.initial_tiles.iter().filter(|o| **o == Op71::ClearRt).count(), 1);
    // the dump's tile list ends with the same sequence of packets
    let kinds: Vec<&str> = pks
        .iter()
        .skip_while(|p| p.0 != "IMPLICIT_TILE_COORDS")
        .skip_while(|p| p.0 != "BRANCH_IMPLICIT_TILE")
        .map(|p| p.0.as_str())
        .collect();
    assert_eq!(&kinds[..7], ["BRANCH_IMPLICIT_TILE", "STORE", "STORE", "STORE", "STORE", "CLEAR_RT", "END_TILE"]);
    // a store slot past the job's four targets is refused
    let mut bad = gl_frame(addrs);
    bad.rt[4] = bad.rt[0];
    assert_eq!(frame71::plan(&bad).err(), Some(frame71::Refusal71::TooManyRenderTargets));
}

fn four(mask: impl Fn(usize) -> u8, blend: impl Fn(usize) -> bool) -> draw71::DrawState {
    let mut d = draw71::DrawState::gl_default(64, 64, false, 4);
    d.independent_blend = true;
    for (i, t) in d.targets.iter_mut().enumerate().take(4) {
        t.color_mask = mask(i);
        t.blend_enable = blend(i);
    }
    d
}

#[test]
fn per_target_blend_and_write_masks() {
    // the dump has no draw: none of draw71's per-draw packets is in it
    let clif = read(CLIF);
    for n in ["BLEND_ENABLES", "BLEND_CFG", "COLOR_WMASKS", "CFG_BITS", "CLIP"] {
        assert!(!clif.lines().any(|l| l.trim_end().ends_with(&format!("MESA: info: {n}"))), "{n} in the dump");
    }
    // GL's initial state on four targets: nothing blends, every channel on
    let p = draw71::translate(&draw71::DrawState::gl_default(64, 64, false, 4)).unwrap();
    assert_eq!((p.blend_enables.mask, p.blend_cfg_count, p.color_write_masks.mask), (0, 0, 0));
    // the glClear as a blitter quad, all channels: the same
    let p = draw71::translate(&draw71::DrawState::blitter_clear(64, 64, false, 4, 0xf, None)).unwrap();
    assert_eq!(p.color_write_masks.mask, 0);
    // one mask for all targets: the packet always carries eight nibbles
    // (a set bit masks the channel)
    let p = draw71::translate(&draw71::DrawState::blitter_clear(64, 64, false, 4, 0b1001, None)).unwrap();
    assert_eq!(p.color_write_masks.mask, 0x6666_6666);
    // a different mask and blend bit per target; targets 4..7 stay all-on
    let p = draw71::translate(&four(|i| i as u8 + 1, |i| i % 2 == 1)).unwrap();
    assert_eq!(p.blend_enables.mask, 0b1010);
    assert_eq!(p.blend_cfg_count, 2);
    let masks = (0xf & !1) | (0xf & !2) << 4 | (0xf & !3) << 8 | (0xf & !4) << 12;
    assert_eq!(p.color_write_masks.mask, masks);
    let cfgs: Vec<u8> = p.blend_cfg.iter().flatten().map(|c| c.render_target_mask).collect();
    assert_eq!(cfgs, [0b0010, 0b1000]);
    // without independent blend target 0's state (blend and mask) is every
    // target's: one blend packet for all eight, one mask in every nibble
    let mut d = four(|i| i as u8 + 1, |i| i == 0);
    d.independent_blend = false;
    let p = draw71::translate(&d).unwrap();
    assert_eq!(p.blend_enables.mask, 0xff);
    assert_eq!(p.blend_cfg_count, 1);
    assert_eq!(p.color_write_masks.mask, 0xeeee_eeee);
    // more than eight targets are refused
    assert_eq!(
        draw71::translate(&draw71::DrawState::gl_default(64, 64, false, 9)).err(),
        Some(draw71::Refusal71::TooManyTargets)
    );
}

#[test]
#[ignore = "gap: no Mesa 7.1 reference draws into four targets. gl_mrt4.clif is a clear only, so the per-target BLEND_ENABLES/BLEND_CFG/COLOR_WMASKS of four targets are pinned against draw71's layout, not a dump"]
fn four_target_draw_state_against_a_mesa_dump() {
    // wanted: a gl_shapes.py shape that draws into four targets under
    // per-target glBlendFunci/glColorMaski and its clif; then compare
    // draw71's packets with clif_sets, as tests/v3d71_draw.rs does
    let clif = read(CLIF);
    assert!(clif.contains("BLEND_ENABLES"), "gl_mrt4.clif has no draw state");
}
