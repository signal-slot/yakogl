// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2021-2025 Collabora Ltd.
// SPDX-FileCopyrightText: 2025 Arm Ltd.
// Derived from Mesa 26.2.1 src/panfrost/compiler/bifrost/bi_opt_push_ubo.c,
// valhall/va_lower_constants.c, valhall/va_validate.c and the constant
// histogram of bifrost_compile.c:4480-4529 (MIT). The upstream part stays
// under its licence (LICENSES/MIT.txt); our changes are Apache-2.0. See
// NOTICE.
//! The push constants (FAU) and the immediates: which 32-bit word of the
//! FAU each sysval, blend constant and promoted constant takes, and what
//! every constant source becomes (a table entry, a FAU word or a move).
//!
//! The layout is one per shader and shared by its IDVS variants, as Mesa's
//! `pan_shader_info::fau` is (bifrost_compile.c:4351-4355: both variants'
//! contexts point at it; the varying variant, compiled second, finds the
//! position variant's constants with `pan_lookup_pushed_imm`).

use super::bir::{self, Dest, Idx, Ins, Op, Prog, Src, Swz, Sysval};
use super::MaliError;

/// What a pushed FAU word holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FauWord {
    /// A component of a sysval (the sysval UBO's word `base + component`).
    Sysval(Sysval, u8),
    /// Blend constant component (the sysval UBO's first slot, which a
    /// fragment shader always pushes first, `bi_pick_blend_constants`).
    BlendConstant(u8),
    /// A promoted immediate.
    Constant(u32),
    /// A word of the uniform block in 32-bit components: the front end's
    /// uniform dword.
    Uniform { dword: u16 },
    /// A word of the fp16 uniform block (a fragment shader's mediump
    /// uniform): the front end's dwords of its low and high half (`None`:
    /// the high half past the uniform, zero).
    UniformHalves { lo: u16, hi: Option<u16> },
    /// Component `c` of `gl_FbWposYTransform`, gl_FragCoord's y transform
    /// (`STATE_FB_WPOS_Y_TRANSFORM`, prog_statevars.c:736-751): `(1, 0, -1,
    /// height)` for a target stored bottom-up, `(-1, height, 1, 0)` for one
    /// stored top-down (a window surface); the draw's value.
    FbWposYTransform(u8),
}

/// The FAU words Mesa allows a shader (`pan_fau_layout`'s `max`, 128: four
/// pages of 32 64-bit slots, the design page's "push uniforms").
pub const FAU_MAX_WORDS: usize = 128;

/// The FAU words this back end encodes: page 0 (32 slots). A word past it
/// needs `va_select_fau_page`'s page choice, which is not transcribed.
pub const FAU_PAGE0_WORDS: usize = 64;

/// `pan_fau_layout` (panfrost/compiler/pan_compiler.h).
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub words: [FauWord; FAU_MAX_WORDS],
    pub count: usize,
    /// The words [`push_ubo`] pushed (the sysvals and the uniform block,
    /// after the blend constants), which [`reorder_push`] pairs.
    pushed: (usize, usize),
}

impl Layout {
    pub const EMPTY: Layout = Layout { words: [FauWord::Constant(0); FAU_MAX_WORDS], count: 0, pushed: (0, 0) };

    pub fn as_slice(&self) -> &[FauWord] {
        &self.words[..self.count]
    }

    /// `pan_fau_available`.
    fn available(&self) -> usize {
        FAU_MAX_WORDS - self.count
    }

    fn emit(&mut self, w: FauWord) -> Result<u16, MaliError> {
        if self.count >= FAU_MAX_WORDS {
            return Err(MaliError::Defect(super::Defect::FauOverflow));
        }
        self.words[self.count] = w;
        self.count += 1;
        Ok((self.count - 1) as u16)
    }

    /// `pan_lookup_pushed_ubo` (pan_compiler.c:184-197): the first word of
    /// the whole layout that holds what word `w` holds. A word the earlier
    /// IDVS variant pushed is read from there, though this variant pushed
    /// it again (measured: sel_v_shared_with_position's varying program
    /// reads the position program's words, its own copies stay unread).
    fn lookup_pushed(&self, w: u16) -> u16 {
        let word = self.words[w as usize];
        self.as_slice().iter().position(|x| *x == word).map_or(w, |i| i as u16)
    }

