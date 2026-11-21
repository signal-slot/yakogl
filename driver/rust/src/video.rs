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

/// 1280x720 at 60 Hz, CTA-861 VIC 4 (CTA-861-G table 1, "720p60"):
/// 74.25 MHz, H 1280 110 40 220 (+), V 720 5 5 20 (+). The fallback when a
/// sink gives no EDID.
pub const HD720_60: VideoMode = VideoMode {
    pixel_clock_hz: 74_250_000,
    hactive: 1280,
    hfront: 110,
    hsync: 40,
    hback: 220,
    vactive: 720,
    vfront: 5,
    vsync: 5,
    vback: 20,
    hsync_high: true,
    vsync_high: true,
    de_high: true,
};

/// 1920x1080 at 60 Hz, CTA-861 VIC 16 (CTA-861-G table 1, "1080p60"):
/// 148.5 MHz, H 1920 88 44 148 (+), V 1080 4 5 36 (+).
pub const FHD1080_60: VideoMode = VideoMode {
    pixel_clock_hz: 148_500_000,
    hactive: 1920,
    hfront: 88,
    hsync: 44,
    hback: 148,
    vactive: 1080,
    vfront: 4,
    vsync: 5,
    vback: 36,
    hsync_high: true,
    vsync_high: true,
    de_high: true,
};

/// A base EDID block is good when it starts with the fixed header
/// 00 ff ff ff ff ff ff 00 (VESA E-EDID 1.4 section 3.3) and its 128 bytes
/// sum to 0 modulo 256 (section 3.3, byte 7Fh is the checksum). Linux
/// drm_edid.c `edid_block_check` (:1855-1880, v6.12) tolerates a header with up to two
/// wrong bytes; a display driver that cannot re-read the sink does not.
pub fn edid_block_ok(block: &[u8; 128]) -> bool {
    block[..8] == [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]
        && block.iter().fold(0u8, |a, &b| a.wrapping_add(b)) == 0
}

/// Decode one 18-byte detailed timing descriptor (VESA E-EDID 1.4 section
/// 3.10.2, table 3.21), the same fields as Linux drm_edid.c
/// `drm_mode_detailed` (v6.12): pixel clock in 10 kHz units (bytes 0-1, LE),
/// active and blanking as 8 + 4 bits (bytes 2-4, 5-7), sync offset and
/// width as 8 + 2 bits (bytes 8-11), flags in byte 17. A zero pixel clock is
/// a display descriptor, not a timing. Interlaced and anything but digital
/// separate sync (bits 4:3 = 11) are refused, as are stereo (bit 5, Linux
/// refuses it at drm_edid.c:3494), a zero sync width (:3505) and a sync that
/// does not fit inside its blanking (Linux clamps it, :3539-3550). Field
/// decode: drm_edid.c:3473-3490 (v6.12); the sync polarity bits :3557-3560.
/// DE is active high: EDID has no DE polarity.
pub fn edid_dtd(d: &[u8; 18]) -> Option<VideoMode> {
    let clock10k = u32::from(d[0]) | u32::from(d[1]) << 8;
    if clock10k == 0 {
        return None;
    }
    let flags = d[17];
    if flags & 0xa0 != 0 || flags & 0x18 != 0x18 {
        return None;
    }
    let hactive = u32::from(d[2]) | u32::from(d[4] & 0xf0) << 4;
    let hblank = u32::from(d[3]) | u32::from(d[4] & 0x0f) << 8;
    let vactive = u32::from(d[5]) | u32::from(d[7] & 0xf0) << 4;
    let vblank = u32::from(d[6]) | u32::from(d[7] & 0x0f) << 8;
    let hfront = u32::from(d[8]) | u32::from(d[11] & 0xc0) << 2;
    let hsync = u32::from(d[9]) | u32::from(d[11] & 0x30) << 4;
    let vfront = u32::from(d[10] >> 4) | u32::from(d[11] & 0x0c) << 2;
    let vsync = u32::from(d[10] & 0x0f) | u32::from(d[11] & 0x03) << 4;
    let (hfp_sync, vfp_sync) = (hfront + hsync, vfront + vsync);
    if hsync == 0 || vsync == 0 || hfp_sync > hblank || vfp_sync > vblank {
        return None;
    }
    Some(VideoMode {
        pixel_clock_hz: clock10k * 10_000,
        hactive,
        hfront,
        hsync,
        hback: hblank - hfp_sync,
        vactive,
        vfront,
        vsync,
        vback: vblank - vfp_sync,
        hsync_high: flags & 0x02 != 0,
        vsync_high: flags & 0x04 != 0,
        de_high: true,
    })
}

