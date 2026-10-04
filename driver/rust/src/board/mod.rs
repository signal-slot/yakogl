// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The board layer's Rust side (driver/zephyr/lib/board/,
//! driver/docs/one-method.md): the result record, defined once
//! ([`record`]), the Raspberry Pi 4's framebuffer decisions ([`rpi4`]), and
//! the i.MX8MP samples' hold before their reset ([`imx8mp`]).
//! The C side reaches both through `ffi::board`.

pub mod imx8mp;
pub mod record;
pub mod rpi4;

#[cfg(test)]
mod tests {
    use super::record;

    /// `driver/zephyr/lib/board/include/yakogl_record.h` is
    /// `record::write_c_header`'s output, byte for byte. With
    /// YK_RECORD_BLESS=1 the test writes the file instead.
    #[test]
    fn c_header_is_generated_from_the_table() {
        let mut want = String::new();
        record::write_c_header(&mut want).unwrap();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../zephyr/lib/board/include/yakogl_record.h");
        if std::env::var_os("YK_RECORD_BLESS").is_some() {
            std::fs::write(path, &want).unwrap();
        }
        let have = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            have == want,
            "{path} is not record.rs's table: run YK_RECORD_BLESS=1 cargo test -p yakogl-drivers board"
        );
    }
}
