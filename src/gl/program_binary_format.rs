// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The program binary format of G7 (`docs/notes/es3-program-binary.md`,
//! sections (b) and (d)): the token, the 80-byte header, the serializer of
//! the link inputs and the bounds-safe parser that the load uses.
//!
//! Plan #364. The module is pure `no_std` code (no `alloc`, no `std`, no
//! `crate::` or `super::` imports) and is **not yet in the module tree**: it
//! is pulled into `tests/es3_program_binary_format.rs` with `#[path]`. The
//! g7-impl item adds it to `src/gl/mod.rs`; its `src/gl/program_binary.rs`
//! (the per-program store, `capture`, `forget`, `length`, `get`, `load`)
//! uses this file for the bytes and holds everything that touches GL state.
//!
//! The caller passes the 32-byte build identity and the 4-byte GPU tag in;
//! this file computes no build identity. It does hash the payload, with the
//! self-contained `src/shaderc/sha256.rs` included by path (the same code
//! as `crate::shaderc::sha256`).
//!
//! Layout, all little-endian:
//!
//! | Offset | Bytes | Field |
//! |---|---|---|
//! | 0 | 4 | magic `YKPB` |
//! | 4 | 2 | format version, 1 |
//! | 6 | 2 | header bytes, 80 |
//! | 8 | 4 | payload bytes |
//! | 12 | 4 | GPU tag |
//! | 16 | 32 | build identity |
//! | 48 | 32 | SHA-256 of the payload |
//! | 80 | n | payload |
//!
//! The payload, in order:
//!
//! ```text
//! u32 vs_bytes, vs source (no NUL)
//! u32 fs_bytes, fs source (no NUL)
//! u8  nbinds (<= MAX_ATTRIB_BINDS); per binding: u8 name_len (1..=31), name (no NUL), u8 index (< MAX_VERTEX_ATTRIBS)
//! u32 tf_mode (0, GL_INTERLEAVED_ATTRIBS or GL_SEPARATE_ATTRIBS)
//! u32 tf_count, u32 tf_bytes, tf names (exactly tf_count NULs, the last byte a NUL; both 0 when there is no request)
//! ```
//!
//! [`parse`] never panics and never reads outside its input: every length is
//! checked against what is left of the slice before it is used. A mismatch in
//! any field, a short field, a count over its limit or bytes after the last
//! field is an `Err` whose string is the info log text of the rejected load
//! (link status FALSE, no GL error). The hashes are not a security boundary;
//! the parser is bounds-safe on any bytes.

#[path = "../shaderc/sha256.rs"]
#[allow(dead_code)]
pub mod sha256;

/// `GL_PROGRAM_BINARY_FORMATS`' one value. 0x6B47 lies in the Khronos range
/// reserved for temporary use (0x6000-0x7FFF); a vendor block replaces it
/// before a release (design (b)).
pub const GL_PROGRAM_BINARY_FORMAT_YAKOGL: u32 = 0x6B47;

/// `GL_INTERLEAVED_ATTRIBS`.
pub const TF_INTERLEAVED: u32 = 0x8C8C;
/// `GL_SEPARATE_ATTRIBS`.
pub const TF_SEPARATE: u32 = 0x8C8D;

pub const MAGIC: [u8; 4] = *b"YKPB";
pub const FORMAT_VERSION: u16 = 1;
pub const HEADER_BYTES: usize = 80;

/// Offsets of the header fields.
pub const OFF_MAGIC: usize = 0;
pub const OFF_VERSION: usize = 4;
pub const OFF_HEADER_BYTES: usize = 6;
pub const OFF_PAYLOAD_BYTES: usize = 8;
pub const OFF_TAG: usize = 12;
pub const OFF_IDENTITY: usize = 16;
pub const OFF_PAYLOAD_HASH: usize = 48;

/// Mirrors `context::MAX_ATTRIB_BINDS` (`shaderc::SHC_MAX_DECLARED_ATTRIBS`).
pub const MAX_ATTRIB_BINDS: usize = 32;
/// Mirrors `context::MAX_VERTEX_ATTRIBS`: a binding index is below it.
pub const MAX_VERTEX_ATTRIBS: usize = 16;
/// `GlAttribBind::name` is 32 bytes with its NUL.
pub const MAX_BIND_NAME: usize = 31;

