<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: three mip levels a render for the PowerVR mipmap chain (design of candidate 2)

Plan #329 item pvr-mip-three-levels-design, issue #1199. Written 2026-11-07
at main `0afc6847`. Every line number is that commit's. Since `4370107f` and
`ea371491`, the commits of `am62p-slow-terrain.md` and `pvr-mip-tail-e1.md`,
no file this note cites under `src/pvr`, `driver/src` or `tools/shaders` has
changed, so their line numbers still hold. This is a design only: no code was
changed, built or run, and no board was used. Every figure is either a record
that a cited note or `tools/cts/RESULTS.md` gives, with its commit, or a
prediction marked as one.

**Candidate 2** (`am62p-slow-terrain.md:241-265`): render level 1 from level
0 alone. Then render levels l+1, l+2 and l+3 from level l in one render with
three colour targets. An 800x600 chain then takes 4 renders (1; 2-4; 5-7;
8-9) and a 256x256 chain takes 4 (1; 2-4; 5-7; 8). This is route 3 of
`pvr-mip-fewer-kicks.md` (`:206-237`) with level 1 drawn alone. Drawing it
alone keeps levels 1 and 2, the only levels terrain's per-frame chains are
read at (`terrain-mip-usage.md:24-28`), as TPU samples. Those two levels stay
byte-identical to today's by construction.

**Verdict.**

- Today's render path already does most of it: one HWRT set for the chain,
  a separate store clip for each target, an end-of-tile program and a PDS
  data segment for 3 targets, three RGBA8 outputs within the four output
  registers, and the 3-target background clear.
- Four things are missing:
  1. The pitch of targets 1 and 2. It is derived from the width
     (`src/pvr/backend.rs:8918`), which is wrong for every NPOT level whose
     storage pitch is a larger power of two.
  2. A fragment program with three outputs.
  3. The draw that fills `targets[1..3]` for a mip render.
  4. The grouping of levels in `mip_chain_gpu`.
- All four are in `src/pvr/backend.rs`, plus one new shader, its list line
  and the regenerated `src/pvr/internal.rs`. Nothing changes in `driver/`, in
  `GlesDraw` or `GlesDrawTarget`, or in `tests/reference/pvr_internal.rs`.
- Terrain: 43 mip kicks a frame become 20, so 52 kicks a frame become 29.
- gl_test case 216: 40 kicks a frame become 21.

## (a) Does a render with three targets of different sizes work today?

The render this design needs, for source level l:

- slot 0 is level l+1;
- slot 1 is level l+2;
- slot 2 is level l+3;
- the frame is on level 1's grid, as every level frame is today;
- one full-screen quad over level l+1's extent.

