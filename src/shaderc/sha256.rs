// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Self-contained SHA-256 (FIPS 180-4). No crypto crate, nothing beyond
//! `core`, so it links on Zephyr / bare metal.
//!
//! Used by the offline shader back end to hash glShaderSource() text and look
//! the result up in the build-time blob table (see `shaderc/offline.rs`). The
//! exact same hash is produced by `yakogl-tools glsl2usc` at build time, so a shader
//! authored into the table is found at runtime by its source bytes.
//!
//! A compact implementation. Not constant-time; it is used only for a
//! content-addressed lookup, never for a security decision.

pub const SHA256_DIGEST_LEN: usize = 32;

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
    0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
    0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
    0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
    0xc67178f2,
];

#[inline]
fn ch(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (!x & z)
}

#[inline]
fn maj(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (x & z) ^ (y & z)
}

#[inline]
fn ep0(x: u32) -> u32 {
    x.rotate_right(2) ^ x.rotate_right(13) ^ x.rotate_right(22)
}

#[inline]
fn ep1(x: u32) -> u32 {
    x.rotate_right(6) ^ x.rotate_right(11) ^ x.rotate_right(25)
}

#[inline]
fn sig0(x: u32) -> u32 {
    x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3)
}

#[inline]
fn sig1(x: u32) -> u32 {
    x.rotate_right(17) ^ x.rotate_right(19) ^ (x >> 10)
}

/// Incremental hashing context (`struct sha256_ctx`).
#[derive(Clone)]
pub struct Sha256 {
    state: [u32; 8],
    bitlen: u64,
    datalen: usize,
    data: [u8; 64],
}

impl Sha256 {
    pub const fn new() -> Self {
        Sha256 {
            state: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19],
            bitlen: 0,
            datalen: 0,
            data: [0; 64],
        }
    }

    fn transform(&mut self) {
        let mut m = [0u32; 64];

        for (w, b) in m.iter_mut().zip(self.data.as_chunks::<4>().0) {
            *w = u32::from_be_bytes(*b);
        }
        for i in 16..64 {
            m[i] = sig1(m[i - 2]).wrapping_add(m[i - 7]).wrapping_add(sig0(m[i - 15])).wrapping_add(m[i - 16]);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;

        for i in 0..64 {
            let t1 = h.wrapping_add(ep1(e)).wrapping_add(ch(e, f, g)).wrapping_add(K[i]).wrapping_add(m[i]);
            let t2 = ep0(a).wrapping_add(maj(a, b, c));
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }

        for (s, v) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        for &byte in data {
            self.data[self.datalen] = byte;
            self.datalen += 1;
            if self.datalen == 64 {
                self.transform();
                self.bitlen = self.bitlen.wrapping_add(512);
                self.datalen = 0;
            }
        }
    }

    pub fn finalize(mut self) -> [u8; SHA256_DIGEST_LEN] {
        let n = self.datalen;

        /* Pad. */
        self.data[n] = 0x80;
        if n < 56 {
            self.data[n + 1..56].fill(0x00);
        } else {
            self.data[n + 1..].fill(0x00);
            self.transform();
            self.data[..56].fill(0x00);
        }

        /* Append length in bits, big-endian. */
        self.bitlen = self.bitlen.wrapping_add(n as u64 * 8);
        self.data[56..].copy_from_slice(&self.bitlen.to_be_bytes());
        self.transform();

        /* Emit big-endian digest. */
        let mut out = [0u8; SHA256_DIGEST_LEN];
        for (o, s) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            *o = s.to_be_bytes();
        }
        out
    }
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience one-shot: the digest of `data`.
pub fn sha256(data: &[u8]) -> [u8; SHA256_DIGEST_LEN] {
    let mut c = Sha256::new();
    c.update(data);
    c.finalize()
}