pub const ERR_SHORT: &str = "program binary rejected: shorter than its 80-byte header";
pub const ERR_MAGIC: &str = "program binary rejected: wrong magic (not a yakogl program binary)";
pub const ERR_VERSION: &str = "program binary rejected: unsupported format version";
pub const ERR_HEADER_BYTES: &str = "program binary rejected: wrong header size";
pub const ERR_TAG: &str = "program binary rejected: made for another GPU";
pub const ERR_IDENTITY: &str = "program binary rejected: made by another build of the driver";
pub const ERR_PAYLOAD_BYTES: &str = "program binary rejected: payload size does not match the binary length";
pub const ERR_PAYLOAD_HASH: &str = "program binary rejected: payload checksum mismatch";
pub const ERR_TRUNCATED: &str = "program binary rejected: payload truncated";
pub const ERR_SOURCE_NUL: &str = "program binary rejected: NUL inside a shader source";
pub const ERR_BIND_COUNT: &str = "program binary rejected: too many attribute bindings";
pub const ERR_BIND_NAME: &str = "program binary rejected: bad attribute binding name";
pub const ERR_BIND_INDEX: &str = "program binary rejected: attribute binding index out of range";
pub const ERR_TF_MODE: &str = "program binary rejected: bad transform feedback mode";
pub const ERR_TF_NAMES: &str = "program binary rejected: bad transform feedback varying names";
pub const ERR_TRAILING: &str = "program binary rejected: trailing bytes after the payload";
/// [`write`] only: the payload does not fit the u32 size field.
pub const ERR_TOO_LARGE: &str = "program binary: link inputs too large for the format";
/// [`write`] only: `out` is shorter than [`serialized_len`].
pub const ERR_BUFFER: &str = "program binary: output buffer too small";

/// One `glBindAttribLocation` binding: the name without its NUL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding<'a> {
    pub name: &'a [u8],
    pub index: u8,
}

/// The transform feedback request as `glTransformFeedbackVaryings` stored it
/// (`tf.rs`, `TfProg::pending*`): `count` NUL-terminated names back to back
/// in `names`, and the mode as given (0 before any call). No request is
/// `count` 0 and empty `names`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TfRequest<'a> {
    pub names: &'a [u8],
    pub count: u32,
    pub mode: u32,
}

/// Everything a binary records: the link inputs and the caller's identity.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot<'a> {
    /// The 32-byte build identity (design (b): SHA-256 of the identity string).
    pub identity: [u8; 32],
    /// `V3D4`, `PVRB`, `GC7K` or `MALI`.
    pub tag: [u8; 4],
    /// The vertex and fragment sources, without a NUL.
    pub vs_src: &'a [u8],
    pub fs_src: &'a [u8],
    pub binds: &'a [Binding<'a>],
    pub tf: TfRequest<'a>,
}

/// The bindings of a parsed binary, still in the input's encoding. Every
/// entry was validated by [`parse`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bindings<'a> {
    raw: &'a [u8],
    count: usize,
}

impl<'a> Bindings<'a> {
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn iter(&self) -> BindingIter<'a> {
        BindingIter { raw: self.raw, left: self.count }
    }
}

pub struct BindingIter<'a> {
    raw: &'a [u8],
    left: usize,
}

impl<'a> Iterator for BindingIter<'a> {
    type Item = Binding<'a>;

    fn next(&mut self) -> Option<Binding<'a>> {
        if self.left == 0 {
            return None;
        }
        let (&n, rest) = self.raw.split_first()?;
        let n = n as usize;
        if rest.len() < n + 1 {
            return None;
        }
        let b = Binding { name: &rest[..n], index: rest[n] };
        self.raw = &rest[n + 1..];
        self.left -= 1;
        Some(b)
    }
}

