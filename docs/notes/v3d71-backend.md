<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The V3D 7.1 back end (Raspberry Pi 5): design

Plan #365, item `design-note`: item P8 of `docs/notes/pre-study-rpi5.md`
(section 8), the online compiler back end for V3D 7.1, on the host only. This
note is the design. It changes no code. It fixes the file names, types and
function signatures that the later items implement (`scaffold71`,
`select71`, `alloc71`, `sched71`, `sched71-cf`, `chain71`, `corpus71`,
`chain-hook`). The Results section at the end is empty. Item `results-note`
fills it after `pins42`.

Sources. Mesa lines are Mesa 26.2.1, MIT. The tree read is the dependency
cache's `mesa-pan/mesa-26.2.1`, from the tarball whose sha256 is
`third_party/mesa-26.2.1/UPSTREAM`'s
(`c47e81bd...4a4442`). Its `src/broadcom/qpu/` is byte-identical to the
vendored one. `compiler/` is not vendored (pre-study U12). Mesa is cited as
`file:line` and never copied. Unless a path is given, `qpu_schedule.c`,
`qpu_validate.c`, `vir_register_allocate.c`, `nir_to_vir.c`, `vir.c`,
`vir_to_qpu.c`, `v3d_compiler.h` and `v3d_nir_lower_io.c` are in
`src/broadcom/compiler/`, and `qpu_pack.c`, `qpu_instr.c` and `qpu_instr.h`
are in `src/broadcom/qpu/`. Repository lines are at `8238361d`. The
exception is `verify71.rs`, which is at main `7e6f0766`, where it was merged.
No GPL source was read.

## What exists, and what P8 adds

On main: the 7.1 instruction model `isa71.rs` (`Instr71`, `Alu71`, `Sig71`,
`AddOp71`, `MulOp71`, `Operand { raddr, unpack }`), the codec `codec71.rs`
(`decode71`, `encode71`, `Gen`, `AnyInstr`), `listing71.rs`, `golden71.rs`,
`roundtrip71.rs`, the interpreter `sim71.rs` (`run71`, or
`sim::run_gen(Gen::V71, ..)`), and the verifier `verify71.rs` (`verify71`,
`registers_fit_threads71`, `peripherals_compatible71`, `is_sfu_op`). The 4.2
back end is `prepare.rs`, `select.rs`, `alloc.rs`, `sched.rs`, `order.rs`,
`verify.rs` and `chain.rs`, and `docs/online-shaderc/v3d-backend.md` records
it. Nothing selects, allocates or schedules for 7.1 yet.

P8 adds a 7.1 back half behind the shared front half:

```text
             shared, unchanged                    7.1 only, new files
IR --prepare.rs--> Prepared --select.rs--> Selected --select71--> Selected71
                                                    --alloc71---> Allocation71
                                                    --sched71/sched71_cf--> Instr71 words
                                                    --codec71 encode71, verify71--> Code71
                                                    chain71: the three stages, prog data
```

## (a) The generation as a parameter, with no 4.2 word moved

### The rule

The generation is decided once, in the chain, and selects which back half
runs. `prepare.rs` and `select.rs` run for both generations. They produce the
4.2-shaped `Selected`, which describes what to compute and is close to
generation-free (see `select71` below). At 7.1, everything after `Selected`
lives in new files. No 4.2 file gains a `Gen` test in its code path, so no
4.2 output word can move. Item `pins42` reruns every 4.2 pin to show it.

The shared files get only these hooks:

| Shared file | Hook | Item | When |
|---|---|---|---|
| `src/shaderc/online/v3d/mod.rs` | `pub mod vop71; pub mod select71; pub mod alloc71; pub mod sched71; mod sched71_cf; pub mod chain71;` and one header sentence | `scaffold71` | after `verify71`'s `mod.rs` edit (merged, `7e6f0766`) |
| `src/shaderc/online/v3d/chain.rs` | factor the body of `compile_program_keyed` into `pub(super) fn compile_with` (below), and give `FsProg`, `VsProg` and `V3dProgram` a defaulted type parameter `C = Code<'a>` for their `code` field. A defaulted parameter keeps every existing `FsProg<'a>` / `V3dProgram<'a>` name and struct literal compiling unchanged | `chain-hook` | after #359 releases `chain.rs`, rebased on the current origin/main |

