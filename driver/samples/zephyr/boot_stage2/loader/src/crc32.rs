// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
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
/// previous result to continue.
pub fn crc32_update(crc: u32, data: &[u8]) -> u32 {
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

    #[test]
    fn incremental_equals_whole() {
        let data: [u8; 1000] = core::array::from_fn(|i| (i * 31 + 7) as u8);
        let whole = crc32(&data);
        let part = crc32_update(crc32(&data[..333]), &data[333..]);
        assert_eq!(whole, part);
    }
}
