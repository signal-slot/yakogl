<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: how a 16384-byte uniform block is read

Plan #359, item `v3d-ubo-note`, the design `ubo-v3d` implements. Line numbers
are at the integration tip `94b733a6` (they drift by a few lines). Mesa lines
are Mesa 26.2.1 (the full tree in the dependency cache,
`mesa-pan/mesa-26.2.1`; the vendored `third_party/` has no v3d compiler).
Mesa is cited, not copied. GC7000 and PowerVR: see
`docs/notes/gc-ubo-const-limit.md` and `docs/notes/ubo-limits.md` (the
per-GPU note of item `ubo-design-note`); this note is the V3D route only.

## Where blocks live today

The GL core does not bind uniform blocks as buffers on the V3D. It flattens
every block (and the default uniforms) of a stage into that stage's one
host array and fills it from the bound ranges at draw time
(`src/gl/state.rs:1258-1261`; the arrays are `p.vs_uniforms` / `p.fs_uniforms`,
`MAX_UNIFORM_DWORDS` = 1536 dwords each, `program.rs:4858-4862`). The V3D
back end reads the host array only (`uniforms_host_only: true`,
`src/v3d/backend.rs:6732`; `draw.rs:3919-3945` skips the upload). The compiler
sees a block member as `LoadUniform { dword_off }` into that array
(`prepare.rs:1375-1385`). Each distinct word becomes one stream entry,
`QUNIFORM_UNIFORM` with the dword index (`select.rs:2100`), and the job
builder copies `words[data]` into the stage's uniform stream, per draw
(`driver/src/v3d/v3d_job.c:426-433`; `v3d_job.c:838-844`).

## The data path per index kind

| index | path today | path for a block past 64 vec4 |
|---|---|---|
| constant | one `ldunif` per word from the stream | same: nothing new in the compiler |
| dynamic, same in every lane | compare-select over every element's stream words (`prepare.rs:1393-1420`), at most 256 components per read | the block in GPU memory, `UBO_ADDR` + index into UNIFA, `ldunifa` |
| dynamic, differs per lane | the same compare-select (correct per lane) | TMU general read at the lane's address |

**Constant index.** Confirmed: it needs no new instruction, only a bigger
stream. `tests/v3d_ubo_large_sim.rs:105-123` reads entries 0, 383, 384 and
1023 of a 1024-vec4 block in the simulator and gets the right dwords (the
stream word index is a `u16`, `SOp::Uniform(u16)`, `uniform_dwords: u16`;
4096 fits, and 12 blocks of 4096 plus 1536 = 50688 still fits 65535). The
cost is one 4-byte stream word per distinct dword read, per stage, per draw,
so a shader that reads all of a 16 KiB block streams 16 KiB. The job's
indirect arena is budgeted at 440 bytes a draw (`backend.rs:60-62`,
`IND_FIXED` 8192); `v3d_cl_reserve` marks overflow and the draw retries in a
new job (`v3d_cl.h:317-330`, `backend.rs:3917`). A draw whose streams exceed a
whole fresh job's indirect arena is refused: that is the `ubo-v3d` step 3
sizing check.

**Dynamic index.** Corrected in the details. The compare-select path is not
limited to 64 vec4 per block, it is limited to 256 *components per read*
(`MAX_SELECTED`, `prepare.rs:1394-1398`): 64 vec4 per *array*. A block of 16
arrays of 64 vec4 (1024 vec4 in all) indexes correctly now
(`v3d_ubo_large_sim.rs:125-145`, not ignored); one array of 65 or 255 vec4
does not (`:147-160`, `#[ignore]`d). An array of 256 vec4 is refused earlier by
the front end: `UniformRow::array_len` is a `u8`
(`src/shaderc/online/reflect.rs:813`, study `uniform-array-len-study`). The
compare-select path needs no address and is right for a lane-divergent
index, so it stays for short arrays (cheap, no staging); the long-array path
below replaces it only past `MAX_SELECTED`.

**Address path (UNIFA).** Confirmed as the design, with one correction: the
compiler does not emit it at all yet. `select.rs`, `prepare.rs` and
`chain.rs` have no `UNIFA` write, no `ldunifa`, no `UBO_ADDR` uniform
(`prepare.rs:1391` and `:1397` only name it); the encoder, listing and
`sim.rs`/`sim71.rs` know the instructions. So the step is a real selection
feature, not a flag. The simulator models it (`sim.rs:113-123`): UNIFA from
lane 0, `ldunifa` auto-increments by 4, the address must lie in
`Env::memory`, and a read within 3 instructions of the write, or a write
within 3 of a thread switch or end, is refused (those are the scheduler
rules, `sched.rs:789-796`, Mesa `qpu_schedule.c:1437-1442`, `2092-2105`).

