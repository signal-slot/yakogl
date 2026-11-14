// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The G7 program binary format of `src/gl/program_binary_format.rs`
//! (plan #364, `docs/notes/es3-program-binary.md` (b) and (d)): the round
//! trip of the serializer, truncation at every byte, a flipped byte in every
//! header field, overlong length fields, the payload rules, the empty and the
//! maximal requests, and a deterministic fuzz loop that must never panic.
//!
//! Feature set: the file needs none, but the package must build, so any one
//! host GPU set works:
//! `cargo test --no-default-features --features host-test,gpu-powervr --test es3_program_binary_format`.
//! The file under test is not in the module tree yet, so it is included by path.

#[path = "../src/gl/program_binary_format.rs"]
#[allow(dead_code)]
mod program_binary_format;

use program_binary_format::*;

const ID: [u8; 32] = [0x5a; 32];
const TAG: [u8; 4] = *b"PVRB";

const VS: &[u8] = b"#version 300 es\nin highp vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";
const FS: &[u8] = b"#version 300 es\nlayout(location = 0) out mediump vec4 o;\nvoid main() { o = vec4(0.0); }\n";

fn full_snapshot<'a>(binds: &'a [Binding<'a>]) -> Snapshot<'a> {
    Snapshot {
        identity: ID,
        tag: TAG,
        vs_src: VS,
        fs_src: FS,
        binds,
        tf: TfRequest { names: b"v_a\0v_b\0", count: 2, mode: TF_SEPARATE },
    }
}

const BINDS: [Binding<'static>; 2] =
    [Binding { name: b"a_position", index: 3 }, Binding { name: b"a_color_with_a_31_byte_name_xyz", index: 15 }];

fn serialize(s: &Snapshot) -> Vec<u8> {
    let mut out = vec![0u8; serialized_len(s)];
    assert_eq!(write(s, &mut out), Ok(out.len()));
    out
}

/// Rewrite the header's payload size and hash after the payload was edited.
fn reseal(b: &mut [u8]) {
    let n = (b.len() - HEADER_BYTES) as u32;
    b[OFF_PAYLOAD_BYTES..OFF_PAYLOAD_BYTES + 4].copy_from_slice(&n.to_le_bytes());
    let h = program_binary_format::sha256::sha256(&b[HEADER_BYTES..]);
    b[OFF_PAYLOAD_HASH..OFF_PAYLOAD_HASH + 32].copy_from_slice(&h);
}

fn check_matches(p: &Parsed, s: &Snapshot) {
    assert_eq!(p.vs_src, s.vs_src);
    assert_eq!(p.fs_src, s.fs_src);
    assert_eq!(p.binds.len(), s.binds.len());
    assert_eq!(p.binds.iter().collect::<Vec<_>>(), s.binds);
    assert_eq!(p.tf, s.tf);
}

#[test]
fn round_trip() {
    let s = full_snapshot(&BINDS);
    let b = serialize(&s);
    assert_eq!(&b[..4], b"YKPB");
    assert_eq!(u16::from_le_bytes([b[4], b[5]]), 1);
    assert_eq!(u16::from_le_bytes([b[6], b[7]]), 80);
    assert_eq!(u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize, b.len() - 80);
    assert_eq!(&b[12..16], &TAG);
    assert_eq!(&b[16..48], &ID);
    assert_eq!(b[48..80], program_binary_format::sha256::sha256(&b[80..]));
    let p = parse(&b, &ID, &TAG).unwrap();
    check_matches(&p, &s);
    // The views point into the input.
    let range = b.as_ptr_range();
    assert!(range.contains(&p.vs_src.as_ptr()) && range.contains(&p.tf.names.as_ptr()));
    // Writing again gives the same bytes; a larger buffer is written at its front only.
    assert_eq!(serialize(&s), b);
    let mut big = vec![0xeeu8; b.len() + 7];
    assert_eq!(write(&s, &mut big), Ok(b.len()));
    assert_eq!(&big[..b.len()], &b[..]);
    assert!(big[b.len()..].iter().all(|&x| x == 0xee));
    assert_eq!(GL_PROGRAM_BINARY_FORMAT_YAKOGL, 0x6B47);
}

#[test]
fn write_refuses_a_small_buffer_and_leaves_it_untouched() {
    let s = full_snapshot(&BINDS);
    let mut out = vec![0x11u8; serialized_len(&s) - 1];
    assert_eq!(write(&s, &mut out), Err(ERR_BUFFER));
    assert!(out.iter().all(|&x| x == 0x11));
    assert_eq!(write(&s, &mut []), Err(ERR_BUFFER));
}

#[test]
fn write_refuses_what_parse_would_refuse() {
    let base = full_snapshot(&BINDS);
    let check = |s: Snapshot, want: &str| {
        let mut out = vec![0x11u8; serialized_len(&s)];
        assert_eq!(write(&s, &mut out), Err(want));
        assert!(out.iter().all(|&x| x == 0x11));
    };
    check(Snapshot { vs_src: b"a\0b", ..base }, ERR_SOURCE_NUL);
    check(Snapshot { fs_src: b"\0", ..base }, ERR_SOURCE_NUL);
    let many = [Binding { name: b"a", index: 0 }; MAX_ATTRIB_BINDS + 1];
    check(Snapshot { binds: &many, ..base }, ERR_BIND_COUNT);
    check(Snapshot { binds: &[Binding { name: b"", index: 0 }], ..base }, ERR_BIND_NAME);
    check(Snapshot { binds: &[Binding { name: &[b'x'; 32], index: 0 }], ..base }, ERR_BIND_NAME);
    check(Snapshot { binds: &[Binding { name: b"a\0", index: 0 }], ..base }, ERR_BIND_NAME);
    check(Snapshot { binds: &[Binding { name: b"a", index: 16 }], ..base }, ERR_BIND_INDEX);
    check(Snapshot { tf: TfRequest { mode: 1, ..base.tf }, ..base }, ERR_TF_MODE);
    check(Snapshot { tf: TfRequest { count: 3, ..base.tf }, ..base }, ERR_TF_NAMES);
    check(Snapshot { tf: TfRequest { names: b"v_a\0v_b", count: 1, ..base.tf }, ..base }, ERR_TF_NAMES);
    check(Snapshot { tf: TfRequest { names: b"", count: 1, ..base.tf }, ..base }, ERR_TF_NAMES);
    check(Snapshot { tf: TfRequest { names: b"\0", count: 0, ..base.tf }, ..base }, ERR_TF_NAMES);
}

#[test]
fn zero_bindings_and_no_tf() {
    let s = Snapshot {
        identity: ID,
        tag: TAG,
        vs_src: VS,
        fs_src: FS,
        binds: &[],
        tf: TfRequest { names: b"", count: 0, mode: 0 },
    };
    let b = serialize(&s);
    assert_eq!(b.len(), 80 + 4 + VS.len() + 4 + FS.len() + 1 + 12);
    let p = parse(&b, &ID, &TAG).unwrap();
    assert!(p.binds.is_empty());
    assert_eq!(p.binds.iter().count(), 0);
    check_matches(&p, &s);
    // A request whose count is 0 keeps the mode it was given.
    for mode in [TF_INTERLEAVED, TF_SEPARATE] {
        let s = Snapshot { tf: TfRequest { mode, ..s.tf }, ..s };
        assert_eq!(parse(&serialize(&s), &ID, &TAG).unwrap().tf.mode, mode);
    }
    // Empty sources are not the format's business: the relink refuses them.
    let s = Snapshot { vs_src: b"", fs_src: b"", ..s };
    check_matches(&parse(&serialize(&s), &ID, &TAG).unwrap(), &s);
}

#[test]
fn maximal_bindings_and_tf_count() {
    // MAX_ATTRIB_BINDS bindings with 31-byte names and every index.
    let names: Vec<[u8; MAX_BIND_NAME]> =
        (0..MAX_ATTRIB_BINDS).map(|i| [b'a' + (i % 26) as u8; MAX_BIND_NAME]).collect();
    let binds: Vec<Binding> =
        names.iter().enumerate().map(|(i, n)| Binding { name: n, index: (i % MAX_VERTEX_ATTRIBS) as u8 }).collect();
    // The most varyings a names blob of its size can hold: every name empty
    // (glTransformFeedbackVaryings stores a null name as a lone NUL).
    let tf_names = vec![0u8; 70_000];
    let s = Snapshot {
        binds: &binds,
        tf: TfRequest { names: &tf_names, count: tf_names.len() as u32, mode: TF_INTERLEAVED },
        ..full_snapshot(&[])
    };
    let b = serialize(&s);
    check_matches(&parse(&b, &ID, &TAG).unwrap(), &s);
    // One more than the NULs, or one fewer, is refused.
    let at = b.len() - tf_names.len() - 8;
    for count in [tf_names.len() as u32 + 1, tf_names.len() as u32 - 1, u32::MAX] {
        let mut c = b.clone();
        c[at..at + 4].copy_from_slice(&count.to_le_bytes());
        reseal(&mut c);
        assert_eq!(parse(&c, &ID, &TAG), Err(ERR_TF_NAMES), "count {count}");
    }
}

#[test]
fn truncation_at_every_byte_offset_is_refused() {
    let b = serialize(&full_snapshot(&BINDS));
    for k in 0..b.len() {
        let want = if k < HEADER_BYTES { ERR_SHORT } else { ERR_PAYLOAD_BYTES };
        assert_eq!(parse(&b[..k], &ID, &TAG), Err(want), "length {k}");
    }
    // Truncated with the header resealed: only the payload parser sees it.
    for k in HEADER_BYTES..b.len() {
        let mut c = b[..k].to_vec();
        reseal(&mut c);
        assert!(parse(&c, &ID, &TAG).is_err(), "resealed length {k}");
    }
}

#[test]
fn trailing_bytes_are_refused() {
    let b = serialize(&full_snapshot(&BINDS));
    let mut c = b.clone();
    c.push(0);
    assert_eq!(parse(&c, &ID, &TAG), Err(ERR_PAYLOAD_BYTES));
    for extra in [&[0u8][..], b"x", &[0xff; 9]] {
        let mut c = b.clone();
        c.extend_from_slice(extra);
        reseal(&mut c);
        assert_eq!(parse(&c, &ID, &TAG), Err(ERR_TRAILING));
    }
}

#[test]
fn a_flipped_byte_in_every_header_field_is_refused() {
    let b = serialize(&full_snapshot(&BINDS));
    let field = |at: usize| match at {
        0..4 => ERR_MAGIC,
        4..6 => ERR_VERSION,
        6..8 => ERR_HEADER_BYTES,
        8..12 => ERR_PAYLOAD_BYTES,
        12..16 => ERR_TAG,
        16..48 => ERR_IDENTITY,
        _ => ERR_PAYLOAD_HASH,
    };
    for at in 0..HEADER_BYTES {
        for mask in [0x01u8, 0x80, 0xff] {
            let mut c = b.clone();
            c[at] ^= mask;
            assert_eq!(parse(&c, &ID, &TAG), Err(field(at)), "byte {at} ^ {mask:#x}");
        }
    }
    // A flipped payload byte: the hash catches it.
    for at in HEADER_BYTES..b.len() {
        let mut c = b.clone();
        c[at] ^= 0x01;
        assert_eq!(parse(&c, &ID, &TAG), Err(ERR_PAYLOAD_HASH), "payload byte {at}");
    }
    // The caller's identity and tag are what is compared.
    let mut other_id = ID;
    other_id[31] ^= 1;
    assert_eq!(parse(&b, &other_id, &TAG), Err(ERR_IDENTITY));
    assert_eq!(parse(&b, &ID, b"V3D4"), Err(ERR_TAG));
}

#[test]
fn overlong_length_fields_are_refused() {
    let s = full_snapshot(&BINDS);
    let b = serialize(&s);
    // Header payload size beyond the slice.
    for n in [b.len() as u32 - 79, u32::MAX] {
        let mut c = b.clone();
        c[OFF_PAYLOAD_BYTES..OFF_PAYLOAD_BYTES + 4].copy_from_slice(&n.to_le_bytes());
        assert_eq!(parse(&c, &ID, &TAG), Err(ERR_PAYLOAD_BYTES));
    }
    // Every u32 length of the payload, and the binding name lengths, made
    // larger than what is left; the header is resealed so the parser sees it.
    let vs_at = HEADER_BYTES;
    let fs_at = vs_at + 4 + VS.len();
    let nb_at = fs_at + 4 + FS.len();
    let name0_at = nb_at + 1;
    let tf_bytes_at = b.len() - s.tf.names.len() - 4;
    for (at, wide) in [(vs_at, true), (fs_at, true), (tf_bytes_at, true), (name0_at, false)] {
        let left = (b.len() - at) as u32;
        for n in [left, left + 1, 0x7fff_ffff, u32::MAX] {
            let mut c = b.clone();
            if wide {
                c[at..at + 4].copy_from_slice(&n.to_le_bytes());
            } else {
                c[at] = n.min(255) as u8;
            }
            reseal(&mut c);
            assert!(parse(&c, &ID, &TAG).is_err(), "field at {at} = {n}");
        }
    }
    // A binding count over its limit, and one larger than the bindings present.
    for (n, want) in [(MAX_ATTRIB_BINDS as u8 + 1, ERR_BIND_COUNT), (255, ERR_BIND_COUNT), (3, ERR_BIND_INDEX)] {
        let mut c = b.clone();
        c[nb_at] = n;
        reseal(&mut c);
        // With 3, the third "binding" reads the TF mode's bytes; whatever it
        // decodes to, it must be refused.
        let r = parse(&c, &ID, &TAG);
        assert!(r.is_err(), "nbinds {n}");
        if n != 3 {
            assert_eq!(r, Err(want));
        }
    }
}

#[test]
fn payload_field_rules() {
    let s = full_snapshot(&BINDS);
    let b = serialize(&s);
    let vs_at = HEADER_BYTES + 4;
    let nb_at = HEADER_BYTES + 4 + VS.len() + 4 + FS.len();
    let index0_at = nb_at + 1 + 1 + BINDS[0].name.len();
    let mode_at = b.len() - s.tf.names.len() - 12;
    let edit = |at: usize, v: u8| {
        let mut c = b.clone();
        c[at] = v;
        reseal(&mut c);
        c
    };
    let refused = |at: usize, v: u8| parse(&edit(at, v), &ID, &TAG).err();
    assert_eq!(refused(vs_at + 3, 0), Some(ERR_SOURCE_NUL));
    assert_eq!(refused(nb_at + 1, 0), Some(ERR_BIND_NAME));
    assert_eq!(refused(nb_at + 2, 0), Some(ERR_BIND_NAME));
    assert_eq!(refused(index0_at, MAX_VERTEX_ATTRIBS as u8), Some(ERR_BIND_INDEX));
    assert_eq!(refused(index0_at, 255), Some(ERR_BIND_INDEX));
    assert_eq!(refused(mode_at, 0x8e), Some(ERR_TF_MODE));
    assert_eq!(refused(mode_at + 3, 1), Some(ERR_TF_MODE));
    assert_eq!(refused(b.len() - 1, b'x'), Some(ERR_TF_NAMES));
    // The names blob ends in a NUL: "v_a\0v_b\0" -> "v_a\0\0v_b" keeps two NULs.
    let mut c = b.clone();
    let n = c.len();
    c[n - 4..].copy_from_slice(b"\0v_b");
    reseal(&mut c);
    assert_eq!(parse(&c, &ID, &TAG), Err(ERR_TF_NAMES));
    // Accepted edits decode to what was written.
    let c = edit(index0_at, 0);
    assert_eq!(parse(&c, &ID, &TAG).unwrap().binds.iter().next(), Some(Binding { name: b"a_position", index: 0 }));
    assert_eq!(parse(&edit(mode_at, 0x8c), &ID, &TAG).unwrap().tf.mode, TF_INTERLEAVED);
}

/// xorshift64*: deterministic, no crates.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn no_panic_on_any_input() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let seed = serialize(&full_snapshot(&BINDS));
    let mut accepted = 0;
    for round in 0..20_000 {
        let mut c = match round % 4 {
            // Random bytes behind a valid header.
            0 => {
                let mut c = seed[..HEADER_BYTES].to_vec();
                c.extend((0..rng.below(200)).map(|_| rng.next() as u8));
                c
            }
            // Pure random bytes.
            1 => (0..rng.below(160)).map(|_| rng.next() as u8).collect(),
            // The valid binary with a few bytes changed, and maybe cut or grown.
            _ => {
                let mut c = seed.clone();
                for _ in 0..1 + rng.below(4) {
                    let at = HEADER_BYTES + rng.below(c.len() - HEADER_BYTES);
                    c[at] = match rng.below(3) {
                        0 => rng.next() as u8,
                        1 => 0,
                        _ => 0xff,
                    };
                }
                match rng.below(4) {
                    0 => c.truncate(HEADER_BYTES + rng.below(c.len() - HEADER_BYTES)),
                    1 => c.push(rng.next() as u8),
                    _ => {}
                }
                c
            }
        };
        if round % 4 != 1 {
            reseal(&mut c);
        }
        if let Ok(p) = parse(&c, &ID, &TAG) {
            accepted += 1;
            // Whatever is accepted is valid: it writes back to the same bytes.
            let binds: Vec<Binding> = p.binds.iter().collect();
            assert_eq!(binds.len(), p.binds.len());
            let s = Snapshot { identity: ID, tag: TAG, vs_src: p.vs_src, fs_src: p.fs_src, binds: &binds, tf: p.tf };
            assert_eq!(serialize(&s), c, "round {round}");
        }
    }
    // Some edits (source text, binding names) stay valid; the loop saw both outcomes.
    assert!(accepted > 0);
}
