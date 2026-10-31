// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/qpu/qpu_instr.h (the layout of
// struct v3d_qpu_instr and its enums) (MIT). The upstream part stays under
// its licence (LICENSES/MIT.txt); our changes are Apache-2.0. See NOTICE.
//! Mesa's QPU packer through FFI: `struct v3d_qpu_instr` mirrored in Rust,
//! the functions of `qpu_pack.c` / `qpu_instr.c` the wrapper calls, and the
//! conversion between the mirror and this compiler's model ([`super::isa`]).
//!
//! The C is Mesa 26.2.1's, unmodified (`third_party/mesa-26.2.1/src/broadcom/
//! qpu/`, MIT, Copyright (c) 2016 Broadcom), compiled by `build.rs` with the
//! adapter `qpu_mesa.c` (the V3D 4.2 and 7.1 devices, the layout pins). Nothing here
//! knows where a field sits in the 64-bit word or which opcode is which
//! operation: that is `v3d_qpu_instr_pack` / `v3d_qpu_instr_unpack`.
//!
//! The mirror is hand-written and pinned: `the_mirror_is_mesas_layout`
//! compares every size and offset with the ones `qpu_mesa.c` reads from the
//! real header, the byte of each `struct v3d_qpu_sig` bit-field with the one
//! C gives it, and every enum value the model uses with Mesa's own name
//! tables (`v3d_qpu_add_op_name` and the rest), so a model variant whose
//! discriminant is not the Mesa value it claims fails by name.
//!
//! The enums are held in the mirror as `u32`, never as Rust enums: a value
//! the C writes that the model has no variant for (a 7.x operation, bdu
//! 4..7) must be a refusal, not undefined behaviour.

use super::isa::*;

/// `struct v3d_qpu_sig`: seventeen `bool :1` bit-fields. Which bit of which
/// byte each one is is measured from C (`yakogl_v3d_qpu_sig_images`), not
/// assumed; [`SIG_BITS`] is the order, and the test pins the bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MSig {
    pub bytes: [u8; 3],
}

/// The fields of `struct v3d_qpu_sig` in declaration order; field `i` is bit
/// `i % 8` of byte `i / 8` (pinned against C).
#[cfg(test)]
const SIG_FIELDS: usize = 17;
const SIG_LDVPM: usize = 7;
const SIG_SMALL_IMM_A: usize = 13;
const SIG_SMALL_IMM_B: usize = 14;
const SIG_SMALL_IMM_C: usize = 15;
const SIG_SMALL_IMM_D: usize = 16;

impl MSig {
    pub(super) fn get(&self, i: usize) -> bool {
        self.bytes[i / 8] & (1 << (i % 8)) != 0
    }
    pub(super) fn set(&mut self, i: usize, v: bool) {
        if v {
            self.bytes[i / 8] |= 1 << (i % 8);
        }
    }
}

/// `struct v3d_qpu_flags`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MFlags {
    pub ac: u32,
    pub mc: u32,
    pub apf: u32,
    pub mpf: u32,
    pub auf: u32,
    pub muf: u32,
}

/// `struct v3d_qpu_input`: `mux` (4.x) shares its storage with the 7.x
/// `uint8_t raddr`, which this mirror never uses.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MInput {
    pub mux: u32,
    pub unpack: u32,
}

/// The `add` and `mul` halves of `struct v3d_qpu_alu_instr` (the same shape).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MAluHalf {
    pub op: u32,
    pub a: MInput,
    pub b: MInput,
    pub waddr: u8,
    pub magic_write: bool,
    pub output_pack: u32,
}

/// `struct v3d_qpu_alu_instr`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MAlu {
    pub add: MAluHalf,
    pub mul: MAluHalf,
}

/// `struct v3d_qpu_branch_instr`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MBranch {
    pub cond: u32,
    pub msfign: u32,
    pub bdi: u32,
    pub bdu: u32,
    pub ub: bool,
    pub raddr_a: u8,
    pub offset: u32,
}

/// The anonymous union of `struct v3d_qpu_instr`.
#[repr(C)]
#[derive(Clone, Copy)]
pub union MBody {
    pub alu: MAlu,
    pub branch: MBranch,
}

/// `struct v3d_qpu_instr`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MInstr {
    pub type_: u32,
    pub sig: MSig,
    pub sig_addr: u8,
    pub sig_magic: bool,
    pub raddr_a: u8,
    pub raddr_b: u8,
    pub flags: MFlags,
    pub body: MBody,
}

