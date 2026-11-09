<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Reflection names of 63+ characters: keep the name whole, never widen a field

Date 2026-11-09, tip `bcab1c1d`. Plan item `refl-long-name-note`. Decision
note only: no file under `src/` or `tests/` changed; the implementing item is
`refl-long-name`, which may correct this note where it checked something wrong.

Closes the "decision needed" row of RC13 (`docs/es3-cts-failures.md:253`,
section at `:237`): dEQP `shaders.uniform_block.valid.long_instance_name_vertex`
and `..._fragment` (`tools/cts/gles3-mustpass.txt:30297-30298`) fail to link —
each GPU builds 3,457 of the 3,461 programs that must build and these are two
of the four (`docs/es3/shaderc-coverage.md:88`, cause row `:138`;
`docs/es3/shader-host-repro.md:108` row 11).

## What the two cases actually are

The data-driven case (`gles3-mustpass` reads
`data/gles3/shaders/uniform_block.test` at the pinned VK-GL-CTS commit
`93bca01861b0e3ef3c387027a9791e6d065f900c`, `tools/cts/deps.toml:254-323`;
read from a clone, nothing from it was executed) declares

```glsl
uniform UniformBlock { vec4 uniformMember; } aaaa...;   // 1024 a's, "// Total of 1024 characters"
```

in the vertex shader (`..._vertex`) or the fragment shader (`..._fragment`),
`version 300 es`, `expect build_successful`. The instance name is exactly
1,024 characters — the largest identifier GLSL ES 3.00 §3.8 allows, the limit
`src/shaderc/online/lexer.rs:540-544` already enforces (ES 1.00 has none).
`build_successful` means the case checks **compile and link only** (the board
message is "expected shaders to compile and link properly, but failed to
link"); no reflection string is compared. And the C++ sibling suite
(`glcUniformBlockCase.cpp`, `getBlockName() + "."` member prefix) expects the
block **type** name in every API name — `UniformBlock` and
`UniformBlock.uniformMember` — never the instance name. Our front end already
agrees: with an instance the block's GL name and the members' prefix are the
type name, and the instance name appears only inside the internal uniform-row
name `inst.member` (`src/shaderc/online/reflect.rs:310-321`, ES 3.0 §2.12.6;
`tests/es3_ubo.rs:518`, `:800-804`). So the long name reaches the GL through
one path only: the row name `<1024 a's>.uniformMember`, 1,038 characters.

Today the link refuses it: `the vertex uniform named 'aaa...' (63 a's) is as
long as the reflection's 64-byte name field stores, so it may have been
truncated` (`src/shaderc/link.rs:103-106`). That refusal is the decision this
note settles.

## 1. Where a reflection name is limited today

| place | limit | why it is there |
|---|---|---|
| `src/shaderc/online/lexer.rs:543` | identifier > 1024 characters: compile error | GLSL ES 3.00 §3.8, a language rule; the only name limit any spec puts on us |
| `src/shaderc/online/reflect.rs:1154` | row names are copied with `util::set_name` into `ShcUniform.name`: at most 63 characters stored, silently cut (`src/util.rs:201-213`) | the place the long instance name is actually lost |
| `src/shaderc/mod.rs:719` (`SHC_UNIFORM_NAME_MAX = 64`), `:921`; `include/gles_shaderc.h:128` (whose comment sizes the field from Qt Quick 3D: longest 39), `:144` | the 64-byte field itself | C ABI across the shaderc seam; offsets pinned at `src/abi_layout.rs:126-133` (size 74, `name` at 0) and `src/abi_layout_check.c:135-142` |
| `src/shaderc/mod.rs:735`, `:750`, `:772`, `:1021`; `include/gles_shaderc.h:89`, `:98`, `:109`, `:193`; pinned `src/abi_layout.rs:104-122` | 32-byte `name` in `ShcVarying`, `ShcAttrib`, `ShcAttribBinding`, `ShcVivIo` | same ABI seam; these names are PAIRED across stages at link, so a cut one is wrong, not just ugly |
| GC online: `src/shaderc/online/abi.rs:375` (`NAME_MAX = 31`), `:377-379` | io name > 31: `AbiError::NameTooLong` (`:350`) | refuses before the cut |
| PowerVR: `src/shaderc/online/powervr/abi.rs:523` (attribs), `:574` (varyings); `src/shaderc/online/codegen.rs:1120` (deferred-vertex declared attribs) | io name ≥ 32: `UscAbiError::NameTooLong` (`:58`) | refuses before the cut; uniform rows are copied verbatim (`:541-545`), truncated upstream or not |
| V3D: `src/shaderc/online/v3d_abi.rs:68-70` | io name ≥ 32: `V3dAbiError::NameTooLong` (`:44`) | refuses before the cut |
| Mali: `src/shaderc/online/mali/mod.rs:1471`, `:1483`; GC: `src/shaderc/online/backend.rs:3264`, `:3270` | plain `util::set_name`: cut silently | the shared link guard below is what stops them |
| `src/shaderc/link.rs:86-111` (`reject_truncated_names`, all four back-end links) | any stored name whose length is `field.len() - 1` is refused, *even intact ones* (`:61`, `:74`) | "the signature of truncation, and the only one there is"; `duplicate_name` (`:42`) catches two names cut to the same bytes; margin today: longest shipped varying 26 of 32 (`:37-38`), longest shipped uniform 59 of 63 (`:599-600`) |
| `src/gl/program.rs:164` (`ACTIVE_NAME_MAX = 68`) | the buffer `active_uniform_name` (`:789-793`) assembles reported names in — it reads only the 64-byte field (`:796`), so 64 + `"[0]"` fits | the field is the source; the cap is a consequence, not a rule |
| `src/gl/program.rs:261` (`block_member_name`, out `ACTIVE_NAME_MAX + 64` = 132), `:421`, `:463` | reported block-member names capped at 132 | same |
| `src/gl/program.rs:2167-2169` (`find_struct_member`, 72-byte key) | a key longer than 72 answers "no such member" | same |
| `src/gl/tf.rs:51` (`NAME_MAX = 64`), `:663`, `:704` | a transform-feedback varying name ≥ 64 cannot match a row: "Transform feedback varying ... undeclared." | fixed row names |

The front end itself has no limit: reflection holds arena `&'a str`
(`reflect.rs:51`, `:196`, `:208`), `block_table` (`:287-335`) builds names of
any length, and the block side table stores `&'static str` precisely because
it never crosses the C seam — "Rust-only, deliberately ... `struct shc_shader`,
its C header and the generated tables are unchanged" (`src/shaderc/mod.rs:824-828`,
team-lead, 2026-09-24). That is the precedent for the whole decision.