    /// `pan_lookup_pushed_imm`.
    fn lookup_imm(&self, v: u32) -> Option<u16> {
        self.as_slice().iter().position(|w| *w == FauWord::Constant(v)).map(|i| i as u16)
    }
}

/// `bi_opt_push_ubo` (bi_opt_push_ubo.c:149-210), then
/// `bi_opt_reorder_push` (:305-378). A fragment shader first pushes the
/// four blend constants (`bi_pick_blend_constants`, :94-114); then every
/// read is pushed from the last UBO down and by ascending word
/// (`bi_pick_ubo`, :120-147): the sysval UBO, one range per sysval of as
/// many words as its highest component read plus one (the `load_ubo`'s
/// channel count after NIR shrinks it), then the uniform block (UBO 0), one
/// range per load in the same way, its words named by `ubo`
/// (select.rs `UboLayout`). Where a range no longer fits the FAU, Mesa
/// stops pushing and loads the rest from the UBO (`bi_pick_ubo`'s return,
/// then the `LOAD` it keeps): refused ([`MaliError::UboFallback`]).
pub fn push_ubo(
    p: &mut Prog<'_>,
    fau: &mut Layout,
    fragment: bool,
    ubo: &super::select::UboLayout,
) -> Result<(), MaliError> {
    if fragment {
        if fau.count != 0 {
            return Err(MaliError::Defect(super::Defect::FauOverflow));
        }
        for c in 0..4 {
            fau.emit(FauWord::BlendConstant(c))?;
        }
    }
    let start = fau.count;
    /* range[] per sysval, in word order of the UBO */
    let sysvals = [Sysval::ViewportScale, Sysval::ViewportOffset];
    let mut range = [0u8; 2];
    for i in p.as_slice() {
        for s in i.srcs() {
            if let Idx::Sysval(sv, c) = s.idx {
                let k = sysval_index(sv);
                range[k] = range[k].max(c + 1);
            }
        }
    }
    let mut first = [0u16; 2];
    for (k, sv) in sysvals.iter().enumerate() {
        if range[k] == 0 {
            continue;
        }
        if fau.available() < range[k] as usize {
            return Err(MaliError::FauFull);
        }
        for c in 0..range[k] {
            let w = fau.emit(FauWord::Sysval(*sv, c))?;
            if c == 0 {
                first[k] = w;
            }
        }
    }
    /* the uniform block: per load base, its lanes up to the highest read;
     * in a fragment shader, the lanes read alone: a lane between two read
     * ones is no load's (measured: femtovg's `frag[10].x` and `frag[10].z`
     * without `frag[10].y`, two words, u8.w0 and u8.w1, in
     * fvg_fill_image_nodiscard_noscissor.expect.json) */
    const BASES: usize = 512;
    let mut ubo_read = [0u8; BASES];
    for i in p.as_slice() {
        for s in i.srcs() {
            if let Idx::Ubo { base, comp } = s.idx {
                let r = ubo_read.get_mut(base as usize).ok_or(MaliError::UboFallback)?;
                *r |= 1u8.checked_shl(u32::from(comp)).ok_or(MaliError::UboFallback)?;
            }
        }
    }
    /* the pushed word of each base's lane */
    let mut ubo_at = [[u16::MAX; 8]; BASES];
    for (base, &read) in ubo_read.iter().enumerate() {
        if read == 0 {
            continue;
        }
        let range = 8 - read.leading_zeros() as u16;
        let lanes = (0..range).filter(|&c| !fragment || read & (1 << c) != 0);
        if fau.available() < lanes.clone().count() {
            break;
        }
        for c in lanes {
            let w = ubo.fau(base as u16 + c).ok_or(MaliError::Defect(super::Defect::FauOverflow))?;
            ubo_at[base][c as usize] = fau.emit(w)?;
        }
    }
    let end = fau.count;
    for i in 0..p.n {
        for s in 0..p.ins[i].ns as usize {
            let pushed = match p.ins[i].src[s].idx {
                Idx::Sysval(sv, c) => first[sysval_index(sv)] + c as u16,
                Idx::Ubo { base, comp } => {
                    let at = ubo_at[base as usize][comp as usize & 7];
                    if at == u16::MAX {
                        return Err(MaliError::UboFallback);
                    }
                    at
                }
                _ => continue,
            };
            p.ins[i].src[s].idx = Idx::Uniform(fau.lookup_pushed(pushed));
        }
    }
    fau.pushed = (start, end);
    Ok(())
}

