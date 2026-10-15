<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# One method on every board (design, 2026-10-03)

Written from the code at `700a4310`; no board was used. Section 1 is the
inventory at that commit, before the migration: `rust_slint`, `gc_slint`,
`v3d_gl_slint`, `v3d_gl_cluster` and `v3d_gl_triangle` have gone since, and
`common/sample_glue.c` moved into the board layer
(`driver/zephyr/lib/board/common/`). State of the migration (section 5):
step 1 is done; step 5 is done, with its i.MX8MP row still pending in the
acceptance run; step 3 has the one `triangle/` sample while `gl_triangle` and
`gc_gl_triangle` remain; steps 2, 4 and 6 are open (`gc_gl_test`,
`v3d_gl_test`, `v3d_gl_femtovg` and `v3d_deqp` still exist).

Paths are relative to `driver/samples/zephyr/` unless they start with
`tools/` or `driver/`.
"Unmeasured" marks a fact that needs a board.

The rule (user, 2026-10-03): the Raspberry Pi 4 (V3D), the Verdin AM62P
(PowerVR) and the Verdin i.MX8MP (GC7000) do the same thing the same way.
Only five basics differ per board: boot method, display bring-up, result
sink, watchdog and GPU backend. Today everything around the GL calls is
written three times. That is a defect on our side, and this document is the fix.

## 1. Inventory

### 1.1 Which samples exist (exact sets)

| Purpose | AM62P | i.MX8MP | Pi 4 |
|---|---|---|---|
| triangle | `gl_triangle` | `gc_gl_triangle` | `v3d_gl_triangle` |
| test (gl_test.c) | `gl_test` | `gc_gl_test` | `v3d_gl_test` |
| slint | `gl_slint`, `rust_slint` | `gc_slint` | `v3d_gl_slint` |
| femtovg | `gl_femtovg` | **none** | `v3d_gl_femtovg` |
| cluster (agl-slint-cluster) | **none** | **none** | `v3d_gl_cluster` |
| deqp | `deqp` | `deqp` (board .conf) | `v3d_deqp` (uses `../deqp/src` as is) |

`rust_triangle` is AM62P only. dEQP is already one source with per-board glue
(`v3d_deqp/CMakeLists.txt:33-57` compiles `../deqp/src/*.cpp`). It is the
pattern the rest should follow.

### 1.2 What is identical: the GL/EGL part

- **triangle**: `eglInitialize`, `eglChooseConfig` (RGBA8888, depth 16,
  ES2), window surface, `passthrough.vert`/`interp_color.frag`, one
  `glDrawArrays`, `eglSwapBuffers`. This is in `gl_triangle/src/main.c:95-149`
  and in `gc_gl_triangle/src/main.c:178-245` with the same calls. On the Pi 4
  the program comes from the online compiler (`v3d_gl_triangle/src/main.c:264-330`).
- **test**: every board runs `driver/tools/pvrlab/gl_test.c` through
  `gl_test_run()` and `gl_test_set_list()` (`gl_test/src/main.c:13-16,82-90`,
  `gc_gl_test/src/main.c:179,494-498`, `v3d_gl_test/src/main.c:271`).
- **slint**: one crate and one UI. `gl_slint/rust` (`#![no_std]`,
  `lib.rs:15`) and `gl_slint/ui/cluster.slint` are built by `gl_slint`,
  `gc_slint` (`gc_slint/CMakeLists.txt:19-30,59,88`) and `v3d_gl_slint`
  (`v3d_gl_slint/CMakeLists.txt:84-96`). The C side calls
  `slint_draw_frame(get_proc, W, H)` and `slint_set_vehicle()`.
- **deqp**: `deqp/src/main.cpp`, `deqp_log.c` (one `deqp: <status> <case>`
  line per case) and `deqp_archive.cpp` on all three.

### 1.3 What differs (the board part)

