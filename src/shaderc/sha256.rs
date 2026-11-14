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
//! content-addressed lookup, never for a security decision. On a bare-metal
//! or Zephyr AArch64 core with the SHA-2 instructions the compression
//! function is those (`hw` below); the digest is the same bytes.

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

/// The compression function over whole 64-byte blocks, in portable Rust.
fn compress_soft(state: &mut [u32; 8], blocks: &[u8]) {
    for block in blocks.as_chunks::<64>().0 {
        let mut m = [0u32; 64];

        for (w, b) in m.iter_mut().zip(block.as_chunks::<4>().0) {
            *w = u32::from_be_bytes(*b);
        }
        for i in 16..64 {
            m[i] = sig1(m[i - 2]).wrapping_add(m[i - 7]).wrapping_add(sig0(m[i - 15])).wrapping_add(m[i - 16]);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;

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

        for (s, v) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }
}

/// The ARMv8 SHA-2 instructions (SHA256H, SHA256H2, SHA256SU0, SHA256SU1),
/// on a bare-metal or Zephyr AArch64 image whose core has them.
///
/// Why: a shader look-up is keyed by the SHA-256 of the source
/// (`record_key`), and the GL core asks at every compile, link and variant.
/// Slint's start on the i.MX8MP links FemtoVG's 35 programs and hashes about
/// 317 KB of GLSL on the way to its first frame; the portable code above
/// took 2.19 ms for that on an x86-64 host (145 MB/s, 2026-11-15) and is
/// several times slower on a Cortex-A53. The instructions do four rounds
/// each, with the message schedule in two more.
///
/// The check is ID_AA64ISAR0_EL1.SHA2 (bits 15:12), read once. The register
/// is readable at EL1 and above, which is where this library runs on these
/// targets (the GL client is a kernel thread: it maps the GPU); a hosted
/// AArch64 build (Linux, where the read would be the kernel's to emulate)
/// keeps the portable code. The keys are the same bytes either way: the
/// digests are checked against each other and the FIPS vectors.
#[cfg(all(target_arch = "aarch64", target_os = "none", target_feature = "neon"))]
mod hw {
    use super::K;
    use core::arch::aarch64::{
        vaddq_u32, vld1q_u32, vld1q_u8, vreinterpretq_u32_u8, vrev32q_u8, vsha256h2q_u32, vsha256hq_u32,
        vsha256su0q_u32, vsha256su1q_u32, vst1q_u32,
    };
    use core::sync::atomic::{AtomicU8, Ordering};

    /// 0: not looked yet; 1: the core has no SHA-2 instructions; 2: it has.
    static HAVE: AtomicU8 = AtomicU8::new(0);

    pub fn available() -> bool {
        match HAVE.load(Ordering::Relaxed) {
            0 => {
                let isar0: u64;
                // SAFETY: a read of an ID register, allowed at EL1 (see the module's comment).
                unsafe { core::arch::asm!("mrs {}, ID_AA64ISAR0_EL1", out(reg) isar0, options(nomem, nostack)) };
                let have = (isar0 >> 12) & 0xf != 0;
                HAVE.store(if have { 2 } else { 1 }, Ordering::Relaxed);
                have
            }
            v => v == 2,
        }
    }

    /// # Safety
    /// The core has the SHA-2 instructions ([`available`]).
    #[target_feature(enable = "sha2")]
    pub unsafe fn compress(state: &mut [u32; 8], blocks: &[u8]) {
        // SAFETY: `state` is eight words, each block 64 bytes, K 64 words;
        // every load and store below is inside one of them.
        unsafe {
            let mut abcd = vld1q_u32(state.as_ptr());
            let mut efgh = vld1q_u32(state.as_ptr().add(4));
            for block in blocks.as_chunks::<64>().0 {
                let (abcd0, efgh0) = (abcd, efgh);
                /* the sixteen message words, big endian, four to a vector */
                let mut w = [
                    vreinterpretq_u32_u8(vrev32q_u8(vld1q_u8(block.as_ptr()))),
                    vreinterpretq_u32_u8(vrev32q_u8(vld1q_u8(block.as_ptr().add(16)))),
                    vreinterpretq_u32_u8(vrev32q_u8(vld1q_u8(block.as_ptr().add(32)))),
                    vreinterpretq_u32_u8(vrev32q_u8(vld1q_u8(block.as_ptr().add(48)))),
                ];
                for i in 0..16 {
                    /* rounds 4i .. 4i+3 with words 4i .. 4i+3 */
                    let t = vaddq_u32(w[i & 3], vld1q_u32(K.as_ptr().add(4 * i)));
                    let prev = abcd;
                    abcd = vsha256hq_u32(abcd, efgh, t);
                    efgh = vsha256h2q_u32(efgh, prev, t);
                    if i < 12 {
                        /* words 4i+16 .. 4i+19, in the place of the four just used */
                        w[i & 3] =
                            vsha256su1q_u32(vsha256su0q_u32(w[i & 3], w[(i + 1) & 3]), w[(i + 2) & 3], w[(i + 3) & 3]);
                    }
                }
                abcd = vaddq_u32(abcd, abcd0);
                efgh = vaddq_u32(efgh, efgh0);
            }
            vst1q_u32(state.as_mut_ptr(), abcd);
            vst1q_u32(state.as_mut_ptr().add(4), efgh);
        }
    }
}

/// The compression function over `blocks`, a whole number of 64-byte blocks.
fn compress(state: &mut [u32; 8], blocks: &[u8]) {
    #[cfg(all(target_arch = "aarch64", target_os = "none", target_feature = "neon"))]
    if hw::available() {
        // SAFETY: the instructions exist (checked just above).
        unsafe { hw::compress(state, blocks) };
        return;
    }
    compress_soft(state, blocks);
}

/// The portable compression function, for a check against [`sha256`] on a
/// core where that one runs the instructions.
pub fn sha256_portable(data: &[u8]) -> [u8; SHA256_DIGEST_LEN] {
    let mut c = Sha256::new();
    c.update_with(data, compress_soft);
    c.finalize_with(compress_soft)
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

    pub fn update(&mut self, data: &[u8]) {
        self.update_with(data, compress);
    }

    /// Whole blocks go to the compression function where they lie; only the
    /// bytes short of a block are copied (the first form of this function
    /// stored every byte into the context's block one at a time).
    fn update_with(&mut self, mut data: &[u8], compress: fn(&mut [u32; 8], &[u8])) {
        if self.datalen != 0 {
            let n = data.len().min(64 - self.datalen);
            self.data[self.datalen..self.datalen + n].copy_from_slice(&data[..n]);
            self.datalen += n;
            data = &data[n..];
            if self.datalen < 64 {
                return;
            }
            compress(&mut self.state, &self.data);
            self.bitlen = self.bitlen.wrapping_add(512);
            self.datalen = 0;
        }
        let whole = data.len() & !63;
        if whole != 0 {
            compress(&mut self.state, &data[..whole]);
            self.bitlen = self.bitlen.wrapping_add(whole as u64 * 8);
        }
        let rest = &data[whole..];
        self.data[..rest.len()].copy_from_slice(rest);
        self.datalen = rest.len();
    }

    pub fn finalize(self) -> [u8; SHA256_DIGEST_LEN] {
        self.finalize_with(compress)
    }

    fn finalize_with(mut self, compress: fn(&mut [u32; 8], &[u8])) -> [u8; SHA256_DIGEST_LEN] {
        let n = self.datalen;

        /* Pad. */
        self.data[n] = 0x80;
        if n < 56 {
            self.data[n + 1..56].fill(0x00);
        } else {
            self.data[n + 1..].fill(0x00);
            compress(&mut self.state, &self.data);
            self.data[..56].fill(0x00);
        }

        /* Append length in bits, big-endian. */
        self.bitlen = self.bitlen.wrapping_add(n as u64 * 8);
        self.data[56..].copy_from_slice(&self.bitlen.to_be_bytes());
        compress(&mut self.state, &self.data);

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