/// A sysval's index in `push_ubo`'s `sysvals` (an exhaustive match: a new
/// sysval fails to compile here until it has its own range).
const fn sysval_index(sv: Sysval) -> usize {
    match sv {
        Sysval::ViewportScale => 0,
        Sysval::ViewportOffset => 1,
    }
}

/// `bi_opt_reorder_push` (bi_opt_push_ubo.c:306-378): pair the pushed
/// words that one instruction reads together, so that each such pair is
/// one 64-bit slot. Run after the modifier propagation and DCE
/// (bifrost_compile.c:4457-4473): an `fneg` of a pushed word folded into
/// its user is what makes `ext.x - rad` read two words in one instruction
/// (measured: fvg_fill_color_nodiscard.expect.json pairs `frag[8].x` with
/// `frag[12].w`).
pub fn reorder_push(p: &mut Prog<'_>, fau: &mut Layout) -> Result<(), MaliError> {
    let (start, end) = fau.pushed;
    const N: usize = FAU_MAX_WORDS;
    let mut adj = [0u128; N];
    for i in p.as_slice() {
        let mut nodes = [0usize; bir::MAX_SRCS];
        let mut n = 0;
        for s in i.srcs() {
            if let Idx::Uniform(w) = s.idx {
                let w = w as usize;
                if w >= start && w < end {
                    nodes[n] = w;
                    n += 1;
                }
            }
        }
        for a in 0..n {
            for b in 0..n {
                if a != b {
                    adj[nodes[a]] |= 1 << nodes[b];
                    adj[nodes[b]] |= 1 << nodes[a];
                }
            }
        }
    }
    let mut visited = 0u128;
    let mut ordering = [0usize; N];
    let mut unpaired = [0usize; N];
    let (mut pushed, mut nunpaired) = (0usize, 0usize);
    for i in start..end {
        if visited & (1 << i) != 0 {
            continue;
        }
        let mut component = [0usize; N];
        let mut size = 0usize;
        find_component(&adj, &mut visited, &mut component, &mut size, i);
        if size % 2 == 1 {
            size -= 1;
            unpaired[nunpaired] = component[size];
            nunpaired += 1;
        }
        ordering[pushed..pushed + size].copy_from_slice(&component[..size]);
        pushed += size;
    }
    ordering[pushed..pushed + nunpaired].copy_from_slice(&unpaired[..nunpaired]);
    pushed += nunpaired;
    if start + pushed != end {
        return Err(MaliError::Defect(super::Defect::FauOverflow));
    }
    let mut old_to_new = [0usize; N];
    for (i, o) in old_to_new.iter_mut().enumerate().take(start) {
        *o = i;
    }
    for (i, &o) in ordering[..pushed].iter().enumerate() {
        old_to_new[o] = start + i;
    }
    for i in 0..p.n {
        for s in 0..p.ins[i].ns as usize {
            if let Idx::Uniform(w) = p.ins[i].src[s].idx {
                p.ins[i].src[s].idx = Idx::Uniform(old_to_new[w as usize] as u16);
            }
        }
    }
    let old = *fau;
    for i in 0..pushed {
        fau.words[start + i] = old.words[ordering[i]];
    }
    Ok(())
}

/// `bi_find_component`: depth first, neighbours in ascending order.
fn find_component(adj: &[u128], visited: &mut u128, component: &mut [usize], size: &mut usize, node: usize) {
    *visited |= 1 << node;
    component[*size] = node;
    *size += 1;
    for nb in 0..adj.len() {
        if adj[node] & (1 << nb) != 0 && *visited & (1 << nb) == 0 {
            find_component(adj, visited, component, size, nb);
        }
    }
}

