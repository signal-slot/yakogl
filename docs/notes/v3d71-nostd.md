<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 7.1: how the recording path enters the no_std product library

Design note of ROADMAP stage 5 (the Raspberry Pi 5, issue #7). A product
`gpu-v3d71` library (`osal-zephyr,gpu-v3d71,shaderc-online,no-shader-table`,
as zephyr/CMakeLists.txt builds it for the rpi_5 images) carries only
`src/v3d/table71.rs`, which refuses every draw and clear with
`REF_NO_RECORDING` (table71.rs:127-128, 1419-1431). The pure 7.1 modules
are declared `#[cfg(all(feature = "gpu-v3d71", feature = "host-test"))]`
(src/v3d/mod.rs:40-73), and so is table71's recording module `rec`
(table71.rs:110-118, 178-1388). They build `Vec`s, `String`s and `Box`es
and some name `std`, while the product crate is `no_std` with no allocator
(src/lib.rs:31). So the board71 run refused all drawing in gl_test cases 5,
58, 59, 66, 67 and 102.

This note lists every alloc and std use on that path with its bound
(section 1), describes how the 4.2 table records without an allocator
(section 2), compares the two ways in (section 3), and gives the choice
with an ordered split into implementation items (section 4). There is no
code here. Every line number was read at `d10fc6d7`.

## 0. The recording path and the two experiments

table71's host-test path (`mod rec`, table71.rs:178 on) calls `glue71`,
`fill71`, `attr71`, `record71`, `uniform71`, `state` (4.2, already no_std)
and `zskeep71`, and includes `backend71.rs` with `#[path]`
(table71.rs:115-118). Through their imports these reach `admit71`,
`clearplan71`, `draw71`, `ffi71`, `format71`, `ident71`, `limits71`,
`prim71` and `rt71` (admit71.rs:21, fill71.rs:108, glue71.rs:107,
backend71.rs:69). Those 17 files make up **the recording path**. The texture
modules (`texglue71`, `texture71`, `texfill71`, `mip71`, `texuni71`,
`backend71tex`, `sampler71`, `texstate71`, `tfuplan71`) are not in the
library in any build. Only tests include them, with `#[path]`. They join
the path when the table gets its texture hooks (the P10 work), so section
1.3 lists them too. `frame71.rs` is the host frame model of the probes and
never enters the library.

Two experiments were run in a scratch copy (none of it is committed):

- **E1, cfg opened, no alloc.** With mod.rs's 7.1 lines set to
  `#[cfg(feature = "gpu-v3d71")]` and table71's `host-test` arms switched
  on, `cargo build --release --no-default-features --features
  osal-zephyr,gpu-v3d71,shaderc-online,no-shader-table --target
  aarch64-unknown-none` stops with 143 name-resolution errors (cargo's
  count): 94 `Vec`, 25 `vec!`, 11 `format!`, 7 `String`, 4 `Box`, 2 `std`.
  The 142 with a location are in admit71 (3), backend71 (15), draw71 (31), fill71 (30), glue71 (9),
  prim71 (14), record71 (8) and table71 (32). The other nine path modules
  (attr71, clearplan71, ffi71, format71, ident71, limits71, rt71, uniform71,
  zskeep71) compile no_std as they are.
- **E2, the same with `extern crate alloc`.** Each of the 17 files imports
  `alloc::{boxed::Box, format, string::{String, ToString}, vec, vec::Vec}`,
  `std::` becomes `core::`, and the capi crate gets a `#[global_allocator]`
  over Zephyr's `k_aligned_alloc`/`k_free`. The library and the capi
  staticlib then build for aarch64-unknown-none with no other change.
  Against the tip's archive (same features, release, LTO), the archive's
  `.text` grows from 1,387,496 to 1,471,752 bytes (+84,256) and `.rodata`
  from 143,894 to 168,542 (+24,648). `.bss` is unchanged (23,895,488). That
  is about 0.11 MiB, 0.11 % of the 96 MiB sram0. Option (a) compiles the
  same logic without the alloc crate's code, so its code size is about the
  same.

So code size does not decide between the options (section 3). Run-time
memory, failure behaviour and the crate's own rules do.

## 1. Inventory: every alloc and std use on the path, with its bound

The bounds come from the table's own limits: `DEFAULT_DRAWS_PER_JOB` 256
(table71.rs:144), `BCL_BYTES_PER_DRAW` 256 + `BCL_FIXED` 4096 (148, 150),
`IND_BYTES_PER_DRAW` 1024 + `IND_FIXED` 8192 (154, 157), `RCL_BYTES` 4 KiB
(159), `DATA_BYTES` 1 MiB (162) and `HISTORY` 64 (209). Other limits are
`attr71::MAX_ATTRS` 16 (attr71.rs:85), `ffi71::V3D_JOB71_MAX_RTS` /
`clearplan71::MAX_TARGETS` 8 (clearplan71.rs:31) and the GL core's
`MAX_UNIFORM_DWORDS` 1536 per stage (src/gl/context.rs:83). A uniform
stream (`ShcV3dUniform` list) has no GL limit: its length is the number of
uniform reads the compiled code makes, which the compiler's keep arena
bounds (`stream71`, src/shaderc/online/v3d/abi71.rs:85-94). Within one job
the indirect-buffer fit check bounds all of them together: `list_bytes`
(table71.rs:760-773) counts 4 bytes a stream word against
`256 * 1024 + 8192` bytes, so at most about 67,500 stream words for a job.
Measured sizes (host, LP64): `GlesDraw` 3608 B, `fill71::DrawIn` 360,
`FilledDraw` 632, `FilledJob` 408, `attr71::Attr` 64,
`ffi71::V3dVertexAttr` 16, `record71::DrawDesc` 112, `V3dJob71State` 188,
`V3dUniform` and `uniform71::Word` 8, `draw71::DrawState` 192, `Refusal` 40,
`Op` 64.

"Kind" names the construct. "Path" says whether a product draw runs it
(**yes**), or whether only the host-test oracles and adapters run it
(**oracle**: a test compares it with Mesa's clif, and the product never
calls it).

### 1.1 The leaves

| file:line | kind | what | bound | path |
| --- | --- | --- | --- | --- |
| admit71.rs:27 | `String` | `Refusal::reason`, the one refusal type of the path (`pub use` in fill71.rs:110, glue71.rs:117, backend71.rs:71) | the longest reason text: 121 bytes for the longest literal in the 7.1 files and state.rs, plus the formatted numbers | yes |
| admit71.rs:37, 41, 49 | `to_string()` | rt71, attr71 and ident71 refusals into `Refusal` | as above | yes |
| admit71.rs:53 | `format!("{r:?}")` | format71 refusal (Debug text) | as above | yes |
| admit71.rs:142 | `to_string()` | "a colour format is not a depth or stencil format" | 49 bytes | yes |
| admit71.rs:149 | `format!` | "no tile size fits {n} colour bytes per pixel" | about 50 bytes | yes |
| record71.rs:413-432 | `String`, `format!` x8 | `Refusal::message()` | about 100 bytes each | yes (admit71.rs:45, backend71.rs:106, 639) |
| prim71.rs:46 | `use std::fmt` | `Display` for `Refusal71` | (`core::fmt` is the same) | yes |
| prim71.rs:185 | `impl std::error::Error` | | none in core 1.99's no_std crate (`core::error::Error` exists) | yes (compile) |
| prim71.rs:225, 236, 244 | `Vec<u8>`, `vec!` | `pack_array`: 10 or 14 bytes | 14 B | yes, through `pack_draw` |
| prim71.rs:263, 275, 285 | `Vec<u8>`, `vec!` | `pack_indexed`: 10 or 14 bytes | 14 B | yes, through `pack_draw` |
| prim71.rs:298, 301 | `Vec<u8>`, `vec!` | `pack_base`: 9 bytes | 9 B | yes, through `pack_draw` |
| prim71.rs:309, 311 | `Vec<u8>`, `vec!` | `pack_index_buffer`: 9 bytes | 9 B | yes, through `pack_draw` |
| prim71.rs:320, 322 | `Vec<u8>` | `pack_draw`: index buffer + base + draw | 9 + 9 + 14 = 32 B | yes: fill71.rs:678 calls it for its refusals only and drops the bytes |
| draw71.rs:335-339 | `String`, `to_string`, `format!` | `Val::clif_text` | | oracle (tests/v3d71_draw.rs, v3d71_stencilops.rs) |
| draw71.rs:377 | `Vec` | `trait Packet { fn pairs() -> Vec<..> }` | a packet's field count, at most 12 | oracle |
| draw71.rs:657, 694, 720, 749, 777, 811, 841, 867, 889, 913, 939, 964, 983, 1005 (and the `vec!` on the next line of each) | `Vec`, `vec!` | the 14 `pairs()` impls, used only by `pack()` (682-1018) and the tests; `translate` (1386) returns `StatePackets71`, plain structs (1260-1275), and the C builder packs | at most 12 pairs | oracle |
| attr71.rs:96 | (string literal "Vec size") | not a use | | |

attr71, clearplan71 (`use core::fmt`, 28), ffi71 (`core::mem`, 33),
format71, ident71, limits71, rt71, uniform71 and zskeep71 (`core::fmt`,
108) have no alloc or std use (E1).

### 1.2 glue71, fill71, backend71 and table71

| file:line | kind | what | bound | path |
| --- | --- | --- | --- | --- |
| glue71.rs:159 | `to_string()` | `refuse` | reason text | yes |
| glue71.rs:550 | `to_string()` | prim71's index-type refusal | reason text | yes |
| glue71.rs:569, 575 | `Vec<attr71::Attr>`, `vec!` | `attrs`: one per GL attribute location | `MAX_ATTRS` 16 (1 KiB) | yes (table71.rs:728) |
| glue71.rs:630, 638, 639 | `Vec<fill71::Op>` | `ops`/`mapped`: a GlesDraw's operations | 2 (a clear and a draw) | yes |
| glue71.rs:647 | `Box::new(DrawIn)` | `Op::Draw` | 1 per draw, 360 B | yes |
| glue71.rs:689, 690, 721 | `Vec<Op>`, `Vec<DrawTmu>`, `vec!` | `TexOps` (the textured draw) | 2 ops, 1 TMU pair a draw | texture (P10) |
| fill71.rs:162, 166 | `to_string()` | `refuse`, `named` | reason text | yes |
| fill71.rs:398 | `Box<DrawIn>` | `Op::Draw` | 360 B a draw | yes |
| fill71.rs:411-413 | `Vec<V3dUniform>` x3 | `FilledDraw`'s FS, VS and CS uniform lists | a program's stream lengths; per job the indirect-buffer check, about 67,500 words in all | yes |
| fill71.rs:415, 416 | `Vec<u32>` x2 | `FilledDraw`'s VS and FS GL uniform blocks | 1536 words each (6 KiB) | yes |
| fill71.rs:417 | `Vec<V3dVertexAttr>` | `FilledDraw::attrs` | 16 (256 B) | yes |
| fill71.rs:439 | `&Vec<u32>` | `FilledDraw::ffi` parameter | | yes |
| fill71.rs:474 | `Vec<FilledDraw>` | `FilledJob::draws` | the draws of a job: `draws_per_job` (256) + the clear quads among them | yes |
| fill71.rs:484 | `Vec<(Rt71, ColorRow)>` | `Frame::rows` | 8 | yes |
| fill71.rs:492-501 | `collect` into `Vec<ColorTarget>` | `frame`'s admission input | 8 | yes |
| fill71.rs:524 | `Vec::with_capacity` | `rows` | 8 | yes |
| fill71.rs:573, 583 | `Vec<FilledDraw>` | `Open::draws`, the job being filled | as fill71.rs:474 | yes |
| fill71.rs:590 | `to_string()` | zskeep71's refusal | reason text | yes |
| fill71.rs:663-664 | `collect` into `Vec<V3dUniform>` | `uniforms`: `Word` to `V3dUniform`, the same 8 bytes | a stream's length | yes |
| fill71.rs:786 | `Vec::with_capacity` | `build_draw`'s vertex attributes | 16 | yes |
| fill71.rs:806 | `vec![0u32; n]` | a stream's filled words, checked by `uniform71::fill` and dropped | a stream's length | yes |
| fill71.rs:820, 842 | `&Vec<V3dUniform>` | parameters | | yes |
| fill71.rs:881, 882 | `to_vec()` | the GL uniform blocks copied into `FilledDraw` | 2 x 1536 words | yes |
| fill71.rs:913-922 | `collect` into `Vec<u8>` | `clear_quad`'s per-target masks | 8 | yes |
| fill71.rs:942 | `collect` into `Vec<u32>` | the clear quad's FS block | 4 words | yes |
| fill71.rs:1014-1015 | `Vec<FilledJob>`, `collect` | `fill`'s result | the jobs of one flush: one, plus one for each whole clear after queued draws, at most the op count (257) | yes |
| fill71.rs:1028, 1031, 1034 | `Vec<(FilledJob, Vec<DrawTmu>)>`, `Vec::new` x2 | `fill_ops` | as above | yes |
| fill71.rs:1164, 1190-1199 | `Vec<DrawTmu>`, `collect` | `TexFilledJob`, `fill_textured` | a job's draws | texture |
| fill71.rs:1242-1248 | `collect` into `Vec<Word>` | `tex_resolved` | a stream's length | texture |
| backend71.rs:78 | `format!` | `refuse`: every backend71 refusal | reason text | yes |
| backend71.rs:296 | `[Vec<u32>; 3]` | `DrawPlan71::streams`, the predicted FS, VS and CS streams | a stream's length | yes |
| backend71.rs:303-306 | `collect` into `Vec<Word>`, `vec!` | `stream`: `V3dUniform` to `Word`, then the filled words | a stream's length | yes |
| backend71.rs:345-357 | `collect` into `Vec<Attr>` | `plan_draw`'s attributes | 16 | yes |
| backend71.rs:423 | `Vec<DrawLayout71>` | `Built71::layouts`, one per draw | a job's draws | yes |
| backend71.rs:527 | `Vec::with_capacity` | `CBuilder71::build`'s layouts | a job's draws | yes |
| backend71.rs:600 | `Vec<DrawPlan71>` | `Assembled71::draws` | a job's draws | yes |
| backend71.rs:609-610 | `collect` into `Vec<u32>` | `words`: indirect-buffer bytes as words, for the stream checks | a stream's length | yes |
| backend71.rs:662-664 | `Vec<DrawPlan71>`, `collect` | `build_checked`'s plans | a job's draws | yes |
| backend71.rs:745-754 | `collect` into `Vec<ColorTarget>` | `admit_framebuffer` | 8 | yes |
| backend71.rs:775, 797 | `Vec<DrawPlan71>`, `collect` | `AssembledJob71::draws`, `assemble_filled` | a job's draws | yes |
| table71.rs:225-227 | `Vec<Word>` x3 | `Prog`'s owned uniform lists (the GlesDraw's `fs` lives for the call only) | a stream's length each | yes |
| table71.rs:253, 996 | `Box<BuiltClear>` | the clear program, built once | 1 | yes |
| table71.rs:271 | `Vec<Attr>` | `Queued::attrs`, by attribute record | 16 | yes |
| table71.rs:272, 273 | `Vec<u32>` x2 | `Queued`'s GL uniform blocks | 1536 words each | yes |
| table71.rs:283, 1236 | `Vec<Queued>` | `Open::queue`, the open frame's draws | `draws_per_job` (256; the OSAL's `OSAL_QUERY_V3D_DRAWS_PER_JOB` may name more) | yes |
| table71.rs:315, 341 | `Vec<u32>` | `State::stored`, the colour buffers a job stored | `HISTORY` 64 | yes |
| table71.rs:317, 342 | `Vec<(u32, Known71, Known71)>` | `State::ds`, the depth/stencil identities | `HISTORY` 64 | yes |
| table71.rs:319, 343, 391 | `Vec<String>`, `to_string()` | `State::logged`, the reasons logged this epoch | the distinct refusals, about 100; the 4.2 table keeps 32 pointers (backend.rs:1452) | yes |
| table71.rs:358, 374, 616, 1212, 1215, 1232, 1301 | `String`, `to_string()` | `Fail::Refused`, and its constructions | reason text | yes |
| table71.rs:603-609 | `Vec<u32>`, `collect` | `words`: a GL uniform block copied | 1536 words | yes |
| table71.rs:629-637 | `Vec<Word>`, `collect` | `program`'s `list`: a uniform stream copied | a stream's length | yes |
| table71.rs:710 | `Vec::new` x3 | `Queued` construction | | yes |
| table71.rs:724 | `Vec::with_capacity` | `staged`: an address for each GL attribute | 16 (`d.attribs`) | yes |
| table71.rs:733 | `vec!` | `attrs` by attribute record | 16 | yes |
| table71.rs:789-793 | `Vec<FilledJob>`, `Vec<&Queued>`, `Vec<Option<Program>>`, `Vec<Op>` | `fill`: the dry run and the flush over the queue | the queue + 1 | yes |
| table71.rs:863 | `to_string()` | `NEEDS_CLEAR_PROGRAM`, `REF_CLEAR_PROGRAM` | reason text | yes |
| table71.rs:900-901 | `String`, `collect` | `log_text`: a compile log as text | the compiler's info log | yes |

