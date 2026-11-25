# Mali: a texture coordinate the link packs beside another varying (plan #367, texcoord-425b)

The texture scenes (glmark2 entries 2, 3 and 4) were refused at fs `425b`
with "a texture coordinate other than a vec2 varying read whole or two
constants". The coordinate there is a plain `vec2` varying read whole. The
problem is where the link puts it.

## What Mesa does (gm2_tc425_1)

vs ace2 writes `Color = vec4(diffuse * MaterialDiffuse.rgb,
MaterialDiffuse.a)`, where `MaterialDiffuse` is `vec4(1.0)`. At the link,
`nir_opt_varyings` does three things:

- It folds the constant alpha into the fragment shader (`u2.w1.h0`, 1.0).
- It keeps Color's three equal lanes as one component.
- It packs `TextureCoord` behind that component, into one slot of three
  lanes.

The fragment program is then:

```
LD_VAR_BUF_IMM.f32.slot0.v3 ... @r0:r1:r2         diffuse, tc.x, tc.y
MOV.i32 r4, r1^ ; MOV.i32 r5, r2^                 the staging pair
MOV.i32 r2, u2.w0 ; MOV.i32 r3, 0x0               the handle
TEX_SINGLE ... .16.2d ... @r2:r3, @r4:r5
FADD.f32.wait1 r0.h0, r0^, 0x0.neg                diffuse to fp16, after the look-up
FMA.v2f16 r1, r2^, r0.h00, 0x0.neg                texel.xy * diffuse
MKVEC.v2i16 r0, r0^.h0, u2.w1.h0                  (diffuse, 1.0)
FMA.v2f16 r0, r3^, r0^, 0x0.neg                   texel.zw * (diffuse, 1.0)
```

## The lowering (`src/shaderc/online/mali/nir_fs.rs`)

- `layout`: a look-up's coordinate may be lanes 1 and 2 of one load, as
  well as lanes 0 and 1 or 2 and 3. This holds only in a mediump shader:
  gm2_tc425_2 is 425b with a highp default, where Mesa keeps the lowp
  sampler's texel at fp16 and the walk does not. The staging pair is the
  look-up's Collect of the two lanes, and the register allocator gives the
  two moves.
- `select`: such a load's other lanes may be converted for the colour. Only
  the coordinate's own lanes must stay unconverted (TEX_COORD_SHARED is
  unchanged otherwise). The load's conversions are emitted after the
  look-up instead of at the load, because Color's `f2fmp` stands where the
  colour reads it.
- The operand form `MKVEC` second in a product is admitted in such a shader
  only. Admitting it everywhere would let deqp_arith_op_binary_mul_fs_mediump
  compile differently from its reference.

## Results

`tests/mali_texcoord_425b.rs`:

- gm2_tc425_1 and _4 are byte-equal.
- _2 (highp) and _3 stay refused at TEX_COORD. In _3 the varyings are
  declared in the other order, and the coordinate straddles two slots
  (`MOV.i32 r6, r5^`, `MOV.i32 r7, r0^`).

The reference sweep's one other change: gm2_varylink_constw_tex's fragment
program now compiles byte-equal, so the pair is compared. Its vertex program
differs in the operand order of `dot(normal, L)` against a constant
(`FMA.f32 r1, r2^, u13.w1` against Mesa's `u13.w1, r2^`). That is a vertex
cause of its own.

glmark2: 425b and vs ace2 compile and link, so scenes 2, 3 and 4 are
compile-clean (18 to 21 of 33). `tools/mali/corpus-verdicts.txt` does not
move.
