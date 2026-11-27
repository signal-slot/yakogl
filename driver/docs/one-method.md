<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# One method on every board (design, 2026-10-03)

Written from the code of 2026-10-03 (its commit is no longer in this
repository); no board was used. Section 1 is the inventory of that tree,
before the migration: rust_slint, gc_slint,
v3d_gl_slint, v3d_gl_cluster and v3d_gl_triangle have gone since (names of deleted samples and
files are written without code marks in this document, and their line cites
were dropped), and
`common/sample_glue.c` moved into the board layer
(`driver/zephyr/lib/board/common/sample_glue.c`). State of the migration (section 5):
step 1 is done; step 5 is done, with its i.MX8MP row still pending in the
acceptance run; step 3 is done (`examples/triangle`, `examples/triangle-rust`;
gl_triangle and gc_gl_triangle are gone); step 4 is done (`examples/femtovg`); steps 2 and 6 are open
(gc_gl_test, v3d_gl_test and v3d_deqp still exist).

Paths are relative to `driver/samples/zephyr/` unless they start with
`tools/` or `driver/`.
"Unmeasured" marks a fact that needs a board.

The rule (user, 2026-10-03): the Raspberry Pi 4 (V3D), the Verdin AM62P
(PowerVR) and the Verdin i.MX8MP (GC7000) do the same thing the same way. The Verdin
i.MX8MP is off the farm since 2026-11-16; today's i.MX8MP board runs are
measured on the Armadillo-X2 (`tools/board.sh x2`, same SoC and GPU).
Only five basics differ per board: boot method, display bring-up, result
sink, watchdog and GPU backend. Today everything around the GL calls is
written three times. That is a defect on our side, and this document is the fix.

## 1. Inventory

### 1.1 Which samples exist (exact sets)

| Purpose | AM62P | i.MX8MP | Pi 4 |
|---|---|---|---|
| triangle | gl_triangle | gc_gl_triangle | v3d_gl_triangle |
| test (gl_test.c) | `gl_test` | gc_gl_test | v3d_gl_test |
| slint | gl_slint, rust_slint | gc_slint | v3d_gl_slint |
| femtovg | `gl_femtovg` | **none** | v3d_gl_femtovg |
| cluster (agl-slint-cluster) | **none** | **none** | v3d_gl_cluster |
| deqp | `deqp` | `deqp` (board .conf) | v3d_deqp (uses ../deqp/src as is) |

