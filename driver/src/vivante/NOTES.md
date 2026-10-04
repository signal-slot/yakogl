# The Vivante GC7000UL driver: what is written, where each number came from,
# and what only the hardware can answer

Status 2026-09-22: milestones 1 to 5 of `docs/imx8mp-backend-plan.md` **run on
silicon**, both ways this code is built. As Zephyr samples on the board
(`gc_identify`, `gc_fence`, `gc_clear`, `gc_triangle`, `gc_gl_triangle`, all
PASS, 2026-09-20), and compiled a second time as an out-of-tree Linux module
(`tools/gclab/`) where `gl_triangle_user` draws through `/dev/gcgpu` and
passes by CPU read-back (2026-09-22). Where a claim below is still a
prediction it says so; the measured ones now say what was measured.

2026-10-03: the kernel side (gc_bringup.c, gc_hw.c, gc_mmu.c, gc_cmdbuf.c) is
Rust now and the C is deleted: `gpu::vivante` in driver/rust, rewritten from
permissive sources only -- rnndb, NXP's galcore (MIT) and the reference
manual -- and run on the board with records byte-identical to the C's
(`driver/docs/vivante-kernel-rust.md`). The out-of-tree Linux module
(`tools/gclab/`) was built from that C and is retired with it. The etnaviv
references and the C file names below describe how the C was written and
what it measured; the Rust does not rely on them.

## The target