| Concern | AM62P (`common/`) | i.MX8MP (`driver/zephyr/lib/gc_board/`) | Pi 4 (`v3d_clear/src/`) |
|---|---|---|---|
| Window memory | static `__noinit` buffer in the image (`gl_triangle/src/main.c:29`; `gl_slint` 3 buffers, `main.c:38`) | GPU carve-out, `gc_render_alloc()` (`gc_gl_triangle/src/main.c:298`) | firmware framebuffer from the mailbox (`v3d_gl_triangle/src/main.c:358`, bottom half `:391`). Slint: own static buffer + guard rows, copied to the fb (`v3d_gl_slint/src/main.c:81-92,119`) |
| Display | SafeUI DSS app plane, `pvr_sample_show_window()` (`common/sample_glue.c:101-159`), vflip | `imx8mp_display_on(phys, pitch)` (`gc_gl_triangle/src/main.c:342`), panel-sized only | HDMI fb from the firmware (`rpi_mbox.c`), `fbcon` console on part of it |
| Console | UART (`LOG_MODE_IMMEDIATE`, `gl_triangle/prj.conf`) | panel console, no UART (`gc_gl_triangle/prj.conf`: `UART_CONSOLE=n`) | HDMI fbcon + mini UART (`v3d_gl_triangle/prj.conf`) |
| Result | UART text; triangle prints a record line (`gl_triangle/src/main.c:170`) | SNVS record (`gc_result.c`) | UART text + the frozen record screen (`v3d_gl_test/src/main.c:60-150`) |
| Watchdog arm | `rti_arm.c` at POST_KERNEL 47 (`common/rti_arm.c:233`) or the SafeUI feeder on kexec (`common/Kconfig:34-87`) | `gc_wdog_arm.c` at PRE_KERNEL_1 1 (`:124`); `wdog_map` PRE_KERNEL_1 0 (`gc_sample.c:44`) | `rpi_wdog.c` at POST_KERNEL 0 (`:80`) |
| Watchdog bound | `YAKOGL_AM62P_WDT_SECONDS` | `GC_BOARD_WDOG_SECONDS` (300) | `RPI_WDOG_HOLD_S` (50, `rpi_wdog.h:14`), per sample |
| End of run | `pvr_sample_finish(rc)`: hold `PVR_SAMPLE_HOLD_SECONDS`, `sys_reboot` (`sample_glue.c:233-249`) | `gc_sample_finish()`: hold 60 s / 40 s on fail / hold codes, PSCI reset (`gc_sample.c:172`) | no call: `for (;;) k_sleep` and the watchdog resets (`v3d_gl_triangle/src/main.c:517`) |
| Verdict rule | sample prints its own | `gc_sample_verdict()`: a GPU reset turns PASS into FAIL (`gc_sample.c:146`) | sample prints its own |
| Fatal handler | resets, writes FAULT if SNVS (`sample_glue.c:261`) | second definition (`gc_sample.c:455`); deqp turns one off (`deqp/boards/verdin_imx8mp_mimx8ml8_a53.conf:17`) | none |
| Early init | build line PRE_KERNEL_2 (`sample_glue.c:50-55`) | GIC EOI EARLY (`gc_board_gic.c:39`), IRQ guard PRE_KERNEL_1 50 (`gc_irq_guard.c:90`), SNVS map PRE_KERNEL_1 0 (`gc_result.c:50`) | none |
| Build id | `sample.cmake` `PVR_SAMPLE_TREE` | `GC_BUILD_STAMP` (time) + the same | `V3D_BUILD` in each CMakeLists |
| Kconfig glue | `common/Kconfig` (`PVR_SAMPLE_*`) | `gc_common/Kconfig` + `gc_board/Kconfig` (`GC_SAMPLE_*`, `GC_COMMON_*`, `GC_BOARD_*`) | CMake `option(V3D_RPI_WDOG)` + per-sample Kconfig |

Board code also sits in the wrong places. The Pi 4's board support lives in
a *sample* (`v3d_clear/src`) and is pulled in by path. The i.MX8MP's lives
in the driver module. The AM62P's lives in `common/`.

### 1.4 The Slint check: the exact differences

