// SPDX-License-Identifier: Apache-2.0
//! CRC-32 (IEEE 802.3, reflected, polynomial 0xEDB88320, initial and final
//! XOR 0xFFFFFFFF): the value Python's `zlib.crc32` and U-Boot's FIT `crc32`
//! hash give, so `tools/am62p-mkboot.py` computes it with the standard library.

const fn make_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static TABLE: [u32; 256] = make_table();

/// Continue a CRC over `data`: start from `crc32_update(0, ...)`, feed the
/// previous result to continue. On an AArch64 core with the CRC32
/// instructions (ID_AA64ISAR0_EL1.CRC32, the A53s of the AM62P and the
/// i.MX8MP have them) the same polynomial runs in hardware: the table loop
/// took 0.48 s per pass over the AM62P's 66 MB Qt image on 2026-10-22.
pub fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    #[cfg(all(target_arch = "aarch64", not(test)))]
    if hw_crc32() {
        // SAFETY: the instructions exist (checked just above).
        return unsafe { crc32_update_hw(crc, data) };
    }
    crc32_update_table(crc, data)
}

/// ID_AA64ISAR0_EL1 bits 19:16 (CRC32) are non-zero when CRC32B/H/W/X exist.
#[cfg(all(target_arch = "aarch64", not(test)))]
fn hw_crc32() -> bool {
    let isar0: u64;
    // SAFETY: a read of an ID register, allowed at EL1 (the loader's level).
    unsafe { core::arch::asm!("mrs {}, ID_AA64ISAR0_EL1", out(reg) isar0, options(nomem, nostack)) };
    (isar0 >> 16) & 0xf != 0
}

/// The hardware loop: one aligned 8-byte load per CRC32X.
///
/// The loader's target (aarch64-unknown-none-softfloat) has `strict-align`,
/// so a `u64` taken from eight bytes at an address the compiler cannot
/// prove aligned (`as_chunks::<8>()` and `u64::from_le_bytes`, the first
/// form of this loop) is put together from eight byte loads, shifts and
/// ORs: 22 instructions for each 8 bytes, 525 MB/s on the i.MX8MP's A53
/// (13.65 MB of stored bytes and images in 26 ms, 2026-11-15). `align_to`
/// gives the bytes up to the first 8-byte boundary, the aligned words and
/// the rest; a word is then one `ldr` and the loop is four instructions
/// (load, CRC32X, compare, branch).
#[cfg(all(target_arch = "aarch64", not(test)))]
#[target_feature(enable = "crc")]
unsafe fn crc32_update_hw(crc: u32, data: &[u8]) -> u32 {
    use core::arch::aarch64::{__crc32b, __crc32d};
    let mut c = !crc;
    // SAFETY: any bit pattern is a u64, and `align_to` returns words only
    // at addresses aligned for one, inside `data`.
    let (head, words, tail) = unsafe { data.align_to::<u64>() };
    for &b in head {
        c = __crc32b(c, b);
    }
    for &w in words {
        // The CRC takes the bytes in memory order: the word as little endian.
        c = __crc32d(c, u64::from_le(w));
    }
    for &b in tail {
        c = __crc32b(c, b);
    }
    !c
}

/// The table version: the host (tests) and a core without the instructions.
pub fn crc32_update_table(crc: u32, data: &[u8]) -> u32 {
    let mut c = !crc;
    for &b in data {
        c = TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

pub fn crc32(data: &[u8]) -> u32 {
    crc32_update(0, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_value() {
        // The CRC-32 check value of "123456789" (and zlib.crc32(b"123456789")).
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    /// The split the hardware loop makes (bytes to the first 8-byte
    /// boundary, aligned words, the rest), with the table in place of the
    /// instructions: every start alignment and every length around the word
    /// size gives the bytewise value. The instructions themselves run only
    /// on the target.
    #[test]
    fn aligned_word_split_equals_bytewise() {
        let buf: [u8; 96] = core::array::from_fn(|i| (i * 131 + 17) as u8);
        for start in 0..16 {
            for len in 0..=64 {
                let data = &buf[start..start + len];
                // SAFETY: any bit pattern is a u64.
                let (head, words, tail) = unsafe { data.align_to::<u64>() };
                assert!(head.len() < 8 && tail.len() < 8 && head.len() + words.len() * 8 + tail.len() == len);
                let mut c = crc32_update_table(0x1234_5678, head);
                for &w in words {
                    c = crc32_update_table(c, &u64::from_le(w).to_le_bytes());
                }
                c = crc32_update_table(c, tail);
                assert_eq!(c, crc32_update_table(0x1234_5678, data), "start {start} len {len}");
            }
        }
    }

    #[test]
    fn incremental_equals_whole() {
        let data: [u8; 1000] = core::array::from_fn(|i| (i * 31 + 7) as u8);
        let whole = crc32(&data);
        let part = crc32_update(crc32(&data[..333]), &data[333..]);
        assert_eq!(whole, part);
    }
}