### 1.3 The texture modules (P10, not in the library yet)

| file:line | kind | what | bound |
| --- | --- | --- | --- |
| texglue71.rs:77, texfill71.rs:37, texture71.rs:76, mip71.rs:53 | `use std::fmt` | `Display` | (`core::fmt`) |
| texture71.rs:160, 354 | `Vec<Slice>`, `vec!` | a texture's slices | levels x faces: 16 x 6 (texstate71.rs:27, MAX_LEVEL 15) |
| texture71.rs:426, 427 | `String`, `std::iter::once` | parsing a pipe-format name's digits | 3 digits |
| mip71.rs:102, 122, 153 | `Vec<MipJob71>` | a mip plan's TFU jobs | `tfuplan71::MAX_NUMMM` 15 |
| tfuplan71.rs:111, 175, 193, 242, 248, 290 | `Vec<Level>`, `vec!`, `collect` | TFU level plans | 16 levels |
| backend71tex.rs:78, 82 | `format!` | refusals | reason text |
| backend71tex.rs:140, 141 | `Vec<(u32, V3dTex71State)>`, `Vec<(u32, V3dTex71Sampler)>` | the texture arena's states and samplers | `GLES_MAX_TEX_BINDS` a draw |
| backend71tex.rs:212, 223 | `Vec<Word>`, `collect` | `masked` | a stream's length |
| backend71tex.rs:249, 250, 260, 261 | `Vec<V3dJob71Tmu>` | a draw's TMU tables | the units a stage binds |
| backend71tex.rs:307, 308, 319, 328 | `Vec<V3dUniform>`, `Vec<Unit>`, `collect` | `resolve` | a stream's length |
| backend71tex.rs:521, 600, 630, 647, 649, 650 | `Vec` of layouts, plans, draws, `BuildDraw71` | the textured job | a job's draws |

