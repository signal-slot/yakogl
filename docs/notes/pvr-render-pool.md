<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The PowerVR render pool

Status: finished record, moved here from `driver/docs/pvr-render-pool.md` on 2026-10-11. Runs and designs of 2026-10-03 to 10-05 on the AM62P. The pool's policy as it is now (free lists in gpu::pvr::pool, the top region, reclaim, frame sets) is driver/docs/pvr-render.md. The files its opening cites, driver/src/pvr_render_pool.h and tools/pvr/render_pool_check.c, are gone; the pool is driver/rust/src/gpu/pvr/pool.rs.

Account of the render pool on the AM62P at 5398fb14. It was written for the
glmark2 item (`build/glmark2-am62p-20261003-230937` in the main checkout:
"render pool exhausted" in terrain, then shadow and refract fail). Every
claim cites the code. Statements about the board run say so. Mechanisms that
have not been measured are labelled as hypotheses.

## What it is

- The carveout is 64 MiB (boot line "GPU carveout phys 0x97800000 (64 MiB)").
  The render pool is its tail after the firmware heap and the page tables:
  57.8 MiB (`driver/src/pvr_internal.h:57-67`; `bench/glmark2:driver/docs/glmark2.md` section 5).
- One **physical** cursor, `k.pool_next`, is shared by every heap
  (`driver/rust/src/gpu/pvr/mem.rs:156`). Each heap has its own **device-VA**
  cursor (`gen_b`, `pds_b`, `usc_b`, `pm_bump`).
  `bo_alloc` (`mem.rs:141`) bumps both. When the physical cursor would pass
  the carveout's end, it prints "render pool exhausted: N KiB requested, M KiB
  left" (`mem.rs:150`) and returns nothing. "Left" is what remains above the
  physical cursor. It is not free memory.

## Who allocates

| user | how | site |
|---|---|---|
| parameter-buffer freelists (backing pages, PFN arrays) | bump, at init | `driver/rust/src/gpu/pvr/freelist.rs:65-87` |
| per-size HWRT buffers (TPC, VHEAP, region headers, macrotile, MList) | bump, **never via the free lists** | `driver/rust/src/gpu/pvr/render.rs:733-751` |
| VDM control stream of the render context | bump | `render.rs:435` |
| GL objects: buffers, textures, renderbuffers, program USC code and uniforms, the backend's frame-set arenas and depth/stencil buffers | `gpu_alloc` → `z_gpu_alloc` (`src/osal/zephyr.c:91`) → `pvr_render_alloc`: free list first, bump second | `driver/src/pvr_render.c:169`, `:176` |

## Who frees, and when

1. **The GL core frees on delete.** Buffers: `src/gl/buffer.rs:346`, `:370`, `:447`.
   Textures and renderbuffers: `release_storage_with`, `src/gl/context.rs:1601-1616`.
   Programs: `src/gl/draw.rs:1113-1123`. The backend's depth/stencil buffers:
   `src/pvr/backend.rs:2695`, `:2998`, `:3062`. Pinned by `src/gl/pool_tests.rs`:
   a 4.6 MiB buffer, an 800x600 texture, and an FBO with a depth renderbuffer
   (bound and unbound, in glmark2's delete order) are each created and deleted
   16 times. The live bytes do not grow. The control test proves the
   instrument can see a leak: with every free refused, the live bytes grow
   by one buffer per round.
2. **The OSAL defers each free until a fence.** `z_gpu_free` (`zephyr.c:190`)
   queues the block with fence `s_last_submitted + 1` (`zephyr.c:204`): the
   kick being recorded at the time of the delete. The queue is
   `src/osal/pending_frees.h`. It drains only in `z_completed` and `z_wait`
   (`zephyr.c:398`, `:415`). Then `pvr_render_free` returns the block.
   The board run printed no "a free refused" line, so the queue never
   overflowed.
3. **The pool keeps a free list per heap** (`driver/src/pvr_render_pool.h`):
   best-fit allocation (`:282-294`) and merging with both neighbours on free
   (`:337`, `:429`). Two blocks merge only when they are adjacent in **both**
   VA and CPU space. Giving a block back to the bump (`bo_rollback`,
   `mem.rs:182`) is opt-in and off by default (`pvr_render.c`,
   `pvr_render_pool_rollback`).

So the pool is not a bump with no free path. The memory of a deleted GL
object does go back, after a fence.

## Why the glmark2 sequence drains it

The run printed no pool summary, so the three mechanisms below cannot yet
be told apart from the console. Each one is derived from the code and is a
hypothesis until a board run measures it.

1. **Capacity.** Terrain's own working set is about 30 MiB at 800x600
   (`scene-terrain.cpp:62-120` in glmark2 22c527cb). The mesh VBOs alone are
   3 x 4.6 MiB + 3 MiB: these are the four refused `glBufferData` requests.
   Then come 800x600 targets with depth, mip chains, and three 512x512
   textures. Fixed use is about 20 MiB: 2 x 6 MiB freelist backing (boot
   lines "backing=1536"), the backend's frame sets, the window and its depth.
   Terrain fits in 57.8 MiB only if almost nothing else is held.
2. **The previous scene's frees are held over the next scene's set-up.**
   Each free waits for the kick after the last one submitted (`zephyr.c:204`).
   glmark2 deletes scene N after its last swap, then creates scene N+1 before
   any kick. So none of scene N's memory is reusable while scene N+1
   allocates. `z_gpu_alloc` never drains or waits on -ENOMEM
   (`zephyr.c:91-115`). The host mock now models this hold (`held` in
   `src/testutil.rs`, separate from its stricter use-after-free fence), and
   `known_wrong_a_scenes_frees_are_held_through_the_next_scenes_setup`
   (`src/gl/pool_tests.rs`) reproduces it: in a pool that holds one and a
   half scenes, the second scene gets GL_OUT_OF_MEMORY with the first
   deleted and its kick complete. It fits once the next kick completes
   (the test's control).
3. **Fragmentation that cannot be merged.** Heaps share the physical cursor,
   and a merge needs CPU adjacency too. A PDS, USC or PM allocation made
   between two GENERAL blocks (a program compiled during a scene's set-up, an
   HWRT MList) therefore splits the GENERAL free space for good, even after
   both sides are freed. A request larger than every split piece (4.6 MiB,
   7.5 MiB) goes to the bump. The host check of the pool models
   `cpu = heap_mem[heap] + hb` (`tools/pvr/render_pool_check.c:92`), so it
   cannot see this either. Its
   reproduction comes with the pool's port to Rust, which runs on the
   driver's own shared cursor.

On the board, the physical cursor stood at 61.3 MiB of the carveout
(64 MiB - "2736 KiB left") when terrain asked for its first VBO.

## What tells them apart

The pool summary (bump high-water, free bytes and largest free block per
heap, bytes held in the pending-free queue), printed at exhaustion and at the
end of a run:

- a small total of free bytes means (1);
- a large pending-free total means (2);
- free bytes large but the largest block smaller than the request means (3).

## The summary (what tells the three apart)

`driver/rust/src/gpu/pvr/pool.rs` prints five lines, each with its own
prefix:

```text
render pool summary: <pool> KiB, bump <now> KiB now, <hw> KiB high-water
render pool general: free <KiB> in <n> ranges, largest <KiB>|none; <n> reused, <n> bumped, <n> refused; <n> frees (<KiB>)
render pool pds: ...
render pool usc: ...
render pool losses: dropped <KiB>, <n> double frees, <n> bad chain headers, rolled back <KiB>
```

When it prints:
- at ERR right after every "render pool exhausted";
- at INF at the end of a run: `osal_zephyr_diag_print()` (gl_test calls
  it; `board_finish()` calls it for every PowerVR YakoGL sample that ends
  there, glmark2 included), `z_shutdown()`, and every close of the Linux
  harness.

`src/osal/zephyr.c` adds "osal(zephyr): <when>: pending frees now N (K
KiB), oldest fence F|none, last submitted S" when an allocation is refused
(`when` = "an allocation refused") and at the end of a run ("at close", "at
shutdown").

Read at the failure:
- free KiB small in every heap means mechanism 1;
- pending KiB large means mechanism 2;
- free KiB large but "largest" below the request means mechanism 3.

## The opt-in fix for mechanism 2: `CONFIG_YAKOGL_PVR_POOL_RECLAIM`

`zephyr/Kconfig`, default n. It stays off until gl_test and glmark2 have
run with it on the AM62P.

- **The tag.** A free made while no frame is open is held only for the last
  submitted kick (`pvr_backend_free_fence`, `src/pvr/backend.rs`). The
  backend submits only from `flush`, which closes the frame. When that kick
  has already completed, the block goes back at once.
- **A refused allocation.** First, without waiting, give back every held
  block whose kick has completed; retry if that gave anything back
  ("osal(zephyr): reclaim without waiting: ..."). If it is still refused and
  a held block waits for a submitted kick: wait for the submitted kicks,
  give back what they held, and try once more ("osal(zephyr): reclaim after
  a wait: waited N ms ..."). The GL call that allocated blocks for that
  wait, inside the frame being recorded. The wait is bounded by
  `PENDING_FREES_WAIT_MS`, the backend's own submit wait. Blocks held for
  the frame being recorded stay held ("osal(zephyr): reclaim found nothing
  held for a submitted kick: ...").
- **Host tests** (`src/gl/pool_tests.rs`, on the mock's model of both
  arms):
  - a scene deleted between frames comes back at once;
  - a refused allocation waits for the submitted kick and is served;
  - a scene deleted inside a frame stays held until that frame's kick.

> Every "GPU resets 0" / "0 resets" in the board runs below came from
> `board_gpu_resets()`, which returned 0 on the PowerVR by construction until
> 17797907. Read them as "not counted" (tools/cts/RESULTS.md, the note at the
> top). From 17797907 on, the count is the firmware's HWR counter.

## The first board run (AM62P, 2026-10-04, knob off)

Run dirs `build/pool-am62p-20261004-003056/{gltest-off,glmark2-off}` in the
main checkout. Images from 0db1c28f: gl_test a0f234a0, glmark2 228b2d4e
(console lines below are verbatim). "Live" below is derived, not printed:
bump - free in all heaps - pending.

**A fixed ~36 MiB, not the ~20 MiB estimated above.** At gl_test's close:
"bump 37216 KiB", free 744 + 264 KiB, pending 64 KiB: 36144 KiB live with
no scene. At glmark2's close: "bump 58844 KiB", free 19348 + 96 KiB,
pending 1996 KiB: 37404 KiB live. So about 22 MiB of the 59200 KiB pool is
left for a scene. What holds the 36 MiB is not yet accounted for item by
item. The 8 PDS blocks never freed, the freelist backing and the HWRT sets
are in it. That accounting is the next item for mechanism 1.

**Terrain (set-up, 00:36:13).** The first refusal:
- "render pool exhausted: 4608 KiB requested, 2736 KiB left"
- "render pool summary: 59200 KiB, bump 56464 KiB now"
- "general: free 3764 KiB in 7 ranges, largest 2960 KiB"
- "pending frees now 45 (11576 KiB), oldest fence 61283, last submitted
  61282"
All 45 are tagged with the unsubmitted kick: jellyfish's teardown after its
last swap (mechanism 2). Live 41048 KiB.

Terrain's refused requests alone are 3 x 4608 + 3072 + 2 x 1876 = 20648 KiB,
while 59200 - 41048 = 18152 KiB is all the pool could hand out at that
point, even with every held and free byte in one range. So terrain is
mechanism 1, with 2 on top: returning the held frees cannot make it fit.

**Shadow is not a pool failure.** Its set-up fails with
"DepthRenderTarget::setup: glCheckFramebufferStatus failed (0x8cdd)"
(FRAMEBUFFER_UNSUPPORTED). No "render pool exhausted" line comes between
terrain's end and that line. That is its own item (the depth-texture
attachment).

**Refract (00:36:28).** Both 7500 KiB refusals are refract's two 1600x1200
textures ("texture 1600x1200 x1 fmt 7 ... out of GPU memory", then fmt 0).
- "general: free 5664 KiB in 8 ranges, largest 2056 KiB"
- "356 KiB left"
- "pending frees now 1 (7500 KiB), oldest fence 61652, last submitted
  61651"
The one held block has exactly the size of the request, and no kick ran
since shadow's failed set-up. That makes it, most likely, shadow's
1600x1200 attachment freed at its teardown (mechanism 2). Live 45480 KiB.

The two textures need 15000 KiB, in two contiguous 7500 KiB ranges. At
most 5664 + 356 + 7500 = 13520 KiB is there in total, and the free part is
in pieces of 2056 KiB or less (mechanism 3). So refract is 1 + 2 + 3:
returning the held block serves one texture, not both.

**Prediction for the knob-on run (glmark2 9618a370),** written before its
lines were read:
- Terrain: still GL_OUT_OF_MEMORY in its set-up (1). Jellyfish's 45 frees
  are tagged with the last submitted kick and come back by terrain's
  set-up, so fewer of its requests are refused, or later ones. At least
  2496 KiB (20648 - 18152) of them cannot be served.
- Shadow: unchanged, 0x8cdd. Not the pool.
- Refract: still fails at set-up (0x8cd6). The first 1600x1200 texture is
  served from shadow's returned 7500 KiB block. The second is refused,
  with "reclaim found nothing held for a submitted kick" (nothing left to
  wait for).
- Score: still 2 failed (shadow, refract), 0 GPU resets.

**What follows for (d).** Per-heap regions (mechanism 3) alone would not
make terrain or refract fit; the fixed ~36 MiB decides it. The next item
is to account for that 36 MiB (and the carveout's size), before the
regions.

## The fixed ~36 MiB, from the code

Every allocation that stays live with no scene, by owner. Sizes are page
rounded, as the pool hands them out; the heap is the device-VA heap, and
all heaps draw on the one physical cursor.

| owner | what | size (formula) | heap | site |
|---|---|---|---|---|
| render statics | parameter-buffer reserve | `PB_RESERVE_BYTES` = 2048 KiB | general | `driver/src/pvr_render.c:546` |
| | border colour table | 4 KiB | general | `pvr_render.c:547` |
| | VDM_SYNC USC, its PDS | 4 + 4 KiB | usc, pds | `pvr_render.c:413`, `:420` |
| | end-of-tile programs, 1..3 targets | 3 x 4 KiB | usc | `pvr_render.c:442` |
| | tile buffers | `PVR_RENDER_TILE_BUFFERS` x `PVR_TILE_BUFFER_BYTES` = 2 x 32 KiB | general | `pvr_render.c:456`, `pvr_render.h:60-64` |
| | their end-of-tile programs | up to 3 x 4 KiB | usc | `pvr_render.c:464` |
| freelists | backing pages, global lists 1 and 2 | (448 listed + 64 ready + 256 grow x 4 reserve) = 1536 pages = 6144 KiB each | general | `pvr_render.c:556-578` (64 MiB carveout: `big_pb` false), `freelist.rs:79-85`, `pvr_fw_boot.c:157` |
| | backing pages, local list 3 | 26 pages = 104 KiB | general | `pvr_render.c:580` |
| | PFN arrays | maxpages x 4 B, rounded to 16 KiB: 3 x 16 KiB | PM meta | `freelist.rs:87`, `mem.rs` `bo_alloc_pm` |
| render context | VDM control stream | 4 KiB | general | `render.rs:435` |
| HWRT sets | per target size, a pair (two members), kept until recycled | per member TPC >= 16 + VHEAP 8 + region headers >= 64 + macrotile 4 + MList 16 (PM) = >= 108 KiB; a pair >= 216 KiB; at most `HWRT_SETS` = 32 members | general, PM meta | `render.rs:724-753` |
| backend frame sets | per set: PDS arena 1024 + PDS static page 4 + GENERAL arena 4096 + VDM stream 64 + depth-bias table 4 + scissor table 4 + background colour 4 KiB = 5200 KiB; `PVR_BACKEND_FRAME_SETS` = 4 sets = 20800 KiB | pds, general | `src/pvr/backend.rs:200`, `:221-223`, `:1002-1017`, `:2247-2258` |
| backend statics | clear (3), frame-load (2 + 3 per format), quad, black texel, 28 masked-clear programs: 38 pages = 152 KiB (the PDS static page and the background colour are per frame set, above) | general, usc | `backend.rs:2001-2102`, `:1509` |
| | sequential index buffer | max(1024, the largest count) x 2 B: 4..128 KiB | general | `backend.rs:2391-2400` |
| backend depth/stencil | per target: ceil(w/ZLS_TILE) x ceil(h/ZLS_TILE) x tile^2 x 4 B (x 2 per axis for MSAA), up to 8 buffers | | general | `backend.rs:2953-2957`, `:2708` |
| GL objects | buffers, textures, renderbuffers, programs (USC code and uniforms), a context's own default surface (`gl_context_create`: w x h x 4) | | general, usc | `src/gl/*` |

**The known part: 35524 KiB.** That is render statics 2148, freelists
12440, render context 4, frame sets 20768, backend statics 160, and the
index buffer at least 4. (Corrected while writing the owner report, whose
host tests balance against the allocations: frame sets 20800, backend
statics 152, render statics 2132 with page-rounded end-of-tile programs.
The known part is 35532 KiB, and the prediction below is ~610 KiB.)

**Prediction for gl_test's close,** where 36144 KiB is live: the
remaining ~620 KiB are HWRT sets (a pair is at least 216 KiB, so at most
two pairs fit in it), depth/stencil buffers, gl_test's leftover objects
(its 64 x 64 harness surface is 16 KiB), the sequential index buffer's
growth, and the 16 KiB alignment gaps before each freelist. The run-end
owner lines (next commit) must add up to the pool's live bytes, and the
gap is printed as "unaccounted".

**What could shrink (each a later, separate knob, measured first):**
- **Frame sets: 20.3 MiB, the largest.** Four sets of 5 MiB are four
  kicks recorded ahead. The GENERAL arena is 4 MiB per set for a Slint
  dashboard's ~130 draws (`backend.rs:193-200`), but its high-water mark has
  never been printed (CLAUDE.md: every arena prints its high-water). The
  owner report prints it. Fewer sets, or smaller arenas, follow from it.
- **Freelist grow reserve: 2 x 4096 KiB.** `PVR_FL_GROW_RESERVE_DEFAULT` 4 x
  grow 256 pages per global list. glmark2 grew list 1 twice (+512 pages)
  and list 2 not at all, and the grows were served from the reserve. A
  reserve of 2 would give back 4 MiB if that is enough. The question is the
  firmware's worst grow, not glmark2's.
- **The parameter-buffer reserve: 2048 KiB.** It is backed by pool pages
  although the PM writes through the freelists' pages. Whether it needs
  physical backing is a hardware question, unmeasured.
- **HWRT sets for sizes no longer drawn** (at least 216 KiB a pair, up to
  16 pairs): freeing a pair whose size has not been used for a while.
- **The carveout.** 64 MiB at 0x97800000 (`verdin_am62p_am62p54_a53.dts:75`);
  the pool is 59200 KiB of it. dEQP's overlay already takes 256 MiB at
  0xa0000000 (`deqp/boards/verdin_am62p_am62p54_a53.overlay`). Note the
  `big_pb` threshold (`pvr_render.c:556`): at >= 128 MiB, a quarter of the
  carveout becomes the freelist grow pool and the lists start at 2048
  pages. So 128 MiB gives a scene about 128 - 6 - (32 + 16) - 23 = ~51 MiB,
  while 96 MiB (below the threshold) gives about 96 - 6 - 36 = ~54 MiB.
  Whether U-Boot's `go` route leaves that DDR to the image has to be
  checked, as the dEQP overlay checked it for kexec.

### The owner report (`CONFIG_YAKOGL_PVR_POOL_OWNERS`, default n)

At the end of a run, `osal_zephyr_diag_print()` prints:
- One line per owner: "pool owner <name>: N blocks, K KiB (general a, pds b,
  usc c, pm|elsewhere d)". The driver's owners are render statics,
  freelists, render context and hwrt sets (plus "hwrt sets: N members built
  of 32"). The backend's are frame sets, statics and depth/stencil. The GL
  core's are buffers, textures, renderbuffers, default surfaces, programs
  and draw staging.
- "pool owner backend frame arenas high-water: general X of 4194304 bytes,
  pds Y of 1048576 bytes, vdm Z of 65536 bytes (4 sets)".
- The balance: "osal(zephyr): pool owners: driver D + backend B + gl G +
  pending P = T KiB; pool live L KiB (handed out H - free F); unaccounted U
  KiB".

U is what no owner claims (a missing owner, or alignment gaps). A negative
U means an owner was counted twice.

The host tests balance each half against the allocations it can see:
- the GL lines against the mock pool's live bytes
  (`the_gl_owner_lines_claim_every_live_byte`; a mutation that drops one
  owner fails it);
- the backend lines against the rig platform's allocations, with the frame
  arenas' high-water after a recorded frame;
- the driver lines against the formulas above.

## The knob-on run, checked against the prediction (AM62P, 2026-10-04)

Run dir `build/pool-am62p-20261004-003056/glmark2-on` (glmark2 9618a370,
`CONFIG_YAKOGL_PVR_POOL_RECLAIM=y`). Result: "glmark2: score 304: 29
scenes ran, 2 failed, 2 unsupported", GPU resets 0 (off: 303, the same
failures).

Mechanism 2 is gone. Every refusal printed "pending frees now 0 (0 KiB),
oldest fence none" and then "reclaim found nothing held for a submitted
kick: no wait, refused". There are 5 refusals in all, against 8 with the
knob off.

| predicted (779f4eee) | measured | verdict |
|---|---|---|
| terrain: still GL_OUT_OF_MEMORY in its set-up, fewer or later refusals | 4 refusals (off: 6). One 4608 KiB VBO (off: three and the 3072), then the 512x512 mip chain (1368 KiB), then two 800x600 textures (1876 KiB). Two of the three 4608 KiB VBOs and the 3072 KiB one were served | yes |
| terrain: at least 2496 KiB of its requests unserved | 4608 + 1368 + 2 x 1876 = 9728 KiB unserved | yes |
| shadow: unchanged, 0x8cdd | "DepthRenderTarget::setup: glCheckFramebufferStatus failed (0x8cdd)" | yes |
| refract: first 1600x1200 texture served from shadow's returned block, the second refused with "reclaim found nothing held for a submitted kick" | one refusal, "texture 1600x1200 x1 fmt 0 ... out of GPU memory" (the second texture). No fmt 7 refusal: the first was served. Then 0x8cd6, set-up failed | yes |
| score: 2 failed, 0 GPU resets | 2 failed (shadow, refract), 0 resets | yes |

What remains is mechanisms 1 and 3, with nothing held:
- **Terrain's first refusal is now contiguity alone.** "4608 KiB requested,
  3296 KiB left", "general: free 5560 KiB in 6 ranges, largest 4232 KiB":
  8856 KiB free, but in no single range of 4608 KiB.
- **Refract.** "7500 KiB requested, 236 KiB left", "general: free 5808 KiB
  in 6 ranges, largest 2568 KiB": too little in all, and in pieces.

**Not measured by this run:** the knob's wait. The run end says "reclaim
on: 5 refusals, 0 served without a wait, 0 after a wait, 5 refused".
Neither the free drain nor the wait-and-retry served a single
allocation: with the free's tag moved to the last submitted kick, every
free here was already back before any allocation needed it. The "served
after a wait" path is unmeasured, not proven. Its host test
(`with_the_reclaim_a_refused_allocation_waits_for_the_submitted_kicks`)
covers the mock only.

A wording point in the refusal line: its "reclaim on: k refusals" is
printed before the refusal it reports is counted. So at the first refusal
it says 0, and the run-end line has the total (5).

### The frame-set knob (`CONFIG_YAKOGL_PVR_FRAME_SETS`, `_ARENA_GEN_KB`, `_ARENA_PDS_KB`)

The defaults (4, 4096, 1024) are what the backend always built. The Zephyr
OSAL hands the three numbers to `pvr_backend_set_frame_arenas()` before the
backend starts.
- It refuses, changing nothing: 0 or more than 4 sets; an arena below one
  draw's worst case (GENERAL 16 KiB, PDS 8 KiB); an arena above the default.
- Once the backend runs, a change is -EBUSY. The configuration it already
  has is accepted, because the OSAL init repeats.
- A non-default configuration prints "osal(zephyr): frame arenas S x
  (general G + pds P KiB), not the default 4 x (4096 + 1024)"; a refusal
  prints its own line.

A frame that overflows an arena is split, as before. A draw that does not
fit an empty frame is refused (GL_OUT_OF_MEMORY) and named by cause: "...
does not fit an empty frame: the general arena (N bytes) is too small; not
drawn". The other causes are the pds arena, the vdm stream, the depth-bias
table and the scissor table, each with its own name. The owner report's
"frame arenas high-water" line is what judges a smaller configuration.

At the defaults nothing changes:
- the gate's km records still hash to `tools/pvr/km/records.sha256`;
- the host-test row passes unchanged;
- the AM62P gl_test image's .config differs from the owners image only by
  the three default lines.

Found on the way, a separate item: `pvr_backend_shutdown()` frees nothing.
Its "bump allocators: nothing to free" comment predates the pool's free
lists, and `pvr_backend_init()` zeroes the state, so a shutdown followed by
an init leaks every frame set, static and depth buffer (~21 MiB at the
defaults). No product path calls the shutdown today (the backend starts
once per run); the host test that restarts it balances only what the
restart allocated.

The prediction for a smaller configuration is written when the
high-water number arrives.

## The owner report on the board (AM62P, 2026-10-04): the account holds to 12 KiB

Run dir `build/poolown-am62p-20261004-010040` (gl_test d693a1f5,
`CONFIG_YAKOGL_PVR_POOL_OWNERS=y`): "1 of 38 cases failed" (case 48, as on
main).

| owner | predicted (f5da97d0, corrected in 7a50f555) | measured |
|---|---|---|
| render statics | 2132 KiB | "11 blocks, 2144 KiB (general 2116, pds 4, usc 24)": the three tile-buffer end-of-tile programs exist (+12 KiB of USC) |
| freelists | 12440 | "6 blocks, 12440 KiB (general 12392, pm 48)" |
| render context | 4 | "1 blocks, 4 KiB" |
| HWRT sets | >= 108 per member, in the ~610 remainder | "25 blocks, 540 KiB (general 460, pm 80)", "5 members built of 32" (108 each) |
| backend frame sets | 20800 | "28 blocks, 20800 KiB (general 16688, pds 4112)" |
| backend statics | 152 + the index buffer | "39 blocks, 156 KiB": 152 + a 4 KiB index buffer |
| backend depth/stencil | in the remainder | "1 blocks, 16 KiB" |
| GL | in the remainder | default surface 16 KiB (the 64x64 harness), draw staging 16 KiB, nothing else |
| balance | ~610 KiB beyond the known part | "driver 15128 + backend 20972 + gl 32 + pending 64 = 36196 KiB; pool live 36208 KiB; unaccounted 12 KiB" |

The fixed use is accounted for to 12 KiB. (Corrected 2026-10-04: the 12 KiB
is not the freelist backings' 16 KiB alignment, as first written here, since
glmark2 builds the same freelists and closes at 0. It is gl_test's own two
pool buffers, never freed: `case_rgb565`'s 64x64 RGB565 window, 8 KiB
(`gl_test.c:3928`), and `case_image`'s 64-byte native pixmap, one page
(`gl_test.c:3101`). They belong to no owner the report walks. Every
gl_test close from 7a50f555 to 3f1d2dab reads exactly "unaccounted 12 KiB".
The two cases now free them, as `case_viewwin` does.) It is 58% frame sets, 34%
freelists, 6% render statics and 1.5% HWRT sets.

**The decisive number:** "backend frame arenas high-water: general 33600 of
4194304 bytes, pds 153516 of 1048576 bytes, vdm 12012 of 65536 bytes (4
sets)". gl_test's largest frame used 0.8% of a GENERAL arena and 15% of a
PDS arena. glmark2's high-water (desktop records 11 kicks a frame) is not
measured yet. The frame-set knob's default proposal waits for it, from the
glmark2 owners run.

**The shutdown leak, fixed** (the item found with the frame-set knob).
`pvr_backend_shutdown()` now:
- submits a frame still being recorded and waits for the last kick;
- then frees the frame sets in use, the persistent programs and tables,
  and the depth/stencil buffers (the set the owner report names).
If the last kick does not complete, it keeps the memory and says so
("kick N did not complete: the backend's pool memory is kept").

A shutdown plus an init leaves the pool's live bytes where they were
(`shutdown_gives_back_everything_and_a_restart_takes_the_same_again`). With
the old shutdown, that test fails with ~21 MiB still live.

## glmark2's owner report, and the frame-set proposal (AM62P, 2026-10-04)

Run dir `build/poolown-glmark2-am62p-20261004-011417` (glmark2 8ad48683
from 01e9a5e1: owners on, reclaim off, frame sets at the defaults):
"Score 302, 2 failed, 0 resets".

- **At the first refusal (terrain):** "driver 15344 + pending 11576 KiB;
  backend + gl (the rest of live) 25704 KiB; pool live 52624 KiB". With
  the backend's 21080 KiB from the run end, the GL core held about 4.6 MiB
  of terrain's set-up at that moment. The held 11576 KiB is the knob-off
  mechanism 2, as in the first run.
- **HWRT sets:** 7 members (756 KiB) at the refusal, 10 (1080 KiB) at the
  close.
- **The frame arenas' high-water over the whole run:** "general 516768 of
  4194304 bytes, pds 325896 of 1048576 bytes, vdm 18408 of 65536 bytes (4
  sets)". glmark2's largest frame uses 12% of a GENERAL arena and 31% of a
  PDS arena; gl_test's used 0.8% and 15%.
- **Balance at the close:** "driver 15668 + backend 21080 + gl 520 + pending
  1996 = 39264 KiB; pool live 39400 KiB; unaccounted 136 KiB".

**The 136 KiB unaccounted** (gl_test: 12) is not named by any line. Most
likely it is the sequential index buffer's growth leak, fixed in c7fcade8
after this image was built. glmark2 draws large sequential counts, so the
buffer grew (backend statics are 280 KiB here, against 156 in gl_test: the
current index buffer is 128 KiB). Each growth lost the smaller one (4 KiB,
then up to 120 KiB), which no owner claims. This attribution is
unmeasured: the next images carry the fix, and their close should show
about 12 KiB unaccounted again.

### The proposal: 4 sets x (1024 KiB GENERAL + 768 KiB PDS)

- **Arenas:** 1024 KiB GENERAL is 2.0 x glmark2's worst frame (505 KiB), and
  768 KiB PDS is 2.4 x its 318 KiB. An overflow is not an error: the frame
  is split, at the cost of one more kick, and only a draw too big for an
  empty frame is refused, by name. The margin is there so glmark2-sized
  frames are not split.
- **A frame shares the PDS programs it holds** (`pds_put` in
  `src/pvr/backend.rs`, gl_test 198): a program the open frame already
  uploaded is pointed at again, so after its first draw a draw of the same
  program takes only its per-draw data segments -- two 128-byte blocks for
  dEQP's multisample draw instead of six. The PDS high-waters above were
  measured before that and are upper bounds. A split is not free on a 4x
  target: its next kick reloads the resolved colour into every sample
  (`docs/HANDOFF.md`, known defects, pvr-msaa-split), which is how six
  blocks a draw in this arena failed two dEQP multisample cases.
- **Four sets, not two.** The count is load-bearing for pipelining, not
  for capacity. A set is reused only after the kick that last used it has
  completed (`backend.rs`, "the next frame set ... waits for its fence"),
  so the count is how many kicks can be recorded ahead of the GPU. The
  constant's own comment says that with two sets a Quick 3D pass "could not
  start until the one before last had finished". glmark2's desktop scene
  records 11 kicks a frame, so two sets would serialise CPU and GPU there.
  Going to two saves 2 x 1872 KiB more; that is a performance question,
  measured separately if ever.
- **Size:** 4 x (1024 + 768 + 64 + 4 + 4 + 4 + 4) = 7488 KiB, instead of
  20800. 13312 KiB more for scenes. The frame sets are allocated at the
  backend's init, before any scene, so the saving moves every later
  allocation down: it is 13312 KiB more above the bump, in one piece.

**Prediction for glmark2 with it (owners on, reclaim on, from c7fcade8),**
written before the run:
- **Terrain: no refusal in its set-up.** With reclaim on, its unserved
  requests were 9728 KiB (4608 + 1368 + 2 x 1876), and the first was
  refused for contiguity (largest free range 4232 KiB, 3296 KiB above the
  bump). With 13312 KiB more above the bump, all of them fit. So: no "0x505
  in glBufferData / glTexImage2D", and no "gl_fb_target: framebuffer N
  incomplete (status 0x8cd6)" in terrain. Its 800x600 depth target's
  backend buffer (~1.9 MiB) fits in what is left. glGenerateMipmap's 0x502
  on its NPOT targets stays (not the pool; glmark2.md).
- **Shadow:** unchanged, 0x8cdd. Not the pool.
- **Refract: its set-up succeeds,** and its first draws may still be
  refused.
  - Its two 1600x1200 textures (2 x 7500 KiB) were short by the second one
    (236 KiB above the bump with reclaim on). With 13548 KiB above the
    bump, both are served, so its "DistanceRenderTarget::setup ...
    (0x8cd6)" goes away.
  - Its depth pass then needs a backend depth buffer for the 1600x1200
    depth texture: 1600 x 1216 x 4 = 7600 KiB. About 6048 KiB will be left
    above the bump, and the free lists held no range above 2568 KiB at
    that point in the earlier run.
  - Likely outcome: "pvr_backend: no depth/stencil buffer for target ...
    (1600x1200)", refract runs with refused draws, and it no longer counts
    as a set-up failure. That is mechanism 1 for refract's depth, about
    1.5 MiB short. It is a prediction about a layout that changed
    everywhere, so it is the weakest line here.
- **Score line:** "2 failed" becomes "1 failed" (shadow) if refract's
  set-up passes, and 0 GPU resets.
- **Balance at close:** unaccounted about 12 KiB (the index-buffer fix).
  Frame sets "28 blocks, 7488 KiB (general 4400, pds 3088)": per set, 1024 + 64 + 4 + 4 + 4 GENERAL and 768 + 4 PDS. The arena
  high-water is at most the earlier 516768 / 325896 bytes, since no frame
  needs a split.

**Prediction for gl_test with the same configuration:** 37 of 38 (case 48,
as on main); the arenas cover gl_test's 33600 / 153516 bytes 31 and 5
times over. Frame sets 7488 KiB, unaccounted about 12 KiB.

## The frame-set run (AM62P, 2026-10-04): every predicted line held

Run dir `build/frames-am62p-20261004-012818/{gltest,glmark2}`. Images from
60d2e073 with 4 x (1024 KiB GENERAL + 768 KiB PDS), owners on, reclaim on.

| predicted (34eb0ff8, 60d2e073) | measured | verdict |
|---|---|---|
| gl_test 37 of 38 | "1 of 38 cases failed (37 passed; 0 timed out; first failure case 48)" (= main) | yes |
| terrain: no refusal in its set-up | ran with no refusal | yes |
| shadow: 0x8cdd | 0x8cdd (the one set-up failure) | yes |
| refract: set-up passes, the 7600 KiB depth buffer refused with ~6 MiB left | set-up passed. "render pool exhausted: 7600 KiB requested, 2252 KiB left", "yakogl/pvr: draw refused: no depth/stencil buffer for a 1600x1200 target ... (110 refused so far)" | yes, with 2.2 MiB left rather than ~6 |
| "1 failed", 0 resets | "Score 293, 30 scenes ran, 1 failed, 2 unsupported", 0 resets | yes |
| frame sets "28 blocks, 7488 KiB (general 4400, pds 3088)" | the same line | yes |
| arena high-water at most 516768 / 325896 | "general 516768 of 1048576 bytes, pds 325896 of 786432 bytes" (no frame split) | yes |
| unaccounted ~12 KiB (the index-buffer growth leak, c7fcade8) | "driver 15668 + backend 7768 + gl 520 + pending 0 = 23956 KiB; pool live 23956 KiB; unaccounted 0 KiB" | yes; the 136 KiB was that leak |

The score is lower (293 against 302) because terrain and refract now run:
terrain at about 7 fps, and refract with its draws refused. They were set-up
failures before, outside the mean. It is not a slowdown of the other scenes.

What remains for refract is mechanism 1: its 1600x1200 depth buffer
(7600 KiB) did not fit, with 2252 KiB above the bump.

## The default's dEQP coverage (AM62P, 2026-10-04): 0 differing per case

The default (dcaafa41) cuts the GENERAL arena from 4096 to 1024 KiB, so a
frame splits sooner. The A/B: images from 6ee6f0b3, a1024 (the new
default) against a4096 (`CONFIG_YAKOGL_PVR_ARENA_GEN_KB=4096`), booted from
U-Boot, one install, environment diff 0. Run dir
`build/deqp-arena-am62p-20261004-015937`.

| image (filter) | a1024 sha256 | a4096 sha256 | cases, both arms | per-case diff |
|---|---|---|---|---|
| img1: `vertex_arrays.*`, `draw.*`, `buffer.*` | `e3eb14cb` | `7886cf07` | 721 begun, 721 with a result, 0 without; 721 Pass | 0 differing |
| img2: `fragment_ops.blend.*` | `70de7796` | `475d8bed` | 1060 begun, 1060 with a result, 0 without; 1060 Pass | 0 differing |
| img3: `fragment_ops` depth, stencil, depth_stencil, random, interaction, scissor | `96405f03` | `4c55b0cd` | 863 begun, 863 with a result, 0 without; 863 Pass | 0 differing |

No run printed "does not fit an empty frame", an exhaustion or a recovery
line. So nothing these 2644 cases cover changed with the smaller arena.
That is the default's suite coverage, and only that: groups outside the
three filters were not run with it.

## Next: refract's 7600 KiB depth buffer, ranked

Refract's depth buffer needs 7600 KiB, with 2252 KiB above the bump at that
point: 5348 KiB short, about 7 MiB with margin. The fixed users left
(the owner report, glmark2, frame sets at the new default):

| user | what one knob frees | risk |
|---|---|---|
| freelist grow reserve (2 x 1024 pages at reserve x4) | up to 8192 KiB; 4096 at x2, 6144 at x1 | an unserved grow (below) |
| parameter-buffer reserve | 2048 KiB | whether the PM/TE needs it backed is unmeasured; if it does, an MMU fault |
| HWRT pairs for sizes no longer drawn | at most what is built: 1080 KiB at glmark2's close (10 members of 108) | small (a CLEANUP first, `render.rs` hwrt_get) |
| PFN arrays (3 x 16 KiB) | under 48 KiB | none worth it |
| frame sets, 2 instead of 4 | 3744 KiB | pipelining: 11 kicks a frame in desktop |

Only the freelist reserve covers refract alone. A plain smaller multiplier
is not the knob, because glmark2's first run grew list 1 twice ("FWCCB:
FREELIST_GROW id=1 -> +256 pages (768 listed, 768 in reserve, pool 0/0)",
then "(1024 listed, 512 in reserve ...)", build/glmark2-am62p-20261003-230937
console lines 177 and 180). At x1, the second grow is unserved, in exactly
the scene that needed it.

**The one knob proposed next: serve freelist grows from the render pool when
the firmware asks, with no pre-reserved pages** (reserve x0, a grow takes
`grow` pages from the pool at the FREELIST_GROW). It frees 8192 KiB at init,
and a grow costs pool pages only in the scene that asks for them. The PM
reads the list as page numbers (`freelist.rs`: a PFN array), so a grow's 256
pages need 16 KiB alignment (four consecutive 4 KiB pages per PM page) but
not one contiguous run.

**What the DDK sizes it from, and what we know:** `freelist.rs` follows the
DDK's `RGXCreateFreeList` and `RGXProcessRequestGrow` (rgxta3d.c). There a
freelist is created with max, initial and grow page counts from its caller,
and a grow allocates its pages when the firmware asks (the request carries
only the list ID: `RGXFWIF_FWCCB_CMD_FREELIST_GS_DATA { ui64FreelistID }`,
`rgx_fwif_km.h:1121-1126`), not from a reserve made at creation. The
vendored subset (`third_party/ti-img-rogue-ddk-25.2`) has the firmware
interface (`ui32GrowPages`, `ui32ReadyPages`, `rgx_fwif_shared.h:949-966`)
but not rgxta3d.c or the user-mode sizing. So the DDK's own numbers are not
in this tree. Ours (448 listed, 1536 backed, grow 256) are "the DDK's
sizing for a small test" (`pvr_render.c`), from an on-board capture.

**The risk:** the worst grow a scene can ask for is bounded only by maxpages
(4096 pages, 16 MiB per list), and nobody has measured the firmware's worst
demand. What an unserved grow does is contested in our own code:
- `freelist.rs:16-21`: "answered with zero pages, and the firmware then
  renders what it has tiled (partial render)";
- `pvr_render.c:603-608`: "a render that cannot grow never completes (fence
  timeout, wedged GPU)", from a Qt Quick 3D scene.

So the knob ships opt-in, and before it may default on:
1. a host test of the on-demand grow (pages taken from the pool, 16 KiB
   aligned, the PFNs listed, an empty pool answered as "no grow" with its
   own line);
2. a board run that provokes an unserved grow on purpose (a pool
   exhausted at the grow) to settle which of the two comments is true, with
   the recovery count;
3. gl_test and glmark2 with it, the grow lines counted.

## The on-demand freelist grow (`CONFIG_SAFEUI_GPU_PVR_FL_GROW_ON_DEMAND`, opt-in)

Branch `pvr/fl-grow-on-demand`, from 21251d51:
- da84b8f3: the knob.
- 17797907: the probe and the PowerVR recovery count.

With the knob on, the two global freelists are built without their grow
reserve, which gives back 2 x 4096 KiB at the 64 MiB carveout's sizing. A
FREELIST_GROW is then served with 256 + 3 pages taken from the render pool
when the firmware asks. Three gates before it may default on.

1. **Host tests** (`freelist.rs` grow_tests, in da84b8f3): the source, the
   16 KiB-aligned listing, the accounting and one line per cause.
2. **The probe** (`CONFIG_SAFEUI_GPU_PVR_FL_PROBE_MAX_PAGES=512`, knob off):
   the global lists cannot grow past their starting 512 pages. The known
   scene that needs a grow is glmark2's `[bump] bump-render=high-poly`: in
   both earlier AM62P runs it asked list 1 for two grows at its start. The
   record carries the recovery count: `board_gpu_resets()` is the
   firmware's HWR counter since the render service started, and the run
   ends with "render freelists: R grow requests, S served, U unserved ...;
   GPU recoveries N".
3. **gl_test and glmark2 with the knob on.**

**Predictions for the probe**, written before it runs, one per comment:
- **If `freelist.rs:16-21` is right ("renders what it has tiled (partial
  render)")**: bump high-poly's frames complete, with a "glmark2-frame:
  [bump] bump-render=high-poly: N frames" line and N > 0. Every grow
  request gets "FWCCB: FREELIST_GROW id=1 -> no grow (reserve 1024, pool
  0/0, 512/512 pages)". There is no "did not complete" line, and "GPU
  recoveries 0". The frame time may rise (a partial render is an extra
  pass), and the picture is complete (a partial render stores and reloads
  what was tiled).