impl MInstr {
    /// All zero: what the C sees as `{0}` (both union members zero).
    pub fn zeroed() -> MInstr {
        MInstr {
            type_: 0,
            sig: MSig::default(),
            sig_addr: 0,
            sig_magic: false,
            raddr_a: 0,
            raddr_b: 0,
            flags: MFlags::default(),
            // The larger member first, so every byte of the union is zero.
            body: if core::mem::size_of::<MAlu>() >= core::mem::size_of::<MBranch>() {
                MBody { alu: MAlu::default() }
            } else {
                MBody { branch: MBranch::default() }
            },
        }
    }
}

/// `enum v3d_qpu_instr_type`.
pub const INSTR_TYPE_ALU: u32 = 0;
pub const INSTR_TYPE_BRANCH: u32 = 1;
/// `V3D_QPU_A_VDWWT` (a V3D 3.3 operation). Mesa's 4.2 table lists it ahead
/// of IID with the same opcode, so its unpack names IID's word VDWWT; the
/// model has only [`AddOp::Iid`] (see there).
const A_VDWWT: u32 = 51;

/// An opaque `struct v3d_device_info`: only Mesa reads it.
#[repr(C)]
pub struct DeviceInfo {
    _private: [u8; 0],
}

extern "C" {
    /// V3D 4.2 and V3D 7.1 (qpu_mesa.c).
    static yakogl_v3d42_devinfo: DeviceInfo;
    static yakogl_v3d71_devinfo: DeviceInfo;

    fn v3d_qpu_instr_pack(devinfo: *const DeviceInfo, instr: *const MInstr, packed: *mut u64) -> bool;
    fn v3d_qpu_instr_unpack(devinfo: *const DeviceInfo, packed: u64, instr: *mut MInstr) -> bool;
    fn v3d_qpu_sig_unpack(devinfo: *const DeviceInfo, packed_sig: u32, sig: *mut MSig) -> bool;
    fn v3d_qpu_sig_writes_address(devinfo: *const DeviceInfo, sig: *const MSig) -> bool;
    fn v3d_qpu_small_imm_pack(devinfo: *const DeviceInfo, value: u32, packed: *mut u32) -> bool;
    fn v3d_qpu_small_imm_unpack(devinfo: *const DeviceInfo, packed: u32, value: *mut u32) -> bool;
    fn v3d_qpu_add_op_has_dst(op: u32) -> bool;
    fn v3d_qpu_mul_op_has_dst(op: u32) -> bool;
    fn v3d_qpu_add_op_num_src(op: u32) -> i32;
    fn v3d_qpu_mul_op_num_src(op: u32) -> i32;
    /// Bounded to Mesa's table (qpu_mesa.c).
    fn yakogl_v3d42_magic_waddr_name(waddr: u32) -> *const u8;
    fn yakogl_v3d71_magic_waddr_name(waddr: u32) -> *const u8;
    fn v3d_qpu_writes_rf0_implicitly(devinfo: *const DeviceInfo, instr: *const MInstr) -> bool;
    fn v3d_qpu_instr_is_sfu(instr: *const MInstr) -> bool;
}

#[cfg(test)]
extern "C" {
    static yakogl_v3d_qpu_layout: [u32; 0];
    static yakogl_v3d_qpu_layout_len: u32;
    static yakogl_v3d_qpu_enum_values: [u32; 0];
    static yakogl_v3d_qpu_enum_values_len: u32;
    static yakogl_v3d_devinfo_facts: [u32; 0];
    static yakogl_v3d_devinfo_facts_len: u32;
    fn yakogl_v3d_qpu_sig_images(out: *mut u8, size: u32) -> u32;
    fn v3d_qpu_add_op_name(op: u32) -> *const u8;
    fn v3d_qpu_mul_op_name(op: u32) -> *const u8;
    fn v3d_qpu_cond_name(c: u32) -> *const u8;
    fn v3d_qpu_pf_name(c: u32) -> *const u8;
    fn v3d_qpu_uf_name(c: u32) -> *const u8;
    fn v3d_qpu_pack_name(c: u32) -> *const u8;
    fn v3d_qpu_unpack_name(c: u32) -> *const u8;
    fn v3d_qpu_branch_cond_name(c: u32) -> *const u8;
    fn v3d_qpu_msfign_name(c: u32) -> *const u8;
}

/// The QPU generation a word is packed for: the `ver` of Mesa's
/// `struct v3d_device_info` (4.2, the Raspberry Pi 4; 7.1, the Raspberry
/// Pi 5). Everything that touches a word takes one; the 4.2 entry points
/// ([`pack`], [`unpack`] and the rest without a `_for`) are `Gen::V42`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gen {
    V42,
    V71,
}

impl Gen {
    pub const ALL: [Gen; 2] = [Gen::V42, Gen::V71];
}

fn devinfo_for(g: Gen) -> *const DeviceInfo {
    // Statics the C defines; only their addresses are taken.
    match g {
        Gen::V42 => &raw const yakogl_v3d42_devinfo,
        Gen::V71 => &raw const yakogl_v3d71_devinfo,
    }
}