`select.rs`, `alloc.rs`, `sched.rs`, `order.rs`, `verify.rs`, `prepare.rs`,
`sim.rs` and `v3d_abi.rs` get no hook. The 7.1 differences that `select.rs`
would otherwise carry (the viewport rounding, the missing `vpmwt`, the
payload numbers, the C coefficient's register) are rewritten by `select71`
on the selected operations. They are not decided inside `select.rs`.

### The files and their interfaces

All files are in `src/shaderc/online/v3d/`. `scaffold71` creates every file
below with these signatures. Bodies that are not yet written refuse by name
(`NotYet` / `ControlFlow` / `NotOn71`); they never return a wrong value. Each
later item fills in its own file. Types that 4.2 already has and whose meaning
is the same at 7.1 are reused, not copied: `select::{V, Dst, Uniform}`,
`alloc::AllocError`, `verify::VerifyError`,
`isa::{Cond, Pf, Uf, BranchCond, Msfign, Branch}`, and `chain::{Prim, Target}`.

**`vop71.rs`** (scaffold71): the 7.1 virtual operations.

```rust
pub use super::isa::{BranchCond, Cond, Msfign, Pf, Uf};
pub use super::isa71::{AddOp71, MulOp71};
pub use super::select::{Dst, Uniform, V};

/// A fragment shader's payload register at 7.1 (nir_to_vir.c:4852-4858).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Payload { W, CentroidW, Z }
impl Payload {
    /// rf3, rf1, rf2 (rf0, rf1, rf2 on 4.2).
    pub const fn rf(self) -> u8;
}

/// The register `ldunif`, `ldunifa` and `ldvary`'s C coefficient write
/// implicitly at 7.1 (qpu_instr.c:1031-1040).
pub const RF_IMPLICIT: u8 = 0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Operand71 {
    V(V), Abs(V), Lo(V), Hi(V),
    SmallImm(u8),
    Payload(Payload),
    /// rf0: the C coefficient of the last `ldvary` (4.2's `Operand::R5`).
    VaryC,
    None,
}
impl Operand71 { pub fn vreg(&self) -> Option<V>; }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlagOp71 { Add(AddOp71), Mul(MulOp71) }

/// One unit of work for sched71: select.rs's `VOp` with 7.1 operations and
/// operands. Variant for variant, the meaning is the same as `VOp`'s.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VOp71 {
    Add { op: AddOp71, dst: Dst, a: Operand71, b: Operand71 },
    Mul { op: MulOp71, dst: Dst, a: Operand71, b: Operand71 },
    Ldunif { dst: V, u: Uniform },
    Ldvary { dst: V },
    Thrsw,
    Flags { unit: FlagOp71, a: Operand71, b: Operand71, pf: Pf, uf: Uf },
    CondMov { dst: V, src: Operand71, cond: Cond },
    Setmsf { src: Operand71, cond: Cond },
    Wrtmuc { u: Uniform },
    Ldtmu { dst: V },
    LoopHead,
    Copy { dst: V, src: Operand71 },
    BackEdge { cond: BranchCond, msfign: Msfign },
    FwdBranch { cond: BranchCond, msfign: Msfign, label: u32 },
    Label(u32),
}
impl VOp71 {
    /// As `VOp::def` (CondMov and Copy are uses of `dst`, not definitions).
    pub fn def(&self) -> Option<V>;
    /// As `VOp::uses`.
    pub fn uses(&self) -> [Operand71; 2];
}

/// `select::Selected` at 7.1: the operations, and the same facts the shader
/// record needs, field for field.
#[derive(Clone, Copy, Debug)]
pub struct Selected71<'a> {
    pub ops: &'a [VOp71],
    pub fragment: bool,
    pub vregs: u32,
    pub vattr_sizes: [u8; 16],
    pub vpm_output_rows: u32,
    pub uses_vid: bool,
    pub uses_iid: bool,
    pub uses_center_w: bool,
    pub implicit_point_varyings: bool,
    pub writes_z: bool,
    pub disable_ez: bool,
}
```

**`select71.rs`** (select71): `Selected` lowered to `Selected71`.

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Select71Error {
    /// A 4.2 operation or construct with no 7.1 form, by name.
    NotOn71(&'static str),
    /// A vertex or coordinate shader with no viewport-scaled x/y pair to
    /// rewrite (a defect: select.rs always emits one).
    ViewportXyMissing,
    OutOfScratch,
}
/// The 7.1 operation with the same Mesa enum value (`AddOp` and `AddOp71`
/// both carry `enum v3d_qpu_add_op` values); `None` for VDWWT, which 7.1
/// lacks.
pub fn add_op71(op: AddOp) -> Option<AddOp71>;
pub fn mul_op71(op: MulOp) -> Option<MulOp71>;
pub fn lower71<'a>(sel: &Selected<'_>, a: &Arena<'a>) -> Result<Selected71<'a>, Select71Error>;
```

`lower71` copies operation for operation, except in five places, each from
Mesa:

1. `Operand::R5` becomes `Operand71::VaryC`. At 7.1 `ldvary`'s C lands in rf0,
   not r5 (`nir_to_vir.c:1054-1057`).
2. `Operand::Payload(0)`, `(1)` and `(2)` become `Payload::W` (rf3),
   `CentroidW` (rf1) and `Z` (rf2) (`nir_to_vir.c:4852-4858`;
   `vir_register_allocate.c:1103-1112`).
3. The viewport-scaled x and y: 4.2 stores `ftoiz(ffloor(t))` because of the
   pre-4.3 ".8 fixed point" quirk, and 7.1 stores
   `f2i32(fround_even(t))` (`v3d_nir_lower_io.c:529-543`). Mesa emits that as
   one `FTOIN` (`nir_to_vir.c:1452-1458`). The pair is recognised exactly as
   `select.rs` builds it (`select.rs:2564-2571`): an `Ffloor` read only by an
   `Ftoiz` read only by a `Stvpmv`, where the `Ffloor`'s operand is the
   `Fmul` chain from an `Ldunif` whose contents are
   `QUNIFORM_VIEWPORT_X_SCALE` or `_Y_SCALE`. No user code can load those
   contents, so the match cannot catch a shader's own `int(floor(x))`. The
   pair becomes one `Add { op: Ftoin }`. A vertex or coordinate shader
   without both pairs is `ViewportXyMissing`.
4. The final `vpmwt` of a vertex or coordinate shader (`select.rs:2594-2595`)
   is dropped. Mesa emits it for GFXH-1684 at `ver == 42` only
   (`nir_to_vir.c:2164-2178`; section (f)).
5. An operation that reads two small immediates becomes an `Ldunif` of
   `Uniform::constant(value)` into a new virtual register for the second one.
   At 7.1 one instruction carries one small immediate
   (`qpu_validate.c:213-217`; `qpu_merge_raddrs`, `qpu_schedule.c:1013-1032`).
   4.2 allowed two when they were equal, because both shared raddr_b
   (`qpu_schedule.c:1042-1053`).

The SFU operations (`Recip` .. `Rsqrt2`) map one to one. See (c).

**`alloc71.rs`** (alloc71): see (b) for the rules.

```rust
pub use super::alloc::AllocError;
/// The first register-file register handed out (rf3 on 4.2).
pub const RF_FIRST71: u8 = 4;
/// One past the last register a thread has: 32 at 4 threads, 64 at 1 or 2,
/// as on 4.2.
pub const fn rf_end71(threads: u32) -> Option<u8>;
#[derive(Clone, Copy, Debug)]
pub struct Allocation71<'a> {
    /// Indexed by virtual register: its register-file address.
    /// RF_IMPLICIT (rf0) only for an `Ldunif` destination, which sched71
    /// then writes as a plain `ldunif`.
    pub reg: &'a [u8],
    /// One past the highest register used (RF_FIRST71 when none).
    pub rf_end: u8,
}
pub fn allocate71<'a>(sel: &Selected71<'_>, threads: u32, a: &Arena<'a>) -> Result<Allocation71<'a>, AllocError>;
```

**`sched71.rs`** (sched71): straight-line packing into `Instr71` words, with
the thread switches, the thread end and the uniform stream. It calls
`sched71_cf` for the five control-flow operations.

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sched71Error {
    /// Operation `.0` does not encode even alone in a word (codec71's refusal).
    NotEncodable(usize, EncodeError),
    CondWithoutFlags(usize),
    /// Operation `.0` reads `VaryC` with no `ldvary` before it.
    VaryCWithoutLdvary(usize),
    LdtmuWithoutLookup(usize),
    NotPlaceable(usize),
    /// A LoopHead / BackEdge / FwdBranch / Label that sched71_cf refused or
    /// that it does not place yet.
    ControlFlow(usize),
    OutOfScratch,
}
#[derive(Clone, Copy, Debug)]
pub struct Scheduled71<'a> {
    pub instrs: &'a [Instr71],
    /// The uniform stream, in the order the program consumes it.
    pub uniforms: &'a [Uniform],
    pub back_edges: &'a [(u32, u32)],
    pub fwd_branches: &'a [(u32, u32)],
}
pub fn schedule71<'a>(sel: &Selected71<'_>, al: &Allocation71<'_>, a: &Arena<'a>) -> Result<Scheduled71<'a>, Sched71Error>;

/// The word being filled, and the packer sched71_cf works on (crate-private;
/// sched71 may add to it, sched71_cf uses only what is listed).
pub(super) struct Word71 { pub alu: Alu71, pub uniform: Option<Uniform>, pub add_used: bool, pub mul_used: bool, pub sig_addr_used: bool }
pub(super) struct Packer71<'a> { /* words, branches, scoreboard */ }
impl<'a> Packer71<'a> {
    /// Instructions placed so far.
    pub(super) fn len(&self) -> usize;
    /// The next operation starts a new word; nothing merges across.
    pub(super) fn fence(&mut self);
    /// Append a branch word; its index.
    pub(super) fn push_branch(&mut self, br: Branch) -> usize;
    pub(super) fn set_branch(&mut self, at: usize, br: Branch);
    /// The ALU word at `at` (`None` for a branch), for the delay-slot checks.
    pub(super) fn word(&self, at: usize) -> Option<&Word71>;
    /// Uniforms consumed by instructions `from..to` (a branch's `ub` delta).
    pub(super) fn uniforms_between(&self, from: usize, to: usize) -> u32;
}
```

**`sched71_cf.rs`** (sched71-cf): branches, counted loops and their delay
slots.

```rust
pub(super) fn place_cf(p: &mut Packer71<'_>, i: usize, op: &VOp71) -> Result<(), Sched71Error>;
/// Resolve every branch's offset and uniform delta; (back_edges, fwd_branches).
pub(super) fn finish_cf<'a>(p: &mut Packer71<'_>, a: &Arena<'a>) -> Result<(&'a [(u32, u32)], &'a [(u32, u32)]), Sched71Error>;
/// verify.rs `branches_are_structured` over Instr71, the same rules
/// (CLAUDE.md: the compiler never emits a program that does not terminate).
pub fn branches_are_structured71(prog: &[Instr71], back_edges: &[(u32, u32)], fwd: &[(u32, u32)]) -> Result<(), VerifyError>;
```

The scaffold's `place_cf` returns `Err(Sched71Error::ControlFlow(i))`, so the
straight-line shapes (pass-through, uniform, texture) do not wait for
sched71-cf. Only the `if` shape needs it, unless if-conversion selected it as
straight-line code (`v3d-backend.md`, "Control flow, step 1").

**`verify71.rs`** (merged): `verify71(prog: &[Instr71], single_seg: bool)`,
`registers_fit_threads71(prog: &[Instr71], threads: u32)`. Nothing new.

**`chain71.rs`** (chain71): the three stages and the shader-record facts.

```rust
#[derive(Clone, Copy, Debug)]
pub struct Code71<'a> {
    pub words: &'a [u64],
    pub instrs: &'a [Instr71],
    pub uniforms: &'a [Uniform],
    pub back_edges: &'a [(u32, u32)],
    pub fwd_branches: &'a [(u32, u32)],
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum V3d71Error {
    NotYet(&'static str),
    /// prepare, select and the shared refusals (chain.rs's error).
    Chain(V3dError),
    Lower(Select71Error),
    Alloc(AllocError),
    Sched(Sched71Error),
    /// Instruction `.0` did not encode (codec71).
    Encode(usize, EncodeError),
    Verify(VerifyError),
    OutOfScratch,
}
/// One stage: lower71, allocate71 at 4 then 2 threads (4 only when
/// `select::tmu_input_writes` <= 4, as chain.rs `build`), schedule71,
/// encode71 of every word, verify71, registers_fit_threads71,
/// branches_are_structured71.
pub fn build71<'a>(sel: &Selected<'_>, single_seg: bool, a: &Arena<'a>) -> Result<(Code71<'a>, u32), V3d71Error>;
/// chain.rs's FsProg / VsProg / V3dProgram field for field, `code: Code71`.
pub struct FsProg71<'a> { /* ... */ }
pub struct VsProg71<'a> { /* ... */ }
pub struct V3dProgram71<'a> { pub fs: FsProg71<'a>, pub vs: VsProg71<'a>, pub cs: VsProg71<'a>, pub attributes: &'a [(u8, Option<u8>)] }
pub fn compile_program71<'a>(vs: &'a ir::Module<'a>, fs: &'a ir::Module<'a>, prim: Prim, target: Target, a: &'a Arena<'a>) -> Result<V3dProgram71<'a>, V3d71Error>;
pub fn compile_program71_keyed<'a>(vs: &'a ir::Module<'a>, fs: &'a ir::Module<'a>, prim: Prim, target: Target, vs_key: LodKey, fs_key: LodKey, a: &'a Arena<'a>) -> Result<V3dProgram71<'a>, V3d71Error>;
/// The generation as the chain's parameter.
pub enum AnyV3dProgram<'a> { V42(V3dProgram<'a>), V71(V3dProgram71<'a>) }
pub fn compile_program_for<'a>(gen: Gen, vs: &'a ir::Module<'a>, fs: &'a ir::Module<'a>, prim: Prim, target: Target, vs_key: LodKey, fs_key: LodKey, a: &'a Arena<'a>) -> Result<AnyV3dProgram<'a>, V3d71Error>;
```

`compile_program_for(Gen::V42, ..)` is `chain::compile_program_keyed`
unchanged; it maps its error to `V3d71Error::Chain`. `Target` at 7.1 is
`vpm_size: 65536`, the 64 KiB that H2 gave v3d_shc (`tools/v3d/README.md`,
"H2"). The Pi 5's own `IDENT1.VPM_SIZE` is unmeasured (U2/D2).

Until `chain-hook` lands, `compile_program71_keyed` has its own copy of
`compile_program_keyed`'s orchestration, built from public functions only:
`powervr::unroll::unroll`, `prepare_fragment_with`, `fragment_inputs`,
`prepare_vertex_with`, `select_fragment`, `select_vertex`, and the
attribute-record and flag code transcribed. It retries the `Pressure` levels
the way chain.rs's `under_pressure` does. It also has a copy of `vpm_fields`
(`vir.c:861-937`, which has no version branch). Each copy carries the comment
"chain-hook replaces this". `chain-hook` then adds:

```rust
// chain.rs
pub(super) fn compile_with<'a, C, E: From<V3dError>>(
    vs: &'a ir::Module<'a>, fs: &'a ir::Module<'a>, prim: Prim, target: Target,
    vs_key: LodKey, fs_key: LodKey,
    build: &dyn Fn(&Selected<'_>, bool, &Arena<'a>) -> Result<(C, u32), E>,
    does_not_fit: &dyn Fn(&E) -> bool,
    a: &'a Arena<'a>,
) -> Result<V3dProgram<'a, C>, E>;
```

`compile_program_keyed` then calls `compile_with` with its own `build`. This
is the same code in the same order, so 4.2 words do not move, and `pins42`
checks it. `chain71` drops its copies, and
`pub type V3dProgram71<'a> = chain::V3dProgram<'a, Code71<'a>>` (the same
field names, so users do not change).

## (b) Register allocation without accumulators

### What 4.2 had, and what 7.1 removes

At 4.2, Mesa allocates over r0..r5 plus rf0..rf63. r5 is a class of its own,
kept for `ldunif` (`vir_register_allocate.c:30`, `:1057-1062`,
`:1291-1312`). Accumulators are favoured for short-lived values because they
do not use a read port (`:876-916`). They are invalidated at a thread switch
(`:1340-1348`). Our `alloc.rs` follows this. It uses r0..r4 for values that
can live in an accumulator. SFU results, `ldvpmv_in` results and values live
across a THRSW are physical (`alloc.rs:30-36`). r5 holds a short-lived uniform
load, which the scheduler then writes as a plain `ldunif`
(`alloc.rs:258-281`, `sched.rs:317-326`).

At 7.1 `has_accumulators = ver < 71` (`common/v3d_device_info.c:76`). Mesa's
allocator has no accumulator nodes. It has one fixed node for rf0, the
register written implicitly (`vir_register_allocate.c:32-33`, `:77-82`,
`:1454-1460`). Every class is "phys" (`:67-75`). So `alloc71` has one
register kind. The 4.2 class rules (SFU, LDVPM, across a THRSW) say nothing at
7.1 because everything is already in the register file. Mesa keeps the SFU and
LDVPM rules (`:1182-1210`), but at 7.1 they choose among one class only.

### The rules alloc71 keeps

- **Linear scan in program order**, as `alloc.rs` does (`alloc.rs:8-12`):
  sched71 also merges without reordering, so "last read" is exact.
- **The register file per thread** is 4.2's: rf0..rf31 at 4 threads,
  rf0..rf63 at 1 or 2. Mesa's register sets have no version branch here
  (`vir_register_allocate.c:1054-1072`, `:1428-1435`), and `verify71`'s
  `registers_fit_threads71` checks it. The thread count is 4, else 2, never 1,
  as in `chain.rs` `build`.
- **rf1, rf2, rf3 are never handed out**, in any stage. At 7.1 they hold the
  fragment payload: centroid W, Z, W (`nir_to_vir.c:4852-4858`;
  `reg_is_payload`, `vir_register_allocate.c:1108-1112`). Mesa starts at rf4
  "to prevent TLB writes from using RF2-3" (`:1417-1421`). In the thread end's
  three instructions nothing may read rf2/rf3 or write them
  (`qpu_schedule.c:2043-2056`; `qpu_validate.c:373-419`). With
  `RF_FIRST71 = 4`, alloc71 never assigns them, so that rule can only fire on
  a payload read, which sched71 checks. This mirrors `alloc.rs`, which never
  hands out the 4.2 payload rf0..rf2 in any stage (`alloc.rs:26-28`). The
  cost: 4.2 had 5 accumulators + rf3..rf31 = 34 registers at 4 threads; 7.1
  gets rf4..rf31 = 28, plus rf0 for loads. Mesa can use rf1..rf3 in vertex
  and coordinate shaders, where they hold no payload
  (`vir_register_allocate.c:1103-1114`, `:990-1004` wraps round to them). If
  the corpus shows a `DoesNotFit` that this would fix, alloc71 widens to them
  in those stages (an open item, not this cut).
- **rf0 is the implicit register.** `ldunif`, `ldunifa` and `ldvary`'s C
  coefficient write it without naming it
  (`v3d_qpu_writes_rf0_implicitly`, `qpu_instr.c:1031-1040`; the RF0 column of
  `v3d71_sig_map`, `qpu_pack.c:164-197`). So:
  - no value may live in rf0 across any of these writes. Mesa makes every
    live temp interfere with the rf0 node there
    (`vir_register_allocate.c:1167-1178`) and tracks `ldvary`'s write, which
    has no temp (`:1245-1262`, `:1479-1491`);
  - an `ldvary` destination is never rf0 (`:1314-1322`);
  - Mesa gives rf0 to an `ldunif`/`ldunifa` destination when it can
    (`try_rf0`, `:1323-1336`, `:965-975`): a load into rf0 is a plain
    `ldunif`, which names no register. That keeps the condition field free,
    because `ldunifrf`'s destination is encoded there
    (`vir_register_allocate.c:965-969`; `qpu_schedule.c:2678-2685`). Mesa
    keeps rf0 out of every other choice (`:993-995`) and uses it only as a
    last resort (`:1006-1012`).

  `alloc71` gives rf0 only to an `Ldunif` destination, under exactly the rule
  `alloc.rs` uses for r5 (`alloc.rs:258-281`). The value is read within
  `R5_SPAN` (2) operations. It is in a fragment shader. No `Ldvary`, no
  `VaryC` read and no `Thrsw` falls inside its life. The load is not within
  `R5_AFTER` (4) operations after an `ldvary` or a `VaryC` read (Mesa's
  `ip > last_ldvary_ip + 4`, `:1330-1332`). One value at a time holds rf0. Every
  other value gets rf4 and up, lowest first, as in `alloc.rs`. rf0 is never
  given to an arithmetic result. That is stricter than Mesa's fallback, and
  it means sched71 never has to check the "no rf0 write right after an
  `ldvary`" rule against an ALU write (see (e)).
- **What `r5` / `r0..r5` handling becomes.** `Reg::Acc(0..=4)` disappears,
  and `Reg::Acc(5)` becomes `RF_IMPLICIT`. `Allocation71::reg` is a plain rf
  number. Every "physical" rule of `alloc.rs` (SFU, LDVPM, across a THRSW)
  holds trivially and is dropped. The hoist-aware choice of `alloc.rs`
  (`barrier`, `early`, `*_touch`, `alloc.rs:283-300`) serves `sched.rs`'s
  `hoist`. alloc71 leaves it out until sched71 has a hoist (an open item).

### The read ports: 4.2's two, 7.1's four

At 4.2 an instruction has two register-file read addresses, `raddr_a` and
`raddr_b`, and the four ALU operands choose among them and the accumulators
through a mux. Two operations can share a word only if together they read at
most two registers, or one register and one small immediate, because the
small immediate sits in `raddr_b` (`qpu_merge_raddrs`, 4.2 branch,
`qpu_schedule.c:1034-1107`; `qpu_raddrs_used`, `:980-999`). That limit is why
Mesa and `alloc.rs` favour accumulators (`vir_register_allocate.c:876-879`).

At 7.1 each operand has its own 6-bit address: `raddr_a` (add a), `raddr_b`
(add b), `raddr_c` (mul a), `raddr_d` (mul b) (`qpu_pack.c:87-111`;
`v3d71_qpu_reads_raddr`, `qpu_instr.c:1055-1064`). Merging two operations
checks only the small immediates (`qpu_schedule.c:1013-1032`, and the comment
at `:1348-1352`). What remains:

- **One small immediate per instruction**, flagged per operand: `small_imm_a`
  or `_b` needs an add operation, `_c` or `_d` a mul operation
  (`qpu_validate.c:204-218`; the signals `qpu_pack.c:164-197`, entries 14, 15,
  30, 31). The value table is 4.2's (`qpu_pack.c:239-264`;
  `v3d_qpu_small_imm_pack`, `:279-294`, takes no version). Moving an
  ADD/SUB to the mul unit moves its flag from a/b to c/d
  (`qpu_schedule.c:1161-1171`). `select71` rule 5 removes the two-immediate
  operation.