| Sample | First-frame state | Window | Check | Tolerance |
|---|---|---|---|---|
| `gl_slint` | `demo_vehicle(0,0)`, 2 frames (`main.c:340-344`) | 1024x600 | **none** (no read-back in `main.c`) | - |
| `rust_slint` | own `drive_cycle` | 1024x600 | 5 points printed, not judged (`lib.rs:390-418`) | - |
| `gc_slint` | `demo_vehicle(0,0)` under PARITY | 1280x800 panel; 1024x600 only with `prj_parity.conf`, never scanned out (`Kconfig:55-63`) | 5 points, PROBES record | 12 per channel, 3 of 5 must match (`main.c:2378,2400`) |
| `v3d_gl_slint` | `demo_vehicle` t=0 | 1024x600 + guard rows | 5 points against the AM62P values, sentinel, alpha, guard rows, glReadPixels = memory; window CRC-32 printed (`main.c:348-359,593`) | 1 |

The expected values disagree. For "track" `rust_slint/src/lib.rs:396`
expects (20,26,34), while `v3d_gl_slint/src/main.c:357` holds the AM62P's
measured (13,17,24).

### 1.5 The result sinks

| Sink | Capacity | Survives reset | Read by | Limits |
|---|---|---|---|---|
| i.MX8MP SNVS LPGPR0..3 (`gc_result.c:11`, 0x30370090) | 16 bytes, last write wins; 12 bytes with `KEEP_LPGPR0` | yes (battery domain) | Linux nvmem `30370000.snvs:snvs-lpgpr0` | one record per boot; a selector travels in (`gc_result.h:212-220`) |
| AM62P UART (ESP32 bridge, `UART_ADDR`) | unbounded text | n/a (captured live) | `tools/am62p_netboot_uart.py`, `tools/cts/deqp-uart-tally.py` | needs the bridge up; one client |
| Pi 4 UART (Debug Probe) | unbounded text | n/a | the tryboot runner's reader (now `board.sh`), `deqp-uart-tally.py` | none from the code |
| Pi 4 record screen | 30 lines x 158 columns (`v3d_gl_test/src/main.c:75-76`) | until the watchdog reset | a camera | read by eye; lost on reset |

AM62P: a persistent register was searched for. `driver/rust/src/soc/am62p/regs.rs`
(the hand-written TRM map) and `driver/dts/arm64/ti/ti_am62p54_a53.dtsi`
define no RTC or CTRL_MMR scratch register, and nothing under `driver/` names
one. Whether SPRUJ83D has a scratch register that survives an RTI reset and
that Linux can read is **unmeasured**. It needs a by-hand TRM read (the
`tools/ti-trm/check_regs.py` route) and then a board test. Until then the
AM62P carries the record on the UART.

### 1.6 The runners

The five runners this inventory found, and the defects found while reading
them, are in `ab43a20c5:tools/cts/log/board-sessions/one-method-acceptance-2026-10-03.md`.
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

`board_watchdog_arm()` is **not** an application call. This departs from the
brief. On all three boards the arming runs from `SYS_INIT` before `main()`
(`gc_wdog_arm.c:124`, `rti_arm.c:233`, `rpi_wdog.c:80`), so that a hang in a
driver's init (GPU firmware, display) is covered too. An arm called from
`main()` would leave that part unguarded. The arm stays automatic, and the
bound is one option, `CONFIG_YAKOGL_BOARD_LIFETIME_S`.
`board_watchdog_feed()` is the `rpi_wdog_alive()` hook (`rpi_wdog.c:49-58`)
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
buffer, as `v3d_gl_slint` does today.

**The scan-out view (`CONFIG_YAKOGL_BOARD_SCANOUT_VIEW`, default n).** With
the knob on, the window is a view into the scan-out buffer: `stride` is the
screen's pitch and `board_show()` copies nothing. YakoGL takes such a window
(branch `unify/stride-view`, 3473586e): EGL accepts a native WINDOW whose
pitch is whole pixels, even, at least the packed pitch and at most 16384; the
GL core carries it per surface (`GlContext.default_pitch`,
`GlesDraw.target_pitch`, 0 = packed) and only for the default framebuffer.
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