| Part | Today's code | Works for three mip levels? |
|---|---|---|
| HWRT set and tile walk | The frame declares level 1's size, `d.target_w/h = hwrt.max(dst)` (`src/pvr/backend.rs:6151-6152`). That becomes the kick's size (`:8753-8754`), and the driver keeps one HWRT set per size (`hwrt_get`, `driver/rust/src/gpu/pvr/render.rs:703`; `HWRT_MEMBERS` = 4, `:85`). The ISP walks every tile of that grid: 475 tiles for 400x300 and 64 for 128x128 (`am62p-slow-terrain.md:191`). | **Yes.** Levels l+2 and l+3 are smaller than l+1, which is no larger than level 1, so all three lie inside the grid. The chain keeps its one HWRT set, so the pair recycling of 2026-10-16 does not return (`:6124-6131`). Each render still walks all the grid's tiles, as each level kick does today. |
| Store clip, slot 0 | `record_mip_level` sets `mip_clip` = the level's size (`:6148`). `frame_begin_with` moves it to `frame_clip` (`:5227-5229`). At flush, slot 0's maxclip is `frame_clip` (`:8903-8916`). | **Yes**, unchanged. |
| Store clip, slots 1 and 2 | Slot 1 and slot 2 each take their maxclip from their own `targets[slot].w/h` (`:8904-8905`, `:8911-8912`), in PBESTATE_REG_WORD1 maxclip_x/y (`pbe_reg_words`, `:918-928`). | **Yes.** A secondary target given its level's GL extent stores only that extent. |
| Pitch, slot 0 | `b.target_pitch = draw_pitch(d)` (`:5230`; `src/backend.rs:1266-1272`), which is `d.target_pitch` = the level's pitch (`:6153`). | **Yes.** |
| **Pitch, slots 1 and 2** | `let pitch = if slot != 0 { tw.wrapping_add(1) & !1 } else { b.target_pitch }` (`:8918`), the target's width rounded up to even. `GlesDrawTarget` has no pitch field (`src/backend.rs:813-825`; C mirror `src/backend.h:327`). | **No, for NPOT chains.** The storage pitch of level l is max(pw >> l, 1) for the power-of-two-rounded width pw (`mip_chain_layout`, `:6095-6111`). For the 800x600 chain (pw = 1024), widths and pitches are 200/256, 100/128, 50/64, 12/16 and 6/8 at levels 2, 3, 4, 6 and 7, so a secondary target would store its rows 56, 28, 14, 4 or 2 texels short of the next row. At level 9 (w 1, pitch 2, one row) the linestride is the same either way. Every level of a POT chain (256x256, 64x64) has pitch = w, so those chains would work as they are. **Missing.** |
| Targets handed to the frame | `frame_begin_with` sets `num_targets` (up to `PVR_BACKEND_MAX_RT`) and copies `d.targets[1..]` (`:5234-5239`). | **Yes.** |
| PBE state and source registers | For each target, `pbe_emit_words(addr, fmt, spos, j)` (`:8616-8678`, `:1014-1052`). An RGBA8 target takes one output register, so the three targets read registers 0, 1, 2 (`surf_out_dwords`, `:994-1000`), within the four (`:8638`, `ROGUE_CR_PIXEL_WIDTH_4REGISTERS` at `:8767`). | **Yes.** |
| End-of-tile program for 3 targets | `pvr_usc_eot3_words` (`driver/src/pvr_usc_eot_blob.h:72-76`, indexed by count at `:106-110`) is uploaded once per count (`driver/src/pvr_render.c:530-541`). Its DOUTU is handed over as `eot_doutu_src0_n[3]` (`pvr_render.c:483-490`; `driver/src/pvr_render.h:57`, `:71-74`; `src/osal/render.rs:20`, `:45-46`). The backend picks it by `nrt` (`src/pvr/backend.rs:8719-8725`). The PDS data always carries `PVR_BACKEND_MAX_RT` pairs (`:8728-8738`). | **Yes.** No driver change is needed. |
| Background clear of 3 targets | The level frame clears its background (`d.do_clear`, `:6158`). With 2 or 3 normalised 8-bit targets, `mrt_clear_dwords` answers 3 (`:2036-2046`), and the frame uses `bg_kick3_off` (`:8937-8941`) and fills all three clear registers (`:8780-8783`). | **Yes.** |
| Fragment program | `mipgen.frag` has one output, `o_color = texture(u_src, gl_FragCoord.xy * u_scale)` (`tools/shaders/internal/es/mipgen.frag:20-23`). It is uploaded as `mipgen_fs` (`src/pvr/backend.rs:1625`, `:3229`, `:3278`). The GC7000 also uses `mipgen.frag` (`tools/shaders/precompiled.list:192`). | **Missing**: a new program with three outputs (part (b)). It must be a new file, because `mipgen.frag` is shared with the GC7000. |
| The draw | `record_mip_level` fills `target_*` alone, never `targets[1..]` or `num_targets` (`:6149-6155`). Its block is two floats (`:6143`, `:6192-6193`; `FbLoad::mip_scale`, `:1903`). | **Missing**: a draw that sets three targets and a block of 12 dwords. |
| The grouping | `for l in 1..=last { record_mip_level(..) }` (`:6268-6278`). | **Missing**: level 1 alone, then groups of up to three. |
| Frame boundaries | Each mip draw has its own `target_gpu`, so `frame_ends_here` (`:5724`) closes the frame before it, as today. | **Yes.** |

Not missing, but never run on a board:

- One render whose emits have **different** maxclips and pitches. The
  3-target renders that have run on the board are gl_test 106 `mrt3`
  (`:2030-2035`, `:8776-8779`), with targets of one size.
- E1's tail patch is the first internal draw with two emits of different
  sizes (`pvr-mip-tail-e1.md:62-65`, `:768-770`), and it has run on the host
  only.

A fault there would show as a hang or a wrong probe in case 216, so case 216
runs first and alone, as in E1's procedure.

### The missing pitch: two ways