- **An add operation with fewer than two operands keeps its opcode in
  `raddr_b`**: for example, opcode 188 is LDVPMV_IN at `raddr_b` 0 and RECIP
  .. RSQRT2 at 32..37 (`qpu_pack.c:654-663`). That operand is not available,
  which `codec71::canonical71` already models.
- **FADD/FADDNF and FMIN/FMAX are chosen by the order of the operands**,
  including the unpack and the small-immediate flag
  (`qpu_pack.c:1232-1241`, `:1935-1970`). The encoder swaps the operands to
  say which one is meant. With both operands the same register and unpack,
  FADDNF and FMAX cannot be said, and `encode71` refuses them
  (`codec71.rs:92-105`), as the 4.2 encoder does. sched71 sees
  `NotEncodable`. select71 never produces them, because 4.2 selection does not.

So alloc71 has no read-port pressure to optimise, and sched71 has no port
check. The 4.2 `SchedError::TooManyReads` has no 7.1 twin.

## (c) The SFU as ALU operations

The pre-study says the 4.2 back end reaches the SFU "through magic
addresses" (section 3.3). That is not what ours or Mesa's 4.2 code emits. Mesa
emits `recip`, `rsqrt`, `exp`, `log`, `sin` and `rsqrt2` as add operations on
both versions (`VIR_SFU`, `v3d_compiler.h:1370-1385`, `:1506-1511`;
`nir_to_vir.c:1742-1753`). At 4.2 the add opcode is 186
(`qpu_pack.c:491`); at 7.1 it is 188 with `raddr_b` 32..37
(`qpu_pack.c:658-663`). The magic addresses 19..24 are the legacy form
(`v3d_qpu_instr_is_legacy_sfu`, `qpu_instr.c:768-785`), reserved at 7.1
(`qpu_instr.h:115-120`; `qpu_validate.c:104-115`). Our 4.2 `select.rs` already
issues `VOp::Add { op: Recip .. }` with a register destination
(`select.rs:186-193`), and `verify.rs` says the cut emits no magic SFU write
(`verify.rs:13-15`). Whether any 4.2 corpus word names a magic SFU address
was not measured here. Item `forecast71` counts the 4.2-only constructs per
program; this note does not.

