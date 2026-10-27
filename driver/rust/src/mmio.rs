// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! 32-bit register access.
//!
//! A driver holds the base address of each block it uses (what the shim's
//! `device_map()` returned) and reads and writes through an [`Mmio`]. On the
//! board that is [`Hw`], which reproduces Zephyr's arm64 `sys_read32` /
//! `sys_write32` (include/zephyr/arch/arm64/sys_io.h): a read is `ldr w`
//! followed by `dmb sy`, a write is `dmb sy` followed by `str w`, inline, with
//! no call per access. In the host tests it is the register model
//! ([`crate::model`]), which sees the same addresses and records every access.

/// Register access by absolute (virtual) address.
///
/// `read8` / `write8` are for blocks whose registers are one byte each at
/// consecutive byte addresses (the Synopsys HDMI TX controller of the
/// i.MX 8M Plus, IMX8MPRM Rev. 3 13.9.3: "Width 8", offsets 0h, 1h, 2h...);
/// a 32-bit access there would touch four registers at once.
pub trait Mmio {
    fn read32(&self, addr: usize) -> u32;
    fn write32(&self, addr: usize, val: u32);
    fn read8(&self, addr: usize) -> u8;
    fn write8(&self, addr: usize, val: u8);

    /// Read-modify-write: clear `mask`, then set `val & mask`.
    fn modify32(&self, addr: usize, mask: u32, val: u32) {
        let v = self.read32(addr);
        self.write32(addr, (v & !mask) | (val & mask));
    }
}

/// `val` placed in the field `mask` (a contiguous run of ones), cut to it.
pub const fn field(mask: u32, val: u32) -> u32 {
    (val << mask.trailing_zeros()) & mask
}

/// The field `mask` of `reg`, shifted down.
pub const fn get(mask: u32, reg: u32) -> u32 {
    (reg & mask) >> mask.trailing_zeros()
}

/// The hardware. Constructing one is the promise that every address a driver
/// is handed is mapped device memory.
pub struct Hw(());

impl Hw {
    /// # Safety
    /// Every address later passed to `read32` / `write32` must be a mapped,
    /// 4-byte aligned device register (the shim's `device_map()` ranges).
    pub const unsafe fn new() -> Hw {
        Hw(())
    }
}

#[cfg(target_arch = "aarch64")]
impl Mmio for Hw {
    #[inline(always)]
    fn read32(&self, addr: usize) -> u32 {
        let val: u32;
        // SAFETY: `Hw::new`'s contract. `ldr w` then `dmb sy`, as sys_read32.
        unsafe {
            core::arch::asm!(
                "ldr {v:w}, [{a}]",
                "dmb sy",
                v = out(reg) val,
                a = in(reg) addr,
                options(nostack, preserves_flags),
            );
        }
        val
    }

    #[inline(always)]
    fn write32(&self, addr: usize, val: u32) {
        // SAFETY: `Hw::new`'s contract. `dmb sy` then `str w`, as sys_write32.
        unsafe {
            core::arch::asm!(
                "dmb sy",
                "str {v:w}, [{a}]",
                v = in(reg) val,
                a = in(reg) addr,
                options(nostack, preserves_flags),
            );
        }
    }

    #[inline(always)]
    fn read8(&self, addr: usize) -> u8 {
        let val: u32;
        // SAFETY: `Hw::new`'s contract. `ldrb` then `dmb sy`, as sys_read8.
        unsafe {
            core::arch::asm!(
                "ldrb {v:w}, [{a}]",
                "dmb sy",
                v = out(reg) val,
                a = in(reg) addr,
                options(nostack, preserves_flags),
            );
        }
        val as u8
    }

    #[inline(always)]
    fn write8(&self, addr: usize, val: u8) {
        // SAFETY: `Hw::new`'s contract. `dmb sy` then `strb`, as sys_write8.
        unsafe {
            core::arch::asm!(
                "dmb sy",
                "strb {v:w}, [{a}]",
                v = in(reg) u32::from(val),
                a = in(reg) addr,
                options(nostack, preserves_flags),
            );
        }
    }
}

/// Other architectures (a host `cargo check` of the staticlib): volatile
/// accesses, never run against hardware.
#[cfg(not(target_arch = "aarch64"))]
impl Mmio for Hw {
    fn read32(&self, addr: usize) -> u32 {
        // SAFETY: `Hw::new`'s contract.
        unsafe { core::ptr::read_volatile(addr as *const u32) }
    }

    fn write32(&self, addr: usize, val: u32) {
        // SAFETY: `Hw::new`'s contract.
        unsafe { core::ptr::write_volatile(addr as *mut u32, val) }
    }

    fn read8(&self, addr: usize) -> u8 {
        // SAFETY: `Hw::new`'s contract.
        unsafe { core::ptr::read_volatile(addr as *const u8) }
    }

    fn write8(&self, addr: usize, val: u8) {
        // SAFETY: `Hw::new`'s contract.
        unsafe { core::ptr::write_volatile(addr as *mut u8, val) }
    }
}

/// 16-bit register access, for blocks whose registers are 16 bits wide at
/// 2-byte steps (the i.MX WDOG: `WCR` at 0h, `WSR` at 2h, ...; hal_nxp
/// `WDOG_Type`, every member `uint16_t`). A 32-bit access there would write
/// two registers at once. Kept apart from [`Mmio`] so that only the drivers
/// that need it, and their test models, implement it.
pub trait Mmio16 {
    fn read16(&self, addr: usize) -> u16;
    fn write16(&self, addr: usize, val: u16);
}

#[cfg(target_arch = "aarch64")]
impl Mmio16 for Hw {
    #[inline(always)]
    fn read16(&self, addr: usize) -> u16 {
        let val: u32;
        // SAFETY: `Hw::new`'s contract. `ldrh` then `dmb sy`, as sys_read16.
        unsafe {
            core::arch::asm!(
                "ldrh {v:w}, [{a}]",
                "dmb sy",
                v = out(reg) val,
                a = in(reg) addr,
                options(nostack, preserves_flags),
            );
        }
        val as u16
    }

    #[inline(always)]
    fn write16(&self, addr: usize, val: u16) {
        // SAFETY: `Hw::new`'s contract. `dmb sy` then `strh`, as sys_write16.
        unsafe {
            core::arch::asm!(
                "dmb sy",
                "strh {v:w}, [{a}]",
                v = in(reg) u32::from(val),
                a = in(reg) addr,
                options(nostack, preserves_flags),
            );
        }
    }
}

/// Other architectures: volatile accesses, never run against hardware.
#[cfg(not(target_arch = "aarch64"))]
impl Mmio16 for Hw {
    fn read16(&self, addr: usize) -> u16 {
        // SAFETY: `Hw::new`'s contract.
        unsafe { core::ptr::read_volatile(addr as *const u16) }
    }

    fn write16(&self, addr: usize, val: u16) {
        // SAFETY: `Hw::new`'s contract.
        unsafe { core::ptr::write_volatile(addr as *mut u16, val) }
    }
}