Vivante GC7000UL on the NXP i.MX8MP, model `0x7000` revision `0x6204`
(etnaviv's `gc7000UL_6200` hardware-database entry). HALTI5, MMU v2, secure
AHB, one pixel pipe, two shader cores, 512 threads, 512 instruction slots, 320
constant registers, 16 varyings, 16 vertex streams.

## Sources, and what "no register value without a reference" means here

There is no public programming manual for this GPU. Every value in this
directory comes from one of three places, named in the comment next to it:

| short name in comments | what it is |
| --- | --- |
| etnaviv | the Linux `drm/etnaviv` kernel driver, read in the lab scratchpad. GPL: **read only**, never copied, never vendored. Facts, register names and values only. |
| Mesa | Mesa 26.2.1's `src/gallium/drivers/etnaviv` (MIT), the userspace 3D state emitter, and `src/etnaviv/hw/*.xml.h`, the register descriptions. |
| the i.MX8MP RM | NXP's reference manual, for the SoC side (GPC, CCM, WDOG), not the GPU. |

`hw/*.xml.h` are the etna_viv rnndb headers, copied unchanged (MIT, notice in
each file). The seven of them define no macro twice, so they can all be
included together.

## The files

| file | milestone | what it does |
| --- | --- | --- |
| `gc_bringup.c` | M1 | deleted 2026-10-03, now Rust (`gpu::vivante::power`, `hw`): GPC power domains, CCM roots and gates, identity, reset to idle |
| `gc_hw.c` | M2 | deleted, now Rust (`hw`, `irq`): the register init after reset, the interrupt, the 30 events |
| `gc_mmu.c` | M2 | deleted, now Rust (`mmu`): the two-level page tables and the secure-AHB enable sequence |
| `gc_cmd.c` | M3 | the command-stream writers (LOAD_STATE, LINK, EVENT, ...) |
| `gc_cmdbuf.c` | M3 | deleted, now Rust (`cmdbuf`): the ring, the front-end idle loop, one submit |
| `gc_draw.c` | M4/M5 | the clear and the one-triangle command streams |
| `gc_shaders.h` | M5 | the vertex and pixel shader the triangle uses |
| `gc_render.c` | — | the service in `gc_render.h`: carveout, fences, stall dump |

## M1: power, clock, identity, reset

Unchanged from the first pass except for one thing, which was a real bug.

**The feature words do not come from registers.** etnaviv keys a hardware
database on model and revision and returns from its identify before reading any
feature word, and `gc_bringup.c` does the same. Before that, `minor_features[1]
& MMU_VERSION` was read from the part, which made the reset loop skip its check
that the MMU had come back disabled -- a reset that reports success while the
MMU is still on is the worst possible starting state.

The reasoning here was first written as "this part leaves `MORE_MINOR_FEATURES`
clear in word 0, so words 1 and above are not registers at all". **Silicon says
otherwise**: it reports `minor0 0xc9799eff`, with `MORE_MINOR_FEATURES` set, so
the higher words probably are readable. The conclusion does not change and the
database is still the right source -- etnaviv keeps one for parts that *do* set
the bit, because the registers on this family are known to disagree with the
hardware, and this part proves the point in the same log line: the registers
say `features 0xe0687ca5` where the database says `0xe0287c8d`. Trusting the
registers would mean trusting exactly one of those two numbers with nothing to
check it against.

Both are read and logged next to the table for that reason, and the difference
above is what the logging is for.

## M2: the register init after reset

The order in `gc_hw_init()`, with the etnaviv condition and how it resolves
for this core. Most of etnaviv's init is errata for other Vivante parts; this
table is as much about what is deliberately *not* written.

| register | value | why, and the condition |
| --- | --- | --- |
| `PM_POWER_CONTROLS` (0x100) | read \| `ENABLE_MODULE_CLOCK_GATING` (0x1) | etnaviv `enable_mlcg`. The `DISABLE_STALL_...` bit is for revisions 0x4301/0x4302 only. Written, then read back: PM writes are posted. |
| `PM_MODULE_CONTROLS` (0x104) | read \| `RA_HZ` \| `RA_EZ` (0x00030000) | the only two etnaviv sets unconditionally. The PA bit needs `BUG_FIXES12` clear (it is set), the PE bit revision < 0x5000, bit 15 revision < 0x5422, the TX bit revision 0x6202 or 0x6203, the SE/RA pair revision 0x6202, `SH_EU` another model. None apply to 0x6204. |
| `HI_AXI_CONFIG` (0x008) | `AWCACHE(2) \| ARCACHE(2)` = 0x00002200 | "cacheable, no allocate". etnaviv writes it unconditionally and notes that without it the i.MX6 locks up. |
| `MMUv2_AHB_CONTROL` (0x3a8) | read \| `NONSEC_ACCESS` (0x2) | this core has the secure AHB, which starts up refusing non-secure accesses, and everything the driver does is non-secure. Read-modify-write: the reset bit is in the same register. |
| `PM_PULSE_EATER` (0x10c) | read \| `INTERNAL_DFS` (bit 18) | etnaviv's path for revision > 0x5420 with a 3D pipe. **Not** the 0x01590880 constant: that is for parts whose pulse eater cannot be read back, and writing it here would clobber what the boot ROM and Linux's frequency scaling left. |
| `HI_INTR_ENBL` (0x014) | 0xffffffff | all 30 event bits plus the MMU exception and the AXI bus error. There is no per-event enable. |

Not written, and etnaviv does not write them either: `HI_AXI_CONTROL`,
`MC_PROFILE_CONFIG0..3`, `MC_BUS_CONFIG` (GC2000 rev 5108 only),
`MC_DEBUG_MEMORY` (GC320 errata). No register is written for the pixel-pipe or
shader-core count, and the kernel driver has nothing HALTI5-specific at all --
HALTI5 is entirely a userspace concern, which is why everything HALTI5 in
`gc_draw.c` is sourced from Mesa and not from etnaviv.

Front-end registers in 0x65c..0x670 are read twice (`gc_rd_fe()`): etnaviv
does the same, noting that on some GC7000 variants the first read is stale.
The stall dump depends on them, so it uses that accessor.

## M2: the MMU

Two levels of 1024 entries. A master entry covers 4 MiB and holds a slave
table's physical address plus a present bit; a slave entry maps one 4 KiB page
and holds bits 31:12 of the physical address, bits 39:32 in bits 11:4, and
present and writable bits. Untouched entries hold the exception bit, so a
stray access faults instead of reading something random.

The carveout is **identity-mapped**: a GPU virtual address equals its physical
address. That is a deliberate debugging choice, not a shortcut -- an address in
a stall dump can be read without a translation table. Memory imported with
`gc_render_map_phys()` (a scan-out buffer, which on this board may be above
4 GiB physical) gets a virtual address from a separate window at 0x10000000 so
it can never collide with a carveout address.

The part a port gets wrong is the enable. This core has `BIT_SECURITY` and
`SECURITY_AHB`, etnaviv's `ETNA_SEC_KERNEL` mode, and in that mode **the page
table base is not in a register**. It lives in a 64-bit entry of a "page table
array" in memory, and the index of that entry can only be set by a command the
front end executes -- with the MMU still off, from a physical address. So
`gc_mmu_enable()` is: the array's address and enable bit, both safe addresses
and their high bits, the array entry in memory, a two-command stream run from
a physical address, wait for idle, and only then the enable bit. Each step is
load-bearing and the order is etnaviv's.

The "safe page" is a 4 KiB page filled with `0xdead55aa` that is deliberately
never mapped. An access the MMU cannot translate, and every speculative fetch,
is redirected there instead of hanging the bus, and the pattern makes such a
read recognisable in a result buffer.

The driver has one address space and never unmaps, so there is no per-submit
MMU flush. Adding unmap later means adding one: the flush is a `LOAD_STATE` of
`MMUv2_CONFIGURATION` with `MODE_MASK | FLUSH_FLUSH` (0x18, **without** the
master table address, which in secure mode comes from the array) followed by
the FE/PE semaphore and stall pair.

## M3: the ring and one submit

The front end is never stopped. The ring ends in a two-command idle loop --
`WAIT` then `LINK` back to that `WAIT` -- and the front end spins there,
re-reading the `WAIT` word. A submit appends a new segment after the loop and
then overwrites that one `WAIT` word with a `LINK` to it. That single store is
the commit point.

It works because a `WAIT` is one dword in a 64-bit slot: the second dword is
unused, so the `LINK`'s address operand can be published while the front end
still sees a `WAIT`, and the header store then turns the slot into a `LINK`
that is already complete. A barrier separates the two stores. The ring lives
in the carveout's Normal non-cacheable mapping, so there is nothing to flush --
and nothing that *could* be flushed safely, since the GPU is reading the same
line the CPU is writing.

Sizes given to the front end -- the `LINK` prefetch field and
`FE_COMMAND_CONTROL` -- are in **64-bit words**, that is bytes / 8. Every
command starts on a 64-bit boundary; `gc_cmd.c` keeps that true by padding
odd-length commands with a `NOP`.

A submit is:

1. prologue: flush colour and depth, semaphore and stall FE to PE, select the
   3D pipe (the selected pipe is undefined after a reset);
2. `LINK` to the caller's command buffer;
3. the caller's commands, then a `LINK` the service writes back into the
   caller's buffer at `cmd_bytes` -- which is why an allocation must carry
   `GC_CMDBUF_TAIL` of slack;
4. epilogue at the return point: flush depth, colour and the level-1 shader
   cache, flush the tile-status cache, semaphore and stall FE to PE, then the
   `EVENT`;
5. a fresh idle loop, and the commit.

The `EVENT` is tagged `FROM_PE`, so the interrupt comes from the unit that
wrote the pixels and not from the front end that merely read the command.
`FROM_FE` would fire with the pixels still in flight. Together with the cache
flushes ahead of it, that is what makes "the fence completed" mean "the CPU
can read the result".

The caller's buffer is linked to, not copied, so a large draw stream costs
nothing per frame.

## M4 and M5: the clear and the triangle

`gc_draw.c` is a bring-up rig, not a GL backend: two fixed command streams for
one fixed surface format, so that the first picture depends on as little as
possible. The real per-draw translation is milestone 6, in YakoGL.

Three facts about this core shape all of it, and each is a place a port goes
wrong silently:

1. **No BLT engine** (the hardware database's `BltEngine` bit is 0), so the
   clear is the **resolve engine**, which is what Mesa picks here. That engine
   touches no 3D state, which is why `gc_clear` is a useful test on its own:
   it isolates memory, the MMU and the submit path from the 3D pipe. Its one
   constraint is that the width must be a multiple of 16; the driver rejects
   anything else rather than letting the hardware do something undefined.
2. **HALTI5 fetches shader code from memory** through the instruction cache.
   There is no instruction memory to write. The whole pre-HALTI5 register set
   -- `VS_OUTPUT(n)`, `VS_INPUT(n)`, `VS_START_PC`/`END_PC`,
   `PA_SHADER_ATTRIBUTES`, the `GL_VARYING_*` block -- must be left alone;
   HALTI5 has its own (`VS_HALTI5_OUTPUT`, `VS_HALTI5_INPUT`,
   `PA_VARYING_NUM_COMPONENTS`, `GL_HALTI5_SHADER_ATTRIBUTES`). Programming
   both sets is the obvious mistake and there is nothing to warn about it.
3. **A linear render target needs `PE_LOGIC_OP`'s `SINGLE_BUFFER` field set to
   1.** Without it the pixel engine writes a tiled layout, and the read-back is
   a scrambled picture rather than a wrong one. Mesa gates its linear-PE path
   behind a debug flag, so this is a real but lightly travelled road.

The state in `emit_reset_state()` is re-emitted at the head of every command
buffer, not once at boot, because that is what Mesa does
(`etna_reset_gpu_state` runs whenever a flush set `needs_gpu_state_reset`).
Several of its values are unnamed magic in Mesa too; they are copied verbatim
with the register they go to, and the four scissor and clip margins
(`0x1119`, `0x1111`, `0xffff`, `0xffff`) are empirical constants that nothing
documents.

The shaders are a pass-through pair from `etna_shc` and both compile to a
single `NOP` instruction: the hardware loads attributes into the shader's
temporary registers and reads the outputs back out of them, so a pass-through
needs no arithmetic. That is deliberate for bring-up -- if the triangle comes
out wrong, the shader cannot be the reason. Gouraud shading needs no shader
change; per-vertex colours and a smooth interpolation mode do it.

The viewport uses a negative Y scale so that clip y = +1 lands on row 0, the
first row in memory. That is what a scan-out buffer wants and it makes a
read-back check read the way the picture looks.

## Recovery: what the wedge of 2026-09-22 was, and what is now done about it

A dEQP sweep stopped the GPU after 1,431 cases. The numbers, from the run that
saw it (`tools/cts/RESULTS.md`, "The remote control sweep wedged the GPU"):
twelve events in flight, fences 25341..25352 unsignalled, a fresh
`es100_triangle` opening `/dev/gcgpu` and then hanging, and the shader server
on 7779 still alive — so the GPU, not the harness.

**What can be concluded from that capture, and what cannot.** The line naming
twelve events is `gc_render_stall_dump()`'s, so a fence timeout had already
fired and the driver had already noticed. What is NOT determinable from what
was captured is *why* the GPU stopped: the dump's other fields — `IDLE_STATE`,
the FE's current address and last-fetched command, the MMU status — were not
kept, only the summary line. Settling the cause needs the board's own dmesg
around that timeout. It is recorded here as open rather than guessed.

Two candidate explanations were ruled out from the code rather than assumed:

- **Not event exhaustion.** Twelve of thirty events were in flight, and
  `gc_event_alloc()` returns `-EBUSY` after its bound rather than looping, so
  no submit could have blocked forever on a free slot.
- **Not a second client.** `gcgpu_open()` refuses a second thread group with
  `-EBUSY`, so nothing else was driving the GPU.

**The defect the wedge exposed is a driver one and it is fixed.** Whatever
stopped the GPU, nothing could un-stop it: `gc_render_wait()` logged the dump,
returned `-ETIMEDOUT`, and left the front end stopped for the rest of the
module's life. `gc_render.h` explained the choice and its reasoning was sound
except for one clause — that recovery "means a reset, which this driver does
not have". It has had one since M1. `gc_bringup_reset()` is the sequence every
boot runs, and it was simply never reached from the timeout path.

`gc_render_recover()` now runs it: reset, then `gc_hw_init` → `gc_mmu_init` →
`gc_mmu_enable` → `gc_ring_init`, the same order and the same calls M3 fixed,
then the bookkeeping. The interrupt is not reconnected: that is a host
registration, not a GPU register.

The bookkeeping is the part worth reading twice, because one number was doing
two jobs. `gc_render_completed()` answers *"is the GPU still reading this
memory?"*, which is what the OSAL's deferred free needs; after a reset the
answer is no, for everything, so it advances. `gc_render_wait()` answers *"did
my kick produce its picture?"*; for work the reset threw away the answer is no,
and it returns `-ECANCELED`. Merging the two would have traded a visible hang
for invisible wrong output, which is the worse failure.

### Why the GPU stopped: the ranking, and the theory the dump refutes

The full 1,833-line dump was recovered before the reload
(`build/imx8mp/milestones/wedge-20260922/`, cts-runner). It answers the
question the recovery deliberately left open, and it answers it against the
theory everyone including me found most plausible.

**Two decoded fields do the work.** Both were in the capture from the start;
neither was readable as printed.

`IDLE_STATE 0x7ffffff2`, against `hw/state_hi.xml.h`, where a SET bit means
idle:

| busy | idle |
|---|---|
| **FE**, **PE**, **SH** | DE, PA, SE, RA, TX, MC |

The shader core and the pixel engine are **still executing**, unchanged across
85 timeouts and fifteen minutes.

And the ring had **nothing pending**. The four dwords "past the WAIT" are the
idle loop itself:

    ring[2564] 0x38007735  op 7  WAIT    (operand at 2565)
    ring[2566] 0x40000002  op 8  LINK    target at 2567 = 0xa0a46810
    0xa0a46810 - ring base 0xa0a44000 = 0x2810 = dword 2564

The LINK target is the WAIT's own address -- the self-looping WAIT/LINK pair,
four dwords -- and `pos` 2568 is exactly its end. `DMA_ADDRESS` is that same
WAIT, so the front end sits in its idle loop precisely where an idle front end
belongs.

**The ranking.**

1. **A shader that never terminates. Consistent with every field.** The front
   end fetched the work, dispatched it and returned to the idle loop; SH is
   still running it; PE is still waiting on SH. No MMU fault, no AXI error, no
   interrupt error, and a state frozen rather than drifting is what a spinning
   shader looks like from outside. It fits the workload: the last case pass 0
   recorded was
   `shaders.indexing.matrix_subscript.mat2_dynamic_loop_write_dynamic_loop_read_fragment`,
   a dynamic loop in a fragment shader. **That guess was wrong twice over; see
   below.** It also did not establish that case as the trigger -- the three retry passes each died after one case on three
   *different* cases, which says the device was already dead. A bisect would
   establish it; assuming it would not.

   **The bisect was run, and it is OUR compiler's output.** cts-runner found
   the trigger deterministic and elsewhere:
   `dEQP-GLES2.functional.shaders.algorithm.hsl_to_rgb_vertex` and
   `rgb_to_hsl_vertex` — and **those sources contain no loop at all**, a
   six-arm if/else-if chain over `sextant`
   (`es2fShaderAlgorithmTests.cpp:173`). Nothing in the GLSL can fail to
   terminate.

   Same four cases, same driver, same GPU: the **online** pair wedges, the
   **reference** pair passes all four with an empty kernel log. And within the
   online pair alone the two `_fragment` variants never wedge while the two
   `_vertex` ones always do, which excludes the GL core, the driver and the
   case's GL state. A branch of ours landing on the wrong target makes a loop
   where the source has none, and the GC7000 runs it forever — the FE/PE/SH
   busy signature every dump of this wedge has shown. The back end's open
   branch-target work is the suspect.

   So the mechanism above stands and the culprit changes: the GPU did not
   *stop*, it never *finished*. That is also why no MMU fault and no interrupt
   error ever appeared.

2. **The front end missed the WAIT/LINK hand-off. REFUTED.** That theory needs
   unconsumed work past the WAIT, and there is none; and it needs SH and PE
   idle, since on that story the work never started, and both are busy.

3. **A lost completion interrupt, the work having finished. REFUTED** -- but
   not by the front end's position. A marker sitting on the idle loop is
   exactly what a front end that *has* finished looks like, so that field does
   not discriminate. SH and PE do: if the work were complete and only the
   interrupt lost, both would be idle.

**What this means for the recovery.** A reset would be hiding a driver defect
if the hand-off theory were right, because the GPU would merely have missed a
LINK. It is not right. On the leading theory the GPU is genuinely executing a
program that will not stop, and a core reset is the only thing that ends it.
The wedge also happened on the **remote** pair, so the shader was not produced
by the online compiler.

**What the next dump carries, and why each line exists.** All of this is now
in the driver rather than in a reader's head, because the decisive fields were
already being printed and were read by nobody:

- `gc_idle_state_decode()` names the busy modules. `IDLE_STATE 0x7ffffff2` was
  in every one of the 85 dumps and was decoded by hand, once, afterwards.
- the ring dump states the pending-dword count as a number, and says NOTHING
  PENDING when the front end has consumed everything. Leaving a reader to
  subtract ring addresses produced the "four dwords of work" reading that this
  section refutes.
- the dump says UNCHANGED when the idle state and completed fence both match
  the previous dump, so "frozen, not progressing" is a printed fact rather
  than something obtained from `sort -u` over an hour of logs.

Still missing and worth adding when the harness can supply it: the dEQP case
name at the timeout.

### Checked against etnaviv, and the three places it changed the code

etnaviv recovers the same hardware family from the same situation, so its
recovery was read (mainline `etnaviv_gpu.c`, `etnaviv_sched.c`,
`etnaviv_iommu_v2.c`) and compared step by step. Three things it settled:

- **The front end is not stopped before the reset.** There is no write that
  clears `VIVS_FE_COMMAND_CONTROL_ENABLE` anywhere on etnaviv's hang path, and
  no halt handshake: `ISOLATE_GPU` + `SOFT_RESET` (or `MMUv2_AHB_CONTROL_RESET`
  in secure mode) plus the idle-state verification loop is the entire
  mechanism. Our `gc_bringup_reset()` is that sequence already, verification
  loop included, so the recovery needs no FE stop and does not attempt one.
- **The page tables must NOT be rebuilt, and this corrected the code.** The
  first version of `gc_render_recover()` called `gc_mmu_init()`, by analogy
  with the boot order. etnaviv's `etnaviv_iommu_restore()` only reprograms the
  MMU registers; the MTLB and slave tables live in memory and survive. Ours do
  too -- they are in the carveout, which a core reset does not touch -- and
  `gc_mmu_init()` wipes the MTLB and rewinds `import_va`, so calling it would
  have pulled the GPU address out from under every dma-buf a client still
  held. Now only `gc_mmu_enable()` runs.
- **-ECANCELED is the same answer etnaviv gives.** Its scheduler sets
  `dma_fence_set_error(..., -ECANCELED)` on every job of the guilty context
  and re-runs the rest. Arriving at the same code independently is worth
  something; it is also where we do better, because etnaviv's
  `DRM_IOCTL_ETNAVIV_WAIT_FENCE` never inspects `fence->error`, so its
  userspace sees a hung submit complete silently and has no way to learn a
  reset happened. There is no `GL_CONTEXT_LOST` equivalent in its uapi at all.

One thing etnaviv does not solve either: **nothing guards an event slot whose
interrupt might arrive late.** `event_alloc()` takes the lowest free bit
immediately after recovery, exactly as ours does. Its only defences are that
the reset quiesced the front end, that `INTR_ENBL` is re-enabled last, and
that a slot with no fence is a no-op in the handler. So our reliance on "the
reset removed the stream" is not a proof -- it is the same assumption the
mainline driver makes, and it is recorded here as an assumption.

### The A/B, and the prediction, written before the run

`recover_off=1` restores the old behaviour, so one module load runs both arms.

Arm A — `recover_off=1`, the control. Reproducing the wedge is not guaranteed
and this arm is not evidence of anything unless it does: the trigger is
unidentified. If it wedges, the run ends there, exactly as it did on the 22nd.

Arm B — default. **Prediction: the sweep does not wedge.** Concretely, on the
run where arm A stops making progress, arm B continues past that case; dmesg
shows one `recovering the GPU (fence timeout)` line per hung kick and a
matching `reset to idle:` from the shared reset; `/sys/module/gcgpu/...` does
not need a reload; and the dEQP total rises above 1,434 cases. Individual
cases whose kick was thrown away **fail**; they must not pass. A run in which
the reset line appears and the case still passes would mean the abandoned
fence reported success, which is the one outcome that would make this change
worse than no change, and it would be a defect in `fence_lost()`, not a
tuning matter.

**Precondition that makes the negative branch mean anything:** arm A must
actually wedge in the same session. If neither arm wedges, the run is VOID —
it says nothing about the recovery, only that the trigger did not occur — and
the honest report is "not exercised", not "no regression".

### VERIFIED ON SILICON (cts-runner, 2026-09-22)

The precondition held — arm A wedged in the same session — so the run counts.
Module `469e79b2f91d`, `GPU_RECOVER` unset in both arms, so they differ by
`recover_off` alone.

| | arm A (`recover_off=1`) | arm B (default) |
|---|---|---|
| `rgb_to_hsl_fragment` | Timeout | **Fail** — the real verdict, returned |
| fence timeouts / recoveries | 3 / 0 | 2 / 2 |
| GPU after the run | dead | alive (`es100_triangle` PASS) |

The mechanism, not only the outcome: every arm A timeout reports
`completed 1` — the GPU stopped at fence 2 and never moved, so fences 3 and 4
went into a dead GPU and the in-flight list grew. Arm B's **second** timeout
reports `completed 3`: fence 3 ran and finished after the first recovery. The
reset restores the GPU rather than merely unblocking the waiter.

**The outcome named above as worse than no change did not occur.** Every
recovery sat beside a Timeout, never a PASS — `fences 2..2 abandoned, 1
already done`, `fences 4..4 abandoned, 3 already done` — here and in the full
sweep. `fence_lost()` did its job on every abandoned kick observed.

The sweep that previously died permanently at 1,431 cases **completed**:
11,366 results, 2 recoveries, GPU alive at the end.

Compiled for the board: `gcgpu.ko` builds clean
against the board's own 6.6.142-7.7.0 tree (`build/imx8mp-ktree/linux`).
The Zephyr build of the same sources is unchanged in shape — recovery is on
there too, with no knob, since `gc_render_recover_disable()` is only called
from the Linux chardev.

### Does the Zephyr image recover? Yes, by construction — and it is checked

Not a rhetorical question: this defect class *hangs the GPU*, and a Zephyr
image has no module to reload. If the recovery lived only in the Linux
harness, a product build meeting a non-terminating shader would stop there.

It does not. `driver/zephyr/CMakeLists.txt` compiles the driver with
`file(GLOB ... "${GC_SRC}/*.c")`, so `gc_render.c` and `gc_bringup.c` are both
in the image, and neither `gc_render_recover()` nor `gc_bringup_reset()` sits
behind `GC_LINUX_MODULE`. The Zephyr path reaches the same reset from the same
timeout through the same `gc_render_wait()`.

Two differences worth stating rather than leaving to be discovered:

- **No knob there.** `gc_render_recover_disable()` is called only from the
  Linux chardev's module parameter, so on Zephyr `s_recover_off` is constant
  false and recovery is always on. That is the right default for a product
  image, and it means the A/B arm reproducing the old behaviour exists only in
  the harness.
- **The timeout that triggers it is the OSAL's**: `GC_WAIT_FOREVER_MS` = 2 s
  in `src/osal/zephyr_gc.c`, chosen so that "nothing waits forever: a board
  booted by kexec has no way back except its own reset". A hung shader now
  costs one 2 s wait and a reset instead of the image.

What a sample does with `-ECANCELED` is the sample's business and is not yet
uniform: the failure reaches the caller as an error rather than a hang, which
is what the hold-code channel needs, but no sample yet encodes "the GPU was
reset" distinctly from any other draw failure. That is the next thing to tidy
in the sample layer, not in the driver.

**This does not make a non-terminating shader acceptable.** A reset costs a
frame and a visible glitch; on a cluster that is a safety-relevant event, not
a recovered one. The compiler must not emit a program that cannot terminate,
and the recovery is the floor under that requirement, not a substitute for it.
Whether the GC7000 can bound shader execution in hardware — an
instruction-count or watchdog limit that would make the class impossible
rather than survivable — is not answered here and is worth knowing before
`shaderc-online` ships.

### The temporary-register count: six bits, both stages, and what that does and does not explain

From rnndb (`hw/state_3d.xml.h`), the complete set — there is no HALTI5
variant, these are the only temp-control definitions in the header:

| | address | field | width |
|---|---|---|---|
| `VS_TEMP_REGISTER_CONTROL` | 0x080c | `NUM_TEMPS` mask 0x3f, shift 0 | **6 bits** |
| `PS_TEMP_REGISTER_CONTROL` | 0x100c | `NUM_TEMPS` mask 0x3f, shift 0 | **6 bits** |

So 63 is the largest count expressible in either stage, and the hwdb row for
this part (`gc7000UL_6200`) gives `register_max` = 64 registers. A shader
wanting 67 cannot be programmed correctly by any value of that field.

Until 2026-09-22 the driver masked instead of refusing, so 67 went on the wire
as **3** and 68 as **4**: the hardware reserved three registers per thread and
the shader wrote sixty-seven. That is fixed — a refusal naming both numbers,
for either stage.

**What the field width explains, and what it does not.** It is **symmetric**:
both stages have the same six bits, so the field alone does not explain why
the vertex shader hangs the GPU while its fragment twin, wrapping identically
(68 → 4), merely fails. Two candidate reasons for the asymmetry, and the
distinction matters because only one is testable from here:

- **The consequence differs by stage.** A fragment shader writing registers
  the hardware did not reserve produces wrong pixels; a vertex shader doing
  the same corrupts the state that drives the rest of the pipeline. Plausible,
  and consistent with the fragment twins failing rather than hanging — but it
  is an argument, not a measurement.
- **A second, genuinely vertex-only mechanism.** `VS_LOAD_BALANCING` has no
  fragment counterpart at all, so a limit expressed through it is
  vertex-only *by construction* rather than by consequence.

The two are separated on silicon by a shader whose temp count does NOT wrap
but does exceed the load-balancing band: the wrap predicts it passes, the band
predicts it wedges. That is `wedge_probe` A (temps 44), and it must be run
against a module built before the refusal landed, or the refusal masks the
very thing being measured.

### START_PC / END_PC: we are right not to program them, and here is the proof

A tempting lead, chased and refuted, recorded so nobody chases it twice.

We never write `VS_END_PC` (0x800), `VS_START_PC` (0x838), `PS_END_PC`
(0x1000) or `PS_START_PC` (0x1018). Mesa's `etnaviv_shader.c` sets all four in
its shader state, which looks like a diff — and a never-written bound on
execution would be history-dependent, which is exactly the shape the wedge
had.

It is not a diff. The emissions live in `emit_pre_halti5_state()`
(`etnaviv_emit.c:186-228`) and each is further guarded by
`!screen->specs.has_unified_instmem`. This part is HALTI5 with unified
instruction memory — the i-cache — so **Mesa does not program them on this
hardware either.** The extent of a program here comes from `RANGE_HIGH` and
`ICACHE_COUNT`, which we do program, from the shader's own length.

That also settles the wider question those registers raised: on this part
there is no separate "where does the program end" state to leave stale.

### The load-balancing bound is not a function of `temps`, and the ceiling is a mitigation

cts-runner's ladder (one shader shape, K live vec4s into one varying, no
control flow) breaks sharply and reproducibly:

    temps  12 16 20 24 25 26 | 27 28 32 44
           P  P  P  P  P  P  | W  W  W  W

But two shaders from the same session refuse to fit any threshold:

| shader | temps | instructions | outcome |
|---|---:|---:|---|
| `k23.vert` | 27 | ~69 | **WEDGE** |
| `rgb_to_hsl_vertex` | 28 | 95 | **draws** |

More registers and more code, and it runs. So the hang is **correlated with**
register count and not **a function of** it, and any constant — 21, 26, 63 —
is a mitigation rather than a rule. The one currently shipping (21 for the
first band, the highest a shipped blob carries) is exactly that, and is
labelled so in `abi.rs`.

Mesa's own bound is no help: `etnaviv_compiler_nir.c:1125` refuses only
`num_temps > max_registers`, which is 64 here, so Mesa would emit the temps-40
shader that hangs. And the reference back end's temps-40 output *does* hang,
so this is not a property of our allocator.

**What would produce a rule rather than a constant:** run the same ladder at a
second `num_out`. The load-balancing word is a function of `num_out` alone, so
if the temps boundary moves with `num_out` the two are related and the
relation can be fitted to the formula's A and B; if the boundary does not
move, the bound is independent of the band and the "band ceiling" framing is
wrong in the first place. That is one cheap session with the existing harness
and it is the next thing worth a board slot.

## Numbered list of things only the hardware can answer

1. **Does the GPU answer at all after a kexec?** Linux's `galcore`/etnaviv
   suspends the GPU after 200 ms idle, so the GPC domains are probably down
   and the CCM gates closed when Zephyr starts. M1 always runs the full
   power-up, but whether the GPC handshake completes without the ack wait that
   Linux skips is unknown (question 1 of milestone 1, still open).
2. **Is the pulse-eater read-modify-write right?** The value depends on what
   the boot ROM and Linux left in the register. If the log shows a
   `PULSE_EATER` that looks nothing like `0x01590880` with bit 18 set, this is
   the first thing to compare against a running Linux.
3. **Does the secure-AHB reset actually reset?** `MMUv2_AHB_CONTROL.RESET`
   replaces the usual soft reset. The reset loop checks that
   `MMUv2_SEC_CONTROL.ENABLE` reads back clear; if it never does, the GPU was
   left in a state the AHB reset cannot clear and a full GPC power cycle is
   needed first.
4. **Does the front end run from a GPU virtual address?** This is what
   `gc_fence` answers. If it hangs with `FE_DMA_ADDRESS` stuck at the ring's
   address, the MMU is not translating: check that the page-table-array
   command stream actually executed (the GPU must have gone idle after it) and
   that `MMUv2_SEC_COMMAND_CONTROL` was written with the same prefetch as
   `FE_COMMAND_CONTROL`.
5. **Is the interrupt wired?** The device tree says `GIC_SPI 3`, taken from
   the i.MX8MP Linux device tree. If `gc_fence` times out but the stall dump
   shows the front end back in its idle loop, the render finished and only the
   interrupt is missing: read `HI_INTR_ACKNOWLEDGE` by hand to confirm.
6. **Is 24414 the right idle-loop `WAIT` delay?** It is the core clock divided
   by 32768, etnaviv's rule for about 30 us of pickup latency, and it assumes
   the 800 MHz the device tree claims. If M1's clock setup actually landed the
   core somewhere else, this is wrong in the same proportion. It only costs
   latency, not correctness.
7. **Is the carveout at 0x58000000 really free?** Measured on the board on
   2026-09-20: Linux reserves 0x49860000-0x49caffff, 0x50300000-0x50fd2fff,
   CMA at 0xa0000000, and regions at 0xfaa8e000 and 0x13b000000. 0x58000000 is
   in none of them, and the plan assumed OP-TEE at 0x56000000, which no node
   confirmed. Both the load address and the carveout are device tree values so
   they can move.
8. **Does the colour actually land in memory in the byte order the CPU reads
   back?** The read-back in `gc_clear` and `gc_triangle` compares against a
   value derived from Mesa's format tables. A swapped result means the
   `PE_COLOR_FORMAT` enum is wrong, not the render.
9. **Is the tile-status cache flush needed, or harmful, with no tile status?**
   The epilogue flushes it because etnaviv does. If the clear works without it
   and something else breaks with it, that is the first thing to drop.
10. **Does the shader instruction cache need the code padded or aligned beyond
    64 bytes?** The triangle's shaders are one instruction each; Mesa pads its
    shader buffers and the exact rule is not written down anywhere read here.
    A GPU that hangs in the shader stage with the rest of the state plausible
    points at this.

## The init-path audit (2026-09-22)

After the per-draw audit came back with the wedge in none of it, the init path
was audited the same way: every PM/HI/MC register the hardware has, against
every one this driver writes. The register set comes from the rnndb header we
vendor (`hw/state_hi.xml.h`, MIT), the write set from this driver, so the
comparison needs no reference driver at all.

**Six of the 84 PM/HI/MC registers are written**, and all six are deliberate:

| register | where | value |
|---|---|---|
| `HI_CLOCK_CONTROL` | `gc_bringup.c` | fscale, isolate, soft reset |
| `PM_POWER_CONTROLS` | `gc_bringup.c` then `gc_hw.c` | 0 during reset, then `\|= ENABLE_MODULE_CLOCK_GATING` |
| `PM_PULSE_EATER` | `gc_bringup.c` then `gc_hw.c` | the reset constant, then read-modify-write `\|= INTERNAL_DFS` |
| `PM_MODULE_CONTROLS` | `gc_hw.c` | `\|= DISABLE_..._RA_HZ \| RA_EZ` |
| `HI_AXI_CONFIG` | `gc_hw.c` | `AWCACHE(2) \| ARCACHE(2)` |
| `HI_INTR_ENBL` | `gc_hw.c` | the interrupt enable |

The rest are identity, status and profiling registers, the per-unit MMU page
table and memory base registers this part does not use (it has MMU v2, one
page-table base, not the per-unit v1 set), and `MC_*` latency counters.

**The audit's negative result.** `gc_enable_mlcg()` already carries etnaviv's
eight conditional `MODULE_CONTROLS` bits with their model and revision tests
written out, and this core (revision 0x6204) takes none of them: the PA bit
wants `BUG_FIXES12` clear and it is set, PE wants revision < 0x5000, bit 15
wants < 0x5422, TX wants 0x6202 or 0x6203, the SE/RA pair wants 0x6202, and the
`SH_EU` bit is for another model. So the init path was audited against etnaviv
when it was written, and there is no missed etnaviv workaround to find. The
pulse eater is likewise the read-modify-write branch for revision > 0x5420, not
the blind constant.

**What the audit does leave.** This driver *enables* module-level clock gating
(`gc_hw.c`, `ppc |= ENABLE_MODULE_CLOCK_GATING`) and disables it for exactly two
units, `RA_HZ` and `RA_EZ`. The wedge is a hang with FE, PE and SH all reading
busy. A unit whose clock is gated while it is stalled would hang exactly this
way: timing-dependent, marginal near a threshold, register-heavy, and
indifferent to which compiler produced the shader.

**This is measurable with what is already in the tree, and costs no board arm.**
`gc_render_stall_dump()` prints, unconditionally and on the safe side of the
read-back knob:

    pm: POWER_CONTROLS 0x%08x MODULE_STATUS 0x%08x PULSE_EATER 0x%08x

and `PM_MODULE_STATUS` (0x108) names the units that are clock-gated *at that
moment* -- `MODULE_CLOCK_GATED_FE/DE/PE/SH/PA/SE/RA/TX`. So a stall dump already
captured from a build carrying that line answers the question.

**The prediction, and the precondition that makes its negative mean anything.**
If the wedge is a clock-gating stall, `MODULE_STATUS` at the wedge is nonzero
and names a unit that `IDLE_STATE` in the same dump reports as busy -- gated and
busy at once. If `MODULE_STATUS` reads 0, nothing is gated at the moment of the
hang and this line dies. **The negative only counts if `POWER_CONTROLS` in the
same line reads back with bit 0 set**; if it reads 0 the enable never took, and
a zero `MODULE_STATUS` says nothing about gating at all.

If it survives, the knob is `gc_enable_mlcg()` skipped entirely -- one boolean,
k23 with it on and off, the same binary. Not a silent default change: this
driver enabling MLCG at all is a choice, and turning it off is a performance
cost that has to be bought with a measurement.

**A second, weaker candidate, stated so it is not lost.** etnaviv disables TX
clock gating for revisions 0x6202 and 0x6203 and not for 0x6204, which is this
part. A workaround that stops one revision short of ours is the shape of one
that was extended in a vendor tree and never upstreamed. Confirming what galcore
does for 0x6204 means reading the vendor kernel driver, which is GPL-2.0 while
these files are Apache-2.0 -- register facts can be used, its code cannot be
copied here, and that boundary is the same one the etnaviv citations above
already sit on.

## CPU reads of the shader-state registers reset this board (2026-09-22)

Two builds that read the 0x0800+ state space from the CPU took the board down;
dozens of runs without such a build, including many deliberate wedges, took it
down never. The read-back paths are **deleted**, not kept behind a knob.

| build | arm | outcome |
|---|---|---|
| `d3be989` | a wedging arm | board rebooted |
| `5f1b0dc` | k22 (temps 26), a **drawing** arm, `dump_after_submit=1` | board rebooted mid-draw |
| `469e79b2` | the same k22 grid, minutes earlier | PASS, 4096/4096 |
| `469e79b2` | dozens of runs, many deliberate wedges | zero reboots |

`5f1b0dc` is what settles it. It existed *because* the first explanation was
"never read a stopped GPU": its reads fire only after a submit that COMPLETED,
on a GPU that has just answered. It rebooted anyway, on the rung below the
boundary, on a shader that draws. So the hazard is not "the GPU is hung" -- it
is the CPU reading those registers at all.

**Corroboration, without a board.** The kernel etnaviv driver reads 35 distinct
registers across the whole file, and **not one of them is at or above 0x800**:
they are HI identity and status, PM, MC debug, and the FE DMA registers
(0x660..0x66c). Its debugfs dump is the same set. The vendor's own driver never
reads the state space from the CPU either, which is what one would expect if
those addresses are write-only from the AHB on this part.

(Method note: the first pass at this classified `VIVS_HI_IDLE_STATE_TS` and
friends as 0x800+ registers. They are **bit masks of `HI_IDLE_STATE`**, not
addresses -- the same field-versus-register trap as the register audit above,
caught the same way, by asking what the driver actually *reads* rather than
which names appear.)

What stays: the HI/PM/MC dump in `gc_render_stall_dump()`, which has printed
`CLOCK_CONTROL`, `IDLE_STATE`, `AXI_STATUS`, `POWER_CONTROLS`, `MODULE_STATUS`
and `PULSE_EATER` all day without incident. Different address space, different
outcome.

## The module-clock-gating candidate is refuted (2026-09-22)

The candidate the init-path audit left -- that a unit clock-gated while stalled
is what hangs -- is **dead**, killed by the grep over dumps that already
existed, on the precondition set before the measurement:

    pm: POWER_CONTROLS 0x00140021 MODULE_STATUS 0x002000f0 PULSE_EATER 0x015d0880
    hi: CLOCK_CONTROL 0x00040100 IDLE_STATE 0x7ffffff2 ...

`POWER_CONTROLS` bit 0 is set, so module clock gating really is enabled and a
reading of `MODULE_STATUS` means something -- the precondition that makes the
negative count. `MODULE_STATUS` 0x002000f0 gates {PA, SE, RA, TX}; `IDLE_STATE`
0x7ffffff2 has FE, PE and SH busy. **The two sets do not intersect**: the gated
set is exactly the idle set, which is clock gating working correctly. No unit is
gated and busy at once, which is the contradiction the hypothesis needed.

So the `mlcg=0` knob is not worth a board slot and was never written. Recorded
because a candidate that dies on a stated precondition is worth as much as one
that survives, and because the next person to notice that we enable MLCG should
find this instead of re-deriving it.

Two limits cts-runner stated rather than let anyone infer: the dumps are all
from one build and one wedge shape, and they say nothing about gating during
the *approach* to the hang -- only at the moment the dump ran.