fn devinfo() -> *const DeviceInfo {
    devinfo_for(Gen::V42)
}

/// Mesa's `v3d_qpu_instr_pack`. The caller has checked
/// [`pack_preconditions`]: Mesa asserts them.
pub fn pack(m: &MInstr) -> Option<u64> {
    pack_for(Gen::V42, m)
}

/// [`pack`] for generation `g`.
pub fn pack_for(g: Gen, m: &MInstr) -> Option<u64> {
    let mut w = 0u64;
    // SAFETY: `m` is a valid `struct v3d_qpu_instr` (layout pinned), `w` a
    // valid out pointer; the C keeps neither.
    unsafe { v3d_qpu_instr_pack(devinfo_for(g), m, &mut w) }.then_some(w)
}

/// Mesa's `v3d_qpu_instr_unpack`, into a zeroed mirror.
pub fn unpack(w: u64) -> Option<MInstr> {
    unpack_for(Gen::V42, w)
}

/// [`unpack`] for generation `g`.
pub fn unpack_for(g: Gen, w: u64) -> Option<MInstr> {
    let mut m = MInstr::zeroed();
    // SAFETY: as in `pack`.
    unsafe { v3d_qpu_instr_unpack(devinfo_for(g), w, &mut m) }.then_some(m)
}

/// Mesa's `v3d_qpu_writes_rf0_implicitly`: on 7.x the signals that used to
/// write r4/r5 (`ldunif`, `ldtmu`, `ldvary`, ...) write rf0 instead.
pub fn writes_rf0_implicitly(g: Gen, m: &MInstr) -> bool {
    // SAFETY: a valid `struct v3d_qpu_instr`.
    unsafe { v3d_qpu_writes_rf0_implicitly(devinfo_for(g), m) }
}

/// Mesa's `v3d_qpu_instr_is_sfu`: the add ALU does a special-function op
/// (RECIP .. RSQRT2; on 7.x these are ALU ops, not magic writes).
pub fn is_sfu(m: &MInstr) -> bool {
    // SAFETY: a valid `struct v3d_qpu_instr`.
    unsafe { v3d_qpu_instr_is_sfu(m) }
}