rust_triangle is AM62P only. dEQP is already one source with per-board glue
(v3d_deqp/CMakeLists.txt compiles ../deqp/src/*.cpp). It is the
pattern the rest should follow.

### 1.2 What is identical: the GL/EGL part

- **triangle**: `eglInitialize`, `eglChooseConfig` (RGBA8888, depth 16,
  ES2), window surface, `passthrough.vert`/`interp_color.frag`, one
  `glDrawArrays`, `eglSwapBuffers`. This is in gl_triangle/src/main.c
  and in gc_gl_triangle/src/main.c with the same calls. On the Pi 4
  the program comes from the online compiler (v3d_gl_triangle/src/main.c).
- **test**: every board runs `driver/tools/pvrlab/gl_test.c` through
  `gl_test_run()` and `gl_test_set_list()` (gl_test/src/main.c,
  gc_gl_test/src/main.c, v3d_gl_test/src/main.c).
- **slint**: one crate and one UI. gl_slint/rust (#![no_std],
  `lib.rs`) and gl_slint/ui/cluster.slint are built by gl_slint,
  gc_slint (gc_slint/CMakeLists.txt) and v3d_gl_slint
  (v3d_gl_slint/CMakeLists.txt). The C side calls
  slint_draw_frame(get_proc, W, H) and slint_set_vehicle().
- **deqp**: `deqp/src/main.cpp`, `deqp_log.c` (one `deqp: <status> <case>`
  line per case) and `deqp_archive.cpp` on all three.

### 1.3 What differs (the board part)

| Concern | AM62P (`common/` of the samples) | i.MX8MP (gc_board/) | Pi 4 (v3d_clear/src/) |
|---|---|---|---|
| Window memory | static `__noinit` buffer in the image (gl_triangle/src/main.c; gl_slint 3 buffers, `main.c`) | GPU carve-out, `gc_render_alloc()` (gc_gl_triangle/src/main.c) | firmware framebuffer from the mailbox (v3d_gl_triangle/src/main.c, bottom half). Slint: own static buffer + guard rows, copied to the fb (v3d_gl_slint/src/main.c) |
| Display | DSS app plane, `pvr_sample_show_window()` (`common/sample_glue.c`), vflip | `imx8mp_display_on(phys, pitch)` (gc_gl_triangle/src/main.c), panel-sized only | HDMI fb from the firmware (`rpi_mbox.c`), `fbcon` console on part of it |
| Console | UART (LOG_MODE_IMMEDIATE, gl_triangle/prj.conf) | panel console, no UART (gc_gl_triangle/prj.conf: `UART_CONSOLE=n`) | HDMI fbcon + mini UART (v3d_gl_triangle/prj.conf) |
| Result | UART text; triangle prints a record line (gl_triangle/src/main.c) | SNVS record (`gc_result.c`) | UART text + the frozen record screen (v3d_gl_test/src/main.c) |
| Watchdog arm | `rti_arm.c` at POST_KERNEL 47 (`am62p/rti_arm.c`) or the SoC-layer feeder on kexec (`common/Kconfig`) | `gc_wdog_arm.c` at PRE_KERNEL_1 1; `wdog_map` PRE_KERNEL_1 0 (`gc_sample.c`) | `rpi_wdog.c` at POST_KERNEL 0 |
| Watchdog bound | `YAKOGL_AM62P_WDT_SECONDS` | `GC_BOARD_WDOG_SECONDS` (300) | `RPI_WDOG_HOLD_S` (50, `rpi_wdog.h`), per sample |
| End of run | `pvr_sample_finish(rc)`: hold `YAKOGL_SAMPLE_HOLD_SECONDS`, `sys_reboot` (`sample_glue.c`) | `gc_sample_finish()`: hold 60 s / 40 s on fail / hold codes, PSCI reset (`gc_sample.c`) | no call: `for (;;) k_sleep` and the watchdog resets (v3d_gl_triangle/src/main.c) |
| Verdict rule | sample prints its own | `gc_sample_verdict()`: a GPU reset turns PASS into FAIL (`gc_sample.c`) | sample prints its own |
| Fatal handler | resets, writes FAULT if SNVS (`sample_glue.c`) | second definition (`gc_sample.c`); deqp turns one off (`deqp/boards/verdin_imx8mp_mimx8ml8_a53.conf`) | none |
| Early init | build line PRE_KERNEL_2 (`sample_glue.c`) | GIC EOI EARLY (gc_board_gic.c), IRQ guard PRE_KERNEL_1 50 (`gc_irq_guard.c`), SNVS map PRE_KERNEL_1 0 (`gc_result.c`) | none |
| Build id | `sample.cmake` `YAKOGL_SAMPLE_TREE` | `GC_BUILD_STAMP` (time) + the same | `V3D_BUILD` in each CMakeLists |
| Kconfig glue | `common/Kconfig` (`YAKOGL_SAMPLE_*`) | gc_common/Kconfig + gc_board/Kconfig (`GC_SAMPLE_*`, `GC_COMMON_*`, GC_BOARD_*) | CMake `option(V3D_RPI_WDOG)` + per-sample Kconfig |

Board code also sits in the wrong places. The Pi 4's board support lives in
a *sample* (v3d_clear/src) and is pulled in by path. The i.MX8MP's lives
in the driver module. The AM62P's lives in `common/`.

### 1.4 The Slint check: the exact differences

| Sample | First-frame state | Window | Check | Tolerance |
|---|---|---|---|---|
| gl_slint | demo_vehicle(0,0), 2 frames (`main.c`) | 1024x600 | **none** (no read-back in `main.c`) | - |
| rust_slint | own drive_cycle | 1024x600 | 5 points printed, not judged (`lib.rs`) | - |
| gc_slint | demo_vehicle(0,0) under PARITY | 1280x800 panel; 1024x600 only with prj_parity.conf, never scanned out (`Kconfig`) | 5 points, PROBES record | 12 per channel, 3 of 5 must match (`main.c`) |
| v3d_gl_slint | demo_vehicle t=0 | 1024x600 + guard rows | 5 points against the AM62P values, sentinel, alpha, guard rows, glReadPixels = memory; window CRC-32 printed (`main.c`) | 1 |

The expected values disagree. For "track" rust_slint/src/lib.rs
expects (20,26,34), while v3d_gl_slint/src/main.c holds the AM62P's
measured (13,17,24).

### 1.5 The result sinks

| Sink | Capacity | Survives reset | Read by | Limits |
|---|---|---|---|---|
| i.MX8MP SNVS LPGPR0..3 (`gc_result.c`, 0x30370090) | 16 bytes, last write wins; 12 bytes with `KEEP_LPGPR0` | yes (battery domain) | Linux nvmem `30370000.snvs:snvs-lpgpr0` | one record per boot; a selector travels in (`gc_result.h`) |
| AM62P UART (ESP32 bridge, UART_ADDR) | unbounded text | n/a (captured live) | `tools/am62p_netboot_uart.py`, `tools/cts/deqp-uart-tally.py` | needs the bridge up; one client |
| Pi 4 UART (Debug Probe) | unbounded text | n/a | the tryboot runner's reader (now `board.sh`), `deqp-uart-tally.py` | none from the code |
| Pi 4 record screen | 30 lines x 158 columns (v3d_gl_test/src/main.c) | until the watchdog reset | a camera | read by eye; lost on reset |

AM62P: a persistent register was searched for. `driver/rust/src/soc/am62p/regs.rs`
(the hand-written TRM map) and `driver/dts/arm64/ti/ti_am62p54_a53.dtsi`
define no RTC or CTRL_MMR scratch register, and nothing under `driver/` names
one. Whether SPRUJ83D has a scratch register that survives an RTI reset and
that Linux can read is **unmeasured**. It needs a by-hand TRM read (the
`tools/ti-trm/check_regs.py` route) and then a board test. Until then the
AM62P carries the record on the UART.

### 1.6 The runners

The five runners this inventory found, and the defects found while reading
them, were in a session record of 2026-10-03 that is no longer in the
repository.
Step 1 replaced them with `tools/board.sh` (section 4).

## 2. The board layer

There is one C header, `driver/zephyr/lib/board/include/yakogl_board.h`, and
it is the only thing a sample includes besides EGL/GL. Each board implements
it once.

```c
struct board_window { uintptr_t phys; void *cpu; uint32_t w, h, stride, format, flags; };
int  board_window(uint32_t w, uint32_t h, struct board_window *out); /* packed rows, GPU-visible */
int  board_show(const struct board_window *win);     /* on screen at (0,0); -ENOTSUP: no screen */
uint32_t board_show_copies(void);                    /* shows that were copies, not flips */
void board_result(unsigned kind, uint32_t w1, uint32_t w2, uint32_t w3);
void board_result_tag(unsigned kind, uint8_t tag, uint32_t w1, uint32_t w2, uint32_t w3);
void board_progress(uint32_t where);                 /* breadcrumb, never over a result */
int  board_selector(void);                           /* -1: none */
void board_watchdog_feed(void);                      /* "still making progress" */
uint32_t board_gpu_resets(void);
FUNC_NORETURN void board_finish(const char *name, int rc);
```

board_watchdog_arm() is **not** an application call. This departs from the
brief. On all three boards the arming runs from `SYS_INIT` before `main()`
(`gc_wdog_arm.c`, `rti_arm.c`, `rpi_wdog.c`), so that a hang in a
driver's init (GPU firmware, display) is covered too. An arm called from
`main()` would leave that part unguarded. The arm stays automatic, and the
bound is one option, `CONFIG_YAKOGL_BOARD_LIFETIME_S`.
`board_watchdog_feed()` is the `rpi_wdog_alive()` hook (`rpi_wdog.c`)
made common: a long loop says it is alive, and the feeder stops at the bound
whatever the loop says.

`board_finish()` does the same thing on every board:
1. It applies the verdict rule of `gc_sample_verdict()` (a GPU reset is never
   a PASS).
2. It writes the APP or CASE record.
3. It holds `CONFIG_YAKOGL_BOARD_HOLD_S`.
4. It resets the board: `sys_reboot` on both Verdins, or stops feeding on
   the Pi 4.

By default `board_window()` returns tightly packed rows. When the screen is
wider than the window, `board_show()` copies the window into the screen
buffer, as v3d_gl_slint does today.

**The scan-out view (`CONFIG_YAKOGL_BOARD_SCANOUT_VIEW`, default n).** With
the knob on, the window is a view into the scan-out buffer: `stride` is the
screen's pitch and `board_show()` copies nothing. YakoGL takes such a window
(branch unify/stride-view, merged; its commit is no longer in this
repository): EGL accepts a native WINDOW whose
pitch is whole pixels, even, at least the packed pitch and at most 16384; the
GL core carries it per surface (GlContext.default_pitch,
GlesDraw.target_pitch, 0 = packed) and only for the default framebuffer.
Each backend writes it into its own stride field: the PowerVR PBE line
stride and the load texture stride, the V3D job target stride, and the
GC7000 `PE_COLOR_STRIDE` and `RS_DEST_STRIDE`. Per board:
- Pi 4: the window is a view into the firmware framebuffer (pitch 5120 in the
  measured run); the console sits beside it. With
  `YAKOGL_BOARD_RPI4_FB_VIRTUAL_HEIGHT` the framebuffer holds two screens,
  so a view can be a double buffer (the cluster's 2 x 720 rows), and
  `board_show()` of the half at row y pans the screen to y
  (`SET_VIRTUAL_OFFSET`, no copy).
- i.MX8MP: a panel-pitch buffer, zeroed, scanned in place (the LCDIF pitch
  is 64-byte aligned).
- AM62P: unchanged. The DSS already shows the window by a flip (VID_ROW_INC),
  and `board_show()` measured 0.0 ms over 5971 frames.

A sample check walks rows at `stride` and never paints a pixel the user can
see: the rows under the window are snapshotted before the first frame and
compared afterwards. gl_test case 171 `viewwin` draws and reads a 64x64
window at pitch 96 and checks that the padding is untouched.

This is the one present path: Step 5's BOARD_WINDOW_SCANOUT flag (the
window is the framebuffer itself, packed rows only) went when the view
landed, and the cluster sample turns the view on for every board. A
window of the panel's size is scanned in place on both Verdins with or
without it. Every copy that remains is counted (`board_show_copies()`),
printed once, and flagged in the APP record (`APP_SHOWN_BY_COPY`, w1 bit
16), so a clear bit 16 proves the run copied nothing.

Rust code reaches the same symbols through `extern "C"`, as rust_slint
does with `pvr_sample_*` today (rust_slint/src/lib.rs).

**Where the code goes** (`driver/zephyr/lib/board/`, `CONFIG_YAKOGL_BOARD`,
chosen by SoC):

| New file (relative to `driver/zephyr/lib/board/`) | Comes from |
|---|---|
| `common/board.c` (finish, verdict, build line, fatal handler, record line) | `common/sample_glue.c`, `gc_sample.c` |
| `common/record.c` (C shim over the Rust encoder) | `gc_result.c` (rules), `gc_result.h` |
| `am62p/board_am62p.c` (window, DSS show, UART sink) | `common/sample_glue.c` |
| `am62p/rti_arm.c` | the AM62P watchdog arm of the samples' `common/` (as is) |
| `imx8mp/board_imx8mp.c` (carve-out window, `imx8mp_display_on`, hold) | `gc_sample.c`, gc_gl_triangle/src/main.c |
| `imx8mp/gc_result.c`, `gc_wdog_arm.c`, `gc_irq_guard.c`, `gc_board_gic.c`, `gc_panel_console.c` (kept their `gc_` names) | gc_board/gc_result.c, `gc_wdog_arm.c`, `gc_irq_guard.c`, gc_board_gic.c, `gc_panel_console.c` |
| `rpi4/board_rpi4.c` (mailbox fb window, copy-to-screen, clock max) | v3d_clear/src/rpi_mbox.c, v3d_gl_slint/src/main.c (window + copy), v3d_gl_triangle/src/main.c |
| `rpi4/rpi_wdog.c`, `rpi4/fbcon.c`, `rpi4/rpi_mbox.c` | v3d_clear/src/rpi_wdog.c, `fbcon.c` |
| `common/record_screen.c` (ring, freeze at end or deadline) | v3d_gl_test/src/main.c, v3d_deqp/src/v3d_board.c (tap) |

One Kconfig replaces three prefixes: `YAKOGL_BOARD_LIFETIME_S`,
`YAKOGL_BOARD_HOLD_S`, `YAKOGL_BOARD_HOLD_FAIL_S` and
`YAKOGL_BOARD_RECORD_SCREEN`. The GC_*, `YAKOGL_SAMPLE_*` and `V3D_RPI_WDOG`
names stay as aliases until the last sample has moved, because the Qt
application outside this repository links gc_board
(gc_board/README.md). The aliases are removed in their own commit.

## 3. The record

The record is the format of `gc_result.h`, unchanged, on every board.
`w0 = 0x594B0000 | kind << 8 | low`, then w1..w3, written magic last. The
kind numbers 1-14 keep their values (14 is STAGE, a stage breadcrumb, from
the i.MX8MP's cold-entry branch, `gc_result.h`). `low` is a sequence number, except for
the kinds whose header says it is a tag: PROBES (status), WDOG (tag), SWEEP
(cases done or 0xff), DEQP_CASES and DEQP_STATUS (index), DEQP_FAILS (count),
PERF (selector) and GPU_STOP (cause). The decoder must know which kinds use a
tag, so that list is part of the format.

**Added (Step 5):** APP's w1 carries flags above the stage and status:
`APP_SHOWN_BY_COPY` (bit 16) says board_show() copied the window to the
screen at least once instead of showing it in place (section 2,
`board_show_copies()`); board_finish() sets it, and the cluster sample sets
it in its own APP. `yakogl-tools record` prints it as "shown by copy".

**Added:** kind 15 FINGERPRINT. The first version of this document gave it
14, which collides with STAGE on imx8mp/cold-entry; 15 is free on that
branch, am62p/netboot-scan-fix and main. w1 is the CRC-32/IEEE of the whole window
(rows top-down, `crc32_ieee`), w2 is the frames drawn, w3 is `W << 16 | H`.
It carries v3d_gl_slint's window CRC (`main.c`) as a fingerprint, not
as a verdict.

**Added:** kind 20 GPU_STOP, the verdict of a run the GPU driver ended:
the PowerVR render service found a kick it cannot complete
(`gpu::pvr::freelist::stop`, with the freelist grow knob only). The tag is
the cause (2 out of memory at the freelist's maximum, 3 a grow past the
host's table; 1 is not used), w1 the fence, w2 the GPU recoveries so far (the runner's
`resets`), and w3 is `list << 24 | KiB`. 19 is KICK_DIFF on
vivante/gc-ab-knobs and is reserved here.

**Added:** kind 22 DISPLAY, the i.MX8MP display chain while it scans a
window (the i.MX8MP board layer, `CONFIG_YAKOGL_BOARD_IMX8MP_DISPLAY_RECORD`,
default n; issue #8). w1 is the pixel clock in kHz with bit 31 set when the LCDIF was
already scanning, w2 the DSI lane rate in kHz, w3 the SN65DSI84's IRQ_STAT a
second into the picture and at the start, the display health and the
LCDIF's format and pitch (record.rs has the bits; `yd_record_display_words`
packs them). board_finish() writes it after a passed run's verdict, so on
the i.MX8MP it is the record the SNVS keeps; its status is n/a, like PICTURE's.

**Added:** kind 23 GL_ERROR, one of the first GL errors a run raised, by
name (issue #814). The GL core keeps the first eight errors with the entry
point that raised them and the client's tag (`yakogl_gl_error_tag`;
glmark2 sets its benchmark-list entry). glmark2 prints them all; with the
selector `GL_ERROR_SEL | k` (0x800 | k) it also stores error k as a GL_ERROR
record, tag k, after its GLMARK2 verdict, so the i.MX8MP's SNVS keeps it. w1
is the code, the tag (bits 16..23) and how many were kept (bits 24..31), w2
the FNV-1a of the entry point's name (`yakogl-tools record` names it from the
GL headers), w3 every GL error raised, 32 bits. Its status is n/a.

**Added:** kinds 24 GM_PAGE and 25 RATE, a glmark2 run's per-scene values
and the cluster's frame rate for a board that keeps one record (the
i.MX8MP; `docs/notes/imx8mp-gm-record.md`). With the selector `GM_SEL | t
<< 6 | n` (0x400) glmark2 runs only one page's scenes and stores the page
last, tag = the selector's low byte: an F page holds three scenes' FPS and
frames, P0 and P1 one scene's picture value (`driver/docs/glmark2.md`, "Per
scene without a console"). With `GM_SEL | 3 << 6` the cluster stores RATE
last: frames, milliseconds, the window; tag = the run's status (the cluster
sample is removed, so no image writes RATE now; `yakogl-tools` still decodes
it). GM_PAGE's status is n/a; RATE's is its tag. `tools/board.sh imx8mp gm` runs the pages
and `gm-lines` turns them back into the console's lines.

**Defined once, in Rust:** `driver/rust/src/board/record.rs` (no_std) holds
the kinds, the packing (`case`, `probes`, `picture`, ...), the CRC-16 used
by PROBES and DEQP, and the decoder. The C shim `common/record.c` calls
yd_record_pack_* and hands the four words to the sink. The same module is
the host decoder, so writer and reader cannot drift.

**How each board carries it:**
- **Every board** prints `yk-record: %08x %08x %08x %08x` on its console at
  every write, through printk, so a log level cannot drop it. gl_triangle
  already does this under another prefix (`main.c`).
- **i.MX8MP**: also the SNVS (the last record survives), plus the selector.
- **AM62P**: the UART line only (see 1.5, persistent register unmeasured).
  There is no selector. A run is chosen by its image.
- **Pi 4**: the UART line, and the record screen shows the last record line
  as its final line.

**One decoder:** `yakogl-tools record`. It takes `<w0> <w1> <w2> <w3>`, or
`--nvmem <file>`, or `--log <uart log>` (every `yk-record:` line, the last
one being the verdict). The dEQP case-name mapping (CRC-16 against the
mustpass list) moves with it. It replaced the Python decoder's `record()`
and the i.MX8MP tree runner's own decode (both deleted in Step 1). It is self-tested on the records quoted in
`docs/notes/imx8mp-netboot.md` (`594b040e ff403f80 35706c4e 04000258` = PICTURE)
and `gc_result.h` before it is used.

## 4. The runner

```text
tools/board.sh <board> status | install | uninstall | disarm
tools/board.sh <board> probe <logdir>
tools/board.sh <board> run <image> <logdir> [name]
tools/board.sh <board> tree <table> <logdir>      # a set of images, resumable
tools/board.sh <board> tally <rundir> | diff <rundir-a> <rundir-b>
   board = pi4 | am62p | imx8mp | x2   BOARD_BOOT = tryboot | uboot-tftp | kexec-dev
```

`tree` is added to the brief's verbs. Three scripts do it today (§4 table),
and the rule "repeat work becomes one tool" covers it.

**Boot backends:** `tryboot` (pi4) and `uboot-tftp` (am62p, imx8mp; it keeps
`tools/uboot-netboot-lib.sh` and `tools/tftp_oneshot.py`) produce rows.
`kexec-dev` (am62p, imx8mp; it keeps `driver/tools/kexec-zephyr/`) writes
`DEV` into every row it makes (bootloader-boot rule).
The Armadillo-X2 (`x2`) was added later as a development board with
`kexec-dev` only (`driver/tools/kexec-zephyr/README.md`, "The Armadillo-X2").

**One `run` sequence on every board** (the board supplies the parts in
brackets):
1. Check the image: the header and footprint [link address, RAM window] and
   one watchdog marker. The marker literal becomes `board: watchdog armed`
   on all three boards; today it is `rti_arm: armed`, `gc_wdog_arm: armed` or
   `board: PM watchdog resets`.
2. Take the lock: `board_lock.sh`, one per board, with the directory from
   `$BOARD_LOCK_DIR` and no default inside a session's scratchpad. Then the
   pre-checks: ssh+sudo, no other host in `who`, nobody holds the UART or
   [`/dev/pvrgpu`, `/dev/gcgpu`], [free space on `/boot/firmware`], [no kexec
   loaded], [hook installed].
3. Clear the sink [SNVS zero + selector].
4. Stage [`/boot/firmware` + `tryboot.txt` | TFTP token | `~/kexec`], read it
   back by sha256.
5. Start the capture [Pi 4 by-id UART | AM62P bridge | i.MX8MP TFTP log].
6. Boot. 7. Wait for the last `yk-record:` line or the bound, then for a new
   `boot_id`.
8. Read: the `yk-record:` lines, and the SNVS on the i.MX8MP. Decode them
   with `yakogl-tools record`.
9. Verdict: `done` only when a result record (not a breadcrumb) is present.
   A GPU reset count other than 0 is never a clean PASS.
10. Clean up: unstage, put the `tryboot.txt` back, check that `yako_once`
    is gone.

**One records layout:** `records/<board>/<YYYYMMDD-HHMM>-<name>-<sha8>/` holds
`INDEX.txt`, `console.txt` (time-stamped as the Pi 4 tryboot runner did) and
`meta.txt`. `INDEX.txt` gives the runner's build, the image sha256 and its
build line, the board, the boot backend, the times, the verdict, the decoded
records, and each file with its sha256 and line count (the `records/` tree is gitignored).

**Every existing script:**

| Script | Fate |
|---|---|
| the Pi 4 tryboot runner | absorbed (`tryboot` backend); deleted in Step 1 |
| the AM62P and i.MX8MP netboot runners | absorbed (`uboot-tftp`); deleted in Step 1 |
| the AM62P kexec runner, the two i.MX8MP kexec runners | absorbed (`kexec-dev`); deleted in Step 1 |
| the Pi 4 and i.MX8MP tree runners, the AM62P chain, the i.MX8MP failure-name pager | absorbed (`tree`, `tally`, `diff`); deleted in Step 1. Their build halves moved: `tools/cts/pi4-deqp-images.sh`, `imx8mp-deqp-build.sh --table` |
| `tools/cts/imx8mp-env.sh` | board half absorbed (BOARD_HOST); stays as the Zephyr build environment of the i.MX8MP build scripts |
| the Python record decoder | deleted in Step 1; decoder and name mapping are `yakogl-tools record` |
| `tools/uboot-netboot-lib.sh`, `tools/tftp_oneshot.py`, `tools/am62p_netboot_uart.py` | stay (board basics: U-Boot hook, TFTP, AM62P bridge) |
| driver/tools/kexec-zephyr/* (`kexec_run.sh`, `board_lock.sh`, `snvs_lpgpr.c`) | stay (kexec-dev backend, the lock, the SNVS reader) |
| am62p-recover.sh | deleted with pvrgpu.ko (2026-10-04): `board.sh` waits for Linux itself (a new boot_id), `kexec_run.sh` checks the RTI itself, and there is no module to load |
| `tools/cts/deqp-uart-tally.py` | stays as `tally`'s dEQP reader (already shared by AM62P and Pi 4); port to Rust later |
| `imx8mp-gltest-build.sh`, `imx8mp-deqp-build.sh`, `am62p-deqp-images.sh` | absorbed by a build step of `tree` |
| cts-run.sh, cts-board-run.sh, cts-module-probe.sh | deleted with pvrgpu.ko (2026-10-04; `tools/cts/README.md` says what replaced each) |
| vendor-*.sh, gc-milestone.sh | deleted with the Linux harnesses (2026-10-04) |
| wedge-next-run.sh, stage-pair.sh | deleted (2026-10-04) |
| runrequest-am62p-headpair.sh | deleted with pvrgpu.ko (2026-10-04) |

## 5. Migration, acceptance first

### Step 1: the same Slint demo on three screens

**The one source.** It is gl_slint's crate and UI (gl_slint/rust,
gl_slint/ui/cluster.slint), with v3d_gl_slint's frame sequence and
check, moved into the crate as Rust. The new sample is
`examples/slint/`. Why this source:
- The crate is already the one all three C samples build (§1.2). It is no_std
  and targets `aarch64-unknown-none` with `-Z build-std=core,alloc`
  (gl_slint/CMakeLists.txt), so it needs no Zephyr Rust module.
- rust_slint needs Zephyr's Rust support plus aarch64 patches that are not
  in this repository (`README.md`, "What else some samples need"). It calls
  AM62P-only glue (`lib.rs`) and has a wrong expected value
  (§1.4). It is all Rust, but it is not the one to build on.
- gl_slint's C main has no check at all. gc_slint is 3879 lines of
  i.MX8MP modes (ring, PERF pages, stages). v3d_gl_slint has the
  most complete check (§1.4) and a fixed state.

**"Same" means:**
- the same `examples/slint/ui/cluster.slint` with the ui/1024x600 layout;
- a 1024x600 window (it fits all three panels: 1024x600, 1280x800,
  1920x720) shown at (0,0);
- demo_vehicle at t = 0, two slint_draw_frame calls, then
  `CONFIG_YAKOGL_BOARD_ANIMATE_S` of animation.

Every board leaves a PROBES record (kind 6) with status 1, at one tolerance
(1 per channel) against one reference table, the AM62P's values in
v3d_gl_slint/src/main.c. It also leaves a FINGERPRINT record. Equal
CRCs across GPUs are a goal and are reported, but they are not the
acceptance, because three rasterisers giving bit-equal anti-aliasing is
unmeasured. Each screen is also photographed into the run's INDEX.

**Order:**
- 1a. Write the board layer, the record module, `yakogl-tools record` and
  `board.sh`, with `--dry-run` on all three boards.
- 1b. `board.sh` reproduces one proven row per board before any old runner is
  deleted: AM62P step C (`docs/notes/am62p-netboot.md` §10), Pi 4 any tryboot
  runner row, i.MX8MP step B (`docs/notes/imx8mp-netboot.md`, step B).
- 1c. Slint on the AM62P (netboot proven), then the Pi 4 (tryboot proven),
  then the i.MX8MP. Its U-Boot entry used to die after the WDOG arm. On
  the cold-entry branch (merged; its commit is no longer in this
  repository) the STAGE breadcrumbs named the stage
  (`died_after=console_lcdif_read`). The fix is the reachability guard
  `soc::imx8mp::media::lcdif_unreachable` (`driver/rust/src/soc/imx8mp/media.rs`,
  exported by `driver/rust/src/ffi.rs` as `yd_imx8mp_lcdif_unreachable`). With it, netboot and kexec both give the baseline PICTURE
  `594b040e ff403f80 35706c4e 04000258`.
  Still unmeasured: whether the LCDIF can
  scan out a 1024x600 window on the 1280x800 panel, or whether the window
  must be copied (`imx8mp_display_on` takes a full-panel buffer and a
  pitch).

**Deleted at the end of step 1:** gl_slint/src/main.c, rust_slint/,
v3d_gl_slint/src/main.c and `CMakeLists.txt` (its host/ recorders move to
slint/host/), gc_slint/ (its ring and PERF modes need an owner decision:
move them under board-layer options or retire them), and the old runners
(the table in section 4). Done in Step 1: the runners and the Python decoder
were deleted; gl_slint's UI and no_std patches moved to slint/ui and
slint/nostd, v3d_gl_slint's host recorders to slint/host (the Slint
recorder now drives the one sample's crate); nothing outside gc_slint used
its ring or PERF modes, so they went with it (and gc_common's ring hook).
gl_slint/rust stays, with gl_slint/ui a link to slint/ui: the
platform's Supervisor builds that crate by path (gl_slint/README.md); it
goes once that user has moved.

### Step 1 acceptance run

The one `slint` sample ran once per board from the bootloader boot path on
2026-10-03; the table is in
a session record of 2026-10-03 that is no longer in the repository. The
PROBES record was identical on the two Verdins and one LSB of green apart on
the Pi 4, inside the tolerance of 1. After that run the old runners were
deleted and `tools/board.sh` became the one runner ("Done in Step 1" above).

### Steps 2-6 (each step: one source, all three boards, one row per board, then delete)

Status (2026-10-25, the sample clean-up of the design order): the samples are
cut down to `triangle`, `gl_test`, `slint`, `deqp` and `glmark2`. `gl_test` and
`deqp` are one source tree each for all three boards (the per-GPU `main_*.c`
and v3d_board.c stay inside them, selected by the Zephyr board); `gc_*`,
`v3d_*`, gl_triangle, `gl_femtovg` and gl_slint are deleted, the NanoPC-T6
probes are `tools/probes/nanopc_t6/`, the host replay is `tools/host-replay/`
and the V3D chain tests' shader JSON is `tools/v3d/tri-shaders/`. The steps
below are the plan that led here; where they name a directory it no longer
exists.

2. **test**: `gl_test/` becomes the one sample. Case selection comes from
   Kconfig or the selector. CASE and SWEEP records go everywhere, and the
   record screen comes from the board layer. Deleted: gc_gl_test/,
   v3d_gl_test/, `tools/cts/imx8mp-gltest-build.sh` (the failure-name pager already went in Step 1).
3. **triangle**: gl_triangle/ becomes the one sample, with one window size.
   The PICTURE CRCs only compare when W and H agree (`gc_result.h`).
   Today they are 64x64 (gl_triangle/Kconfig), 1280x800
   (gc_gl_triangle/prj.conf) and 1280x360 (v3d_gl_triangle, half the
   fb). v3d_gl_triangle's whole-window classification becomes the common
   check. Deleted: gc_gl_triangle/, v3d_gl_triangle/, rust_triangle/.
4. **femtovg**: one sample from `gl_femtovg` plus v3d_gl_femtovg's
   program check (fvg_programs.c). The i.MX8MP gets femtovg for the first
   time. Deleted: v3d_gl_femtovg/.
5. **cluster** (removed on 2026-10-19: the sample drew the UI of the external
   agl-slint-cluster repository, the same kind of cluster as the `slint`
   sample, which stays; the text below is the record of the step):
   v3d_gl_cluster becomes `gl_cluster` on all three boards
   (the UI scales itself with fit-width, v3d_gl_cluster/src/main.c).
   Deleted: v3d_gl_cluster/ (its host/ tools move along).
   Done (branch unify/cluster, rows below): the sample is
   `driver/samples/zephyr/cluster/`. Its `rust/`, ui/, host/ and
   font-sizes.txt were git-moved from v3d_gl_cluster/, which is deleted
   after the three rows, with its symlinks, its C main and present
   accounting (and their host check), its ref_probes.h (whose 1280x720
   table had the reference's transform-scale defect), the crate's STRESS
   and gradient-pool entry points and host/ui_omit.py (its measurement
   modes). Its default build (demo off) no longer compiled since an earlier commit
   (v3d_tex_waits declared only under the demo option); that goes with it.
   - The window is the panel's own size (1024x600, 1280x800, 1920x720,
     boards/*.conf), in two halves of one `board_window()`. The animation
     keeps one frame in flight and shows each half once its fence is seen:
     a flip on the Verdins, a pan on the Pi 4, whose two halves are the
     firmware framebuffer's (no copy on any board).
   - The check is cluster/check (Rust): the host reference's probe points
     within 12 per channel, one table per panel size (fit-width puts a
     design point on a different pixel on each panel), all made by
     host/ref/run.sh.
   - The records: FINGERPRINT, PERF page 0 (fps x10; the mean and worst
     frame; the mean draw, swap and show), APP (frames presented and their
     milliseconds), then PROBES (the verdict).
   - The row: fps alone does not compare across the three panel sizes, so a
     cluster row gives the window and the normalised rate beside it, as
     `yakogl-tools record --log` prints it from those records (record.rs
     `Throughput`) and the sample prints it as `cluster: throughput:`:
     `WxH: F frames in M ms, X fps, Y Mpx/s; draw D ms mean, Z ms/Mpx`,
     where Mpx/s = W * H * fps / 1e6 and ms/Mpx = D / (W * H / 1e6). The
     rows of 2026-10-03: Pi 4 1920x720, 8395 frames in 60012 ms, 139.9 fps,
     193.4 Mpx/s, draw 5.0 ms, 3.62 ms/Mpx; AM62P 1024x600, 5936 frames in
     60016 ms, 98.9 fps, 60.8 Mpx/s, draw 9.9 ms, 16.11 ms/Mpx.
   - Not carried over (V3D-only measurement modes of v3d_gl_cluster): the
     femtovg program survey (Step 4's), the GL call counters, the OSAL job
     split, OMIT, STRESS, the render probe and the paced loop.

   The Step 5 acceptance run of 2026-10-03 (AM62P and Pi 4 PASS; the
   i.MX8MP row pending then) is in
   a session record of 2026-10-03 that is no longer in the repository.
6. **deqp**: `deqp/` keeps its source. v3d_deqp/src/v3d_board.c (tap,
   record) and `deqp/src/deqp_snvs.c` go to the board layer's sinks. The
   tree runner replaces the chains (deleted already in Step 1). Deleted:
   v3d_deqp/, and the build scripts once `tree` builds.

After step 6 the aliases (§2) go, along with common/sample_glue.*,
gc_common/ and v3d_clear/src/{rpi_mbox,fbcon,rpi_wdog}.*, which have
then moved. Driver bring-up samples (`gc_identify`, `gc_fence`, v3d_clear,
v3d_triangle, ...) are not GL purposes and are out of scope.

## 6. Unmeasured, and what this design is least sure of

- An AM62P register that survives the RTI reset and that Linux can read
  (§1.5). Until then, the UART line.
- Whether the three GPUs draw bit-equal cluster pixels. The acceptance uses
  probes at tolerance 1. Whether the i.MX8MP meets 1 (it uses 12 today) is
  unmeasured.
- Whether a 1024x600 window can be shown on the i.MX8MP's 1280x800 panel
  without a copy (the Pi 4's view is measured:
  a first-runs record of 2026-10 that is no longer in the repository,
  `tools/cts/RESULTS.md` "Scan-out view"), and whether the AM62P's cold display path
  (the display path is in `docs/notes/am62p-scanout-view.md`) shows the window after a U-Boot entry. Both are
  unmeasured. (The i.MX8MP cold-entry death is fixed, §5 step 1c.)
- `tools/check.sh` builds one gl_test image per GPU (`gl_test`,
  gc_gl_test, v3d_gl_test, with `-Werror` on our C), but not the one
  `slint` sample. Building it for three boards needs a gate row of its own,
  or the three-board claim is untested between board runs.