texuni71, sampler71 and texstate71 have none.

## 2. How the 4.2 table records without an allocator

The 4.2 table (`src/v3d/backend.rs`, `state.rs`, `texture.rs`) has been in
the no_std product since M6. Its rules are what the 7.1 path follows:

1. **Record at the call, do not queue copies.** `v3d_draw`
   (backend.rs:6732) resolves the draw and calls `record`
   (backend.rs:3830), which stages the arrays into the data arena (`stage`,
   3528) and has the C builder write the draw into the open job's
   binning list and indirect buffer at once (`record_arrays_and_draw`,
   3880). The uniform blocks are passed as a pointer and a count
   (`storage`, 3798-3804). The builder copies them into the indirect buffer
   during the call, so nothing of the GlesDraw outlives the call.
2. **Every per-job collection is a fixed array with a count.** `Job`
   (backend.rs:953-1026) holds `progs: [_; JOB_PROGS]` (8), `attrs: [_;
   JOB_ATTRS]` (16), `streams: [_; JOB_STREAMS]` (6), `spills: [_;
   JOB_SPILLS]` (8), `tex_*: [_; TEX_CACHE]` (16) and `sampler_*: [_;
   SAMPLER_CACHE]` (16). The tables that outlive a job are fixed too:
   `zs: [ZsEntry; ZS_TARGETS]` with `nzs` (`zs_known`/`zs_record`,
   1608-1630, the twin of table71's `HISTORY` vectors) and the render
   images `RT_IMAGES` (313).
3. **"Full" is an outcome, not a panic.** `Attempt::Full` (backend.rs:3807-3809)
   ends the job and records the draw again in a new one. A draw that does
   not fit an empty job is refused by name. A table that is full answers
   the safe value (`zs_known`: `Written` once `nzs == ZS_TARGETS`).
4. **Refusals are `&'static str`.** `state::Refusal = &'static str`
   (state.rs:23). `count_refusal` (backend.rs:1591-1601) keeps the reasons
   it logged as pointers in `logged: [*const u8; 32]`.
5. **The big buffers come from the OSAL at init.** These are the arenas
   `ArenaSet` (backend.rs:1112) and `DATA_BYTES`/`TEX_BYTES`. The table's
   own state is one `Global<V3dState>` static (1566), not the stack.

The online compiler is the crate's second heap-free precedent: a bump arena
over a buffer its caller lends, with `AVec` for growing lists
(src/shaderc/online/arena.rs:1-20, `Arena::new` 72, `AVec::push` 447). The
module's doc names the rule: "`grep -rn "use alloc::" src/` finds nothing".
src/gl/program.rs:4180 and 4509 use the same `Arena` outside the compiler.

The 7.1 path cannot copy rule 1 as it is. fill71 decides the job split
(loads, stores, clear quads, a whole clear after draws opening a job) over
the whole queue, and table71 runs `fill71::fill` dry over queue + draw on
every draw (table71.rs:1272-1307). So the 7.1 table must keep each queued
draw's inputs until the flush. Rules 2 to 5 carry over unchanged.

## 3. The options

### (a) Fixed capacity and caller-lent storage, no allocator

Each module's product form uses `core` only:

- the refusal text is a fixed `Reason71` buffer (192 bytes, written with
  `core::fmt::Write`; the longest reason today is 121 bytes) in place of
  `String`;
- small bounded lists (attributes, render targets, masks, prim71's 32
  packet bytes, `HISTORY`, the logged reasons) are inline arrays with a
  count, like the 4.2 `Job`;
- lists bounded only by the job (the queue, a job's `FilledDraw`s, the
  uniform lists, the GL uniform blocks, the plans and streams) live in
  table71's static `State`. fill71 and backend71 borrow them as slices or
  write into slices the caller lends, and give back a job at a time rather
  than a `Vec<FilledJob>`.

Static memory: table71's `State` holds the queue (256 x about 4.9 KiB,
mostly the 3608-byte `GlesDraw` copy and 16 `Attr`s: about 1.25 MiB), a GL
uniform word pool (1 MiB: 256 K words, a mean of 1024 words a draw against
the worst case of 3072), a uniform-list pool (0.5 MiB: the indirect
buffer's 67,500-word bound at 8 bytes a `Word`), one job's `FilledDraw`
and `DrawPlan71` scratch with a stream pool (about 0.4 MiB) and the ops
array (257 inline `Op`s, about 95 KiB). That is about 3.3 MiB of `.bss`.
It must be cacheable memory: the 7.1 OSAL maps its carve-out Normal
non-cacheable (src/osal/zephyr_v3d71.c:65-66, 123, 241), and the dry run reads
the whole queue on every draw. The gl_test link uses 89.6 % of the 96 MiB
sram0 (about 86 MiB), so this takes it to about 93 %. The deqp and glmark2
rpi_5 images pin their own sram0 layouts (tests/v3d71_deqp_rpi5_conf.rs,
tests/v3d71_glmark2_rpi5_conf.rs), so their links must be checked too.
Two knobs bring the figure down: `QUEUE_CAP` (= the 256 default), and a
queue entry that keeps only the GlesDraw fields glue71 reads instead of the
whole struct.

- Host-test parity: the same code in both builds, apart from the
  host-test adapters (section 4, rule P2). Full pools and capacities are
  real outcomes ("Full": flush first, then a named refusal), and the host
  tests can provoke them with small capacities.
- The 4.2 build is unchanged: no 4.2 file changes, and mod.rs changes only
  its 7.1 lines.
- Risk: the largest diff (fill71, backend71 and table71 change their
  result types), stack use if a large array ends up on the stack (Zephyr's
  main stack is 256 KiB, driver/samples/zephyr/gl_test/prj.conf:24, so
  every large array goes in `State`), and the O(n²) dry run, which stays
  as it is.

### (b) `extern crate alloc` with a GlobalAlloc, for gpu-v3d71 product builds only

`extern crate alloc` under `cfg(feature = "gpu-v3d71")` in src/lib.rs, a
`#[global_allocator]` in capi/src/lib.rs (and in every Rust image that
links the crate), and the 17 files import `alloc::` names (E2: that is the
whole source change).

- Code size: +0.11 MiB (E2), as in (a).
- Heap: the same data as (a), but on a heap. Zephyr's libc arena is 1 MiB
  (`CONFIG_COMMON_LIBC_MALLOC_ARENA_SIZE`, gl_test/boards/rpi_5.conf), and
  `glShaderSource` copies already use it. It would have to grow by about
  the same 3.3 MiB, so sram0 use rises as in (a). The other heap, the
  carve-out, is the wrong one: it is non-cacheable, its allocator is a
  first-fit bitmap of 256-byte granules (src/osal/zephyr_v3d71.c:107-108,
  src/osal/v3d_granules.h), and there is no CPU-side allocation op in
  `OsalOps` (src/osal/mod.rs:214-274). A draw makes about a dozen small
  allocations, thousands a frame.
- Failure: an allocation that fails calls `handle_alloc_error`, which is a
  panic, and the images are `panic = "abort"`. Running out becomes a dead
  board instead of a named refusal: no `Full`, no log line, no verdict.
  This breaks the rule that every limit is a named refusal
  (docs/notes/v3d71-refusals.md) and the arena's "no path can silently
  truncate" (arena.rs:13-16). Fragmentation over a long dEQP run (thousands
  of contexts and programs) has no host test.
