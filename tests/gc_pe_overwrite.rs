// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000: the PE COMPONENTS / OVERWRITE rule of `src/vivante/pe_overwrite.rs`.
//!
//! Worked out from Mesa 26.2.1 etnaviv (`src/gallium/drivers/etnaviv/`):
//! `etnaviv_blend.c:97` and `:145-151` (OVERWRITE = no blend and a full
//! colour mask), `util/format/u_format.h:751-786` (the X byte of an X8
//! format is no channel, so RGB is a full mask), `etnaviv_state.c:291-293`
//! (single-sample targets only), `etnaviv_format.c:359-360` (a sampler of a
//! format without alpha reads 1; ours does not, hence the FBO case).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_pe_overwrite
//! ```

#[path = "../src/vivante/pe_overwrite.rs"]
#[allow(dead_code)]
mod pe_overwrite;

use pe_overwrite::*;

fn c(components: u32, overwrite: bool) -> PeColor {
    PeColor { components, overwrite }
}

#[test]
fn window_full_mask_overwrites() {
    // the xrgb window with glColorMask(1,1,1,1) or (1,1,1,0)
    assert_eq!(pe_color(0xf, false, 0, PeTarget::Window, false), c(0xf, true));
    assert_eq!(pe_color(0x7, false, 0, PeTarget::Window, false), c(0xf, true));
}

#[test]
fn window_denials() {
    // blend, MSAA, partial RGB: today's answer (mask & 7, no OVERWRITE)
    assert_eq!(pe_color(0xf, true, 0, PeTarget::Window, false), c(0x7, false));
    assert_eq!(pe_color(0xf, false, 0, PeTarget::Window, true), c(0x7, false));
    assert_eq!(pe_color(0xb, false, 0, PeTarget::Window, false), c(0x3, false));
    assert_eq!(pe_color(0x0, false, 0, PeTarget::Window, false), c(0x0, false));
    assert_eq!(pe_color(0x8, false, 0, PeTarget::Window, false), c(0x0, false));
}

#[test]
fn rgb_fbo_never_writes_alpha() {
    for mask in 0..16 {
        for blend in [false, true] {
            for msaa in [false, true] {
                let r = pe_color(mask, blend, 0, PeTarget::Fbo, msaa);
                assert_eq!(r.components & 0x8, 0, "mask {mask:#x}");
                assert!(!r.overwrite, "mask {mask:#x}");
                assert_eq!(r.components, mask & 7);
            }
        }
    }
}

#[test]
fn no_overwrite_means_todays_components() {
    for mask in 0..16 {
        for blend in [false, true] {
            for msaa in [false, true] {
                for t in [PeTarget::Window, PeTarget::Fbo] {
                    let r = pe_color(mask, blend, 0, t, msaa);
                    if !r.overwrite {
                        assert_eq!(r.components, mask & 7);
                    }
                }
            }
        }
    }
}

#[test]
fn rgba_targets_keep_the_inline_answer() {
    // (mask, blend, msaa, components, overwrite) as backend.rs:1655-1666
    let table = [
        (0xf, false, false, 0xf, true),
        (0xf, true, false, 0xf, false),
        (0xf, false, true, 0xf, false),
        (0x7, false, false, 0x7, false),
        (0x8, false, false, 0x8, false),
        (0x0, false, false, 0x0, false),
        (0xe, true, true, 0xe, false),
    ];
    for t in [PeTarget::Window, PeTarget::Fbo] {
        for (m, b, ms, comp, ow) in table {
            assert_eq!(pe_color(m, b, 8, t, ms), c(comp, ow), "mask {m:#x} blend {b} msaa {ms}");
        }
    }
    // bits above the mask are ignored
    assert_eq!(pe_color(0x1f, false, 8, PeTarget::Fbo, false), c(0xf, true));
}