So at 7.1 the SFU operations map one to one (`select71`'s `add_op71`). What
changes is the timing and the slot rules, in sched71:

- **Latency.** Mesa gives an SFU result a latency of 2
  (`instruction_latency`, `qpu_schedule.c:1859-1871`) on both versions. At
  7.1 a read in the next instruction is legal: the hardware stalls
  (`read_stalls`, `qpu_schedule.c:779-786`; the scoreboard,
  `:1663-1669`). `sim71` models the stall (`sim71.rs:66-69`), and `verify71`
  does not refuse it (`verify71.rs:46-48`). sched71's first cut keeps
  `sched.rs`'s rule, readable two instructions later (`sched.rs:40-41`), which
  the Pi 4 ran with. Allowing +1 (one word less where nothing fills the gap)
  is an open item for after the board.
- **Peripheral.** An SFU operation is the restricted peripheral of its
  instruction (`qpu_peripherals`, `qpu_schedule.c:855-893`, through
  `v3d_qpu_uses_sfu`, `qpu_instr.c:759-762`). At 7.1 it shares an
  instruction with no TMU write, WRTMUC, TSY, TLB read or VPM access
  (`qpu_schedule.c:937-962`; `verify71::peripherals_compatible71`).
- **Thread switch.** No SFU operation in a THRSW's delay slots 1 and 2: its
  result would land in the other thread (`qpu_schedule.c:2079-2084`). At 4.2
  Mesa refuses only the legacy form there. `sched.rs` already keeps `recip`
  out (`sched.rs:46-50`), so the rule is the same in sched71.
- **No r4.** The 4.2 rules about R4 near an SFU write
  (`qpu_validate.c:316-328`; `writes_too_soon_after_write`,
  `qpu_schedule.c:705-711`) count only magic SFU writes and r4, so they never
  fire at 7.1.

The SFU precision verdict of `v3d-backend.md` ("SFU precision") carries over
unchanged: board precision unmeasured, equivalence with Mesa by construction.

## (d) ldvary, ldunif/ldunifrf, ldunifa, the TMU and thrsw at 7.1

| | 4.2 (what `select.rs`/`sched.rs` emit) | 7.1 | Source |
|---|---|---|---|
| `ldvary` | value into its named destination, readable the next instruction; C into r5, readable two instructions later | the same destination rule; C into **rf0**, a delayed write, readable two instructions later; nothing else may write rf0 in the next instruction except another `ldvary` | sig writes address: `qpu_instr.c:1095-1107`; C register: `nir_to_vir.c:1054-1057`; `reads_too_soon` raddr 0, `qpu_schedule.c:623-635`; `writes_too_soon_after_write`, `:713-727`; rf0 delayed by one tick, `:1739-1742` |
| `ldvary` and a THRSW | not in either delay slot | allowed in slot 1, not in slot 2 | `qpu_validate.c:298-306`; `qpu_schedule.c:1547-1558`, `:2086-2091`, `:2687-2695` |
| `ldunif` | the next uniform into r5 | into **rf0** | `qpu_instr.c:1031-1040`; `qpu_pack.c:164-197` |
| `ldunifrf` | into a named register; the destination takes the condition field | the same | `vir_to_qpu.c:329-345` (rf0 at 7.1 stays `ldunif`, `:338`) |
| `ldunif` after `ldvary` | refused in the next instruction | the same | `qpu_validate.c:220-227` |
| `ldunifa`, `ldunifarf` | not emitted | not emitted. If ever: rf0 like `ldunif` (`qpu_instr.c:1034-1035`), and the unifa write keeps 3 instructions from a thread switch (`qpu_schedule.c:2093-2108`) | |
| TMU writes | magic 32..46 and 11..13 | the same addresses, not reserved at 7.1 | `qpu_instr.h:107-109,121-135`; `qpu_validate.c:104-115`; `v3d_tex.c` has no version branch |
| `ldtmu` | names its destination (an accumulator or rf) | names an rf destination; can share its signal with `wrtmuc` (map entries 26, 27); is not counted among the "one of" peripherals | `qpu_pack.c:164-197`; `qpu_validate.c:333-344` (`ver == 42 ? ldtmu : 0`); `qpu_schedule.c:961-965` (one TMU read) |
| `thrsw` | two delay slots; accumulators and flags invalid after; FS ends with the last-THRSW pair, VS/CS single segment | the same, minus the accumulators, plus the rf0 rule below | `qpu_schedule.c:485-501`; `nir_to_vir.c:4997-5034` (no version test; "spawn the non-fragment shaders already in the post-last-THRSW state", `:5013-5015`); `vir.c:1041` |
| rf0 across a switch | no rule | if the most recent rf0 write came from an `ldunif`/`ldunifa`/`ldvary` whose instruction also had another register-file signal write, and the section's final instruction has a register-file signal write, rf0 is undefined after the switch | `qpu_schedule.c:1704-1745`, `:2110-2120` |

What each component does with this:

- **select71** turns `Operand::R5` into `VaryC`. Nothing else in the varying
  sequence changes: still `ldvary * W + C` for a smooth varying, W from
  `Payload::W` (rf3).
- **alloc71** gives rf0 only to a short `Ldunif` with no `ldvary` and no
  `VaryC` read inside its life ((b)). So a `VaryC` read always sees the C of
  the last `ldvary`.
- **sched71** places a `VaryC` read at least two instructions after its
  `ldvary` (4.2's r5 rule, `sched.rs:1149-1150`, with rf0). Its first cut
  writes an `Ldunif` into rf0 as plain `ldunif`, and any other as `ldunifrf`
  (`sched.rs:317-337`). It never puts an `ldunif` (rf0) into the instruction
  after an `ldvary`. For the thread switch it applies Mesa's rf0 rule as Mesa
  does: no register-file signal write in slot 2 when the conflict flag is set
  or the implicit rf0 write lands in that instruction. That holds whether or
  not anything reads rf0 afterwards.
- **TMU sequences** are 4.2's: the same writes, the same retiring S write, the
  same `wrtmuc` P0/P1 uniform words (`v3d-backend.md`, "Textures: the P0/P1
  uniform words"; `v3d_tex.c` has no version branch). The first cut does not
  pair a new look-up with an earlier `ldtmu`. Mesa does that only when the
  `ldtmu` cannot stall (`qpu_schedule.c:1561-1575`). An `ldtmu` with no thread
  switch before it is refused, as `sim.rs`'s `NO_SWITCH_HANG` does at 4.2
  (`sim71.rs:86-88`). It is unmeasured at 7.1.

## (e) The scheduler's 7.1 hazards

Every `ver >= 71` / `ver == 42` / `ver < 71` / `has_accumulators` branch of
`qpu_schedule.c` and `qpu_validate.c`, with the component that carries it.
"verify71" means `verify71.rs` already checks the rule. sched71 must never
produce a word that breaks it, and `build71` runs `verify71` on every program.

| # | Rule at 7.1 | Mesa | Carried by |
|---|---|---|---|
| 1 | Dependencies through `raddr` per operand, small-immediate operands excluded | `process_raddr_deps`, `qpu_schedule.c:179-185`, `:343-375` | sched71 (in-order: an operand is ready one instruction after its write) |
| 2 | An implicit rf0 write is a write of rf0 | `qpu_schedule.c:479-480` | sched71, alloc71 |
| 3 | No read of rf0 (C) within one instruction of an `ldvary` | `reads_too_soon`, `qpu_schedule.c:623-635`, called for each raddr at `:653-686` | sched71 |
| 4 | No rf0 write (explicit, or implicit other than `ldvary`) in the instruction after an `ldvary` | `qpu_schedule.c:713-727` | sched71 (alloc71 makes the explicit case impossible) |
| 5 | SFU result: latency 2, or a stall at +1 | `qpu_schedule.c:1869-1871`, `:779-786` | sched71 (2, see (c)) |
| 6 | One restricted peripheral (TMU write, WRTMUC, TSY, TLB read, SFU, VPM read, VPM write), except WRTMUC with a non-TMUC TMU write; one TMU read; one TLB access | `qpu_compatible_peripheral_access`, `qpu_schedule.c:937-975` (4.2's rules `:910-935`) | sched71; verify71 (`peripherals_compatible71`) |
| 7 | At most one small immediate; a/b only with an add operation, c/d only with a mul operation | `qpu_schedule.c:1013-1032`; `qpu_validate.c:204-218` | select71 rule 5, sched71; verify71 |
| 8 | No read-port limit | `qpu_schedule.c:1348-1352` | (nothing to check) |
| 9 | MOV/FMOV may go to the add unit, ADD/SUB to the mul unit | `can_do_mul_as_add`, `qpu_schedule.c:1175-1183`; `qpu_convert_mul_to_add`, `:1200-1233`; `:1110-1171` | sched71 (optional: a mul `Mov` takes a free add unit) |
| 10 | One signal that writes an address per instruction | `qpu_schedule.c:1372-1374` | sched71 (`sig_addr_used`), codec71 |
| 11 | The signal combinations of `v3d71_sig_map`: no LDVPM, no ROT; LDTMU+WRTMUC; LDUNIF with LDTMU or LDVARY (both into rf0) | `qpu_pack.c:164-197` | codec71 (refuses the rest), sched71 |
| 12 | THRSW delay slots: no SFU in slot 1 or 2 | `qpu_schedule.c:2079-2084`; `qpu_validate.c:290-297` (legacy form only) | sched71 |
| 13 | THRSW delay slots: `ldvary` not in slot 2 (allowed in slot 1) | `qpu_schedule.c:2086-2091`, `:1547-1558`; `qpu_validate.c:298-306` | sched71; verify71 |
| 14 | THRSW slot 2: no register-file signal write when the rf0 flops conflict is set or the implicit rf0 write lands there | `qpu_schedule.c:1692-1745`, `:2110-2120` | sched71 |
| 15 | After a THRSW, as 4.2: no THRSW, TLB, branch, TMU write, WRTMUC, TMU wait, MULTOP, flag write or BARRIERID in its slots; no accumulator rule | `qpu_schedule.c:2132-2213` (`v3d_qpu_writes_accum` is false at 7.1, `qpu_instr.c:1008-1012`) | sched71 |
| 16 | Thread end, slot 0: no add or mul register-file write | `qpu_schedule.c:2018-2027`; `qpu_validate.c:373-408` | sched71 (merge-back of the final THRSW); verify71 |
| 17 | Thread end, slots 0..2: no read of rf2/rf3, no write of rf2/rf3 (ALU or signal) | `qpu_schedule.c:2043-2056`; `qpu_validate.c:380-419` | sched71 (W is rf3: no `Payload::W` read there); verify71 |
| 18 | Thread end: register-file writes in slots 1 and 2 are allowed (4.2: none in any slot) | `qpu_schedule.c:2003-2016` (4.2 only); `qpu_validate.c:374-419` | sched71 (more merge-back than `sched.rs`'s `ok_in_thrend`, `sched.rs:466`) |
| 19 | Thread end: no uniform after slot 0, no `ldvary`, no TLB-Z write in slot 2; no VPM-wait rule; no GFXH-1625 rule | `qpu_schedule.c:1979-2001` | sched71 |
| 20 | Branch after SETMSF: any `msfign`/condition at 7.1 | `is_safe_msf_branch`, `qpu_schedule.c:2430-2438` | sched71-cf (`sched.rs`'s `msf_safe` becomes always true) |
| 21 | Branch: three delay slots, nothing in them that `sched.rs` refuses; the 4.2 branch rules are unchanged otherwise | `qpu_schedule.c:2415-2514`; `qpu_validate.c:156-160`, `:349-352` | sched71-cf; verify71 |
| 22 | `ldvary` pipelining (an `ldvary` moved one instruction back): its rf0 write moves with it | `fixup_pipelined_ldvary`, `qpu_schedule.c:2613-2726` (7.1 at `:2687-2724`) | sched71, if it ports `sched.rs`'s `ldvary_back` |
| 23 | Reserved magic write addresses 0..4, 10, 14..15, 19..31, 47..63 | `qpu_validate.c:104-115`, `:309-314` | codec71 (`magic_waddr_name`), verify71 |
| 24 | The 4.2 accumulator rules (R4 after SFU, r5 after `ldvary`, mux deps) do not exist | `qpu_schedule.c:157-174`, `:601-620`, `:699-711`; `qpu_validate.c:316-328` | (nothing) |

sched71 owns rows 1-19 and 22. sched71-cf owns rows 20-21. Its `place_cf`
also keeps `sched.rs`'s structural rules (`LoopHead`/`Label` as word
boundaries, REL branches with `ub` moving the uniform stream,
`v3d-backend.md` "Branch delay slots").

What sched71's first cut leaves out, so it stays reviewable: `sched.rs`'s
`hoist` (`sched.rs:678`), `ldvary_back` (`:751`), `order.rs`'s reorder
pre-pass, and the +1 SFU read. Each one shortens programs without changing
correctness. They are listed as open items so the corpus counts against Mesa
can be read with them in mind.

## (f) Which 4.2 errata do not apply at 7.1

Mesa 26.2.1 has no 7.1-only erratum workaround (pre-study U9, answered
2026-11-11). For the compiler, these are the 4.2 rules that stop at 7.1:

- **GFXH-1684**, "VPM writes need to be complete by the end of the shader":
  Mesa ends a vertex and a geometry shader with `vpmwt` only when
  `ver == 42` (`nir_to_vir.c:2164-2178`). Its thread-end check "no VPM wait
  in a thread-end slot" is also `ver == 42` only
  (`qpu_schedule.c:1990-1991`). So at 7.1 select71 drops the `vpmwt` that
  `select.rs` appends (`select.rs:2594-2595`), and sched71 does not keep VPM
  waits out of the thread end. Why it does not apply: Mesa conditions it on
  the 4.2 version and has no 7.1 counterpart. The note takes that as "fixed
  in 7.x" from Mesa alone; no Broadcom document was read. `VPMWT` still
  encodes at 7.1 (`qpu_pack.c:642`), so keeping it would be legal and would
  cost a word. Dropping it matches Mesa's 7.1 output, which is what the shape
  comparison measures.
- **GFXH-1625**, "TMUWT not allowed in the final instruction": `ver == 42`
  only, in the scheduler (`qpu_schedule.c:1997-2001`) and the validator
  (`qpu_validate.c:422-426`). At 7.1 a TMUWT may sit in thread-end slot 2.
  `sched.rs`'s rule (`sched.rs:476`) is not carried into sched71.
  `verify71` already leaves it out (`verify71.rs:33-34`).
- **GFXH-1633** (LDUNIF next to LDUNIFA): below 4.2 only
  (`qpu_validate.c:229-248`). Not a rule at 4.2 or at 7.1.
- **The pre-4.3 ".8 fixed point" quirk** of the scaled viewport x/y (not a
  GFXH label): `ftoiz(ffloor())` at `ver == 42`, `ftoin` otherwise
  (`v3d_nir_lower_io.c:529-543`, `nir_to_vir.c:1452-1458`). This is select71
  rule 3.
- **The 4.2 thread-end register rules**: no register-file write in any
  thread-end slot, no read of rf0..rf2 (`qpu_schedule.c:2003-2041`;
  `qpu_validate.c:374-419`). At 7.1 they become rows 16-18 of (e).

These stay at 7.1 because Mesa applies them with no version condition. The
compiler-relevant entries of the U9 list:

- **GFXH-1602**, attribute 0 active when builtins are read
  (`nir_to_vir.c:2908-2911`): the chain's `driver_row` (`chain.rs:323-333`)
  and the driver's dummy record carry it, unchanged.
- **GFXH-1744**, a VCM cache of at least 2 batches (`vir.c:927`, `:2964`):
  `vpm_fields`' `clamp(2, 4)` (`chain.rs:342`), unchanged.
- **GFXH-930**, at least one attribute (`gallium/drivers/v3d/v3dx_draw.c:749`,
  `:824-830`, `:850-853`): driver side, P9.

The rest of U9 (GFXH-1461, 1918, 1689, 1742, 1559, HW-2726,
GFXH-1895/1888/1897) concerns packets and the driver. It belongs to P9 and
P10, not to this back end.

One 4.2 rule of our own stays at 7.1 until measured: no `ldtmu` without a
thread switch before it (the Pi 4 hang, `sim.rs` `NO_SWITCH_HANG`). It is
unmeasured on the Pi 5, and Mesa always switches first when it is threaded.

## (g) The test plan

Run locally only with the feature sets in each test's header (CLAUDE.md). The
7.1 rows join `tools/check.sh` only after #341 and #362 release it.

### The four P8 shapes (chain71: `tests/v3d71_shapes.rs`)

The shapes are the ones `shapes71-harness` writes in `tests/v3d71_common/`:
sources, inputs, expected outputs and a codec71/sim71 runner. They are the
programs the Pi 4 first drew with our compiler (`v3d-backend.md`, status line;
`tools/v3d/tri-shaders/`), plus a texture:

1. **pass-through**: `passthrough.vert` + `interp_color.frag`;
2. **uniform**: `utri`, a uniform-scaled triangle;
3. **`if`**: `tri_if`, a branch on a varying;
4. **texture**: a sampled 2D texture (the harness picks the source and its
   texels).

For each shape, `compile_program71` must succeed. Then, for each of fs, vs
and cs:

- **verify at 7.1**: `verify71(instrs, single_seg)` and
  `registers_fit_threads71(instrs, threads)` pass. `build71` already ran them;
  the test asserts them again on the returned words.
- **codec71 round trip**: every word is `decode71` and `encode71` back to the
  same bits, and `decode71(w)` equals the scheduled `Instr71` under
  `same_instr71`.
- **sim71**: `sim71::run71` on the words and the uniform stream, with the
  harness inputs, gives the harness's expected outputs. Those are the f16 TLB
  colour and the discards, and for the VS/CS the scaled x/y (round-to-even at
  7.1, rule 3), Zs, 1/Wc and the clip position. The tolerance is the 4.2
  file's: bit for bit, except where an SFU ran (`v3d-backend.md`, "SFU
  precision").
- **Mesa's 7.1 output** for the same GLSL: the reference that `mesa71-ref`
  keeps in `tests/v3d71_ref/` (`tests/v3d71_mesa_shapes.rs` checks the
  reference itself) runs in sim71 with the same inputs and must agree with
  ours on the same outputs. Both instruction counts are printed side by side,
  not pinned against each other: our packer is not Mesa's list scheduler.
  Regenerating the reference needs `V3D_SHC` (or `MESA_V3D_TREE`'s
  `build-v3d`). A shape whose reference is missing is reported as a named gap
  (`gap: V3D_SHC`) and fails the test. It never passes silently.
- **4.2 unchanged**: the same shapes through `chain::compile_program` give
  the words they gave before P8. `pins42` checks this on the whole corpus.

Negative controls, as the 4.2 files have: a mutant of each lowering (the
`ldvary` C read placed one instruction early; W read from rf0 instead of rf3;
the viewport pair left as `ffloor`+`ftoiz`) must fail sim71 or verify71.

### The corpus (corpus71: `tests/v3d71_online_corpus.rs`)

- **The es100 corpus the 4.2 back end is gated on**: the sources and pairs of
  `tests/v3d_online_corpus.rs` (`tools/shaders/`, 324 sources, es100
  included). Each pair goes through `compile_program71`. The test pins the
  count that compiles at 7.1 beside the 4.2 count, and the refusals by name
  in a list file of its own (one `<pair>: <exact message>` line each, kept
  exact the way `known-uncompilable-v3d.txt` is). Every program that compiles
  has passed `verify71` and the codec71 round trip inside `build71`. Item
  `forecast71` gives the expected scope: which programs use which 4.2-only
  construct.
- **`tools/v3d/qpu_corpus_71.txt`'s programs**: the 1090 pairs of
  `qpu_counts_71.tsv`. Per pair, our fs/vs/cs instruction counts beside
  Mesa's 7.1 counts from the tsv; totals and the refusals by name. Counts are
  reported, not required to match.
- **Mesa's 7.1 output for es100** (`mesa71-corpus`: the reference table and
  its test, a named gap without `V3D_SHC`). Where both compile and sim71
  models the program, ours and Mesa's agree on the outputs, as in the 4.2
  three-way check (`tests/v3d_sim_mesa.rs`,
  `our_programs_agree_with_the_ir_and_with_mesa`).

### The 4.2 pins (pins42)

Unchanged, and rerun on the integration tip after `chain-hook`:
`tests/v3d_online_corpus.rs` (324 sources and its known-refusal list), the
`roundtrip` counts (8,893 unique words), the 45 `golden` rows, the `mesa_ref`
digest, `tests/v3d_sim_mesa.rs`, and `tests/es2_report_guard.rs`.

## Order of the items

`scaffold71` (all files above, stubs that refuse by name, `mod.rs`) comes
first. Then `select71`, `alloc71`, `sched71` and `sched71-cf` run in parallel
on their own files. `chain71` (`chain71.rs`, `tests/v3d71_shapes.rs`) needs
all four. `corpus71` comes after `chain71`. `chain-hook` waits for #359 to
release `chain.rs`. `pins42` comes after `chain-hook`, and `results-note`
last. `shapes71-harness`, `mesa71-ref`, `mesa71-corpus` and `forecast71`
touch only their own new paths.

## Results

(Empty. Item `results-note` fills this in after `pins42`.)