/// The table of immediates (`valhall_immediates[]`, valhall/ISA.xml:16-47).
pub const IMMEDIATES: [u32; 32] = [
    0x0000_0000,
    0xffff_ffff,
    0x7fff_ffff,
    0xfafc_fdfe,
    0x0100_0000,
    0x8000_2000,
    0x7060_5030,
    0xc0b0_a090,
    0x0302_0100,
    0x0706_0504,
    0x0b0a_0908,
    0x0f0e_0d0c,
    0x1312_1110,
    0x1716_1514,
    0x1b1a_1918,
    0x1f1e_1d1c,
    0x3f80_0000,
    0x3dcc_cccd,
    0x3ea2_f983,
    0x3f31_7218,
    0x4049_0fdb,
    0x0000_0000,
    0x477f_ff00,
    0x5c00_5bf8,
    0x2e66_0000,
    0x3400_0000,
    0x3800_0000,
    0x3c00_0000,
    0x4000_0000,
    0x4400_0000,
    0x4800_0000,
    0x4248_0000,
];

/// A table entry, possibly one half of it, possibly negated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Lut {
    i: u8,
    swz: Swz,
    neg: bool,
}

/// `va_lut_index_32`.
fn lut32(v: u32) -> Option<Lut> {
    IMMEDIATES.iter().position(|&x| x == v).map(|i| Lut { i: i as u8, swz: Swz::H01, neg: false })
}

/// `va_lut_index_16`: the first half of the table equal to `v`.
fn lut16(v: u16) -> Option<Lut> {
    for (i, &x) in IMMEDIATES.iter().enumerate() {
        if x as u16 == v {
            return Some(Lut { i: i as u8, swz: Swz::H00, neg: false });
        }
        if (x >> 16) as u16 == v {
            return Some(Lut { i: i as u8, swz: Swz::H11, neg: false });
        }
    }
    None
}

/// `va_lut_index_8`: the first byte of the table equal to `v`, read as a
/// byte lane of its entry (`bi_byte(va_lut(i >> 2), i & 3)`; the bytes in
/// memory order, each entry's low byte first). Measured: 1 is
/// `0x1000000.b3`, 2 is `0x3020100.b2` (fvg_texture_copy_unclipped.expect.json).
fn lut8(v: u8) -> Option<Lut> {
    for (i, &x) in IMMEDIATES.iter().enumerate() {
        if let Some(b) = x.to_le_bytes().iter().position(|&y| y == v) {
            let swz = [Swz::B0, Swz::B1, Swz::B2, Swz::B3][b];
            return Some(Lut { i: i as u8, swz, neg: false });
        }
    }
    None
}

fn neg(l: Option<Lut>) -> Option<Lut> {
    l.map(|l| Lut { neg: true, ..l })
}

/// An fp16 to fp32 conversion of the half `h` (`_mesa_half_to_float`), for
/// the values `va_resolve_swizzles` widens. Exact: every fp16 value is an
/// fp32 value.
pub(super) fn half_to_float(h: u16) -> u32 {
    let sign = ((h & 0x8000) as u32) << 16;
    let exp = ((h >> 10) & 0x1f) as u32;
    let man = (h & 0x3ff) as u32;
    match exp {
        0 if man == 0 => sign,
        0 => {
            /* subnormal: normalise */
            let mut e = 127 - 15 + 1;
            let mut m = man;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | (e << 23) | ((m & 0x3ff) << 13)
        }
        0x1f => sign | 0x7f80_0000 | (man << 13),
        _ => sign | ((exp + 127 - 15) << 23) | (man << 13),
    }
}

/// A constant's lookup in the table: an entry (whole, a half or a byte,
/// possibly negated), or none.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lookup {
    Found(Lut),
    None,
}

