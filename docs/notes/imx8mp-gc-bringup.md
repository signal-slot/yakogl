<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# YakoGL on the i.MX8MP (Vivante GC7000UL): backend plan

Status: finished record, moved here from `driver/docs/imx8mp-backend-plan.md` on 2026-10-11. M1 to M5 ran on the board on 2026-09-20 and the back end draws since. The C kernel side and the gclab harness it names are deleted. The kernel side is driver/docs/vivante-kernel-rust.md; the back end is src/vivante/VIVANTE_NOTES.md.

Note 2026-10-03: the C kernel side named below (`gc_bringup.c`, `gc_hw.c`,
`gc_mmu.c`, `gc_cmdbuf.c`) is deleted; the kernel side is Rust
(`driver/rust/src/gpu/vivante`, `driver/docs/vivante-kernel-rust.md`). The
steps below are the plan as it was written.

Status: started 2026-09-16. The board is the **Toradex Verdin iMX8M Plus** (decided
2026-09-16; it keeps the SafeUI carrier and display, and has a Cortex-M7 where the AM62P
has the R5F). No i.MX8MP board has been used yet: everything below is derived from the
sources on the lab host and from what the AM62P port taught, and the code written so
far is compile-tested only.

## What is reused

- **The GL core** (`yakogl/src/gl_*.c`, EGL, the state model, the gl_test suite): it
  reaches the GPU through nine entry points of the backend
  (`pvr_backend_init/draw/flush/finish/finish_previous/kick_id/kick_pending/heap_base`
  and `pvr_surf_bytes_per_pixel`) plus `struct pvr_draw`, the resolved draw description.
  Step one is to put those behind `struct yakogl_backend` (function table + the draw
  struct renamed generically) so `pvr/` and a new `vivante/` implement the same contract.
- **The OSAL** (`yakogl/src/osal/`): unchanged in shape; a Zephyr OSAL for the Vivante
  driver's render service replaces the PowerVR one.
- **The offline shader path**: `struct shc_shader` keeps the reflection (attributes,
  varyings, uniforms, samplers); only the binary changes (`usc` becomes `code`). The
  compiler is Mesa's etnaviv NIR back end run on the host, as `pco_shc` is for PowerVR.
- **The samples and the Supervisor**: `gl_test`, `gl_slint`, the SafeUI rich UI need no
  change beyond the board port.

## What is new

### 1. A Vivante GPU driver for Zephyr (`src/vivante/`, sibling of the PowerVR driver)

The GC7000UL has no firmware: the host writes command streams into memory and the front
end (FE) fetches them. The reference is the Linux etnaviv kernel driver (fetched into the
scratchpad, `etnaviv-kmd/`; GPL, reference only, never vendored) and the i.MX8MP entry
of its hardware database (model 0x7000, revision 0x6204: 2 shader cores, 512 threads,
16 varyings, 320 constants, 512 instructions per shader, 1 pixel pipe).

Milestones, in the order the AM62P port used:

- **M1 power and identity**: i.MX8MP GPC power domain and CCM clocks for gpu3d
  (`etnaviv_gpu_reset_deassert`, `etnaviv_gpu_load_clock`), read `CHIP_IDENTITY`,
  `CHIP_MODEL`, `CHIP_REV`, the feature words (`etnaviv_hw_identify`, `etnaviv_hw_specs`).
- **M2 reset and hw init**: `etnaviv_hw_reset`, `etnaviv_gpu_hw_init` (cache flush,
  clock gating, memory config, MMU v2 with a page table in the carveout,
  `etnaviv_iommu_v2.c`), the interrupt line and the event mechanism (`event_alloc`,
  `irq_handler`: a submit ends with an EVENT command whose id the IRQ reports).
- **M3 front end**: a ring/command buffer and `etnaviv_gpu_start_fe`; the first command
  stream is a NOP + EVENT and the fence is the event interrupt. `etnaviv_buffer.c` shows
  the per-submit prologue/epilogue (cache flushes, TS/pipe switches, the LINK back into
  the ring).
