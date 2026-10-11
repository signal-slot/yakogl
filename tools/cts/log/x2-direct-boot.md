<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Direct boot on the Armadillo-X2: Zephyr as TF-A's BL33

The Armadillo-X2 powers on into a Zephyr image, with no Linux and no kexec:
Zephyr takes the place of U-Boot proper as TF-A's BL33. The target is about
0.5 s from power-on to the first frame.

Status (2026-10-11): an unrun plan, moved from `docs/direct-boot.md`. The
Armadillo-X2 is not one of the three boards of `README.md`, and
`tools/cts/RESULTS.md` (the direct-boot rows) has it as NOT RUN. The board
variant, the flash tool and the sample configuration it describes are in the
tree: `driver/boards/atmark/armadillo_x2/`, `tools/x2-mkflash.py`,
`driver/samples/zephyr/gc_gl_triangle/boards/armadillo_x2_mimx8ml8_a53_direct.conf`.

State: the image is built and checked offline. **It has not run on the
board**, and the board's eMMC has not been read (see "Risks and open
questions"). The first image is `gc_gl_triangle`, which renders off-screen and
reports on UART2. It shows no picture: the HDMI bring-up from cold (LCDIF3 ->
PVI -> DW-HDMI) is written, with host tests, but has never run and is not
part of this chain (`driver/drivers/display/display_yakogl_imx8mp_hdmi.c`,
`driver/docs/x2-hdmi-cold.md`).

The three boards of `README.md` boot standalone through their own boot
loaders instead (`docs/standalone-boot.md`).

## The chain

```
Boot ROM ── reads the imx-boot image (IVT at image offset 0) from the boot device
  └─ U-Boot SPL 2020.04-at31 (OCRAM 0x920000): DDR training, PMIC, UART2 banner
       └─ loads the FIT at image offset + 0x58000 through the ROM API:
            uboot-1 = zephyr.bin   -> 0x40200000   (no entry property)
            fdt-1   = Atmark's DTB -> appended after uboot-1 (unused by Zephyr)
            atf-1   = TF-A BL31    -> 0x970000, entry
            tee-1   = OP-TEE       -> 0x56000000, entry
       └─ jumps to the first loadable that has an entry point: BL31
            └─ BL31 v2.4 (imx-atf, SPD=opteed) runs OP-TEE's init, which returns
            └─ enters BL33 = 0x40200000 in non-secure EL2, MMU/caches off, DAIF masked
                 └─ Zephyr: __start (image header word 0 is "b __start"),
                    EL2 -> EL1, gc_gl_triangle
```

U-Boot proper is gone. Its slot in the FIT holds Zephyr. Every other part of
Atmark's image is copied unchanged.

## What the prebuilt image is

The image is Atmark's prebuilt `imx-boot_armadillo_x2_2020.04-at31`, the
current "latest", from <https://download.atmark-techno.com/armadillo-x2/bootloader/>:

