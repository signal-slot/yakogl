// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Building a GC7000 instruction from an operation and its operands.
//!
//! [`super::decode`] and [`super::encode`] can read an instruction and write
//! it back; a code generator has to make one that never existed. That is what
//! this is: name an operation, give it a destination and sources, get 128
//! bits. It is the seam between the code generator (M3) and the machine.
//!
//! The operand model is the machine's, not an invention. A destination is a
//! register index, a write mask and an addressing mode, packed into the
//! 14-bit `DST` field. A source is a register - bank, index and addressing
//! mode, which live in the instruction's own `SRCn_RGROUP` / `SRCn_REG` /
//! `SRCn_AMODE` fields - plus a per-component swizzle and the negate and
//! absolute modifiers, packed into the 10-bit `SRCn` field. The data type is
//! three bits split across the instruction (bit 53 and bits 95:94), which
//! [`Builder::ty`] writes so a caller never has to know that.
//!
//! Nothing here allocates and nothing panics on bad input: an operation that
//! does not exist, or a field an operation does not have, is a
//! [`BuildError`].

use super::isa::{Field, Instr, MAX_FIELDS};
use super::isa_tables::OPS;

/// Which register file an operand comes from (`#reg_group`).
pub const RGROUP_TEMP: u8 = 0;
pub const RGROUP_INTERNAL: u8 = 1;
pub const RGROUP_UNIFORM_0: u8 = 2;
pub const RGROUP_UNIFORM_1: u8 = 3;
pub const RGROUP_THREAD: u8 = 4;
pub const RGROUP_IMMED: u8 = 7;

/// A swizzle that takes each component from itself (`.xyzw`).
pub const SWIZZLE_IDENTITY: [u8; 4] = [0, 1, 2, 3];
/// Every component written.
pub const WRITE_MASK_ALL: u8 = 0xf;

/// One source operand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Src {
    /// `#reg_group`: which register file.
    pub group: u8,
    /// The register index (9 bits).
    pub reg: u16,
    /// `#reg_addressing_mode` (3 bits).
    pub amode: u8,
    /// Where each of x, y, z, w comes from (2 bits each).
    pub swizzle: [u8; 4],
    pub neg: bool,
    pub abs: bool,
}

impl Src {
    /// A plain read of a whole register, unswizzled and unmodified.
    pub fn reg(group: u8, reg: u16) -> Src {
        Src { group, reg, amode: 0, swizzle: SWIZZLE_IDENTITY, neg: false, abs: false }
    }

    /// The 10-bit `SRCn` field: swizzle, then negate, then absolute.
    fn packed(&self) -> u32 {
        let sw = (self.swizzle[0] as u32 & 3)
            | ((self.swizzle[1] as u32 & 3) << 2)
            | ((self.swizzle[2] as u32 & 3) << 4)
            | ((self.swizzle[3] as u32 & 3) << 6);
        sw | ((self.neg as u32) << 8) | ((self.abs as u32) << 9)
    }

    fn from_parts(group: u32, reg: u32, amode: u32, packed: u32) -> Src {
        Src {
            group: group as u8,
            reg: reg as u16,
            amode: amode as u8,
            swizzle: [
                (packed & 3) as u8,
                ((packed >> 2) & 3) as u8,
                ((packed >> 4) & 3) as u8,
                ((packed >> 6) & 3) as u8,
            ],
            neg: (packed >> 8) & 1 != 0,
            abs: (packed >> 9) & 1 != 0,
        }
    }
}

/// The register file an immediate lives in: the source's other fields stop
/// being a register and become the value.
pub const RGROUP_IMMED_GROUP: u8 = RGROUP_IMMED;

/// What kind of immediate a source carries (`IMMED_TYPE`, which the encoding
/// derives as `SRC_AMODE >> 1`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum ImmKind {
    /// A 32-bit float whose low 12 mantissa bits are zero.
    F20 = 0,
    S20 = 1,
    U20 = 2,
    F16 = 3,
}

