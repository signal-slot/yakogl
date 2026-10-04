// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A video mode as the display drivers share it: active size, porches and
//! sync widths in the panel-timing terms the devicetree uses (front porch,
//! sync, back porch; not DRM's hsync_start/_end), the pixel clock, and the
//! sync polarities at the far end of the chain.

use crate::errno::EINVAL;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoMode {
    pub pixel_clock_hz: u32,
    pub hactive: u32,
    pub hfront: u32,
    pub hsync: u32,
    pub hback: u32,
    pub vactive: u32,
    pub vfront: u32,
    pub vsync: u32,
    pub vback: u32,
    pub hsync_high: bool,
    pub vsync_high: bool,
    pub de_high: bool,
}

impl VideoMode {
    pub const fn htotal(&self) -> u32 {
        self.hactive + self.hfront + self.hsync + self.hback
    }

    pub const fn vtotal(&self) -> u32 {
        self.vactive + self.vfront + self.vsync + self.vback
    }

    pub const fn hblank(&self) -> u32 {
        self.hfront + self.hsync + self.hback
    }

    pub const fn vblank(&self) -> u32 {
        self.vfront + self.vsync + self.vback
    }

    /// Frames per second x 1000 at `pixel_hz`.
    pub fn refresh_mhz(&self, pixel_hz: u32) -> u32 {
        let frame = u64::from(self.htotal()) * u64::from(self.vtotal());
        if frame == 0 {
            return 0;
        }
        (u64::from(pixel_hz) * 1000 / frame) as u32
    }

    /// Every field non-zero and the totals within 16 bits (the widest timing
    /// field any of the blocks has).
    pub fn check(&self) -> Result<(), i32> {
        let all = [
            self.pixel_clock_hz,
            self.hactive,
            self.hfront,
            self.hsync,
            self.hback,
            self.vactive,
            self.vfront,
            self.vsync,
            self.vback,
        ];
        if all.contains(&0) || self.htotal() > 0xffff || self.vtotal() > 0xffff {
            return Err(-EINVAL);
        }
        Ok(())
    }
}

/// 1920x1200 at 60 Hz with reduced blanking (VESA CVT 1.2, "1920x1200R"):
/// 154 MHz, H 1920 48 32 80 (+), V 1200 3 6 26 (-). The Armadillo-X2's
/// monitor; the board's devicetree states it, this is for the tests.
pub const WUXGA_60_RB: VideoMode = VideoMode {
    pixel_clock_hz: 154_000_000,
    hactive: 1920,
    hfront: 48,
    hsync: 32,
    hback: 80,
    vactive: 1200,
    vfront: 3,
    vsync: 6,
    vback: 26,
    hsync_high: true,
    vsync_high: false,
    de_high: true,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totals_and_refresh() {
        let m = WUXGA_60_RB;
        assert_eq!((m.htotal(), m.vtotal()), (2080, 1235));
        assert_eq!((m.hblank(), m.vblank()), (160, 35));
        assert_eq!(m.refresh_mhz(154_000_000), 59_950);
        assert!(m.check().is_ok());
        assert_eq!(VideoMode { vsync: 0, ..m }.check(), Err(-EINVAL));
    }
}