/// What `v3d_qpu_instr_pack` asserts rather than refuses (qpu_pack.c's
/// `QPU_SET_FIELD` asserts that a value fits its field; the LDVPM* cases
/// assert `!magic_write`). A mirror that fails this is never handed to it.
pub fn pack_preconditions(m: &MInstr) -> bool {
    match m.type_ {
        INSTR_TYPE_ALU => {
            // SAFETY: an ALU instruction's body is `alu`; zeroed() initialised
            // the whole union.
            let alu = unsafe { m.body.alu };
            let ldvpm = [AddOp::LdvpmvIn, AddOp::LdvpmvOut, AddOp::LdvpmdIn, AddOp::LdvpmdOut, AddOp::Ldvpmp]
                .iter()
                .chain(&[AddOp::LdvpmgIn, AddOp::LdvpmgOut])
                .any(|&o| o as u32 == alu.add.op);
            m.raddr_a < 64
                && m.raddr_b < 64
                && m.sig_addr < 64
                && alu.add.waddr < 64
                && alu.mul.waddr < 64
                && !(ldvpm && alu.add.magic_write)
        }
        // SAFETY: a branch's body is `branch`.
        INSTR_TYPE_BRANCH => unsafe { m.body.branch }.raddr_a < 64,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Queries the model answers through Mesa.
// ---------------------------------------------------------------------------

/// `v3d_qpu_sig_writes_address` for 4.2.
pub fn sig_writes_address(sig: &Sig) -> bool {
    sig_writes_address_mesa(Gen::V42, &sig_to_mesa(sig))
}

/// `v3d_qpu_sig_writes_address` of a mirror signal, for generation `g`.
pub fn sig_writes_address_mesa(g: Gen, m: &MSig) -> bool {
    // SAFETY: a valid `struct v3d_qpu_sig`.
    unsafe { v3d_qpu_sig_writes_address(devinfo_for(g), m) }
}

/// The signals of SIG field value `packed` (`v3d_qpu_sig_unpack`), `None`
/// for a reserved one.
pub fn sig_unpack(packed: u32) -> Option<Sig> {
    let mut m = MSig::default();
    // SAFETY: a valid out pointer.
    if !unsafe { v3d_qpu_sig_unpack(devinfo(), packed, &mut m) } {
        return None;
    }
    sig_from_mesa(&m)
}

/// `v3d_qpu_small_imm_unpack`.
pub fn small_imm_unpack(i: u32) -> Option<u32> {
    let mut v = 0;
    // SAFETY: a valid out pointer.
    unsafe { v3d_qpu_small_imm_unpack(devinfo(), i, &mut v) }.then_some(v)
}

/// `v3d_qpu_small_imm_pack`.
pub fn small_imm_pack(v: u32) -> Option<u32> {
    let mut i = 0;
    // SAFETY: a valid out pointer.
    unsafe { v3d_qpu_small_imm_pack(devinfo(), v, &mut i) }.then_some(i)
}

/// `v3d_qpu_add_op_has_dst` / `_num_src` as [`ARG_D`]/[`ARG_A`]/[`ARG_B`].
pub fn add_op_args(op: AddOp) -> u8 {
    add_op_args_raw(op as u32)
}

/// [`add_op_args`] of a Mesa add op value (the 7.1 model's operations too).
pub fn add_op_args_raw(op: u32) -> u8 {
    // SAFETY: `op` is a Mesa add op (pinned by name); the C asserts that.
    let (d, n) = unsafe { (v3d_qpu_add_op_has_dst(op), v3d_qpu_add_op_num_src(op)) };
    args(d, n)
}

/// `v3d_qpu_mul_op_has_dst` / `_num_src` as [`ARG_D`]/[`ARG_A`]/[`ARG_B`].
pub fn mul_op_args(op: MulOp) -> u8 {
    mul_op_args_raw(op as u32)
}

/// [`mul_op_args`] of a Mesa mul op value (the 7.1 model's operations too).
pub fn mul_op_args_raw(op: u32) -> u8 {
    // SAFETY: as in add_op_args.
    let (d, n) = unsafe { (v3d_qpu_mul_op_has_dst(op), v3d_qpu_mul_op_num_src(op)) };
    args(d, n)
}

fn args(d: bool, n: i32) -> u8 {
    (if d { ARG_D } else { 0 }) | if n >= 1 { ARG_A } else { 0 } | if n >= 2 { ARG_B } else { 0 }
}

/// `v3d_qpu_magic_waddr_name` for 4.2, `None` where Mesa's table has no
/// entry (25..31, 47..54) and past its end (56..63, where Mesa itself would
/// read out of bounds).
pub fn magic_waddr_name(waddr: u8) -> Option<&'static str> {
    magic_waddr_name_for(Gen::V42, waddr)
}

/// [`magic_waddr_name`] for generation `g`: on 7.1, 5 is `quad` and 55 `rep`.
pub fn magic_waddr_name_for(g: Gen, waddr: u8) -> Option<&'static str> {
    // SAFETY: NULL or a pointer to one of the C's static string literals.
    let p = unsafe {
        match g {
            Gen::V42 => yakogl_v3d42_magic_waddr_name(waddr as u32),
            Gen::V71 => yakogl_v3d71_magic_waddr_name(waddr as u32),
        }
    };
    if p.is_null() {
        return None;
    }
    // SAFETY: a NUL-terminated static ASCII string.
    let c = unsafe { core::ffi::CStr::from_ptr(p.cast()) };
    c.to_str().ok()
}

// ---------------------------------------------------------------------------
// The model <-> the mirror.
// ---------------------------------------------------------------------------

fn sig_to_mesa(s: &Sig) -> MSig {
    let mut m = MSig::default();
    for (i, v) in [
        s.thrsw,
        s.ldunif,
        s.ldunifa,
        s.ldunifrf,
        s.ldunifarf,
        s.ldtmu,
        s.ldvary,
        false,
        s.ldtlb,
        s.ldtlbu,
        s.ucb,
        s.rotate,
        s.wrtmuc,
        false,
        s.small_imm_b,
    ]
    .into_iter()
    .enumerate()
    {
        m.set(i, v);
    }
    m
}

fn sig_from_mesa(m: &MSig) -> Option<Sig> {
    if [SIG_LDVPM, SIG_SMALL_IMM_A, SIG_SMALL_IMM_C, SIG_SMALL_IMM_D].iter().any(|&i| m.get(i)) {
        return None;
    }
    let g = |i| m.get(i);
    Some(Sig {
        thrsw: g(0),
        ldunif: g(1),
        ldunifa: g(2),
        ldunifrf: g(3),
        ldunifarf: g(4),
        ldtmu: g(5),
        ldvary: g(6),
        ldtlb: g(8),
        ldtlbu: g(9),
        ucb: g(10),
        rotate: g(11),
        wrtmuc: g(12),
        small_imm_b: g(SIG_SMALL_IMM_B),
    })
}

/// The variant of `all` whose discriminant is `v`.
pub(super) fn find<T: Copy>(all: &[T], v: u32, disc: impl Fn(T) -> u32) -> Option<T> {
    all.iter().copied().find(|&x| disc(x) == v)
}