/// Why a value cannot be an immediate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImmError {
    /// A float whose low 12 mantissa bits are not zero. The encoding keeps
    /// the top 20 bits of the f32 and nothing else, so this value cannot be
    /// carried exactly - and rounding it silently is how a shader draws a
    /// colour nobody asked for. Put it in the constant file instead.
    NotExactlyRepresentable,
    /// An integer outside the 20 bits the encoding has.
    OutOfRange,
}

impl Src {
    /// A 32-bit float carried inside the instruction.
    ///
    /// The encoding is not a register: the 20-bit payload is spread across
    /// what would otherwise be `SRC_REG`, `SRC_SWIZ`, `SRC_NEG`, `SRC_ABS`
    /// and the low bit of `SRC_AMODE`, and it is the TOP 20 bits of the f32
    /// (the description's own expression, kept verbatim in the table:
    /// `((((SRC_AMODE & 0x1) << 19) | (SRC_ABS << 18) | (SRC_NEG << 17) |
    /// (SRC_SWIZ << 9) | SRC_REG) << 12)`).
    ///
    /// So a value whose low 12 mantissa bits are not zero **cannot be
    /// represented** and is refused rather than rounded. 1.0, 0.5, 2.0 and
    /// every small power of two are exact; 0.1 is not.
    pub fn imm_f32(v: f32) -> Result<Src, ImmError> {
        let bits = v.to_bits();
        if bits & 0xfff != 0 {
            return Err(ImmError::NotExactlyRepresentable);
        }
        Ok(Src::imm_payload(bits >> 12, ImmKind::F20))
    }

    /// A 20-bit signed immediate.
    pub fn imm_s20(v: i32) -> Result<Src, ImmError> {
        if !(-(1 << 19)..(1 << 19)).contains(&v) {
            return Err(ImmError::OutOfRange);
        }
        Ok(Src::imm_payload((v as u32) & 0xf_ffff, ImmKind::S20))
    }

    /// A 20-bit unsigned immediate.
    pub fn imm_u20(v: u32) -> Result<Src, ImmError> {
        if v >= (1 << 20) {
            return Err(ImmError::OutOfRange);
        }
        Ok(Src::imm_payload(v, ImmKind::U20))
    }

    /// Spread a 20-bit payload across the fields the encoding uses for it.
    fn imm_payload(payload: u32, kind: ImmKind) -> Src {
        Src {
            group: RGROUP_IMMED,
            reg: (payload & 0x1ff) as u16,
            // IMMED_TYPE is `SRC_AMODE >> 1`, and the low bit of AMODE is the
            // payload's bit 19: the two share the field.
            amode: (((kind as u32) << 1) | ((payload >> 19) & 1)) as u8,
            swizzle: [
                ((payload >> 9) & 3) as u8,
                ((payload >> 11) & 3) as u8,
                ((payload >> 13) & 3) as u8,
                ((payload >> 15) & 3) as u8,
            ],
            neg: (payload >> 17) & 1 != 0,
            abs: (payload >> 18) & 1 != 0,
        }
    }

    /// Is this source an immediate rather than a register? A listing that
    /// does not ask prints a plausible register that does not exist.
    pub fn is_immediate(&self) -> bool {
        self.group == RGROUP_IMMED
    }

    /// The 20-bit payload, for a source that carries one.
    pub fn imm_payload_bits(&self) -> Option<u32> {
        if !self.is_immediate() {
            return None;
        }
        let sw = (self.swizzle[0] as u32 & 3)
            | ((self.swizzle[1] as u32 & 3) << 2)
            | ((self.swizzle[2] as u32 & 3) << 4)
            | ((self.swizzle[3] as u32 & 3) << 6);
        Some(
            ((self.amode as u32 & 1) << 19)
                | ((self.abs as u32) << 18)
                | ((self.neg as u32) << 17)
                | (sw << 9)
                | self.reg as u32,
        )
    }
}

