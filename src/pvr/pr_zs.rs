// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The depth/stencil words of the partial render's 3D_PR command, built from
//! the normal 3D command's (docs/notes/pvr-pr-zs.md, change list items 1 and
//! 2; Mesa pvr_arch_job_render.c:1009-1027, :1389). Pure; nothing here is
//! wired into a kick yet.
//!
//! The PR stores into, and the resumed render reloads from, the frame's own
//! depth/stencil buffer, so every word is the 3D's, except that
//! `isp_zlsctl` has the store bits on: a swapped frame's 3D has them off
//! (`swap_drops_zs`), and a partial render does read what it stores.

use super::csbgen as csb;

/// The 3D_PR's depth/stencil words: the 3D's, with the stores forced on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrZsWords {
    /// CR_ISP_ZLSCTL, `zstoreen | sstoreen | forcezstore` set.
    pub isp_zlsctl: u64,
    pub zload_base: u64,
    pub stencil_load_base: u64,
    pub isp_bgobjdepth: u64,
    /// The cmd_flags DEPTHBUFFER|STENCILBUFFER bits (and any others given).
    pub cmd_flags: u32,
}

/// CR_ISP_ZLSCTL with only the three store bits set, packed by Mesa's
/// generated packer.
fn store_mask() -> u64 {
    let v = csb::ROGUE_CR_ISP_ZLSCTL { zstoreen: true, sstoreen: true, forcezstore: true, ..Default::default() };
    let mut dw = [0u32; 2];
    // SAFETY: the packer reads one initialised structure and writes its two
    // dwords (ROGUE_CR_ISP_ZLSCTL_length, pinned by csb_pack_lengths).
    unsafe { csb::ROGUE_CR_ISP_ZLSCTL_pack(dw.as_mut_ptr().cast(), &v) };
    u64::from(dw[0]) | u64::from(dw[1]) << 32
}

/// The 3D_PR's depth/stencil words from the normal 3D's. Only `isp_zlsctl`
/// differs: the store bits are ORed in, the load bits, `zonlyrender` and the
/// extents stay as they are.
pub fn pr_zs_words(
    isp_zlsctl: u64,
    zload_base: u64,
    stencil_load_base: u64,
    isp_bgobjdepth: u64,
    cmd_flags: u32,
) -> PrZsWords {
    PrZsWords { isp_zlsctl: isp_zlsctl | store_mask(), zload_base, stencil_load_base, isp_bgobjdepth, cmd_flags }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(store: bool, zonly: bool, load: bool) -> u64 {
        let v = csb::ROGUE_CR_ISP_ZLSCTL {
            zlsextent_x_z: 24,
            zlsextent_y_z: 18,
            zloadformat: csb::ROGUE_CR_ZLS_FORMAT_TYPE_24BITINT,
            zstoreformat: csb::ROGUE_CR_ZLS_FORMAT_TYPE_24BITINT,
            zstoreen: store,
            sstoreen: store,
            forcezstore: store,
            zloaden: load,
            sloaden: load,
            forcezload: load,
            zonlyrender: zonly,
            ..Default::default()
        };
        let mut dw = [0u32; 2];
        // SAFETY: as store_mask.
        unsafe { csb::ROGUE_CR_ISP_ZLSCTL_pack(dw.as_mut_ptr().cast(), &v) };
        u64::from(dw[0]) | u64::from(dw[1]) << 32
    }

    fn words(z: u64) -> PrZsWords {
        pr_zs_words(z, 0x1000, 0x2000, 0x00ff_ffff, 0x180)
    }

    #[test]
    fn pr_zlsctl_swapped_stores_on() {
        assert_ne!(store_mask(), 0);
        let off = pack(false, false, true);
        assert_ne!(off, pack(true, false, true));
        assert_eq!(words(off).isp_zlsctl, pack(true, false, true));
    }

    #[test]
    fn pr_zlsctl_unswapped_equal() {
        let on = pack(true, false, true);
        assert_eq!(words(on).isp_zlsctl, on);
    }

    #[test]
    fn pr_zlsctl_keeps_zonlyrender_and_loads() {
        for load in [false, true] {
            let off = pack(false, true, load);
            let w = words(off).isp_zlsctl;
            assert_eq!(w, pack(true, true, load));
            assert_eq!(w & !store_mask(), off & !store_mask());
            assert_ne!(w, pack(true, false, load));
        }
    }

    #[test]
    fn pr_zs_words_copy_of_3d() {
        // packed D24S8: both bases the same buffer; split: two buffers
        for (z, s) in [(0x8000_1000u64, 0x8000_1000u64), (0x8000_1000, 0x9000_2000)] {
            let w = pr_zs_words(pack(false, false, true), z, s, 0x00ab_cdef, 0x80 | 0x100);
            assert_eq!(w.zload_base, z);
            assert_eq!(w.stencil_load_base, s);
            assert_eq!(w.isp_bgobjdepth, 0x00ab_cdef);
            assert_eq!(w.cmd_flags, 0x180);
        }
    }
}
