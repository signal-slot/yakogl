// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! errno numbers, one table per C library (src/osal/OSAL_NOTES.md, "errno
//! numbers at the C boundaries"). A negative errno crosses between Rust and
//! C as a bare number, so a Rust crate must use the numbering of the C
//! library on the other side of that boundary:
//!
//! - the driver crate (`yakogl_drivers::errno`): [`ZEPHYR`], its only C
//!   library;
//! - the YakoGL crate (`src/util.rs`, which includes this file): [`numbering`]
//!   of its OSAL, [`ZEPHYR`] for the Zephyr OSALs and [`LINUX`] for the
//!   others.
//!
//! The names below 35 and `ENODATA` are the same in both; `ESHUTDOWN`,
//! `ETIMEDOUT`, `EMSGSIZE`, `ENOTSUP` and `ECANCELED` are not, and Zephyr's
//! `ESHUTDOWN` is Linux's `ETIMEDOUT`.

/// The errno values our code uses, in one C library's numbering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Numbering {
    pub enoent: i32,
    pub eio: i32,
    pub eagain: i32,
    pub enomem: i32,
    pub efault: i32,
    pub ebusy: i32,
    pub enodev: i32,
    pub einval: i32,
    pub enospc: i32,
    pub erange: i32,
    pub enodata: i32,
    pub eshutdown: i32,
    pub etimedout: i32,
    pub emsgsize: i32,
    pub enotsup: i32,
    pub ecanceled: i32,
}

/// Zephyr's C libraries: `lib/libc/minimal/include/errno.h` (zephyr
/// af17c0c8aa2a, the line in each comment) and picolibc's
/// `newlib/libc/include/sys/errno.h`, which give the same numbers for every
/// name here.
pub const ZEPHYR: Numbering = Numbering {
    enoent: 2,      // :41
    eio: 5,         // :44
    eagain: 11,     // :50
    enomem: 12,     // :51
    efault: 14,     // :53
    ebusy: 16,      // :55
    enodev: 19,     // :58
    einval: 22,     // :61
    enospc: 28,     // :67
    erange: 34,     // :73
    enodata: 61,    // :78
    eshutdown: 110, // :95
    etimedout: 116, // :101
    emsgsize: 122,  // :107
    enotsup: 134,   // :115
    ecanceled: 140, // :118
};

/// Linux: `include/uapi/asm-generic/errno-base.h` (the names up to 34) and
/// `errno.h` (the rest); glibc defines `ENOTSUP` as `EOPNOTSUPP` (95).
pub const LINUX: Numbering = Numbering {
    enoent: 2,
    eio: 5,
    eagain: 11,
    enomem: 12,
    efault: 14,
    ebusy: 16,
    enodev: 19,
    einval: 22,
    enospc: 28,
    erange: 34,
    enodata: 61,
    eshutdown: 108,
    etimedout: 110,
    emsgsize: 90,
    enotsup: 95,
    ecanceled: 125,
};

/// The numbering a build's C side uses: [`ZEPHYR`] when the OSAL is one of
/// the Zephyr ones (`src/osal/zephyr*.c`, built against Zephyr's libc), else
/// [`LINUX`] (`linux.c` against glibc; `baremetal.c`
/// returns `LINUX`'s literal -95 for `ENOTSUP`).
pub const fn numbering(osal_zephyr: bool) -> Numbering {
    if osal_zephyr {
        ZEPHYR
    } else {
        LINUX
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(n: &Numbering) -> [(&'static str, i32); 16] {
        [
            ("ENOENT", n.enoent),
            ("EIO", n.eio),
            ("EAGAIN", n.eagain),
            ("ENOMEM", n.enomem),
            ("EFAULT", n.efault),
            ("EBUSY", n.ebusy),
            ("ENODEV", n.enodev),
            ("EINVAL", n.einval),
            ("ENOSPC", n.enospc),
            ("ERANGE", n.erange),
            ("ENODATA", n.enodata),
            ("ESHUTDOWN", n.eshutdown),
            ("ETIMEDOUT", n.etimedout),
            ("EMSGSIZE", n.emsgsize),
            ("ENOTSUP", n.enotsup),
            ("ECANCELED", n.ecanceled),
        ]
    }

    /// Within one C library every name is its own number: a value read back
    /// names exactly one cause.
    #[test]
    fn each_numbering_is_one_to_one() {
        for n in [&ZEPHYR, &LINUX] {
            let v = values(n);
            for (i, a) in v.iter().enumerate() {
                assert!(a.1 > 0, "{} must be positive", a.0);
                for b in &v[i + 1..] {
                    assert_ne!(a.1, b.1, "{} and {} share {}", a.0, b.0, a.1);
                }
            }
        }
    }

    /// The names that differ are exactly the five the doc lists, and the
    /// collision it names is there: Zephyr's ESHUTDOWN is Linux's ETIMEDOUT.
    #[test]
    fn the_numberings_differ_where_the_doc_says() {
        let z = values(&ZEPHYR);
        let l = values(&LINUX);
        let mut differ = [""; 16];
        let mut n = 0;
        for (a, b) in z.iter().zip(l.iter()) {
            if a.1 != b.1 {
                differ[n] = a.0;
                n += 1;
            }
        }
        assert_eq!(differ[..n], ["ESHUTDOWN", "ETIMEDOUT", "EMSGSIZE", "ENOTSUP", "ECANCELED"]);
        assert_eq!(ZEPHYR.eshutdown, LINUX.etimedout);
        assert_eq!(numbering(true), ZEPHYR);
        assert_eq!(numbering(false), LINUX);
    }
}