/// The preferred mode of an EDID: the first detailed timing descriptor of
/// the base block, bytes 54-71 (VESA E-EDID 1.4 section 3.10.2: the first
/// DTD is the preferred timing mode). `edid` is at least the 128-byte base
/// block; extension blocks are not read. `-EINVAL` for a short buffer, a bad
/// header or checksum, or a first descriptor that is not a usable timing.
pub fn edid_preferred(edid: &[u8]) -> Result<VideoMode, i32> {
    let block: &[u8; 128] = edid.get(..128).and_then(|b| b.try_into().ok()).ok_or(-EINVAL)?;
    if !edid_block_ok(block) {
        return Err(-EINVAL);
    }
    let dtd: &[u8; 18] = block[54..72].try_into().map_err(|_| -EINVAL)?;
    let mode = edid_dtd(dtd).ok_or(-EINVAL)?;
    mode.check()?;
    Ok(mode)
}

/// One mode an EDID offers, as [`edid_modes`] reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdidMode {
    pub mode: VideoMode,
    /// CTA-861 VIC when the mode came from a video data block (SVD).
    pub vic: Option<u8>,
    /// SVD native flag (bit 7, only meaningful for VIC 1-64).
    pub native: bool,
    /// The base block's first DTD (VESA E-EDID 1.4 section 3.10.2).
    pub preferred: bool,
}

/// The mode of a CTA-861 VIC, for the two this crate carries: VIC 4 is
/// [`HD720_60`], VIC 16 is [`FHD1080_60`]. Any other VIC is `None`.
pub fn cea_vic_mode(vic: u8) -> Option<VideoMode> {
    match vic {
        4 => Some(HD720_60),
        16 => Some(FHD1080_60),
        _ => None,
    }
}

/// Walk one CTA-861 extension block (tag 0x02; CTA-861-G section 7.5): the
/// video data blocks' SVDs (data block tag 2 in bits 7:5 of the header byte,
/// length in bits 4:0, collection from byte 4 up to the DTD offset in byte 2)
/// and the DTDs from that offset on. An SVD is a VIC in bits 6:0 with the
/// native flag in bit 7 for VIC 1-64 (Linux drm_edid.c `svd_to_vic` and
/// `cea_db_iter`/`do_cea_modes`, v6.12); VICs without a constant here are
/// skipped. DTDs end at a zero pixel clock or the block's checksum byte.
/// `-EINVAL` for a bad checksum, a DTD offset past the block or a data block
/// that overruns the collection. A block with another tag is ignored (Ok).
pub fn cea_modes<F: FnMut(EdidMode)>(block: &[u8; 128], mut f: F) -> Result<(), i32> {
    if block[0] != 0x02 {
        return Ok(());
    }
    if block.iter().fold(0u8, |a, &b| a.wrapping_add(b)) != 0 {
        return Err(-EINVAL);
    }
    let d = usize::from(block[2]);
    if d == 0 {
        return Ok(()); // no data blocks, no DTDs
    }
    if !(4..=127).contains(&d) {
        return Err(-EINVAL);
    }
    let mut i = 4;
    while i < d {
        let (tag, len) = (block[i] >> 5, usize::from(block[i] & 0x1f));
        if i + 1 + len > d {
            return Err(-EINVAL);
        }
        if tag == 2 {
            for &svd in &block[i + 1..i + 1 + len] {
                let (vic, native) =
                    if (1..=64).contains(&(svd & 0x7f)) { (svd & 0x7f, svd & 0x80 != 0) } else { (svd, false) };
                if let Some(mode) = cea_vic_mode(vic) {
                    f(EdidMode { mode, vic: Some(vic), native, preferred: false });
                }
            }
        }
        i += 1 + len;
    }
    // DTDs start at the offset byte and must end before the checksum byte.
    let mut at = d;
    while at + 18 <= 127 {
        let dtd: &[u8; 18] = block[at..at + 18].try_into().map_err(|_| -EINVAL)?;
        if dtd[0] == 0 && dtd[1] == 0 {
            break;
        }
        if let Some(mode) = edid_dtd(dtd) {
            f(EdidMode { mode, vic: None, native: false, preferred: false });
        }
        at += 18;
    }
    Ok(())
}

