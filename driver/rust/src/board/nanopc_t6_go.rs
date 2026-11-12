// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The NanoPC-T6 image entered by U-Boot's `go`: the MMU and both caches are
//! turned off before anything of Zephyr runs (issue #4627,
//! driver/tools/uboot-t6/README.md, "The route").
//!
//! **What `go` hands over** (U-Boot v2026.07, GPL-2.0: facts and file:line
//! only). U-Boot proper runs at EL2 with the MMU, the data cache and the
//! instruction cache on (common/board_r.c:110-113; arch/arm/cpu/armv8/
//! cache_v8.c:817, :880-894, :1172-1175). `go` is a plain call of the entry
//! (cmd/boot.c:17-22, :41; the `flush()` at :35 is the console's, and the
//! ARM override arch/arm/lib/cmd_boot.c is built only for ARMv7-M,
//! arch/arm/lib/Makefile:35). Nothing on the way from `ext4load` to the entry
//! cleans the data cache: no cache call in cmd/ext4.c:48-51, fs/fs.c:845-865
//! (`do_load`), :625-627 (`_fs_read`) or fs/ext4/ext4fs.c:71-200. The bulk of
//! the file arrives by SDMA (configs/nanopc-t6-rk3588_defconfig:64,
//! drivers/mmc/sdhci.c:104-105, :179-183, with include/linux/dma-mapping.h:
//! 29-42, :55-62 around it), so it is in DRAM; but some bytes are stored by
//! the CPU through its data cache and can still be dirty there at `go`: a
//! sector read in part (fs/fs_internal.c:51, :69, :91; the file's last
//! sector), a hole in the file (fs/ext4/ext4fs.c:188, `memset`), a block
//! from the block cache (drivers/block/blk-uclass.c:441-443).
//!
//! **Why here.** Zephyr's `reset.S` changes only `SCTLR_ELx.A` on the way in
//! (zephyr af17c0c8 arch/arm64/core/reset.S:62-66) and expects the MMU and
//! the data cache off: it zeroes `.bss` and copies `.data` with the MMU off
//! (non-cacheable) while a stale or dirty line of the same address may still
//! be in the cache, and the EL1 MMU it turns on later would hit those lines.
//! The earliest hook it gives is `z_arm64_el2_plat_prep_c` (reset.S:69),
//! before any C code and any stack; the board glue registers
//! `yd_t6_early_entry` there (driver/zephyr/lib/board/nanopc_t6/nanopc_t6.c).
//! Before it only `header.S`'s branch, `__start`'s exception mask and stack
//! address loads and `__reset_prep_c`'s `SCTLR_EL2.A` clear run; none reads
//! a byte the CPU stored. Zephyr's own `CONFIG_ARM64_BOOT_DISABLE_DCACHE`
//! (reset.c:157-170) does the same later, in C, after a stack is in use, by
//! set/way; the entry does it by address, so the order below needs no stack.
//!
//! **The sequence** (`yd_t6_early_entry`, ffi::rk3588; its text is
//! [`t6_go_off_asm`], tested below):
//!
//! 1. Not at EL2 (`CurrentEL`; the glue registers the same entry for EL1):
//!    branch past everything. At EL2 read `SCTLR_EL2`; when M, C and I are
//!    all clear (the kexec entry: Linux's `relocate_kernel` clears them, and
//!    Z1 measured `SCTLR_EL2` 0x30400800 at `T6:A`), branch past
//!    everything too. The kexec path runs seven instructions more than
//!    before, which write only x9 and x10, and sets x1 = 0. No system
//!    register is written on it, no memory is touched.
//! 2. Otherwise clear M, C and I in `SCTLR_EL2`, `isb`. From here no data
//!    line is allocated and no translation is used (VA = PA).
//! 3. Clean and invalidate by address to the point of coherency (`dc civac`)
//!    every line of the image's memory, zephyr,sram: `__rom_region_start`
//!    (the load address, the window's base) to `__kernel_ram_end` (the
//!    window's end), with the line size of `CTR_EL0.DminLine`. A dirty line
//!    reaches DRAM; any line of the window, dirty or clean, leaves the cache,
//!    so the EL1 MMU cannot hit one later. As U-Boot's own `dcache_disable`
//!    (cache_v8.c:915-918), the cache goes off first and is cleaned after,
//!    so no line can be refilled in between; the code in between stores
//!    nothing and loads nothing.
//! 4. `dsb sy`; `ic iallu` and `tlbi alle2` (U-Boot's instructions and
//!    translations), `dsb sy`, `isb`.
//! 5. Pass the `SCTLR_EL2` found (x1), and the range cleaned (x2, x3), to
//!    [`print_go`] for the `T6:G` line.
//!
//! The entry's code itself is fetched before step 2 through U-Boot's caches.
//! It lies in the image's first pages, which arrive by SDMA into DRAM (not
//! the file's last sector, a hole or a block cache entry); the instruction
//! cache holds nothing of the window, since U-Boot invalidates it when it
//! turns it on (cache_v8.c:1172-1175) and never runs code there.

