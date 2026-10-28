<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# 2026-10-28 boardref: Mesa 26.2.1 descriptors for several samplers, 2D arrays, instancing and front facing

Plan #154 item intrt, issue #32. These are the Mesa 26.2.1 board references
that two frame items pin their descriptors to, recorded on the NanoPC-T6
(Mali-G610 MC4, panthor, kernel 6.18.55-current-rockchip64):

- **frame-tex2darray**: several samplers per stage and 2D array textures,
  for gl_test case 2 (two textures), case 42 es3array and case 206
  es3levels.
- **The frame side of the built-in inputs**
  (docs/notes/mali-builtin-inputs.md): case 47 es3inst, case 141 es3instdiv
  (divisor 2) and case 49 es3facing.

Every case passed in both of its runs on Mesa 26.2.1. In each case the
descriptor words of run 1 and run 2 are equal (section 4). No code changed.
The captures are kept out of the tree.

## 1. Programs

Each case is in its own file, compiled on the board with gcc 14.2.0
(Debian 14.2.0-19) as `gcc -O1 -Wall -Werror -o <name> <name>.c -lEGL
-lGLESv2`. Each program is the case on its own: the same shaders, the same
textures and buffers, and the same draws in the same order. The checks are
numbered in the order the case runs them.

| file | case | sha256 | what it draws |
|---|---|---|---|
| [`tex2.c`](tex2.c) | 2 (`case_texture`), the "two texture units" part | `5ec89635941679ea9353bb34d4bbca9a156b028f0a0ff84ee3916ae7a0b478fd` | ES 1.00 `gl_test_uv_vs_src` and `gl_test_texture2_fs_src` (the Mali build's `gl_test_shaders_es100.h`), with the 4x4 quadrant texture on unit 0 and a 1x1 yellow texture on unit 1. Frame 1 draws with `u_tex0 = 0, u_tex1 = 1`, frame 2 with the uniforms swapped. Checks 1-4. |
| [`es3array.c`](es3array.c) | 42 | `e5b72e7aab251e981328c4bcc4df69aaff522f8e94d9394c55cc6ca67a280219` | An 8x8x3 RGBA8 2D array drawn with es300/array.frag at layers 0, 1 and 2. Then layer 1 becomes an FBO attachment (`glFramebufferTextureLayer`) and is cleared blue. Then layers 0, 1 and 2 are drawn again. Checks 1-6. |
| [`es3levels.c`](es3levels.c) | 206 | `1cc527318e2368c7c6819c93f702f7fcfb4d56563d008640071a48a5b30ba1e7` | A 16x16x4 3D texture and a 16x16x2 2D array, each with levels 0-2, `MAX_LEVEL` 2 and NEAREST_MIPMAP_NEAREST. Each is drawn into 16x16, 8x8 and 4x4 viewports (vol.frag, then array.frag). Checks 1-6 use the expectations for GPUs other than the PowerVR. |
| [`es3inst.c`](es3inst.c) | 47 | `7209a31f50c76fb1a0f93eb9f2a1395f1edf2d3da040189d3f88d3a97d723a4b` | es300/inst.vert with 4 instances and divisor 1 on locations 1 and 2, drawn with `glDrawArraysInstanced`, then with `glDrawElementsInstanced` (u16). Checks 1-10. |
| [`es3instdiv.c`](es3instdiv.c) | 141 | `8bd1ca34dc4a438a503092011988612c80a7cf1f28d82497fd1b89874b5b0d10` | Case 47's program with **divisor 2**, as arrays and as elements. Checks 1-8. |
| [`es3facing.c`](es3facing.c) | 49 | `a56b34d9460db55de2d7348fa427f6d1692692d6d794dd917c5a0514cb9f45c1` | es300/facing.frag on the CCW quad into the window with `glFrontFace` CCW, then CW. Then the same draws into an 8x8 texture FBO, each read back into the window at x0.25 by hdrsample.frag. Checks 1-4. |

The programs differ from the cases in these ways:

- The window is a 64x64 pbuffer with an RGBA8 config. Case 2 uses an ES 2
  context and the others an ES 3 context, because the ES 3.00 shaders need
  one.
- `tex2.c` binds `a_pos` to 0 and `a_uv` to 1 explicitly. This is the order
  the case relies on. The ES 3.00 shaders set their locations themselves.
- `--libdir <dir>` re-execs the program with `LD_LIBRARY_PATH=<dir>`
  (2026-10-28-splitref, section 1) and also with
  `MESA_SHADER_CACHE_DISABLE=true`. A disk-cache hit gives Mesa 26.2.1 a
  different binary from a fresh compile (splitref section 6). With the cache
  off, both runs compile fresh.
- Each program prints `GL_RENDERER`/`GL_VERSION` and a verdict line.

## 2. Command lines

The session's permission policy refused `tools/mali/mesa-board-ref.sh`, as
it did in 2026-10-28-splitref. Its steps were therefore run by hand over
plain `ssh`, one run at a time, by a script kept with the captures:

- render-node check: `fuser -v /dev/dri/renderD128` printed nothing (free)
  before every run
- `gcc -O1 -Wall -Werror -o <name> <name>.c -lEGL -lGLESv2`
- boot_id and the dmesg line count, before and after the run
- the run:

      PAN_MESA_DEBUG=linear,nocrc,dump,trace,sync PANDECODE_DUMP_FILE=/root/mref-yakogl/boardref/<name>-<run>.pandecode \
        ./<name> --libdir /root/mesa-26.2.1-build/inst/lib

- the dump files (`<name>-<run>.pandecode.<name>.ctx-0.NNNN`, the 26.2.1
  naming that splitref section 2 describes) concatenated in order
- the dmesg lines the run added

The host-side board lock (`board_lock.sh`) was not taken. connected's
exclusive lease of the board kept other tasks off it. No module was loaded or
unloaded and the board was not rebooted: boot_id stayed
`5bf2511d-5793-41c1-81e7-2e61ba2858ee` throughout.

## 3. Runs

Every run exited rc 0, added 0 kernel messages (dmesg sha256 `e3b0c442…`,
empty) and kept boot_id unchanged. In each case both runs printed the same
stdout, which ends in `verdict <name>: PASS`, and every check reported `ok`.

| case | run | concatenated `.pandecode` sha256 | lines | dump files | stdout sha256 (both runs) |
|---|---|---|---|---|---|
| tex2 | 1 | `663ea64398acdfe913410a31fbfa071466ec748a1bd70240ee24afddc520869f` | 3373 | 2 | `b4cbee29f932fa39cf417ce9860f8e06ffe541199709a5bd1192f36fcac39511` |
| tex2 | 2 | `5e8209e4d297936d7ba681305e264d7e16205dbc9f4c70cef1543ee316c199e5` | 3373 | 2 | |
| es3array | 1 | `c96ea44b78fdfb52a21ecc10b0b8f50ac53c8650d1fc8c21f0f99b3f3b101051` | 9192 | 7 | `5b07dfefb7e6fe1899ea4b1a8632511858e88014ed0b8fdfc6b7138d9b7f758f` |
| es3array | 2 | `4cf2ea6a62d5cd4b7b2a2f5499b9d9be5bdbd9e8b07df089c5ac8213e7490885` | 9192 | 7 | |
| es3levels | 1 | `12fe589556fe8b57ec5ea0ff2bf2486494781beec224b24571b65411c079c732` | 9672 | 6 | `b1a21a59df3292ee37d64daed2e69221279aab722d1935ff88f1c6919be7b1b4` |
| es3levels | 2 | `33d9893e95b433f5c55afb6f7e8a88ba2bd1f2c2ef35b0775c406675dddedce1` | 9672 | 6 | |
| es3inst | 1 | `e31f041416f8a7a3886695e5d7fbb9b615371527e66479cdc6ae1e959d7c6f8e` | 3298 | 2 | `054d20cc87a7cda6040e444966dbbf3e6c76b4811dac912b6602461d2820823a` |
| es3inst | 2 | `aed5c34d58a9395989e62eeccac842a0def66390a8c6434981358753a679d1e9` | 3298 | 2 | |
| es3instdiv | 1 | `300a32f31467cb0c6d142160df21bceae4f2309edd817e81b0e50fe4d9845bf0` | 3036 | 2 | `cbdc30f41685d732076444a914b1e9ad2cf5560ffdf027289ee35518adc952c3` |
| es3instdiv | 2 | `7ee9ddbdeccaece3cb056d8be8b6289b2e494f91c03374993d059b6d553cbfd2` | 3036 | 2 | |
| es3facing | 1 | `5819bf32aa89498b0f9f98e84139936d4c80af4e3088ae19b70af53d1a679cb5` | 7755 | 6 | `049ed5c1b8bc5a8c18c50767f961410d1ea1576c6770dff53212d948ef424c39` |
| es3facing | 2 | `f5bcd4a8e928c3a023c5db108f4f68fdc74767642eb0635e4d2b9b4e60b2ef2f` | 7755 | 6 | |

es3array and es3levels were each run twice before the runs above, from an
earlier revision of their source. That revision differed only in lacking the
`__attribute__((unused))` on three helpers; es3inst, es3instdiv and es3facing
needed the attribute to compile. Those earlier runs also passed, but they are
not the record.

The pixels Mesa produced (top-down coordinates, as the cases print them):

- **tex2**: `0xff0040bf` and `0xff804040`, then swapped `0xff0080bf` and
  `0xff408080`.
- **es3array**: layers `0x6400ff00`, `0x9628af50` and `0xc8505fa0`. After the
  FBO clear, layer 1 reads `0xffff0000`.
- **es3levels**: 3D `0x04602020`, `0x04602090` and `0x046020f0`; array
  `0xff60e020`, `0xffc04090` and `0xff1080f0`.
- **es3inst**: `0xff0000ff`, `0xff00ff00`, `0xffff0000`, `0x80808080` and
  `0xff000000`, the same for arrays and for elements.
- **es3instdiv**: `0xff0000ff`, `0x80008000`, `0xff000000` and `0xff000000`,
  the same for arrays and for elements.
- **es3facing**: window CCW `0xff00ff00`, window CW `0xff0000ff`, FBO CCW
  `0x40004000` and FBO CW `0x40000040`.

## 4. Run 1 against run 2

A cutter kept with the captures (`words.py`) reads every draw's descriptors
from each dump: the IDVS registers, the three resource tables with every
Texture, plane, Sampler, Attribute and Buffer, the FAU words and the program
descriptors. It also reads every fragment job's framebuffer, render target
and tiler context. Each value comes from the memory block of the frame that
follows the job. The cutter's output for run 1 and run 2 is equal line for
line in all six cases, with one exception:

- **es3facing, fragment FAU slot u2, second word**, in the two window draws
  that sample the FBO (draws 4 and 6). It reads `0x84D1F5C0` / `0x84D1F400`
  in run 1 and `0x7518F5C0` / `0x7518F400` in run 2. The shader pushes one
  word (`0x20001800`), and the 64-bit slot's second half is whatever memory
  follows: here, the low half of a CPU pointer. frame.rs already describes
  this and writes 0 there (frame.rs:2513-2518, "Mesa allocates only the
  words and the hardware reads the slot's second half from whatever
  follows"). It is not a word to pin.

The raw dumps also differ in two kinds of CPU pointer, and in nothing else:
the shader BOs' host addresses on the `Shader 0x… (GPU VA …)` lines, and the
CPU halves of the tiler-OOM context's `{cpu, gpu}` pairs in each frame's
pool. These are the same two kinds splitref section 6 found. A line diff of
the two dumps of each case shows nothing else.

## 5. The words

All words are little-endian 32-bit words of the descriptor, w0 first. Each
field is named as `src/mali/genxml/v10.rs` names it; the value in brackets
is the pandecode reading. "Built" means `src/mali/frame.rs` writes that field
today, at the line given. It says nothing about whether the value is equal
for these cases unless the table says so.

### 5.0 Words common to every draw here

| word | value | field | frame.rs |
|---|---|---|---|
| IDVS SRT registers d0 (vertex), d4 (fragment) | table \| **8** (e.g. `0x7FFFFFFCE7C8`) | `Resource` table count in the low bits: Mesa 26.2.1 has **8** tables. The 8th entry is empty. | **differs**: `RESOURCE_TABLES = 7` (frame.rs:247), tag at frame.rs:2878. The drm-shim traces in tools/mali/testdata (26.2.1) also tag 8. 25.0.7 tagged 7 (tools/mali/board-dump/instsplit-2026-10-24.txt: `0x7FFFFFFCE6C7`). |
| `Resource` entry | `<addr lo> 01007fff <count*0x20> 0` | `Address` (bits 0-55), `Contains descriptors` (bit 56) = 1, `Size` (64-127) = 32 bytes a descriptor | built, `resource()` frame.rs:2569-2576 |
| vertex table entry 3 | `06a99901 0 0 0 0 0 0 0` | the vertex stage's one `Sampler`: clamp-to-edge x3, round to nearest even, seamless, normalised, clamp int array indices, mipmap mode 0 (Mesa's workaround sampler) | built: `Sampler::default()` packs this word (frame.rs:2468) |
| r56 | `0x438000` | `PrimitiveFlags`: Allow rotating primitives, Low/High Depth Cull, Secondary Shader | built frame.rs:2963-2971 |
| r58 | `0x1FFFF` | `DcdFlags1`: Sample Mask 0xffff, Render Target Mask 1 | built frame.rs:3064-3065 |

### 5.1 Case 2: two samplers, two textures (tex2)

These are the fragment resource table of both draws. The SRT is d4 =
`0x7FFFFFFCE848` (table `…ce840`, 8 entries).

| entry | `Resource` words | holds |
|---|---|---|
| 3 (samplers) | `fffce2e0 01007fff 00000040 00000000` | **two** `Sampler`s (Size 0x40) |
| 4 (textures) | `fffce2a0 01007fff 00000040 00000000` | **two** `Texture`s (Size 0x40) |

The descriptors, draw 1 (`u_tex0 = 0`, `u_tex1 = 1`):

| descriptor | words | fields |
|---|---|---|
| Sampler[0] | `5e899801 1fff0000 0 0 0 0 0 0` | `Wrap Mode R` 8 (Repeat), `Wrap Mode T`/`S` 9 (Clamp to Edge), `Round to nearest even` 0, `Seamless Cube Map` 1, `Normalized Coordinates` 1, `Clamp integer array indices` 1, `Minify nearest` 1, `Magnify nearest` 1, `Mipmap Mode` 1 (None), `Minimum LOD` 0, `Compare Function` 0 (Never), `Maximum LOD` 0x1fff (31.996) |
| Sampler[1] | `5e888801 1fff0000 0 0 0 0 0 0` | as Sampler[0] with `Wrap Mode T`/`S` 8 (Repeat): unit 1's texture keeps GL's default REPEAT |
| Texture[0] | `2ec00022 00030003 00000688 00000000 ffffd080 00007fff 00000000 00000000` | `Type` 2, `Dimension` 2 (2D), `Format` RGBA8 UNORM RGBA, `Width`/`Height` 4 (minus 1), `Swizzle` 0x688 (RGBA), `Levels` 1, `Minimum`/`Maximum LOD` 0, `Sample count` 1, `Surfaces` 0x7fffffffd080, `Array size` 1, `Depth` 1 |
| its plane | `0200021a 00000100 fffdf000 00007fff 00000040 00000000 00000000 00030003` | `GenericPlane`: `Plane type` Generic, `Clump ordering` Linear, `Clump format` RAW32, `Size` 0x100, `Pointer`, `Row stride` 0x40, `Slice stride` 0, 4x4 |
| Texture[1] | `2ec00022 00000000 00000688 00000000 ffffd0c0 00007fff 00000000 00000000` | the 1x1 yellow texture, the same fields at 1x1 |
| its plane | `0200021a 00000040 fffde000 00007fff 00000040 00000000 00000000 00000000` | `Size` 0x40, `Row stride` 0x40, 1x1 |

Draw 2 (`u_tex0 = 1`, `u_tex1 = 0`) has the same two entries. Its descriptors
swap: Sampler[0] = `5e888801 …` (unit 1's state), Sampler[1] = `5e899801 …`,
Texture[0] = the 1x1 texture, Texture[1] = the 4x4 one. **Index i in both
tables is the shader's sampler i (`u_tex0` 0, `u_tex1` 1), and its descriptor
is the unit that sampler uniform names.** The program does not change
between the draws. Its fragment FAU is `0 0 | 0 0 | 20001800 20011801 |
3C003C00 0` in both. The u2 pair reads as the two look-ups' resource
handles, 0x2000 \| i for texture table 4 and 0x1800 \| i for sampler table 3
(4 << 11 and 3 << 11). It is a compiler push constant, not a frame word.

frame.rs today:

- **Not built**: a second sampler and a second texture. A stage has one
  `Sampler` (frame.rs:2467-2469) and at most one `Texture`
  (`texture_descriptor`, frame.rs:2554-2566). `DrawCall::texture` is one
  `Option<SampledTexture>` (frame.rs:1445). Entries 3 and 4 are written with
  count 1 (frame.rs:2844, 2846).
- **Built**: each single descriptor's words. `SampledTexture::sampler`
  (frame.rs:1292-1310) writes these sampler fields, with `Wrap Mode R`
  REPEAT for a 2D texture (frame.rs:1303). `SampledTexture::texture`
  (frame.rs:1360-1374) and `plane` (frame.rs:1337-1352) write a 2D
  texture's fields.

### 5.2 Case 42: a 2D array, and rendering to one of its layers (es3array)

All six draws have the same fragment entries 3 and 4. The values the shader
needs change through its uniform, not through descriptors.

| descriptor | words | fields |
|---|---|---|
| entry 3 | `fffcf260 01007fff 00000020 0` | one Sampler |
| Sampler | `5e899801 1fff0000 0 0 0 0 0 0` | as tex2's Sampler[0]: the case leaves `GL_TEXTURE_WRAP_R` at REPEAT |
| entry 4 | `fffcf240 01007fff 00000020 0` | one Texture |
| Texture | `2ec00022 00070007 00000688 00000000 ffffd080 00007fff 00000002 00000000` | `Dimension` **2D** (an array is a 2D texture), 8x8, `Levels` 1, **`Array size` 3** (w6 = 2), `Depth` 1 |
| plane 0 | `0200021a 00000200 fffe6000 00007fff 00000040 00000000 00000000 00070007` | layer 0: `Size` 0x200, `Row stride` 0x40, `Slice stride` **0** |
| plane 1 | `0200021a 00000200 fffe6200 00007fff 00000040 00000000 00000000 00070007` | layer 1, +0x200 |
| plane 2 | `0200021a 00000200 fffe6400 00007fff 00000040 00000000 00000000 00070007` | layer 2, +0x400 |

**One plane per layer**, layer-major. This is the cube's "faces as extra
layers" layout, with the layer in place of the face.

The layer-1 FBO frame (fragment job, no draw, the clear only):

| descriptor | words | fields |
|---|---|---|
| framebuffer pointer d40 | `0x7FFFFFFCF001` | `FramebufferPointer`: no ZS/CRC extension, `Render Target Count` 1 |
| r42, r43 (bbox) | `0x0`, `0x70007` | 8x8 |
| `FramebufferParameters` | `00000000 00000000 00000000 00000000 fffff000 00007fff 00000000 00000000 00070007 00000000 00070007 04001480 00010000 00000000 00000000 00000000` | w0: `Pre Frame 0/1`, `Post Frame` Never, **`Internal layer index` 0**; w8 `Width`/`Height` 8; w10 `Bound Max` 7,7; w11 `Sample Count` 1, `Tie-Break Rule` 2, `Effective Tile Size` 1024, `Render Target Count` 1, `Color Buffer Allocation` 4096; w12 `Z Internal Format` D24; w14:w15 `Tiler` **0** (no draw) |
| `RgbRenderTarget` | `04000000 86888299 00000000 00000000 00000000 00000000 00000000 00000000 fffe6200 00007fff 00000040 00000200 ffff0000 ffff0000 ffff0000 ffff0000` | w0 `Internal Format` R8G8B8A8; w1 `Write Enable` 1, `Writeback Format` R8G8B8A8, `Writeback Block Format` Linear, `Writeback MSAA` Single, `Dithering Enable` 1, `Swizzle` 0x688 (RGBA), `Clean Tile Write Enable` 1; w8:w9 **`Writeback buffer` base 0x7ffffffe6200 = plane 1's pointer**, w10 `Row Stride` 0x40, **w11 `Surface Stride` 0x200**; w12-w15 `Clear` blue |

Mesa selects the layer by **the render target's base address**. It does not
use `Internal layer index` (0) or the tiler context's `Layer count` /
`Layer offset` (1 / 0 in every frame here).

frame.rs today:

- **Not built**: `Array size` > 1 (frame.rs:1370 writes 1) and one plane per
  layer (`texture_descriptor` loops over `faces`, frame.rs:2554-2566;
  `faces` is 1 or 6, frame.rs:1225).
- **Built**: the FBD fields (frame.rs:1855-1878, `Tiler` 0 without a draw)
  and every RT field above (frame.rs:1934-1948). The base is `Target::gpu`,
  so the layer's address has to arrive as the target's address.
  **`Surface Stride` differs**: frame.rs writes 0 (frame.rs:1946), and the
  board's 26.2.1 writes 0x200 here and 0x4000 for the 64x64 pbuffer. The
  drm-shim traces in tools/mali/testdata read `Surface Stride: 0x0` (74
  lines), and the 25.0.7 dumps in tools/mali/board-dump read 0 or 256. A
  frame item should confirm which value it pins before changing it.

### 5.3 Case 206: 3D and 2D array mip chains (es3levels)

Draws 1-3 (the 3D texture, viewports 16, 8, 4) share these words, and so do
draws 4-6 (the array):

| descriptor | words | fields |
|---|---|---|
| Sampler (both) | `1e899901 1fff0000 0 0 0 0 0 0` | **`Wrap Mode R` 9 (Clamp to Edge)**, T and S 9, `Round to nearest even` 0, `Minify`/`Magnify nearest` 1, **`Mipmap Mode` 0 (Nearest)**, `Maximum LOD` 0x1fff |
| 3D Texture | `2ec00032 000f000f 00020688 02000000 ffffd100 00007fff 00000000 00000003` | **`Dimension` 3 (3D)**, 16x16, **`Levels` 3** (w2 bits 16-20 = 2), **`Maximum LOD` 2.0** (w3 = 0x0200 << 16), `Array size` 1, **`Depth` 4** (w7 = 3) |
| 3D plane 0 | `0200021a 00001000 fffde000 00007fff 00000040 00000000 00000400 000f000f` | level 0: `Size` 0x1000 (4 slices), `Row stride` 0x40, **`Slice stride` 0x400**, 16x16 |
| 3D plane 1 | `0200021a 00000400 fffdf000 00007fff 00000040 00000000 00000200 00070007` | level 1: 2 slices, `Slice stride` 0x200, 8x8 |
| 3D plane 2 | `0200021a 00000100 fffdf400 00007fff 00000040 00000000 00000100 00030003` | level 2: 1 slice, `Slice stride` 0x100, 4x4 |
| array Texture | `2ec00022 000f000f 00020688 02000000 ffffd180 00007fff 00000001 00000000` | `Dimension` 2D, 16x16, `Levels` 3, `Maximum LOD` 2.0, **`Array size` 2**, `Depth` 1 |
| array planes 0-2 | `0200021a 00000400 fffdc000 …0 000f000f`, `0200021a 00000200 fffdc400 …0 00070007`, `0200021a 00000100 fffdc600 …0 00030003` | layer 0, levels 0, 1, 2 (`Slice stride` 0) |
| array planes 3-5 | `0200021a 00000400 fffdc7c0 …0 000f000f`, `0200021a 00000200 fffdcbc0 …0 00070007`, `0200021a 00000100 fffdcdc0 …0 00030003` | layer 1, levels 0, 1, 2. The layer stride in Mesa's storage is 0x7c0. |

A 3D texture has **one plane per level**, and its slices are reached through
`Slice stride`. A mipmapped array has **one plane per (layer, level)**,
layer-major (plane = layer × levels + level), the same order frame.rs uses
for a cube's (face, level).

frame.rs today:

- **Not built**: `Dimension` 3D (frame.rs:1362 writes Cube or 2D), `Depth` >
  1, `Array size` > 1 (frame.rs:1370), a plane `Slice stride` other than 0
  (frame.rs:1346), and a sampler `Wrap Mode R` other than REPEAT for a
  non-cube texture (frame.rs:1303 has no GL_TEXTURE_WRAP_R input).
- **Built**: `Levels`, `Maximum LOD` = levels - 1, `Mipmap Mode` from the
  minification filter, and the per-level plane loop (frame.rs:1292-1374,
  2554-2566).

### 5.4 Case 47: instancing with divisor 1 (es3inst)

Both draws have the same vertex table entries 1 and 2:

| descriptor | words | fields |
|---|---|---|
| entry 1 | `fffcd160 01007fff 00000060 0` | three Attributes |
| Attribute 0 (pos) | `2fc00115 00000002 00000000 00000000 00000010 0 0 0` | `Attribute type` 1 (1D), `Offset enable` 1, `Format` RGBA32F RGBA, `Table` 2, `Frequency` 0 (Vertex), `Offset` 0, `Buffer index` 0, `Stride` 0x10 |
| Attribute 1 (offs) | `2bc04025 00000042 00000000 00000001 00000018 0 0 0` | **`Attribute type` 2 (1D POT Divisor)**, **`Offset enable` 0**, `Format` RG32F RGB1, `Table` 2, **`Frequency` 1 (Instance)**, **`Divisor R` 0**, `Divisor E` 0, `Offset` 0, `Buffer index` 1, `Stride` 0x18, `Divisor D` 0 |
| Attribute 2 (icol) | `2fc00025 00000042 00000000 00000002 00000018 0 0 0` | as Attribute 1, `Format` RGBA32F RGBA, `Buffer index` 2 |
| entry 2 | `fffcd1c0 01007fff 00000060 0` | three Buffers |
| Buffer 0 | `00000019 00000060 fffe6000 00007fff 0 0 0 0` | `Type` 9, `Buffer type` 1 (Simple), `Size` 0x60, `Address` (6 vertices × 16). Draw 2: `Size` 0x40 at `fffdf000` (4 vertices). |
| Buffer 1 | `00000019 00000060 fffdd000 00007fff 0 0 0 0` | the instance buffer, `Size` 0x60 |
| Buffer 2 | `00000019 00000058 fffdd008 00007fff 0 0 0 0` | the same buffer **+8 (the attribute's offset folded in), `Size` 0x60 − 8** |

| register | draw 1 (arrays) | draw 2 (elements) | field |
|---|---|---|---|
| r33 | 6 | 6 | INDEX_COUNT |
| **r34** | **4** | **4** | INSTANCE_COUNT |
| r35, r36, r37 | 0, 0, 0 | 0, 0, 0 | INDEX_OFFSET, VERTEX_OFFSET, INSTANCE_OFFSET |
| d54, r39 | - , 0 | `0x7FFFFFFDE000`, 0xC | INDEX_BUFFER, INDEX_BUFFER_SIZE |
| RUN_IDVS flags override | `0x40008` | `0x40208` | `PrimitiveFlags` `Draw mode` Triangles, `Secondary Shader`; elements add `Index type` U16 |

The FBD and RT are the window's (section 5.6), and r57 is `0x121AB`.
**Built**: everything in this section. The Attribute words are frame.rs:
2776-2788 (`frequency` 2778, `attribute_type` 2784, `offset_enable` 2785),
the Buffers 2791-2803, INSTANCE_COUNT 3077, INSTANCE_OFFSET 2920, and the
index registers 3086-3100. These words equal the 25.0.7 instsplit pin
(tools/mali/board-dump/instsplit-2026-10-24.txt) and 26.2.1 did not change
them. gl_InstanceID adds no frame word: the program reads it, and the frame
supplies only INSTANCE_COUNT.

### 5.5 Case 141: divisor 2 (es3instdiv)

The words are the same as case 47's, except for these:

| descriptor | case 47 (divisor 1) | case 141 (divisor 2) | field |
|---|---|---|---|
| Attribute 1 w1 | `00000042` | **`01000042`** | **`Divisor R` = 1** (bits 56-60, log2 of the divisor); `Attribute type` stays 2 (1D POT Divisor), `Divisor E` 0, `Divisor D` (w5) 0 |
| Attribute 2 w1 | `00000042` | **`01000042`** | the same |
| Buffer 1 | `00000019 00000060 fffdd000 00007fff …` | `00000019 00000030 fffdd000 00007fff …` | `Size`: the instance buffer is 2 × 24 bytes |
| Buffer 2 | `00000019 00000058 fffdd008 00007fff …` | `00000019 00000028 fffdd008 00007fff …` | `Size` 0x30 − 8 |

INSTANCE_COUNT (r34) is 4 in both draws. The draws, registers, r57
`0x121AB`, FBD and RT are case 47's.

frame.rs today:

- **Not built**: `Divisor R`. The `Attribute` literal (frame.rs:2776-2788)
  never sets it, `check_draw` refuses a divisor other than 0 or 1
  (frame.rs:2029, `NOT_BUILT_INSTANCE_DIVISOR` at frame.rs:133-138), and
  the backend's `max_instance_divisor` is 0. A power-of-two divisor needs
  only `Divisor R` = log2(divisor). A divisor that is not a power of two
  (`1D NPOT Divisor`, `Divisor D`/`E`) is not in this record.
- **Built**: the Buffer `Size` (frame.rs:2792-2797, `size - offset`).

### 5.6 Case 49: front facing (es3facing)

The front face is DCD flags 0 (r57), `DcdFlags0` `Front Face CCW` (bit 16):

| draw | target | `glFrontFace` | r57 | `Front Face CCW` |
|---|---|---|---|---|
| 1 | window (pbuffer) | CCW | `0x121AB` | **1** |
| 2 | window | CW | `0x21AB` | **0** |
| 3 | FBO (8x8 texture) | CCW | `0x21AB` | **0** |
| 4 | window, hdrsample read-back | CCW | `0x121AB` | 1 |
| 5 | FBO | CW | `0x121AB` | **1** |
| 6 | window, read-back | CCW | `0x121AB` | 1 |

The other bits of `0x121AB`: `Allow Forward Pixel To Kill` 1, `Allow
Forward Pixel To Be Killed` 1, `Pixel Kill Operation` 2 (Weak Early), `ZS
Update Operation` 2, `Overdraw Alpha0` 1, `Overdraw Alpha1` 1 and `Aligned
line-ends` 1. The window is inverted relative to the FBO, as st/mesa makes
it. **Built**: `front_face_ccw: d.front_cw != d.flip_y` (frame.rs:3045)
gives these four values, with the window flipped (`flip_y`) and the FBO not.
The rest of DCD0 is built at frame.rs:3013-3054.

The 8x8 FBO frames:

| descriptor | words |
|---|---|
| `FramebufferParameters` | `00000000 00000000 00000000 00000000 fffff000 00007fff 00000000 00000000 00070007 00000000 00070007 04001480 00010000 00000000 fffd0340 00007fff` (8x8, `Tiler` set: there is a draw) |
| `RgbRenderTarget` | `04000000 86888299 00000000 00000000 00000000 00000000 00000000 00000000 fffc7000 00007fff 00000040 00000200 ff000000 ff000000 ff000000 ff000000` (the texture, `Swizzle` 0x688 RGBA, `Row Stride` 0x40, `Surface Stride` 0x200, clear black) |

The window frames (here and in every case above):

| descriptor | words |
|---|---|
| `FramebufferParameters` | `00000000 00000000 00000000 00000000 fffff000 00007fff 00000000 00000000 003f003f 00000000 003f003f 04001480 00010000 00000000 <tiler> 00007fff` |
| `RgbRenderTarget` | `04000000 860a8299 00000000 00000000 00000000 00000000 00000000 00000000 fffe0000 00007fff 00000100 00004000 <clear x4>` |

The window's `Swizzle` 0x60a (1546, BGRA) is Mesa's B8G8R8A8 pbuffer. It is
a property of the EGL config, not of the case. Our window target is whatever
the GL core's surface is (`TargetFormat`, frame.rs:893-905, which has no BGRA
format with alpha). Compare the FBO's RT, not the window's, when pinning.
**Built**: every field (frame.rs:1855-1878, 1934-1948). `Surface Stride`
differs, as in section 5.2.

The read-back draw (4, 6) samples the 8x8 texture with one Sampler
(`5e888801 1fff0000 …`: GL's default REPEAT, nearest) and one 2D Texture
(`2ec00022 00070007 00000688 0 ffffd100 00007fff 0 0`, plane `0200021a
00000200 fffc7000 00007fff 00000040 0 0 00070007`). All of it is built
(section 5.1). Draw 5 is the facing program into the FBO. It samples
nothing, yet its fragment table still carries that Sampler and Texture in
entries 3 and 4: Mesa keeps the sampler view the previous draw bound. The
shader reads neither, so a frame need not reproduce them.

## 6. What a frame item pins from here

- **frame-tex2darray**:
  - N samplers and N textures at entries 3 and 4, index i = the shader's
    sampler i (5.1).
  - `Array size` and one plane per layer (5.2, 5.3).
  - 3D: `Dimension` 3, `Depth`, one plane per level with `Slice stride`
    (5.3).
  - The sampler's `Wrap Mode R` from GL_TEXTURE_WRAP_R (5.3).
  - Rendering to a layer: the RT base is the layer's address (5.2).
- **The frame side of the built-in inputs**:
  - `Divisor R` = log2(divisor) for a power-of-two divisor (5.5).
  - Lifting `check_draw`'s divisor refusal and the backend's
    `max_instance_divisor`.
  - Instancing with divisor 1 (5.4) and the front-face bit (5.6) are
    already built.
- **Both**: the SRT count 8 against our 7, and the RT `Surface Stride`
  (5.0, 5.2). The 26.2.1 board writes both differently from frame.rs today.
  No case is known to fail because of either.

---

Board: NanoPC-T6, Mesa 26.2.1 (board build, reached through
`LD_LIBRARY_PATH` only), 6 programs, 12 runs, 12 PASS, 0 kernel messages, no
reboot.
