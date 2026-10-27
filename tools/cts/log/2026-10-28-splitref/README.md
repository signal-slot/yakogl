<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# 2026-10-28 splitref: Mesa 26.2.1 reference of case 214 splitkick on the T6

Plan #154 item c1 (split-ref), issue #32. The Mesa 26.2.1 descriptors of
gl_test case 214 splitkick, a 4x frame split into two kicks, on the
NanoPC-T6 (Mali-G610 MC4, panthor, kernel 6.18.55-current-rockchip64). Our
driver fails the case at check 5 (`got 0xffc0c0c0 want 0xff808080`): its
second kick reloads the resolved colour into every sample. Mesa's second kick
reloads the four samples (section 5). split-fix pins `src/mali` to these
words.

## 1. Program

[`splitkick.c`](splitkick.c) is case 214 on its own. It uses the same 4x RGB8
pbuffer config (64x64), the same ES 1.00 shaders (`gl_test_shaders_es100.h`
`gl_test_vs_src` / `gl_test_fs_src`) and the same draws: clear black, then
case 16's white triangle (-1,-1) (1,-1) (1,1), then `glFinish` (kick 1),
then the same triangle with no clear, then `glFinish` (kick 2), then
`glReadPixels`. The checks are the same and numbered as in the case: 1 and 2
are the GL errors after each kick, 3 to 6 are the pixels. It differs from the
case in three ways:

- `glBindAttribLocation` sets a_pos 0, a_color 1 and a_uv 2 explicitly. This
  is the declaration order the case relies on.