use crate::ip::ns16550_tx::Tx;
use crate::mmio::Mmio;

/// `SCTLR_ELx.M`: the stage 1 MMU.
pub const SCTLR_M: u64 = 1 << 0;
/// `SCTLR_ELx.C`: data accesses cacheable.
pub const SCTLR_C: u64 = 1 << 2;
/// `SCTLR_ELx.I`: instruction fetches cacheable.
pub const SCTLR_I: u64 = 1 << 12;
/// The three bits the entry clears; when none is set it does nothing.
pub const SCTLR_GO_MASK: u64 = SCTLR_M | SCTLR_C | SCTLR_I;

/// What Z1's kexec entry measured at `T6:A` (driver/docs/nanopc-t6-mali.md,
/// the Z1 record; driver/tools/kexec-zephyr/testdata/t6_hello-pass.uart).
pub const SCTLR_EL2_KEXEC_MEASURED: u64 = 0x3040_0800;

/// The `SCTLR_EL2` value the entry writes, or `None` when it leaves the
/// register as found (M, C and I already clear).
pub const fn sctlr_off(found: u64) -> Option<u64> {
    if found & SCTLR_GO_MASK == 0 {
        None
    } else {
        Some(found & !SCTLR_GO_MASK)
    }
}

/// The smallest data cache line in bytes: `CTR_EL0.DminLine` (bits 19:16) is
/// its log2 in 4-byte words.
pub const fn dcache_line(ctr_el0: u64) -> u64 {
    4 << ((ctr_el0 >> 16) & 0xf)
}

/// The addresses `dc civac` is given for `[start, end)`, as the entry's loop
/// walks them: from `start` rounded down to the line, one line at a time,
/// while below `end`.
pub fn clean_lines(start: u64, end: u64, line: u64) -> impl Iterator<Item = u64> {
    let first = start & !(line - 1);
    (first..end).step_by(line as usize)
}

/// The text of the entry's first instructions, shared by `global_asm!` in
/// ffi::rk3588 and the tests here. Registers: x9..x13 scratch; x0 (the
/// flags from the glue), x22..x25 and x30 are not touched; x1 is set to the
/// `SCTLR_EL2` found on the go path and to 0 on the other, x2 and x3 to the
/// range cleaned (go path only). The mask is [`SCTLR_GO_MASK`], checked by
/// `the_asm_masks_the_three_bits`.
#[cfg(any(target_arch = "aarch64", test))]
macro_rules! t6_go_off_asm {
    () => {
        concat!(
            "    mrs x9, CurrentEL\n",
            "    cmp x9, #0x8\n",
            "    b.ne 3f\n",
            "    mrs x9, sctlr_el2\n",
            "    mov x10, #0x1005\n",
            "    tst x9, x10\n",
            "    b.eq 3f\n",
            "    bic x10, x9, x10\n",
            "    msr sctlr_el2, x10\n",
            "    isb\n",
            "    adrp x2, __rom_region_start\n",
            "    add x2, x2, :lo12:__rom_region_start\n",
            "    adrp x3, __kernel_ram_end\n",
            "    add x3, x3, :lo12:__kernel_ram_end\n",
            "    mrs x11, ctr_el0\n",
            "    ubfx x11, x11, #16, #4\n",
            "    mov x12, #4\n",
            "    lsl x12, x12, x11\n",
            "    sub x13, x12, #1\n",
            "    bic x11, x2, x13\n",
            "2:\n",
            "    dc civac, x11\n",
            "    add x11, x11, x12\n",
            "    cmp x11, x3\n",
            "    b.lo 2b\n",
            "    dsb sy\n",
            "    ic iallu\n",
            "    tlbi alle2\n",
            "    dsb sy\n",
            "    isb\n",
            "    mov x1, x9\n",
            "    b 4f\n",
            "3:\n",
            "    mov x1, xzr\n",
            "4:\n",
        )
    };
}
#[cfg(any(target_arch = "aarch64", test))]
pub(crate) use t6_go_off_asm;