/// `va_lookup_constant` (va_lower_constants.c:112-193).
fn lookup(value: u32, info: bir::SrcInfo, signed: bool) -> Lookup {
    if let Some(l) = lut32(value) {
        return Lookup::Found(l);
    }
    if info.absneg && info.bits == 32 {
        if let Some(l) = neg(lut32(value ^ 0x8000_0000)) {
            return Lookup::Found(l);
        }
    }
    if info.absneg && info.bits == 16 {
        if let Some(l) = neg(lut32(value ^ 0x8000_8000)) {
            return Lookup::Found(l);
        }
    }
    let replicated = (value & 0xffff) == (value >> 16);
    if info.swizzle && info.bits == 16 && replicated {
        if let Some(l) = lut16(value as u16) {
            return Lookup::Found(l);
        }
        if info.absneg {
            if let Some(l) = neg(lut16((value as u16) ^ 0x8000)) {
                return Lookup::Found(l);
            }
        }
    }
    let can_widen = info.widen && info.bits == 32;
    let ext8 = if signed { value <= 0x7f || (value >> 7) == 0x1ff_ffff } else { value <= 0xff };
    if (can_widen || info.lanes || info.lane) && ext8 {
        if let Some(l) = lut8(value as u8) {
            return Lookup::Found(l);
        }
    }
    let ext16 = if signed { value <= 0x7fff || (value >> 15) == 0x1_ffff } else { value <= 0xffff };
    if can_widen && ext16 {
        if let Some(l) = lut16(value as u16) {
            return Lookup::Found(l);
        }
    }
    if info.swizzle && info.bits == 32 {
        let demote = |v: u32| super::select::f16_exact(v).and_then(lut16);
        if let Some(l) = demote(value) {
            return Lookup::Found(l);
        }
        if info.absneg {
            if let Some(l) = neg(demote(value ^ 0x8000_0000)) {
                return Lookup::Found(l);
            }
        }
    }
    Lookup::None
}

/// `va_resolve_swizzles` (va_lower_constants.c:214-251) for the swizzles
/// this back end emits.
fn resolve(s: &Src, info: bir::SrcInfo, value: u32) -> Result<u32, MaliError> {
    Ok(match info.bits {
        32 => {
            let v = match s.swz {
                Swz::H00 => value & 0xffff,
                Swz::H11 => value >> 16,
                Swz::H01 => value,
                /* a 32-bit source has no swapped halves; a byte lane is
                 * what this pass gives, never what it is handed */
                Swz::H10 | Swz::B0 | Swz::B1 | Swz::B2 | Swz::B3 => {
                    return Err(MaliError::Defect(super::Defect::Operand))
                }
            };
            if info.swizzle && s.swz != Swz::H01 {
                half_to_float(v as u16)
            } else {
                v
            }
        }
        _ => match s.swz {
            /* bi_apply_swizzle */
            Swz::H01 => value,
            Swz::H00 => (value & 0xffff) | (value << 16),
            Swz::H11 => (value >> 16) | (value & 0xffff_0000),
            Swz::H10 => value.rotate_left(16),
            Swz::B0 | Swz::B1 | Swz::B2 | Swz::B3 => return Err(MaliError::Defect(super::Defect::Operand)),
        },
    })
}

/// The distinct immediates outside the table one program's histogram holds.
pub const MAX_CONSTANTS: usize = 64;

/// Mesa's `hash_table_u64` on a 64-bit host (util/hash_table.c, 26.2.1),
/// as the constant histogram uses it: the key a pointer-sized value hashed
/// by `_mesa_hash_pointer`, the table one of `hash_sizes` from 19 slots,
/// open addressing with the double hash `1 + hash % rehash`, a rehash to
/// the next size checked at every insert, an existing key's entry replaced
/// in place, and no deletion. Only its slot order is read: the order
/// `hash_table_u64_foreach` walks.
struct U64Table {
    /// (present, hash, key, data) per slot.
    slots: [(bool, u32, u32, u32); TABLE_SLOTS],
    size_index: usize,
    entries: usize,
}

/// `hash_sizes` (util/hash_table.c:91-...): (max_entries, size, rehash),
/// from the first enabled row; the histogram holds at most
/// [`MAX_CONSTANTS`] keys, which the third size holds and a fourth would
/// grow past.
const HASH_SIZES: [(usize, usize, u32); 4] = [(16, 19, 17), (32, 43, 41), (64, 73, 71), (128, 151, 149)];
const TABLE_SLOTS: usize = 151;

/// `_mesa_hash_pointer` (util/hash_table.c:735-739).
fn hash_pointer(key: u32) -> u32 {
    let n = key as u64;
    ((n >> 2) ^ (n >> 6) ^ (n >> 10) ^ (n >> 14)) as u32
}

impl U64Table {
    const fn new() -> U64Table {
        U64Table { slots: [(false, 0, 0, 0); TABLE_SLOTS], size_index: 0, entries: 0 }
    }