- **If `pvr_render.c:603-608` is right ("never completes")**: the first
  frame that needs the grow does not complete. "pvr_backend: kick N did not
  complete" comes after the 20 s wait. Then either the firmware recovers
  it, giving "render: HWR during fence N (counter A -> B)", GPU recoveries
  above 0 and a FAIL verdict that names the recoveries; or nothing recovers
  it and the run ends at the watchdog with no result record (the runner
  says `died_after`).
- **Which one I expect:** the second, with HWR. Our pvr_render.c comment
  records a Qt Quick 3D scene that hung at exactly this point. And the
  firmware's freelist has `ui32ReadyPages` "reserved pages to be used only
  on PM OOM event" (`rgx_fwif_shared.h:965`), so an out-of-memory path
  exists only for pages it was given. Ours gives it none: `freelist::build`
  writes `FREELIST_READY_PAGES` 0 and lists the 64 "ready" pages as
  current pages, so there is no OOM reserve for the firmware to fall back
  on. The probe tells.

**Predictions for gate 3, knob on, no probe cap:**
- gl_test: 37 of 38 (case 48), no grow (gl_test never grew in any run).
- glmark2: bump high-poly asks list 1 for two grows. Both are served from
  the render pool: "+256 pages from the render pool ... 1036 KiB taken on
  demand", then 2072. GPU recoveries 0.
- The owner report's freelists line at close: 12440 - 8192 + 2072 = 6320
  KiB, so 6120 KiB more for the scenes than at the old sizing. Terrain and
  refract start that much lower, at this branch's (old) frame-set default.

## The probe and gate 3 on the AM62P (2026-10-04): the probe measured the list at its maximum

Run dir `build/flgrow-am62p-20261004-021446` (booted from U-Boot,
environment diff 0). Line numbers are those of each run's `console.txt`.

### The probe (image 0efa0642, cap 512 pages, knob off)

What it printed at glmark2's `[bump] bump-render=high-poly`:
- :180 "FWCCB: cmd type=0x2abc006b (unhandled, skipping)", and no
  FREELIST_GROW line anywhere in the run.
- :181-182 "render: fence 30681 (frame fence) not moved at 20 ms,
  re-posted 1 kick(s)", then "render: fence 30681 timeout (ufo=30680)".
- :207 "render: stall: freelist1 cur=512 top=507 max=512 grow=256 ready=0
  pending=0 alloc=4/0; fwccb grow req/done=0/0"; :196 "fl_refusals=0".
- :309-338 "pvr_backend: kick 30681 did not complete (1)", the same for
  30682, then "[bump] bump-render=high-poly: 4 frames".
- No "render: HWR during fence" line up to the reset: `wait_for` reads the
  HWR counter every millisecond, so the firmware did not recover the kick
  in the 61 s from 02:16:43 to 02:17:44.
- :190 "rti_arm: the sample stopped saying it is alive; the RTI resets the
  SoC", and U-Boot at 02:17:51. The run has no record and no run-end line
  (`verdict=no_record`, `back_after=189s`): the RTI watchdog ended it, as
  the image is built to.

**What this measured is not an unserved grow.** The cap made the list's
maximum equal to its starting size (512 = 512), so there was no room for
a grow (`max - cur < grow`), and the firmware never asked for one: 0 grow
requests, 0 refusals. The measured case is: **the PM ran out of pages with
its list at the maximum. The firmware then sent one 0x2abc006b and no
grow, and the kick never completed.** It did not render partially, and it
did not recover in 61 s.

0x2abc006b is `RGXFWIF_FWCCB_CMD_UPDATE_STATS` (DDK 25.2
`rgx_fwif_km.h:1202`). Its data names a statistic to increase: 1 partial
renders, 2 out of memory, and others (`RGXFWIF_FWCCB_CMD_UPDATE_STATS_TYPE`).
`ccb.rs` logs it as unhandled and drops the data word, so which one it was
is unmeasured. In gate 3's glmark2 the same command follows each of the two
served grows (:183, :186). That fits an out-of-memory notice that goes with
a grow request when a grow is possible and alone when it is not. It is a
hypothesis until the element word is printed.

So, of the two comments the probe was meant to decide between:
- `pvr_render.c` ("a render that cannot grow never completes") matches
  what was measured, for a list at its maximum.
- `freelist.rs` ("a grow that cannot be served is answered with zero
  pages, and the firmware then renders what it has tiled") is still
  unmeasured. No zero-page answer was ever sent. Its comment now says so.

The probe for that case is another image: the knob off, the reserve
multiplier 0 and the maximum left at 4096. Then the firmware asks, and the
host answers zero pages.

**My prediction was wrong in its premise**, not only in its outcome. I
expected "the second, with HWR". The kick did wedge, but with no HWR, and
for a reason the prediction did not consider (no grow was asked). The
`fwccb grow req/done=0/0` field in the stall line was there to show it.

### Gate 3 (knob on, no cap): every predicted line held

| predicted | measured | verdict |
|---|---|---|
| gl_test 37 of 38, no grow | :1955 "1 of 38 cases failed (37 passed; 0 timed out; first failure case 48 at index 31)"; :1964 "render freelists: 0 grow requests, 0 served, 0 unserved, 0 KiB taken on demand; GPU recoveries 0 (HWR counter 0)" | yes |
| freelists at gl_test's close: 12440 - 8192 = 4248 KiB | :1966 "pool owner freelists: 6 blocks, 4248 KiB" | yes |
| glmark2: two grows of list 1 from the render pool, 1036 then 2072 KiB | :181 "+256 pages from the render pool (768 listed, 1036 KiB taken on demand, first page phys 0x9a210000)", :184 "(1024 listed, 2072 KiB ...)"; :819 "2 grow requests, 2 served, 0 unserved, 2072 KiB taken on demand; GPU recoveries 0 (HWR counter 0)" | yes |
| freelists at close 6320 KiB | :821 "pool owner freelists: 7 blocks, 6320 KiB" | yes |
| — | :808 "glmark2 Score: 303" | (not predicted) |

### What the knob still lacks before it may default on

- **A grow the pool refuses wedges the kick for as long as the watchdog
  allows.** That is likely, given the probe, but unmeasured (see above).
  The knob must make one grow always servable: one grow's pages kept per
  list and refilled after each served grow. When that still fails, the run
  must end with a named record, never a silent hang.
- **The list at its maximum** (4096 pages, 16 MiB at the 64 MiB carveout)
  wedges the kick with no grow request. That is measured. Refill does not
  help there; it needs the same named end, triggered from the
  out-of-memory notice once its element word is measured.
- **Reconstruction after an on-demand grow lists the wrong pages.**
  `freelist::reconstruct` (the answer to FREELISTS_RECONSTRUCTION after an
  HWR) lists `backing_phys + j` for every current page. That is right only
  while all of them are contiguous behind the list's backing. After an
  on-demand grow (and after a grow from the shared pool, at the larger
  carveouts), it hands the PM pages that belong to other objects. No run
  has had an HWR after a grow, so it is unmeasured on the board; it is a
  defect by reading the code.

## The guarantee: one grow kept per list, refilled after each grow

With the knob on, the two global lists are built with one grow's reserve
(`fl_reserve_mult` 1: 256 pages behind their 512) instead of none.
`freelist::serve` takes a grow's pages from these sources, in order:
1. the list's own reserve;
2. the shared grow pool (larger carveouts only);
3. the reserve refilled after the list's last grow (`fl_spare_pfn`);
4. the render pool, asked at that moment.

After a served grow, `freelist::refill` gives the list one grow's pages
from the render pool again. It does so only when the list has no reserve
left, there is no shared pool, and the list has room for another grow. So
the firmware's grow never waits on the render pool's state at that moment.
A grow is refused only when the refill was refused and the render pool
refuses again when the firmware asks. Each case has its own line:
- "FWCCB: FREELIST_GROW id=N reserve refilled: K KiB from the render pool
  (T KiB taken on demand, first page phys 0x...)".
- "... reserve not refilled: the render pool refused K KiB; the next grow
  asks it again". This is counted as `fl_refills_refused` and
  `fl_refill_refused_kib`.
- "FWCCB: FREELIST_GROW id=N -> +256 pages from its refilled reserve (L
  listed, first page phys 0x...)".
- "... -> no grow: G grows served, the host's table of grows holds 64". The
  table (`fl_grow_pfn`) is what reconstruction lists again. A grow past it
  is refused before any source is asked. At the 64 MiB carveout a list can
  take 14 grows.

The same commit fixes three more things:
- **Reconstruction and the audit.** Both now follow the grow table. The km
  record `ccb` showed the old defect: tools/pvr/km/README.md, "What
  'identical' means here".
- **UPDATE_STATS is decoded.** The firmware's notice gets its own line,
  "FWCCB: UPDATE_STATS element=E (name) adjust=A (seen N)", and its own
  run-end count.
- **The owner report counts each on-demand block as a block.** Gate 3's
  "pool owner freelists: 7 blocks, 6320 KiB" was list 1's two on-demand
  blocks counted as one. The KiB was right; the blocks should have read 8.

The run end gains a second line: "render freelists: reserve refills
refused F (K KiB); firmware UPDATE_STATS: partial renders A, out of memory
B, TA stores C, 3D stores D, CDM stores E, TDM stores F, other G".

What it costs at init: 2 x 1024 KiB, against 2 x 4096 KiB without the knob.

**Predictions for the refill images (knob on), written before they run:**
- **gl_test:** 37 of 38 (case 48), no grow, "0 grow requests". "pool owner
  freelists: 6 blocks, 6296 KiB" (4248 + 2 x 1024), "reserve refills
  refused 0 (0 KiB)", and every UPDATE_STATS count 0 (gate 3's gl_test had
  no 0x2abc006b).
- **glmark2,** at bump high-poly, in this order:
  - "FWCCB: FREELIST_GROW id=1 -> +256 pages (768 listed, 0 in reserve,
    pool 0/0)";
  - "reserve refilled: 1036 KiB from the render pool (1036 KiB taken on
    demand, ...)";
  - "-> +256 pages from its refilled reserve (1024 listed, ...)";
  - "reserve refilled: 1036 KiB ... (2072 KiB taken on demand, ...)".
  - Two UPDATE_STATS lines, one after each grow. I expect element 2 (out
    of memory); this is the first run that prints it, so it is the
    measurement, not a check.
  - At close: "2 grow requests, 2 served, 0 unserved, 2072 KiB taken on
    demand; GPU recoveries 0", "reserve refills refused 0 (0 KiB)", and
    "pool owner freelists: 8 blocks, 8368 KiB (general 8320, pds 0, usc 0,
    pm 48)".
  - The score stays near gate 3's 303. Terrain and refract start 2048 KiB
    lower than at gate 3, and 6144 KiB higher than with the knob off.
- **The lead's figure of about 7.4 MiB** (6320 + about 1024) would hold
  only if one list kept a reserve. Both global lists keep one, so I expect
  8368.

## The stop: a kick that cannot complete ends the run, by name

With the knob on, the render service stops taking kicks when it finds a
kick it cannot complete (`freelist::stop`). There are three causes. Each
has its own line and its own number (the record's tag):

| cause | when | line |
|---|---|---|
| ~~1 `GROW_REFUSED`~~ | retired after the no-reserve run (below): a refused grow is survived, counted ("unserved") and said, not a stop | — |
| 2 `AT_MAXIMUM` | an out-of-memory notice with no grow request in the same FWCCB batch, and a list with less than one grow of room left | "render: stopped: the parameter manager out of memory with its freelist at the maximum (list N, 0 KiB); ..." |
| 3 `GROW_TABLE_FULL` | a grow past the table of 64 | "render: stopped: a freelist grow past the host's table of grows (list N, 0 KiB); ..." |

Cause 2 is the probe's case. It rests on the notice being element 2 (out
of memory), which is unmeasured: the probe did not print the element. If
the element is another value, cause 2 never fires and the run wedges as
the probe's did. The next run that prints "FWCCB: UPDATE_STATS element=..."
settles it.

From the stop on:
- every wait returns -ESHUTDOWN at once, and the first one says "render:
  fence F (who) cannot complete: the render service stopped (...); not
  waited for";
- every submit returns -ESHUTDOWN before it builds anything.

(-ESHUTDOWN, not -ENOSPC: the backend's "frame full" is -ENOSPC,
`src/pvr/backend.rs` `EFRAME_FULL`.)

The first -ESHUTDOWN in `pvr_render.c` prints the driver's run-end lines
(`pvr_render_pool_report`: the pool summary and the two "render
freelists:" lines), then calls the board's handler. `board.c`,
`board_pvr_stopped`, then:
- prints "board: FAIL the GPU driver stopped the run (cause C, fence F,
  list N, K KiB, GPU recoveries R)";
- stores the record GPU_STOP (kind 20, tag = cause, w1 = fence, w2 = GPU
  recoveries, w3 = list << 24 | KiB);
- holds `CONFIG_YAKOGL_BOARD_HOLD_FAIL_S`, then resets.

The GL side's run-end lines (the owner report's GL rows, the backend's
depth/stencil line) are not printed: the stop is inside a GL call, and
`yakogl_pool_owners` is sound only between frames. The runner reads
GPU_STOP as the verdict (`resets` = w2): `verdict=done
result=record:GPU_STOP`, or `done_with_resets` when the GPU recovered
first.

Without the knob nothing changes: a refused grow is the old "no grow", and
no stop is made. That keeps the knob-off probe of the zero-page answer (the
reserve multiplier 0, the maximum 4096) measuring what the firmware does
with it.

Host tests:
- `freelist.rs` grow_tests: the three causes; the first cause stays; no
  stop without the knob.
- `render.rs` wait_tests: the probe's FWCCB (one UPDATE_STATS element 2,
  list 1 at 512 of 512) ends a 20 s wait after 0 us with the knob, and
  times out as before without it; a stopped service takes no kick. The
  wait test fails with the post-service check removed.

**Prediction for an image with the knob on and the probe cap 512**
(`CONFIG_SAFEUI_GPU_PVR_FL_GROW_ON_DEMAND=y`,
`CONFIG_SAFEUI_GPU_PVR_FL_PROBE_MAX_PAGES=512`), glmark2, written before it
runs:
- At bump high-poly: "FWCCB: UPDATE_STATS element=2 (out of memory)
  adjust=A (seen 1)" (A unmeasured), then "render: stopped: the parameter manager out of memory
  with its freelist at the maximum (list 1, 0 KiB); ...", then "render:
  fence F (...) cannot complete ...".
- Then the two "render freelists:" lines: 0 grow requests, GPU recoveries
  0; out of memory 1.