This is the one present path: Step 5's `BOARD_WINDOW_SCANOUT` flag (the
window is the framebuffer itself, packed rows only) went when the view
landed, and the cluster sample turns the view on for every board. A
window of the panel's size is scanned in place on both Verdins with or
without it. Every copy that remains is counted (`board_show_copies()`),
printed once, and flagged in the APP record (`APP_SHOWN_BY_COPY`, w1 bit
16), so a clear bit 16 proves the run copied nothing.

Rust code reaches the same symbols through `extern "C"`, as `rust_slint`
does with `pvr_sample_*` today (`rust_slint/src/lib.rs:147-151`).

**Where the code goes** (`driver/zephyr/lib/board/`, `CONFIG_YAKOGL_BOARD`,
chosen by SoC):

| New file | Comes from |
|---|---|
| `common/board.c` (finish, verdict, build line, fatal handler, record line) | `common/sample_glue.c:50-55,233-279`, `gc_sample.c:146-230,455-` |
| `common/record.c` (C shim over the Rust encoder) | `gc_result.c:59-115` (rules), `gc_result.h` |
| `am62p/board_am62p.c` (window, DSS show, UART sink) | `common/sample_glue.c:79-211` |
| `am62p/rti_arm.c` | `common/rti_arm.c` (as is) |
| `imx8mp/board_imx8mp.c` (carve-out window, `imx8mp_display_on`, hold) | `gc_sample.c`, `gc_gl_triangle/src/main.c:278-305,337-345` |
| `imx8mp/sink_snvs.c`, `wdog_arm.c`, `irq_guard.c`, `gic.c`, `panel_console.c` | `gc_board/gc_result.c`, `gc_wdog_arm.c`, `gc_irq_guard.c`, `gc_board_gic.c`, `gc_panel_console.c` |
| `rpi4/board_rpi4.c` (mailbox fb window, copy-to-screen, clock max) | `v3d_clear/src/rpi_mbox.c`, `v3d_gl_slint/src/main.c` (window + copy), `v3d_gl_triangle/src/main.c:371-377` |
| `rpi4/wdog.c`, `rpi4/fbcon.c` | `v3d_clear/src/rpi_wdog.c`, `fbcon.c` |
| `rpi4/record_screen.c` (ring, freeze at end or deadline) | `v3d_gl_test/src/main.c:60-150`, `v3d_deqp/src/v3d_board.c` (tap) |

One Kconfig replaces three prefixes: `YAKOGL_BOARD_LIFETIME_S`,
`YAKOGL_BOARD_HOLD_S`, `YAKOGL_BOARD_HOLD_FAIL_S` and
`YAKOGL_BOARD_RECORD_SCREEN`. The `GC_*`, `PVR_SAMPLE_*` and `V3D_RPI_WDOG`
names stay as aliases until the last sample has moved, because the Qt
application outside this repository links `gc_board`
(`gc_board/README.md:4-8`). The aliases are removed in their own commit.

## 3. The record