    fn size(&self) -> usize {
        HASH_SIZES[self.size_index].1
    }

    /// `_mesa_hash_table_search`.
    fn search(&self, key: u32) -> u32 {
        let hash = hash_pointer(key);
        let (size, rehash) = (self.size(), HASH_SIZES[self.size_index].2);
        let start = hash as usize % size;
        let step = 1 + (hash % rehash) as usize;
        let mut at = start;
        loop {
            let e = self.slots[at];
            if !e.0 {
                return 0;
            }
            if e.1 == hash && e.2 == key {
                return e.3;
            }
            at += step;
            if at >= size {
                at -= size;
            }
            if at == start {
                return 0;
            }
        }
    }

    /// `hash_table_insert_rehash` into the current size.
    fn place(&mut self, hash: u32, key: u32, data: u32) {
        let (size, rehash) = (self.size(), HASH_SIZES[self.size_index].2);
        let step = 1 + (hash % rehash) as usize;
        let mut at = hash as usize % size;
        while self.slots[at].0 {
            at += step;
            if at >= size {
                at -= size;
            }
        }
        self.slots[at] = (true, hash, key, data);
    }

    /// `_mesa_hash_table_insert` (`hash_table_get_entry`, :475-533): the
    /// rehash first when the table is full, then the key's own slot or the
    /// first free one.
    fn insert(&mut self, key: u32, data: u32) -> Result<(), MaliError> {
        if self.entries >= HASH_SIZES[self.size_index].0 {
            if self.size_index + 1 >= HASH_SIZES.len() {
                return Err(MaliError::TooManyConstants);
            }
            /* `_mesa_hash_table_rehash`: the old slots in order */
            let old = self.slots;
            let old_size = self.size();
            self.slots = [(false, 0, 0, 0); TABLE_SLOTS];
            self.size_index += 1;
            for e in old[..old_size].iter().filter(|e| e.0) {
                self.place(e.1, e.2, e.3);
            }
        }
        let hash = hash_pointer(key);
        let (size, rehash) = (self.size(), HASH_SIZES[self.size_index].2);
        let start = hash as usize % size;
        let step = 1 + (hash % rehash) as usize;
        let mut at = start;
        loop {
            let e = &mut self.slots[at];
            if !e.0 {
                *e = (true, hash, key, data);
                self.entries += 1;
                return Ok(());
            }
            if e.1 == hash && e.2 == key {
                e.3 = data;
                return Ok(());
            }
            at += step;
            if at >= size {
                at -= size;
            }
            if at == start {
                return Err(MaliError::Defect(super::Defect::FauOverflow));
            }
        }
    }

    /// The counts in `hash_table_u64_foreach`'s order (slot order).
    fn counts(&self, out: &mut [u32; MAX_CONSTANTS]) -> usize {
        let mut n = 0;
        for e in self.slots[..self.size()].iter().filter(|e| e.0) {
            if n < out.len() {
                out[n] = e.3;
                n += 1;
            }
        }
        n
    }
}

/// The constant sources and how often each value that needs FAU or a move
/// occurs (`va_count_constants`, va_lower_constants.c:293-319): the values
/// in first-use order with their counts, and Mesa's hash table of them,
/// filled one occurrence at a time as `va_count_constants` does (a search,
/// then an insert of the count plus one).
fn histogram(
    p: &Prog<'_>,
    values: &mut [(u32, u32); MAX_CONSTANTS],
    n: &mut usize,
    table: &mut U64Table,
) -> Result<(), MaliError> {
    /* "Phis become single moves so shouldn't be affected" (:4486-4491) */
    for i in p.as_slice().iter().filter(|i| i.op != Op::Phi) {
        for (s, src) in i.srcs().iter().enumerate() {
            let Idx::Const(c) = src.idx else { continue };
            if s < bir::nr_staging_srcs(i.op) {
                continue;
            }
            let info = bir::src_info(i.op, s);
            let v = resolve(src, info, c)?;
            if lookup(v, info, bir::is_signed(i.op)) != Lookup::None {
                continue;
            }
            let count = table.search(v);
            table.insert(v, count + 1)?;
            match values[..*n].iter_mut().find(|(x, _)| *x == v) {
                Some(e) => e.1 += 1,
                None => {
                    if *n >= values.len() {
                        return Err(MaliError::TooManyConstants);
                    }
                    values[*n] = (v, 1);
                    *n += 1;
                }
            }
        }
    }
    Ok(())
}