/// A validated binary: slices into the bytes given to [`parse`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parsed<'a> {
    pub vs_src: &'a [u8],
    pub fs_src: &'a [u8],
    pub binds: Bindings<'a>,
    pub tf: TfRequest<'a>,
}

fn valid_bind_name(name: &[u8]) -> bool {
    (1..=MAX_BIND_NAME).contains(&name.len()) && !name.contains(&0)
}

fn valid_tf_mode(mode: u32) -> bool {
    matches!(mode, 0 | TF_INTERLEAVED | TF_SEPARATE)
}

/// `count` NULs, the last byte one of them; nothing when `count` is 0.
fn valid_tf_names(names: &[u8], count: u32) -> bool {
    if count == 0 {
        return names.is_empty();
    }
    names.last() == Some(&0) && names.iter().filter(|&&b| b == 0).count() as u64 == count as u64
}

/// The payload's length in bytes, saturating (an overlong snapshot is then
/// refused by [`write`]).
fn payload_len(s: &Snapshot) -> usize {
    let binds = s.binds.iter().fold(0usize, |a, b| a.saturating_add(2).saturating_add(b.name.len()));
    [4, s.vs_src.len(), 4, s.fs_src.len(), 1, binds, 12, s.tf.names.len()]
        .iter()
        .fold(0usize, |a, &n| a.saturating_add(n))
}

/// The bytes [`write`] needs for `s`: the header and the payload.
pub fn serialized_len(s: &Snapshot) -> usize {
    HEADER_BYTES.saturating_add(payload_len(s))
}

struct Writer<'a> {
    out: &'a mut [u8],
    at: usize,
}

impl Writer<'_> {
    /// Callers sized `out` with [`serialized_len`] first.
    fn put(&mut self, bytes: &[u8]) {
        self.out[self.at..self.at + bytes.len()].copy_from_slice(bytes);
        self.at += bytes.len();
    }
}

/// Serialize `s` into the front of `out` and return the bytes written
/// ([`serialized_len`]). Refuses a snapshot [`parse`] would refuse, so a
/// written binary always parses back to the same inputs. `out` is untouched
/// on an error.
pub fn write(s: &Snapshot, out: &mut [u8]) -> Result<usize, &'static str> {
    if s.vs_src.contains(&0) || s.fs_src.contains(&0) {
        return Err(ERR_SOURCE_NUL);
    }
    if s.binds.len() > MAX_ATTRIB_BINDS {
        return Err(ERR_BIND_COUNT);
    }
    for b in s.binds {
        if !valid_bind_name(b.name) {
            return Err(ERR_BIND_NAME);
        }
        if b.index as usize >= MAX_VERTEX_ATTRIBS {
            return Err(ERR_BIND_INDEX);
        }
    }
    if !valid_tf_mode(s.tf.mode) {
        return Err(ERR_TF_MODE);
    }
    if !valid_tf_names(s.tf.names, s.tf.count) {
        return Err(ERR_TF_NAMES);
    }
    let payload = payload_len(s);
    let u32_fits = |n: usize| u32::try_from(n).is_ok();
    if !u32_fits(payload) || !u32_fits(HEADER_BYTES + payload) {
        return Err(ERR_TOO_LARGE);
    }
    let total = HEADER_BYTES + payload;
    if out.len() < total {
        return Err(ERR_BUFFER);
    }
    let mut w = Writer { out: &mut out[..total], at: HEADER_BYTES };
    w.put(&(s.vs_src.len() as u32).to_le_bytes());
    w.put(s.vs_src);
    w.put(&(s.fs_src.len() as u32).to_le_bytes());
    w.put(s.fs_src);
    w.put(&[s.binds.len() as u8]);
    for b in s.binds {
        w.put(&[b.name.len() as u8]);
        w.put(b.name);
        w.put(&[b.index]);
    }
    w.put(&s.tf.mode.to_le_bytes());
    w.put(&s.tf.count.to_le_bytes());
    w.put(&(s.tf.names.len() as u32).to_le_bytes());
    w.put(s.tf.names);
    debug_assert_eq!(w.at, total);
    let hash = sha256::sha256(&w.out[HEADER_BYTES..]);
    w.at = 0;
    w.put(&MAGIC);
    w.put(&FORMAT_VERSION.to_le_bytes());
    w.put(&(HEADER_BYTES as u16).to_le_bytes());
    w.put(&(payload as u32).to_le_bytes());
    w.put(&s.tag);
    w.put(&s.identity);
    w.put(&hash);
    Ok(total)
}

struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], &'static str> {
        if n > self.rest.len() {
            return Err(ERR_TRUNCATED);
        }
        let (head, tail) = self.rest.split_at(n);
        self.rest = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, &'static str> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, &'static str> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A u32 length, then that many bytes.
    fn blob(&mut self) -> Result<&'a [u8], &'static str> {
        let n = self.u32()?;
        self.take(usize::try_from(n).map_err(|_| ERR_TRUNCATED)?)
    }
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Validate `bytes` (a whole binary as given to `glProgramBinary`) against
/// this build and return views of its link inputs. The checks run in header
/// order (magic, version, header size, GPU tag, identity, payload size,
/// payload hash), then the payload field by field.
pub fn parse<'a>(
    bytes: &'a [u8],
    expected_identity: &[u8; 32],
    expected_tag: &[u8; 4],
) -> Result<Parsed<'a>, &'static str> {
    if bytes.len() < HEADER_BYTES {
        return Err(ERR_SHORT);
    }
    let (header, payload) = bytes.split_at(HEADER_BYTES);
    if header[OFF_MAGIC..OFF_MAGIC + 4] != MAGIC {
        return Err(ERR_MAGIC);
    }
    if le16(header, OFF_VERSION) != FORMAT_VERSION {
        return Err(ERR_VERSION);
    }
    if le16(header, OFF_HEADER_BYTES) as usize != HEADER_BYTES {
        return Err(ERR_HEADER_BYTES);
    }
    if header[OFF_TAG..OFF_TAG + 4] != expected_tag[..] {
        return Err(ERR_TAG);
    }
    if header[OFF_IDENTITY..OFF_IDENTITY + 32] != expected_identity[..] {
        return Err(ERR_IDENTITY);
    }
    if le32(header, OFF_PAYLOAD_BYTES) as u64 != payload.len() as u64 {
        return Err(ERR_PAYLOAD_BYTES);
    }
    if header[OFF_PAYLOAD_HASH..OFF_PAYLOAD_HASH + 32] != sha256::sha256(payload) {
        return Err(ERR_PAYLOAD_HASH);
    }
    parse_payload(payload)
}

/// The payload rules alone, without the header (for [`parse`]).
fn parse_payload(payload: &[u8]) -> Result<Parsed<'_>, &'static str> {
    let mut r = Reader { rest: payload };
    let vs_src = r.blob()?;
    let fs_src = r.blob()?;
    if vs_src.contains(&0) || fs_src.contains(&0) {
        return Err(ERR_SOURCE_NUL);
    }
    let nbinds = r.u8()? as usize;
    if nbinds > MAX_ATTRIB_BINDS {
        return Err(ERR_BIND_COUNT);
    }
    let binds_at = r.rest;
    for _ in 0..nbinds {
        let n = r.u8()? as usize;
        let name = r.take(n)?;
        if !valid_bind_name(name) {
            return Err(ERR_BIND_NAME);
        }
        if r.u8()? as usize >= MAX_VERTEX_ATTRIBS {
            return Err(ERR_BIND_INDEX);
        }
    }
    let binds = Bindings { raw: &binds_at[..binds_at.len() - r.rest.len()], count: nbinds };
    let mode = r.u32()?;
    if !valid_tf_mode(mode) {
        return Err(ERR_TF_MODE);
    }
    let count = r.u32()?;
    let names = r.blob()?;
    if !valid_tf_names(names, count) {
        return Err(ERR_TF_NAMES);
    }
    if !r.rest.is_empty() {
        return Err(ERR_TRAILING);
    }
    Ok(Parsed { vs_src, fs_src, binds, tf: TfRequest { names, count, mode } })
}