/// The destination operand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Dst {
    /// The temporary register index (7 bits).
    pub reg: u8,
    /// Which components are written, bit 0 = x (`#wrmask`).
    pub write_mask: u8,
    /// `#reg_addressing_mode` (3 bits).
    pub amode: u8,
}

impl Dst {
    pub fn reg(reg: u8, write_mask: u8) -> Dst {
        Dst { reg, write_mask, amode: 0 }
    }

    /// The destination of `movar` and `movai`: the **address register**, not
    /// a temporary. Those two operations carry the same `DST` field as every
    /// other ALU operation, but its `REG` half does not name a temporary --
    /// the operation itself says where the write goes, and `REG` is 0. Only
    /// the write mask is a choice, and it is a real one: the hardware
    /// database carries an erratum bit for parts that write dirty data into
    /// a `movai`/`movar` destination's *unused* components
    /// (`SH_MOVAI_MOVAR_UNUSED_COMPONENTS_WRITE_DIRTY_DATA_FIX`).
    ///
    /// Measured: all 7 `movar`s in the shipped GC7000 table encode
    /// `DST = 0x400`, that is `reg = 0, write_mask = 0x1, amode = 0`, and
    /// Mesa's etnaviv emits the same shape (`dst.use = 1`,
    /// `dst.write_mask = ISA_WRMASK_X___`, no `dst.reg`) followed by a read
    /// with `amode = AX`.
    pub fn address(write_mask: u8) -> Dst {
        Dst { reg: 0, write_mask, amode: 0 }
    }

    /// The 14-bit `DST` field: addressing mode, register, write mask.
    fn packed(&self) -> u32 {
        (self.amode as u32 & 7) | ((self.reg as u32 & 0x7f) << 3) | ((self.write_mask as u32 & 0xf) << 10)
    }

    fn from_packed(v: u32) -> Dst {
        Dst { amode: (v & 7) as u8, reg: ((v >> 3) & 0x7f) as u8, write_mask: ((v >> 10) & 0xf) as u8 }
    }
}

/// An operation, resolved once. A code generator resolves the handful it
/// emits at start-up and passes these, not names: `Op::name` is unique but
/// `Op::mnemonic` is not (`branch_unary` and `branch_binary` are both spelled
/// `branch`), so a name in an inner loop is both a cost and an ambiguity
/// waiting to be written.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OpId(pub u16);

/// Resolve an operation by its bitset name (the unique one).
pub fn op_id(name: &str) -> Option<OpId> {
    OPS.iter().position(|o| o.name == name).map(|i| OpId(i as u16))
}