**Lane-divergent index.** Claim (c) is half right. The simulator does model
a general TMU access, but only as Mesa's spills and fills use it
(`sim.rs:927-990`): a read returns a word an earlier TMU write of the same
run put in its scratch map, and it refuses a read "which nothing wrote"
(`sim.rs:970-974`); it does not read `Env::memory`. So the path needs a
simulator extension (a general read from `Env::memory`, 32-bit, per lane) and
a selection feature (TMUA address = block base + index * stride + member, the
config word `0xffffffff`), plus `ldtmu`. Correct only through the TMU, as
claimed, because UNIFA takes lane 0's address.

## Mesa's choice

Mesa never streams user uniform data for a block. Every `load_ubo` goes
through `ntq_emit_load_unifa` (`nir_to_vir.c:3241`) with an address uniform
`QUNIFORM_UBO_ADDR` (`:3327-3331`, index shifted by 1: slot 0 is the
gallium default block); it falls back to the TMU general read
(`ntq_emit_tmu_general`, `:540-600` for the address base) exactly when the
offset is divergent (`:3257-3259`), or the access is a sub-32-bit dynamic
one (`:3298-3310`), or inside non-uniform control flow with a register-derived
offset (`:3262-3278`). Constant offsets keep the last UNIFA address and skip
the write by dummy `ldunifa`s up to `MAX_UNIFA_SKIP_DISTANCE` 16 bytes
(`:3334-3350`, `v3d_compiler.h:69`). `load_uniform` (the default block) tries
`ldunif` for a constant offset first (`:2820-2840`); only a dynamic one goes
the UNIFA way. The driver resolves `UBO_ADDR` per draw to the buffer's GPU
address plus the offset, and uploads a user-memory constant buffer 0 to GPU
memory first (`src/gallium/drivers/v3d/v3d_uniforms.c:320-338`); Mesa reports
`max_const_buffer0_size` 64 KiB, `max_const_buffers` 16
(`v3d_screen.c:164-165`). So YakoGL's constant-index stream is the cheaper
route for small blocks and a deliberate departure from Mesa; the staged-copy
+ UNIFA route is Mesa's for everything else.

## Size caps, layer by layer

| layer | cap | where |
|---|---|---|
| reported `MAX_UNIFORM_BLOCK_SIZE` | 6144 = `MAX_UNIFORM_DWORDS*4` | `state.rs:1266` |
| reported combined components, both stages | 1536 | `state.rs:1268-1270` |
| the constant | 1536 dwords (all GPUs; C twin `GL_MAX_UNIFORM_DWORDS`) | `context.rs:83`, `src/gl_context.h:41` |
| shader `gl_MaxVertex/FragmentUniformVectors`, link packing | 384 vec4 | `context.rs:89-90`, `lower.rs:12650-12655`, `shaderc/packing.rs:150` |
| GL core, link: a stage's block | `.min(MAX_UNIFORM_DWORDS)` | `program.rs:1869` (`block_dwords`) |
| GL core, program arrays | `calloc(MAX_UNIFORM_DWORDS, 4)`, cap field | `program.rs:4858-4862` |
| GL core, draw | VS and FS block clamped | `draw.rs:3908-3913` |
| GL core, staging buffer | `2*MAX_UNIFORM_DWORDS*4 + 16` bytes | `draw.rs:2314`, `:3976`, `:8404` |
| back end | takes `*_uniform_dwords` as the stream's word count | `backend.rs:3712-3718`, `:3883-3884` |
| job builder | refuses (`bad_field`), does not clamp, a `QUNIFORM_UNIFORM` with `data >= count`; refuses any contents it does not know, so `UBO_ADDR` | `v3d_job.c:426-433`, `:488-491` |
| indirect arena (streams) | 8192 + 440 per draw | `backend.rs:60-68` |
| data arena (staged copies) | 1 MiB, overflow to `spill` (`JOB_SPILLS` pieces) | `backend.rs:75`, `:3445-3475` |
| compiler, compare-select | 256 components per read | `prepare.rs:1394-1398` |
| front end | array length `u8` | `reflect.rs:813` |
| compiler, a load past the block | `UniformOutOfBlock` | `prepare.rs:1380`, `:1405` |

Correction to claim (a): the job builder does not clamp; it refuses, and the
only thing that lowers its `count` is the GL core's clamp
(`draw.rs:3908-3913` feeds `backend.rs:3883`). Lifting the GL caps is enough
for constant reads; the job builder needs no change until the address
contents exists. `ubo-core-stage-block` removes the GL caps.

## The ES 3.0 combined-components formula

`MAX_COMBINED_x_UNIFORM_COMPONENTS = MAX_x_UNIFORM_COMPONENTS +
MAX_x_UNIFORM_BLOCKS * MAX_UNIFORM_BLOCK_SIZE / 4` (ES 3.0 chapter 6,
the implementation-dependent values table). The ES 3.0 minimums, 1024 (vertex) and 896 (fragment)
components, 12 blocks and 16384 bytes, give 50176 and 50048.

