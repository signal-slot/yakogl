# yakogl_gc_board -- board support for an image kexec'd onto the Verdin iMX8M Plus

The Zephyr library the driver module builds when `CONFIG_YAKOGL_GC_BOARD=y`.
It started as the i.MX8MP samples' `gc_common` and moved here (2026-09-23) so
that an application outside this repository (Qt on the i.MX8MP) links the same
watchdog, fatal handler, interrupt guard, SNVS record and panel console as the
samples do. The samples turn it on through `samples/zephyr/gc_common/Kconfig`.
The option names keep their `GC_SAMPLE_*` / `GC_COMMON_*` prefixes.

## The bounded lifetime (`gc_sample.c`)

The samples boot with kexec out of Torizon, so Linux is gone and nothing can
reset the board remotely. `gc_sample_finish()` services the watchdog Linux left
armed for `CONFIG_GC_SAMPLE_HOLD_SECONDS` (60), then resets the SoC through
PSCI so the boot loader brings Linux back. Sixty seconds is the time it takes
to read the panel and photograph it. Every sample must call it on every path,
failures included.

## WDOG1 armed by the image (`gc_wdog_arm.c`, opt-in)

`CONFIG_GC_BOARD_WDOG_SELF_ARM` (default n) is for an image U-Boot starts with
`go` (docs/standalone-boot.md, "What the image does on a `go` entry"), where no
earlier stage can be relied on to have armed WDOG1. At PRE_KERNEL_1 it arms
WDOG1 for 60 s from whatever state it finds (the register logic is driver/rust
`ip::imx_wdog`), leaves a kind WDOG record saying what it found, and from then
on every feeder here stops at `CONFIG_GC_BOARD_WDOG_SECONDS` of uptime. A
refusal resets the SoC at once. `CONFIG_GC_BOARD_WDOG_PROOF` builds the image
that proves on the board that WDOG1 really resets it.

## The interrupt guard (`gc_irq_guard.c`)

A kexec'd image inherits a GIC the previous kernel left configured, and on the
i.MX8MP it dies on an interrupt no Zephyr driver owns within its first minute.
The guard walks every line still pointing at `z_irq_spurious` and gives it a
handler that disables the line, counts it and prints it. It runs at
`PRE_KERNEL_1` priority 50 -- *before* every driver, and immediately after the
GIC, which a `BUILD_ASSERT` enforces. Before the GIC driver has recorded this
CPU's redistributor address, setting a line's priority writes through a null
pointer and faults at 0x10400. Running first is safe because the static ISR
table is complete at link time, so a line an `IRQ_CONNECT` driver owns is
already spoken for and gets skipped.

**What it can and cannot take, because this has been misread once.** The guard
claims a line with `irq_connect_dynamic`, so `GC_COMMON_IRQ_GUARD` selects
`DYNAMIC_INTERRUPTS`; if that select ever goes, the guard still builds and
still prints, and covers nothing. But it can only claim lines that are *still
spurious* when it runs, and a driver that connects with the `IRQ_CONNECT`
macro is not one of them: that macro is resolved by `gen_isr_tables` at build
time, so the entry is already the driver's before any code runs.

Two lines that get blamed for hangs are connected exactly that way, and
neither can be the guard's doing:

* the **arch timer** (`arm_arch_timer.c`), so a dead system tick is not this;
* the **Vivante GPU** (`gc_irq_connect` in `drivers/gpu/gpu_vivante_gc_rust.c`;
  `src/vivante/gc_hw.c` when this was checked), which uses
  `IRQ_CONNECT` and then calls `irq_enable` itself, so a GPU init that waits
  forever for an interrupt is not this either. Checked 2026-09-22 against every image then in the tree
-- `gc_slint`, its route-3 variants and the `gc_gl_test` builds all had
`IRQ_GUARD=y DYNAMIC_INTERRUPTS=y`.

Reading the config out of a build directory tells you about that build and
nothing else: a directory holding a deliberate `IRQ_GUARD=n` run shows
`DYNAMIC_INTERRUPTS` unset for exactly that reason, which is how the misreading
happened.

## The panel console (`gc_panel_console.c`, `gc_font8x16.h`)