- Host-test parity: the best of the two (no source change in the modules),
  but the product runs an allocator that no host test runs.
- The 4.2 build stays unchanged only if the cfg holds. It touches three
  shared files (src/lib.rs, capi/src/lib.rs, Cargo.toml if the feature
  needs a new name), and it breaks the crate-wide "no allocator" rule that
  arena.rs:5-11 and tools/check.sh:97-102 rely on: the shaderc-online row
  exists to catch "an accidental `alloc` dependency".
- Risk: small diff, large behaviour change. The product's first allocator
  would arrive through one GPU's back end.

### Comparison

| | (a) fixed capacity | (b) alloc + GlobalAlloc |
| --- | --- | --- |
| code size | about +0.1 MiB | +0.11 MiB (measured) |
| static RAM | about +3.3 MiB `.bss` (cacheable) | about +3.3 MiB heap (libc arena) |
| running out | `Full`: flush, else a named refusal | panic, abort |
| host-test parity | the same code; capacities testable | same source, an allocator only the board runs |
| 4.2 build | unchanged (7.1 files and mod.rs's 7.1 lines) | unchanged only if the cfg holds |
| shared files | src/v3d/mod.rs (7.1 lines) | mod.rs, src/lib.rs, capi/src/lib.rs, maybe Cargo.toml |
| crate rules | kept (no `use alloc::`) | broken (the first allocator) |
| diff | large (fill71, backend71, table71 result types) | small |

## 4. The choice and the implementation split

**Choice: (a)**, fixed capacity with the queue and pools in table71's
static `State`, and no allocator. (b) costs the same RAM and code, and its
only gain, a small diff, does not make up for a panic in place of a named
refusal and for the crate's first allocator.

### Rules every item follows

- **P1, core only.** No `Vec`, `String`, `Box`, `vec!`, `format!`,
  `to_string`, `to_vec`, `collect` into an owned container, or `std::`, in
  a path file outside a `#[cfg(feature = "host-test")]` item. Large arrays
  live in table71's `State`, never on the stack.
- **P2, the existing tests do not change.** The 48 tests/v3d71_*.rs files
  that `#[path]`-include these modules keep compiling unchanged. Where a
  `pub` signature changes, the old one stays as a `#[cfg(feature =
  "host-test")]` adapter with the same name that calls the core form
  (examples: `fill71::fill` collecting the per-job core into
  `Vec<FilledJob>`; `record71::Refusal::message() -> String` over its
  `Display`). In a host-test build the `Refusal::reason` field keeps type
  `String`. Three tests build `Refusal { reason: x.to_string() }`
  (tests/v3d71_admit.rs, v3d71_fill.rs, v3d71_glue.rs) and seven build
  `Op::Draw(Box::new(..))` (v3d71_backend71_cases, _tex, _shapes,
  v3d71_case5_record, v3d71_glue_prims, v3d71_fill, v3d71_glue). For them
  `Reason71` and `DrawBox` are type aliases: `String` and `Box<DrawIn>` with
  host-test, the fixed buffer and the inline `DrawIn` without. Both are
  built through one constructor (`reason(format_args!(..))`, `boxed(..)`),
  so the text and the logic are the same. The new tests run the fixed
  `Reason71` buffer itself. Every existing test runs the core code. If an
  item cannot keep a test unchanged, it names the test, and the planner
  clears the edit with plan #400, which holds tests/**.
- **P3, the product cfg is compiled on every item.** Until mod.rs opens
  (N8), an item proves its files no_std the way E1 does: a scratch copy
  with the module's mod.rs line set to `cfg(feature = "gpu-v3d71")`,
  `cargo build -p yakogl --no-default-features --features
  osal-zephyr,gpu-v3d71,shaderc-online,no-shader-table --target
  aarch64-unknown-none`. The scan test of N1 pins the same thing in the
  tree.

### The items, in order

| # | item | files | host test | about |
| --- | --- | --- | --- | --- |
| N1 | `nostd71-scan`: the scan test. It scans each listed path file for the P1 tokens outside `cfg(feature = "host-test")` items (comments and string literals skipped). Its list starts with the nine clean leaves (attr71, clearplan71, ffi71, format71, ident71, limits71, rt71, uniform71, zskeep71) and texuni71, sampler71, texstate71. Each later item adds its files | tests/v3d71_nostd_scan.rs (new) | itself: `cargo test --no-default-features --features host-test,gpu-v3d71,shaderc-online,no-shader-table --test v3d71_nostd_scan` | 1 h |
| N2 | `reason71`: `Reason71` (fixed 192-byte buffer, `Display`, `Deref<str>`, `PartialEq<str>`, a mark when cut short) and the `reason(fmt::Arguments)` constructor in admit71.rs. admit71's seven sites, record71's `message` as `Display` plus the host-test adapter, prim71's `std::fmt`/`std::error` as `core::` | src/v3d/admit71.rs, src/v3d/record71.rs, src/v3d/prim71.rs (lines 46, 185 only) | tests/v3d71_reason71.rs (new): every `*_REFUSALS` table and each formatted refusal at its widest numbers fits 192 bytes uncut; the scan lists admit71, record71 | 1.5 h |
| N3 | `prim71-draw71`: prim71's `pack_*` into `[u8; N]` with a length (the 32-byte `pack_draw`), with `Vec` adapters under host-test. draw71's `pairs`, `pack`, `clif_text` (the oracles) move under `cfg(feature = "host-test")` | src/v3d/prim71.rs, src/v3d/draw71.rs | the existing v3d71_prim, v3d71_draw, v3d71_stencilops, v3d71_compose2-5, v3d71_instanced, v3d71_record unchanged; the scan lists prim71, draw71 | 1.5 h |
| N4 | `glue71-nostd`: `attrs` into `[Attr; MAX_ATTRS]`, `ops` into at most two `Op`s (an inline `[Option<Op>; 2]`), `DrawBox` and `boxed` in fill71.rs (the `Op` type only), the refusal sites through `reason`. The `TexOps` path stays host-test (P10) | src/v3d/glue71.rs, src/v3d/fill71.rs (lines 396-399 only) | the existing v3d71_glue, v3d71_glue_prims, v3d71_glue_tex unchanged; the scan lists glue71 | 1.5 h |
| N5 | `fill71-nostd`: `FilledDraw` borrows its uniform lists and GL blocks (`&'a [..]`; the clear quad's 4 words inline), attributes, rows and masks inline. The core `fill_each(fb, rev, hist, quad, ops, draws: &mut [FilledDraw], sink)` hands one job at a time to the sink. `fill` and `fill_textured` stay as host-test adapters | src/v3d/fill71.rs | the existing v3d71_fill and v3d71_backend71_* unchanged through the adapter; tests/v3d71_fill_each.rs (new): `fill_each` with a two-draw scratch gives `Full` and the jobs `fill` gives on the six gl_test cases' op lists | 2 h |
| N6 | `backend71-nostd`: `DrawPlan71::streams` as ranges of a stream pool the caller lends, layouts, plans and attributes into lent slices or inline arrays, `assemble_filled` over the job `fill_each` gives, `refuse` through `reason`, `Vec` adapters under host-test | src/v3d/backend71.rs | the existing v3d71_backend71, v3d71_backend71_cases, _shapes unchanged; tests/v3d71_backend71_lent.rs (new): the lent form assembles gl_test cases 5, 66, 102 to the bytes of the adapter form | 2 h |
| N7 | `table71-nostd`: `rec` with no alloc: `State` gets the fixed queue (`QUEUE_CAP` 256; a larger OSAL budget is clamped to it and logged), the word and list pools (`Full` flushes first, else a named refusal "the draw's uniforms do not fit the 7.1 table's pools"), `stored`/`ds` as `[_; HISTORY]` with counts, `logged` as `[*const u8; 32]` keyed by the reason's static text as on 4.2, `Fail::Refused(Reason71, i32)`, `log_text` as `&str` (backend.rs:4251's form), the clear program inline (no `Box`) | src/v3d/table71.rs | the existing v3d71_table, v3d71_table_cases, v3d71_table_draw, v3d71_table_indexed, v3d71_instanced unchanged (they drive `rec` through `yakogl_backend_v3d71`); tests/v3d71_table_full.rs (new): 300 draws in one frame flush at 256, an oversized uniform block is refused by name | 2 h |
| N8 | `mod71-open`: mod.rs's 7.1 lines become `#[cfg(feature = "gpu-v3d71")]`; table71's `host-test` arms (110-118, 178-179, 1393-1524) lose their cfg; `REF_NO_RECORDING` goes; `backend71` moves from table71's `#[path]` into mod.rs. The 4.2 lines are not changed | src/v3d/mod.rs (7.1 lines only; plan #399 holds the file), src/v3d/table71.rs (cfg lines and docs) | product build for aarch64-unknown-none and the scan's whole list; the c2 4.2 host tests (`host-test,gpu-v3d,shaderc-online,no-shader-table`) unchanged; tests/v3d71_table_cases.rs with host-test; the 4.2 product archive (`osal-zephyr,gpu-v3d,shaderc-online,no-shader-table`) with the tip's `.text`, `.rodata` and `.bss` sizes | 1 h |
| N9 | `img71-link`: the rpi_5 gl_test, deqp and glmark2 images link with `-DYAKOGL_WERROR=ON`, and their sram0 use is recorded against the conf tests' margins (a measurement, no board) | none (a measurement); if a margin breaks, `QUEUE_CAP` and the word pool in table71.rs | the three conf tests (v3d71_gltest_rpi5_conf, v3d71_deqp_rpi5_conf, v3d71_glmark2_rpi5_conf) | 1 h |

Then comes board71b: the six cases with real verdicts, U11 from case 5's
record, and the N8 replay testdata refreshed. The texture modules
(section 1.3) follow the same rules when the texture hooks join the table
(P10). They are not in this split.

N2 to N7 depend on one another in this order: N4 uses N2's `reason`, N5
uses N4's `Op`, N6 uses N5's job view, and N7 uses all of them. N1 can run
first or in parallel with N2. N3 depends only on N2. N8 waits for N7, and
N9 for N8.

### Shared, non-7.1 files the split needs

- `src/v3d/mod.rs`: only its 7.1 cfg lines (40-73) and its module docs
  (25-31), in N8. Plan #399 holds the file. Nothing in the 4.2 lines
  changes.
- `src/lib.rs`, `Cargo.toml`, `capi/`: **no change** with option (a).
  (Option (b) would have needed src/lib.rs and capi/src/lib.rs.)
- `tools/check.sh` (plan #387 holds it): `TARGET_CONFIGS`
  (tools/check.sh:3532-3537) has no gpu-v3d71 row, so no gate row compiles
  the product 7.1 library today. This is the row text to add (N8 makes it
  meaningful):

  ```
    "osal-zephyr,gpu-v3d71,shaderc-online,no-shader-table"
  ```

  It goes after the `gpu-v3d` line: it is the clippy row `clippy
  osal-zephyr,gpu-v3d71,shaderc-online,no-shader-table
  (aarch64-unknown-none, no host-test)`.
- tests/**: new files only (plan #400). P2 keeps the existing ones
  unchanged.