/// Every mode of an EDID: the base block's DTDs (the first is `preferred`;
/// VESA E-EDID 1.4 section 3.10.2, the descriptors at bytes 54-125) and then
/// those of each CTA-861 extension block (`edid[126]` blocks follow). The base
/// block must pass [`edid_block_ok`]; a short buffer, a bad CEA checksum or a
/// malformed CEA block is `-EINVAL`. Extension blocks with another tag are
/// ignored. [`edid_preferred`] is unchanged and reads only the base block.
pub fn edid_modes<F: FnMut(EdidMode)>(edid: &[u8], mut f: F) -> Result<(), i32> {
    let base: &[u8; 128] = edid.get(..128).and_then(|b| b.try_into().ok()).ok_or(-EINVAL)?;
    if !edid_block_ok(base) {
        return Err(-EINVAL);
    }
    for (n, at) in (54..=108).step_by(18).enumerate() {
        let dtd: &[u8; 18] = base[at..at + 18].try_into().map_err(|_| -EINVAL)?;
        if let Some(mode) = edid_dtd(dtd) {
            if mode.check().is_ok() {
                f(EdidMode { mode, vic: None, native: false, preferred: n == 0 });
            }
        }
    }
    for k in 1..=usize::from(base[126]) {
        let blk: &[u8; 128] = edid.get(k * 128..(k + 1) * 128).and_then(|b| b.try_into().ok()).ok_or(-EINVAL)?;
        cea_modes(blk, &mut f)?;
    }
    Ok(())
}