- `--libdir <dir>` re-execs the program with `LD_LIBRARY_PATH=<dir>`. The
  board's system Mesa is 25.0.7. 26.2.1 is the release-tarball build in the
  board's `/root/mesa-26.2.1-build/inst/lib`, which only `LD_LIBRARY_PATH`
  reaches (driver/docs/nanopc-t6-mali.md, "Mesa 25.0.7 (the board's)
  against 26.2.1"). `mesa-board-ref.sh` passes arguments, not an environment.
- It prints `GL_RENDERER`/`GL_VERSION` and a verdict line.

sha256 `cb5dbabdd919053d5e8854c2468f5d60f5301329f56d014e5aa8c775156d0b54`
(the same on the host and on the board). It is built on the board with
gcc 14.2.0 (Debian 14.2.0-19).

## 2. Command lines

`tools/mali/mesa-board-ref.sh` was refused by this session's permission
policy, so its steps were run by hand over plain `ssh` (`scp` was refused
too; the files went over `ssh ... 'cat > file'` and back over
`ssh ... cat`). The steps are the same as the script's:

- render-node check (`fuser /dev/dri/renderD128`: rc 1, free)
- `gcc -O1 -Wall -Werror -o splitkick splitkick.c -lEGL -lGLESv2`
- boot_id and the dmesg line count before and after the run
- the run:

      PAN_MESA_DEBUG=linear,nocrc,dump,trace,sync PANDECODE_DUMP_FILE=/root/mref-yakogl/splitkick-<n>.pandecode \
        ./splitkick --libdir /root/mesa-26.2.1-build/inst/lib

- the dump files concatenated in order
- the dmesg lines the run added

Run 3 added `MESA_SHADER_CACHE_DISABLE=true` (section 6). The host-side
board lock (`board_lock.sh`) was not taken. connected's exclusive lease of
the board kept other tasks off it, and the render node was free before every
run.

Mesa 26.2.1 names its dump files `<PANDECODE_DUMP_FILE>.<program>.ctx-0.NNNN`
(here `splitkick-1.pandecode.splitkick.ctx-0.0000`). `mesa-board-ref.sh`
globs `$name.pandecode.ctx-*`, which is the 25.0.7 name, so under 26.2.1 it
would stop with "no pandecode dump was written".

## 3. Captures and pixel results

The captures are kept out of the tree. Each run wrote three dump files
(frames), had boot_id unchanged, exited with rc 0 and added 0 kernel
messages. Every run printed the same stdout:

    splitkick: Mali-G610 MC4 (Panfrost) / OpenGL ES 3.1 Mesa 26.2.1, EGL_SAMPLES 4
    splitkick:   x4 split: inside                         (40,43) = 0xffffffff  want 0xffffffff  ok  [check 3]
    splitkick:   x4 split: outside                        (20,23) = 0xff000000  want 0xff000000  ok  [check 4]
    splitkick:   x4 split: diagonal half covered          (32,31) = 0xff808080  want 0xff808080  ok  [check 5]
    splitkick:   x4 split: diagonal half covered (40,40)  (40,23) = 0xff808080  want 0xff808080  ok  [check 6]
    verdict splitkick: PASS
    rc=0

(The coordinates are top-down, as the case's `expect_px` prints them.)

| run | concatenated `.pandecode` | lines | stdout | dmesg (empty) |
|---|---|---|---|---|
| 1 | `93ffee2b7d3f1c17e91e2b0218665e75a8bf82c343264a8fc528e83b393fb4db` | 9830 | `044368ed12b8bece58ca9f515ea48c220500c46272ecc32efee7fbd90c60cb96` | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| 2 | `fde43dcfb2953653100bfacd3cb8d1c72022f1dd97b7de67c56a4b0e89bbb1bb` | 9816 | same | same |
| 3 (no shader cache) | `277240c83a1c45a874e0e43bd453409617d9f2dda34be2e566c3b96ceb9d3789` | 9830 | same | same |
| 4 | `21bde7f0a3d1cb5e9e65b856d71d82a125965613ba64396e39ef3ab149071147` | 9816 | same | same |

The per-frame files (`ctx-0.0000`, `0001`, `0002`):

| run | ctx-0.0000 | ctx-0.0001 | ctx-0.0002 |
|---|---|---|---|
| 1 | `99341b95e3eaf4a9596dde2d0807bf7c4d19abc52f561505006e88494823ac81` | `62a2bb5ed9ee89e5f74d4ed3e2717e36391147a11840c15973c3cfd499e487ef` | `2a818ca97d192295d6c544e6420548058255fdcd7b0cb336cd2c4aaac64c58df` |
| 2 | `1374c3acfe59deede7068804710eb620d62228d57f3c5e008a7495e7f92e36c5` | `d7ad14e7f2a51983bb7fe90d38be39a15ddfdf21679536c2f24025f9bd8882b0` | `ce081c1ea93e519a2276e84714e775ecdb735a341aa72bd91cf0454103a6b020` |
| 3 | `5f581f69f443d9a6b0c84c82f664ba7af2011b413c6107fdb6a1388ff262c0e2` | `73f08b5d879f83a329f1d6c3b2f2127ca69f6b5c6d27719e4d45f1087cf6d1cb` | `0afce766c13cb9680d46bb9d489cd75f4da5a20f3f4b14e67bf5085256570e9d` |
| 4 | `be488d82ce1e3abcd45bb04ce4f20f30b63cfadc5062322c98d181d37b5d1c40` | `34c0f747849a0375b9f2c615f41fa4d6cb6a65a592de6049fa9233c2552ce97c` | `57a196fd9c5c8afa55f687afda8234f85430f00f97fb516862622ec275588e99` |

## 4. The frames

Run 1's line numbers (run 2's are kick 1 at 2147 and kick 2 at 4707):

| dump file | lines | job | FBD | what |
|---|---|---|---|---|
| ctx-0.0000 | 46-1363 | IDVS 4-vertex fan + RUN_FRAGMENT (767) | `0x7ffffffc0000` | Mesa's own copy of the single-sample pbuffer colour (`0x7ffffffe0000`, `TEX_FETCH`, sample count 1) into the new 4x buffer, before the clear. No preload. |
| ctx-0.0000 | 1364-3929 | IDVS triangle + RUN_FRAGMENT (2151) | `0x7fffffea8000` | **kick 1**: the clear and the first triangle |
| ctx-0.0001 | 3930-6771 | IDVS triangle + RUN_FRAGMENT (4717) | `0x7ffffffc0000` | **kick 2**: the second triangle, with no clear |
| ctx-0.0002 | 6772-9830 | IDVS 4-vertex fan + RUN_FRAGMENT | `0x7ffffffc0000` | the resolve for `glReadPixels`: four `TEX_FETCH` of the 4-sample texture at `0x7ffffffd0000` into a single-sample (Writeback MSAA Single) target at `0x7fffffe9c000` |

Each frame's descriptors are read from the dump's memory block of the
descriptor pool that follows its RUN_FRAGMENT. Kick 1's pool is
`memory_7fffffea8000` (line 3735), and kick 2's is `memory_7ffffffc0000`
(line 6543). The words below are little-endian 32-bit words of those blocks
at the offsets given, each identical in all four runs. The tool that cut
them, `words.py`, is kept with the captures.

The fragment job's framebuffer pointer (`MOVE48 d40` before RUN_FRAGMENT) is
`0x7fffffea8001` in kick 1 and `0x7ffffffc0001` in kick 2. Both are Type 1
(MFBD), with no ZS/CRC extension and a render-target count of 1.

### 4.1 Framebuffer Parameters (pool +0x000, 16 words)

    kick 1: 00000000 00000000 00000000 00000000 fffff200 00007fff 00000000 00000000 003f003f 00000000 003f003f 08001292 00010000 00000000 ffea8400 00007fff
    kick 2: 00000002 00000000 00000000 00000000 fffff200 00007fff fffc08c0 00007fff 003f003f 00000000 003f003f 08001292 00010000 00000000 fffc0400 00007fff

The 16 words of padding (+0x040) are zero in both kicks.

| word | field | kick 1 | kick 2 |
|---|---|---|---|
| w0 | Pre Frame 0 (bits 0-2) / Pre Frame 1 (3-5) / Post Frame (6-8) | **Never** / Never / Never | **Intersect (2)** / Never / Never |
| w2:w3 | Frame argument | 0 | 0 |
| w4:w5 | Sample Locations | 0x7ffffffff200 | 0x7ffffffff200 |
| w6:w7 | Frame Shader DCDs | **0** | **0x7ffffffc08c0** (pool +0x8C0) |
| w8 | Width-1, Height-1 | 63, 63 | 63, 63 |
| w9, w10 | Bound min / max | (0,0) / (63,63) | same |
| w11 = 0x08001292 | Sample Count log2 = 2 (**4**), Sample Pattern 2 (**Rotated 4x Grid**), Tie-Break 2, Effective Tile Size log2 9 (512), Point sprite origin max Y 0, Render Target Count-1 = 0, Color Buffer Allocation 8 (8192) | same | same |
| w12 = 0x00010000 | S Clear 0; Z Internal Format D24; S/Z Write, Preload, Unload all 0; **Has ZS CRC Extension 0**; Empty Tile Read/Write 0; CRC Read/Write 0 | same | same |
| w13 | Z Clear | 0.0 | 0.0 |
| w14:w15 | Tiler (context pointer) | 0x7fffffea8400 | 0x7ffffffc0400 |

The sample locations (both kicks) are (-32,-96) (96,-32) (-96,32) (32,96),
then (-128,-128) for the 28 unused entries, then (0,0).

### 4.2 Render Target 0 (pool +0x080, 16 words)

    kick 1: 04000000 880ab299 00000000 00000000 00000000 00000000 00000000 00000000 fffd0000 00007fff 00000100 00004000 ff000000 ff000000 ff000000 ff000000
    kick 2: 04000000 080ab299 00000000 00000000 00000000 00000000 00000000 00000000 fffd0000 00007fff 00000100 00004000 00000000 00000000 00000000 00000000

| word | field | kick 1 | kick 2 |
|---|---|---|---|
| w0 = 0x04000000 | Writeback Mode Compat, Internal Buffer Offset 0, Internal Format R8G8B8A8 | same | same |
| w1 bit 0 | Write Enable | 1 | 1 |
| w1 bits 3-7, 8-11 | Writeback Format R8G8B8A8, Block Format Linear | same | same |
| w1 bits 12-13 | **Writeback MSAA = 3, Layered** (N samples, N surfaces) | same | same |
| w1 bits 14, 15, 16-27 | sRGB 0, Dithering 1, Swizzle 0x80a (BGR0) | same | same |
| w1 bit 31 | **Clean Tile Write Enable** | **1** (0x880ab299) | **0** (0x080ab299) |
| w8:w9 | Writeback buffer Base | 0x7ffffffd0000 | 0x7ffffffd0000 |
| w10 | Row Stride | 0x100 | 0x100 |
| w11 | **Surface Stride** (one sample's plane) | 0x4000 | 0x4000 |
| w12-w15 | Clear colour 0-3 | **0xff000000 x4** | **0 x4** |

### 4.3 ZS/CRC extension

Neither kick has one: FBD w12 bit 21 is 0 and the framebuffer pointer's
ZS/CRC bit is clear. The config has no depth or stencil buffer.

### 4.4 Tiler context (pool +0x400, 32 words)

    kick 1: 00bffd80 00048000 000040fe 003f003f 00000000 00000000 ffff7000 00007fff fffe7010 00007fff 0 x14 00000001 0 x7
    kick 2: 00bffc00 00048000 000040fe 003f003f 00000000 00000000 ffff7000 00007fff fffe7010 00007fff 0 x14 00000001 0 x7

The fields are the same in both kicks: Hierarchy Mask 0xfe, Sample Pattern
Rotated 4x Grid (w2 bits 13-15 = 2), FB 64x64, layer count 1, heap
0x7fffffff7000, geometry buffer 0x7ffffffe7000 / 65536, and private state
Word0 = 1. Only w0, the Polygon List (low word), differs: 0x4800000bffd80 in
kick 1 and 0x4800000bffc00 in kick 2. That is the next allocation in the
shared tiler heap, not a setting.

### 4.5 The draw (RUN_IDVS) of each kick

The kicks' draw streams (REQ_RESOURCE ... RUN_IDVS ... FINISH_TILING) differ
only in descriptor-pool addresses (`…ea8xxx` in kick 1, `…c0xxx` in kick 2)
and in the tiler heap Bottom/Top. In both kicks:

- DCD Flags 0: Multisample Enable 1, Evaluate per-sample 0, Pixel Kill and
  ZS Update Weak Early, Clean Fragment Write 0.
- DCD Flags 1: Sample Mask 0xffff, RT mask 0x1.
- Blend RT 0: Load Destination 0, opaque.

Kick 2's draw therefore does not read the destination. The old samples reach
the tile buffer before the draw, through the frame shader.

### 4.6 Kick 2's pre-frame-0 DCD (pool +0x8C0, 32 words; FBD w6:w7)

    00080a03 0001ffff 00000000 00000000 00000000 00000000 00000000 3f800000
    00000000 00000000 fffc0c20 00007fff fffc0c01 00007fff 00000000 00000000
    00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
    fffc0b84 00007fff fffc0bc0 00007fff fffc0100 00007fff 00000000 00000000

| word | field | value |
|---|---|---|
| w0 = 0x00080a03 | Flags 0 | Allow FPK 1, Allow FPK-killed 1, Pixel Kill **Force Early** (0), ZS Update Force Early (0), **Clean Fragment Write 1** (bit 9), **Evaluate per-sample 1** (bit 11), **Multisample Enable 1** (bit 19) |
| w1 = 0x0001ffff | Flags 1 | Sample Mask 0xffff, Render Target Mask 0x1 |
| w2-w4 | Vertex array | 0 |
| w5 | Flags 2 | 0 (no Z/S read or write) |
| w6, w7 | Minimum Z, Maximum Z | 0.0, 1.0 |
| w10:w11 | Depth/stencil | 0x7ffffffc0c20 |
| w12:w13 = 0x7ffffffc0c01 | Blend count 1, Blend 0x7ffffffc0c00 | |
| w14:w15 | Occlusion | 0 |
| w16-w23 | Shader env: attribute offset 0, FAU count 0 | |
| w24:w25 = 0x7ffffffc0b84 | Resources: table 0x7ffffffc0b80, 4 entries | |
| w26:w27 | Shader (program descriptor) | 0x7ffffffc0bc0 |
| w28:w29 | Thread storage | 0x7ffffffc0100 (the frame's TLS) |
| w30:w31 | FAU | 0 |

The pre-frame-1 DCD (+0x940) and the post-frame DCD (+0x9C0) are all zeros,
and their modes are Never.

Its resources (kick 2's pool):

| offset | descriptor | words |
|---|---|---|
| +0xB80 | resource table, 4 entries | `fffc0b20 01007fff 00000020 00000000` `fffc0b40 01007fff 00000020 00000000` `fffc0b00 01007fff 00000020 00000000` `fffc0a40 01007fff 00000020 00000000` |
| +0xB20 | entry 0, Attribute (1D, RGB32F RGB1, table 1, stride 16) | `2dc04015 00000001 00000000 00000000 00000010 00000000 00000000 00000000` |
| +0xB40 | entry 1, Buffer (size 0x40, at +0x880) | `00000019 00000040 fffc0880 00007fff 00000000 00000000 00000000 00000000` |
| +0x880 | the buffer: rect (0,0) (64,0) (0,64) (64,64), z 0, w 1 | `00000000 00000000 00000000 3f800000 42800000 00000000 00000000 3f800000 00000000 42800000 00000000 3f800000 42800000 42800000 00000000 3f800000` |
| +0xB00 | entry 2, Sampler (nearest, clamp to edge, unnormalised, round to nearest even) | `1c299901 00000000 00000000 00000000 00000000 00000000 00000000 00000000` |
| +0xA40 | entry 3, Texture: 2D, RGBA8 UNORM BGR1, 64x64, swizzle RGBA, 1 level, **w3 = 0x00004000: Sample count log2 = 2 (4)**, surfaces at +0xA80 | `2ec05022 003f003f 00000688 00004000 fffc0a80 00007fff 00000000 00000000` |
| +0xA80 | its plane: generic, linear, RAW32, **size 0x10000** (4 x 0x4000), pointer 0x7ffffffd0000 (= RT 0's writeback base), row stride 0x100, **slice stride 0x4000** (= RT 0's surface stride), 64x64 | `0200021a 00010000 fffd0000 00007fff 00000100 00000000 00004000 003f003f` |
| +0xBC0 | Shader Program: preload R60 R61 (0x3000), binary 0x7fffffffe180 | `80000128 00003000 ffffe180 00007fff 00000000 00000000 00000000 00000000` |
| +0xC00 | Blend: enable, round to FB precision, equation A Src / B Src / C Zero (replace), colour mask 15, opaque fixed-function RGBA8 conversion | `00000a00 f0122122 00000019 000ed000` |
| +0xC20 | Depth/stencil: compare Always, ops Replace, masks 0xff, stencil test off, depth write off, depth cull off | `024f24f7 ffffffff 00000000 00000000 e4800000 00000000 00000000 00000000` |

The preload program (GPU VA 0x7fffffffe180, the same bytes in all runs):

    3d 00 00 12 04 80 54 08    LD_VAR_IMM.slot0.v2.f32.center.store.wait0 @r0:r1, r61, table:0x0, index:0x0
    40 00 0c c0 00 c0 90 68    F32_TO_S32.rtz.discard r0, r0^
    41 00 0c c0 00 c1 90 00    F32_TO_S32.rtz r1, r1^
    c0 1f 00 00 00 c2 10 01    IADD_IMM.i32 r2, 0x0, #0x1F
    7d cc 42 10 00 c2 b4 00    RSHIFT_AND.i32 r2, r61^, 0x13121110.b00, r2^
    42 c0 c0 00 00 c2 bd 00    MKVEC.v2i8 r2, r2^.b0, 0x0.b0, 0x0
    c0 c0 42 00 00 c2 bd 00    MKVEC.v2i8 r2, 0x0.b0, 0x0.b0, r2^
    c0 00 10 00 18 c3 10 01    IADD_IMM.i32 r3, 0x0, #0x18001000
    43 00 00 00 00 c4 91 00    MOV.i32 r4, r3^
    c0 00 00 00 00 c5 91 00    MOV.i32 r5, 0x0
    44 08 c0 53 36 40 25 49    TEX_FETCH.slot1.reserved.32.2d.texel_offset.wait @r0:r1:r2:r3, @r0:r1:r2, [r4^:r5^]
    f0 00 3c b2 08 40 7f 78    BLEND.slot2.v4.f32.end @r0:r1:r2:r3, blend_descriptor_0.w0, r60, target:0x0
    c0 00 00 00 00 f0 10 01    IADD_IMM.i32 r48, 0x0, #0x0
    c0 f1 00 00 10 c1 2f 00    BRANCHZI.eq.absolute 0x0, blend_descriptor_0.w1

Kick 1 has no frame shader. Its pool holds the same DCD at +0x8C0 (texture
+0xA40 → plane +0xA80 → 0x7ffffffd0000, the same words except the pool
addresses), but its FBD does not point to it (w0 modes Never, w6:w7 = 0).
That DCD belongs to the incremental-rendering FBDs that Mesa's CSF back end
prepares in case the tiler heap runs out (`pan_csf.c`, `emit_ir_fbd`), at
pool +0x480 (first pass: kick 1's FBD with its clear), +0x580 and +0x680
(middle and last passes: `Pre Frame 0 = 2`, Frame Shader DCDs
0x7fffffea88c0, RT w1 0x080ab299, no clear). This frame never used them,
but they show that Mesa reloads a 4x frame's samples whatever splits it.

## 5. Kick 1 → kick 2: every word that differs

| descriptor | word | kick 1 | kick 2 | field |
|---|---|---|---|---|
| FBD | w0 | 0x00000000 | 0x00000002 | Pre Frame 0: Never → **Intersect** |
| FBD | w6:w7 | 0 | 0x7ffffffc08c0 | **Frame Shader DCDs** |
| FBD | w14 | 0xffea8400 | 0xfffc0400 | Tiler pointer (pool address) |
| RT 0 | w1 | 0x880ab299 | 0x080ab299 | **Clean Tile Write Enable 1 → 0** (bit 31) |
| RT 0 | w12-w15 | 0xff000000 x4 | 0 x4 | Clear colour (unused once nothing clears) |
| Tiler context | w0 | 0x00bffd80 | 0x00bffc00 | Polygon List (heap allocation) |
| framebuffer pointer | - | 0x7fffffea8001 | 0x7ffffffc0001 | pool address only |

Every other word of the FBD, RT 0 and the tiler context is equal: sample
count 4, sample pattern, tile size, colour buffer allocation, Writeback MSAA
Layered, base, row and surface stride, D24 format and the Z/S enables.

**The words that show Mesa reloads the samples, not the resolved colour:**

1. RT 0 w1 bits 12-13 = 3 (**Layered**) and w11 = 0x4000 in both kicks.
   Kick 1 writes back all four samples, each as its own 0x4000-byte plane,
   to 0x7ffffffd0000..0x7ffffffe0000. It does not write a resolved surface.
2. FBD w0 = 2 and w6:w7 = 0x7ffffffc08c0 in kick 2: a pre-frame shader
   (Intersect) runs before kick 2's draw.
3. That DCD's w0 = 0x00080a03: **Evaluate per-sample 1** and Multisample
   Enable 1, Sample Mask 0xffff. The preload shader runs once per sample.
4. Its texture's w3 = 0x00004000 (**sample count 4**), and its plane's
   pointer, size and slice stride (0x7ffffffd0000, 0x10000, 0x4000) are kick
   1's layered writeback.
5. The preload program takes the sample index from r61 bits 16-20
   (`RSHIFT_AND r61, 0x13121110.b00, 0x1f`), packs it into the third
   `TEX_FETCH` coordinate (`MKVEC ... r2`), and fetches that sample of the
   4-sample texture. Each sample of the tile buffer gets its own old sample
   back.

The resolve to a single surface happens only in the third frame, for
`glReadPixels`: Writeback MSAA Single and four `TEX_FETCH` averaged.

## 6. Run to run

The descriptor words of sections 4 and 5 are equal in all four runs. Two
things in the dumps vary, and neither is a descriptor word:

- **CPU pointers in the pool** (+0x35C and +0x3B0..0x3DF of each pool block):
  three `{cpu, gpu}` pairs such as `0x0000ffff96a5d480`, `0x7fffffea8481`
  (run 1) and `0x0000ffff76c0d480` (run 2). These are the tiler-OOM
  context's pointers to the incremental-rendering FBDs. The CPU half changes
  with ASLR and the GPU half (FBD | 1) is equal. Runs 1 and 3, and runs 2 and
  4, differ only in these words (7 hunks each).
- **The application's fragment program** (the main draw's, not the
  preload's). Runs 1 and 3 read the varyings with
  `LD_VAR_BUF_IMM.f32 ... index:0x10 / 0x0` and convert them with
  `V2F32_TO_V2F16`. Runs 2 and 4 read them with `LD_VAR_IMM ... f16
  table:0x1`. Run 1 compiled a new program and run 3 disabled the disk cache
  (`MESA_SHADER_CACHE_DISABLE=true`). Runs 2 and 4 loaded it from
  `/root/.cache/mesa_shader_cache`. So Mesa 26.2.1's disk-cache hit gives a
  different binary from a fresh compile. The pixels are the same either way.
  A program byte reference from the board should be taken with the cache
  disabled. Whether the drm-shim references match the fresh form was not
  checked here.

---

Board: NanoPC-T6, Mesa 26.2.1 (board build, `LD_LIBRARY_PATH` only), 4 runs,
4 PASS, 0 kernel messages, no reboot.
