// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Raspberry Pi framebuffer decisions with the board as a parameter: the
//! Pi 4's (BCM2711, [`super::rpi4`], unchanged) and the Pi 5's (BCM2712,
//! driver/docs/rpi5-v3d71-facts.md section 9). What differs is the mailbox
//! answer's address (the Pi 4 answers a bus address with the 0xc0000000
//! alias, the Pi 5 an ARM physical address), the checks of a
//! FRAMEBUFFER_ALLOCATE answer, and the adopted firmware scanout (a plane
//! the firmware already scans out, read by later work from the HVS). Pure
//! functions; the mailbox calls stay in C (rpi_mbox.c).

use super::rpi4::{self, Fb, FORMAT_RGBA8888, FORMAT_XRGB8888};

/// The board, as `yd_rpi_*` take it: the number is the Pi's model.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Board {
    Pi4 = 4,
    Pi5 = 5,
}

impl Board {
    /// `None` for any other number.
    pub fn from_u32(n: u32) -> Option<Board> {
        match n {
            4 => Some(Board::Pi4),
            5 => Some(Board::Pi5),
            _ => None,
        }
    }
}

/// The Pi 4's VideoCore bus alias of ARM memory (rpi_mbox.c clears it).
pub const PI4_BUS_ALIAS_MASK: u32 = 0xc000_0000;

/// The ARM physical address of the framebuffer the mailbox answered
/// (`bus`, the answer's address word): the Pi 4 clears the 0xc0000000 alias,
/// the Pi 5 has none (identity). 0 is no framebuffer on either board, before
/// and after the alias is cleared.
pub fn fb_phys(board: Board, bus: u32) -> Option<u64> {
    let phys = match board {
        Board::Pi4 => bus & !PI4_BUS_ALIAS_MASK,
        Board::Pi5 => bus,
    };
    (phys != 0).then_some(phys as u64)
}

/// Whether a FRAMEBUFFER_ALLOCATE answer is usable for a `width` x `rows`
/// 32 bpp framebuffer: a non-zero `size`, `pitch` at least a row of pixels,
/// and `size` holding all `rows` rows of that pitch. Board-independent (the
/// firmware's tag is the same); `rows` is the virtual height.
pub fn allocate_ok(width: u32, rows: u32, pitch: u32, size: u32) -> bool {
    width != 0
        && rows != 0
        && size != 0
        && pitch as u64 >= width as u64 * 4
        && size as u64 >= pitch as u64 * rows as u64
}

/// [`Fb`] from a mailbox answer: the address by [`fb_phys`], the screen's
/// `width` x `height`, `rows` in memory, `pitch` and `size` checked by
/// [`allocate_ok`]. `None` when any check fails.
pub fn fb_from_answer(board: Board, bus: u32, width: u32, height: u32, rows: u32, pitch: u32, size: u32) -> Option<Fb> {
    if height == 0 || rows < height || !allocate_ok(width, rows, pitch, size) {
        return None;
    }
    Some(Fb { phys: fb_phys(board, bus)?, width, height, pitch, rows })
}

/// Format codes of an adopted plane, as the (later) HVS6 register decode
/// hands them in. They are this module's own, not the register's field: the
/// decode maps the hardware format and its byte order to one of these.
/// Both are 32 bpp with the fourth byte unused or alpha; the name is the
/// bytes in memory, first byte first. R, G, B, X in memory.
pub const SCANOUT_FMT_RGBX: u32 = 0;
/// B, G, R, X in memory.
pub const SCANOUT_FMT_BGRX: u32 = 1;

/// An adopted firmware scanout: the plane at `phys` with `pitch`, `width` x
/// `height` and format `fmt` ([`SCANOUT_FMT_RGBX`] / [`SCANOUT_FMT_BGRX`]).
/// The framebuffer (one screen: `rows` = `height`) and the window format,
/// or `None` for an address of 0, a zero size, a pitch short of a row or not
/// whole pixels, a plane that wraps the address space, or another format.
pub fn adopt_scanout(phys: u64, pitch: u32, width: u32, height: u32, fmt: u32) -> Option<(Fb, u32)> {
    let format = match fmt {
        SCANOUT_FMT_RGBX => FORMAT_RGBA8888,
        SCANOUT_FMT_BGRX => FORMAT_XRGB8888,
        _ => return None,
    };
    if phys == 0 || width == 0 || height == 0 || !pitch.is_multiple_of(4) || (pitch as u64) < width as u64 * 4 {
        return None;
    }
    phys.checked_add(pitch as u64 * height as u64)?;
    Some((Fb { phys, width, height, pitch, rows: height }, format))
}