/// The constants: `va_count_constants`, the promotion threshold, then
/// `va_lower_constants` per instruction in order (va_lower_constants.c:
/// 253-291), and the FAU check `va_repair_fau` would act on.
///
/// The threshold (bifrost_compile.c:4480-4510) is `sorted[max_amount - 1]`
/// of the histogram's counts, which `hash_table_u64_foreach` lists in the
/// table's slot order, after `util_qsort_r` with `compare_u32`
/// (:4268-4273). That comparator casts the element POINTERS it is handed
/// to `uint32_t` and subtracts them: it orders by address, which is the
/// order the array is already in, so glibc's qsort_r (2.44, the host
/// pan_shc runs on) moves nothing (measured over 3200 arrays of 1 to 64
/// counts, tools/mali/qsort_pointer_order.c). The threshold is the count of
/// the `max_amount`-th entry in slot order ([`U64Table`]); a constant whose
/// count reaches it is promoted while the FAU has room.
///
/// Returns the histogram's distinct values (its use of [`MAX_CONSTANTS`]).
pub fn lower_constants(p: &mut Prog<'_>, fau: &mut Layout, promote: bool) -> Result<usize, MaliError> {
    let mut hist = [(0u32, 0u32); MAX_CONSTANTS];
    let mut n = 0usize;
    let mut table = U64Table::new();
    histogram(p, &mut hist, &mut n, &mut table)?;
    let mut sorted = [0u32; MAX_CONSTANTS];
    let listed = table.counts(&mut sorted);
    if listed != n {
        return Err(MaliError::Defect(super::Defect::FauOverflow));
    }
    /* `inputs.fau.promote_immediates` is set by the gallium driver
     * (pan_shader.c:207), so `max_amount` is not 0 while the FAU has room;
     * Mesa's internal shaders are compiled without it (the preload) */
    let max_amount = if promote { n.min(fau.available()) } else { 0 };
    let min_count = if max_amount > 0 { sorted[max_amount - 1] } else { u32::MAX };
    let mut i = 0;
    while i < p.n {
        /* a phi keeps its constant: the out-of-SSA pass makes it a move
         * (bifrost_compile.c:4512-4516, bi_ra.c:1033-1040) */
        if p.ins[i].op == Op::Phi {
            i += 1;
            continue;
        }
        let mut moved = 0;
        for s in 0..p.ins[i + moved].ns as usize {
            let at = i + moved;
            let src = p.ins[at].src[s];
            let Idx::Const(c) = src.idx else { continue };
            let op = p.ins[at].op;
            let staging = s < bir::nr_staging_srcs(op);
            let info = bir::src_info(op, s);
            let value = resolve(&src, info, c)?;
            let count = hist[..n].iter().find(|(v, _)| *v == value).map_or(0, |e| e.1);
            let mut cons = None;
            if !staging {
                if let Lookup::Found(l) = lookup(value, info, bir::is_signed(op)) {
                    cons = Some(Src { idx: Idx::Lut(l.i), swz: l.swz, neg: l.neg, abs: false, discard: false });
                }
            }
            if cons.is_none() && !staging && count >= min_count {
                let w = match fau.lookup_imm(value) {
                    Some(w) => Some(w),
                    None if fau.available() > 0 => Some(fau.emit(FauWord::Constant(value))?),
                    None => None,
                };
                cons = w.map(|w| Src::of(Idx::Uniform(w)));
            }
            let mut cons = match cons {
                Some(c) => c,
                None => {
                    /* va_mov_imm: IADD_IMM.i32 of the table's zero */
                    let t = p.temp()?;
                    p.insert(at, Ins::new(Op::IaddImmI32(value), &[Dest::ssa(t)], &[Src::of(Idx::Lut(0))]))?;
                    moved += 1;
                    Src::ssa(t)
                }
            };
            /* the walk folds the fabs of a constant: none reaches here */
            if src.abs {
                return Err(MaliError::Defect(super::Defect::Operand));
            }
            cons.neg ^= src.neg;
            /* a lane select gets the bottom byte or half (:276-288, the
             * asserted sizes 8 and 16; LSHIFT_AND's shift is the 8-bit one) */
            if (info.lane || info.lanes || info.halfswizzle) && cons.swz == Swz::H01 {
                cons.swz = match info.bits {
                    8 => Swz::B0,
                    16 => Swz::H00,
                    _ => return Err(MaliError::Defect(super::Defect::Operand)),
                };
            }
            p.ins[i + moved].src[s] = cons;
        }
        let at = i + moved;
        moved += repair_fau(p, at)?;
        i += moved + 1;
    }
    super::opt::cse(p);
    super::opt::dce(p);
    Ok(n)
}