fn input_to_mesa(i: &Input) -> MInput {
    MInput { mux: i.mux as u32, unpack: i.unpack as u32 }
}

fn input_from_mesa(m: &MInput) -> Option<Input> {
    Some(Input { mux: find(&Mux::ALL, m.mux, |x| x as u32)?, unpack: find(&Unpack::ALL, m.unpack, |x| x as u32)? })
}

/// The model as Mesa's struct. Total: every model value is a Mesa value.
pub fn to_mesa(instr: &Instr) -> MInstr {
    let mut m = MInstr::zeroed();
    match instr {
        Instr::Alu(a) => {
            m.type_ = INSTR_TYPE_ALU;
            m.sig = sig_to_mesa(&a.sig);
            m.sig_addr = a.sig_addr;
            m.sig_magic = a.sig_magic;
            m.raddr_a = a.raddr_a;
            m.raddr_b = a.raddr_b;
            let f = &a.flags;
            m.flags = MFlags {
                ac: f.ac as u32,
                mc: f.mc as u32,
                apf: f.apf as u32,
                mpf: f.mpf as u32,
                auf: f.auf as u32,
                muf: f.muf as u32,
            };
            m.body = MBody {
                alu: MAlu {
                    add: MAluHalf {
                        op: a.add.op as u32,
                        a: input_to_mesa(&a.add.a),
                        b: input_to_mesa(&a.add.b),
                        waddr: a.add.waddr,
                        magic_write: a.add.magic,
                        output_pack: a.add.pack as u32,
                    },
                    mul: MAluHalf {
                        op: a.mul.op as u32,
                        a: input_to_mesa(&a.mul.a),
                        b: input_to_mesa(&a.mul.b),
                        waddr: a.mul.waddr,
                        magic_write: a.mul.magic,
                        output_pack: a.mul.pack as u32,
                    },
                },
            };
        }
        Instr::Branch(b) => branch_to_mesa(b, &mut m),
    }
    m
}

/// A branch as Mesa's struct (the same on every generation).
pub(super) fn branch_to_mesa(b: &Branch, m: &mut MInstr) {
    m.type_ = INSTR_TYPE_BRANCH;
    m.body = MBody {
        branch: MBranch {
            cond: b.cond as u32,
            msfign: b.msfign as u32,
            bdi: b.bdi as u32,
            bdu: b.bdu as u32,
            ub: b.ub,
            raddr_a: b.raddr_a,
            offset: b.offset,
        },
    };
}

/// Mesa's branch as the model, `None` for a value it has no variant for.
pub(super) fn branch_from_mesa(b: &MBranch) -> Option<Branch> {
    Some(Branch {
        cond: find(&BranchCond::ALL, b.cond, |x| x as u32)?,
        msfign: find(&Msfign::ALL, b.msfign, |x| x as u32)?,
        bdi: find(&BranchDest::ALL, b.bdi, |x| x as u32)?,
        ub: b.ub,
        bdu: find(&BranchDest::ALL, b.bdu, |x| x as u32)?,
        raddr_a: b.raddr_a,
        offset: b.offset,
    })
}