/// `T6:G`, printed after `T6:A` when the entry found the MMU or a cache on
/// (U-Boot's `go`); the kexec entry prints no `T6:G`.
pub fn banner_g<M: Mmio>(tx: &mut Tx<'_, M>, found: u64, start: u64, end: u64) {
    tx.str(b"T6:G go: SCTLR_EL2 ");
    tx.hex(found, 16);
    tx.str(b" -> ");
    tx.hex(found & !SCTLR_GO_MASK, 16);
    tx.str(b" (MMU, D-cache, I-cache off); ");
    tx.hex(start, 8);
    tx.str(b"-");
    tx.hex(end, 8);
    tx.str(b" cleaned and invalidated to PoC, TLB and I-cache invalidated\r\n");
}

/// The `T6:G` line when the entry took the go path (`found` not 0).
pub fn print_go<M: Mmio>(tx: &mut Tx<'_, M>, found: u64, start: u64, end: u64) {
    if found != 0 {
        banner_g(tx, found, start, end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ip::ns16550_tx::tests::{Uart, BASE};

    /// U-Boot's `SCTLR_EL2` at `go`: M, C and I set (cache_v8.c:817, :894,
    /// :1175); the other bits are illustrative, not measured.
    const UBOOT_SCTLR: u64 = 0x30c5_1835;

    #[test]
    fn the_kexec_state_leaves_sctlr_as_found() {
        assert_eq!(sctlr_off(SCTLR_EL2_KEXEC_MEASURED), None);
        // Any value with M, C and I clear, whatever the rest.
        assert_eq!(sctlr_off(!SCTLR_GO_MASK), None);
        assert_eq!(sctlr_off(0), None);
    }

    #[test]
    fn the_go_state_clears_exactly_m_c_and_i() {
        assert_eq!(sctlr_off(UBOOT_SCTLR), Some(0x30c5_0830));
        // One bit of the three is enough to take the go path.
        for b in [SCTLR_M, SCTLR_C, SCTLR_I] {
            assert_eq!(sctlr_off(SCTLR_EL2_KEXEC_MEASURED | b), Some(SCTLR_EL2_KEXEC_MEASURED));
        }
        // A (bit 1), SA (bit 3), WXN (bit 19), EE (bit 25) stay as found.
        let others = (1 << 1) | (1 << 3) | (1 << 19) | (1 << 25);
        assert_eq!(sctlr_off(others | SCTLR_GO_MASK), Some(others));
    }

    #[test]
    fn the_line_size_is_ctr_el0_dminline() {
        // Cortex-A55 and A76 report 64-byte lines: DminLine 4.
        assert_eq!(dcache_line(0x8444_c004), 64);
        assert_eq!(dcache_line(0x9444_c004), 64);
        assert_eq!(dcache_line(0x0003_0000), 32);
    }

    #[test]
    fn the_walk_covers_every_line_of_the_window_once() {
        let (start, end, line) = (0x1000_0000u64, 0x2000_0000u64, 64u64);
        let mut n = 0u64;
        let mut last = None;
        for a in clean_lines(start, end, line) {
            assert_eq!(a % line, 0);
            if let Some(p) = last {
                assert_eq!(a, p + line);
            }
            last = Some(a);
            n += 1;
        }
        assert_eq!(n, (end - start) / line);
        assert_eq!(last, Some(end - line));
    }

    #[test]
    fn an_unaligned_range_starts_at_its_line_and_ends_on_the_last_line_it_touches() {
        let v: Vec<u64> = clean_lines(0x1000_0030, 0x1000_0081, 64).collect();
        assert_eq!(v, vec![0x1000_0000, 0x1000_0040, 0x1000_0080]);
        assert_eq!(clean_lines(0x1000_0000, 0x1000_0000, 64).count(), 0);
    }

    fn lines() -> Vec<&'static str> {
        // by path, as ffi::rk3588 names it
        crate::board::nanopc_t6_go::t6_go_off_asm!().lines().map(str::trim).collect()
    }

    /// The register names an instruction mentions (x0..x30, sp, xzr).
    fn regs(s: &str) -> Vec<&str> {
        s.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .filter(|t| *t == "sp" || (t.starts_with('x') && t.len() > 1 && t[1..].bytes().all(|b| b.is_ascii_digit())))
            .collect()
    }

    /// The register an instruction writes: the first operand of a mov, mrs or
    /// ALU instruction; None for tst, branches, barriers, msr and cache ops.
    fn dest(s: &str) -> Option<&str> {
        let op = s.split_whitespace().next()?;
        if ["mov", "mrs", "bic", "add", "sub", "lsl", "ubfx", "adrp"].contains(&op) {
            regs(s).first().copied()
        } else {
            None
        }
    }

    #[test]
    fn the_asm_masks_the_three_bits() {
        let l = lines();
        assert_eq!(l[4], format!("mov x10, #{:#x}", SCTLR_GO_MASK));
    }

    #[test]
    fn the_kexec_path_writes_no_system_register_and_only_x1_x9_x10() {
        // The path with M, C and I clear: the first seven instructions, then
        // from label 3 to label 4. (Entered at EL1 the path is the first
        // three, then the same label 3.)
        let l = lines();
        assert_eq!(
            &l[..7],
            &[
                "mrs x9, CurrentEL",
                "cmp x9, #0x8",
                "b.ne 3f",
                "mrs x9, sctlr_el2",
                "mov x10, #0x1005",
                "tst x9, x10",
                "b.eq 3f"
            ]
        );
        let at3 = l.iter().position(|s| *s == "3:").unwrap();
        let at4 = l.iter().position(|s| *s == "4:").unwrap();
        assert_eq!(&l[at3 + 1..at4], &["mov x1, xzr"]);
        assert_eq!(at4, l.len() - 1, "label 4 ends the text: the old entry follows");
        let path: Vec<&str> = l[..7].iter().chain(&l[at3 + 1..at4]).copied().collect();
        let mut written = Vec::new();
        for s in &path {
            assert!(!s.starts_with("msr") && !s.starts_with("dc ") && !s.starts_with("tlbi") && !s.starts_with("ic "));
            if let Some(r) = dest(s) {
                written.push(r);
            }
        }
        assert_eq!(written, vec!["x9", "x9", "x10", "x1"]);
    }

    #[test]
    fn the_go_path_turns_off_before_it_cleans_and_never_touches_memory_or_the_kept_registers() {
        let l = lines();
        let pos = |s: &str| l.iter().position(|x| *x == s).unwrap_or_else(|| panic!("{s}"));
        assert!(pos("msr sctlr_el2, x10") < pos("isb"));
        assert!(pos("isb") < pos("dc civac, x11"));
        assert!(pos("dc civac, x11") < pos("dsb sy"));
        assert!(pos("dsb sy") < pos("ic iallu") && pos("ic iallu") < pos("tlbi alle2"));
        for s in &l {
            // no load or store, and x0, x22..x25, x30 and sp not named at all
            let op = s.split_whitespace().next().unwrap_or("");
            assert!(!op.starts_with("ld") && !op.starts_with("st"), "{s}");
            for r in regs(s) {
                assert!(!["x0", "x22", "x23", "x24", "x25", "x30", "sp"].contains(&r), "{s}");
            }
        }
    }

    fn sent(f: impl FnOnce(&mut Tx<'_, Uart>)) -> String {
        let u = Uart::linux_left(1);
        {
            let mut tx = Tx::new(&u, BASE);
            f(&mut tx);
        }
        String::from_utf8(u.sent()).unwrap()
    }

    #[test]
    fn the_g_line_names_the_values_found_and_left_and_the_range() {
        assert_eq!(
            sent(|tx| print_go(tx, UBOOT_SCTLR, 0x1000_0000, 0x2000_0000)),
            "T6:G go: SCTLR_EL2 0x0000000030c51835 -> 0x0000000030c50830 (MMU, D-cache, I-cache off); \
             0x10000000-0x20000000 cleaned and invalidated to PoC, TLB and I-cache invalidated\r\n"
        );
        // The kexec entry: x1 = 0, no line.
        assert_eq!(sent(|tx| print_go(tx, 0, 0, 0)), "");
    }
}