## 2. What the spec requires

- Queries hand out the **whole** name. `glGetActiveUniform` and
  `glGetActiveUniformBlockName` copy at most `bufSize - 1` bytes plus a NUL,
  and `length` receives the bytes written without the NUL; that is exactly
  `util::copy_out_str` (`src/util.rs:227-241`, tests `:482-538`), which
  `glGetActiveUniformBlockName` already uses for the full block name
  (`src/gl/program.rs:610-639`). Truncation is the caller's buffer decision,
  taken at the copy — never a reason to store less.
- `GL_ACTIVE_UNIFORM_MAX_LENGTH` and `GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH`
  are the longest active name + 1, "not the size of the buffer this
  implementation keeps names in" (`src/gl/program.rs:3030-3047`; the block
  side `:3002` already answers from the full `&'static str` block name;
  `GL_UNIFORM_BLOCK_NAME_LENGTH` `:587` likewise). Both stay right only if the
  reported names are whole.
- ES 3.0 §2.12.6 fixes the *shape* of the names (`Block`, `Block.member`,
  arrays as `name[0]`) and nothing about length. Beyond the compiler's
  1,024-character identifier (GLSL ES 3.00 §3.8) no spec limits a name; a link
  failure caused by name length is a conformance failure, and these two cases
  ask only for `build_successful` plus, for the C++ suite, the type name in
  the API names — which we already report.

## 3. What Mesa does

`third_party/mesa-26.2.1/` (UPSTREAM: tag `mesa-26.2.1`, commit `da14d65e`)
is vendored **without `src/mesa/`** — only the GPU compilers, gallium,
include and util — so there is nothing to cite in-tree and everything in this
paragraph is from knowledge, not from the vendored copy: Mesa keeps
`gl_uniform::Name` and `gl_uniform_block::Name` as heap `char *` inside
`gl_shader_program_data` (`src/mesa/main/shaderobj.h`), reports them through
`_mesa_copy_string` (`src/mesa/main/imports.c`) so the only cut is against the
caller's `bufSize`, answers the `*_NAME_LENGTH` queries with `strlen + 1`, and
the GLSL front end carries `ir_variable` names as ralloc strings — no
fixed-size name field anywhere on the reflection path. Mesa simply does not
have the problem this note decides; nobody "widens the field" because there is
no field.

## 4. The decision

**Names stay whole on the host side: in the compiler's keep arena, and in the
GL core, which reads them from there. A reflection name is never required to
fit a fixed field in order to survive.** The ABI does not move: the
64-byte `shc_uniform.name` and the 32-byte io fields stay byte-for-byte, with
every offset pinned in `src/abi_layout.rs:104-133` and
`src/abi_layout_check.c:113-142` unchanged — nothing here touches them, and if
a future layout test pins a new one, that change is its own commit.

The seam is handled the way `ShcBlocks` already handles it
(`src/shaderc/mod.rs:824-828`): a record keeps a *best-effort* fixed field for
consumers that only print names, and everything that needs the whole name
travels Rust-side, keyed by an index — not a hash; a 64-bit hash would re-open
exactly the "two names, one cut" collision `duplicate_name`
(`src/shaderc/link.rs:42`) exists to catch, and an index cannot collide.