- **M4 clear**: the 3D state machine is a flat register file written with LOAD_STATE
  commands (`state.xml.h`, `state_3d.xml.h`, `cmdstream.xml.h` in Mesa's `src/etnaviv/hw`).
  Mesa's `etnaviv_emit.c` / `etnaviv_state.c` are the ground truth for which registers a
  draw needs; `etnaviv_clear_blit.c` for the clear (a resolve/BLT or a full-screen quad).
- **M5 triangle**: vertex streams (`VS_INPUT`, `VERTEX_ELEMENT_CONFIG`), the shader
  upload (instruction memory or ICACHE from memory), PS output to a linear
  render target, `DRAW_PRIMITIVES`. This GPU is an immediate-mode renderer: no
  parameter buffer, freelists, tail pointers or region headers, and a frame is one
  command stream, which removes most of what took the time on the PowerVR.
- **M6 the GL backend** (`yakogl/src/vivante/`): the `pvr_draw` translation: blend and
  colour mask are real hardware state here (`PE_ALPHA_CONFIG`, `PE_COLOR_FORMAT`), not
  shader variants; depth/stencil are `PE_DEPTH_CONFIG`/`PE_STENCIL_*` with a depth
  buffer in memory; textures are `TE_SAMPLER_*` or texture descriptors on HALTI5;
  MSAA needs the tile-status (TS) and resolve path.

### 2. Shader compiler

`yakogl/tools/glsl2vivante.sh`: GLSL ES 1.00 -> NIR (glslang + Mesa) -> the etnaviv
compiler (`src/gallium/drivers/etnaviv/etnaviv_compiler_nir*.c`, ISA in
`src/etnaviv/isa`) -> instruction dwords + the reflection JSON the offline table
already consumes. The existing blob table format stays; blend states are no longer
shader variants.

### 3. Board port

Zephyr has an i.MX8MP A53 board (`imx8mp_evk/mimx8ml8/a53`); a Verdin i.MX8M Plus
port would follow the AM62P one in the SafeUI repo (SoC, display: LCDIF + MIPI DSI,
touch). The kexec route from Linux applies unchanged (the tool already stops the
Ethernet DMA before the jump; the i.MX FEC/EQOS interfaces are on its list once known).
The safety core is a Cortex-M7 (no MMU, TCM + DDR), so the R5F firmware becomes an M7
build; the DSS split (two planes, R5F on top) has to be re-done on the LCDIF, which
has fewer planes: the safety layer may have to be composited by the A53 instead.

## Order of work

1. *(done 2026-09-16)* `struct yakogl_backend` and the generic draw struct
   (`yakogl/src/backend.h`, `gl_backend.c`); the PowerVR backend is the only table;
   case 7 24/24 on the board.
2. *(done 2026-09-16)* Shader tool on the host against Mesa (no hardware needed):
   `yakogl/tools/vivante/` (`etna_shc.c` built inside a Mesa 26.2.1 tree with
   `mesa-etna_shc.patch`, `glsl2vivante.py` for the GLSL ES 1.00 and 450 sources).
   All 40 shaders under `yakogl/tools/shaders` compile and disassemble; the output is
   JSON (code, temps, inputs/outputs, the constant register file with its uniform
   mapping, `vs_pos_out_reg`, `ps_color_out_reg`, `input_count_unk8`, discard).
   Not done yet: a table generator (the `shader_blobs.c` equivalent) waits for the
   backend's shader ABI (step 3/4 decide how uniforms and samplers are bound).
3. *(written 2026-09-16, not run)* The Zephyr board and milestone 1. Zephyr ships
   `verdin_imx8mp` for its Cortex-M7 only and rejects a second board of the same
   name, so the A53 cluster is a board extension in the SafeUI repository
   (`boards/toradex/verdin_imx8mp/`: `board.yml` with `extend:`, the dts, defconfig
   and a Kconfig fragment that selects `SOC_MIMX8ML8_A53`). It needs the `hal_nxp`
   module (added to the SafeUI `west.yml` allow-list; the lab workspace has it as a
   clone at Zephyr's pinned revision). Console is UART3 (Verdin UART_1), the image
   loads at 0x42000000, the GPU carveout is 64 MiB at 0x58000000 (above OP-TEE at
   0x56000000, below the M7's DDR at 0x80000000: check against `/proc/iomem` on the
   real board first). `src/vivante/gc_bringup.c` is M1 (GPC power-up of GPUMIX and
   GPU3D, CCM roots and gates, identity registers, etnaviv's reset sequence for a
   core with the secure AHB) behind `CONFIG_YAKOGL_DRV_GPU_VIVANTE` and the `vivante,gc`
   binding; `samples/zephyr/gc_identify` runs it, services WDOG1 for a bounded
   time and resets the SoC through PSCI. Builds: 708 KiB image, `gc_init` linked.
   First thing on the board: `kexec_zephyr zephyr.bin 0x42000000` and read
   `gc_identify: PASS`. Open questions the board answers: whether Torizon's Linux
   leaves WDOG1 armed, whether the GPC handshake needs the ack wait Linux skips,
   the Ethernet interface names for the kexec link-down (`eth0`, `end0` are already
   on the list).
4. *(written 2026-09-20, not run)* M2..M5. `src/vivante/gc_hw.c` and
   `gc_mmu.c` are the hardware init, the MMU v2 page tables behind the
   secure-AHB page-table-array indirection, the interrupt and the 30 events;
   `gc_cmd.c` and `gc_cmdbuf.c` the command writers and the front-end ring,
   whose idle loop is re-armed by overwriting one WAIT word with a LINK;
   `gc_draw.c` the resolve-engine clear (this core has no BLT engine) and one
   Gouraud triangle through the HALTI5 3D pipe with pass-through shaders from
   `etna_shc`. `gc_render.c` is the service the YakoGL backend will call:
   allocate from the carveout, submit a command buffer, wait a monotonic
   fence, and a stall dump on a timeout. Samples `gc_fence`, `gc_clear` and
   `gc_triangle` check each layer with pixel read-back; `imx8mp-gc-driver-notes.md`
   has the register list with its references and a numbered list of what only
   the board can answer. Builds: 1688..1692 KiB. Run them in that order after
   `gc_identify`. A Linux harness next to etnaviv unloaded is still the
   fallback if register-level debugging is needed.
5. M6 and the gl_test suite on silicon, then the samples; the shader table
   generator once the binding ABI is fixed. `docs/notes/imx8mp-draw-state.md` is the
   preparation for this step: which registers a draw writes, group by group,
   and where each part of YakoGL's resolved draw goes, read out of Mesa's
   etnaviv driver. Blending and colour masks stop being shader variants there,
   and vertex attributes stop needing float conversion.
6. The safety core: the R5F firmware becomes an M7 build (`verdin_imx8mp/mimx8ml8/m7/ddr`
   exists upstream); Zephyr has an LCDIFv3 driver (`nxp,imx-lcdifv3`) but no DSIM
   MIPI DSI driver for this SoC, and the LCDIF has one plane, so the safety layer
   is composited by the M7 into the scan-out buffer (the M7 owns the display; it
   copies the A53's last complete frame and draws its layer over it) rather than by
   a second hardware plane. To be confirmed with the user.

## On the board, 2026-09-20

The Verdin iMX8M Plus is in the lab on a **Mallow** carrier (not Dahlia) with the
10-inch DSI capacitive-touch panel, running Torizon 7 (kernel 6.6.142, galcore
6.4.11.p2) and a Slint demo in a container. Reached over Ethernet
(`verdin-imx8mp-00000000.local`), `torizon` in the `docker` group, so a
`--privileged --pid=host` container stands in for root (no sudo password
needed): that is how `kexec_zephyr` runs there.

Measured: 4 GiB RAM at 0x40000000, CMA 0xa0000000..0xbfffffff, no OP-TEE,
no M7 running, galcore at 0x38000000/0x38008000/0x38500000, GIC at
0x38800000, the console UART3 at 0x30880000 is `ttymxc2`, WDOG1 armed with a
30 s timeout, the Ethernet interface is `ethernet0` (already on the kexec
tool's list). `gc_identify` (M1) builds, loads at 0x42000000 through kexec,
and Linux comes back by itself; both runs ended in a reset, but nothing could
be read.

**There is no console.** The Mallow's debug UARTs are only on its unassembled
X11 header (1.8 V), and its USB-C ports carry no serial converter. Every
reset-surviving result channel tried from Linux (so that a blind run could
report) failed on this board:

| channel | result |
| --- | --- |
| SNVS LPGPR0..3 (0x30370100) | secure world only: writes from Linux read back 0 |
| SRC_GPR8, GPR9 (0x30390090/94) | GPR1..8 are TF-A's CPU entry registers, GPR9 masks the write |
| OCRAM (0x900000..0x96ffff) | rewritten by ATF and U-Boot on every boot |
| DDR (a `no-map` region at 0xc0000000 added by a Torizon device-tree overlay, readable via `/dev/mem`) | the SoC reset is a PMIC power cycle: a marker written before the reboot comes back as noise |
| M7 TCM / OCRAM_S | an access from Linux hung the board (domain off); do not touch |

`tools/kexec-zephyr/mmio_rd.c` (freestanding, mmap on `/dev/mem`) is the tool
those measurements were made with; the overlay `zephyr-mailbox_overlay.dtbo`
is still installed on the board and does no harm.

### The two things that were actually wrong (same day, later)

Found with no console at all, by making the image reset the SoC at a chosen
point and reading the time until Linux was back (`samples/zephyr/gc_boot_probe`
on the `vivante-gl` branch is that instrument):

1. **The normal kexec turns the panel off.** `reboot(KEXEC)` runs Linux's
   `device_shutdown()`, and the DRM drivers switch the LCDIF and the backlight
   off in theirs, so nothing Zephyr draws can be seen. The fix is the kernel's
   crash route: `kexec_zephyr ... --crash` loads the image with `KEXEC_ON_CRASH`
   and triggers `sysrq-c`; `__crash_kexec` shuts no device down. It needs
   `crashkernel=64M@0x60000000` in U-Boot's `tdxargs` (set on the board), and
   the board dts now links the image at 0x60000000. `kexec_run.sh` takes
   `BOARD_KEXEC_CRASH=1`, `BOARD_LOAD_ADDR`, `BOARD_SUDO_PASS`.
2. **A stray interrupt kills the image.** At a random moment in its first
   minute a kexec'd image takes an interrupt no driver connected
   (`K_ERR_SPURIOUS_IRQ`; the line read back as GIC 38/39, still to be
   confirmed) and halts; WDOG1 then resets the SoC (128 s on the normal route,
   sooner on the crash route). `zephyr/lib/gc_board/gc_irq_guard.c`
   connects every unregistered line at PRE_KERNEL_1 and disables it on its
   first hit. The GIC EOI-mode fix that the AM62P needed is in the SafeUI
   board extension too (`board_a53.c`).

With both, on 2026-09-20, read on the panel by the user and confirmed by the
return time (PASS holds 60 s, FAIL 40 s, a fatal 15 s):

| sample | result |
| --- | --- |
| gc_identify | PASS: GC7000 rev 0x6204, reset to idle |
| gc_fence | PASS: MMU v2, ring, events, fences |
| gc_clear | PASS: two clears, read back |
| gc_triangle | PASS: one Gouraud triangle, vertex read-back |
| gc_gl_triangle | PASS: the same through YakoGL, EGL + GL ES 2.0 (one earlier run died in its first second, before the guard was moved to PRE_KERNEL_1) |

The Vivante display driver (`imx8mp-display` branch: LCDIFv3, DSIM + D-PHY,
SN65DSI83, PWM3 backlight) exists so that the normal kexec route can be used
again; its first run is pending. The M7 side of this board is reached over
RPMsg (`/dev/ttyRPMSG0`), see the RPMsg notes (not in this repository).


## Linux userspace on the same board, 2026-09-22 (the second CTS board)

dEQP-GLES2 is a Linux program. The AM62P runs it because YakoGL builds as
`libEGL.so.1` / `libGLESv2.so.2` over `/dev/pvrgpu`, a chardev exposed by the
PowerVR driver built as an out-of-tree module. This is the same arrangement for
the i.MX8MP, and it now works: **`gl_triangle_user` PASSes on the board**, in a
pbuffer and in an imported window buffer, verified by CPU read-back.

```
gl_triangle_user: GL_RENDERER = Vivante GC7000UL (YakoGL)
gl_triangle_user: red apex                     PASS (128,99) = d81314ff
gl_triangle_user: green left                   PASS  (40,17) = 0ce70dff
gl_triangle_user: blue right                   PASS (215,17) = 0c0de7ff
gl_triangle_user: window buffer written by GPU PASS  cleared=22262 red=484 ...
gl_triangle_user: PASS (0 failing checks)
```

### The pieces

| where | what |
| --- | --- |
| `tools/gclab/` | the module: `gc_mod.c` (platform glue), `gc_chardev.c` (`/dev/gcgpu`), `shim/gc_compat.h` (Zephyr APIs on kernel ones), `Kbuild` |
| `tools/gclab/gcgpu_uapi.h` | the ABI, deliberately the same shape as `pvrgpu_uapi.h`; its header names the three places the two differ and why |
| `src/osal/gcgpu.c` (yakogl) | the userspace OSAL: ioctl + one mmap of the carveout |
| `tools/gclab/user/build.sh` | cross-builds the two libraries and the test program |

`src/vivante/` is unchanged Zephyr driver code, compiled a second time. Two
narrow `#ifndef GC_LINUX_MODULE` fences carry the difference: under Linux the
GPC power domain and the CCM gates belong to the kernel's genpd and clk
drivers, so `gc_mod.c` does that half with `pm_runtime` and `clk_bulk` and then
calls `gc_bringup_attach()`, which is the rest of `gc_init()` -- identity and
reset, through the same `gc_identify()` and `gc_reset()`.

### The board is shared, and stays shared

An M7 firmware owns the panel through route 3. Removing galcore and loading
`gcgpu.ko` does not disturb it: measured before and after, `LCDIF DISP_PARA`
stays 0x80000000 and the scan-out register keeps alternating between the M7's
two buffers (0x81808000 / 0x81c08000). The GPU and the display controller are
independent blocks and this module touches only the GPU's. Stage under
`~torizon/yakogl-user/`; galcore has to be unloaded first, because both drivers
bind `38000000.gpu3d`, and `rmmod gcgpu && modprobe galcore` puts the board
back.

### What cost the day: BIT_MASK

The GPU accepted submits and retired none of them -- 30 events in flight, one
interrupt for thirty kicks, a front end sitting in its idle loop. It looked
exactly like hardware that had never started.

`BIT_MASK(n)` is `(BIT(n) - 1)` in Zephyr and `(1UL << (n % BITS_PER_LONG))` in
the Linux kernel. `gc_render.c` initialised the event bitmap with
`BIT_MASK(GC_EVENT_COUNT)`: thirty free events on Zephyr, bit 30 alone here. So
`gc_event_alloc()` handed out event id **30**, which is one past the end of
`event_fence[]` (the next field is `fence_next`, overwritten on every submit)
and an EVENT the part does not implement (no completion interrupt, ever).

The general lesson for this shim, and the reason it is written down: a shared
source file compiled against two different sets of system headers can pick up
*the same name with a different meaning*, and nothing warns. The shim now
poisons `BIT_MASK` so a reuse fails to compile. An audit of every macro the
driver uses that both worlds define found no other disagreement.

### Two register findings, unrelated and still open

Measured on silicon, both worth knowing before anyone chases a ghost:

- **`MMUv2_AHB_CONTROL` (0x3a8) does not exist on this part.** It reads
  `0xdeaddeae`, the interconnect's pattern for an address no slave decodes.
  The driver used to read-modify-write it, which fed the poison back in; it
  now writes the non-secure grant alone and says so in the log. Whether the
  grant is needed at all is open -- either the AHB is already non-secure
  (the evidence says so: everything works) or the control lives elsewhere.
- **`HI_AXI_CONFIG` (0x08) reads back 0** after a `AWCACHE(2)|ARCACHE(2)`
  (0x2200) write. Write-only or absent; the write is kept, since etnaviv
  makes it unconditionally and it is harmless either way.

`HI_INTR_ENBL` does read back, which is the useful control: the register block
itself is alive and decoding.

### Running it

```sh
# once: the board's own kernel tree, and modules_prepare against it
scp torizon@<board>:/usr/src/linux.tar.bz2 . && tar xjf linux.tar.bz2
make -C linux ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu- \
     LOCALVERSION=-7.7.0 modules_prepare      # LOCALVERSION is not optional

make -C driver/tools/gclab KDIR=<that tree>              # gcgpu.ko
OUT=build/user-gc driver/tools/gclab/user/build.sh       # the libraries + the triangle

scp gcgpu.ko libGLESv2.so.2.0.0 libEGL.so.1.0.0 gl_triangle_user \
    torizon@<board>:~/yakogl-user/
ssh torizon@<board> 'cd ~/yakogl-user && sudo sh -c "rmmod galcore; insmod gcgpu.ko" &&
    LD_LIBRARY_PATH=. ./gl_triangle_user'
```

The host's own `aarch64-linux-gnu-gcc` builds the module; no board-matched
toolchain and no package installs were needed.

### Waits, and why a stuck GPU no longer costs a reboot

Two mistakes of mine, both found the hard way when the GPU was retiring
nothing, both fixed:

- `gc_render_wait()` with no deadline (what `eglSwapBuffers` asks for) never
  looked at whether the waiting thread had been told to stop, so the process
  was unkillable and the board needed a reboot. It now returns `-EINTR`.
  A *bounded* wait is still never cut short: its caller may be the release path
  of a dying process, which has to finish before the memory goes back to the
  pool.
- The chardev held its one mutex across `WAIT`, so a client stuck on a fence
  also blocked every other process in `open()`, which is how the module ended
  up unremovable at refcount 2. `WAIT` now runs with the mutex dropped.

### gl_test in userspace, 2026-09-22, and what the failures are

`driver/tools/pvrlab/user/gl_test_user.c` runs the same
`driver/tools/pvrlab/gl_test.c` the Zephyr sample runs, as a process — both
shared with the AM62P, and built for this board by
`driver/tools/gclab/user/build.sh` (there is no `gl_test_user.c` under
`gclab/user/`; it holds only `build.sh` and `clear_probe.c`). Two modes, and the
comparison between them is
the point: **one process per case** (a hung case then costs one line of the
table, not the run) and **in-process** (case 7, the regression set, the way the
board runs it).

| | one per process | in process (set) |
| --- | --- | --- |
| pass | 41 of 58 | 28 of 38 |
| fail | 15 | 10 |

Sorted by what each failure actually is, which is the only useful form:

**Depends on state an earlier case left behind -- 2 (texture), 12 (stencil2),
23 (winding).** PASS in the in-process run, FAIL standalone. These three are
exactly the ones recorded as passing on the Zephyr board (tasks #1, #8, #9), so
the two agree completely: nothing is wrong with them. The suite's cases are not
all independent, and a case that reads a pixel an earlier case cleared cannot
start from a fresh, zero-filled surface.

**Fails both ways, and passed on Zephyr at `525b2d2` -- 5 (scissor), 9
(stencil), 13 (zls).** The real suspects. Either shared-core behaviour changed
under the Vivante backend since that baseline, or the Linux userspace path
differs. Not separable from here: it needs the same cases on the PowerVR board
(a case failing on both is core by construction) or a same-tree Zephyr run on
this one.

**Never demonstrated on this GPU -- 42, 44, 45, 55, 56, 58, 59, 61.** None is in
the then hand-over's passing list and several postdate it: 58 (`clearscis`) and 59
(`clearmask`) arrived with `99092c7`, 61 (`texincomp`) with `0c0a029`, and 55
(`es3mrt`) is the documented PowerVR-only MRT sweep. 42/44/45 belong to the ES
3.00 sweep the hand-over already called partially measured. These are gaps, not
regressions: they have never been shown to work here.

**Documented gaps -- 16** (4x MSAA, not modelled), **39** (PowerVR performance
counters).

**Long, not hung -- 25, 26.** At a 120 s limit case 26 **passes in 98 s**, and
case 25 was still progressing (`stress: 3500/6000 kicks ok`), about 210 s. The
sweep's 30 s limit is what reported them as TIMEOUT. They are the pair written
to reproduce the PowerVR VDM stall, so a timeout here is the first thing that
would be misread as that stall.

**Intermittent -- 20 (etc).** PASS in one sweep, FAIL in the next, then 5 of 5
standalone. Every case runs in its own process, so the only thing that carries
between them is the driver and the GPU -- state the module keeps across client
processes. Worth knowing before a single failure is taken for a verdict.

**Not a case at all -- 50.** My sweep counted it as a failure. The numbering has
gaps and `run_case()` returned the same value for "no such case" as for a case
that failed, so a sweep over a range invented one. Fixed (`-2`), and the row is
now left out rather than counted.

Nothing here is a driver or chardev fault: every case reaches the GPU, renders
and reads pixels back, and a full sweep leaves no stall dump, no bus error, and
the module at refcount 0.

### Case 20 (etc): a cross-client failure, parked with what is ruled out

`gl_test` case 20 fails sometimes, only when another client process has used
the GPU first. It is the one open item from the userspace sweep, and it is
parked here rather than half-diagnosed, because what has been *eliminated* is
worth more than the remaining guess.

**The recipe.** Run case 19, then case 20, as separate processes, repeatedly:

```sh
for i in $(seq 1 10); do ./gl_test_user 19 >/dev/null; ./gl_test_user 20 || echo FAIL; done
```

Rates measured: 6 of 6, then 0 of 8, then 2 of 6, then 5 of 10 — it is
**probabilistic** and the rate moves with how fragmented the pool is. Standalone
it has never failed (0 of 20, 0 of 6). A module reload clears it. An earlier
report of this as a deterministic reproducer was wrong and was withdrawn.

**The symptom.** Every ETC texel reads `0xff000000`, which is not corruption: it
is the 1x1 opaque black texel the backend keeps for **GL's incomplete texture**
(`black_texel_gpu()`, used by `texture.rs` when `t.incomplete != 0`). So the
texture is being judged incomplete, or the binding is being pointed at that
texel. `texture_is_complete()` in `gl/draw.rs` fails a texture whose uploaded
levels are fewer than its mip chain needs, which is the shape to look at next.

**Ruled out, each for its own reason:**

| hypothesis | why not |
| --- | --- |
| stale GPU MMU translation | impossible by construction: the carveout is identity-mapped once at init and never unmapped or repointed, so a reused range keeps the same VA→PA mapping. (The same defect *is* real on the PowerVR, where VA reclaim repoints an address.) |
| stale GPU texture/shader cache | the backend emits `TEXTURE \| TEXTUREVS \| SHADER_L1` before every draw's state block, unconditionally |
| CPU cache coherency of the client mapping | `dma_mmap_coherent()`: the client gets the coherent allocation's attributes, so a CPU write is in DRAM with no maintenance |
| allocating over memory another client freed while the GPU still reads it | **measured**: `gcgpu.reuse_check=1` across ten runs with five failures, zero warnings |
| an allocation failing during the upload | **measured**: the OSAL logs every failure; none in a failing run |
| the GL errors in case 20 | a passing run reports the *same* two (`0x501`, `0x500` from `glCompressedTexImage2D`) — they are the case's own negative checks, not the fault |

**The tools, all kept and off by default:**
- `YAKOGL_GCGPU_TRACE=1` — every allocation and deferred free with GPU address, phys and size, plus every allocation failure.
- `insmod gcgpu.ko reuse_check=1` — the module warns when a client allocates over a range another client freed while its fence is outstanding, naming both processes.

**The next step**, for whoever takes it: log where `t.incomplete` is decided in a
failing run (level count, format, size, the allocation result behind it) and
diff it against a passing one. Everything else on the list above has been tried.

### The render pool's free list, 2026-09-22: measured, then sized

`gc_render_pool.h` dropped 91 % of everything freed under pooltest's mixed
workload. Two defects, and the measurement decided which redesign was *not*
needed (the task had proposed size classes or a buddy allocator; neither is
justified by the numbers).

**Policy.** A full list used to drop the block that had just arrived, so what
survived depended on arrival order rather than usefulness. Evicting the
*smallest* listed block instead — a large free block serves any request a small
one can, not the reverse — takes 91 % to 3.5 % at the unchanged 128 slots. The
second-order effect is the bigger one: with reuse working, the bump stops
running away, 65 MiB down to 10 MiB. A dropped block does not only lose its own
memory, it forces the next allocation to take fresh bump.

**Capacity.** The peak number of free blocks does not scale with how many
allocations are live (256 live or 8192: peak 215) because coalescing closes the
holes. It scales with the number of distinct sizes and alignments:

| workload (200k ops, 3 seeds)   | peak free blocks | dropped |
|--------------------------------|------------------|---------|
| 15 sizes x 6 alignments        | 215 / 313 / 222  | 3.3 %   |
| continuous 4..256 KiB          | 836 / 911 / 971  | 1.2 %   |

The second row gives every allocation a different size — harsher than any
renderer — and still peaks under a thousand, so 1024 slots (16 KiB of BSS
against the 64 MiB it manages) covers both with headroom.

**On silicon** (`03ca1e8`, module rebuilt against the board's own tree):

```
regression set:  30 of 38, FAIL 9 61 16 20 42 49 45 44   (unchanged)
`all`:           49 passed, 7 failed, 2 skipped, 2 timed out   (unchanged)
no "pool: N KiB dropped" warning during either run
```

Stated precisely, because the distinction matters: **the board run is a
no-regression gate, not evidence the fix helps.** gl_test lives inside the
"exact reuse" assumption the old list was written for and never fills it — the
drop warning does not fire either way. The improvement is pooltest's, measured
on the shipped header; the board's job here was to show that changing the
allocator under the GPU broke nothing.

**The kernel tree is a prerequisite nobody had on this host.** The module needs
`6.6.142-7.7.0` with modversions, so a mismatched build will not load. That
makes it a **hard gate on any change under `driver/src/vivante/` or
`driver/tools/gclab/`**: without the tree those sources cannot be rebuilt at
all -- only the userspace side -- so such a change cannot reach the board and
cannot be measured.
It lives at one absolute path, **`build/imx8mp-ktree/`** under the repository root
— the *main checkout's* gitignored `build/`, not a worktree's. That is
deliberate: it was once kept inside a worktree, vanished with it, and every
worktree then had to rebuild its own. Refer to it by the absolute path from
wherever you are working. The recipe takes about two minutes:

```sh
K=$PWD/build/imx8mp-ktree
mkdir -p "$K" && cd "$K"
scp torizon@<imx8mp>:/usr/src/linux.tar.bz2 . && tar xjf linux.tar.bz2
make -C linux ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu- \
     LOCALVERSION=-7.7.0 modules_prepare        # LOCALVERSION is not optional
make -C driver/tools/gclab KDIR="$K/linux"      # from the repository root
```

`build/` rather than `/tmp`: the tree is 106 MB of build input that a reboot
would silently take away, and the next person would meet the same wall with no
sign of why.

**Check the restore rather than assuming it.** `modules_prepare` must exit 0 and
`linux/include/config/kernel.release` must read `6.6.142-7.7.0`; then rebuild
`gcgpu.ko` and compare its `srcversion` with the loaded module's
(`modinfo -F srcversion driver/tools/gclab/gcgpu.ko` against
`/sys/module/gcgpu/srcversion`). Equal means the tree really is the board's, and
not merely a tree that compiles. Restored and checked that way on 2026-09-22:
both `577EE72754D98530CCEF13C`, vermagic
`6.6.142-7.7.0 SMP preempt mod_unload modversions aarch64`.

### The free that jumped its kick, 2026-09-22

gl_test case 20 passed alone and failed after case 40. It was not a GL state
leak -- case 40 disables depth properly -- but a use-after-free that only
appears when the pool hands the block straight back:

```
default:            40,20 FAIL   case 20's ETC2 half read the background
pool_no_reuse=1:    40,20 PASS
pool_defer_free=1:  40,20 PASS   (the knob of the day; see the note below)
scrub_allocs=1:     40,20 FAIL, same wrong value
```

Scrubbing not changing the value is what ruled out "reads stale contents": the
block was live to the GPU, not merely old. GL lets an application delete or
re-specify a buffer while a draw that reads it is in flight, so the lifetime is
the implementation's.

Deferring a free until its own kick retires is the driver's path now
(`gc_render_defer.h`, in the render service so Zephyr gets it too), stamped
with the referencing kick rather than "the next one", and the queue **never
frees early**: it drains, waits, and failing that leaks the block loudly. The
old chardev queue fell back to an immediate free when full, which reintroduced
the defect under pressure -- the same shape as the pool's old "the list is
full, drop the newcomer".

**The knob names above are the ones of that day, and two of them no longer
exist.** Because deferral became the path rather than an option, there is no
`pool_defer_free`; the live parameter is **`defer_off`**, with the opposite
sense — `defer_off=1` turns deferral *off* and reproduces the use-after-free on
demand, which is how the row above would be taken today. `pool_no_reuse`,
`scrub_allocs` and `reuse_check` are unchanged. `insmod gcgpu.ko
pool_defer_free=1` is rejected. The queue depth is `GC_DEFER_SLOTS`, a
compile-time `#define` in `gc_render_defer.h` — not a module parameter, and not
`GCGPU_DEFER_SLOTS`, which appears in older notes and never existed.

`GC_DEFER_SLOTS` is sized from pooltest's churn workload rather than guessed:

| frames in flight | 1 | 2 | 3 | 4 | 6 | 8 | 16 |
|---|---|---|---|---|---|---|---|
| peak deferred, seed A | 42 | 42 | 44 | 42 | 46 | 42 | 75 |
| peak deferred, seed B | 55 | 64 | 63 | 64 | 64 | 64 | 77 |

It barely tracks pipeline depth, so 256 is a little over three times the worst
of those, for 6 KiB of BSS.

**On silicon**, with the control run both ways. First on `facd514`, then
re-run in full on `627900e` after `adf7731` grew `struct osal_ops` and put a
mutex in every EGL entry point -- a change that touches every call this suite
makes, so the earlier gate no longer described the tree being staged. Identical
both times:

```
deferral on (default):  40,20 PASS
defer_off=1:            40,20 FAIL      the defect, reproduced deliberately
regression set, 20 runs: 31 of 38 every run, FAIL 9 16 20 42 44 45 61
                         case 49 stable in all 20 (it was failing before)
"defer queue full": 0 occurrences        the 256 bound was never approached
```

Unlike the pool-capacity change, this one *is* measured as a fix rather than
only as a no-regression gate: the knob reproduces the defect on demand, and the
set gains a case.

**Still open: case 20 has a second contributor.** With deferral on it fails in
the full set but not in any pair; the minimal failing prefix is 19 cases whose
last element is case 58, and removing 58 or replacing it with 59 makes it pass.
