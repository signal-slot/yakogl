<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pvr-msaa-split, 2026-10-15, branch ccfarm/01a13ef4-1325-7212-b3f7-d66bba5e0da8

## Known defects

- **pvr-msaa-split** (no issue yet): fixed on the host, unmeasured on the
  board. A 4x frame the PowerVR splits into two kicks (a full arena, the
  1024-draw cap, or a glFinish / glReadPixels before more drawing) now keeps
  its samples: a multisampled kick that is not ended by a swap emits each
  tile twice, resolved into the target as before (PBE REG_WORD0
  `downscale`) and unresolved into a sample store the back end owns
  (`MsStore`, `src/pvr/backend.rs`: the TPU's multisampled image of the
  target, `smpcnt` = log2 of the samples over the target's own width,
  height and stride, four images' worth of bytes from the render pool). The
  next kick's load quad binds the store in place of the target, with the
  same load program, at the full sample rate, so each instance reloads its
  own sample. A swap's kick stores the resolved colour only, and a frame
  drawn into the surface after it reloads that, as before. The 1x path and
  the dEQP-GLES2 answers do not change.
  `a_split_multisampled_frame_reloads_each_sample_not_the_resolved_colour`
  (`src/pvr/backend_tests.rs`) is no longer ignored; beside it
  `a_4x_kick_before_a_readback_stores_its_samples_and_the_next_kick_loads_them`,
  `a_swap_kick_keeps_no_samples_and_the_next_frame_reloads_the_resolved_colour`
  and `a_4x_frame_split_at_the_draw_cap_or_a_full_arena_keeps_its_samples`
  pin the two emits, the store's binding and the rates. Two things are
  chosen from the vendored headers, not measured: that the PBE's unresolved
  store (pbestate.xml:255, `downscale` off) is laid out as the TPU's
  `smpcnt` image (texstate.xml:273) over the same stride, and that a
  per-sample fetch of that image reads the running instance's sample. The
  line "Known defects" keeps this entry until the board check below passes;
  then it closes.

## Next board session

AM62P, the ms4 image (`rgba8888d24s8ms4`), after the GLES2 ms4 row. One
measurement decides pvr-msaa-split, in `driver/tools/pvrlab/gl_test.c` as
a new case next to 16 (the 4x diagonal triangle) and 173 (`splitzs`):

1. Clear black. Draw case 16's white triangle (its hypotenuse the window
   diagonal) into the 4x pbuffer. `glFinish()` (the frame splits: kick 1
   stores the samples). Draw the same triangle again, `glReadPixels`.
2. Expected: the pixels on the diagonal read `0x7f` (two of four samples
   white), interior `0xff`, exterior `0x00` -- the same picture as one
   unsplit draw. The console shows `kick 1: ... 1 draws, 0 quads, samples`
   and `kick 2: ... rt 1, 1 draws, 1 quads, samples`.
3. The readings that name a wrong choice: `0xbf` on the diagonal means the
   store held the resolved colour in every sample (the unresolved layout
   is not the `smpcnt` image over the target's stride: next, a 2w x 2h
   store with twice the pitch, the ZLS's 2x2 spread); `0xff` or `0x00` on
   the diagonal means every instance read one sample (the per-sample fetch
   needs the sample number: `gl_SampleID` in the load shader, SMP `sno`);
   a BIF fault or a hang on kick 1 means the second emit's address or
   extent is wrong (the kick log's `pbe1` words).
4. Then the ms4 row again (`tools/cts/am62p-ms4-tree.txt`): 16 / 16
   expected, as on `452d6c31`.