Concretely, for `refl-long-name`:

- `uniform_rows` (`src/shaderc/online/reflect.rs:1146-1163`) keeps filling the
  64-byte field as today for rows that fit; a row that does not fit gets the
  field filled with its truncated prefix **plus a "truncated" marker**, and its
  whole `&str` goes into a Rust-only side table on the keep record (row index
  → whole name). The C header, `gles_shaderc.h`, is unchanged.
- `reject_truncated_names` (`src/shaderc/link.rs:86`) loses its uniform arm:
  once the whole name is reachable, "may have been truncated" is no longer
  true of the uniform rows, and the exact-length refusal at `:74` (test
  `:596-614`) goes with it — a 63- and a 64-character uniform name must both
  link and report whole.
- The GL core reads row names through the side table where one exists, which
  retires the caps that existed only because the 64-byte field was the source:
  `active_uniform_name`'s 68-byte buffer (`src/gl/program.rs:789-793`) grows to
  the same whole-name path as the block side, and `find_struct_member`'s
  72-byte key (`:2167-2169`) becomes a slice comparison. The `*_NAME_LENGTH`
  queries need no change once the names they measure are whole.
- The 32-byte io guards **stay** (GC `abi.rs:377-379`, PVR `abi.rs:523`/`:574`,
  V3D `v3d_abi.rs:68-70`, `codegen.rs:1120`, and the varying/attrib arm of
  `reject_truncated_names`): those names are paired across stages by
  `shaderc::link`, the fields are real ABI records on four back ends, and no
  failing test puts an io name anywhere near 32 characters (longest shipped:
  26, `link.rs:37-38`). Task #47 closes for uniforms; the io half of it is a
  separate, still-open question.
- The lexer's 1,024-character identifier rule stays — it is language, not
  implementation. The whole-name path must carry the worst case it allows: a
  1,024-character instance name and a 1,038-character row name, bounded, in
  arena memory.

## 5. Tests that pin it

- New host test `tests/es3_uniform_block_long_name.rs`, run under each online
  back end's feature the way `tests/es3_shaderc_corpus.rs` is:
  - both dEQP programs (1,024-a instance name in the vertex, resp. fragment,
    shader) **compile and link** — the CTS case is build-only, so a green link
    is the case;
  - the API names are the type names: `glGetUniformBlockIndex("UniformBlock")`
    finds it, `glGetActiveUniformBlockName` returns exactly `UniformBlock`, and
    the member is reported as `UniformBlock.uniformMember`;
  - the whole row name through the truncation edges of the copy-out contract:
    `bufSize` exactly `len + 1` (whole name + NUL, `length == len`), `bufSize 1`
    (empty + NUL, `length 0`), `bufSize 0` / NULL buffer (nothing written,
    `length 0`) — the `util::copy_out_str` semantics (`src/util.rs:227-241`);
  - `GL_ACTIVE_UNIFORM_MAX_LENGTH` equals the longest reported name + 1,
    `GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH` equals 13 (`UniformBlock` + NUL),
    and both stay `GL_INVALID_ENUM` under `REPORT_ES2`
    (`src/gl/program.rs:2988-2995`, the `es2_report_guard` rule);
  - edges that must not regress: a 63- and a 64-character uniform name link
    and report whole (replacing `link.rs:596-614`'s exact-length expectation),
    a 1,025-character identifier is still refused (`lexer.rs:543`);
  - any fixture or reference table it adds is regenerated in the same commit
    (CLAUDE.md, the gate compares the bytes).
- `tests/es3_shaderc_corpus.rs`: the build count moves **3,457 → 3,459** of
  3,461 on each GPU — the only other two refusals are
  `constant_expressions.other.complex_{vertex,fragment}`, the
  `gl_MaxDrawBuffers` row (`docs/es3/shaderc-coverage.md:88`, `:126`, `:136`) —
  and the `uniform_block` row `:87`/`:125` moves 64 / 2 → 66 / 0.
- Board: the two dEQP cases replace their `tools/cts/RESULTS.md` rows and the
  `docs/es3-cts-failures.md:253` row flips from "decision needed"; the
  `docs/es3/` rows are the es3-docs item's to move.

## What this note did not do

No code was changed and nothing was executed; the dEQP case text was read
from a clone of the `deps.toml`-pinned VK-GL-CTS commit (blobs via
`git cat-file`, no checkout, nothing run), and the Mesa §3 paragraph is from
knowledge because `src/mesa/` is not vendored. One correction to the task
text: the comment near `src/shaderc/online/abi.rs:192` about "more than 63"
being refused by the allocator is about `temps` (the six-bit `NUM_TEMPS`
field), not about names — the allocator puts no name limit at all.