/// `struct fau_state` (va_validate.c:25-28): the one uniform slot and the
/// two 32-bit words an instruction outside the message unit may read.
#[derive(Clone, Copy, Default)]
struct FauState {
    slot: Option<u16>,
    buffer: [Option<Idx>; 2],
}

/// `bi_is_word_equiv`: the same FAU word (a special value's two halves are
/// two words).
fn word_equiv(a: Idx, b: Idx) -> bool {
    match (a, b) {
        (Idx::Uniform(x), Idx::Uniform(y)) => x == y,
        (Idx::Lut(x), Idx::Lut(y)) => x == y,
        (Idx::Special(x, xh), Idx::Special(y, yh)) => x == y && xh == yh,
        _ => false,
    }
}

impl FauState {
    /// `fau_state_buffer` (va_validate.c:30-43).
    fn buffer(&mut self, idx: Idx) -> bool {
        for b in self.buffer.iter_mut() {
            match *b {
                Some(x) if word_equiv(x, idx) => return true,
                None => {
                    *b = Some(idx);
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    fn has_special(&self) -> bool {
        self.buffer.iter().flatten().any(|b| matches!(b, Idx::Special(..)))
    }

    /// `valid_src` (va_validate.c:105-128), with `fau_state_uniform`
    /// (:62-83) and `fau_state_special` (:85-103; `bi_is_equiv` ignores the
    /// half). Page 0 only: a uniform past it is refused before.
    fn valid(&mut self, op: Op, idx: Idx) -> bool {
        let msg = bir::props(op).msg;
        let two = msg != bir::Msg::None && msg != bir::Msg::Atest && msg != bir::Msg::Blend;
        match idx {
            Idx::Lut(_) => two || self.buffer(idx),
            Idx::Uniform(w) => {
                let mut ok = self.buffer(idx);
                let sl = w / 2;
                if *self.slot.get_or_insert(sl) != sl {
                    ok = false;
                }
                if !two && self.has_special() {
                    ok = false;
                }
                ok
            }
            Idx::Special(sp, _) => {
                let mut ok = self.buffer(idx);
                if self.buffer.iter().flatten().any(|b| matches!(b, Idx::Special(o, _) if *o != sp)) {
                    ok = false;
                }
                ok && (self.slot.is_none() || two)
            }
            _ => true,
        }
    }
}

/// `va_repair_fau` (va_validate.c:156-176) on the instruction at `at`: each
/// source the FAU state cannot take is read through a `MOV.i32` inserted
/// before it, and the state rolled back. Returns the moves inserted. A
/// uniform word past FAU page 0 is refused first (`va_select_fau_page` is
/// not transcribed).
fn repair_fau(p: &mut Prog<'_>, at: usize) -> Result<usize, MaliError> {
    let i = p.ins[at];
    if i.srcs().iter().any(|s| matches!(s.idx, Idx::Uniform(w) if w as usize >= FAU_PAGE0_WORDS)) {
        return Err(MaliError::FauPage);
    }
    let mut st = FauState::default();
    let mut inserted = 0;
    for s in 0..i.ns as usize {
        let saved = st;
        let idx = i.src[s].idx;
        if st.valid(i.op, idx) {
            continue;
        }
        st = saved;
        let t = p.temp()?;
        p.insert(at + inserted, Ins::new(Op::MovI32, &[Dest::ssa(t)], &[Src::of(idx)]))?;
        inserted += 1;
        p.ins[at + inserted].src[s].idx = Idx::Ssa(t, 0);
    }
    Ok(inserted)
}