/// Pick the mode to drive from `supported` (the caller's modes, best first).
/// Order: the base block's preferred mode if supported, else a CTA native SVD
/// that is supported, else the first entry of `supported` the sink lists
/// (VIC or DTD, timing equal). `Ok(None)` when the sink lists none of them;
/// errors as [`edid_modes`].
pub fn best_supported(edid: &[u8], supported: &[VideoMode]) -> Result<Option<VideoMode>, i32> {
    let (mut pref, mut native) = (None, None);
    let mut listed = [false; 16];
    edid_modes(edid, |m| {
        let Some(idx) = supported.iter().position(|s| *s == m.mode) else { return };
        if m.preferred {
            pref = Some(m.mode);
        }
        if m.native && native.is_none() {
            native = Some(m.mode);
        }
        if let Some(l) = listed.get_mut(idx) {
            *l = true;
        }
    })?;
    if pref.is_some() || native.is_some() {
        return Ok(pref.or(native));
    }
    Ok(supported.iter().zip(listed).find(|(_, l)| *l).map(|(m, _)| *m))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Linux v6.12 drivers/gpu/drm/tests/drm_kunit_edid.h `test_edid_dvi_1080p`
    /// (base block; GPL-2.0 test vector, "Test EDID", 1920x1080@60).
    const EDID_DVI_1080P: [u8; 128] = [
        0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x31, 0xd8, 0x2a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21,
        0x01, 0x03, 0x81, 0xa0, 0x5a, 0x78, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
        0x02, 0x3a, 0x80, 0x18, 0x71, 0x38, 0x2d, 0x40, 0x58, 0x2c, 0x45, 0x00, 0x40, 0x84, 0x63, 0x00, 0x00, 0x1e,
        0x00, 0x00, 0x00, 0xfc, 0x00, 0x54, 0x65, 0x73, 0x74, 0x20, 0x45, 0x44, 0x49, 0x44, 0x0a, 0x20, 0x20, 0x20,
        0x00, 0x00, 0x00, 0xfd, 0x00, 0x32, 0x46, 0x1e, 0x46, 0x0f, 0x00, 0x0a, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
        0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0xab,
    ];

    /// Same file, `test_edid_hdmi_1080p_rgb_max_200mhz` (base block).
    const EDID_HDMI_1080P: [u8; 128] = [
        0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x31, 0xd8, 0x2a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21,
        0x01, 0x03, 0x81, 0xa0, 0x5a, 0x78, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x20,
        0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
        0x02, 0x3a, 0x80, 0x18, 0x71, 0x38, 0x2d, 0x40, 0x58, 0x2c, 0x45, 0x00, 0x40, 0x84, 0x63, 0x00, 0x00, 0x1e,
        0x00, 0x00, 0x00, 0xfc, 0x00, 0x54, 0x65, 0x73, 0x74, 0x20, 0x45, 0x44, 0x49, 0x44, 0x0a, 0x20, 0x20, 0x20,
        0x00, 0x00, 0x00, 0xfd, 0x00, 0x32, 0x46, 0x00, 0x00, 0xc4, 0x00, 0x0a, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
        0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01, 0x41,
    ];

    /// The 720p60 detailed timing descriptor as monitors carry it for
    /// CTA-861 VIC 4 (74.25 MHz, 1280/110/40/220, 720/5/5/20, +/+). Written
    /// from the timing, not copied from a device.
    const DTD_720P: [u8; 18] =
        [0x01, 0x1d, 0x00, 0x72, 0x51, 0xd0, 0x1e, 0x20, 0x6e, 0x28, 0x55, 0x00, 0xc4, 0x8e, 0x21, 0x00, 0x00, 0x1e];

    #[test]
    fn cea_modes() {
        assert!(HD720_60.check().is_ok());
        assert!(FHD1080_60.check().is_ok());
        assert_eq!((HD720_60.htotal(), HD720_60.vtotal()), (1650, 750));
        assert_eq!((FHD1080_60.htotal(), FHD1080_60.vtotal()), (2200, 1125));
        assert_eq!(HD720_60.refresh_mhz(HD720_60.pixel_clock_hz), 60_000);
        assert_eq!(FHD1080_60.refresh_mhz(FHD1080_60.pixel_clock_hz), 60_000);
    }

    #[test]
    fn edid_vectors_decode_to_vic16() {
        for e in [&EDID_DVI_1080P, &EDID_HDMI_1080P] {
            assert!(edid_block_ok(e));
            assert_eq!(edid_preferred(e), Ok(FHD1080_60));
        }
        // Extra bytes (an extension block) are ignored.
        let mut long = [0u8; 256];
        long[..128].copy_from_slice(&EDID_DVI_1080P);
        assert_eq!(edid_preferred(&long), Ok(FHD1080_60));
    }

    #[test]
    fn dtd_720p_decodes_to_vic4() {
        assert_eq!(edid_dtd(&DTD_720P), Some(HD720_60));
    }

    #[test]
    fn dtd_refusals() {
        let mut d = DTD_720P;
        d[17] |= 0x80; // interlaced
        assert_eq!(edid_dtd(&d), None);
        let mut d = DTD_720P;
        d[17] &= !0x10; // analog / composite sync
        assert_eq!(edid_dtd(&d), None);
        let mut d = DTD_720P;
        d[17] |= 0x20; // stereo
        assert_eq!(edid_dtd(&d), None);
        let mut d = DTD_720P;
        d[9] = 0; // zero hsync width
        d[11] &= !0x30;
        assert_eq!(edid_dtd(&d), None);
        let mut d = DTD_720P;
        d[0] = 0;
        d[1] = 0; // display descriptor
        assert_eq!(edid_dtd(&d), None);
        let mut d = DTD_720P;
        d[3] = 0x10; // hblank smaller than front porch + sync
        d[4] &= 0xf0;
        assert_eq!(edid_dtd(&d), None);
        let mut d = DTD_720P;
        d[17] &= !0x06; // negative syncs
        let m = edid_dtd(&d).unwrap();
        assert!(!m.hsync_high && !m.vsync_high);
    }

    #[test]
    fn edid_refusals() {
        let mut bad = EDID_DVI_1080P;
        bad[127] ^= 1;
        assert!(!edid_block_ok(&bad));
        assert_eq!(edid_preferred(&bad), Err(-EINVAL));
        let mut bad = EDID_DVI_1080P;
        bad[0] = 0x01;
        bad[127] = bad[127].wrapping_sub(1);
        assert!(!edid_block_ok(&bad));
        assert_eq!(edid_preferred(&bad), Err(-EINVAL));
        assert_eq!(edid_preferred(&EDID_DVI_1080P[..127]), Err(-EINVAL));
        // A good block whose first descriptor is a display descriptor.
        let mut nodtd = EDID_DVI_1080P;
        nodtd[54] = 0;
        nodtd[55] = 0;
        nodtd[127] = nodtd[127].wrapping_add(0x02 + 0x3a);
        assert!(edid_block_ok(&nodtd));
        assert_eq!(edid_preferred(&nodtd), Err(-EINVAL));
    }

    #[test]
    fn totals_and_refresh() {
        let m = WUXGA_60_RB;
        assert_eq!((m.htotal(), m.vtotal()), (2080, 1235));
        assert_eq!((m.hblank(), m.vblank()), (160, 35));
        assert_eq!(m.refresh_mhz(154_000_000), 59_950);
        assert!(m.check().is_ok());
        assert_eq!(VideoMode { vsync: 0, ..m }.check(), Err(-EINVAL));
    }

    fn fix_sum(b: &mut [u8; 128]) {
        b[127] = 0;
        b[127] = b.iter().fold(0u8, |a, &x| a.wrapping_add(x)).wrapping_neg();
    }

    /// Two-block EDID: the 1080p base block (preferred DTD cleared when
    /// `dvi` is false is not needed) plus a CEA block built by `cea`.
    fn edid2(cea: impl FnOnce(&mut [u8; 128])) -> [u8; 256] {
        let mut e = [0u8; 256];
        e[..128].copy_from_slice(&EDID_DVI_1080P);
        e[126] = 1;
        let mut b = [0u8; 128];
        b[0] = 0x02;
        b[1] = 3;
        cea(&mut b);
        fix_sum(&mut b);
        let mut base = [0u8; 128];
        base.copy_from_slice(&e[..128]);
        fix_sum(&mut base);
        e[..128].copy_from_slice(&base);
        e[128..].copy_from_slice(&b);
        e
    }

    fn collect(e: &[u8]) -> Result<Vec<EdidMode>, i32> {
        let mut v = Vec::new();
        edid_modes(e, |m| v.push(m)).map(|_| v)
    }

    #[test]
    fn cea_vic16_native() {
        // Video data block: VIC 4, VIC 16 native, VIC 5 (no constant, skipped).
        let e = edid2(|b| {
            b[2] = 8;
            b[4] = 0x40 | 3;
            b[5..8].copy_from_slice(&[4, 0x80 | 16, 5]);
        });
        let v = collect(&e).unwrap();
        let vics: Vec<_> = v.iter().filter_map(|m| m.vic.map(|c| (c, m.native, m.mode))).collect();
        assert_eq!(vics, vec![(4, false, HD720_60), (16, true, FHD1080_60)]);
        assert!(v[0].preferred && v[0].mode == FHD1080_60);
        assert_eq!(best_supported(&e, &[HD720_60]), Ok(Some(HD720_60)));
        assert_eq!(best_supported(&e, &[WUXGA_60_RB]), Ok(None));
        assert_eq!(edid_preferred(&e), Ok(FHD1080_60));
    }

    #[test]
    fn cea_native_beats_list_order() {
        // Base block without a usable preferred timing.
        let mut e = edid2(|b| {
            b[2] = 7;
            b[4] = 0x40 | 2;
            b[5..7].copy_from_slice(&[4, 0x80 | 16]);
        });
        e[54] = 0;
        e[55] = 0;
        let mut base = [0u8; 128];
        base.copy_from_slice(&e[..128]);
        fix_sum(&mut base);
        e[..128].copy_from_slice(&base);
        assert_eq!(best_supported(&e, &[HD720_60, FHD1080_60]), Ok(Some(FHD1080_60)));
        // VIC 4 listed but not native; with FHD unsupported it is still chosen.
        assert_eq!(best_supported(&e, &[HD720_60]), Ok(Some(HD720_60)));
    }

    #[test]
    fn cea_dtds_only() {
        let e = edid2(|b| {
            b[2] = 4; // no data blocks, DTDs from byte 4
            b[4..22].copy_from_slice(&DTD_720P);
        });
        let v = collect(&e).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!((v[1].mode, v[1].vic, v[1].preferred), (HD720_60, None, false));
    }

    #[test]
    fn cea_refusals() {
        let mut e = edid2(|b| b[2] = 4);
        e[255] ^= 1;
        assert_eq!(collect(&e), Err(-EINVAL));
        // Data block longer than the collection.
        let e = edid2(|b| {
            b[2] = 6;
            b[4] = 0x40 | 5;
        });
        assert_eq!(collect(&e), Err(-EINVAL));
        // Missing extension block.
        let mut e = edid2(|b| b[2] = 4);
        assert_eq!(collect(&e[..200]), Err(-EINVAL));
        e[126] = 1;
        // Bad base block.
        e[0] = 1;
        assert_eq!(collect(&e), Err(-EINVAL));
    }

    #[test]
    fn non_cea_extension_ignored() {
        // Tag 0x70 (DisplayID) with a bad checksum and CEA-looking bytes.
        let mut e = edid2(|b| {
            b[0] = 0x70;
            b[2] = 5;
            b[4] = 0x41;
            b[5] = 0x80 | 4;
        });
        e[255] ^= 1;
        let v = collect(&e).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(best_supported(&e, &[HD720_60, FHD1080_60]), Ok(Some(FHD1080_60)));
    }
}
