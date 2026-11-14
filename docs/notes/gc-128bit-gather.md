<!-- SPDX-License-Identifier: Apache-2.0 -->
# textureGather on a two-plane RGBA32 unit (GC7000UL): design

Status: design only, nothing built, nothing measured on a board. Read with
`gc-128bit-texture-sampling.md` ("The shader join", "What is left" 1) and
`gc-128bit-swizzle.md` (sections 5.2 and 7 item 3). Citations are to Mesa
26.2.1 (`third_party/mesa-26.2.1/UPSTREAM`; the etnaviv sources are not
vendored, they are read from the release tarball's
`src/gallium/drivers/etnaviv/`, here as `etna/`).

## 1. What Mesa does

A gather (`nir_texop_tg4`) on a 128-bit sampler is not cloned onto the
companion like every other look-up. `etna_nir_lower_128bit` has its own arm
(`etna/etnaviv_nir_lower_128bit.c:115-151`, comment `:109-114`: "Blob's
gather lowering for paired G32R32F emulation ... works for both float and int
128-bit formats - the bit pattern reconstructs the original value either
way"):

- The pass runs after `etna_nir_lower_texture`
  (`etnaviv_compiler_nir.c:1299-1300`), so a `textureGatherOffset` has
  already been turned into a coordinate shift by `lower_tg4_offset`
  (`etnaviv_nir_lower_texture.c:255-290`, registered `:481`). The join never
  sees a gather with an offset source. Our `split_planes` refuses offsets
  (`wide128.rs:165-167`); a gather arm must run after, or without, an
  equivalent of that lowering (section 4).
- The same keying as the other look-ups (`tex_is_128bit`,
  `sampler_companion`, `:101-107`). A gather is paired through
  `sampler_index` (`:97-101`; only `txf` uses `texture_index`).
- With `orig_comp = tex->component` (the GLSL `comp` argument, 0..3):
  - plane: `orig_comp < 2` uses the unit itself, otherwise the companion
    (`:117`). Component 0,1 are plane 0's R,G; 2,3 are plane 1's R,G.
  - 16-bit halves: `lo_comp = (orig_comp & 1) ? 2 : 0`, `hi_comp = lo_comp
    + 1` (`:118-119`). So the 32-bit channel `orig_comp & 1` of the plane is
    read as the two 16-bit "components" `2*(orig_comp&1)` and `+1` of the
    plane's texel, low half first.
- Two new `tg4` look-ups on that one plane, `component = lo_comp` and
  `hi_comp` (`:123-145`). Each copies every source of the original, keeps
  `coord_components`, `sampler_dim` and `is_array` (`:126-139`), forces
  `dest_type = nir_type_uint32` (`:131`) and sets both `texture_index` and
  `sampler_index` to the chosen plane's unit (`:132-133`). The result keeps
  the original's width (`:141-142`): a vec4, one lane per gather footprint
  texel.
- Result: `combined = lo | (hi << 16)` per lane (`:147`), replacing the
  original's uses (`:148-149`). The `ior`/`ishl` are bit operations, so the
  same code serves `sampler2D`, `isampler2D` and `usampler2D`: the 32 bits
  reassembled are the stored value's bit pattern (float bits for RGBA32F).
- Emission is the plain `TG4` opcode with the component as an immediate in
  src1 (`etnaviv_compiler_nir_emit.c:238-241`; coordinate in src0 `:221`,
  `.tex.id` from the sampler index `:216-220`). `TG4` has `LOW_HALF`/
  `HIGH_HALF` thread bits in the ISA (`isa/etnaviv.xml:154-155, 231-232`;
  our `isa_tables.rs:195-196`) but Mesa does not set them here; the "halves"
  are component numbers 0..3, not those bits.

What Mesa does **not** do:

- No descriptor change. The companion is the byte copy of plane 0 with new
  level addresses (`etnaviv_texture_desc.c:404-415`); there is no gather
  field in the TE sampler state, and nothing in
  `etnaviv_texture_desc.c`/`etnaviv_texture_state.c` mentions tg4 or gather
  (grep: the only hits in `etna/` are `etnaviv_nir_lower_texture.c:255-257,
  481`, `etnaviv_nir_lower_128bit.c`, the emit `:238-239` and
  `etnaviv_screen.c:292`).
- No swizzle handling: the descriptor's swizzle is in both planes
  (`etnaviv_texture_desc.c:150-152, 186`), the join ignores it
  (`gc-128bit-swizzle.md` section 2), and the gather arm reads component
  numbers of the unswizzled planes.
- No gating by format class: float and integer take the same arm. Only
  `format_is_128bit` units reach it, via the key.
- Gather is not a shipped feature: `max_texture_gather_components` is 4 only
  with `HALTI5 && DBG_ENABLED(ETNA_DBG_DEQP)`, else 0
  (`etnaviv_screen.c:292-293`). The path was written for dEQP runs, "the
  blob's" shape (its own comment), and nothing in the tree shows it checked
  against a board; neither can we.

## 2. Which component of which plane each gather lane reads

For GLSL `textureGather(s, uv, comp)` with the four-texel footprint lanes
`i = 0..3` (the order the hardware's `tg4` returns, not changed by the
lowering):

| `comp` | GL channel | plane (unit) | 32-bit channel of plane | `tg4` components (lo, hi) |
| ------ | ---------- | ------------ | ----------------------- | ------------------------- |
| 0 | R | plane 0 (`unit`) | R (x) | (0, 1) |
| 1 | G | plane 0 (`unit`) | G (y) | (2, 3) |
| 2 | B | plane 1 (`companion`) | R (x) | (0, 1) |
| 3 | A | plane 1 (`companion`) | G (y) | (2, 3) |

`lane[i] = lo_i | (hi_i << 16)` where `lo_i`/`hi_i` are lane `i` of the two
`tg4` results. Both read the same plane, so they share one footprint and one
filter-free texel set; this is why two `tg4` on the *same* plane compose and
no join across planes is needed. (Our join for `texture()` takes `.xy` of
each plane's result; the gather takes all four lanes of both results and
uses `comp` only to pick plane and pair.)

- **Float vs integer.** Mesa does the same thing: the halves are
  `nir_type_uint32` and are OR-ed (`:131, :147`). For RGBA32F the result is
  the float's bit pattern in a float-typed `vec4`; NIR carries it untyped.
  In our IR the join's two `Tex` keep `ret` of the sampler (F32, I32 or U32)
  and `int_tex` retypes afterwards (`wide128.rs:23-27`); a gather arm has
  to do the same for its halves *and* decide the type of the combine: the
  halves are raw 16-bit integers, so the two `tg4` need `ret = U32` (the
  integer-returning, un-converted form), the combine is an integer `Or` of
  `lo` and `hi << 16`, and for an F32 sampler the result is bit-cast back,
  not converted. This is the one place the pass would move bits instead of
  values. Whether the GC7000UL's `tg4` on a G32R32 format returns the raw
  16-bit halves as integers (as Mesa's `dest_type = uint32` assumes) or as
  converted values is **unmeasured**; it is the first thing for a board
  probe (section 6).
- **Sign.** `etnaviv_texture_desc.c:187-189` sets the signed-int8/16 CONFIG2
  bits only for those widths, so there is no sign extension of a half; the
  reassembly `lo | hi << 16` of two unsigned halves gives the right two's
  complement bits for RGBA32I.

## 3. Where it sits in `split_planes`

Today `split_planes` (`wide128.rs:135-243`) handles `Op::Tex` and refuses
shadow, gradient and offset on a keyed unit (`:159-167`). The IR has no
gather at all: `Op::Tex` (`src/shaderc/online/ir.rs:414-423`) has no gather
form, the parser has no `textureGather` builtin (no occurrence in `src/`)
and the back end's `builtins.rs:452` notes the corpus holds no `tg4`
(Mesa's emit transcribed, not measured). So there are two prerequisites that
are not about planes: a front-end `Gather { comp }` form (ES 3.2 core, ES
3.1 + `GL_EXT_texture_gather`-style extensions; **not ES 3.0**, which has no
`textureGather`) and a `tg4` selection path. Until they exist, a gather is a
compile error before it can reach the join, and the plan's "refused by
name" item has to place its refusal where the gather is first named (the
builtin lookup), with the text pointing at this note, and keep an equivalent
check in `split_planes` for when the IR grows the op.

Once the IR has a gather (call it `TexLod::Gather { comp }` or a field of
`Op::Tex`; the choice is the front-end item's), the arm in `split_planes` is:

1. Refusals stay: shadow gather (`textureGather(sampler2DShadow, uv,
   refZ)`) and offset gathers (`:160-167`; Mesa's offset lowering is a
   coordinate shift by `offset / textureSize`, `lower_tg4_offset`
   `:255-290`, which this back end would have to supply itself and refuse
   until then).
2. For a gather on a keyed unit, with plane `p = comp >> 1` (unit or
   `companion_of(unit)`) and `k = comp & 1`: emit
   `lo = Tex(gather, comp = 2*k)` and `hi = Tex(gather, comp = 2*k + 1)`,
   both at sampler `p`, `ret = U32` (see section 2); then
   `StoreLocal`/`Op` sequence `v = lo | (hi << 16)` (existing integer ops
   `Ior`, `Ishl` by constant 16; if the IR lacks them as `Op`s, that is the
   second prerequisite), bit-cast to `ret` of the original for an F32
   sampler. Result: 4 lanes, no joining stores into lanes by mask, because
   the gather's lanes are footprint texels, not channels. This differs from
   the `texture()` arm (`:200-216`): `EXTRA` is not 4 but the number of
   instructions of this sequence (two gathers, shift, or, and any cast), per
   split gather, and `comps`/`locals` follow it as they do for `EXTRA`.
3. `comp` is a compile-time constant in GLSL (ES 3.2: "must be a constant
   expression"; GL 4.x allows a dynamic one in desktop GLSL). A dynamic
   `comp` is refused by name; the constant is the only form the table in
   section 2 can pick a plane from at compile time.
4. `textureSize`, `texelFetch` on the same unit are untouched (they are not
   gathers).

## 4. With the swizzle key of gc-128bit-swizzle.md section 5.2

Section 5.2 changes the key to carry per-unit lane sources
`sw[k]` (0..3 a lane of the joined RGBA, 4 zero, 5 one) and keeps the
descriptors identity in both planes. GL applies a texture swizzle to the
texel the gather reads from (ARB_texture_gather/GLSL 4.00 and ES 3.2:
"swizzle is applied before the gather component is selected"; this note
assumes that and a board or CTS check should confirm it, section 6). Then a
gather with component `comp` on a unit with sources `sw` reads
`c = sw[comp]`:

- `c < 4`: the table of section 2 with `c` in place of `comp`; the gather
  never touches the plane the sources do not name, and a swizzle that moves
  a channel across the planes (`(B,G,R,A)`) is simply a different `c`, so
  the gather is *easier* than the `texture()` arm, which needs two look-ups
  and lane stores.
- `c = 4` or `5`: GL says the gather returns zero or one in every lane.
  Emit a constant `vec4(0)` or `vec4(1)` of the sampler's type: `ConstF32`
  0.0/1.0 for F32 sampler, `ConstI32` 0/1 for I32/U32 (the same typing rule
  as 5.2 step 2, "never `0x3f800000` in an integer sampler"), and emit no
  `tg4` at all.
- Identity key: `c = comp` and the output is exactly the section 1 shape.
- `no_alpha` (RGB32F/I/UI stored as RGBA32): `sw[3] = 5`, so `comp = 3`
  yields the constant one, with no plane-1 gather (this is why section 4 of
  the swizzle note's no-alpha-as-one design makes the gather correct for
  RGB32 textures without relying on stored alpha).

The key's compile-cache hash (5.2 step 1) already covers a per-unit change
of `c`; two gathers on one unit with different `comp` in one shader are two
separate look-ups with their own `c` and need no extra key.

If the swizzle key (gc-w128-key-pass) is not in yet, the gather arm is
written for identity only, and the guard of 5.1 (a non-identity swizzle is
refused by name) covers it unchanged: a gather on a swizzled unit is as
refused as a `texture()`.

## 5. Do the descriptor words change?

No. A gather uses the same two descriptors as every look-up on the unit
(`etnaviv_texture_desc.c:374-415, 479-513`): format `EXT_G32R32F` or
`EXT_G32R32I` at CONFIG1, identity swizzle (ours), the companion's
`DESC_ADDR_COMPANION`. Mesa has no gather-specific sampler or descriptor
word; the only per-op state is the immediate component in the `tg4`
instruction (`etnaviv_compiler_nir_emit.c:240`). `src/vivante/texture.rs`
(`format_words`, `emit_slot`) is unchanged, and `tests/es3_wide128_planes_gc.rs`
pins stay.

One thing to look at on the board, not in the words: the format
`G32R32F`'s `tg4` returning four 16-bit components (0..3) is how Mesa treats
it, so the hardware must expose a G32R32 texel as four 16-bit components to
`tg4`, while `texld` gives the two 32-bit ones. That is the claim "the blob
does this" carries (`:109`); we have no corpus `tg4`.

## 6. Host tests and the sequence

Everything here is inert: `GC_SAMPLED_TEXFMTS` holds no 128-bit class until
gc-w128-flip, and the arm runs only under a non-empty key.

Sequence (each item is its own commit, in order; steps 1-2 are not about
planes and may be done by whoever adds `textureGather` to this back end):

1. **Front end**: `textureGather(sampler2D, vec2, int comp)` and the array,
   cube, `isampler`/`usampler` forms, constant `comp`; an IR form for it
   and a `tg4` selection that `lodkey`/`int_tex`/`select` handle (corpus
   has none; Mesa's emit `:238-241` is transcribed). Offset and shadow
   forms refused by name.
2. **The named refusal** (the plan's gc-w128-gather item): `split_planes`
   refuses a gather on a keyed unit ("a gather on a 128-bit sampler is not
   split into its two planes"); until step 1 the refusal is at the builtin.
3. **The arm** for the identity key (sections 1-3) in `split_planes`.
4. **The swizzle case** (section 4), after gc-w128-key-pass.
5. **Board**: an RGBA32F, an RGBA32I and an RGBA32UI texture gathered at
   `comp` 0..3, read back; the first board session that draws a two-plane
   texture (`gc-128bit-swizzle.md` section 6). It settles the section 2
   `tg4` half question and the swizzle-then-gather order of section 4.

Host tests (`wide128.rs` tests, using its `compile_fs`/`listing`/`units`;
and `tests/es3_wide128_gc.rs` for the GL-core key; a new file
`tests/es3_wide128_gather_gc.rs` only if that one is held):

- empty key: a gather compiles to the same words as without the pass
  (`an_empty_key_changes_nothing` shape);
- identity key, `comp` 0, 1, 2, 3 on `sampler2D`: exactly two `tg4` per
  gather; the table of section 2 is the expected (unit, component) of each:
  `comp 0 -> (u,0),(u,1)`, `1 -> (u,2),(u,3)`, `2 -> (c,0),(c,1)`,
  `3 -> (c,2),(c,3)`; the unit field of each `tg4` word is read with
  `units()`;
- the combine: `Or(lo, Shl(hi, 16))` is present and no float conversion
  (`I2f`/`F2i`) sits between the `tg4` and the use, for `sampler2D`
  (bit-cast, not convert), `isampler2D` and `usampler2D`; `int_tex` leaves
  the two `tg4` as `U32` and the combine as is;
- companion untouched by the shader: a gather's second plane is the
  companion, and the "second plane is on a unit the shader samples itself"
  refusal (`:151`) applies to a gather as to `texture()`;
- 2D array, cube and 3D-less gather forms: the footprint lanes and the
  coordinate are copied to both `tg4` (the layer in the coordinate); one
  test per dimension;
- refusals by name: shadow gather, `textureGatherOffset(s)` (constant and
  dynamic offsets), a dynamic `comp`, each with the exact refusal text;
- a gather and a `texture()` on the same keyed unit in one shader: both
  lower, the instruction counts are the sum (`EXTRA` bookkeeping, `new_id`,
  `comps`, `locals` still consistent: run the module validator the existing
  tests use);
- with the swizzle key: `comp = 0` on `(B,G,R,A)` reads the companion's
  components (0,1); `comp = 3` on `(R,G,B,ONE)` (no-alpha) is a constant
  one with no `tg4`; `(ZERO,ONE,ZERO,ONE)` on `usampler2D` gives
  `ConstI32` 0/1, never `0x3f800000`; identity gives the section 1 shape
  byte for byte;
- GL core (`tests/es3_wide128_gc.rs`): a draw of an RGBA32F texture with a
  gathering shader is refused by name before the arm exists, and compiles
  under the key after (the `wide128_select` Err text is the existing
  pattern `draw.rs:4494-4498`);
- descriptor: pin that `format_words`/`emit_slot` output for an RGBA32F
  binding is the same words whether the shader gathers or not (the draw
  stream does not depend on the op), as the section 5 claim above.

Not host-testable: what the hardware returns for a `tg4` of a G32R32 plane
(raw halves or converted), and the order of swizzle versus gather
component. Both are board checks.

## 7. What is left

- The front end has no `textureGather`; ES 3.0 does not have it. If the
  roadmap's stage 4 (ES 3.0) is the only target, the refusal is all that is
  needed and the arm is stage-5 (ES 3.1/3.2) work. Mesa itself only
  advertises gather under a debug flag (`etnaviv_screen.c:292`).
- The `tg4` half semantics are inherited from a comment in Mesa about the
  blob and are not measured.
- Offset gathers need the coordinate-shift lowering of `lower_tg4_offset`
  (`:255-290`) in our back end first.