1. **A field `pitch` in `GlesDrawTarget`.** It is an ABI change:
   - the Rust struct (`src/backend.rs:813-829`) and its C mirror
     (`src/backend.h:327`);
   - 42 struct literals in 9 files (`grep -rc 'GlesDrawTarget {' tests src`:
     `src/pvr/backend_tests.rs` 20, `src/vivante/backend_tests.rs` 8,
     `src/v3d/backend_tests.rs` 4, `src/backend.rs` 3, `src/gl/draw.rs` 2,
     `tests/es3_pvr_modf.rs` 2, and one each in `src/pvr/backend.rs`,
     `src/vivante/mipmap.rs` and `src/v3d/state.rs`);
   - a pitch to fill in from the GL core's `resolve_targets` for each
     attachment.

   This is the case of CLAUDE.md's first failure, and it touches files of
   #325, #310 and #308.
2. **A pitch kept inside the backend, as `mip_clip` is.** Add a field
   `mip_pitch: [u32; PVR_RENDER_MAX_RT]` beside `mip_clip`
   (`src/pvr/backend.rs:1651`). The mip draw sets it, and `frame_begin_with`
   moves it into a `frame_pitch` and zeroes it, as `:5228-5229` does for the
   clip. `:8918` then becomes `if slot != 0 && b.frame_pitch[slot] != 0 {
   b.frame_pitch[slot] } else if slot != 0 { tw.wrapping_add(1) & !1 } else
   { .. }`. Only `src/pvr/backend.rs` changes, and no app draw is affected:
   the field is zero outside a mip frame.

**Way 2 is the design.** It is what E1's patch already does for its clip
(`pvr-mip-tail-e1.md:291`).

## (b) The fragment program: `mipgen3.frag`

Inputs:

- `u_src`, level l bound alone: one level, linear filters, CLAMP_TO_EDGE, as
  `record_mip_level` binds it (`src/pvr/backend.rs:6170-6187`).
- A 12-dword block: `u_l1`, `u_l2`, `u_l3` = (1/W, 1/H, W, H) for levels
  l+1, l+2 and l+3, each 1/W and 1/H computed in f32 exactly as `:6143` does
  (`1.0 / dst.w as f32`).
- When the render has two targets, `u_l3` is zero and output 2 is not
  emitted. The render {8} of a 256x256 chain has one target and stays a
  `record_mip_level` kick.

With `Q(v)` the 8-bit pack and `T(L, c)` the TPU's bilinear sample of level
L at c, pixel (x, y) (`gl_FragCoord.xy` = (x + 0.5, y + 0.5)) writes:

| Output | Level, extent | Taps of level l | What it computes |
|---|---|---|---|
| `o0` | l+1, W1 x H1 (the quad's viewport) | 1 | `texture(u_src, gl_FragCoord.xy * u_l1.xy)`: `mipgen.frag`'s statement, with the same sampler, the same coordinate bits and the same output pack, so `Q(T(L_l, c1))` is **byte-identical by construction**. This is the 2x box on an even step and the bilinear sample of the nearest four on an odd one (`mipgen.frag:7-12`). |
| `o1` | l+2, only if x < W2 and y < H2 | 4 | Today's level l+2 is `Q(T(L_{l+1}, c2))` with c2 = (x + 0.5, y + 0.5) * u_l2.xy. In the ALU: u = c2 * (W1, H1) - 0.5, i0 = floor(u), f = u - i0. The four corners i0 + {0,1}^2 are clamped to [0, W1-1] x [0, H1-1]. Each corner's value is a leaf: `Q(T(L_l, (i + 0.5) * u_l1.xy))`, computed as `o0` would compute it at that pixel, with (float(i) + 0.5) exact, so the coordinate bits are the level kick's. The result is `Q(mix(mix(t00, t10, f.x), mix(t01, t11, f.x), f.y))`. On an even step f = 0.5 and this is the 4x box of level l, rounded at each step as the iterated chain rounds. On an odd step it is the bilinear of today's per-level kick, with the TPU's arithmetic replaced by the ALU's. |
| `o2` | l+3, only if x < W3 and y < H3 | 16 | The same bilinear over four level-l+2 values. Each of those is `o1`'s computation at its clamped corner (4 leaves each), so the result is the 8x box of level l on even steps. The 16 leaves are the 4x4 block of level l+1 on an even step. On an odd step neighbours may share leaves, but the program does not deduplicate them. |

Rules the program must keep:

- **The leaves outside uniform control flow, or `textureLod`.** `o1` and
  `o2` sample inside a branch on gl_FragCoord, so their 20 taps are
  `textureLod(u_src, c, 0.0)`. With a one-level sampler whose min and mag
  filters are both linear, that selects the same filter as `texture()`
  (`pvr-mip-tail-e1.md:729-733`, E1's leaf). `o0` stays `texture()`
  outside any branch, word for word as `mipgen.frag` writes it.
- **The intermediate `Q` is the output pack's rounding.** Today each level
  goes to memory through our compiler's pack (`source_format` 8 per channel,
  `src/pvr/backend.rs:1037-1041`; `pck.u8888` with `rtz = 0`, round to
  nearest, `src/shaderc/online/powervr/assemble.rs:1189`, as
  `pvr-mip-tail-e1.md:628-630` cites it). The program's `Q` must round ties
  the same way. On an even step a tie happens whenever a + b + c + d = 2
  mod 4, so even steps are affected too. The program writes `Q` as one
  macro, `floor(v * 255.0 + 0.5)` (E1's choice, `pvr-mip-tail-e1.md:492`) or
  `roundEven(v * 255.0)`. Which one is decided by E1's board read-back
  (`pvr-mip-tail-e1.md:693-715`). If it cannot be read, the host test pins
  the macro and the board's dEQP run decides (part (e)).
- **Odd steps take E1's coordinate arithmetic.** `u = (p + 0.5) * u_lk.xy *
  (W, H) - 0.5`, then floor, the fraction and the clamp: E1's `DOWN` macro
  (`pvr-mip-tail-e1.md:480-487`). Its host evaluation read 0 texels
  differing against the iterated chain when the TPU is modelled as that
  arithmetic, and at most 1 LSB in 800x600 levels 6 to 8 under the `gl`
  and `w8` models (`pvr-mip-tail-e1.md:600-626`). The NPOT odd steps of
  terrain's 800x600 chain, from `am62p-pvr-mipmap-kicks.md:64-67` and the sizes
  `mip_chain_layout` gives:
  - level 4: 75 -> 37 in y;
  - level 5: 37 -> 18 in y;
  - level 6: 25 -> 12 in x;
  - level 7: 9 -> 4 in y;
  - level 9: 3 -> 1 in x.

  Every other step is even in both axes.
- **No loop, no array.** The 21 taps are straight-line code under two
  branches, unlike E1's loop with a per-pixel trip count and 219 temporaries
  (`pvr-mip-tail-e1.md:32-37`, `:106-108`). The register count is the
  generated record's. The host test reads it and fails above a bound (part
  (e)). `mipgen.frag` takes 26 temporaries (`pvr-mip-tail-e1.md:107-108`).
- **Three outputs in the record.** `fs.num_outputs` must read 3, or the
  frame logs "colour attachments but the fragment shader writes N output(s)"
  (`src/pvr/backend.rs:7490-7497`). The host test pins it.

Where the levels come from, by chain (`TPU` = byte-identical by
construction, `ALU` = the program's bilinear over TPU leaves):

| Chain (GL size, levels above 0) | Render 1 | Render 2 (from 1) | Render 3 (from 4) | Render 4 (from 7) |
|---|---|---|---|---|
| 800x600, 9: 400x300, 200x150, 100x75, 50x37, 25x18, 12x9, 6x4, 3x2, 1x1 | 1 TPU | 2 TPU, 3 ALU (even step), 4 ALU (odd in y) | 5 TPU, 6 ALU (odd in x), 7 ALU (odd in y) | 8 TPU, 9 ALU (odd in x) |
| 256x256, 8: 128 .. 1 | 1 TPU | 2 TPU, 3 ALU, 4 ALU | 5 TPU, 6 ALU, 7 ALU | 8 TPU (`record_mip_level`) |
| 64x64, 6 (dEQP-GLES2's generate cases are of this kind) | 1 TPU | 2 TPU, 3 ALU, 4 ALU | 5 TPU, 6 ALU | - |

Shading work of the 800x600 chain's render 2:

- 30,000 pixels of level 2 at 1 tap, 7,500 of level 3 at 4 taps and 1,850
  of level 4 at 16 taps: 89,600 TPU taps;
- today's three kicks for those levels sample 39,350 times.

It is the heaviest render of a chain, and its extra work is about
50,000 taps on a GPU that is busy 0.19 ms of a 1.1 ms kick
(`am62p-slow-terrain.md:139`, `:202-205`). Prediction: under 0.3 ms more
GPU time a chain. The case 216 `TA|3D` figure measures it (part (f)).

## (c) The code change: files, order, and who holds them

The order of the commit, all in one commit (the generated file moves with
its source, CLAUDE.md):

1. `tools/shaders/internal/es/mipgen3.frag` (new): the program of part (b).
2. `tools/shaders/precompiled.list`: one line, `internal powervr mipgen3
   internal/es/mipgen3.frag`, after `mipgen` (`:181`). It is not a `gc7000`
   line.
3. `src/pvr/backend.rs`:
   - `mipgen3_fs: OsalGpuMem` beside `mipgen_fs` (`:1625`), uploaded in
     `static_objects_init` (`:3229`), and added to the shutdown list
     (`:3473`) and the pool owners list (`:9389`);
   - `FbLoad::mip3_fs_sh` and `mip3_u: [f32; 12]` beside `mip_fs_sh` and
     `mip_scale` (`:1902-1903`, set at `:3278`). These are the same four
     places E1's patch touches (`pvr-mip-tail-e1.md:148-191`, `:373-380`);
   - `mip_pitch`, `frame_pitch` and the `:8918` change of part (a);
   - `record_mip_levels3(p, base, lv, l, m)`, a sibling of `record_mip_level`
     (`:6138-6202`). It sets slot 0 as today, `num_targets = m`,
     `targets[1..m]` with each level's address (`base + off`), GL extent
     (w, h) and `GLES_SURF_RGBA8888`, `mip_pitch[1..m]` with the levels'
     pitches, and the 12-dword block;
   - `mip_chain_gpu`'s loop (`:6268-6278`) draws level 1 by
     `record_mip_level`, then groups of min(3, n - l) levels by
     `record_mip_levels3`, or by `record_mip_level` for a group of one. The
     grouping applies only on the all-GPU arm (`last + 1 == n`). The path-B
     prefix (`mip_cpu_tail`, `:6234-6236`, `:6285-6294`) keeps one kick a
     level, so `MipChain::Prefix(n)` keeps its meaning.
4. `src/pvr/internal.rs`: regenerated by `cargo run -q -p yakogl-precompile
   --features gpu-powervr -- --internal -o src/pvr/internal.rs`, then
   checked with `--verify`. The library must build before the new row
   exists, so use E1's bootstrap order (`pvr-mip-tail-e1.md:115-125`). The
   gate row is `gen src/pvr/internal.rs == yakogl-precompile --internal`
   (`tools/check.sh:3105`).
5. `src/pvr/backend_tests.rs`: new rig tests, part (e). These tests pin one
   kick a level and change in the same commit
   (`pvr-mip-tail-e1.md:527-534`):
   - `a_64x64_chain_is_drawn_level_by_level_into_the_storage`;
   - `an_800x600_chain_is_drawn_as_the_power_of_two_chain_it_is_laid_out_in`;
   - `a_chain_asked_for_during_a_frame_into_level_0_follows_that_frames_kick`;
   - `asking_the_chain_back_waits_for_its_last_kick`;
   - `chains_the_gpu_cannot_make_are_left_to_the_cpu_by_name`;
   - their helper `level_draws` (`:5835`).
6. `tests/mipmap_offer_guard.rs`: the three
   `pvr::*_gets_its_chain_from_the_gpu` tests count one kick a level
   (`pvr-mip-tail-e1.md:535-536`).
7. `tests/pvr_mip3_levels.rs` (new): the level-content test of part (e).
8. If they are on main by then:
   - `tests/pvr_terrain_kicks.rs` (item pvr-terrain-kick-pin): 52 kicks a
     frame becomes 29;
   - `tests/pvr_mip_chain_pin.rs` (item mip-pin-pvr): the bytes of the ALU
     levels 3, 4, 6, 7 and 9 may move.

   Either is updated in the same commit, never silenced.

Not changed:

- `driver/**`: the EOT and PDS for 3 targets exist (part (a)).
- `GlesDraw` and `GlesDrawTarget`, so none of the 42 literals change.
- `src/gl/**`: the GL core still sees `MipChain::Whole`.
- `tests/reference/pvr_internal.rs`: `tools/glsl2usc.sh` writes only the
  `fbload` blocks into it (`tools/glsl2usc.sh:50-62`), and the mip program
  has no Mesa twin (`tests/pvr_mipgen_program.rs:18-20`). This also means
  the change needs no recorded pco_shc and glslang toolchain.

Who holds them while their plans run (plan #329's goal text):

| File | Held by |
|---|---|
| `src/pvr/backend.rs`, `src/pvr/backend_tests.rs`, `src/pvr/internal.rs` | **#325** (PowerVR ES 3.0 host work); `src/pvr/**` also **#301** (partial render) |
| `tests/pvr_terrain_kicks.rs`, `tests/pvr_mip_chain_pin.rs` | this plan's running items pvr-terrain-kick-pin and mip-pin-pvr |
| `tools/shaders/internal/es/mipgen3.frag`, `tools/shaders/precompiled.list`, `tests/mipmap_offer_guard.rs`, `tests/pvr_mip3_levels.rs` | no listed plan. E1's note counts `tools/shaders/**` with #325's set (`pvr-mip-tail-e1.md:12-14`), and the list line and `internal.rs` must move in one commit, so the change waits for #325 in any case |

What must come before it:

- #325 and #301 release `src/pvr`.
- E1's board session (item pvr-mip-tail-e1-board) answers whether the ALU
  bilinear equals the TPU's, and how ties round. Part (b)'s `Q` macro takes
  that answer.
- pvr-terrain-kick-pin and mip-pin-pvr land, so this change moves pinned
  numbers instead of meeting unpinned ones.

## (d) Kicks a frame, before and after

The formula: a chain of n levels above 0 takes 1 + ceil((n - 1) / 3)
renders. That is 4 for n = 8, 9 or 10, and 3 for n = 5, 6 or 7.

**glmark2 terrain** (five chains a frame, `terrain-mip-usage.md:18-23`,
`:133`; the host count `am62p-slow-terrain.md:96-118`, measured at
`4370107f`):

| Chain | Levels above 0 | Kicks now | Kicks after |
|---|---:|---:|---:|
| terrain target, first build, 800x600 | 9 | 9 | 4 |
| bloom H, 256x256 | 8 | 8 | 4 |
| bloom V, 256x256 | 8 | 8 | 4 |
| terrain target, second build, 800x600 | 9 | 9 | 4 |
| tilt-shift H, 800x600 | 9 | 9 | 4 |
| **mip kicks a frame** | | **43** | **20** |
| passes (8) and the window's dead clear (1) | | 9 | 9 |
| **kicks a frame** | | **52** | **29** |

The predicted `kicks.py` line after the change is `fb1+2 fb2+1 fb4+1 fb5+5
fb6+5 fb4+5 fb7+5 fb0+5`. Now it is `... fb5+10 fb6+9 fb4+9 fb7+10 fb0+10`
(`am62p-slow-terrain.md:102`).

The board's 52.7 kicks a frame (`ce3354ac`, 5802 kicks over 110 frames,
which also counts the setup) becomes about 29.7.

The setup's 512x512 specular chain (9 levels, built once) goes from 9 kicks
to 4.

For comparison, E1's tail route reaches 32 (`pvr-mip-tail-e1.md:553-559`).

**gl_test case 216** `mipchains` (four chains a frame, per
`driver/tools/pvrlab/gl_test.c:20033-20042`, `:20157-20160`; the record
`tools/cts/RESULTS.md:300-305`, main `61ad38a1`):

| Chain | Levels above 0 | Kicks now | Kicks after |
|---|---:|---:|---:|
| A 800x600 (terrain) | 9 | 9 | 4 |
| B 256x256 (bloom) | 8 | 8 | 4 |
| A 800x600 again (the overlay) | 9 | 9 | 4 |
| C 800x600 (tilt-shift) | 9 | 9 | 4 |
| passes | | 5 | 5 |
| **kicks a frame** | | **40** (record: 40.00) | **21** |

Other scenes: refract's 1600x1200 chain (10 levels above 0) goes from 10
kicks to 4.

## (e) The ES 2.0 risk and the host test that pins it

**API answers: no risk.** `tests/es2_report_guard.rs` runs a stand-in whose
`generate_mipmap` is `None` (`tests/es2_report_guard.rs:132`), so it never
reaches this chain. Its 16 tests must still pass at the change's commit (the
gate's `--changed` rows include it).

**Texel answers on the board.**

- In scope: dEQP-GLES2 `functional.texture.mipmap.2d.generate.rgba8888_fastest`,
  `_nicest`, `_non_square_fastest` and `_non_square_nicest`
  (`tools/cts/gles2-mustpass.txt:14907-14910`).
  - They are 4 of the 32 `generate.*` cases. They are the only ones the GPU
    chain takes: plain RGBA8 2D (`src/pvr/backend.rs:6242-6247`).
  - The a8, l8, la88, rgb565, rgb888, rgba4444 and rgba5551 cases, and every
    cube case, stay with the GL core's CPU box.
  - Any other `texture.mipmap.2d.*` or `texture.filtering.*` case that calls
    `glGenerateMipmap` on an RGBA8 2D texture also changes, as soon as its
    chain has 3 or more levels above 0 (render 2 exists).
- The levels that can move are the ALU levels of part (b): 3, 4, 6 and 7 of
  a 64x64 or 256x256 chain, and 3, 4, 6, 7 and 9 of an 800x600 chain.
  - They move only where the TPU's filter differs from the ALU's, or where
    ties round differently than `Q`.
  - On POT chains every weight is exactly 1/2, and E1's host models read
    0 texels differing on them (`pvr-mip-tail-e1.md:624-626`). The one open
    point there is the tie rounding.
- ES 2.0 asks for no particular filter (`pvr-mip-fewer-kicks.md:110`), and
  dEQP compares with a threshold that allows a filter's rounding, so a 1-LSB
  move is expected to pass. A failure means the program is wrong by more
  than that.
- **Larger than E1's risk.** Levels 3 and 4 of an 800x600 chain are 100x75
  and 50x37. In E1 they stay TPU levels.
- **Terrain's picture is unaffected.** The per-frame chains are read at
  levels 0-2 only (`terrain-mip-usage.md:24-28`, `:144-148`), and those are
  TPU levels here. The 512x512 specular chain is read at every level
  (`:149`), so a 1-LSB move there would show only as a sub-tolerance change
  (column `am62p`, tolerance 8).

The host tests, in order of what they catch:

1. **`tests/pvr_mip3_levels.rs`** (new), with the features `host-test`,
   `gpu-powervr` and `shaderc-online`. It is built on E1's appendix B
   (`pvr-mip-tail-e1.md:580-626`, `:837-`):
   - It compiles `mipgen3.frag` with the online front end, checks the
     block's offsets (`u_l1` 0, `u_l2` 4, `u_l3` 8), and checks that the
     record has 3 outputs and stays under a temporary bound.
   - It runs the module with the reference IR evaluator
     (`src/shaderc/online/eval.rs`, `run_in_with_textures`) at every pixel
     of level l+1, with a `TexelSource` that answers the leaves.
   - It compares levels l+1, l+2 and l+3 against today's chain (one kick a
     level, each the same TPU model's sample of the level above, packed by
     the same `Q`).
   - Sizes: the dEQP shapes, square and non-square POT chains (64x64,
     64x32, 32x64, 128x128), and terrain's 800x600 and 256x256, with
     random and smooth level-0 content.
   - Pins:
     - (i) levels 1, 2, 5 and 8 (each render's `o0`) byte-equal to today's
       in every TPU model;
     - (ii) every level byte-equal under the `alu` model;
     - (iii) under the `gl` and `w8` models, max |d| <= 1 and the count of
       differing texels per level printed and bounded;
     - (iv) a pixel outside a level's extent writes nothing (the PBE's clip
       does that on the board; the evaluator checks the branch).

   The figures (iii) bounds are the ones item mip-multilevel-error measures.
   `tools/hostharness/mip-multilevel/` is not on main at `0afc6847`, so its
   error figures come from that item, not from this note.
2. **`src/pvr/backend_tests.rs`** rig tests (the backend's recorded words,
   as E1's appendix A, `pvr-mip-tail-e1.md:565-578`):
   - 800x600: 4 kicks; render 2 has `num_rt` 3, emits at the addresses of
     levels 2, 3 and 4, and `pbe_reg_words(RGBA8888, w, h, pitch)` with
     pitches **256, 128, 64** (not 200, 100, 50: the `:8918` fix), maxclips
     200x150, 100x75 and 50x37, on level 1's 400x300 grid; render 4 has
     `num_rt` 2.
   - 256x256: 4 kicks, and render 4 is a `record_mip_level` kick.
   - 64x64: 3 kicks.
   - All three: the fragment program is `mipgen3_fs` on renders 2 and up,
     the block holds each level's `1/w, 1/h` exactly as `:6143` makes them,
     and every render of a chain declares one target size.
   - The path-B arm (`mip_cpu_tail` on) still records one kick a level.
   - An app draw after a mip render gets the derived pitch again
     (`frame_pitch` cleared).
3. **`tests/es2_report_guard.rs`** unchanged and green.
4. **`tests/pvr_mipgen_program.rs`** unchanged and green (`mipgen.frag` is
   unchanged).

## (f) The board record values that confirm it

From images built from origin/main commits, with `target:am62p` and
`target:am62p-console`.

- **Before**: the commit before the change; am62p-board-before's baseline
  serves if it is the same commit.
- **After**: the change's merge commit.
- **c**: the marginal cost of one queued render, from gl_test case 219
  `kickpair` (`two - one`, `pvr-mip-tail-e1.md:666`). The record's figure is
  about 1.15 ms (`pvr-mip-fewer-kicks.md:31-35`).

| Record value | Before (last record) | After, expected (prediction) |
|---|---|---|
| gl_test 216 `gl_test:   per frame: N kicks (M over 20 frames)` (`gl_test.c:20189-20193`) | 40.00 (`61ad38a1`) | **21.00** |
| gl_test 216 `gl_test: mipchains: 20 frames, X ms a frame` (`:20179`) | 46.15 | 46.15 - 19 x c: about **24 ms** at c = 1.15, or about 34 ms if a kick is 0.65 ms plus one 20 ms fence a frame (`am62p-slow-terrain.md:212-215`) |
| gl_test 216 `per frame: TA busy, 3D busy, TA\|3D` (`:20184-20187`) | TA\|3D 7.75 ms | about the same: fewer geometry phases, more taps in render 2 (part (b)) |
| gl_test 216 `mipchains: reposts N over 20 frames` (`:20196`) | read | not more |
| gl_test 216 four quadrant probes, `case 216 mipchains: PASS` | PASS | **PASS**. The probes see only quadrant centres (`gl_test.c:20052-20054`, `:20203-20214`), so they catch a hang or a broken store (a wrong pitch shears the level), not a 1-LSB move |
| glmark2 terrain ms a frame (`board-stage.sh am62p common` or `board.sh am62p gm`, per-scene list; `FPS:`) | 83.06 ms (`6a7021ec`, `tools/cts/RESULTS.md:83`), or am62p-board-before's at the current main | 83.06 - 23 x c: about **57 ms** at c = 1.15 (`am62p-slow-terrain.md:254`: about -25 ms) |
| glmark2 refract ms a frame | 35.09 ms | lower by about 6 x c: about 28 ms |
| `glmark2-frame: [terrain]` gl and swap us | read | gl time falls with the kicks |
| `render: fence N (frame fence) not moved at 20 ms, re-posted ...` (M so far) | read | not more |
| GL errors, incomplete draws, `verdict=done`, resets | 0, 0, done, 0 | **the same** |
| pictures, column `am62p` of `tools/cts/glmark2-pictures.txt`, tolerance 8 | 28 match / 5 mismatch (animated probes), `6a7021ec` | **the same split**; terrain's mean `119 129 44` within tolerance |
| dEQP-GLES2 `functional.texture.mipmap.2d.*` per case (`board-stage.sh am62p deqp` or a case-list image) | read on the before commit | **the same pass and fail per case**; a new failure means an ALU level is off by more than dEQP's threshold |

**Candidate 2 is confirmed when** all of these hold:

- case 216 reads 21.00 kicks and PASS;
- its ms a frame falls by at least 19 x c - 2 ms (the 2 ms is slack for the
  heavier renders), so with c = 1.15, by at least 20 ms, to about 26 ms or
  less;
- terrain's frame falls by at least 23 x c - 3 ms;
- the dEQP set and the pictures are unchanged.

If the kicks fall and the milliseconds do not, the frame is not kick-bound:
it is the waits, as path B showed (`am62p-pvr-mipmap-kicks.md:201-210`).

A level-by-level read-back on the board, like E1's step 3
(`pvr-mip-tail-e1.md:703-715`), would turn "the same dEQP answers" into
"differing bytes N, max |d| M" per ALU level. It is a new gl_test case, not
part of this change.

## Open

- How ties round in the pack, and whether the TPU's bilinear equals the
  ALU's on odd steps. E1's board session answers both, and part (b)'s `Q`
  macro takes the answer.
- Whether a render whose emits have different pitches and maxclips stores
  correctly on the board. It is first run by E1's patch (two emits) or by
  this change (three).
- The combination with E1's tail, {1}, {2-4} and a tail kick for 5..9,
  would be 3 kicks a chain (15 mip kicks a frame, 52 -> 24). It is not
  designed here.