The record is the format of `gc_result.h`, unchanged, on every board.
`w0 = 0x594B0000 | kind << 8 | low`, then w1..w3, written magic last. The
kind numbers 1-14 keep their values (14 is STAGE, a stage breadcrumb, from
`imx8mp/cold-entry`'s `gc_result.h:44`). `low` is a sequence number, except for
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
14, which collides with STAGE on `imx8mp/cold-entry`; 15 is free on that
branch, `am62p/netboot-scan-fix` and main. w1 is the CRC-32/IEEE of the whole window
(rows top-down, `crc32_ieee`), w2 is the frames drawn, w3 is `W << 16 | H`.
It carries `v3d_gl_slint`'s window CRC (`main.c:593`) as a fingerprint, not
as a verdict.

**Added:** kind 20 GPU_STOP, the verdict of a run the GPU driver ended:
the PowerVR render service found a kick it cannot complete
(`gpu::pvr::freelist::stop`, with the freelist grow knob only). The tag is
the cause (2 out of memory at the freelist's maximum, 3 a grow past the
host's table; 1 is not used), w1 the fence, w2 the GPU recoveries so far (the runner's
`resets`), and w3 is `list << 24 | KiB`. 19 is KICK_DIFF on
`vivante/gc-ab-knobs` and is reserved here.

**Added:** kind 22 DISPLAY, the i.MX8MP display chain while it scans a
window (gc_gl_triangle, `CONFIG_GC_GL_TRIANGLE_DISPLAY_RECORD`, default n;
issue #23). w1 is the pixel clock in kHz with bit 31 set when the LCDIF was
already scanning, w2 the DSI lane rate in kHz, w3 the SN65DSI84's IRQ_STAT a
second into the picture and at the start, the display health and the
LCDIF's format and pitch (record.rs has the bits; `yd_record_display_words`
packs them). It is written after PICTURE, so on the i.MX8MP it is the
record the SNVS keeps; its status is n/a, like PICTURE's.

**Defined once, in Rust:** `driver/rust/src/board/record.rs` (no_std) holds
the kinds, the packing (`case`, `probes`, `picture`, ...), the CRC-16 used
by PROBES and DEQP, and the decoder. The C shim `common/record.c` calls
`yd_record_pack_*` and hands the four words to the sink. The same module is
the host decoder, so writer and reader cannot drift.

**How each board carries it:**
- **Every board** prints `yk-record: %08x %08x %08x %08x` on its console at
  every write, through printk, so a log level cannot drop it. `gl_triangle`
  already does this under another prefix (`main.c:170`).
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
   board = pi4 | am62p | imx8mp       BOARD_BOOT = tryboot | uboot-tftp | kexec-dev
```

`tree` is added to the brief's verbs. Three scripts do it today (§4 table),
and the rule "repeat work becomes one tool" covers it.

**Boot backends:** `tryboot` (pi4) and `uboot-tftp` (am62p, imx8mp; it keeps
`tools/uboot-netboot-lib.sh` and `tools/tftp_oneshot.py`) produce rows.
`kexec-dev` (am62p, imx8mp; it keeps `driver/tools/kexec-zephyr/`) writes
`DEV` into every row it makes (bootloader-boot rule).

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
records, and each file with its sha256 and line count (`records/README.md`).

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
| `driver/tools/kexec-zephyr/*` (`kexec_run.sh`, `board_lock.sh`, `snvs_lpgpr.c`) | stay (kexec-dev backend, the lock, the SNVS reader) |
| `tools/cts/am62p-recover.sh` | deleted with pvrgpu.ko (2026-10-04): `board.sh` waits for Linux itself (a new boot_id), `kexec_run.sh` checks the RTI itself, and there is no module to load |
| `tools/cts/deqp-uart-tally.py` | stays as `tally`'s dEQP reader (already shared by AM62P and Pi 4); port to Rust later |
| `imx8mp-gltest-build.sh`, `imx8mp-deqp-build.sh`, `am62p-deqp-images.sh` | absorbed by a build step of `tree` |
| `cts-run.sh`, `cts-board-run.sh`, `cts-module-probe.sh` | deleted with pvrgpu.ko (2026-10-04; `tools/cts/README.md` says what replaced each) |
| `vendor-*.sh`, `gc-milestone.sh` | deleted with the Linux harnesses (2026-10-04) |
| `wedge-next-run.sh`, `stage-pair.sh` | deleted (2026-10-04) |
| `runrequest-am62p-headpair.sh` | deleted with pvrgpu.ko (2026-10-04) |

## 5. Migration, acceptance first

### Step 1: the same Slint demo on three screens

**The one source.** It is `gl_slint`'s crate and UI (`gl_slint/rust`,
`gl_slint/ui/cluster.slint`), with `v3d_gl_slint`'s frame sequence and
check, moved into the crate as Rust. The new sample is
`driver/samples/zephyr/slint/`. Why this source:
- The crate is already the one all three C samples build (§1.2). It is no_std
  and targets `aarch64-unknown-none` with `-Z build-std=core,alloc`
  (`gl_slint/CMakeLists.txt`), so it needs no Zephyr Rust module.
- `rust_slint` needs Zephyr's Rust support plus aarch64 patches that are not
  in this repository (`README.md`, "What else some samples need"). It calls
  AM62P-only glue (`lib.rs:147-151`) and has a wrong expected value
  (§1.4). It is all Rust, but it is not the one to build on.
- `gl_slint`'s C main has no check at all. `gc_slint` is 3879 lines of
  i.MX8MP modes (SafeUI ring, PERF pages, stages). `v3d_gl_slint` has the
  most complete check (§1.4) and a fixed state.

**"Same" means:**
- the same `cluster.slint` with the SafeUI `ui/1024x600` layout;
- a 1024x600 window (it fits all three panels: 1024x600, 1280x800,
  1920x720) shown at (0,0);
- `demo_vehicle` at t = 0, two `slint_draw_frame` calls, then
  `CONFIG_YAKOGL_BOARD_ANIMATE_S` of animation.

Every board leaves a PROBES record (kind 6) with status 1, at one tolerance
(1 per channel) against one reference table, the AM62P's values in
`v3d_gl_slint/src/main.c:353-357`. It also leaves a FINGERPRINT record. Equal
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
  `imx8mp/cold-entry` (`18487926`) the STAGE breadcrumbs named the stage
  (`died_after=console_lcdif_read`). The fix is the reachability guard
  `soc::imx8mp::media::lcdif_unreachable` (`driver/rust/src/soc/imx8mp/media.rs:202`
  on that branch, now `driver/rust/src/ffi.rs` `yd_imx8mp_lcdif_unreachable`
  on main). With it, netboot and kexec both give the baseline PICTURE
  `594b040e ff403f80 35706c4e 04000258`.
  Still unmeasured: whether the LCDIF can
  scan out a 1024x600 window on the 1280x800 panel, or whether the window
  must be copied (`imx8mp_display_on` takes a full-panel buffer and a
  pitch).

**Deleted at the end of step 1:** `gl_slint/src/main.c`, `rust_slint/`,
`v3d_gl_slint/src/main.c` and `CMakeLists.txt` (its `host/` recorders move to
`slint/host/`), `gc_slint/` (its ring and PERF modes need an owner decision:
move them under board-layer options or retire them), and the old runners
(the table in section 4). Done in Step 1: the runners and the Python decoder
were deleted; `gl_slint`'s UI and no_std patches moved to `slint/ui` and
`slint/nostd`, `v3d_gl_slint`'s host recorders to `slint/host` (the Slint
recorder now drives the one sample's crate); nothing outside `gc_slint` used
its ring or PERF modes, so they went with it (and gc_common's ring hook).
`gl_slint/rust` stays, with `gl_slint/ui` a link to `slint/ui`: the SafeUI
repository's Supervisor builds that crate by path (`gl_slint/README.md`); it
goes once that user has moved.

### Step 1 acceptance run

The one `slint` sample ran once per board from the bootloader boot path on
2026-10-03; the table is in
`ab43a20c5:tools/cts/log/board-sessions/one-method-acceptance-2026-10-03.md`. The
PROBES record was identical on the two Verdins and one LSB of green apart on
the Pi 4, inside the tolerance of 1. After that run the old runners were
deleted and `tools/board.sh` became the one runner ("Done in Step 1" above).

### Steps 2-6 (each step: one source, all three boards, one row per board, then delete)

2. **test**: `gl_test/` becomes the one sample. Case selection comes from
   Kconfig or the selector. CASE and SWEEP records go everywhere, and the
   record screen comes from the board layer. Deleted: `gc_gl_test/`,
   `v3d_gl_test/`, `tools/cts/imx8mp-gltest-build.sh` (the failure-name pager already went in Step 1).
3. **triangle**: `gl_triangle/` becomes the one sample, with one window size.
   The PICTURE CRCs only compare when W and H agree (`gc_result.h:89-90`).
   Today they are 64x64 (`gl_triangle/Kconfig`), 1280x800
   (`gc_gl_triangle/prj.conf`) and 1280x360 (`v3d_gl_triangle`, half the
   fb). `v3d_gl_triangle`'s whole-window classification becomes the common
   check. Deleted: `gc_gl_triangle/`, `v3d_gl_triangle/`, `rust_triangle/`.
4. **femtovg**: one sample from `gl_femtovg` plus `v3d_gl_femtovg`'s
   program check (`fvg_programs.c`). The i.MX8MP gets femtovg for the first
   time. Deleted: `v3d_gl_femtovg/`.
5. **cluster**: `v3d_gl_cluster` becomes `gl_cluster` on all three boards
   (the UI scales itself with fit-width, `v3d_gl_cluster/src/main.c:10-16`).
   Deleted: `v3d_gl_cluster/` (its `host/` tools move along).
   Done (branch `unify/cluster`, rows below): the sample is
   `driver/samples/zephyr/cluster/`. Its `rust/`, `ui/`, `host/` and
   `font-sizes.txt` were git-moved from `v3d_gl_cluster/`, which is deleted
   after the three rows, with its symlinks, its C main and present
   accounting (and their host check), its `ref_probes.h` (whose 1280x720
   table had the reference's transform-scale defect), the crate's STRESS
   and gradient-pool entry points and `host/ui_omit.py` (its measurement
   modes). Its default build (demo off) no longer compiled since `baf8588b`
   (`v3d_tex_waits` declared only under the demo option); that goes with it.
   - The window is the panel's own size (1024x600, 1280x800, 1920x720,
     `boards/*.conf`), in two halves of one `board_window()`. The animation
     keeps one frame in flight and shows each half once its fence is seen:
     a flip on the Verdins, a pan on the Pi 4, whose two halves are the
     firmware framebuffer's (no copy on any board).
   - The check is `cluster/check` (Rust): the host reference's probe points
     within 12 per channel, one table per panel size (`fit-width` puts a
     design point on a different pixel on each panel), all made by
     `host/ref/run.sh`.
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
   - Not carried over (V3D-only measurement modes of `v3d_gl_cluster`): the
     femtovg program survey (Step 4's), the GL call counters, the OSAL job
     split, OMIT, STRESS, the render probe and the paced loop.

   The Step 5 acceptance run of 2026-10-03 (AM62P and Pi 4 PASS; the
   i.MX8MP row pending then) is in
   `ab43a20c5:tools/cts/log/board-sessions/one-method-acceptance-2026-10-03.md`.
6. **deqp**: `deqp/` keeps its source. `v3d_deqp/src/v3d_board.c` (tap,
   record) and `deqp/src/deqp_snvs.c` go to the board layer's sinks. The
   tree runner replaces the chains (deleted already in Step 1). Deleted:
   `v3d_deqp/`, and the build scripts once `tree` builds.

After step 6 the aliases (§2) go, along with `common/sample_glue.*`,
`gc_common/` and `v3d_clear/src/{rpi_mbox,fbcon,rpi_wdog}.*`, which have
then moved. Driver bring-up samples (`gc_identify`, `gc_fence`, `v3d_clear`,
`v3d_triangle`, ...) are not GL purposes and are out of scope.

## 6. Unmeasured, and what this design is least sure of

- An AM62P register that survives the RTI reset and that Linux can read
  (§1.5). Until then, the UART line.
- Whether the three GPUs draw bit-equal cluster pixels. The acceptance uses
  probes at tolerance 1. Whether the i.MX8MP meets 1 (it uses 12 today) is
  unmeasured.
- Whether a 1024x600 window can be shown on the i.MX8MP's 1280x800 panel
  without a copy (the Pi 4's view is measured:
  `ab43a20c5:tools/cts/log/benchmarks-baseline/glmark2-first-runs-2026-10.md` 5d,
  `tools/cts/RESULTS.md` "Scan-out view"), and whether the AM62P's cold display path
  (`am62p-display-cold.md`) shows the window after a U-Boot entry. Both are
  unmeasured. (The i.MX8MP cold-entry death is fixed, §5 step 1c.)
- `tools/check.sh` builds one gl_test image per GPU (`gl_test`,
  `gc_gl_test`, `v3d_gl_test`, with `-Werror` on our C), but not the one
  `slint` sample. Building it for three boards needs a gate row of its own,
  or the three-board claim is untested between board runs.