/// Mesa's struct as the model, as it stands (no canonical form applied),
/// `None` for a value the 4.2 model has no variant for.
pub fn from_mesa(m: &MInstr) -> Option<Instr> {
    match m.type_ {
        INSTR_TYPE_ALU => {
            // SAFETY: an ALU instruction's body is `alu`.
            let alu = unsafe { m.body.alu };
            let f = &m.flags;
            let add_op = if alu.add.op == A_VDWWT { AddOp::Iid as u32 } else { alu.add.op };
            Some(Instr::Alu(Alu {
                sig: sig_from_mesa(&m.sig)?,
                sig_addr: m.sig_addr,
                sig_magic: m.sig_magic,
                flags: Flags {
                    ac: find(&Cond::ALL, f.ac, |x| x as u32)?,
                    mc: find(&Cond::ALL, f.mc, |x| x as u32)?,
                    apf: find(&Pf::ALL, f.apf, |x| x as u32)?,
                    mpf: find(&Pf::ALL, f.mpf, |x| x as u32)?,
                    auf: find(&Uf::ALL, f.auf, |x| x as u32)?,
                    muf: find(&Uf::ALL, f.muf, |x| x as u32)?,
                },
                raddr_a: m.raddr_a,
                raddr_b: m.raddr_b,
                add: AddAlu {
                    op: find(&AddOp::ALL, add_op, |x| x as u32)?,
                    a: input_from_mesa(&alu.add.a)?,
                    b: input_from_mesa(&alu.add.b)?,
                    waddr: alu.add.waddr,
                    magic: alu.add.magic_write,
                    pack: find(&OutPack::ALL, alu.add.output_pack, |x| x as u32)?,
                },
                mul: MulAlu {
                    op: find(&MulOp::ALL, alu.mul.op, |x| x as u32)?,
                    a: input_from_mesa(&alu.mul.a)?,
                    b: input_from_mesa(&alu.mul.b)?,
                    waddr: alu.mul.waddr,
                    magic: alu.mul.magic_write,
                    pack: find(&OutPack::ALL, alu.mul.output_pack, |x| x as u32)?,
                },
            }))
        }
        INSTR_TYPE_BRANCH => {
            // SAFETY: a branch's body is `branch`.
            let b = unsafe { m.body.branch };
            Some(Instr::Branch(branch_from_mesa(&b)?))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, offset_of, size_of};
    use std::string::String;
    use std::vec::Vec;

    fn c_str(p: *const u8) -> Option<String> {
        if p.is_null() {
            return None;
        }
        // SAFETY: Mesa's name tables hold static NUL-terminated strings.
        Some(unsafe { core::ffi::CStr::from_ptr(p.cast()) }.to_str().unwrap().into())
    }

    /// `the_mirror_is_mesas_layout`'s list, in qpu_mesa.c's order.
    fn layout() -> Vec<usize> {
        type H = MAluHalf;
        vec![
            size_of::<MInstr>(),
            align_of::<MInstr>(),
            offset_of!(MInstr, type_),
            offset_of!(MInstr, sig),
            offset_of!(MInstr, sig_addr),
            offset_of!(MInstr, sig_magic),
            offset_of!(MInstr, raddr_a),
            offset_of!(MInstr, raddr_b),
            offset_of!(MInstr, flags),
            offset_of!(MInstr, body),
            offset_of!(MInstr, body),
            size_of::<MSig>(),
            align_of::<MSig>(),
            size_of::<MFlags>(),
            offset_of!(MFlags, ac),
            offset_of!(MFlags, mc),
            offset_of!(MFlags, apf),
            offset_of!(MFlags, mpf),
            offset_of!(MFlags, auf),
            offset_of!(MFlags, muf),
            size_of::<MInput>(),
            offset_of!(MInput, mux),
            offset_of!(MInput, mux),
            offset_of!(MInput, unpack),
            size_of::<MAlu>(),
            offset_of!(MAlu, add),
            offset_of!(MAlu, add) + offset_of!(H, op),
            offset_of!(MAlu, add) + offset_of!(H, a),
            offset_of!(MAlu, add) + offset_of!(H, b),
            offset_of!(MAlu, add) + offset_of!(H, waddr),
            offset_of!(MAlu, add) + offset_of!(H, magic_write),
            offset_of!(MAlu, add) + offset_of!(H, output_pack),
            offset_of!(MAlu, mul),
            offset_of!(MAlu, mul) + offset_of!(H, op),
            offset_of!(MAlu, mul) + offset_of!(H, a),
            offset_of!(MAlu, mul) + offset_of!(H, b),
            offset_of!(MAlu, mul) + offset_of!(H, waddr),
            offset_of!(MAlu, mul) + offset_of!(H, magic_write),
            offset_of!(MAlu, mul) + offset_of!(H, output_pack),
            size_of::<MBranch>(),
            offset_of!(MBranch, cond),
            offset_of!(MBranch, msfign),
            offset_of!(MBranch, bdi),
            offset_of!(MBranch, bdu),
            offset_of!(MBranch, ub),
            offset_of!(MBranch, raddr_a),
            offset_of!(MBranch, offset),
            // the enums, held as u32; bool
            4,
            4,
            4,
            4,
            4,
            4,
            4,
            4,
            4,
            4,
            4,
            4,
            size_of::<bool>(),
        ]
    }

    /// The layout pins: the mirror's sizes and offsets are the ones the C
    /// compiler gives the real header; the sig bit-fields sit where C puts
    /// them; the enum values the model and the mirror use are Mesa's, by
    /// Mesa's own name tables.
    #[test]
    fn the_mirror_is_mesas_layout() {
        // SAFETY: arrays of `_len` u32 defined in qpu_mesa.c.
        let c: &[u32] = unsafe {
            core::slice::from_raw_parts((&raw const yakogl_v3d_qpu_layout).cast(), yakogl_v3d_qpu_layout_len as usize)
        };
        let c: Vec<usize> = c.iter().map(|&v| v as usize).collect();
        assert_eq!(layout(), c, "the Rust mirror of struct v3d_qpu_instr differs from the header");

        // The same layout serves both devices; each device is what the model
        // says it is (ver, accumulators), read back from the C structs the
        // packer is called with.
        // SAFETY: arrays of `_len` u32 defined in qpu_mesa.c.
        let facts: &[u32] = unsafe {
            core::slice::from_raw_parts(
                (&raw const yakogl_v3d_devinfo_facts).cast(),
                yakogl_v3d_devinfo_facts_len as usize,
            )
        };
        assert_eq!(facts, &[42, 1, 71, 0], "devinfo: ver and has_accumulators for 4.2, then 7.1");
        // Both devices pack and unpack through the one mirror: a NOP word is
        // the same on both, and each gives back the word it packed.
        for g in Gen::ALL {
            let nop = {
                let mut m = MInstr::zeroed();
                m.type_ = INSTR_TYPE_ALU;
                // Writing a union field is safe: an ALU body.
                m.body.alu.add.op = AddOp::Nop as u32;
                m.body.alu.mul.op = MulOp::Nop as u32;
                m.body.alu.add.waddr = WADDR_NOP;
                m.body.alu.add.magic_write = true;
                m.body.alu.mul.waddr = WADDR_NOP;
                m.body.alu.mul.magic_write = true;
                if g == Gen::V71 {
                    m.body.alu.mul.b.mux = 63;
                }
                m
            };
            let w = pack_for(g, &nop).expect("a NOP packs");
            let back = unpack_for(g, w).expect("and unpacks");
            assert_eq!(pack_for(g, &back), Some(w), "{g:?}");
        }
        let mut img = [[0u8; 3]; SIG_FIELDS];
        // SAFETY: SIG_FIELDS images of size_of::<MSig>() bytes.
        let n = unsafe { yakogl_v3d_qpu_sig_images(img.as_mut_ptr().cast(), size_of::<MSig>() as u32) };
        assert_eq!(n as usize, size_of::<MSig>());
        for (i, bytes) in img.iter().enumerate() {
            let mut m = MSig::default();
            m.set(i, true);
            assert_eq!(m.bytes, *bytes, "sig field {i}");
        }

        // SAFETY: as above.
        let e: &[u32] = unsafe {
            core::slice::from_raw_parts(
                (&raw const yakogl_v3d_qpu_enum_values).cast(),
                yakogl_v3d_qpu_enum_values_len as usize,
            )
        };
        let mut want = vec![INSTR_TYPE_ALU, INSTR_TYPE_BRANCH];
        want.extend(Mux::ALL.iter().map(|&x| x as u32));
        want.extend(BranchDest::ALL.iter().map(|&x| x as u32));
        want.push(WADDR_NOP as u32);
        assert_eq!(e, &want[..], "instr type, mux, branch dest, WADDR_NOP");

        // Every variant against Mesa's name for its discriminant.
        let snake = |s: String| {
            let mut o = String::new();
            for (k, ch) in s.chars().enumerate() {
                if ch.is_ascii_uppercase() && k > 0 {
                    o.push('_');
                }
                o.push(ch.to_ascii_lowercase());
            }
            o
        };
        for op in AddOp::ALL {
            // SAFETY: a valid add op index (asserted by the C).
            let mesa = c_str(unsafe { v3d_qpu_add_op_name(op as u32) });
            assert_eq!(mesa, Some(snake(format!("{op:?}"))), "AddOp::{op:?} = {}", op as u32);
        }
        // SAFETY: as above.
        assert_eq!(c_str(unsafe { v3d_qpu_add_op_name(A_VDWWT) }).as_deref(), Some("vdwwt"));
        for op in MulOp::ALL {
            // SAFETY: as above.
            let mesa = c_str(unsafe { v3d_qpu_mul_op_name(op as u32) });
            assert_eq!(mesa, Some(snake(format!("{op:?}"))), "MulOp::{op:?} = {}", op as u32);
        }
        let names = |f: unsafe extern "C" fn(u32) -> *const u8, vals: &[u32]| -> Vec<Option<String>> {
            // SAFETY: indices inside Mesa's tables (each is its enum's range).
            vals.iter().map(|&v| c_str(unsafe { f(v) })).collect()
        };
        let s = |v: &[&str]| -> Vec<Option<String>> { v.iter().map(|x| Some(String::from(*x))).collect() };
        let cond: Vec<u32> = Cond::ALL.iter().map(|&x| x as u32).collect();
        assert_eq!(names(v3d_qpu_cond_name, &cond), s(&["", ".ifa", ".ifb", ".ifna", ".ifnb"]));
        let pf: Vec<u32> = Pf::ALL.iter().map(|&x| x as u32).collect();
        assert_eq!(names(v3d_qpu_pf_name, &pf), s(&["", ".pushz", ".pushn", ".pushc"]));
        let uf: Vec<u32> = Uf::ALL.iter().map(|&x| x as u32).collect();
        assert_eq!(
            names(v3d_qpu_uf_name, &uf),
            s(&[
                "", ".andz", ".andnz", ".nornz", ".norz", ".andn", ".andnn", ".nornn", ".norn", ".andc", ".andnc",
                ".nornc", ".norc"
            ])
        );
        let pack: Vec<u32> = OutPack::ALL.iter().map(|&x| x as u32).collect();
        assert_eq!(names(v3d_qpu_pack_name, &pack), s(&["", ".l", ".h"]));
        let unpack: Vec<u32> = Unpack::ALL.iter().map(|&x| x as u32).collect();
        assert_eq!(names(v3d_qpu_unpack_name, &unpack), s(&["", ".abs", ".l", ".h", ".ff", ".ll", ".hh", ".swp"]));
        // The 7.1 enums against Mesa's name tables, as the 4.2 ones above.
        for op in super::super::isa71::AddOp71::ALL {
            // SAFETY: a valid add op index.
            let mesa = c_str(unsafe { v3d_qpu_add_op_name(op as u32) });
            assert_eq!(mesa, Some(snake(format!("{op:?}"))), "AddOp71::{op:?} = {}", op as u32);
        }
        for op in super::super::isa71::MulOp71::ALL {
            // SAFETY: as above.
            let mesa = c_str(unsafe { v3d_qpu_mul_op_name(op as u32) });
            assert_eq!(mesa, Some(snake(format!("{op:?}"))), "MulOp71::{op:?} = {}", op as u32);
        }
        // Mesa names the unpacks up to MAX0 (4..6 are 7.1's); UL .. IH have
        // no name function entry (it is unreachable for them).
        let u71: Vec<u32> = (0..=10).collect();
        assert_eq!(
            names(v3d_qpu_unpack_name, &u71),
            s(&["", ".abs", ".l", ".h", ".sat", ".nsat", ".max0", ".ff", ".ll", ".hh", ".swp"])
        );
        let bc: Vec<u32> = BranchCond::ALL.iter().map(|&x| x as u32).collect();
        assert_eq!(names(v3d_qpu_branch_cond_name, &bc), s(&["", ".a0", ".na0", ".alla", ".anyna", ".anya", ".allna"]));
        let ms: Vec<u32> = Msfign::ALL.iter().map(|&x| x as u32).collect();
        assert_eq!(names(v3d_qpu_msfign_name, &ms), s(&["", "p", "q"]));
    }

    /// The model converts to the mirror and back unchanged, for every
    /// variant of every enum it holds.
    #[test]
    fn the_model_survives_the_mirror() {
        let mut a = Alu::nop();
        for op in AddOp::ALL {
            a.add.op = op;
            let i = Instr::Alu(a);
            assert_eq!(from_mesa(&to_mesa(&i)), Some(i));
        }
        for op in MulOp::ALL {
            a.mul.op = op;
            let i = Instr::Alu(a);
            assert_eq!(from_mesa(&to_mesa(&i)), Some(i));
        }
        for u in Unpack::ALL {
            a.add.a.unpack = u;
            a.mul.b.unpack = u;
            let i = Instr::Alu(a);
            assert_eq!(from_mesa(&to_mesa(&i)), Some(i));
        }
        for k in 0..13 {
            let mut s = Sig::default();
            let f = [
                &mut s.thrsw,
                &mut s.ldunif,
                &mut s.ldunifa,
                &mut s.ldunifrf,
                &mut s.ldunifarf,
                &mut s.ldtmu,
                &mut s.ldvary,
                &mut s.ldtlb,
                &mut s.ldtlbu,
                &mut s.ucb,
                &mut s.rotate,
                &mut s.wrtmuc,
                &mut s.small_imm_b,
            ];
            *f.into_iter().nth(k).unwrap() = true;
            a.sig = s;
            let i = Instr::Alu(a);
            assert_eq!(from_mesa(&to_mesa(&i)), Some(i));
        }
        for cond in BranchCond::ALL {
            for bdu in BranchDest::ALL {
                let b = Instr::Branch(Branch {
                    cond,
                    msfign: Msfign::Q,
                    bdi: BranchDest::Regfile,
                    ub: true,
                    bdu,
                    raddr_a: 7,
                    offset: 0x1234_5678,
                });
                assert_eq!(from_mesa(&to_mesa(&b)), Some(b));
            }
        }
        // A value the model has no variant for is a refusal.
        let mut m = to_mesa(&Instr::Alu(Alu::nop()));
        m.body.alu.add.op = 90; // V3D_QPU_A_FMOV, 7.x only
        assert_eq!(from_mesa(&m), None);
        let mut m = to_mesa(&Instr::Alu(Alu::nop()));
        m.sig.set(SIG_LDVPM, true);
        assert_eq!(from_mesa(&m), None);
    }
}