The V3D reports 1536 default-block components and 12 blocks per stage
(`state.rs:1250-1251`, `:1262`). After the raise `MAX_UNIFORM_BLOCK_SIZE` is
16384 and the combined values follow from what is reported:
1536 + 12 * 4096 = **50688** for both stages (above the minimums, as the
spec allows: it is a floor). Compute them with one const fn so the three
queries cannot disagree (item `gl-limits-formula`).

Caveat the implementation must settle (decision for `ubo-v3d`, step 1):
blocks are flattened into one per-stage array, so a stage that really used
12 blocks of 16 KiB would need 49152 + 1536 dwords. Take this option: the
per-stage array is sized by the linked program's own block layout, not a
fixed 1536 (`program.rs:4858`), capped at 50688 dwords (fits the `u16`
fields), and the link refuses a stage whose layout exceeds it with a
link error. A fixed 50688-dword array per program per stage (200 KB) is the
fallback and wasteful. Until that lands, the reported combined value must not
exceed what a stage can hold: the `ubo-core-stage-block` cap (4096 dwords)
then bounds the reported combined components too, and the formula's value is
reported only after step 1 completes.

## The ordered steps (each one item, V3D build only, `!report-es2` only)

1. **GL core: per-stage block size** (`ubo-core-stage-block`). `block_dwords`
   (`program.rs:1869`), the arrays (`:4858-4862`), the draw clamps
   (`draw.rs:3908-3913`) and the staging size (`draw.rs:2314`, `:3976`,
   `:8404`) take a per-GPU stage cap, V3D 4096 dwords first, 1536 elsewhere.
   Reported values unchanged. Host-testable.
2. **Compiler: the address route.** A `UBO_ADDR` uniform contents (Mesa's
   value is the quniform code; `v3d_uniforms.c:320`), selection of the UNIFA
   write and `ldunifa` for a uniform-index read of an array past
   `MAX_SELECTED` (`prepare.rs:1394`), honouring the scheduler's 3-instruction
   rules, and the front end's `array_len` (`reflect.rs:813`, study
   `uniform-array-len-study`). The index must be provably uniform
   (derived from uniforms and constants only); anything else keeps
   compare-select, or goes to step 3. Un-ignore the two long-array cases of
   `v3d_ubo_large_sim.rs`, whose test feeds `Env::memory` for `contents`
   `UBO_ADDR`. Host-testable (simulator).
3. **Compiler and simulator: divergent index past 256 components.** Select the
   TMU general read; extend `sim.rs:934-990` to read `Env::memory`. If the
   step is not done, the shader is refused by name (`NotYet`), never wrong.
   Host-testable.
4. **Job builder and back end: stage and address.** `v3d_job.c:488` gets the
   `UBO_ADDR` contents; `V3dJobDraw` gets a GPU address per stage; the back
   end copies the stage's block into the data arena per draw with `stage`
   (`backend.rs:3445`, the same copy-per-draw the GC does, `ubo_copy` /
   `take_uniforms`, `docs/notes/gc-ubo-const-limit.md`) and passes it. A
   struct field added here: grep `tests/` and `src/` for every literal
   (`CLAUDE.md`). Item `v3d-job-uniform-cap` covers the
   constant-index stream sizing at the same time (`v3d_job.c`, indirect
   arena). Host-testable up to the packed job words (`tools/v3d/job_check.c`).
5. **Raise the reported values** (`state.rs:1266-1270`) under the V3D build
   only, after steps 1 and 4 and the sizing decision above; update
   `tests/es3_caps.rs known_gaps`, `tests/es3_limits_v3d.rs`,
   `tests/es2_report_guard.rs`.

## What the host can test, and what needs the Pi 4

Host: every compile and simulator case above (`v3d_ubo_large_sim.rs`), the GL
core caps, the draws through the host back end, the job words
(`job_check.c`), the query values, and dEQP `functional.ubo` on `GPU=v3d`
(API verdicts through the host back end, `tools/cts/README.md` 2b).

Needs the board (#320): that UNIFA reads the staged block with the real
prefetch behaviour (`ldunifa` past the last word of the arena: leave one
more 16-byte slot behind the copy, as the stream does, `v3d_job.c:386-388`),
the per-lane TMU read, and `functional.ubo` pixels on the Pi 4. The simulator is not
the silicon for these.

## Corrections to the earlier study's claims

- (a) Confirmed: constant reads past dword 1536 compile and run in the
  simulator. Corrected: the job builder refuses, it does not clamp
  (`v3d_job.c:426-433`); the only clamp is in the GL core. Also, a
  dynamic read of 16 arrays of 64 vec4 already works; it is a single array
  past 64 vec4 that does not.
- (b) Confirmed in kind (stage the block per draw, read by address), with
  the corrections that the compiler has no UNIFA emission yet, only the
  simulator and encoder do, that the data arena (1 MiB, `spill` beyond it)
  is the staging place, and that Mesa uses this route for every UBO and
  stages constant buffer 0 per draw only when it is user memory.
- (c) Corrected: the simulator models a general TMU read, but only of its
  own scratch writes, not of `Env::memory`; the divergent route needs that
  extension.