/// The pixel order the firmware answered and the window's format, for either
/// board: the Pi 4's [`rpi4::pixel_order`] and [`rpi4::window_format`] (the
/// Pi 5's tag is the same; the board checks confirm it).
pub fn window_format(_board: Board, code: u32, value: u32) -> Option<(u32, u32)> {
    let o = rpi4::pixel_order(code, value)?;
    Some((o, rpi4::window_format(o)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_numbers() {
        assert_eq!(Board::from_u32(4), Some(Board::Pi4));
        assert_eq!(Board::from_u32(5), Some(Board::Pi5));
        assert_eq!(Board::from_u32(3), None);
    }

    #[test]
    fn bus_address_per_board() {
        assert_eq!(fb_phys(Board::Pi4, 0xfc10_0000), Some(0x3c10_0000), "the Pi 4 clears the alias");
        assert_eq!(fb_phys(Board::Pi4, 0x3c10_0000), Some(0x3c10_0000));
        assert_eq!(fb_phys(Board::Pi4, 0xc000_0000), None, "nothing behind the alias");
        assert_eq!(fb_phys(Board::Pi4, 0), None);
        assert_eq!(fb_phys(Board::Pi5, 0x3c10_0000), Some(0x3c10_0000), "the Pi 5 is the identity");
        assert_eq!(fb_phys(Board::Pi5, 0xfc10_0000), Some(0xfc10_0000), "no alias to clear");
        assert_eq!(fb_phys(Board::Pi5, 0), None);
    }

    #[test]
    fn allocate_answers() {
        assert!(allocate_ok(1920, 1080, 7680, 7680 * 1080));
        assert!(allocate_ok(1920, 2160, 7680 + 64, (7680 + 64) * 2160), "a wider pitch");
        assert!(!allocate_ok(1920, 1080, 7680, 0), "no size");
        assert!(!allocate_ok(1920, 1080, 7676, 7680 * 1080), "pitch short of a row");
        assert!(!allocate_ok(1920, 1080, 7680, 7680 * 1080 - 1), "size short of the rows");
        assert!(!allocate_ok(0, 1080, 0, 100));
        assert!(!allocate_ok(1920, 0, 7680, 100));
        assert!(!allocate_ok(u32::MAX, 2, u32::MAX, u32::MAX), "no overflow");
    }

    #[test]
    fn fb_from_the_answer() {
        let fb = fb_from_answer(Board::Pi5, 0x3c10_0000, 1920, 1080, 2160, 7680, 7680 * 2160).unwrap();
        assert_eq!((fb.phys, fb.width, fb.height, fb.pitch, fb.rows), (0x3c10_0000, 1920, 1080, 7680, 2160));
        let p4 = fb_from_answer(Board::Pi4, 0xfc10_0000, 1920, 720, 1440, 7680, 7680 * 1440).unwrap();
        assert_eq!(p4.phys, 0x3c10_0000);
        assert!(rpi4::window_is_view(&p4, 1920, 1440));
        assert!(fb_from_answer(Board::Pi5, 0, 1920, 1080, 1080, 7680, 7680 * 1080).is_none());
        assert!(fb_from_answer(Board::Pi5, 0x1000, 1920, 1080, 1080, 7680, 100).is_none());
        assert!(
            fb_from_answer(Board::Pi5, 0x1000, 1920, 1080, 540, 7680, 7680 * 540).is_none(),
            "fewer rows than the screen"
        );
    }

    #[test]
    fn adopted_scanout() {
        let (fb, f) = adopt_scanout(0x3e00_0000, 7680, 1920, 1080, SCANOUT_FMT_BGRX).unwrap();
        assert_eq!((fb.phys, fb.pitch, fb.width, fb.height, fb.rows), (0x3e00_0000, 7680, 1920, 1080, 1080));
        assert_eq!(f, FORMAT_XRGB8888);
        assert_eq!(adopt_scanout(0x3e00_0000, 7680, 1920, 1080, SCANOUT_FMT_RGBX).unwrap().1, FORMAT_RGBA8888);
        assert!(rpi4::window_is_view(&fb, 1920, 1080), "the plane is a window view");
        assert_eq!(rpi4::pan_row(&fb, fb.phys, 1920, 1080, fb.pitch), Some(0));
        assert!(adopt_scanout(0, 7680, 1920, 1080, 0).is_none());
        assert!(adopt_scanout(0x1000, 7680, 1920, 1080, 2).is_none(), "another format");
        assert!(adopt_scanout(0x1000, 7676, 1920, 1080, 0).is_none(), "short pitch");
        assert!(adopt_scanout(0x1000, 7682, 1920, 1080, 0).is_none(), "not whole pixels");
        assert!(adopt_scanout(0x1000, 7680, 0, 1080, 0).is_none());
        assert!(adopt_scanout(0x1000, 7680, 1920, 0, 0).is_none());
        assert!(adopt_scanout(u64::MAX - 100, 7680, 1920, 1080, 0).is_none(), "wraps");
    }

    #[test]
    fn pixel_order_both_boards() {
        for b in [Board::Pi4, Board::Pi5] {
            assert_eq!(window_format(b, rpi4::TAG_ANSWERED | 4, 1), Some((1, FORMAT_RGBA8888)));
            assert_eq!(window_format(b, rpi4::TAG_ANSWERED | 4, 0), Some((0, FORMAT_XRGB8888)));
            assert_eq!(window_format(b, 4, 1), None);
        }
    }
}