- Then "board: FAIL the GPU driver stopped the run (cause 2, fence F, list
  1, 0 KiB, GPU recoveries 0)" and `yk-record: 594b1402 <F> 00000000
  01000000`.
- The runner prints `verdict=done result=record:GPU_STOP resets=0`. The run
  ends within seconds of the scene's start, not at the watchdog 189 s
  later.
- If the element is not 2: no stop line, the probe's wedge again, and the
  UPDATE_STATS line names the element. The trigger is then corrected to the
  measured value.

### Where the shared-pool path runs, and the cause numbers

**The reconstruction fault reaches the shared-pool path on main too.** That
path is taken at a carveout of 128 MiB or more (`pvr_render.c`, `big_pb`).
The board's own 64 MiB carveout does not take it. **The AM62P dEQP image
does:** `driver/samples/zephyr/deqp/boards/verdin_am62p_am62p54_a53.overlay:19-27`
gives the GPU a 256 MiB carveout. The six dEQP consoles of
`build/deqp-arena-am62p-20261004-015932` each print "parameter buffer:
carveout 256". So on a dEQP run, a grow followed by a hardware recovery
would have handed the PM another list's pages.

None of those six consoles has a FREELIST_GROW or a FREELISTS_RECON line.
No `console*.txt` under this host's `build/` directories has a
reconstruction: 17 consoles have grows, all glmark2 on the 64 MiB
carveout. The known positive, gate 3's glmark2, counts 2. The search did
not cover consoles kept elsewhere or under other names. So the fault is
reachable on the conformance image and not shown to have fired. On this
branch, `freelist::reconstruct` lists the grow table on every path.

**Cause numbers** (the record's tag, `record.rs` GPU_STOP_*, the lead's
numbering): 1 a grow the render pool refused, 2 out of memory at the
list's maximum, 3 a grow past the table. The stop image (knob + cap 512)
exercises cause 2, not 1: no grow is asked there. Cause 1 needs the render
pool to refuse a refill and then the grow, which no image provokes yet.

### The refused-grow probe (`CONFIG_SAFEUI_GPU_PVR_FL_PROBE_NO_RESERVE`)

The knob off, the reserve multiplier 0, the maximum 4096. The firmware asks
for a grow, and the host answers zero pages. It is the probe the cap was
meant to be.

**Predictions, glmark2, written before it runs:**
- At init: "render: PROBE: the global freelists have no grow reserve:
  every grow is answered with zero pages, on purpose".
- At bump high-poly:
  - "FWCCB: FREELIST_GROW id=1 -> no grow (reserve 0, pool 0/0, 512/4096
    pages)";
  - the grow ack line;
  - "FWCCB: UPDATE_STATS element=E ...". This is the second measurement of
    the element; the first is the refill image.
  - No stop line: the knob is off.
- **If freelist.rs's old comment is right (partial render):** bump's
  frames complete, "[bump] bump-render=high-poly: N frames" with N > 0.
  There are more grow requests, all answered "no grow", and possibly
  UPDATE_STATS element 1 (partial renders). No "did not complete", and the
  run reaches its end lines with "N grow requests, 0 served, N unserved;
  GPU recoveries 0".
- **If not:** "pvr_backend: kick K did not complete" after the 20 s wait,
  with no HWR line (as at the maximum), and the RTI watchdog ends the run
  without a record. The kicks carry a partial-render fence address
  (`render.rs:1232`), so the firmware may have what a partial render
  needs. Nothing measured says it does.
- **Which one I expect:** the second, weakly. The one PM out-of-memory
  case measured (at the maximum) wedged, and nothing on this host shows a
  partial render ever completing.

## The four runs of 76e7d58d (AM62P, 2026-10-04)

Run dir `build/flgrow2-am62p-20261004-030916/{fl-grow-gltest-am62p,
fl-grow-glmark2-am62p,fl-stop-glmark2-am62p,fl-nr-glmark2-am62p}`, booted
from U-Boot, one install, environment diff 0.

| image | predicted | measured | verdict |
|---|---|---|---|
| refill gl_test 5d4061c2 | 37/38; 0 grows; 6 blocks, 6296 KiB; every UPDATE_STATS count 0 | "1 of 38 cases failed (37 passed; ... case 48)"; "0 grow requests, 0 served, 0 unserved"; "6 blocks, 6296 KiB (general 6248, pm 48)"; every count 0 | yes |
| refill glmark2 ed1b5754 | two grows, both served (reserve, then refilled reserve); 2072 KiB; 8 blocks, 8368 KiB; refills refused 0 | "2 grow requests, 2 served, 0 unserved, 2072 KiB taken on demand; GPU recoveries 0"; "reserve refills refused 0 (0 KiB)"; "8 blocks, 8368 KiB (general 8320, pm 48)"; Score 303 | yes |
| stop probe d9e225eb (knob + cap 512) | at bump: element 2, the at-maximum stop, GPU_STOP tag 2, the run ends after the hold | "FWCCB: UPDATE_STATS element=2 (out of memory) adjust=1 (seen 1)", "render: stopped: the parameter manager out of memory with its freelist at the maximum (list 1, 0 KiB) ...", "render: fence 30871 (frame fence) cannot complete ...", "board: FAIL the GPU driver stopped the run (cause 2, fence 30871, list 1, 0 KiB, GPU recoveries 0) ...", `yk-record: 594b1402 00007897 00000000 01000000`; back after 162 s, not at the watchdog | yes |
| no-reserve probe 91bde95d (the refused grow) | I expected the kick to wedge (weakly) | the run completed: Score 300, 29 ran / 2 failed / 2 unsupported, 0 resets; "235 grow requests, 0 served, 235 unserved, 0 KiB taken on demand; GPU recoveries 0"; "partial renders 0, out of memory 235" | **no**: the refused grow is survived |

The runner printed `result=record:?` for the stop probe: main's decoder does
not know kind 20 yet. This branch's does (`record.rs`, the golden file).

**What the firmware does with a refused grow.** It carries on. Each of the
235 requests (all from list 1) was answered "no grow (reserve 0, pool
0/0, 512/4096 pages)". Every acknowledgement shows the list unchanged
("fl cur=512 ... ready=0 pending=0"). Every kick completed, with no
recovery. The cost shows in the one scene that needed the pages, bump
high-poly:

| run | frames | us per frame |
|---|---|---|
| gate 3 (grows served on demand) b0c21799 | 2228 | 4490 |
| refill (grows served from the reserve) ed1b5754 | 2226 | 4494 |
| refused grows 91bde95d | 1139 | 8799 |

The refused grow costs that scene 1.96 times its frame time. The score
moves 1% (300 against 303) only because it is a mean over 31 scenes. The
firmware renders in more passes when it is denied pages, which is
consistent with a partial render. It does not report one: UPDATE_STATS
"partial renders" stayed 0.

**What UPDATE_STATS reports.** Element 2 (out of memory), adjust 1, once
for every grow request. That held in all runs: 2 notices with 2 requests
(refill), 235 with 235 (no reserve), and 1 with no request at the
maximum (stop probe). So element 2 is the firmware's out-of-memory notice,
sent with each grow request and alone when the list cannot grow. Cause 2's
trigger is no longer an assumption.

**So the two comments were each right about their own case.**
`freelist.rs`'s old comment was right for a refused grow: the firmware
renders on, at a cost. `pvr_render.c`'s was right for the list at its
maximum: the kick never completes.

**Consequences.**
- A refused grow is not a stop. It is a counted, survivable event: zero
  pages answered, its line ("no grow: the render pool refused K KiB"), and
  the run-end count "U unserved". The stop stays for cause 2 (out of
  memory at the maximum) and cause 3 (a grow past the table). Record value
  1 is no longer a stop cause.
- The guarantee (one grow kept per list, refilled) is still the right
  behaviour for the knob, measured. Served grows kept bump high-poly at
  4494 us/frame against 8799 refused. The refill made both grows' pages
  ready before the firmware asked.

**The terrain lines of the refill and no-reserve runs.** These are "render:
no pool memory for the 256x256 x1 HWRT" (303 in the refill run, 181 in the
no-reserve run), "pvr_backend: submit failed (-12)" and "texture 8: the
kicks using it did not complete; the update is dropped" (202 and 91).
They are the render pool's exhaustion on this branch's frame-set default
(4 x (4096 + 1024), 20800 KiB). They are not a new refusal path:
- `hwrt_get` (`render.rs:767`) is an existing pool consumer. It refuses a
  render target it cannot back and makes the submit return -ENOMEM.
- The texture upload's flush then fails, and `texture.rs:684` drops the
  update, by name.

Gate 3 did not reach them because terrain started with more room. The
first refusal at terrain reads:
- gate 3: "4608 KiB requested, 4248 KiB left";
- refill: "2200 KiB left". That is 4248 - 2048, the two kept reserves.
- no reserve: "1712 KiB left". It took no on-demand grows, so it had
  6320 KiB, served terrain's first 4608 KiB texture and refused the second
  (6320 - 4608 = 1712).

Each close balances ("unaccounted 0 KiB"). The landed frame-set default
(4 x (1024 + 768), 7488 KiB, main e55ac0ab) gives terrain 13312 KiB more,
which the merge brings to this branch.

### The refill pair at the landed frame-set default (3f1d2dab), predictions

Images from 3f1d2dab (the merge of main e55ac0ab):
- the knob on, 4 x (1024 KiB GENERAL + 768 KiB PDS);
- the owner report on: it prints at the first refusal and at close and
  changes no allocation (`src/osal/zephyr.c`);
- no other knob.

Written before they run.

**gl_test:** 37 of 38 (case 48), 0 grow requests, "pool owner freelists:
6 blocks, 6296 KiB", "backend frame sets: 28 blocks, 7488 KiB", every
UPDATE_STATS count 0, "unaccounted 0 KiB".

**glmark2:**
- **Bump high-poly:** the same four grow lines as ed1b5754. At close: "2
  grow requests, 2 served, 0 unserved, 2072 KiB taken on demand; GPU
  recoveries 0", "reserve refills refused 0 (0 KiB); firmware
  UPDATE_STATS: ... out of memory 2 ...". Bump high-poly at about 4490 us
  per frame.
- **Pool:** "pool owner freelists: 8 blocks, 8368 KiB (general 8320, pds 0,
  usc 0, pm 48)", "backend frame sets: 28 blocks, 7488 KiB (general 4400,
  pds 3088)", "unaccounted 0 KiB".
- **Terrain: "no pool memory for the ... HWRT" 0, "the update is dropped"
  0, and no "render pool exhausted" before refract.** The arithmetic:
  - The frame-set run (60d2e073: the same frame sets, the knob off,
    freelists 12440 KiB) ran terrain with no refusal. Its reclaim served
    nothing ("0 served"), so terrain fitted without it.
  - This image holds 12440 - 8368 = 4072 KiB less in freelists by terrain,
    with the same frame sets. So terrain has 4072 KiB more than it had
    there.
  - Against ed1b5754 (the old frame sets) it has 13312 KiB more: 2200 +
    13312 = 15512 KiB at terrain's first request.
- **Refract:** its 1600x1200 depth buffer is still refused: "render pool
  exhausted: 7600 KiB requested, about 6324 KiB left" (2252 + 4072), with
  "draw refused: no depth/stencil buffer". That is not an HWRT line, and
  not a set-up failure.
- **Score line:** 30 ran, 1 failed (shadow, 0x8cdd), 2 unsupported, as in
  60d2e073. Score near its 293: terrain and refract run, both slow.
- **The risk in this prediction:** main's other changes between 60d2e073
  and 3f1d2dab (the fence poll and the kick times, both off by default; the
  triangle and cluster landings) are assumed to allocate nothing more
  before terrain. If terrain does refuse, its first "render pool
  exhausted" line gives the difference.

### gl_test's 12 KiB, and the frees

The flgrow3 gl_test (2515408a) matched every predicted line except the
close: "unaccounted 12 KiB" against my 0. The 0 was my error: I carried it
over from glmark2. The 12 KiB is gl_test.c's own buffers (the correction in
"The owner report on the board" above). `case_image` and `case_rgb565` now
free them at their end. **Prediction for the next gl_test with the frees:
"unaccounted 0 KiB", and pool live 12 KiB lower than 2515408a's 16752 KiB
(16740), every other owner line unchanged.** A case that fails before its
end still keeps its buffer, as `case_viewwin` does.


## The refill glmark2 at the landed default: 536 recoveries (AM62P, 2026-10-04)

The run: 4c25118c from 3f1d2dab (knob on, 4 x (1024 + 768)),
`build/flgrow3-am62p-20261004-034218/fl-grow-glmark2-am62p`. glmark2 FAIL
rc=-5, Score 77, 30 ran / 1 failed / 2 unsupported, "GPU recoveries 536
(HWR counter 536)". land163 was held on it.

In order, from its console:
1. At the start of [shading] blinn-phong-inf (the first time any run grew
   a list in that scene; every earlier run grew at bump), the same four
   grow lines as ed1b5754. The firmware ends at "cur=768 ready=256", and
   the host lists 1024.
2. "CONTEXT_RESET ctxid=2 reason=2 dm=2 ... PC=0 MMU=0", and the same for
   ctxid=3 dm=3. Reason 2 is `RGX_CONTEXT_RESET_REASON_INNOCENT_LOCKUP`
   (`rgx_fwif_shared.h:297`). No context is named guilty.
3. "FREELISTS_RECON count=3 -> reconstructed+ack", then "HWR during fence
   23118 (counter 0 -> 1)": hwrno=1, type 0, no fault address.
4. hwrno=2: "BIF write VA=0x0000000040 TA TPW tag_id=46 ... mmu=0x8021".
   From here every reset carries MMU=0x6e0000000040, each followed by a
   reconstruction: 536 in all.

### The loop: our reconstruction was not upstream's

By reading, against Linux drm/imagination `pvr_free_list_reconstruct`
(Linux 7.3-rc5, drivers/gpu/drm/imagination/pvr_free_list.c, "GPL-2.0-only
OR MIT"):
- (a) Upstream puts every HWRT data that uses the list into
  `RGXFWIF_RTDATA_STATE_HWR` and clears `HWRTDATA_HAS_LAST_TA` (its
  `HAS_LAST_GEOM`). Ours touched no HWRT: our render service writes the
  state only when it builds a member (`render.rs`, `RTDATA_STATE_NONE`).
- (b) Upstream writes only the stack top (from the firmware's current
  count) and the two allocated counts. Ours also wrote current = the host's
  listed pages (1024 here, where the firmware had 768 + 256 ready), ready =
  0 and grow-pending = 0.
- (c) The page order differs (upstream: the build at the top, each grow
  below; ours: the build from the bottom). It is kept: both list the same
  set, and the host tests do not need upstream's order.

(a) and (b) are older than this branch (the C-era port) and are on main's
path. The knob-off path runs the same function. The fix makes
`freelist::reconstruct` do (a) and (b) (`reset_hwrts`). The render service
passes the heap offsets of its built HWRT datas through
`fwccb_service_with`. Host tests
(`reconstruction_leaves_the_firmware_counts_and_writes_the_stack_top`,
`reconstruction_resets_every_hwrt_that_uses_the_list`) both fail on the
old body. The km `ccb` record moves by the read and the stack top only
(tools/pvr/km/README.md).

That (a) and (b) caused the repeating VA 0x40 fault is the hypothesis this
fix tests on the board. A tail-pointer write against a zero base, after
the firmware resumes on HWRT state from before the reset, fits it. It is
not measured.

### Open: "HWR 1 at blinn-phong-inf after the second grow"

The first recovery is an innocent lockup of TA and 3D, with no guilty
context, PC 0 and no fault address. It came about 1 s after the second
grow of list 1, at blinn-phong-inf, a scene that never grew before.
- The console does not attribute it. It is not a freelist page handed over
  at VA 0: the VA 0x40 fault starts only after the first reconstruction.
- What the knob-off control (be707e64, 3f1d2dab, knob off, the same frame
  sets) decides:
  - 0 recoveries: HWR 1 needs the knob's refill path.
  - A recovery right after a grow: it does not need the knob.
- What the control does not decide: why the grows move to blinn-phong-inf,
  and what the TA and 3D were doing at the lockup.
- This item stays open until a run attributes it.

### The images with the fix, predictions (written before they run)

All from the commit that carries this section, AM62P, the landed frame
sets, owner report on (report-only), no other knob:
- **glmark2, knob on** (the failing configuration):
  - HWR 1 may still happen.
  - If it does: one "CONTEXT_RESET ... reason=2", one FREELISTS_RECON, and
    then no "BIF write VA=0x40" loop. The run completes with "GPU
    recoveries 1" (or 0 if HWR 1 does not recur).
  - Score near 293; 30 ran / 1 failed (shadow) / 2 unsupported.
  - "pool owner freelists: 8 blocks, 8368 KiB", with two grows or with
    three. A third grow is served from the refilled reserve, which adds no
    block, and if its refill is refused there is none either: 4c25118c
    read "8 blocks, 8368 KiB" with three grows and "reserve refills
    refused 1 (1036 KiB)".
  - If the loop recurs after the reset, (a) and (b) are not the loop's
    cause, and the hypothesis is wrong.
- **gl_test, knob on:** 37 of 38 (case 48), 0 grows, "6 blocks, 6296 KiB",
  "unaccounted 0 KiB" (the frees), GPU recoveries 0.
- **glmark2, knob off:**
  - Grows from the own reserve: "(768 listed, 768 in reserve ...)",
    "(1024 listed, 512 in reserve ...)", possibly a third at terrain.
  - "pool owner freelists: 6 blocks, 12440 KiB"; GPU recoveries 0 or 1;
    score near 293.
  - If HWR 1 happens here too: one recovery, no loop. The new
    reconstruction runs on the default path, and that run is its board
    measurement there.

### The 8 KiB unaccounted of 4c25118c: a report gap

`hwrt_get` allocates a member's blocks one at a time: tpc, vheap (8 KiB,
`VHEAP_BYTES`), rgnhdr, macrotile, mlist. When one is refused it says "no
pool memory for the WxH HWRT" and returns, and the member keeps what it
got. That is not a leak: the next build of that member reuses those
blocks. At refract's 1600x1200 member (console lines 16106-16109):
- "64 KiB requested, 8 KiB left": tpc refused;
- vheap 8 KiB served (no line; the next line reads 0 left);
- rgnhdr, macrotile and mlist refused.

The owner report skipped a member with neither tpc nor rgnhdr, so that
vheap was "unaccounted 8 KiB". The report now counts every block of every
member, and "members built" counts the complete ones. A host test covers
it (`a_half_built_hwrt_member_is_counted_and_not_built`) and fails on the
old skip. **Prediction for the fix images: "unaccounted 0 KiB" in all
three**, even where an HWRT is refused part way.

## The knob-off control and the fix runs (AM62P, 2026-10-04)

### The control: be707e64 (3f1d2dab, knob off, landed frame sets)

Run dir `build/floff-am62p-20261004-041102`. "glmark2 Score: 294", "30
scenes ran, 1 failed, 2 unsupported", 0 CONTEXT_RESET, 0 FREELISTS_RECON,
0 "HWR during", 0 fence timeouts. That is outcome (A): **HWR 1 needs the
knob's on-demand grow and refill path.** The reconstruction defect is
shared with the default path but was not reached there.

Grows from list 1's own reserve:
- at bump high-poly (:179, :182);
- at refract (:2371, "(1280 listed, 256 in reserve ...)").

"3 grow requests, 3 served", "pool owner freelists: 6 blocks, 12440 KiB",
"unaccounted 0 KiB". The 238 lines that contain "did not complete" are
all "texture N: the kicks using it did not complete; the update is
dropped": the pool's exhaustion at terrain and refract (a submit refused
with -ENOMEM), not a GPU kick. All 238 were checked by the whole line.

**The one line that missed: "out of memory 4" against 3 grow requests.**
I had predicted them equal. At :2371-2374 (04:15:05, refract's first
kicks) come the grow, its acknowledgement and then two UPDATE_STATS
element 2 notices, on consecutive lines, in one FWCCB batch as far as the
console can show (the service loop drains the ring). Which list the second
notice is for is not measurable: UPDATE_STATS carries no freelist ID. It
is list 1 again, or the local list (26 of 32 pages, grow 0), which never
asks for a grow. The rule is "at least one notice per grow request", not
equality. The stop's cause 2 would not have fired with the knob on:
- the batch had a grow, so `at_maximum` returns at once;
- even alone, no list that grows was within one grow of its maximum (list
  1 at 1280 of 4096). The local list has grow 0 and is excluded.

### The fix runs: ebb5a080, run dir `build/flfix-am62p-20261004-042126`

| image | predicted | measured | verdict |
|---|---|---|---|
| glmark2 knob on a71c9b8b | 0 or 1 recoveries, no loop; score ~293; 8 blocks, 8368 KiB; unaccounted 0; terrain 0 HWRT refusals | "Score: 293", 30 / 1 / 2, "GPU recoveries 0"; 0 CONTEXT_RESET, 0 RECON; "8 blocks, 8368 KiB"; "unaccounted 0 KiB"; terrain 0 "no pool memory", 0 dropped | yes. HWR 1 did not recur |
| gl_test knob on 3e40f66f | 37/38, 0 grows, 6296 KiB, unaccounted 0, live 12 KiB lower | "1 of 38 ... case 48", "0 grow requests", "6 blocks, 6296 KiB", "unaccounted 0 KiB"; live 16744, 8 KiB lower, with pending 68 KiB where it was 64 | yes, except live: see below |

**The reconstruction fix is not measured on the board.** HWR 1 did not
recur in a71c9b8b, so no reconstruction ran. "One reconstruction and no
loop" stands only on the host tests (both fail on the old body) and on the
km `ccb` record.

**Where gl_test's third freed page went.** Case 18 (`image`) is the last
case of the run (:1951). Its native-pixmap free comes after the run's last
kick. With reclaim off, a free with no frame open is tagged with fence
last-submitted + 1 (151 here, against "last submitted 150"). No later kick
completes it, so it is still pending at close: "pending frees now 9 (68
KiB)" against 8 (64 KiB). Case 21's window free (8 KiB) came before cases
17 and 18 kicked, so its fence completed and it went back to the pool:
"161 frees (1640 KiB)" against "160 frees (1632 KiB)". Live counts pending
frees, so it is 8 KiB lower, not 12. Unaccounted is 0 because pending is
an owner term.

**In a71c9b8b, after refract's grow**:
- the refill was refused ("reserve not refilled: the render pool refused
  1036 KiB");
- 3 grows were answered "no grow: the render pool refused 1036 KiB
  (1280/4096 pages listed)", survived as the no-reserve run's were;
- 21 out-of-memory notices came with no grow beside them (:788-811).
"6 grow requests, 3 served, 3 unserved", "out of memory 27". No stop: list
1 had room.

### Open: "the grow scene: blinn-phong-inf in 4c25118c, bump in every other run"

What decides when the firmware asks for pages is the parameter manager
running out of the pages it has. That depends on the parameter buffer the
TA writes for the kicks whose pages are not yet released (TA done, 3D not
done), and so on how many kicks are in flight and how big their geometry
is.

Before the first grow the two consoles match scene for scene: build x2,
texture x3, gouraud. The frame counts agree within 4 and the frame times
within 0.2% (e.g. gouraud 3681 / 3685 frames, 2717 / 2713 us). In
4c25118c the grow came at 03:45:09, about 3 s into blinn-phong-inf, with
nothing logged between the scene's start and the grow. In a71c9b8b
blinn-phong-inf ran 3301 frames at 3030 us, and the first grow came at
bump high-poly, as in every other run of this knob and of the control.

What differs between 3f1d2dab and ebb5a080 before the first grow:
- the frees: gl_test code, not in glmark2;
- the reconstruction: acts only after an HWR, and none had happened;
- the owner tally: prints at a refusal or at close, and none had
  happened;
- `fwccb_service_r` now builds its HWRT list (at most 32 entries) on each
  call. That is a timing change, not a behaviour change.
So the same behaviour ran, and the grow fell one scene earlier: a
nondeterminism with no name yet.

The instrument that would name it is per-kick timing around the first
grow: `CONFIG_SAFEUI_GPU_PVR_KICK_TIMES` (main's kick times, off in every
image here) and the PM's allocated page count at each grow acknowledgement.
Of the 286 grow acknowledgements in this host's consoles, 285 read
"alloc=4/0". The one exception is 4c25118c's third grow (03:48:29, after the
recovery loop): "fl cur=1280 top=194 ... alloc=1076/9". So the field does
carry the PM's live use, and why it reads 4 at every other grow is not
known. (An earlier version of this paragraph said the field was not live
use; the exception disproves that.) Until a run shows it, this item and "HWR 1
at blinn-phong-inf after the second grow" stay open together: HWR 1 has
been seen only under the earlier grow.

### The instrument for "the grow scene", and the series

After every FWCCB batch that holds a grow request, the render service now
says what was in flight (`render::grow_context`):
"render: at grow request N: last submitted S, last fragment done D; K kicks
in flight: A with TA done and 3D pending, B with TA pending".
- A kick holds parameter-manager pages from its geometry to the end of its
  fragment. So A (and the geometry of those kicks) is what makes a list run
  out.
- A kick's geometry counts as done when its geometry UFO slot holds its
  sequence or a later one.
- Host test: `the_grow_context_counts_the_kicks_that_hold_pm_pages`. The km
  `render` record gains this line and two invalidates, nothing else
  (tools/pvr/km/README.md).

The PM's allocated count is already in every grow acknowledgement line
("alloc=A/M"), so that line is unchanged.

`CONFIG_SAFEUI_GPU_PVR_KICK_TIMES` adds glmark2's per-scene kick line:
submit, post to geometry and to fragment fence with error bars,
fragment-to-fragment, and the wait. Those are means over each scene, not
the state at a grow; `grow_context` is the state at a grow.

**Predictions for the series** (glmark2, knob on, kick times on, landed
frame sets, four runs), written before they run:
- **Grow scene per run:** 4 of 4 at bump high-poly is what every run but
  4c25118c did. Whatever the split, the reading is per run, not a union.
- **At a grow at bump high-poly:** I expect the line to show 1 or 2 kicks
  with TA done and 3D pending. Bump high-poly's geometry is large
  (high-poly), so a few kicks fill 512 pages.
- **If a run grows at blinn-phong-inf:** I expect more kicks in flight (A
  of 3 or 4: the GPU behind the CPU by the frame sets' depth). Its geometry
  is small, so only many kicks together fill 512 pages. Then that scene's
  kick line should show a longer kick-to-fragment time than the same scene
  in a run without the grow.
  - If A is 1 or 2 there too, the number of kicks does not explain the
    early grow. Something made blinn-phong's geometry larger, and the next
    instrument is the TA's output size.
- **For any run with HWR 1:** the first board measurement of the new
  reconstruction: one CONTEXT_RESET pair, one FREELISTS_RECON, no "BIF
  write VA=0x40", "GPU recoveries 1", and the run completes.
- **Every run:** "pool owner freelists: 8 blocks, 8368 KiB",
  "unaccounted 0 KiB", score near 293. Kick times adds reads of the UFO
  words to every poll of the fence wait (docs/notes/pvr-fence-wait.md).

### The knob-off fix run: ea9fb532 (ebb5a080, knob off)

Run dir `build/flfix-am62p-20261004-042126/fl-fix-glmark2-off-am62p`, as
reported by the lead (the numbers below are the lead's, from its console):
- "glmark2 Score: 294", "30 ran, 1 failed, 2 unsupported; GPU resets 0";
  0 CONTEXT_RESET, 0 FREELISTS_RECON.
- Grows from the own reserve at bump (768 / 768, 1024 / 512) and at refract
  (1280 / 256).
- "3 grow requests, 3 served, 0 unserved; GPU recoveries 0", "refills
  refused 0", "out of memory 4".
- "pool owner freelists: 6 blocks, 12440 KiB"; "driver 15496 + backend 7768
  + gl 520 + pending 1996 = 25780 KiB; unaccounted 0 KiB".

Line for line it is the control's (be707e64): 294, 4 notices, 12440,
25780, 0. Where no HWR occurs, the default path is unchanged by the branch.
The reconstruction did not run here either.

### The reconstruction on silicon: the BGTAG HWR generator

Every fix run had 0 reconstructions. To run the new reconstruction on the
board, a throwaway branch `pvr/fl-grow-hwrgen` (never landed) builds gl_test
with pvr/bgtag's opt-in (`CONFIG_GL_TEST_PVR_BGTAG=y`), which produced 12
recoveries per gl_test run on main.

`d93acfad` does not apply to this branch alone: ten files conflict, because
it needs main's later work. So the throwaway branch is:
- 8092f756;
- merged with origin/main 8b53557f, which merged cleanly;
- `d93acfad` cherry-picked (clean);
- c108abcb cherry-picked (the reconstruction's own line).

Main's BGTAG run (d93acfad, the old reconstruction,
`build/bgtag-am62p-20261004-035241/bgtag-gltest-on`) had:
- 12 CONTEXT_RESET: 7 "reason=1 dm=2 ... PC=0 MMU=0", the rest guilty dm=2
  or dm=3 with PCs;
- 12 FREELISTS_RECON, 12 "HWR during";
- the faults: "write VA=0x0 MMU" (48), "read VA=0x0 IPF" (26), "write
  VA=0x0 TA PSGSTREAM" (5);
- "12 of 40 cases failed (28 passed; 7 timed out)".

**Prediction for the HWR-generator gl_test** (92305d73, tree 1d7cc188, knob
off, BGTAG on), written before it runs:
- About 12 CONTEXT_RESET with the same faults as main's run (MMU write at
  VA 0, IPF reads at VA 0, TA PSGSTREAM). BGTAG makes them; the
  reconstruction does not.
- Each followed by one "FWCCB: FREELISTS_RECON count=3 ->
  reconstructed+ack", and by three lines:
  - "freelist id=1 reconstructed: 512 pages listed; firmware current 512
    ready 0, stack top 511; K HWRT datas reset to HWR";
  - the same for id=2;
  - "freelist id=3 reconstructed: 26 pages listed; firmware current 26
    ready 0, stack top 25; K ...".
  - K is the number of built HWRT members: at least 1, the same for the
    three lists, since every member uses all three.
- No "BIF write VA=0x40".
- The cases as in main's run: about 28 PASS, 5 FAIL, 7 TIMEOUT of its case
  set. The cases after a reset PASS as there.

What this run can and cannot show:
- **(a), the HWRT reset, is exercised:** K > 0 on silicon, and the
  recoveries behave as before or better.
- **(b), the firmware counts left alone, cannot be told from the old
  behaviour here.** gl_test never grows a list, so the firmware's current
  count equals the listed count (512, 26), and the old code wrote the same
  stack top. (b) is exercised only by a recovery after a grow. HWR 1 was the
  one such run, and it has not recurred.

### The HWR-generator run: the HWRT reset on silicon (AM62P, 2026-10-04)

92305d73 (tree 1d7cc188: 8092f756 + origin/main 8b53557f + d93acfad +
c108abcb; knob off, BGTAG on), run dir
`build/hwrgen-kt-am62p-20261004-044429/hwrgen-gltest` (numbers from the
console):
- "PVR_BACKEND_OPT_BGTAG=1".
- 6 CONTEXT_RESET, all "ctxid=2 reason=1 dm=2 ... PC=0 MMU=0".
- Each followed by the three reconstruction lines as predicted, e.g. "freelist
  id=1 reconstructed: 512 pages listed; firmware current 512 ready 0, stack
  top 511; 2 HWRT datas reset to HWR", the same for id=2, and id=3 with 26 /
  26 / 25. Then "FREELISTS_RECON count=3 -> reconstructed+ack" and "HWR
  during fence N".
- K = 2, 2, 3, 5, 5, 5, the same for the three lists each time.
- 0 "VA=0x0000000040".
- "11 of 40 cases failed (29 passed; 6 timed out)": 29 PASS, 5 FAIL, 6
  TIMEOUT.
- "GPU recoveries 6 (HWR counter 6)"; UPDATE_STATS all 0.

Every predicted line held but the count of resets: 6, not about 12.

**Per case, against main's BGTAG run** (d93acfad, the old
reconstruction, `build/bgtag-am62p-20261004-035241/bgtag-gltest-on`). Each
reset is placed in the case whose result line follows it.

| case | main (old reconstruction) | this run (new) |
|---|---|---|
| 2 texture | TA, then 3D (guilty, with a PC) | TA |
| 13 zls | TA, then 3D (guilty, with a PC) | TA |
| 16 msaa | TA | TA |
| 173 splitzs | TA, then TA | TA |
| 21 rgb565 | TA (with a PC), then 3D (with a PC) | TA |
| 17 ctx2 | TA, then TA | TA |
| 18 image | 3D (with a PC) | none |

Every case keeps its first TA reset. Every follow-on reset is gone: the 3D
faults after a TA reset in cases 2, 13 and 21, the second TA reset in
cases 173 and 17, and case 18's lone 3D reset. The two runs list the same
40 cases. The one case result that differs is case 18: TIMEOUT -> PASS.
Main: 28 PASS / 5 FAIL / 7 TIMEOUT; this run: 29 / 5 / 6.

**Reading:** the HWRT reset removed the resets that followed a first
recovery: 12 -> 6 recoveries on the same generator. The old reconstruction
left each HWRT data in its pre-reset state. The next kick on it (3D, or TA
again) faulted, as 4c25118c's TA did at VA 0x40 in a loop. Now each one is
in RTDATA_STATE_HWR with HAS_LAST_TA cleared, and K > 0 shows it was done.
- **The caveat as first written:** one run each, and the two trees differ
  by more than the reconstruction. **Superseded by the isolating A/B
  below**: the same tree with one constant changed (the HWRT reset off)
  brings every follow-on reset back, so the reset is what removed them (one
  run per arm).
- **What stays unexercised:** the counts rule. gl_test never grew a list
  (current = listed = 512 / 26), so the stack top written is the same under
  the old and the new code.

**Landing note:** the HWRT reset was exercised by the BGTAG HWR generator
(6 recoveries, each with one reconstruction and K HWRT datas reset; the
follow-on resets of main's run gone), and isolated by one constant on the
same tree (the A/B below: 12 resets with it off).

**Both halves of the reconstruction are now exercised on silicon:**
- the HWRT reset by the isolating A/B;
- the counts rule (the firmware's current and ready counts left alone) by
  variant B ("The counts rule on silicon: variant B's run").

Open by name: "HWR 1 at blinn-phong-inf after the second grow" and "the
grow scene".

**The isolating arm** (pvr/fl-grow-hwrgen df02c7d1, never landed): the tree
of 92305d73 with one build-time constant, `HWRGEN_RESET_HWRTS = false`,
which skips `reset_hwrts` in `freelist::reconstruct`. The two images
differ in exactly that.

Prediction, written before it runs:
- If the reading is right, the follow-on resets return: about 12
  CONTEXT_RESET, with the 3D after the TA in cases 2, 13 and 21, a second
  TA in 173 and 17, and case 18's lone 3D. Case 18 TIMEOUT. About 28 PASS /
  5 FAIL / 7 TIMEOUT.
- Every reconstruction line reads "... 0 HWRT datas reset to HWR". The
  stack tops are unchanged (512 / 26 as before).
- If it shows about 6 resets with no follow-on, the HWRT reset is not what
  removed them, and the other differences between the trees are. "12 -> 6"
  is then withdrawn as the reset's effect.

### The grow line's sample point, moved

**An instrument defect, found on the board.** 157b76e3's grow line read
"0 kicks in flight" at every request of fl-kt-glmark2-on-1, with "last
submitted == last fragment done" at all 25. It sampled after the whole
FWCCB batch (`render.rs`, `fwccb_service_r` then `grow_context`). By then
the batch had printed its lines synchronously: `CONFIG_LOG_MODE_MINIMAL`,
a direct printk, at 115200 baud (the board's dts, `current-speed`). That is
about 4 lines of 100-200 characters, tens of milliseconds, plus the
acknowledgement wait. A bump frame is 4.5 ms, so every kick had finished
before the sample. The firmware's grow command cannot help: it carries only
`ui64FreelistID` (`RGXFWIF_FWCCB_CMD_FREELIST_GS_DATA`, DDK 25.2
rgx_fwif_km.h:1124-1126), no context or job. The in-flight numbers of
157b76e3's four runs are instrument-defect rows. Their grow scene stands:
the console shows it whatever the line says.

The sample is now taken when the batch's first command is found, before
any command is handled or said (`fwccb_service_with`'s `first_read`,
`render::InFlight`). The line says so: "render: at grow request N (sampled
when the batch was first read): ...". It still comes after the firmware
wrote the command, by at most one wait poll (1 ms, or 20 us with
fine-wait).

The same function had an underflow: a fragment fence past the last
submitted one gave 16 kicks "in flight" and a wrapped range (a panic in a
host build, a garbage count on the board). It now reads 0.

Host test: `the_grow_line_is_sampled_before_the_batch_is_served` posts a
FREELIST_GROW in the wait rig, whose "GPU" completes the kick as soon as a
line is said. It reads "4 kicks in flight: 2 with TA done and 3D pending".
With the sample after the batch, it reads the board's "last fragment done
10; 0 kicks in flight" and fails. The km render record: the two
invalidates move to the batch's first read, and the line text changes.

**Predictions for the new kick-times image, two runs** (written before they
run):
- **The grow scene:** bump in both. A run that grows at blinn-phong-inf is
  the case the series is for.
- **At a bump grow:** "K kicks in flight" with K >= 1 and A (TA done, 3D
  pending) of 1 or 2. Every request must have K >= 1: the PM asks for
  pages while a TA is running, so the requesting kick is in flight with its
  TA pending (B >= 1). A request with K = 0 would mean the sample point is
  still wrong.
- **At refract's grow and its "no grow" answers:** K >= 1 each time, B >= 1.
- **At a blinn-phong-inf grow, if one comes:** A of 3 or 4. If it is 1 or 2
  there too, the in-flight count does not explain the early grow.

### The grow scene, four runs of 157b76e3 (kick times on, AM62P, 2026-10-04)

Run dir `build/hwrgen-kt-am62p-20261004-044429/fl-kt-glmark2-on-{1..4}`,
image f26f9be2 (knob on, landed frame sets). Each run's record is `594b1201
00000125 021e0102 03200258` (293), with 0 CONTEXT_RESET, 0 FREELISTS_RECON
and 0 "VA=0x40". Each row is read from that run's own console:

| run | grow scene | blinn-phong-inf | refract: grow requests / frames / out-of-memory notices | empty FWCCB read | re-posts |
|---|---|---|---|---|---|
| 1 | bump (:182, :187) | clean, 3300 frames, 3030 us | 23 / 23 / 23 | 0 | 2, at terrain's first 256x256 kicks |
| 2 | bump (:180, :185) | clean, 3295 frames, 3035 us | 23 / 23 / 23 | 0 | 1, at terrain's start |
| 3 | bump (:180, :185) | clean, 3297 frames, 3032 us | 12 / 24 / 24 | 1 (05:03:48) | 1 at terrain's start, then 7 at refract after the empty read |
| 4 | bump (:180, :185) | clean, 3298 frames | 23 / 23 / 23 | 0 | 1, at terrain's start |

**The grow scene: bump in 4 of 4.** With a71c9b8b and the knob-off
control and fix runs, every run of this branch's code at the landed frame
sets grew at bump, except 4c25118c at blinn-phong-inf. The item stays open
by name.

**At refract, one out-of-memory notice per frame, and one grow request per
frame until the scene's end.** The request is the refused grow re-asked;
each is answered "no grow: the render pool refused 1036 KiB (1280/4096
pages listed)". Run 3's 12 is not a second count: at 05:03:48 a FWCCB slot
was read empty ("cmd type=0x00000000 (unhandled, skipping)"). From then
the firmware asked for no grow, and every refract kick waited for the 20 ms
re-post. That is the named defect "FWCCB slot read empty (a lost command)",
seen in kt-3 and a71c9b8b: 2 of the 4 runs with refused grows, and no
other console on this host.

**The re-post at terrain's start is not that defect.** It is in every
run, on the first kick after the first 256x256 HWRT is built. That is the
first-kick stall the 20 ms re-post exists for (`wait_repost`, on by default
since 2026-09-24).

The in-flight numbers of these four runs are instrument-defect rows (the
sample after the batch, fixed in 344e2493).

### The isolating A/B: the HWRT reset off, the same tree (AM62P, 2026-10-04)

ba9183cc (pvr/fl-grow-hwrgen df02c7d1: the tree of 92305d73 with
`HWRGEN_RESET_HWRTS = false`, one file, +7/-1, the two .config files
identical). Run dir `build/hwrgen-noreset-am62p-20261004-052343/hwrgen-noreset-gltest`,
read from its console:
- 12 CONTEXT_RESET (8 TA, 4 3D), 12 FREELISTS_RECON;
- all 36 reconstruction lines read "0 HWRT datas reset to HWR";
- 0 "VA=0x0000000040";
- "12 of 40 cases failed (28 passed; 7 timed out)".

| case | main (d93acfad, old reconstruction) | reset ON (92305d73) | reset OFF (ba9183cc) |
|---|---|---|---|
| 2 texture | TA, then 3D | TA | TA, then 3D |
| 13 zls | TA, then 3D | TA | TA, then 3D |
| 16 msaa | TA | TA | TA |
| 173 splitzs | TA, then TA | TA | TA, then TA |
| 21 rgb565 | TA, then 3D | TA | TA, then 3D |
| 17 ctx2 | TA, then TA | TA | TA, then TA |
| 18 image | 3D | none (PASS) | 3D (TIMEOUT) |
| resets / PASS / FAIL / TIMEOUT | 12 / 28 / 5 / 7 | 6 / 29 / 5 / 6 | 12 / 28 / 5 / 7 |

The reset-off arm's 40 case results equal main's run exactly (sorted and
diffed). Against the reset-on arm they differ in case 18 only. The
prediction (69dad921) held line for line.

**The claim, isolated:** the HWRT reset of the reconstruction is what
removes the resets that follow a first recovery. On the same generator and
the same tree, with one constant changed, 12 becomes 6, and case 18 goes
from TIMEOUT to PASS. One run per arm. The counts rule (firmware current and
ready left alone) is still unexercised on silicon.

### The grow line with the sample point fixed: kt2-1 and kt2-2 (344e2493, AM62P, 2026-10-04)

Image b06e00b0 (knob on, kick times on, landed frame sets), run dirs
`build/kt2-am62p-20261004-052630/fl-kt2-glmark2-on-{1,2}`. Both runs:
record `594b1201 00000125 021e0102 03200258` (293), 0 CONTEXT_RESET, 0
FREELISTS_RECON, 0 "VA=0x40", 25 grow lines, and 0 lines with "0 kicks in
flight". The defect's shape is gone. Grow scene: bump in both; blinn-phong-inf
clean (3297 and 3299 frames). Overall at the landed frame sets: 8 runs at
bump, 1 (4c25118c) at blinn-phong-inf.

The lines, the same in both runs (kt2-2's numbers). The firmware's ready
pages are from the acknowledgement of the request before:

| request | scene | firmware's ready pages before it | sampled: K in flight (A TA done and 3D pending, B TA pending) |
|---|---|---|---|
| 1 | bump | 0 (the list is built with none) | 2 (1 + 1) |
| 2 | bump | 256 | 2 (1 + 1) |
| 3 | refract, the grow from the refilled reserve | 256 | 1 (1 + 0) |
| 4 | refract, the refill refused, "no grow" | 256 (0 at its own ack) | 1 (0 + 1) |
| 5..25 | refract, "no grow" | 0 | 3 (2 + 1) |

**Reading.** The firmware keeps "ready pages" it may use on an
out-of-memory event without waiting for the host: DDK 25.2
rgx_fwif_shared.h:965, "reserved pages to be used only on PM OOM event";
Linux drm/imagination `pvr_free_list_reconstruct`, "reserved to allow the
FW to process OOM quickly and asynchronously request a grow".
- With ready pages in hand, the requesting TA goes on and the request is
  asynchronous. So at request 3 the TA had finished before our first read
  (B = 0).
- With none, the TA waits for the answer (B = 1: requests 1 and 5..25).

Bump grows with 1 kick holding pages (A = 1) and refract's refused run with
2 (A = 2). The 3D side does not ask.

**My prediction "B >= 1 at every request" was wrong for requests made with
ready pages.** The rule is "B >= 1 when the firmware had no ready pages".
"A = 1-2 at bump" held. The sample is still up to one wait poll (1 ms here)
after the firmware wrote the command.

What this says about "the grow scene": at bump one kick in flight holds
pages when the list runs out, so bump high-poly's own geometry fills 512
pages. No run with this instrument has grown at blinn-phong-inf, so the
4c25118c item stays open: the series reached bump 2 of 2.

### The counts rule on silicon: the HWR generator after the grows (image B)

The rule: after a recovery, the reconstruction leaves the firmware's
current, ready and grow-pending fields as they are, and writes the stack
top from the firmware's own current count. Every run so far left it
unexercised, because it needs a recovery after a grow.

**Why the plain BGTAG glmark2 (image A, daeef13c, built, not run) cannot
prove it.** Main's own BGTAG glmark2 run (d93acfad,
`build/bgtag-am62p-20261004-035241/bgtag-glmark2-on`) read 483
CONTEXT_RESET, 441 FREELISTS_RECON, and 0 FREELIST_GROW and 0
UPDATE_STATS. With a reset on almost every frame, list 1 never runs out, so
no recovery follows a grow. Every reconstruction there finds current =
listed = 512, ready 0, and the old and the new code write the same stack
top.

**Image B** (97ddb7ac, throwaway pvr/fl-grow-hwrgen 9a7d992d: fb89d3ff +
d93acfad + `CONFIG_GLMARK2_PVR_BGTAG_AFTER_BUMP`) turns BGTAG on when bump
high-poly ends. The knob's grows happen first, on the clean path. The
firmware's acks then read "cur=512 ready=256" and "cur=768 ready=256", and
the host lists 1024.

**Prediction, written before the run:**
- "glmark2: PVR_BACKEND_OPT_BGTAG=1 from the next scene (after bump
  high-poly)" after bump high-poly's frame line. Then resets as in main's
  BGTAG run, each followed by FREELISTS_RECON and three lines.
- **The proof, list 1's line:** "freelist id=1 reconstructed: 1024 pages
  listed; firmware current 768 ready 256, stack top 767; K HWRT datas reset
  to HWR". The old reconstruction (before d2da87fc) would have written
  current 1024, ready 0, grow-pending 0 and stack top 1023, so one such line
  proves the rule on silicon.
- **The case that proves nothing:** if the firmware has used its ready pages
  by the first recovery, the line reads its own counts (e.g. "current 1024
  ready 0, stack top 1023"). Old and new then write the same; the row says
  so, line by line.
- No "BIF write VA=0x40" loop; the run reaches its end (the score is
  irrelevant). Lists 2 and 3 read 512 / 512 / 511 and 26 / 26 / 25.

### The counts rule on silicon: variant B's run (AM62P, 2026-10-04)

Image 97ddb7ac (pvr/fl-grow-hwrgen 9a7d992d), U-Boot boot. Run dir
`build/hwrgen-after-am62p-20261004-060033/hwrgen-glmark2-after`, console
`20261004-0600-glmark2-97ddb7ac/console.txt`; uninstall ok, env diff 0.
The counts below are taken from that console.

**The prediction (4c6334a5) held line for line:**
- :129 "PVR_BACKEND_OPT_BGTAG=0". Bump high-poly grew twice on the clean
  path:
  - 768 from the own reserve, ack "cur=512 top=507 ready=256";
  - 1024 from the refilled reserve, ack "cur=768 top=763 ready=256".
  
  Then 3121 frames at 3.2 ms.
- :194 "PVR_BACKEND_OPT_BGTAG=1 from the next scene (after bump
  high-poly)". From bump normals on there was a reset on almost every frame:
  179 CONTEXT_RESET, 180 FREELISTS_RECON, 0 "VA=0x40".
- **The proof, list 1's line:** "freelist id=1 reconstructed: 1024 pages
  listed; firmware current 768 ready 256, stack top 767; 2 HWRT datas reset
  to HWR". It appears 100 times (:199 to :2007), and the old form "current
  1024 ready 0" 0 times. The old reconstruction would have written 1024 / 0
  / 1023 into the firmware's structure. The new one left its 768 / 256 and
  wrote the stack top from its own count.

**Beyond the prediction:**
- A third grow came at refract, inside the reset loop (:10397, 06:05:00).
  It was served from the refilled reserve to 1280 listed; the refill was
  refused, and the next request got "no grow ... (1280/4096 pages
  listed)". The totals: 4 grow requests, 3 served, 1 unserved, 2072 KiB
  taken on demand.
- The other 79 reconstructions of list 1 read "1280 pages listed; firmware
  current 1280 ready 0, stack top 1279". The old code writes the same
  there, so those lines prove nothing either way. The 100 lines above are
  the evidence.

**What ended the run:** neither the 960 s bound nor the RTI.
- glmark2 ran its whole list: "score 204: 30 scenes ran, 1 failed, 2
  unsupported; ... GPU resets 178" (06:06:44, GLMARK2 record 594b1201
  000000cc).
- In the close, one more reset (counter 178 -> 179) left the last kick
  61721 "did not complete (-5)". The sample then reported "glmark2: FAIL
  rc=-5" and wrote APP record 594b0202 00000002 00000000 000000b3.
- It held 40 s (`YAKOGL_BOARD_HOLD_FAIL_S`) and reset the SoC itself
  ("board: resetting the SoC", 06:07:26), back at 418 s.
- So the failed verdict is the BGTAG generator's last reset failing the
  sample's own close.

## The runaway grow and the stop that did not fire (fwccb-2, AM62P, 2026-10-04)

Image 1f9ff20e (pvr/fwccb-empty-slot 0a9e68ea: main 53328237 + the FWCCB
check; knob on, landed frame sets). Run dir
`build/fwccb-am62p-20261004-054022/fwccb-glmark2-on-2`. The same image as
fwccb-1, which ran clean (473). Run 2: board.sh verdict=no_record; the RTI
reset the SoC.

**What the console shows, in order:**
1. Bump high-poly, the two usual grows. The acks read "fl cur=512 ...
   ready=256" and "cur=768 ... ready=256".
2. Bump normals runs clean (727 fps).
3. At bump height (05:50:00), in **one FWCCB batch** (one grow-context line
   follows it, "at grow request 14"), twelve grows, each served from a
   refilled reserve: 1280, 1536, ... 4096.
   - Each ack came in 1 ms and read "ready=0 ... alloc=4/0".
   - Each answer was followed at once by the next request.
   - At 4096 (the list's maximum): "cur=4096 top=4091 ready=0", then two
     out-of-memory notices with no grow beside them (seen 15, 16).
4. "fence 73784 (frame fence) not moved at 20 ms, re-posted", then
   nothing for 20 s. At 05:50:20 "rti_arm: the sample stopped saying it is
   alive; the RTI resets the SoC", and in the same second the wait's own
   "fence 73784 timeout".
5. No "render: stopped", no GPU_STOP record, no CONTEXT_RESET, no empty
   FWCCB slot.

### (1) The firmware's ready pages, and what our answers never give it

The firmware's ready pages in every grow acknowledgement, by run:

| run | acks with ready = 256 | then |
|---|---|---|
| kt-1, kt2-1 | 3 (cur 512, 768, 1024) | 22 x "cur=1280 ready=0", one re-ask per refract frame, each refused |
| fwccb-1 | 3 | 23 x "cur=1280 ready=0", one per refract frame, refused |
| fwccb-2 | 2 (512, 768) | "cur=1280 ready=0" at bump height, then 11 more served back to back to 4096 |

In every run, once the firmware's ready pages were 0, every answer was
followed by another request:
- one per frame when the answer was a refusal (the firmware renders on, then
  asks at the next out-of-memory event);
- immediately when the answer was a grow.

While it held ready pages, it asked about once per scene.

**Our protocol never gives the firmware ready pages:**
- `freelist::build` writes FREELIST_READY_PAGES 0 and lists the would-be
  ready pages as current;
- `freelist::grow` answers DELTA = grow, NEW = listed, READY = 0
  (`t.ready` is always 0).

Linux drm/imagination (pvr_free_list.c, Linux 7.3-rc5, MIT) reserves them
at creation and after every grow:
- `ready_pages = min(current_pages * grow_threshold / 100, grow_pages)`,
  aligned (`calculate_free_list_ready_pages_locked`);
- "to allow the FW to process OOM quickly and asynchronously request a
  grow";
- the grow answer is `new_pages = current + ready`, `ready_pages = ready`
  (`pvr_free_list_process_grow_req`).

The 256 ready pages in our acks are the firmware's own split of a grow we
answered with READY 0. Once it had spent them, nothing gave them back.

**Reading (a hypothesis, consistent with every ack above, not
isolated):**
- The firmware asks again as long as its ready pages are 0, and our answers
  keep them at 0.
- With refusals that is a request per frame (the refract tails).
- With grows served at once from a refilled reserve it is a runaway to the
  list's maximum, and at the maximum the probe's wedge: the out-of-memory
  event with no grow possible, which never completes.
- Why fwccb-2 spent its ready pages at bump height, where fwccb-1 did not,
  is the same unnamed variation as "the grow scene". The runaway that
  followed is what the protocol makes of it.
- Whether HWR 1 (4c25118c, a lockup right after the second grow) is the
  same class is unmeasured. Its first grow's ack read "cur=768 ready=0",
  where every clean run's first read "cur=512 ready=256"; its second read
  "cur=768 ready=256". The firmware spent its ready pages at once there
  too. That is consistent, but not shown.

The isolating test is the protocol fix itself: reserve ready pages as the
DDK protocol does (the threshold to be taken from the DDK's or Mesa's
value, not chosen), answer with them, and run the same image config. The
prediction: no back-to-back grows and no re-ask per frame. Not done: no fix
before this is read.

### (2) Why the at-maximum stop did not fire, and why the watchdog won

- `ccb::at_maximum` runs once per FWCCB batch and returns at once when the
  batch held any grow request (`grows != 0`): "with a grow beside the
  notice the notice is the grow's". The two lone notices at 4096 came in the
  same batch as the twelfth grow, so the stop was never considered. No later
  batch came, because the firmware sent nothing more. The condition the
  stop was built for was met (list 1 at 4096 of 4096, notices beyond the
  grow's own) and missed by the batch rule.
- The rule must look at the notices after the batch's last grow:
  - each grow brings its own notice;
  - a further notice, with a growable list at its maximum, is the probe's
    case;
  - in the knob-off control, the fourth notice came with room left (1280 of
    4096), so it stays no stop.
- The wait then sat in its 20 s timeout. The RTI's "alive" window is also
  20 s, so the watchdog reset the SoC in the same second as the timeout. A
  wedge the stop does not recognise ends at the watchdog with no record.
  The stop is meant to make that impossible, and it did not.

Both are defects of the knob's path (and of the stop built for it). The
branch's default path is not affected: knob off, the own reserve ends at 4
grows and then refuses. The knob stays opt-in and must not default on
before both are fixed and measured.

### Fix (2): the at-maximum rule, and the stop's own time

- **The rule** (`ccb::at_maximum`): it counts the out-of-memory notices
  after the batch's last grow request. A batch with no grow needs one notice
  (the probe's case). A batch with grows needs two after the last one: each
  grow brings its own notice, so the second is "beyond" it (fwccb-2's case).
  The stop then also needs a growable list without room for another grow.
  Kept as no stop:
  - the knob-off control's fourth notice (two after refract's grow, 1280 of
    4096: room left);
  - a grow that reaches the maximum with only its own notice.
- **The stop's own time** (`render::STOP_WAIT_MS` = 10 s, cause 4
  "a kick that did not complete in the stop's time"): with the knob, a fence
  that has not moved for 10 s stops the render service and ends the run with
  GPU_STOP. That is half the board's 20 s "alive" window. The stop then
  prints the driver's run-end lines (about 15 lines at 115200 baud, about
  0.2 s) and the record well before the watchdog. 10 s is over 12 times the
  longest frame on record (refract 435 ms; phong in the 536-recovery loop
  787 ms). `YD_PVR_STOP_WAIT_MS` mirrors it in yakogl_drivers.h (a host test
  keeps them equal), and pvr_render.c asserts the alive window is at least
  twice it. A record that races the watchdog is not a record.
- **Host tests:**
  - `ccb::at_maximum_tests` runs fwccb-2's exact batch through the FWCCB
    service: twelve grows from 1024 to 4096 (two notices after the first,
    one after each other), then one more notice. It stops at the maximum;
    with the old rule it does not (checked by putting the old rule back).
    The control's shape (a grow to 1280, two notices) and a grow to 4096 with
    its own notice do not stop.
  - `render` wait tests: with the knob, a fence that never moves returns
    -ESHUTDOWN after exactly 10 s with cause 4. Without the knob it is the
    old 20 s timeout.

### Fix (1): the firmware's ready pages, as the protocol reserves them

fwccb-2 read as: once the firmware's ready pages are 0, every grow answer
brings the next request (one per frame when refused, back to back when
served), up to 4096. Our protocol never gives ready pages: build writes
READY 0 and lists them as current, and every grow answers READY 0.

- **Upstream** (Linux 7.3-rc5 drivers/gpu/drm/imagination/pvr_free_list.c,
  GPL-2.0-only OR MIT, used under MIT):
  - `calculate_free_list_ready_pages_locked`: `pages * grow_threshold / 100`,
    at most one grow, rounded down to 4 pages.
  - `free_list_fw_init`: CURRENT = initial - ready, READY = ready,
    stack top = CURRENT - 1, the current base from CURRENT.
  - `pvr_free_list_process_grow_req`: current += ready, ready = 0, grow,
    then (in `pvr_free_list_grow`) ready = the share of the new total and
    current -= ready. The answer is NEW = current + ready, READY = ready.
    A refused grow answers READY 0.
  - `pvr_free_list_reconstruct`: lists every node, then takes the ready
    pages off the host's current count.
- **The threshold** comes from Mesa 26.2.1, never chosen here:
  `PVR_GLOBAL_FREE_LIST_GROW_THRESHOLD 13U`
  (src/imagination/vulkan/pvr_arch_device.c:47-50, "intended to be 12.5%,
  but has been rounded up"), passed for the global list at :785. The local
  list is created with 0 (pvr_arch_job_render.c:660). Ours has no grow, so
  its share is 0 at any threshold. At the 64 MiB carveout's 512 pages,
  13 % is 66, aligned 64: 448 current + 64 ready, which is the DDK's sizing
  the C already passed (and folded into current).
- **The switch:** `CONFIG_SAFEUI_GPU_PVR_FL_READY_PAGES`, its own opt-in,
  independent of the grow knob, so both arms can run with it.
  `yd_pvr_fl_set_ready_pages` sets `fl_ready_threshold` (was `pad0`); each
  list keeps its own `grow_threshold` (was `pad`). With it off both are 0
  and every word is the old one: the km records are unchanged.
- **What changes with it on** (list 1, 64 MiB carveout):

  | when | CURRENT | READY | answer DELTA / NEW / READY |
  |---|---|---|---|
  | build | 448 | 64 | - |
  | grow 1 | 672 | 96 | 256 / 768 / 96 |
  | grow 2 | 892 | 132 | 256 / 1024 / 132 |
  | grow 3 | 1116 | 164 | 256 / 1280 / 164 |
  | a refused grow | all listed | 0 | 0 / listed / 0 |

  From the sixth grow on (2048 listed; 13 % is 266), the share is a whole
  grow (256).
- **Host tests** (`freelist::ready_pages_tests`, the real bump and MMU):
  - build's firmware words (448/64/447, base at (4096 - 448) * 4);
  - the answers of two served grows and a refused one;
  - the old words with the threshold 0;
  - a reconstruction lists all 768 pages after a grow, and the audit is clean.
  
  Known negatives, each checked by putting today's line back, and each
  fails its test: build with READY 0; a grow with ready 0; a reconstruction
  and `page_phys` that count only the current pages. The refill's room check
  (now on current + ready) has no test of its own: it only matters within
  one grow of the maximum.

### The run with both fixes: predictions (written before it runs)

Image c18f9bfd (pvr/fl-grow-on-demand 678661c1: the two fixes 1b50e97a and
678661c1 on e3e2e39d; knob on, ready
pages on, kick times off, landed frame sets, the base of fwccb-1:
land162's swap is in, so the score is near fwccb-1's 473, not 293).
`build/fl-ready-glmark2-on-am62p`. It runs after variant B.

Line by line:
- **Boot:** "render: freelist ready pages: the global lists keep the
  firmware's ready pages" once. The build lines are unchanged
  ("freelist id=1 np=512 max=4096 ...", id=2 the same, id=3 np=26 max=32).
  The firmware is told 448 current and 64 ready for lists 1 and 2.
- **The first grow** (bump high-poly, as in 8 of 9 runs; blinn-phong-inf
  would be the 4c25118c case again):
  - host: "FWCCB: FREELIST_GROW id=1 -> +256 pages (768 listed, 0 in
    reserve, pool 0/0)", then the refill line;
  - grow line: B = 0 (the firmware had 64 ready pages when it asked, so
    the TA did not wait: kt2's rule);
  - **ack: "fl cur=672 ... ready=96"**: the share of 768, as we answered.
    Under the old protocol the first ack read "cur=512 ready=256" (the
    firmware's own split of a READY 0 answer). If it reads that again, the
    firmware does not take READY from this answer, and the row says so.
- **The second grow** (bump): "(1024 listed ...)", ack "cur=892 ...
  ready=132", B = 0.
- **No back-to-back grows:** no FWCCB batch holds two grow requests for
  one list, and no ack is followed at once by the next request. In
  fwccb-2 there were twelve.
- **Refract:** the grow to 1280 (ack "cur=1116 ready=164"), then the
  refill is refused as in every run (the render pool's 1036 KiB). The next
  grow is refused. **After a refusal the protocol answers READY 0, as
  upstream does** (`pvr_free_list_process_grow_req`: the firmware has
  already taken its ready pages; a failed grow gives none back). So from
  then on the firmware has none, as today: one request per refract frame,
  each "no grow: the render pool refused 1036 KiB (1280/4096 pages
  listed)", acks "cur=1280 ready=0", B >= 1. No stop, no wedge (a refused
  grow is survived; the list has room).
- **Terrain and refract** as before: the re-post at terrain's first
  256x256 kick, refract's frame time as in fwccb-1.
- **If the maximum is reached anyway:** "render: stopped: the parameter
  manager out of memory with its freelist at the maximum (list N, ...)"
  and a GPU_STOP record with tag 2. Or, for a wedge no rule recognises,
  tag 4 after 10 s. Never the watchdog with no record.
- 0 CONTEXT_RESET, 0 FREELISTS_RECON, "GPU recoveries 0", the run
  completes.

### The run with both fixes: c18f9bfd (AM62P, 2026-10-04)

Image c18f9bfd (678661c1; knob on, ready pages on), U-Boot boot. Run dir
`build/flready-am62p-20261004-061334/fl-ready-glmark2-on`, console
`20261004-0613-glmark2-c18f9bfd/console.txt`; uninstall ok, env diff 0.

**What held:**
- Score 470: 30 ran, 1 failed, 2 unsupported. Record 594b1201 000001d6
  021e0102 03200258.
- 0 GPU resets, 0 CONTEXT_RESET, 0 FREELISTS_RECON, unaccounted 0 KiB.
- :122 "render: freelist ready pages: the global lists keep the firmware's
  ready pages".
- No "render: stopped". List 1 ended at 1280 of 4096, so the at-maximum
  rule correctly did not fire on the lone notices below.
- Close: "6 grow requests, 3 served, 3 unserved, 2072 KiB taken on
  demand; GPU recoveries 0", "reserve refills refused 1 (1036 KiB)",
  27 out-of-memory notices.

**(i) The prediction "the first ack reads cur=672 ready=96" failed.** The
acks are word for word those of the old protocol:

| ack | fwccb-1 (old protocol, 1f9ff20e) | c18f9bfd (ready pages) | predicted |
|---|---|---|---|
| build line, list 1 | cur_dev=0xfa00003800 (CURRENT 512) | **cur_dev=0xfa00003900 (CURRENT 448)** | 0x3900 |
| grow 1 (bump) | cur=512 top=507 ready=256 | cur=512 top=507 ready=256 | cur=672 ready=96 |
| grow 2 (bump) | cur=768 top=763 ready=256 | cur=768 top=763 ready=256 | cur=892 ready=132 |
| grow 3 (refract) | cur=1024 top=1019 ready=256 | cur=1024 top=1019 ready=256 | cur=1116 ready=164 |
| first refusal | cur=1280 top=1275 ready=0 | cur=1280 top=1275 ready=0 pending=1 | - |

Reading, from the code and the ack arithmetic:
- **The image did write the share.** The build line's current base,
  0xfa00003900, is arr + (4096 - 448) * 4. The old build wrote 0x3800
  ((4096 - 512) * 4), and so did fwccb-1 and variant B. The C switch
  reached `yd_pvr_fl_set_ready_pages` (:122), and `build` wrote CURRENT 448
  and READY 64.
- **The answer words are not on the console.** No line prints them. They
  come from the code the host tests pin (256/768/96, 256/1024/132,
  256/1280/164), at the offsets the DDK pins check (DELTA 12, NEW 16,
  READY 20 in the KCCB command; `ddk_abi_pins.h`
  RGXFWIF_FREELIST_GS_DATA). This is the one link not observed on silicon.
- **The firmware's split is the same under both answers.** After every
  served grow the ack reads current unchanged and ready = DELTA (256),
  whether we answered READY 0 (NEW 768) or READY 96 (NEW 768). Neither NEW
  nor READY changes a word of the ack. At its next out-of-memory event the
  firmware moves those 256 into current (the next ack's current is the
  last one + 256).
- So this firmware (36.53.104.796) keeps each grow's own pages as its
  ready pages, whatever READY says, at least for a grow it asked for while
  out of memory. The DDK's comments are all we have:
  - rgx_fwif_km.h:767, "Number of ready pages to be held in reserve until
    OOM";
  - rgx_fwif_shared.h:965, "Reserved pages to be used only on PM OOM event".
  
  The firmware itself is not readable. What READY in a grow answer means to
  this firmware is open by name: **"the grow answer's READY word has no
  effect visible in the ack"**.
- **The fwccb-2 reading (1) is withdrawn as stated.** It said "our answers
  never give the firmware ready pages, and that makes the runaway". But the
  firmware gives itself the grow's 256 as ready pages under the old answer
  too. fwccb-2's acks read "ready=0" because, within the 1 ms before the
  read, the firmware had already moved them into current: it was still out
  of memory. The runaway is then a TA at bump height that needed more and
  more pages, up to 4096 pages, in one run of five, where every other run
  grew 0 times there. That is the "grow scene" variation again, not the
  protocol.
- **The ready-pages switch stays opt-in.** It is not yet doing what the
  protocol says, per the acks. No change before the item is read.

**(ii) The lost command recurred, without the check.**
- :790, :795, :800: three "no grow" answers at refract (acks after 1, 9 and
  8 ms).
- Then :804 "FWCCB: cmd type=0x00000000 (unhandled, skipping)", right after
  the 8 ms ack.
- After it: no further grow request (6 in all, where refract asked ~24
  times in the other runs) and lone out-of-memory notices (seen 7 to 27).
  :806 to :820 hold 8 "frame fence not moved at 20 ms, re-posted" lines.
  Refract ran 25 frames.
- That is 3 of 5 runs with refused grows without the check (a71c9b8b,
  kt-3, c18f9bfd), and 0 of 2 with it (fwccb-1; fwccb-2 never reached a
  refused grow, so it is no case at all). So the check's runs are **1
  case, not 2**: fwccb-1.
- **What the check changes on a poll that finds no empty slot** (0a9e68ea,
  `ccb.rs`):
  - `write_offset_check`: a compare of the write offset already read, and
    two stores to `YdPvrKm` (normal memory, not the control line);
  - a compare of the slot's type against the command magic.
  
  It adds no invalidate, no flush, no read of the control line or the
  slot, no barrier and no wait. The re-read (20 us steps, its own
  invalidates) runs only after an empty read. So on the path of a run with
  no empty slot the check adds a few instructions and nothing that touches
  the cache or the GPU. The other difference between the images is the
  km struct's layout (three new fields after the existing ones), which
  moves nothing the firmware reads.
- The rates do not separate the arms: with 3 empty reads among 6 cases
  (5 without, 1 with), a clean case with the check has probability 1/2.
  The check's "absence of the defect" is not evidence of a mechanism. The
  item stays open by name, and the next run with the check that reaches
  refused grows is the next case.

### Reopened: "the grow answer's READY word has no effect visible in the ack" (the sources)

Closed by the run of 70b79a74 below ("The firmware's counts before the
answer: the run"), then **reopened by d320c8a5's first ack**, which reads
our READY ("The one-kick-in-flight run" below). The reading from the
sources stays as it was written.

Host-only reading. No code.

**(1) Linux drm/imagination** (Linux 7.3-rc5, pvr_free_list.c, MIT). It
supports this exact firmware version: pvr_device.c:535
`PVR_PACKED_BVNC(36, 53, 104, 796)`, pvr_drv.c:1539
`MODULE_FIRMWARE("powervr/rogue_36.53.104.796_v1.fw")`. That is
linux-firmware's build; ours is TI's DDK build `rgx.fw.36.53.104.796`, so
the two firmware binaries may differ.
- `pvr_free_list_process_grow_req` (:374-388), the comment and the code:
  "Since the FW made the request, it has already consumed the ready pages,
  update the host struct." `current_pages += ready_pages; ready_pages = 0`.
  Then the grow (in `pvr_free_list_grow`, :336-342, "Reserve a number ready
  pages to allow the FW to process OOM quickly and asynchronously request a
  grow": ready = the share of the new total, current -= ready). Then
  `resp->delta_pages = grow_pages; resp->new_pages = current_pages +
  ready_pages; resp->ready_pages = ready_pages`.
- It **never reads the firmware's ready or current count back** after an
  answer: no `fw_data->ready_pages` or `fw_data->current_pages` read
  anywhere in the grow path. Its own counts change before the answer, from
  its own arithmetic.
- The only read of the firmware's counts is in the reconstruction (:586):
  `fw_data->current_stack_top = fw_data->current_pages - 1`. The host's
  `current_pages -= ready_pages` (:583) there uses the host's ready, which
  only positions the pages in the array.
- So upstream expects READY to be the firmware's new reserve, but nothing
  in it checks what the firmware did with it.

**(2) The DDK headers we vendor** (ti-img-rogue-ddk-25.2, include/rogue):
- `RGXFWIF_FREELIST_GS_DATA` (rgx_fwif_km.h:760-768), the KCCB
  FREELIST_GROW_UPDATE's data: "Amount of the Freelist change" (DeltaPages),
  "New amount of pages on the freelist (including ready pages)"
  (NewPages), "Number of ready pages to be held in reserve until OOM"
  (ReadyPages).
- `RGXFWIF_FREELIST` (rgx_fwif_shared.h:944-967), the firmware's own
  struct: CurrentPages "Total no. of pages made available to the PM HW";
  bGrowPending "Freelist grow is pending"; ReadyPages "Reserved pages to be
  used only on PM OOM event".
- `RGXFWIF_FWCCB_CMD_FREELIST_GROW` (rgx_fwif_km.h:1186-1189): "Requests
  an on-demand freelist grow"; its data is only the freelist ID
  (:1124-1126).
- Nothing says when the firmware applies NewPages or ReadyPages, or what
  it does on an answer to a grow it asked for while out of memory.

**(3) Mesa 26.2.1's pvr** never answers a grow. The Vulkan driver passes
`grow_threshold` to the kernel once, at creation (pvr_free_list.c:133-134
into `pvr_drm_job_render.c:85` `.grow_threshold` for drm/imagination, and
`pvr_srv_job_render.c:101-102` for the DDK's kernel). The kernel answers.
Mesa has no FREELIST_GROW handling and no ready-page arithmetic.

**What the acks' arithmetic supports** (fwccb-1 under the old answers,
c18f9bfd under the new; the table in "The run with both fixes"):

| hypothesis | grow 1 predicts | grow 2 predicts | observed (both runs) | verdict |
|---|---|---|---|---|
| ready := answer READY | ready 0 (old), 96 (new) | 0, 132 | ready 256, 256 | rejected |
| current := NEW - READY | 768 (old), 672 (new) | 1024, 892 | cur 512, 768 | rejected |
| ready := DELTA, current := NEW - DELTA | 512 / 256 | 768 / 256 | 512/256, 768/256 | fits both runs |
| ready := DELTA, current := own current + own ready (the OOM consumed the reserve) | 512 / 256 | 768 / 256 | the same | fits both runs |

- **Supported:** "the firmware moves DELTA pages to ready on a served
  grow, regardless of READY (and of NEW)". It fits both runs, every served
  grow, and no ack word moved when READY changed from 0 to the share.
- **Not decided: "READY only matters at init".** Build's READY 64 reached
  the firmware (current base 0x3900). The first ack's current, 512, equals
  448 + 64 (the reserve moved in at the out-of-memory event). But it also
  equals NEW - DELTA, and the old build's 512 + 0. The acks cannot tell
  whether the init READY was used as a reserve or folded in some other
  way. Telling them apart needs an ack before the first grow (a read of
  the firmware's FREELIST at the first FREELIST_GROW, before the answer).
  That is an instrument, not a fix, and not built.
- **Not decided: whether READY in an answer matters in some state the
  runs never reached.** For example, an answer to a grow the firmware
  asked for with ready pages still in hand (B = 0 at the request), or
  outside an out-of-memory event. No source says.

The sources do not decide it. Upstream writes READY and never looks
again; the DDK's comments describe the fields, not the firmware's update
rule; Mesa never sees an answer. The item stays open with the sentences
above.

### The instrument: the firmware's counts before the answer, and its predictions

2613f5c9 prints "FWCCB: grow req id=N: fw before the answer:
cur=C top=T ready=R pending=P alloc=A/M; host H current + R' ready"
(`freelist::fw_before_the_answer`). It reads the list's RGXFWIF_FREELIST,
invalidated first, when our FWCCB service handles the request, before
`serve` and before the answer is posted. It is under the ready-pages
switch, so it prints only for a list built with a share. The existing ack
line shows the same words after the answer.

**The sample point.** The line is read up to one poll after the firmware
wrote the request. A firmware that asked while it still held ready pages
does not wait for us, so its words can move in between. A request made
with none (B >= 1 in the grow line) waits.

**Image 70b79a74** (pvr/fl-grow-on-demand 2613f5c9; knob on, ready pages on,
kick times off, landed frame sets; the base of c18f9bfd). Predictions per
grow, written before it runs:
- **Grow 1 (bump high-poly), before.** This decides "READY matters at
  init":
  - "cur=512 ready=0 pending=1" (host 448 + 64): the firmware took build's
    64 ready pages at its out-of-memory event, then asked. Build's READY
    was used, as a reserve.
  - "cur=448 ready=64": it asked with build's reserve untouched. Build's
    READY was then not the reserve the PM fell back on, and the first ack's
    512 came from NEW - DELTA.
  - Anything else is a third reading, recorded as it reads.
- **Grow 1, after (the ack):** "cur=512 ready=256", as in c18f9bfd.
- **Grow 2 (bump), before:** "cur=768 ready=0 pending=1": the 256 of grow
  1 moved into current at this out-of-memory event. After: "cur=768
  ready=256" while we answered READY 132. The pair shows the DELTA move:
  ready from 0 to 256 = DELTA, not READY.
- **Grow 3 (refract):** before "cur=1024 ready=0", after "cur=1024
  ready=256".
- **The first refusal:** before "cur=1280 ready=0", after "cur=1280
  ready=0" (DELTA 0).
- **The rest as c18f9bfd:** about 470, 0 resets, no stop. The lost
  command may recur after a refusal: 3 of 5 refused-grow runs without the
  FWCCB check, and this image has none.

### The firmware's counts before the answer: the run (AM62P, 2026-10-04)

Image 70b79a74 (2613f5c9; knob on, ready pages on), U-Boot boot. Run dir
`build/flbefore-am62p-20261004-064909/fl-before-glmark2-on`; uninstall ok,
env diff 0.
- Score 473: 30 ran, 1 failed, 2 unsupported. Record 594b1201 000001d9
  021e0102 03200258.
- 0 GPU resets, 0 CONTEXT_RESET, 0 "cmd type=0x00000000", 0 re-posts.
- "25 grow requests, 3 served, 22 unserved, 2072 KiB taken on demand",
  unaccounted 0. 25 "before" lines, one per request.

| request | before the answer (the firmware's words; the host's) | our answer DELTA / NEW / READY | the ack after |
|---|---|---|---|
| 1, :180 (bump) | cur=512 top=507 ready=0 pending=1; host 448 + 64 | 256 / 768 / 96 | cur=512 top=507 ready=256 |
| 2, :186 (bump) | cur=768 top=763 ready=0 pending=1; host 672 + 96 | 256 / 1024 / 132 | cur=768 top=763 ready=256 |
| 3, :771 (refract) | cur=1024 top=1019 ready=0 pending=1; host 892 + 132 | 256 / 1280 / 164 | cur=1024 top=1019 ready=256 |
| 4, :789 (refract, refused) | cur=1280 top=1275 ready=0 pending=1; host 1116 + 164 | 0 / 1280 / 0 | cur=1280 top=1275 ready=0 |
| 5-25 (refused) | cur=1280 top=1275 ready=0 pending=1; host 1280 + 0 (21 lines, identical) | 0 / 1280 / 0 | cur=1280 ready=0 |

The answers are the code's (pinned by the host tests); the other columns are
read from the console. Every prediction in 16fbdb40 held, with prediction
1's first case.

**The item is closed:**
- **READY at init is a reserve the firmware uses.** Build wrote CURRENT 448
  and READY 64. Before any answer the firmware shows current 512, ready 0
  and a grow pending: it moved build's 64 into current at its
  out-of-memory event, then asked.
- **In a grow answer the firmware takes NEW and sets ready = DELTA.**
  - At every served grow, current is unchanged across the answer, and
    ready goes from 0 to 256 = DELTA. We answered READY 96, 132, 164.
  - Current + ready after the answer equals our NEW (768, 1024, 1280).
  - In every run NEW = the firmware's own total + DELTA, so whether it
    reads NEW or adds DELTA is not separated. The READY word changed
    nothing.
- So the protocol's reserve exists on this firmware, but after a grow its
  size is the grow's own pages, not the share we compute.
- Our share matters only at build (64 of 512 pages). The ready-pages
  switch therefore changes one thing: at the first out-of-memory event of
  each global list the firmware has 64 pages to go on with, instead of 0.
  It stays opt-in.

**Every ack took 1 ms (25 of 25).** In fwccb-1, -3 and -4, the refused
grows' acks took 4-11 ms, and 5 of 5 served ones 1 ms. This image prints
the "before" line (about 120 bytes) before the answer is posted. The
console is synchronous (`CONFIG_LOG_MODE_MINIMAL`, 115200 baud: about 10
ms). The refused grow's "no grow" line was already printed before the
post in every image. So here the answer is posted about 10 ms later after
the request, and the firmware reads it at once.
- **Reading:** after a request it will be refused, the firmware is busy
  for up to about 10-20 ms. An answer posted in that window waits; one
  posted after it does not.
- This run had no lost command either (no check; 0 type-0 skips with 22
  refusals). Every lost command so far came right after an 8-9 ms ack.
- But fwccb-1, -3 and -4 had 4-11 ms acks with no lost command. So
  "posted while the firmware is busy" is not enough for the defect by
  itself. It is recorded as a timing fact about the item, not a cause.

### fwccb-5 and fwccb-6: a refused grow that hung (AM62P, 2026-10-04)

The FWCCB-check image 1f9ff20e (pvr/fwccb-empty-slot 0a9e68ea: main
53328237 + the check; knob on). Run dir
`build/fwccb3-am62p-20261004-065708/fwccb-glmark2-on-{5,6}`.

The stable copies (gitignored, records/README.md):
- `records/am62p/fwccb3-20261004-065708/fwccb-glmark2-on-5/20261004-0657-glmark2-1f9ff20e/console.txt`:
  sha256 b01684e12104d883..., 1058 lines;
- `records/am62p/fwccb3-20261004-065708/fwccb-glmark2-on-6/20261004-0703-glmark2-1f9ff20e/console.txt`:
  sha256 27444cfa183a240a..., 1180 lines.

The times below are those consoles' timestamps.

**fwccb-5:**
- 471, record 594b1201 000001d7, "slots read empty 0 (filled within 2000
  us 0, still empty 0); write offset went back 0".
- "26 grow requests, 3 served, 23 unserved", all 23 refusals at refract.
- Conditionals then ran 8507 frames at 1.17 ms.
- With the check, 0 empty slots in the 4 runs that completed (1, 3, 4, 5).

**fwccb-6: no record.**
1. Refract ended at 07:08:15 (last submitted 100556) after 23 refusals that
   rendered on (acks 7-9 ms).
2. Conditionals then ran about 191 frames normally: request 27 reads "last
   submitted 100749", 193 kicks later, in the same second.
3. Then the 24th refusal: "no grow ... (1280/4096 pages listed)", ack after
   1 ms (KCCB r/w=4/4), out-of-memory notice 27.
4. Kick 100748's TA never completed. Its geometry UFO slot still held
   100492, frag[0] waited on it, and the fragment UFO stayed at 100747.
   "freelist1 cur=1280 top=1275 ... alloc=4/0; fwccb grow req/done=27/3".
5. The 20 ms re-post did not move it. There were three 20 s timeouts
   (07:08:35, :56, 07:09:16).
6. Conditionals' frame line averaged them out ("193 frames ... 318117
   us"). "rti_arm: the sample stopped saying it is alive" came at 07:08:35,
   and the SoC reset at 07:09:18.

**Reading:**
- **No record:** the image is older than fix (2) (1b50e97a), so its wait
  was 20 s, the same as the alive window: fwccb-2's race. With land177 the
  hang ends at 10 s with GPU_STOP tag 4. The at-maximum rule rightly would
  not fire (1280 of 4096: room).
- **Not the lost command:**
  - the request and its notice both arrived and were answered;
  - the check prints "read empty" at once, and there is none.
- **What differs from the 23 refusals that rendered on is not measured
  here.**
  - This image's grow line is the pre-344e2493 sampler. It has no
    "(sampled when the batch was first read)"; the line is read after the
    batch's synchronous prints. So its "0 with TA done and 3D pending, 2
    with TA pending" is not the state at the request.
  - In the fixed-sampler runs (kt2-1/2), each refract refusal had kicks
    with the TA done and 3D pending (2 + 1). Their 3D returns pages, so the
    TA can go on.
  - UPDATE_STATS read "partial renders 0" in every run so far.
  - Candidate, named, not shown: **"out of memory with no 3D pending and
    no partial render: the TA waits forever"**.
- **The nondeterminism, named:** "a refused grow in conditionals at frame
  ~193 (fwccb-6), where fwccb-5 ran 8507 conditionals frames with no grow
  request". A trivial scene emptying a 1280-page list after 191 good frames
  suggests pages not back on the list (a leak, or a 3D not yet done). The
  console cannot tell which.
- **What the grow-scene line would have said** (ae18b42d's code, where the
  glmark2 sample marks each scene): "scene 26: 193 kicks since its start
  (the first 100557), T ms". Conditionals with no vertex and fragment steps
  is the 26th entry, and 100749 - 100556 = 193. So the line would have
  named the scene and placed the request at its 193rd kick. The fixed
  sampler in the same line would have said whether a 3D was pending.

### The grow-scene instrument: the scene's kicks and time at each request

**What the firmware exposes, read from the vendored DDK headers:**
- `RGXFWIF_FREELIST` (rgx_fwif_shared.h:944-967): CurrentPages, the
  stack top, AllocatedPageCount, AllocatedMMUPageCount, ReadyPages,
  bGrowPending.
  - These are not a live measure of the TA's use. In 70b79a74 the
    allocated count read 4 (and 0) at all 25 requests.
  - The stack top was always current - 1 - 4 (507, 763, 1019, 1275).
  - So "top" at the request is the firmware's bookkeeping at its last
    update, not how far the TA had got.
- `RGXFWIF_HWRTDATA` (rgx_fwif_shared.h:1242-1302): the PM's MList
  pointers, `eState` (with RGXFWIF_RTDATA_STATE_TAOUTOFMEM), and
  `aui32FreeListHWRSnapshot` (written at a recovery). Nothing counts pages.
- The PM's own stack registers are not in the vendored register header
  (rgx_cr_defs_km.h defines RGX_CR_PM_PARTIAL_RENDER_ENABLE and the
  out-of-memory event bits, no stack or count register).

So the line carries the host's side.

**The instrument** (615b0839, `CONFIG_SAFEUI_GPU_PVR_GROW_SCENE`, drivers
feature `pvr-grow-scene`; a sibling of the kick-times instrument, sharing
its clock):
- The grow line ends "; scene S: K kicks since its start (the first F),
  T ms": the kicks submitted, and the ARM generic timer, since the
  application's last `pvr_render_scene_begin()`. The glmark2 sample calls
  it as each scene's frames start (`log_scene_info`, after the scene's
  setup).
- Upstream's main loop calls `log_scene_info` for every named entry of the
  33, including the unsupported and failed ones. So S is the entry's
  position in glmark2's default list:
  - build 1-2, texture 3-5, shading 6-9 (blinn-phong-inf 7);
  - bump 10-12 (high-poly 10);
  - effect2d 13-14, pulsar 15, desktop 16-17, buffer 18-20, ideas 21,
    jellyfish 22, terrain 23, shadow 24, refract 25.
- Each glmark2 frame is one kick here, so K is about the scene's frame
  count at the request.

**Image ae18b42d** (`build/fl-scene-glmark2-on-am62p`; 615b0839: knob on,
ready pages on, grow scene on, landed frame sets). Predictions per
request, written before it runs:
- **Grow 1 at bump high-poly:** "scene 10: K kicks since its start ...", K
  small (a few to a few hundred kicks at 3.2 ms a frame), T under 1000 ms.
  In the before-the-answer run, both bump grows came within the second the
  scene began. The "before" line as in 70b79a74 (cur=512 ready=0 pending=1).
- **Grow 2:** scene 10, K and T larger than grow 1's, T under 2000 ms.
- **Grow 3:** scene 25 (refract), K a few.
- **The refusals:** scene 25, K rising by about 1 per request and T by
  about 435 ms (one request per refract frame).
- **If a grow comes at blinn-phong-inf instead** (4c25118c's case):
  "scene 7", with its K and T. That is the comparison the item needs. A
  different scene for the first grow is a "grow scene" row, not a defect
  of the instrument.
- **The rest as 70b79a74:** about 473, 0 resets. With the before line, the
  answer is posted about 10 ms late (all acks 1 ms).
- **Added after fwccb-6** (a refused grow in conditionals at its frame ~193
  hung kick 100748's TA, with no record: an image without fix (2)): if a
  refused grow comes in conditionals (scene 26) again, the line reads A = 0
  (no kick with TA done and 3D pending to return pages). The UPDATE_STATS
  line reads "partial renders 0", as in every run so far. And the run ends
  in a GPU_STOP record with tag 4 at 10 s, not at the watchdog. A >= 1 with
  a hang would refute the candidate "out of memory with no 3D pending and
  no partial render: the TA waits forever".

### The grow-scene runs: ae18b42d x 2 (AM62P, 2026-10-04)

Image ae18b42d (615b0839; knob on, ready pages on, grow scene on), U-Boot
boot, run dir `build/flscene-am62p-20261004-071714/fl-scene-glmark2-on-{1,2}`;
uninstall ok, env diff 0. Consoles, the stable copies:
- `records/am62p/flscene-20261004-071714/fl-scene-glmark2-on-1/20261004-0717-glmark2-ae18b42d/console.txt`
  (sha256 307ad90f69901e6b..., 1073 lines);
- `.../fl-scene-glmark2-on-2/20261004-0723-glmark2-ae18b42d/console.txt`
  (85e89f649344c343..., 1079 lines).

Both runs:
- 470 (record 594b1201 000001d6 021e0102 03200258), GPU resets 0.
- "25 grow requests, 3 served, 22 unserved", "partial renders 0, out of
  memory 25", unaccounted 0.
- 0 CONTEXT_RESET, 0 "did not complete", 0 "render: stopped".
- 1 re-post (terrain's first kick).
- All 25 acks 1 ms. 25 scene lines.

| request | run 1: in flight (A TA done + 3D pending, B TA pending); scene | run 2 | before the answer (both) |
|---|---|---|---|
| 1 (served) | 2 (0 + 2); scene 10: 1030 kicks (the first 58062), 3221 ms | 2 (0 + 2); 1044 kicks, 3265 ms | cur=512 ready=0 pending=1 |
| 2 (served) | 2 (0 + 2); scene 10: 1126 kicks, 3583 ms | 2 (1 + 1); 1128 kicks, 3592 ms | cur=768 ready=0 pending=1 |
| 3 (served) | 1 (1 + 0); scene 25: 2 kicks, 108 ms | the same, 108 ms | cur=1024 ready=0 pending=1 |
| 4 (refused) | 1 (0 + 1); scene 25: 5 kicks, 658 ms | 1 (0 + 1); 5 kicks, 657 ms | cur=1280 ready=0 pending=1 |
| 5-25 (refused) | 3 (2 + 1) each; scene 25: 8, 11, ... 68 kicks, 1098 ... 9885 ms | the same; ... 9883 ms | cur=1280 ready=0 pending=1 (22 lines with 4) |

**Against the predictions (41e02b8f, 62f35583):**
- **Held:**
  - grows 1 and 2 at scene 10 (bump high-poly), grow 3 at scene 25
    (refract) a few kicks in;
  - the before line at every request;
  - about 470, 0 resets, acks 1 ms.
- **Missed: "grow 1 under 1000 ms, grow 2 under 2000 ms".** Grow 1 came at
  kick 1030-1044, 3.2 s into the scene, and grow 2 96 and 84 kicks later.
  Bump high-poly then ran its other ~2000 frames (3108 in all) with no
  further request. I had read "the same second as the scene's start" from
  1 s console stamps that cannot say that.
- **Missed: "refusals +1 kick per request".** It is +3 kicks and about 440
  ms per request. Refract submits about 3 kicks per frame (23 frames, 68
  kicks to request 25), so it is still one request per frame.
- **The comparison cases did not occur:** no grow at scene 7
  (blinn-phong-inf) and no refusal at scene 26 (conditionals) in either
  run.

**What the lines say about "the grow scene":**
- **The first grow is not set by bump high-poly's own geometry filling
  512 pages at its first frames.** It comes after about 1000 frames of the
  same geometry, with 2 kicks' TAs in flight together (B = 2, A = 0 or 1)
  in 3 of the 4 bump requests.
- **Reading (a hypothesis, from 4 requests):** the list runs out when two
  high-poly TAs are in flight at once, which happens at a frame the
  timing picks. That would make the grow's frame, and in a run like
  4c25118c even its scene, depend on when two TAs overlap.
- The kick numbers (1030 / 1044) are the instrument that can follow this
  from run to run.

**What request 4 says about fwccb-6's candidate:**
- In both runs request 4 read A = 0, B = 1: one kick in flight, its TA out
  of memory, no 3D pending. It was refused, and the kick completed (no
  timeout).
- So "out of memory with no 3D pending and no partial render: the TA
  waits forever" does not hold in that general form.
- The sample is up to one poll after the request. A 3D that finished in
  between (kick 100445's, the one before) reads as 0 here but could still
  have returned pages to the waiting TA. So the candidate is weakened, not
  refuted: what the TA could wait for at request 4 was the 3D of the kick
  just before it.
- fwccb-6's request 27 (pre-344e2493 sampler) cannot be read the same way.

**The next run: the two-TA reading, tested directly** (written before it
runs). Image d320c8a5 (`build/fl-scene1-glmark2-on-am62p`): the tree of
ae18b42d plus `CONFIG_GLMARK2_PVR_ONE_KICK_IN_FLIGHT=y`, the sample's
existing measurement option. Each swap waits for its own frame's kick, so
a one-kick-per-frame scene never has two TAs in flight. The score is a
measurement, not a score.
- "glmark2: PVR_BACKEND_OPT_ONE_IN_FLIGHT=1 ..." at the start.
- **Scene 10 (bump high-poly): no grow request.** A grow there with B <= 1
  refutes "the list runs out when two high-poly TAs overlap".
- **Scene 25 (refract): requests still come.** Grow 3 came there with A = 1
  and B = 0 (its own kicks: refract submits about 3 per frame, and one of
  them overlaps). There will be at most as many as the 25 of ae18b42d. The
  first one is served from the list's own reserve (the bump grows no
  longer use it).
- Any grow in another scene names a scene whose kicks need more than 512
  pages alone. It is recorded by scene and kick.
- 0 resets; a hang ends in GPU_STOP tag 4 at 10 s.

### The one-kick-in-flight run: d320c8a5 (AM62P, 2026-10-04)

Image d320c8a5 (ae18b42d's tree + `CONFIG_GLMARK2_PVR_ONE_KICK_IN_FLIGHT=y`),
U-Boot boot. Run dir
`build/flscene2-am62p-20261004-073640/fl-scene1-glmark2-on-1`, console
`20261004-0736-glmark2-d320c8a5/console.txt` (sha256 f8ff2495260a2337...,
684 lines).
- **Record 594b1404 00011822 00000000 00000000: GPU_STOP, cause 4.** The
  board came back after 304 s with no watchdog reset.
- :128 "PVR_BACKEND_OPT_ONE_IN_FLIGHT=1".
- "4 grow requests, 2 served, 2 unserved, 1036 KiB taken on demand; GPU
  recoveries 0", "partial renders 0, out of memory 5".

| request | scene, kick, time | in flight (A TA done + 3D pending, B TA pending) | before the answer | answer | ack after |
|---|---|---|---|---|---|
| 1 (served, own reserve) | scene 23 (terrain), the batch of request 2 | (said once per batch, below) | cur=512 ready=0 pending=1; host 448 + 64 | 256 / 768 / 96 | **cur=672 top=667 ready=96** |
| 2 (served, refilled reserve; refill then refused) | scene 23: 5 kicks (the first 71351), 249 ms | 2 (0 + 2) | cur=768 ready=0 pending=1; host 672 + 96 | 256 / 1024 / 132 | cur=768 top=763 ready=256 |
| 3 (refused: the pool, 184 KiB left) | scene 25: 2 kicks (the first 71712) | 1 (1 + 0) | cur=1024 ready=0 pending=1 | 0 / 1024 / 0 | cur=1024 ready=0 |
| 4 (refused) | scene 25: 3 kicks | **1 (0 + 1)** | cur=1024 ready=0 pending=1 | 0 / 1024 / 0 | cur=1024 ready=0 |

Then at 07:40:55, 10 s after request 4: "render: stopped: a kick that did
not complete in the stop's time (list 0, 0 KiB)", "fence 71714 ... cannot
complete", "board: FAIL the GPU driver stopped the run (cause 4, fence
71714 ...)", and the record.

**Against 552b474e:**
- **Held:** no grow request at scene 10. Bump high-poly ran 2459 frames
  at 4.07 ms with one TA at a time. The two-TA reading survives its test;
  bump's grow needs two high-poly TAs in flight.
- **Missed:** "refract's first request is served from the own reserve".
  - The own reserve served terrain (scene 23), which now grew first (5
    kicks in, B = 2: terrain submits more than one kick per frame).
  - Its refill was refused: the render pool had 340 KiB left after one
    refill, where the other images had room for two.
  - So refract met an empty pool at 1024 listed.
- **Held:** "a hang ends in GPU_STOP tag 4 at 10 s". Fix (2) did what it
  was built for: the record, no watchdog.

**The hang, now read with the fixed sampler:**
- Request 3 (A = 1) was refused, and its kick completed: the pending 3D
  returned pages.
- Request 4 came one kick later with **A = 0, B = 1**: the only kick in
  flight was the out-of-memory TA itself. It was refused, and that TA never
  completed. The firmware did no partial render (0, as in every run).
- This is fwccb-6's candidate, measured: **out of memory, refused, no 3D
  pending, no partial render: the TA waits forever**.
- ae18b42d's request 4 read the same A = 0, B = 1 and completed. With up
  to 3 kicks per refract frame in flight there, the kick before's 3D could
  have finished inside the sample's poll window and returned pages to the
  waiting TA. With one kick in flight there is no such 3D.
- So the condition is "nothing pending that can return pages", and A at the
  sample is its reading, exact only when nothing can finish in between.

**What this makes of the refusal policy.** A refused grow is survived only
while a 3D that can return pages is pending (the firmware renders on when
that 3D frees its pages). When none is pending, the out-of-memory TA has
nothing to wait for. The three ways out, as the lead named them:
1. **Do not refuse while A = 0**, from a reserve kept for exactly that.
   That moves the wall but does not remove it. At the list's maximum, or
   with the reserve spent, the same wait remains: the probe's case
   (0efa0642) is this with no room at all. It is a mitigation.
2. **The firmware's partial render.** This is how the protocol makes an
   out-of-memory TA progress without the host. The firmware renders what
   the TA has binned so far, frees those pages and lets the TA go on (the
   UPDATE_STATS element "partial renders", 0 in every run here).
   - The DDK and Mesa set up a kick so that the firmware can do it. That
     needs depth/stencil store and load for the partial render, and Mesa's
     render job carries the partial-render state.
   - Ours never has, which is why every refusal depends on a pending 3D.
   - It is the root of the hang, and the fix belongs there. What enables
     it in our kicks is not yet read: the next host item.
3. **Kill the TA on a refusal** (a context reset). The frame is lost; a
   GPU reset is the floor, not the fix (CLAUDE.md). It is not an answer.

**The READY item, reopened by ack 1:**
- Request 1's answer (256 / 768 / 96) was acknowledged as **cur=672
  ready=96**: our READY, upstream's split. Before it was "cur=512 ready=0
  pending=1", as everywhere.
- The firmware then took the 96 at once. Two out-of-memory notices
  followed, and request 2 came in the same FWCCB batch, its before line
  "cur=768 ready=0 pending=1" (672 + 96). That is the back-to-back shape
  of fwccb-2.
- Ack 2 in the same run (256 / 1024 / 132) read cur=768 ready=256 =
  DELTA, as all 15 served acks of fwccb-1, c18f9bfd, 70b79a74 and
  ae18b42d x 2 did (3 + 3 + 3 + 6; read from those consoles).
- So the firmware applies READY in some state and DELTA in another. Ack 1
  is the only one of the 17 served acks read so far with our split. Its request was the
  first grow of a scene that submits several kicks a frame, with two TAs
  pending.
- The rule that tells the two states apart is not read. The sentence "in a
  grow answer the firmware takes NEW and sets ready = DELTA" is withdrawn
  as general.

## FWCCB slot read empty (a lost command): the instrument

**The defect, as measured.** In two AM62P runs, a71c9b8b and kt-3 of
157b76e3, the host read a FWCCB slot whose type was 0x00000000, said
"FWCCB: cmd type=0x00000000 (unhandled, skipping)", and advanced past it.
That is 2 of the 4 runs with refused grows, and no other console on this
host. After it the firmware asked for no grow, and each later kick needed
the 20 ms re-post. Both reads were on the ring's first lap (slot 12 and
slot 28 of `FWCCB_NUM_CMDS` = 32), so the slots held their initial zeros:
the write offset said they were written, but their data was not in memory.

**The consumers compared.**
- The DDK's own consumer is not in our tree: third_party vendors headers
  and configuration only.
- Linux drm/imagination's (pvr_ccb.c `pvr_fwccb_process`, Linux 7.3-rc5,
  "GPL-2.0-only OR MIT") reads `read_offset` and `write_offset` with
  READ_ONCE, copies the whole command, writes only `read_offset`, and then
  processes the copy. It needs no barrier or cache operation because its
  firmware objects are mapped uncached. A command without the magic is
  dropped with a warning: it would lose this slot too.
- Ours (`ccb.rs`, `fwccb_service_with`) reads through the CPU cache. It
  invalidates the control block before each read of the offsets, and the
  slot after the write offset is read. Zephyr's range operations end with a
  DSB (include/zephyr/arch/arm64/cache.h:77-148, `arm64_dcache_range`), so
  the slot's invalidate is complete before its type is read.
- What ours does that upstream cannot: `read_offset` (ours) and
  `write_offset` (the firmware's) share one 64-byte line, and `flush(ctl)`
  after writing `read_offset` writes the whole line back with our copy of
  `write_offset`.

**The 9 ms acknowledgement before each empty read** is not specific to it.
In every run, served grows were acknowledged in 1 ms (3 of 3 each) and
refused grows in 5-11 ms: kt-1 22 of 22 between 1 and 11 ms, and the same
in kt-2, kt-3, kt-4 and a71c9b8b. After a zero-page answer the firmware is
busy for several ms before it reads our KCCB answer. That fits rendering
on without the pages (bump high-poly at 1.96 times its frame time in the
no-reserve run). Each empty read followed such an ack, but so did about 20
acks per run that led to none.

**The instrument** (`CONFIG_SAFEUI_GPU_PVR_FWCCB_CHECK`, off by default;
`YdPvrKm::fwccb_check`). It tells the candidates apart:
1. **The firmware publishes the write offset before the command is
   visible.** A slot without the command magic is re-read every 20 us for
   up to 2 ms, and consumed once it fills: "FWCCB: slot S read empty; type
   0x... after N us (read offset R, write offset W)".
2. **Our invalidate does not see the firmware's write:** "FWCCB: slot S
   read empty; still empty after 2000 us, ...", and the slot is skipped as
   before.
3. **Our writeback moved the write offset back:** every read of the offsets
   is compared with the last one: "FWCCB: write offset went back from X to
   Y (the host's writeback of the shared control line)".

The run end adds "render fwccb: slots read empty E (filled within 2000 us
F, still empty S); write offset went back B". Host tests (ccb.rs
`fwccb_check_tests`), on a rig whose "firmware" writes the slot's type
after a number of waits:
- an empty slot that fills after 60 us is re-read and consumed;
- without the check, today's consumer skips it and the command is lost;
- a slot that stays empty is said after the bound;
- a write offset that went back is said, and one moving forward over the
  ring's end is not.
The tests for the first and third fail with the re-read removed.

**Predictions for the image** (glmark2, knob on, the check on, landed frame
sets), written before it runs. It reaches refract's refused grows, where 2
of 4 runs had an empty read:
- If an empty slot occurs, I expect (1): "slot S read empty; type
  0x2abc0067 after N us" with N under 100 us. The FREELIST_GROW is then
  answered, the re-asks go one per refract frame to the scene's end, and
  there is no 20 ms re-post at refract.
- "still empty after 2000 us" would point to (2); a "went back" line to
  (3).
- With no empty slot in the run (2 of 4 runs had none): "slots read empty
  0", and the run says nothing about the defect. The rest as kt-1 (293, 8
  blocks, 8368 KiB, unaccounted 0).

### The two runs with the check on (AM62P, 2026-10-04)

Image 1f9ff20e (0a9e68ea: main 53328237 + the check; knob on, landed
frame sets), U-Boot boot. Run dir `build/fwccb-am62p-20261004-054022`,
consoles `fwccb-glmark2-on-{1,2}/*/console.txt`. Both runs are the same
image.

| run | record | grow requests (served / unserved) | the check's run-end line | empty reads | still empty | went back |
|---|---|---|---|---|---|---|
| fwccb-1 | 594b1201 000001d9 021e0102 03200258 (473) | 26 (3 / 23) | "slots read empty 0 (filled within 2000 us 0, still empty 0); write offset went back 0" | 0 | 0 | 0 |
| fwccb-2 | none (the RTI reset the SoC) | 14 served, 0 unserved | absent: the run never reached its end | 0 lines | 0 lines | 0 lines |

- **fwccb-1:** 2 grow requests at bump high-poly, 24 at refract, where the
  scene ran 24 frames: one request per refract frame. The served one was
  from the refilled reserve, and the 23 refused were each "no grow: the
  render pool refused 1036 KiB". 26 out-of-memory notices, 0
  CONTEXT_RESET, no "unhandled, skipping".
- **fwccb-2:** the runaway grow at bump height and the at-maximum wedge
  (pvr/fl-grow-on-demand's doc has the whole run). The run-end line is
  absent because the run never closed. It is not a zero. The console holds
  0 "read empty" lines, 0 "went back" lines and 0 "unhandled, skipping".
  Those lines are printed when they happen, so up to the RTI reset at
  05:50:20 none happened.

**The score of 473 is not 293.** fwccb-1 is the first knob-on glmark2 on
top of land162's default: 57547c34 ("pvr backend: the swap drops the
window's depth/stencil by default").
- 57547c34 is an ancestor of 0a9e68ea, and not of 344e2493 (kt2's tree,
  293).
- The image's `.config` differs from kt2's (b06e00b0) in one line only:
  `CONFIG_SAFEUI_GPU_PVR_FWCCB_CHECK=y` where kt2 has
  `CONFIG_SAFEUI_GPU_PVR_KICK_TIMES=y`.

The score belongs to the base, not to the check.

**Verdict.**
- The lost-command defect is unseen with the check on in 2 of 2 runs. Only
  fwccb-1 is a case, though: every empty read so far came after a refused
  grow, and fwccb-2 never reached one (its grows were all served). So it
  is 1 case with the check, not 2.
- The earlier observation stands: 2 of the 4 runs with refused grows read
  an empty slot without the check (a71c9b8b, kt-3).
- The check has not met the defect, so it says nothing yet about which
  candidate (1, 2 or 3) is the cause.
- It stays opt-in (`CONFIG_SAFEUI_GPU_PVR_FWCCB_CHECK`, off by default): an
  instrument for the runs that reach refused grows. "FWCCB slot read empty
  (a lost command)" stays open by name.

**After this run (c18f9bfd, 2026-10-04, pvr/fl-grow-on-demand's doc):** the
lost command recurred without the check (after the third refused grow at
refract). That is 3 of 5 refused-grow runs without the check, against the
1 clean case with it. On a poll with no empty slot the check adds only a
compare and two stores to `YdPvrKm`: no invalidate, flush, barrier, wait
or extra read of the control line or slot. At 3 empty reads in 6 cases,
one clean case is a coin toss (1/2), so the counts do not show the check
changes anything.

**fwccb-3 and fwccb-4** (the same image 1f9ff20e, U-Boot boot, run dir
`build/fwccb2-am62p-20261004-062715/fwccb-glmark2-on-{3,4}`). Each is read
from its own console:

| run | record | grow requests (served / unserved) | refract frames | the check's run-end line | type-0 skips | re-posts |
|---|---|---|---|---|---|---|
| fwccb-3 | 594b1201 000001d8 021e0102 03200258 (472) | 26 (3 / 23) | 24 | "slots read empty 0 (filled within 2000 us 0, still empty 0); write offset went back 0" | 0 | 0 |
| fwccb-4 | 594b1201 000001d8 021e0102 03200258 (472) | 26 (3 / 23) | 24 | the same, all 0 | 0 | 1, :338 at terrain's first 256x256 kick |

- Both runs had 0 CONTEXT_RESET and 0 GPU recoveries. In both, refract
  re-asked once per frame to the scene's end, and every request was
  answered.
- fwccb-4's one re-post is the first-kick stall at terrain's start that
  every run has, not the defect's re-posts at refract.

**The tally, refused-grow cases:** the empty read in 3 of 5 without the
check (a71c9b8b, kt-3, c18f9bfd), and 0 of 3 with it (fwccb-1, -3, -4).
The chance of 0 of 3 if the check changes nothing:
- the 8 cases fixed, with 3 empty reads among them (hypergeometric):
  C(5,3)/C(8,3) = 10/56 = 0.18;
- each case reading empty independently at the without-check rate 3/5:
  (2/5)^3 = 0.064.

So "0.06 to 0.18": suggestive, not yet a separation. The check adds no
cache operation on a poll with no empty slot (the paragraph above). If the
difference is real, the mechanism is not the check's cache behaviour; the
candidates left are timing (a few instructions per poll) and code layout.
The item stays open by name, and the check stays opt-in.

## What enables the firmware's partial render, and what our kick lacks (reading, no code)

Host-only. Mesa 26.2.1 (MIT) is quoted. Linux drm/imagination (7.3-rc5,
GPL-2.0-only OR MIT) and the DDK headers (ti-img-rogue-ddk-25.2) are cited
by file:line only.

**The protocol: a partial-render 3D command waits in the fragment CCB of
every kick.**
- **The DDK command types** (rgx_fwif_km.h):
  - :1325-1327 `RGXFWIF_CCB_CMD_TYPE_3D_PR`, "3D DM command for Partial
    render";
  - :1355-1357 `RGXFWIF_CCB_CMD_TYPE_FENCE_PR`, "Fence dependencies of a
    PR command".
- **Mesa submits three jobs per render, every time** (vulkan/winsys/powervr/
  pvr_drm_job_render.c:490-517):
  - the geometry job;
  - a fragment job flagged `DRM_PVR_SUBMIT_JOB_FRAG_CMD_PARTIAL_RENDER`,
    whose stream is `submit_info->fragment_pr`;
  - the fragment job.
  
  pvr_arch_job_render.c:1437-1452: "For now we'll always setup the pr."
- **The kernel turns the flag into `ROGUE_FWIF_CCB_CMD_TYPE_FRAG_PR`**
  (pvr_job.c:187-189) and pairs the geometry job with it (pvr_job.c:602-675).
  In the fragment CCB the PR command is preceded by `FENCE_PR` on the
  paired geometry job's done fence (pvr_queue.c:698-716) and followed by an
  UPDATE (:729-737).
- **The geometry command carries the key to that fence**
  (pvr_queue.c:718-727): `partial_render_geom_frag_fence` = the geometry
  queue's timeline UFO, value = this job's seqno - 1. The struct field's
  comment (pvr_rogue_fwif_client.h:152-156): "Holds the geometry/fragment
  fence value to allow the fragment partial render command to go through".
- **Reading:** at an out-of-memory event the firmware can run the queued PR
  command, because its fence is satisfied by the running geometry's
  reference value. That renders what the TA has binned so far, frees those
  pages and lets the TA go on, all without the host. When the TA finishes
  without one, the normal fragment command follows.

**What the PR command is** (pvr_arch_job_render.c:1354-1406, "the state of
a fragment PR is almost the same as of that for a normal fragment job apart
the PBE words and the EOT program, both of which are necessary for the
render to use the SPM scratch buffer instead of the final render targets"):
- its PBE words write to an **SPM scratch buffer** (pvr_arch_framebuffer.c:
  102-190: one scratch buffer per framebuffer, with SPM EOT and background
  object state per render);
- its end-of-tile program stores the tile to that buffer
  (pvr_arch_spm.c:455-460, `pvr_arch_spm_init_eot_state`: "stores
  off-chip tile data to the SPM scratch buffer on the EOT event");
- the render that resumes loads it back through a **PR background object**:
  `pds_pr_bgnd[3]` in the fragment registers (pvr_rogue_fwif_client.h:232,
  next to `pds_bgnd[3]` :227), filled from `pr_pds_bgnd_reg_values`
  (pvr_arch_job_render.c:1077-1083; the load program
  pvr_arch_spm.c:760, `pvr_arch_spm_init_bgobj_state`);
- depth and stencil go through the PR's ZLS. The DDK's CMDTA3D_SHARED has
  `asPRBuffer[RGXFWIF_PRBUFFER_MAXSUPPORTED]`, "Supported PR Buffers like
  Z/S/MSAA Scratch" (rgx_fwif_shared.h:166-178), with on-demand Z/S/MSAA
  buffers (:136-146).

**What our kick builds** (driver/rust/src/gpu/pvr/render.rs, `submit`):
- **Geometry CCB:** GEOM + UPDATE. `dk[13] = gufo_fa; // partial-render
  fence address` (:1261). The value word next to it is never written (0).
- **Fragment CCB:** FENCE (geometry UFO) + 3D + UPDATE. **There is no
  FENCE_PR and no 3D_PR.** The stall dumps show exactly that: frag[0] FENCE,
  frag[1] type 0x2abc80cb (3D), frag[2] UPDATE.
- **The 3D command:** sets `pds_bgnd` (dk[70..75]) but not `pds_pr_bgnd`.
  There is no SPM scratch buffer, no SPM end-of-tile program and no PR
  background object, and CMDTA3D_SHARED's PR buffers are 0.

**The gap, named:** with no PR command queued, the firmware has nothing to
run at an out-of-memory event. Its only way on is a grow from the host, or
a 3D already pending that frees pages. That is exactly the measured
behaviour:
- "partial renders 0" in every run;
- refusals survived only with A >= 1;
- the TA waited forever at A = 0 (d320c8a5, fwccb-6) and at the list's
  maximum (the probe).

**What a fix needs** (a list for a design, not a design):
1. a PR 3D command per kick in the fragment CCB, after a FENCE_PR on this
   kick's geometry, with the geometry command's PR fence value set so the
   firmware can let it through;
   - our geometry UFO is a ring of slots (`geom_ufo_slot`), not one
     timeline. Upstream's "seqno - 1" assumes a timeline whose current
     value is the previous geometry's. The fence for a slot needs its own
     derivation, measured on the board;
2. an SPM scratch buffer per render target size and the PBE words that
   write to it;
3. an end-of-tile program that stores the tile, and a PR background object
   program (`pds_pr_bgnd`) that loads it at the resumed render. These are
   USC/PDS programs. Mesa generates them (`pvr_uscgen_spm_load`, the EOT
   setup), and we would transcribe them with attribution (Mesa is MIT);
4. depth/stencil store and load for the PR: our ZLS words, or the PR
   buffers.

Items 3 and 4 are the bulk. Item 1 alone would make the firmware try a
partial render whose output goes nowhere valid, so it ships only with 2-4.
Any of it ships opt-in, behind its own knob, with host tests before a
board run.

### ae18b42d run 5: the lost command, then the A = 0 hang at conditionals (AM62P, 2026-10-04)

Run dir `build/flscene2-am62p-20261004-073640/fl-scene-glmark2-on-5`,
console `20261004-0755-glmark2-ae18b42d/console.txt` (sha256
50041ced04f35807..., 957 lines). Record 594b1404 0001a489 00000000
00000000: GPU_STOP cause 4, no watchdog. Runs 3 and 4 of the batch were
clean (472, 470).

**The sequence:**
- :181-192 grows 1 and 2 at scene 10 (B = 2), acks 512/256 and 768/256.
- :773-779 grow 3 at scene 25 (refract), served (ack 1024/256), refill
  refused.
- :791-808 requests 4, 5, 6 refused (A/B 0/1, 2/1, 2/1), each with its
  before line, its 1 ms ack and its notice (seen 4, 5, 6).
- **:809 "FWCCB: cmd type=0x00000000 (unhandled, skipping)"**: the lost
  command, on the host's side, right after request 6's batch. This image
  has no FWCCB check.
- From then on: no grow request. Out-of-memory notices seen 7 to 27
  through refract, 7 "re-posted" lines (:812-824), refract completed
  (25 frames at 412 ms).
- :840 conditionals starts; notice 28; :841 "render: stopped: a kick
  that did not complete in the stop's time", fence 107657. "6 grow
  requests, 3 served, 3 unserved", partial renders 0, HWR 0.

**Reading:**
- The missing requests after request 6 are **not** a second firmware
  state. A FWCCB slot was read empty and skipped (:809), as in a71c9b8b,
  kt-3 and c18f9bfd.
- The firmware's request 7 was the lost command: it was never answered,
  its grow stayed pending, and it asked for nothing more. The lone notices
  are what it still sends.
- Refract went on while 3Ds were pending to return pages (A >= 1 at
  requests 5 and 6).
- Conditionals' first kick then met the empty list with nothing pending:
  the A = 0 wait of d320c8a5, with no PR command to run (the
  partial-render gap). Without the loss it would have been a refused
  request, with the same outcome.
- **One fact differs from the earlier empty reads:** this one followed a
  **1 ms** ack. All acks in this image are 1 ms, because the before line
  delays the answer's post. The earlier three followed 8-9 ms acks. So
  "an empty read follows a slow ack" no longer holds.
- The with/without-check tally needs this run added. I will re-derive it
  from the consoles, not from the earlier sentence.

## Design: the firmware's partial render (PR), for the user's go/no-go

No code. The implementation (about 12 days, sized below) is the user's
decision. Mesa 26.2.1 (MIT) is transcribed with attribution where code is
taken. Linux drm/imagination and the DDK are cited only. The reading this
builds on is the section above.

**The knob:** `CONFIG_SAFEUI_GPU_PVR_PARTIAL_RENDER`, off by default. It is
a new runtime switch in `YdPvrRenderCfg` (`partial_render`, set by
pvr_render.c like `fl_grow_on_demand`) and a backend option
`PVR_BACKEND_OPT_PARTIAL_RENDER` for the frame-side parts. With it off,
every command and word is today's (the km records and the backend's golden
traces unchanged). It is independent of the grow knob.

### The four parts, where each lives

**1. The PR command and its fence** (driver/rust/src/gpu/pvr/render.rs,
`submit`):
- **Fragment CCB:** before today's FENCE + 3D + UPDATE, the order
  upstream uses (pvr_queue.c:698-737):
  1. FENCE_PR (`CCB_CMD_TYPE_FENCE_PR`, already pinned in fwif.rs:315) on
     this kick's geometry fence;
  2. 3D_PR (`RGXFWIF_CCB_CMD_TYPE_3D_PR`, rgx_fwif_km.h:1325, a new pin)
     with the PR 3D command (part 2);
  3. its UPDATE.
  
  The ring reservation grows by that (`ring_reserve`).
- **Geometry command:** the PR fence's {address, value} in the words after
  the flags (`dk[13]`, today the address only, and `dk[14]`, today 0). Their
  offsets are taken from the client struct's layout (pvr_rogue_fwif_client.h
  :150-156) and pinned through driver/tools/offsetof against the DDK's own
  CMDTA, never by count.
- **The fence value for our ring of geometry UFO slots.** Upstream's
  reference is "the current queue fence seqno minus one"
  (pvr_queue.c:721-726): one timeline whose value, while this geometry
  runs, is the previous geometry's. Our geometry UFO is 16 slots
  (`geom_ufo_slot`, render.rs:53, :83).
  - **Option A (proposed): add one geometry-timeline UFO word.** Every
    geometry command gets a second UPDATE to it (value = seq), and the PR
    fence and FENCE_PR use it exactly as upstream: reference seq - 1,
    FENCE_PR value seq. Nothing is derived; it is upstream's arithmetic.
  - **Option B:** use the slot, reference = the slot's previous occupant
    (seq - 16, which the ring throttle guarantees is done). That is a
    derivation of a firmware test we cannot read, kept only if A is
    refused.
  - **Measured by** the first board run: partial renders >= 1 in
    UPDATE_STATS at the forced case (below). Zero there, with the PR
    command in the CCB (the stall dump shows it), means the fence or its
    reference is wrong, and the dump says which value the firmware held.

**2. The PR 3D command: the SPM scratch and the PR PBE words**
(src/pvr/backend.rs, where today's frame-side words are built: the PBE
words at the frame's colour targets, `isp_zlsctl` :5957, the EOT data
:6001-6008):
- The PR command is today's 3D command with two differences, as in Mesa
  (pvr_arch_job_render.c:1366-1406, "apart the PBE words and the EOT
  program"):
  - the PBE words point at the SPM scratch;
  - the EOT data address points at the PR EOT program's data.
- The backend hands both 3D commands to the driver in the kick struct
  (`PvrRenderKick` gains a `frag_pr` block; the ABI checks
  src/pvr/ffi_layout_check.c and abi_layout_check.c pin it).
- **The SPM scratch:** one per render-target size (with the HWRT set of
  that size), allocated through the OSAL at the size's first use, never
  per frame. Its size is Mesa's formula
  (pvr_arch_spm.c:39-75, `pvr_arch_spm_scratch_buffer_calc_required_size`,
  transcribed): align(width, the PBE line-stride alignment) x height x
  4 B x samples x output dwords x (1 + tile buffers).
  - **Our case:** 1 output dword (RGBA8) and no tile buffers, so 800x600 is
    1.92 MB and 1600x1200 7.7 MB.
  - It comes from the render pool, which glmark2 already empties (the
    1036 KiB refills refused). So the scratch is reserved at init for the
    window's size and counted in the pool-owner lines. A size that cannot
    get its scratch kicks without a PR command, says so, and is counted;
    it is no worse than today.
- **The PR PBE words:** Mesa's `pvr_spm_setup_pbe_state` and
  `pvr_spm_get_pbe_packmode` (pvr_arch_spm.c:259-347), transcribed: raw
  dwords to the scratch, packmode by dword count.

**3. The PR EOT store and the PR background-object load programs:**
- **EOT store:** Mesa's `pvr_arch_spm_init_eot_state` (pvr_arch_spm.c:455-
  600) builds a USC EOT program that emits the output registers through
  the PR PBE state, and its PDS pixel-event program. Ours already builds an
  EOT program per colour-target count (backend.rs, `event_pixel_pds_info_n`,
  :5853). The PR one is the same shape with the scratch's PBE state. So it
  is built at init with the existing PDS pixel-event generator, beside
  today's. No new compiler work.
- **The background-object load:** the resumed render must load the stored
  tiles back. That is `pds_pr_bgnd[3]` (pvr_rogue_fwif_client.h:232, in
  the 3D command next to `pds_bgnd` :227, which we set at dk[70..75]). Its
  programs:
  - a USC shader that reads the scratch, Mesa's `pvr_uscgen_spm_load`
    (generated by pco, pvr_arch_spm.c:77-250);
  - a PDS program that DMAs its texture/buffer state and kicks it.
- Mesa generates the USC shader at run time with pco, and we have no pco on
  the target. **Proposed:** generate it once on the host with the same
  Mesa pco the table generator uses (tools/glsl2usc.sh ->
  pco_shc), for our one configuration (1 output dword, 1 sample, no tile
  buffers). Commit it as a table entry beside the offline shader table
  (src/shaderc/blobs.rs's form), with the generator script and its Mesa
  version. The PDS program comes from our PDS generators (the bg kick and
  DMA programs, backend.rs :1136-:2249).
- **Depth/stencil:** the PR stores depth and stencil, and the resumed
  render loads them. Today `isp_zlsctl_stores` (backend.rs:5957) stores
  only when the swap keeps them (`PVR_BACKEND_OPT_SWAP_DROPS_ZS`, on by
  default). The PR command always stores Z/S to a PR depth scratch, and
  the 3D command loads it when a PR happened.
  - Whether the firmware switches the load on itself after a PR (the
    DDK's `RGXFWIF_PRBUFFER_ZSBUFFER` on-demand Z/S buffer,
    rgx_fwif_shared.h:122-146, and CMDTA3D_SHARED.asPRBuffer :175-176)
    or the host must, is read from Mesa's ZLS setup for the PR
    (pvr_arch_job_render.c:917-1037) before this part is written.

**4. The HWRT side** (render.rs `hwrt_get` :596, the HWRT builder): the
HWRT data's `RTANumPartialRenders` array (rgx_fwif_shared.h:1227) and
whatever the PR state machine needs in the HWRT
(`RGXFWIF_RTDATA_STATE_PARTIALRENDERFINISHED`, `HWRTDATA_PARTIAL_RENDERED`,
rgx_fwif_shared.h:1152, :1174) are allocated and zeroed there, per the
DDK's layout through driver/tools/offsetof. The reconstruction
(`freelist::reset_hwrts`) also clears `HWRTDATA_PARTIAL_RENDERED`, as
upstream's reset of the HWRTs does: it is read before writing.

### Host tests, landing with each part

- **render.rs:** with the knob, the fragment CCB of one kick holds, in this
  order, FENCE_PR {timeline, seq}, 3D_PR (its size), UPDATE, then FENCE,
  3D, UPDATE. The geometry command's PR fence words are {timeline, seq - 1}.
  Without the knob, the bytes are today's (the km render record unchanged).
  Known negative: today's code fails the first assertion.
- **backend.rs:**
  - the PR PBE words for 800x600 RGBA8 equal Mesa's
    `pvr_spm_setup_pbe_state` output for the same inputs (the expected
    words computed by the transcribed function and pinned by a literal);
  - the PR EOT data address differs from the normal one and points into
    the PR program;
  - `pds_pr_bgnd` is set only with the knob.
- **The scratch size per RT size** against Mesa's formula, for 256x256,
  800x600, 1600x1200 and 4x MSAA. Each value is a literal, and one is
  checked by hand in the test's comment.
- **The background-load shader table entry:** decoded by our USC decoder,
  with its generator's Mesa version and input recorded. A rebuild that
  differs fails.

### The first board measurement

d320c8a5's exact case, made deterministic:
1. **gl_test, a new case:** "a draw whose parameter data exceeds the
   list", with the list capped (`CONFIG_SAFEUI_GPU_PVR_FL_PROBE_MAX_PAGES`
   = 512, no grow possible) and one kick in flight. Many large triangles of
   known colours, then pixel read-back of the whole buffer against the
   expected image.
   - **Today (the knob off):** the stop (cause 2 or 4), recorded. That is
     the known negative and must reproduce first.
   - **With the knob:** the kick completes, UPDATE_STATS "partial renders
     >= 1", 0 resets, and the read-back equal pixel for pixel. Depth is
     tested by overlapping triangles across the PR boundary.
2. **Then d320c8a5's glmark2 configuration** (one kick in flight, knob on):
   - the refused grow at A = 0 completes;
   - partial renders >= 1;
   - no stop.
3. **Then the regression set** (gl_test all cases, glmark2 at the landed
   default) with the knob on and off before it ships opt-in. A PR changes
   what 3D runs, so every case is a pixel case.

### Sizing (days of focused work, one agent, the board available when needed)

| part | content | days |
|---|---|---|
| 1 | CCB commands, timeline UFO, fence words, pins, host tests | 1.5 |
| 2 | SPM scratch per RT size, pool reservation, PR PBE words, kick ABI | 2 |
| 3a | PR EOT program (existing generator, PR state) | 1 |
| 3b | background-load shader (host generation with pco, table, decoder test) and its PDS program | 3 |
| 3c | depth/stencil PR store and load (read Mesa's PR ZLS first) | 1.5 |
| 4 | HWRT PR fields, reconstruction clear | 0.5 |
| - | gl_test forced-PR case, board runs, the regression set, fixes found there | 2.5 |
| | **total** | **about 12** |

**The risks the sizing carries:**
- the firmware's PR fence test is not readable, so Option A rests on
  upstream's arithmetic holding for TI's firmware build;
- 3b depends on pco producing a shader our firmware accepts in our
  register setup;
- the render pool must hold one more 1.9 MB colour scratch, plus about
  2.4 MB for the PR depth and stencil (4 + 1 bytes a pixel at 800x600),
  at the window size, where glmark2 already runs it dry.

**Until the decision:** refusals are survived only while a 3D is pending.
The stop ends a hang at 10 s with a record (fix 2), and the knob stays
opt-in.

### The landing tip with every switch on: image 67f54d8a, predictions (written before it runs)

Image 67f54d8a (`build/fl-tip-glmark2-on-am62p`, the tip 5609117d: grow knob,
ready pages, grow scene and the FWCCB check all on, landed frame sets). It
is the first image with the check and the fixed sampler/scene line
together:
- **The record:** a score near 470 (30 ran, 1 failed, 2 unsupported, 0
  resets), or a GPU_STOP cause 4 at 10 s if a refusal meets A = 0. Never
  the watchdog.
- **The grows:** at scene 10 at about kick 1000 with B = 2, and at scene 25
  kick 2. Then refusals at scene 25, +3 kicks each, with "fw before the
  answer: cur=1280 ready=0 pending=1".
- **The check's run-end line:** "slots read empty E (filled within 2000
  us F, still empty S); write offset went back B", printed only if the run
  ends normally.
  - If E >= 1, the first measurement of which candidate it is: (1) "type
    0x... after N us", or (2) "still empty".
  - If E = 0, a fifth clean case with the check (fwccb-1, -3, -4, -5).
- **A hang at conditionals (scene 26)** reads A = 0 at its request and
  stops at 10 s, as in d320c8a5 and ae18b42d run 5.

### The ae18b42d batch, runs 3-10 (AM62P, 2026-10-04)

Consoles: `records/am62p/flscene2-20261004-073640/fl-scene-glmark2-on-N/<run>/console.txt`
(copies of `build/flscene2-am62p-20261004-073640`). Each row is read from
its run's console. "type 0" counts "cmd type=0x00000000 (unhandled,
skipping)".

| run | console sha256, lines | record | grow requests | type 0 | the end |
|---|---|---|---|---|---|
| on-3 | bb0762d004f3c044..., 1079 | 594b1201 000001d8 (472) | 25 (last: scene 25, 68 kicks) | 0 | clean |
| on-4 | fd8bf1ec95e40be7..., 1079 | 594b1201 000001d6 (470) | 25 (scene 25, 68) | 0 | clean |
| on-5 | 50041ced04f35807..., 957 | 594b1404 0001a489 | 6 (scene 25, 11) | 1 (:809) | stop at conditionals 0:0 (scene 26), notices to 28 |
| on-6 | 5432acae8139da71..., 997 | 594b1404 0002177c | 12 (scene 25, 29) | 1 (:845) | stop at loop fragment-loop=false (scene 31), notices to 27 |
| on-7 | 277a9e4c4dc491bb..., 964 | 594b1404 0001d7af | 6 (scene 25, 11) | 1 (:812) | stop at conditionals vertex-steps=5 (scene 28), notices to 28 |
| on-8 | d2897879f8d61119..., 1054 | 594b1404 0001d958 | 26: request 26 at "scene 28: 7048 kicks", A = 0, B = 2, refused | 0 | stop at scene 28, re-post of fence 121176 then 10 s |
| on-9 | a58c8214b5adadb9..., 1079 | 594b1201 000001d7 (471) | 25 (scene 25, 68) | 0 | clean |
| on-10 | 071cde15c4078959..., 682 | 594b1404 0001889b | 1: scene 10, 1043 kicks, B = 2, served | 1 (:189) | the second bump request lost, stop in refract, notices to 3 |

Corrections and notes:
- **on-7's stop** is at "conditionals fragment-steps=0:vertex-steps=5",
  the scene after "conditionals fragment-steps=5:vertex-steps=0" (5025
  frames, completed). By default-benchmarks.h, 0:5 (vertex 5) is entry 28.
- **on-8's request 26** is at scene 28 too, after 7048 kicks of a light
  scene with two TAs in flight and no 3D pending.
- **on-10 lost the first command after the first grow:** the type-0 line
  comes right after request 1. So the lost command is not tied to refused
  grows: it followed a served grow.

**Reading, by the two markers:**
- **Clean, 3 of 8:** on-3, -4, -9. 25 requests (3 served, 22 refused)
  each.
- **The lost command, then a hang, 4 of 8:** on-5, -6, -7, -10.
  - Each has one type-0 line. After it, no further request (the lost one
    stayed pending) and only lone notices.
  - Each stops at the first kick that met the empty list with nothing
    pending.
- **Refusals to the end, then the A = 0 hang, 1 of 8:** on-8 (A = 0,
  B = 2: two TAs pending, no 3D), with d320c8a5 the second such case.
- **Every stop was the 10 s GPU_STOP (cause 4).** Fix (2) held 6 of 6
  (d320c8a5 and on-5, -6, -7, -8, -10). The watchdog fired none.
- **No scene-7 grow and no scene-26 refusal** in 10 runs of ae18b42d
  (on-5's and fwccb-6's hangs at scene 26 followed a lost command and a
  refusal respectively).

**The with/without-check tally, re-derived from consoles** (runs with at
least one grow request; each count read from the console by the same
search. The search finds the known positive, c18f9bfd's line):
- **Without the check, 5 of 13:** c18f9bfd 1, 70b79a74 0, ae18b42d on-1
  to on-10 4 (on-5, -6, -7, -10), d320c8a5 0.
- **With the check, 0 of 6:** fwccb-1 to fwccb-6. No "read empty" line
  and no type-0 line in any. In fwccb-2 and fwccb-6 the run-end line is
  absent, but the per-event lines would have printed.
- **The chance of 0 of 6 if the check changes nothing:** (8/13)^6 = 0.054
  per case independently, or C(14,6)/C(19,6) = 0.111 over the 19 cases
  as fixed. So "0.05 to 0.11": stronger than before, still not a
  separation.
- The earlier consoles (a71c9b8b, kt-1 to kt-4, kt2-1/2) are left out
  here, because they were not re-read with this search. They would add 2
  (a71c9b8b, kt-3) to the without-check side.
- The check adds no cache operation on a clean poll. If the difference is
  real, the candidates are timing and code layout.

**The check A/B, arms that differ in the check alone** (written before it
runs):
- ON is 67f54d8a. OFF is 3689e47f (`build/fl-tipoff-glmark2-on-am62p`).
- Both are built from the same tree: the tip 5609117d plus docs-only
  commits (no code change since).
- Their `.config` files differ in one line,
  `CONFIG_SAFEUI_GPU_PVR_FWCCB_CHECK=y`.
- ae18b42d is not used as the OFF arm, because it is built from 615b0839:
  without main's commits since 09f06230, without the FWCCB branch's code,
  and with a different km layout.

Prediction, from the 5 of 13 seen without the check:
- the OFF arm loses a command in about 1 or 2 of its 4 runs;
- if the check changes nothing, the ON arm does the same;
- an ON run with a lost command prints "FWCCB: slot S read empty; ...",
  which names the candidate.

### "HWR 1 at blinn-phong-inf after the grows" recurred on an older tree (common set, 2026-10-04 08:35)

`build/common-am62p-20261004-083423/tree/20261004-0834-GM-15701a95/console.txt`
(image 15701a95, the common set's own run, its row pvr-fence's).
- **The tree is 04cca45d:** the drivers/module-leftovers line forked at
  8b53557f (land162). It has none of land163 (no d2da87fc reconstruction
  fix, no HWRT reset, no GPU_STOP), land173 or land177 (verified by
  ancestry). The knob is off (reserve x4).
- **The run:**
  - At blinn-phong-inf (scene 7), three grows from the own reserve (768,
    1024, 1280), each answered at once by the next request. The first
    ack read **"cur=768 top=763 ready=0"**, where every clean run's first
    ack reads "cur=512 ready=256". That is 4c25118c's first ack, and the
    shape fwccb-2's runaway began with.
  - Each request has its out-of-memory notice, printed as "cmd
    type=0x2abc006b (unhandled, skipping)": RGXFWIF_FWCCB_CMD_UPDATE_STATS
    (fwif.rs:114), named since c023c8f9.
  - Then CONTEXT_RESET (dm=3, reason 1), HWR 1, and FREELISTS_RECON
    through the **old** reconstruction.
  - Then one more grow (1536, the reserve's last), a refusal ("no grow
    (reserve 0, ...)") and a TA that never completed (geom_ufo frozen at
    46720).
  - No GPU_STOP exists in this tree, so the RTI reset the SoC (no
    record).
- **Reading:**
  - This is the scene-7 grow case, with the HWR the 4c25118c item names.
    It now has a third instance (4c25118c, and fwccb-2's start without the
    HWR).
  - What main has since this tree:
    - land163's reconstruction and HWRT reset (the 536-recovery loop
      gone);
    - land177's 10 s stop (a record instead of the RTI).
  - The HWR itself is not addressed by anything on main. The refusal's
    hang is the partial-render gap.
  - The first-ack signature "ready=0 at the first grow" now marks every
    bad run seen (4c25118c, fwccb-2, this one). It is the observable to
    watch in the A/B runs.

**fl-tip run 1** (67f54d8a, every switch on), run dir
`build/fltip-am62p-20261004-084426/fl-tip-glmark2-on-1`, console
`20261004-0844-glmark2-67f54d8a/console.txt` (sha256 91d07d5bb5754a86...,
1080 lines):
- 471 (594b1201 000001d7), GPU resets 0, "25 grow requests, 3 served, 22
  unserved".
- Grows at scene 10, kicks 1039 and 1126 (B = 2 both), and at scene 25
  kick 2 (A = 1).
- The check's line: "slots read empty 0 (filled within 2000 us 0, still
  empty 0); write offset went back 0". 0 type-0 lines, 0 "read empty;"
  lines.
- **First ack "cur=512 top=507 ready=256":** the clean-run signature, not
  the "ready=0" of 4c25118c, fwccb-2 and the common set's HWR run.
- The predictions held (the E = 0 branch). The check arm is 7 clean of 7.

### The check A/B, ab3-on: the runaway on the check-on arm, ended by the at-maximum stop (AM62P, 2026-10-04 09:19)

Run dir `build/fltip4-am62p-20261004-085200/ab3-on-fl-tip`, console
`20261004-0918-glmark2-67f54d8a/console.txt` (sha256 564e63078ad530b6...,
382 lines).
- Record 594b1402 00012326 00000000 01000000: **GPU_STOP cause 2**.
  "render: stopped: the parameter manager out of memory with its freelist
  at the maximum (list 1, 0 KiB)", back after 193 s.
- "14 grow requests, 14 served, 0 unserved, 13468 KiB taken on demand;
  GPU recoveries 0", "reserve refills refused 0", "partial renders 0, out
  of memory 19".
- The check's line: "slots read empty 0 ... went back 0". 0 type-0 lines.

**The sequence:**
- Grows 1 and 2 at scene 10 (kicks 1025, 1136, B = 2), acks "cur=512
  ready=256" and "cur=768 ready=256": **the clean-run shape**.
- Grows 3 to 14 come **in one FWCCB batch** at scene 12 (bump height),
  "scene 12: 6059 kicks since its start, 9566 ms": the end of the scene,
  not its start. The context line is said once, after the batch: "at
  grow request 14 ... 2 kicks in flight: 0 with TA done and 3D pending, 2
  with TA pending".
- Every one of those 12 is served (the pool had room: no refill refused
  in this run). Every ack reads **current = all listed, ready = 0,
  pending = 1** (1280, 1536, ... 3840). The firmware put the answer's
  pages into current at once and had already asked again.
- Two out-of-memory notices follow each answer. At 4096 the ack reads
  "cur=4096 top=4091 ready=0 pending=0", then two notices (seen 18, 19)
  and the stop.

**Reading:**
- **This is fwccb-2's runaway, on the check-on arm, with no lost
  command.** The runaway and the lost command are separate defects.
- **Fix (2)'s at-maximum rule fired as designed:** two notices after the
  last grow, with the list at its maximum. Cause 2, a record, no watchdog:
  the case it was written for (fwccb-2), now measured on silicon.
- **Correction to the signature I gave** ("ready=0 at the first grow ack
  marks every bad run"): wrong as stated. Here the first two acks are the
  clean shape.
- The signature that holds in every bad run seen is **a served grow
  acknowledged with ready=0** (the firmware took the new pages into
  current at once):
  - 4c25118c's first ack;
  - fwccb-2 from bump height on;
  - the common set's HWR run;
  - this run from grow 3 on.
  
  In clean runs every served ack reads ready=256.
- **Where:** bump height (scene 12), late in the scene, with two TAs in
  flight and no 3D pending. One or two bump-height kicks used 3328 more
  pages (13 MiB) without finishing. That is not the scene's normal need:
  bump height runs 6000+ frames with no grow in every clean run. It is a
  TA whose page use does not end, the open item behind 4c25118c and
  fwccb-2.
- What makes a TA's use run away is not measured. The candidates are a
  geometry fault, or pages the PM never sees freed. Both would need the
  TA's state at the batch, which no instrument reads yet.

### The check A/B, all eight runs (AM62P, 2026-10-04 08:52-09:43)

ON is 67f54d8a and OFF is 3689e47f: the same tree, one `.config` line
apart. They were interleaved (ab1-on, ab1-off, ... ab4-off). Consoles:
`records/am62p/fltip4-20261004-085200/<run>/<stamp>/console.txt`. Each
count below is from its console:
- "requests" = "fw before the answer" lines;
- "served" = "-> +256" lines;
- "type 0" = "cmd type=0x00000000";
- "served, acked ready=0" = a served grow whose ack reads ready=0. That
  search finds ab3-on's 12.

| run | console sha256, lines | record | requests / served | type 0 | first ack | served, acked ready=0 |
|---|---|---|---|---|---|---|
| ab1-on | 9dbdb59b28431c5f..., 1079 | 594b1201 000001d8 (472) | 25 / 3 | 0 | 512 / 256 | 0 |
| ab1-off | 8884d45054aa7cac..., 988 | 594b1201 000001d8 (472) | 6 / 3 | **1** | 512 / 256 | 0 |
| ab2-on | a8acdfa59aff1ba4..., 1080 | 594b1201 000001d8 (472) | 25 / 3 | 0 | 512 / 256 | 0 |
| ab2-off | 42f53d375802404e..., 1079 | 594b1201 000001d8 (472) | 25 / 3 | 0 | 512 / 256 | 0 |
| ab3-on | 564e63078ad530b6..., 382 | 594b1402 00012326 (GPU_STOP cause 2) | 14 / 14 | 0 | 512 / 256 | **12** |
| ab3-off | c3158db7fdf792b4..., 1078 | 594b1201 000001d6 (470) | 25 / 3 | 0 | 512 / 256 | 0 |
| ab4-on | a627f57ba9e8f719..., 1079 | 594b1201 000001d6 (470) | 25 / 3 | 0 | 512 / 256 | 0 |
| ab4-off | 58488a58b4905b1b..., 1077 | 594b1201 000001d7 (471) | 25 / 3 | 0 | 512 / 256 | 0 |

**ab1-off lost a command and completed.** After the loss, the firmware
asked for nothing more (6 requests). No later kick met the empty list
with nothing pending, so the loss alone did not hang it. A hang needs the
loss and then such a kick (on-5, -6, -7, -10 of ae18b42d).

**The pairs alone:** losses ON 0 of 4, OFF 1 of 4. By themselves that is
no evidence (0.5).

**The tally over every console read the same way** (runs with grow
requests):
- **without the check, 6 of 17:** c18f9bfd, ae18b42d on-5, -6, -7,
  -10, ab1-off;
- **with the check, 0 of 11:** fwccb-1 to -6, fl-tip run 1, ab1-on to
  ab4-on. ab3-on counts: it made 14 requests and lost none;
- **the chance of 0 of 11 if the check changes nothing:** (11/17)^11 =
  0.008 per case independently, or C(22,11)/C(28,11) = 0.033 over the 28
  cases as fixed.
- **Reading:** the check's presence now goes with the loss's absence at
  0.01-0.03. It is not proven. The A/B pairs alone do not show it (0.5),
  so the evidence is mostly the unpaired runs, which differ in tree and
  time.
- **What the check changes on a clean poll** (no cache operation, a
  compare and two stores) gives no mechanism for it. So the reading stays
  "a timing or layout candidate". The defect stays open by name.

**The runaway is the other defect, with its own signature:** a served
grow acked with ready=0. It appears in no run here except ab3-on, which
is check-on. It is not prevented by the check.

### The runaway instrument (55fb6660) and its image 70b86b80: predictions (written before it runs)

- **The instrument:** with the grow-scene feature, a FWCCB batch that
  served 2 grows or more ends with one line, "render: runaway batch: N
  grows served in one FWCCB batch; TA_CYCLE ... TA_PHASE ... TE_SIG ...
  VCE ... VDM_CTX_STORE ...; PM_CAT ... PM 0x190 ... 0x288 ... stack 0x270
  ... 0x280 ... 0x2c0 ...; HWRT set H (last kick S): state ... flags ...
  alist_sp ... mlist_sp ...; vdm ...".
  - It reads only, and comes after the batch's synchronous prints.
  - HWRT words: state, flags, and the PM's ALIST/MList stack pointers.
    The two new pins (56, 64) are checked against the DDK headers.
  - The PM registers are read by raw offset, as the stall dump reads them.
    Their meaning is not in the vendored headers, so the reading compares
    a runaway's values with a clean batch's, not with a definition.
- **Image 70b86b80** (`build/fl-run-glmark2-on-am62p`): the fl-tip
  configuration (grow knob, ready pages, grow scene and the FWCCB check
  on) plus the dump. It is meant to run in a batch until a runaway hits
  (about 2 in 35 runs so far).
- **Per run, written before:**
  - **a clean run:** no "runaway batch" line, except at a terrain batch
    of 2 like d320c8a5's. Such a line is the dump's control: its values
    are a TA that went on;
  - **a runaway:** one line with N >= 10, the TA registers showing it
    still busy (TA_PHASE non-zero, TA_CYCLE moving between it and the
    stop's dump), the HWRT state of a TA out of memory, and then the
    at-maximum stop (cause 2) as in ab3-on;
  - the rest as fl-tip run 1: about 471, the check's line, the scene
    lines.

**From ab5 on, the A/B's ON arm is 70b86b80** (= 67f54d8a + the runaway
dump, 55fb6660). OFF stays 3689e47f. ab1-ab4 used 67f54d8a. The two ON
images differ only by the dump, which prints only at a batch with 2 or
more served grows. So the check comparison is unchanged, and every pair
also hunts the runaway. The separate fl-run batch is dropped. The odds
at 2/35 a run: 8 runs miss a runaway 62% of the time, 16 39%, 24 24%, 32
15%.

## The default configuration hangs to the watchdog: the stops made knob-independent (AM62P, 2026-10-04)

**What the common set found** (round 2, image 52be99c3 = main 6004d8b3,
every knob off: the product configuration; its row is pvr-fence's):
- glmark2 ran the render pool dry from terrain's 256x256 HWRT on (502
  "render pool exhausted" lines).
- At conditionals a refused grow with A = 0 hung the TA. "render:
  stopped" 0 times; the RTI reset the SoC with no record.

**Why no stop:** fix (2)'s stops were gated on the grow knob:
- render.rs `fl_grow_on_demand != 0 && i == STOP_WAIT_MS`;
- freelist.rs `stop_cause.filter(|_| knob && ...)`;
- ccb.rs `if ondemand.is_some() { at_maximum }`.

So the knob-off default kept the 20 s wait that ties with the 20 s alive
window, which is fwccb-2's race. That was my scoping error: the stop was
written for the knob that met the race first, not for every configuration
that can hang.

**Why the pool runs dry at the defaults:**
- The freelists' backing comes from the same render pool (`bo_alloc`).
  At the defaults (reserve x4) they hold 12440 KiB, against 8368 KiB with
  the knob.
- In the knob-off runs that passed (be707e64, ea9fb532) the pool's
  high-water was already 59184 of 59200 KiB. The default has always run
  the pool at its edge.
- List 2 never grew in any run read, yet its x4 reserve is held from boot.
- Which landing since 3f1d2dab moved the default over its 16 KiB margin
  is not measured: no knob-off glmark2 ran in between, and the hung run
  printed no owner lines.
- The default itself (the grow knob on, or list 2 at one grow's reserve)
  is the user's decision.

**The fix, a842cffd:** every stop applies in every configuration. A
refused grow with room left is still no stop. The host tests and the km
record's third deliberate difference are in the commit.

**The default-configuration pair, predictions written before they run.**
Both are built from a842cffd (it holds 6004d8b3) with no knob set:
grow-on-demand, ready pages, grow scene, FWCCB check and kick times all
off. `CONFIG_YAKOGL_PVR_POOL_OWNERS=y` is on: it only prints at run end,
from state the driver keeps, and its owner lines are the instrument for
"what holds the pool".
- **gl_test, eb965d8a** (`build/fl-def-gltest-am62p`):
  - the same case verdicts as main's last default gl_test row, and no
    "render: stopped" line;
  - a case that today ran into the 20 s wait would now end the run with a
    GPU_STOP record (cause 4) at 10 s. If one appears, its case is named
    in the row.
- **glmark2, cbbc5616** (`build/fl-def-glmark2-am62p`):
  - the pool runs dry as in round 2 (this fix does not touch item 1);
  - the run ends in a **GPU_STOP record**: cause 4 at 10 s for the A = 0
    hang, or cause 2 if a list reaches its maximum. Never the RTI;
  - the board's record is 594b1404 / 594b1402, not "no record".

## The lost command: the firmware's cached window (AM62P, 2026-10-04)

**The measurement** (ab5-on, ab6-on: 70b86b80, check on; consoles
`build/fltip6-am62p-20261004-095537/ab5-on-fl-run/20261004-0955-glmark2-70b86b80`
and `.../ab6-on-fl-run/20261004-1008-glmark2-70b86b80`):
- Both read "FWCCB: slot 12 read empty; still empty after 2000 us, type
  0x00000000 (read offset 12, write offset 14)", then skipped it.
- The run-end line: "slots read empty 1 (filled within 2000 us 0, still
  empty 1)". Slot 13, written after it, read fine on the next line.
- Slot 12 is the 13th command, request 7, at heap 0xc5220 + 12 x 160 =
  0xc59a0.
- ab5-off (3689e47f) lost the same command and stopped (cause 4).

**Reading:**
- We never write a FWCCB slot. An invalidate before each of about 100
  re-reads over 2 ms read zero, so memory held zero. The firmware's write
  had not reached it, while its write offset had.
- The candidate "the firmware publishes the offset early" (the slot fills
  within 2 ms) did not occur. The candidate "the host writes back a stale
  offset" did not occur ("went back 0").
- The earlier 0 of 11 with the check was not the check preventing a loss:
  it re-reads and then skips the same slot. "Suggestive" is withdrawn.

**The cause, from the sources:**
- `heap_fwaddr` (mem.rs) handed the firmware the cached window
  (`RGXRISCVFW_SHARED_CACHED_DATA0_BASE | off`, region 0xC) for every host
  object.
- Linux drm/imagination allocates every host-shared firmware object
  `PVR_BO_FW_FLAGS_DEVICE_UNCACHED`:
  - pvr_ccb.c:68-75 (CCB control and ring);
  - pvr_queue.c:1345, :1359;
  - pvr_context.c:335;
  - pvr_free_list.c:207;
  - pvr_hwrt.c:339-408;
  - pvr_fw.c:292-704;
  - pvr_fw_trace.c:122, :144.
- On RISC-V, drm/imagination hands the firmware the SHARED_UNCACHED_DATA
  region's address for such an object (pvr_fw_riscv.c:110-120). The DDK
  header has the same two windows: rgx_riscv.h:102-108, cached 0xC,
  uncached 0x4 (0x4000_0000).
- Through the cached window, the firmware's writes can stay in its own
  data cache. The firmware is built for a host that never gives it that
  window for shared objects, so it has no reason to flush them.

**The opt-in** (147d27f8, `CONFIG_SAFEUI_GPU_PVR_FW_UNCACHED`, off by
default): `heap_fwaddr` returns `RGXRISCVFW_SHARED_UNCACHED_DATA0_BASE |
off`. Every firmware address goes through it. Pin checked, host test on
the region nibble, km records byte-identical with it off.

**The A/B, predictions written before it runs.** Both arms are built from
147d27f8's tree. Their `.config` files differ in one line,
`CONFIG_SAFEUI_GPU_PVR_FW_UNCACHED=y`. Both have the grow knob, ready
pages, grow scene and the FWCCB check on.
- ON: 963fecff (`build/fl-unc-glmark2-on-am62p`).
- OFF: 3246c6fe (`build/fl-uncoff-glmark2-on-am62p`).

The predictions:
1. **The first thing the ON console says, if the window is not mapped:**
   the firmware faults at its first read of a host object. That is a BIF
   or MMU fault, or no boot ack, before the first kick. Region 0xD does
   fault on this firmware (the comment in `heap_fwaddr`). If it happens,
   the arm is stopped there and the row says so.
2. **ON, if it boots:**
   - **0 "read empty" lines across the batch**, and the check's run-end
     line "slots read empty 0" in every run;
   - every grow request answered;
   - the scores as the OFF arm (about 470).
3. **OFF:** about half its runs read slot 12 empty, as ab5-on/ab6-on did.
4. **The side effect to watch:** the re-post at terrain's first kick
   ("fence N (frame fence) not moved at 20 ms, re-posted", present in every
   run so far). If the firmware read a stale KCCB or client CCB through
   its cached window, it disappears with ON. If it stays, it is not this
   cause.
5. The runaway (a served grow acked with ready=0) and the A = 0 hang are
   not predicted to change. If they do, the row says so.

### The check A/B, pairs ab5-ab8 (AM62P, 2026-10-04 09:55-10:57)

ON is 70b86b80 (67f54d8a + the runaway dump) and OFF is 3689e47f.
Consoles: `records/am62p/fltip6-20261004-095537/<run>/<stamp>/console.txt`.
- "still" = "still empty after" lines (the check's event line; ON only).
- "re-posts" = "re-posted" lines.

| run | console sha256, lines | record | requests | type 0 | still | re-posts | the end |
|---|---|---|---|---|---|---|---|
| ab5-on | 8add7be5e141e35c..., 996 | 594b1201 000001d7 (471) | 6 | 1 | 1 | 8 | completed after the loss |
| ab5-off | 23fe3f00930620ec..., 986 | 594b1404 0001926b | 12 | 1 | - | 8 | the loss, then the A = 0 stop (cause 4) |
| ab6-on | 021874bce36376e9..., 997 | 594b1201 000001d6 (470) | 6 | 1 | 1 | 8 | completed after the loss |
| ab6-off | be7a993bf3f19504..., 1079 | 594b1201 000001d8 (472) | 25 | 0 | - | 1 | clean |
| ab7-on | b07abc49e6291907..., 961 | 594b1404 00019a72 | 6 | 1 | 1 | 8 | the loss, then the A = 0 stop (cause 4) |
| ab7-off | 041ef6586a5ab66f..., 1080 | 594b1201 000001d7 (471) | 25 | 0 | - | 2 | clean |
| ab8-on | d6fa03a747ce5c3e..., 1075 | 594b1201 000001d9 (473) | 25 | 0 | 0 | 1 | clean |
| ab8-off | no console | absent | - | - | - | - | no verdict: the console capture could not reach the AM62P's UART bridge at 10:41 ("capture: cannot reach ...: timed out"; it answered again at 10:58). Not a loss and not a pass |

- **Every ON loss has the same line:** "slot 12 read empty; still empty
  after 2000 us, type 0x00000000 (read offset 12, write offset 14)".
- No runaway line in any run. Every first ack is 512/256.
- **The re-posts:** 8 in every run with a loss (the frames after the loss
  wait for the 20 ms re-post), and 1 or 2 in every clean run (terrain's
  first kick).
- **Losses by arm:**
  - ON 3 of 4 here (70b86b80), against 0 of 4 in ab1-ab4 (67f54d8a). The
    two ON images differ only by the dump's code. 0 of 4 against 3 of 4 is
    0.07 by Fisher's test, so it is not a separation either. The ON arm's
    rate moved with a code-only change that is quiet in these runs. That is
    consistent with a cause sensitive to layout and timing, such as the
    firmware's own data cache through the cached window.
  - OFF 1 of 3 with a console (ab8-off has none: the capture failed).

### The default pair's runs (a842cffd, every knob off; AM62P, 2026-10-04 10:58-11:05)

Consoles: `records/am62p/fldef-20261004-105802/{fl-def-gltest-on-1,fl-def-glmark2-on-1}/<stamp>/console.txt`.

**gl_test (eb965d8a)**, console sha256 016d4f7c4ee64a59..., 2167 lines:
- "1 of 40 cases failed (39 passed; 0 timed out; first failure case 48 at
  index 31)". That is case 48, as on main.
- 0 "render: stopped", 0 resets; "pool owner freelists: 6 blocks, 12440
  KiB".
- **The prediction held.** No case reached the 10 s stop, so the
  knob-independent stop changed nothing in gl_test.

**glmark2 (cbbc5616)**, console sha256 9772154f6340e685..., 3630 lines:
- **It completed:** 472 (594b1201 000001d8), 30 ran / 1 failed / 2
  unsupported, GPU resets 0. The predicted stop did not occur. **The
  stop's glmark2 half is "not exercised"**: no fence stood still for 10 s
  and no list reached its maximum.
- The pool ran dry as in round 2: 502 "render pool exhausted" lines. The
  first-refusal snapshot (11:03:25): "driver 15344 + pending 11576 KiB;
  backend + gl (the rest of live) 30520 KiB; pool live 57440 KiB (handed
  out 58344 - free 904)". The freelists are 12440 KiB of the driver's
  15344.
- From 11:03:28 terrain's 256x256 HWRT got no pool memory ("submit
  failed (-12)", textures dropped, framebuffers incomplete). Terrain ran
  121 frames at 82.8 ms.
- **The 8 re-posts (fences 100166-100310) are in terrain, among those
  failed submits, and there is no type-0 line in the console.** They are
  not the lost command. The "8 re-posts" marker is the loss's only when
  a type-0 line precedes it. Here the frames wait on the re-post because
  their own work was lost to the pool.
- **3 grow requests**, all from the reserve:
  - two at bump high-poly (acks 512/256, 768/256);
  - the third at refract (kick 100357, two TAs pending, no 3D), ack
    "cur=1280 top=1275 ready=0", then notices 3 and 4 and no further
    request.
  
  That is the knob-off control's refract shape (be707e64: 4 notices,
  1280): a lone served grow whose ack reads ready=0. It is not the
  runaway, which serves many grows in one batch.
- No refusal; conditionals ran 8220 frames and never asked.

**Why round 2 hung and this run did not:**
- Round 2 grew to 1536 and refused at conditionals (A = 0).
- Here refract needed one grow, and conditionals none: the run-to-run
  variation in demand already named ("the grow scene").
- So the default configuration's hang is not deterministic. With
  a842cffd, when it occurs it ends in a record.

## The AM62P glmark2 cell (the common-set table)

From 2026-10-04 this item is named by its cell in the common-set table:
row glmark2, column AM62P. The goal is that cell green through
`tools/cts/cts-common.sh` on the **default configuration** (every knob at
its default).
- **Today it reads:** "default configuration hangs / completes by
  chance". Round 2 (52be99c3) hung with no record. The default pair's
  glmark2 (a842cffd) completed at 472 because that run's demand never
  reached a refusal.

**The root causes, the measurement that moves each, and how the cell is
re-measured** (a cts-common round on the default configuration after the
change lands, and for a default flip, after the user decides):

| root cause | measurement / change | what it does to the cell | state |
|---|---|---|---|
| a hang ends at the watchdog with no record (the stops were gated on the grow knob) | a842cffd: every stop applies in every configuration | a hang becomes a GPU_STOP record (still red, but named) | gl_test half measured (39/40, no stop); glmark2 half not exercised |
| the lost FWCCB command (the firmware writes through its cached window) | 147d27f8 + the uncached A/B (963fecff vs 3246c6fe) | removes the loss and the hang that follows it | A/B running |
| the render pool at its edge (reserve x4: 12440 KiB of freelists) | the user's decision: grow knob on by default, or list 2 at one grow's reserve | terrain's HWRT gets its memory; fewer refusals | with the user |
| a refused grow with no 3D pending hangs the TA (no partial render) | the PR design (050b5be3), about 12 days | a refusal no longer hangs | the user's go/no-go |
| the runaway (a served grow acked with ready=0, to 4096) | the runaway dump (55fb6660) in every ON run | none yet: instrument | waiting for a hit |

The cell turns green only when a default-configuration cts-common round
completes. One completed run is not enough: the default pair's 472 is a
completion by chance, not a green cell. A green row will need the round's
own repeat count, set by the tool.

### The AM62P glmark2 cell: the uncached A/B, unc1-on (963fecff, FW_UNCACHED on; 2026-10-04 11:06)

Console `build/flunc-am62p-20261004-110627/unc1-on/20261004-1106-glmark2-963fecff/console.txt`
(sha256 be9ec415358b6ce9..., 1084 lines).
- **Prediction 1 (a fault at the first host-object read) did not occur.**
  The firmware booted and ran through region 0x4: no BIF/MMU fault, HWR
  0.
- 497 (594b1201 000001f1), 30 ran / 1 failed / 2 unsupported, GPU resets
  0. "25 grow requests, 3 served, 22 unserved". First acks 512/256,
  768/256, 1024/256, then 1280/0 at the first refusal.
- The check's line: "slots read empty 0 (filled within 2000 us 0, still
  empty 0); write offset went back 0". 0 type-0 lines.
- **Re-posts 0.** The re-post at terrain's first kick, in every run
  before this one, is gone (prediction 4's side effect).
- **The score, 497 against 470-473 in every earlier run of this
  configuration.** Per-scene frame times against ab8-on (473, check on,
  the same knobs except FW_UNCACHED; >3 % only):
  - shorter: texture mipmap 1087 against 1172 us (0.93), blinn-phong-inf
    0.95, phong 0.96, cel 0.95, bump normals 0.92, bump height 0.93;
  - longer: build use-vbo=true 1145 against 1081 us (1.06).
  - Every other scene is within 3 %.
  - What makes those frames shorter is not read. The candidate is the
    firmware no longer reading stale or slow cached copies of the CCBs
    and UFOs (as the terrain re-post's disappearance suggests), but no
    instrument here shows the firmware's side.
- One run. The other three pairs follow; the OFF arm is unc1-off.

### The AM62P glmark2 cell: the lost command's minimal reproducer (gl_test 174 fwccbloss)

The user's rule: a problem is worked on with the smallest program that
reproduces it, not the large one again and again. glmark2 ran about 40
times as this item's instrument; that stops. The uncached A/B was stopped
during unc1-off; unc1-on's row stands as one run.

**The case** (4e90d1c4, `driver/tools/pvrlab/gl_test.c` case 174):
- Frames of N full-screen quads into a 1024x1024 target, each flushed
  without a wait. N grows by a quarter per 16-frame phase until a phase
  asks for a grow, then stays.
- With `FL_PROBE_NO_RESERVE` every grow is refused. The list stays at 512
  pages, and each overlap of two frames asks again: the request and its
  notice, two FWCCB commands a frame.
- Each phase prints "gl_test: fwccbloss phase P, N quads: requests ...
  (served, refused), notices, slots read empty E (filled F), re-posts R,
  stop S". The case fails when E > 0 or S != 0.
- It runs for `CONFIG_GL_TEST_FWCCB_LOSS_SECONDS` (25 s).

**The images** (both with the check, `NO_RESERVE`,
`GL_TEST_CASES="174"`; their `.config` files differ in one line,
`FW_UNCACHED=y`):
- cached 73686a91 (`build/fwl-cached-am62p`);
- uncached 0fb9fbee (`build/fwl-uncached-am62p`).

**Predictions, written before they run:**
1. **The reproducer reaches the mechanism:** within a few phases a phase
   asks for grows (N held), then about one request and one notice per
   frame. If no phase up to 2048 quads asks, the case prints "NOT
   EXERCISED", and the reproducer must change (larger target or N)
   before anything is read from it.
2. **The known positive on the cached arm:** in a few runs, "slots read
   empty >= 1 (filled 0)", with the check's event line "slot S read
   empty; still empty after 2000 us". glmark2's was slot 12 at the 13th
   command. Here the position may differ, so the row reports S and the
   offsets. Until the cached arm shows it in some run, the reproducer does
   not count, and the uncached arm says nothing.
3. **The uncached arm:** "slots read empty 0" in every run.
4. **Once 2 holds:** 20+ runs per arm. The loss rate per run (cached)
   against 0 (uncached) settles the A/B.
5. **The risk:** a phase whose single frame exceeds 512 pages (A = 0 at a
   refusal) hangs the TA, which is the PR gap. The run then ends in the
   cause-4 stop at 10 s with a record. That is reported as reproducer 3's
   case, not as a loss.

### The AM62P glmark2 cell: the refused-grow hang's minimal reproducer (gl_test 175 refusehang)

**The case** (8dad3d71):
- One kick in flight (glFinish every frame), frames of N full-screen
  quads into a 1024x1024 target, N up by a quarter each frame until a
  frame asks for a grow.
- With `FL_PROBE_NO_RESERVE` that grow is refused with no 3D pending
  (A = 0).
- It prints "gl_test: refusehang: F frames, the last of N quads asked for
  a grow: requests ..., notices ..., re-posts ..., stop S, partial renders
  P".
- It is the partial-render design's forced case, and it proves the
  knob-independent stop: this image has every knob off.

**Image 0ea4c598** (`build/rfh-am62p`, GL_TEST_CASES="175",
`NO_RESERVE`, nothing else). Predictions, written before it runs:
1. A frame asks for a grow within about 10-20 frames. Otherwise "NOT
   EXERCISED", and the case must change.
2. That frame's kick never completes. At 10 s: "render: stopped: a kick
   that did not complete in the stop's time", and a GPU_STOP record with
   cause 4. **Not the watchdog** (a842cffd, the knob off).
3. If the run reaches the case's own line: "partial renders 0", "stop 4".
4. The same every run. It is deterministic by construction (one kick in
   flight, no reserve), so 3 runs settle it, not 20.

**Case 174 v1 on the AM62P** (`build/fwl-am62p-20261004-112856/fwl-cached-on-1`,
image 73686a91):
- Phases 0-10 (32 to 291 full-screen quads) asked nothing, with 16
  re-posts a phase. Two frames in flight wait on the 20 ms re-post every
  frame here, so in this workload re-posts are no loss marker.
- Phase 11 (363 quads) asked once: "no grow (reserve 0, pool 0/0,
  512/4096 pages)", ack "cur=512 top=483 ready=0", "2 kicks in flight: 1
  with TA done and 3D pending, 1 with TA pending" (A = 1, B = 1).
- Then "render: stopped: a kick that did not complete in the stop's time"
  at 10 s (cause 4), case TIMEOUT.
- **Three readings:**
  - **a842cffd's knob-off half, measured:** the 10 s stop fired with every
    grow knob off, and no watchdog.
  - **A counterexample to "a refusal is survived while a 3D is pending":**
    here A = 1 and the TA still hung. The pending 3D's pages were not
    enough for a frame of 363 full-screen quads in 512 pages. The
    condition is "a pending 3D that frees enough", which the instrument
    cannot see. The reading "A = 0 hangs" stays, but "A >= 1 survives" is
    withdrawn as stated.
  - **The case's own defect:** its summary said "NOT EXERCISED ... never
    asked" though phase 11 asked (the stop broke the loop first). Fixed
    in v2, which counts by number.
- Runs 2-5 were stopped: they would repeat run 1.

**Case 174 v2** (c20524c5): served grows (the grow knob on), and layers of
a 4x4-pixel quad in each of the 1024 tiles, 8 layers a draw, L up by a
quarter a phase to 1024. Its verdict needs at least 13 FWCCB commands
(glmark2's count at the loss), no empty slot, no refusal, no stop.
- Images, their `.config` files one line apart (`FW_UNCACHED=y`):
  cached cb8a30c9 (`build/fwl2-cached-am62p`), uncached 04779640
  (`build/fwl2-uncached-am62p`).
- **Predictions, written before they run:**
  1. A phase asks within a few seconds. Then about one served grow per
     phase while L rises, with a notice each, until the list stops
     growing (L at its cap, or the pool). Requests + notices >= 13 in
     about 15 s. If not, "NOT EXERCISED: N FWCCB commands", and the case
     changes again.
  2. **Cached:** in some of 5 runs, "slots read empty >= 1 (filled 0)",
     with the "still empty after 2000 us" line. The row gives its slot
     and offsets.
  3. **Uncached:** 0 empty in every run.
  4. **A refusal** would mean the render pool refused a refill: the case
     fails by name. It is not expected in gl_test's small pool use.

**Case 174 v2 with the dump: reproducers 1 and 2 in one run.** These
images replace cb8a30c9/04779640: the same tree (c20524c5) plus
`GROW_SCENE=y`, so the runaway dump fires at any batch with 2 or more
served grows. The `.config` files are one line apart (`FW_UNCACHED=y`):
cached b11d0eec (`build/fwl3-cached-am62p`), uncached 60f885c5
(`build/fwl3-uncached-am62p`). Planned: 20 runs per arm, interleaved, in
one chain.

Predictions, added to v2's:
- the runaway shows as a "render: runaway batch: N grows served in one
  FWCCB batch; ..." line with N >= 10, then the at-maximum stop (cause 2).
  A batch of 2 is the control;
- whether the runaway occurs in this workload is open. 0 of 40 would say
  it needs something glmark2's bump height has and this lacks. That is a
  result, not a pass.

**Case 175 refusehang, 3 runs** (image 0ea4c598, every grow knob off,
`NO_RESERVE`; consoles
`records/am62p/rfh-am62p-20261004-113324/rfh-on-{1,2,3}/<stamp>/console.txt`,
sha256 b5c5db69fa8b1e55..., 00016ae9edabd740..., f9cf67790104eda8...,
309/310/312 lines):
- Each run: "gl_test: refusehang: 15 frames, the last of 707 quads asked
  for a grow: requests 1 (served 0, refused 1), notices 1, re-posts 15,
  stop 4, partial renders 0".
- Then "render: stopped: a kick that did not complete in the stop's time",
  case 175 TIMEOUT, "1 of 1 cases failed". The board came back after
  70/66/76 s; no watchdog.
- **3 of 3, identical:** the predictions held. The reproducer is
  deterministic.
- **What it proves:**
  - the knob-independent stop (a842cffd) on the default path: a hang ends
    at 10 s by name;
  - the partial-render gap: a refused grow with no 3D pending is never
    survived (partial renders 0).
- **The PR fix's yardstick:** this case must pass, with the frame complete
  and "partial renders" >= 1.
- gl_test prints no "yk-record" line in these runs. The board verdict is
  gl_test's own summary.

**Case 174 v2, cached x5** (b11d0eec, `records/am62p/fwl3-am62p-20261004-114010/`):
- Five identical runs: "12 phases, last 97 layers, about 3655 ms:
  requests 14 (served 14, refused 0), notices 17-19, FWCCB commands
  about 31-33, slots read empty 0 (filled 0), re-posts 31-34, stop 2".
- No runaway line. Each run ended in the at-maximum stop, as v3 fixes.
- **The positive: 0 of 5.**

**glmark2's sequence before the loss** (read from ab5-on, ab6-on, ab7-on,
c18f9bfd):

| slots | commands | acks |
|---|---|---|
| 0-5 | three served grows, each followed by its notice | 512/256, 768/256, 1024/256 |
| 6-11 | three grows the render pool refused ("refused 1036 KiB (1280/4096 pages listed)"), each followed by its notice | 1280/0; at A/B 0/1, 2/1, 2/1 |
| 12 | request 7, never visible | - |

The clean glmark2 runs have the same first 12 commands, and their
request 7 arrives. So the loss sits at one point of one sequence, which
v2/v3 never make: they serve every grow.

**v4** (95a03be9 and 892cf5fe): the probe
`CONFIG_SAFEUI_GPU_PVR_FL_PROBE_SERVE_MAX=3` (refuse grows after 3
served, by name, no stop), with case 174 holding its load after 3 grows
(`GL_TEST_FWCCB_LOSS_HOLD_GROWS=3`).
- Two overlapping frames then pass 1280 pages while one fits, so each
  overlap asks and is refused, and the frame goes on when the other
  frame's 3D frees its pages. That is refract's shape.
- Refusals are printed, not a failure.
- **Images**, one `.config` line apart: cached c3b3b742
  (`build/fwl5-cached-am62p`), uncached 2b4b3dc6
  (`build/fwl5-uncached-am62p`).

**Predictions:**
1. Each run's first 12 FWCCB commands are 3 x (served grow, notice) and
   then 3 x (refused grow, notice), with the acks above.
2. **Cached:** request 7 read empty ("still empty after 2000 us", read
   offset 12, write offset 14) in some runs, at about glmark2's rate on
   this arm (3 of 4 in ab5-ab8).
3. **Uncached:** 0.
4. **The risk** (v1's): a refused overlap whose other frame's 3D does
   not free enough hangs (cause 4). That run then counts as reproducer
   3's, and the hold point must move lower.

**v3 (a6cd74e9 / 050efebf), the three runs before it was stopped:**
- c1-cached, u1-uncached, c2-cached each read "requests 11 (served 11,
  refused 0), FWCCB commands about 23, slots read empty 0", PASS at about
  25 s, no runaway line.
- Served-only traffic does not reach the loss, as predicted from
  glmark2's sequence.

**v4 cached x5 (c3b3b742):**
- Every run read "205 phases, last 18 layers: requests 3 (served 3,
  refused 0), FWCCB commands about 6", with 0 probe refusals.
- The hold after the third grow froze a load at which two frames fit, so
  nothing asked again. Each run read FAIL with no failure in it.

**v5** (e64aeeef): rise to the first refused grow, then hold. Fewer than
13 commands is "skipped by name" (NOT EXERCISED); a loss or a stop fails.
Images, one `.config` line apart: cached c805d874
(`build/fwl6-cached-am62p`), uncached 5cacabc6 (`build/fwl6-uncached-am62p`).
Both have the probe at 3, the grow knob, the check and the dump.

**Predictions:**
1. One cached run shows glmark2's 12-command prefix: three served grows
   with notices, then refusals ("the probe's limit of 3 served grows"),
   each with its notice, about one per frame, and more than 13 commands.
2. Cached: request 7 read empty in some runs. Uncached: 0.
3. **The risk:** a refused overlap that does not free enough hangs
   (cause 4), which reads as reproducer 3's case.

**v5 cached x2 (c805d874)**, identical:
- 3 served grows, one "no grow: the probe's limit of 3 served grows
  (1280/4096 pages listed)";
- then the hold at 22 layers and no further request in 174 phases (8
  commands, NOT EXERCISED, 0 empty).
After the refusal the following frames fit, so nothing asked again.

**v6** (a70aafc2): `fwl_next` rises by an eighth while a phase asked in
fewer than 12 of its 16 frames, then holds for good. It was checked on the
host with 8 cases, built from the function's own text. Images, one
`.config` line apart: cached 27c75b70 (`build/fwl7-cached-am62p`),
uncached 28b5b3d8 (`build/fwl7-uncached-am62p`).

**Predictions:**
1. After 3 served grows, refusals by the probe, rising per phase until a
   phase has at least 12 requests; then about one refused request and one
   notice per frame for the rest of the 25 s: hundreds of FWCCB commands,
   many laps of the ring.
2. Cached: slot 12 (request 7) read empty in some runs. Uncached: 0.
3. **The risk** (v1's): a refused overlap that frees too little hangs
   (cause 4). If the rise passes the point where one frame alone exceeds
   1280 pages, that frame hangs.

### The AM62P glmark2 cell: case 174 v6 reproduces slot 12 (2026-10-04 12:16)

v6 cached x5 (27c75b70), run dir `build/fwl7-am62p-20261004-121605/fwl7-cached-on-{1..5}`.
Consoles (sha256, lines): b18a740adc2d1006 428, b3df05aebeb93317 429,
35a2becf5007318a 430, ce7d9cc0ec21677c 430, abbe700805e15637 432.
- **Every run makes glmark2's sequence:**
  - 3 served grows (phases 3 and 9), each with its notice;
  - then probe refusals from phase 11 on (22 layers), each with its
    notice;
  - so slot 12 holds request 7 on the ring's first lap, as in glmark2.
  - Each run: "requests 21 (served 3, refused 18), notices 21, FWCCB
    commands about 42".
- **Every run then ends in the A = 0 hang:** at phase 15 (33 layers) a
  frame alone exceeds 1280 pages, giving "render: stopped: a kick that did
  not complete in the stop's time", cause 4, about 68 s (prediction 3's
  risk).
- **Run 5, line 189:** "FWCCB: slot 12 read empty; type 0x2abc0067 after
  240 us (read offset 12, write offset 14)".
  - That is the same slot and offsets as every glmark2 loss, and the same
  command (request 7, a FREELIST_GROW, 0x2abc0067).
  - Here the check's re-read saw it fill 240 us after the write offset
    said it was written. glmark2's stayed empty for 2 ms.
  - The run-end line: "slots read empty 1 (filled within 2000 us 1,
    still empty 0)".
  - Runs 1-4 read no empty slot.
- **Reading:**
  - The reproducer reaches glmark2's exact point. At that point the
    command became visible in memory after the write offset that
    publishes it, here by 240 us and in glmark2 by more than 2 ms.
  - Both are the same fact: the firmware's write of the slot reaches
    memory late relative to its write offset. That is what the firmware's
    cached window permits, and what FW_UNCACHED removes.
  - "Filled within 2000 us" is the first measurement of the first
    candidate's branch. The check, which re-reads, recovered this one;
    without it the slot would have been skipped as glmark2's was.
  - 1 of 5 runs here, at one slot-12 event a run.
- **The A/B can run on v6 as it is.** Each run passes slot 12 on lap 1
  before its hang, and the hang does not touch the count. A cleaner v7
  would hold once 16 requests were made (after phase 14 at 30 layers,
  which survived).

**v7** (2534fd2c): hold once the run made 16 requests. Images, one tree,
the `.config` files one line apart (`FW_UNCACHED=y`): cached a21eeb88
(`build/fwl8-cached-am62p`), uncached 22d50a4c (`build/fwl8-uncached-am62p`).

**Expected run-end lines:**
- "gl_test: fwccbloss: P phases, last 30 layers (or near it), about 25000
  ms: requests R (served 3, refused R-3), notices about R, FWCCB commands
  about 2R, slots read empty E (filled F), re-posts X, stop 0";
- "render fwccb: slots read empty E (filled within 2000 us F, still empty
  S); write offset went back 0".
- R well above 16 (the refusals go on at the held load). The case PASSes
  with E = 0 and stop 0, and FAILs on E >= 1.

**The A/B prediction** (20+20 interleaved, cached first):
- cached E >= 1 in about 1 in 5 runs, up to glmark2's rate, each naming
  slot 12 at read offset 12 / write offset 14 (filled or still empty);
- uncached E = 0 in all 20.
- **The named risk:** the held load (30 layers) survived one phase in v6.
  A later hang (cause 4) would end that run after its slot 12, which does
  not change its count.

## The AM62P glmark2 cell: the shadow scene's depth-only framebuffer (the PowerVR half, plan; no code)

**The contract** (gc-reader, gc/depth-only-fbo 20e430f7):
- `backend::DEPTH_ONLY_TARGETS` comes from each backend's
  `DRAWS_WITHOUT_COLOUR`. Where it is true, a framebuffer with only a depth
  and/or stencil attachment is complete.
- A draw or clear into it reaches the backend with `target_gpu` 0 at the
  depth (else stencil) size, colour writes off. A colour clear is dropped.
- Today the PowerVR says false: glmark2's shadow scene reads "Set up failed"
  on the AM62P, as round 2 did.
- gl_test case 177 (zonly) is the shared proof. It SKIPs by name where the
  status is GL_FRAMEBUFFER_UNSUPPORTED.

**Mesa 26.2.1's pvr for a render with no colour attachment**
(eot_surface_count == 0):
- one PBE emit whose state word 1 has `emptytile = true`: nothing is
  written (pvr_arch_cmd_buffer.c:1508-1515). The surface setup zeroes the
  other PBE words for it (pvr_arch_job_common.c:102-111);
- `job->z_only_render = !eot_surface_count && !frag_has_side_effects &&
  !has_depth_feedback` (pvr_arch_cmd_buffer.c:1687-1689), packed as
  `CR_ISP_ZLSCTL.zonlyrender` (pvr_arch_job_render.c:1005). Our csbgen
  already has the field (src/pvr/csbgen.rs:316, :357);
- the depth attachment stored by ZLS as for any render.

**The plan for src/pvr/backend.rs** (after the lost command, as decided):
1. `DRAWS_WITHOUT_COLOUR = true`, and at `target_gpu` 0 no colour target:
   - one EOT emit with the empty-tile PBE word (Mesa's words, transcribed);
   - `isp_zlsctl` with `zonlyrender` and the depth store to the attached
     texture (our ZLS store path: `isp_zlsctl_stores`, case 169 dstex);
   - the frame's size from the depth attachment.
2. Host tests:
   - the kick's PBE words for a depth-only frame equal Mesa's z-only words;
   - `zonlyrender` is set;
   - the ZLS store base is the depth texture's;
   - a framebuffer with colour keeps today's words (the known negative:
     today's status test refuses it, 0x8cdd).
3. **The board:** gl_test 177 (zonly) on the AM62P PASSes with the depth
   read back, then glmark2's shadow scene runs (the cell's "1 failed"
   becomes 0). That is checked by a cts-common round on the default
   configuration, not by repeated glmark2 runs.

### The AM62P glmark2 cell: the cached window delays the firmware's read of the KCCB too (v7 A/B part 1)

Part 1 of the v7 A/B (`build/fwl8ab-am62p-p1-20261004-124152`), from the
consoles. **The ack time** (our KCCB post to the firmware consuming it),
over every grow ack of a run:

| run | acks |
|---|---|
| cached-1 (a21eeb88) | 3 at 1 ms, 1 at 2, 1 at 3, **54 at 5 ms, 329 at 8 ms** |
| uncached-2 (22d50a4c) | **435 of 435 at 1 ms** |
| unc1-on (glmark2, 963fecff, uncached) | 25 of 25 at 1 ms |

- The firmware's ack words are identical in both arms: "cur=1280
  top=1093 ready=0 pending=0 alloc=186/0" at every refusal.
- **The instrument** (freelist.rs `grow`, after `kccb_post`):
  - Each loop invalidates the KCCB control block, compares the read and
    write offsets, then `busy_wait_us(1000)`, and "after N ms" is the
    count of those loops. So N resolves 1 ms (the consumption came between
    loop N - 1 and N, each loop a little over 1 ms), and every value from
    1 up is reachable: 1, 2 and 3 do occur in cached-1.
  - The two values 5 and 8, with nothing at 4, 6 or 7, are therefore not
    our poll's quantum. They are the firmware's.
  - **5 ms equals this firmware's `ActivePMLatency`** (`PVR_FW_PM_LATENCY_MS`
    5, pvr_fw_boot.c), but the PM-latency-10 probe below rules that out as
    the cause: at 10 ms the acks stay at 5 and 8. That is the period which an earlier measurement
    found the KCCB MMU-invalidate ack delay to follow: medians 98.45 /
    18.95 / 4.10 ms at 100 / 20 / 5 (tools/cts/RESULTS.md, the
    am62p/pm-latency row of 9/29, on the Linux harness).
  - The other configured periods are no candidate: the safety watchdog is
    2 s and the HCS deadline is unlimited.
  - **8 ms is not explained.** The candidate: 5 ms counted from the GPU
    going idle, which here comes about 3 ms after our post (the other
    frame's 3D). It is not shown.
- **Reading** (corrected after the PM-latency-10 probe):
  - through the cached window, the firmware takes our KCCB answer only
    after a firmware period of 5 or 8 ms, not at the doorbell. That
    period is not the ActivePMLatency (ruled out by the 10 ms probe) and
    not our poll (1 ms loops). What it is is not known;
  - through the uncached window, at once (1 ms, the poll's floor).
  - That is the host-to-firmware direction of the same cause as the lost
    FWCCB command (firmware-to-host).
  - The earlier "MMU-invalidate acks follow ActivePMLatency" (Linux
    harness, 9/29) is a different measurement, and the probe did not test
    it. It stays as recorded.
- **The test that decides it:** case 174 cached with
  `CONFIG_SAFEUI_GPU_PVR_PM_LATENCY_MS=10`. If the 5/8 ms acks are the PM
  timer they move to 10 and about 13. The uncached arm stays at 1 ms.
- **The probe: the image and its prediction** (written before the run).
  - Image `fwl10-cached-pm10-am62p`: built from the v8 tree 01caef79,
    with the cached arm's build line plus one setting,
    `-DCONFIG_SAFEUI_GPU_PVR_PM_LATENCY_MS=10`. Its `.config` differs
    from fwl9-cached's in that line only (5 -> 10).
  - Runs: two, right after part 2. The uncached arm is not rerun; its
    1 ms floor stands from part 1.
  - **If it is the PM timer:** the refused-grow acks move from 5 and 8
    to about 10 and about 13. A few still come at 1-3, as in cached-1.
  - **If it is not:** the acks stay at 5 and 8, and the PM latency is
    ruled out as their cause.
  - Anything else (a spread, or new values) is written down as it reads.
  - The 8 ms stays "unexplained" until a line shows its cause.
- **Correction:** the "refused grows acked in 4-11 ms, the firmware busy
  after a refusal" reading of the earlier cached glmark2 runs (fwccb-1,
  -3, -4 and the c18f9bfd row) was this, not a busy firmware. ab8-on's
  and 70b79a74's 1 ms acks came from our post being delayed by the before
  line, as recorded then.
- **The consequence for case 174:**
  - in the cached arm, each refusal costs 5-8 ms, the frames back up, and
    at 27 layers two frames overlap and every frame asks (7 a phase);
  - in the uncached arm the refusal resolves in 1 ms (requests 3-4 even
    read A = 2, B = 0), and 27 layers asks nothing; it starts at 30;
  - so v7's hold at 16 requests by phase 14 was met by the cached arm and
    missed by the uncached arm, which rose to 33 and hung (4 of 6).
- v8's cap at 30 keeps the two arms comparable for the lap-1 slot-12
  count: both reach 30 by phase 14, and both make the same first 12
  commands. The timing difference is the knob's own effect.

### The AM62P glmark2 cell: v7 A/B part 1, the tally (2026-10-04 12:41-13:15)

Part 1 (`build/fwl8ab-am62p-p1-20261004-124152`; v7, cached a21eeb88 /
uncached 22d50a4c, 10 + 10 interleaved, cached first; uninstall rc 0,
environment diff 0). The tally is the lead's, from the consoles.

| arm | runs | read empty (filled) | still empty | hangs (cause 4) | PASS |
|---|---|---|---|---|---|
| cached | 10 | **1** | 0 | 0 | 9 |
| uncached | 10 | **0** | 0 | 7 (runs 1, 3, 4, 5, 7, 8, 9) | 3 |

- **The one event**, cached-7, console line 222:
  "FWCCB: slot 28 read empty; type 0x2abc0067 after 240 us (read offset
  28, write offset 30)".
  - It came at grow request **15** (after 3 served and 11 refused), the
    29th command (each request is 2 commands, GROW then UPDATE_STATS, so
    slot 28 is request 15's GROW), still lap 1 of the 32-slot ring.
    Correction: first written as request 14, from the "at grow request
    14" line just before it; that line closes request 14's lines.
  - The ack after it came at 1 ms.
- **So the lap-1 event is not pinned to slot 12.**
  - glmark2's and v6 run 5's slot 12 was request 7 of those sequences;
    slot 28 is request 14 here.
  - What the three share: the write offset is the slot + 2 (the firmware
    had posted two more commands), and the slot filled after **240 us**.
- **Exposure, per arm** (the runs that reached the slot):
  - slot 12: all 10 cached and all 10 uncached runs. The first 12
    commands are the same in both arms. Events: cached 0, uncached 0.
  - slot 28: the 10 cached runs (none hung), against 5 uncached runs:
    the 3 that held, and uncached-4 and -8, which hung after 34 commands.
    The other 5 hung after 28 and never read slot 28. Events: cached 1
    of 10, uncached 0 of 5. (First written as 3 uncached runs, from an
    estimate of 26 commands per hung run; the consoles count 28 or 34.)
  - Over all slots read: cached 1 event in 10 runs, uncached 0 in 10.
    1 in 10 is too few to call the arms different; part 2 (v8, both arms
    hold at 30 layers and every run passes lap 1) is the comparison.
- **The instrument: what "after N us" is** (ccb.rs `reread_empty_slot`,
  the code that prints the line):
  - Each loop busy-waits 20 us (`STEP_US`), then invalidates the slot
    and reads its type word. "after N us" is the loop count times 20,
    bounded at 2000 us (`BOUND_US`, then "still empty").
  - So N resolves 20 us: 240 is the 12th re-read, each after a little
    more than 20 us of wall time (the invalidate and the read are not
    counted). The test's own line reads 60 us, so 240 is not a constant
    of the loop.
  - 240 twice (v6 run 5 and cached-7) is therefore the firmware's time,
    within 20 us, from moving the write offset past the slot to its
    command words becoming visible to us. A cause is not shown.
    The candidate, from the cached-window reading above: the
    firmware's data cache holds the slot's words and writes them back
    later than the write offset. Two events are not yet a quantum; part
    2 adds more.

### The AM62P glmark2 cell: the slot-28 event has a fixed place (parts 1 and 2)

Part 2's first cached run (`build/fwl9ab-am62p-p2-20261004-131550`,
cached-1, v8 e58330b8) printed the same line as part 1's cached-7, at the
same console line 222: "FWCCB: slot 28 read empty; type 0x2abc0067 after
240 us (read offset 28, write offset 30)".

**The ack times of the first 18 grow requests** (`grep 'grow ack|FWCCB:
slot'` over each console; E = the empty read, which comes before request
15's GROW):

| runs | requests 1-18 |
|---|---|
| part 1 cached 1-6, 8-10; part 2 cached-2, -3 | 1 1 1 2-3 5 5 5 5 5 5 5 2 8 8 8 8 8 8 |
| part 1 cached-7; part 2 cached-1 | 1 1 1 3 5 5 5 5 5 5 5 2 8 8 **E** 1 8 8 |
| part 2 uncached-1, -2 | 1 at every request |

- **The schedule is deterministic:** every cached run acks the same
  sequence, except 2 or 3 ms at request 4. The event comes at request 15,
  the third request acked in 8 ms, in 2 of 13 cached runs. It came at no
  other request in any run: 388 acks per full run, 0 other empty reads.
- **The kick state does not single request 15 out.** "2 kicks in flight:
  1 with TA done and 3D pending, 1 with TA pending" is printed at
  requests 1 and 3-19 alike, so request 15 is not the first with two
  kicks in flight. Nothing we log differs between requests 13-16.
- **What the event changes:** request 15's ack came in **1 ms**, where
  every other run takes 8 ms. Our KCCB post was taken at once, so the
  firmware was still active (not waiting for its timer) when it came.
- **The reading this supports** (a hypothesis, not shown):
  - in the event, the firmware advanced the write offset (to 30, past
    slot 29's UPDATE_STATS too) and wrote slot 28's words visibly only
    240-260 us later, and it was still running when our ack arrived;
  - so the slot's words became visible later than the offset, within
    one active period of the firmware.
  - Why only request 15 is not explained.
- **What part 2 can still tell:** whether every cached event is at
  request 15 with the 1 ms ack after it, and whether the uncached arm
  shows none over runs that all pass slot 28.

### The AM62P glmark2 cell: v8 A/B part 2, the tally, and both parts together (2026-10-04 13:15-13:50)

Part 2 (`build/fwl9ab-am62p-p2-20261004-131550`; v8, cached e58330b8 /
uncached e893a03a, one tree 01caef79, the knob `FW_UNCACHED` the only
difference; 10 + 10 interleaved; uninstall rc 0, environment diff 0).
Read from each console: the "W: FWCCB: slot" lines, "render fwccb:"
summaries, "still empty after", "render: stopped" and the case verdict.

| arm | runs | hangs | runs with an empty read | still empty | commands per run |
|---|---|---|---|---|---|
| cached | 10 | 0 | **4** (cached-1, -4, -6, -9; one each) | 0 | 777 (all) |
| uncached | 10 | 0 | **0** | 0 | 857-871 |

("Commands" here is the count of grow-ack plus UPDATE_STATS lines: the
FWCCB commands we read, within one.)

- **Every event, read from its console:**

  | run | line | request | the ack after it |
  |---|---|---|---|
  | cached-1 | slot 28 read empty; ... after 240 us (read offset 28, write offset 30) | 15 | 1 ms (the schedule: 8) |
  | cached-4 | the same | 15 | 1 ms (8) |
  | cached-9 | the same | 15 | 1 ms (8) |
  | cached-6 | slot 12 read empty; ... after 240 us (read offset 12, write offset 14) | 7 | 1 ms (5) |

  - Every event: type 0x2abc0067 (GROW), filled after exactly 240 us,
    write offset = slot + 2, and the next ack taken in 1 ms.
  - Request 7 / slot 12 is glmark2's slot from the cached glmark2 runs.

- **Both parts together** (one tree per part; v7 and v8 differ only in
  the reproducer's hold):

  | arm | runs | reached slot 12 | reached slot 28 | runs with an event | events | commands read |
  |---|---|---|---|---|---|---|
  | cached | 20 | 20 | 20 | **5** (slot 12 x1 at request 7, slot 28 x4 at request 15) | 5 | 15540 |
  | uncached | 20 | 20 | 15 | **0** | 0 | 11467 |

- **The test and its number:**
  - Unit: the run. The events sit at fixed requests (7 and 15) in a
    deterministic schedule, so a run is one trial at those places.
  - Fisher's exact test on runs with an event, 5 of 20 against 0 of 20:
    p = C(20,5)/C(40,5) = **0.024** one-sided (0.047 two-sided).
  - Counting only the uncached runs that reached slot 28 (15): p =
    C(20,5)/C(35,5) = **0.048** one-sided.
  - Per command read (5 events in 15540 against 0 in 11467), the
    conditional binomial gives p = (15540/27007)^5 = 0.063. Commands
    are not independent trials here, so this is a check, not the test.
  - **Reading:** at the run level the cached window loses commands and
    the uncached window has not, at p 0.024-0.048. That is not proof by
    count alone. The other evidence:
    - the symptom's shape: every event is a slot whose words became
      visible 240 us after the write offset passed it. That the cached
      window is what delays them is a hypothesis, not shown;
    - the reference: drm/imagination hands the firmware the uncached
      window (`RGXRISCVFW_SHARED_UNCACHED_DATA0_BASE`);
    - the uncached arm's acks: 1 ms throughout, against 5-8 ms cached.

### The AM62P glmark2 cell: the PM-latency-10 probe (be42a690, 2026-10-04 13:52)

Two runs of `fwl10-cached-pm10-am62p` (sha16 be42a690dc6b040e; the v8
cached image with `CONFIG_SAFEUI_GPU_PVR_PM_LATENCY_MS=10`, its `.config`
different in that line only), `build/fwl10pm-am62p-20261004-135216`
(uninstall and environment diff in the directory).

- **The setting took effect:** console line 97 "runtime cfg:
  ActivePMLatency 10 ms" in both runs. The v8 cached runs print "5 ms"
  at the same line.
- **The ack times** (every grow ack of the run, from the console):

  | run | acks |
  |---|---|
  | pm10-cached-1 | 3 at 1 ms, 1 at 2, 1 at 3, **54 at 5, 329 at 8** (388) |
  | pm10-cached-2 | 3 at 1 ms, 1 at 2, 1 at 3, **53 at 5, 1 at 6, 329 at 8** (388) |
  | for comparison, part 1 cached-1 (PM latency 5) | 3 at 1 ms, 1 at 2, 1 at 3, 54 at 5, 329 at 8 |

- **Against the prediction** (written before the run): "If it is not
  [the PM timer]: the acks stay at 5 and 8, and the PM latency is ruled
  out as their cause." They stayed. **The ActivePMLatency is ruled out**
  as the cause of the 5 and 8 ms acks.
- **What stands:** a firmware period of 5 and 8 ms, which is neither the
  ActivePMLatency nor our poll (1 ms loops), delays the firmware's
  taking of our KCCB post through the cached window. Through the
  uncached window there is no such delay (1 ms, every ack). Its cause
  is open.
- **The lost command:** both runs PASS with 0 empty reads.
  - Cached runs over the three images: 5 of 22 with an event.
  - This image is not one of the A/B's arms (its PM latency differs),
    so the rate test stays the same-tree one: 5 of 20 against 0 of 20,
    Fisher one-sided p = 0.024.
  - Pooling all 22 cached runs gives p = C(22,5)/C(42,5) = 0.031.

### The AM62P glmark2 cell: the PowerVR z-only render, built (1d63dec4, on 520a6c0f) and its board prediction

Branch `pvr/zonly-tip` from the landing tip 520a6c0f, one commit (1d63dec4),
following the plan above:
- `DRAWS_WITHOUT_COLOUR = true`. A draw with no colour target and a depth or
  stencil attachment makes a z-only kick. The kick has one end-of-tile emit
  with the empty-tile PBE state (word 0 zero, word 1 `emptytile`), zero PBE
  register words and `CR_ISP_ZLSCTL.zonlyrender`. Depth is stored to the
  object's own buffer, and the texture reads it through `depth_resolve`, as
  case 169 does.
- The frame's first quad loads no colour (there is none). It initialises
  the depth with the clear program and samples nothing.
- A draw with no colour, depth or stencil target is refused (-EINVAL).
- Host tests: the bit positions come from Mesa's cr.xml and pbestate.xml,
  not from our packer. Each of the five parts was undone in turn, and each
  undo fails a test. The sixth part is the constant: setting it to false
  fails the tests' compile-time check.

**The image:** `build/zonly-am62p` in pvr-flgrow, sha16 82c9612a9f350be2,
tree 1d63dec4, `CONFIG_GL_TEST_CASES="169,177"`, every knob at its
default (FW_UNCACHED on, as on 520a6c0f).

**Prediction (written before any run):**
- **169 (dstex) PASS: the control.** It has a colour target, so its kick
  is unchanged. If it fails, that is read before 177 is.
- **177 (zonly) PASS:**
  - check 3 reads the status GL_FRAMEBUFFER_COMPLETE, not the SKIP;
  - checks 4 and 5 read no GL error;
  - checks 6-9 read the four depth points equal to `dst_ramp_byte`, within 2.
- GPU resets 0. No "draw refused" line.
- **If 177 fails:**
  - at check 3: the GL core did not see the constant (the image is not
    this tree);
  - at checks 6-9 with 1.0 everywhere: the z-only kick stored no depth
    (the ISP's store under `zonlyrender` is then the suspect);
  - a fault or reset: the empty-tile emit or the zero register words.
- After that, glmark2's shadow scene is checked by the next common round,
  not by repeated glmark2 runs.

### The AM62P glmark2 cell: the render pool's size, and room for 23 MiB more

From the board definition (SafeUI repository,
`boards/toradex/verdin_am62p/verdin_am62p_am62p54_a53.dts`) and
`driver/src/pvr_internal.h`:

- **DDR:** 2 GiB at 0x80000000 (Verdin AM62P Quad 2GB).
  - 0x82000000, 344 MiB: the Zephyr A53 image and its RAM (`zephyr,sram`).
  - 0x97800000, **64 MiB: the GPU carveout.**
  - 0x9b800000 to about 0x9e800000: the MCU R5F vrings and firmware
    (16 MiB), the WKUP/DM R5F (31 MiB), TF-A and OP-TEE.
  - 0xa0000000 and up (about 1.5 GiB): nothing in this board definition.
- **The 64 MiB carveout:**
  - firmware heap backing: 4 MiB main + 192 KiB config;
  - page tables: 2 MiB;
  - **the render pool: the rest, 59200 KiB** (the "render pool summary"
    line). 64 MiB - 4 MiB - 192 KiB - 2 MiB = 59200 KiB.
- **Who takes the pool**, at the default pair's first refusal (knob off,
  cbbc5616, 11:03:25):
  - driver 15344 KiB, of which the freelists are 12440;
  - kicks pending free 11576 KiB;
  - backend and GL objects 30520 KiB.
  - High-water 59184 of 59200 KiB with the knob off, 58668 with it on.
- **23 MiB more (refract's 22 refused grows of 1036 KiB):**
  - It does not fit in today's map: the carveout touches the MCU R5F
    region at 0x9b800000.
  - **It is possible by moving the carveout.** The deqp sample already
    does this: its overlay puts a 256 MiB carveout at 0xa0000000 (dEQP's
    2048x2048 textures needed it, 2026-09-24), and dEQP ran with it on this
    board. A glmark2 / gl_test overlay could do the same at 128 MiB.
  - The other way, taking 23 MiB from `zephyr,sram` (344 MiB), shrinks the
    application's own RAM.
  - **Not shown:**
    - whether 0xa0000000+ is free when booted from U-Boot rather than
      kexec. The deqp overlay's note says "the image is kexec'ed".
    - whether the SafeUI platform (the R5F owner) agrees to the move.
  - Cost: carveout memory is not usable as Zephyr heap.

### The AM62P glmark2 cell: the z-only render on the board, gl_test 177 PASS (82c9612a, 2026-10-04 14:19)

One run from U-Boot, `build/zonly-am62p-20261004-141901/zonly-1/`.
- **Against the prediction written before it:**
  - 177 PASS, 9 of 9: COMPLETE at check 3, no GL error, and the four
    depth points read b8 / 49 / 4b / b6 against b7 / 48 / 4a / b5 (within
    2). The +1 is the half-texel offset the Pi 4 also reads.
  - Resets 0, recoveries 0.
  - **Met.**
- **169, the control, did not run:** its `dsk_*` shaders are not in the
  PowerVR offline table ("offline shaderc: vertex shader not in offline
  table (sha256=c10b216b...)"), and the image has no online compiler. My
  prediction called it a PASS: that was wrong, because I did not check its
  shaders against this image's table. 177's own checks do not depend on it.
- **For the gl_test row:** cases 162 (dskeep), 167 (stcycle), 168
  (stlayers) and 169 (dstex) all link `dsk_vs_src` / `dsk_fs_src`, which
  neither the PowerVR nor the GC7000 offline table has (gc-reader's
  finding on the i.MX8MP, now on both GPUs). They run only with the pair
  added to the tables, or in an image with the in-process compiler. None
  of them is in the 40-case regression list.
- Next: glmark2's shadow scene in the next common round (33/0/0 predicted).

## The AM62P glmark2 cell: its state, as a list to act on (2026-10-04 14:25)

**Measured on main + land189 + land190** (the default configuration):
- glmark2 completes from U-Boot: 470, 32 ran / 1 failed (shadow) /
  0 unsupported, resets 0 (round on 520a6c0f). Round 3 ended in the RTI
  watchdog.
- The lost FWCCB command is gone: the firmware uses the uncached window
  (5 of 20 cached runs against 0 of 20 uncached, gl_test 174).
- Every stop is bounded: a kick that cannot complete ends in a GPU_STOP
  record after 10 s, not the watchdog.
- shadow: the PowerVR draws depth-only (gl_test 177 PASS 9/9). That glmark2
  now runs shadow is the measurement round's to show (6242497b, running).

**Not green: two open items, both the user's decision.**
1. **Dropped submits, the freelist default.** With the default freelists
   (12440 KiB) the render pool fills, and terrain's frames drop their
   submits: 345-372 per run, 7 of 7 runs. The choices:
   - a. grow on demand on (8368 KiB): 0 dropped submits in 41 of 41 runs,
     but 1-23 refused grows per run at refract (risk: item 2);
   - b. a bigger pool: about 23 MiB more, by moving the carveout to
     0xa0000000 as the deqp sample does (not shown from U-Boot);
   - c. both.
   The measurement round on 6242497b (a) is the evidence for a.
2. **A refused grow with no 3D pending (A = 0) cannot complete:** the
   firmware's partial render is not set up (no FENCE_PR / 3D_PR in our
   fragment CCB).
   - Today it ends in the 10 s stop record: the run fails, the board does
     not reset. gl_test 175 reproduces it 3 of 3.
   - In the default configuration it is not exercised (0 refusals). With
     option a, every refusal is a chance of it.
   - The fix is the partial-render design in this document (about 12
     days). Go or no-go is the user's.

**Unexplained, kept open (they do not block the cell):**
- the cached firmware's 5 / 8 ms period before it takes a KCCB post (not
  ActivePMLatency);
- the 240 us late FWCCB slot, and why only at requests 7 and 15;
- gl_test 162, 167, 168, 169 need the `dsk_*` pair in the offline tables
  (both GPUs), or an image with the in-process compiler.

### The AM62P glmark2 cell: the measurement round, grow on demand on (6242497b, 2026-10-04 14:27; a row for decision 1, not a landing)

Tree 6242497b: land190's tip plus the one Kconfig commit, FL_GROW_ON_DEMAND
y (freelists 8368 KiB). Run dir `build/common-am62p-20261004-142711`
(copied to `records/am62p/`), U-Boot, uninstall rc 0, environment diff 0.
The diff against the 520a6c0f round: every row's record and verdict are
identical, and only the image shas differ.

| row | record | verdict, resets | the same as 520a6c0f |
|---|---|---|---|
| GM (8c6c4a67) | 594b1201 ffd701d6 2280103f 03200258: 470, 32 ran / 1 failed (shadow) / 0 unsupported | failed (shadow), resets 0, back 402 s | yes |
| SO, SV | 594b0601 08420882 00210842 98080883 | pass, resets 0 | yes |
| GT | "1 of 40 cases failed (39 passed; 0 timed out; first failure case 48 at index 31)" | resets 0, back 72 s | yes |

**The GM console, against the knob-off round:**
| | knob off (520a6c0f) | knob on (6242497b) |
|---|---|---|
| grow requests, served, refused | 3, 3, 0 | 26, 3, 23 ("the render pool refused 1036 KiB") |
| "render pool exhausted" | 510 | 27 |
| "submit failed" | 372 (terrain) | **0** |
| terrain | 124 frames, 80638 us | 63 frames, 159535 us |
| refract | 134 frames, 74634 us | 24 frames, 431776 us |
| stop records | 0 | 0 |

**Against the prediction:**
- Met: 0 dropped submits, 23 refused grows (predicted 1-23, most likely
  22), resets 0, score in band, SO/SV/GT identical.
- Missed: GM 33/0/0. Shadow failed again, at 0x8cd6 instead of 0x8cdd:
  its depth texture was refused by the pool (mechanism 2, the section
  after this one).
- **What it shows for decision 1, option a:** the knob removes the
  dropped submits with no row regressing. The cost is 23 refused grows per
  run (none hung here), and terrain and refract at their real, slower
  frame times.

### The AM62P glmark2 cell: the measurement round's shadow 0x8cd6 is mechanism 2, and the identical score (6242497b, 2026-10-04 14:27)

**Why shadow read 0x8cd6** (GM console, lines 747-755): it is the pool,
not a GL rule.
- "render pool exhausted: 7500 KiB requested, 844 KiB left". The general
  free lists held 12084 KiB in 8 ranges, the largest 5848 KiB: smaller
  than the texture.
- "pending frees now 38 (23932 KiB), oldest fence 103994, last submitted
  103993; reclaim off": the previous scenes' frees were held for a kick not
  yet submitted. That is mechanism 2 (above), the case
  `CONFIG_YAKOGL_PVR_POOL_RECLAIM` (c4c31346, opt-in, default off) exists
  for.
- "texture 1600x1200 x1 fmt 7 (1 levels, 7680000 bytes): out of GPU
  memory". The depth texture had no storage. check_status's depth arm then
  finds no storage (`attachment_storage`: `t.mem.impl_` null) and returns
  GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, which glmark2 printed.
- Before the z-only change, the same framebuffer stopped earlier at 0x8cdd
  (UNSUPPORTED), so this allocation was never reached.
- Case 177's texture (twice gl_test's 64x64 window) is 64 KiB and was
  allocated with nothing held: it could not see this.
- **Host tests** (src/gl/pool_tests.rs):
  - `known_wrong_shadows_depth_texture_is_refused_beside_held_frees_and_its_framebuffer_is_incomplete`
    reproduces GL_OUT_OF_MEMORY and 0x8cd6 with reclaim off. A previous
    scene's buffer of the texture's size is drawn, swapped (its kick
    completed) and deleted, in a pool of 1.5 times the texture.
  - `with_the_reclaim_shadows_depth_texture_fits_and_its_framebuffer_is_complete`
    is the same set-up with reclaim on: the texture is served and the
    status is COMPLETE.

**Why the score is identical** (470 both, from the glmark2-frame lines):
- 30 of the 32 scenes are within 2 fps of each other.
- terrain: 124 frames at 80638 us (12 fps) with the knob off, against 63
  at 159535 us (6 fps) with it on. With the knob on, terrain's submits are
  no longer dropped, so it does all its work.
- refract: 134 at 74634 us (13 fps), against 24 at 431776 us (2 fps). With
  the knob on, its 23 refused grows each wait out the firmware's
  out-of-memory handling.
- These two scenes lose 17 fps of the 32-scene sum (15078 against 15062
  from the frame lines). The mean moves from 471.19 to 470.69: within one
  unit, so the record reads 470 both times.
- So the score hides a real change: the knob-off score counts terrain
  frames that were never rendered.

**What this changes in the cell's decision 1:**
- The reclaim is the existing fix for mechanism 2. Shadow's set-up needs
  it whatever the freelist default is.
- One earlier run with reclaim on and the knob off (frames-am62p bcde4b51,
  01:29, freelists 12440) had 110 "render pool exhausted" and **0
  "submit failed"**. That was on an older tree, but it suggests terrain's
  dropped submits are mechanism 2 too, not only the freelist size.
- **The next measurement:** reclaim on, the knob off, on the current tree.
  If it shows 0 dropped submits and shadow COMPLETE, decision 1 is "reclaim
  on" (no refused grows), and the knob can stay off.

### The AM62P glmark2 cell: gl_test 178 heldfree and the reclaim default, images and predictions (written before the runs)

Branch `pvr/glmark2-cell-state` (on land190's tip 0759bafc):
- 2beb8b56: gl_test 178 heldfree. The pool is filled with 1 MiB buffers
  until GL_OUT_OF_MEMORY, then glFinish, then everything is deleted with no
  frame open. Then case 177's steps run at 1600x1200.
- 8e7257f8: `CONFIG_YAKOGL_PVR_POOL_RECLAIM` defaults to y.
- Both images have the grow knob off and FW_UNCACHED on. They run cases
  177 and 178.

| image | sha16 | reclaim |
|---|---|---|
| `build/hf-off-am62p` | a9427b05f1dd2108 | n (the old hold) |
| `build/hf-on-am62p` | 5cd3d9610f87ef49 | y (the new default) |

**Predictions:**
- **Both images, case 177:** PASS 9/9, as on 82c9612a.
- **hf-off, case 178: FAIL at check 3** ("heldfree: error after setup" =
  0x00000505). The status line then reads 0x8cd6, the console has
  "texture 1600x1200 x1 fmt 7 ... out of GPU memory", and the refusal line
  reads "pending frees now N (about the fill) ... reclaim off".
  - Check 2 (filled MiB >= 8) passes. The fill should be about 30 MiB: the
    pool's 59200 KiB less what gl_test holds.
  - This is the board reproduction of glmark2's shadow failure.
- **hf-on, case 178: PASS, 10 checks.**
  - The texture is served. Either no refusal line, or "reclaim without
    waiting: gave back ... the retry was served".
  - The four depth points match 177's within 2. At 1600 texels the
    half-texel offset is under one unit, so they may read the exact value.
- Resets 0 in both. If hf-off's 178 passes, the fill did not leave the
  texture short. That would be an instrument defect, read from the
  refusal line before any verdict.
- **Then:** the AM62P common round on 8e7257f8 (reclaim on, knob off). Its
  prediction:
  - GM: 33 ran / 0 failed / 0 unsupported, and 0 "submit failed" if
    terrain's dropped submits are mechanism 2 too (one earlier run,
    bcde4b51, says so), with 0 refused grows (the knob is off);
  - GT: 39 of 40, first failure 48.

### The AM62P glmark2 cell: gl_test 178 heldfree on the board, the reclaim off and on (2026-10-04 14:47)

Run dir `build/hf-am62p-20261004-144723/{hf-off,hf-on}` (copied to
`records/am62p/`), U-Boot, uninstall rc 0, environment diff 0. Tree
8e7257f8, cases 177 and 178.

| | hf-off (a9427b05, reclaim n) | hf-on (5cd3d961, reclaim y) |
|---|---|---|
| 177 zonly | PASS | PASS |
| 178 fill | "35 MiB allocated, then deleted with no frame open" | the same |
| the texture | "pending frees now 43 (36040 KiB), oldest fence 3, last submitted 2; reclaim off", "texture 1600x1200 x1 fmt 7 ... out of GPU memory" | served, no refusal line |
| 178 checks | check 3 "error after setup" 0x505 MISMATCH, "status 0x8cd6": **FAIL** | 10 of 10, status 0x8cd5, depth b7 / 48 / 4a / b5 = want exactly: **PASS** |
| resets | 0 | 0 |

- **Both predictions met word for word.** Shadow's refusal is reproduced
  on the board with the old hold, and removed with the reclaim.
- At 1600x1200 the depth points read the exact ramp values. The +1 of
  case 177 is its half-texel at 128x128, as said.
- In hf-on, the one "reclaim ... refused" line is the fill's own end (its
  last 1 MiB buffer, with nothing held), which is what the fill waits for.
- In hf-off, the 36040 KiB were still held at close ("at close: pending
  frees now 43"): no later kick released them.

### The AM62P glmark2 cell: the reclaim round (8e7257f8: reclaim on, the knob off; 2026-10-04 14:55) — AMBER

Run dir `build/common-am62p-20261004-145517` (copied to `records/am62p/`),
U-Boot, uninstall rc 0, environment diff 0. The diff against the 520a6c0f
round:

| row | 520a6c0f | 8e7257f8 |
|---|---|---|
| GM | 594b1201 ffd701d6 2280103f 03200258: 470, 32 / 1 (shadow) / 0, failed | **594b1201 fc3f01c9 2184003f 03200258: 457, 33 / 0 / 0, pass**, resets 0, back 390 s |
| SO, SV | 594b0601 08420882 00210842 98080883 | identical, resets 0 |
| GT | 39 of 40, first failure 48 | identical, resets 0, back 66 s |

**GM console:**
- "submit failed" 0 (372 before), "no grow" 0, 221 reclaim lines. Pool
  high-water 57260 of 59200 KiB.
- terrain does its full work (51 frames at 198168 us), and shadow runs (27
  frames at 382892 us).
- **AMBER:** refract's depth-pass draw is refused in every frame, 110 of
  110: "render pool exhausted: 7600 KiB requested, 1940 KiB left", "draw
  refused: no depth/stencil buffer for a 1600x1200 target", "glDrawArrays
  was not recorded (-12); the frame is missing it". The cause is
  fragmentation, not a hold: "free 9712 KiB in 8 ranges, largest 3936
  KiB", "pending frees now 0", "reclaim found nothing held". 7600 KiB is
  needed; the largest free range is 3936 KiB. The 520a6c0f round refused
  the same draw 134 times.
- **The score, 457, missed my 470-530.** shadow enters the mean as a 33rd
  scene at 2-3 fps, and terrain does its full work (12 to 5 fps). From the
  frame lines the mean is 457.97.
- What remains: refract (fragmentation). The options are the knob (round
  5af1f031, running) or the scattered-pages design (below).

## The AM62P glmark2 cell: design, large buffers from scattered pages (the root fix for refract's depth; no code)

**The problem, measured** (reclaim round 8e7257f8, 2026-10-04 14:55):
- refract's depth/stencil buffer for its 1600x1200 depth target needs
  7600 KiB in one piece.
- The pool had about 11.6 MiB free: "free 9712 KiB in 8 ranges, largest
  3936 KiB", plus 1940 KiB above the bump. Nothing was held.
- So 110 of 110 refract frames lost their depth-pass draw ("glDrawArrays
  was not recorded (-12); the frame is missing it").
- The free memory is there; it is not contiguous.

**Why a block must be contiguous today** (gpu::pvr::mem `bo_alloc`,
gpu::pvr::pool):
- A block is one physical range of the carveout (the shared cursor
  `pool_next`), mapped at one device-VA range of its heap (the heap's
  cursor), with one CPU pointer into the carveout's linear CPU mapping.
- The free lists keep (VA offset, size, CPU pointer) and merge only
  neighbours contiguous in both VA and CPU space.
- Nothing is ever unmapped.
- The GPU does not need physical contiguity: it reads through its MMU, page
  by page. Only our bookkeeping, and the CPU's view of the block, do.

**The change:**
1. **A scattered allocation in the pool** (`Pool::alloc_scattered`, Rust):
   when `alloc` finds no single range and the bump cannot serve it, and
   the request is at least `SCATTER_MIN` (1 MiB), take free ranges
   (largest first) until their sum covers the request.
   - The pieces are recorded in a small piece table (at most
     `SCATTER_MAX_PIECES`, 16, per buffer; a fixed array per pool, no
     allocation).
   - Too many pieces needed: refuse as today, with its own line.
2. **A fresh device-VA range** for the buffer: from the heap's VA cursor
   (the GENERAL heap's window is 1 GiB; the carveout is 64 MiB). Each
   piece's pages are mapped there in order with `mmu::map`, so the GPU sees
   one contiguous buffer.
   - The pieces' old VA mappings stay, as every mapping does today. They
     alias freed memory nobody uses.
   - The new VA range must be covered by the 2 MiB page-table area. Every
     2 MiB of fresh VA costs one 4 KiB L0 table. So the scattered ranges
     reuse their VA: freed scattered VA ranges go on their own VA free
     list, and a new scattered buffer takes VA from there first.
3. **The MMU's caches:** a VA range mapped to new pages must be
   invalidated before the next kick reads it. The render service already
   posts an MMU cache invalidate before every kick (`mmu_cache_sync`,
   pvr_render.c), so a remap is covered when it happens between kicks.
   A remap inside an open frame is refused (the frame's kick would race it).
4. **Free:** a scattered buffer gives each piece back to the free lists at
   its own old VA and CPU pointer, as if freed separately, and its VA range
   to the scattered VA list.
5. **Which buffers:** first only the backend's depth/stencil buffers
   (`ds_get`, src/pvr/backend.rs). They are GPU-only, except two CPU
   readers: `pvr_backend_depth_resolve` (a depth texture's texels) and the
   DS_DUMP instrument.
   - Both read through a piece-aware reader: offset to (piece, offset in
     piece).
   - Textures and buffers keep contiguous blocks: their CPU uploads and
     maps write through one CPU pointer. They come second, only if a
     measurement names one.

**The risks:**
- **A wrong page in the map is a GPU fault or a silent corruption.**
  Guard: a host test maps a scattered buffer through the mock MMU and
  checks every page's translation against the piece table. The board case
  below reads back every pixel's depth, not a few points.
- **Page-table exhaustion:** fresh VA without reuse. Guard: the VA free
  list and a high-water line for the PT area, printed every run (the arena
  rule).
- **Aliasing:** a freed piece's old VA still maps its pages. A stale GPU
  read through the old VA reads the new owner's data. That is the same
  exposure as any reused block today; the pending-free fences keep it
  covered.
- **The CPU readers:** a reader that assumes one pointer reads the wrong
  memory. Guard: the ds buffer's CPU pointer is replaced by the piece
  table, so a contiguous read does not compile.

**The proof case:** gl_test 182 "scatter".
- Fragment the pool: allocate 1 MiB buffers until GL_OUT_OF_MEMORY, then
  delete every other one with no frame open (reclaim on). The largest free
  range is then 1 MiB, and about half the fill is free.
- Then case 177's depth-only pass at 1600x1200 (7600 KiB of depth buffer),
  with the depth read back at every pixel of the window and compared with
  the ramp.
- Prediction without the change: the draw is refused, "no depth/stencil
  buffer ... (1600x1200)", FAIL. With it: PASS, and one "scattered: N
  pieces" line.
- Then the AM62P common round: refract's "draw refused" goes from 110 to 0,
  with the grow knob off.

**Days** (one agent, the board when needed):
- the pool's scattered allocation, piece table, VA free list, host tests
  against the golden trace's rules: 2;
- the backend's ds buffer on pieces, the two CPU readers, host tests: 1;
- gl_test 182, the board pair, the common round: 1;
- review and the PT-area high-water line: 0.5.
- **About 4.5 days.**

**The choice for the user (decision 1, both options now measured or
designed):**
- **Knob on** (measured, 6242497b): refract fits today. Cost: about 23
  refused grows per run at refract, each a chance of the A = 0 hang (the
  partial-render gap) and a 10 s stop record. No code.
- **This fix** (designed): refract fits with the knob off. No refused
  grows. About 4.5 days. A mapping defect would be a GPU fault, guarded by
  the tests above.
- Measured next, already queued: reclaim + knob on (5af1f031), the
  mitigation's own row.

### The AM62P glmark2 cell: reclaim + grow on demand on (5af1f031, 2026-10-04 15:15; a row for decision 1, not a landing)

Tree 5af1f031 = 9921f676 (reclaim on) + the Kconfig-only knob commit.
U-Boot, run dir `build/common-am62p-20261004-151541` (copied to
`records/am62p/`), uninstall rc 0, environment diff 0. The GM image sha
(a651ffa1...) equals the one built independently in the pvr-flgrow
worktree from the same tree: the build reproduces.

**Against the reclaim round (8e7257f8):** every row's record and verdict
is identical, and only the image shas differ.
- GM 594b1201 fc3f01c9 2184003f 03200258: 457, 33 / 0 / 0, resets 0,
  back 392 s.
- SO and SV: identical.
- GT: 39 of 40, first failure 48, back 63 s.

**GM console:**
- The knob was in effect: 3 grow requests, 3 served, 3108 KiB taken on
  demand, 0 refused grows. ("about 23" was predicted; with the pool less
  full, every grow was served.)
- Pool high-water: 53532 KiB at refract (57260 in the reclaim round),
  54568 at close.
- Refract's depth-pass draw is **still refused in every frame**, 110 of
  110:
  - "render pool exhausted: 7600 KiB requested, 5668 KiB left";
  - "free 10056 KiB in 9 ranges, largest 3792 KiB";
  - pending frees 0.
- The knob's freelist saving shows as 3728 KiB more above the bump (1940
  to 5668). That is still 1932 KiB short. The other 344 KiB of the 4072
  is not accounted for here.
- **Missed prediction:** 0 refused draws. The prediction assumed the
  knob-on round of 6242497b, where shadow had failed and never allocated.
  Here shadow runs: "ds bytes peak 9728000" = its 1600x1216 depth buffer
  plus the window's.
- **The order, inferred and not shown** (no owner lines in this image):
  shadow frees its 7500 KiB texture and 7600 KiB depth buffer; refract's
  two 1600x1200 textures take those blocks by best fit; refract's own
  7600 KiB depth buffer then finds no range.

**What this settles for decision 1:**
- No measured configuration fits refract's 7600 KiB once shadow runs:
  knob off leaves 1940 KiB at the bump top, knob on 5668.
- About 15.7 MiB is free in total.
- So the only path on the table is the scattered-pages design (the section
  "design, large buffers from scattered pages" above):
  start the 4.5-day fix, yes or no. The knob is not needed for this cell.

### The AM62P glmark2 cell: a third amber item, terrain's picture (2026-10-04, the lead's ruling)

- **Terrain's picture is wrong:** NPOT mipmaps are refused in ES 2.0.
  glmark2's 800x600 render targets stay mip-incomplete, so terrain samples
  black: "a texture on unit 0 is incomplete and samples (0,0,0,1) ...
  800x600, levels 1 of 1". **The fix is in work by pvr-fence:**
  GL_OES_texture_npot in the GL core, for every GPU. `src/gl/texture.rs`
  is theirs meanwhile.
- "GL error 0x502 in glGenerateMipmap" per round, counted from each GM
  console:

  | round | 0x502 lines |
  |---|---|
  | 520a6c0f (knob off, reclaim off) | 6 |
  | 6242497b (knob on, reclaim off) | 6 |
  | 8e7257f8 (reclaim on) | **16** |
  | 5af1f031 (reclaim + knob on) | **16** |

  - The 6 against 16 follows the reclaim: with reclaim on, terrain's
    set-up gets its memory and reaches more glGenerateMipmap calls. That
    is read from the counts, not traced call by call.
  - The scene a line belongs to is not printed on the line itself.
- This item is closed by a GM round showing 0 such lines.

## The glmark2 row: NPOT mip chains on the PowerVR, stage 2 (design; code after pvr-fence's stage 1 lands)

**Where it comes from:**
- glmark2's terrain makes mipmaps of its 800x600 render targets, and the ES
  2.0 surface refuses them ("GL error 0x502 in glGenerateMipmap", 16 a
  round with reclaim on). Terrain samples black.
- No GPU stores an NPOT chain today: `chain_layout` (src/gl/texture.rs)
  makes a chain only when both sides are powers of two (`multi`).
- **Stage 1 (pvr-fence):** NPOT chains and generation in the GL core,
  behind a backend capability `NPOT_MIPMAPS`; the V3D's and the GC7000's
  samplers.
- **Stage 2 (this):** the PowerVR's layout for an NPOT chain, and its
  capability.

**What the PowerVR's texture unit needs** (Mesa 26.2.1 pvr, MIT):
- `pvr_image_plane_init_physical_extent`: with more than one level, the
  physical extent is the extent rounded up to a power of two, on each axis
  (`util_next_power_of_two`).
- `pvr_image_plane_setup_mip_levels`: level l has pitch `cpp * (POT(w) >>
  l)` and `POT(h) >> l` rows. The levels follow each other in that order,
  and the layer is sized as if the chain went to 1x1.
- The image word keeps the API extent (800x600). The unit is handed level
  0's address and the stride, and finds every later level by that
  arithmetic.
- So an 800x600 chain is laid out as a 1024x1024 chain. Level l's NPOT
  image (GL's floor sizes: 400x300, 200x150, ...) sits in the top-left of
  its power-of-two block.
- This is the rule `chain_layout` already applies to a stacked cube's
  height (`pot_stack`, the C-era fix measured 2026-09-22). Stage 2 applies
  it to both axes of a 2D NPOT chain.
- native_cube.rs (the twiddled copy for cubes) is the same arithmetic over
  a derived copy. Here the texture's own storage is laid out this way, with
  no copy.

**The change** (after stage 1, on a branch from it):
1. `chain_layout`, the PowerVR arm (`pot_stack` true, `lvl_align` 0): an
   NPOT chain with `NPOT_MIPMAPS` gets per-level pitch `POT(w) >> l` and
   rows `POT(h) >> l` (each at least 1). A one-level NPOT texture stays as
   today, packed at `tex_pitch(w)`, as Mesa's one-level image is not
   rounded.
2. `GlTexture::level_pitch` returns that pitch for such a chain: a
   physical-width field set by `chain_layout`, not `tex_pitch(lw)`. Every
   reader and writer already goes through `level_pitch` and `level_off`:
   uploads, glGenerateMipmap, render-to-level, read-back and depth resolve.
3. **Rendering into level 0** (terrain's path): the target's pitch is
   `POT(w)` (1024 for 800). The backend already renders into a view whose
   pitch is wider than its width (`draw_pitch`, the stride view), so the
   PBE writes rows 1024 texels apart.
4. **The descriptor** (src/pvr/backend.rs `build_tex_desc`): the STRIDE
   image word's stride is `POT(w)`, and the level count is the chain's.
   The image word keeps 800x600.
5. **The capability:** `pvr::backend::NPOT_MIPMAPS = true`, declared with
   this layout. It is pinned by a compile-time check, as
   `DRAWS_WITHOUT_COLOUR` is.

**Host tests:**
- the level offsets and pitches for 800x600, 100x60 and 3x5 equal Mesa's
  arithmetic, computed independently in the test;
- a POT chain's layout is unchanged (the known negative);
- an upload to level 2 of a 100x60 chain lands at the offset the descriptor
  implies;
- the render target of an 800x600 level 0 has pitch 1024.

**The proof: gl_test 181 npotmip, on the AM62P** (prediction written before
the run):
- **A:** a 100x60 RGBA8 chain, every level uploaded with its own solid
  colour (100x60, 50x30, 25x15, 12x7, 6x3, 3x1, 1x1), NEAREST_MIPMAP_NEAREST.
  - Each level is sampled by a fragment-shader bias that selects it.
  - The whole window is read back per level: one colour per level, no
    other.
- **B (terrain's path):** an 800x600 RGBA8 level 0 rendered through a
  framebuffer with a two-colour split, then glGenerateMipmap. Levels 0, 3
  and 6 are sampled and read back: the split lands where the level's floor
  size puts it, within one texel of the box filter.
- Before stage 2 (stage 1 only, the PowerVR capability false): the case
  reads the base level for every bias and FAILs by name at level 1.

**The memory cost in the pool**, per 800x600 RGBA8 target with a chain:
- today: one level, 800 x 600 x 4 = 1875 KiB;
- with the chain: a 1024 x 1024 chain, 4 MiB x 4/3 = **about 5461 KiB**,
  about 3586 KiB more per target (x2.9).
- The terrain scene's own count of such targets is read from stage 1's
  round before this is built: one "texture ... (N levels, bytes)" line per
  target.
- **With today's pool this may not fit.** The reclaim round's high-water was
  57260 of 59200 KiB, and refract is already short for fragmentation. So
  this lands either with the scatter fix (decision 1, which can serve
  textures after depth buffers) or with a measured count that fits.

**Days:**
- layout and `level_pitch`, with host tests: 1;
- the descriptor and the render-target pitch: 0.5;
- gl_test 181, the board run, a GM round: 1;
- **about 2.5 days**, after stage 1.

## The top region: large GENERAL blocks from the carveout's end (refract after shadow and terrain; host evidence, not yet on the board)

**The problem, measured** (the reclaim round 8e7257f8, and 5af1f031 with
grow on demand; tools/cts/RESULTS.md, "glmark2 cell AMBER"):
- refract's depth pass needs a 1600x1216 depth/stencil buffer, 7600 KiB in
  one piece; it was refused in 110 of 110 frames ("glDrawArrays was not
  recorded (-12)");
- "free 9712 KiB in 8 ranges, largest 3936 KiB", "1940 KiB left" above the
  bump, nothing held ("pending frees now 0");
- with grow on demand on: "free 10056 KiB in 9 ranges, largest 3792 KiB",
  "5668 KiB left": 1932 KiB short.

The bytes were there in both; one range was not. Scattering a buffer over
pieces through the GPU MMU (the design section above, about 4.5 days) is
out of scope for the benchmark plan. This is the smaller change.

### The policy (`gpu::pvr::pool`, `gpu::pvr::mem`)

- **Two ends.** A GENERAL request of more than `pool_top_above` bytes
  (`struct yd_pvr_km`; the driver sets `YD_PVR_POOL_TOP_ABOVE`, 256 KiB,
  in `pvr_fw_boot.c` `km_setup`) is a large one. It is placed from the
  carveout's end downward (`mem.rs` `bo_alloc_top`). Everything else keeps
  coming from the bump, as before: the freelists, the render statics, the
  HWRT sets (all of them `bo_alloc` directly), programs, uniforms, the PDS
  and USC heaps. The bump stops at the top region's floor (`bo_alloc`'s
  limit is `pool_floor()`, not the carveout's end), so the two ends share
  one gap.
- **The order** (`pool.rs` `alloc`):
  - large: best fit over the top region's free ranges, then over the ranges
    below, then the gap (the floor moves down), then the bump's refusal
    (its "render pool exhausted" line, "N KiB left" = the gap, and the
    summary);
  - small: best fit over the ranges below, then the bump, then a range of
    the top region (only when the gap is too small).
- **Merging.** A top block at carveout offset `co` is mapped at GENERAL heap
  offset `TOP_VA + co` (`TOP_VA` = 0x4000_0000, above every bump offset and
  every placed span). VA adjacency is carveout adjacency, so the existing
  merge rule (adjacent in VA and in CPU space) holds for every pair of top
  blocks. A page's mapping never changes: only pages below the region's
  high-water are mapped (`pool_top_hw`), so reuse maps nothing and leaves
  the MMU caches alone.
- **Giving back.** A free that merges into a range reaching the floor moves
  the floor up at once (`bo_top_rollback`, always, not opt-in like the
  bump's rollback). So the top region never has a free range at its floor:
  a scene's large blocks freed at teardown go back into the gap in one
  piece.
- **What the run prints.** The summary's head line gains "; top T KiB now,
  H KiB high-water" once the region has been used. The owner report's
  "handed out" (`yd_pvr_pool_usage`) counts the top's bytes.
- **Off.** `pool_top_above` 0 is the old pool exactly. A build with
  `-DPVR_POOL_TOP_ABOVE=0` (`pvr_fw_boot.c`) is the A/B arm.

**Cost and limits:**
- `struct yd_pvr_km` grows by 16 bytes (`pool_top_above`, `pool_top`,
  `pool_top_hw`, `pad`), at its end; `include/yakogl_drivers.h` and the
  Rust layout asserts move together.
- Page-table pages: the top region's VA span equals its physical span, so
  at most one 4 KiB table per 2 MiB of the carveout (32 for 64 MiB), plus
  one directory for the VA above 1 GiB. The 2 MiB page-table area has 512.
- Device addresses up to 0x80_7fff_ffff in GENERAL: inside the heap
  (128 GiB, `rgxheapconfig.h`) and the 40-bit device address space. That
  every field the backend writes takes such an address is the board run's
  to show (a fault would name the address).
- Large blocks of the PDS and USC heaps are not moved (their windows are
  small, and their blocks are code).

### Why 256 KiB, and the order

The replay below decided both. At a 1 MiB threshold jellyfish's 32 caustic
textures (344 KiB each, 11 MiB) stay below; they raise the bump for good,
and refract's last block met a gap of 6596 KiB at the board's bytes. At
256 KiB they go to the top and come back at jellyfish's teardown: the gap
at refract's depth buffer is 11276 KiB. And with "the ranges below" tried
before the gap, the C pool's golden trace replays to equality with the
region on (its two GENERAL requests above 256 KiB: a refused 32 MiB, and
1 MiB from a merged range). Tried the other way (the gap first), the
glmark2 replay gave the same verdicts at 256 KiB in the variants tried (up
to 8.5 MiB unnamed, below), and the golden trace would differ at its last
1 MiB request. At 1 MiB, ranges below first was the weaker order: a large
block in a hole below, once freed, is split by small ones.

### The host replay (`pool/tests/replay.rs`)

The real pool and bump on the AM62P's layout (64 MiB carveout, the pool
its last 59200 KiB). The board prints no per-allocation trace, so the
sequence is a reconstruction; the module's header says where each number
comes from (the owner report's fixed part, jellyfish's 45 frees of 11576
KiB, terrain's refused requests, shadow's and refract's 1600x1200 blocks,
HWRT members by `render.rs`'s formulas, glmark2's default order). Frees
come back at teardown, as with the reclaim (the board: "pending frees now
0"). "Unnamed" is a number of 256 KiB blocks kept from shadow on: what the
board held that the reconstruction does not name.

| GENERAL (KiB) | top region off | on (256 KiB) |
|---|---|---|
| before refract, 0 unnamed: largest range, gap | 15100, 1784 | 684, **31908** |
| at refract's depth buffer, 24 unnamed: free, gap | **9728, 1784** (board: 9712, 1940) | 236, 11276 |
| refract's depth buffer served, unnamed 0..30 | yes | yes |
| unnamed 31..38 (7.75..9.5 MiB) | **refused** ("largest" 7248 down to 5452) | yes (gap 9484 down to 7692) |
| unnamed 39 | refused | refused (gap 7436: capacity) |

- Without the region, no range before refract holds its three blocks
  (7500 + 7500 + 7600 = 22600 KiB); the largest is shadow's two freed
  blocks merged (15100). Refract's textures take it, and its depth buffer
  lives on what terrain's teardown happened to leave.
- At 24 unnamed blocks the reconstruction's bytes are the board's (free
  9728 against 9712, left 1784 against 1940), but its largest range is 7600
  where the board's was 3936: the board's free space was in smaller pieces
  than the reconstruction can say. From 31 unnamed blocks on, the
  reconstruction refuses the depth buffer as the board did, with the bytes
  free in pieces, and nothing else.
- With the region, before refract one range (the gap) holds all three of
  its blocks for every unnamed count up to 38, and nothing in the run is
  refused. At 39 the gap is short of refract's blocks (22436 KiB): that is
  capacity, not placement.
- **Slint (800x480) and the AGL cluster (1280x720)**, the boot plus the
  application's objects, a glyph atlas that grows once, and 200 frames of
  small buffers: every allocation served in both arms, and the largest
  range left is the same (the gap: 27360 and 12696 KiB).
- **The frame sets** (4 x (1 MiB GENERAL + 768 KiB PDS)): the four GENERAL
  arenas are the top region's first blocks, at the carveout's end one after
  another; the PDS arenas and the tables come from the bump as before.
- **The golden trace** (2769 operations of the C pool): equal with the
  region off and with it on at 256 KiB; the region is never used by it.
- The top region's own tests (`pool/tests.rs`): placement and mapping,
  merging and giving back in any order, reuse before the floor moves, the
  bump stopping at the floor and a small request falling back to a top
  range, frees outside the region dropped and counted, and a random run of
  8000 steps with conservation per heap (1759 blocks placed in the top).

**Not shown on the host:** the board's own placement. The replay says the
gap before refract is at least 22600 KiB while the blocks the board holds
beyond the reconstruction stay under 9.5 MiB; the board run says whether
they do.

#### The replay with the NPOT mip chains (pvr-pool-npot, 2026-10-17)

The table above sizes terrain's four 800x600 targets at level 0 (1875 KiB
each), as the board measured them before the PowerVR stored NPOT chains.
Since pvr-npot-mips a mipmapped NPOT texture holds the chain of its
power-of-two extent (src/gl/texture.rs `pot_chain_layout`, Mesa pvr's
rule), and glmark2 mipmaps two 800x600 targets of terrain
(`terrain_renderer`, `tilt_h_renderer`: `setup_texture(GL_LINEAR_MIPMAP_
LINEAR, ...)` at glmark2's default options) and refract's 1600x1200 colour
texture (`glGenerateMipmap` after every frame's depth pass). What the GL
core asks the pool for each, in its order, is pinned by
`tests/pvr_pool_npot.rs` on a backend with the PowerVR's layout flags:

| texture | `glTexImage2D` | first `glGenerateMipmap` |
|---|---|---|
| 800x600 RGBA8, mipmapped (terrain, 2 of them) | level 0, 1,920,000 bytes (1875 KiB) | level 0 freed, then the 1024x1024 chain: 5,592,404 bytes (**5462 KiB**, 2.9x) |
| 1600x1200 RGBA8, mipmapped (refract's colour texture) | level 0, 7,680,000 (7500 KiB) | level 0 freed, then the 2048x2048 chain: 22,369,620 bytes (**21846 KiB**, 2.9x) |
| 1600x1200 depth (refract, shadow; NEAREST) | one level, 7500 KiB | never |
| 512x512 / 256x256 RGBA8, mipmapped | the whole chain at once: 1,398,100 / 349,524 bytes | nothing more |

`Regrow` in `pool/tests/replay.rs` is that order (the old storage is
released before the chain is asked for, `texture_alloc_laid_out`; the
texture was settled first, so the reclaim drains the held level 0 without
a wait). The replay sweeps terrain's chains from none to four and adds
refract's chain at the end of its first frame, on the board's 64 MiB
carveout and on the glmark2 image's, read from its overlay (256 MiB,
504d4fae). The freelists are sized by the carveout as `pvr_render.c`
sizes them (from 128 MiB a quarter of the carveout is the grow pool, and
the global lists are 2048 pages), and each scene's live high-water is
kept as the board's `osal_mem_line` counts live bytes. The findings this
follows, and the board records it is checked against, are the
baseline3-am62p investigation
(docs/notes/am62p-render-pool-glmark2.md).

**On the 64 MiB carveout** (the pool its last 59200 KiB); the lists are the
same alone and with the board's 24 unnamed blocks:

| terrain's chains | top region on: refused | off: refused |
|---|---|---|
| 0 | nothing | nothing |
| 1 | terrain's 800x608 depth buffer (1900) | the same |
| 2 (the source) | two 800x600 targets (1875 each), the depth buffer | one chain (5462) |
| 3 | a target, a chain, a target, the depth buffer | two chains, **refract's 1600x1216 depth buffer (7600)** |
| 4 | a target, a chain, a target, a chain, the depth buffer | three chains, **refract's depth buffer** |

- **Refract's three blocks still fit, with the region.** Terrain's blocks
  all come back at its teardown, so GENERAL before refract is the same as
  with level-0 targets (gap 31908 KiB alone, 26276 at the board's bytes,
  holding refract's 22600), and refract's depth buffer is served at every
  chain count. Without the region, from three chains on the old
  fragmentation is back and refract's depth buffer is refused again. The
  threshold stays 256 KiB; the placement is unchanged.
- **Terrain's own set-up no longer fits 59200 KiB**, from one chain on:
  3587 KiB more per chain on top of its 30 MiB. Both arms refuse about the
  same bytes of it (which blocks differs with the order the chains are
  asked in), so this is capacity, not placement.
- **Refract's own chain is beyond the 64 MiB carveout's capacity.** At the
  chain, GENERAL has 748 KiB free and a gap of 9308 KiB with the region
  (9984 free and 72 KiB with it off): 10056 KiB in either arm for a 21846
  KiB request. Refract needs 7500 + 21846 + 7600 = 36946 KiB while about
  26 MiB are held before it whatever the arm. `glGenerateMipmap` then
  raises GL_OUT_OF_MEMORY, and as level 0 was released first the texture
  has no storage at all: refract's framebuffer is incomplete (0x8cd6)
  every frame, cd738b1a's 1694 `framebuffer 1 incomplete` lines.
- **On the glmark2 image's 256 MiB carveout** (pool 255808 KiB, the
  persistent set about 95 MiB with the 64 MiB grow pool) every block of
  the run is served in both arms, with four chains, refract's chain and
  0, 24, 30 or 38 unnamed blocks (gap before refract 158756 KiB with the
  region, alone), and every scene's live high-water stays under 3/4 of the
  pool: jellyfish 106560 KiB, terrain 140768 (four chains), refract 139356
  at the board's bytes, 142940 at 38 unnamed. Against the e3749a25 board
  run the replay's end bump is 90544 KiB (board 91356) and refract's peak
  139356 (board 139212). This is the image the AM62P rows have run since
  504d4fae (`1d54a4eb`: 33 scenes, 0 failed, 0 GL errors, refract's
  11-level chain filtered on the CPU each frame, 2.08 s). The test reads
  the carveout from the overlay, so shrinking it below what glmark2 needs
  fails on the host.
- The recorded 2769-operation golden trace is unchanged; the pool test
  count is 32 (`tools/check.sh`, "pvr render pool").

### The other candidates

- **Releasing scene resources at teardown:** already prompt. With the
  reclaim on (the default since 8e7257f8) the board printed "pending frees
  now 0" and "reclaim found nothing held" at refract; nothing to release.
- **A bigger pool:** the carveout is the board's reserved-memory region,
  64 MiB at 0x97800000 (SafeUI `boards/toradex/verdin_am62p/
  verdin_am62p_am62p54_a53.dts:75`), ending where the MCU R5F's region
  starts (0x9b800000). There is no Kconfig for its size: the driver takes
  the memory-region's (`pvr_carveout_size`, at least
  `PVR_GPU_MEM_MIN_SIZE`, 64 MiB, `driver/src/pvr_internal.h`). The deqp
  sample moves it to 256 MiB at 0xa0000000 by an overlay
  (`driver/samples/zephyr/deqp/boards/verdin_am62p_am62p54_a53.overlay`),
  but its note relies on the image being kexec'ed; whether U-Boot (the
  route of every glmark2 round since 10-04) leaves that DDR free is not
  shown. Kept as the fallback if the board run below misses. (Taken since:
  the glmark2 image carries the same 256 MiB overlay, 504d4fae, and the
  NPOT chains need it, "The replay with the NPOT mip chains" above.)

### The board expectation (AM62P, glmark2 image from this commit)

- refract: no "draw refused" line and no "render pool exhausted" line in
  refract's part of the console; its frames are drawn whole;
- "glmark2: 33 ran / 0 failed / 0 unsupported", GPU resets 0;
- the summary at close has "; top T KiB now, H KiB high-water", with H at
  least 22600 + 4096 (refract's blocks and the frame arenas);
- the verdict is no longer amber for refract. (Terrain's black picture is
  the npot item's, not this one's.)
- If refract is still refused: the refusal's summary says which. "top 0"
  in it means the image did not carry `pool_top_above`; a gap short of
  7600 KiB with little free below means capacity (then the bigger pool
  above); free ranges in the top region with a short gap mean a long-lived
  large block in the middle, named by the owner lines.
- The A/B arm is the same tree built with `-DPVR_POOL_TOP_ABOVE=0`: it must
  reproduce the 110 refused draws.