impl OpId {
    pub fn op(self) -> &'static super::isa::Op {
        &OPS[self.0 as usize]
    }

    /// Does this operation have source slot `n`? On this machine the slot is
    /// the hardware's, not the operand ordinal - a `mov` reads SRC2 and
    /// nothing else - so a generator that fills slots by ordinal produces an
    /// instruction that reads the wrong registers and still encodes.
    pub fn has_src(self, n: usize) -> bool {
        let f = match n {
            0 => "SRC0",
            1 => "SRC1",
            2 => "SRC2",
            _ => return false,
        };
        self.op().fields.iter().any(|x| x.name == f)
    }

    /// Is this a plain register move? Used to classify the corpus's copies:
    /// a `mov` between two values may be removable by coalescing, while one
    /// that materialises a constant or crosses into an io register is not.
    pub fn is_mov(self) -> bool {
        self.op().name == "mov"
    }

    /// Does this operation leave its `DST` field unused?
    ///
    /// Some operations carry a `DST` field and have no result to put in it.
    /// Measured over the shipped GC7000 table: 36 distinct operations with a
    /// `DST` field appear, and exactly two never set `DST_USE` --
    /// `texkill` (16 instructions, 0 with `DST_USE = 1`) and `nop`
    /// (15 instructions, 0). The other 34 set it in **every** instance, so
    /// this is a clean split rather than a threshold.
    ///
    /// `texkill_unary` and `texkill_binary` are in the set on the family
    /// argument, not on a measurement: neither appears in the table, and a
    /// conditional discard has no more of a result than an unconditional
    /// one. Being wrong about them costs a refusal, not a wrong picture.
    ///
    /// This is the twin of the `texld` SRC2 exception in `check_shape`: an
    /// operation the hardware never gives a destination must not be made to
    /// carry one, and a zero write mask is not a way out -- it would still
    /// set `DST_USE`, which no shipped instruction does.
    pub fn writes_no_dst(self) -> bool {
        matches!(self.op().name, "texkill" | "texkill_unary" | "texkill_binary" | "nop")
    }

    /// May a source operand of `group` be given to THIS operation?
    ///
    /// The per-instruction caps (`tests/operand_banks.rs`: temp 3, uniform 3,
    /// immediate 2) say how MANY operands of a group an instruction holds.
    /// They do not say which operations hold them, and the reference compiler
    /// answers that separately, per operation, over the same 13,908
    /// instructions:
    ///
    /// ```text
    ///   texld     568 temp operands, 0 uniform, 0 immediate
    ///   branch*   208 temp operands, 0 uniform, 0 immediate
    ///   mad      6962 temp,       2912 uniform,  221 immediate
    ///   mul      4132 temp,        947 uniform,   97 immediate
    /// ```
    ///
    /// **A texture instruction's coordinate is always a temporary** in the
    /// shipped table, and so is a branch's condition. Folding a uniform read
    /// or a literal into one is not something this machine is known to
    /// accept, so it is refused rather than tried -- and being wrong about a
    /// refusal costs an instruction, while being wrong about a fold costs a
    /// picture.
    ///
    /// The lists are not a guess at the encoding: they are what the reference
    /// emits, and `tests/operand_banks.rs` re-derives them from the shipped
    /// blobs and fails if this disagrees. `texldl` and `txf` do carry
    /// immediates (their LOD), which is the reason this is stated per
    /// operation and not for "texture instructions".
    ///
    /// `load` takes a uniform since the table's run-time-index rows became
    /// Mesa's UBO load (2026-10-04): `load.u32 t, t.x, u[ubo_addr]`, the
    /// base address in SRC1 (3 of 3 loads in the table; gl_test 45 PASS on
    /// the i.MX8MP with it).
    ///
    /// A temporary is allowed wherever the operation has a source slot at
    /// all: every operation that appears with any operand appears with a
    /// temporary one.
    pub fn takes_group(self, group: u8) -> bool {
        let name = self.op().name;
        match group {
            RGROUP_TEMP => true,
            RGROUP_INTERNAL => name == "cmp",
            RGROUP_UNIFORM_0 => matches!(
                name,
                "add"
                    | "and"
                    | "cmp"
                    | "dp2"
                    | "dp3"
                    | "f2i"
                    | "floor"
                    | "i2f"
                    | "iabs"
                    | "imullo0"
                    | "load"
                    | "mad"
                    | "mov"
                    | "mul"
                    | "rcp"
                    | "rshift"
                    | "select"
                    | "sqrt"
                    | "texldl"
                    | "xor"
            ),
            RGROUP_IMMED => matches!(
                name,
                "add"
                    | "and"
                    | "cmp"
                    | "dp3"
                    | "imullo0"
                    | "mad"
                    | "mov"
                    | "mul"
                    | "rshift"
                    | "select"
                    | "texldl"
                    | "txf"
            ),
            /* `uniform_1` and `thread` appear in no instruction of the
             * reference at all. */
            _ => false,
        }
    }

    /// Does this operation write the **address register** instead of a
    /// temporary? True for `movar` and `movai`, and for nothing else.
    ///
    /// The two are one family: the hardware feature database names them
    /// together in `SH_MOVAI_MOVAR_UNUSED_COMPONENTS_WRITE_DIRTY_DATA_FIX`,
    /// an erratum about the unused components of *their destination*, which
    /// is what says the destination is the same special one for both. Their
    /// field lists in the bitset are identical.
    ///
    /// Only `movar` appears in the shipped table (7 instructions, all
    /// `DST = 0x400`); `movai` is included on the shared-erratum evidence
    /// rather than on a measurement of our own. Being wrong about `movai`
    /// costs a refusal, not a wrong picture: a generator that wanted a
    /// temporary destination there is told so instead of quietly writing r0.
    pub fn dst_is_address_register(self) -> bool {
        matches!(self.op().name, "movar" | "movai")
    }

    /// The hardware source slots this operation has, in ascending order.
    /// That order is the operand order: the description prints an operation's
    /// operands in slot order and fills the gaps with `void`
    /// (`{INSTR_ALU} {DST}, {SRC0}, void, {SRC2}`), so operand *n* of an
    /// operation is its *n*th slot counting up.
    pub fn slots(self) -> [Option<usize>; 3] {
        let mut out = [None; 3];
        let mut k = 0;
        for n in 0..3 {
            if self.has_src(n) {
                out[k] = Some(n);
                k += 1;
            }
        }
        out
    }

    /// The hardware slot that carries operand `ordinal`.
    ///
    /// This is the translation the code generator does not have to know: a
    /// `mov` takes one operand and it goes in SRC2, not SRC0. Slot knowledge
    /// lives here and nowhere else.
    pub fn slot_of_operand(self, ordinal: usize) -> Option<usize> {
        self.slots().get(ordinal).copied().flatten()
    }

    /// How many operands this operation takes.
    pub fn num_operands(self) -> usize {
        self.slots().iter().filter(|s| s.is_some()).count()
    }

    /// Does this operation read only ONE component of its sources?
    ///
    /// The transcendentals, the divide and the integer multiply are scalar on
    /// this machine: Mesa composes their source swizzle with `xxxx` before
    /// encoding ("scalar instructions we want src to be in x component",
    /// `etnaviv_compiler_nir_emit.c`), and every shipped shader agrees --
    /// across the 13,852 instructions the driver ships, all 2,323 sources of a
    /// scalar operation carry a uniform swizzle and none carries a varied one,
    /// while non-scalar operations are varied about half the time. So the
    /// property discriminates rather than being an artefact of most swizzles
    /// happening to be uniform.
    ///
    /// What the corpus does NOT say is whether the hardware ignores the other
    /// three lanes or acts on them. Either way the lowering is the same and
    /// matches what the machine has been fed; that is recorded rather than
    /// resolved.
    pub fn reads_one_component(self) -> bool {
        matches!(self.op().name, "div" | "log" | "sin" | "cos" | "rsq" | "rcp" | "exp" | "sqrt" | "imullo0")
    }

    /// Does this operation compute each destination lane from the same lane
    /// of its sources?
    ///
    /// True for the ordinary arithmetic, and it is what says whether a
    /// consumer's swizzle may be pushed back into the producer: to compute in
    /// lane `i` what it computed in lane `j`, a componentwise operation needs
    /// only its sources read with `j` where they were read with `i`, and the
    /// rest of the lanes are masked off.
    ///
    /// **A dot product is the counter-example and the reason this is a
    /// predicate rather than an assumption**: `dp3` sums across the lanes, so
    /// permuting its sources' swizzles changes the sum rather than moving it.
    /// So does a texture fetch, whose lanes are coordinates. Being wrong here
    /// is not a lost instruction, it is a shader computing something else.
    ///
    /// A scalar operation (`reads_one_component`) is componentwise for this
    /// purpose: the builder splats its source lanes, so every destination
    /// lane holds the same value and any permutation of them is the identity.
    pub fn computes_lanewise(self) -> bool {
        !matches!(self.op().name, "dp2" | "dp3" | "dp4" | "texld" | "texldb" | "texldd" | "texldl" | "txf")
    }

    /// Whether the operation carries a field by that name at all.
    pub fn has_field(self, name: &str) -> bool {
        self.op().fields.iter().any(|x| x.name == name)
    }

    pub fn has_dst(self) -> bool {
        self.op().fields.iter().any(|x| x.name == "DST")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildError {
    /// No operation of that name.
    NoSuchOp,
    /// The operation has no such field: it does not take that operand.
    NoSuchField(&'static str),
    /// A value does not fit the field it was given for.
    ValueTooWide(&'static str),
}

/// Reading the operands back off a decoded instruction, so a generated
/// instruction and a decoded one can be compared as operands rather than as
/// bits.
impl Instr {
    pub fn dst(&self) -> Option<Dst> {
        self.get("DST").map(Dst::from_packed)
    }

    /// Source `n` (0, 1 or 2), when the operation has one and, where it
    /// carries a `SRC{n}_USE` field, marks it used: a slot whose use bit is
    /// clear is not read, whatever its other fields hold (texld's SRC2).
    pub fn src(&self, n: usize) -> Option<Src> {
        let (g, r, a, p, u) = match n {
            0 => ("SRC0_RGROUP", "SRC0_REG", "SRC0_AMODE", "SRC0", "SRC0_USE"),
            1 => ("SRC1_RGROUP", "SRC1_REG", "SRC1_AMODE", "SRC1", "SRC1_USE"),
            2 => ("SRC2_RGROUP", "SRC2_REG", "SRC2_AMODE", "SRC2", "SRC2_USE"),
            _ => return None,
        };
        if self.get(u) == Some(0) {
            return None;
        }
        Some(Src::from_parts(self.get(g)?, self.get(r)?, self.get(a)?, self.get(p)?))
    }
}

/// Builds one instruction. Every setter is fallible and the errors are kept
/// until [`Builder::build`], so a chain reads as one expression.
pub struct Builder {
    instr: Instr,
    err: Option<BuildError>,
}

impl Builder {
    /// Start an instruction of the named operation.
    pub fn new(op: &str) -> Builder {
        match OPS.iter().position(|o| o.name == op) {
            /* Through `of`, so the two constructors cannot drift apart in
             * their defaults -- which is exactly how PMODE would come back. */
            Some(i) => Builder::of(OpId(i as u16)),
            None => {
                Builder { instr: Instr { op: 0, vals: [0; MAX_FIELDS], dont_care: 0 }, err: Some(BuildError::NoSuchOp) }
            }
        }
    }

    /// Start an instruction of an already-resolved operation.
    ///
    /// `dont_care` starts zero. That is the measured choice rather than a
    /// default nobody thought about: no instruction in the 13,854 the driver
    /// ships has one of those bits set
    /// (`no_dont_care_bit_is_set_in_the_corpus`), so zero is what the machine
    /// has actually been fed. A shader that needs one non-zero is a decision
    /// someone makes on purpose, through [`Builder::set`].
    ///
    /// The fields every operation carries, and which no operand sets, are the
    /// group most likely to be quietly wrong, so each is measured against the
    /// corpus by `every_builder_default_matches_the_corpus` rather than
    /// assumed. Over all 13,910 instructions the driver ships:
    ///
    /// | field                   | corpus            | default |
    /// |-------------------------|-------------------|---------|
    /// | `PMODE`                 | 1 in 13,164, 0 in the 16 `nop`s | **1**, and 0 for `nop` |
    /// | `SKPHP`, `UNK`, `DST_FULL` | 0 in all 13,180 | 0     |
    /// | `LOW_HALF`, `HIGH_HALF` | 0 in all 13,607   | 0       |
    /// | `RMODE`                 | 0 in 13,524, 1 in 83 | 0    |
    ///
    /// `PMODE` was 0 here until it was measured, and the justification that
    /// stood in this comment was that the round trip would have caught it.
    /// It could not: the round trip decodes an instruction and re-encodes it,
    /// so it carries whatever the corpus had through both directions and
    /// agrees with itself. It never constructs a default and asks what that
    /// produces. gles2-core found the bit by diffing our generated words
    /// against a shipped blob on silicon; a check that cannot fail under the
    /// hypothesis is not a check.
    ///
    /// The 83 `RMODE` exceptions are the transcendentals, which Mesa rounds
    /// toward zero on a part with the newer units; a caller that needs one
    /// sets it. `SAT` is not in the table because it is an operand here, not
    /// a default.
    pub fn of(op: OpId) -> Builder {
        let b = Builder { instr: Instr { op: op.0, vals: [0; MAX_FIELDS], dont_care: 0 }, err: None };
        /* Every instruction the driver ships has PMODE 1 except `nop`, whose
         * whole encoding is zero -- 13,164 to 16, with no other exception.
         * Operations without the field skip it rather than raise
         * NoSuchField. */
        if op.has_field("PMODE") && op.op().name != "nop" {
            b.set("PMODE", 1)
        } else {
            b
        }
    }

    fn field(&self, name: &str) -> Option<(usize, &'static Field)> {
        self.instr.op().fields.iter().enumerate().find(|(_, f)| f.name == name)
    }

    /// Set one field of the instruction by name. The escape hatch, and what
    /// every setter below is written in terms of.
    pub fn set(mut self, name: &'static str, v: u32) -> Builder {
        if self.err.is_some() {
            return self;
        }
        match self.field(name) {
            None => self.err = Some(BuildError::NoSuchField(name)),
            Some((i, f)) => {
                let bits = f.bits();
                if bits < 32 && v >= (1u32 << bits) {
                    self.err = Some(BuildError::ValueTooWide(name));
                } else {
                    self.instr.vals[i] = v;
                }
            }
        }
        self
    }

    /// Write the destination, and mark it used.
    pub fn dst(self, d: Dst) -> Builder {
        self.set("DST_USE", 1).set("DST", d.packed())
    }

    /// Write source `n`: its register file, index and addressing mode, and
    /// its swizzle and modifiers -- and mark it used where the operation
    /// carries a `SRC{n}_USE` field.
    ///
    /// Most operations pin their use bits in the table, but `texldb` and
    /// `texldl` carry all three as fields (and `texld` its SRC2), and a
    /// source written without its bit is a source the machine does not read:
    /// every texldl and texldb went out with SRC0_USE = SRC1_USE = 0, no
    /// coordinate and no level (the corpus's 78 texldl have both set), and
    /// dEQP's texture_functions failed on the i.MX8MP for it, even on a
    /// one-level texture (2026-09-24).
    pub fn src(self, n: usize, s: Src) -> Builder {
        let (g, r, a, p, u) = match n {
            0 => ("SRC0_RGROUP", "SRC0_REG", "SRC0_AMODE", "SRC0", "SRC0_USE"),
            1 => ("SRC1_RGROUP", "SRC1_REG", "SRC1_AMODE", "SRC1", "SRC1_USE"),
            2 => ("SRC2_RGROUP", "SRC2_REG", "SRC2_AMODE", "SRC2", "SRC2_USE"),
            _ => return Builder { instr: self.instr, err: Some(BuildError::NoSuchField("SRC?")) },
        };
        let b = if self.field(u).is_some() { self.set(u, 1) } else { self };
        b.set(g, s.group as u32).set(r, s.reg as u32).set(a, s.amode as u32).set(p, s.packed())
    }

    /// The operand data type (`#type`), three bits the encoding splits: the
    /// low two at bits 95:94 and the high one at bit 53.
    pub fn ty(self, t: u8) -> Builder {
        self.set("TYPE_BIT01", t as u32 & 3).set("TYPE_BIT2", (t as u32 >> 2) & 1)
    }

    /// Set a boolean modifier that the description gives a spelling to
    /// (`.sat`, `.pack`, `.hp`, ...).
    pub fn flag(self, name: &'static str, on: bool) -> Builder {
        self.set(name, on as u32)
    }

    pub fn build(self) -> Result<Instr, BuildError> {
        match self.err {
            Some(e) => Err(e),
            None => Ok(self.instr),
        }
    }
}