**This board has no console but the picture.** The Verdin iMX8M Plus on a
Mallow carrier leaves the debug header X11 unassembled, so there is no
reachable UART, and every RAM channel measured on 2026-09-20 is gone by the
time a kexec'd Zephyr runs:

| channel | why it does not work |
| --- | --- |
| SNVS LPGPR | secure; writes are ignored |
| SRC_GPR8/9 | TF-A owns them; writes are masked |
| OCRAM | rewritten by ATF and U-Boot |
| DDR | the SoC reset is a PMIC power cycle, so a marker comes back as noise |

What survives the kexec jump is the scan-out. Linux leaves the LCDIFv3 at
`0x32e80000` (Linux's device tree calls it `lcd-controller@32e80000`, the
instance wired to the DSI bridge) reading a linear XRGB8888 framebuffer and
sending it to the 10-inch DSI panel, and it keeps doing so after the jump.

So the console reads that controller instead of configuring it:

1. `CTRLDESCL0_5` (0x214) for the layer enable and the pixel format,
   `DISP_PARA` (0x10) for the display enable, `CTRLDESCL0_1` (0x200) for width
   and height, `CTRLDESCL0_3` (0x208) for the stride, and
   `CTRLDESCL_LOW0_4`/`HIGH0_4` (0x20c/0x210) for the address Linux left.
   Offsets and field names are mainline Linux
   `drivers/gpu/drm/mxsfb/lcdif_regs.h`; each one is cited again next to the
   `#define`.
2. It clears a framebuffer of its own with the same geometry and points the
   controller at it with the flip the Linux driver performs
   (`lcdif_plane_primary_atomic_update` then `lcdif_crtc_atomic_flush` in
   `lcdif_kms.c`): the two address halves, then `CTRLDESCL0_5_SHADOW_LOAD_EN`,
   then wait for the hardware to clear that bit. If it never clears, the old
   address goes back and the console draws into Linux's buffer instead. The
   picture is the only channel there is, so the code must not be able to lose
   it.
3. `__printk_hook_install()` renders characters with an 8x16 bitmap font.
   160x50 characters on this panel, scrolling by moving the picture up one
   text line.
4. Every pixel written is cleaned out of the data cache, per glyph row, because
   the LCDIF reads DDR and knows nothing about our caches. Per glyph rather
   than per line so that a `printk()` without a trailing newline -- the last
   thing a hang prints -- still reaches the panel.

Timing, clocks, the pixel format, the DSI bridge and the panel are never
touched. The only registers written are the two halves of the scan-out address
and the shadow-load bit.

The first line printed names the sample, the board and the build stamp, so a
photograph identifies the run.

### Configuration

`CONFIG_GC_COMMON_PANEL_CONSOLE` is on by default. A sample that uses it must
also set, in its `prj.conf`, `CONFIG_UART_CONSOLE=n` (the board defconfig turns
it on and a defconfig beats a Kconfig default) and `CONFIG_LOG_MODE_MINIMAL=y`
(so `LOG_*` goes through `printk`). See `Kconfig` for the framebuffer size and
the glyph zoom.

Every build prints one CMake warning, `No SOURCES given to Zephyr library:
drivers__console`. That is the expected shape of this arrangement: the console
driver lives in the application, so Zephyr's own `drivers/console` library ends
up empty once `CONFIG_UART_CONSOLE` is off. Nothing is lost by it.

### The font

`gc_font8x16.h` is generated by `gen_font8x16.py` from the X11 "misc-fixed"
8x13 font that every distribution ships as `/usr/share/fonts/misc/8x13.pcf.gz`
(package `font-misc-misc`). Its PCF `COPYRIGHT` property reads "Public domain
font.  Share and enjoy.", so the bitmaps carry no licence of their own. The
13-row glyph box sits in a 16-row cell, one blank row above and two below.

## Status

Built for `verdin_imx8mp/mimx8ml8/a53`, and run on that board: the bounded
lifetime, the panel console and the interrupt guard are all exercised by every
kexec run the `gc_gl_test` and `gc_slint` work does. This line used to say no
board had ever run any of it; it had been false for a while.