| file | sha256 |
|---|---|
| `imx-boot_armadillo_x2_2020.04-at31` (1,839,472 bytes) | `c1f164e95eba0ebc29d9b75e4552218d06528f86395c554dd526bd3b820e557e` (matches Atmark's `.sha256`) |
| `imx-boot_armadillo_x2_2020.04-at30` (3,466,992 bytes), what ABOS 3.24 runs | `65c677402c8f8a167527a183952804ff2924e5d1328d47deb21053bf379034aa` |

at31 rather than at30, for one reason. at30 carries OP-TEE as a 2.1 MB padded
binary, so at30 plus a 2.4 MB Zephyr is about 5 MB. That does not fit the eMMC
boot partition, which holds U-Boot's environment at 0x3FA000 and 0x3FE000. at31
switched to `tee-raw.bin`; its Changelog says "imx-boot size reduced by 1.5MB",
and its OP-TEE is 0.48 MB. BL31 is byte-identical in both. at31's other change
is to U-Boot proper ("x2 chosen/fallback-boot support"), which this chain does
not run. The SPL banner therefore says at31, while the board's current U-Boot is
at30.

Layout of the at31 image (`tools/x2-mkflash.py info`):

```
IVT @0x0: entry 0x920000 self 0x91ffc0 boot_data 0x91ffe0 csf 0x94a5c0
boot data: start 0x91ffc0 length 0x2c660 (what the ROM loads: SPL + DDR firmware)
FIT @0x58000: header 1031 bytes, external data (data-position)
  uboot-1 "U-Boot (64-bit)"      standalone pos 0x005000 size 0x0d8790 load 0x40200000 entry -
  fdt-1   "evk"                  flat_dt    pos 0x0dd790 size 0x0079f0
  atf-1   "ARM Trusted Firmware" firmware   pos 0x0e5180 size 0x00b150 load 0x970000   entry 0x970000
  tee-1   "TEE firmware"         firmware   pos 0x0f02d0 size 0x078ea0 load 0x56000000 entry 0x56000000
config-1 (default): firmware=uboot-1, loadables=atf-1, tee-1, fdt=fdt-1
```

The FIT also has an i.MX IVT at FIT + 0x1000 for HAB. SPL checks it only when
HAB is closed (`board_spl_fit_post_load`). With HAB open, a FIT without an IVT
passes silently. The new FIT has none.

### Facts from Atmark's source, imx-boot-2020.04-at30.tar.gz

The source tarball was read, but nothing was built from it. Building would
need the NXP firmware EULA.

- TF-A `plat/imx/imx8m/imx8mp`: `PLAT_NS_IMAGE_OFFSET 0x40200000` is the BL33
  entry, and `get_spsr_for_bl33_entry()` returns EL2 with SP_ELx and all
  exceptions masked. `BL32_BASE 0x56000000` and `BL32_SIZE 0x2000000` are the
  OP-TEE region; they are passed to BL33 in x1/x2, and x3 = `BL32_FDT_OVERLAY_ADDR`
  = 0x43200000. Zephyr reads none of x0-x3. `IMX_BOOT_UART_BASE 0x30890000` is
  UART2, so BL31 prints there. The build uses `SPD=opteed`, so BL31 always runs
  BL32 at 0x56000000 first, and **OP-TEE has to stay in the FIT**: without it,
  BL31 would jump into empty DDR. BL31 also writes 0 to 0x43200000 before BL33
  runs, and `bl31_platform_setup` releases the Cortex-M7 from reset.
- U-Boot `spl_imx_romapi.c`: the FIT is read from `image_offset +
  CONFIG_SYS_MMCSD_RAW_MODE_U_BOOT_SECTOR * 512 - 0x8000` = image offset +
  0x58000. `x2_defconfig` has `CONFIG_SYS_TEXT_BASE=0x40200000`,
  `CONFIG_IMX_HAB=y`, `CONFIG_ENV_OFFSET=0x3FE000`, and
  `CONFIG_ENV_OFFSET_REDUND=0x3FA000`.
- `spl_load_simple_fit`: an image with no `entry` hands the jump to the first
  loadable that has one. The "firmware" gets the FIT's `fdt` appended at
  `load + size`, and SPL fails if there is none. So `fdt-1` stays, and the
  Zephyr payload is padded to 16 bytes.
- Where the image lives on the eMMC: Atmark's installer (`flash_image.sh`)
  writes the bootloader to the boot partition and selects it with
  `mmc partconf ... 1 0`. ABOS updates (mkswu `pre_boot.sh` / `post_boot.sh`)
  keep two copies, in `mmcblk2boot0` and `mmcblk2boot1`, write the inactive one
  at offset 0, and flip `mmc bootpart enable N 0`. On an SD card the image
  sits at 32 KiB. The "image offset 0x8000" in Atmark's SPL log is that
  SD/user-area case.

## The Zephyr side: `armadillo_x2/mimx8ml8/a53/direct`

The board variant lives in `driver/boards/atmark/armadillo_x2`
(`armadillo_x2_mimx8ml8_a53_direct.{dts,yaml,_defconfig}`). The kexec variant
is unchanged.

- **Link address** 0x40200000 (`zephyr,sram`, 256 MiB), so
  `CONFIG_SRAM_BASE_ADDRESS` and `CONFIG_KERNEL_VM_BASE` are 0x40200000. The
  crash-window address 0x60000000 is not used.
- **DRAM map** (2 GiB at 0x40000000):

  | range | owner |
  |---|---|
  | 0x40000000 - 0x401fffff | below the entry; SPL's saved DDR timing for TF-A (`SAVED_DRAM_TIMING_BASE`) |
  | 0x40200000 + 256 MiB | Zephyr. zephyr.bin is 2.4 MB; `_end` is 0x42779000 |
  | 0x43200000 | TF-A/OP-TEE write here **before** BL33 runs, so zephyr.bin must stay under 48 MiB |
  | 0x56000000 + 32 MiB | OP-TEE, secure, never mapped |
  | 0x80000000 + 64 MiB | GPU carveout (the same as the kexec variant) |

- **EL2 entry**: Zephyr's `reset.S` handles EL3, EL2 and EL1 entry.
  `z_arm64_el2_init` drops to EL1. The SoC's `ARM64_BOOT_DISABLE_DCACHE`
  (default y for this SoC, "when booting from EL2") covers the cache state.
  `CONFIG_ARMV8_A_NS=y` matches the non-secure entry.
- **GIC**: TF-A initialised GICv3 for the secure side, and
  `GIC_SAFE_CONFIG=n` re-initialises the non-secure distributor, as on the
  kexec route. The secondary cores start through PSCI CPU_ON, which TF-A
  serves.
- **Timer**: SPL starts the system counter and programs CNTFRQ to 8 MHz, which
  is the SoC's `SYS_CLOCK_HW_CYCLES_PER_SEC` default.
- **No display node**: nothing has powered MEDIAMIX or clocked LCDIF3, and the
  adopt driver would read a dead block. The console is UART2.
- **The GPU** is powered by the driver itself, through the GPC and CCM
  (`gpu::vivante::power` in driver/rust); its clock parent is SYS_PLL1, which
  SPL enables.

`gc_gl_triangle` has `boards/armadillo_x2_mimx8ml8_a53_direct.conf` (UART
console, no panel console, no display driver). It also prints the system
counter at `main()` and at the verdict. On this route nothing restarts that
counter after SPL starts it, so the value is close to the time since power-on,
minus the Boot ROM.

Build it (`-DZEPHYR_MODULES` as in `driver/samples/zephyr/README.md`; a
nightly cargo for the Rust parts):

```sh
RUSTUP_TOOLCHAIN=nightly west build -p always -b armadillo_x2/mimx8ml8/a53/direct \
    -d build/x2/gc_gl_triangle_direct yakogl/driver/samples/zephyr/gc_gl_triangle
```

The two warnings this build prints are the kexec variant's too:
`GC_COMMON_PANEL_CONSOLE_SCALE` is assigned while the panel console is off, and
"No SOURCES given to Zephyr library: drivers__gpio".

## Assembling the image

```sh
tools/x2-mkflash.py build --base imx-boot_armadillo_x2_2020.04-at31 \
    --bl33 build/x2/gc_gl_triangle_direct/zephyr/zephyr.bin -o build/x2/flash-zephyr.bin
```

The tool uses the Python standard library only; no `mkimage` or `dtc` is
needed. It copies the first 0x58000 bytes (IVT, SPL, DDR firmware) unchanged.
It writes a new FIT with the same nodes and properties: the firmware's
description becomes "Zephyr (BL33)", the data positions start at 0x3000 and
are 4 KiB-aligned, and `uboot-1` carries zephyr.bin. It then reads the result
back and checks four things: the payloads, the load addresses, that the SPL
region is identical, and that the image stays under 0x3FA000, the environment.

The FIT writer was checked by rebuilding at31 with its own U-Boot and
decompiling both FITs with `dtc`. The only differences are the data positions
and the description. The rebuilt image is kept as
`build/x2/flash-uboot-repacked.bin`, a control: if it boots ABOS, the packing
is right.

The result on 2026-10-01 (from `a53/direct` at this commit):

```
flash-zephyr.bin: 3415712 bytes (0x341ea0)
  uboot-1 "Zephyr (BL33)" pos 0x003000 size 0x259990 load 0x40200000 entry -
  fdt-1   pos 0x25d000 size 0x0079f0
  atf-1   pos 0x265000 size 0x00b150 load 0x970000   entry 0x970000
  tee-1   pos 0x271000 size 0x078ea0 load 0x56000000 entry 0x56000000
uboot-1 word 0: 0x14012be3 = b 0x4024af8c = __start; "ARM\x64" at 0x38
```

## At the board

### 1. Read-only checks (root shell on ABOS)

```sh
cat /sys/block/mmcblk2boot0/size /sys/block/mmcblk2boot1/size   # 512-byte sectors; 8192 = 4 MiB expected
mmc extcsd read /dev/mmcblk2 | grep -i -A2 'PARTITION_CONFIG'    # "Boot partition N enabled" = active copy
dd if=/dev/mmcblk2boot0 bs=4 count=1 2>/dev/null | xxd             # d1002041 = an IVT at offset 0
dd if=/dev/mmcblk2boot1 bs=4 count=1 2>/dev/null | xxd
hexdump -C -s 0x1c -n 4 /sys/bus/nvmem/devices/imx-ocotp0/nvmem  # bank 1 word 3: bit 25 (0x02000000) set = HAB CLOSED
```

**Stop if HAB is closed.** SPL would then refuse the unsigned FIT. Otherwise,
call the active boot partition A (N = 1 means boot0, 2 means boot1) and the
other one B.

### 2. Back up, write the inactive copy, flip

```sh
# laptop (BOARD = how ssh reaches it: armadillo.local, or -P 2222 root@localhost via the tunnel)
scp build/x2/flash-zephyr.bin root@BOARD:/tmp/
# board
dd if=/dev/mmcblk2boot0 of=/tmp/mmcblk2boot0.bak bs=1M
dd if=/dev/mmcblk2boot1 of=/tmp/mmcblk2boot1.bak bs=1M
# laptop: keep the backups (/tmp on the board is lost at power-off)
scp 'root@BOARD:/tmp/mmcblk2boot*.bak' build/x2/
# board, B = the inactive partition, e.g. boot1 when boot0 is active
echo 0 > /sys/block/mmcblk2boot1/force_ro
dd if=/tmp/flash-zephyr.bin of=/dev/mmcblk2boot1 bs=1M conv=fsync
echo 1 > /sys/block/mmcblk2boot1/force_ro
cmp -n 3415712 /tmp/flash-zephyr.bin /dev/mmcblk2boot1 && echo written
mmc bootpart enable 2 0 /dev/mmcblk2        # 2 = boot1; use 1 for boot0
poweroff                                     # then remove and restore power
```

The write ends at 3.4 MB, so U-Boot's environment at 0x3FA000 and 0x3FE000 is
untouched, and partition A still holds the untouched at30 image.

### 3. What to look for on `/dev/ttyUSB0` (UART2, 115200 8N1)

The following prints each line with the seconds since the first byte, which is
SPL's banner, so it does not include the Boot ROM's time:

```sh
stty -F /dev/ttyUSB0 115200 raw -echo
python3 -u -c '
import os, time
fd = os.open("/dev/ttyUSB0", os.O_RDONLY); t0 = None; buf = b""
while True:
    b = os.read(fd, 4096); now = time.monotonic(); t0 = t0 or now; buf += b
    *lines, buf = buf.split(b"\n")
    for l in lines: print("%8.3f %s" % (now - t0, l.decode(errors="replace").rstrip()))
' | tee build/x2/boot-$(date +%H%M%S).log
```

The expected sequence:

1. `U-Boot SPL 2020.04-at31 ...`, DDR, then `Trying to boot from BOOTROM` and
   `image offset 0x0, pagesize 0x200, ivt offset 0x0` (boot partition; 0x8000 on SD).
2. `NOTICE:  BL31: v2.4(release):2020.04-at28`, then OP-TEE's `I/TC:` lines.
3. `*** Booting Zephyr OS build ... ***`, then `gc_gl_triangle: 1280x800 window ...`,
   `system counter at main(): N us`, and the GPU lines, then
   `system counter at the verdict: N us` and `PASS` or `FAIL`.

After `GC_SAMPLE_HOLD_SECONDS` (60 s), the sample resets the SoC through PSCI.
On this route that boots the same image again, so each reset gives another
boot-time sample. If it stops after step 2, write
`build/x2/flash-uboot-repacked.bin` the same way. If ABOS then boots, the
packing is right and the fault is in Zephyr's entry.

The first measurement to record has three parts: the host time from the SPL
banner to the Zephyr banner, the system counter at `main()`, and the counter
at the verdict.

## Recovery

- **Back to ABOS, eMMC intact.** Close JP1, put an SD card with Atmark's
  image at 32 KiB in CON1, power on, and stop U-Boot on UART2. Then
  `mmc partconf 2 0 1 0` (boot0; `2 0 2 0` for boot1), open JP1, and `reset`.
  If U-Boot cannot be stopped, it boots ABOS from the eMMC; then
  `mmc bootpart enable 1 0 /dev/mmcblk2` from Linux does the same thing.
  Prepare the card on the laptop (the device name is an example; check it
  with `lsblk` first):

  ```sh
  sudo dd if=build/x2/imx-boot_armadillo_x2_2020.04-at30 of=/dev/sdb bs=1k seek=32 conv=fsync
  ```

- **Zero-risk first try.** The same card trick runs the direct image without
  touching the eMMC:
  `sudo dd if=build/x2/flash-zephyr.bin of=/dev/sdb bs=1k seek=32 conv=fsync`,
  close JP1, and power on. The write starts at 32 KiB, so the card's partition
  table (MBR or GPT) survives.
- **Restore a backup:** `dd if=mmcblk2bootN.bak of=/dev/mmcblk2bootN bs=1M conv=fsync`
  after `echo 0 > /sys/block/mmcblk2bootN/force_ro`.

## Risks and open questions

- **Not run.** Every statement above about runtime behaviour comes from
  Atmark's source and the prebuilt image's bytes, not from the board.
- **The board's real boot image was not read.** On 2026-10-01 reading
  `mmcblk2boot*` and `/proc/iomem` over ssh was denied by the session's
  permission check. The at30 in the table is what the board's U-Boot version
  string says, not a byte comparison. Step 1 confirms where the image is.
- **The HAB state is assumed open.** It is checked in step 1.
- **OP-TEE** stays, because BL31 is built with `SPD=opteed`. It costs boot
  time; measure it from the gap between the BL31 and Zephyr lines. Dropping it
  needs a BL31 without opteed, which means building TF-A ourselves.
- **0x43200000 / zephyr.bin size**: TF-A and OP-TEE write there before Zephyr
  runs. A zephyr.bin over 48 MiB would be corrupted, so the limit is checked
  by eye for now (gc_slint is larger than gc_gl_triangle).
- **Boot partition size**: assumed 4 MiB, because of the environment at
  0x3FE000. The tool refuses images over 0x3FA000; for an SD card use
  `--limit 0`.
- **Cortex-M7**: BL31 releases it from reset with whatever is in its TCM.
  ABOS does the same today.
- **The display**: the cold LCDIF3 -> PVI -> DW-HDMI bring-up is separate
  work: written and host-tested, never run (`driver/docs/x2-hdmi-cold.md`).
  Until it runs in this route, the route shows nothing on HDMI.
